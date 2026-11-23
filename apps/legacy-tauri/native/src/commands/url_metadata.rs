use super::AppState;
use crate::models::UrlMeta;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use tauri::State;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use url::{Host, Url};

const HTML_BODY_MAX_BYTES: u64 = 1_048_576;
const FAVICON_BODY_MAX_BYTES: u64 = 65_536;
const URL_MAX_BYTES: usize = 8_192;
const TITLE_MAX_CHARS: usize = 512;
const DESCRIPTION_MAX_CHARS: usize = 2_048;
const SITE_NAME_MAX_CHARS: usize = 256;
const URL_META_MAX_CONCURRENCY: usize = 2;
static ACTIVE_URL_META_REQUESTS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Default)]
struct PublicOnlyResolver {
    inner: DefaultResolver,
}

impl Resolver for PublicOnlyResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let addresses = self.inner.resolve(uri, config, timeout)?;
        validate_resolved_addresses(&addresses)?;
        Ok(addresses)
    }
}

struct RequestPermit;

impl RequestPermit {
    fn acquire() -> Result<Self, String> {
        let previous = ACTIVE_URL_META_REQUESTS.fetch_add(1, Ordering::AcqRel);
        if previous >= URL_META_MAX_CONCURRENCY {
            ACTIVE_URL_META_REQUESTS.fetch_sub(1, Ordering::Release);
            return Err("链接预览请求繁忙，请稍后重试".to_string());
        }
        Ok(Self)
    }
}

impl Drop for RequestPermit {
    fn drop(&mut self) {
        ACTIVE_URL_META_REQUESTS.fetch_sub(1, Ordering::Release);
    }
}

/// 抓取 URL 的 Open Graph 元数据并写入缓存。
#[tauri::command]
pub async fn fetch_url_meta(url: String, state: State<'_, AppState>) -> Result<UrlMeta, String> {
    validate_url_syntax(&url)?;

    let url_clone = url.clone();
    let storage = state.storage.clone();
    let meta = tauri::async_runtime::spawn_blocking(move || -> Result<UrlMeta, String> {
        {
            let storage = storage.lock().map_err(|error| error.to_string())?;
            if let Ok(Some(cached)) = storage.get_url_meta(&url_clone) {
                return Ok(cached);
            }
        }
        let _permit = RequestPermit::acquire()?;
        let agent = safe_agent();
        let response = agent
            .get(&url_clone)
            .header("User-Agent", "Clippy/0.1 (Link Preview)")
            .call()
            .map_err(|e| format!("请求失败: {}", e))?;

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !content_type
            .split(';')
            .next()
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
        {
            return Err("非 HTML 页面".to_string());
        }

        let body = response
            .into_body()
            .with_config()
            .limit(HTML_BODY_MAX_BYTES)
            .read_to_string()
            .map_err(|e| format!("读取失败: {}", e))?;
        let mut meta = parse_og_meta(&url_clone, &body);
        if let Some(favicon_url) = meta.favicon.take() {
            match fetch_favicon(&agent, &favicon_url) {
                Ok(favicon) => meta.favicon = favicon,
                Err(error) => log::debug!("忽略不安全或不可用的站点图标: {error}"),
            }
        }
        Ok(meta)
    })
    .await
    .map_err(|e| format!("线程异常: {}", e))??;

    let storage = state.storage.lock().map_err(|error| error.to_string())?;
    let _ = storage.set_url_meta(&meta);
    Ok(meta)
}

fn safe_agent() -> ureq::Agent {
    let config = ureq::config::Config::builder()
        .timeout_global(Some(std::time::Duration::from_secs(5)))
        .max_redirects(0)
        // 环境代理会让校验器只看到代理地址，无法证明目标主机是公网地址。
        .proxy(None)
        .build();
    ureq::Agent::with_parts(
        config,
        DefaultConnector::default(),
        PublicOnlyResolver::default(),
    )
}

fn validate_resolved_addresses(addresses: &ResolvedSocketAddrs) -> Result<(), ureq::Error> {
    if addresses.is_empty() || addresses.iter().any(|address| is_private_ip(address.ip())) {
        return Err(ureq::Error::HostNotFound);
    }
    Ok(())
}

fn fetch_favicon(agent: &ureq::Agent, url: &str) -> Result<Option<String>, String> {
    validate_url_syntax(url)?;
    let response = agent
        .get(url)
        .header("User-Agent", "Clippy/0.1 (Link Preview)")
        .call()
        .map_err(|error| format!("图标请求失败: {error}"))?;
    let Some(mime) = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .and_then(safe_favicon_mime)
    else {
        return Ok(None);
    };
    let bytes = response
        .into_body()
        .with_config()
        .limit(FAVICON_BODY_MAX_BYTES + 1)
        .read_to_vec()
        .map_err(|error| format!("图标读取失败: {error}"))?;
    if bytes.is_empty() || bytes.len() as u64 > FAVICON_BODY_MAX_BYTES {
        return Ok(None);
    }
    Ok(Some(favicon_data_url(mime, &bytes)))
}

fn safe_favicon_mime(content_type: &str) -> Option<&'static str> {
    match content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/png" => Some("image/png"),
        "image/jpeg" => Some("image/jpeg"),
        "image/gif" => Some("image/gif"),
        "image/webp" => Some("image/webp"),
        "image/x-icon" | "image/vnd.microsoft.icon" => Some("image/x-icon"),
        _ => None,
    }
}

fn favicon_data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", STANDARD.encode(bytes))
}

fn parse_og_meta(url: &str, html: &str) -> UrlMeta {
    let get_meta = |property: &str| -> Option<String> {
        let pattern = format!(
            r#"<meta[^>]+(?:property|name)=["']{}["'][^>]+content=["']([^"']*)["']"#,
            regex_lite::escape(property)
        );
        if let Ok(re) = regex_lite::Regex::new(&pattern) {
            if let Some(captures) = re.captures(html) {
                let value = captures.get(1)?.as_str().trim().to_string();
                if !value.is_empty() {
                    return Some(html_decode(&value));
                }
            }
        }

        let reverse_pattern = format!(
            r#"<meta[^>]+content=["']([^"']*)["'][^>]+(?:property|name)=["']{}["']"#,
            regex_lite::escape(property)
        );
        if let Ok(re) = regex_lite::Regex::new(&reverse_pattern) {
            if let Some(captures) = re.captures(html) {
                let value = captures.get(1)?.as_str().trim().to_string();
                if !value.is_empty() {
                    return Some(html_decode(&value));
                }
            }
        }
        None
    };

    let title = get_meta("og:title")
        .or_else(|| {
            let re = regex_lite::Regex::new(r"<title[^>]*>([^<]+)</title>").ok()?;
            let captures = re.captures(html)?;
            let value = captures.get(1)?.as_str().trim().to_string();
            if value.is_empty() {
                None
            } else {
                Some(html_decode(&value))
            }
        })
        .map(|value| truncate_chars(value, TITLE_MAX_CHARS));
    let description = get_meta("og:description")
        .or_else(|| get_meta("description"))
        .map(|value| truncate_chars(value, DESCRIPTION_MAX_CHARS));
    let site_name =
        get_meta("og:site_name").map(|value| truncate_chars(value, SITE_NAME_MAX_CHARS));

    let favicon = {
        let re = regex_lite::Regex::new(
            r#"<link[^>]+rel=["'](?:icon|shortcut icon)["'][^>]+href=["']([^"']+)["']"#,
        )
        .ok();
        let href = re
            .and_then(|regex| regex.captures(html))
            .and_then(|captures| captures.get(1))
            .map(|favicon_match| favicon_match.as_str().trim().to_string());
        Url::parse(url).ok().and_then(|base| {
            href.as_deref()
                .map(|value| base.join(value))
                .unwrap_or_else(|| base.join("/favicon.ico"))
                .ok()
                .filter(|resolved| matches!(resolved.scheme(), "http" | "https"))
                .filter(|resolved| resolved.as_str().len() <= URL_MAX_BYTES)
                .map(|resolved| resolved.to_string())
        })
    };

    UrlMeta {
        url: url.to_string(),
        title,
        description,
        favicon,
        site_name,
    }
}

fn truncate_chars(mut value: String, max_chars: usize) -> String {
    if let Some((byte_index, _)) = value.char_indices().nth(max_chars) {
        value.truncate(byte_index);
    }
    value
}

fn html_decode(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

fn validate_url_syntax(url: &str) -> Result<(), String> {
    if url.is_empty() || url.len() > URL_MAX_BYTES || url != url.trim() {
        return Err("URL 不能为空或包含首尾空白".to_string());
    }
    let parsed = Url::parse(url).map_err(|_| "仅支持有效的 http/https URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("仅支持不含凭据的 http/https URL".to_string());
    }
    let private_literal = match parsed.host() {
        Some(Host::Ipv4(address)) => is_private_ip(IpAddr::V4(address)),
        Some(Host::Ipv6(address)) => is_private_ip(IpAddr::V6(address)),
        Some(Host::Domain(name)) => {
            is_private_domain_name(&name.trim_end_matches('.').to_ascii_lowercase())
        }
        None => true,
    };
    if private_literal {
        return Err("不允许请求内网地址".to_string());
    }
    Ok(())
}

fn is_private_domain_name(name: &str) -> bool {
    name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local")
}

fn is_private_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => is_private_ipv4(ip),
        IpAddr::V6(ip) => is_private_ipv6(ip),
    }
}

fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    a == 0
        || a == 10
        || (a == 100 && (64..=127).contains(&b))
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0)
        || (a == 192 && b == 168)
        || (a == 192 && b == 2)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224
}

fn is_private_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    if ip.is_loopback()
        || ip.is_unspecified()
        // IPv4-compatible 与 IPv4-mapped 都是特殊用途范围，不直接连接。
        || segments[..6].iter().all(|segment| *segment == 0)
        || (segments[..5].iter().all(|segment| *segment == 0) && segments[5] == 0xffff)
        || segments[0] & 0xfe00 == 0xfc00
    {
        return true;
    }
    if segments[0] & 0xffc0 == 0xfe80
        || segments[0] & 0xffc0 == 0xfec0
        || (segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2] == 1)
        || (segments[0] == 0x0100 && segments[1..4].iter().all(|segment| *segment == 0))
        // IETF 特殊用途、6to4、旧 6bone 与文档地址都不是普通公网 Web 目标。
        || (segments[0] == 0x2001 && segments[1] < 0x0200)
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || segments[0] == 0x2002
        || segments[0] == 0x3ffe
        || (segments[0] == 0x3fff && segments[1] & 0xf000 == 0)
        || segments[0] == 0x5f00
    {
        return true;
    }
    // 标准 NAT64 地址仍可服务 IPv6-only 网络，但内嵌地址必须通过 IPv4 公网策略。
    if segments[0] == 0x0064
        && segments[1] == 0xff9b
        && segments[2..6].iter().all(|segment| *segment == 0)
    {
        let mapped = Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        );
        return is_private_ipv4(mapped);
    }
    segments[0] & 0xff00 == 0xff00
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_syntax_and_rejects_private_literal_ranges() {
        assert!(validate_url_syntax("https://example.invalid/path").is_ok());
        for url in [
            "http://127.0.0.1:8080",
            "https://[::1]/",
            "https://[fd00::1]/",
            "http://169.254.10.2/",
            "https://user:secret@example.com/",
            "file:///tmp/example",
            " https://example.com",
        ] {
            assert!(
                validate_url_syntax(url).is_err(),
                "{url} should be rejected"
            );
        }
    }

    #[test]
    fn classifies_mapped_and_reserved_addresses() {
        assert!(validate_url_syntax("https://[::ffff:127.0.0.1]/").is_err());
        assert!(validate_url_syntax("https://[::ffff:8.8.8.8]/").is_err());
        assert!(validate_url_syntax("https://[::8.8.8.8]/").is_err());
        assert!(validate_url_syntax("https://224.0.0.1/").is_err());
        assert!(validate_url_syntax("https://198.51.100.10/").is_err());
        for url in [
            "https://[64:ff9b:1::1]/",
            "https://[100::1]/",
            "https://[2001:2::1]/",
            "https://[2001:db8::1]/",
            "https://[2002::1]/",
            "https://[3fff::1]/",
            "https://[5f00::1]/",
            "https://[fec0::1]/",
        ] {
            assert!(
                validate_url_syntax(url).is_err(),
                "{url} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_an_entire_dns_answer_if_any_address_is_not_public() {
        let mut addresses = <PublicOnlyResolver as Resolver>::empty(&PublicOnlyResolver::default());
        addresses.push("93.184.216.34:443".parse().unwrap());
        assert!(validate_resolved_addresses(&addresses).is_ok());

        addresses.push("127.0.0.1:443".parse().unwrap());
        assert!(matches!(
            validate_resolved_addresses(&addresses),
            Err(ureq::Error::HostNotFound)
        ));
    }

    #[test]
    fn resolves_relative_favicon_against_the_page_path() {
        let meta = parse_og_meta(
            "https://example.com/docs/page.html",
            r#"<link rel="icon" href="assets/icon.png">"#,
        );
        assert_eq!(
            meta.favicon.as_deref(),
            Some("https://example.com/docs/assets/icon.png")
        );
    }

    #[test]
    fn favicon_data_is_limited_to_passive_image_types() {
        assert_eq!(
            safe_favicon_mime("image/png; charset=binary"),
            Some("image/png")
        );
        assert_eq!(safe_favicon_mime("image/svg+xml"), None);
        assert_eq!(safe_favicon_mime("text/html"), None);
        assert_eq!(
            favicon_data_url("image/png", &[0x89, b'P', b'N', b'G']),
            "data:image/png;base64,iVBORw=="
        );
    }

    #[test]
    fn metadata_fields_and_urls_have_storage_bounds() {
        let long_title = "界".repeat(TITLE_MAX_CHARS + 1);
        let html = format!(r#"<meta property="og:title" content="{long_title}">"#);
        let meta = parse_og_meta("https://example.com", &html);
        assert_eq!(meta.title.unwrap().chars().count(), TITLE_MAX_CHARS);

        let oversized_url = format!("https://example.com/{}", "a".repeat(URL_MAX_BYTES));
        assert!(validate_url_syntax(&oversized_url).is_err());
    }

    #[test]
    fn request_permit_enforces_the_global_limit_and_releases_capacity() {
        ACTIVE_URL_META_REQUESTS.store(0, Ordering::Release);
        let first = RequestPermit::acquire().unwrap();
        let second = RequestPermit::acquire().unwrap();
        assert!(RequestPermit::acquire().is_err());
        drop(first);
        assert!(RequestPermit::acquire().is_ok());
        drop(second);
    }
}
