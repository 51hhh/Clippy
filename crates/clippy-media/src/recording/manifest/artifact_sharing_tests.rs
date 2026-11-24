use super::super::artifact_sharing_fixture::FileHold;
use super::*;
use crate::recording::mux::avi_mjpeg::AviMjpegWriter;
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

fn avi_config(id: &str) -> RecordingJournalConfig {
    RecordingJournalConfig {
        session_id: id.to_string(),
        source_id: "synthetic".to_string(),
        physical_x: 0,
        physical_y: 0,
        width: 2,
        height: 2,
        target_fps_numerator: 10,
        target_fps_denominator: 1,
        encoder: "mjpeg-diagnostic".to_string(),
        container: "avi".to_string(),
        include_cursor: false,
        audio: None,
    }
}

fn avi_segment(journal: &RecordingJournal) -> (PendingSegment, File) {
    let (file, pending) = journal.begin_segment().unwrap();
    let mut writer = AviMjpegWriter::new(file, 2, 2, 10, 1, 85).unwrap();
    writer.push_rgba(&[120; 16], 0).unwrap();
    writer.push_rgba(&[140; 16], 100_000_000).unwrap();
    let output = writer.finish_with_stats(200_000_000).unwrap();
    assert_eq!(output.frame_count, 2);
    (pending, output.writer)
}

fn verify_segment(directory: &Path, segment: &SegmentManifest) {
    let path = directory.join(&segment.file_name);
    assert!(crate::private_files::is_private(&path));
    verify_library_artifact(&ResolvedRecordingArtifact {
        path,
        suggested_file_name: segment.file_name.clone(),
        byte_length: segment.byte_length,
        sha256: segment.sha256.clone(),
    })
    .unwrap();
}

#[test]
fn transient_native_partial_share_allows_journal_segment_commit() {
    let temporary = tempfile::tempdir().unwrap();
    let mut journal =
        RecordingJournal::create(temporary.path(), avi_config("artifact-segment")).unwrap();
    let directory = journal.session_directory().to_path_buf();
    let (pending, file) = avi_segment(&journal);
    let partial = pending.partial_path.clone();
    let hold = FileHold::after_manifest(&partial, &directory, |value| {
        !value["segments"].as_array().unwrap().is_empty()
    });
    let result = journal.commit_segment(pending, file, 200_000_000, 2, 0);
    let observed = hold.release();
    assert!(result.is_ok(), "实际分段提升原错：{result:?}");
    assert!(observed);
    journal.complete().unwrap();
    let manifest = read_manifest(&directory.join(MANIFEST_FILE)).unwrap();
    assert_eq!(manifest.state, RecordingState::Complete);
    assert_eq!(manifest.segments.len(), 1);
    assert_eq!(manifest.segments[0].frame_count, 2);
    assert_eq!(manifest.segments[0].duration_ns, 200_000_000);
    verify_segment(&directory, &manifest.segments[0]);
    assert!(!partial.exists());
}

#[test]
fn transient_share_allows_committed_segment_startup_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    let mut journal =
        RecordingJournal::create(temporary.path(), avi_config("artifact-recovery")).unwrap();
    let directory = journal.session_directory().to_path_buf();
    let (pending, file) = avi_segment(&journal);
    let partial = pending.partial_path.clone();
    let destination = journal
        .commit_segment(pending, file, 200_000_000, 2, 0)
        .unwrap();
    fs::rename(&destination, &partial).unwrap();
    let before = read_manifest(&directory.join(MANIFEST_FILE)).unwrap();
    let hold = FileHold::after_manifest(&partial, &directory, |_| true);
    let result = reconcile_session(&directory, OsStr::new("artifact-recovery"));
    let observed = hold.release();
    assert!(result.is_ok(), "实际已提交分段恢复原错：{result:?}");
    assert!(observed);
    assert_eq!(result.unwrap(), Some(1));
    let manifest = read_manifest(&directory.join(MANIFEST_FILE)).unwrap();
    assert_eq!(manifest.state, RecordingState::Interrupted);
    assert_eq!(manifest.segments[0].sha256, before.segments[0].sha256);
    assert_eq!(
        manifest.segments[0].byte_length,
        before.segments[0].byte_length
    );
    verify_segment(&directory, &manifest.segments[0]);
    assert!(!partial.exists());
}

#[test]
fn permanent_native_partial_share_preserves_committed_prefix_for_later_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    let mut journal =
        RecordingJournal::create(temporary.path(), avi_config("artifact-permanent")).unwrap();
    let directory = journal.session_directory().to_path_buf();
    let (pending, file) = avi_segment(&journal);
    let partial = pending.partial_path.clone();
    let held = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&partial)
        .unwrap();
    let result = journal.commit_segment(pending, file, 200_000_000, 2, 0);
    assert!(
        result.as_ref().unwrap_err().contains("os error 32"),
        "实际永久锁原错：{result:?}"
    );
    let before = read_manifest(&directory.join(MANIFEST_FILE)).unwrap();
    assert_eq!(before.segments.len(), 1);
    assert!(partial.exists());
    assert!(crate::private_files::is_private(&partial));
    assert!(!directory.join(&before.segments[0].file_name).exists());
    drop(held);
    assert_eq!(
        reconcile_session(&directory, OsStr::new("artifact-permanent")).unwrap(),
        Some(1)
    );
    let after = read_manifest(&directory.join(MANIFEST_FILE)).unwrap();
    assert_eq!(after.segments[0].sha256, before.segments[0].sha256);
    verify_segment(&directory, &after.segments[0]);
    assert!(!partial.exists());
}

#[test]
fn invalid_committed_partial_is_not_promoted_even_when_native_share_is_blocked() {
    for same_length in [true, false] {
        let temporary = tempfile::tempdir().unwrap();
        let mut journal =
            RecordingJournal::create(temporary.path(), avi_config("artifact-invalid")).unwrap();
        let directory = journal.session_directory().to_path_buf();
        let (pending, file) = avi_segment(&journal);
        let partial = pending.partial_path.clone();
        let destination = journal
            .commit_segment(pending, file, 200_000_000, 2, 0)
            .unwrap();
        fs::rename(&destination, &partial).unwrap();
        let manifest = read_manifest(&directory.join(MANIFEST_FILE)).unwrap();
        let mut bytes = fs::read(&partial).unwrap();
        if same_length {
            bytes[0] ^= 1;
        } else {
            bytes.pop();
        }
        fs::write(&partial, bytes).unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&partial)
            .unwrap();
        assert!(!promote_committed_partial(&directory, &manifest, &manifest.segments[0]).unwrap());
        assert!(partial.exists());
        assert!(!destination.exists());
        drop(held);
    }
}

#[cfg(feature = "recording-vp9-prototype")]
fn vp9_final(journal: &RecordingJournal) -> (PendingFinalOutput, File) {
    let (file, pending) = journal.begin_final_output().unwrap();
    let mut writer =
        crate::recording::mux::vp9_webm::Vp9WebmWriter::new(file, 64, 48, 10, 1).unwrap();
    let rgba = vec![200; 64 * 48 * 4];
    writer.push_rgba(&rgba, 0).unwrap();
    writer.push_rgba(&rgba, 100_000_000).unwrap();
    let output = writer.finish_with_stats(200_000_000).unwrap();
    (pending, output.writer)
}

#[cfg(feature = "recording-vp9-prototype")]
fn verify_final(directory: &Path, manifest: &RecordingManifest) {
    use crate::recording::mux::webm_remux::{remux_vp9_segments, WebmRemuxSource, WebmRemuxSpec};
    let output = manifest.final_output.as_ref().unwrap();
    let path = directory.join(&output.file_name);
    assert!(crate::private_files::is_private(&path));
    verify_library_artifact(&ResolvedRecordingArtifact {
        path: path.clone(),
        suggested_file_name: output.file_name.clone(),
        byte_length: output.byte_length,
        sha256: output.sha256.clone(),
    })
    .unwrap();
    let parsed = remux_vp9_segments(
        &[WebmRemuxSource {
            path,
            byte_length: output.byte_length,
            sha256: output.sha256.clone(),
            started_at_ns: 0,
            duration_ns: output.duration_ns,
            frame_count: output.frame_count,
        }],
        std::io::Cursor::new(Vec::new()),
        WebmRemuxSpec {
            width: 64,
            height: 48,
            fps_numerator: 10,
            fps_denominator: 1,
        },
    )
    .unwrap();
    assert_eq!(parsed.frame_count, output.frame_count);
    assert_eq!(parsed.duration_ns, output.duration_ns);
    for segment in &manifest.segments {
        verify_segment(directory, segment);
    }
}

#[cfg(feature = "recording-vp9-prototype")]
#[test]
fn transient_native_partial_share_allows_final_output_commit() {
    let temporary = tempfile::tempdir().unwrap();
    let mut journal =
        RecordingJournal::create(temporary.path(), vp9_journal_config("artifact-final")).unwrap();
    commit_vp9_segment(&mut journal, [30, 60, 90]);
    let directory = journal.session_directory().to_path_buf();
    let (pending, file) = vp9_final(&journal);
    let partial = pending.partial_path.clone();
    let hold = FileHold::after_manifest(&partial, &directory, |value| {
        !value["finalOutput"].is_null()
    });
    let result = journal.commit_final_output(pending, file, 200_000_000, 2);
    let observed = hold.release();
    assert!(result.is_ok(), "实际最终输出提升原错：{result:?}");
    assert!(observed);
    journal.complete().unwrap();
    let manifest = read_manifest(&directory.join(MANIFEST_FILE)).unwrap();
    assert_eq!(manifest.state, RecordingState::Complete);
    verify_final(&directory, &manifest);
    assert!(!partial.exists());
}

#[cfg(feature = "recording-vp9-prototype")]
#[test]
fn transient_share_allows_committed_final_output_startup_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    let mut journal = RecordingJournal::create(
        temporary.path(),
        vp9_journal_config("artifact-final-recovery"),
    )
    .unwrap();
    commit_vp9_segment(&mut journal, [30, 60, 90]);
    let directory = journal.session_directory().to_path_buf();
    let (pending, file) = vp9_final(&journal);
    let partial = pending.partial_path.clone();
    let destination = journal
        .commit_final_output(pending, file, 200_000_000, 2)
        .unwrap();
    fs::rename(&destination, &partial).unwrap();
    let before = read_manifest(&directory.join(MANIFEST_FILE)).unwrap();
    let hold = FileHold::after_manifest(&partial, &directory, |_| true);
    let result = reconcile_session(&directory, OsStr::new("artifact-final-recovery"));
    let observed = hold.release();
    assert!(result.is_ok(), "实际最终输出恢复原错：{result:?}");
    assert!(observed);
    assert_eq!(result.unwrap(), None);
    let manifest = read_manifest(&directory.join(MANIFEST_FILE)).unwrap();
    assert_eq!(manifest.state, RecordingState::Complete);
    assert_eq!(
        manifest.final_output.as_ref().unwrap().sha256,
        before.final_output.as_ref().unwrap().sha256
    );
    verify_final(&directory, &manifest);
    assert!(!partial.exists());
}

#[cfg(feature = "recording-vp9-prototype")]
#[test]
fn transient_share_allows_recovery_merge_output_promotion() {
    let temporary = tempfile::tempdir().unwrap();
    let mut journal =
        RecordingJournal::create(temporary.path(), vp9_journal_config("artifact-remux")).unwrap();
    for index in 0..4 {
        commit_vp9_segment(&mut journal, [20 + index, 60, 90]);
    }
    journal.interrupt().unwrap();
    let directory = journal.session_directory().to_path_buf();
    let partial = directory.join(final_output_partial_name("webm"));
    let hold =
        FileHold::future_partial(&partial, &directory, |value| value["state"] == "finalizing");
    let result = merge_interrupted_vp9_session(temporary.path(), "artifact-remux");
    let observed = hold.release();
    assert!(result.is_ok(), "实际恢复合并提升原错：{result:?}");
    assert!(observed, "真实恢复 partial 必须已取得原生删除共享冲突证据");
    let manifest = read_manifest(&directory.join(MANIFEST_FILE)).unwrap();
    assert_eq!(manifest.state, RecordingState::Complete);
    assert_eq!(manifest.segments.len(), 4);
    assert_eq!(manifest.final_output.as_ref().unwrap().frame_count, 8);
    verify_final(&directory, &manifest);
    assert!(!partial.exists());
}
