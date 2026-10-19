use std::thread::JoinHandle;

/// 两条桥接线程均结束后，报告是否有任一线程 panic。
pub(super) fn join_bridges_panicked(video: JoinHandle<()>, audio: JoinHandle<()>) -> bool {
    // join 必须先分别执行；把调用放进 || 会在视频 panic 时丢弃音频 handle。
    let video_result = video.join();
    let audio_result = audio.join();
    video_result.is_err() || audio_result.is_err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{self, RecvTimeoutError, Sender};
    use std::thread;
    use std::time::Duration;

    const FIXTURE_DEADLINE: Duration = Duration::from_secs(5);
    const BLOCKED_OBSERVATION: Duration = Duration::from_millis(500);

    struct ExitSignal(Sender<()>);

    impl Drop for ExitSignal {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }

    fn assert_complete_join(video_panics: bool, audio_panics: bool) {
        let (video_ready, video_started) = mpsc::channel();
        let video = thread::spawn(move || {
            video_ready.send(()).unwrap();
            assert!(!video_panics, "injected video bridge panic");
        });
        let (audio_ready, audio_started) = mpsc::channel();
        let (release_audio, audio_gate) = mpsc::channel();
        let (audio_cleanup, audio_exited) = mpsc::channel();
        let audio = thread::spawn(move || {
            let _cleanup = ExitSignal(audio_cleanup);
            audio_ready.send(()).unwrap();
            audio_gate
                .recv_timeout(FIXTURE_DEADLINE)
                .expect("audio gate was not released");
            assert!(!audio_panics, "injected audio bridge panic");
        });
        video_started.recv_timeout(FIXTURE_DEADLINE).unwrap();
        audio_started.recv_timeout(FIXTURE_DEADLINE).unwrap();

        let (owner_ready, owner_started) = mpsc::channel();
        let (owner_result, owner_finished) = mpsc::channel();
        let owner = thread::spawn(move || {
            owner_ready.send(()).unwrap();
            let panicked = join_bridges_panicked(video, audio);
            owner_result.send(panicked).unwrap();
        });
        owner_started.recv_timeout(FIXTURE_DEADLINE).unwrap();
        let early = owner_finished.recv_timeout(BLOCKED_OBSERVATION);
        let returned_before_release = early.is_ok();

        // 即使旧代码提前返回，先释放音频并等待 fixture 清理，再报告断言失败。
        release_audio.send(()).unwrap();
        audio_exited.recv_timeout(FIXTURE_DEADLINE).unwrap();
        let panicked = match early {
            Ok(panicked) => panicked,
            Err(RecvTimeoutError::Timeout) => {
                owner_finished.recv_timeout(FIXTURE_DEADLINE).unwrap()
            }
            Err(RecvTimeoutError::Disconnected) => panic!("join owner disconnected"),
        };
        owner.join().unwrap();
        assert!(
            !returned_before_release,
            "owner returned while audio bridge was still gated"
        );
        assert_eq!(panicked, video_panics || audio_panics);
    }

    #[test]
    fn normal_bridges_wait_for_audio_cleanup() {
        assert_complete_join(false, false);
    }

    #[test]
    fn video_panic_still_waits_for_audio_cleanup() {
        assert_complete_join(true, false);
    }

    #[test]
    fn audio_panic_is_reported_after_cleanup() {
        assert_complete_join(false, true);
    }

    #[test]
    fn both_panics_still_wait_for_audio_cleanup() {
        assert_complete_join(true, true);
    }
}
