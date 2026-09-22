//! Linux PipeWire 系统声与默认麦克风 source。
//!
//! PipeWire 对象留在专用 main loop 线程；回调只解析一个有界 packet 并写入八块桥接队列。
//! `SPA_META_Header.pts` 通过 `CLOCK_MONOTONIC` 中点校准映射到录屏共享时钟，不能用回调抵达时间
//! 逐块打点。

use crate::pipewire_frame::init_pipewire;
use crate::recording::audio::{CapturedAudioChunk, AUDIO_SAMPLE_RATE_HZ};
use crate::recording::audio_worker::RecordingAudioSource;
use crate::recording::clock::RecordingSessionClock;
use crate::recording::platform::linux_audio_contract::{
    packet_to_chunks, LinuxAudioContractError, LinuxAudioPtsMapper, MAX_PIPEWIRE_PACKET_BYTES,
    PIPEWIRE_CHANNELS, PIPEWIRE_FRAME_BYTES,
};
use anyhow::Context;
use nix::time::{clock_gettime, ClockId};
use pipewire as pw;
use pw::spa;
use spa::param::audio::{AudioFormat as SpaAudioFormat, AudioInfoRaw};
use spa::param::format::{MediaSubtype, MediaType};
use spa::param::format_utils;
use spa::pod::{Pod, Property, Value};
use std::collections::VecDeque;
use std::mem::size_of;
use std::ptr::NonNull;
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;

const PIPEWIRE_INIT_TIMEOUT: Duration = Duration::from_secs(5);
const PIPEWIRE_CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const AUDIO_BRIDGE_CAPACITY: usize = 8;
const MAX_BUFFER_MEMBERS: u32 = 8;
const MAX_BUFFER_METADATA: u32 = 32;

#[derive(Debug, Error)]
pub(in crate::recording) enum LinuxPipeWireAudioSourceError {
    #[error("PipeWire 音频初始化失败: {0}")]
    Initialize(String),
    #[error("PipeWire 音频控制失败: {0}")]
    Control(String),
    #[error("PipeWire 音频初始化超过五秒")]
    InitializeTimeout,
    #[error("PipeWire 音频控制超过五秒")]
    ControlTimeout,
    #[error("PipeWire 音频流已经关闭")]
    StreamClosed,
    #[error("PipeWire 音频流失败: {0}")]
    Stream(String),
    #[error("PipeWire 音频流尚未运行")]
    StreamStopped,
    #[error("PipeWire 音频桥接锁已损坏")]
    BridgePoisoned,
    #[error("PipeWire 音频单调时间戳耗尽")]
    TimestampExhausted,
    #[error(transparent)]
    Contract(#[from] LinuxAudioContractError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinuxAudioSourceKind {
    SystemAudio,
    DefaultMicrophone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::recording) struct LinuxPipeWireAudioSourcePlan {
    kind: LinuxAudioSourceKind,
}

impl LinuxPipeWireAudioSourcePlan {
    pub const fn system_audio() -> Self {
        Self {
            kind: LinuxAudioSourceKind::SystemAudio,
        }
    }

    pub const fn default_microphone() -> Self {
        Self {
            kind: LinuxAudioSourceKind::DefaultMicrophone,
        }
    }

    pub const fn channels(&self) -> u16 {
        PIPEWIRE_CHANNELS
    }

    pub fn connect(
        self,
        clock: RecordingSessionClock,
    ) -> Result<LinuxPipeWireAudioSource, LinuxPipeWireAudioSourceError> {
        LinuxPipeWireAudioSource::connect(self, clock)
    }
}

#[derive(Default)]
struct AudioBridgeState {
    chunks: VecDeque<CapturedAudioChunk>,
    error: Option<String>,
    closed: bool,
}

#[derive(Default)]
struct AudioBridge {
    state: Mutex<AudioBridgeState>,
    changed: Condvar,
}

impl AudioBridge {
    fn push(&self, mut chunks: VecDeque<CapturedAudioChunk>) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "PipeWire 音频桥接锁已损坏".to_string())?;
        if state.closed || state.error.is_some() {
            return Err("PipeWire 音频桥接已经关闭".to_string());
        }
        if state.chunks.len().saturating_add(chunks.len()) > AUDIO_BRIDGE_CAPACITY {
            return Err("PipeWire 音频有界队列已满".to_string());
        }
        state.chunks.append(&mut chunks);
        self.changed.notify_one();
        Ok(())
    }

    fn fail(&self, error: impl Into<String>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.error.is_none() {
            state.error = Some(error.into());
        }
        state.closed = true;
        self.changed.notify_all();
    }

    fn close(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.closed = true;
        self.changed.notify_all();
    }

    fn discard(&self) -> Result<(), LinuxPipeWireAudioSourceError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| LinuxPipeWireAudioSourceError::BridgePoisoned)?;
        state.chunks.clear();
        Ok(())
    }

    fn take_after(
        &self,
        minimum_timestamp_ns: u64,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, LinuxPipeWireAudioSourceError> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .state
            .lock()
            .map_err(|_| LinuxPipeWireAudioSourceError::BridgePoisoned)?;
        loop {
            if let Some(error) = state.error.take() {
                return Err(LinuxPipeWireAudioSourceError::Stream(error));
            }
            while let Some(chunk) = state.chunks.pop_front() {
                if chunk.captured_at_ns >= minimum_timestamp_ns {
                    return Ok(Some(chunk));
                }
            }
            if state.closed {
                return Err(LinuxPipeWireAudioSourceError::StreamClosed);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let (next, waited) = self
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| LinuxPipeWireAudioSourceError::BridgePoisoned)?;
            state = next;
            if waited.timed_out() && state.chunks.is_empty() {
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

struct PipeWireAudioThread {
    commands: pw::channel::Sender<PipeWireCommand>,
    thread: Option<JoinHandle<()>>,
}

impl PipeWireAudioThread {
    fn spawn(
        kind: LinuxAudioSourceKind,
        bridge: Arc<AudioBridge>,
        clock: RecordingSessionClock,
    ) -> Result<Self, LinuxPipeWireAudioSourceError> {
        let (commands, receiver) = pw::channel::channel();
        let (initialized_tx, initialized_rx) = mpsc::sync_channel(1);
        let initialization = Arc::new(Initialization::new(initialized_tx));
        let thread_bridge = Arc::clone(&bridge);
        let thread_initialization = Arc::clone(&initialization);
        let thread = thread::Builder::new()
            .name("clippy-recording-linux-audio".to_string())
            .spawn(move || {
                if let Err(error) = run_pipewire_audio(
                    kind,
                    receiver,
                    Arc::clone(&thread_bridge),
                    clock,
                    Arc::clone(&thread_initialization),
                ) {
                    thread_initialization.complete(Err(error.clone()));
                    thread_bridge.fail(error);
                }
            })
            .map_err(|error| LinuxPipeWireAudioSourceError::Initialize(error.to_string()))?;
        match initialized_rx.recv_timeout(PIPEWIRE_INIT_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                commands,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(LinuxPipeWireAudioSourceError::Initialize(error))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let (reply, _reply_rx) = mpsc::sync_channel(1);
                let _ = commands.send(PipeWireCommand {
                    action: PipeWireAction::Stop,
                    reply,
                });
                drop(thread);
                Err(LinuxPipeWireAudioSourceError::InitializeTimeout)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                Err(LinuxPipeWireAudioSourceError::Initialize(
                    "PipeWire 音频初始化线程提前退出".to_string(),
                ))
            }
        }
    }

    fn command(&self, action: PipeWireAction) -> Result<(), LinuxPipeWireAudioSourceError> {
        if self.thread.as_ref().is_none_or(JoinHandle::is_finished) {
            return Err(LinuxPipeWireAudioSourceError::Control(
                "PipeWire 音频线程已经退出".to_string(),
            ));
        }
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.commands
            .send(PipeWireCommand {
                action,
                reply: reply_tx,
            })
            .map_err(|_| {
                LinuxPipeWireAudioSourceError::Control("PipeWire 音频控制通道已关闭".to_string())
            })?;
        match reply_rx.recv_timeout(PIPEWIRE_CONTROL_TIMEOUT) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(LinuxPipeWireAudioSourceError::Control(error)),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                Err(LinuxPipeWireAudioSourceError::ControlTimeout)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(
                LinuxPipeWireAudioSourceError::Control("PipeWire 音频控制线程提前退出".to_string()),
            ),
        }
    }

    fn stop_and_join(&mut self) -> Result<(), LinuxPipeWireAudioSourceError> {
        let command_result = if self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            self.command(PipeWireAction::Stop)
        } else {
            Ok(())
        };
        if let Err(error) = command_result {
            let should_join = self.thread.as_ref().is_some_and(JoinHandle::is_finished);
            if should_join {
                let _ = self.thread.take().and_then(|thread| thread.join().ok());
            } else {
                // 第三方 loop 卡住时 Rust 无法安全终止线程；分离句柄可保证调用方仍受五秒边界保护。
                drop(self.thread.take());
            }
            return Err(error);
        }
        self.thread
            .take()
            .map(|thread| thread.join())
            .transpose()
            .map_err(|_| {
                LinuxPipeWireAudioSourceError::Control("PipeWire 音频线程发生 panic".to_string())
            })
            .map(|_| ())
    }
}

impl Drop for PipeWireAudioThread {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}

struct Initialization {
    sender: Mutex<Option<SyncSender<Result<(), String>>>>,
}

impl Initialization {
    fn new(sender: SyncSender<Result<(), String>>) -> Self {
        Self {
            sender: Mutex::new(Some(sender)),
        }
    }

    fn complete(&self, result: Result<(), String>) {
        let Ok(mut sender) = self.sender.lock() else {
            return;
        };
        if let Some(sender) = sender.take() {
            let _ = sender.send(result);
        }
    }
}

fn run_pipewire_audio(
    kind: LinuxAudioSourceKind,
    commands: pw::channel::Receiver<PipeWireCommand>,
    bridge: Arc<AudioBridge>,
    clock: RecordingSessionClock,
    initialization: Arc<Initialization>,
) -> Result<(), String> {
    let setup = || -> anyhow::Result<()> {
        init_pipewire();
        let mainloop = pw::main_loop::MainLoopRc::new(None)?;
        let context = pw::context::ContextRc::new(&mainloop, None)?;
        let core = context.connect_rc(None)?;
        let mut properties = pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Production",
            *pw::keys::NODE_LATENCY => "960/48000",
        };
        if kind == LinuxAudioSourceKind::SystemAudio {
            properties.insert(*pw::keys::STREAM_CAPTURE_SINK, "true");
        }
        let stream = pw::stream::StreamRc::new(
            core,
            match kind {
                LinuxAudioSourceKind::SystemAudio => "clippy-recording-system-audio",
                LinuxAudioSourceKind::DefaultMicrophone => "clippy-recording-microphone",
            },
            properties,
        )?;
        let mapper = calibrate_monotonic_clock(&clock)?;
        let listener = stream
            .add_local_listener_with_user_data(PipeWireAudioUserData {
                bridge: Arc::clone(&bridge),
                mainloop: mainloop.clone(),
                initialization: Arc::clone(&initialization),
                mapper,
                next_sequence: 0,
                format_ready: false,
                streaming: false,
                ever_streaming: false,
            })
            .state_changed(|_, data, _, new| data.state_changed(new))
            .param_changed(|stream, data, id, param| data.param_changed(stream, id, param))
            .process(|stream, data| data.process(stream))
            .register()?;
        let weak_stream = stream.downgrade();
        let command_loop = mainloop.clone();
        let control = commands.attach(mainloop.loop_(), move |command| {
            let should_stop = matches!(command.action, PipeWireAction::Stop);
            let result = weak_stream
                .upgrade()
                .ok_or_else(|| "PipeWire 音频 stream 已释放".to_string())
                .and_then(|stream| match command.action {
                    PipeWireAction::SetActive(active) => {
                        stream.set_active(active).map_err(|error| error.to_string())
                    }
                    PipeWireAction::Stop => stream.disconnect().map_err(|error| error.to_string()),
                });
            let _ = command.reply.send(result);
            if should_stop {
                command_loop.quit();
            }
        });
        let format = audio_format_pod()?;
        let mut params =
            [Pod::from_bytes(&format).context("PipeWire audio EnumFormat pod 不合法")?];
        stream.connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )?;
        mainloop.run();
        drop(control);
        drop(listener);
        drop(stream);
        bridge.close();
        Ok(())
    };
    setup().map_err(|error| format!("{error:#}"))
}

struct PipeWireAudioUserData {
    bridge: Arc<AudioBridge>,
    mainloop: pw::main_loop::MainLoopRc,
    initialization: Arc<Initialization>,
    mapper: LinuxAudioPtsMapper,
    next_sequence: u64,
    format_ready: bool,
    streaming: bool,
    ever_streaming: bool,
}

impl PipeWireAudioUserData {
    fn state_changed(&mut self, new: pw::stream::StreamState) {
        match new {
            pw::stream::StreamState::Streaming => {
                self.streaming = true;
                self.ever_streaming = true;
                self.try_initialize();
            }
            pw::stream::StreamState::Paused | pw::stream::StreamState::Connecting => {
                self.streaming = false;
            }
            pw::stream::StreamState::Unconnected if self.ever_streaming => {
                self.fail("PipeWire 音频 stream 已断开".to_string());
            }
            pw::stream::StreamState::Unconnected => {
                self.streaming = false;
            }
            pw::stream::StreamState::Error(message) => {
                self.fail(format!("PipeWire 音频 stream 报错: {message}"));
            }
        }
    }

    fn param_changed(&mut self, stream: &pw::stream::Stream, id: u32, param: Option<&Pod>) {
        if id != spa::param::ParamType::Format.as_raw() {
            return;
        }
        let result = param
            .ok_or_else(|| "PipeWire 音频格式被清除".to_string())
            .and_then(validate_audio_format)
            .and_then(|()| request_header_metadata(stream));
        match result {
            Ok(()) => {
                self.format_ready = true;
                self.try_initialize();
            }
            Err(error) => self.fail(error),
        }
    }

    fn process(&mut self, stream: &pw::stream::Stream) {
        if !self.format_ready || !self.streaming {
            return;
        }
        let result = read_audio_packet(stream, &mut self.mapper, self.next_sequence);
        match result {
            Ok(Some(packet)) => {
                self.next_sequence = packet.next_sequence;
                if let Err(error) = self.bridge.push(packet.chunks) {
                    self.fail(error);
                }
            }
            Ok(None) => {}
            Err(error) => self.fail(error.to_string()),
        }
    }

    fn try_initialize(&self) {
        if self.streaming && self.format_ready {
            self.initialization.complete(Ok(()));
        }
    }

    fn fail(&self, message: String) {
        self.initialization.complete(Err(message.clone()));
        self.bridge.fail(message);
        self.mainloop.quit();
    }
}

fn audio_format_pod() -> anyhow::Result<Vec<u8>> {
    let mut info = AudioInfoRaw::new();
    info.set_format(SpaAudioFormat::F32LE);
    info.set_rate(AUDIO_SAMPLE_RATE_HZ);
    info.set_channels(u32::from(PIPEWIRE_CHANNELS));
    let mut positions = [0; spa::param::audio::MAX_CHANNELS];
    positions[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    positions[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    info.set_position(positions);
    serialize_object(spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    })
}

fn header_metadata_pod() -> anyhow::Result<Vec<u8>> {
    let header_size = i32::try_from(size_of::<spa::sys::spa_meta_header>())
        .context("SPA header metadata 尺寸溢出")?;
    serialize_object(spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamMeta.as_raw(),
        id: spa::param::ParamType::Meta.as_raw(),
        properties: vec![
            Property::new(
                spa::sys::SPA_PARAM_META_type,
                Value::Id(spa::utils::Id(spa::sys::SPA_META_Header)),
            ),
            Property::new(spa::sys::SPA_PARAM_META_size, Value::Int(header_size)),
        ],
    })
}

fn request_header_metadata(stream: &pw::stream::Stream) -> Result<(), String> {
    let metadata = header_metadata_pod().map_err(|error| format!("{error:#}"))?;
    let metadata =
        Pod::from_bytes(&metadata).ok_or_else(|| "PipeWire audio Meta pod 不合法".to_string())?;
    stream
        .update_params(&mut [metadata])
        .map_err(|error| format!("请求 SPA header metadata 失败: {error}"))
}

fn serialize_object(object: spa::pod::Object) -> anyhow::Result<Vec<u8>> {
    Ok(spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(object),
    )?
    .0
    .into_inner())
}

fn validate_audio_format(param: &Pod) -> Result<(), String> {
    let (media_type, subtype) =
        format_utils::parse_format(param).map_err(|error| error.to_string())?;
    if media_type != MediaType::Audio || subtype != MediaSubtype::Raw {
        return Err("PipeWire 协商结果不是 raw audio".to_string());
    }
    let mut info = AudioInfoRaw::new();
    info.parse(param).map_err(|error| error.to_string())?;
    let positions = info.position();
    if info.format() != SpaAudioFormat::F32LE
        || info.rate() != AUDIO_SAMPLE_RATE_HZ
        || info.channels() != u32::from(PIPEWIRE_CHANNELS)
        || positions[0] != spa::sys::SPA_AUDIO_CHANNEL_FL
        || positions[1] != spa::sys::SPA_AUDIO_CHANNEL_FR
    {
        return Err(format!(
            "PipeWire 音频格式必须为 F32LE/{}Hz/{}ch FL/FR，实际为 {:?}/{}Hz/{}ch {}/{}",
            AUDIO_SAMPLE_RATE_HZ,
            PIPEWIRE_CHANNELS,
            info.format(),
            info.rate(),
            info.channels(),
            positions[0],
            positions[1]
        ));
    }
    Ok(())
}

fn calibrate_monotonic_clock(clock: &RecordingSessionClock) -> anyhow::Result<LinuxAudioPtsMapper> {
    let before = clock.now_ns();
    let native = clock_gettime(ClockId::CLOCK_MONOTONIC).context("读取 CLOCK_MONOTONIC 失败")?;
    let after = clock.now_ns();
    let seconds = u64::try_from(native.tv_sec()).context("CLOCK_MONOTONIC 秒数无效")?;
    let nanos = u64::try_from(native.tv_nsec()).context("CLOCK_MONOTONIC 纳秒无效")?;
    let native_ns = seconds
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_add(nanos))
        .context("CLOCK_MONOTONIC 纳秒溢出")?;
    LinuxAudioPtsMapper::from_calibration(native_ns, before, after).map_err(Into::into)
}

struct RequeuedBuffer<'a> {
    stream: &'a pw::stream::Stream,
    raw: NonNull<pw::sys::pw_buffer>,
}

impl Drop for RequeuedBuffer<'_> {
    fn drop(&mut self) {
        unsafe { self.stream.queue_raw_buffer(self.raw.as_ptr()) };
    }
}

fn read_audio_packet(
    stream: &pw::stream::Stream,
    mapper: &mut LinuxAudioPtsMapper,
    first_sequence: u64,
) -> Result<Option<super::linux_audio_contract::PacketChunks>, LinuxPipeWireAudioSourceError> {
    let Some(raw) = NonNull::new(unsafe { stream.dequeue_raw_buffer() }) else {
        return Ok(None);
    };
    let mut guard = RequeuedBuffer { stream, raw };
    let spa_buffer = unsafe { guard.raw.as_ref().buffer };
    let spa_buffer = NonNull::new(spa_buffer).ok_or_else(|| {
        LinuxPipeWireAudioSourceError::Initialize("PipeWire 返回空 SPA buffer".to_string())
    })?;
    let (header, bytes) = unsafe { copy_packet_parts(spa_buffer)? };
    let frame_count = u32::try_from(bytes.len() / PIPEWIRE_FRAME_BYTES)
        .map_err(|_| LinuxAudioContractError::SampleLengthOverflow)?;
    unsafe {
        guard.raw.as_mut().size = u64::from(frame_count);
    }
    let gap = header.flags & spa::sys::SPA_META_HEADER_FLAG_GAP != 0;
    let samples = if gap {
        None
    } else {
        Some(decode_f32le(&bytes)?)
    };
    let captured_at_ns = mapper.map_packet(header.pts, frame_count)?;
    let packet = packet_to_chunks(
        first_sequence,
        captured_at_ns,
        frame_count,
        samples.as_deref(),
    )?;
    drop(guard);
    Ok(Some(packet))
}

unsafe fn copy_packet_parts(
    buffer: NonNull<spa::sys::spa_buffer>,
) -> Result<(spa::sys::spa_meta_header, Vec<u8>), LinuxPipeWireAudioSourceError> {
    let buffer = buffer.as_ref();
    if buffer.n_metas == 0 || buffer.n_metas > MAX_BUFFER_METADATA || buffer.metas.is_null() {
        return Err(LinuxPipeWireAudioSourceError::Initialize(
            "PipeWire 音频缺少有效 SPA header metadata".to_string(),
        ));
    }
    let metas = std::slice::from_raw_parts(buffer.metas, buffer.n_metas as usize);
    let mut headers = metas
        .iter()
        .filter(|meta| meta.type_ == spa::sys::SPA_META_Header);
    let meta = headers.next().ok_or_else(|| {
        LinuxPipeWireAudioSourceError::Initialize("PipeWire 音频缺少 SPA_META_Header".to_string())
    })?;
    if headers.next().is_some()
        || meta.data.is_null()
        || usize::try_from(meta.size).unwrap_or(0) < size_of::<spa::sys::spa_meta_header>()
    {
        return Err(LinuxPipeWireAudioSourceError::Initialize(
            "PipeWire 音频 SPA_META_Header 结构无效".to_string(),
        ));
    }
    let header = std::ptr::read_unaligned(meta.data.cast::<spa::sys::spa_meta_header>());
    if header.pts == i64::MIN || header.flags & spa::sys::SPA_META_HEADER_FLAG_CORRUPTED != 0 {
        return Err(LinuxPipeWireAudioSourceError::Initialize(
            "PipeWire 音频 header 时间戳无效或数据已损坏".to_string(),
        ));
    }
    if buffer.n_datas != 1 || buffer.n_datas > MAX_BUFFER_MEMBERS || buffer.datas.is_null() {
        return Err(LinuxPipeWireAudioSourceError::Initialize(
            "PipeWire 音频必须包含一个 interleaved data member".to_string(),
        ));
    }
    let data = &*buffer.datas;
    let chunk = NonNull::new(data.chunk).ok_or_else(|| {
        LinuxPipeWireAudioSourceError::Initialize("PipeWire 音频 chunk 为空".to_string())
    })?;
    let chunk = chunk.as_ref();
    if chunk.flags as u32 & spa::sys::SPA_CHUNK_FLAG_CORRUPTED != 0 {
        return Err(LinuxPipeWireAudioSourceError::Initialize(
            "PipeWire 音频 chunk 已损坏".to_string(),
        ));
    }
    if chunk.stride != 0 && chunk.stride != PIPEWIRE_FRAME_BYTES as i32 {
        return Err(LinuxPipeWireAudioSourceError::Initialize(format!(
            "PipeWire 音频 stride {} 无效",
            chunk.stride
        )));
    }
    let size = usize::try_from(chunk.size).map_err(|_| {
        LinuxPipeWireAudioSourceError::Initialize("PipeWire 音频 chunk 尺寸溢出".to_string())
    })?;
    let maxsize = usize::try_from(data.maxsize).map_err(|_| {
        LinuxPipeWireAudioSourceError::Initialize("PipeWire 音频 buffer 尺寸溢出".to_string())
    })?;
    if size == 0
        || size > MAX_PIPEWIRE_PACKET_BYTES
        || size > maxsize
        || size % PIPEWIRE_FRAME_BYTES != 0
        || maxsize == 0
    {
        return Err(LinuxPipeWireAudioSourceError::Initialize(
            "PipeWire 音频 chunk 长度无效".to_string(),
        ));
    }
    let gap = header.flags & spa::sys::SPA_META_HEADER_FLAG_GAP != 0;
    if data.data.is_null() {
        if gap {
            return Ok((header, vec![0; size]));
        }
        return Err(LinuxPipeWireAudioSourceError::Initialize(
            "PipeWire 音频普通 packet 缺少映射数据".to_string(),
        ));
    }
    let offset = usize::try_from(chunk.offset).unwrap_or(0) % maxsize;
    let first_len = size.min(maxsize - offset);
    let base = data.data.cast::<u8>();
    let first = std::slice::from_raw_parts(base.add(offset), first_len);
    let mut bytes = Vec::with_capacity(size);
    bytes.extend_from_slice(first);
    if first_len < size {
        let second = std::slice::from_raw_parts(base, size - first_len);
        bytes.extend_from_slice(second);
    }
    Ok((header, bytes))
}

fn decode_f32le(bytes: &[u8]) -> Result<Box<[f32]>, LinuxPipeWireAudioSourceError> {
    let (samples, remainder) = bytes.as_chunks::<{ size_of::<f32>() }>();
    if !remainder.is_empty() {
        return Err(LinuxAudioContractError::SampleLengthMismatch.into());
    }
    Ok(samples
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect::<Vec<_>>()
        .into_boxed_slice())
}

pub(in crate::recording) struct LinuxPipeWireAudioSource {
    pipewire: PipeWireAudioThread,
    bridge: Arc<AudioBridge>,
    clock: RecordingSessionClock,
    minimum_timestamp_ns: u64,
    last_control_timestamp_ns: Option<u64>,
    last_delivered_end_ns: Option<u64>,
    running: bool,
}

impl LinuxPipeWireAudioSource {
    fn connect(
        plan: LinuxPipeWireAudioSourcePlan,
        clock: RecordingSessionClock,
    ) -> Result<Self, LinuxPipeWireAudioSourceError> {
        let bridge = Arc::new(AudioBridge::default());
        let pipewire = PipeWireAudioThread::spawn(plan.kind, Arc::clone(&bridge), clock.clone())?;
        Ok(Self {
            pipewire,
            bridge,
            clock,
            minimum_timestamp_ns: 0,
            last_control_timestamp_ns: None,
            last_delivered_end_ns: None,
            running: true,
        })
    }

    fn next_control_timestamp_ns(&mut self) -> Result<u64, LinuxPipeWireAudioSourceError> {
        let mut timestamp = self.clock.now_ns();
        if let Some(last_end) = self.last_delivered_end_ns {
            timestamp = timestamp.max(last_end);
        }
        if let Some(last_control) = self.last_control_timestamp_ns {
            timestamp = timestamp.max(
                last_control
                    .checked_add(1)
                    .ok_or(LinuxPipeWireAudioSourceError::TimestampExhausted)?,
            );
        }
        self.last_control_timestamp_ns = Some(timestamp);
        Ok(timestamp)
    }

    fn pause(&mut self) -> Result<u64, LinuxPipeWireAudioSourceError> {
        if self.running {
            self.pipewire.command(PipeWireAction::SetActive(false))?;
            self.running = false;
        }
        self.bridge.discard()?;
        let timestamp = self.next_control_timestamp_ns()?;
        self.minimum_timestamp_ns = timestamp;
        Ok(timestamp)
    }

    fn resume(&mut self) -> Result<u64, LinuxPipeWireAudioSourceError> {
        self.bridge.discard()?;
        let timestamp = self.next_control_timestamp_ns()?;
        self.minimum_timestamp_ns = timestamp;
        if !self.running {
            self.pipewire.command(PipeWireAction::SetActive(true))?;
            self.running = true;
        }
        Ok(timestamp)
    }

    fn stop(&mut self) -> Result<u64, LinuxPipeWireAudioSourceError> {
        self.pipewire.stop_and_join()?;
        self.running = false;
        self.bridge.discard()?;
        self.next_control_timestamp_ns()
    }
}

impl RecordingAudioSource for LinuxPipeWireAudioSource {
    type Error = LinuxPipeWireAudioSourceError;

    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        if !self.running {
            return Err(LinuxPipeWireAudioSourceError::StreamStopped);
        }
        let Some(chunk) = self.bridge.take_after(self.minimum_timestamp_ns, timeout)? else {
            return Ok(None);
        };
        let duration_ns = u64::from(chunk.frame_count)
            .checked_mul(1_000_000_000)
            .map(|value| value / u64::from(AUDIO_SAMPLE_RATE_HZ))
            .ok_or(LinuxPipeWireAudioSourceError::TimestampExhausted)?;
        self.last_delivered_end_ns = Some(
            chunk
                .captured_at_ns
                .checked_add(duration_ns)
                .ok_or(LinuxPipeWireAudioSourceError::TimestampExhausted)?,
        );
        Ok(Some(chunk))
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        self.next_control_timestamp_ns()
    }

    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        self.pause()
    }

    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.resume()
    }

    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.stop()
    }
}

impl Drop for LinuxPipeWireAudioSource {
    fn drop(&mut self) {
        let _ = self.pipewire.stop_and_join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_plans_keep_system_and_microphone_distinct() {
        assert_eq!(
            LinuxPipeWireAudioSourcePlan::system_audio().kind,
            LinuxAudioSourceKind::SystemAudio
        );
        assert_eq!(
            LinuxPipeWireAudioSourcePlan::default_microphone().kind,
            LinuxAudioSourceKind::DefaultMicrophone
        );
        assert_eq!(
            LinuxPipeWireAudioSourcePlan::system_audio().channels(),
            PIPEWIRE_CHANNELS
        );
    }

    #[test]
    fn requested_audio_and_header_pods_are_well_formed() {
        let format = audio_format_pod().unwrap();
        let format = Pod::from_bytes(&format).unwrap();
        validate_audio_format(format).unwrap();
        let metadata = header_metadata_pod().unwrap();
        assert!(Pod::from_bytes(&metadata).is_some());
    }

    #[test]
    fn negotiated_audio_rejects_reversed_channel_positions() {
        let mut info = AudioInfoRaw::new();
        info.set_format(SpaAudioFormat::F32LE);
        info.set_rate(AUDIO_SAMPLE_RATE_HZ);
        info.set_channels(u32::from(PIPEWIRE_CHANNELS));
        let mut positions = [0; spa::param::audio::MAX_CHANNELS];
        positions[0] = spa::sys::SPA_AUDIO_CHANNEL_FR;
        positions[1] = spa::sys::SPA_AUDIO_CHANNEL_FL;
        info.set_position(positions);
        let bytes = serialize_object(spa::pod::Object {
            type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: spa::param::ParamType::Format.as_raw(),
            properties: info.into(),
        })
        .unwrap();
        let error = validate_audio_format(Pod::from_bytes(&bytes).unwrap()).unwrap_err();
        assert!(error.contains("FL/FR"));
    }

    #[test]
    fn little_endian_decode_preserves_values() {
        let mut bytes = Vec::new();
        for value in [0.0_f32, 0.25, -0.5, 1.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(
            decode_f32le(&bytes).unwrap().as_ref(),
            &[0.0, 0.25, -0.5, 1.0]
        );
    }

    #[test]
    fn bridge_is_bounded_and_filters_pre_resume_chunks() {
        let bridge = AudioBridge::default();
        let chunks = (0..AUDIO_BRIDGE_CAPACITY)
            .map(|sequence| CapturedAudioChunk {
                sequence: sequence as u64,
                captured_at_ns: sequence as u64,
                format: crate::recording::audio::AudioFormat::normalized(PIPEWIRE_CHANNELS),
                frame_count: 1,
                samples: vec![0.0, 0.0].into_boxed_slice(),
            })
            .collect();
        bridge.push(chunks).unwrap();
        let extra = VecDeque::from([CapturedAudioChunk {
            sequence: 9,
            captured_at_ns: 9,
            format: crate::recording::audio::AudioFormat::normalized(PIPEWIRE_CHANNELS),
            frame_count: 1,
            samples: vec![0.0, 0.0].into_boxed_slice(),
        }]);
        assert_eq!(bridge.push(extra).unwrap_err(), "PipeWire 音频有界队列已满");
        assert_eq!(
            bridge
                .take_after(5, Duration::ZERO)
                .unwrap()
                .unwrap()
                .captured_at_ns,
            5
        );
    }

    fn exercise_native_source(plan: LinuxPipeWireAudioSourcePlan) {
        assert_eq!(
            std::env::var("CLIPPY_TEST_PIPEWIRE_AUDIO").as_deref(),
            Ok("1"),
            "必须显式设置 CLIPPY_TEST_PIPEWIRE_AUDIO=1"
        );
        let mut source = plan.connect(RecordingSessionClock::new()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let first = loop {
            if let Some(chunk) = source
                .capture_next_available(Duration::from_millis(100))
                .unwrap()
            {
                break chunk;
            }
            assert!(Instant::now() < deadline, "等待首个 PipeWire 音频块超时");
        };
        assert_eq!(first.format.sample_rate_hz, AUDIO_SAMPLE_RATE_HZ);
        assert_eq!(first.format.channels, PIPEWIRE_CHANNELS);
        source.pause_capture().unwrap();
        source.resume_capture().unwrap();
        source.stop_capture().unwrap();
    }

    #[test]
    #[ignore = "需要真实 PipeWire 默认 sink/source，且显式允许采集本机音频"]
    fn native_default_system_audio_smoke() {
        exercise_native_source(LinuxPipeWireAudioSourcePlan::system_audio());
    }

    #[test]
    #[ignore = "需要真实 PipeWire 默认 sink/source，且显式允许采集本机音频"]
    fn native_default_microphone_smoke() {
        exercise_native_source(LinuxPipeWireAudioSourcePlan::default_microphone());
    }
}
