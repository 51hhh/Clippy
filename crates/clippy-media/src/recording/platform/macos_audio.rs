use crate::recording::{
    audio::{
        frames_to_ns, AudioFormat, CapturedAudioChunk, AUDIO_SAMPLE_RATE_HZ, MAX_AUDIO_CHUNK_FRAMES,
    },
    audio_worker::RecordingAudioSource,
    clock::RecordingSessionClock,
};
pub use clippy_platform::capture::macos_audio::system_audio_available;
use clippy_platform::capture::{
    macos_audio::{SystemAudioPacket, SystemAudioStream},
    DesktopRect, Region,
};
use std::{
    collections::VecDeque,
    io,
    time::{Duration, Instant},
};

const CHANNELS: u16 = 2;

pub struct MacSystemAudioPlan {
    output: String,
    geometry: DesktopRect,
}

impl MacSystemAudioPlan {
    pub fn for_region(region: &Region) -> Self {
        Self {
            output: region.output.clone(),
            geometry: region.geometry,
        }
    }

    pub fn channels(&self) -> u16 {
        CHANNELS
    }

    pub fn connect(self, clock: RecordingSessionClock) -> io::Result<MacSystemAudioSource> {
        let stream = SystemAudioStream::open(&self.output, self.geometry, AUDIO_SAMPLE_RATE_HZ)
            .map_err(io::Error::other)?;
        Ok(MacSystemAudioSource {
            stream,
            clock,
            minimum_capture: None,
            timeline: PcmTimeline::default(),
        })
    }
}

pub struct MacSystemAudioSource {
    stream: SystemAudioStream,
    clock: RecordingSessionClock,
    minimum_capture: Option<Instant>,
    timeline: PcmTimeline,
}

impl MacSystemAudioSource {
    fn activate(&mut self) -> io::Result<u64> {
        let started = self.stream.start().map_err(io::Error::other)?;
        self.minimum_capture = Some(started);
        Ok(self.clock.timestamp_at(started))
    }

    fn next(&mut self, timeout: Duration) -> io::Result<Option<CapturedAudioChunk>> {
        if let Some(chunk) = self.timeline.ready.pop_front() {
            return Ok(Some(chunk));
        }
        let deadline = Instant::now() + timeout;
        while let Some(packet) = self
            .stream
            .read(deadline.saturating_duration_since(Instant::now()))
            .map_err(io::Error::other)?
        {
            if self
                .minimum_capture
                .is_some_and(|started| packet.captured_at < started)
            {
                continue;
            }
            let timestamp = self.clock.timestamp_at(packet.captured_at);
            self.timeline.append(timestamp, packet)?;
            return Ok(self.timeline.ready.pop_front());
        }
        Ok(None)
    }
}

impl RecordingAudioSource for MacSystemAudioSource {
    type Error = io::Error;

    fn start_capture(&mut self) -> io::Result<Option<u64>> {
        self.activate().map(Some)
    }

    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> io::Result<Option<CapturedAudioChunk>> {
        self.next(timeout)
    }

    fn control_timestamp_ns(&mut self) -> io::Result<u64> {
        Ok(self.clock.now_ns().max(self.timeline.end))
    }

    fn pause_capture(&mut self) -> io::Result<u64> {
        let paused = self.stream.pause().map_err(io::Error::other)?;
        self.timeline.ready.clear();
        Ok(self.clock.timestamp_at(paused).max(self.timeline.end))
    }

    fn resume_capture(&mut self) -> io::Result<u64> {
        self.activate()
    }

    fn stop_capture(&mut self) -> io::Result<u64> {
        self.stream.stop().map_err(io::Error::other)?;
        let mut remaining = VecDeque::new();
        while let Some(chunk) = self.next(Duration::ZERO)? {
            remaining.push_back(chunk);
        }
        self.timeline.ready = remaining;
        self.control_timestamp_ns()
    }

    fn take_stopped_chunks(&mut self) -> io::Result<Vec<CapturedAudioChunk>> {
        Ok(self.timeline.ready.pop_front().into_iter().collect())
    }
}

#[derive(Default)]
struct PcmTimeline {
    ready: VecDeque<CapturedAudioChunk>,
    sequence: u64,
    end: u64,
}

impl PcmTimeline {
    fn append(&mut self, timestamp: u64, packet: SystemAudioPacket) -> io::Result<()> {
        let SystemAudioPacket {
            frames, samples, ..
        } = packet;
        self.end = offset_timestamp(timestamp, frames)?;
        if frames <= MAX_AUDIO_CHUNK_FRAMES {
            self.push(timestamp, frames, samples);
        } else {
            let samples_per_chunk = MAX_AUDIO_CHUNK_FRAMES as usize * usize::from(CHANNELS);
            let mut offset = 0;
            for samples in samples.chunks(samples_per_chunk) {
                let frame_count = (samples.len() / usize::from(CHANNELS)) as u32;
                self.push(
                    offset_timestamp(timestamp, offset)?,
                    frame_count,
                    samples.into(),
                );
                offset += frame_count;
            }
        }
        Ok(())
    }

    fn push(&mut self, captured_at_ns: u64, frame_count: u32, samples: Box<[f32]>) {
        self.ready.push_back(CapturedAudioChunk {
            sequence: self.sequence,
            captured_at_ns,
            format: AudioFormat::normalized(CHANNELS),
            frame_count,
            samples,
        });
        self.sequence += 1;
    }
}

fn offset_timestamp(timestamp: u64, frames: u32) -> io::Result<u64> {
    timestamp
        .checked_add(frames_to_ns(frames).map_err(io::Error::other)?)
        .ok_or_else(|| io::Error::other("ScreenCaptureKit audio timestamp overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_split_preserves_native_pts_and_stop_end() {
        let mut timeline = PcmTimeline::default();
        let start = 700_000_000;
        let frames = MAX_AUDIO_CHUNK_FRAMES + 3;
        timeline
            .append(
                start,
                SystemAudioPacket {
                    captured_at: Instant::now(),
                    frames,
                    samples: vec![0.5; frames as usize * 2].into_boxed_slice(),
                },
            )
            .unwrap();
        let first = timeline.ready.pop_front().unwrap();
        let tail = timeline.ready.pop_front().unwrap();
        assert_eq!(
            (first.sequence, first.captured_at_ns, first.frame_count),
            (0, start, MAX_AUDIO_CHUNK_FRAMES)
        );
        assert_eq!(
            (tail.sequence, tail.captured_at_ns, tail.frame_count),
            (1, start + 100_000_000, 3)
        );
        assert_eq!(tail.samples.as_ref(), &[0.5; 6]);
        assert_eq!(timeline.end, start + 100_062_500);
    }
}
