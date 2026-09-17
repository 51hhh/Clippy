//! Pin 与普通查看器共用的可信图像输出。
//!
//! 这里拥有画布 wire DTO、可信原图选择和 PNG 生成规则。调用方可以直接测试这些
//! 规则，不需要构造 Tauri command 或窗口。
use super::model::{PinEntry, PinSource};
use serde::{Deserialize, Serialize};

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

pub(crate) fn decode_source(png: &[u8]) -> Result<image::RgbaImage, String> {
    super::project::decode_png(png, super::project::MAX_SOURCE_PNG_BYTES, "图片原图")
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
    let disk = match mode {
        PinCanvasSaveMode::Flat => rendered.clone(),
        PinCanvasSaveMode::Editable => {
            let (annotations, adjustments) = document.map(|doc| (doc.annotations.clone(), doc.adjustments.clone()))
                .unwrap_or_else(|| (serde_json::json!([]), serde_json::json!({
                    "grayscale":false,"brightness":0,"contrast":0,"saturation":0,"cornerRadius":0
                })));
            let project = super::project::PinProject::new(
                source,
                &rendered,
                super::render_v2::RENDERER_VERSION,
                annotations,
                adjustments,
            )?;
            super::project::embed(&rendered, &project)?
        }
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
    mode: PinCanvasSaveMode,
    project: Option<PinCanvasProject>,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let Some(png_base64) = png_base64 else {
        if let Some(project) = project {
            if project.renderer_version != super::render_v2::RENDERER_VERSION {
                return Err("只有 renderer v2 文档可以由后端生成合成图".to_string());
            }
            let png = render_pin_document(entry, &project)?;
            let to_disk = match mode {
                PinCanvasSaveMode::Flat => super::project::flatten(&png)?,
                PinCanvasSaveMode::Editable => embed_pin_project(&png, project, entry)?,
            };
            return Ok((png, to_disk));
        }
        let PinSource::Project {
            source_png,
            preview_png,
            project,
        } = &*entry.source
        else {
            return Err("只有未修改的导入工程可以复用合成预览".to_string());
        };
        let to_disk = match mode {
            PinCanvasSaveMode::Flat => preview_png.clone(),
            PinCanvasSaveMode::Editable => {
                let materialized = project.materialize(source_png);
                super::project::embed(preview_png, &materialized)?
            }
        };
        return Ok((preview_png.clone(), to_disk));
    };

    let png = decode_canvas_png(png_base64)?;
    if project
        .as_ref()
        .is_some_and(|project| project.renderer_version == super::render_v2::RENDERER_VERSION)
    {
        return Err("renderer v2 不接受 WebView 上传的合成 PNG".to_string());
    }
    let to_disk = match mode {
        PinCanvasSaveMode::Flat => super::project::flatten(&png)?,
        PinCanvasSaveMode::Editable => {
            let project = project.ok_or_else(|| "保存可编辑 PNG 缺少工程数据".to_string())?;
            embed_pin_project(&png, project, entry)?
        }
    };
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

/// 给落盘的 PNG 加上工程块；任何一步失败都让 editable save 明确失败。
fn embed_pin_project(
    png: &[u8],
    project: PinCanvasProject,
    entry: &PinEntry,
) -> Result<Vec<u8>, String> {
    if !matches!(
        project.renderer_version,
        super::project::LEGACY_RENDERER_VERSION | super::project::RENDERER_VERSION
    ) {
        return Err("工程渲染器版本不受支持".to_string());
    }
    let source =
        source_png(&entry.source).ok_or_else(|| "文本贴图不能保存为可编辑 PNG".to_string())?;
    let dimensions =
        crate::screenshot::png_dimensions(source).map_err(|_| "贴图原图无效".to_string())?;
    if dimensions != (project.source_width, project.source_height) {
        return Err("工程 sourceWidth/sourceHeight 与原图不匹配".to_string());
    }
    let rendered_dimensions =
        crate::screenshot::png_dimensions(png).map_err(|_| "合成 PNG 无效".to_string())?;
    if rendered_dimensions != dimensions {
        return Err("合成 PNG 尺寸必须与工程原图一致".to_string());
    }
    let document = super::project::PinProject::new(
        source,
        png,
        project.renderer_version,
        project.annotations,
        project.adjustments,
    )?;
    super::project::embed(png, &document)
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
