use clippy_media::recording::{
    clock::RecordingSessionClock, frame::CapturedFrame, worker::RecordingFrameSource,
};
use clippy_platform::capture::{Region, RegionCancellation, RegionReader};
use std::io;

pub(super) struct Source {
    clock: RecordingSessionClock,
    region: RegionReader,
    sequence: u64,
    progress: VideoProgress,
    #[cfg(feature = "qa-build")]
    trace_clock: bool,
}

impl Source {
    pub fn open(
        clock: RecordingSessionClock,
        region: Region,
        cancellation: RegionCancellation,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            clock,
            region: RegionReader::open(region, cancellation)?,
            sequence: 0,
            progress: VideoProgress::default(),
            #[cfg(feature = "qa-build")]
            trace_clock: std::env::var_os("CLIPPY_QA_RECORDING_CLOCK").is_some(),
        })
    }
}

impl RecordingFrameSource for Source {
    type Error = io::Error;

    fn capture_next(&mut self) -> io::Result<CapturedFrame> {
        self.capture_next_available()?
            .ok_or_else(|| io::ErrorKind::WouldBlock.into())
    }

    fn capture_next_available(&mut self) -> io::Result<Option<CapturedFrame>> {
        let poll_started = self.clock.now_ns();
        let Some(frame) = self.region.read_new().map_err(io::Error::other)? else {
            self.progress.idle_at(poll_started);
            return Ok(None);
        };
        let captured_at_ns = self.clock.timestamp_at(frame.time.instant());
        if !self.progress.accept(captured_at_ns) {
            return Ok(None);
        }
        let (width, height) = frame.pixels.dimensions();
        let sequence = self.sequence;
        self.sequence += 1;
        #[cfg(feature = "qa-build")]
        if self.trace_clock {
            use clippy_platform::capture::FrameTime;
            let kind = match frame.time {
                FrameTime::Native(_) => "native",
                FrameTime::Observed(_) => "observed",
            };
            eprintln!(
                "recording frame={sequence} time={kind} captured_ns={captured_at_ns} received_ns={}",
                self.clock.now_ns()
            );
        }
        Ok(Some(CapturedFrame {
            sequence,
            captured_at_ns,
            width,
            height,
            stride: width * 4,
            rgba: frame.pixels.into_raw().into_boxed_slice(),
        }))
    }

    fn capture_lower_bound_ns(&mut self) -> io::Result<Option<u64>> {
        Ok(self.progress.lower_bound)
    }

    fn control_timestamp_ns(&mut self) -> io::Result<u64> {
        Ok(self.clock.now_ns())
    }

    fn resume_capture(&mut self) -> io::Result<u64> {
        let at = self.clock.now_ns();
        self.progress.lower_bound = Some(at.saturating_add(1));
        Ok(at)
    }
}

#[derive(Default)]
struct VideoProgress {
    last: Option<u64>,
    lower_bound: Option<u64>,
}

impl VideoProgress {
    fn accept(&mut self, source_time: u64) -> bool {
        if self.last.is_some_and(|last| source_time <= last)
            || self.lower_bound.is_some_and(|bound| source_time < bound)
        {
            return false;
        }
        self.last = Some(source_time);
        if self.lower_bound.is_some() {
            self.lower_bound = Some(source_time);
        }
        true
    }

    fn idle_at(&mut self, poll_started: u64) {
        // 保留一次取帧等待的余量。更早的迟到帧无法再替换已经确认的编码位置。
        if let Some(last) = self.last {
            self.lower_bound = Some(self.lower_bound.unwrap_or(last).max(poll_started));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::VideoProgress;

    #[test]
    fn idle_and_resume_exclude_stale_frames_without_restamping() {
        let mut progress = VideoProgress::default();
        progress.idle_at(90);
        assert_eq!(progress.lower_bound, None);
        assert!(progress.accept(100));
        progress.idle_at(150);
        assert!(!progress.accept(149));
        assert!(progress.accept(150));
        assert!(!progress.accept(150));
        assert_eq!(progress.lower_bound, Some(150));
        progress.lower_bound = Some(501);
        assert!(!progress.accept(500));
        assert!(progress.accept(520));
        assert_eq!(progress.last, Some(520));
    }
}
