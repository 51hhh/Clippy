use super::*;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};

#[test]
fn recording_artifacts_success_removes_owned_nested_directory() {
    let enclosing = ::tempfile::tempdir().unwrap();
    let directory = TestDirectory::from(::tempfile::tempdir_in(enclosing.path()).unwrap());
    let path = directory.path().to_path_buf();
    fs::create_dir(path.join("nested")).unwrap();
    fs::write(path.join("nested/manifest.json"), b"synthetic").unwrap();
    drop(directory);
    assert!(!path.exists());
    assert!(enclosing.path().is_dir());
}

#[test]
fn recording_artifacts_unwind_preserves_exact_nested_bytes() {
    let enclosing = ::tempfile::tempdir().unwrap();
    let directory = TestDirectory::from(::tempfile::tempdir_in(enclosing.path()).unwrap());
    let path = directory.path().to_path_buf();
    fs::create_dir(path.join("recordings")).unwrap();
    fs::write(path.join("recordings/manifest.json"), b"original manifest").unwrap();
    fs::write(path.join("recordings/.segment.partial"), [0, 255, 17]).unwrap();
    let result = catch_unwind(move || {
        let _owned = directory;
        panic!("controlled synthetic fixture unwind");
    });
    assert!(result.is_err());
    assert_eq!(
        fs::read(path.join("recordings/manifest.json")).unwrap(),
        b"original manifest"
    );
    assert_eq!(
        fs::read(path.join("recordings/.segment.partial")).unwrap(),
        [0, 255, 17]
    );
}

#[test]
fn recording_artifacts_json_path_handles_unicode_and_spaces() {
    let enclosing = ::tempfile::tempdir().unwrap();
    let mut directory = ::tempfile::Builder::new()
        .prefix("诊断 space-")
        .tempdir_in(enclosing.path())
        .unwrap();
    let path = directory.path().to_path_buf();
    let mut output = Vec::new();
    retain(&mut directory, &mut output);
    drop(directory);
    let line = String::from_utf8(output).unwrap();
    let reported: std::path::PathBuf = serde_json::from_str(
        line.strip_prefix("CLIPPY_RECORDING_TEST_ARTIFACTS=")
            .unwrap()
            .trim(),
    )
    .unwrap();
    assert_eq!(reported, path);
    assert!(reported.is_dir());
}

struct BrokenOutput;

#[test]
fn recording_artifacts_one_unwind_does_not_keep_or_remove_other_directory() {
    let enclosing = ::tempfile::tempdir().unwrap();
    let first = TestDirectory::from(::tempfile::tempdir_in(enclosing.path()).unwrap());
    let second = TestDirectory::from(::tempfile::tempdir_in(enclosing.path()).unwrap());
    let retained = first.path().to_path_buf();
    let cleaned = second.path().to_path_buf();
    fs::write(retained.join("marker"), b"first").unwrap();
    fs::write(cleaned.join("marker"), b"second").unwrap();
    assert!(catch_unwind(move || {
        let _owned = first;
        panic!("controlled independent fixture unwind");
    })
    .is_err());
    assert_eq!(fs::read(cleaned.join("marker")).unwrap(), b"second");
    drop(second);
    assert!(!cleaned.exists());
    assert_eq!(fs::read(retained.join("marker")).unwrap(), b"first");
}

impl Write for BrokenOutput {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "controlled diagnostic output failure",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn recording_artifacts_output_error_still_keeps_directory_without_panic() {
    let enclosing = ::tempfile::tempdir().unwrap();
    let mut directory = ::tempfile::tempdir_in(enclosing.path()).unwrap();
    let path = directory.path().to_path_buf();
    fs::write(path.join("segment.avi"), b"original segment").unwrap();
    assert!(catch_unwind(AssertUnwindSafe(|| retain(
        &mut directory,
        &mut BrokenOutput
    )))
    .is_ok());
    drop(directory);
    assert_eq!(
        fs::read(path.join("segment.avi")).unwrap(),
        b"original segment"
    );
}
