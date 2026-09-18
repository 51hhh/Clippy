//! `.clippy.zip` 本地交换格式。
//!
//! ZIP 只是一个具名容器：所有条目都必须是 Stored，真正的完整性由 manifest 中的
//! SHA-256 与关系校验提供。导入在任何 SQLite 写入之前读完并验证整个归档。

use crate::storage::{
    ArchiveClipSnapshot, ArchiveGroupSnapshot, ArchiveRevisionSnapshot, ArchiveScope,
    ArchiveSnapshot, ArchiveWorkspaceContentSnapshot, ArchiveWorkspaceSnapshot, ImportSummary,
    StoredPinPlacement,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use tauri::{Emitter, Manager};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const FORMAT: &str = "clippy-archive";
const VERSION: u32 = 1;
const MANIFEST_NAME: &str = "manifest.json";
const MAX_ENTRIES: usize = 50_000;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_BLOB_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ARCHIVE_FILE_BYTES: u64 = MAX_TOTAL_BYTES + 8 * 1024 * 1024;
const MAX_ITEMS: usize = 10_000;
const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;
const MAX_REVISION_DOCUMENT_BYTES: usize = 96 * 1024 * 1024;
const MAX_WORKSPACE_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct BlobRef {
    sha256: String,
    byte_length: u64,
    media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestRevision {
    source: BlobRef,
    source_width: u32,
    source_height: u32,
    renderer_version: u32,
    annotations: BlobRef,
    adjustments: BlobRef,
    document_hash: String,
    rendered_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestClip {
    content_type: crate::models::ContentType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<BlobRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    html: Option<BlobRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image: Option<BlobRef>,
    content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ocr_text: Option<BlobRef>,
    is_favorite: bool,
    is_sensitive: bool,
    created_at: i64,
    byte_size: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision: Option<ManifestRevision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestGroup {
    source_id: i64,
    name: String,
    sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ManifestWorkspaceContent {
    Image {
        revision: ManifestRevision,
    },
    Snapshot {
        content_type: crate::models::ContentType,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<BlobRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        html: Option<BlobRef>,
        content_hash: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestPlacement {
    x: f64,
    y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    display_x: f64,
    display_y: f64,
    display_width: f64,
    display_height: f64,
    display_scale: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestWorkspace {
    source_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group_source_id: Option<i64>,
    content: ManifestWorkspaceContent,
    content_width: f64,
    content_height: f64,
    scale: f64,
    opacity: f64,
    locked: bool,
    above: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    placement: Option<ManifestPlacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: String,
    version: u32,
    archive_id: String,
    created_at: i64,
    scope: ArchiveScope,
    clips: Vec<ManifestClip>,
    groups: Vec<ManifestGroup>,
    workspaces: Vec<ManifestWorkspace>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveExportResult {
    pub path: String,
    pub clips: usize,
    pub groups: usize,
    pub workspaces: usize,
}

#[derive(Default)]
struct BlobStore(BTreeMap<String, Vec<u8>>);

impl BlobStore {
    fn insert(&mut self, bytes: Vec<u8>, media_type: &str) -> Result<BlobRef, String> {
        let byte_length = u64::try_from(bytes.len()).map_err(|error| error.to_string())?;
        if byte_length > MAX_BLOB_BYTES {
            return Err("归档单个内容超过 128 MiB 上限".to_string());
        }
        let sha256 = hash(&bytes);
        if let Some(existing) = self.0.get(&sha256) {
            if existing != &bytes {
                return Err("SHA-256 相同的归档内容字节不一致".to_string());
            }
        } else {
            self.0.insert(sha256.clone(), bytes);
        }
        Ok(BlobRef {
            sha256,
            byte_length,
            media_type: media_type.to_string(),
        })
    }
}

pub fn write_archive(
    path: &Path,
    snapshot: &ArchiveSnapshot,
) -> Result<ArchiveExportResult, String> {
    let (mut manifest, blobs) = manifest_from_snapshot(snapshot)?;
    if blobs.0.len().saturating_add(1) > MAX_ENTRIES {
        return Err("归档文件数量超过 50,000 项上限".to_string());
    }
    manifest.archive_id.clear();
    manifest.archive_id = hash(
        &serde_json::to_vec(&manifest).map_err(|error| format!("序列化归档清单失败: {error}"))?,
    );
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("序列化归档清单失败: {error}"))?;
    if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("归档清单超过 16 MiB 上限".to_string());
    }
    let total = blobs
        .0
        .values()
        .try_fold(manifest_bytes.len() as u64, |sum, bytes| {
            sum.checked_add(bytes.len() as u64)
                .ok_or_else(|| "归档大小计算溢出".to_string())
        })?;
    if total > MAX_TOTAL_BYTES {
        return Err("归档内容超过 512 MiB 上限".to_string());
    }

    let parent = path
        .parent()
        .ok_or_else(|| "归档目标缺少父目录".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| format!("创建归档目录失败: {error}"))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "归档文件名无效".to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = parent.join(format!(".{file_name}.{}-{nonce}.tmp", std::process::id()));
    let result = (|| -> Result<(), String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&temp)
            .map_err(|error| format!("创建归档临时文件失败: {error}"))?;
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .unix_permissions(0o600);
        writer
            .start_file(MANIFEST_NAME, options)
            .map_err(|error| format!("写入归档清单失败: {error}"))?;
        writer
            .write_all(&manifest_bytes)
            .map_err(|error| format!("写入归档清单失败: {error}"))?;
        for (sha256, bytes) in &blobs.0 {
            writer
                .start_file(format!("blobs/{sha256}"), options)
                .map_err(|error| format!("写入归档内容失败: {error}"))?;
            writer
                .write_all(bytes)
                .map_err(|error| format!("写入归档内容失败: {error}"))?;
        }
        let file = writer
            .finish()
            .map_err(|error| format!("结束归档写入失败: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("同步归档失败: {error}"))?;
        crate::private_files::replace_private_file(&temp, path)
            .map_err(|error| format!("落地归档失败: {error}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result?;
    Ok(ArchiveExportResult {
        path: path.to_string_lossy().into_owned(),
        clips: manifest.clips.len(),
        groups: manifest.groups.len(),
        workspaces: manifest.workspaces.len(),
    })
}

pub fn read_archive(path: &Path) -> Result<ArchiveSnapshot, String> {
    let file = File::open(path).map_err(|error| format!("打开归档失败: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("读取归档信息失败: {error}"))?;
    if metadata.len() > MAX_ARCHIVE_FILE_BYTES {
        return Err("归档文件超过安全上限".to_string());
    }
    let mut archive = ZipArchive::new(file).map_err(|error| format!("读取 ZIP 失败: {error}"))?;
    if archive.is_empty() || archive.len() > MAX_ENTRIES {
        return Err("归档文件数量无效".to_string());
    }
    let mut names = HashSet::new();
    let mut blobs = HashMap::new();
    let mut manifest_bytes = None;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("读取 ZIP 条目失败: {error}"))?;
        let name = entry.name().to_string();
        if !names.insert(name.clone()) {
            return Err(format!("归档包含重复路径: {name}"));
        }
        if entry.encrypted() || entry.compression() != CompressionMethod::Stored || entry.is_dir() {
            return Err(format!("归档条目必须是未加密 Stored 文件: {name}"));
        }
        let limit = if name == MANIFEST_NAME {
            MAX_MANIFEST_BYTES
        } else if valid_blob_name(&name) {
            MAX_BLOB_BYTES
        } else {
            return Err(format!("归档包含不允许的路径: {name}"));
        };
        if entry.size() > limit {
            return Err(format!("归档条目超过大小上限: {name}"));
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| "归档展开大小溢出".to_string())?;
        if total > MAX_TOTAL_BYTES {
            return Err("归档展开内容超过 512 MiB 上限".to_string());
        }
        let entry_size = entry.size();
        let capacity = usize::try_from(entry_size).map_err(|error| error.to_string())?;
        let mut bytes = Vec::with_capacity(capacity);
        let mut limited = entry.take(limit + 1);
        limited
            .read_to_end(&mut bytes)
            .map_err(|error| format!("读取归档条目失败: {error}"))?;
        if bytes.len() as u64 != entry_size {
            return Err(format!("归档条目长度不一致: {name}"));
        }
        if name == MANIFEST_NAME {
            manifest_bytes = Some(bytes);
        } else {
            let expected = name.trim_start_matches("blobs/");
            if hash(&bytes) != expected {
                return Err(format!("归档 blob 哈希错误: {name}"));
            }
            blobs.insert(expected.to_string(), bytes);
        }
    }
    let manifest_bytes = manifest_bytes.ok_or_else(|| "归档缺少 manifest.json".to_string())?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("归档清单 JSON 无效: {error}"))?;
    validate_manifest_identity(&manifest)?;
    snapshot_from_manifest(manifest, &blobs)
}

fn manifest_from_snapshot(snapshot: &ArchiveSnapshot) -> Result<(Manifest, BlobStore), String> {
    if snapshot
        .clips
        .len()
        .saturating_add(snapshot.workspaces.len())
        > MAX_ITEMS
    {
        return Err("归档项目超过 10,000 项上限".to_string());
    }
    let mut blobs = BlobStore::default();
    let clips = snapshot
        .clips
        .iter()
        .map(|clip| {
            Ok(ManifestClip {
                content_type: clip.content_type.clone(),
                text: clip
                    .text_content
                    .as_ref()
                    .map(|value| {
                        blobs.insert(value.as_bytes().to_vec(), "text/plain; charset=utf-8")
                    })
                    .transpose()?,
                html: clip
                    .html_content
                    .as_ref()
                    .map(|value| {
                        blobs.insert(value.as_bytes().to_vec(), "text/html; charset=utf-8")
                    })
                    .transpose()?,
                image: clip
                    .image_png
                    .as_ref()
                    .map(|value| blobs.insert(value.clone(), "image/png"))
                    .transpose()?,
                content_hash: clip.content_hash.clone(),
                ocr_text: clip
                    .ocr_text
                    .as_ref()
                    .map(|value| {
                        blobs.insert(value.as_bytes().to_vec(), "text/plain; charset=utf-8")
                    })
                    .transpose()?,
                is_favorite: clip.is_favorite,
                is_sensitive: clip.is_sensitive,
                created_at: clip.created_at,
                byte_size: clip.byte_size,
                revision: clip
                    .revision
                    .as_ref()
                    .map(|value| manifest_revision(value, &mut blobs))
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let groups = snapshot
        .groups
        .iter()
        .map(|group| ManifestGroup {
            source_id: group.source_id,
            name: group.name.clone(),
            sort_order: group.sort_order,
        })
        .collect();
    let workspaces = snapshot
        .workspaces
        .iter()
        .map(|workspace| {
            let content = match &workspace.content {
                ArchiveWorkspaceContentSnapshot::Image { revision } => {
                    ManifestWorkspaceContent::Image {
                        revision: manifest_revision(revision, &mut blobs)?,
                    }
                }
                ArchiveWorkspaceContentSnapshot::Snapshot {
                    content_type,
                    text_content,
                    html_content,
                    content_hash,
                } => ManifestWorkspaceContent::Snapshot {
                    content_type: content_type.clone(),
                    text: text_content
                        .as_ref()
                        .map(|value| {
                            blobs.insert(value.as_bytes().to_vec(), "text/plain; charset=utf-8")
                        })
                        .transpose()?,
                    html: html_content
                        .as_ref()
                        .map(|value| {
                            blobs.insert(value.as_bytes().to_vec(), "text/html; charset=utf-8")
                        })
                        .transpose()?,
                    content_hash: content_hash.clone(),
                },
            };
            Ok(ManifestWorkspace {
                source_id: workspace.source_id,
                group_source_id: workspace.group_source_id,
                content,
                content_width: workspace.content_width,
                content_height: workspace.content_height,
                scale: workspace.scale,
                opacity: workspace.opacity,
                locked: workspace.locked,
                above: workspace.above,
                placement: workspace.placement.as_ref().map(manifest_placement),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((
        Manifest {
            format: FORMAT.to_string(),
            version: VERSION,
            archive_id: String::new(),
            created_at: chrono::Utc::now().timestamp(),
            scope: snapshot.scope,
            clips,
            groups,
            workspaces,
        },
        blobs,
    ))
}

fn manifest_revision(
    revision: &ArchiveRevisionSnapshot,
    blobs: &mut BlobStore,
) -> Result<ManifestRevision, String> {
    Ok(ManifestRevision {
        source: blobs.insert(revision.source_png.clone(), "image/png")?,
        source_width: revision.source_width,
        source_height: revision.source_height,
        renderer_version: revision.renderer_version,
        annotations: blobs.insert(
            revision.annotations_json.as_bytes().to_vec(),
            "application/json",
        )?,
        adjustments: blobs.insert(
            revision.adjustments_json.as_bytes().to_vec(),
            "application/json",
        )?,
        document_hash: revision.document_hash.clone(),
        rendered_hash: revision.rendered_hash.clone(),
    })
}

fn manifest_placement(value: &StoredPinPlacement) -> ManifestPlacement {
    ManifestPlacement {
        x: value.x,
        y: value.y,
        display_name: value.display_name.clone(),
        display_x: value.display_x,
        display_y: value.display_y,
        display_width: value.display_width,
        display_height: value.display_height,
        display_scale: value.display_scale,
    }
}

fn snapshot_from_manifest(
    manifest: Manifest,
    blobs: &HashMap<String, Vec<u8>>,
) -> Result<ArchiveSnapshot, String> {
    if manifest
        .clips
        .len()
        .saturating_add(manifest.workspaces.len())
        > MAX_ITEMS
    {
        return Err("归档项目超过 10,000 项上限".to_string());
    }
    let group_ids: HashSet<i64> = manifest
        .groups
        .iter()
        .map(|group| group.source_id)
        .collect();
    if group_ids.len() != manifest.groups.len()
        || manifest
            .groups
            .iter()
            .any(|group| !valid_group_name(&group.name))
    {
        return Err("归档 Pin 分组无效或 ID 重复".to_string());
    }
    let mut used = HashSet::new();
    let clips = manifest
        .clips
        .into_iter()
        .map(|clip| clip_snapshot(clip, blobs, &mut used))
        .collect::<Result<Vec<_>, _>>()?;
    let groups = manifest
        .groups
        .into_iter()
        .map(|group| ArchiveGroupSnapshot {
            source_id: group.source_id,
            name: group.name,
            sort_order: group.sort_order,
        })
        .collect();
    let mut workspace_ids = HashSet::new();
    let workspaces = manifest
        .workspaces
        .into_iter()
        .map(|workspace| {
            if !workspace_ids.insert(workspace.source_id) {
                return Err("归档工作区 ID 重复".to_string());
            }
            if let Some(group_id) = workspace.group_source_id {
                if !group_ids.contains(&group_id) {
                    return Err("归档工作区引用了不存在的分组".to_string());
                }
            }
            let content = match workspace.content {
                ManifestWorkspaceContent::Image { revision } => {
                    ArchiveWorkspaceContentSnapshot::Image {
                        revision: revision_snapshot(revision, blobs, &mut used)?,
                    }
                }
                ManifestWorkspaceContent::Snapshot {
                    content_type,
                    text,
                    html,
                    content_hash,
                } => {
                    let text_content =
                        optional_utf8(text, blobs, &mut used, "text/plain; charset=utf-8")?;
                    let html_content =
                        optional_utf8(html, blobs, &mut used, "text/html; charset=utf-8")?;
                    let snapshot_bytes = text_content
                        .as_deref()
                        .map(str::len)
                        .unwrap_or_default()
                        .saturating_add(html_content.as_deref().map(str::len).unwrap_or_default());
                    if snapshot_bytes > MAX_WORKSPACE_SNAPSHOT_BYTES {
                        return Err("归档 Pin 文本快照超过 16 MiB 上限".to_string());
                    }
                    validate_snapshot_shape(
                        &content_type,
                        text_content.as_deref(),
                        html_content.as_deref(),
                    )?;
                    validate_primary_hash(
                        &content_type,
                        text_content.as_deref(),
                        html_content.as_deref(),
                        None,
                        &content_hash,
                    )?;
                    ArchiveWorkspaceContentSnapshot::Snapshot {
                        content_type,
                        text_content,
                        html_content,
                        content_hash,
                    }
                }
            };
            let placement = workspace.placement.map(|value| StoredPinPlacement {
                x: value.x,
                y: value.y,
                display_name: value.display_name,
                display_x: value.display_x,
                display_y: value.display_y,
                display_width: value.display_width,
                display_height: value.display_height,
                display_scale: value.display_scale,
            });
            validate_presentation(
                workspace.content_width,
                workspace.content_height,
                workspace.scale,
                workspace.opacity,
                placement.as_ref(),
            )?;
            Ok(ArchiveWorkspaceSnapshot {
                source_id: workspace.source_id,
                group_source_id: workspace.group_source_id,
                content,
                content_width: workspace.content_width,
                content_height: workspace.content_height,
                scale: workspace.scale,
                opacity: workspace.opacity,
                locked: workspace.locked,
                above: workspace.above,
                placement,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if used.len() != blobs.len() {
        return Err("归档包含未被 manifest 引用的 blob".to_string());
    }
    Ok(ArchiveSnapshot {
        scope: manifest.scope,
        clips,
        groups,
        workspaces,
    })
}

fn clip_snapshot(
    clip: ManifestClip,
    blobs: &HashMap<String, Vec<u8>>,
    used: &mut HashSet<String>,
) -> Result<ArchiveClipSnapshot, String> {
    let text_content = optional_utf8(clip.text, blobs, used, "text/plain; charset=utf-8")?;
    let html_content = optional_utf8(clip.html, blobs, used, "text/html; charset=utf-8")?;
    let image_png = optional_blob(clip.image, blobs, used, "image/png")?;
    let ocr_text = optional_utf8(clip.ocr_text, blobs, used, "text/plain; charset=utf-8")?;
    validate_clip_shape(
        &clip.content_type,
        text_content.as_deref(),
        html_content.as_deref(),
        image_png.as_deref(),
    )?;
    validate_primary_hash(
        &clip.content_type,
        text_content.as_deref(),
        html_content.as_deref(),
        image_png.as_deref(),
        &clip.content_hash,
    )?;
    let primary_len = match &clip.content_type {
        crate::models::ContentType::Text => text_content.as_deref().unwrap_or_default().len(),
        crate::models::ContentType::Html => html_content.as_deref().unwrap_or_default().len(),
        crate::models::ContentType::Image => image_png.as_deref().unwrap_or_default().len(),
    };
    if clip.byte_size < 0
        || clip.byte_size as u64 > MAX_BLOB_BYTES
        || usize::try_from(clip.byte_size).ok() != Some(primary_len)
        || clip.created_at < 0
    {
        return Err("归档历史条目的 byteSize 无效".to_string());
    }
    let revision = clip
        .revision
        .map(|value| revision_snapshot(value, blobs, used))
        .transpose()?;
    if let Some(revision) = &revision {
        let image = image_png
            .as_deref()
            .ok_or_else(|| "图片修订缺少当前渲染 PNG".to_string())?;
        if hash(image) != revision.rendered_hash {
            return Err("图片修订的当前 PNG 与 renderedHash 不一致".to_string());
        }
    }
    Ok(ArchiveClipSnapshot {
        content_type: clip.content_type,
        text_content,
        html_content,
        image_png,
        content_hash: clip.content_hash,
        ocr_text,
        is_favorite: clip.is_favorite,
        is_sensitive: clip.is_sensitive,
        created_at: clip.created_at,
        byte_size: clip.byte_size,
        revision,
    })
}

fn revision_snapshot(
    revision: ManifestRevision,
    blobs: &HashMap<String, Vec<u8>>,
    used: &mut HashSet<String>,
) -> Result<ArchiveRevisionSnapshot, String> {
    let source_blob_hash = revision.source.sha256.clone();
    let source_png = required_blob(revision.source, blobs, used, "image/png")?;
    validate_png_dimensions(&source_png, revision.source_width, revision.source_height)?;
    let annotations_json = required_utf8(revision.annotations, blobs, used, "application/json")?;
    let adjustments_json = required_utf8(revision.adjustments, blobs, used, "application/json")?;
    if annotations_json
        .len()
        .saturating_add(adjustments_json.len())
        > MAX_REVISION_DOCUMENT_BYTES
    {
        return Err("归档图片修订文档超过 96 MiB 上限".to_string());
    }
    let annotations: serde_json::Value = serde_json::from_str(&annotations_json)
        .map_err(|error| format!("归档标注 JSON 无效: {error}"))?;
    let adjustments: serde_json::Value = serde_json::from_str(&adjustments_json)
        .map_err(|error| format!("归档调整 JSON 无效: {error}"))?;
    let document_hash = project_hash(
        revision.renderer_version,
        revision.source_width,
        revision.source_height,
        &annotations_json,
        &adjustments_json,
    );
    if document_hash != revision.document_hash || !valid_hash(&revision.rendered_hash) {
        return Err("归档图片修订文档哈希无效".to_string());
    }
    let project = crate::pin::output::PinCanvasProject {
        renderer_version: revision.renderer_version,
        source_width: revision.source_width,
        source_height: revision.source_height,
        annotations,
        adjustments,
    };
    let rendered = crate::pin::output::render_document(&source_png, Some(&project))?;
    if hash(&rendered) != revision.rendered_hash {
        return Err("归档图片修订无法重放到 declared renderedHash".to_string());
    }
    Ok(ArchiveRevisionSnapshot {
        source_hash: source_blob_hash,
        source_png,
        source_width: revision.source_width,
        source_height: revision.source_height,
        renderer_version: revision.renderer_version,
        annotations_json,
        adjustments_json,
        document_hash: revision.document_hash,
        rendered_hash: revision.rendered_hash,
    })
}

fn validate_manifest_identity(manifest: &Manifest) -> Result<(), String> {
    if manifest.format != FORMAT {
        return Err("不是 Clippy 归档".to_string());
    }
    if manifest.version != VERSION {
        return Err(format!("不支持的 Clippy 归档版本: {}", manifest.version));
    }
    if !valid_hash(&manifest.archive_id) {
        return Err("归档 ID 无效".to_string());
    }
    let mut identity = manifest.clone();
    let declared = std::mem::take(&mut identity.archive_id);
    let actual = hash(
        &serde_json::to_vec(&identity).map_err(|error| format!("序列化归档身份失败: {error}"))?,
    );
    if declared != actual {
        return Err("归档 manifest 身份校验失败".to_string());
    }
    Ok(())
}

fn required_blob(
    reference: BlobRef,
    blobs: &HashMap<String, Vec<u8>>,
    used: &mut HashSet<String>,
    media_type: &str,
) -> Result<Vec<u8>, String> {
    if !valid_hash(&reference.sha256) || reference.media_type != media_type {
        return Err("归档 blob 引用或媒体类型无效".to_string());
    }
    let bytes = blobs
        .get(&reference.sha256)
        .ok_or_else(|| format!("归档缺少 blob {}", reference.sha256))?;
    if bytes.len() as u64 != reference.byte_length || hash(bytes) != reference.sha256 {
        return Err(format!("归档 blob 长度或哈希无效: {}", reference.sha256));
    }
    used.insert(reference.sha256);
    Ok(bytes.clone())
}

fn optional_blob(
    reference: Option<BlobRef>,
    blobs: &HashMap<String, Vec<u8>>,
    used: &mut HashSet<String>,
    media_type: &str,
) -> Result<Option<Vec<u8>>, String> {
    reference
        .map(|value| required_blob(value, blobs, used, media_type))
        .transpose()
}

fn required_utf8(
    reference: BlobRef,
    blobs: &HashMap<String, Vec<u8>>,
    used: &mut HashSet<String>,
    media_type: &str,
) -> Result<String, String> {
    String::from_utf8(required_blob(reference, blobs, used, media_type)?)
        .map_err(|_| "归档文本 blob 不是 UTF-8".to_string())
}

fn optional_utf8(
    reference: Option<BlobRef>,
    blobs: &HashMap<String, Vec<u8>>,
    used: &mut HashSet<String>,
    media_type: &str,
) -> Result<Option<String>, String> {
    reference
        .map(|value| required_utf8(value, blobs, used, media_type))
        .transpose()
}

fn validate_clip_shape(
    content_type: &crate::models::ContentType,
    text: Option<&str>,
    html: Option<&str>,
    image: Option<&[u8]>,
) -> Result<(), String> {
    match content_type {
        crate::models::ContentType::Text if text.is_some() && html.is_none() && image.is_none() => {
            Ok(())
        }
        crate::models::ContentType::Html if text.is_some() && html.is_some() && image.is_none() => {
            Ok(())
        }
        crate::models::ContentType::Image
            if text.is_none() && html.is_none() && image.is_some() =>
        {
            validate_png(image.unwrap()).map(|_| ())
        }
        _ => Err("归档历史条目的内容类型与 blob 不一致".to_string()),
    }
}

fn validate_snapshot_shape(
    content_type: &crate::models::ContentType,
    text: Option<&str>,
    html: Option<&str>,
) -> Result<(), String> {
    match content_type {
        crate::models::ContentType::Text if text.is_some() && html.is_none() => Ok(()),
        crate::models::ContentType::Html if text.is_some() && html.is_some() => Ok(()),
        _ => Err("归档 Pin 文本快照类型无效".to_string()),
    }
}

fn validate_primary_hash(
    content_type: &crate::models::ContentType,
    text: Option<&str>,
    html: Option<&str>,
    image: Option<&[u8]>,
    declared: &str,
) -> Result<(), String> {
    let bytes = match content_type {
        crate::models::ContentType::Text => text.unwrap_or_default().as_bytes(),
        crate::models::ContentType::Html => html.unwrap_or_default().as_bytes(),
        crate::models::ContentType::Image => image.unwrap_or_default(),
    };
    if hash(bytes) != declared {
        return Err("归档内容哈希与主内容不一致".to_string());
    }
    Ok(())
}

fn validate_png(bytes: &[u8]) -> Result<(u32, u32), String> {
    let dimensions = png_dimensions(bytes)?;
    let decoded_dimensions = crate::pin::output::validate_archive_png(
        bytes,
        usize::try_from(MAX_BLOB_BYTES).map_err(|error| error.to_string())?,
    )?;
    if decoded_dimensions != dimensions {
        return Err("归档 PNG 解码尺寸与文件头不一致".to_string());
    }
    Ok(dimensions)
}

fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    let reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| format!("归档 PNG 头无效: {error}"))?;
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err("归档 PNG 尺寸超过安全上限".to_string());
    }
    Ok((width, height))
}

fn validate_png_dimensions(bytes: &[u8], width: u32, height: u32) -> Result<(), String> {
    // 修订随后会由权威渲染器完整解码；这里先在分配 RGBA 前执行归档的更窄尺寸预算。
    if png_dimensions(bytes)? != (width, height) {
        return Err("归档根图尺寸与 manifest 不一致".to_string());
    }
    Ok(())
}

fn validate_presentation(
    width: f64,
    height: f64,
    scale: f64,
    opacity: f64,
    placement: Option<&StoredPinPlacement>,
) -> Result<(), String> {
    if ![width, height, scale, opacity]
        .into_iter()
        .all(f64::is_finite)
        || width <= 0.0
        || height <= 0.0
        || !(0.25..=4.0).contains(&scale)
        || !(0.15..=1.0).contains(&opacity)
    {
        return Err("归档 Pin 显示状态无效".to_string());
    }
    if let Some(value) = placement {
        let values = [
            value.x,
            value.y,
            value.display_x,
            value.display_y,
            value.display_width,
            value.display_height,
            value.display_scale,
        ];
        if !values.into_iter().all(f64::is_finite)
            || value.display_width <= 0.0
            || value.display_height <= 0.0
            || value.display_scale <= 0.0
        {
            return Err("归档 Pin 显示器位置无效".to_string());
        }
    }
    Ok(())
}

fn project_hash(
    renderer_version: u32,
    source_width: u32,
    source_height: u32,
    annotations_json: &str,
    adjustments_json: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(renderer_version.to_be_bytes());
    digest.update(source_width.to_be_bytes());
    digest.update(source_height.to_be_bytes());
    for value in [annotations_json, adjustments_json] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn hash(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_blob_name(name: &str) -> bool {
    name.strip_prefix("blobs/").is_some_and(valid_hash)
}

fn valid_group_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name.chars().count() <= 64 && !name.chars().any(char::is_control)
}

fn default_archive_name() -> String {
    format!(
        "clippy-{}.clippy.zip",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    )
}

#[tauri::command]
pub async fn export_clippy_archive(
    scope: ArchiveScope,
    include_sensitive: bool,
    app_handle: tauri::AppHandle,
) -> Result<Option<ArchiveExportResult>, String> {
    let start = app_handle
        .state::<crate::commands::AppState>()
        .save_target()
        .directory;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) =
            crate::dialogs::choose_archive_save(&app_handle, &start, &default_archive_name())
        else {
            return Ok(None);
        };
        let state = app_handle.state::<crate::commands::AppState>();
        let snapshot = state
            .storage
            .lock()
            .map_err(|error| error.to_string())?
            .export_archive_snapshot(scope, include_sensitive)
            .map_err(|error| error.to_string())?;
        write_archive(&path, &snapshot).map(Some)
    })
    .await
    .map_err(|error| format!("归档导出线程异常: {error}"))?
}

#[tauri::command]
pub async fn import_clippy_archive(
    app_handle: tauri::AppHandle,
) -> Result<Option<ImportSummary>, String> {
    let start = app_handle
        .state::<crate::commands::AppState>()
        .save_target()
        .directory;
    let result = tauri::async_runtime::spawn_blocking({
        let app_handle = app_handle.clone();
        move || -> Result<Option<ImportSummary>, String> {
            let Some(path) = crate::dialogs::choose_archive_open(&app_handle, &start) else {
                return Ok(None);
            };
            let snapshot = read_archive(&path)?;
            let state = app_handle.state::<crate::commands::AppState>();
            let imported = {
                let storage = state.storage.lock().map_err(|error| error.to_string())?;
                storage
                    .import_archive_snapshot(&snapshot)
                    .map_err(|error| error.to_string())?
            };
            Ok(Some(imported))
        }
    })
    .await
    .map_err(|error| format!("归档导入线程异常: {error}"))??;
    if result.is_some() {
        if result
            .as_ref()
            .is_some_and(|summary| summary.workspaces_added > 0)
        {
            let restore_app = app_handle.clone();
            match tauri::async_runtime::spawn_blocking(move || {
                let state = restore_app.state::<crate::commands::AppState>();
                crate::pin::restore_saved(&restore_app, &state)
            })
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => log::warn!("导入后恢复 Pin 工作区失败: {error}"),
                Err(error) => log::warn!("导入后恢复 Pin 工作区线程异常: {error}"),
            }
        }
        let _ = app_handle.emit("archive-imported", ());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_snapshot() -> ArchiveSnapshot {
        let text = "hello 归档".to_string();
        ArchiveSnapshot {
            scope: ArchiveScope::Full,
            clips: vec![ArchiveClipSnapshot {
                content_type: crate::models::ContentType::Text,
                text_content: Some(text.clone()),
                html_content: None,
                image_png: None,
                content_hash: hash(text.as_bytes()),
                ocr_text: None,
                is_favorite: true,
                is_sensitive: false,
                created_at: 123,
                byte_size: text.len() as i64,
                revision: None,
            }],
            groups: Vec::new(),
            workspaces: Vec::new(),
        }
    }

    fn image_snapshot() -> ArchiveSnapshot {
        let source_png = crate::screenshot::encode_png(&[20, 40, 60, 255], 1, 1).unwrap();
        let project = crate::pin::output::identity_project(&source_png).unwrap();
        let rendered = crate::pin::output::render_document(&source_png, Some(&project)).unwrap();
        let annotations_json = serde_json::to_string(&project.annotations).unwrap();
        let adjustments_json = serde_json::to_string(&project.adjustments).unwrap();
        let revision = ArchiveRevisionSnapshot {
            source_hash: hash(&source_png),
            source_png,
            source_width: 1,
            source_height: 1,
            renderer_version: project.renderer_version,
            annotations_json: annotations_json.clone(),
            adjustments_json: adjustments_json.clone(),
            document_hash: project_hash(
                project.renderer_version,
                1,
                1,
                &annotations_json,
                &adjustments_json,
            ),
            rendered_hash: hash(&rendered),
        };
        ArchiveSnapshot {
            scope: ArchiveScope::Full,
            clips: vec![ArchiveClipSnapshot {
                content_type: crate::models::ContentType::Image,
                text_content: None,
                html_content: None,
                image_png: Some(rendered.clone()),
                content_hash: hash(&rendered),
                ocr_text: Some("pixel".to_string()),
                is_favorite: false,
                is_sensitive: false,
                created_at: 456,
                byte_size: rendered.len() as i64,
                revision: Some(revision.clone()),
            }],
            groups: vec![ArchiveGroupSnapshot {
                source_id: 4,
                name: "Reference".to_string(),
                sort_order: 0,
            }],
            workspaces: vec![ArchiveWorkspaceSnapshot {
                source_id: 8,
                group_source_id: Some(4),
                content: ArchiveWorkspaceContentSnapshot::Image { revision },
                content_width: 1.0,
                content_height: 1.0,
                scale: 1.0,
                opacity: 1.0,
                locked: false,
                above: true,
                placement: None,
            }],
        }
    }

    #[test]
    fn archive_round_trip_and_hash_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.clippy.zip");
        let snapshot = text_snapshot();
        write_archive(&path, &snapshot).unwrap();
        assert_eq!(read_archive(&path).unwrap(), snapshot);

        let bad = dir.path().join("bad.clippy.zip");
        let file = File::create(&bad).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file("../manifest.json", options).unwrap();
        writer.write_all(b"{}").unwrap();
        writer.finish().unwrap();
        assert!(read_archive(&bad).unwrap_err().contains("不允许的路径"));
    }

    #[test]
    fn truncated_and_tampered_archives_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.clippy.zip");
        write_archive(&good, &text_snapshot()).unwrap();

        let mut truncated_bytes = std::fs::read(&good).unwrap();
        truncated_bytes.truncate(truncated_bytes.len() - 12);
        let truncated = dir.path().join("truncated.clippy.zip");
        std::fs::write(&truncated, truncated_bytes).unwrap();
        assert!(read_archive(&truncated).is_err());

        let (manifest, blobs) = manifest_from_snapshot(&text_snapshot()).unwrap();
        let tampered = dir.path().join("tampered.clippy.zip");
        let file = File::create(&tampered).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file(MANIFEST_NAME, options).unwrap();
        writer
            .write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        for (sha, mut bytes) in blobs.0 {
            bytes[0] ^= 0xff;
            writer.start_file(format!("blobs/{sha}"), options).unwrap();
            writer.write_all(&bytes).unwrap();
        }
        writer.finish().unwrap();
        assert!(read_archive(&tampered)
            .unwrap_err()
            .contains("blob 哈希错误"));
    }

    #[test]
    fn image_revision_and_workspace_survive_file_and_database_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("images.clippy.zip");
        let snapshot = image_snapshot();
        write_archive(&path, &snapshot).unwrap();
        let decoded = read_archive(&path).unwrap();
        assert_eq!(decoded, snapshot);

        let storage = crate::storage::StorageEngine::new_in_memory().unwrap();
        let summary = storage.import_archive_snapshot(&decoded).unwrap();
        assert_eq!(summary.clips_added, 1);
        assert_eq!(summary.groups_added, 1);
        assert_eq!(summary.workspaces_added, 1);
        let clips = storage.get_clips(None, false, 0, 10).unwrap();
        assert_eq!(clips.len(), 1);
        assert!(storage
            .get_image_revision_for_clip(clips[0].id)
            .unwrap()
            .is_some());
        let exported = storage
            .export_archive_snapshot(ArchiveScope::Full, true)
            .unwrap();
        assert_eq!(exported.clips.len(), 1);
        assert_eq!(exported.groups.len(), 1);
        assert_eq!(exported.workspaces.len(), 1);
    }

    #[test]
    fn duplicate_merge_is_conservative_and_relation_failure_rolls_back() {
        let storage = crate::storage::StorageEngine::new_in_memory().unwrap();
        let mut first = text_snapshot();
        first.clips[0].is_favorite = false;
        let summary = storage.import_archive_snapshot(&first).unwrap();
        assert_eq!(summary.clips_added, 1);

        let mut duplicate = text_snapshot();
        duplicate.clips[0].is_sensitive = true;
        let summary = storage.import_archive_snapshot(&duplicate).unwrap();
        assert_eq!(summary.clips_merged, 1);
        let clips = storage.get_clips(None, false, 0, 10).unwrap();
        assert_eq!(clips.len(), 1);
        assert!(clips[0].is_favorite);
        assert!(clips[0].is_sensitive);

        let mut broken = text_snapshot();
        broken.clips[0].text_content = Some("another".to_string());
        broken.clips[0].content_hash = hash(b"another");
        broken.clips[0].byte_size = 7;
        broken.workspaces.push(ArchiveWorkspaceSnapshot {
            source_id: 99,
            group_source_id: Some(404),
            content: ArchiveWorkspaceContentSnapshot::Snapshot {
                content_type: crate::models::ContentType::Text,
                text_content: Some("workspace".to_string()),
                html_content: None,
                content_hash: hash(b"workspace"),
            },
            content_width: 100.0,
            content_height: 50.0,
            scale: 1.0,
            opacity: 1.0,
            locked: false,
            above: false,
            placement: None,
        });
        assert!(storage.import_archive_snapshot(&broken).is_err());
        assert_eq!(storage.get_clips(None, false, 0, 10).unwrap().len(), 1);
    }

    #[test]
    fn export_scope_and_sensitive_opt_in_are_enforced() {
        let storage = crate::storage::StorageEngine::new_in_memory().unwrap();
        let regular = storage
            .insert_clip(
                &crate::models::ContentType::Text,
                Some("regular"),
                None,
                None,
                &hash(b"regular"),
                7,
                false,
            )
            .unwrap();
        let sensitive = storage
            .insert_clip(
                &crate::models::ContentType::Text,
                Some("secret"),
                None,
                None,
                &hash(b"secret"),
                6,
                true,
            )
            .unwrap();
        storage.toggle_favorite(sensitive.id).unwrap();

        let safe = storage
            .export_archive_snapshot(ArchiveScope::Full, false)
            .unwrap();
        assert_eq!(safe.clips.len(), 1);
        assert_eq!(safe.clips[0].content_hash, regular.content_hash);

        let favorites = storage
            .export_archive_snapshot(ArchiveScope::Favorites, true)
            .unwrap();
        assert_eq!(favorites.clips.len(), 1);
        assert_eq!(favorites.clips[0].content_hash, sensitive.content_hash);
        assert!(favorites.groups.is_empty());
        assert!(favorites.workspaces.is_empty());
    }

    #[test]
    fn item_limit_and_duplicate_content_have_deterministic_results() {
        let clip = text_snapshot().clips.remove(0);
        let oversized = ArchiveSnapshot {
            scope: ArchiveScope::Full,
            clips: vec![clip.clone(); MAX_ITEMS + 1],
            groups: Vec::new(),
            workspaces: Vec::new(),
        };
        let dir = tempfile::tempdir().unwrap();
        assert!(
            write_archive(&dir.path().join("too-many.clippy.zip"), &oversized)
                .unwrap_err()
                .contains("10,000")
        );

        let storage = crate::storage::StorageEngine::new_in_memory().unwrap();
        let duplicate = ArchiveSnapshot {
            scope: ArchiveScope::Full,
            clips: vec![clip.clone(), clip],
            groups: Vec::new(),
            workspaces: Vec::new(),
        };
        let summary = storage.import_archive_snapshot(&duplicate).unwrap();
        assert_eq!(summary.clips_added, 1);
        assert_eq!(summary.clips_merged, 1);
        assert_eq!(storage.get_clips(None, false, 0, 10).unwrap().len(), 1);
    }

    #[test]
    fn unknown_future_version_is_rejected() {
        let (mut manifest, blobs) = manifest_from_snapshot(&text_snapshot()).unwrap();
        manifest.version = VERSION + 1;
        manifest.archive_id.clear();
        manifest.archive_id = hash(&serde_json::to_vec(&manifest).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.clippy.zip");
        let file = File::create(&path).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file(MANIFEST_NAME, options).unwrap();
        writer
            .write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        for (sha, bytes) in blobs.0 {
            writer.start_file(format!("blobs/{sha}"), options).unwrap();
            writer.write_all(&bytes).unwrap();
        }
        writer.finish().unwrap();
        assert_eq!(
            read_archive(&path).unwrap_err(),
            "不支持的 Clippy 归档版本: 2"
        );
    }

    #[test]
    fn unknown_fields_in_current_version_are_ignored() {
        let snapshot = text_snapshot();
        let (mut manifest, blobs) = manifest_from_snapshot(&snapshot).unwrap();
        manifest.archive_id.clear();
        manifest.archive_id = hash(&serde_json::to_vec(&manifest).unwrap());
        let mut value = serde_json::to_value(&manifest).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("futureOptionalHint".to_string(), serde_json::json!(true));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unknown-field.clippy.zip");
        let file = File::create(&path).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file(MANIFEST_NAME, options).unwrap();
        writer
            .write_all(&serde_json::to_vec(&value).unwrap())
            .unwrap();
        for (sha, bytes) in blobs.0 {
            writer.start_file(format!("blobs/{sha}"), options).unwrap();
            writer.write_all(&bytes).unwrap();
        }
        writer.finish().unwrap();
        assert_eq!(read_archive(&path).unwrap(), snapshot);
    }
}
