use super::*;
use crate::recording::manifest::artifact_sharing_fixture::FileHold;
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct ReleasedSource {
    source: FixtureSource,
    release: Option<mpsc::Receiver<()>>,
}

impl RecordingFrameSource for ReleasedSource {
    type Error = FixtureSourceError;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        if let Some(release) = self.release.take() {
            release
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| FixtureSourceError)?;
        }
        Ok(self.source.capture_next().unwrap())
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(self.source.control_timestamp_ns().unwrap())
    }
}

#[test]
fn transient_partial_sharing_does_not_terminate_original_periodic_session() {
    let temporary = tempfile::tempdir().unwrap();
    let mut configuration = config("artifact-share-session");
    configuration.segment_duration_ns = 200_000_000;
    let (start, release) = mpsc::channel();
    let session = DiagnosticRecordingSession::start(
        temporary.path(),
        configuration,
        ReleasedSource {
            source: FixtureSource {
                sequence: 0,
                timestamp_ns: 100,
            },
            release: Some(release),
        },
    )
    .unwrap();
    let directory = session.session_directory().to_path_buf();
    let partial = directory.join(".segment-000000.avi.partial");
    let hold = FileHold::after_manifest(&partial, &directory, |manifest| {
        !manifest["segments"].as_array().unwrap().is_empty()
    });
    start.send(()).unwrap();
    let deadline = Instant::now() + ASYNC_TEST_TIMEOUT;
    let committed = loop {
        if directory.join("segment-000000.avi").exists() {
            break true;
        }
        if session.has_terminated_worker() || Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let observed = hold.release();
    let result = session.stop();
    assert!(
        committed,
        "短暂 partial 共享冲突不得中止原 session；实际原错：{result:?}"
    );
    assert!(observed);
    let report = result.unwrap();
    assert_eq!(report.dropped_by_backpressure, 0);
    assert_eq!(report.captured_frames, report.encoder_input_frames);
    assert_eq!(report.encoded_frames, report.encoder_input_frames);
    assert!(report.segment_paths.len() >= 2);
    let manifest = manifest_value(&directory);
    assert_eq!(manifest["state"], "complete");
    let segments = manifest["segments"].as_array().unwrap();
    assert_eq!(segments.len(), report.segment_paths.len());
    let mut frames = 0;
    let mut duration = 0;
    for segment in segments {
        frames += segment["frameCount"].as_u64().unwrap();
        duration += segment["durationNs"].as_u64().unwrap();
        let file_name = segment["fileName"].as_str().unwrap();
        let path = directory.join(file_name);
        assert_eq!(&std::fs::read(&path).unwrap()[..4], b"RIFF");
        crate::recording::manifest::verify_library_artifact(
            &crate::recording::manifest::ResolvedRecordingArtifact {
                path,
                suggested_file_name: file_name.to_string(),
                byte_length: segment["byteLength"].as_u64().unwrap(),
                sha256: segment["sha256"].as_str().unwrap().to_string(),
            },
        )
        .unwrap();
    }
    assert_eq!(frames, report.encoded_frames);
    assert_eq!(duration, report.duration_ns);
}
