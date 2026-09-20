//! Wayland XDG ScreenCast Portal + PipeWire 区域录屏帧源。
//!
//! Portal 只能授权整块显示器，不能接收 Clippy 的任意矩形。用户授权后必须再次核对返回的 monitor
//! 逻辑几何，PipeWire 帧也必须恰好等于冻结帧的物理尺寸；随后才按可信 crop 无缩放裁剪。Portal、
//! PipeWire stream 与回调都留在采集 worker 及其专用 loop 线程，不借 XWayland 读取原生窗口。

use super::region::{crop_tight_rgba, validate_selection, RegionFrameError};
use super::RecordingSourceDescriptor;
use crate::capture::RecordingCaptureSpec;
use crate::pipewire_frame::{
    enum_format_pod, frame_from_buffer, init_pipewire, parse_video_format, PipeWireRgbaFrame,
};
use crate::recording::frame::{CapturedFrame, FrameError};
use crate::recording::worker::RecordingFrameSource;
use anyhow::Context;
use ashpd::desktop::screencast::{
    CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream as PortalStream,
};
use ashpd::desktop::{PersistMode, Session};
use pipewire as pw;
use pw::spa;
use spa::param::video::VideoInfoRaw;
use spa::pod::Pod;
use std::os::fd::OwnedFd;
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;

const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const FRAME_POLL_TIMEOUT: Duration = Duration::from_millis(50);
const PORTAL_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const PIPEWIRE_INIT_TIMEOUT: Duration = Duration::from_secs(5);
const PIPEWIRE_CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub(in crate::recording) enum WaylandFrameSourceError {
    #[error("当前桌面不是原生 Wayland 会话")]
    NotWayland,
    #[error("Wayland 显示器准备失败: {0}")]
    Monitor(String),
    #[error("Wayland 显示器物理几何已变化")]
    MonitorGeometryChanged,
    #[error("ScreenCast Portal 不支持显示器源")]
    MonitorSourceUnavailable,
    #[error("ScreenCast Portal 不支持把光标嵌入画面")]
    EmbeddedCursorUnavailable,
    #[error("ScreenCast Portal 初始化失败: {0}")]
    Portal(String),
    #[error("ScreenCast Portal 返回的不是冻结选区所在显示器")]
    PortalSourceMismatch,
    #[error("ScreenCast Portal 必须返回且只能返回一条显示器流")]
    PortalStreamCount,
    #[error("PipeWire 初始化失败: {0}")]
    PipeWireInitialize(String),
    #[error("PipeWire 帧流失败: {0}")]
    PipeWireStream(String),
    #[error("PipeWire 控制失败: {0}")]
    PipeWireControl(String),
    #[error("PipeWire 控制超过五秒未完成")]
    PipeWireControlTimeout,
    #[error("PipeWire 帧流已经关闭")]
    StreamClosed,
    #[error("PipeWire 等待首帧超时")]
    FirstFrameTimeout,
    #[error("Wayland 录屏帧桥接锁已损坏")]
    BridgePoisoned,
    #[error("Wayland 录屏帧序号耗尽")]
    SequenceExhausted,
    #[error("Wayland 录屏单调时间戳耗尽")]
    TimestampExhausted,
    #[error("Wayland 录屏物理坐标溢出")]
    CoordinateOverflow,
    #[error(transparent)]
    Region(#[from] RegionFrameError),
    #[error(transparent)]
    Frame(#[from] FrameError),
}

#[derive(Debug, Clone)]
pub(in crate::recording) struct WaylandPortalFrameSourcePlan {
    selection: RecordingCaptureSpec,
    expected: ExpectedPortalMonitor,
    descriptor: RecordingSourceDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedPortalMonitor {
    logical_x: i32,
    logical_y: i32,
    logical_width: u32,
    logical_height: u32,
    monitor_count: usize,
}

impl WaylandPortalFrameSourcePlan {
    pub fn prepare(selection: RecordingCaptureSpec) -> Result<Self, WaylandFrameSourceError> {
        if !crate::platform::is_wayland() {
            return Err(WaylandFrameSourceError::NotWayland);
        }
        // Portal 只提供整块显示器，进入 Clippy 前仍会产生完整 RGBA，因此沿用整屏 64 MiB 预算。
        validate_selection(selection)?;
        let monitor = crate::screenshot::wayland_recording_monitor(selection.monitor_id)
            .map_err(|error| WaylandFrameSourceError::Monitor(error.to_string()))?;
        if monitor.id != selection.monitor_id
            || monitor.pixel_width != selection.monitor_pixel_width
            || monitor.pixel_height != selection.monitor_pixel_height
        {
            return Err(WaylandFrameSourceError::MonitorGeometryChanged);
        }
        let physical_x = checked_physical_coordinate(
            monitor.logical_x,
            monitor.logical_width,
            monitor.pixel_width,
            selection.crop_left,
        )?;
        let physical_y = checked_physical_coordinate(
            monitor.logical_y,
            monitor.logical_height,
            monitor.pixel_height,
            selection.crop_top,
        )?;
        Ok(Self {
            selection,
            expected: ExpectedPortalMonitor {
                logical_x: monitor.logical_x,
                logical_y: monitor.logical_y,
                logical_width: monitor.logical_width,
                logical_height: monitor.logical_height,
                monitor_count: monitor.monitor_count,
            },
            descriptor: RecordingSourceDescriptor {
                source_id: format!("wayland-portal-{}", selection.monitor_id),
                physical_x,
                physical_y,
                width: selection.crop_width,
                height: selection.crop_height,
            },
        })
    }

    pub fn descriptor(&self) -> &RecordingSourceDescriptor {
        &self.descriptor
    }

    pub fn connect(self) -> Result<WaylandPortalRegionFrameSource, WaylandFrameSourceError> {
        WaylandPortalRegionFrameSource::connect(self)
    }
}

struct PortalSession {
    runtime: tokio::runtime::Runtime,
    session: Option<Session<Screencast>>,
}

impl Drop for PortalSession {
    fn drop(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        match self
            .runtime
            .block_on(tokio::time::timeout(PORTAL_CLOSE_TIMEOUT, session.close()))
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("关闭录屏 ScreenCast Portal 会话失败: {error}"),
            Err(_) => log::warn!("关闭录屏 ScreenCast Portal 会话超过五秒"),
        }
    }
}

async fn open_portal(
    expected: &ExpectedPortalMonitor,
) -> Result<(Session<Screencast>, u32, OwnedFd), WaylandFrameSourceError> {
    let proxy = Screencast::new()
        .await
        .map_err(|error| WaylandFrameSourceError::Portal(error.to_string()))?;
    let source_types = proxy
        .available_source_types()
        .await
        .map_err(|error| WaylandFrameSourceError::Portal(error.to_string()))?;
    if !source_types.contains(SourceType::Monitor) {
        return Err(WaylandFrameSourceError::MonitorSourceUnavailable);
    }
    let cursor_modes = proxy
        .available_cursor_modes()
        .await
        .map_err(|error| WaylandFrameSourceError::Portal(error.to_string()))?;
    if !cursor_modes.contains(CursorMode::Embedded) {
        return Err(WaylandFrameSourceError::EmbeddedCursorUnavailable);
    }
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|error| WaylandFrameSourceError::Portal(error.to_string()))?;
    let opened = async {
        proxy
            .select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_cursor_mode(CursorMode::Embedded)
                    .set_sources(Some(SourceType::Monitor.into()))
                    .set_multiple(false)
                    .set_restore_token(None)
                    .set_persist_mode(PersistMode::DoNot),
            )
            .await
            .and_then(|request| request.response())
            .map_err(|error| WaylandFrameSourceError::Portal(error.to_string()))?;
        let response = proxy
            .start(&session, None, Default::default())
            .await
            .and_then(|request| request.response())
            .map_err(|error| WaylandFrameSourceError::Portal(error.to_string()))?;
        let streams = response.streams();
        if streams.len() != 1 {
            return Err(WaylandFrameSourceError::PortalStreamCount);
        }
        validate_portal_stream(&streams[0], expected)?;
        let node_id = streams[0].pipe_wire_node_id();
        let remote = proxy
            .open_pipe_wire_remote(&session, Default::default())
            .await
            .map_err(|error| WaylandFrameSourceError::Portal(error.to_string()))?;
        Ok((node_id, remote))
    }
    .await;
    match opened {
        Ok((node_id, remote)) => Ok((session, node_id, remote)),
        Err(error) => {
            if let Err(close_error) = session.close().await {
                log::warn!("Portal 录屏准备失败后关闭会话也失败: {close_error}");
            }
            Err(error)
        }
    }
}

fn validate_portal_stream(
    stream: &PortalStream,
    expected: &ExpectedPortalMonitor,
) -> Result<(), WaylandFrameSourceError> {
    validate_portal_identity(
        stream.source_type(),
        stream.position(),
        stream.size(),
        expected,
    )
}

fn validate_portal_identity(
    source_type: Option<SourceType>,
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
    expected: &ExpectedPortalMonitor,
) -> Result<(), WaylandFrameSourceError> {
    if source_type.is_some_and(|value| value != SourceType::Monitor) {
        return Err(WaylandFrameSourceError::PortalSourceMismatch);
    }
    match (position, size) {
        (Some(position), Some(size))
            if position == (expected.logical_x, expected.logical_y)
                && size
                    == (
                        i32::try_from(expected.logical_width)
                            .map_err(|_| WaylandFrameSourceError::PortalSourceMismatch)?,
                        i32::try_from(expected.logical_height)
                            .map_err(|_| WaylandFrameSourceError::PortalSourceMismatch)?,
                    ) =>
        {
            Ok(())
        }
        (None, None) if expected.monitor_count == 1 => Ok(()),
        _ => Err(WaylandFrameSourceError::PortalSourceMismatch),
    }
}

struct StampedFrame {
    captured_at_ns: u64,
    frame: PipeWireRgbaFrame,
}

#[derive(Default)]
struct LatestFrame {
    frame: Option<StampedFrame>,
    error: Option<String>,
    closed: bool,
}

#[derive(Default)]
struct FrameBridge {
    latest: Mutex<LatestFrame>,
    changed: Condvar,
}

impl FrameBridge {
    fn replace(&self, frame: StampedFrame) {
        let Ok(mut latest) = self.latest.lock() else {
            return;
        };
        if latest.closed || latest.error.is_some() {
            return;
        }
        latest.frame = Some(frame);
        self.changed.notify_one();
    }

    fn fail(&self, error: String) {
        let Ok(mut latest) = self.latest.lock() else {
            return;
        };
        if latest.error.is_none() {
            latest.error = Some(error);
        }
        latest.closed = true;
        self.changed.notify_all();
    }

    fn close(&self) {
        let Ok(mut latest) = self.latest.lock() else {
            return;
        };
        latest.closed = true;
        self.changed.notify_all();
    }

    fn discard(&self) -> Result<(), WaylandFrameSourceError> {
        let mut latest = self
            .latest
            .lock()
            .map_err(|_| WaylandFrameSourceError::BridgePoisoned)?;
        latest.frame = None;
        Ok(())
    }

    fn take_after(
        &self,
        minimum_timestamp_ns: u64,
        timeout: Duration,
    ) -> Result<Option<StampedFrame>, WaylandFrameSourceError> {
        let deadline = Instant::now() + timeout;
        let mut latest = self
            .latest
            .lock()
            .map_err(|_| WaylandFrameSourceError::BridgePoisoned)?;
        loop {
            if let Some(error) = latest.error.take() {
                return Err(WaylandFrameSourceError::PipeWireStream(error));
            }
            if let Some(frame) = latest.frame.take() {
                if frame.captured_at_ns >= minimum_timestamp_ns {
                    return Ok(Some(frame));
                }
            }
            if latest.closed {
                return Err(WaylandFrameSourceError::StreamClosed);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let (guard, waited) = self
                .changed
                .wait_timeout(latest, remaining)
                .map_err(|_| WaylandFrameSourceError::BridgePoisoned)?;
            latest = guard;
            if waited.timed_out() && latest.frame.is_none() {
                return Ok(None);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PipeWireAction {
    SetActive(bool),
    Stop,
}

struct PipeWireCommand {
    action: PipeWireAction,
    reply: SyncSender<Result<(), String>>,
}

struct PipeWireThread {
    commands: pw::channel::Sender<PipeWireCommand>,
    thread: Option<JoinHandle<()>>,
}

impl PipeWireThread {
    fn spawn(
        node_id: u32,
        remote: OwnedFd,
        bridge: Arc<FrameBridge>,
        clock_origin: Instant,
        expected_size: (u32, u32),
    ) -> Result<Self, WaylandFrameSourceError> {
        let (commands, receiver) = pw::channel::channel();
        let (initialized_tx, initialized_rx) = mpsc::sync_channel(1);
        let thread_bridge = Arc::clone(&bridge);
        let thread = thread::Builder::new()
            .name("clippy-recording-wayland-pipewire".to_string())
            .spawn(move || {
                if let Err(error) = run_pipewire(
                    node_id,
                    remote,
                    receiver,
                    Arc::clone(&thread_bridge),
                    clock_origin,
                    expected_size,
                    initialized_tx,
                ) {
                    thread_bridge.fail(error);
                }
            })
            .map_err(|error| WaylandFrameSourceError::PipeWireInitialize(error.to_string()))?;
        match initialized_rx.recv_timeout(PIPEWIRE_INIT_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                commands,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(WaylandFrameSourceError::PipeWireInitialize(error))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let (reply, _reply_rx) = mpsc::sync_channel(1);
                let _ = commands.send(PipeWireCommand {
                    action: PipeWireAction::Stop,
                    reply,
                });
                // Rust 无法安全终止卡在第三方初始化中的线程。这里必须保持调用方五秒有界；
                // Portal 会话随后关闭，使迟到的 PipeWire 连接失效，JoinHandle 则安全分离。
                drop(thread);
                Err(WaylandFrameSourceError::PipeWireInitialize(
                    "初始化超过五秒".to_string(),
                ))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                Err(WaylandFrameSourceError::PipeWireInitialize(
                    "初始化线程提前退出".to_string(),
                ))
            }
        }
    }

    fn command(&self, action: PipeWireAction) -> Result<(), WaylandFrameSourceError> {
        if self.thread.as_ref().is_none_or(JoinHandle::is_finished) {
            return Err(WaylandFrameSourceError::PipeWireControl(
                "PipeWire 控制线程已经退出".to_string(),
            ));
        }
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.commands
            .send(PipeWireCommand {
                action,
                reply: reply_tx,
            })
            .map_err(|_| WaylandFrameSourceError::PipeWireControl("控制通道已关闭".to_string()))?;
        match reply_rx.recv_timeout(PIPEWIRE_CONTROL_TIMEOUT) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(WaylandFrameSourceError::PipeWireControl(error)),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                Err(WaylandFrameSourceError::PipeWireControlTimeout)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(
                WaylandFrameSourceError::PipeWireControl("控制线程提前退出".to_string()),
            ),
        }
    }

    fn stop_and_join(&mut self) {
        if self.thread.is_none() {
            return;
        }
        let _ = self.command(PipeWireAction::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for PipeWireThread {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

fn run_pipewire(
    node_id: u32,
    remote: OwnedFd,
    commands: pw::channel::Receiver<PipeWireCommand>,
    bridge: Arc<FrameBridge>,
    clock_origin: Instant,
    expected_size: (u32, u32),
    initialized: SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let setup = || -> anyhow::Result<()> {
        init_pipewire();
        let mainloop = pw::main_loop::MainLoopRc::new(None)?;
        let context = pw::context::ContextRc::new(&mainloop, None)?;
        let core = context.connect_fd_rc(remote, None)?;
        let stream = pw::stream::StreamRc::new(
            core,
            "clippy-recording-wayland",
            pw::properties::properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            },
        )?;
        let listener = stream
            .add_local_listener_with_user_data(PipeWireUserData {
                format: None,
                bridge: Arc::clone(&bridge),
                mainloop: mainloop.clone(),
                clock_origin,
                last_timestamp_ns: None,
                expected_width: expected_size.0,
                expected_height: expected_size.1,
            })
            .state_changed(|_, data, _, new| {
                if let pw::stream::StreamState::Error(message) = new {
                    data.bridge.fail(format!("PipeWire 流报错: {message}"));
                    data.mainloop.quit();
                }
            })
            .param_changed(|_, data, id, param| {
                let Some(param) = param else { return };
                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                match parse_video_format(param) {
                    Ok(info) => {
                        let size = info.size();
                        if let Err(error) = validate_source_geometry(
                            size.width,
                            size.height,
                            data.expected_width,
                            data.expected_height,
                        ) {
                            data.bridge.fail(error);
                            data.mainloop.quit();
                            return;
                        }
                        data.format = Some(info);
                    }
                    Err(error) => {
                        data.bridge.fail(error.to_string());
                        data.mainloop.quit();
                    }
                }
            })
            .process(|stream, data| data.process(stream))
            .register()?;
        let weak_stream = stream.downgrade();
        let command_loop = mainloop.clone();
        let control = commands.attach(mainloop.loop_(), move |command| {
            let result = weak_stream
                .upgrade()
                .ok_or_else(|| "PipeWire stream 已释放".to_string())
                .and_then(|stream| match command.action {
                    PipeWireAction::SetActive(active) => {
                        stream.set_active(active).map_err(|error| error.to_string())
                    }
                    PipeWireAction::Stop => stream.disconnect().map_err(|error| error.to_string()),
                });
            let should_stop = matches!(command.action, PipeWireAction::Stop);
            let _ = command.reply.send(result);
            if should_stop {
                command_loop.quit();
            }
        });
        let format = enum_format_pod()?;
        let mut params = [Pod::from_bytes(&format).context("EnumFormat pod 不合法")?];
        stream.connect(
            spa::utils::Direction::Input,
            Some(node_id),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )?;
        let _ = initialized.send(Ok(()));
        mainloop.run();
        drop(control);
        drop(listener);
        drop(stream);
        bridge.close();
        Ok(())
    };
    match setup() {
        Ok(()) => Ok(()),
        Err(error) => {
            let message = format!("{error:#}");
            let _ = initialized.send(Err(message.clone()));
            Err(message)
        }
    }
}

struct PipeWireUserData {
    format: Option<VideoInfoRaw>,
    bridge: Arc<FrameBridge>,
    mainloop: pw::main_loop::MainLoopRc,
    clock_origin: Instant,
    last_timestamp_ns: Option<u64>,
    expected_width: u32,
    expected_height: u32,
}

impl PipeWireUserData {
    fn process(&mut self, stream: &pw::stream::Stream) {
        let Some(info) = self.format else {
            return;
        };
        let Some(mut buffer) = stream.dequeue_buffer() else {
            return;
        };
        match frame_from_buffer(&mut buffer, info) {
            Ok(frame) => {
                let sampled = self
                    .clock_origin
                    .elapsed()
                    .as_nanos()
                    .min(u128::from(u64::MAX)) as u64;
                let captured_at_ns = self
                    .last_timestamp_ns
                    .and_then(|last| last.checked_add(1))
                    .map_or(sampled, |next| sampled.max(next));
                self.last_timestamp_ns = Some(captured_at_ns);
                self.bridge.replace(StampedFrame {
                    captured_at_ns,
                    frame,
                });
            }
            Err(error) => {
                self.bridge.fail(error.to_string());
                self.mainloop.quit();
            }
        }
    }
}

pub(in crate::recording) struct WaylandPortalRegionFrameSource {
    pipewire: PipeWireThread,
    portal: Option<PortalSession>,
    bridge: Arc<FrameBridge>,
    selection: RecordingCaptureSpec,
    descriptor: RecordingSourceDescriptor,
    clock_origin: Instant,
    minimum_timestamp_ns: u64,
    last_timestamp_ns: Option<u64>,
    next_sequence: u64,
    first_frame: bool,
    running: bool,
}

impl WaylandPortalRegionFrameSource {
    fn connect(plan: WaylandPortalFrameSourcePlan) -> Result<Self, WaylandFrameSourceError> {
        let current = WaylandPortalFrameSourcePlan::prepare(plan.selection)?;
        if current.expected != plan.expected || current.descriptor != plan.descriptor {
            return Err(WaylandFrameSourceError::MonitorGeometryChanged);
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| WaylandFrameSourceError::Portal(error.to_string()))?;
        let (session, node_id, remote) = runtime.block_on(open_portal(&plan.expected))?;
        let portal = PortalSession {
            runtime,
            session: Some(session),
        };
        let bridge = Arc::new(FrameBridge::default());
        let clock_origin = Instant::now();
        let pipewire = PipeWireThread::spawn(
            node_id,
            remote,
            Arc::clone(&bridge),
            clock_origin,
            (
                plan.selection.monitor_pixel_width,
                plan.selection.monitor_pixel_height,
            ),
        )?;
        Ok(Self {
            pipewire,
            portal: Some(portal),
            bridge,
            selection: plan.selection,
            descriptor: plan.descriptor,
            clock_origin,
            minimum_timestamp_ns: 0,
            last_timestamp_ns: None,
            next_sequence: 0,
            first_frame: true,
            running: true,
        })
    }

    pub fn descriptor(&self) -> &RecordingSourceDescriptor {
        &self.descriptor
    }

    fn next_timestamp_ns(&self) -> Result<u64, WaylandFrameSourceError> {
        let sampled = self
            .clock_origin
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        match self.last_timestamp_ns {
            Some(last) => Ok(sampled.max(
                last.checked_add(1)
                    .ok_or(WaylandFrameSourceError::TimestampExhausted)?,
            )),
            None => Ok(sampled),
        }
    }

    fn set_running(&mut self, running: bool) -> Result<u64, WaylandFrameSourceError> {
        if self.running != running {
            if running {
                self.bridge.discard()?;
            }
            self.pipewire.command(PipeWireAction::SetActive(running))?;
            self.running = running;
        }
        let timestamp = self.next_timestamp_ns()?;
        self.last_timestamp_ns = Some(timestamp);
        self.minimum_timestamp_ns = timestamp;
        if !running {
            self.bridge.discard()?;
        }
        Ok(timestamp)
    }

    fn take_frame(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedFrame>, WaylandFrameSourceError> {
        let Some(stamped) = self.bridge.take_after(self.minimum_timestamp_ns, timeout)? else {
            if self.first_frame && self.clock_origin.elapsed() >= FIRST_FRAME_TIMEOUT {
                return Err(WaylandFrameSourceError::FirstFrameTimeout);
            }
            return Ok(None);
        };
        self.first_frame = false;
        let rgba = crop_tight_rgba(
            self.selection,
            stamped.frame.width,
            stamped.frame.height,
            &stamped.frame.rgba,
        )?;
        let sequence = self.next_sequence;
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or(WaylandFrameSourceError::SequenceExhausted)?;
        self.last_timestamp_ns = Some(stamped.captured_at_ns);
        let frame = CapturedFrame {
            sequence,
            captured_at_ns: stamped.captured_at_ns,
            width: self.selection.crop_width,
            height: self.selection.crop_height,
            stride: self.selection.crop_width * 4,
            rgba,
        };
        frame.validate()?;
        Ok(Some(frame))
    }
}

impl RecordingFrameSource for WaylandPortalRegionFrameSource {
    type Error = WaylandFrameSourceError;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        self.take_frame(FIRST_FRAME_TIMEOUT)?
            .ok_or(WaylandFrameSourceError::FirstFrameTimeout)
    }

    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        self.take_frame(FRAME_POLL_TIMEOUT)
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        let timestamp = self.next_timestamp_ns()?;
        self.last_timestamp_ns = Some(timestamp);
        Ok(timestamp)
    }

    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        self.set_running(false)
    }

    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.set_running(true)
    }

    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.set_running(false)
    }
}

impl Drop for WaylandPortalRegionFrameSource {
    fn drop(&mut self) {
        self.pipewire.stop_and_join();
        self.portal.take();
    }
}

fn checked_physical_coordinate(
    logical_origin: i32,
    logical_extent: u32,
    physical_extent: u32,
    offset: u32,
) -> Result<i32, WaylandFrameSourceError> {
    if logical_extent == 0 || physical_extent == 0 {
        return Err(WaylandFrameSourceError::MonitorGeometryChanged);
    }
    let origin = f64::from(logical_origin) * f64::from(physical_extent) / f64::from(logical_extent);
    if !origin.is_finite() || origin < f64::from(i32::MIN) || origin > f64::from(i32::MAX) {
        return Err(WaylandFrameSourceError::CoordinateOverflow);
    }
    (origin.round() as i32)
        .checked_add(
            i32::try_from(offset).map_err(|_| WaylandFrameSourceError::CoordinateOverflow)?,
        )
        .ok_or(WaylandFrameSourceError::CoordinateOverflow)
}

fn validate_source_geometry(
    width: u32,
    height: u32,
    expected_width: u32,
    expected_height: u32,
) -> Result<(), String> {
    if width == expected_width && height == expected_height {
        Ok(())
    } else {
        Err(format!(
            "PipeWire 显示器帧尺寸 {width}x{height} 与冻结尺寸 {expected_width}x{expected_height} 不一致"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected(monitors: usize) -> ExpectedPortalMonitor {
        ExpectedPortalMonitor {
            logical_x: -1920,
            logical_y: 0,
            logical_width: 1920,
            logical_height: 1080,
            monitor_count: monitors,
        }
    }

    #[test]
    fn portal_source_must_match_the_frozen_monitor() {
        assert!(validate_portal_identity(
            Some(SourceType::Monitor),
            Some((-1920, 0)),
            Some((1920, 1080)),
            &expected(2),
        )
        .is_ok());
        for mismatch in [
            validate_portal_identity(
                Some(SourceType::Window),
                Some((-1920, 0)),
                Some((1920, 1080)),
                &expected(2),
            ),
            validate_portal_identity(
                Some(SourceType::Monitor),
                Some((0, 0)),
                Some((1920, 1080)),
                &expected(2),
            ),
            validate_portal_identity(Some(SourceType::Monitor), None, None, &expected(2)),
        ] {
            assert!(matches!(
                mismatch,
                Err(WaylandFrameSourceError::PortalSourceMismatch)
            ));
        }
    }

    #[test]
    fn single_monitor_can_accept_portals_without_optional_geometry() {
        assert!(validate_portal_identity(None, None, None, &expected(1)).is_ok());
        assert!(matches!(
            validate_portal_identity(None, Some((-1920, 0)), None, &expected(1)),
            Err(WaylandFrameSourceError::PortalSourceMismatch)
        ));
    }

    #[test]
    fn signed_logical_origins_scale_before_crop_offset() {
        assert_eq!(
            checked_physical_coordinate(-1920, 1920, 2880, 30).unwrap(),
            -2850
        );
        assert!(matches!(
            checked_physical_coordinate(i32::MAX, 1, 4, 0),
            Err(WaylandFrameSourceError::CoordinateOverflow)
        ));
    }

    #[test]
    fn negotiated_frame_must_match_the_frozen_monitor_before_allocation() {
        assert!(validate_source_geometry(2560, 1440, 2560, 1440).is_ok());
        assert_eq!(
            validate_source_geometry(3840, 2160, 2560, 1440).unwrap_err(),
            "PipeWire 显示器帧尺寸 3840x2160 与冻结尺寸 2560x1440 不一致"
        );
    }
}
