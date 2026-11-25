use std::{
    sync::{Condvar, Mutex},
    time::Duration,
};

enum State<T> {
    Open(Option<T>),
    Closed(String),
}

pub(super) struct FrameSlot<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
}

impl<T> Default for FrameSlot<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::Open(None)),
            changed: Condvar::new(),
        }
    }
}

impl<T> FrameSlot<T> {
    pub(super) fn publish(&self, frame: T) {
        let replaced = match &mut *self.state.lock().unwrap() {
            State::Open(pending) => {
                let old = pending.replace(frame);
                self.changed.notify_one();
                old
            }
            State::Closed(_) => Some(frame),
        };
        // 归还原生帧可能唤醒 FrameArrived，因此必须在解锁后释放。
        drop(replaced);
    }

    pub(super) fn take(&self, timeout: Duration) -> Result<Option<T>, String> {
        let (mut state, _) = self
            .changed
            .wait_timeout_while(self.state.lock().unwrap(), timeout, |state| {
                matches!(state, State::Open(None))
            })
            .unwrap();
        match &mut *state {
            State::Open(pending) => Ok(pending.take()),
            State::Closed(reason) => Err(reason.clone()),
        }
    }

    pub(super) fn check_open(&self) -> Result<(), String> {
        match &*self.state.lock().unwrap() {
            State::Open(_) => Ok(()),
            State::Closed(reason) => Err(reason.clone()),
        }
    }

    pub(super) fn close(&self, reason: String) {
        let replaced = {
            let mut state = self.state.lock().unwrap();
            if matches!(*state, State::Closed(_)) {
                return;
            }
            let old = std::mem::replace(&mut *state, State::Closed(reason));
            self.changed.notify_all();
            old
        };
        drop(replaced);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };

    struct Frame(usize, Arc<AtomicUsize>);

    impl Drop for Frame {
        fn drop(&mut self) {
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn replaces_and_releases_the_pending_frame() {
        let slot = FrameSlot::default();
        let dropped = Arc::new(AtomicUsize::new(0));
        slot.publish(Frame(1, dropped.clone()));
        slot.publish(Frame(2, dropped.clone()));
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        let latest = slot.take(Duration::ZERO).unwrap().unwrap();
        assert_eq!(latest.0, 2);
        assert!(slot.take(Duration::ZERO).unwrap().is_none());
        assert!(slot.check_open().is_ok());
    }

    #[test]
    fn close_releases_pending_frames_and_rejects_late_callbacks() {
        let slot = FrameSlot::default();
        let dropped = Arc::new(AtomicUsize::new(0));
        slot.publish(Frame(1, dropped.clone()));
        slot.close("monitor closed".into());
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        slot.publish(Frame(2, dropped.clone()));
        assert_eq!(dropped.load(Ordering::SeqCst), 2);
        slot.close("duplicate event".into());
        assert_eq!(
            slot.take(Duration::ZERO).err().as_deref(),
            Some("monitor closed")
        );
    }

    #[test]
    fn close_wakes_a_reader_waiting_for_the_first_frame() {
        let slot = Arc::new(FrameSlot::<()>::default());
        let (send, receive) = mpsc::channel();
        std::thread::scope(|scope| {
            let reading = slot.clone();
            scope.spawn(move || {
                send.send(reading.take(Duration::from_secs(5))).unwrap();
            });
            slot.close("device lost".into());
            assert_eq!(
                receive.recv_timeout(Duration::from_secs(1)).unwrap(),
                Err("device lost".into())
            );
        });
    }
}
