//! Pin 与普通查看器共用的可信图像输出。
//!
//! 这里拥有画布 wire DTO、可信原图选择和 PNG 生成规则。调用方可以直接测试这些
//! 规则，不需要构造 Tauri command 或窗口。
use super::model::{PinEntry, PinSource};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PinCanvasSaveMode {
    Editable,
    Flat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PinCanvasSaveResult {
    pub path: String,
    pub clipboard_written: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clipboard_error: Option<String>,
}

/// 前端送来的工程操作层。原图一律由后端从 Pin 条目或 Capture 会话取。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinCanvasProject {
    pub renderer_version: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub annotations: serde_json::Value,
    pub adjustments: serde_json::Value,
}

pub(super) fn effective_project(
    entry: &PinEntry,
    submitted: Option<&PinCanvasProject>,
) -> Option<PinCanvasProject> {
    submitted.cloned().or_else(|| {
        let PinSource::Project { project, .. } = &*entry.source else {
            return None;
        };
        let (renderer_version, source_width, source_height, annotations, adjustments) =
            project.document_parts();
        Some(PinCanvasProject {
            renderer_version,
            source_width,
            source_height,
            annotations,
            adjustments,
        })
    })
}

/// 把已由 renderer v2 验证并合成的结果登记为内部修订。系统剪贴板仍只收到扁平
/// 像素；稍后 watcher 以相同规范 PNG 哈希入库时会在同一事务里自动关联该修订。
pub(super) fn register_image_revision(
    storage: &crate::storage::StorageEngine,
    entry: &PinEntry,
    project: &PinCanvasProject,
    rendered_png: &[u8],
) -> Result<i64, String> {
    if project.renderer_version == super::project::LEGACY_RENDERER_VERSION {
        // renderer v1 的历史坐标/字体语义不能伪装成 v2 累计文档。未修改保存时以当前
        // 已验证预览作为新的无损根图，后续编辑从这组像素开始；第一次真实编辑会由
        // 前端提交 renderer v2 文档，并继续使用旧工程原图走下面的正常路径。
        let identity = identity_project(rendered_png)?;
        return register_source_revision(storage, rendered_png, None, &identity, rendered_png);
    }
    let source = source_png(&entry.source).ok_or_else(|| "文本贴图不能登记图片修订".to_string())?;
    let root_clip_id = match &*entry.source {
        PinSource::Clip { item, .. } => Some(item.id),
        PinSource::Screenshot { .. } | PinSource::Project { .. } => None,
    };
    register_source_revision(storage, source, root_clip_id, project, rendered_png)
}

pub(crate) fn register_source_revision(
    storage: &crate::storage::StorageEngine,
    source: &[u8],
    root_clip_id: Option<i64>,
    project: &PinCanvasProject,
    rendered_png: &[u8],
) -> Result<i64, String> {
    if project.renderer_version != super::render_v2::RENDERER_VERSION {
        return Err("只有 renderer v2 文档可以登记内部图片修订".to_string());
    }
    let dimensions = decode_source(source)?.dimensions();
    if dimensions != (project.source_width, project.source_height) {
        return Err("图片修订原图尺寸不匹配".to_string());
    }
    let canonical_render = super::project::flatten(rendered_png)?;
    let source_hash = crate::clipboard_watcher::content::compute_hash(source);
    let rendered_hash = crate::clipboard_watcher::content::compute_hash(&canonical_render);
    let annotations_json = serde_json::to_string(&project.annotations)
        .map_err(|error| format!("标注序列化失败: {error}"))?;
    let adjustments_json = serde_json::to_string(&project.adjustments)
        .map_err(|error| format!("调整序列化失败: {error}"))?;
    let mut digest = Sha256::new();
    digest.update(project.renderer_version.to_be_bytes());
    digest.update(project.source_width.to_be_bytes());
    digest.update(project.source_height.to_be_bytes());
    for value in [&annotations_json, &adjustments_json] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    let document_hash = format!("{:x}", digest.finalize());
    storage
        .register_image_revision(crate::storage::ImageRevisionWrite {
            source_png: source,
            source_hash: &source_hash,
            source_width: project.source_width,
            source_height: project.source_height,
            renderer_version: project.renderer_version,
            annotations_json: &annotations_json,
            adjustments_json: &adjustments_json,
            document_hash: &document_hash,
            rendered_hash: &rendered_hash,
            root_clip_id,
        })
        .map_err(|error| error.to_string())
}

pub(crate) fn identity_project(source: &[u8]) -> Result<PinCanvasProject, String> {
    let (source_width, source_height) = decode_source(source)?.dimensions();
    Ok(PinCanvasProject {
        renderer_version: super::render_v2::RENDERER_VERSION,
        source_width,
        source_height,
        annotations: serde_json::json!([]),
        adjustments: serde_json::json!({
            "grayscale": false,
            "brightness": 0,
            "contrast": 0,
            "saturation": 0,
            "cornerRadius": 0
        }),
    })
}

/// 把存储层修订恢复为查看器所需的根图、累计操作层与初始 payload。预览只参与
/// 像素身份校验；画布必须从根图重放操作，不能在扁平预览上重复绘制。
pub(crate) fn restore_managed_revision(
    preview_png: &[u8],
    revision: crate::storage::StoredImageRevision,
) -> Result<(Vec<u8>, PinCanvasProject, serde_json::Value), String> {
    let project = super::project::RuntimeProject::from_managed(
        &revision.source_png,
        preview_png,
        revision.renderer_version,
        revision.source_width,
        revision.source_height,
        revision.annotations.clone(),
        revision.adjustments.clone(),
    )?;
    let initial = serde_json::to_value(project.initial_payload())
        .map_err(|error| format!("图片修订 payload 序列化失败: {error}"))?;
    let document = PinCanvasProject {
        renderer_version: revision.renderer_version,
        source_width: revision.source_width,
        source_height: revision.source_height,
        annotations: revision.annotations,
        adjustments: revision.adjustments,
    };
    Ok((revision.source_png, document, initial))
}

pub(crate) fn decode_source(png: &[u8]) -> Result<image::RgbaImage, String> {
    super::project::decode_png(png, super::project::MAX_SOURCE_PNG_BYTES, "图片原图")
}

/// 归档导入使用严格 PNG 容器校验，但保留归档自己的字节与像素预算。
pub(crate) fn validate_archive_png(png: &[u8], byte_limit: usize) -> Result<(u32, u32), String> {
    super::image_validation::validate_strict_png(png, byte_limit, "归档 PNG")
}

pub(crate) fn render_document(
    source: &[u8],
    document: Option<&PinCanvasProject>,
) -> Result<Vec<u8>, String> {
    match document {
        Some(document) if document.renderer_version == super::render_v2::RENDERER_VERSION => {
            super::render_v2::render(
                source,
                document.source_width,
                document.source_height,
                &document.annotations,
                &document.adjustments,
            )
        }
        Some(_) => Err("仅接受 renderer v2 操作文档".into()),
        None => super::project::flatten(source),
    }
}

pub(crate) fn prepare_save(
    source: &[u8],
    document: Option<&PinCanvasProject>,
    mode: PinCanvasSaveMode,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let rendered = render_document(source, document)?;
    // 新修订的可编辑状态由 SQLite asset/revision 管理，磁盘与剪贴板都只得到扁平
    // 合成图。这样分享打码图不会夹带原图，根图也不会在每个版本里重复保存。
    let disk = match mode {
        PinCanvasSaveMode::Flat | PinCanvasSaveMode::Editable => rendered.clone(),
    };
    Ok((rendered, disk))
}

/// 生成 Pin 命令写入剪贴板的扁平像素与写入磁盘的 PNG 容器。
///
/// `png_base64 = None` 只表示“导入工程尚未发生本地编辑”：这时后端持有的 IDAT 预览
/// 比任一平台重新跑 Canvas 更权威。普通图片或已编辑文档必须提交最新渲染结果。
pub(super) fn prepare_pin_save(
    entry: &PinEntry,
    png_base64: Option<&str>,
    _mode: PinCanvasSaveMode,
    project: Option<PinCanvasProject>,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let Some(png_base64) = png_base64 else {
        if let Some(project) = project {
            if project.renderer_version != super::render_v2::RENDERER_VERSION {
                return Err("只有 renderer v2 文档可以由后端生成合成图".to_string());
            }
            let png = render_pin_document(entry, &project)?;
            let to_disk = super::project::flatten(&png)?;
            return Ok((png, to_disk));
        }
        let PinSource::Project { preview_png, .. } = &*entry.source else {
            return Err("只有未修改的导入工程可以复用合成预览".to_string());
        };
        let to_disk = super::project::flatten(preview_png)?;
        return Ok((preview_png.clone(), to_disk));
    };

    let png = decode_canvas_png(png_base64)?;
    if project
        .as_ref()
        .is_some_and(|project| project.renderer_version == super::render_v2::RENDERER_VERSION)
    {
        return Err("renderer v2 不接受 WebView 上传的合成 PNG".to_string());
    }
    let to_disk = super::project::flatten(&png)?;
    Ok((png, to_disk))
}

/// 已编辑贴图的 Copy/Ctrl+C：只返回最新合成像素，不携带 iTXt。
pub(super) fn prepare_pin_copy(
    entry: &PinEntry,
    png_base64: Option<&str>,
    project: Option<PinCanvasProject>,
) -> Result<Vec<u8>, String> {
    match project {
        Some(project) if project.renderer_version == super::render_v2::RENDERER_VERSION => {
            if png_base64.is_some() {
                return Err("renderer v2 不接受 WebView 上传的合成 PNG".to_string());
            }
            render_pin_document(entry, &project)
        }
        Some(_) => {
            let encoded = png_base64.ok_or_else(|| "renderer v1 复制缺少合成 PNG".to_string())?;
            decode_canvas_png(encoded)
        }
        None => {
            let encoded = png_base64.ok_or_else(|| "复制画布缺少工程文档".to_string())?;
            decode_canvas_png(encoded)
        }
    }
}

pub(super) fn decode_canvas_png(png_base64: &str) -> Result<Vec<u8>, String> {
    // 在 base64 分配前先做保守长度判断；精确字节与完整 PNG 随后再验证。
    if png_base64.len() > super::project::MAX_RENDERED_PNG_BYTES.saturating_mul(4) / 3 + 64 {
        return Err("画布内容过大".to_string());
    }
    let png = crate::screenshot::decode_png_base64(png_base64)
        .map_err(|_| "画布内容 base64 无效".to_string())?;
    super::project::validate_rendered_png(&png)?;
    Ok(png)
}

fn render_pin_document(entry: &PinEntry, project: &PinCanvasProject) -> Result<Vec<u8>, String> {
    let source = source_png(&entry.source).ok_or_else(|| "文本贴图不能渲染画布工程".to_string())?;
    super::render_v2::render(
        source,
        project.source_width,
        project.source_height,
        &project.annotations,
        &project.adjustments,
    )
}

/// 贴图内容里的 canonical PNG 字节，文本贴图没有。
pub(super) fn source_png(source: &PinSource) -> Option<&[u8]> {
    match source {
        PinSource::Clip { image, .. } => image.as_deref(),
        PinSource::Screenshot { png } => Some(png.as_slice()),
        PinSource::Project { source_png, .. } => Some(source_png),
    }
}

/// 屏幕清晰度补偿基于合成预览；工程 canonical source 只供画布使用。
pub(super) fn display_png(source: &PinSource) -> Option<&[u8]> {
    match source {
        PinSource::Clip { image, .. } => image.as_deref(),
        PinSource::Screenshot { png } => Some(png.as_slice()),
        PinSource::Project { preview_png, .. } => Some(preview_png),
    }
}

pub(super) fn image_bytes(entry: &PinEntry) -> Result<Vec<u8>, String> {
    match &*entry.source {
        PinSource::Clip {
            image: Some(png), ..
        } => Ok(png.clone()),
        PinSource::Screenshot { png } => Ok(png.as_ref().clone()),
        PinSource::Project { preview_png, .. } => Ok(preview_png.clone()),
        _ => Err("文本贴图不能保存或编辑为图片".to_string()),
    }
}

/// 分离系统剪贴板边界，保证所有 Pin 都从持有的快照复制，而不按历史 id 回查。
pub(super) fn copy_source(
    source: &PinSource,
    write_clip: impl FnOnce(&crate::models::ClipItem, Option<&[u8]>) -> Result<(), String>,
    write_png: impl FnOnce(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    match source {
        PinSource::Clip { item, image } => write_clip(item, image.as_deref()),
        PinSource::Screenshot { png } => write_png(png.as_slice()),
        PinSource::Project { preview_png, .. } => write_png(preview_png),
    }
}
