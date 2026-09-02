use super::error::PasteError;
use std::path::Path;

fn io(error: impl std::fmt::Display) -> PasteError {
    PasteError::TokenIo(error.to_string())
}

pub(super) fn read_restore_token(path: &Path) -> Option<String> {
    if !crate::private_files::is_private(path) {
        log::warn!("拒绝读取权限过宽的 Portal restore token");
        return None;
    }
    let token = std::fs::read_to_string(path).ok()?;
    let token = token.trim();
    if token.is_empty() || token.len() > 4096 {
        return None;
    }
    Some(token.to_string())
}

pub(super) fn write_restore_token(path: &Path, token: &str) -> Result<(), PasteError> {
    if token.is_empty() || token.len() > 4096 {
        return Err(PasteError::TokenInvalidLength);
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or(PasteError::TokenPathMissingParent)?;
    std::fs::create_dir_all(parent).map_err(io)?;
    crate::private_files::restrict_directory(parent).map_err(io)?;
    let temp = path.with_extension("tmp");
    crate::private_files::write_private(&temp, token.as_bytes()).map_err(io)?;
    crate::private_files::replace_private_file(&temp, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_uses_separate_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portal-token");
        write_restore_token(&path, "token-value").unwrap();
        assert_eq!(read_restore_token(&path).as_deref(), Some("token-value"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn rolling_token_update_replaces_the_existing_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portal-token");
        write_restore_token(&path, "first-token").unwrap();
        write_restore_token(&path, "second-token").unwrap();

        assert_eq!(read_restore_token(&path).as_deref(), Some("second-token"));
        assert!(crate::private_files::is_private(&path));
    }

    #[test]
    fn rejects_empty_and_oversized_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portal-token");
        assert!(write_restore_token(&path, "").is_err());
        assert!(write_restore_token(&path, &"x".repeat(4097)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_group_or_world_readable_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portal-token");
        std::fs::write(&path, "token-value").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(read_restore_token(&path), None);
    }
}
