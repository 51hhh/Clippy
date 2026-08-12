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
    pub overlay_labels: Vec<String>,
    pub restore_labels: Vec<String>,
    frames: Vec<CapturedMonitorFrame>,
    windows: HashMap<u32, Vec<WindowCandidate>>,
}

impl CaptureManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn begin(
        &self,
        frames: Vec<CapturedMonitorFrame>,
        restore_labels: Vec<String>,
    ) -> Result<Vec<OverlaySpec>, String> {
        if frames.is_empty() {
            return Err("截图没有可用显示器帧".to_string());
        }
        let mut current = self.session.lock().map_err(|error| error.to_string())?;
        if current.is_some() {
            return Err("已有截图会话正在进行".to_string());
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
            overlay_labels: specs.iter().map(|spec| spec.label.clone()).collect(),
            restore_labels,
            windows,
        });
        Ok(specs)
    }

    pub(super) fn payload(&self, label: &str) -> Result<CaptureOverlayPayload, String> {
        let current = self.session.lock().map_err(|error| error.to_string())?;
        let session = current
            .as_ref()
            .ok_or_else(|| "截图会话不存在".to_string())?;
        let index = session
            .overlay_labels
            .iter()
            .position(|candidate| candidate == label)
            .ok_or_else(|| "覆盖层不属于当前截图会话".to_string())?;
        let frame = session
            .frames
            .get(index)
            .ok_or_else(|| "覆盖层帧不存在".to_string())?;
        let png = crate::screenshot::encode_png(&frame.rgba, frame.pixel_width, frame.pixel_height)
            .map_err(|error| error.to_string())?;
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

    pub(super) fn crop(&self, selection: &CaptureSelection) -> Result<Vec<u8>, String> {
        let current = self.session.lock().map_err(|error| error.to_string())?;
        let session = current
            .as_ref()
            .ok_or_else(|| "截图会话不存在".to_string())?;
        if session.id != selection.session_id {
            return Err("截图会话已经更新，请重新选择".to_string());
        }
        let frame = session
            .frames
            .iter()
            .find(|frame| frame.monitor_id == selection.monitor_id)
            .ok_or_else(|| "选择区域不属于当前显示器".to_string())?;
        crop_frame(frame, selection)
    }

    pub(super) fn finish(&self, session_id: &str) -> Result<CaptureSession, String> {
        let mut current = self.session.lock().map_err(|error| error.to_string())?;
        let session = current.take().ok_or_else(|| "截图会话不存在".to_string())?;
        if session.id != session_id {
            *current = Some(session);
            return Err("截图会话已经更新".to_string());
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
            .is_some_and(|session| session.overlay_labels.iter().any(|item| item == label))
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
) -> Result<Vec<u8>, String> {
    for value in [selection.x, selection.y, selection.width, selection.height] {
        if !value.is_finite() {
            return Err("截图选择区域包含无效数值".to_string());
        }
    }
    if selection.width < 2.0 || selection.height < 2.0 {
        return Err("截图选择区域太小".to_string());
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
        return Err("截图选择区域为空".to_string());
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
            .ok_or_else(|| "截图帧裁剪越界".to_string())?;
        rgba.extend_from_slice(source);
    }
    crate::screenshot::encode_png(&rgba, width, height).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
            overlay_labels: vec![label.clone()],
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
    fn failed_crop_can_finish_its_session() {
        let manager = CaptureManager::new();
        let label = "capture-overlay-session-1-7".to_string();
        *manager.session.lock().unwrap() = Some(CaptureSession {
            id: "session-1".to_string(),
            overlay_labels: vec![label.clone()],
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

        assert_eq!(manager.crop(&selection).unwrap_err(), "截图选择区域太小");
        let session = manager.finish(&selection.session_id).unwrap();
        assert_eq!(session.overlay_labels, vec![label.clone()]);
        assert_eq!(session.restore_labels, vec!["main"]);
        assert!(manager.payload(&label).is_err());
    }

    #[test]
    fn finish_mismatch_preserves_newer_session() {
        let manager = CaptureManager::new();
        let label = "capture-overlay-session-2-7".to_string();
        *manager.session.lock().unwrap() = Some(CaptureSession {
            id: "session-2".to_string(),
            overlay_labels: vec![label.clone()],
            restore_labels: Vec::new(),
            frames: vec![frame(1.0)],
            windows: HashMap::new(),
        });

        assert_eq!(
            manager.finish("session-1").err().unwrap(),
            "截图会话已经更新"
        );
        assert!(manager.payload(&label).is_ok());
        assert_eq!(manager.finish("session-2").unwrap().id, "session-2");
    }
}
