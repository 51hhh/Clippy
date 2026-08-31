use crate::models::ClipItem;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug)]
pub(super) enum PinSource {
    Clip {
        item: ClipItem,
        image: Option<Vec<u8>>,
    },
    Screenshot {
        png: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
pub(super) struct PinEntry {
    pub label: String,
    /// 内容放在 `Arc` 后面，**克隆 `PinEntry` 才不会连着整张 PNG 一起复制**。
    ///
    /// `update_pin` 是滚轮缩放的每帧热路径，它要克隆两份条目（回滚用的 `previous`
    /// 加上更新后的那份）。一张全屏截图两三 MB，按值放在这里等于每帧白 memcpy 四五 MB，
    /// 而且是在主线程上。内容从建窗到关窗都不会变，共享它没有任何取舍。
    pub source: Arc<PinSource>,
    pub content_width: f64,
    pub content_height: f64,
    pub scale: f64,
    pub opacity: f64,
    pub locked: bool,
    pub position: Option<PinPosition>,
    /// 这张图原本在屏幕上的位置与大小（逻辑像素）。截图选区带着它过来，
    /// 于是贴图能贴回原处、原尺寸；从别处来的图片没有它，落回光标/居中。
    pub origin: Option<PinOrigin>,
    /// 内容所在那块屏上，一个逻辑像素等于几个设备像素（真实缩放，不是 GTK 报的
    /// 整数缓冲区缩放）。内容尺寸就是按它把图片像素折算成 CSS 像素的。
    ///
    /// 建窗时定下来就不再变：payload 是前端起来之后才取的，那时候光标可能已经挪到
    /// 另一块缩放不同的屏上，现场再查会得出和内容尺寸不配套的比例。
    pub device_scale: f64,
}

/// 图片在屏幕上的来源矩形，逻辑像素、桌面全局坐标（与截图覆盖层同一坐标系）。
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinOrigin {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PinOrigin {
    /// 只接受有限、且大到看得见的矩形。选区来自前端，NaN 或 0 尺寸会一路污染窗口几何。
    pub(crate) fn sanitized(self) -> Option<Self> {
        let finite = [self.x, self.y, self.width, self.height]
            .into_iter()
            .all(f64::is_finite);
        (finite && self.width >= 2.0 && self.height >= 2.0).then_some(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PinPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinPayload {
    pub label: String,
    pub kind: &'static str,
    pub text: Option<String>,
    pub image_base64: Option<String>,
    pub content_width: f64,
    pub content_height: f64,
    pub scale: f64,
    pub opacity: f64,
    pub locked: bool,
    pub can_save: bool,
    pub position: Option<PinPosition>,
    /// 见 `PinEntry::device_scale`。前端拿它判断"屏上一个图片像素是不是正好一个设备
    /// 像素"，只有相等时才让 WebKit 用最近邻搬图（`src/react/pin/rendering.ts`）。
    pub device_scale: f64,
}

/// `update_pin` 的应答：只有这次真的可能变的那几个字段。
///
/// **故意不带 `image_base64` 与 `text`。** 滚轮缩放时每一帧都会调一次 `update_pin`，
/// 而贴图的内容从头到尾没变过；带上图片意味着每帧都要把 PNG 重新 base64 编一遍
/// （一张全屏截图 2 MB → 2.8 MB 字符串）再过一次 IPC，纯粹的浪费。前端拿到这个应答后
/// 合并进已有的 payload（`App.tsx` 的 `mergePinState`），图片对象 URL 因此也不会被重建。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinState {
    pub label: String,
    pub content_width: f64,
    pub content_height: f64,
    pub scale: f64,
    pub opacity: f64,
    pub locked: bool,
    pub position: Option<PinPosition>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinUpdate {
    pub scale: Option<f64>,
    pub opacity: Option<f64>,
    pub locked: Option<bool>,
}

/// 贴图窗口的原生标题。
///
/// 窗口无装饰、不进任务栏，这个标题不出现在任何界面上——它唯一的用途是让
/// GNOME Shell 扩展能在 Shell 进程里认出这个窗口。Wayland 下客户端既摆不了自己的
/// 位置也置不了顶，只有扩展做得到，而扩展只能按标题 + pid 查找（见
/// `capture::shell_extension_place_window`）。所以标题必须唯一且稳定，跟着 label 走。
pub(crate) fn window_marker(label: &str) -> String {
    format!("Clippy Pin {label}")
}

pub(super) fn validate_label(label: &str) -> Result<(), String> {
    if is_safe_pin_label(label) {
        Ok(())
    } else {
        Err("无效的贴图窗口标签".to_string())
    }
}

pub(super) fn is_safe_pin_label(label: &str) -> bool {
    label.starts_with("pin-")
        && label.len() <= 96
        && label
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}
