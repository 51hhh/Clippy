//! 需要保存用户剪贴板内容或凭据的本地文件工具。

use std::fs;
use std::io;
use std::path::Path;

#[cfg(target_os = "windows")]
#[path = "private_files/windows.rs"]
mod windows;

/// 将文件权限收紧为仅当前用户可读写。
pub fn restrict_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(target_os = "windows")]
    {
        windows::restrict(path, false)?;
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    let _ = path;
    Ok(())
}

/// 将目录权限收紧为仅当前用户可访问。
pub fn restrict_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(target_os = "windows")]
    {
        windows::restrict(path, true)?;
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    let _ = path;
    Ok(())
}

/// 确保文件存在，并在调用方打开前就具有私有权限。
pub fn ensure_private_file(path: &Path) -> io::Result<()> {
    if path.exists() {
        restrict_file(path)?;
    }
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?;
    restrict_file(path)
}

/// 以私有权限创建或覆盖文件，并在写入后再次校正权限。
pub fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write;

    if path.exists() {
        restrict_file(path)?;
    }
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    restrict_file(path)
}

/// 用已经完整写入的私有临时文件原子替换目标，并再次校正最终路径权限。
///
/// Unix `rename` 可以覆盖目标；Windows 必须显式传 `MOVEFILE_REPLACE_EXISTING`，否则配置和
/// Portal restore token 从第二次保存开始就会失败。
pub fn replace_private_file(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    windows::replace(source, destination)?;
    #[cfg(not(target_os = "windows"))]
    fs::rename(source, destination)?;
    restrict_file(destination)
}

/// 判断文件是否没有向组或其他用户授予权限。
pub fn is_private(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o077 == 0)
            .unwrap_or(false)
    }
    #[cfg(target_os = "windows")]
    {
        windows::is_private(path)
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = path;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_private_creates_a_private_file() {
        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let path = directory.path().join("private");
        write_private(&path, b"secret").expect("写入私有文件失败");
        assert_eq!(fs::read(&path).expect("读取私有文件失败"), b"secret");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        #[cfg(target_os = "windows")]
        assert!(is_private(&path));
    }

    #[test]
    fn private_replace_overwrites_without_losing_permissions() {
        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let source = directory.path().join("source.tmp");
        let destination = directory.path().join("destination");
        write_private(&source, b"new").expect("写入临时文件失败");
        write_private(&destination, b"old").expect("写入旧文件失败");

        replace_private_file(&source, &destination).expect("替换私有文件失败");
        assert_eq!(fs::read(&destination).unwrap(), b"new");
        assert!(!source.exists());
        assert!(is_private(&destination));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_file_acl_is_repaired_and_verified() {
        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let path = directory.path().join("existing");
        fs::write(&path, b"secret").expect("创建文件失败");

        restrict_file(&path).expect("收紧 Windows 文件 ACL 失败");
        assert!(is_private(&path));
        assert_eq!(fs::read(path).unwrap(), b"secret");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_directory_acl_is_repaired_and_verified() {
        let directory = tempfile::tempdir().expect("创建临时目录失败");
        restrict_directory(directory.path()).expect("收紧 Windows 目录 ACL 失败");
        assert!(is_private(directory.path()));
    }

    #[cfg(unix)]
    #[test]
    fn restrict_file_repairs_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let path = directory.path().join("existing");
        fs::write(&path, b"secret").expect("创建文件失败");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        restrict_file(&path).expect("收紧文件权限失败");
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn restrict_directory_repairs_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("创建临时目录失败");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        restrict_directory(directory.path()).expect("收紧目录权限失败");
        assert_eq!(
            fs::metadata(directory.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_private_file_does_not_truncate_existing_content() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let path = directory.path().join("existing");
        fs::write(&path, b"clipboard").expect("创建文件失败");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        ensure_private_file(&path).expect("准备私有文件失败");
        assert_eq!(fs::read(&path).unwrap(), b"clipboard");
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
