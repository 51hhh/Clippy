//! Windows WASAPI 系统声与默认麦克风音频源。
//!
//! 本模块只进入 `recording-windows-audio` QA feature。COM 与全部 WASAPI 对象都在音频
//! worker 线程内创建和销毁；只有 `recording-windows-av-qa` 组合 feature 才会把它接入双轨会话。

use super::super::audio::CapturedAudioChunk;
use super::super::audio_worker::RecordingAudioSource;
use super::super::clock::RecordingSessionClock;
use super::windows_audio_contract::{
    packet_to_chunks, safe_control_timestamp, QpcClockMapper, WindowsAudioContractError,
    WindowsAudioEndpointFlow, WindowsAudioSourceKind, WASAPI_CHANNELS,
};
use std::collections::VecDeque;
use std::ptr;
use std::time::Duration;
use thiserror::Error;
use windows::core::{Error as WindowsError, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_NOPERSIST,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
};
use windows::Win32::Media::Multimedia::WAVE_FORMAT_IEEE_FLOAT;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

const WASAPI_BUFFER_DURATION_100NS: i64 = 200_000;

#[derive(Debug, Error)]
pub(in crate::recording) enum WindowsWasapiAudioSourceError {
    #[error("WASAPI {operation} 失败（HRESULT {code:#010x}）")]
    WindowsApi { operation: &'static str, code: i32 },
    #[error("WASAPI 音频事件等待失败（HRESULT {0:#010x}）")]
    WaitFailed(i32),
    #[error("WASAPI 返回了无效的音频 packet 指针")]
    InvalidPacketPointer,
    #[error("WASAPI packet 时间戳无效")]
    InvalidPacketTimestamp,
    #[error("WASAPI packet 与上一 packet 的时间区间重叠")]
    PacketTimelineOverlap,
    #[error("WASAPI 音频流已经停止")]
    StreamStopped,
    #[error(transparent)]
    Contract(#[from] WindowsAudioContractError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::recording) struct WindowsWasapiAudioSourcePlan {
    kind: WindowsAudioSourceKind,
}

impl WindowsWasapiAudioSourcePlan {
    pub const fn system_loopback() -> Self {
        Self {
            kind: WindowsAudioSourceKind::SystemLoopback,
        }
    }

    pub const fn default_microphone() -> Self {
        Self {
            kind: WindowsAudioSourceKind::DefaultMicrophone,
        }
    }

    pub const fn channels(self) -> u16 {
        WASAPI_CHANNELS
    }

    pub fn connect(
        self,
        clock: RecordingSessionClock,
    ) -> Result<WindowsWasapiAudioSource, WindowsWasapiAudioSourceError> {
        WindowsWasapiAudioSource::connect(self.kind, clock)
    }
}

pub(in crate::recording) struct WindowsWasapiAudioSource {
    capture_client: IAudioCaptureClient,
    audio_client: IAudioClient,
    event: OwnedEvent,
    clock: RecordingSessionClock,
    mapper: QpcClockMapper,
    pending: VecDeque<CapturedAudioChunk>,
    next_sequence: u64,
    last_packet_end_ns: Option<u64>,
    not_before_ns: u64,
    running: bool,
    // 必须最后销毁：前面的 COM interface 字段先 Drop，再执行 CoUninitialize。
    _com: ComApartment,
}

impl WindowsWasapiAudioSource {
    fn connect(
        kind: WindowsAudioSourceKind,
        clock: RecordingSessionClock,
    ) -> Result<Self, WindowsWasapiAudioSourceError> {
        let com = ComApartment::initialize()?;
        let enumerator: IMMDeviceEnumerator = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|error| windows_api_error("创建 endpoint enumerator", error))?
        };
        let data_flow = match kind.endpoint_flow() {
            WindowsAudioEndpointFlow::Render => eRender,
            WindowsAudioEndpointFlow::Capture => eCapture,
        };
        let device = unsafe {
            enumerator
                .GetDefaultAudioEndpoint(data_flow, eConsole)
                .map_err(|error| windows_api_error("取得默认 endpoint", error))?
        };
        let audio_client: IAudioClient = unsafe {
            device
                .Activate(CLSCTX_ALL, None)
                .map_err(|error| windows_api_error("激活 IAudioClient", error))?
        };
        let format = normalized_wave_format();
        let stream_flags = stream_flags(kind);
        unsafe {
            audio_client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    stream_flags,
                    WASAPI_BUFFER_DURATION_100NS,
                    0,
                    &format,
                    None,
                )
                .map_err(|error| windows_api_error("初始化 shared stream", error))?;
        }
        let event = OwnedEvent::create()?;
        unsafe {
            audio_client
                .SetEventHandle(event.raw())
                .map_err(|error| windows_api_error("绑定 capture event", error))?;
        }
        let capture_client: IAudioCaptureClient = unsafe {
            audio_client
                .GetService()
                .map_err(|error| windows_api_error("取得 IAudioCaptureClient", error))?
        };
        let mapper = calibrate_qpc(&clock)?;
        unsafe {
            audio_client
                .Start()
                .map_err(|error| windows_api_error("启动 capture stream", error))?;
        }

        Ok(Self {
            capture_client,
            audio_client,
            event,
            clock,
            mapper,
            pending: VecDeque::new(),
            next_sequence: 0,
            last_packet_end_ns: None,
            not_before_ns: 0,
            running: true,
            _com: com,
        })
    }

    fn read_packet(&mut self) -> Result<(), WindowsWasapiAudioSourceError> {
        let mut data = ptr::null_mut();
        let mut frame_count = 0_u32;
        let mut flags = 0_u32;
        let mut qpc_position_100ns = 0_u64;
        unsafe {
            self.capture_client
                .GetBuffer(
                    &mut data,
                    &mut frame_count,
                    &mut flags,
                    None,
                    Some(&mut qpc_position_100ns),
                )
                .map_err(|error| windows_api_error("读取 capture packet", error))?;
        }

        let packet_result = self.copy_packet(data, frame_count, flags, qpc_position_100ns);
        let release_result = unsafe {
            self.capture_client
                .ReleaseBuffer(frame_count)
                .map_err(|error| windows_api_error("释放 capture packet", error))
        };
        match (packet_result, release_result) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn copy_packet(
        &mut self,
        data: *mut u8,
        frame_count: u32,
        flags: u32,
        qpc_position_100ns: u64,
    ) -> Result<(), WindowsWasapiAudioSourceError> {
        if flags & (AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32) != 0 {
            return Err(WindowsWasapiAudioSourceError::InvalidPacketTimestamp);
        }
        let captured_at_ns = self.mapper.map_100ns(qpc_position_100ns)?;
        let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
        let sample_count = usize::try_from(frame_count)
            .ok()
            .and_then(|frames| frames.checked_mul(usize::from(WASAPI_CHANNELS)))
            .ok_or(WindowsAudioContractError::SampleLengthOverflow)?;
        let samples = if silent {
            None
        } else {
            if data.is_null() {
                return Err(WindowsWasapiAudioSourceError::InvalidPacketPointer);
            }
            Some(unsafe { std::slice::from_raw_parts(data.cast::<f32>(), sample_count) })
        };
        let packet = packet_to_chunks(self.next_sequence, captured_at_ns, frame_count, samples)?;
        if self
            .last_packet_end_ns
            .is_some_and(|last_end_ns| captured_at_ns < last_end_ns)
        {
            return Err(WindowsWasapiAudioSourceError::PacketTimelineOverlap);
        }
        self.next_sequence = packet.next_sequence;
        self.last_packet_end_ns = Some(packet.end_ns);
        if captured_at_ns >= self.not_before_ns {
            self.pending.extend(packet.chunks);
        }
        Ok(())
    }

    fn next_packet_available(&self) -> Result<bool, WindowsWasapiAudioSourceError> {
        let frames = unsafe {
            self.capture_client
                .GetNextPacketSize()
                .map_err(|error| windows_api_error("查询 capture packet", error))?
        };
        Ok(frames > 0)
    }

    fn stop_and_reset(&mut self) -> Result<(), WindowsWasapiAudioSourceError> {
        if self.running {
            unsafe {
                self.audio_client
                    .Stop()
                    .map_err(|error| windows_api_error("停止 capture stream", error))?;
                self.audio_client
                    .Reset()
                    .map_err(|error| windows_api_error("重置 capture stream", error))?;
            }
            self.running = false;
        }
        self.pending.clear();
        Ok(())
    }

    fn safe_control_timestamp(&self) -> u64 {
        safe_control_timestamp(self.clock.now_ns(), self.last_packet_end_ns)
    }
}

impl RecordingAudioSource for WindowsWasapiAudioSource {
    type Error = WindowsWasapiAudioSourceError;

    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        if !self.running {
            return Err(WindowsWasapiAudioSourceError::StreamStopped);
        }
        if let Some(chunk) = self.pending.pop_front() {
            return Ok(Some(chunk));
        }
        if !self.next_packet_available()? {
            match unsafe { WaitForSingleObject(self.event.raw(), duration_to_wait_ms(timeout)) } {
                WAIT_OBJECT_0 => {}
                WAIT_TIMEOUT => return Ok(None),
                WAIT_FAILED => {
                    return Err(WindowsWasapiAudioSourceError::WaitFailed(
                        WindowsError::from_thread().code().0,
                    ));
                }
                result => {
                    return Err(WindowsWasapiAudioSourceError::WaitFailed(result.0 as i32));
                }
            }
        }
        if !self.next_packet_available()? {
            return Ok(None);
        }
        self.read_packet()?;
        Ok(self.pending.pop_front())
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(self.safe_control_timestamp())
    }

    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        self.stop_and_reset()?;
        Ok(self.safe_control_timestamp())
    }

    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        if !self.running {
            unsafe {
                self.audio_client
                    .Start()
                    .map_err(|error| windows_api_error("恢复 capture stream", error))?;
            }
            self.running = true;
        }
        let resumed_at_ns = self.safe_control_timestamp();
        self.not_before_ns = resumed_at_ns;
        Ok(resumed_at_ns)
    }

    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.stop_and_reset()?;
        Ok(self.safe_control_timestamp())
    }
}

impl Drop for WindowsWasapiAudioSource {
    fn drop(&mut self) {
        let _ = self.stop_and_reset();
    }
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, WindowsWasapiAudioSourceError> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|error| windows_api_error("初始化 COM apartment", error))?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct OwnedEvent(HANDLE);

impl OwnedEvent {
    fn create() -> Result<Self, WindowsWasapiAudioSourceError> {
        let event = unsafe { CreateEventW(None, false, false, PCWSTR::null()) }
            .map_err(|error| windows_api_error("创建 capture event", error))?;
        Ok(Self(event))
    }

    const fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedEvent {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn normalized_wave_format() -> WAVEFORMATEX {
    let block_align = WASAPI_CHANNELS * 4;
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
        nChannels: WASAPI_CHANNELS,
        nSamplesPerSec: super::super::audio::AUDIO_SAMPLE_RATE_HZ,
        nAvgBytesPerSec: super::super::audio::AUDIO_SAMPLE_RATE_HZ * u32::from(block_align),
        nBlockAlign: block_align,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

fn stream_flags(kind: WindowsAudioSourceKind) -> u32 {
    let common = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
        | AUDCLNT_STREAMFLAGS_NOPERSIST;
    if kind.uses_loopback() {
        common | AUDCLNT_STREAMFLAGS_LOOPBACK
    } else {
        common
    }
}

fn calibrate_qpc(
    clock: &RecordingSessionClock,
) -> Result<QpcClockMapper, WindowsWasapiAudioSourceError> {
    let mut frequency = 0_i64;
    let mut counter = 0_i64;
    unsafe {
        QueryPerformanceFrequency(&mut frequency)
            .map_err(|error| windows_api_error("读取 QPC frequency", error))?;
    }
    let before_ns = clock.now_ns();
    unsafe {
        QueryPerformanceCounter(&mut counter)
            .map_err(|error| windows_api_error("读取 QPC counter", error))?;
    }
    let after_ns = clock.now_ns();
    Ok(QpcClockMapper::from_calibration(
        counter, frequency, before_ns, after_ns,
    )?)
}

fn duration_to_wait_ms(timeout: Duration) -> u32 {
    if timeout.is_zero() {
        return 0;
    }
    timeout.as_millis().clamp(1, u128::from(u32::MAX)) as u32
}

fn windows_api_error(
    operation: &'static str,
    error: WindowsError,
) -> WindowsWasapiAudioSourceError {
    WindowsWasapiAudioSourceError::WindowsApi {
        operation,
        code: error.code().0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_format_is_normalized_stereo_float() {
        let format = normalized_wave_format();
        let format_tag = format.wFormatTag;
        let channels = format.nChannels;
        let sample_rate = format.nSamplesPerSec;
        let block_align = format.nBlockAlign;
        let average_bytes = format.nAvgBytesPerSec;
        let bits_per_sample = format.wBitsPerSample;
        assert_eq!(format_tag, WAVE_FORMAT_IEEE_FLOAT as u16);
        assert_eq!(channels, 2);
        assert_eq!(sample_rate, 48_000);
        assert_eq!(block_align, 8);
        assert_eq!(average_bytes, 384_000);
        assert_eq!(bits_per_sample, 32);
    }

    #[test]
    fn loopback_flag_is_only_added_for_system_sound() {
        let system = stream_flags(WindowsAudioSourceKind::SystemLoopback);
        let microphone = stream_flags(WindowsAudioSourceKind::DefaultMicrophone);

        assert_ne!(system & AUDCLNT_STREAMFLAGS_LOOPBACK, 0);
        assert_eq!(microphone & AUDCLNT_STREAMFLAGS_LOOPBACK, 0);
        assert_ne!(microphone & AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 0);
        assert_ne!(microphone & AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, 0);
    }

    #[test]
    fn wait_timeout_rounds_up_sub_millisecond_values() {
        assert_eq!(duration_to_wait_ms(Duration::ZERO), 0);
        assert_eq!(duration_to_wait_ms(Duration::from_nanos(1)), 1);
        assert_eq!(duration_to_wait_ms(Duration::from_millis(50)), 50);
    }
}
