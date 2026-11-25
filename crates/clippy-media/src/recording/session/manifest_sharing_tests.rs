use super::*;
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

struct SharingSource {
    inner: FixtureSource,
    start: Option<mpsc::Receiver<()>>,
    progress: mpsc::Sender<u64>,
}

impl RecordingFrameSource for SharingSource {
    type Error = FixtureSourceError;
    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        if let Some(start) = self.start.take() {
            start
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| FixtureSourceError)?;
        }
        let frame = self.inner.capture_next().unwrap();
        let _ = self.progress.send(frame.sequence);
        Ok(frame)
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(self.inner.control_timestamp_ns().unwrap())
    }
}

#[test]
fn transient_native_manifest_share_conflict_does_not_end_periodic_session() {
    let temporary = tempfile::tempdir().unwrap();
    let mut configuration = config("manifest-share-runtime");
    configuration.segment_duration_ns = 200_000_000;
    let (release, start) = mpsc::channel();
    let (progress, events) = mpsc::channel();
    let session = DiagnosticRecordingSession::start(
        temporary.path(),
        configuration,
        SharingSource {
            inner: FixtureSource {
                sequence: 0,
                timestamp_ns: 100,
            },
            start: Some(start),
            progress,
        },
    )
    .unwrap();
    let directory = session.session_directory().to_path_buf();
    // 使用原生共享模式，真实阻止 MoveFileEx 的删除/替换；不替换生产 I/O。
    let held = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(directory.join("manifest.json"))
        .unwrap();
    let locker = std::thread::spawn(move || {
        while events.recv_timeout(Duration::from_secs(5)).unwrap() != 2 {}
        std::thread::sleep(Duration::from_millis(75));
        drop(held);
    });
    release.send(()).unwrap();
    let deadline = Instant::now() + ASYNC_TEST_TIMEOUT;
    let committed = loop {
        let value = manifest_value(&directory);
        if !value["segments"].as_array().unwrap().is_empty()
            && directory.join("segment-000000.avi").exists()
        {
            break true;
        }
        if session.has_terminated_worker() || Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    locker.join().unwrap();
    let result = session.stop();
    assert!(
        committed,
        "真实暂时共享冲突不得中止周期 session；worker 原错：{result:?}"
    );
    let report = result.unwrap();
    assert_eq!(report.dropped_by_backpressure, 0);
    assert_eq!(report.captured_frames, report.encoder_input_frames);
    assert_eq!(report.encoded_frames, report.encoder_input_frames);
    assert!(report.segment_paths.len() >= 2);
    let value = manifest_value(&directory);
    assert_eq!(value["state"], "complete");
    assert_eq!(
        value["segments"].as_array().unwrap().len(),
        report.segment_paths.len()
    );
    let mut frames = 0;
    let mut duration = 0;
    for segment in value["segments"].as_array().unwrap() {
        frames += segment["frameCount"].as_u64().unwrap();
        duration += segment["durationNs"].as_u64().unwrap();
        let path = directory.join(segment["fileName"].as_str().unwrap());
        assert_eq!(&std::fs::read(&path).unwrap()[..4], b"RIFF");
        crate::recording::manifest::verify_library_artifact(
            &crate::recording::manifest::ResolvedRecordingArtifact {
                path,
                suggested_file_name: segment["fileName"].as_str().unwrap().to_string(),
                byte_length: segment["byteLength"].as_u64().unwrap(),
                sha256: segment["sha256"].as_str().unwrap().to_string(),
            },
        )
        .unwrap();
    }
    assert_eq!(frames, report.encoded_frames);
    assert_eq!(duration, report.duration_ns);
}
