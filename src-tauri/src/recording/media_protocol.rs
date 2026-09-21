//! 录屏结果窗专属的媒体读取协议。
//!
//! 播放准备命令先按恢复清单完整校验产物，再签发短小的不透明租约。协议只接受 `recordings`
//! WebView，并把每次响应限制为 2 MiB；这样视频元素可以用 byte range 解码，而不会把整段录屏复制
//! 到 invoke、JSON 或 JS Blob。

use super::manifest::ResolvedRecordingArtifact;
use crate::commands::AppState;
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;
use tauri::{http, Manager, Runtime, UriSchemeContext};

const LIBRARY_LABEL: &str = "recordings";
const MAX_LEASES: usize = 8;
const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordingMediaLeaseInfo {
    pub token: String,
    pub mime_type: &'static str,
}

#[derive(Debug, Clone)]
struct RecordingMediaLease {
    session_id: String,
    path: PathBuf,
    byte_length: u64,
    modified: SystemTime,
    mime_type: &'static str,
}

#[derive(Debug, Default)]
struct RecordingMediaState {
    generation: u64,
    next_token: u64,
    order: VecDeque<String>,
    leases: HashMap<String, RecordingMediaLease>,
}

#[derive(Debug, Default)]
pub(crate) struct RecordingMediaManager {
    inner: Mutex<RecordingMediaState>,
}

impl RecordingMediaManager {
    pub(super) fn generation(&self) -> Result<u64, String> {
        self.inner
            .lock()
            .map(|state| state.generation)
            .map_err(|error| format!("录屏播放租约锁损坏: {error}"))
    }

    pub(super) fn issue(
        &self,
        generation: u64,
        session_id: &str,
        artifact: &ResolvedRecordingArtifact,
    ) -> Result<RecordingMediaLeaseInfo, String> {
        if artifact
            .path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| !value.eq_ignore_ascii_case("webm"))
            .unwrap_or(true)
        {
            return Err("当前录屏格式不支持库内播放".to_string());
        }
        let metadata_before = fs::symlink_metadata(&artifact.path)
            .map_err(|error| format!("读取录屏产物失败: {error}"))?;
        if metadata_before.file_type().is_symlink()
            || !metadata_before.is_file()
            || metadata_before.len() == 0
            || metadata_before.len() != artifact.byte_length
        {
            return Err("录屏产物与恢复清单不一致".to_string());
        }
        let modified = metadata_before
            .modified()
            .map_err(|error| format!("读取录屏产物修改时间失败: {error}"))?;
        // 清单哈希按 64 KiB 读取；每约 16 MiB 回看一次窗口代次，避免用户关闭结果窗后仍把
        // 数 GiB 文件完整扫完。首块也检查，已关闭的窗口不会重新开始 I/O。
        let mut checkpoint_index = 0_u8;
        super::manifest::verify_library_artifact_with_checkpoint(artifact, || {
            checkpoint_index = checkpoint_index.wrapping_add(1);
            if (checkpoint_index == 1 || checkpoint_index == 0) && self.generation()? != generation
            {
                return Err("录屏结果窗已经关闭".to_string());
            }
            Ok(())
        })?;
        let metadata_after = fs::symlink_metadata(&artifact.path)
            .map_err(|error| format!("再次读取录屏产物失败: {error}"))?;
        if metadata_after.file_type().is_symlink()
            || !metadata_after.is_file()
            || metadata_after.len() != metadata_before.len()
            || metadata_after.modified().ok() != Some(modified)
        {
            return Err("录屏产物在校验期间发生变化".to_string());
        }
        let lease = RecordingMediaLease {
            session_id: session_id.to_string(),
            path: artifact.path.clone(),
            byte_length: artifact.byte_length,
            modified,
            mime_type: "video/webm",
        };
        let mut state = self
            .inner
            .lock()
            .map_err(|error| format!("录屏播放租约锁损坏: {error}"))?;
        if state.generation != generation {
            return Err("录屏结果窗已经关闭".to_string());
        }
        state.next_token = state.next_token.wrapping_add(1).max(1);
        let token = format!("media-{:016x}", state.next_token);
        state.order.push_back(token.clone());
        state.leases.insert(token.clone(), lease);
        while state.order.len() > MAX_LEASES {
            if let Some(expired) = state.order.pop_front() {
                state.leases.remove(&expired);
            }
        }
        Ok(RecordingMediaLeaseInfo {
            token,
            mime_type: "video/webm",
        })
    }

    pub(crate) fn revoke(&self, token: &str) -> Result<(), String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|error| format!("录屏播放租约锁损坏: {error}"))?;
        state.leases.remove(token);
        state.order.retain(|candidate| candidate != token);
        Ok(())
    }

    pub(crate) fn revoke_session(&self, session_id: &str) -> Result<(), String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|error| format!("录屏播放租约锁损坏: {error}"))?;
        let expired: Vec<_> = state
            .leases
            .iter()
            .filter_map(|(token, lease)| (lease.session_id == session_id).then_some(token.clone()))
            .collect();
        for token in &expired {
            state.leases.remove(token);
        }
        state.order.retain(|token| !expired.contains(token));
        Ok(())
    }

    pub(crate) fn clear(&self) -> Result<(), String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|error| format!("录屏播放租约锁损坏: {error}"))?;
        state.generation = state.generation.wrapping_add(1);
        state.order.clear();
        state.leases.clear();
        Ok(())
    }

    fn get(&self, token: &str) -> Result<Option<RecordingMediaLease>, String> {
        let state = self
            .inner
            .lock()
            .map_err(|error| format!("录屏播放租约锁损坏: {error}"))?;
        Ok(state.leases.get(token).cloned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ByteRange {
    start: u64,
    end: u64,
}

fn parse_token<'a>(path: &'a str, query: Option<&str>) -> Option<&'a str> {
    if query.is_some() {
        return None;
    }
    let token = path.strip_prefix('/')?;
    if token.len() != 22
        || !token.starts_with("media-")
        || !token[6..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    Some(token)
}

fn parse_range(value: Option<&str>, total: u64) -> Result<Option<ByteRange>, ()> {
    let Some(value) = value else {
        return Ok(None);
    };
    let raw = value.strip_prefix("bytes=").ok_or(())?;
    if raw.contains(',') || total == 0 {
        return Err(());
    }
    let (start, end) = raw.split_once('-').ok_or(())?;
    let requested = if start.is_empty() {
        let suffix = end.parse::<u64>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        let suffix = suffix.min(total).min(MAX_RESPONSE_BYTES);
        ByteRange {
            start: total - suffix,
            end: total - 1,
        }
    } else {
        let start = start.parse::<u64>().map_err(|_| ())?;
        if start >= total {
            return Err(());
        }
        let end = if end.is_empty() {
            total - 1
        } else {
            end.parse::<u64>().map_err(|_| ())?.min(total - 1)
        };
        if end < start {
            return Err(());
        }
        ByteRange { start, end }
    };
    Ok(Some(ByteRange {
        start: requested.start,
        end: requested
            .end
            .min(requested.start.saturating_add(MAX_RESPONSE_BYTES - 1)),
    }))
}

fn response(
    status: http::StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, content_type)
        .header(http::header::CACHE_CONTROL, "no-store")
        .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(http::header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(body)
        .expect("录屏媒体协议响应头必须有效")
}

fn range_error(total: u64) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(http::StatusCode::RANGE_NOT_SATISFIABLE)
        .header(http::header::CONTENT_TYPE, "text/plain")
        .header(http::header::CONTENT_RANGE, format!("bytes */{total}"))
        .header(http::header::ACCEPT_RANGES, "bytes")
        .header(http::header::CACHE_CONTROL, "no-store")
        .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(b"invalid range".to_vec())
        .expect("录屏媒体 range 错误响应头必须有效")
}

fn serve(
    lease: &RecordingMediaLease,
    method: &http::Method,
    range_header: Option<&str>,
) -> http::Response<Vec<u8>> {
    if method != http::Method::GET && method != http::Method::HEAD {
        return http::Response::builder()
            .status(http::StatusCode::METHOD_NOT_ALLOWED)
            .header(http::header::ALLOW, "GET, HEAD")
            .body(Vec::new())
            .expect("录屏媒体 method 错误响应头必须有效");
    }
    let metadata = match fs::symlink_metadata(&lease.path) {
        Ok(metadata)
            if !metadata.file_type().is_symlink()
                && metadata.is_file()
                && metadata.len() == lease.byte_length
                && metadata.modified().ok() == Some(lease.modified) =>
        {
            metadata
        }
        _ => {
            return response(
                http::StatusCode::CONFLICT,
                "text/plain",
                b"recording changed".to_vec(),
            );
        }
    };
    let requested = match parse_range(range_header, metadata.len()) {
        Ok(value) => value,
        Err(()) => return range_error(metadata.len()),
    };
    let (range, partial) = match requested {
        Some(range) => (range, true),
        None if metadata.len() > MAX_RESPONSE_BYTES => (
            ByteRange {
                start: 0,
                end: MAX_RESPONSE_BYTES - 1,
            },
            true,
        ),
        None => (
            ByteRange {
                start: 0,
                end: metadata.len().saturating_sub(1),
            },
            false,
        ),
    };
    let length = range.end.saturating_sub(range.start).saturating_add(1);
    let body = if method == http::Method::HEAD {
        Vec::new()
    } else {
        let mut file = match File::open(&lease.path) {
            Ok(file) => file,
            Err(_) => {
                return response(
                    http::StatusCode::NOT_FOUND,
                    "text/plain",
                    b"recording unavailable".to_vec(),
                );
            }
        };
        if file.seek(SeekFrom::Start(range.start)).is_err() {
            return response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                "text/plain",
                b"recording seek failed".to_vec(),
            );
        }
        let Ok(buffer_len) = usize::try_from(length) else {
            return range_error(metadata.len());
        };
        let mut bytes = vec![0; buffer_len];
        if file.read_exact(&mut bytes).is_err() {
            return response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                "text/plain",
                b"recording read failed".to_vec(),
            );
        }
        bytes
    };
    let mut builder = http::Response::builder()
        .status(if partial {
            http::StatusCode::PARTIAL_CONTENT
        } else {
            http::StatusCode::OK
        })
        .header(http::header::CONTENT_TYPE, lease.mime_type)
        .header(http::header::CONTENT_LENGTH, length.to_string())
        .header(http::header::ACCEPT_RANGES, "bytes")
        .header(http::header::CACHE_CONTROL, "no-store")
        .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(http::header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    if partial {
        builder = builder.header(
            http::header::CONTENT_RANGE,
            format!("bytes {}-{}/{}", range.start, range.end, metadata.len()),
        );
    }
    builder.body(body).expect("录屏媒体响应头必须有效")
}

pub(crate) fn handle<R: Runtime>(
    context: UriSchemeContext<'_, R>,
    request: http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    if context.webview_label() != LIBRARY_LABEL {
        return response(
            http::StatusCode::FORBIDDEN,
            "text/plain",
            b"forbidden".to_vec(),
        );
    }
    let Some(token) = parse_token(request.uri().path(), request.uri().query()) else {
        return response(
            http::StatusCode::FORBIDDEN,
            "text/plain",
            b"forbidden".to_vec(),
        );
    };
    let state = context.app_handle().state::<AppState>();
    let lease = match state.recording_media.get(token) {
        Ok(Some(lease)) => lease,
        Ok(None) => {
            return response(
                http::StatusCode::NOT_FOUND,
                "text/plain",
                b"recording unavailable".to_vec(),
            );
        }
        Err(_) => {
            return response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                "text/plain",
                b"recording unavailable".to_vec(),
            );
        }
    };
    serve(
        &lease,
        request.method(),
        request
            .headers()
            .get(http::header::RANGE)
            .and_then(|value| value.to_str().ok()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;
    use std::io::Write;

    fn lease(path: PathBuf) -> RecordingMediaLease {
        let metadata = fs::symlink_metadata(&path).unwrap();
        RecordingMediaLease {
            session_id: "session-a".to_string(),
            path,
            byte_length: metadata.len(),
            modified: metadata.modified().unwrap(),
            mime_type: "video/webm",
        }
    }

    #[test]
    fn token_and_range_parsing_are_bounded() {
        assert_eq!(
            parse_token("/media-0000000000000001", None),
            Some("media-0000000000000001")
        );
        assert!(parse_token("/media-1", None).is_none());
        assert!(parse_token("/media-0000000000000001", Some("x=1")).is_none());
        assert_eq!(
            parse_range(Some("bytes=2-5"), 10),
            Ok(Some(ByteRange { start: 2, end: 5 }))
        );
        assert_eq!(
            parse_range(Some("bytes=7-"), 10),
            Ok(Some(ByteRange { start: 7, end: 9 }))
        );
        assert_eq!(
            parse_range(Some("bytes=-3"), 10),
            Ok(Some(ByteRange { start: 7, end: 9 }))
        );
        assert!(parse_range(Some("bytes=10-"), 10).is_err());
        assert!(parse_range(Some("bytes=0-1,4-5"), 10).is_err());
    }

    #[test]
    fn media_response_supports_get_head_ranges_and_change_detection() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("recording.webm");
        fs::write(&path, b"0123456789").unwrap();
        let lease = lease(path.clone());

        let full = serve(&lease, &http::Method::GET, None);
        assert_eq!(full.status(), http::StatusCode::OK);
        assert_eq!(full.body(), b"0123456789");
        assert_eq!(full.headers()[http::header::ACCEPT_RANGES], "bytes");

        let partial = serve(&lease, &http::Method::GET, Some("bytes=3-6"));
        assert_eq!(partial.status(), http::StatusCode::PARTIAL_CONTENT);
        assert_eq!(partial.body(), b"3456");
        assert_eq!(
            partial.headers()[http::header::CONTENT_RANGE],
            "bytes 3-6/10"
        );

        let open_ended = serve(&lease, &http::Method::GET, Some("bytes=7-"));
        assert_eq!(open_ended.status(), http::StatusCode::PARTIAL_CONTENT);
        assert_eq!(open_ended.body(), b"789");

        let suffix = serve(&lease, &http::Method::GET, Some("bytes=-3"));
        assert_eq!(suffix.status(), http::StatusCode::PARTIAL_CONTENT);
        assert_eq!(suffix.body(), b"789");

        let head = serve(&lease, &http::Method::HEAD, Some("bytes=0-3"));
        assert_eq!(head.status(), http::StatusCode::PARTIAL_CONTENT);
        assert!(head.body().is_empty());
        assert_eq!(head.headers()[http::header::CONTENT_LENGTH], "4");

        assert_eq!(
            serve(&lease, &http::Method::GET, Some("bytes=10-")).status(),
            http::StatusCode::RANGE_NOT_SATISFIABLE
        );
        assert_eq!(
            serve(&lease, &http::Method::GET, Some("bytes=0-1,4-5")).status(),
            http::StatusCode::RANGE_NOT_SATISFIABLE
        );
        assert_eq!(
            serve(&lease, &http::Method::POST, None).status(),
            http::StatusCode::METHOD_NOT_ALLOWED
        );

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"x").unwrap();
        let changed = serve(&lease, &http::Method::GET, None);
        assert_eq!(changed.status(), http::StatusCode::CONFLICT);
    }

    #[test]
    fn manager_rejects_tampered_or_unsupported_artifacts() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("recording.webm");
        fs::write(&path, b"changed").unwrap();
        let manager = RecordingMediaManager::default();
        let generation = manager.generation().unwrap();
        let artifact = ResolvedRecordingArtifact {
            path: path.clone(),
            suggested_file_name: "Clippy-session.webm".to_string(),
            byte_length: 7,
            sha256: format!("{:x}", sha2::Sha256::digest(b"planned")),
        };
        assert!(manager.issue(generation, "session-a", &artifact).is_err());

        let unsupported = ResolvedRecordingArtifact {
            path: temporary.path().join("recording.avi"),
            suggested_file_name: "Clippy-session.avi".to_string(),
            byte_length: 7,
            sha256: artifact.sha256.clone(),
        };
        fs::write(&unsupported.path, b"planned").unwrap();
        assert!(manager
            .issue(generation, "session-a", &unsupported)
            .is_err());
    }

    #[test]
    fn open_ended_requests_are_split_into_two_mib_chunks() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("recording.webm");
        fs::write(&path, vec![7; MAX_RESPONSE_BYTES as usize + 17]).unwrap();
        let lease = lease(path);
        let response = serve(&lease, &http::Method::GET, Some("bytes=0-"));
        assert_eq!(response.status(), http::StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.body().len(), MAX_RESPONSE_BYTES as usize);
        assert_eq!(
            response.headers()[http::header::CONTENT_RANGE],
            format!(
                "bytes 0-{}/{}",
                MAX_RESPONSE_BYTES - 1,
                MAX_RESPONSE_BYTES + 17
            )
        );
    }

    #[test]
    fn manager_evicts_old_leases_and_revokes_sessions() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("recording.webm");
        fs::write(&path, b"webm").unwrap();
        let artifact = ResolvedRecordingArtifact {
            path,
            suggested_file_name: "Clippy-session.webm".to_string(),
            byte_length: 4,
            sha256: format!("{:x}", sha2::Sha256::digest(b"webm")),
        };
        let manager = RecordingMediaManager::default();
        let generation = manager.generation().unwrap();
        let first = manager.issue(generation, "session-a", &artifact).unwrap();
        for index in 0..MAX_LEASES {
            manager
                .issue(generation, &format!("session-{index}"), &artifact)
                .unwrap();
        }
        assert!(manager.get(&first.token).unwrap().is_none());
        let current = manager.issue(generation, "session-a", &artifact).unwrap();
        manager.revoke_session("session-a").unwrap();
        assert!(manager.get(&current.token).unwrap().is_none());

        manager.clear().unwrap();
        assert!(manager.issue(generation, "session-a", &artifact).is_err());
    }
}
