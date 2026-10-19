use super::model::is_viewer_label;
use crate::commands::AppState;
use tauri::{http, Manager, Runtime, UriSchemeContext};

pub(super) fn parse_path<'a>(
    caller: &str,
    path: &'a str,
    query: Option<&str>,
) -> Option<(&'a str, &'a str)> {
    if query.is_some() {
        return None;
    }
    let mut parts = path.strip_prefix('/')?.split('/');
    let label = parts.next()?;
    let snapshot = parts.next()?;
    if parts.next().is_some()
        || label != caller
        || !is_viewer_label(label)
        || !snapshot.starts_with("snapshot-")
        || snapshot.len() > 96
        || !snapshot
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return None;
    }
    Some((label, snapshot))
}

pub(crate) fn handle<R: Runtime>(
    context: UriSchemeContext<'_, R>,
    request: http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    let Some((label, snapshot)) = parse_path(
        context.webview_label(),
        request.uri().path(),
        request.uri().query(),
    ) else {
        return response(http::StatusCode::FORBIDDEN, b"forbidden".to_vec(), false);
    };
    let state = context.app_handle().state::<AppState>();
    let Ok(entry) = state.viewer_manager.get(label) else {
        return response(http::StatusCode::NOT_FOUND, vec![], false);
    };
    if entry.payload.handle.snapshot_id != snapshot || !entry.is_active() {
        return response(http::StatusCode::NOT_FOUND, vec![], false);
    }
    response(
        http::StatusCode::OK,
        entry.source_png.as_ref().clone(),
        true,
    )
}
fn response(status: http::StatusCode, body: Vec<u8>, image: bool) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(status)
        .header(
            http::header::CONTENT_TYPE,
            if image { "image/png" } else { "text/plain" },
        )
        .header(http::header::CACHE_CONTROL, "no-store")
        .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(body)
        .expect("静态查看器响应头")
}
