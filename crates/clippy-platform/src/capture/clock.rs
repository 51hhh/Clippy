use super::FrameTime;
use anyhow::Result;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(super) struct NativeClock {
    local: Instant,
    native_ns: u64,
}

impl NativeClock {
    pub(super) fn correlate(sample_native: impl FnOnce() -> Result<u64>) -> Result<Self> {
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

    #[cfg(target_os = "windows")]
    pub fn performance_counter() -> Result<Self> {
        use anyhow::Context;
        use std::io;
        use windows_sys::Win32::System::Performance::{
            QueryPerformanceCounter, QueryPerformanceFrequency,
        };

        let mut frequency = 0;
        if unsafe { QueryPerformanceFrequency(&mut frequency) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        let frequency = u64::try_from(frequency)?;
        Self::correlate(|| {
            let mut ticks = 0;
            if unsafe { QueryPerformanceCounter(&mut ticks) } == 0 {
                return Err(io::Error::last_os_error().into());
            }
            ticks_to_ns(u64::try_from(ticks)?, frequency).context("Invalid performance counter")
        })
    }

    #[cfg(target_os = "windows")]
    pub fn windows_frame_time(self, time_100ns: Option<i64>, observed: Instant) -> FrameTime {
        let native_ns = time_100ns
            .and_then(|time| u64::try_from(time).ok())
            .and_then(|time| time.checked_mul(100));
        self.timestamp(native_ns, observed)
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

#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub(super) fn ticks_to_ns(ticks: u64, frequency: u64) -> Option<u64> {
    let ns = (u128::from(ticks) * 1_000_000_000).checked_div(u128::from(frequency))?;
    u64::try_from(ns).ok()
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

    #[test]
    fn scales_large_raw_counters_without_rounding_to_milliseconds() {
        assert_eq!(ticks_to_ns(1, 24_000_000), Some(41));
        assert_eq!(
            ticks_to_ns(987_654_321_123_456, 10_000_000),
            Some(98_765_432_112_345_600)
        );
        assert_eq!(ticks_to_ns(u64::MAX, 1_000_000_000), Some(u64::MAX));
        assert_eq!(ticks_to_ns(u64::MAX, 1), None);
        assert_eq!(ticks_to_ns(1, 0), None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn converts_winrt_intervals_in_the_performance_counter_domain() {
        let start = NativeClock::performance_counter().unwrap();
        let later = NativeClock::performance_counter().unwrap();
        assert!(later.native_ns >= start.native_ns);

        let clock = NativeClock::correlate(|| Ok(50_000)).unwrap();
        let observed = clock.local + Duration::from_millis(1);
        assert_eq!(
            clock.windows_frame_time(Some(516), observed),
            FrameTime::Native(clock.local + Duration::from_nanos(1_600))
        );
        for invalid in [None, Some(-1), Some(0), Some(i64::MAX)] {
            assert_eq!(
                clock.windows_frame_time(invalid, observed),
                FrameTime::Observed(observed)
            );
        }
    }
}
