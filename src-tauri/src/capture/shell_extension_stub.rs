//! 非 Linux 平台的 GNOME Shell 扩展占位实现。
//!
//! 它只保持跨平台 IPC 和截图领域接口稳定，不伪装扩展可用。Windows/macOS 的窗口枚举
//! 与截图由各自原生后端负责。

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellExtensionStatus {
    pub supported: bool,
    pub installed: bool,
    pub enabled: bool,
    pub active: bool,
    pub stale: bool,
    pub user_extensions_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    pub needs_logout: bool,
    pub status: ShellExtensionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShellWindow {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub title: String,
    pub wm_class: String,
    pub pid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CaptureArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AreaCapture {
    Raw {
        path: PathBuf,
        width: u32,
        height: u32,
        stride: usize,
    },
    Png {
        path: PathBuf,
    },
}

impl AreaCapture {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Raw { path, .. } | Self::Png { path } => path,
        }
    }
}

fn unsupported() -> String {
    "GNOME Shell extension is only available on Linux".to_string()
}

pub(crate) fn probe() -> Option<Vec<ShellWindow>> {
    None
}

pub(crate) fn request_screenshot() -> Result<PathBuf, String> {
    Err(unsupported())
}

pub(crate) fn request_area_captures(_areas: &[CaptureArea]) -> Result<Vec<AreaCapture>, String> {
    Err(unsupported())
}

pub(crate) fn place_window(
    _marker: &str,
    _position: Option<(i32, i32)>,
    _above: bool,
) -> Result<bool, String> {
    Err(unsupported())
}

pub(super) fn hint_needed() -> bool {
    false
}

pub fn status() -> ShellExtensionStatus {
    ShellExtensionStatus::default()
}

pub fn install() -> Result<InstallOutcome, String> {
    Err(unsupported())
}

pub fn uninstall() -> Result<ShellExtensionStatus, String> {
    Ok(ShellExtensionStatus::default())
}

pub fn reconcile_on_startup() {}
