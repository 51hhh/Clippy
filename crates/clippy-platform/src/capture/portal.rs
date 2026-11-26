use crate::paths::local_file_uri;
use anyhow::{anyhow, Context, Result};
use ashpd::desktop::{screenshot::Screenshot, ResponseError};
use std::path::PathBuf;

pub fn system_screenshot() -> Result<Option<PathBuf>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        // 每次操作使用本 worker 的连接，避免复用上一个已退出的 Tokio runtime。
        let connection = zbus::Connection::session().await?;
        let request = Screenshot::request()
            .interactive(true)
            .modal(false)
            .connection(Some(connection));
        match request.send().await.and_then(|request| request.response()) {
            Ok(screenshot) => local_file_uri(screenshot.uri().as_str())
                .context("The system screenshot did not return a local file")
                .map(Some),
            Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(None),
            Err(ashpd::Error::Response(ResponseError::Other)) => {
                Err(anyhow!("The system screenshot did not return an image"))
            }
            Err(error) => Err(error.into()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_local_screenshot_files() {
        assert_eq!(
            local_file_uri("file:///tmp/Screenshot%201.png").unwrap(),
            PathBuf::from("/tmp/Screenshot 1.png")
        );
        assert!(local_file_uri("file://remote/tmp/screenshot.png").is_err());
        assert!(local_file_uri("https://example.invalid/screenshot.png").is_err());
        assert!(local_file_uri("file:///tmp/screenshot.png?mode=remote").is_err());
    }
}
