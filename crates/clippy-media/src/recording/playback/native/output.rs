#[cfg_attr(target_os = "linux", path = "output/pulse.rs")]
#[cfg_attr(target_os = "macos", path = "output/coreaudio.rs")]
#[cfg_attr(target_os = "windows", path = "output/system_cpal.rs")]
mod system;

use crate::recording::{
    audio::AUDIO_SAMPLE_RATE_HZ,
    mux::webm_remux::OPUS_PACKET_FRAMES,
    native_playback::{NativeWebmDecoder, PcmBlock},
};
use rtrb::{Consumer, Producer, RingBuffer};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
pub(super) use system::{open_device, OutputDevice};
use system::{prepare_stream, start_stream, OutputStream};

const QUEUE_FRAMES: usize = 8 * OPUS_PACKET_FRAMES;
const DEVICE_PROGRESS_TIMEOUT: Duration = Duration::from_secs(5);
type StereoFrame = [f32; 2];

pub(super) struct AudioOutput {
    _stream: OutputStream,
    queue: PcmQueue,
    started_at: Instant,
}

struct PcmQueue {
    producer: Producer<StereoFrame>,
    shared: Arc<OutputStatus>,
    first_frame: u64,
    queued_until: u64,
}

#[derive(Default)]
struct OutputStatus {
    tail: Mutex<Option<PlayedTail>>,
    input_ended: AtomicBool,
    underrun: AtomicU64,
    invalid_clock: AtomicBool,
    error: Mutex<Option<String>>,
}

#[derive(Clone, Copy)]
struct PlayedTail {
    end_frame: u64,
    submitted_at: Instant,
    audible_end: Instant,
}

impl PlayedTail {
    fn after_start(mut self, started_at: Instant, first_frame: u64) -> Self {
        if self.submitted_at < started_at {
            let duration = Duration::from_nanos(frame_time_ns(self.end_frame - first_frame));
            self.audible_end = self.audible_end.max(started_at + duration);
        }
        self
    }

    fn media_at(self, now: Instant) -> u64 {
        let remaining_ns = self.audible_end.saturating_duration_since(now).as_nanos();
        u128::from(frame_time_ns(self.end_frame)).saturating_sub(remaining_ns) as u64
    }
}

struct OutputCallback {
    consumer: Consumer<StereoFrame>,
    shared: Arc<OutputStatus>,
    next_frame: u64,
    last_tail: Option<PlayedTail>,
}

impl OutputCallback {
    fn write(&mut self, output: &mut [f32], at: Instant, delay: Duration) {
        let (frames, remainder) = output.as_chunks_mut::<2>();
        remainder.fill(0.0);
        let requested = frames.len();
        let (submitted, silence) = self.consumer.pop_partial_slice(frames);
        if !silence.is_empty() {
            silence.fill([0.0; 2]);
            if !self.shared.input_ended.load(Ordering::Acquire) {
                // 一次原子写入保留首次缺帧的完整现场，避免阻塞设备回调。
                let shortage = ((requested as u64) << 32) | silence.len() as u64;
                let _ = self.shared.underrun.compare_exchange(
                    0,
                    shortage,
                    Ordering::Release,
                    Ordering::Relaxed,
                );
            }
        }
        if !submitted.is_empty() {
            self.next_frame += submitted.len() as u64;
            let duration = Duration::from_nanos(frame_time_ns(submitted.len() as u64));
            let Some(audible_end) = at
                .checked_add(delay)
                .and_then(|start| start.checked_add(duration))
            else {
                self.shared.invalid_clock.store(true, Ordering::Release);
                return;
            };
            self.last_tail = Some(PlayedTail {
                end_frame: self.next_frame,
                submitted_at: at,
                audible_end,
            });
        }
        // 即使末次提交碰到读锁，后续回调仍会公布尾包时间。
        if let Ok(mut tail) = self.shared.tail.try_lock() {
            *tail = self.last_tail;
        }
    }
}

impl AudioOutput {
    pub(super) fn open(
        device: &OutputDevice,
        decoder: &mut NativeWebmDecoder,
        first: PcmBlock,
        checkpoint: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        let first_frame = first.first_frame;
        let (producer, consumer) = RingBuffer::new(QUEUE_FRAMES);
        let shared = Arc::new(OutputStatus::default());
        let mut queue = PcmQueue {
            producer,
            shared: Arc::clone(&shared),
            first_frame,
            queued_until: first_frame,
        };

        // PulseAudio 在流仍处于暂停状态时就会请求首批样本。
        let channels = decoder.audio_channels().expect("有 PCM 的录像带音轨");
        queue.push(first, channels)?;
        queue.fill(decoder, checkpoint)?;
        checkpoint()?;

        let callback = OutputCallback {
            consumer,
            shared: Arc::clone(&shared),
            next_frame: first_frame,
            last_tail: None,
        };
        let stream = prepare_stream(device, callback)?;
        checkpoint()?;
        let started_at = Instant::now();
        let stream = start_stream(stream)?;
        Ok(Self {
            _stream: stream,
            queue,
            started_at,
        })
    }

    pub(super) fn fill(
        &mut self,
        decoder: &mut NativeWebmDecoder,
        checkpoint: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        self.queue.fill(decoder, checkpoint)
    }

    pub(super) fn progress(&self, floor_ns: u64, now: Instant) -> Result<(u64, bool), String> {
        let shared = &self.queue.shared;
        if let Some(error) = shared.error.lock().expect("播放设备错误锁被污染").take() {
            return Err(format!("播放音频设备出错: {error}"));
        }
        let shortage = shared.underrun.load(Ordering::Acquire);
        if shortage != 0 {
            let requested = shortage >> 32;
            let missing = u64::from(shortage as u32);
            return Err(format!(
                "播放音频队列供给不足，设备请求 {requested} 帧，缺少 {missing} 帧"
            ));
        }
        if shared.invalid_clock.load(Ordering::Acquire) {
            return Err("播放设备给出了无效时间".into());
        }
        let tail = *shared.tail.lock().expect("播放时钟锁被污染");
        let tail = tail.map(|tail| tail.after_start(self.started_at, self.queue.first_frame));
        let last_progress = tail.map_or(self.started_at, |tail| tail.audible_end);
        if now.saturating_duration_since(last_progress) >= DEVICE_PROGRESS_TIMEOUT {
            return Err("播放设备长时间没有推进音频".into());
        }
        let Some(tail) = tail else {
            return Ok((floor_ns, false));
        };
        let drained = shared.input_ended.load(Ordering::Acquire)
            && tail.end_frame == self.queue.queued_until
            && now >= tail.audible_end;
        Ok((floor_ns.max(tail.media_at(now)), drained))
    }
}

impl PcmQueue {
    fn fill(
        &mut self,
        decoder: &mut NativeWebmDecoder,
        checkpoint: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        if self.shared.input_ended.load(Ordering::Acquire) {
            return Ok(());
        }
        let channels = decoder.audio_channels().expect("有系统音频流的录像带音轨");
        while self.producer.slots() >= OPUS_PACKET_FRAMES {
            let Some(block) = decoder.next_pcm(&mut *checkpoint)? else {
                self.shared.input_ended.store(true, Ordering::Release);
                break;
            };
            self.push(block, channels)?;
        }
        Ok(())
    }

    fn push(&mut self, block: PcmBlock, channels: u16) -> Result<(), String> {
        if block.first_frame != self.queued_until {
            return Err("待播 PCM 采样序号不连续".into());
        }
        let channels = usize::from(channels);
        let mut stereo = [[0.0; 2]; OPUS_PACKET_FRAMES];
        let count = block.samples.len() / channels;
        for (target, source) in stereo[..count]
            .iter_mut()
            .zip(block.samples.chunks_exact(channels))
        {
            *target = [source[0], source[channels - 1]];
        }
        self.producer
            .push_entire_slice(&stereo[..count])
            .map_err(|_| "播放 PCM 队列已满".to_string())?;
        self.queued_until += count as u64;
        Ok(())
    }
}

fn frame_time_ns(frame: u64) -> u64 {
    (u128::from(frame) * 1_000_000_000 / u128::from(AUDIO_SAMPLE_RATE_HZ)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_has_fixed_capacity_and_marks_only_unexpected_shortage() {
        let (mut producer, consumer) = RingBuffer::new(QUEUE_FRAMES);
        let shared = Arc::new(OutputStatus::default());
        let mut callback = OutputCallback {
            consumer,
            shared: Arc::clone(&shared),
            next_frame: 48_000,
            last_tail: None,
        };
        let frames = vec![[0.25, 0.5]; QUEUE_FRAMES];
        producer.push_entire_slice(&frames).unwrap();
        assert!(producer.push([0.5; 2]).is_err());
        let at = Instant::now();
        callback.write(&mut [0.0; 480 * 2], at, Duration::from_millis(12));
        assert_eq!(shared.underrun.load(Ordering::Acquire), 0);
        let tail = shared.tail.lock().unwrap().unwrap();
        assert_eq!(tail.end_frame, 48_480);
        assert_eq!(tail.media_at(at + Duration::from_millis(12)), 1_000_000_000);
        assert_eq!(tail.media_at(at + Duration::from_millis(32)), 1_010_000_000);
        let start = at + Duration::from_millis(50);
        let prefilled = tail.after_start(start, 48_000);
        assert_eq!(prefilled.media_at(start), 1_000_000_000);
        assert_eq!(prefilled.audible_end, start + Duration::from_millis(10));
        assert_eq!(tail.after_start(at, 48_000).audible_end, tail.audible_end);

        shared.input_ended.store(true, Ordering::Release);
        let mut output = vec![1.0; QUEUE_FRAMES * 2];
        callback.write(&mut output, at, Duration::ZERO);
        assert_eq!(&output[..2], &[0.25, 0.5]);
        assert_eq!(&output[output.len() - 2..], &[0.0, 0.0]);
        assert_eq!(shared.underrun.load(Ordering::Acquire), 0);
        shared.input_ended.store(false, Ordering::Release);
        callback.write(&mut output, at, Duration::ZERO);
        let shortage = shared.underrun.load(Ordering::Acquire);
        assert_eq!(shortage >> 32, QUEUE_FRAMES as u64);
        assert_eq!(shortage as u32, QUEUE_FRAMES as u32);
    }
}
