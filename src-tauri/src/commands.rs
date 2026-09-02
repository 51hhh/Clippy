mod capture_entry;
mod clipboard;
mod ocr;
mod platform;
mod settings;
mod tmux;
mod url_metadata;

use crate::clipboard_watcher::ClipboardWatcher;
use crate::models::AppConfig;
use crate::paste::PasteManager;
use crate::storage::StorageEngine;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

// 截图入口只对 crate 内部可见（托盘 / 快捷键调用），没有 pub 项可以再导出。
pub(crate) use capture_entry::*;
pub use clipboard::*;
pub use ocr::*;
pub use platform::*;
pub use settings::*;
pub use tmux::*;
pub use url_metadata::*;

/// 全局应用状态，通过 Tauri 的 manage() 注入并在各命令中共享。
pub struct AppState {
    pub storage: Arc<Mutex<StorageEngine>>,
    pub config: Arc<Mutex<AppConfig>>,
    pub config_path: PathBuf,
    /// Tauri 按当前操作系统解析出的图片目录；配置留空时保存到其 `Clippy` 子目录。
    pub default_screenshot_dir: PathBuf,
    pub watcher: ClipboardWatcher,
    pub preview_visible: Arc<Mutex<bool>>,
    pub codec_visible: Arc<Mutex<bool>>,
    pub main_window_transition: Mutex<()>,
    pub pin_transition: Mutex<()>,
    pub main_window_position_generation: AtomicU64,
    pub capture_manager: Arc<crate::capture::CaptureManager>,
    pub pin_manager: Arc<crate::pin::PinManager>,
    /// 我们自己截下来、复制进剪贴板的图 → 它原本在屏幕上的位置。
    /// 之后从历史里 Pin 这张图时靠它贴回原处，见 `pin::PinOriginRegistry`。
    pub pin_origins: Arc<crate::pin::PinOriginRegistry>,
    pub paste_manager: Arc<PasteManager>,
    pub translation: Arc<crate::translation::TranslationService>,
    pub shortcuts_paused: AtomicBool,
    pub shortcut_transition: Mutex<()>,
    /// 非 GNOME Wayland 使用 XDG GlobalShortcuts Portal；其它会话不启动 worker。
    #[cfg(target_os = "linux")]
    pub portal_shortcuts: Option<Arc<crate::portal_shortcuts::PortalShortcutManager>>,
    /// 快捷键注册失败记录（按动作）。启动阶段的失败事件早于前端监听，
    /// 因此必须留一份可查询的状态，否则设置页永远看不到它。
    pub shortcut_failures: Mutex<Vec<crate::app::shortcuts::ShortcutRegisterFailure>>,
}

impl AppState {
    /// 截图/Pin 的保存位置来自运行时配置；配置锁损坏时退回内置默认目录，
    /// 保存动作不该因为别处的 panic 而失败。
    pub fn save_target(&self) -> crate::image_io::SaveTarget {
        match self.config.lock() {
            Ok(config) => {
                crate::image_io::SaveTarget::from_config(&config, &self.default_screenshot_dir)
            }
            Err(error) => {
                log::warn!("读取保存目录配置失败，使用默认目录: {error}");
                crate::image_io::SaveTarget {
                    directory: self.default_screenshot_dir.clone(),
                    template: crate::image_io::DEFAULT_FILENAME_TEMPLATE.to_string(),
                }
            }
        }
    }

    /// 托盘菜单与原生窗口标题用的文案；配置锁损坏时退回英文，
    /// 界面语言不该让开窗动作失败。
    pub fn native_text(&self) -> crate::i18n::NativeText {
        match self.config.lock() {
            Ok(config) => crate::i18n::text_for_language(&config.language),
            Err(error) => {
                log::warn!("读取界面语言配置失败，原生文案使用英文: {error}");
                crate::i18n::native_text(crate::i18n::Locale::En)
            }
        }
    }
}
