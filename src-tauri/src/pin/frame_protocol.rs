use crate::commands::AppState;
use tauri::{http, Manager, Runtime, UriSchemeContext};

/// 只允许贴图 WebView 读取与自己 label 对应的显示图，避免其它窗口枚举剪贴板内容。
pub(crate) fn handle<R: Runtime>(
    context: UriSchemeContext<'_, R>,
    request: http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    let label = request.uri().path().trim_start_matches('/');
    if label != context.webview_label() || super::model::validate_label(label).is_err() {
        return response(
            http::StatusCode::FORBIDDEN,
            "text/plain",
            b"forbidden".to_vec(),
        );
    }

    let revision = request_revision(request.uri().query());
    let state = context.app_handle().state::<AppState>();
    let entry = match state.pin_manager.get(label) {
        Ok(entry) => entry,
        Err(error) => {
            return response(
                http::StatusCode::NOT_FOUND,
                "text/plain",
                error.to_string().into_bytes(),
            );
        }
    };

    let bytes = if revision == 0 {
        entry
            .sharpen
            .take_for_initial_request()
            .or_else(|| super::commands::display_png(&entry.source).map(<[u8]>::to_vec))
    } else {
        entry.sharpen.take_for_update_request()
    };
    match bytes {
        Some(bytes) => response(http::StatusCode::OK, "image/png", bytes),
        None => response(
            http::StatusCode::NOT_FOUND,
            "text/plain",
            b"pin image unavailable".to_vec(),
        ),
    }
}

fn request_revision(query: Option<&str>) -> u8 {
    query
        .and_then(|query| {
            query.split('&').find_map(|pair| {
                let (key, value) = pair.split_once('=')?;
                (key == "revision")
                    .then(|| value.parse::<u8>().ok())
                    .flatten()
            })
        })
        .unwrap_or(0)
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
        .body(body)
        .expect("静态贴图协议响应头必须有效")
}

#[cfg(test)]
mod tests {
    use super::request_revision;

    #[test]
    fn revision_defaults_to_initial_image() {
        assert_eq!(request_revision(None), 0);
        assert_eq!(request_revision(Some("unused=1")), 0);
        assert_eq!(request_revision(Some("revision=bad")), 0);
    }

    #[test]
    fn revision_is_read_from_query_without_order_dependency() {
        assert_eq!(request_revision(Some("cache=7&revision=1")), 1);
        assert_eq!(request_revision(Some("revision=2&cache=7")), 2);
    }
}
