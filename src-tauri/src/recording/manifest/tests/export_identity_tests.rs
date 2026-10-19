use super::{create_complete_session, export_library_artifact, resolve_library_artifact};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Storage::FileSystem::{
    FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
};

// 直接读取实际文件身份，红基线无需新增生产身份 helper，导出函数正文保持原样。
fn native_identity(path: &Path) -> (u64, [u8; 16]) {
    let file = File::open(path).unwrap();
    let mut info = FILE_ID_INFO::default();
    // SAFETY: file 的句柄在整个调用内有效，info 的实际大小与 FileIdInfo 结构一致。
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            std::ptr::from_mut(&mut info).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    assert_ne!(
        result,
        0,
        "读取真实文件身份失败: {}",
        std::io::Error::last_os_error()
    );
    (info.VolumeSerialNumber, info.FileId.Identifier)
}

fn directory_names(path: &Path) -> BTreeSet<PathBuf> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into())
        .collect()
}

fn reject_alias(make_alias: impl FnOnce(&Path) -> PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let session = create_complete_session(temporary.path(), "identity-alias");
    let artifact = resolve_library_artifact(temporary.path(), "identity-alias", "final").unwrap();
    let alias = make_alias(&artifact.path);
    assert_ne!(alias, artifact.path, "别名必须绕过原路径文本相等检查");
    let before_id = native_identity(&artifact.path);
    assert_eq!(
        native_identity(&alias),
        before_id,
        "夹具必须指向同一实际文件"
    );
    let before_bytes = fs::read(&artifact.path).unwrap();
    let before_names = directory_names(&session);

    let result = export_library_artifact(&artifact, &alias);

    assert_eq!(
        result,
        Err("不能用导出文件覆盖内部恢复产物".to_string()),
        "原导出入口必须拒绝同一文件的路径别名"
    );
    assert_eq!(native_identity(&artifact.path), before_id);
    assert_eq!(native_identity(&alias), before_id);
    assert_eq!(fs::read(&artifact.path).unwrap(), before_bytes);
    assert_eq!(directory_names(&session), before_names);
}

#[test]
fn windows_export_identity_rejects_case_alias() {
    reject_alias(|source| source.with_file_name("RECORDING.WEBM"));
}

#[test]
fn windows_export_identity_rejects_parent_directory_alias() {
    reject_alias(|source| {
        let parent = source.parent().unwrap();
        parent
            .join("..")
            .join(parent.file_name().unwrap())
            .join(source.file_name().unwrap())
    });
}

#[test]
fn windows_export_identity_rejects_extended_path_alias() {
    reject_alias(|source| fs::canonicalize(source).unwrap());
}

#[test]
fn windows_export_identity_rejects_hard_link_alias() {
    reject_alias(|source| {
        let alias = source.with_file_name("recording-linked.webm");
        fs::hard_link(source, &alias).unwrap();
        alias
    });
}

#[test]
fn windows_export_identity_new_destination_preserves_source_identity() {
    let temporary = tempfile::tempdir().unwrap();
    create_complete_session(temporary.path(), "identity-new");
    let artifact = resolve_library_artifact(temporary.path(), "identity-new", "final").unwrap();
    let before_id = native_identity(&artifact.path);
    let destination = temporary.path().join("new-export.webm");
    assert!(!destination.exists());
    export_library_artifact(&artifact, &destination).unwrap();
    assert_eq!(
        fs::read(&destination).unwrap(),
        fs::read(&artifact.path).unwrap()
    );
    assert_eq!(native_identity(&artifact.path), before_id);
    assert_ne!(native_identity(&destination), before_id);
    assert_eq!(directory_names(temporary.path()).len(), 2);
}

#[test]
fn windows_export_identity_distinct_same_content_destination_remains_overwritable() {
    let temporary = tempfile::tempdir().unwrap();
    create_complete_session(temporary.path(), "identity-other");
    let artifact = resolve_library_artifact(temporary.path(), "identity-other", "final").unwrap();
    let destination = temporary.path().join("different-export.webm");
    let expected = fs::read(&artifact.path).unwrap();
    fs::write(&destination, &expected).unwrap();
    let before_id = native_identity(&artifact.path);
    assert_ne!(native_identity(&destination), before_id);
    export_library_artifact(&artifact, &destination).unwrap();
    assert_eq!(fs::read(&destination).unwrap(), expected);
    assert_eq!(native_identity(&artifact.path), before_id);
    assert_ne!(native_identity(&destination), before_id);
    assert_eq!(directory_names(temporary.path()).len(), 2);
}
