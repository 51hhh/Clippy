use super::*;
use crate::recording::test_artifacts::TestDirectory;
use std::panic::{catch_unwind, AssertUnwindSafe};

#[test]
fn recording_artifacts_session_unwind_joins_before_retaining_committed_prefix() {
    let enclosing = ::tempfile::tempdir().unwrap();
    let mut directory = None;
    let result = catch_unwind(AssertUnwindSafe(|| {
        let temporary = TestDirectory::from(::tempfile::tempdir_in(enclosing.path()).unwrap());
        let mut configuration = config("failure-artifact-session");
        configuration.segment_duration_ns = 200_000_000;
        let session = DiagnosticRecordingSession::start(
            temporary.path(),
            configuration,
            FixtureSource {
                sequence: 0,
                timestamp_ns: 100,
            },
        )
        .unwrap();
        let path = session.session_directory().to_path_buf();
        wait_for_committed_segment(&path, 0, "avi", &[], ASYNC_TEST_TIMEOUT);
        directory = Some(path);
        panic!("controlled owner unwind after a real synthetic committed segment");
    }));
    assert!(result.is_err());
    let directory = directory.unwrap();
    let manifest = manifest_value(&directory);
    assert_eq!(manifest["state"], "interrupted");
    let segments = manifest["segments"].as_array().unwrap();
    assert!(!segments.is_empty());
    for segment in segments {
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
}
