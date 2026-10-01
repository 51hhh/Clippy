use super::*;
use std::sync::{mpsc, Arc};

type Preparation = RecordingMediaPreparation;

fn prepare(manager: &RecordingMediaManager, session: &str) -> Preparation {
    manager.prepare_session(session).unwrap()
}

fn issue(
    manager: &RecordingMediaManager,
    preparation: &Preparation,
    session: &str,
    artifact: &ResolvedRecordingArtifact,
) -> Result<RecordingMediaLeaseInfo, String> {
    assert_eq!(preparation.session.session_id, session);
    manager.issue(preparation, artifact)
}

fn artifact(directory: &std::path::Path) -> ResolvedRecordingArtifact {
    let bytes = b"synthetic media lease contract";
    let path = directory.join("recording.webm");
    fs::write(&path, bytes).unwrap();
    ResolvedRecordingArtifact {
        path,
        suggested_file_name: "Clippy-session.webm".to_string(),
        byte_length: bytes.len() as u64,
        sha256: format!("{:x}", sha2::Sha256::digest(bytes)),
    }
}

fn expect_revoked(result: Result<RecordingMediaLeaseInfo, String>) {
    assert!(result.is_err(), "撤销前准备不应重新签发: {result:?}");
}

#[test]
fn media_revoke_cancels_pending_session_without_existing_lease() {
    let directory = tempfile::tempdir().unwrap();
    let artifact = artifact(directory.path());
    let manager = RecordingMediaManager::default();
    let pending = prepare(&manager, "session-a");
    manager.revoke_session("session-a").unwrap();
    expect_revoked(issue(&manager, &pending, "session-a", &artifact));
    assert!(manager.inner.lock().unwrap().leases.is_empty());
}

#[test]
fn media_revoke_cancels_issued_and_pending_leases_together() {
    let directory = tempfile::tempdir().unwrap();
    let artifact = artifact(directory.path());
    let manager = RecordingMediaManager::default();
    let pending = prepare(&manager, "session-a");
    let issued = issue(&manager, &pending, "session-a", &artifact).unwrap();
    manager.revoke_session("session-a").unwrap();
    assert!(manager.get(&issued.token).unwrap().is_none());
    expect_revoked(issue(&manager, &pending, "session-a", &artifact));
}

#[test]
fn media_revoke_new_preparation_does_not_restore_old_one() {
    let directory = tempfile::tempdir().unwrap();
    let artifact = artifact(directory.path());
    let manager = RecordingMediaManager::default();
    let old = prepare(&manager, "session-a");
    manager.revoke_session("session-a").unwrap();
    let fresh = prepare(&manager, "session-a");
    let current = issue(&manager, &fresh, "session-a", &artifact).unwrap();
    expect_revoked(issue(&manager, &old, "session-a", &artifact));
    assert!(manager.get(&current.token).unwrap().is_some());
}

#[test]
fn media_revoke_other_session_preserves_pending_and_issued_lease() {
    let directory = tempfile::tempdir().unwrap();
    let artifact = artifact(directory.path());
    let manager = RecordingMediaManager::default();
    let pending = prepare(&manager, "session-b");
    let issued = issue(&manager, &pending, "session-b", &artifact).unwrap();
    manager.revoke_session("session-a").unwrap();
    let later = issue(&manager, &pending, "session-b", &artifact).unwrap();
    assert!(manager.get(&issued.token).unwrap().is_some());
    assert!(manager.get(&later.token).unwrap().is_some());
}

#[test]
fn media_revoke_window_clear_cancels_every_pending_session() {
    let directory = tempfile::tempdir().unwrap();
    let artifact = artifact(directory.path());
    let manager = RecordingMediaManager::default();
    let first = prepare(&manager, "session-a");
    let second = prepare(&manager, "session-b");
    let issued = issue(&manager, &first, "session-a", &artifact).unwrap();
    manager.clear().unwrap();
    expect_revoked(issue(&manager, &first, "session-a", &artifact));
    expect_revoked(issue(&manager, &second, "session-b", &artifact));
    assert!(manager.get(&issued.token).unwrap().is_none());
    let fresh = prepare(&manager, "session-a");
    assert!(issue(&manager, &fresh, "session-a", &artifact).is_ok());
}

#[test]
fn media_revoke_queued_worker_cannot_issue_after_revocation() {
    let directory = tempfile::tempdir().unwrap();
    let artifact = artifact(directory.path());
    let manager = Arc::new(RecordingMediaManager::default());
    let pending = prepare(&manager, "session-a");
    let worker_manager = Arc::clone(&manager);
    let (ready_tx, ready_rx) = mpsc::channel();
    let (continue_tx, continue_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        ready_tx.send(()).unwrap();
        continue_rx.recv().unwrap();
        issue(&worker_manager, &pending, "session-a", &artifact)
    });
    ready_rx.recv().unwrap();
    manager.revoke_session("session-a").unwrap();
    continue_tx.send(()).unwrap();
    expect_revoked(worker.join().unwrap());
    assert!(manager.inner.lock().unwrap().leases.is_empty());
}

#[test]
fn media_revoke_preparation_tracking_releases_only_finished_identity() {
    let manager = RecordingMediaManager::default();
    for index in 0..64 {
        let pending = prepare(&manager, &format!("session-{index}"));
        assert_eq!(manager.inner.lock().unwrap().preparations.len(), 1);
        drop(pending);
        assert!(manager.inner.lock().unwrap().preparations.is_empty());
    }
    let old = prepare(&manager, "session-a");
    manager.revoke_session("session-a").unwrap();
    let fresh = prepare(&manager, "session-a");
    let sibling = prepare(&manager, "session-a");
    drop(old);
    manager.ensure_preparation_current(&fresh).unwrap();
    drop(fresh);
    manager.ensure_preparation_current(&sibling).unwrap();
    assert_eq!(manager.inner.lock().unwrap().preparations.len(), 1);
    drop(sibling);
    assert!(manager.inner.lock().unwrap().preparations.is_empty());
}

#[test]
fn media_revoke_preparation_cannot_cross_manager_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let artifact = artifact(directory.path());
    let owner = RecordingMediaManager::default();
    let other = RecordingMediaManager::default();
    let preparation = prepare(&owner, "session-a");
    let unrelated = prepare(&other, "session-a");
    expect_revoked(issue(&other, &preparation, "session-a", &artifact));
    assert!(other.inner.lock().unwrap().leases.is_empty());
    assert!(issue(&owner, &preparation, "session-a", &artifact).is_ok());
    assert!(issue(&other, &unrelated, "session-a", &artifact).is_ok());
}
