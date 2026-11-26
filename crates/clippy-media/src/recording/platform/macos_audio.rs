use super::macos_audio_timeline::{PcmTimeline, CHANNELS};
use crate::recording::{
    audio::CapturedAudioChunk,
    audio_devices::{NativeRecordingAudioDevice, RecordingAudioDeviceKind},
    audio_worker::RecordingAudioSource,
    clock::RecordingSessionClock,
};
pub use clippy_platform::capture::macos_audio::{
    microphone_available, request_microphone_permission, system_audio_available,
};
use clippy_platform::capture::{
    macos_audio::{enumerate_microphones, AudioInput, AudioStream, StereoPcm},
    DesktopRect, Region,
};
use std::{
    collections::VecDeque,
    io,
    time::{Duration, Instant},
};

pub struct MacAudioPlan {
    output: String,
    geometry: DesktopRect,
    input: AudioInput,
}

impl MacAudioPlan {
    pub fn for_region(region: &Region, input: AudioInput) -> Self {
        Self {
            output: region.output.clone(),
            geometry: region.geometry,
            input,
        }
    }

    pub fn channels(&self) -> u16 {
        CHANNELS
    }

    pub fn connect(self, clock: RecordingSessionClock) -> io::Result<MacAudioSource> {
        let stream =
            AudioStream::open(&self.output, self.geometry, self.input).map_err(io::Error::other)?;
        Ok(MacAudioSource {
            stream,
            clock,
            minimum_capture: None,
            timeline: PcmTimeline::default(),
        })
    }
}

pub fn enumerate_recording_audio_devices() -> io::Result<Vec<NativeRecordingAudioDevice>> {
    Ok(enumerate_microphones()
        .map_err(io::Error::other)?
        .into_iter()
        .map(|device| NativeRecordingAudioDevice {
            kind: RecordingAudioDeviceKind::Microphone,
            native_id: device.id,
            label: device.label,
            is_default: device.is_default,
        })
        .collect())
}

pub struct MacAudioSource {
    stream: AudioStream,
    clock: RecordingSessionClock,
    minimum_capture: Option<Instant>,
    timeline: PcmTimeline,
}

impl MacAudioSource {
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
            let StereoPcm {
                frames,
                rate_hz,
                samples,
            } = packet.pcm;
            if let Some(pts) = packet.microphone_pts {
                self.timeline
                    .append_microphone(pts, timestamp, rate_hz, frames, samples)?;
            } else {
                self.timeline.append_stereo(timestamp, frames, samples)?;
            }
            if let Some(chunk) = self.timeline.ready.pop_front() {
                return Ok(Some(chunk));
            }
        }
        Ok(None)
    }
}

impl RecordingAudioSource for MacAudioSource {
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
        self.timeline.discard();
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
        self.timeline.finish_microphone()?;
        remaining.append(&mut self.timeline.ready);
        self.timeline.ready = remaining;
        self.control_timestamp_ns()
    }

    fn take_stopped_chunks(&mut self) -> io::Result<Vec<CapturedAudioChunk>> {
        Ok(self.timeline.ready.pop_front().into_iter().collect())
    }
}
