use super::pixel_rect;
use anyhow::{anyhow, ensure, Context, Result};
use image::RgbaImage;
use std::{
    sync::{Condvar, Mutex},
    time::Duration,
};

pub(super) struct RegionLayout {
    pub points: [f64; 4],
    pub pixels: (u32, u32),
}

pub(super) fn region_layout(
    source: (u32, u32),
    logical: (f64, f64),
    bounds: [f32; 4],
) -> Result<RegionLayout> {
    ensure!(
        logical.0.is_finite() && logical.0 > 0.0 && logical.1.is_finite() && logical.1 > 0.0,
        "Invalid display size"
    );
    let [left, top, right, bottom] = pixel_rect(source.0, source.1, bounds)?;
    let pixels = (right - left, bottom - top);
    let x_scale = logical.0 / f64::from(source.0);
    let y_scale = logical.1 / f64::from(source.1);
    Ok(RegionLayout {
        points: [
            f64::from(left) * x_scale,
            f64::from(top) * y_scale,
            f64::from(pixels.0) * x_scale,
            f64::from(pixels.1) * y_scale,
        ],
        pixels,
    })
}

pub(super) fn bgra_span(width: u32, height: u32, stride: usize) -> Result<usize> {
    let row = width as usize * 4;
    ensure!(stride >= row, "Invalid ScreenCaptureKit row stride");
    stride
        .checked_mul((height - 1) as usize)
        .and_then(|bytes| bytes.checked_add(row))
        .filter(|bytes| *bytes <= isize::MAX as usize)
        .context("ScreenCaptureKit buffer size overflow")
}

pub(super) fn bgra_to_rgba(bytes: &[u8], (width, height): (u32, u32), stride: usize) -> RgbaImage {
    let row_bytes = width as usize * 4;
    let mut rgba = Vec::with_capacity(row_bytes * height as usize);
    for row in bytes.chunks(stride) {
        for bgra in row[..row_bytes].as_chunks::<4>().0 {
            rgba.extend_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
        }
    }
    RgbaImage::from_raw(width, height, rgba).expect("已经按选区逐行转换 BGRA")
}

struct FrameState<T> {
    sequence: u64,
    frame: Result<Option<T>, String>,
}

pub(super) struct ScreenFrames<T> {
    state: Mutex<FrameState<T>>,
    changed: Condvar,
}

impl<T: Clone> ScreenFrames<T> {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FrameState {
                sequence: 0,
                frame: Ok(None),
            }),
            changed: Condvar::new(),
        }
    }

    pub fn publish(&self, frame: Result<Option<T>, String>) {
        let previous = {
            let mut state = self.state.lock().unwrap();
            if state.frame.is_err() {
                return;
            }
            state.sequence += 1;
            std::mem::replace(&mut state.frame, frame)
        };
        drop(previous);
        self.changed.notify_one();
    }

    pub fn wait(&self, seen: u64, timeout: Duration) -> Result<(u64, T)> {
        let (state, _) = self
            .changed
            .wait_timeout_while(self.state.lock().unwrap(), timeout, |state| {
                matches!(&state.frame, Ok(None))
                    || (matches!(&state.frame, Ok(Some(_))) && state.sequence == seen)
            })
            .unwrap();
        Ok((state.sequence, Self::active_frame(&state)?.clone()))
    }

    pub fn check_active(&self) -> Result<()> {
        Self::active_frame(&self.state.lock().unwrap()).map(|_| ())
    }

    fn active_frame(state: &FrameState<T>) -> Result<&T> {
        state
            .frame
            .as_ref()
            .map_err(|message| anyhow!("{message}"))?
            .as_ref()
            .context("No active ScreenCaptureKit frame")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, thread};

    #[test]
    fn macos_region_projects_retina_and_fractional_pixels_from_top_left() {
        let backwards = [0.75, 0.75, 0.25, 0.25];
        let retina = region_layout((12, 20), (6.0, 10.0), backwards).unwrap();
        assert_eq!(retina.points, [1.5, 2.5, 3.0, 5.0]);
        assert_eq!(retina.pixels, (6, 10));
        let scaled = region_layout((12, 20), (8.0, 16.0), backwards).unwrap();
        assert_eq!(scaled.points, [2.0, 4.0, 4.0, 8.0]);
        assert_eq!(scaled.pixels, (6, 10));
        assert!(region_layout((12, 20), (0.0, 16.0), backwards).is_err());
    }

    #[test]
    fn macos_bgra_readback_skips_row_padding_and_preserves_alpha() {
        let bytes = [1, 2, 3, 4, 90, 91, 92, 93, 5, 6, 7, 8];
        assert_eq!(bgra_span(1, 2, 8).unwrap(), bytes.len());
        assert_eq!(
            bgra_to_rgba(&bytes, (1, 2), 8).into_raw(),
            [3, 2, 1, 4, 7, 6, 5, 8]
        );
        assert!(bgra_span(2, 2, 4).is_err());
        assert!(bgra_span(2, 2, usize::MAX).is_err());
    }

    #[test]
    fn macos_latest_replaces_surfaces_and_keeps_static_frame() {
        let frames = ScreenFrames::new();
        let previous = Arc::new(12);
        let newest = Arc::new(34);
        frames.publish(Ok(Some(previous.clone())));
        frames.publish(Ok(Some(newest.clone())));
        assert_eq!(Arc::strong_count(&previous), 1);
        let (sequence, sample) = frames.wait(0, Duration::ZERO).unwrap();
        assert!(Arc::ptr_eq(&sample, &newest));
        assert!(Arc::ptr_eq(
            &frames.wait(sequence, Duration::ZERO).unwrap().1,
            &newest
        ));
        assert!(frames.check_active().is_ok());
        frames.publish(Ok(None));
        assert!(frames.check_active().is_err());
        assert!(frames.wait(sequence, Duration::ZERO).is_err());
        assert_eq!(Arc::strong_count(&newest), 2);
    }

    #[test]
    fn macos_stopped_frames_wake_waiters_and_drop_pending_surface() {
        let frames = ScreenFrames::new();
        let sample = Arc::new(12);
        frames.publish(Ok(Some(sample.clone())));
        let sequence = frames.wait(0, Duration::ZERO).unwrap().0;
        thread::scope(|scope| {
            let wait = scope.spawn(|| frames.wait(sequence, Duration::from_secs(3)));
            frames.publish(Err("Display disconnected".into()));
            frames.publish(Ok(Some(sample.clone())));
            assert_eq!(
                wait.join().unwrap().unwrap_err().to_string(),
                "Display disconnected"
            );
        });
        assert_eq!(Arc::strong_count(&sample), 1);
    }
}
