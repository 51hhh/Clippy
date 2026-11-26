use crate::identity::DeploymentIdentity;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
    pub database_file: PathBuf,
    pub recordings_dir: PathBuf,
    pub screenshot_fallback_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve(identity: &DeploymentIdentity) -> Result<Self, PathError> {
        let base = dirs::data_dir().ok_or(PathError::DataDirectoryUnavailable)?;
        Ok(Self::from_data_root(&base, identity))
    }

    pub fn from_data_root(base: &Path, identity: &DeploymentIdentity) -> Self {
        let data_dir = base.join(identity.current_platform().data_namespace);
        Self {
            config_file: data_dir.join("config.json"),
            database_file: data_dir.join("clips.db"),
            recordings_dir: data_dir.join("recordings"),
            screenshot_fallback_dir: data_dir.join("Screenshots"),
            data_dir,
        }
    }

    pub fn screenshot_default(&self, identity: &DeploymentIdentity) -> PathBuf {
        if identity.channel == crate::identity::DistributionChannel::Qa {
            return self.screenshot_fallback_dir.clone();
        }
        clippy_core::image_output::system_screenshot_dir(
            dirs::picture_dir().as_deref(),
            &self.data_dir,
        )
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn local_file_uri(uri: &str) -> anyhow::Result<PathBuf> {
    let url = url::Url::parse(uri)?;
    anyhow::ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "The file link includes unexpected parameters"
    );
    url.to_file_path()
        .map_err(|()| anyhow::anyhow!("Expected a local file"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    DataDirectoryUnavailable,
}

impl fmt::Display for PathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DataDirectoryUnavailable => formatter.write_str("无法获取当前用户的数据目录"),
        }
    }
}

impl std::error::Error for PathError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{PRODUCTION_IDENTITY, SLINT_QA_IDENTITY};

    #[test]
    fn production_paths_match_the_tauri_data_directory_contract() {
        let base = Path::new("/user-data");
        let paths = AppPaths::from_data_root(base, &PRODUCTION_IDENTITY);
        assert_eq!(
            paths.data_dir,
            base.join(PRODUCTION_IDENTITY.current_platform().data_namespace)
        );
        assert_eq!(paths.config_file, paths.data_dir.join("config.json"));
        assert_eq!(paths.database_file, paths.data_dir.join("clips.db"));
        assert_eq!(paths.recordings_dir, paths.data_dir.join("recordings"));
    }

    #[test]
    fn qa_paths_are_isolated_from_production() {
        let base = Path::new("/user-data");
        let production = AppPaths::from_data_root(base, &PRODUCTION_IDENTITY);
        let qa = AppPaths::from_data_root(base, &SLINT_QA_IDENTITY);
        assert_ne!(qa.data_dir, production.data_dir);
        assert_ne!(qa.database_file, production.database_file);
    }
}
