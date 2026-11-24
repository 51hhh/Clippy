slint::include_modules!();

use clippy_core::models::{ClipItem, ContentType};
use clippy_platform::identity::DeploymentIdentity;
use clippy_platform::paths::{AppPaths, PathError};
use clippy_platform::private_files::restrict_directory;
use clippy_storage::{ClipListRequest, StorageDispatchError, StorageError, StorageRuntime};
use slint::{ModelRc, SharedString, VecModel};
use std::fmt;
use std::fs;

pub const UI_BACKEND: &str = "Slint";
pub const FRAMEWORK_VERSION: &str = "1.19.0";

pub fn run(identity: &'static DeploymentIdentity) -> Result<(), AppError> {
    let paths = AppPaths::resolve(identity)?;
    fs::create_dir_all(&paths.data_dir)?;
    restrict_directory(&paths.data_dir)?;
    let storage = StorageRuntime::open(&paths.database_file)?;

    let window = AppWindow::new()?;
    window.set_product_name(identity.product_name.into());
    window.set_build_label(build_label(identity).into());
    load_initial_history(&window, &storage)?;
    window.run()?;
    Ok(())
}

fn load_initial_history(window: &AppWindow, storage: &StorageRuntime) -> Result<(), AppError> {
    let weak = window.as_weak();
    storage.client().list_clips(
        ClipListRequest::new(1, None, false, 0, 50)?,
        move |_generation, result| {
            let _ = weak.upgrade_in_event_loop(move |window| match result {
                Ok(clips) => {
                    let count = clips.len();
                    let rows = clips.into_iter().map(clip_row).collect::<Vec<_>>();
                    window.set_clips(ModelRc::new(VecModel::from(rows)));
                    window.set_status_text(if count == 0 {
                        "Clipboard history is empty".into()
                    } else {
                        format!("{count} recent items").into()
                    });
                }
                Err(error) => {
                    window.set_status_text(format!("Unable to load history: {error}").into());
                }
            });
        },
    )?;
    Ok(())
}

fn clip_row(clip: ClipItem) -> ClipRow {
    let kind = match &clip.content_type {
        ContentType::Text => "Text",
        ContentType::Html => "Rich text",
        ContentType::Image => "Image",
    };
    let preview = if clip.is_sensitive {
        "Sensitive content hidden".to_string()
    } else {
        clip.text_content
            .as_deref()
            .map(|text| truncate_preview(text, 160))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| match &clip.content_type {
                ContentType::Image => "Image content".to_string(),
                ContentType::Html => "Rich text content".to_string(),
                ContentType::Text => "Empty text".to_string(),
            })
    };
    ClipRow {
        kind: SharedString::from(kind),
        preview: preview.into(),
        metadata: format!("{} bytes", clip.byte_size).into(),
        favorite: clip.is_favorite,
        sensitive: clip.is_sensitive,
    }
}

fn truncate_preview(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let preview = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

pub fn build_label(identity: &DeploymentIdentity) -> String {
    format!(
        "v{} · {UI_BACKEND} {FRAMEWORK_VERSION} · {}",
        env!("CARGO_PKG_VERSION"),
        identity.channel.as_str()
    )
}

#[derive(Debug)]
pub enum AppError {
    Platform(slint::PlatformError),
    Paths(PathError),
    Io(std::io::Error),
    Storage(StorageError),
    Dispatch(StorageDispatchError),
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Platform(error) => write!(formatter, "UI 初始化失败: {error}"),
            Self::Paths(error) => write!(formatter, "数据路径初始化失败: {error}"),
            Self::Io(error) => write!(formatter, "本地目录初始化失败: {error}"),
            Self::Storage(error) => write!(formatter, "存储初始化失败: {error}"),
            Self::Dispatch(error) => write!(formatter, "存储请求提交失败: {error}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<slint::PlatformError> for AppError {
    fn from(error: slint::PlatformError) -> Self {
        Self::Platform(error)
    }
}

impl From<PathError> for AppError {
    fn from(error: PathError) -> Self {
        Self::Paths(error)
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<StorageError> for AppError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<StorageDispatchError> for AppError {
    fn from(error: StorageDispatchError) -> Self {
        Self::Dispatch(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clippy_platform::identity::{PRODUCTION_IDENTITY, SLINT_QA_IDENTITY};

    #[test]
    fn framework_identity_is_explicit() {
        assert_eq!(env!("CARGO_PKG_VERSION"), "0.2.0");
        assert_eq!(UI_BACKEND, "Slint");
        assert_eq!(FRAMEWORK_VERSION, "1.19.0");
        assert_eq!(clippy_core::PRODUCT_NAME, "Clippy");
    }

    #[test]
    fn build_label_exposes_the_selected_distribution_channel() {
        assert!(build_label(&PRODUCTION_IDENTITY).ends_with("production"));
        assert!(build_label(&SLINT_QA_IDENTITY).ends_with("qa"));
    }

    #[test]
    fn sensitive_rows_never_copy_their_text_into_the_ui_model() {
        let row = clip_row(ClipItem {
            id: 1,
            content_type: ContentType::Text,
            text_content: Some("secret-token".into()),
            html_content: None,
            image_data: None,
            content_hash: "hash".into(),
            is_favorite: false,
            is_sensitive: true,
            created_at: 0,
            byte_size: 12,
        });
        assert_eq!(row.preview.as_str(), "Sensitive content hidden");
        assert!(row.sensitive);
    }

    #[test]
    fn preview_truncation_respects_unicode_boundaries() {
        let preview = truncate_preview(&format!("{}尾", "界".repeat(160)), 160);
        assert_eq!(preview.chars().count(), 161);
        assert!(preview.ends_with('…'));
    }
}
