//! WASAPI packet 到录屏 PCM 块的纯函数合同。
//!
//! 这里不引用 Windows API，因此 Linux 本地门禁也能验证 QPC 映射、静音和拆块边界。

use super::super::audio::{
    AudioFormat, AudioTimestampPrecision, CapturedAudioChunk, AUDIO_SAMPLE_RATE_HZ,
};
use std::collections::VecDeque;
use thiserror::Error;

pub(super) const WASAPI_CHANNELS: u16 = 2;
pub(super) const WASAPI_CHUNK_FRAMES: u32 = AUDIO_SAMPLE_RATE_HZ / 50;
const HUNDRED_NS_PER_SECOND: u128 = 10_000_000;
const NANOS_PER_HUNDRED_NS: u64 = 100;
const NANOS_PER_SECOND: u64 = 1_000_000_000;

pub(super) const WASAPI_TIMESTAMP_PRECISION: AudioTimestampPrecision =
    AudioTimestampPrecision::HundredNanoseconds;

pub(super) fn packet_timestamp_is_valid(start_ns: u64, previous_end_ns: Option<u64>) -> bool {
    previous_end_ns.is_none_or(|end| {
        end.saturating_sub(start_ns) <= WASAPI_TIMESTAMP_PRECISION.overlap_allowance_ns()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowsAudioSourceKind {
    SystemLoopback,
    DefaultMicrophone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowsAudioEndpointFlow {
    Render,
    Capture,
}

impl WindowsAudioSourceKind {
    pub const fn uses_loopback(self) -> bool {
        matches!(self, Self::SystemLoopback)
    }

    pub const fn endpoint_flow(self) -> WindowsAudioEndpointFlow {
        match self {
            Self::SystemLoopback => WindowsAudioEndpointFlow::Render,
            Self::DefaultMicrophone => WindowsAudioEndpointFlow::Capture,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(in crate::recording) enum WindowsAudioContractError {
    #[error("WASAPI QPC 频率无效")]
    InvalidQpcFrequency,
    #[error("WASAPI QPC 计数器无效")]
    InvalidQpcCounter,
    #[error("WASAPI 时间戳无法映射到录屏会话")]
    TimestampOutOfRange,
    #[error("WASAPI packet 不能为空")]
    EmptyPacket,
    #[error("WASAPI packet 样本长度溢出")]
    SampleLengthOverflow,
    #[error("WASAPI packet 样本长度不匹配")]
    SampleLengthMismatch,
    #[error("WASAPI packet 序号耗尽")]
    SequenceExhausted,
    #[error("WASAPI 停止尾包超过 endpoint 实际帧容量")]
    StopDrainBudgetExceeded,
    #[error("WASAPI 控制时间戳已经耗尽")]
    ControlTimestampExhausted,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StopTailMode {
    Discard,
    Preserve,
}

/// 原生对象留在音频线程；这个入口允许离线测试控制、排空和清理顺序。
pub(super) trait WasapiStopEndpoint {
    type Error: From<WindowsAudioContractError>;

    fn stop_stream(&mut self) -> Result<(), Self::Error>;
    fn next_packet_frames(&mut self) -> Result<u32, Self::Error>;
    fn read_packet(&mut self) -> Result<(), Self::Error>;
    fn reset_stream(&mut self) -> Result<(), Self::Error>;
    fn discard_pending(&mut self);
}

pub(super) fn stop_endpoint<S: WasapiStopEndpoint>(
    source: &mut S,
    mode: StopTailMode,
    buffer_frames: u32,
) -> Result<(), S::Error> {
    source.stop_stream()?;
    if mode == StopTailMode::Preserve {
        // Stop 后不再产生新包；以实际 endpoint 容量约束排空，不能无界轮询。
        let mut remaining_frames = buffer_frames;
        loop {
            let frames = source.next_packet_frames()?;
            if frames == 0 {
                break;
            }
            remaining_frames = remaining_frames
                .checked_sub(frames)
                .ok_or(WindowsAudioContractError::StopDrainBudgetExceeded)?;
            source.read_packet()?;
        }
    }
    source.reset_stream()?;
    if mode == StopTailMode::Discard {
        source.discard_pending();
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct QpcClockMapper {
    qpc_anchor_100ns: u64,
    session_anchor_ns: u64,
}

impl QpcClockMapper {
    pub fn from_calibration(
        qpc_counter: i64,
        qpc_frequency: i64,
        session_before_ns: u64,
        session_after_ns: u64,
    ) -> Result<Self, WindowsAudioContractError> {
        let qpc_anchor_100ns = qpc_ticks_to_100ns(qpc_counter, qpc_frequency)?;
        let elapsed = session_after_ns.saturating_sub(session_before_ns);
        let session_anchor_ns = session_before_ns.saturating_add(elapsed / 2);
        Ok(Self {
            qpc_anchor_100ns,
            session_anchor_ns,
        })
    }

    pub fn map_100ns(self, qpc_position_100ns: u64) -> Result<u64, WindowsAudioContractError> {
        if qpc_position_100ns >= self.qpc_anchor_100ns {
            let delta_ns = qpc_position_100ns
                .checked_sub(self.qpc_anchor_100ns)
                .and_then(|delta| delta.checked_mul(NANOS_PER_HUNDRED_NS))
                .ok_or(WindowsAudioContractError::TimestampOutOfRange)?;
            self.session_anchor_ns
                .checked_add(delta_ns)
                .ok_or(WindowsAudioContractError::TimestampOutOfRange)
        } else {
            let delta_ns = self
                .qpc_anchor_100ns
                .checked_sub(qpc_position_100ns)
                .and_then(|delta| delta.checked_mul(NANOS_PER_HUNDRED_NS))
                .ok_or(WindowsAudioContractError::TimestampOutOfRange)?;
            self.session_anchor_ns
                .checked_sub(delta_ns)
                .ok_or(WindowsAudioContractError::TimestampOutOfRange)
        }
    }
}

fn qpc_ticks_to_100ns(
    qpc_counter: i64,
    qpc_frequency: i64,
) -> Result<u64, WindowsAudioContractError> {
    let counter =
        u128::try_from(qpc_counter).map_err(|_| WindowsAudioContractError::InvalidQpcCounter)?;
    let frequency = u128::try_from(qpc_frequency)
        .ok()
        .filter(|frequency| *frequency > 0)
        .ok_or(WindowsAudioContractError::InvalidQpcFrequency)?;
    let value = counter
        .checked_mul(HUNDRED_NS_PER_SECOND)
        .ok_or(WindowsAudioContractError::TimestampOutOfRange)?
        / frequency;
    u64::try_from(value).map_err(|_| WindowsAudioContractError::TimestampOutOfRange)
}

pub(super) struct PacketChunks {
    pub chunks: VecDeque<CapturedAudioChunk>,
    pub next_sequence: u64,
    pub end_ns: u64,
}

pub(super) fn packet_to_chunks(
    first_sequence: u64,
    captured_at_ns: u64,
    frame_count: u32,
    samples: Option<&[f32]>,
) -> Result<PacketChunks, WindowsAudioContractError> {
    if frame_count == 0 {
        return Err(WindowsAudioContractError::EmptyPacket);
    }
    let sample_count = usize::try_from(frame_count)
        .ok()
        .and_then(|frames| frames.checked_mul(usize::from(WASAPI_CHANNELS)))
        .ok_or(WindowsAudioContractError::SampleLengthOverflow)?;
    if samples.is_some_and(|samples| samples.len() != sample_count) {
        return Err(WindowsAudioContractError::SampleLengthMismatch);
    }

    let mut chunks = VecDeque::new();
    let mut frame_offset = 0_u32;
    let mut sequence = first_sequence;
    while frame_offset < frame_count {
        let chunk_frames = (frame_count - frame_offset).min(WASAPI_CHUNK_FRAMES);
        let chunk_start = usize::try_from(frame_offset)
            .ok()
            .and_then(|offset| offset.checked_mul(usize::from(WASAPI_CHANNELS)))
            .ok_or(WindowsAudioContractError::SampleLengthOverflow)?;
        let chunk_samples = usize::try_from(chunk_frames)
            .ok()
            .and_then(|frames| frames.checked_mul(usize::from(WASAPI_CHANNELS)))
            .ok_or(WindowsAudioContractError::SampleLengthOverflow)?;
        let owned_samples = match samples {
            Some(samples) => samples[chunk_start..chunk_start + chunk_samples]
                .to_vec()
                .into_boxed_slice(),
            None => vec![0.0; chunk_samples].into_boxed_slice(),
        };
        chunks.push_back(CapturedAudioChunk {
            sequence,
            captured_at_ns: captured_at_ns
                .checked_add(frames_to_ns(frame_offset)?)
                .ok_or(WindowsAudioContractError::TimestampOutOfRange)?,
            format: AudioFormat::normalized(WASAPI_CHANNELS),
            frame_count: chunk_frames,
            samples: owned_samples,
        });
        sequence = sequence
            .checked_add(1)
            .ok_or(WindowsAudioContractError::SequenceExhausted)?;
        frame_offset += chunk_frames;
    }

    let end_ns = captured_at_ns
        .checked_add(frames_to_ns(frame_count)?)
        .ok_or(WindowsAudioContractError::TimestampOutOfRange)?;
    Ok(PacketChunks {
        chunks,
        next_sequence: sequence,
        end_ns,
    })
}

/// 只裁切已复制 PCM 的前缀；原 QPC、之后完整块与 packet 序号/末尾保持。
pub(super) fn packet_after_activation(
    mut packet: PacketChunks,
    not_before_ns: u64,
) -> Result<PacketChunks, WindowsAudioContractError> {
    while let Some(chunk) = packet.chunks.front_mut() {
        if chunk.captured_at_ns >= not_before_ns {
            break;
        }
        // 首个合法样本必须不早于下界；u128 避免大时间差乘采样率溢出。
        let delta_ns = not_before_ns - chunk.captured_at_ns;
        let leading_frames = (u128::from(delta_ns) * u128::from(AUDIO_SAMPLE_RATE_HZ))
            .div_ceil(u128::from(NANOS_PER_SECOND));
        if leading_frames >= u128::from(chunk.frame_count) {
            packet.chunks.pop_front();
            continue;
        }
        // 上面的帧数比较保证转换有界；格式与样本形状已由 packet_to_chunks 固定。
        let leading_frames = leading_frames as u32;
        let leading_samples = usize::try_from(leading_frames)
            .ok()
            .and_then(|frames| frames.checked_mul(usize::from(chunk.format.channels)))
            .ok_or(WindowsAudioContractError::SampleLengthOverflow)?;
        let samples = chunk
            .samples
            .get(leading_samples..)
            .ok_or(WindowsAudioContractError::SampleLengthMismatch)?;
        let retained_start = chunk
            .captured_at_ns
            .checked_add(frames_to_ns(leading_frames)?)
            .ok_or(WindowsAudioContractError::TimestampOutOfRange)?;
        chunk.samples = samples.to_vec().into_boxed_slice();
        chunk.frame_count -= leading_frames;
        chunk.captured_at_ns = retained_start;
        break;
    }
    Ok(packet)
}

fn frames_to_ns(frame_count: u32) -> Result<u64, WindowsAudioContractError> {
    u64::from(frame_count)
        .checked_mul(NANOS_PER_SECOND)
        .map(|value| value / u64::from(AUDIO_SAMPLE_RATE_HZ))
        .ok_or(WindowsAudioContractError::TimestampOutOfRange)
}

pub(super) fn safe_control_timestamp(current_ns: u64, last_packet_end_ns: Option<u64>) -> u64 {
    current_ns.max(last_packet_end_ns.unwrap_or_default())
}

/// packet 末尾可能抬高控制下界；控制操作按 1 ns 排序，不修改任何原生 PCM PTS。
#[derive(Debug, Default)]
pub(super) struct WasapiControlTimeline {
    last_control_ns: Option<u64>,
}

impl WasapiControlTimeline {
    pub fn next_timestamp(
        &mut self,
        current_ns: u64,
        last_packet_end_ns: Option<u64>,
    ) -> Result<u64, WindowsAudioContractError> {
        let mut timestamp = safe_control_timestamp(current_ns, last_packet_end_ns);
        if let Some(last) = self.last_control_ns {
            timestamp = timestamp.max(
                last.checked_add(1)
                    .ok_or(WindowsAudioContractError::ControlTimestampExhausted)?,
            );
        }
        self.last_control_ns = Some(timestamp);
        Ok(timestamp)
    }
}

#[cfg(test)]
mod tests {
    mod activation_tail_tests {
        use super::*;
        include!("windows_audio_contract/activation_tail_tests.rs");
    }

    mod qpc_precision_tests {
        use super::*;
        include!("windows_audio_contract/qpc_precision_tests.rs");
    }

    mod qpc_precision_diagnostic_tests {
        use super::*;
        include!("windows_audio_contract/qpc_precision_diagnostic_tests.rs");
    }

    mod control_clock_tests {
        use super::*;
        include!("windows_audio_contract/control_clock_tests.rs");
    }

    mod activation_boundary_tests {
        use super::*;
        include!("windows_audio_contract/activation_boundary_tests.rs");
    }

    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    enum StopTestError {
        Api(&'static str),
        Contract(WindowsAudioContractError),
    }

    impl From<WindowsAudioContractError> for StopTestError {
        fn from(error: WindowsAudioContractError) -> Self {
            Self::Contract(error)
        }
    }

    struct StopFixture {
        running: bool,
        native: VecDeque<(u32, u64, Option<f32>)>,
        pending: VecDeque<CapturedAudioChunk>,
        next_sequence: u64,
        last_end_ns: u64,
        events: Vec<&'static str>,
        fail_at: Option<&'static str>,
        repeat_packet: bool,
    }

    impl StopFixture {
        fn with_tail() -> Self {
            let samples = vec![0.25; 1920 * 2];
            let mut copied = packet_to_chunks(7, 0, 1920, Some(&samples)).unwrap();
            copied.chunks.pop_front();
            Self {
                running: true,
                native: VecDeque::from([(17, 40_000_000, Some(0.75)), (480, 40_354_167, None)]),
                pending: copied.chunks,
                next_sequence: copied.next_sequence,
                last_end_ns: copied.end_ns,
                events: Vec::new(),
                fail_at: None,
                repeat_packet: false,
            }
        }

        fn operation(&mut self, name: &'static str) -> Result<(), StopTestError> {
            self.events.push(name);
            if self.fail_at == Some(name) {
                Err(StopTestError::Api(name))
            } else {
                Ok(())
            }
        }
    }

    impl WasapiStopEndpoint for StopFixture {
        type Error = StopTestError;

        fn stop_stream(&mut self) -> Result<(), Self::Error> {
            self.operation("stop")?;
            self.running = false;
            Ok(())
        }

        fn next_packet_frames(&mut self) -> Result<u32, Self::Error> {
            self.operation("query")?;
            assert!(!self.running);
            Ok(if self.repeat_packet {
                1
            } else {
                self.native.front().map_or(0, |packet| packet.0)
            })
        }

        fn read_packet(&mut self) -> Result<(), Self::Error> {
            self.operation("read")?;
            assert!(!self.running);
            let (frames, timestamp, value) = if self.repeat_packet {
                (1, self.last_end_ns, Some(0.5))
            } else {
                self.native.pop_front().unwrap()
            };
            let samples = value.map(|value| vec![value; frames as usize * 2]);
            let packet =
                packet_to_chunks(self.next_sequence, timestamp, frames, samples.as_deref())?;
            self.pending.extend(packet.chunks);
            self.next_sequence = packet.next_sequence;
            self.last_end_ns = packet.end_ns;
            Ok(())
        }

        fn reset_stream(&mut self) -> Result<(), Self::Error> {
            self.operation("reset")?;
            self.native.clear();
            Ok(())
        }

        fn discard_pending(&mut self) {
            self.events.push("discard");
            self.pending.clear();
        }
    }

    #[test]
    fn normal_stop_retains_split_pending_and_endpoint_tail() {
        let mut source = StopFixture::with_tail();
        stop_endpoint(&mut source, StopTailMode::Preserve, 497).unwrap();
        assert_eq!(source.pending.len(), 3);
        assert_eq!(
            source
                .pending
                .iter()
                .map(|chunk| chunk.sequence)
                .collect::<Vec<_>>(),
            [8, 9, 10]
        );
        assert_eq!(source.pending[0].captured_at_ns, 20_000_000);
        assert_eq!(source.pending[0].frame_count, 960);
        assert_eq!(source.pending[0].samples.as_ref(), vec![0.25; 1920]);
        assert_eq!(source.pending[1].captured_at_ns, 40_000_000);
        assert_eq!(source.pending[1].samples.as_ref(), vec![0.75; 34]);
        assert_eq!(source.pending[2].captured_at_ns, 40_354_167);
        assert_eq!(source.pending[2].samples.as_ref(), vec![0.0; 960]);
        assert_eq!(source.last_end_ns, 50_354_167);
        assert_eq!(source.next_sequence, 11);
        assert_eq!(
            source.events,
            ["stop", "query", "read", "query", "read", "query", "reset"]
        );
    }

    #[test]
    fn pause_discards_pending_without_reading_native_tail() {
        let mut source = StopFixture::with_tail();
        stop_endpoint(&mut source, StopTailMode::Discard, 0).unwrap();
        assert!(source.pending.is_empty());
        assert!(source.native.is_empty());
        assert!(!source.running);
        assert_eq!(source.events, ["stop", "reset", "discard"]);
    }

    #[test]
    fn stop_drain_never_exceeds_actual_endpoint_capacity() {
        for (capacity, repeated, expected_reads) in [(0, false, 0), (17, false, 1), (2, true, 2)] {
            let mut source = StopFixture::with_tail();
            source.repeat_packet = repeated;
            assert_eq!(
                stop_endpoint(&mut source, StopTailMode::Preserve, capacity),
                Err(StopTestError::Contract(
                    WindowsAudioContractError::StopDrainBudgetExceeded
                ))
            );
            assert_eq!(
                source
                    .events
                    .iter()
                    .filter(|event| **event == "read")
                    .count(),
                expected_reads
            );
            assert!(!source.events.contains(&"reset"));
            assert!(!source.events.contains(&"discard"));
        }
    }

    #[test]
    fn normal_stop_with_no_endpoint_packets_keeps_copied_pcm() {
        let mut source = StopFixture::with_tail();
        source.native.clear();
        stop_endpoint(&mut source, StopTailMode::Preserve, 0).unwrap();
        assert_eq!(source.pending.len(), 1);
        assert_eq!(source.pending[0].samples.as_ref(), vec![0.25; 1920]);
        assert_eq!(source.last_end_ns, 40_000_000);
        assert_eq!(source.events, ["stop", "query", "reset"]);
    }

    #[test]
    fn stop_drain_errors_never_report_success_or_discard_pcm() {
        for operation in ["stop", "query", "read", "reset"] {
            let mut source = StopFixture::with_tail();
            source.fail_at = Some(operation);
            assert_eq!(
                stop_endpoint(&mut source, StopTailMode::Preserve, 497),
                Err(StopTestError::Api(operation))
            );
            assert_eq!(source.events.last(), Some(&operation));
            assert!(!source.events.contains(&"discard"));
            assert!(!source.pending.is_empty());
        }
    }

    #[test]
    fn paused_stop_with_no_tail_succeeds_with_zero_capacity() {
        let mut source = StopFixture::with_tail();
        stop_endpoint(&mut source, StopTailMode::Discard, 0).unwrap();
        stop_endpoint(&mut source, StopTailMode::Preserve, 0).unwrap();
        assert!(!source.running);
        assert!(source.pending.is_empty());
        assert!(source.native.is_empty());
    }

    #[test]
    fn source_kind_only_enables_loopback_for_system_sound() {
        assert!(WindowsAudioSourceKind::SystemLoopback.uses_loopback());
        assert!(!WindowsAudioSourceKind::DefaultMicrophone.uses_loopback());
        assert_eq!(
            WindowsAudioSourceKind::SystemLoopback.endpoint_flow(),
            WindowsAudioEndpointFlow::Render
        );
        assert_eq!(
            WindowsAudioSourceKind::DefaultMicrophone.endpoint_flow(),
            WindowsAudioEndpointFlow::Capture
        );
    }

    #[test]
    fn qpc_mapper_uses_midpoint_and_maps_both_directions() {
        let mapper = QpcClockMapper::from_calibration(25_000, 10_000, 8_000, 8_200).unwrap();

        assert_eq!(mapper.map_100ns(25_001_000).unwrap(), 108_100);
        assert_eq!(mapper.map_100ns(24_999_919).unwrap(), 0);
    }

    #[test]
    fn qpc_mapper_rejects_invalid_frequency_and_underflow() {
        assert_eq!(
            QpcClockMapper::from_calibration(1, 0, 0, 0),
            Err(WindowsAudioContractError::InvalidQpcFrequency)
        );
        let mapper = QpcClockMapper::from_calibration(10, 10, 0, 0).unwrap();
        assert_eq!(
            mapper.map_100ns(0),
            Err(WindowsAudioContractError::TimestampOutOfRange)
        );
    }

    #[test]
    fn packet_is_split_on_twenty_millisecond_boundaries() {
        let frames = WASAPI_CHUNK_FRAMES * 2 + 17;
        let samples = vec![0.25; frames as usize * usize::from(WASAPI_CHANNELS)];
        let packet = packet_to_chunks(7, 2_000_000, frames, Some(&samples)).unwrap();

        assert_eq!(packet.chunks.len(), 3);
        assert_eq!(packet.chunks[0].sequence, 7);
        assert_eq!(packet.chunks[0].frame_count, WASAPI_CHUNK_FRAMES);
        assert_eq!(packet.chunks[1].captured_at_ns, 22_000_000);
        assert_eq!(packet.chunks[2].frame_count, 17);
        assert_eq!(packet.next_sequence, 10);
        assert_eq!(packet.end_ns, 42_354_166);
    }

    #[test]
    fn silent_packet_allocates_exact_zeroed_pcm() {
        let packet = packet_to_chunks(0, 0, 480, None).unwrap();
        let chunk = &packet.chunks[0];

        assert_eq!(chunk.samples.len(), 960);
        assert!(chunk.samples.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn packet_rejects_empty_or_mismatched_samples() {
        assert!(matches!(
            packet_to_chunks(0, 0, 0, None),
            Err(WindowsAudioContractError::EmptyPacket)
        ));
        assert!(matches!(
            packet_to_chunks(0, 0, 4, Some(&[0.0; 7])),
            Err(WindowsAudioContractError::SampleLengthMismatch)
        ));
    }

    #[test]
    fn control_timestamp_never_precedes_the_last_pcm_frame() {
        assert_eq!(safe_control_timestamp(300, None), 300);
        assert_eq!(safe_control_timestamp(300, Some(250)), 300);
        assert_eq!(safe_control_timestamp(300, Some(450)), 450);
    }
}
