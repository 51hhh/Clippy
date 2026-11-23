use super::{delete_session_with, RecordingLibraryError, RecordingMergeRegistry};
use crate::recording::manifest::{self, RecordingJournal, RecordingJournalConfig};
use crate::recording::mux::vp9_webm::Vp9WebmWriter;
use std::cell::Cell;
use std::fs;
use std::io::Seek;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::Duration;

// 使用生产 journal/writer 提交两个真实 VP9 分段，避免把虚构文件当作可恢复录屏。
fn interrupted_recording(root: &Path, session_id: &str) -> PathBuf {
    let mut journal = RecordingJournal::create(
        root,
        RecordingJournalConfig {
            session_id: session_id.to_string(),
            source_id: "delete-owner-fixture".to_string(),
            physical_x: -64,
            physical_y: 0,
            width: 64,
            height: 48,
            target_fps_numerator: 10,
            target_fps_denominator: 1,
            encoder: "vp9-prototype".to_string(),
            container: "webm".to_string(),
            include_cursor: false,
            audio: None,
        },
    )
    .unwrap();
    for color in [[16, 32, 64], [200, 180, 160]] {
        let rgba = (0..64 * 48)
            .flat_map(|_| [color[0], color[1], color[2], 255])
            .collect::<Vec<_>>();
        let (file, pending) = journal.begin_segment().unwrap();
        let mut writer = Vp9WebmWriter::new(file, 64, 48, 10, 1).unwrap();
        writer.push_rgba(&rgba, 0).unwrap();
        writer.push_rgba(&rgba, 100_000_000).unwrap();
        let mut output = writer.finish_with_stats(200_000_000).unwrap();
        output.writer.rewind().unwrap();
        journal
            .commit_segment(pending, output.writer, 200_000_000, output.frame_count, 0)
            .unwrap();
    }
    journal.interrupt().unwrap();
    assert!(manifest::list_library(root).unwrap()[0].can_merge);
    journal.session_directory().to_path_buf()
}

fn delete_files(root: &Path, session_id: &str) -> Result<(), RecordingLibraryError> {
    manifest::delete_library_session(root, session_id).map_err(RecordingLibraryError::storage)
}

#[test]
fn recording_delete_owner_merge_protects_files_then_completed_merge_can_be_deleted() {
    let temporary = tempfile::tempdir().unwrap();
    let session = interrupted_recording(temporary.path(), "merge-owned");
    let names = [
        "manifest.json",
        "segment-000000.webm",
        "segment-000001.webm",
    ];
    let before = names.map(|name| fs::read(session.join(name)).unwrap());
    let registry = Arc::new(RecordingMergeRegistry::default());
    let merge = registry.begin("merge-owned").unwrap();
    let calls = Cell::new(0);
    let result = delete_session_with(&registry, "merge-owned", || {
        calls.set(calls.get() + 1);
        delete_files(temporary.path(), "merge-owned")
    });

    let after = names.map(|name| fs::read(session.join(name)).unwrap_or_default());
    assert_eq!(after, before, "合并所有权必须保留已提交文件的原始字节");
    assert_eq!(result.unwrap_err().code, "recording_library_delete_busy");
    assert_eq!(calls.get(), 0);
    manifest::merge_interrupted_vp9_session(temporary.path(), "merge-owned").unwrap();
    drop(merge);
    delete_session_with(&registry, "merge-owned", || {
        delete_files(temporary.path(), "merge-owned")
    })
    .unwrap();
    assert!(!session.exists());
}

#[test]
fn recording_delete_owner_blocks_same_session_merge_and_duplicate_delete() {
    let registry = Arc::new(RecordingMergeRegistry::default());
    let duplicate_calls = Cell::new(0);
    delete_session_with(&registry, "deleting", || {
        let merge = registry.begin("deleting");
        let merge_blocked = merge.is_err();
        drop(merge);
        let duplicate = delete_session_with(&registry, "deleting", || {
            duplicate_calls.set(duplicate_calls.get() + 1);
            Ok(())
        });
        assert!(merge_blocked, "删除持有期间不能开始同会话合并");
        assert_eq!(duplicate.unwrap_err().code, "recording_library_delete_busy");
        assert_eq!(duplicate_calls.get(), 0);
        Ok(())
    })
    .unwrap();
    assert!(registry.begin("deleting").is_ok());
}

#[test]
fn recording_delete_owner_other_sessions_remain_available_and_merge_stays_single() {
    let registry = Arc::new(RecordingMergeRegistry::default());
    delete_session_with(&registry, "deleting", || {
        let merge = registry.begin("merging-other").unwrap();
        assert!(registry.begin("second-merge").is_err());
        delete_session_with(&registry, "deleting-other", || Ok(())).unwrap();
        drop(merge);
        Ok(())
    })
    .unwrap();
}

#[test]
fn recording_delete_owner_file_error_is_preserved_and_retry_releases_owner() {
    let temporary = tempfile::tempdir().unwrap();
    let session = interrupted_recording(temporary.path(), "retry-delete");
    let before = fs::read(session.join("manifest.json")).unwrap();
    fs::write(session.join("unexpected.txt"), b"keep me").unwrap();
    let registry = Arc::new(RecordingMergeRegistry::default());
    let error = delete_session_with(&registry, "retry-delete", || {
        delete_files(temporary.path(), "retry-delete")
    })
    .unwrap_err();
    assert_eq!(error.code, "recording_library_storage_failed");
    assert_eq!(error.message, "录屏会话包含未知或不安全文件，拒绝删除");
    assert_eq!(fs::read(session.join("manifest.json")).unwrap(), before);
    assert_eq!(
        fs::read(session.join("unexpected.txt")).unwrap(),
        b"keep me"
    );
    drop(registry.begin("retry-delete").unwrap());
    fs::remove_file(session.join("unexpected.txt")).unwrap();
    delete_session_with(&registry, "retry-delete", || {
        delete_files(temporary.path(), "retry-delete")
    })
    .unwrap();
    assert!(!session.exists());
}

#[test]
fn recording_delete_owner_panic_releases_owner() {
    let registry = Arc::new(RecordingMergeRegistry::default());
    let result = std::panic::catch_unwind(|| {
        delete_session_with(&registry, "panic-delete", || {
            panic!("注入删除 worker panic")
        })
    });
    assert!(result.is_err());
    drop(registry.begin("panic-delete").unwrap());
    delete_session_with(&registry, "panic-delete", || Ok(())).unwrap();
}

#[test]
fn recording_delete_owner_real_worker_holds_owner_until_file_work_finishes() {
    let temporary = tempfile::tempdir().unwrap();
    let session = interrupted_recording(temporary.path(), "worker-delete");
    let registry = Arc::new(RecordingMergeRegistry::default());
    let worker_registry = Arc::clone(&registry);
    let worker_root = temporary.path().to_path_buf();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        delete_session_with(&worker_registry, "worker-delete", || {
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            delete_files(&worker_root, "worker-delete")
        })
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let merge = registry.begin("worker-delete");
    let merge_blocked = merge.is_err();
    drop(merge);
    let duplicate_calls = Cell::new(0);
    let duplicate = delete_session_with(&registry, "worker-delete", || {
        duplicate_calls.set(duplicate_calls.get() + 1);
        Ok(())
    });
    // 先释放并 join 真正的 worker，再断言，红基线失败也不能把测试线程留在后台。
    release_tx.send(()).unwrap();
    worker.join().unwrap().unwrap();
    assert!(merge_blocked);
    assert_eq!(duplicate.unwrap_err().code, "recording_library_delete_busy");
    assert_eq!(duplicate_calls.get(), 0);
    assert!(!session.exists());
    assert!(registry.begin("worker-delete").is_ok());
}
