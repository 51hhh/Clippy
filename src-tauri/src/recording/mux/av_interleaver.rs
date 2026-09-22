//! VP9 与 Opus packet 的有界时间序合并器。
//!
//! 两个 encoder 都暴露“下一 packet 不会早于”的 frontier。本层只在另一轨 frontier 已经越过
//! 待写 timestamp 后提交；因此不依赖采集线程调度顺序，也不需要缓存完整分段。

use super::opus_webm::{
    AvWebmOutput, AvWebmPacketMux, EncodedOpusPacket, OpusTrackConfig, OpusWebmError,
};
use std::collections::VecDeque;
use std::io::{Seek, Write};

const MAX_PENDING_PACKETS_PER_TRACK: usize = 32;

struct PendingVideoPacket {
    data: Box<[u8]>,
    timestamp_ns: u64,
    keyframe: bool,
}

pub(in crate::recording) struct AvPacketInterleaver<W: Write + Seek> {
    mux: AvWebmPacketMux<W>,
    video: VecDeque<PendingVideoPacket>,
    audio: VecDeque<EncodedOpusPacket>,
    last_video_timestamp_ns: Option<u64>,
    last_audio_timestamp_ns: Option<u64>,
}

impl<W: Write + Seek> AvPacketInterleaver<W> {
    pub fn new(
        writer: W,
        width: u32,
        height: u32,
        audio: &OpusTrackConfig,
    ) -> Result<Self, OpusWebmError> {
        Ok(Self {
            mux: AvWebmPacketMux::new(writer, width, height, audio)?,
            video: VecDeque::new(),
            audio: VecDeque::new(),
            last_video_timestamp_ns: None,
            last_audio_timestamp_ns: None,
        })
    }

    pub fn enqueue_video(
        &mut self,
        data: &[u8],
        timestamp_ns: u64,
        keyframe: bool,
    ) -> Result<(), OpusWebmError> {
        if self.video.len() >= MAX_PENDING_PACKETS_PER_TRACK {
            return Err(OpusWebmError::InterleaveQueueFull);
        }
        if self
            .last_video_timestamp_ns
            .is_some_and(|last| timestamp_ns <= last)
        {
            return Err(OpusWebmError::InvalidMuxTimestamp);
        }
        self.last_video_timestamp_ns = Some(timestamp_ns);
        self.video.push_back(PendingVideoPacket {
            data: data.to_vec().into_boxed_slice(),
            timestamp_ns,
            keyframe,
        });
        Ok(())
    }

    pub fn enqueue_audio(&mut self, packet: EncodedOpusPacket) -> Result<(), OpusWebmError> {
        if self.audio.len() >= MAX_PENDING_PACKETS_PER_TRACK {
            return Err(OpusWebmError::InterleaveQueueFull);
        }
        if self
            .last_audio_timestamp_ns
            .is_some_and(|last| packet.timestamp_ns <= last)
        {
            return Err(OpusWebmError::InvalidMuxTimestamp);
        }
        self.last_audio_timestamp_ns = Some(packet.timestamp_ns);
        self.audio.push_back(packet);
        Ok(())
    }

    pub fn flush_ready(
        &mut self,
        next_video_timestamp_ns: u64,
        next_audio_timestamp_ns: u64,
    ) -> Result<(), OpusWebmError> {
        while self.front_is_ready(next_video_timestamp_ns, next_audio_timestamp_ns) {
            self.flush_one()?;
        }
        Ok(())
    }

    pub fn finish(mut self, duration_ns: u64) -> Result<AvWebmOutput<W>, OpusWebmError> {
        while self.next_timestamp().is_some() {
            self.flush_one()?;
        }
        self.mux.finish(duration_ns)
    }

    fn next_timestamp(&self) -> Option<u64> {
        match (self.video.front(), self.audio.front()) {
            (Some(video), Some(audio)) => Some(video.timestamp_ns.min(audio.timestamp_ns)),
            (Some(video), None) => Some(video.timestamp_ns),
            (None, Some(audio)) => Some(audio.timestamp_ns),
            (None, None) => None,
        }
    }

    fn front_is_ready(&self, next_video_timestamp_ns: u64, next_audio_timestamp_ns: u64) -> bool {
        match (self.video.front(), self.audio.front()) {
            // 同时间戳固定视频在前，因此视频只需确认未来音频不会更早；音频则必须等待
            // 未来视频严格越过自己，不能在相等 frontier 时抢先提交。
            (Some(video), Some(audio)) if video.timestamp_ns <= audio.timestamp_ns => {
                video.timestamp_ns <= next_audio_timestamp_ns
            }
            (Some(_), Some(audio)) => audio.timestamp_ns < next_video_timestamp_ns,
            (Some(video), None) => video.timestamp_ns <= next_audio_timestamp_ns,
            (None, Some(audio)) => audio.timestamp_ns < next_video_timestamp_ns,
            (None, None) => false,
        }
    }

    fn flush_one(&mut self) -> Result<(), OpusWebmError> {
        let video_first = match (self.video.front(), self.audio.front()) {
            (Some(video), Some(audio)) => video.timestamp_ns <= audio.timestamp_ns,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => return Ok(()),
        };
        if video_first {
            let packet = self.video.pop_front().expect("已检查视频队首");
            self.mux
                .add_video_packet(&packet.data, packet.timestamp_ns, packet.keyframe)
        } else {
            let packet = self.audio.pop_front().expect("已检查音频队首");
            self.mux.add_audio_packet(&packet)
        }
    }

    #[cfg(test)]
    fn pending_counts(&self) -> (usize, usize) {
        (self.video.len(), self.audio.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::mux::opus_webm::OpusPacketEncoder;
    use std::io::Cursor;

    fn audio_packet(timestamp_ns: u64) -> EncodedOpusPacket {
        EncodedOpusPacket {
            data: vec![1, 2, 3].into_boxed_slice(),
            timestamp_ns,
            duration_ns: 20_000_000,
            discard_padding_ns: 0,
        }
    }

    #[test]
    fn audio_that_arrives_first_waits_for_the_earlier_video_packet() {
        let encoder = OpusPacketEncoder::new(2).unwrap();
        let mut interleaver =
            AvPacketInterleaver::new(Cursor::new(Vec::new()), 2, 2, encoder.track_config())
                .unwrap();
        interleaver.enqueue_audio(audio_packet(6_500_000)).unwrap();
        interleaver.flush_ready(0, 26_500_000).unwrap();
        assert_eq!(interleaver.pending_counts(), (0, 1));

        interleaver.enqueue_video(&[9], 0, true).unwrap();
        interleaver.flush_ready(16_666_666, 26_500_000).unwrap();
        assert_eq!(interleaver.pending_counts(), (0, 0));

        interleaver.enqueue_video(&[8], 16_666_666, false).unwrap();
        interleaver.enqueue_audio(audio_packet(26_500_000)).unwrap();
        let output = interleaver.finish(40_000_000).unwrap();
        assert_eq!(output.video_frame_count, 2);
        assert_eq!(output.audio_packet_count, 2);
    }

    #[test]
    fn same_timestamp_is_deterministic_and_each_track_is_strict() {
        let encoder = OpusPacketEncoder::new(1).unwrap();
        let mut interleaver =
            AvPacketInterleaver::new(Cursor::new(Vec::new()), 2, 2, encoder.track_config())
                .unwrap();
        interleaver.enqueue_audio(audio_packet(10_000_000)).unwrap();
        interleaver.flush_ready(10_000_000, 30_000_000).unwrap();
        assert_eq!(interleaver.pending_counts(), (0, 1));

        interleaver.enqueue_video(&[1], 0, true).unwrap();
        interleaver.enqueue_video(&[2], 10_000_000, false).unwrap();
        interleaver.flush_ready(20_000_000, 20_000_000).unwrap();
        assert_eq!(interleaver.pending_counts(), (0, 0));
        assert_eq!(
            interleaver.enqueue_video(&[3], 10_000_000, false),
            Err(OpusWebmError::InvalidMuxTimestamp)
        );
    }

    #[test]
    fn queue_budget_fails_instead_of_becoming_unbounded() {
        let encoder = OpusPacketEncoder::new(2).unwrap();
        let mut interleaver =
            AvPacketInterleaver::new(Cursor::new(Vec::new()), 2, 2, encoder.track_config())
                .unwrap();
        for index in 0..MAX_PENDING_PACKETS_PER_TRACK {
            interleaver
                .enqueue_video(&[1], index as u64, index == 0)
                .unwrap();
        }
        assert_eq!(
            interleaver.enqueue_video(&[2], MAX_PENDING_PACKETS_PER_TRACK as u64, false),
            Err(OpusWebmError::InterleaveQueueFull)
        );
    }
}
