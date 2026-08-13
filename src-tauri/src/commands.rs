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
    pub main_window_position_generation: AtomicU64,
    pub capture_manager: Arc<crate::capture::CaptureManager>,
    pub pin_manager: Arc<crate::pin::PinManager>,
    pub paste_manager: Arc<PasteManager>,
    pub translation: Arc<crate::translation::TranslationService>,
    pub shortcuts_paused: AtomicBool,
    pub shortcut_transition: Mutex<()>,
}
