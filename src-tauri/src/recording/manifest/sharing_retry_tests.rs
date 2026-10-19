use super::super::windows_sharing::{delete_open_has_sharing_violation, replace, retry};
use super::*;
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::sync::mpsc;
use std::time::Duration;
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

fn files() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let target = directory.path().join("target");
    write_private(&source, b"new").unwrap();
    write_private(&target, b"old").unwrap();
    (directory, source, target)
}

#[test]
fn native_transient_access_denied_is_retried_only_with_delete_sharing_proof() {
    let (_directory, source, target) = files();
    let held = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&target)
        .unwrap();
    assert!(delete_open_has_sharing_violation(&target));
    let (release, signal) = mpsc::channel();
    let locker = std::thread::spawn(move || {
        signal.recv_timeout(Duration::from_secs(5)).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        drop(held);
    });
    let mut first = None;
    let mut release = Some(release);
    let result = retry(
        || {
            let result = replace_private_file(&source, &target);
            if let Err(error) = &result {
                first.get_or_insert(error.raw_os_error());
                if let Some(release) = release.take() {
                    release.send(()).unwrap();
                }
            }
            result
        },
        || source.exists(),
        || delete_open_has_sharing_violation(&source) || delete_open_has_sharing_violation(&target),
    );
    locker.join().unwrap();
    result.unwrap();
    assert!(matches!(first, Some(Some(5 | 32))));
    assert!(!source.exists());
    assert_eq!(fs::read(&target).unwrap(), b"new");
    assert!(crate::private_files::is_private(&target));
    assert!(!delete_open_has_sharing_violation(&target));
}

#[test]
fn native_permanent_share_conflict_preserves_both_private_files_and_original_error() {
    let (_directory, source, target) = files();
    let held = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&target)
        .unwrap();
    let first = replace_private_file(&source, &target)
        .unwrap_err()
        .raw_os_error();
    let mut attempts = 0;
    let error = retry(
        || {
            attempts += 1;
            replace_private_file(&source, &target)
        },
        || source.exists(),
        || delete_open_has_sharing_violation(&target),
    )
    .unwrap_err();
    assert_eq!(error.raw_os_error(), first);
    assert!((2..=21).contains(&attempts));
    assert_eq!(fs::read(&source).unwrap(), b"new");
    assert_eq!(fs::read(&target).unwrap(), b"old");
    assert!(crate::private_files::is_private(&source));
    assert!(crate::private_files::is_private(&target));
    drop(held);
}

#[test]
fn native_readonly_access_denied_is_not_classified_as_sharing() {
    let (_directory, source, target) = files();
    let original = fs::metadata(&target).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&target, readonly).unwrap();
    let proof = delete_open_has_sharing_violation(&target);
    let result = replace(&source, &target);
    fs::set_permissions(&target, original).unwrap();
    assert!(!proof);
    assert_eq!(result.unwrap_err().raw_os_error(), Some(5));
    assert_eq!(fs::read(&source).unwrap(), b"new");
    assert_eq!(fs::read(&target).unwrap(), b"old");
}

#[test]
fn unconfirmed_access_denied_and_unrelated_errors_return_on_first_attempt() {
    for code in [5, 2, 87] {
        let mut calls = 0;
        let error = retry(
            || {
                calls += 1;
                Err(io::Error::from_raw_os_error(code))
            },
            || true,
            || false,
        )
        .unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(error.raw_os_error(), Some(code));
    }
}

#[test]
fn post_promotion_error_is_not_retried_after_source_disappears() {
    let (_directory, source, target) = files();
    let mut calls = 0;
    let error = retry(
        || {
            calls += 1;
            replace_private_file(&source, &target).unwrap();
            Err(io::Error::from_raw_os_error(32))
        },
        || source.exists(),
        || true,
    )
    .unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(error.raw_os_error(), Some(32));
    assert!(!source.exists());
    assert_eq!(fs::read(&target).unwrap(), b"new");
    assert!(crate::private_files::is_private(&target));
}

#[test]
fn direct_sharing_and_lock_violation_use_same_bounded_retry_contract() {
    let mut calls = 0;
    retry(
        || {
            calls += 1;
            match calls {
                1 => Err(io::Error::from_raw_os_error(32)),
                2 => Err(io::Error::from_raw_os_error(33)),
                _ => Ok(()),
            }
        },
        || true,
        || panic!("32/33 不需要把错误5重新分类"),
    )
    .unwrap();
    assert_eq!(calls, 3);
}

#[test]
fn repeated_transient_error_cannot_expand_attempt_budget() {
    let mut calls = 0;
    let error = retry(
        || {
            calls += 1;
            Err(io::Error::from_raw_os_error(32))
        },
        || true,
        || true,
    )
    .unwrap_err();
    assert!((2..=21).contains(&calls));
    assert_eq!(error.raw_os_error(), Some(32));
}
