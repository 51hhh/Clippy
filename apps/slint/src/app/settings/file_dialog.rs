use super::{send, App, AppWindow, FileChoice, Message};
use anyhow::{ensure, Context, Result};
use clippy_core::image_output::expand_user_path;
use clippy_platform::file_dialog::{select, Mode, Request};
use slint::{winit_030::WinitWindowAccessor, ComponentHandle, Weak};
use std::path::{Path, PathBuf};

const ARCHIVE_SUFFIX: &str = ".clippy.zip";

pub(super) fn start(app: &mut App, kind: FileChoice, path: &str) -> Result<()> {
    ensure!(
        !app.file_dialog_pending,
        "Close the current file chooser first"
    );
    let window = app.window.as_ref().expect("设置窗口已打开");
    let requester = window.as_weak();
    let path = expand_user_path(path.trim());
    let request = match kind {
        FileChoice::ScreenshotDirectory => Request {
            mode: Mode::Directory,
            title: "Choose screenshot directory",
            directory: Some(if path.is_absolute() {
                path
            } else {
                app.paths.screenshot_default(app.identity)
            }),
        },
        FileChoice::OcrManifest => Request {
            mode: Mode::Open {
                label: "JSON files",
                extension: "json",
            },
            title: "Choose OCR manifest",
            directory: path
                .parent()
                .filter(|path| path.is_absolute())
                .map(Path::to_owned),
        },
        FileChoice::ExportArchive => Request {
            mode: Mode::Save {
                suggested_name: "Clippy-backup.clippy.zip",
            },
            title: "Export Clippy archive",
            directory: None,
        },
        FileChoice::ImportArchive => Request {
            mode: Mode::Open {
                label: "Clippy archives",
                extension: "zip",
            },
            title: "Import Clippy archive",
            directory: None,
        },
    };
    slint::spawn_local(async move {
        let result = choose(&requester, request)
            .await
            .context("Could not open the file chooser");
        send(Message::FileChosen(kind, requester, result));
    })?;
    app.file_dialog_pending = true;
    window.set_file_dialog_pending(true);
    Ok(())
}

async fn choose(requester: &Weak<AppWindow>, request: Request) -> Result<Option<PathBuf>> {
    let window = requester.upgrade().context("Settings closed")?;
    let native = window.window().winit_window().await?;
    drop(window);
    #[cfg(target_os = "linux")]
    {
        let (send, receive) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("clippy-file-dialog".into())
            .spawn(move || {
                let _ = send.send(select(request, native));
            })?;
        receive.await?
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    select(request, native.as_ref()).await
}

pub(super) fn completed(
    app: &mut App,
    kind: FileChoice,
    requester: Weak<AppWindow>,
    result: Result<Option<PathBuf>>,
) {
    app.file_dialog_pending = false;
    let Some(window) = app.window.as_ref() else {
        return;
    };
    window.set_file_dialog_pending(false);
    if !requester
        .upgrade()
        .is_some_and(|owner| std::ptr::eq(owner.window(), window.window()))
    {
        return;
    }
    let result = result.and_then(|path| path.map(|path| selected_path(kind, path)).transpose());
    let result = match result {
        Ok(Some(path)) => match kind {
            FileChoice::ScreenshotDirectory => {
                window.set_save_directory_draft(path.into());
                Ok(())
            }
            FileChoice::OcrManifest => {
                window.set_ocr_manifest_draft(path.into());
                Ok(())
            }
            FileChoice::ExportArchive => app.data_action(1, &path),
            FileChoice::ImportArchive => app.data_action(2, &path),
        },
        Ok(None) => Ok(()),
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        app.status(format!("{error:#}"));
    }
}

fn selected_path(kind: FileChoice, mut path: PathBuf) -> Result<String> {
    ensure!(path.is_absolute(), "Choose an absolute local path");
    if kind == FileChoice::ExportArchive {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("Choose an archive filename")?;
        if !name.ends_with(ARCHIVE_SUFFIX) {
            let stem = name.strip_suffix(".zip").unwrap_or(name);
            path.set_file_name(format!("{stem}{ARCHIVE_SUFFIX}"));
        }
    }
    let value = path
        .to_str()
        .context("The selected path is not valid UTF-8")?;
    match kind {
        FileChoice::ScreenshotDirectory => ensure!(
            value == value.trim_end(),
            "Choose a directory without trailing whitespace"
        ),
        FileChoice::OcrManifest => ensure!(
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json")),
            "Choose a JSON manifest"
        ),
        FileChoice::ImportArchive => ensure!(
            value.ends_with(ARCHIVE_SUFFIX),
            "Choose a .clippy.zip archive"
        ),
        FileChoice::ExportArchive => {}
    }
    Ok(value.to_owned())
}
