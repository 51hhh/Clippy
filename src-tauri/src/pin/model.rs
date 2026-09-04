use crate::models::ClipItem;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug)]
pub(super) enum PinSource {
    Clip {
        item: ClipItem,
        image: Option<Vec<u8>>,
    },
    Screenshot {
        png: Vec<u8>,
    },
    /// 从可编辑 PNG 恢复：屏幕/快速复制使用保存时合成图，画布与再次保存使用 canonical
    /// 原图和已验证工程。整个枚举位于 `Arc` 后，不会在缩放热路径复制这些字节。
    Project {
        source_png: Vec<u8>,
        preview_png: Vec<u8>,
        project: super::project::RuntimeProject,
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
    /// 压在普通窗口上面（工具条里的图钉）。**默认关**。
    ///
    /// 关着的时候贴图就是一个普普通通的窗口：谁最后拿到焦点谁在上面，别的窗口能盖住它，
    /// 全交给合成器。开着的时候进 Mutter 的 above 层，于是压在所有普通窗口之上；
    /// 同时开着的几张贴图之间仍然是"谁最后拿到焦点谁在上面"，因为它们在同一层里，
    /// 层内顺序照旧由合成器按栈序管——这一层的语义和普通窗口完全一致，见
    /// `super::window::keep_pin_above`。
    pub above: bool,
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
    /// 同一块屏上 GTK 报的**整数缓冲区缩放**。它与 `device_scale` 的差值就是
    /// WebKit 缓冲区里那一趟逃不掉的放大，`super::resample` 靠这两个数把它抵消掉。
    pub buffer_scale: f64,
    /// 后台补偿出来的清晰版图片。见 [`SharpenSlot`]。
    pub sharpen: Arc<SharpenSlot>,
}

/// 清晰版贴图的交接点：后台线程往里放，取 payload 的那一刻往外拿。
///
/// 补偿要几百毫秒（见 `super::resample`），而建窗 + WebKit 起步 + React 挂载本来也要
/// 几百毫秒。所以补偿在**建条目时就开跑**，和开窗并行；等前端来取 payload 时它多半
/// 已经算完了，第一帧就是清楚的。没赶上也不要紧：那时原图先上屏，算完之后走
/// `pin-image-sharpened` 事件换进去。这个类型存在的唯一理由就是把"谁先到"这件事
/// 收在一把锁里判断——不然会出现"payload 刚发走、清晰版刚算完、事件没人听"的空窗。
#[derive(Debug, Default)]
pub(super) struct SharpenSlot {
    inner: Mutex<SharpenState>,
}

#[derive(Debug, Default)]
struct SharpenState {
    image: Option<Arc<Vec<u8>>>,
    /// 原图（或清晰版）已经随 payload 发出去了。
    served: bool,
    /// 这个 slot 所属的窗口已关闭；同 label 后来的新窗口会有自己的 slot。
    cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SharpenFinish {
    /// 赶在 payload 之前，图已存入 slot。
    Stored,
    /// payload 已发，需要在再次核对未取消后发事件。
    NeedsEvent,
    /// 计算期间窗口已关闭，结果必须丢弃。
    Cancelled,
}

impl SharpenSlot {
    /// 取 payload 时叫一次：算好了就把清晰版交出来。
    ///
    /// 无论有没有算好都会记下"payload 已发出"，好让后台线程知道自己得改走事件。
    pub(super) fn take_for_payload(&self) -> Option<Arc<Vec<u8>>> {
        let mut state = self.lock();
        state.served = true;
        state.image.clone()
    }

    /// 后台线程算完时叫一次。取消与结果入槽在同一把锁下判定，不会在
    /// `cancel` 之后把旧图再塞回来。
    ///
    /// **没赶上首帧的那一份不留副本**：它会直接随事件走出去，槽里再存一份就是白占
    /// 十几 MB（2560x1440 的补偿结果 13 MB），而且一直占到窗口关闭。
    pub(super) fn finish(&self, image: &Arc<Vec<u8>>) -> SharpenFinish {
        let mut state = self.lock();
        if state.cancelled {
            return SharpenFinish::Cancelled;
        }
        if state.served {
            return SharpenFinish::NeedsEvent;
        }
        state.image = Some(Arc::clone(image));
        SharpenFinish::Stored
    }

    /// 前端已经把图画上屏了，槽里那份可以扔了。
    ///
    /// 补偿结果最大十几 MB，贴图窗口能开好几个，留着就是纯占内存——`copy_pin`/`save_pin`
    /// 用的是原图，缩放走 `update_pin`（不带图片），谁都不会再来取它。唯一会再取一次的
    /// 是 webview 重新加载（只有开发时手动刷新会发生），那时拿到的是原图，比清晰版略软。
    pub(super) fn release(&self) {
        self.lock().image = None;
    }

    /// 窗口已关闭：让还在全局工作位后面排队的补偿任务直接退出。
    pub(super) fn cancel(&self) {
        let mut state = self.lock();
        state.cancelled = true;
        state.image = None;
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    /// 把“未取消”检查与事件发布放在同一个线性化区间。
    ///
    /// `cancel` 要取同一把锁：若它先完成，`publish_if_active` 不会执行；若发布
    /// 先开始，`cancel` 会等它结束后才返回。因此关窗返回后绝不会再有旧事件命中
    /// 同 label 的新窗口。
    pub(super) fn publish_if_active<T>(&self, publish: impl FnOnce() -> T) -> Option<T> {
        let state = self.lock();
        if state.cancelled {
            return None;
        }
        Some(publish())
    }

    /// 临界区里只有两次赋值，不会 panic，所以中毒的锁直接接着用。
    fn lock(&self) -> MutexGuard<'_, SharpenState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod sharpen_tests {
    use super::{SharpenFinish, SharpenSlot};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// 补偿赶在 payload 之前算完：清晰版直接随 payload 走，不需要事件。
    #[test]
    fn early_compensation_rides_along_with_the_payload() {
        let slot = SharpenSlot::default();
        assert_eq!(
            slot.finish(&Arc::new(vec![7, 8, 9])),
            SharpenFinish::Stored,
            "还没发 payload，应直接存入 slot"
        );
        assert_eq!(
            slot.take_for_payload().map(|image| image.to_vec()),
            Some(vec![7, 8, 9])
        );
        // 开发时刷新 webview 会再取一次；这条路必须还能拿到东西，不能变成空图。
        assert_eq!(
            slot.take_for_payload().map(|image| image.to_vec()),
            Some(vec![7, 8, 9])
        );
    }

    /// 没赶上：payload 已经带着原图走了，这时必须补一个事件。
    #[test]
    fn late_compensation_asks_for_an_event() {
        let slot = SharpenSlot::default();
        assert!(slot.take_for_payload().is_none(), "还没算完，只能发原图");
        let bytes = Arc::new(vec![1]);
        assert_eq!(
            slot.finish(&bytes),
            SharpenFinish::NeedsEvent,
            "原图已上屏，必须发事件换图"
        );
        // 事件已经把这份字节送出去了，槽里不该再留一份十几 MB 的副本。
        assert_eq!(Arc::strong_count(&bytes), 1);
    }

    /// 上屏之后释放：十几 MB 的补偿结果不该跟着贴图窗口一直活着。
    #[test]
    fn release_drops_the_compensated_bytes() {
        let slot = SharpenSlot::default();
        let bytes = Arc::new(vec![4, 5]);
        assert_eq!(slot.finish(&bytes), SharpenFinish::Stored);
        assert!(slot.take_for_payload().is_some());
        assert_eq!(Arc::strong_count(&bytes), 2, "槽里还留着一份");
        slot.release();
        assert_eq!(Arc::strong_count(&bytes), 1, "release 之后只剩调用方那份");
        assert!(slot.take_for_payload().is_none());
    }

    /// 文本贴图那种"永远不会算完"的情况：取 payload 不该被卡住，也不该报错。
    #[test]
    fn a_slot_that_never_finishes_is_harmless() {
        let slot = SharpenSlot::default();
        assert!(slot.take_for_payload().is_none());
        assert!(slot.take_for_payload().is_none());
    }

    #[test]
    fn closed_pin_cancels_queued_compensation() {
        let slot = SharpenSlot::default();
        assert!(!slot.is_cancelled());
        slot.cancel();
        assert!(slot.is_cancelled());
        assert!(slot.take_for_payload().is_none());
    }

    /// worker 已经开始计算才关窗：完成结果不能再入槽，也不能请求发事件。
    #[test]
    fn cancellation_after_work_started_discards_the_result() {
        let slot = SharpenSlot::default();
        assert!(
            slot.take_for_payload().is_none(),
            "模拟 worker 开始后原图先上屏"
        );
        slot.cancel();
        let bytes = Arc::new(vec![9, 9, 9]);
        assert_eq!(slot.finish(&bytes), SharpenFinish::Cancelled);
        assert_eq!(Arc::strong_count(&bytes), 1, "取消后不得留存结果");
    }

    /// 旧 slot 的 label 即使被新窗口复用，旧 worker 也不能发布任何事件。
    #[test]
    fn reused_label_never_receives_an_event_from_the_cancelled_old_slot() {
        let old = SharpenSlot::default();
        let replacement = SharpenSlot::default();
        let published = AtomicUsize::new(0);

        old.take_for_payload();
        old.cancel();
        assert_eq!(old.finish(&Arc::new(vec![1])), SharpenFinish::Cancelled);
        assert!(old
            .publish_if_active(|| published.fetch_add(1, Ordering::SeqCst))
            .is_none());

        assert!(replacement
            .publish_if_active(|| published.fetch_add(1, Ordering::SeqCst))
            .is_some());
        assert_eq!(published.load(Ordering::SeqCst), 1, "只有新 slot 可以发布");
    }
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
    /// 见 `PinEntry::above`。默认 false，工具条据此画图钉的按下态。
    pub above: bool,
    pub can_save: bool,
    pub position: Option<PinPosition>,
    /// 见 `PinEntry::device_scale`。前端拿它判断"屏上一个图片像素是不是正好一个设备
    /// 像素"，只有相等时才让 WebKit 用最近邻搬图（`src/react/pin/rendering.ts`）。
    pub device_scale: f64,
    /// 见 `PinEntry::buffer_scale`。图片已按缓冲区分辨率补偿过时，
    /// `pixelWidth == cssWidth * bufferScale`，前端据此知道这张图是 1:1 搬进缓冲区的。
    pub buffer_scale: f64,
    /// 只有从合法 v2/v3 工程打开的贴图才有。原图字节由 `get_pin_source_image` 按需提供。
    pub initial_project: Option<super::project::InitialProject>,
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
    pub above: bool,
    pub position: Option<PinPosition>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinUpdate {
    pub scale: Option<f64>,
    pub opacity: Option<f64>,
    pub locked: Option<bool>,
    pub above: Option<bool>,
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

/// 窗口标题的固定前缀。窗口枚举那侧靠它认出"这是一张贴图"。
const WINDOW_MARKER_PREFIX: &str = "Clippy Pin ";

/// 这个窗口标题是一张贴图吗？是的话给出它的 label。
///
/// 截图的窗口速选要用它：Clippy 自己的窗口一律不该成为速选目标（悬浮面板、覆盖层），
/// **但贴图是例外**——它在用户眼里就是屏幕上的一块内容，和别的窗口一样该能被框选。
/// 所以那边的过滤条件不能停留在"pid 是自己就跳过"。
pub(crate) fn label_from_window_marker(title: &str) -> Option<&str> {
    let label = title.strip_prefix(WINDOW_MARKER_PREFIX)?;
    is_safe_pin_label(label).then_some(label)
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
