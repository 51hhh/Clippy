use anyhow::Result;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::path::PathBuf;

pub enum Mode {
    Directory,
    Open {
        label: &'static str,
        extension: &'static str,
    },
    Save {
        suggested_name: &'static str,
    },
}

pub struct Request {
    pub mode: Mode,
    pub title: &'static str,
    pub directory: Option<PathBuf>,
}

#[cfg(target_os = "linux")]
pub fn select(
    request: Request,
    parent: impl HasWindowHandle + HasDisplayHandle,
) -> Result<Option<PathBuf>> {
    use anyhow::{ensure, Context};
    use ashpd::{
        desktop::{
            file_chooser::{FileChooserProxy, FileFilter, SelectedFiles},
            ResponseError,
        },
        WindowIdentifier,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        // 独立连接随这次交互退出，后续选择不共享已停止的运行时。
        let connection = zbus::Connection::session().await?;
        if matches!(request.mode, Mode::Directory) {
            let chooser = FileChooserProxy::with_connection(connection.clone()).await?;
            ensure!(chooser.version() >= 3, "This desktop cannot choose folders");
        }
        let identifier = WindowIdentifier::from_raw_handle(
            &parent.window_handle()?.as_raw(),
            Some(&parent.display_handle()?.as_raw()),
        )
        .await;
        drop(parent);
        let result = match request.mode {
            Mode::Save { suggested_name } => {
                SelectedFiles::save_file()
                    .identifier(identifier)
                    .title(request.title)
                    .connection(Some(connection))
                    .current_name(suggested_name)
                    .current_folder::<PathBuf>(request.directory)?
                    .send()
                    .await
            }
            mode => {
                let picker = SelectedFiles::open_file()
                    .identifier(identifier)
                    .title(request.title)
                    .connection(Some(connection))
                    .current_folder::<PathBuf>(request.directory)?;
                let picker = match mode {
                    Mode::Directory => picker.directory(true),
                    Mode::Open { label, extension } => {
                        picker.filter(FileFilter::new(label).glob(&format!("*.{extension}")))
                    }
                    Mode::Save { .. } => unreachable!(),
                };
                picker.send().await
            }
        };
        match result.and_then(|request| request.response()) {
            Ok(files) => {
                let [uri] = files.uris() else {
                    anyhow::bail!("The file chooser did not return one path");
                };
                crate::paths::local_file_uri(uri.as_str())
                    .context("The file chooser did not return a local path")
                    .map(Some)
            }
            Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(None),
            Err(error) => Err(error.into()),
        }
    })
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub async fn select(
    request: Request,
    parent: &(impl HasWindowHandle + HasDisplayHandle),
) -> Result<Option<PathBuf>> {
    let mut picker = rfd::AsyncFileDialog::new()
        .set_parent(parent)
        .set_title(request.title);
    if let Some(directory) = request.directory {
        picker = picker.set_directory(directory);
    }
    let file = match request.mode {
        Mode::Directory => picker.pick_folder().await,
        Mode::Open { label, extension } => picker.add_filter(label, &[extension]).pick_file().await,
        Mode::Save { suggested_name } => picker.set_file_name(suggested_name).save_file().await,
    };
    Ok(file.map(|file| file.path().to_path_buf()))
}
