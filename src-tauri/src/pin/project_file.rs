//! Bounded file-system input for editable Pin PNG projects.
//!
//! This module only reads and validates a candidate file. Window creation stays
//! in the lifecycle layer, after all fallible file parsing has completed.

use std::io::Read;
use std::path::Path;

pub(super) struct PreparedPinImage {
    pub(super) preview_png: Vec<u8>,
    pub(super) project: (Vec<u8>, super::project::RuntimeProject),
}

pub(super) fn prepare_pin_project_file(path: &Path) -> Result<Option<PreparedPinImage>, String> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
    {
        return Ok(None);
    }
    let (container, extracted) = read_png_file_with_project(path)?;
    let Some(project) = extracted else {
        return Ok(None);
    };
    prepare_opened_png(container, project).map(Some)
}

fn prepare_opened_png(
    container: Vec<u8>,
    extracted: super::project::PinProject,
) -> Result<PreparedPinImage, String> {
    // `extract` 已做完整验证；进入运行时立即丢弃 base64 副本，只保留唯一原图字节。
    let project = extracted.into_runtime()?;
    let preview_png = super::project::flatten_container(&container)?;
    Ok(PreparedPinImage {
        preview_png,
        project,
    })
}

#[cfg(test)]
pub(super) fn read_png_file(path: &Path) -> Result<Vec<u8>, String> {
    read_png_file_with_project(path).map(|(bytes, _)| bytes)
}

fn read_png_file_with_project(
    path: &Path,
) -> Result<(Vec<u8>, Option<super::project::PinProject>), String> {
    let file = std::fs::File::open(path).map_err(|error| format!("打开文件失败: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("读取文件信息失败: {error}"))?;
    if !metadata.is_file() {
        return Err("所选路径不是普通文件".to_string());
    }
    if metadata.len() > super::project::MAX_CONTAINER_BYTES as u64 {
        return Err("PNG 文件超过 160 MiB 上限".to_string());
    }
    // 文件可能在 metadata 后被别的进程替换/增长；take(+1) 保证竞争下也不会无界读取。
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(super::project::MAX_CONTAINER_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取文件失败: {error}"))?;
    if bytes.len() > super::project::MAX_CONTAINER_BYTES {
        return Err("PNG 文件超过 160 MiB 上限".to_string());
    }
    // `extract` 同时验证整张 PNG；损坏工程只返回 None，损坏 IDAT 会报错。
    let project = super::project::extract(&bytes)?;
    Ok((bytes, project))
}
