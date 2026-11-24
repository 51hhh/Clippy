use anyhow::{Context, Result};
use clippy_platform::{paths::AppPaths, private_files::write_private};
use std::fs;

pub(super) fn backup_existing(paths: &AppPaths) -> Result<()> {
    let destination = paths.data_dir.join("before-slint-v0.2");
    if destination.is_dir() {
        return Ok(());
    }
    let stage = tempfile::Builder::new()
        .prefix(".upgrade-")
        .tempdir_in(&paths.data_dir)?;
    if paths.database_file.try_exists()? {
        clippy_storage::backup_database(&paths.database_file, &stage.path().join("clips.db"))
            .context("Could not create a consistent backup of the existing clipboard database")?;
    }
    if paths.config_file.try_exists()? {
        let config = fs::read(&paths.config_file)?;
        write_private(&stage.path().join("config.json"), &config)
            .context("Could not back up the existing settings")?;
    }
    fs::rename(stage.path(), destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clippy_core::models::ContentType;
    use clippy_platform::identity::PRODUCTION_IDENTITY;
    use clippy_storage::StorageEngine;

    #[test]
    fn first_native_launch_keeps_settings_and_a_consistent_database_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_data_root(root.path(), &PRODUCTION_IDENTITY);
        fs::create_dir(&paths.data_dir).unwrap();
        let live = StorageEngine::new(&paths.database_file).unwrap();
        let clip = live
            .insert_clip(
                &ContentType::Text,
                Some("legacy fixture"),
                None,
                None,
                "legacy-fixture",
                14,
                false,
            )
            .unwrap();
        fs::write(&paths.config_file, b"old settings bytes").unwrap();
        backup_existing(&paths).unwrap();
        live.delete_clip(clip.id).unwrap();
        fs::write(&paths.config_file, b"new settings bytes").unwrap();
        backup_existing(&paths).unwrap();
        let backup = paths.data_dir.join("before-slint-v0.2");
        assert_eq!(
            fs::read(backup.join("config.json")).unwrap(),
            b"old settings bytes"
        );
        let saved = StorageEngine::new(&backup.join("clips.db")).unwrap();
        assert_eq!(
            saved
                .get_clip_by_id(clip.id)
                .unwrap()
                .text_content
                .as_deref(),
            Some("legacy fixture")
        );
    }
}
