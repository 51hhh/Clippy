mod capture_editor;
mod clipboard;
mod ocr;
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

pub use capture_editor::*;
pub use clipboard::*;
pub use ocr::*;
pub use settings::*;
pub use tmux::*;
pub use url_metadata::*;

/// 全局应用状态，通过 Tauri 的 manage() 注入并在各命令中共享。
pub struct AppState {
    pub storage: Arc<Mutex<StorageEngine>>,
    pub config: Arc<Mutex<AppConfig>>,
    pub config_path: PathBuf,
    pub watcher: ClipboardWatcher,
    pub preview_visible: Arc<Mutex<bool>>,
    pub codec_visible: Arc<Mutex<bool>>,
    pub latest_capture: Arc<Mutex<Option<crate::screenshot::CapturedScreenshot>>>,
    pub capture_generation: AtomicU64,
    pub capture_window_generation: AtomicU64,
    pub capture_editor_transition: Mutex<()>,
    pub main_window_transition: Mutex<()>,
    pub pin_transition: Mutex<()>,
    pub main_window_position_generation: AtomicU64,
    pub capture_manager: Arc<crate::capture::CaptureManager>,
    pub pin_manager: Arc<crate::pin::PinManager>,
    pub paste_manager: Arc<PasteManager>,
    pub translation: Arc<crate::translation::TranslationService>,
    pub shortcuts_paused: AtomicBool,
    pub shortcut_transition: Mutex<()>,
    /// 快捷键注册失败记录（按动作）。启动阶段的失败事件早于前端监听，
    /// 因此必须留一份可查询的状态，否则设置页永远看不到它。
    pub shortcut_failures: Mutex<Vec<crate::app::shortcuts::ShortcutRegisterFailure>>,
}

impl AppState {
    /// 截图/Pin 的保存位置来自运行时配置；配置锁损坏时退回内置默认目录，
    /// 保存动作不该因为别处的 panic 而失败。
    pub fn save_target(&self) -> crate::image_io::SaveTarget {
        match self.config.lock() {
            Ok(config) => crate::image_io::SaveTarget::from_config(&config),
            Err(error) => {
                log::warn!("读取保存目录配置失败，使用默认目录: {error}");
                crate::image_io::SaveTarget::default()
            }
        }
    }

    /// 框选完成后的默认动作；配置锁损坏时退回「直接开编辑器」，
    /// 截图流程不该因为别处的 panic 停在覆盖层上。
    pub fn capture_commit_action(&self) -> &'static str {
        match self.config.lock() {
            Ok(config) => config.capture_commit_action(),
            Err(error) => {
                log::warn!("读取选区提交动作配置失败，使用默认动作: {error}");
                crate::models::CAPTURE_COMMIT_ACTION_EDITOR
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
