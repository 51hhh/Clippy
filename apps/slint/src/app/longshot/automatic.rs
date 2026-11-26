use crate::LongshotDirection;
#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::{ensure, Result};
use clippy_capture::{
    longshot::{overlap::ScrollDirection, LongshotSession},
    CaptureError,
};
#[cfg(target_os = "linux")]
use clippy_platform::capture::wayland_scroll::AuthorizedScroll;
use clippy_platform::scroll::{ControlWindow, Direction, Target};
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
    pub permit: Permit,
}

enum Driver<'a> {
    Native {
        target: &'a mut dyn Target,
        read: &'a mut dyn FnMut() -> Result<RgbaImage>,
    },
    #[cfg(target_os = "linux")]
    Portal(&'a mut AuthorizedScroll),
}

impl Driver<'_> {
    fn read(&mut self) -> Result<RgbaImage> {
        match self {
            Self::Native { target, read } => {
                target.check()?;
                let frame = read()?;
                target.check()?;
                Ok(frame)
            }
            #[cfg(target_os = "linux")]
            Self::Portal(portal) => portal.read(),
        }
    }

    fn step(&mut self, permit: &Permit, direction: Direction, _before: &RgbaImage) -> Result<()> {
        match self {
            Self::Native { target, .. } => permit.input(|| target.step(direction)),
            #[cfg(target_os = "linux")]
            Self::Portal(portal) => {
                let verify = |portal: &mut AuthorizedScroll| {
                    ensure!(
                        portal.matches_view(_before)?,
                        "The scroll target changed. Add a frame manually to continue."
                    );
                    Ok(())
                };
                verify(portal)?;
                permit.input(|| portal.position_pointer())?;
                verify(portal)?;
                permit.input(|| portal.step(direction))
            }
        }
    }

    fn verify_frame_target(&self) -> Result<()> {
        match self {
            Self::Native { target, .. } => target.verify_frame_target(),
            #[cfg(target_os = "linux")]
            Self::Portal(portal) => portal.check(),
        }
    }
}

impl Request {
    pub fn run_native(
        self,
        control_window: ControlWindow,
        target: &mut dyn Target,
        session: &mut LongshotSession,
        read: &mut dyn FnMut() -> Result<RgbaImage>,
    ) -> Result<&'static str> {
        let result = self
            .prepare_native(control_window, target)
            .and_then(|()| self.drive(&mut Driver::Native { target, read }, session));
        let _mutation = self.permit.0.mutation.lock().unwrap();
        let restore = target.finish(self.permit.0.active.load(Ordering::Acquire));
        result.and_then(|status| restore.map(|()| status))
    }

    #[cfg(target_os = "linux")]
    pub fn run_portal(
        self,
        hidden: Instant,
        portal: &mut AuthorizedScroll,
        session: &mut LongshotSession,
    ) -> Result<&'static str> {
        self.permit.check()?;
        portal.refresh_after(hidden)?;
        self.drive(&mut Driver::Portal(portal), session)
    }

    fn prepare_native(&self, control_window: ControlWindow, target: &mut dyn Target) -> Result<()> {
        let deadline = Instant::now() + STEP_TIMEOUT;
        self.wait_for_native_state(
            deadline,
            || target.control_is_hidden(control_window),
            "Scrolling controls did not hide in time",
        )?;
        if !target.is_active() {
            self.permit.input(|| target.activate())?;
        }
        self.wait_for_native_state(
            deadline,
            || Ok(target.is_active()),
            "Scroll target did not become active",
        )?;
        self.permit.input(|| target.position_pointer())
    }

    fn drive(
        &self,
        target: &mut Driver<'_>,
        session: &mut LongshotSession,
    ) -> Result<&'static str> {
        let (native_direction, expected_direction) = match self.direction {
            LongshotDirection::Down => (Direction::Down, ScrollDirection::Down),
            LongshotDirection::Up => (Direction::Up, ScrollDirection::Up),
            LongshotDirection::Left => (Direction::Left, ScrollDirection::Left),
            LongshotDirection::Right => (Direction::Right, ScrollDirection::Right),
        };
        let mut first_view = match target {
            Driver::Native { .. } => Some(self.read_view(target)?),
            #[cfg(target_os = "linux")]
            Driver::Portal(_) => None,
        };
        loop {
            self.permit.check()?;
            let previous = match target {
                Driver::Native { .. } => first_view
                    .as_ref()
                    .unwrap_or_else(|| session.current_frame()),
                #[cfg(target_os = "linux")]
                Driver::Portal(_) => session.committed_view().context(
                    "Move to new content and add a frame manually before automatic scrolling",
                )?,
            };
            target.step(&self.permit, native_direction, previous)?;
            let frame = self.settled_frame(target, previous)?;
            drop(first_view.take());
            let Some(frame) = frame else {
                return Ok("Reached the end of the page");
            };
            target.verify_frame_target()?;
            self.permit.check()?;
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

    fn wait_for_native_state(
        &self,
        deadline: Instant,
        mut ready: impl FnMut() -> Result<bool>,
        timeout: &str,
    ) -> Result<()> {
        loop {
            self.permit.check()?;
            let ready = ready()?;
            let now = Instant::now();
            ensure!(now < deadline, "{timeout}");
            if ready {
                return Ok(());
            }
            thread::sleep(SAMPLE_PERIOD.min(deadline - now));
        }
    }

    fn read_view(&self, target: &mut Driver<'_>) -> Result<RgbaImage> {
        self.permit.check()?;
        let frame = target.read()?;
        self.permit.check()?;
        Ok(frame)
    }

    fn settled_frame(
        &self,
        target: &mut Driver<'_>,
        before: &RgbaImage,
    ) -> Result<Option<RgbaImage>> {
        let deadline = Instant::now() + STEP_TIMEOUT;
        let mut candidate = None;
        let mut updated = Instant::now();
        loop {
            let frame = self.read_view(target)?;
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
