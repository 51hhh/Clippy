mod output;

use super::PlaybackUpdate;
use crate::recording::native_playback::NativeWebmDecoder;
use output::{open_device, AudioOutput, OutputDevice};
use std::{path::Path, time::Instant};

const NS_PER_MS: u64 = 1_000_000;

enum Timeline {
    Starting,
    Paused,
    Audio(Box<AudioOutput>),
    Silent {
        position_ns: u64,
        started_at: Instant,
    },
}

pub struct NativeRecordingPlayer {
    timeline: Timeline,
    decoder: NativeWebmDecoder,
    position_ns: u64,
    device: Option<OutputDevice>,
    device_target: Option<String>,
}

impl NativeRecordingPlayer {
    pub fn open(
        data_root: &Path,
        session: &str,
        artifact: &str,
        device_target: Option<String>,
        checkpoint: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        let decoder =
            NativeWebmDecoder::open_library_artifact(data_root, session, artifact, checkpoint)?;
        Ok(Self {
            decoder,
            timeline: Timeline::Starting,
            position_ns: 0,
            device: None,
            device_target,
        })
    }

    pub fn apply_controls(
        &mut self,
        paused: bool,
        seek_ms: Option<u64>,
        checkpoint: impl FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        let target = match (seek_ms, paused, &self.timeline) {
            (Some(ms), _, _) => ms.saturating_mul(NS_PER_MS),
            (None, true, Timeline::Paused)
            | (None, false, Timeline::Starting | Timeline::Audio(_) | Timeline::Silent { .. }) => {
                return Ok(())
            }
            (None, true, _) => self.progress(Instant::now())?.0,
            (None, false, Timeline::Paused) => {
                self.timeline = Timeline::Starting;
                return Ok(());
            }
        };
        // 重置解码位置前结束旧流，让服务端丢弃尚未播出的旧样本。
        self.timeline = if paused {
            Timeline::Paused
        } else {
            Timeline::Starting
        };
        self.position_ns = self.decoder.seek(target, checkpoint)?;
        Ok(())
    }

    pub fn poll(
        &mut self,
        mut checkpoint: impl FnMut() -> Result<(), String>,
    ) -> Result<PlaybackUpdate, String> {
        if let Timeline::Audio(output) = &mut self.timeline {
            output.fill(&mut self.decoder, &mut checkpoint)?;
        }
        let now = Instant::now();
        let duration_ns = self.decoder.duration_ns();
        let (position_ns, audio_drained) = self.progress(now)?;
        self.position_ns = position_ns.min(duration_ns);
        if audio_drained {
            self.timeline = Timeline::Silent {
                position_ns: self.position_ns,
                started_at: now,
            };
        }
        let frame = self
            .decoder
            .video_through(self.position_ns, &mut checkpoint)?
            .map(|frame| frame.image);
        if matches!(self.timeline, Timeline::Starting) {
            // 跳转时先从关键帧解出目标画面，避免此时耗尽已经开始播放的 PCM。
            self.timeline = self.start(&mut checkpoint)?;
        }
        let ended = self.position_ns == duration_ns && !matches!(self.timeline, Timeline::Audio(_));
        let duration_ms = duration_ns.div_ceil(NS_PER_MS);
        let position_ms = if ended {
            duration_ms
        } else {
            self.position_ns / NS_PER_MS
        };
        Ok(PlaybackUpdate {
            duration_ms,
            position_ms,
            frame,
            ended,
        })
    }

    fn start(
        &mut self,
        checkpoint: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<Timeline, String> {
        let first = self.decoder.next_pcm(&mut *checkpoint)?;
        let Some(first) = first else {
            return Ok(Timeline::Silent {
                position_ns: self.position_ns,
                started_at: Instant::now(),
            });
        };
        if self.device.is_none() {
            self.device = Some(open_device(self.device_target.as_deref())?);
        }
        checkpoint()?;
        let device = self.device.as_ref().expect("播放设备已打开");
        let output = AudioOutput::open(device, &mut self.decoder, first, checkpoint)?;
        Ok(Timeline::Audio(Box::new(output)))
    }

    fn progress(&self, now: Instant) -> Result<(u64, bool), String> {
        match &self.timeline {
            Timeline::Audio(output) => output.progress(self.position_ns, now),
            Timeline::Silent {
                position_ns,
                started_at,
            } => {
                let elapsed = now
                    .saturating_duration_since(*started_at)
                    .as_nanos()
                    .min(u128::from(u64::MAX)) as u64;
                Ok((position_ns.saturating_add(elapsed), false))
            }
            Timeline::Paused | Timeline::Starting => Ok((self.position_ns, false)),
        }
    }
}
