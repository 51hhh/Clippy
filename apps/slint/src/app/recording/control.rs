use super::{send, Control, Message};
use crate::RecordingControlWindow;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use anyhow::Context;
use anyhow::Result;
#[cfg(target_os = "macos")]
use clippy_platform::capture::MacControlWindow;
use clippy_platform::{
    capture::{CaptureSource, Region},
    recording_control::{
        plan_control_window, ControlSize, ControlWindowPlan, PhysicalRect,
        WindowExclusionCapability,
    },
};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use slint::winit_030::winit::raw_window_handle::HasWindowHandle;
use slint::winit_030::{
    winit::{event::WindowEvent, window::WindowLevel},
    EventResult, WinitWindowAccessor,
};
use slint::{CloseRequestResponse, ComponentHandle, Timer, TimerMode};
#[cfg(target_os = "macos")]
use slint::{LogicalPosition, LogicalSize};
#[cfg(not(target_os = "macos"))]
use slint::{PhysicalPosition, PhysicalSize};
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

#[cfg(target_os = "macos")]
type ControlWindowId = MacControlWindow;
#[cfg(not(target_os = "macos"))]
type ControlWindowId = slint::winit_030::winit::window::WindowId;

fn exclusion_capability() -> Option<WindowExclusionCapability> {
    #[cfg(target_os = "windows")]
    {
        Some(clippy_platform::recording_control::windows_exclusion())
    }
    #[cfg(target_os = "linux")]
    {
        (!clippy_platform::session::is_wayland()).then_some(WindowExclusionCapability::GeometryOnly)
    }
    #[cfg(target_os = "macos")]
    {
        MacControlWindow::supported().then_some(WindowExclusionCapability::Native)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
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
    selection: PhysicalRect,
    capability: WindowExclusionCapability,
    prepared_window: Cell<Option<ControlWindowId>>,
    armed: Rc<Cell<bool>>,
    elapsed: Rc<Cell<Duration>>,
    since: Rc<Cell<Option<Instant>>>,
    timer: Timer,
}

impl View {
    pub(super) fn open(region: &Region, size: (u32, u32)) -> Result<Option<Self>> {
        let Some(capability) = exclusion_capability() else {
            return Ok(None);
        };
        let geometry = region.geometry;
        let selection = if cfg!(target_os = "macos") {
            let [left, top, right, bottom] = clippy_platform::capture::pixel_rect(
                geometry.width,
                geometry.height,
                region.bounds,
            )?;
            PhysicalRect {
                x: geometry.x + left as i32,
                y: geometry.y + top as i32,
                width: right - left,
                height: bottom - top,
            }
        } else {
            recorded_rectangle(region, size)
        };
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
        #[cfg(target_os = "macos")]
        {
            window
                .window()
                .set_position(LogicalPosition::new(position.x as f32, position.y as f32));
            window.window().set_size(LogicalSize::new(
                position.width as f32,
                position.height as f32,
            ));
        }
        #[cfg(not(target_os = "macos"))]
        {
            window
                .window()
                .set_position(PhysicalPosition::new(position.x, position.y));
            window
                .window()
                .set_size(PhysicalSize::new(position.width, position.height));
        }
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
            window.show()?;
        }
        Ok(Some(Self {
            window,
            selection,
            capability,
            prepared_window: Cell::default(),
            armed,
            elapsed: Rc::default(),
            since: Rc::default(),
            timer: Timer::default(),
        }))
    }

    pub(super) fn schedule_begin(&self, begin: impl FnOnce() + 'static) -> Result<()> {
        if self.capability == WindowExclusionCapability::GeometryOnly {
            // 等窗口管理器落实坐标，再检查实际摆位。
            Timer::single_shot(Duration::from_millis(90), begin);
        } else {
            let window = self.window.clone_strong();
            slint::spawn_local(async move {
                // Winit 在下一轮创建隐藏窗口，此时才能有可用的原生句柄。
                if let Err(error) = window.window().winit_window().await {
                    eprintln!("Recording control native window: {error}");
                }
                begin();
            })?;
        }
        Ok(())
    }

    pub(super) fn arm(&self, _source: &mut CaptureSource) -> bool {
        self.window.window().with_winit_window(|native| {
            native.set_window_level(WindowLevel::AlwaysOnTop);
            native.set_resizable(false);
        });
        let ready = match self.capability {
            WindowExclusionCapability::GeometryOnly => safe(self.window.window(), self.selection),
            WindowExclusionCapability::Native => {
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                match self.prepare_native_exclusion(_source) {
                    Ok(id) => {
                        self.prepared_window.set(Some(id));
                        true
                    }
                    Err(error) => {
                        eprintln!("Recording control capture exclusion: {error:#}");
                        false
                    }
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                unreachable!("当前平台只允许按选区摆放浮窗");
            }
        };
        self.armed.set(ready);
        ready
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn prepare_native_exclusion(&self, _source: &mut CaptureSource) -> Result<ControlWindowId> {
        self.window
            .window()
            .with_winit_window(|native| {
                let handle = native.window_handle()?;
                #[cfg(target_os = "windows")]
                {
                    clippy_platform::recording_control::exclude_windows_control_from_capture(
                        handle,
                    )?;
                    Ok(native.id())
                }
                #[cfg(target_os = "macos")]
                {
                    let window = MacControlWindow::from_handle(handle)?;
                    _source.exclude_recording_control(window);
                    Ok(window)
                }
            })
            .context("Cannot access the native recording control")?
    }

    fn current_window_id(&self) -> Option<ControlWindowId> {
        self.window
            .window()
            .with_winit_window(|native| {
                #[cfg(target_os = "macos")]
                {
                    MacControlWindow::from_handle(native.window_handle().ok()?).ok()
                }
                #[cfg(not(target_os = "macos"))]
                Some(native.id())
            })
            .flatten()
    }

    pub(super) fn start(&self) -> Result<()> {
        self.window.set_started(true);
        if self.capability == WindowExclusionCapability::Native {
            anyhow::ensure!(
                self.prepared_window
                    .get()
                    .is_some_and(|id| self.current_window_id() == Some(id)),
                "The recording control window changed before capture started"
            );
            self.window.show()?;
        }
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
        Ok(())
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
