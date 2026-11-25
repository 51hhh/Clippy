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
    selection: PhysicalRect,
    armed: Rc<Cell<bool>>,
    elapsed: Rc<Cell<Duration>>,
    since: Rc<Cell<Option<Instant>>>,
    timer: Timer,
}

impl View {
    pub(super) fn open(region: &Region, size: (u32, u32)) -> Result<Option<Self>> {
        // 仅在采集和窗口管理器都使用同一套 X11 根窗口物理坐标时摆放浮窗。
        if !cfg!(target_os = "linux") || clippy_platform::session::is_wayland() {
            return Ok(None);
        }
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
            WindowExclusionCapability::GeometryOnly,
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
        window.window().on_winit_window_event({
            let armed = armed.clone();
            move |host, event| {
                if armed.get()
                    && matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_))
                    && !safe(host, selection)
                {
                    send(Message::HideControl);
                }
                EventResult::Propagate
            }
        });
        window.show()?;
        window.window().with_winit_window(|native| {
            native.set_window_level(WindowLevel::AlwaysOnTop);
            native.set_resizable(false);
        });
        Ok(Some(Self {
            window,
            selection,
            armed,
            elapsed: Rc::default(),
            since: Rc::default(),
            timer: Timer::default(),
        }))
    }

    pub(super) fn arm(&self) -> bool {
        let ready = safe(self.window.window(), self.selection);
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
