use anyhow::{anyhow, ensure, Result};
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
            Ok(screenshot) => file_path(screenshot.uri().as_str()).map(Some),
            Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(None),
            Err(ashpd::Error::Response(ResponseError::Other)) => {
                Err(anyhow!("The system screenshot did not return an image"))
            }
            Err(error) => Err(error.into()),
        }
    })
}

fn file_path(uri: &str) -> Result<PathBuf> {
    let url = url::Url::parse(uri)?;
    ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "The system screenshot link includes unexpected parameters"
    );
    url.to_file_path()
        .map_err(|()| anyhow!("The system screenshot did not return a local file"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_local_screenshot_files() {
        assert_eq!(
            file_path("file:///tmp/Screenshot%201.png").unwrap(),
            PathBuf::from("/tmp/Screenshot 1.png")
        );
        assert!(file_path("file://remote/tmp/screenshot.png").is_err());
        assert!(file_path("https://example.invalid/screenshot.png").is_err());
        assert!(file_path("file:///tmp/screenshot.png?mode=remote").is_err());
    }
}
