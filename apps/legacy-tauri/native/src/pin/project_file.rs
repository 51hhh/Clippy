//! Bounded file-system input for legacy and internally managed Pin revisions.
//!
//! Legacy iTXt containers are validated directly. Flat PNGs regain editing only when their
//! canonical pixels exactly match a revision in the local database.

use std::io::Read;
use std::path::Path;

pub(super) struct PreparedPinImage {
    pub(super) preview_png: Vec<u8>,
    pub(super) project: (Vec<u8>, super::project::RuntimeProject),
}

pub(super) enum PreparedPinCandidate {
    Project(PreparedPinImage),
    Flat {
        preview_png: Vec<u8>,
        rendered_hash: String,
    },
}

pub(super) fn prepare_pin_project_candidate(
    path: &Path,
) -> Result<Option<PreparedPinCandidate>, String> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
    {
        return Ok(None);
    }
    let (container, extracted) = read_png_file_with_project(path)?;
    if let Some(project) = extracted {
        return prepare_opened_png(container, project)
            .map(PreparedPinCandidate::Project)
            .map(Some);
    }
    let preview_png = super::project::flatten_container(&container)?;
    let rendered_hash = crate::clipboard_watcher::content::compute_hash(&preview_png);
    Ok(Some(PreparedPinCandidate::Flat {
        preview_png,
        rendered_hash,
    }))
}

#[cfg(test)]
pub(super) fn prepare_pin_project_file(path: &Path) -> Result<Option<PreparedPinImage>, String> {
    match prepare_pin_project_candidate(path)? {
        Some(PreparedPinCandidate::Project(prepared)) => Ok(Some(prepared)),
        Some(PreparedPinCandidate::Flat { .. }) | None => Ok(None),
    }
}

/// 新版内部修订的磁盘 PNG 只含合成像素。它必须以规范 RGBA 哈希精确命中本机数据库
/// 才恢复编辑；普通图片或被第三方改过像素的文件继续返回 None。
#[cfg(test)]
pub(super) fn prepare_managed_pin_project_file(
    path: &Path,
    storage: &crate::storage::StorageEngine,
) -> Result<Option<PreparedPinImage>, String> {
    let Some(candidate) = prepare_pin_project_candidate(path)? else {
        return Ok(None);
    };
    match candidate {
        PreparedPinCandidate::Project(prepared) => Ok(Some(prepared)),
        PreparedPinCandidate::Flat {
            preview_png,
            rendered_hash,
        } => {
            let revision = storage
                .get_image_revision_by_rendered_hash(&rendered_hash)
                .map_err(|error| error.to_string())?;
            revision
                .map(|revision| prepare_managed_pin_image(preview_png, revision))
                .transpose()
        }
    }
}

pub(super) fn prepare_managed_pin_image(
    preview_png: Vec<u8>,
    revision: crate::storage::StoredImageRevision,
) -> Result<PreparedPinImage, String> {
    let project = super::project::RuntimeProject::from_managed(
        &revision.source_png,
        &preview_png,
        revision.renderer_version,
        revision.source_width,
        revision.source_height,
        revision.annotations,
        revision.adjustments,
    )?;
    Ok(PreparedPinImage {
        preview_png,
        project: (revision.source_png, project),
    })
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
