//! Pin 与普通查看器共用的可信图像输出，不依赖任何窗口条目或 WebView 合成 PNG。
use super::commands::{PinCanvasProject, PinCanvasSaveMode};

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
