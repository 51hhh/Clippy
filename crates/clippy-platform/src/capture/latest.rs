use anyhow::{anyhow, Context, Result};
use std::{
    sync::{Condvar, Mutex},
    time::Duration,
};

struct FrameState<T> {
    sequence: u64,
    frame: Result<Option<T>, String>,
}

pub(super) struct LatestFrames<T> {
    state: Mutex<FrameState<T>>,
    changed: Condvar,
}

impl<T: Clone> LatestFrames<T> {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FrameState {
                sequence: 0,
                frame: Ok(None),
            }),
            changed: Condvar::new(),
        }
    }

    pub fn publish(&self, frame: Result<Option<T>, String>) {
        let previous = {
            let mut state = self.state.lock().unwrap();
            if state.frame.is_err() {
                return;
            }
            state.sequence += 1;
            std::mem::replace(&mut state.frame, frame)
        };
        drop(previous);
        self.changed.notify_one();
    }

    pub fn wait(&self, seen: u64, timeout: Duration) -> Result<(u64, T)> {
        let (state, _) = self
            .changed
            .wait_timeout_while(self.state.lock().unwrap(), timeout, |state| {
                matches!(&state.frame, Ok(None))
                    || (matches!(&state.frame, Ok(Some(_))) && state.sequence == seen)
            })
            .unwrap();
        Ok((state.sequence, Self::active_frame(&state)?.clone()))
    }

    pub fn check_active(&self) -> Result<()> {
        Self::active_frame(&self.state.lock().unwrap()).map(|_| ())
    }

    fn active_frame(state: &FrameState<T>) -> Result<&T> {
        state
            .frame
            .as_ref()
            .map_err(|message| anyhow!("{message}"))?
            .as_ref()
            .context("No active capture frame")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, thread};

    #[test]
    fn latest_replaces_surfaces_and_keeps_static_frame() {
        let frames = LatestFrames::new();
        let previous = Arc::new(12);
        let newest = Arc::new(34);
        frames.publish(Ok(Some(previous.clone())));
        frames.publish(Ok(Some(newest.clone())));
        assert_eq!(Arc::strong_count(&previous), 1);
        let (sequence, sample) = frames.wait(0, Duration::ZERO).unwrap();
        assert!(Arc::ptr_eq(&sample, &newest));
        assert!(Arc::ptr_eq(
            &frames.wait(sequence, Duration::ZERO).unwrap().1,
            &newest
        ));
        assert!(frames.check_active().is_ok());
        frames.publish(Ok(None));
        assert!(frames.check_active().is_err());
        assert!(frames.wait(sequence, Duration::ZERO).is_err());
        assert_eq!(Arc::strong_count(&newest), 2);
    }

    #[test]
    fn stopped_frames_wake_waiters_and_drop_pending_surface() {
        let frames = LatestFrames::new();
        let sample = Arc::new(12);
        frames.publish(Ok(Some(sample.clone())));
        let sequence = frames.wait(0, Duration::ZERO).unwrap().0;
        thread::scope(|scope| {
            let wait = scope.spawn(|| frames.wait(sequence, Duration::from_secs(3)));
            frames.publish(Err("Display disconnected".into()));
            frames.publish(Ok(Some(sample.clone())));
            assert_eq!(
                wait.join().unwrap().unwrap_err().to_string(),
                "Display disconnected"
            );
        });
        assert_eq!(Arc::strong_count(&sample), 1);
    }
}
