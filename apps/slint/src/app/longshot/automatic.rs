use crate::LongshotDirection;
use anyhow::{ensure, Result};
use clippy_capture::{
    longshot::{overlap::ScrollDirection, LongshotSession},
    CaptureError,
};
use clippy_platform::scroll::{Direction, Target};
use image::RgbaImage;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const STEP_TIMEOUT: Duration = Duration::from_millis(500);
const SAMPLE_PERIOD: Duration = Duration::from_millis(35);
const STABLE_PERIOD: Duration = Duration::from_millis(70);

#[derive(Clone)]
pub(super) struct Permit(Arc<InputGate>);

struct InputGate {
    active: AtomicBool,
    mutation: Mutex<()>,
}

impl Permit {
    pub fn new() -> Self {
        Self(Arc::new(InputGate {
            active: AtomicBool::new(true),
            mutation: Mutex::new(()),
        }))
    }

    pub fn revoke(&self) {
        self.0.active.store(false, Ordering::Release);
        let _mutation = self.0.mutation.lock().unwrap();
    }

    fn check(&self) -> Result<()> {
        ensure!(
            self.0.active.load(Ordering::Acquire),
            "Automatic scrolling stopped"
        );
        Ok(())
    }

    fn input(&self, operation: impl FnOnce() -> Result<()>) -> Result<()> {
        // 撤销后只结算已经开始的输入，抓帧和拼接不占用许可。
        let _mutation = self.0.mutation.lock().unwrap();
        self.check()?;
        operation()
    }
}

pub(super) struct Request {
    pub direction: LongshotDirection,
    pub control_window: u32,
    pub permit: Permit,
}

impl Request {
    pub fn run(
        self,
        target: &mut dyn Target,
        session: &mut LongshotSession,
        read: &mut dyn FnMut() -> Result<RgbaImage>,
    ) -> Result<&'static str> {
        let result = self.drive(target, session, read);
        let _mutation = self.permit.0.mutation.lock().unwrap();
        let restore = target.finish(self.permit.0.active.load(Ordering::Acquire));
        result.and_then(|status| restore.map(|()| status))
    }

    fn drive(
        &self,
        target: &mut dyn Target,
        session: &mut LongshotSession,
        read: &mut dyn FnMut() -> Result<RgbaImage>,
    ) -> Result<&'static str> {
        let deadline = Instant::now() + STEP_TIMEOUT;
        while !target.control_is_hidden(self.control_window)? {
            self.permit.check()?;
            ensure!(
                Instant::now() < deadline,
                "Scrolling controls are still visible"
            );
            thread::sleep(SAMPLE_PERIOD);
        }
        self.permit.input(|| target.position_pointer())?;
        let (native_direction, expected_direction) = match self.direction {
            LongshotDirection::Down => (Direction::Down, ScrollDirection::Down),
            LongshotDirection::Up => (Direction::Up, ScrollDirection::Up),
            LongshotDirection::Left => (Direction::Left, ScrollDirection::Left),
            LongshotDirection::Right => (Direction::Right, ScrollDirection::Right),
        };
        let mut first_view = Some(self.read_view(target, read)?);
        loop {
            self.permit.input(|| target.step(native_direction))?;
            let previous = first_view
                .as_ref()
                .unwrap_or_else(|| session.current_frame());
            let frame = self.settled_frame(target, read, previous)?;
            drop(first_view.take());
            let Some(frame) = frame else {
                return Ok("Reached the end of the page");
            };
            let result = match session.append_after_scroll(frame, expected_direction) {
                Ok(result) => result,
                Err(error) => {
                    eprintln!("Could not extend the scrolling capture: {error}");
                    return Ok(match error {
                        CaptureError::LongshotEstimateUnexpectedDirection => {
                            "The page moved in a different direction"
                        }
                        CaptureError::LongshotEstimateNoExtension => {
                            "This part is already captured"
                        }
                        CaptureError::LongshotFrameLimit | CaptureError::LongshotResourceLimit => {
                            "Reached the scrolling capture size limit"
                        }
                        _ => "Could not align the next frame",
                    });
                }
            };
            if !result.committed {
                return Ok("This part is already captured");
            }
        }
    }

    fn read_view(
        &self,
        target: &dyn Target,
        read: &mut dyn FnMut() -> Result<RgbaImage>,
    ) -> Result<RgbaImage> {
        self.permit.check()?;
        target.check()?;
        let frame = read()?;
        target.check()?;
        self.permit.check()?;
        Ok(frame)
    }

    fn settled_frame(
        &self,
        target: &dyn Target,
        read: &mut dyn FnMut() -> Result<RgbaImage>,
        before: &RgbaImage,
    ) -> Result<Option<RgbaImage>> {
        let deadline = Instant::now() + STEP_TIMEOUT;
        let mut candidate = None;
        let mut updated = Instant::now();
        loop {
            let frame = self.read_view(target, read)?;
            let now = Instant::now();
            if candidate.as_ref() != Some(&frame) {
                candidate = (&frame != before).then_some(frame);
                updated = now;
            } else if now <= deadline && now.duration_since(updated) >= STABLE_PERIOD {
                return Ok(candidate);
            }
            if now >= deadline {
                ensure!(candidate.is_none(), "New content did not settle");
                return Ok(None);
            }
            thread::sleep(SAMPLE_PERIOD.min(deadline - now));
        }
    }
}
