use super::error::CaptureError;
use super::types::{CaptureOverlayPayload, CaptureSelection, OverlaySpec, WindowCandidate};
use super::window_probe::probe_windows;
use crate::screenshot::CapturedMonitorFrame;
use std::collections::HashMap;
use std::sync::Mutex;
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
}

pub(super) struct CaptureSession {
    pub id: String,
    pub overlays: Vec<OverlaySpec>,
    pub restore_labels: Vec<String>,
    frames: Vec<CapturedMonitorFrame>,
    windows: HashMap<u32, Vec<WindowCandidate>>,
    /// 本次会话要不要在覆盖层里提示安装窗口速选服务。由 `begin` 的调用方决定，
    /// manager 不去碰桌面环境与配置。
    probe_hint: bool,
    /// 已经有覆盖层拿到键盘焦点。没有它的话，光标不在任何覆盖层里（Wayland 下拿不到光标时）
    /// 就没人接 Esc，整个会话只能靠杀窗口退出。
    focus_assigned: bool,
    timings: StageTimings,
}

impl CaptureSession {
    pub(super) fn overlay_labels(&self) -> Vec<String> {
        self.overlays
            .iter()
            .map(|spec| spec.label.clone())
            .collect()
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
        probe_hint: bool,
        mut timings: StageTimings,
    ) -> Result<Vec<OverlaySpec>, CaptureError> {
        if frames.is_empty() {
            return Err(CaptureError::NoMonitorFrames);
        }
        let mut current = self.session.lock().map_err(CaptureError::state_lock)?;
        if current.is_some() {
            return Err(CaptureError::SessionBusy);
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
        *current = Some(CaptureSession {
            id,
            frames,
            overlays: specs.clone(),
            restore_labels,
            windows,
            probe_hint,
            focus_assigned: false,
            timings,
        });
        Ok(specs)
    }

    /// 覆盖层报告"首帧已经画好，可以显示了"。
    ///
    /// 覆盖层是隐藏建窗的：webview 加载 + 取 payload + 铺底图期间窗口一旦可见，
    /// 用户看到的就是一整屏 webview 默认底色（白屏）。所以显示时机由前端决定。
    pub(super) fn reveal(
        &self,
        label: &str,
        cursor: Option<(f64, f64)>,
    ) -> Result<RevealPlan, CaptureError> {
        let mut current = self.session.lock().map_err(CaptureError::state_lock)?;
        let session = current.as_mut().ok_or(CaptureError::SessionMissing)?;
        if !session.overlays.iter().any(|spec| spec.label == label) {
            return Err(CaptureError::OverlayNotInSession);
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
        let at = Instant::now();
        let mut current = self.session.lock().map_err(CaptureError::state_lock)?;
        let session = current.as_mut().ok_or(CaptureError::SessionMissing)?;
        let index = session
            .overlays
            .iter()
            .position(|spec| spec.label == label)
            .ok_or(CaptureError::OverlayNotInSession)?;
        let rgba = session
            .frames
            .get(index)
            .ok_or(CaptureError::OverlayFrameMissing)?
            .rgba
            .clone();
        session.timings.deliver_ms += since(at);
        Ok(rgba)
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

    pub(super) fn finish(&self, session_id: &str) -> Result<CaptureSession, CaptureError> {
        let mut current = self.session.lock().map_err(CaptureError::state_lock)?;
        let session = current.take().ok_or(CaptureError::SessionMissing)?;
        if session.id != session_id {
            *current = Some(session);
            return Err(CaptureError::SessionSuperseded);
        }
        Ok(session)
    }

    pub(super) fn abort(&self) -> Option<CaptureSession> {
        self.session.lock().ok()?.take()
    }

    pub(super) fn abort_if_overlay(&self, label: &str) -> Option<CaptureSession> {
        let mut current = self.session.lock().ok()?;
        if current
            .as_ref()
            .is_some_and(|session| session.overlays.iter().any(|spec| spec.label == label))
        {
            current.take()
        } else {
            None
        }
    }
}

fn crop_frame(
    frame: &CapturedMonitorFrame,
    selection: &CaptureSelection,
) -> Result<Vec<u8>, CaptureError> {
    for value in [selection.x, selection.y, selection.width, selection.height] {
        if !value.is_finite() {
            return Err(CaptureError::SelectionNotFinite);
        }
    }
    if selection.width < 2.0 || selection.height < 2.0 {
        return Err(CaptureError::SelectionTooSmall);
    }
    let left = (selection.x.max(0.0) * frame.scale_x as f64).floor() as u32;
    let top = (selection.y.max(0.0) * frame.scale_y as f64).floor() as u32;
    let right = ((selection.x + selection.width).min(frame.logical_width as f64)
        * frame.scale_x as f64)
        .ceil() as u32;
    let bottom = ((selection.y + selection.height).min(frame.logical_height as f64)
        * frame.scale_y as f64)
        .ceil() as u32;
    let (left, top) = (left.min(frame.pixel_width), top.min(frame.pixel_height));
    let (right, bottom) = (right.min(frame.pixel_width), bottom.min(frame.pixel_height));
    if right <= left || bottom <= top {
        return Err(CaptureError::SelectionEmpty);
    }
    let width = right - left;
    let height = bottom - top;
    let row_bytes = width as usize * 4;
    let mut rgba = Vec::with_capacity(row_bytes * height as usize);
    for row in top..bottom {
        let start = (row * frame.pixel_width + left) as usize * 4;
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
    use std::sync::Arc;

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
            overlays: vec![overlay(&label)],
            focus_assigned: false,
            timings: StageTimings::default(),
            probe_hint: false,
            restore_labels: Vec::new(),
            frames: vec![monitor_frame],
            windows: HashMap::new(),
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
        assert!(manager.payload(&label).is_ok());
    }

    #[test]
    fn payload_carries_geometry_and_window_candidates_but_no_commit_action() {
        let manager = CaptureManager::new();
        let label = "capture-overlay-session-3-7".to_string();
        *manager.session.lock().unwrap() = Some(CaptureSession {
            id: "session-3".to_string(),
            overlays: vec![overlay(&label)],
            focus_assigned: false,
            timings: StageTimings::default(),
            probe_hint: false,
            restore_labels: Vec::new(),
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
            overlays: vec![overlay(&label)],
            focus_assigned: false,
            timings: StageTimings::default(),
            probe_hint: false,
            restore_labels: Vec::new(),
            frames: vec![frame(2.0)],
            windows: HashMap::new(),
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
                true,
                StageTimings::default(),
            )
            .unwrap();
        assert_eq!(specs.len(), 2);

        for spec in &specs {
            assert!(manager.payload(&spec.label).unwrap().probe_hint);
        }
    }

    #[test]
    fn failed_crop_can_finish_its_session() {
        let manager = CaptureManager::new();
        let label = "capture-overlay-session-1-7".to_string();
        *manager.session.lock().unwrap() = Some(CaptureSession {
            id: "session-1".to_string(),
            overlays: vec![overlay(&label)],
            focus_assigned: false,
            timings: StageTimings::default(),
            probe_hint: false,
            restore_labels: vec!["main".to_string()],
            frames: vec![frame(1.0)],
            windows: HashMap::new(),
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
            overlays: vec![overlay(&label)],
            focus_assigned: false,
            timings: StageTimings::default(),
            probe_hint: false,
            restore_labels: Vec::new(),
            frames: vec![frame(1.0)],
            windows: HashMap::new(),
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
            frames: vec![frame(1.0)],
            windows: HashMap::new(),
        });
        (left, right)
    }

    #[test]
    fn reveal_gives_focus_to_the_overlay_under_the_cursor() {
        let manager = CaptureManager::new();
        let (left, right) = two_monitor_session(&manager);
        // 光标在右屏：左屏先报告首帧也不该抢走焦点。
        let cursor = Some((2400.0, 300.0));
        assert!(!manager.reveal(&left, cursor).unwrap().take_focus);
        assert!(manager.reveal(&right, cursor).unwrap().take_focus);
    }

    #[test]
    fn reveal_still_focuses_one_overlay_when_the_cursor_is_unknown() {
        let manager = CaptureManager::new();
        let (left, right) = two_monitor_session(&manager);
        // Wayland 下拿不到光标位置时必须有人接键盘，否则 Esc 取消都用不了。
        assert!(manager.reveal(&left, None).unwrap().take_focus);
        assert!(!manager.reveal(&right, None).unwrap().take_focus);
    }

    #[test]
    fn reveal_rejects_labels_outside_the_current_session() {
        let manager = CaptureManager::new();
        assert_eq!(
            manager
                .reveal("capture-overlay-none-1", None)
                .unwrap_err()
                .code(),
            "session_missing"
        );
        let (left, _) = two_monitor_session(&manager);
        assert_eq!(
            manager
                .reveal("capture-overlay-other-1", None)
                .unwrap_err()
                .code(),
            "overlay_not_in_session"
        );
        assert!(manager.reveal(&left, None).is_ok());
    }
}
