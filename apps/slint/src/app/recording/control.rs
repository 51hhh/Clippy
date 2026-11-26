use super::{send, Control, Message};
use crate::RecordingControlWindow;
use anyhow::Result;
use clippy_platform::{
    capture::Region,
    recording_control::{
        plan_control_window, ControlSize, ControlWindowPlan, PhysicalRect,
        WindowExclusionCapability,
    },
};
use slint::winit_030::{
    winit::{event::WindowEvent, window::WindowLevel},
    EventResult, WinitWindowAccessor,
};
use slint::{
    CloseRequestResponse, ComponentHandle, PhysicalPosition, PhysicalSize, Timer, TimerMode,
};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

const CONTROL_SIZE: ControlSize = ControlSize {
    width: 280,
    height: 56,
};
const CONTROL_MARGIN: u32 = 12;

fn exclusion_capability() -> Option<WindowExclusionCapability> {
    #[cfg(target_os = "windows")]
    {
        Some(clippy_platform::recording_control::windows_exclusion())
    }
    #[cfg(target_os = "linux")]
    {
        (!clippy_platform::session::is_wayland()).then_some(WindowExclusionCapability::GeometryOnly)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

pub(super) fn recorded_rectangle(region: &Region, size: (u32, u32)) -> PhysicalRect {
    let geometry = region.geometry;
    PhysicalRect {
        x: geometry.x + (region.bounds[0] * geometry.width as f32).floor() as i32,
        y: geometry.y + (region.bounds[1] * geometry.height as f32).floor() as i32,
        width: size.0,
        height: size.1,
    }
}

pub(super) struct View {
    window: RecordingControlWindow,
    safety: CaptureSafety,
    armed: Rc<Cell<bool>>,
    elapsed: Rc<Cell<Duration>>,
    since: Rc<Cell<Option<Instant>>>,
    timer: Timer,
}

enum CaptureSafety {
    OutsideSelection(PhysicalRect),
    #[cfg(target_os = "windows")]
    NativeExclusion(bool),
}

impl View {
    pub(super) fn open(region: &Region, size: (u32, u32)) -> Result<Option<Self>> {
        let Some(capability) = exclusion_capability() else {
            return Ok(None);
        };
        let selection = recorded_rectangle(region, size);
        let geometry = region.geometry;
        let monitor = PhysicalRect {
            x: geometry.x,
            y: geometry.y,
            width: geometry.width,
            height: geometry.height,
        };
        let ControlWindowPlan::Visible(position) = plan_control_window(
            selection,
            &[monitor],
            CONTROL_SIZE,
            CONTROL_MARGIN,
            capability,
        ) else {
            return Ok(None);
        };
        let window = RecordingControlWindow::new()?;
        window
            .window()
            .set_position(PhysicalPosition::new(position.x, position.y));
        window
            .window()
            .set_size(PhysicalSize::new(position.width, position.height));
        window.on_pause(|| send(Message::Control(Control::PauseResume)));
        window.on_stop(|| send(Message::Control(Control::Stop)));
        window.window().on_close_requested(|| {
            send(Message::HideControl);
            CloseRequestResponse::HideWindow
        });
        let armed = Rc::new(Cell::new(false));
        if capability == WindowExclusionCapability::GeometryOnly {
            let armed = armed.clone();
            window.window().on_winit_window_event(move |host, event| {
                if armed.get()
                    && matches!(
                        event,
                        WindowEvent::Moved(_)
                            | WindowEvent::Resized(_)
                            | WindowEvent::ScaleFactorChanged { .. }
                    )
                    && !safe(host, selection)
                {
                    send(Message::HideControl);
                }
                EventResult::Propagate
            });
        }
        window.show()?;
        window.window().with_winit_window(|native| {
            native.set_window_level(WindowLevel::AlwaysOnTop);
            native.set_resizable(false);
        });
        let safety = CaptureSafety::OutsideSelection(selection);
        #[cfg(target_os = "windows")]
        let safety = if capability == WindowExclusionCapability::Native {
            use slint::winit_030::winit::raw_window_handle::HasWindowHandle;

            let exclusion = window
                .window()
                .with_winit_window(|native| -> Result<()> {
                    let handle = native.window_handle()?;
                    clippy_platform::recording_control::exclude_windows_control_from_capture(
                        handle,
                    )?;
                    Ok(())
                })
                .unwrap_or_else(|| {
                    Err(anyhow::anyhow!(
                        "Cannot access the Windows recording control"
                    ))
                });
            if let Err(error) = &exclusion {
                eprintln!("Recording control capture exclusion: {error:#}");
                // 保留启动阶段的等待，让 WGC 创建前先完成撤窗。
                let _ = window.hide();
            }
            CaptureSafety::NativeExclusion(exclusion.is_ok())
        } else {
            safety
        };
        Ok(Some(Self {
            window,
            safety,
            armed,
            elapsed: Rc::default(),
            since: Rc::default(),
            timer: Timer::default(),
        }))
    }

    pub(super) fn arm(&self) -> bool {
        let ready = match self.safety {
            CaptureSafety::OutsideSelection(selection) => safe(self.window.window(), selection),
            #[cfg(target_os = "windows")]
            CaptureSafety::NativeExclusion(excluded) => excluded,
        };
        self.armed.set(ready);
        ready
    }

    pub(super) fn start(&self) {
        self.window.set_started(true);
        self.since.set(Some(Instant::now()));
        let weak = self.window.as_weak();
        let elapsed = self.elapsed.clone();
        let since = self.since.clone();
        self.timer
            .start(TimerMode::Repeated, Duration::from_millis(250), move || {
                if let Some(window) = weak.upgrade() {
                    let seconds = (elapsed.get()
                        + since.get().map_or(Duration::ZERO, |start| start.elapsed()))
                    .as_secs();
                    window.set_elapsed(format!("{:02}:{:02}", seconds / 60, seconds % 60).into());
                }
            });
    }

    pub(super) fn sync(&self, paused: bool, pending: bool, finishing: bool) {
        if paused || finishing {
            if let Some(start) = self.since.take() {
                self.elapsed.set(self.elapsed.get() + start.elapsed());
            }
        } else if self.window.get_started() && self.since.get().is_none() {
            self.since.set(Some(Instant::now()));
        }
        self.window.set_paused(paused);
        self.window.set_pending(pending);
        self.window.set_finishing(finishing);
    }
}

impl Drop for View {
    fn drop(&mut self) {
        let _ = self.window.hide();
    }
}

fn safe(window: &slint::Window, selection: PhysicalRect) -> bool {
    window
        .with_winit_window(|native| {
            let Ok(position) = native.outer_position() else {
                return false;
            };
            let size = native.outer_size();
            let margin = i64::from(CONTROL_MARGIN);
            let (x, y) = (i64::from(position.x), i64::from(position.y));
            let (left, top) = (i64::from(selection.x), i64::from(selection.y));
            x + i64::from(size.width) + margin <= left
                || y + i64::from(size.height) + margin <= top
                || x >= left + i64::from(selection.width) + margin
                || y >= top + i64::from(selection.height) + margin
        })
        .unwrap_or(false)
}
