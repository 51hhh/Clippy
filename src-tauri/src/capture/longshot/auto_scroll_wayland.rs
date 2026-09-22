//! Wayland XDG RemoteDesktop + ScreenCast 自动滚动边界。
//!
//! Portal 会话只属于一个长截图代次。用户明确选择冻结选区所在显示器后，后端核对 stream
//! 几何，再把可信的选区中心换算为 stream 内坐标并发送固定离散滚轮事件。所有 ashpd 对象
//! 都留在专用线程和同一个 Tokio runtime 内，避免在 Tauri runtime 上嵌套 `block_on`。

use super::auto_scroll::LongshotAutoDirection;
use super::CaptureError;
use ashpd::desktop::remote_desktop::{
    Axis, DeviceType, NotifyPointerAxisDiscreteOptions, NotifyPointerMotionAbsoluteOptions,
    RemoteDesktop, SelectDevicesOptions,
};
use ashpd::desktop::screencast::{
    CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream as PortalStream,
};
use ashpd::desktop::Session;
use ashpd::WindowIdentifier;
use image::RgbaImage;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::time::Instant;

const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(120);
const PORTAL_CALL_TIMEOUT: Duration = Duration::from_secs(5);
const PORTAL_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const POINTER_SETTLE: Duration = Duration::from_millis(45);
const DRIVER_RESPONSE_MARGIN: Duration = Duration::from_secs(1);
const SCROLL_STEPS: i32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WaylandMonitorIdentity {
    pub logical_x: i32,
    pub logical_y: i32,
    pub logical_width: u32,
    pub logical_height: u32,
    pub monitor_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PortalPointerTarget {
    stream_x: f64,
    stream_y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PortalRequestPlan {
    pointer: bool,
    monitor: bool,
    multiple: bool,
    persistent: bool,
}

impl Default for PortalRequestPlan {
    fn default() -> Self {
        Self {
            pointer: true,
            monitor: true,
            multiple: false,
            persistent: false,
        }
    }
}

enum PortalCommand {
    Scroll {
        direction: LongshotAutoDirection,
        response: SyncSender<Result<(), CaptureError>>,
    },
    Close,
}

struct PortalDriver {
    commands: SyncSender<PortalCommand>,
    cancellation: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl PortalDriver {
    fn authorize(
        monitor: WaylandMonitorIdentity,
        pointer: PortalPointerTarget,
        parent: WindowIdentifier,
        cancellation: Arc<AtomicBool>,
    ) -> Result<Self, CaptureError> {
        let worker_cancellation = Arc::clone(&cancellation);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (commands, command_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("clippy-longshot-wayland-input".to_string())
            .spawn(move || {
                let result = run_portal_worker(
                    monitor,
                    pointer,
                    parent,
                    worker_cancellation,
                    command_rx,
                    ready_tx,
                );
                if let Err(error) = result {
                    log::warn!("Wayland 长截图 Portal worker 结束: {error}");
                }
            })
            .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                commands,
                cancellation,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(error) => {
                let _ = thread.join();
                Err(CaptureError::LongshotAutoInput(error.to_string()))
            }
        }
    }

    fn scroll(&self, direction: LongshotAutoDirection) -> Result<(), CaptureError> {
        let (response, result) = mpsc::sync_channel(1);
        self.commands
            .send(PortalCommand::Scroll {
                direction,
                response,
            })
            .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
        result
            .recv_timeout(PORTAL_CALL_TIMEOUT + POINTER_SETTLE + DRIVER_RESPONSE_MARGIN)
            .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?
    }

    fn cancel(&self) {
        self.cancellation.store(true, Ordering::SeqCst);
        let _ = self.commands.try_send(PortalCommand::Close);
    }
}

impl Drop for PortalDriver {
    fn drop(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            if thread.thread().id() != thread::current().id() && thread.join().is_err() {
                log::warn!("回收 Wayland 长截图 Portal worker 时发生 panic");
            }
        }
    }
}

pub(super) struct WaylandAutoTarget {
    monitor: WaylandMonitorIdentity,
    pointer: PortalPointerTarget,
    visual: Mutex<VisualIdentity>,
    authorization_cancel: Arc<AtomicBool>,
    driver: Mutex<Option<PortalDriver>>,
}

struct VisualIdentity {
    current: [u8; 32],
    committed: Vec<[u8; 32]>,
}

impl std::fmt::Debug for WaylandAutoTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WaylandAutoTarget")
            .field("monitor", &self.monitor)
            .field("pointer", &self.pointer)
            .field("authorized", &self.is_authorized())
            .finish_non_exhaustive()
    }
}

impl WaylandAutoTarget {
    pub(super) fn new(
        monitor: WaylandMonitorIdentity,
        global_point: (i32, i32),
        first_frame: &RgbaImage,
    ) -> Result<Self, CaptureError> {
        Ok(Self {
            monitor,
            pointer: portal_pointer_target(monitor, global_point)?,
            visual: Mutex::new(VisualIdentity {
                current: pixel_identity(first_frame),
                committed: vec![pixel_identity(first_frame)],
            }),
            authorization_cancel: Arc::new(AtomicBool::new(false)),
            driver: Mutex::new(None),
        })
    }

    pub(super) fn authorize(&self, parent: WindowIdentifier) -> Result<(), CaptureError> {
        if self
            .driver
            .lock()
            .map_err(CaptureError::state_lock)?
            .is_some()
        {
            return Ok(());
        }
        self.authorization_cancel.store(false, Ordering::SeqCst);
        let authorized = PortalDriver::authorize(
            self.monitor,
            self.pointer,
            parent,
            Arc::clone(&self.authorization_cancel),
        )?;
        if self.authorization_cancel.load(Ordering::SeqCst) {
            drop(authorized);
            return Err(CaptureError::LongshotAutoUserInterrupted);
        }
        let mut driver = self.driver.lock().map_err(CaptureError::state_lock)?;
        if driver.is_none() {
            *driver = Some(authorized);
        }
        Ok(())
    }

    pub(super) fn is_authorized(&self) -> bool {
        self.driver
            .lock()
            .map(|driver| driver.is_some())
            .unwrap_or(false)
    }

    pub(super) fn verify_preflight(&self, frame: &RgbaImage) -> Result<(), CaptureError> {
        let expected = self.visual.lock().map_err(CaptureError::state_lock)?;
        if expected.current == pixel_identity(frame) {
            Ok(())
        } else {
            Err(CaptureError::LongshotAutoTargetLost)
        }
    }

    pub(super) fn commit_frame(&self, frame: &RgbaImage, committed: bool) {
        let identity = pixel_identity(frame);
        let update = |visual: &mut VisualIdentity| {
            visual.current = identity;
            if committed {
                visual.committed.push(identity);
            }
        };
        match self.visual.lock() {
            Ok(mut visual) => update(&mut visual),
            Err(poisoned) => update(&mut poisoned.into_inner()),
        }
    }

    pub(super) fn undo_frame(&self, previous_count: usize, next_count: usize) {
        if next_count >= previous_count {
            return;
        }
        let update = |visual: &mut VisualIdentity| {
            visual.committed.truncate(next_count.max(1));
            if let Some(identity) = visual.committed.last().copied() {
                visual.current = identity;
            }
        };
        match self.visual.lock() {
            Ok(mut visual) => update(&mut visual),
            Err(poisoned) => update(&mut poisoned.into_inner()),
        }
    }

    pub(super) fn scroll(&self, direction: LongshotAutoDirection) -> Result<(), CaptureError> {
        let mut driver = self.driver.lock().map_err(CaptureError::state_lock)?;
        let Some(active) = driver.as_ref() else {
            return Err(CaptureError::LongshotAutoPermissionRequired);
        };
        if let Err(error) = active.scroll(direction) {
            driver.take();
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn cancel(&self) {
        self.authorization_cancel.store(true, Ordering::SeqCst);
        if let Ok(driver) = self.driver.lock() {
            if let Some(driver) = driver.as_ref() {
                driver.cancel();
            }
        }
    }
}

fn run_portal_worker(
    monitor: WaylandMonitorIdentity,
    pointer: PortalPointerTarget,
    parent: WindowIdentifier,
    cancellation: Arc<AtomicBool>,
    commands: Receiver<PortalCommand>,
    ready: SyncSender<Result<(), CaptureError>>,
) -> Result<(), CaptureError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    let opened = runtime.block_on(open_portal(monitor, parent, Arc::clone(&cancellation)));
    let session = match opened {
        Ok(session) => {
            let _ = ready.send(Ok(()));
            session
        }
        Err(error) => {
            let code = error.code().to_string();
            let message = error.to_string();
            let _ = ready.send(Err(error));
            return Err(CaptureError::LongshotAutoInput(format!(
                "Portal 授权失败 ({code}): {message}"
            )));
        }
    };

    while !cancellation.load(Ordering::SeqCst) {
        match commands.recv_timeout(Duration::from_millis(50)) {
            Ok(PortalCommand::Scroll {
                direction,
                response,
            }) => {
                let result = runtime.block_on(send_scroll(
                    &session,
                    pointer,
                    direction,
                    Arc::clone(&cancellation),
                ));
                let failed = result.is_err();
                let _ = response.send(result);
                if failed {
                    break;
                }
            }
            Ok(PortalCommand::Close) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    runtime.block_on(close_portal(session));
    Ok(())
}

struct ActivePortalSession {
    proxy: RemoteDesktop,
    session: Session<RemoteDesktop>,
    stream_id: u32,
}

async fn open_portal(
    monitor: WaylandMonitorIdentity,
    parent: WindowIdentifier,
    cancellation: Arc<AtomicBool>,
) -> Result<ActivePortalSession, CaptureError> {
    let deadline = Instant::now() + AUTHORIZATION_TIMEOUT;
    let proxy = portal_setup_call(
        Arc::clone(&cancellation),
        deadline,
        "RemoteDesktop 初始化",
        RemoteDesktop::new(),
    )
    .await?;
    let screencast = portal_setup_call(
        Arc::clone(&cancellation),
        deadline,
        "ScreenCast 初始化",
        Screencast::new(),
    )
    .await?;
    let devices = portal_setup_call(
        Arc::clone(&cancellation),
        deadline,
        "读取 Pointer 能力",
        proxy.available_device_types(),
    )
    .await?;
    let sources = portal_setup_call(
        Arc::clone(&cancellation),
        deadline,
        "读取 Monitor 能力",
        screencast.available_source_types(),
    )
    .await?;
    if !devices.contains(DeviceType::Pointer) || !sources.contains(SourceType::Monitor) {
        return Err(CaptureError::LongshotAutoUnsupported);
    }

    let session = portal_setup_call(
        Arc::clone(&cancellation),
        deadline,
        "创建 RemoteDesktop session",
        proxy.create_session(Default::default()),
    )
    .await?;
    let result = tokio::select! {
        _ = wait_for_cancellation(Arc::clone(&cancellation)) => {
            Err(CaptureError::LongshotAutoUserInterrupted)
        }
        _ = tokio::time::sleep_until(deadline) => {
            Err(CaptureError::LongshotAutoInput("Wayland Portal 授权超时".to_string()))
        }
        result = async {
            let plan = PortalRequestPlan::default();
            debug_assert!(plan.pointer && plan.monitor && !plan.multiple && !plan.persistent);
            proxy
                .select_devices(
                    &session,
                    SelectDevicesOptions::default()
                        .set_devices(Some(DeviceType::Pointer.into())),
                )
                .await
                .and_then(|request| request.response())
                .map_err(|_| CaptureError::LongshotAutoPermissionRequired)?;
            screencast
                .select_sources(
                    &session,
                    SelectSourcesOptions::default()
                        .set_cursor_mode(CursorMode::Hidden)
                        .set_sources(Some(SourceType::Monitor.into()))
                        .set_multiple(false),
                )
                .await
                .and_then(|request| request.response())
                .map_err(|_| CaptureError::LongshotAutoPermissionRequired)?;
            let response = proxy
                .start(&session, Some(&parent), Default::default())
                .await
                .and_then(|request| request.response())
                .map_err(|_| CaptureError::LongshotAutoPermissionRequired)?;
            if !response.devices().contains(DeviceType::Pointer) {
                return Err(CaptureError::LongshotAutoPermissionRequired);
            }
            let streams = response.streams();
            if streams.len() != 1 {
                return Err(CaptureError::LongshotAutoTargetLost);
            }
            validate_portal_stream(&streams[0], monitor)?;
            Ok(streams[0].pipe_wire_node_id())
        } => result,
    };
    match result {
        Ok(stream_id) => Ok(ActivePortalSession {
            proxy,
            session,
            stream_id,
        }),
        Err(error) => {
            match tokio::time::timeout(PORTAL_CLOSE_TIMEOUT, session.close()).await {
                Ok(Ok(())) => {}
                Ok(Err(close_error)) => {
                    log::warn!("Wayland 长截图授权失败后关闭 Portal session 也失败: {close_error}");
                }
                Err(_) => log::warn!("Wayland 长截图授权失败后关闭 Portal session 超时"),
            }
            Err(error)
        }
    }
}

async fn portal_setup_call<T, E, F>(
    cancellation: Arc<AtomicBool>,
    authorization_deadline: Instant,
    operation: &str,
    future: F,
) -> Result<T, CaptureError>
where
    E: std::fmt::Display,
    F: std::future::Future<Output = Result<T, E>>,
{
    let call_deadline = std::cmp::min(authorization_deadline, Instant::now() + PORTAL_CALL_TIMEOUT);
    tokio::select! {
        _ = wait_for_cancellation(cancellation) => Err(CaptureError::LongshotAutoUserInterrupted),
        _ = tokio::time::sleep_until(call_deadline) => {
            Err(CaptureError::LongshotAutoInput(format!("{operation}超时")))
        }
        result = future => result
            .map_err(|error| CaptureError::LongshotAutoInput(format!("{operation}失败: {error}"))),
    }
}

async fn send_scroll(
    active: &ActivePortalSession,
    pointer: PortalPointerTarget,
    direction: LongshotAutoDirection,
    cancellation: Arc<AtomicBool>,
) -> Result<(), CaptureError> {
    let (axis, steps) = scroll_input(direction);
    tokio::select! {
        _ = wait_for_cancellation(cancellation) => Err(CaptureError::LongshotAutoUserInterrupted),
        _ = tokio::time::sleep(PORTAL_CALL_TIMEOUT) => {
            Err(CaptureError::LongshotAutoInput("Wayland Portal 输入超时".to_string()))
        }
        result = async {
            active.proxy
                .notify_pointer_motion_absolute(
                    &active.session,
                    active.stream_id,
                    pointer.stream_x,
                    pointer.stream_y,
                    NotifyPointerMotionAbsoluteOptions::default(),
                )
                .await
                .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
            tokio::time::sleep(POINTER_SETTLE).await;
            active.proxy
                .notify_pointer_axis_discrete(
                    &active.session,
                    axis,
                    steps,
                    NotifyPointerAxisDiscreteOptions::default(),
                )
                .await
                .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))
        } => result,
    }
}

async fn close_portal(active: ActivePortalSession) {
    match tokio::time::timeout(PORTAL_CLOSE_TIMEOUT, active.session.close()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => log::warn!("关闭 Wayland 长截图 Portal session 失败: {error}"),
        Err(_) => log::warn!("关闭 Wayland 长截图 Portal session 超时"),
    }
}

async fn wait_for_cancellation(cancellation: Arc<AtomicBool>) {
    while !cancellation.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn validate_portal_stream(
    stream: &PortalStream,
    monitor: WaylandMonitorIdentity,
) -> Result<(), CaptureError> {
    validate_portal_identity(
        stream.source_type(),
        stream.position(),
        stream.size(),
        monitor,
    )
}

fn validate_portal_identity(
    source_type: Option<SourceType>,
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
    monitor: WaylandMonitorIdentity,
) -> Result<(), CaptureError> {
    if source_type.is_some_and(|value| value != SourceType::Monitor) {
        return Err(CaptureError::LongshotAutoTargetLost);
    }
    let width =
        i32::try_from(monitor.logical_width).map_err(|_| CaptureError::LongshotAutoTargetLost)?;
    let height =
        i32::try_from(monitor.logical_height).map_err(|_| CaptureError::LongshotAutoTargetLost)?;
    match (position, size) {
        (Some(position), Some(size))
            if position == (monitor.logical_x, monitor.logical_y) && size == (width, height) =>
        {
            Ok(())
        }
        (None, None) if monitor.monitor_count == 1 => Ok(()),
        _ => Err(CaptureError::LongshotAutoTargetLost),
    }
}

fn portal_pointer_target(
    monitor: WaylandMonitorIdentity,
    global_point: (i32, i32),
) -> Result<PortalPointerTarget, CaptureError> {
    let x = i64::from(global_point.0) - i64::from(monitor.logical_x);
    let y = i64::from(global_point.1) - i64::from(monitor.logical_y);
    if x < 0
        || y < 0
        || x >= i64::from(monitor.logical_width)
        || y >= i64::from(monitor.logical_height)
    {
        return Err(CaptureError::LongshotFrameInvalid);
    }
    Ok(PortalPointerTarget {
        stream_x: x as f64,
        stream_y: y as f64,
    })
}

fn scroll_input(direction: LongshotAutoDirection) -> (Axis, i32) {
    match direction {
        LongshotAutoDirection::Down => (Axis::Vertical, SCROLL_STEPS),
        LongshotAutoDirection::Up => (Axis::Vertical, -SCROLL_STEPS),
        LongshotAutoDirection::Right => (Axis::Horizontal, SCROLL_STEPS),
        LongshotAutoDirection::Left => (Axis::Horizontal, -SCROLL_STEPS),
    }
}

fn pixel_identity(frame: &RgbaImage) -> [u8; 32] {
    Sha256::digest(frame.as_raw()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(count: usize) -> WaylandMonitorIdentity {
        WaylandMonitorIdentity {
            logical_x: -1920,
            logical_y: 120,
            logical_width: 1920,
            logical_height: 1080,
            monitor_count: count,
        }
    }

    #[test]
    fn portal_plan_is_pointer_only_single_monitor_and_non_persistent() {
        assert_eq!(
            PortalRequestPlan::default(),
            PortalRequestPlan {
                pointer: true,
                monitor: true,
                multiple: false,
                persistent: false,
            }
        );
    }

    #[test]
    fn portal_stream_must_match_the_frozen_monitor() {
        assert!(validate_portal_identity(
            Some(SourceType::Monitor),
            Some((-1920, 120)),
            Some((1920, 1080)),
            monitor(2),
        )
        .is_ok());
        assert_eq!(
            validate_portal_identity(
                Some(SourceType::Monitor),
                Some((0, 0)),
                Some((1920, 1080)),
                monitor(2),
            )
            .unwrap_err()
            .code(),
            "longshot_auto_target_lost"
        );
        assert!(validate_portal_identity(None, None, None, monitor(1)).is_ok());
        assert!(validate_portal_identity(None, None, None, monitor(2)).is_err());
    }

    #[test]
    fn trusted_global_point_is_converted_to_stream_coordinates() {
        assert_eq!(
            portal_pointer_target(monitor(2), (-960, 660)).unwrap(),
            PortalPointerTarget {
                stream_x: 960.0,
                stream_y: 540.0,
            }
        );
        assert!(portal_pointer_target(monitor(2), (0, 660)).is_err());
    }

    #[test]
    fn four_directions_map_to_fixed_discrete_axis_events() {
        assert_eq!(
            scroll_input(LongshotAutoDirection::Down),
            (Axis::Vertical, 5)
        );
        assert_eq!(
            scroll_input(LongshotAutoDirection::Up),
            (Axis::Vertical, -5)
        );
        assert_eq!(
            scroll_input(LongshotAutoDirection::Right),
            (Axis::Horizontal, 5)
        );
        assert_eq!(
            scroll_input(LongshotAutoDirection::Left),
            (Axis::Horizontal, -5)
        );
    }

    #[test]
    fn visual_identity_changes_only_after_explicit_commit() {
        let first = RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255]));
        let second = RgbaImage::from_pixel(4, 4, image::Rgba([4, 5, 6, 255]));
        let target = WaylandAutoTarget::new(monitor(1), (-1918, 122), &first).unwrap();
        assert!(target.verify_preflight(&first).is_ok());
        assert_eq!(
            target.verify_preflight(&second).unwrap_err().code(),
            "longshot_auto_target_lost"
        );
        target.commit_frame(&second, true);
        assert!(target.verify_preflight(&second).is_ok());
        target.undo_frame(2, 1);
        assert!(target.verify_preflight(&first).is_ok());
    }
}
