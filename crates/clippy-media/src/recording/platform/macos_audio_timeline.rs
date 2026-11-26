use super::{
    macos_audio_contract::{MacAudioContractError, MappedPcmChunk},
    macos_audio_resampler::{MacAudioResamplerError, MacAudioStreamResampler},
};
use crate::recording::audio::{
    frames_to_ns, AudioFormat, CapturedAudioChunk, AUDIO_SAMPLE_RATE_HZ, MAX_AUDIO_CHUNK_FRAMES,
};
use std::{cmp::Ordering, collections::VecDeque, io};

pub(super) const CHANNELS: u16 = 2;
const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;

#[derive(Default)]
pub(super) struct PcmTimeline {
    pub ready: VecDeque<CapturedAudioChunk>,
    pub end: u64,
    sequence: u64,
    microphone_segment: Option<MicrophoneSegment>,
    resampler: MacAudioStreamResampler,
}

struct MicrophoneSegment {
    source_start: (u64, u32),
    session_start: u64,
    frames: u64,
    rate_hz: u32,
}

impl MicrophoneSegment {
    fn input_position(&self, pts: (u64, u32)) -> io::Result<u64> {
        let (value, scale) = (i128::from(pts.0), i128::from(pts.1));
        let (start, start_scale) = (
            i128::from(self.source_start.0),
            i128::from(self.source_start.1),
        );
        let elapsed = value * start_scale - start * scale;
        if elapsed < 0 {
            return Err(io::Error::other(MacAudioContractError::TimestampRegression));
        }
        let denominator = scale * start_scale;
        // 只按相对源时间对齐采样格，避免各包 NS 舍入误差误判为重叠或空洞。
        let position = (elapsed * i128::from(self.rate_hz) + denominator / 2) / denominator;
        u64::try_from(position).map_err(io::Error::other)
    }
}

impl PcmTimeline {
    pub fn append_microphone(
        &mut self,
        source_pts: (u64, u32),
        timestamp: u64,
        rate_hz: u32,
        frames: u32,
        samples: Box<[f32]>,
    ) -> io::Result<()> {
        let continuous = match &self.microphone_segment {
            None => false,
            Some(segment) => match segment.input_position(source_pts)?.cmp(&segment.frames) {
                Ordering::Less => {
                    return Err(io::Error::other(MacAudioContractError::TimestampOverlap))
                }
                Ordering::Greater => false,
                Ordering::Equal if segment.rate_hz == rate_hz => true,
                Ordering::Equal => {
                    return Err(io::Error::other(
                        MacAudioResamplerError::SampleRateChanged {
                            from: segment.rate_hz,
                            to: rate_hz,
                        },
                    ));
                }
            },
        };
        if !continuous {
            self.finish_microphone()?;
            // 封尾向上取整最多多占一个输出采样的时长，保留原生段首的更大间隔。
            if self.end.saturating_sub(timestamp)
                > NANOSECONDS_PER_SECOND.div_ceil(u64::from(AUDIO_SAMPLE_RATE_HZ))
            {
                return Err(io::Error::other(MacAudioContractError::TimestampOverlap));
            }
            self.microphone_segment = Some(MicrophoneSegment {
                source_start: source_pts,
                session_start: timestamp.max(self.end),
                frames: 0,
                rate_hz,
            });
        }

        let segment = self.microphone_segment.as_mut().expect("麦克风段已定位");
        let start = segment.session_start;
        let preceding_frames = segment.frames;
        segment.frames = preceding_frames
            .checked_add(u64::from(frames))
            .ok_or_else(overflow)?;
        if rate_hz == AUDIO_SAMPLE_RATE_HZ {
            let elapsed_ns = u64::try_from(
                u128::from(preceding_frames) * u128::from(NANOSECONDS_PER_SECOND)
                    / u128::from(AUDIO_SAMPLE_RATE_HZ),
            )
            .map_err(io::Error::other)?;
            self.append_stereo(
                start.checked_add(elapsed_ns).ok_or_else(overflow)?,
                frames,
                samples,
            )
        } else {
            let chunks = self
                .resampler
                .push_packet(start, rate_hz, frames, &samples, false)
                .map_err(io::Error::other)?;
            self.queue_resampled(chunks)
        }
    }

    pub fn finish_microphone(&mut self) -> io::Result<()> {
        if self.microphone_segment.take().is_some() {
            let chunks = self.resampler.finish().map_err(io::Error::other)?;
            self.queue_resampled(chunks)?;
        }
        Ok(())
    }

    pub fn discard(&mut self) {
        self.ready.clear();
        self.microphone_segment = None;
        self.resampler.discard();
    }

    pub fn append_stereo(
        &mut self,
        timestamp: u64,
        frames: u32,
        samples: Box<[f32]>,
    ) -> io::Result<()> {
        if frames <= MAX_AUDIO_CHUNK_FRAMES {
            return self.push(timestamp, frames, samples);
        }
        let samples_per_chunk = MAX_AUDIO_CHUNK_FRAMES as usize * usize::from(CHANNELS);
        let mut offset = 0;
        for samples in samples.chunks(samples_per_chunk) {
            let frame_count = (samples.len() / usize::from(CHANNELS)) as u32;
            self.push(
                offset_timestamp(timestamp, offset)?,
                frame_count,
                samples.into(),
            )?;
            offset += frame_count;
        }
        Ok(())
    }

    fn queue_resampled(&mut self, chunks: Vec<MappedPcmChunk>) -> io::Result<()> {
        for chunk in chunks {
            self.push(chunk.captured_at_ns, chunk.frame_count, chunk.samples)?;
        }
        Ok(())
    }

    fn push(
        &mut self,
        captured_at_ns: u64,
        frame_count: u32,
        samples: Box<[f32]>,
    ) -> io::Result<()> {
        self.end = offset_timestamp(captured_at_ns, frame_count)?;
        let sequence = self.sequence;
        self.sequence = sequence.checked_add(1).ok_or_else(overflow)?;
        self.ready.push_back(CapturedAudioChunk {
            sequence,
            captured_at_ns,
            format: AudioFormat::normalized(CHANNELS),
            frame_count,
            samples,
        });
        Ok(())
    }
}

fn offset_timestamp(timestamp: u64, frames: u32) -> io::Result<u64> {
    timestamp
        .checked_add(frames_to_ns(frames).map_err(io::Error::other)?)
        .ok_or_else(overflow)
}

fn overflow() -> io::Error {
    io::Error::other(MacAudioResamplerError::TimelineOverflow)
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
            .append_stereo(
                start,
                frames,
                vec![0.5; frames as usize * 2].into_boxed_slice(),
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

    fn mic(
        timeline: &mut PcmTimeline,
        pts: (u64, u32),
        timestamp: u64,
        rate: u32,
        frames: u32,
    ) -> io::Result<()> {
        timeline.append_microphone(
            pts,
            timestamp,
            rate,
            frames,
            vec![0.25; frames as usize * 2].into_boxed_slice(),
        )
    }

    #[test]
    fn fractional_packet_pts_share_one_filter_and_flush_at_stop() {
        let mut timeline = PcmTimeline::default();
        let rate = 44_100;
        let origin = 3_600 * u64::from(rate) + 13;
        let start = 700_000_000;
        let mut offset = 0;
        for (index, frames) in [137, 503, 83, 223].into_iter().enumerate() {
            let pts = if index % 2 == 0 {
                (
                    (origin + offset) * NANOSECONDS_PER_SECOND / u64::from(rate),
                    1_000_000_000,
                )
            } else {
                (origin + offset, rate)
            };
            let timestamp = start + offset * NANOSECONDS_PER_SECOND / u64::from(rate);
            mic(&mut timeline, pts, timestamp, rate, frames).unwrap();
            offset += u64::from(frames);
        }
        assert!(timeline.ready.is_empty());
        timeline.finish_microphone().unwrap();
        let length = (offset * 48_000).div_ceil(u64::from(rate)) as u32;
        assert_eq!(timeline.ready.front().unwrap().captured_at_ns, start);
        assert_eq!(
            timeline
                .ready
                .iter()
                .map(|chunk| chunk.frame_count)
                .sum::<u32>(),
            length
        );
        assert_eq!(timeline.end, start + frames_to_ns(length).unwrap());
    }

    #[test]
    fn preserves_a_real_gap_and_rejects_overlap_or_in_segment_rate_change() {
        let mut timeline = PcmTimeline::default();
        let origin = 200_000;
        let start = 1_000_000_000;
        mic(&mut timeline, (origin, 44_100), start, 44_100, 100).unwrap();
        assert!(mic(&mut timeline, (origin + 99, 44_100), start, 44_100, 100).is_err());
        assert!(mic(&mut timeline, (origin + 100, 44_100), start, 48_000, 100).is_err());
        let next = start + 100_000_000;
        mic(&mut timeline, (origin + 4_410, 44_100), next, 48_000, 100).unwrap();
        let old = timeline.ready.pop_front().unwrap();
        let new = timeline.ready.pop_front().unwrap();
        assert_eq!((old.captured_at_ns, old.frame_count), (start, 109));
        assert_eq!((new.captured_at_ns, new.frame_count), (next, 100));
    }

    #[test]
    fn pause_discards_unplayed_filter_tail_and_resume_reanchors() {
        let mut timeline = PcmTimeline::default();
        mic(&mut timeline, (44_100, 44_100), 1_000_000_000, 44_100, 100).unwrap();
        timeline.discard();
        timeline.finish_microphone().unwrap();
        assert!(timeline.ready.is_empty());
        let start = 2_000_000_000;
        mic(&mut timeline, (0, 44_100), start, 44_100, 50).unwrap();
        timeline.finish_microphone().unwrap();
        let resumed = timeline.ready.pop_front().unwrap();
        assert_eq!((resumed.captured_at_ns, resumed.frame_count), (start, 55));
        assert!(timeline.ready.is_empty());
    }
}
