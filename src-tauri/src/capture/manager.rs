use super::error::CaptureError;
use super::frame_crop::selection_pixel_rect;
use super::mode_gate::CaptureModeOwnership;
use super::types::{CaptureOverlayPayload, CaptureSelection, OverlaySpec, WindowCandidate};
use super::window_probe::probe_windows;
use crate::screenshot::CapturedMonitorFrame;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// 一次截图从按下快捷键到覆盖层显示的分段耗时。
///
/// "截图要三五秒"这类报障只能靠分段定位：链路每一环都成功，问题在于加起来太久，
/// 而各段的量级完全不同（实测冻结帧 ~550 ms、窗口候选 ~3 ms、后端交付 ~0 ms、
/// webview 冷启动 ~240 ms、前端绘制 ~130 ms）。所以每次会话都记一条汇总日志，别再靠猜。
/// 完整分解见 docs/capture-linux.md §3.1。
#[derive(Debug, Clone, Copy)]
pub(super) struct StageTimings {
    /// 会话开始（`show_capture_overlay` 进入）的时刻。
    pub started: Instant,
    /// 隐藏源窗口 + 等合成器 + 后端取冻结帧。
    pub frames_ms: f64,
    /// 窗口速选候选枚举。
    pub probe_ms: f64,
    /// 覆盖层第一次来取 payload 时距会话开始的时间，等价于"建窗 + webview 冷启动"。
    pub payload_at_ms: f64,
    /// 后端交付 payload 与原始帧字节的累计时间（多屏会累加）。
    pub deliver_ms: f64,
}

impl Default for StageTimings {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            frames_ms: 0.0,
            probe_ms: 0.0,
            payload_at_ms: 0.0,
            deliver_ms: 0.0,
        }
    }
}

impl StageTimings {
    pub(super) fn start() -> Self {
        Self::default()
    }

    fn elapsed_ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }
}

fn since(at: Instant) -> f64 {
    at.elapsed().as_secs_f64() * 1000.0
}

#[derive(Default)]
pub struct CaptureManager {
    session: Mutex<Option<CaptureSession>>,
    /// 最近一次 I4 观测。**刻意活得比会话长**：用户总是"截完图发现界面错位"之后才去点诊断，
    /// 那时会话早就结束了。存在这里，诊断报告才有唯一那条闭环自检的结果可写。
    last_viewport: Mutex<Option<ViewportObservation>>,
}

/// renderer v2 在锁外消耗的冻结帧快照。
pub(super) struct CaptureRenderInput {
    pub source: image::RgbaImage,
    pub crop: (u32, u32, u32, u32),
}

/// 一次 I4 观测：后端算的逻辑尺寸、前端实测的视口，以及两者的差。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ViewportObservation {
    /// 后端下发给这块覆盖层的显示器逻辑尺寸。
    pub expected: (u32, u32),
    /// 前端 `window.innerWidth/innerHeight` 实测到的。
    pub actual: (u32, u32),
    /// `None` 表示这次对上了。
    pub mismatch: Option<(i64, i64)>,
}

#[derive(Debug)]
pub(super) struct CaptureSession {
    pub id: String,
    /// 每次 begin 都新建的后端身份，阻止可重复字符串 id 形成 ABA。
    #[cfg_attr(not(test), allow(dead_code))]
    identity: Arc<()>,
    pub overlays: Vec<OverlaySpec>,
    pub restore_labels: Vec<String>,
    /// 截图期间被临时降出置顶层的贴图。会话无论怎么结束都要把它们放回去，
    /// 所以跟着会话走而不是留在调用方的局部变量里（取消、覆盖层被杀、提交失败都算结束）。
    pub lowered_pins: Vec<String>,
    frames: Vec<CapturedMonitorFrame>,
    windows: HashMap<u32, Vec<WindowCandidate>>,
    /// 本次会话要不要在覆盖层里提示安装窗口速选服务。由 `begin` 的调用方决定，
    /// manager 不去碰桌面环境与配置。
    probe_hint: bool,
    /// 已经有覆盖层拿到键盘焦点。没有它的话，光标不在任何覆盖层里（Wayland 下拿不到光标时）
    /// 就没人接 Esc，整个会话只能靠杀窗口退出。
    focus_assigned: bool,
    timings: StageTimings,
    mode_ownership: CaptureModeOwnership,
}

impl CaptureSession {
    pub(super) fn overlay_labels(&self) -> Vec<String> {
        self.overlays
            .iter()
            .map(|spec| spec.label.clone())
            .collect()
    }

    /// 资源恢复完成后的唯一模式终结入口。
    pub(super) fn finalize_mode(self) -> Result<(), CaptureError> {
        self.mode_ownership.release()
    }
}

/// 成功启动的精确会话身份与覆盖层规格。
#[derive(Debug)]
pub(super) struct CaptureStart {
    pub session_id: String,
    pub overlays: Vec<OverlaySpec>,
}

/// `begin` 失败时把尚未转交的模式所有权原样返还给入口。
#[derive(Debug)]
pub(super) struct CaptureBeginFailure {
    pub error: CaptureError,
    pub ownership: CaptureModeOwnership,
}

/// 长截图首帧与精确普通会话身份的只读候选。
///
/// 候选不可复制；冻结帧像素只浅克隆其 `Arc`，prepare 不消费普通会话。
#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct CaptureLongshotCandidate {
    selection: CaptureSelection,
    frame: CapturedMonitorFrame,
    identity: Arc<()>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl CaptureLongshotCandidate {
    pub(super) fn selection(&self) -> &CaptureSelection {
        &self.selection
    }

    pub(super) fn frame(&self) -> &CapturedMonitorFrame {
        &self.frame
    }
}

/// 从普通截图会话唯一移出的桌面清理资源。
#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct OrdinaryCaptureResources {
    pub overlays: Vec<OverlaySpec>,
    pub restore_labels: Vec<String>,
    pub lowered_pins: Vec<String>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl OrdinaryCaptureResources {
    pub(super) fn overlay_labels(&self) -> Vec<String> {
        self.overlays
            .iter()
            .map(|spec| spec.label.clone())
            .collect()
    }
}

/// 普通截图原子移交给长截图后的唯一所有权与清理资源。
#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct CaptureLongshotHandoff {
    pub ownership: CaptureModeOwnership,
    pub resources: OrdinaryCaptureResources,
}

fn selected_frame_in_session<'a>(
    session: &'a CaptureSession,
    selection: &CaptureSelection,
) -> Result<&'a CapturedMonitorFrame, CaptureError> {
    if session.id != selection.session_id {
        return Err(CaptureError::SessionSupersededRetry);
    }
    session
        .frames
        .iter()
        .find(|frame| frame.monitor_id == selection.monitor_id)
        .ok_or(CaptureError::SelectionMonitorMismatch)
}

/// 视口比对允许的误差：CSS 像素和逻辑像素之间会有一格取整噪声。
const VIEWPORT_TOLERANCE: i64 = 1;

/// **不变量 I4：覆盖层的真实可见视口必须等于它那块屏的逻辑尺寸。**
///
/// 这是整条几何链路上唯一的**闭环**。I1–I3 都是后端拿自己的数据互相对账，只有这一条
/// 看到了"合成器最终摆出来的样子"——而那才是用户看到的东西。全屏请求并不保证窗口被压到
/// 显示器尺寸：内容的最小尺寸比显示器大时，合成器给了 fullscreen 状态、GTK 仍按内容尺寸
/// 分配，窗口于是居中摆放并溢到隔壁屏（实测，见 docs/capture-linux.md §4）。那种情况下
/// I1–I3 全过，用户看到的却是错位加工具条消失。
///
/// 返回宽高各自的差值（实测 − 预期），在容差内返回 `None`。
pub(super) fn viewport_mismatch(expected: (u32, u32), actual: (u32, u32)) -> Option<(i64, i64)> {
    // 视口为 0 说明窗口还没布局完（或前端拿不到），不是几何错误。
    if actual.0 == 0 || actual.1 == 0 {
        return None;
    }
    let dx = actual.0 as i64 - expected.0 as i64;
    let dy = actual.1 as i64 - expected.1 as i64;
    if dx.abs() <= VIEWPORT_TOLERANCE && dy.abs() <= VIEWPORT_TOLERANCE {
        None
    } else {
        Some((dx, dy))
    }
}

/// `reveal` 的结论：这块覆盖层要不要顺带抢键盘焦点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RevealPlan {
    pub take_focus: bool,
}

impl CaptureManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn begin(
        &self,
        frames: Vec<CapturedMonitorFrame>,
        restore_labels: Vec<String>,
        lowered_pins: Vec<String>,
        probe_hint: bool,
        mut timings: StageTimings,
        ownership: CaptureModeOwnership,
    ) -> Result<CaptureStart, CaptureBeginFailure> {
        if frames.is_empty() {
            return Err(CaptureBeginFailure {
                error: CaptureError::NoMonitorFrames,
                ownership,
            });
        }
        let id = crate::image_io::unique_image_id();
        let at = Instant::now();
        let windows = probe_windows(&frames);
        timings.probe_ms = since(at);
        let specs: Vec<_> = frames
            .iter()
            .map(|frame| OverlaySpec {
                label: format!("capture-overlay-{id}-{}", frame.monitor_id),
                x: frame.x,
                y: frame.y,
                width: frame.logical_width,
                height: frame.logical_height,
            })
            .collect();
        let mut current = match self.session.lock() {
            Ok(current) => current,
            Err(error) => {
                return Err(CaptureBeginFailure {
                    error: CaptureError::state_lock(error),
                    ownership,
                });
            }
        };
        if current.is_some() {
            return Err(CaptureBeginFailure {
                error: CaptureError::SessionBusy,
                ownership,
            });
        }
        *current = Some(CaptureSession {
            id: id.clone(),
            identity: Arc::new(()),
            frames,
            overlays: specs.clone(),
            restore_labels,
            lowered_pins,
            windows,
            probe_hint,
            focus_assigned: false,
            timings,
            mode_ownership: ownership,
        });
        Ok(CaptureStart {
            session_id: id,
            overlays: specs,
        })
    }

    /// 短锁验证创建调用仍属于精确会话。
    pub(super) fn ensure_current(&self, session_id: &str) -> Result<(), CaptureError> {
        let current = self.session.lock().map_err(CaptureError::state_lock)?;
        let session = current.as_ref().ok_or(CaptureError::SessionMissing)?;
        if session.id != session_id {
            return Err(CaptureError::SessionSuperseded);
        }
        Ok(())
    }

    /// 覆盖层报告"首帧已经画好，可以显示了"。
    ///
    /// 覆盖层是隐藏建窗的：webview 加载 + 取 payload + 铺底图期间窗口一旦可见，
    /// 用户看到的就是一整屏 webview 默认底色（白屏）。所以显示时机由前端决定。
    /// `viewport` 是前端实测的可见视口（CSS 像素），用来闭合不变量 I4；
    /// 拿不到时传 `None`，只是少一条自检，绝不影响显示。
    pub(super) fn reveal(
        &self,
        label: &str,
        cursor: Option<(f64, f64)>,
        viewport: Option<(u32, u32)>,
    ) -> Result<RevealPlan, CaptureError> {
        let mut current = self.session.lock().map_err(CaptureError::state_lock)?;
        let session = current.as_mut().ok_or(CaptureError::SessionMissing)?;
        let spec = session
            .overlays
            .iter()
            .find(|spec| spec.label == label)
            .ok_or(CaptureError::OverlayNotInSession)?;

        // **不变量 I4**：合成器最终摆出来的尺寸和我们算的逻辑尺寸对不上，这里是唯一的发现机会。
        if let Some(actual) = viewport {
            let expected = (spec.width, spec.height);
            let mismatch = viewport_mismatch(expected, actual);
            // 对上了也要记：诊断报告里"查过、通过了"和"没查过"是两回事。
            if let Ok(mut last) = self.last_viewport.lock() {
                *last = Some(ViewportObservation {
                    expected,
                    actual,
                    mismatch,
                });
            }
            if let Some((dx, dy)) = mismatch {
                log::error!(
                    "I4 覆盖层 {label} 的可见视口 {}x{} 与显示器逻辑尺寸 {}x{} 不一致（差 {dx}x{dy}）：\
                     几何很可能算错了，界面会错位；诊断见 docs/capture-linux.md §4",
                    actual.0,
                    actual.1,
                    spec.width,
                    spec.height,
                );
            }
        }
        // 光标所在的那块覆盖层独占焦点：合成器可能拒绝第二次 set_focus，
        // 所以不能让先画完的那块先抢一次再让给它。
        let cursor_owner = cursor.and_then(|(x, y)| {
            session
                .overlays
                .iter()
                .find(|spec| spec.contains(x, y))
                .map(|spec| spec.label.as_str())
        });
        let take_focus = match cursor_owner {
            Some(owner) => owner == label,
            // 拿不到光标位置（Wayland 常见），或光标落在没截到的显示器上：先画完的拿焦点，
            // 至少保证有一块能接 Esc。
            None => !session.focus_assigned,
        };
        if take_focus {
            session.focus_assigned = true;
        }
        let timings = session.timings;
        log::info!(
            "截图覆盖层 {label} 就绪：总 {:.0} ms = 冻结帧 {:.0} + 候选 {:.0} + 建窗与 webview {:.0} + 后端交付 {:.0} + 前端绘制 {:.0}",
            timings.elapsed_ms(),
            timings.frames_ms,
            timings.probe_ms,
            timings.payload_at_ms - timings.frames_ms - timings.probe_ms,
            timings.deliver_ms,
            timings.elapsed_ms() - timings.payload_at_ms - timings.deliver_ms,
        );
        Ok(RevealPlan { take_focus })
    }

    /// 最近一次 I4 观测，给诊断报告用。`None` 表示这个进程还没有截过图
    /// （或前端一次都没报上视口）——那时报告必须说"未观测"，不能说"通过"。
    pub(crate) fn last_viewport(&self) -> Option<ViewportObservation> {
        self.last_viewport.lock().ok().and_then(|last| *last)
    }

    pub(super) fn payload(&self, label: &str) -> Result<CaptureOverlayPayload, CaptureError> {
        let at = Instant::now();
        let mut current = self.session.lock().map_err(CaptureError::state_lock)?;
        let session = current.as_mut().ok_or(CaptureError::SessionMissing)?;
        // 多屏时每块覆盖层各取一次；只记第一次，它才代表"建窗 + webview 冷启动"。
        if session.timings.payload_at_ms == 0.0 {
            session.timings.payload_at_ms = session.timings.elapsed_ms();
        }
        let index = session
            .overlays
            .iter()
            .position(|spec| spec.label == label)
            .ok_or(CaptureError::OverlayNotInSession)?;
        let frame = session
            .frames
            .get(index)
            .ok_or(CaptureError::OverlayFrameMissing)?;
        let payload = CaptureOverlayPayload {
            session_id: session.id.clone(),
            monitor_id: frame.monitor_id,
            logical_x: frame.x,
            logical_y: frame.y,
            logical_width: frame.logical_width,
            logical_height: frame.logical_height,
            pixel_width: frame.pixel_width,
            pixel_height: frame.pixel_height,
            windows: session
                .windows
                .get(&frame.monitor_id)
                .cloned()
                .unwrap_or_default(),
            probe_hint: session.probe_hint,
        };
        session.timings.deliver_ms += since(at);
        Ok(payload)
    }

    /// 这块覆盖层的冻结帧原始 RGBA。
    ///
    /// 直接把 `Arc<[u8]>` 交出去（只有一次引用计数），由 IPC 以二进制原样送进 webview：
    /// 前端 `putImageData` 就能得到底图，全链路一次编解码都没有。曾经这里是
    /// "Rust 编 PNG → base64 → JSON → atob → webview 解 PNG"，实测四段加起来占了
    /// 覆盖层出现前的一半时间。
    pub(super) fn frame_rgba(&self, label: &str) -> Result<std::sync::Arc<[u8]>, CaptureError> {
        Ok(self.frame_source(label)?.0)
    }

    /// 自定义帧协议需要一次取得像素与尺寸，避免先取 payload、再取像素时重复锁会话。
    pub(super) fn frame_source(
        &self,
        label: &str,
    ) -> Result<(std::sync::Arc<[u8]>, u32, u32), CaptureError> {
        let at = Instant::now();
        let mut current = self.session.lock().map_err(CaptureError::state_lock)?;
        let session = current.as_mut().ok_or(CaptureError::SessionMissing)?;
        let index = session
            .overlays
            .iter()
            .position(|spec| spec.label == label)
            .ok_or(CaptureError::OverlayNotInSession)?;
        let frame = session
            .frames
            .get(index)
            .ok_or(CaptureError::OverlayFrameMissing)?;
        let source = (frame.rgba.clone(), frame.pixel_width, frame.pixel_height);
        session.timings.deliver_ms += since(at);
        Ok(source)
    }

    pub(super) fn crop(&self, selection: &CaptureSelection) -> Result<Vec<u8>, CaptureError> {
        let current = self.session.lock().map_err(CaptureError::state_lock)?;
        let session = current.as_ref().ok_or(CaptureError::SessionMissing)?;
        if session.id != selection.session_id {
            return Err(CaptureError::SessionSupersededRetry);
        }
        let frame = session
            .frames
            .iter()
            .find(|frame| frame.monitor_id == selection.monitor_id)
            .ok_or(CaptureError::SelectionMonitorMismatch)?;
        crop_frame(frame, selection)
    }

    /// 只在锁内核对普通截图会话并浅克隆目标帧；整屏像素仍由同一个 `Arc` 承载。
    pub(super) fn selected_frame(
        &self,
        selection: &CaptureSelection,
    ) -> Result<CapturedMonitorFrame, CaptureError> {
        let current = self.session.lock().map_err(CaptureError::state_lock)?;
        let session = current.as_ref().ok_or(CaptureError::SessionMissing)?;
        selected_frame_in_session(session, selection).cloned()
    }

    /// 准备长截图首帧候选；普通会话和 Ordinary ownership 均保持原样。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn prepare_longshot(
        &self,
        selection: &CaptureSelection,
    ) -> Result<CaptureLongshotCandidate, CaptureError> {
        let current = self.session.lock().map_err(CaptureError::state_lock)?;
        let session = current.as_ref().ok_or(CaptureError::SessionMissing)?;
        let frame = selected_frame_in_session(session, selection)?.clone();
        Ok(CaptureLongshotCandidate {
            selection: selection.clone(),
            frame,
            identity: Arc::clone(&session.identity),
        })
    }

    /// 精确消费 prepare 候选，并在 manager 锁内原子转换 mode ownership。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn commit_longshot(
        &self,
        candidate: CaptureLongshotCandidate,
    ) -> Result<CaptureLongshotHandoff, CaptureError> {
        let mut current = self.session.lock().map_err(CaptureError::state_lock)?;
        let Some(session) = current.take() else {
            return Err(CaptureError::SessionMissing);
        };
        if session.id != candidate.selection.session_id
            || !Arc::ptr_eq(&session.identity, &candidate.identity)
        {
            *current = Some(session);
            return Err(CaptureError::SessionSuperseded);
        }

        let CaptureSession {
            id,
            identity,
            overlays,
            restore_labels,
            lowered_pins,
            frames,
            windows,
            probe_hint,
            focus_assigned,
            timings,
            mode_ownership,
        } = session;

        let ownership = match mode_ownership.into_longshot() {
            Ok(ownership) => ownership,
            Err(failure) => {
                *current = Some(CaptureSession {
                    id,
                    identity,
                    overlays,
                    restore_labels,
                    lowered_pins,
                    frames,
                    windows,
                    probe_hint,
                    focus_assigned,
                    timings,
                    mode_ownership: failure.ownership,
                });
                return Err(failure.error);
            }
        };

        Ok(CaptureLongshotHandoff {
            ownership,
            resources: OrdinaryCaptureResources {
                overlays,
                restore_labels,
                lowered_pins,
            },
        })
    }

    /// 只在锁内核对会话并复制帧；数秒级合成必须由调用方在 blocking worker 执行。
    pub(super) fn render_input(
        &self,
        selection: &CaptureSelection,
    ) -> Result<CaptureRenderInput, CaptureError> {
        let current = self.session.lock().map_err(CaptureError::state_lock)?;
        let session = current.as_ref().ok_or(CaptureError::SessionMissing)?;
        if session.id != selection.session_id {
            return Err(CaptureError::SessionSupersededRetry);
        }
        let frame = session
            .frames
            .iter()
            .find(|frame| frame.monitor_id == selection.monitor_id)
            .ok_or(CaptureError::SelectionMonitorMismatch)?;
        let crop = selection_pixel_rect(frame, selection)?;
        let source =
            image::RgbaImage::from_raw(frame.pixel_width, frame.pixel_height, frame.rgba.to_vec())
                .ok_or(CaptureError::OverlayFrameMissing)?;
        Ok(CaptureRenderInput {
            source,
            crop: crop.as_crop(),
        })
    }

    pub(super) fn finish(&self, session_id: &str) -> Result<CaptureSession, CaptureError> {
        let mut current = self.session.lock().map_err(CaptureError::state_lock)?;
        let session = current.take().ok_or(CaptureError::SessionMissing)?;
        if session.id != session_id {
            *current = Some(session);
            return Err(CaptureError::SessionSuperseded);
        }
        Ok(session)
    }

    pub(super) fn abort_if_overlay(
        &self,
        label: &str,
    ) -> Result<Option<CaptureSession>, CaptureError> {
        let mut current = self.session.lock().map_err(CaptureError::state_lock)?;
        if current
            .as_ref()
            .is_some_and(|session| session.overlays.iter().any(|spec| spec.label == label))
        {
            Ok(current.take())
        } else {
            Ok(None)
        }
    }
}

fn crop_frame(
    frame: &CapturedMonitorFrame,
    selection: &CaptureSelection,
) -> Result<Vec<u8>, CaptureError> {
    let crop = selection_pixel_rect(frame, selection)?;
    let width = crop.width();
    let height = crop.height();
    let row_bytes = width as usize * 4;
    let mut rgba = Vec::with_capacity(row_bytes * height as usize);
    for row in crop.top..crop.bottom {
        let start = (row * frame.pixel_width + crop.left) as usize * 4;
        let source = frame
            .rgba
            .get(start..start + row_bytes)
            .ok_or(CaptureError::CropOutOfBounds)?;
        rgba.extend_from_slice(source);
    }
    crate::screenshot::encode_png(&rgba, width, height).map_err(CaptureError::codec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{CaptureMode, CaptureModeGate};
    use std::sync::Arc;

    fn ownership() -> CaptureModeOwnership {
        Arc::new(CaptureModeGate::new())
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("测试应取得 Ordinary")
    }

    fn ownership_on(gate: &Arc<CaptureModeGate>) -> CaptureModeOwnership {
        gate.try_claim_owned(CaptureMode::Ordinary)
            .expect("测试应取得指定 gate")
    }

    fn overlay(label: &str) -> OverlaySpec {
        OverlaySpec {
            label: label.to_string(),
            x: 0,
            y: 0,
            width: 100,
            height: 50,
        }
    }

    fn frame(scale: f32) -> CapturedMonitorFrame {
        let (logical_width, logical_height) = (100, 50);
        let pixel_width = (logical_width as f32 * scale) as u32;
        let pixel_height = (logical_height as f32 * scale) as u32;
        CapturedMonitorFrame {
            monitor_id: 7,
            x: 0,
            y: 0,
            logical_width,
            logical_height,
            pixel_width,
            pixel_height,
            scale_x: scale,
            scale_y: scale,
            rgba: Arc::from(vec![255; pixel_width as usize * pixel_height as usize * 4]),
        }
    }

    #[test]
    fn crop_maps_logical_selection_to_scaled_frame() {
        let png = crop_frame(
            &frame(2.0),
            &CaptureSelection {
                session_id: "test".to_string(),
                monitor_id: 7,
                x: 10.0,
                y: 5.0,
                width: 20.0,
                height: 10.0,
            },
        )
        .unwrap();
        assert_eq!(crate::screenshot::png_dimensions(&png).unwrap(), (40, 20));
    }

    #[test]
    fn crop_rejects_empty_and_non_finite_selection() {
        let mut selection = CaptureSelection {
            session_id: "test".to_string(),
            monitor_id: 7,
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 10.0,
        };
        assert!(crop_frame(&frame(1.0), &selection).is_err());
        selection.width = f64::NAN;
        assert!(crop_frame(&frame(1.0), &selection).is_err());
    }

    #[test]
    fn crop_keeps_session_alive_for_follow_up_actions() {
        let manager = CaptureManager::new();
        let monitor_frame = frame(1.0);
        let label = "capture-overlay-test-7".to_string();
        *manager.session.lock().unwrap() = Some(CaptureSession {
            id: "session-1".to_string(),
            identity: Arc::new(()),
            overlays: vec![overlay(&label)],
            focus_assigned: false,
            timings: StageTimings::default(),
            probe_hint: false,
            restore_labels: Vec::new(),
            lowered_pins: Vec::new(),
            frames: vec![monitor_frame],
            windows: HashMap::new(),
            mode_ownership: ownership(),
        });

        let selection = CaptureSelection {
            session_id: "session-1".to_string(),
            monitor_id: 7,
            x: 1.0,
            y: 1.0,
            width: 10.0,
            height: 10.0,
        };
        assert!(manager.crop(&selection).is_ok());
        let input = manager.render_input(&selection).unwrap();
        assert_eq!(input.source.dimensions(), (100, 50));
        assert_eq!(input.crop, (1, 1, 10, 10));
        assert!(manager.payload(&label).is_ok());
    }

    #[test]
    fn payload_carries_geometry_and_window_candidates_but_no_commit_action() {
        let manager = CaptureManager::new();
        let label = "capture-overlay-session-3-7".to_string();
        *manager.session.lock().unwrap() = Some(CaptureSession {
            id: "session-3".to_string(),
            identity: Arc::new(()),
            overlays: vec![overlay(&label)],
            focus_assigned: false,
            timings: StageTimings::default(),
            probe_hint: false,
            restore_labels: Vec::new(),
            lowered_pins: Vec::new(),
            frames: vec![frame(2.0)],
            windows: HashMap::from([(
                7,
                vec![WindowCandidate {
                    x: 4.0,
                    y: 6.0,
                    width: 40.0,
                    height: 30.0,
                    title: "picked".to_string(),
                }],
            )]),
            mode_ownership: ownership(),
        });

        let json = serde_json::to_value(manager.payload(&label).unwrap()).unwrap();
        // 逻辑尺寸给覆盖层排版，物理尺寸给画布导出；两者都必须下发。
        assert_eq!(json["logicalWidth"], 100);
        assert_eq!(json["logicalHeight"], 50);
        assert_eq!(json["pixelWidth"], 200);
        assert_eq!(json["pixelHeight"], 100);
        assert_eq!(json["windows"][0]["title"], "picked");
        assert_eq!(json["probeHint"], false);
        // 提交动作已经不由后端配置决定：工具条恒定显示在选区旁边。
        assert!(json.get("commitAction").is_none());
        // 像素不走 JSON：编一次 PNG + base64 再让 webview 解回来是纯粹的浪费，
        // 冻结帧由 frame_rgba 以二进制单独交付。
        assert!(json.get("pngBase64").is_none());
    }

    /// 覆盖层的底图走这条路：原始 RGBA、长度必须正好等于 4 × 像素数，
    /// 前端才能直接 `new ImageData(...)`。
    #[test]
    fn frame_rgba_serves_the_exact_pixel_buffer_of_that_overlay() {
        let manager = CaptureManager::new();
        let label = "capture-overlay-session-4-7".to_string();
        *manager.session.lock().unwrap() = Some(CaptureSession {
            id: "session-4".to_string(),
            identity: Arc::new(()),
            overlays: vec![overlay(&label)],
            focus_assigned: false,
            timings: StageTimings::default(),
            probe_hint: false,
            restore_labels: Vec::new(),
            lowered_pins: Vec::new(),
            frames: vec![frame(2.0)],
            windows: HashMap::new(),
            mode_ownership: ownership(),
        });

        let rgba = manager.frame_rgba(&label).unwrap();
        assert_eq!(rgba.len(), 200 * 100 * 4);
        assert!(rgba.iter().all(|byte| *byte == 255));
        // 不认识的 label 不能拿到任何一块屏的像素。
        assert_eq!(
            manager
                .frame_rgba("capture-overlay-session-4-9")
                .unwrap_err()
                .code(),
            "overlay_not_in_session"
        );
        manager.finish("session-4").unwrap();
        assert_eq!(
            manager.frame_rgba(&label).unwrap_err().code(),
            "session_missing"
        );
    }

    /// 多显示器时提示只应该出现一次，所以标志位挂在会话上而不是每块覆盖层各判一次。
    #[test]
    fn probe_hint_reaches_every_overlay_of_the_session() {
        let manager = CaptureManager::new();
        let specs = manager
            .begin(
                vec![frame(1.0), frame(1.0)],
                Vec::new(),
                Vec::new(),
                true,
                StageTimings::default(),
                ownership(),
            )
            .unwrap();
        assert_eq!(specs.overlays.len(), 2);

        for spec in &specs.overlays {
            assert!(manager.payload(&spec.label).unwrap().probe_hint);
        }
    }

    #[test]
    fn begin_returns_ownership_on_empty_busy_and_state_lock_failures() {
        let empty_manager = CaptureManager::new();
        let empty_gate = Arc::new(CaptureModeGate::new());
        let failure = empty_manager
            .begin(
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&empty_gate),
            )
            .expect_err("空帧应失败");
        assert_eq!(failure.error.code(), "no_monitor_frames");
        failure.ownership.release().expect("返还所有权仍可释放");
        assert_eq!(empty_gate.active_mode().unwrap(), None);

        let busy_manager = CaptureManager::new();
        let first_gate = Arc::new(CaptureModeGate::new());
        let first = busy_manager
            .begin(
                vec![frame(1.0)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&first_gate),
            )
            .expect("首个会话启动");
        let rejected_gate = Arc::new(CaptureModeGate::new());
        let failure = busy_manager
            .begin(
                vec![frame(1.0)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&rejected_gate),
            )
            .expect_err("已有会话应失败");
        assert_eq!(failure.error.code(), "session_busy");
        failure.ownership.release().expect("Busy 返还可释放");
        assert_eq!(rejected_gate.active_mode().unwrap(), None);
        busy_manager
            .finish(&first.session_id)
            .unwrap()
            .finalize_mode()
            .unwrap();

        let poisoned = CaptureManager::new();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = poisoned.session.lock().unwrap();
            panic!("制造 manager poison");
        }));
        let poison_gate = Arc::new(CaptureModeGate::new());
        let failure = poisoned
            .begin(
                vec![frame(1.0)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&poison_gate),
            )
            .expect_err("poison 应结构化失败");
        assert_eq!(failure.error.code(), "state_lock");
        failure.ownership.release().expect("StateLock 返还可释放");
        assert_eq!(poison_gate.active_mode().unwrap(), None);
    }

    #[test]
    fn exact_session_id_preserves_new_owner_against_late_finish() {
        let manager = CaptureManager::new();
        let first_gate = Arc::new(CaptureModeGate::new());
        let first = manager
            .begin(
                vec![frame(1.0)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&first_gate),
            )
            .unwrap();
        assert!(manager.ensure_current(&first.session_id).is_ok());
        let first_session = manager.finish(&first.session_id).unwrap();
        first_session.finalize_mode().unwrap();

        let second_gate = Arc::new(CaptureModeGate::new());
        let second = manager
            .begin(
                vec![frame(1.0)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&second_gate),
            )
            .unwrap();
        assert_eq!(
            manager.finish(&first.session_id).unwrap_err().code(),
            "session_superseded"
        );
        assert_eq!(
            manager
                .ensure_current(&first.session_id)
                .unwrap_err()
                .code(),
            "session_superseded"
        );
        assert_eq!(
            second_gate.active_mode().unwrap(),
            Some(CaptureMode::Ordinary)
        );
        manager
            .finish(&second.session_id)
            .unwrap()
            .finalize_mode()
            .unwrap();
    }

    #[test]
    fn read_and_render_operations_never_finalize_the_active_mode() {
        let manager = CaptureManager::new();
        let gate = Arc::new(CaptureModeGate::new());
        let start = manager
            .begin(
                vec![frame(1.0)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&gate),
            )
            .unwrap();
        let label = &start.overlays[0].label;
        let payload = manager.payload(label).unwrap();
        assert!(manager.frame_rgba(label).is_ok());
        assert!(manager.reveal(label, None, None).is_ok());
        let mut selection = CaptureSelection {
            session_id: payload.session_id,
            monitor_id: 7,
            x: 1.0,
            y: 1.0,
            width: 10.0,
            height: 10.0,
        };
        assert!(manager.crop(&selection).is_ok());
        assert!(manager.render_input(&selection).is_ok());
        selection.width = 1.0;
        assert_eq!(
            manager.crop(&selection).unwrap_err().code(),
            "selection_too_small"
        );
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));

        manager
            .finish(&start.session_id)
            .unwrap()
            .finalize_mode()
            .unwrap();
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn abort_if_overlay_is_single_winner_and_reports_poison() {
        let manager = Arc::new(CaptureManager::new());
        let gate = Arc::new(CaptureModeGate::new());
        let start = manager
            .begin(
                vec![frame(1.0)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&gate),
            )
            .unwrap();
        let label = start.overlays[0].label.clone();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let manager = Arc::clone(&manager);
            let barrier = Arc::clone(&barrier);
            let label = label.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                manager.abort_if_overlay(&label).unwrap()
            }));
        }
        barrier.wait();
        let mut winner = None;
        let mut none = 0;
        for worker in workers {
            match worker.join().unwrap() {
                Some(session) => winner = Some(session),
                None => none += 1,
            }
        }
        assert_eq!(none, 1);
        let session = winner.expect("只能有一个终结者");
        assert!(manager.abort_if_overlay(&label).unwrap().is_none());
        session.finalize_mode().unwrap();
        assert_eq!(gate.active_mode().unwrap(), None);

        let poisoned = CaptureManager::new();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = poisoned.session.lock().unwrap();
            panic!("制造 manager poison");
        }));
        assert_eq!(
            poisoned.abort_if_overlay("any").unwrap_err().code(),
            "state_lock"
        );
    }

    #[test]
    fn failed_crop_can_finish_its_session() {
        let manager = CaptureManager::new();
        let label = "capture-overlay-session-1-7".to_string();
        *manager.session.lock().unwrap() = Some(CaptureSession {
            id: "session-1".to_string(),
            identity: Arc::new(()),
            overlays: vec![overlay(&label)],
            focus_assigned: false,
            timings: StageTimings::default(),
            probe_hint: false,
            restore_labels: vec!["main".to_string()],
            lowered_pins: Vec::new(),
            frames: vec![frame(1.0)],
            windows: HashMap::new(),
            mode_ownership: ownership(),
        });
        let selection = CaptureSelection {
            session_id: "session-1".to_string(),
            monitor_id: 7,
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 10.0,
        };

        assert_eq!(
            manager.crop(&selection).unwrap_err().code(),
            "selection_too_small"
        );
        let session = manager.finish(&selection.session_id).unwrap();
        assert_eq!(session.overlay_labels(), vec![label.clone()]);
        assert_eq!(session.restore_labels, vec!["main"]);
        assert!(manager.payload(&label).is_err());
    }

    #[test]
    fn finish_mismatch_preserves_newer_session() {
        let manager = CaptureManager::new();
        let label = "capture-overlay-session-2-7".to_string();
        *manager.session.lock().unwrap() = Some(CaptureSession {
            id: "session-2".to_string(),
            identity: Arc::new(()),
            overlays: vec![overlay(&label)],
            focus_assigned: false,
            timings: StageTimings::default(),
            probe_hint: false,
            restore_labels: Vec::new(),
            lowered_pins: Vec::new(),
            frames: vec![frame(1.0)],
            windows: HashMap::new(),
            mode_ownership: ownership(),
        });

        assert_eq!(
            manager.finish("session-1").err().unwrap().code(),
            "session_superseded"
        );
        assert!(manager.payload(&label).is_ok());
        assert_eq!(manager.finish("session-2").unwrap().id, "session-2");
    }

    /// 双屏会话：左屏 (0,0) 1920x1200、右屏 (1920,0) 1920x1200。
    fn two_monitor_session(manager: &CaptureManager) -> (String, String) {
        let (left, right) = (
            "capture-overlay-session-9-1".to_string(),
            "capture-overlay-session-9-2".to_string(),
        );
        *manager.session.lock().unwrap() = Some(CaptureSession {
            id: "session-9".to_string(),
            identity: Arc::new(()),
            overlays: vec![
                OverlaySpec {
                    label: left.clone(),
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1200,
                },
                OverlaySpec {
                    label: right.clone(),
                    x: 1920,
                    y: 0,
                    width: 1920,
                    height: 1200,
                },
            ],
            focus_assigned: false,
            timings: StageTimings::default(),
            probe_hint: false,
            restore_labels: Vec::new(),
            lowered_pins: Vec::new(),
            frames: vec![frame(1.0)],
            windows: HashMap::new(),
            mode_ownership: ownership(),
        });
        (left, right)
    }

    #[test]
    fn viewport_mismatch_only_fires_on_real_disagreement() {
        // 一致（含 1 像素取整噪声）→ 不报。
        assert_eq!(viewport_mismatch((1920, 1200), (1920, 1200)), None);
        assert_eq!(viewport_mismatch((1920, 1200), (1919, 1201)), None);
        // 还没布局完 → 不是几何错误。
        assert_eq!(viewport_mismatch((1920, 1200), (0, 0)), None);
        // 真机上出过的那一幕：几何被算大 1.125 倍，窗口按 1920x1200 摆，
        // 画布却按 2160x1350 画，右下工具条落到窗口外面。
        assert_eq!(
            viewport_mismatch((2160, 1350), (1920, 1200)),
            Some((-240, -150))
        );
    }

    #[test]
    fn reveal_reports_the_viewport_without_refusing_to_show_the_overlay() {
        // I4 失败只该留日志：拒绝显示等于让用户完全用不了截图。
        let manager = CaptureManager::new();
        let (left, _) = two_monitor_session(&manager);
        assert!(
            manager
                .reveal(&left, None, Some((800, 600)))
                .unwrap()
                .take_focus
        );
    }

    #[test]
    fn reveal_gives_focus_to_the_overlay_under_the_cursor() {
        let manager = CaptureManager::new();
        let (left, right) = two_monitor_session(&manager);
        // 光标在右屏：左屏先报告首帧也不该抢走焦点。
        let cursor = Some((2400.0, 300.0));
        assert!(!manager.reveal(&left, cursor, None).unwrap().take_focus);
        assert!(manager.reveal(&right, cursor, None).unwrap().take_focus);
    }

    #[test]
    fn reveal_still_focuses_one_overlay_when_the_cursor_is_unknown() {
        let manager = CaptureManager::new();
        let (left, right) = two_monitor_session(&manager);
        // Wayland 下拿不到光标位置时必须有人接键盘，否则 Esc 取消都用不了。
        assert!(manager.reveal(&left, None, None).unwrap().take_focus);
        assert!(!manager.reveal(&right, None, None).unwrap().take_focus);
    }

    #[test]
    fn reveal_rejects_labels_outside_the_current_session() {
        let manager = CaptureManager::new();
        assert_eq!(
            manager
                .reveal("capture-overlay-none-1", None, None)
                .unwrap_err()
                .code(),
            "session_missing"
        );
        let (left, _) = two_monitor_session(&manager);
        assert_eq!(
            manager
                .reveal("capture-overlay-other-1", None, None)
                .unwrap_err()
                .code(),
            "overlay_not_in_session"
        );
        assert!(manager.reveal(&left, None, None).is_ok());
    }

    fn selection_for(session_id: &str) -> CaptureSelection {
        CaptureSelection {
            session_id: session_id.to_string(),
            monitor_id: 7,
            x: 1.0,
            y: 1.0,
            width: 10.0,
            height: 10.0,
        }
    }

    #[test]
    fn longshot_prepare_shares_pixels_and_preserves_the_ordinary_session() {
        let manager = CaptureManager::new();
        let gate = Arc::new(CaptureModeGate::new());
        let source = frame(1.0);
        let source_pixels = Arc::clone(&source.rgba);
        let start = manager
            .begin(
                vec![source],
                vec!["main".to_string()],
                vec!["pin-a".to_string()],
                true,
                StageTimings::default(),
                ownership_on(&gate),
            )
            .expect("启动普通会话");
        let selection = selection_for(&start.session_id);

        let candidate = manager.prepare_longshot(&selection).expect("prepare 成功");
        assert_eq!(candidate.selection().session_id, selection.session_id);
        assert_eq!(candidate.selection().monitor_id, selection.monitor_id);
        assert!(Arc::ptr_eq(&candidate.frame().rgba, &source_pixels));
        assert!(Arc::ptr_eq(
            &manager.selected_frame(&selection).unwrap().rgba,
            &candidate.frame().rgba
        ));
        assert!(manager.payload(&start.overlays[0].label).is_ok());
        assert!(manager.crop(&selection).is_ok());
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));

        drop(candidate);
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));
        manager
            .finish(&start.session_id)
            .unwrap()
            .finalize_mode()
            .unwrap();
    }

    #[test]
    fn longshot_prepare_error_order_preserves_session_and_gate() {
        let missing = CaptureManager::new();
        assert_eq!(
            missing
                .prepare_longshot(&selection_for("none"))
                .unwrap_err()
                .code(),
            "session_missing"
        );

        let manager = CaptureManager::new();
        let gate = Arc::new(CaptureModeGate::new());
        let start = manager
            .begin(
                vec![frame(1.0)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&gate),
            )
            .unwrap();
        let mut stale = selection_for("stale");
        assert_eq!(
            manager.prepare_longshot(&stale).unwrap_err().code(),
            "session_superseded_retry"
        );
        stale.session_id = start.session_id.clone();
        stale.monitor_id = 999;
        assert_eq!(
            manager.prepare_longshot(&stale).unwrap_err().code(),
            "selection_monitor_mismatch"
        );
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));
        assert!(manager.payload(&start.overlays[0].label).is_ok());
        manager
            .finish(&start.session_id)
            .unwrap()
            .finalize_mode()
            .unwrap();

        let poisoned = CaptureManager::new();
        let poisoned_gate = Arc::new(CaptureModeGate::new());
        poisoned
            .begin(
                vec![frame(1.0)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&poisoned_gate),
            )
            .unwrap();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = poisoned.session.lock().unwrap();
            panic!("制造 manager poison");
        }));
        assert_eq!(
            poisoned
                .prepare_longshot(&selection_for("none"))
                .unwrap_err()
                .code(),
            "state_lock"
        );
        assert_eq!(
            poisoned_gate.active_mode().unwrap(),
            Some(CaptureMode::Ordinary)
        );
    }

    #[test]
    fn longshot_commit_moves_resources_and_has_one_explicit_finalizer() {
        let manager = CaptureManager::new();
        let gate = Arc::new(CaptureModeGate::new());
        let start = manager
            .begin(
                vec![frame(1.0)],
                vec!["main", "settings"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                vec!["pin-b", "pin-a"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                false,
                StageTimings::default(),
                ownership_on(&gate),
            )
            .unwrap();
        let candidate = manager
            .prepare_longshot(&selection_for(&start.session_id))
            .unwrap();
        let second = manager
            .prepare_longshot(&selection_for(&start.session_id))
            .unwrap();

        let handoff = manager.commit_longshot(candidate).expect("commit 成功");
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));
        assert_eq!(
            handoff.resources.overlay_labels(),
            vec![start.overlays[0].label.clone()]
        );
        assert_eq!(handoff.resources.restore_labels, vec!["main", "settings"]);
        assert_eq!(handoff.resources.lowered_pins, vec!["pin-b", "pin-a"]);
        assert_eq!(
            manager.finish(&start.session_id).unwrap_err().code(),
            "session_missing"
        );
        assert!(manager
            .abort_if_overlay(&start.overlays[0].label)
            .unwrap()
            .is_none());
        assert_eq!(
            manager.commit_longshot(second).unwrap_err().code(),
            "session_missing"
        );
        handoff.ownership.release().expect("handoff 唯一释放 gate");
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn ordinary_finish_before_commit_keeps_the_candidate_inert() {
        let manager = CaptureManager::new();
        let gate = Arc::new(CaptureModeGate::new());
        let start = manager
            .begin(
                vec![frame(1.0)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&gate),
            )
            .unwrap();
        let candidate = manager
            .prepare_longshot(&selection_for(&start.session_id))
            .unwrap();
        let ordinary = manager.finish(&start.session_id).unwrap();

        assert_eq!(
            manager.commit_longshot(candidate).unwrap_err().code(),
            "session_missing"
        );
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));
        ordinary.finalize_mode().unwrap();
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn longshot_candidate_rejects_same_string_id_aba() {
        let manager = CaptureManager::new();
        let old_gate = Arc::new(CaptureModeGate::new());
        let old = manager
            .begin(
                vec![frame(1.0)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&old_gate),
            )
            .unwrap();
        let candidate = manager
            .prepare_longshot(&selection_for(&old.session_id))
            .unwrap();
        manager
            .finish(&old.session_id)
            .unwrap()
            .finalize_mode()
            .unwrap();

        let new_gate = Arc::new(CaptureModeGate::new());
        let new = manager
            .begin(
                vec![frame(2.0)],
                vec!["new-window".to_string()],
                vec!["new-pin".to_string()],
                true,
                StageTimings::default(),
                ownership_on(&new_gate),
            )
            .unwrap();
        manager.session.lock().unwrap().as_mut().unwrap().id = old.session_id.clone();
        let (new_identity, new_pixels) = {
            let guard = manager.session.lock().unwrap();
            let session = guard.as_ref().unwrap();
            (
                Arc::clone(&session.identity),
                Arc::clone(&session.frames[0].rgba),
            )
        };

        assert_eq!(
            manager.commit_longshot(candidate).unwrap_err().code(),
            "session_superseded"
        );
        assert_eq!(new_gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));
        assert!(manager.payload(&new.overlays[0].label).is_ok());
        {
            let guard = manager.session.lock().unwrap();
            let session = guard.as_ref().unwrap();
            assert!(Arc::ptr_eq(&new_identity, &session.identity));
            assert!(Arc::ptr_eq(&new_pixels, &session.frames[0].rgba));
            assert!(session.probe_hint);
        }
        let preserved = manager.finish(&old.session_id).unwrap();
        assert_eq!(preserved.restore_labels, vec!["new-window"]);
        assert_eq!(preserved.lowered_pins, vec!["new-pin"]);
        preserved.finalize_mode().unwrap();
    }

    #[test]
    fn longshot_generation_exhaustion_restores_the_complete_session() {
        let manager = CaptureManager::new();
        let gate = Arc::new(CaptureModeGate::with_last_generation(u64::MAX - 1));
        let session_id = "exhausted-session".to_string();
        let label = "capture-overlay-exhausted-session-7".to_string();
        let timings = StageTimings {
            started: Instant::now(),
            frames_ms: 11.0,
            probe_ms: 12.0,
            payload_at_ms: 13.0,
            deliver_ms: 14.0,
        };
        *manager.session.lock().unwrap() = Some(CaptureSession {
            id: session_id.clone(),
            identity: Arc::new(()),
            overlays: vec![overlay(&label)],
            restore_labels: vec!["restore-b", "restore-a"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            lowered_pins: vec!["pin-b", "pin-a"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            frames: vec![frame(1.0)],
            windows: HashMap::from([(
                7,
                vec![WindowCandidate {
                    x: 4.0,
                    y: 6.0,
                    width: 40.0,
                    height: 30.0,
                    title: "preserved-window".to_string(),
                }],
            )]),
            probe_hint: true,
            focus_assigned: true,
            timings,
            mode_ownership: ownership_on(&gate),
        });
        let selection = selection_for(&session_id);
        let candidate = manager.prepare_longshot(&selection).unwrap();
        let before = {
            let guard = manager.session.lock().unwrap();
            let session = guard.as_ref().unwrap();
            (
                session.id.clone(),
                Arc::clone(&session.identity),
                Arc::clone(&session.frames[0].rgba),
                (
                    session.frames[0].monitor_id,
                    session.frames[0].x,
                    session.frames[0].y,
                    session.frames[0].logical_width,
                    session.frames[0].logical_height,
                    session.frames[0].pixel_width,
                    session.frames[0].pixel_height,
                    session.frames[0].scale_x,
                    session.frames[0].scale_y,
                ),
                session
                    .overlays
                    .iter()
                    .map(|spec| (spec.label.clone(), spec.x, spec.y, spec.width, spec.height))
                    .collect::<Vec<_>>(),
                session.restore_labels.clone(),
                session.lowered_pins.clone(),
                serde_json::to_value(&session.windows).unwrap(),
                session.probe_hint,
                session.focus_assigned,
                session.timings.started,
                session.timings.frames_ms,
                session.timings.probe_ms,
                session.timings.payload_at_ms,
                session.timings.deliver_ms,
            )
        };

        assert_eq!(
            manager.commit_longshot(candidate).unwrap_err().code(),
            "capture_mode_generation_exhausted"
        );
        let guard = manager.session.lock().unwrap();
        let restored = guard.as_ref().expect("失败必须放回 session");
        assert_eq!(before.0, restored.id);
        assert!(Arc::ptr_eq(&before.1, &restored.identity));
        assert!(Arc::ptr_eq(&before.2, &restored.frames[0].rgba));
        assert_eq!(
            before.3,
            (
                restored.frames[0].monitor_id,
                restored.frames[0].x,
                restored.frames[0].y,
                restored.frames[0].logical_width,
                restored.frames[0].logical_height,
                restored.frames[0].pixel_width,
                restored.frames[0].pixel_height,
                restored.frames[0].scale_x,
                restored.frames[0].scale_y,
            )
        );
        assert_eq!(
            before.4,
            restored
                .overlays
                .iter()
                .map(|spec| { (spec.label.clone(), spec.x, spec.y, spec.width, spec.height,) })
                .collect::<Vec<_>>()
        );
        assert_eq!(before.5, restored.restore_labels);
        assert_eq!(before.6, restored.lowered_pins);
        assert_eq!(before.7, serde_json::to_value(&restored.windows).unwrap());
        assert_eq!(before.8, restored.probe_hint);
        assert_eq!(before.9, restored.focus_assigned);
        assert_eq!(before.10, restored.timings.started);
        assert_eq!(before.11, restored.timings.frames_ms);
        assert_eq!(before.12, restored.timings.probe_ms);
        assert_eq!(before.13, restored.timings.payload_at_ms);
        assert_eq!(before.14, restored.timings.deliver_ms);
        drop(guard);
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));
        assert!(manager.payload(&label).is_ok());
        assert!(manager.crop(&selection).is_ok());
        manager
            .finish(&session_id)
            .unwrap()
            .finalize_mode()
            .unwrap();
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn eight_concurrent_longshot_commits_have_exactly_one_winner() {
        let manager = Arc::new(CaptureManager::new());
        let gate = Arc::new(CaptureModeGate::new());
        let start = manager
            .begin(
                vec![frame(1.0)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&gate),
            )
            .unwrap();
        let candidates: Vec<_> = (0..8)
            .map(|_| {
                manager
                    .prepare_longshot(&selection_for(&start.session_id))
                    .unwrap()
            })
            .collect();
        let barrier = Arc::new(std::sync::Barrier::new(9));
        let workers: Vec<_> = candidates
            .into_iter()
            .map(|candidate| {
                let manager = Arc::clone(&manager);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    manager.commit_longshot(candidate)
                })
            })
            .collect();
        barrier.wait();

        let mut winner = None;
        let mut loser_codes = Vec::new();
        for worker in workers {
            match worker.join().expect("commit worker 不应 panic") {
                Ok(handoff) => {
                    assert!(winner.replace(handoff).is_none(), "只能产生一份 handoff")
                }
                Err(error) => loser_codes.push(error.code()),
            }
        }
        assert_eq!(loser_codes.len(), 7);
        assert!(loser_codes
            .iter()
            .all(|code| matches!(*code, "session_missing" | "session_superseded")));
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));
        winner.expect("必须有一个赢家").ownership.release().unwrap();
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn dropping_a_successful_handoff_does_not_release_longshot_mode() {
        let manager = CaptureManager::new();
        let gate = Arc::new(CaptureModeGate::new());
        let start = manager
            .begin(
                vec![frame(1.0)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ownership_on(&gate),
            )
            .unwrap();
        let handoff = manager
            .commit_longshot(
                manager
                    .prepare_longshot(&selection_for(&start.session_id))
                    .unwrap(),
            )
            .unwrap();

        drop(handoff);
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));
    }
}
