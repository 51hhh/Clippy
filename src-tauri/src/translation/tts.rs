//! 文本转语音：有道词典的公开 dictvoice 端点。
//!
//! 行为参考 translator 的 `audio.rs`（GPL-3.0-only，未复制代码）：同一个 dictvoice
//! 端点，`le` 为语种、`type` 为口音。它是词典发音端点，只对短文本有意义，
//! 因此这里对长度设了上限而不是把整段剪贴板内容发出去。
//!
//! 音频由 Rust 取回后以 base64 交给前端播放，而不是让 webview 直接请求远端：
//! 这样超时、1 MiB 上限和端点白名单只有 `http.rs` 一处实现，
//! webview 的 CSP 也不必为一个第三方主机放开 `media-src`。
//! web 端点没有可用性承诺，代价见 docs/reference-project-guidance.md。

use super::http::HttpRequest;
use super::types::TranslationError;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;

/// dictvoice 所在主机。不开放给用户配置：它不是翻译服务，没有自托管对应物。
const AUDIO_ENDPOINT: &str = "https://dict.youdao.com";
/// dictvoice 只发一个词或一句话。超过这个长度直接拒绝，不把长文本发出去。
const MAX_SPOKEN_CHARS: usize = 200;
/// dictvoice 返回 MP3。前端据此拼 data URL，不再猜格式。
const AUDIO_MIME_TYPE: &str = "audio/mpeg";

/// 一段可直接播放的音频。前端拼成 `data:{mime_type};base64,{audio_base64}`。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SpokenText {
    pub mime_type: String,
    pub audio_base64: String,
}

/// 取回一段文本的发音。`language` 为 None 或 "auto" 时按英语发音，
/// 因为 dictvoice 不做语种检测，必须给它一个具体值。
pub(crate) fn fetch_audio(
    text: &str,
    language: Option<&str>,
) -> Result<SpokenText, TranslationError> {
    fetch_audio_from(AUDIO_ENDPOINT, text, language)
}

fn fetch_audio_from(
    endpoint: &str,
    text: &str,
    language: Option<&str>,
) -> Result<SpokenText, TranslationError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(TranslationError::EmptyInput);
    }
    if text.chars().count() > MAX_SPOKEN_CHARS {
        return Err(TranslationError::InputTooLarge);
    }
    let audio = HttpRequest::get(format!("{}/dictvoice", endpoint.trim_end_matches('/')))
        .header("Accept", "audio/mpeg")
        .query(&[
            ("audio", text),
            ("le", &spoken_language(language)),
            // 1 是英式，2 是美式；只有英语用得到，其他语种忽略这个参数。
            ("type", "2"),
        ])
        .send_bytes()?;
    // 端点出错时会返回 HTML 页面。把它当成 MP3 交给前端只会得到一次静默失败。
    if !is_mp3(&audio) {
        return Err(TranslationError::InvalidResponse);
    }
    Ok(SpokenText {
        mime_type: AUDIO_MIME_TYPE.to_string(),
        audio_base64: STANDARD.encode(audio),
    })
}

/// dictvoice 的 `le` 只认主语种码：中文的各种变体都发普通话，
/// 没有语种信息时按英语发音而不是让端点自己猜。
fn spoken_language(language: Option<&str>) -> String {
    let language = language.unwrap_or_default().trim().to_ascii_lowercase();
    let primary = language
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_string();
    match primary.as_str() {
        "" | "auto" => "en".to_string(),
        primary => primary.to_string(),
    }
}

/// ID3 标签或 MPEG 帧同步头。只用于区分音频与错误页面，不解析音频内容。
fn is_mp3(audio: &[u8]) -> bool {
    audio.starts_with(b"ID3") || matches!(audio, [0xFF, second, ..] if second & 0xE0 == 0xE0)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{MockResponse, MockServer};
    use super::*;

    /// MP3 帧同步头开头的一小段假音频。
    const FAKE_MP3: &str = "ID3\u{3}fake-mp3-body";

    #[test]
    fn the_request_carries_the_text_language_and_accent() {
        let server = MockServer::new(vec![MockResponse::audio(FAKE_MP3)]);
        let spoken = fetch_audio_from(&server.base_url, "  你好，世界  ", Some("zh-Hans")).unwrap();

        let request = server.recv();
        assert_eq!(request.method(), "GET");
        assert!(request.target().starts_with("/dictvoice?"));
        let query = request.query();
        // 前后空白不发出去，语种折叠成主语种码。
        assert_eq!(query.get("audio").unwrap(), "你好，世界");
        assert_eq!(query.get("le").unwrap(), "zh");
        assert_eq!(query.get("type").unwrap(), "2");
        assert_eq!(spoken.mime_type, "audio/mpeg");
        assert_eq!(
            STANDARD.decode(spoken.audio_base64).unwrap(),
            FAKE_MP3.as_bytes()
        );
        server.finish();
    }

    #[test]
    fn a_missing_language_falls_back_to_english_instead_of_guessing() {
        for language in [None, Some(""), Some("auto"), Some("  AUTO  ")] {
            assert_eq!(spoken_language(language), "en");
        }
        assert_eq!(spoken_language(Some("zh-TW")), "zh");
        assert_eq!(spoken_language(Some("en_US")), "en");
        assert_eq!(spoken_language(Some("JA")), "ja");
    }

    #[test]
    fn blank_and_overlong_text_never_reaches_the_endpoint() {
        assert_eq!(
            fetch_audio_from("https://dict.example.test", "   \n", None),
            Err(TranslationError::EmptyInput)
        );
        let long_text = "字".repeat(MAX_SPOKEN_CHARS + 1);
        assert_eq!(
            fetch_audio_from("https://dict.example.test", &long_text, None),
            Err(TranslationError::InputTooLarge)
        );
    }

    #[test]
    fn an_error_page_is_rejected_instead_of_being_played_as_audio() {
        let server = MockServer::new(vec![MockResponse::html("<html>error</html>")]);
        assert_eq!(
            fetch_audio_from(&server.base_url, "hello", Some("en")),
            Err(TranslationError::InvalidResponse)
        );
        server.recv();
        server.finish();
    }

    #[test]
    fn mpeg_frame_headers_count_as_audio() {
        assert!(is_mp3(b"ID3\x03rest"));
        assert!(is_mp3(&[0xFF, 0xFB, 0x90]));
        assert!(!is_mp3(&[0xFF, 0x01]));
        assert!(!is_mp3(b"<html>"));
        assert!(!is_mp3(b""));
    }
}
