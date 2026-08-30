use super::error::CaptureError;
use super::types::{CaptureOverlayPayload, CaptureSelection, OverlaySpec, WindowCandidate};
use super::window_probe::probe_windows;
use crate::screenshot::CapturedMonitorFrame;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::collections::HashMap;
use std::sync::Mutex;

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
    /// 已经有覆盖层拿到键盘焦点。没有它的话，光标不在任何覆盖层里（Wayland 下拿不到光标时）
    /// 就没人接 Esc，整个会话只能靠杀窗口退出。
    focus_assigned: bool,
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
    ) -> Result<Vec<OverlaySpec>, CaptureError> {
        if frames.is_empty() {
            return Err(CaptureError::NoMonitorFrames);
        }
        let mut current = self.session.lock().map_err(CaptureError::state_lock)?;
        if current.is_some() {
            return Err(CaptureError::SessionBusy);
        }
        let id = crate::image_io::unique_image_id();
        let windows = probe_windows(&frames);
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
            focus_assigned: false,
        });
        Ok(specs)
    }

    /// 覆盖层报告"首帧已经画好，可以显示了"。
    ///
    /// 覆盖层是隐藏建窗的：webview 加载 + 取 payload + 解 PNG 期间窗口一旦可见，
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
        Ok(RevealPlan { take_focus })
    }

    pub(super) fn payload(&self, label: &str) -> Result<CaptureOverlayPayload, CaptureError> {
        let current = self.session.lock().map_err(CaptureError::state_lock)?;
        let session = current.as_ref().ok_or(CaptureError::SessionMissing)?;
        let index = session
            .overlays
            .iter()
            .position(|spec| spec.label == label)
            .ok_or(CaptureError::OverlayNotInSession)?;
        let frame = session
            .frames
            .get(index)
            .ok_or(CaptureError::OverlayFrameMissing)?;
        let png = crate::screenshot::encode_png(&frame.rgba, frame.pixel_width, frame.pixel_height)
            .map_err(CaptureError::codec)?;
        Ok(CaptureOverlayPayload {
            session_id: session.id.clone(),
            monitor_id: frame.monitor_id,
            png_base64: STANDARD.encode(png),
            logical_width: frame.logical_width,
            logical_height: frame.logical_height,
            pixel_width: frame.pixel_width,
            pixel_height: frame.pixel_height,
            windows: session
                .windows
                .get(&frame.monitor_id)
                .cloned()
                .unwrap_or_default(),
        })
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
        // 提交动作已经不由后端配置决定：工具条恒定显示在选区旁边。
        assert!(json.get("commitAction").is_none());
    }

    #[test]
    fn failed_crop_can_finish_its_session() {
        let manager = CaptureManager::new();
        let label = "capture-overlay-session-1-7".to_string();
        *manager.session.lock().unwrap() = Some(CaptureSession {
            id: "session-1".to_string(),
            overlays: vec![overlay(&label)],
            focus_assigned: false,
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
