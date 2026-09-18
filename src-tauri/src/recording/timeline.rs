use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(super) enum TimelineError {
    #[error("录屏采集时间戳必须严格递增")]
    SourceTimestampNotIncreasing,
    #[error("录屏输出时间戳必须严格递增")]
    PresentationTimestampNotIncreasing,
    #[error("录屏时间线计算溢出")]
    Overflow,
    #[error("首帧前不能暂停录屏")]
    PauseBeforeFirstFrame,
    #[error("录屏已经暂停")]
    AlreadyPaused,
    #[error("录屏尚未暂停")]
    NotPaused,
}

#[derive(Debug, Default)]
pub(super) struct RecordingTimeline {
    origin_ns: Option<u64>,
    last_source_ns: Option<u64>,
    last_presentation_ns: Option<u64>,
    paused_at_ns: Option<u64>,
    accumulated_pause_ns: u64,
}

impl RecordingTimeline {
    /// 返回 `None` 表示暂停期间收到的帧；这类帧不属于背压丢帧，也不推进任何时钟。
    pub fn map_frame(&mut self, captured_at_ns: u64) -> Result<Option<u64>, TimelineError> {
        if self.paused_at_ns.is_some() {
            return Ok(None);
        }
        let Some(origin_ns) = self.origin_ns else {
            self.origin_ns = Some(captured_at_ns);
            self.last_source_ns = Some(captured_at_ns);
            self.last_presentation_ns = Some(0);
            return Ok(Some(0));
        };
        if self
            .last_source_ns
            .is_some_and(|last| captured_at_ns <= last)
        {
            return Err(TimelineError::SourceTimestampNotIncreasing);
        }
        let elapsed = captured_at_ns
            .checked_sub(origin_ns)
            .and_then(|value| value.checked_sub(self.accumulated_pause_ns))
            .ok_or(TimelineError::Overflow)?;
        if self
            .last_presentation_ns
            .is_some_and(|last| elapsed <= last)
        {
            return Err(TimelineError::PresentationTimestampNotIncreasing);
        }
        self.last_source_ns = Some(captured_at_ns);
        self.last_presentation_ns = Some(elapsed);
        Ok(Some(elapsed))
    }

    pub fn pause(&mut self, captured_at_ns: u64) -> Result<(), TimelineError> {
        if self.paused_at_ns.is_some() {
            return Err(TimelineError::AlreadyPaused);
        }
        let last = self
            .last_source_ns
            .ok_or(TimelineError::PauseBeforeFirstFrame)?;
        if captured_at_ns < last {
            return Err(TimelineError::SourceTimestampNotIncreasing);
        }
        self.last_source_ns = Some(captured_at_ns);
        self.paused_at_ns = Some(captured_at_ns);
        Ok(())
    }

    pub fn resume(&mut self, captured_at_ns: u64) -> Result<(), TimelineError> {
        let paused_at_ns = self.paused_at_ns.ok_or(TimelineError::NotPaused)?;
        if captured_at_ns <= paused_at_ns {
            return Err(TimelineError::SourceTimestampNotIncreasing);
        }
        let pause_duration = captured_at_ns
            .checked_sub(paused_at_ns)
            .ok_or(TimelineError::Overflow)?;
        self.accumulated_pause_ns = self
            .accumulated_pause_ns
            .checked_add(pause_duration)
            .ok_or(TimelineError::Overflow)?;
        self.last_source_ns = Some(captured_at_ns);
        self.paused_at_ns = None;
        Ok(())
    }
}
