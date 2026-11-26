use super::FrameTime;
use anyhow::Result;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(super) struct NativeClock {
    local: Instant,
    native_ns: u64,
}

impl NativeClock {
    fn correlate(sample_native: impl FnOnce() -> Result<u64>) -> Result<Self> {
        let before = Instant::now();
        let native_ns = sample_native()?;
        let after = Instant::now();
        Ok(Self {
            local: before + (after - before) / 2,
            native_ns,
        })
    }

    #[cfg(target_os = "linux")]
    pub fn monotonic() -> Result<Self> {
        Self::correlate(|| Ok(u64::try_from(super::screencast_stream::monotonic_ns()?)?))
    }

    pub fn timestamp(self, native_ns: Option<u64>, observed: Instant) -> FrameTime {
        let mapped = native_ns
            .filter(|stamp| *stamp > 0)
            .and_then(|stamp| stamp.checked_sub(self.native_ns))
            .and_then(|elapsed| self.local.checked_add(Duration::from_nanos(elapsed)))
            .filter(|sample| *sample <= observed);
        mapped.map_or(FrameTime::Observed(observed), FrameTime::Native)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correlates_native_clock_and_keeps_observation_for_unusable_times() {
        let clock = NativeClock::correlate(|| Ok(17_000)).unwrap();
        let observation = clock.local + Duration::from_millis(10);
        assert_eq!(
            clock.timestamp(Some(19_000), observation),
            FrameTime::Native(clock.local + Duration::from_nanos(2_000))
        );
        for stamp in [None, Some(0), Some(16_999), Some(10_018_000)] {
            assert_eq!(
                clock.timestamp(stamp, observation),
                FrameTime::Observed(observation)
            );
        }
    }
}
