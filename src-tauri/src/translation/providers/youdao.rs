//! 有道翻译适配：官方开放平台 API + 未配置凭据时的 web 回退。
//!
//! 官方路径需要 appKey + appSecret 两个字段，签名是 v3 的 SHA-256 方案。
//! web 路径要先取一次密钥，再用 MD5 签名请求，响应是 AES-128-CBC 密文。
//! 参考项目还解析词典结果，Clippy 的结果模型没有词典字段，因此不实现。
//! web 路径没有可用性承诺，代价见 docs/reference-project-guidance.md。

use super::super::http::HttpRequest;
use super::super::service::ProviderClient;
use super::super::types::{
    ProviderCredentials, ProviderTranslation, TranslationError, TranslationRequest,
};
use super::routing::{route, Route};
use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockDecryptMut, KeyIvInit};
use base64::Engine;
use md5::Md5;
use serde_json::Value;
use sha2::{Digest, Sha256};

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

const WEB_REFERER: &str = "https://fanyi.youdao.com";
/// web 端点会按 cookie 里的 user id 做分桶，缺失时直接返回错误码。
const WEB_COOKIE: &str = "OUTFOX_SEARCH_USER_ID=1796239350@10.110.96.157;";
const WEB_CLIENT: &str = "fanyideskweb";
const WEB_PRODUCT: &str = "webfanyi";
/// 取密钥这一步用的固定 key，来自有道 web 前端。
const WEB_KEY_GETTER_KEY: &str = "asdjnjfenknafdfsdfsd";

pub(crate) struct YoudaoProvider;

impl ProviderClient for YoudaoProvider {
    fn translate(
        &self,
        request: &TranslationRequest,
        credentials: &ProviderCredentials,
    ) -> Result<ProviderTranslation, TranslationError> {
        match route(request.provider, credentials)? {
            Route::Official { key, secret } => {
                let secret = secret.ok_or(TranslationError::IncompleteCredentials)?;
                translate_official(request, key, secret)
            }
            Route::Web => translate_web(request),
        }
    }
}

fn translate_official(
    request: &TranslationRequest,
    app_key: &str,
    app_secret: &str,
) -> Result<ProviderTranslation, TranslationError> {
    let salt = salt();
    let curtime = unix_seconds().to_string();
    let sign = sign_v3(app_key, &request.text, &salt, &curtime, app_secret);
    let from = request.source().map(language).unwrap_or("auto");
    let to = language(&request.target_language);

    let response = HttpRequest::post(format!("{}/api", request.endpoint()))
        .form(&[
            ("q", request.text.as_str()),
            ("from", from),
            ("to", to),
            ("appKey", app_key),
            ("salt", salt.as_str()),
            ("curtime", curtime.as_str()),
            ("sign", sign.as_str()),
            ("signType", "v3"),
        ])
        .send_json()?;
    parse_official(&response)
}

/// 有道用 `zh-CHS`/`zh-CHT` 而不是 BCP 47 的中文标签。
fn language(code: &str) -> &str {
    match code.trim().to_ascii_lowercase().as_str() {
        "zh-hans" | "zh-cn" => "zh-CHS",
        "zh-hant" | "zh-tw" | "zh-hk" => "zh-CHT",
        "" => "auto",
        _ => code.trim(),
    }
}

/// 官方 v3 签名规则：`sha256(appKey + truncate(q) + salt + curtime + appSecret)`。
fn sign_v3(app_key: &str, text: &str, salt: &str, curtime: &str, app_secret: &str) -> String {
    let raw = format!("{app_key}{}{salt}{curtime}{app_secret}", truncate(text));
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hex_lower(hasher.finalize().as_slice())
}

/// 有道规定的 q 截断：超过 20 个字符时取「前 10 + 长度 + 后 10」。
/// 长度按字符数而不是字节数，中文输入下两者不同。
fn truncate(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= 20 {
        return text.to_string();
    }
    let head: String = chars.iter().take(10).collect();
    let tail: String = chars.iter().skip(chars.len() - 10).collect();
    format!("{head}{}{tail}", chars.len())
}

/// salt 只需要每次请求不同，不用于安全用途，所以用纳秒时间戳的十六进制即可。
fn salt() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}")
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}

fn parse_official(response: &Value) -> Result<ProviderTranslation, TranslationError> {
    let error_code = response
        .get("errorCode")
        .and_then(Value::as_str)
        .ok_or(TranslationError::InvalidResponse)?;
    if error_code != "0" {
        return Err(map_error_code(error_code));
    }

    let translated_text = join_lines(
        response
            .get("translation")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    )
    .ok_or(TranslationError::InvalidResponse)?;
    // `l` 形如 `en2zh-CHS`，前半段就是实际识别到的源语言。
    let detected_source_language = response
        .get("l")
        .and_then(Value::as_str)
        .and_then(detected_source);

    Ok(ProviderTranslation {
        translated_text,
        detected_source_language,
    })
}

/// 有道把所有失败都放在 errorCode 里返回 HTTP 200，所以必须自己映射成领域错误，
/// 否则用户只会看到“响应无效”，不知道是密钥错还是配额用完。
fn map_error_code(code: &str) -> TranslationError {
    match code {
        // 101 缺 appKey，102 缺 appSecret，108 两者不匹配。
        "101" | "102" | "108" => TranslationError::InvalidCredentials,
        "401" => TranslationError::QuotaExceeded,
        "411" => TranslationError::RateLimited,
        _ => TranslationError::InvalidResponse,
    }
}

fn detected_source(kind: &str) -> Option<String> {
    kind.split('2')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "auto")
        .map(str::to_string)
}

fn join_lines(lines: Vec<String>) -> Option<String> {
    let joined = lines.join("\n").trim().to_string();
    (!joined.is_empty()).then_some(joined)
}

fn translate_web(request: &TranslationRequest) -> Result<ProviderTranslation, TranslationError> {
    let base = request.web_endpoint().to_string();
    let key = fetch_web_key(&base)?;
    let timestamp = unix_millis().to_string();
    let sign = web_sign(&timestamp, &key.secret_key);
    let from = request.source().map(language).unwrap_or("auto");
    let to = language(&request.target_language);

    let body = HttpRequest::post(format!("{base}/webtranslate"))
        .header("Referer", WEB_REFERER)
        .header("Cookie", WEB_COOKIE)
        .form(&[
            ("client", WEB_CLIENT),
            ("product", WEB_PRODUCT),
            ("appVersion", "1.0.0"),
            ("vendor", "web"),
            ("pointParam", "client,mysticTime,product"),
            ("keyfrom", "fanyi.web"),
            ("i", request.text.as_str()),
            ("from", from),
            ("to", to),
            ("dictResult", "true"),
            ("keyid", WEB_PRODUCT),
            ("sign", sign.as_str()),
            ("mysticTime", timestamp.as_str()),
        ])
        .send_text()?;

    let decrypted = decrypt_web_payload(&body, &key.aes_key, &key.aes_iv)?;
    let response: Value =
        serde_json::from_str(&decrypted).map_err(|_| TranslationError::ProviderEndpointBroken)?;
    parse_web(&response)
}

/// web 路径每次请求都要先换一组临时密钥（签名密钥 + AES key/iv）。
struct WebKey {
    secret_key: String,
    aes_key: String,
    aes_iv: String,
}

fn fetch_web_key(base: &str) -> Result<WebKey, TranslationError> {
    let timestamp = unix_millis().to_string();
    let sign = web_sign(&timestamp, WEB_KEY_GETTER_KEY);
    let response = HttpRequest::get(format!("{base}/webtranslate/key"))
        .header("Referer", WEB_REFERER)
        .header("Cookie", WEB_COOKIE)
        .query(&[
            ("client", WEB_CLIENT),
            ("product", WEB_PRODUCT),
            ("appVersion", "1.0.0"),
            ("vendor", "web"),
            ("pointParam", "client,mysticTime,product"),
            ("keyfrom", "fanyi.web"),
            ("keyid", "webfanyi-key-getter"),
            ("sign", sign.as_str()),
            ("mysticTime", timestamp.as_str()),
        ])
        .send_json()?;
    parse_web_key(&response)
}

fn parse_web_key(response: &Value) -> Result<WebKey, TranslationError> {
    if response.get("code").and_then(Value::as_i64) != Some(0) {
        return Err(TranslationError::ProviderEndpointBroken);
    }
    let data = response
        .get("data")
        .ok_or(TranslationError::ProviderEndpointBroken)?;
    let field = |name: &str| {
        data.get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or(TranslationError::ProviderEndpointBroken)
    };

    Ok(WebKey {
        secret_key: field("secretKey")?,
        aes_key: field("aesKey")?,
        aes_iv: field("aesIv")?,
    })
}

/// web 端点的签名：对固定字段串做 MD5。
fn web_sign(timestamp: &str, key: &str) -> String {
    let raw = format!("client={WEB_CLIENT}&mysticTime={timestamp}&product={WEB_PRODUCT}&key={key}");
    let mut hasher = Md5::new();
    hasher.update(raw.as_bytes());
    hex_lower(hasher.finalize().as_slice())
}

/// 响应是 URL-safe base64 的 AES-128-CBC 密文，key/iv 是各自 MD5 的原始 16 字节。
fn decrypt_web_payload(encrypted: &str, key: &str, iv: &str) -> Result<String, TranslationError> {
    let mut encoded = encrypted.trim().replace('-', "+").replace('_', "/");
    while !encoded.len().is_multiple_of(4) {
        encoded.push('=');
    }
    let mut buffer = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| TranslationError::ProviderEndpointBroken)?;

    let decryptor = Aes128CbcDec::new_from_slices(&md5_bytes(key), &md5_bytes(iv))
        .map_err(|_| TranslationError::ProviderEndpointBroken)?;
    let plain = decryptor
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map_err(|_| TranslationError::ProviderEndpointBroken)?;
    String::from_utf8(plain.to_vec()).map_err(|_| TranslationError::ProviderEndpointBroken)
}

fn md5_bytes(value: &str) -> [u8; 16] {
    let mut hasher = Md5::new();
    hasher.update(value.as_bytes());
    hasher.finalize().into()
}

fn parse_web(response: &Value) -> Result<ProviderTranslation, TranslationError> {
    if response.get("code").and_then(Value::as_i64) != Some(0) {
        return Err(TranslationError::ProviderEndpointBroken);
    }
    // translateResult 是「段落 -> 分句」两层数组，同段内的分句直接相连。
    let groups = response
        .get("translateResult")
        .and_then(Value::as_array)
        .ok_or(TranslationError::ProviderEndpointBroken)?;
    let translated_text = join_lines(
        groups
            .iter()
            .filter_map(Value::as_array)
            .map(|group| {
                group
                    .iter()
                    .filter_map(|item| item.get("tgt").and_then(Value::as_str))
                    .collect::<String>()
            })
            .collect(),
    )
    .ok_or(TranslationError::ProviderEndpointBroken)?;
    let detected_source_language = response
        .get("type")
        .and_then(Value::as_str)
        .and_then(detected_source);

    Ok(ProviderTranslation {
        translated_text,
        detected_source_language,
    })
}

#[cfg(test)]
mod tests {
    use super::super::super::test_support::{MockResponse, MockServer};
    use super::super::super::types::{ProviderOptions, TranslationProvider};
    use super::*;
    use aes::cipher::{block_padding::Pkcs7 as EncPkcs7, BlockEncryptMut};
    use serde_json::json;

    type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

    const AES_KEY: &str = "test-aes-key";
    const AES_IV: &str = "test-aes-iv";

    fn request(source: &str, options: ProviderOptions) -> TranslationRequest {
        TranslationRequest::with_options(
            "Hello".to_string(),
            source.to_string(),
            "zh-Hans".to_string(),
            TranslationProvider::Youdao,
            options,
            1,
        )
    }

    /// 用与实现相同的 key/iv 派生方式加密，验证解密路径而不是重复实现。
    fn encrypt_web_payload(plain: &str) -> String {
        let mut buffer = vec![0_u8; plain.len() + 16];
        buffer[..plain.len()].copy_from_slice(plain.as_bytes());
        let encrypted = Aes128CbcEnc::new_from_slices(&md5_bytes(AES_KEY), &md5_bytes(AES_IV))
            .unwrap()
            .encrypt_padded_mut::<EncPkcs7>(&mut buffer, plain.len())
            .unwrap();
        base64::engine::general_purpose::STANDARD
            .encode(encrypted)
            .replace('+', "-")
            .replace('/', "_")
            .trim_end_matches('=')
            .to_string()
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        assert_eq!(truncate("short text"), "short text");
        let long = "abcdefghijklmnopqrstuvwxyz";
        assert_eq!(truncate(long), "abcdefghij26qrstuvwxyz");
        // 26 个中文字符同样按字符数截断。
        let chinese: String = "中".repeat(26);
        let truncated = truncate(&chinese);
        assert!(truncated.contains("26"));
        assert_eq!(truncated.chars().count(), 22);
    }

    #[test]
    fn sign_v3_follows_the_documented_field_order() {
        let expected = {
            let mut hasher = Sha256::new();
            hasher.update(b"app-keyHellosalt1700000000app-secret");
            hex_lower(hasher.finalize().as_slice())
        };
        assert_eq!(
            sign_v3("app-key", "Hello", "salt", "1700000000", "app-secret"),
            expected
        );
        assert_eq!(sign_v3("k", "t", "s", "1", "x").len(), 64);
    }

    #[test]
    fn chinese_targets_use_the_youdao_language_ids() {
        assert_eq!(language("zh-Hans"), "zh-CHS");
        assert_eq!(language("zh-TW"), "zh-CHT");
        assert_eq!(language(""), "auto");
        assert_eq!(language(" de "), "de");
    }

    #[test]
    fn official_path_sends_the_v3_signature_fields() {
        let server = MockServer::json_once(json!({
            "errorCode": "0",
            "translation": ["你好"],
            "l": "en2zh-CHS"
        }));
        let request = request(
            "auto",
            ProviderOptions {
                endpoint: server.base_url.clone(),
                ..ProviderOptions::default()
            },
        );

        let result = YoudaoProvider
            .translate(
                &request,
                &ProviderCredentials::new(
                    Some("app-key".to_string()),
                    Some("app-secret".to_string()),
                ),
            )
            .unwrap();
        let captured = server.recv();
        server.finish();

        assert_eq!(captured.method(), "POST");
        assert_eq!(captured.target(), "/api");
        let form = captured.form();
        assert_eq!(form.get("q").map(String::as_str), Some("Hello"));
        assert_eq!(form.get("from").map(String::as_str), Some("auto"));
        assert_eq!(form.get("to").map(String::as_str), Some("zh-CHS"));
        assert_eq!(form.get("appKey").map(String::as_str), Some("app-key"));
        assert_eq!(form.get("signType").map(String::as_str), Some("v3"));
        // appSecret 只参与签名，绝不能出现在请求体里。
        assert!(!captured.body.contains("app-secret"));
        let salt = form.get("salt").unwrap();
        let curtime = form.get("curtime").unwrap();
        assert_eq!(
            form.get("sign").map(String::as_str),
            Some(sign_v3("app-key", "Hello", salt, curtime, "app-secret").as_str())
        );
        assert_eq!(result.translated_text, "你好");
        assert_eq!(result.detected_source_language.as_deref(), Some("en"));
    }

    #[test]
    fn official_error_codes_map_to_actionable_errors() {
        assert_eq!(
            parse_official(&json!({ "errorCode": "108" })),
            Err(TranslationError::InvalidCredentials)
        );
        assert_eq!(
            parse_official(&json!({ "errorCode": "401" })),
            Err(TranslationError::QuotaExceeded)
        );
        assert_eq!(
            parse_official(&json!({ "errorCode": "411" })),
            Err(TranslationError::RateLimited)
        );
        assert_eq!(
            parse_official(&json!({ "errorCode": "0", "translation": [] })),
            Err(TranslationError::InvalidResponse)
        );
        assert_eq!(
            parse_official(&json!({ "errorCode": "0", "translation": ["a", "b"] }))
                .unwrap()
                .translated_text,
            "a\nb"
        );
    }

    #[test]
    fn official_path_requires_both_credential_fields() {
        let request = request(
            "auto",
            ProviderOptions {
                endpoint: "https://openapi.youdao.test".to_string(),
                ..ProviderOptions::default()
            },
        );
        assert_eq!(
            YoudaoProvider.translate(
                &request,
                &ProviderCredentials::new(Some("app-key".to_string()), None)
            ),
            Err(TranslationError::IncompleteCredentials)
        );
    }

    #[test]
    fn web_signature_covers_the_fixed_field_order() {
        let expected = {
            let mut hasher = Md5::new();
            hasher.update(
                b"client=fanyideskweb&mysticTime=1700000000000&product=webfanyi&key=secret",
            );
            hex_lower(hasher.finalize().as_slice())
        };
        assert_eq!(web_sign("1700000000000", "secret"), expected);
    }

    #[test]
    fn web_payload_round_trips_through_aes_cbc() {
        let plain = r#"{"code":0,"translateResult":[[{"tgt":"你好","src":"Hello"}]]}"#;
        let decrypted = decrypt_web_payload(&encrypt_web_payload(plain), AES_KEY, AES_IV).unwrap();
        assert_eq!(decrypted, plain);

        // 密文被截断或换了密钥时是接口失效，不是用户配置问题。
        assert_eq!(
            decrypt_web_payload("not-base64!!", AES_KEY, AES_IV),
            Err(TranslationError::ProviderEndpointBroken)
        );
        assert_eq!(
            decrypt_web_payload(&encrypt_web_payload(plain), "other-key", AES_IV),
            Err(TranslationError::ProviderEndpointBroken)
        );
    }

    #[test]
    fn web_path_fetches_a_key_then_decrypts_the_translation() {
        let payload = json!({
            "code": 0,
            "type": "en2zh-CHS",
            "translateResult": [[{ "tgt": "你好", "src": "Hello" }]]
        })
        .to_string();
        let server = MockServer::new(vec![
            MockResponse::json(json!({
                "code": 0,
                "msg": "OK",
                "data": { "secretKey": "secret", "aesKey": AES_KEY, "aesIv": AES_IV }
            })),
            MockResponse::text(encrypt_web_payload(&payload)),
        ]);
        let request = request(
            "auto",
            ProviderOptions {
                web_endpoint: server.base_url.clone(),
                ..ProviderOptions::default()
            },
        );

        let result = YoudaoProvider
            .translate(&request, &ProviderCredentials::default())
            .unwrap();
        let key_request = server.recv();
        let translate_request = server.recv();
        server.finish();

        assert_eq!(key_request.method(), "GET");
        assert!(key_request.target().starts_with("/webtranslate/key?"));
        assert_eq!(
            key_request.query().get("keyid").map(String::as_str),
            Some("webfanyi-key-getter")
        );
        assert_eq!(key_request.header("referer").as_deref(), Some(WEB_REFERER));

        assert_eq!(translate_request.method(), "POST");
        assert_eq!(translate_request.target(), "/webtranslate");
        let form = translate_request.form();
        assert_eq!(form.get("i").map(String::as_str), Some("Hello"));
        assert_eq!(form.get("to").map(String::as_str), Some("zh-CHS"));
        let timestamp = form.get("mysticTime").unwrap();
        assert_eq!(
            form.get("sign").map(String::as_str),
            Some(web_sign(timestamp, "secret").as_str())
        );
        assert_eq!(result.translated_text, "你好");
        assert_eq!(result.detected_source_language.as_deref(), Some("en"));
    }

    #[test]
    fn web_responses_report_a_broken_endpoint_on_shape_changes() {
        assert_eq!(
            parse_web(&json!({ "code": 40 })),
            Err(TranslationError::ProviderEndpointBroken)
        );
        assert_eq!(
            parse_web(&json!({ "code": 0 })),
            Err(TranslationError::ProviderEndpointBroken)
        );
        assert_eq!(
            parse_web(&json!({ "code": 0, "translateResult": [[]] })),
            Err(TranslationError::ProviderEndpointBroken)
        );
        assert_eq!(
            parse_web_key(&json!({ "code": 0, "data": { "secretKey": "s" } }))
                .err()
                .map(|error| error.code()),
            Some("provider_endpoint_broken")
        );
        // 多段结果按段落换行拼接。
        assert_eq!(
            parse_web(&json!({
                "code": 0,
                "translateResult": [[{ "tgt": "第一" }, { "tgt": "段" }], [{ "tgt": "第二段" }]]
            }))
            .unwrap()
            .translated_text,
            "第一段\n第二段"
        );
    }
}
