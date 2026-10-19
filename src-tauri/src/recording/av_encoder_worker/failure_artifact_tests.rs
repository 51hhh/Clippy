use super::*;
use crate::recording::test_artifacts::TestDirectory;
use std::panic::{catch_unwind, AssertUnwindSafe};
include!("../idle_frontier_read_tests.rs");

#[test]
fn recording_artifacts_av_unwind_joins_before_retaining_committed_prefix() {
    let enclosing = ::tempfile::tempdir().unwrap();
    let mut directory = None;
    let result = catch_unwind(AssertUnwindSafe(|| {
        let temporary = TestDirectory::from(::tempfile::tempdir_in(enclosing.path()).unwrap());
        let (writer, video, audio_pipeline, path) = setup(temporary.path(), "failure-artifact-av");
        let _worker =
            AvEncoderWorker::spawn(writer, Arc::clone(&video), Arc::clone(&audio_pipeline), 0)
                .unwrap();
        video.push(frame(0, 0, 1)).unwrap();
        video.publish_capture_lower_bound(600_000_000).unwrap();
        for sequence in 0..6 {
            audio_pipeline
                .push(audio(sequence, sequence * 100_000_000, 4_800))
                .unwrap();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let value: Value =
                serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap();
            if !value["segments"].as_array().unwrap().is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "controlled fixture must commit before owner unwind"
            );
            thread::sleep(Duration::from_millis(5));
        }
        directory = Some(path);
        panic!("controlled AV owner unwind after a real synthetic committed segment");
    }));
    assert!(result.is_err());
    let directory = directory.unwrap();
    let manifest: Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["state"], "interrupted");
    let segments = manifest["segments"].as_array().unwrap();
    assert!(!segments.is_empty());
    for segment in segments {
        read_idle_artifact(&directory, &manifest, segment, 10);
    }
}
