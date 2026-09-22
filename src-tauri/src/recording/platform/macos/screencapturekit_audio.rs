//! ScreenCaptureKit 系统音频 source。
//!
//! 原生回调只复制、校验并向有界队列 `try_send`。worker 线程拥有 stream、输出对象和接收端，
//! 因而 Objective-C 对象不会跨线程移动，队列拥塞也不会阻塞 ScreenCaptureKit 串行队列。

use super::screencapturekit::{request_shareable_content, unique_display};
use super::MacRegionFrameSourcePlan;
use crate::recording::audio::{AudioFormat, CapturedAudioChunk, AUDIO_SAMPLE_RATE_HZ};
use crate::recording::audio_worker::RecordingAudioSource;
use crate::recording::clock::RecordingSessionClock;
use crate::recording::platform::macos_audio_contract::{
    normalize_float_pcm, split_stereo_packet, MacAudioContractError, MacAudioPtsMapper,
    MappedPcmChunk, NativeFloatPcm,
};
use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::{define_class, rc::Retained, runtime::ProtocolObject, AllocAnyThread, DefinedClass};
use objc2_core_audio_types::{
    kAudioFormatFlagIsBigEndian, kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved,
    kAudioFormatFlagIsPacked, kAudioFormatLinearPCM, AudioBuffer, AudioBufferList,
    AudioStreamBasicDescription,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{
    kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
    CMAudioFormatDescriptionGetStreamBasicDescription, CMBlockBuffer, CMSampleBuffer,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCStream, SCStreamConfiguration, SCStreamDelegate, SCStreamOutput,
    SCStreamOutputType, SCWindow,
};
use std::mem::{align_of, size_of};
use std::ptr::{self, NonNull};
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;

const ASYNC_OPERATION_TIMEOUT: Duration = Duration::from_secs(8);
const NATIVE_AUDIO_QUEUE_CAPACITY: usize = 8;
const OUTPUT_CHANNELS: u16 = 2;
const BYTES_PER_FLOAT_SAMPLE: usize = size_of::<f32>();

#[derive(Debug, Error)]
pub(in crate::recording) enum MacScreenCaptureKitAudioSourceError {
    #[error("macOS 系统音频初始化失败: {0}")]
    Initialize(String),
    #[error("macOS 系统音频控制失败: {0}")]
    Control(String),
    #[error("macOS 系统音频流已经关闭")]
    StreamClosed,
    #[error("macOS 系统音频桥接锁已损坏")]
    BridgePoisoned,
    #[error("macOS 系统音频块序号耗尽")]
    SequenceExhausted,
    #[error("macOS 系统音频单调时间戳耗尽")]
    TimestampExhausted,
    #[error(transparent)]
    Contract(#[from] MacAudioContractError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::recording) struct MacScreenCaptureKitAudioSourcePlan {
    monitor_id: u32,
}

impl MacScreenCaptureKitAudioSourcePlan {
    pub fn from_frame_plan(plan: &MacRegionFrameSourcePlan) -> Self {
        Self {
            monitor_id: plan.selection.monitor_id,
        }
    }

    pub const fn channels(&self) -> u16 {
        OUTPUT_CHANNELS
    }

    pub fn connect(
        self,
        clock: RecordingSessionClock,
    ) -> Result<MacScreenCaptureKitAudioSource, MacScreenCaptureKitAudioSourceError> {
        MacScreenCaptureKitAudioSource::connect(self, clock)
    }
}

struct NativeAudioChunk {
    captured_at_ns: u64,
    frame_count: u32,
    samples: Box<[f32]>,
}

#[derive(Debug, Clone)]
struct AudioOutputVars {
    chunks: SyncSender<NativeAudioChunk>,
    failure: Arc<Mutex<Option<String>>>,
    timeline: Arc<Mutex<MacAudioPtsMapper>>,
    running: Arc<AtomicBool>,
    clock: RecordingSessionClock,
}

impl AudioOutputVars {
    fn fail(&self, message: impl Into<String>) {
        self.running.store(false, Ordering::Release);
        let Ok(mut failure) = self.failure.lock() else {
            return;
        };
        if failure.is_none() {
            *failure = Some(message.into());
        }
    }

    fn capture(&self, sample_buffer: &CMSampleBuffer, output_type: SCStreamOutputType) {
        if output_type != SCStreamOutputType::Audio || !self.running.load(Ordering::Acquire) {
            return;
        }
        let native = unsafe { copy_audio_packet(sample_buffer) };
        let (pts_value, pts_timescale, frame_count, samples) = match native {
            Ok(native) => native,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let captured_at_ns = {
            let Ok(mut timeline) = self.timeline.lock() else {
                self.fail("ScreenCaptureKit 音频时间线锁已损坏");
                return;
            };
            match timeline.map_packet(pts_value, pts_timescale, self.clock.now_ns(), frame_count) {
                Ok(timestamp) => timestamp,
                Err(error) => {
                    drop(timeline);
                    self.fail(error.to_string());
                    return;
                }
            }
        };
        let chunks = match split_stereo_packet(captured_at_ns, frame_count, &samples) {
            Ok(chunks) => chunks,
            Err(error) => {
                self.fail(error.to_string());
                return;
            }
        };
        for MappedPcmChunk {
            captured_at_ns,
            frame_count,
            samples,
        } in chunks
        {
            if !self.running.load(Ordering::Acquire) {
                return;
            }
            match self.chunks.try_send(NativeAudioChunk {
                captured_at_ns,
                frame_count,
                samples,
            }) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    self.fail("ScreenCaptureKit 音频有界队列已满");
                    return;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.running.store(false, Ordering::Release);
                    return;
                }
            }
        }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ClippyScreenCaptureKitAudioOutput"]
    #[ivars = AudioOutputVars]
    #[derive(Debug)]
    struct AudioOutput;

    unsafe impl SCStreamOutput for AudioOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        #[allow(non_snake_case)]
        unsafe fn stream_didOutputSampleBuffer_ofType(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            output_type: SCStreamOutputType,
        ) {
            self.ivars().capture(sample_buffer, output_type);
        }
    }

    unsafe impl SCStreamDelegate for AudioOutput {
        #[unsafe(method(stream:didStopWithError:))]
        #[allow(non_snake_case)]
        unsafe fn stream_didStopWithError(&self, _stream: &SCStream, error: &NSError) {
            self.ivars().fail(format!(
                "ScreenCaptureKit audio stream stopped: {}",
                error.localizedDescription()
            ));
        }
    }
);

unsafe impl NSObjectProtocol for AudioOutput {}

impl AudioOutput {
    fn new(vars: AudioOutputVars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(vars);
        unsafe { objc2::msg_send![super(this), init] }
    }
}

pub(in crate::recording) struct MacScreenCaptureKitAudioSource {
    stream: Retained<SCStream>,
    _output: Retained<AudioOutput>,
    _queue: DispatchRetained<DispatchQueue>,
    chunks: Receiver<NativeAudioChunk>,
    failure: Arc<Mutex<Option<String>>>,
    timeline: Arc<Mutex<MacAudioPtsMapper>>,
    running_flag: Arc<AtomicBool>,
    clock: RecordingSessionClock,
    minimum_timestamp_ns: u64,
    last_control_timestamp_ns: Option<u64>,
    next_sequence: u64,
    running: bool,
}

impl MacScreenCaptureKitAudioSource {
    fn connect(
        plan: MacScreenCaptureKitAudioSourcePlan,
        clock: RecordingSessionClock,
    ) -> Result<Self, MacScreenCaptureKitAudioSourceError> {
        if !crate::recording::macos_screencapturekit_audio_runtime_available() {
            return Err(MacScreenCaptureKitAudioSourceError::Initialize(
                "ScreenCaptureKit 系统音频要求 macOS 13.0 或更高版本".to_string(),
            ));
        }
        let content = request_shareable_content()
            .map_err(|error| MacScreenCaptureKitAudioSourceError::Initialize(error.to_string()))?;
        let display = unique_display(&content, plan.monitor_id)
            .map_err(|error| MacScreenCaptureKitAudioSourceError::Initialize(error.to_string()))?;
        let excluded = NSArray::<SCWindow>::new();
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingWindows(
                SCContentFilter::alloc(),
                &display,
                &excluded,
            )
        };
        let configuration = unsafe { SCStreamConfiguration::new() };
        unsafe {
            configuration.setCapturesAudio(true);
            configuration.setSampleRate(AUDIO_SAMPLE_RATE_HZ as isize);
            configuration.setChannelCount(OUTPUT_CHANNELS as isize);
            configuration.setExcludesCurrentProcessAudio(true);
        }

        let (chunk_tx, chunks) = mpsc::sync_channel(NATIVE_AUDIO_QUEUE_CAPACITY);
        let failure = Arc::new(Mutex::new(None));
        let timeline = Arc::new(Mutex::new(MacAudioPtsMapper::default()));
        let running_flag = Arc::new(AtomicBool::new(true));
        let output = AudioOutput::new(AudioOutputVars {
            chunks: chunk_tx,
            failure: Arc::clone(&failure),
            timeline: Arc::clone(&timeline),
            running: Arc::clone(&running_flag),
            clock: clock.clone(),
        });
        let delegate: &ProtocolObject<dyn SCStreamDelegate> = ProtocolObject::from_ref(&*output);
        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &configuration,
                Some(delegate),
            )
        };
        let queue = DispatchQueue::new(
            "com.clippy.recording.screencapturekit.audio",
            DispatchQueueAttr::SERIAL,
        );
        let output_protocol: &ProtocolObject<dyn SCStreamOutput> =
            ProtocolObject::from_ref(&*output);
        unsafe {
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    output_protocol,
                    SCStreamOutputType::Audio,
                    Some(&queue),
                )
                .map_err(|error| {
                    MacScreenCaptureKitAudioSourceError::Initialize(
                        error.localizedDescription().to_string(),
                    )
                })?;
        }
        if let Err(error) = set_stream_running(&stream, true) {
            running_flag.store(false, Ordering::Release);
            return Err(error);
        }

        Ok(Self {
            stream,
            _output: output,
            _queue: queue,
            chunks,
            failure,
            timeline,
            running_flag,
            clock,
            minimum_timestamp_ns: 0,
            last_control_timestamp_ns: None,
            next_sequence: 0,
            running: true,
        })
    }

    fn take_failure(&self) -> Result<Option<String>, MacScreenCaptureKitAudioSourceError> {
        self.failure
            .lock()
            .map(|failure| failure.clone())
            .map_err(|_| MacScreenCaptureKitAudioSourceError::BridgePoisoned)
    }

    fn next_control_timestamp_ns(&mut self) -> Result<u64, MacScreenCaptureKitAudioSourceError> {
        let sampled = self
            .timeline
            .lock()
            .map_err(|_| MacScreenCaptureKitAudioSourceError::BridgePoisoned)?
            .control_timestamp_ns(self.clock.now_ns());
        let timestamp = match self.last_control_timestamp_ns {
            Some(last) => sampled.max(
                last.checked_add(1)
                    .ok_or(MacScreenCaptureKitAudioSourceError::TimestampExhausted)?,
            ),
            None => sampled,
        };
        self.last_control_timestamp_ns = Some(timestamp);
        Ok(timestamp)
    }

    fn discard_chunks(&self) {
        while self.chunks.try_recv().is_ok() {}
    }

    fn set_running(&mut self, running: bool) -> Result<u64, MacScreenCaptureKitAudioSourceError> {
        if self.running != running {
            if running {
                self.discard_chunks();
                self.running_flag.store(true, Ordering::Release);
                if let Err(error) = set_stream_running(&self.stream, true) {
                    self.running_flag.store(false, Ordering::Release);
                    return Err(error);
                }
            } else {
                self.running_flag.store(false, Ordering::Release);
                set_stream_running(&self.stream, false)?;
            }
            self.running = running;
        }
        let timestamp = self.next_control_timestamp_ns()?;
        self.minimum_timestamp_ns = timestamp;
        self.discard_chunks();
        Ok(timestamp)
    }
}

impl RecordingAudioSource for MacScreenCaptureKitAudioSource {
    type Error = MacScreenCaptureKitAudioSourceError;

    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        if let Some(message) = self.take_failure()? {
            return Err(MacScreenCaptureKitAudioSourceError::Control(message));
        }
        loop {
            let native = match self.chunks.recv_timeout(timeout) {
                Ok(chunk) => chunk,
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(message) = self.take_failure()? {
                        return Err(MacScreenCaptureKitAudioSourceError::Control(message));
                    }
                    return Ok(None);
                }
                Err(RecvTimeoutError::Disconnected) => {
                    if let Some(message) = self.take_failure()? {
                        return Err(MacScreenCaptureKitAudioSourceError::Control(message));
                    }
                    return Err(MacScreenCaptureKitAudioSourceError::StreamClosed);
                }
            };
            if native.captured_at_ns < self.minimum_timestamp_ns {
                continue;
            }
            let sequence = self.next_sequence;
            self.next_sequence = self
                .next_sequence
                .checked_add(1)
                .ok_or(MacScreenCaptureKitAudioSourceError::SequenceExhausted)?;
            return Ok(Some(CapturedAudioChunk {
                sequence,
                captured_at_ns: native.captured_at_ns,
                format: AudioFormat::normalized(OUTPUT_CHANNELS),
                frame_count: native.frame_count,
                samples: native.samples,
            }));
        }
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        self.next_control_timestamp_ns()
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

impl Drop for MacScreenCaptureKitAudioSource {
    fn drop(&mut self) {
        self.running_flag.store(false, Ordering::Release);
        if self.running {
            let _ = set_stream_running(&self.stream, false);
            self.running = false;
        }
    }
}

fn set_stream_running(
    stream: &SCStream,
    running: bool,
) -> Result<(), MacScreenCaptureKitAudioSourceError> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let completion = RcBlock::new(move |error: *mut NSError| {
        let result = unsafe { error.as_ref() }
            .map(|error| Err(error.localizedDescription().to_string()))
            .unwrap_or(Ok(()));
        let _ = sender.try_send(result);
    });
    unsafe {
        if running {
            stream.startCaptureWithCompletionHandler(Some(&completion));
        } else {
            stream.stopCaptureWithCompletionHandler(Some(&completion));
        }
    }
    match receiver.recv_timeout(ASYNC_OPERATION_TIMEOUT) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => Err(MacScreenCaptureKitAudioSourceError::Control(message)),
        Err(_) => Err(MacScreenCaptureKitAudioSourceError::Control(format!(
            "等待 ScreenCaptureKit 音频{}超时或回调已经断开",
            if running { "启动" } else { "停止" }
        ))),
    }
}

#[repr(C)]
struct StereoAudioBufferList {
    number_buffers: u32,
    buffers: [AudioBuffer; 2],
}

/// 在 ScreenCaptureKit 回调返回前复制 PCM；`CFRetained<CMBlockBuffer>` 保证 ABL 数据在复制期间有效。
unsafe fn copy_audio_packet(
    sample_buffer: &CMSampleBuffer,
) -> Result<(i64, i32, u32, Box<[f32]>), String> {
    if !unsafe { sample_buffer.is_valid() } {
        return Err("ScreenCaptureKit 返回无效音频 sample buffer".to_string());
    }
    let frame_count = u32::try_from(unsafe { sample_buffer.num_samples() })
        .ok()
        .filter(|count| *count != 0)
        .ok_or_else(|| "ScreenCaptureKit 返回空音频 packet".to_string())?;
    let pts = unsafe { sample_buffer.presentation_time_stamp() };
    if pts.value < 0 || pts.timescale <= 0 {
        return Err("ScreenCaptureKit 返回无效音频 PTS".to_string());
    }
    let format = unsafe { sample_buffer.format_description() }
        .ok_or_else(|| "ScreenCaptureKit 音频缺少格式描述".to_string())?;
    let asbd = unsafe { CMAudioFormatDescriptionGetStreamBasicDescription(&format) };
    let asbd =
        unsafe { asbd.as_ref() }.ok_or_else(|| "ScreenCaptureKit 音频缺少 ASBD".to_string())?;
    validate_audio_format(asbd)?;

    let mut list = StereoAudioBufferList {
        number_buffers: 0,
        buffers: [
            AudioBuffer {
                mNumberChannels: 0,
                mDataByteSize: 0,
                mData: ptr::null_mut(),
            },
            AudioBuffer {
                mNumberChannels: 0,
                mDataByteSize: 0,
                mData: ptr::null_mut(),
            },
        ],
    };
    let mut required_size = 0_usize;
    let mut block_buffer = ptr::null_mut::<CMBlockBuffer>();
    let status = unsafe {
        sample_buffer.audio_buffer_list_with_retained_block_buffer(
            &mut required_size,
            (&mut list as *mut StereoAudioBufferList).cast::<AudioBufferList>(),
            size_of::<StereoAudioBufferList>(),
            None,
            None,
            kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
            &mut block_buffer,
        )
    };
    if status != 0 {
        return Err(format!(
            "ScreenCaptureKit 读取 AudioBufferList 失败: OSStatus {status}"
        ));
    }
    if required_size > size_of::<StereoAudioBufferList>() {
        return Err(format!(
            "ScreenCaptureKit 音频缓冲数量超出双声道预算: {required_size} bytes"
        ));
    }
    let block_buffer = NonNull::new(block_buffer)
        .map(|pointer| unsafe { CFRetained::from_raw(pointer) })
        .ok_or_else(|| "ScreenCaptureKit 音频缺少 backing block buffer".to_string())?;
    let _block_buffer = block_buffer;
    let buffers = list
        .buffers
        .get(..usize::try_from(list.number_buffers).unwrap_or(usize::MAX))
        .ok_or_else(|| "ScreenCaptureKit 返回无效 AudioBuffer 数量".to_string())?;
    let non_interleaved = asbd.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0;
    let channels = u16::try_from(asbd.mChannelsPerFrame)
        .map_err(|_| "ScreenCaptureKit 音频声道数超出 u16".to_string())?;
    let samples = copy_and_normalize_buffers(buffers, channels, frame_count, non_interleaved)
        .map_err(|error| error.to_string())?;
    Ok((pts.value, pts.timescale, frame_count, samples))
}

fn validate_audio_format(asbd: &AudioStreamBasicDescription) -> Result<(), String> {
    if asbd.mSampleRate != f64::from(AUDIO_SAMPLE_RATE_HZ)
        || asbd.mFormatID != kAudioFormatLinearPCM
        || !(1..=2).contains(&asbd.mChannelsPerFrame)
        || asbd.mBitsPerChannel != 32
        || asbd.mFormatFlags & kAudioFormatFlagIsFloat == 0
        || asbd.mFormatFlags & kAudioFormatFlagIsPacked == 0
        || asbd.mFormatFlags & kAudioFormatFlagIsBigEndian != 0
    {
        return Err(format!(
            "ScreenCaptureKit 返回不支持的 PCM 格式: rate={}, format=0x{:08X}, flags=0x{:08X}, channels={}, bits={}",
            asbd.mSampleRate,
            asbd.mFormatID,
            asbd.mFormatFlags,
            asbd.mChannelsPerFrame,
            asbd.mBitsPerChannel
        ));
    }
    let non_interleaved = asbd.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0;
    let expected_bytes_per_frame = if non_interleaved {
        BYTES_PER_FLOAT_SAMPLE
    } else {
        usize::try_from(asbd.mChannelsPerFrame)
            .ok()
            .and_then(|channels| channels.checked_mul(BYTES_PER_FLOAT_SAMPLE))
            .ok_or_else(|| "ScreenCaptureKit PCM bytes/frame 溢出".to_string())?
    };
    if usize::try_from(asbd.mBytesPerFrame).ok() != Some(expected_bytes_per_frame)
        || asbd.mFramesPerPacket != 1
        || usize::try_from(asbd.mBytesPerPacket).ok() != Some(expected_bytes_per_frame)
    {
        return Err("ScreenCaptureKit 返回不支持的 PCM frame/packet 布局".to_string());
    }
    Ok(())
}

fn copy_and_normalize_buffers(
    buffers: &[AudioBuffer],
    channels: u16,
    frame_count: u32,
    non_interleaved: bool,
) -> Result<Box<[f32]>, MacAudioContractError> {
    let frames =
        usize::try_from(frame_count).map_err(|_| MacAudioContractError::SampleLengthOverflow)?;
    if non_interleaved {
        if buffers.len() != usize::from(channels) {
            return Err(MacAudioContractError::InvalidBufferLayout);
        }
        let mut planes = Vec::with_capacity(buffers.len());
        for buffer in buffers {
            if buffer.mNumberChannels != 1 {
                return Err(MacAudioContractError::InvalidBufferLayout);
            }
            planes.push(unsafe { audio_buffer_samples(buffer, frames)? });
        }
        normalize_float_pcm(
            NativeFloatPcm::NonInterleaved(&planes),
            channels,
            frame_count,
        )
    } else {
        if buffers.len() != 1 || buffers[0].mNumberChannels != u32::from(channels) {
            return Err(MacAudioContractError::InvalidBufferLayout);
        }
        let sample_count = frames
            .checked_mul(usize::from(channels))
            .ok_or(MacAudioContractError::SampleLengthOverflow)?;
        let samples = unsafe { audio_buffer_samples(&buffers[0], sample_count)? };
        normalize_float_pcm(NativeFloatPcm::Interleaved(samples), channels, frame_count)
    }
}

unsafe fn audio_buffer_samples<'a>(
    buffer: &'a AudioBuffer,
    expected_samples: usize,
) -> Result<&'a [f32], MacAudioContractError> {
    let expected_bytes = expected_samples
        .checked_mul(BYTES_PER_FLOAT_SAMPLE)
        .ok_or(MacAudioContractError::SampleLengthOverflow)?;
    if usize::try_from(buffer.mDataByteSize).ok() != Some(expected_bytes)
        || buffer.mData.is_null()
        || (buffer.mData as usize) % align_of::<f32>() != 0
    {
        return Err(MacAudioContractError::SampleLengthMismatch);
    }
    Ok(unsafe { slice::from_raw_parts(buffer.mData.cast::<f32>(), expected_samples) })
}
