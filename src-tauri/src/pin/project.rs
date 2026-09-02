//! 可编辑贴图 PNG 工程格式。
//!
//! PNG 的 IDAT 始终是最新合成图；压缩 iTXt `clippy-project` 保存自包含的 v3 工程。
//! 元数据是用户可控输入，本模块在它进入运行时状态前完成全部验证。工程损坏、v1 或未来
//! 版本只会让调用方退回扁平 PNG，不会妨碍 IDAT 被正常使用。v2 仍可读取和无损再存；
//! v3 新增合成像素摘要，防止合法 iTXt 被移植到另一张合法 IDAT 后冒充同一工程。

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub(super) const PROJECT_KEYWORD: &str = "clippy-project";
const PROJECT_FORMAT: &str = "clippy-pin-project";
const LEGACY_PROJECT_VERSION: u32 = 2;
pub(super) const PROJECT_VERSION: u32 = 3;
pub(super) const RENDERER_VERSION: u32 = 1;

pub(super) const MAX_RENDERED_PNG_BYTES: usize = 64 * 1024 * 1024;
pub(super) const MAX_SOURCE_PNG_BYTES: usize = 64 * 1024 * 1024;
pub(super) const MAX_PROJECT_JSON_BYTES: usize = 96 * 1024 * 1024;
pub(super) const MAX_CONTAINER_BYTES: usize = 160 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 32_768;
const MAX_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_ANNOTATIONS: usize = 10_000;
const MAX_STROKE_POINTS: usize = 100_000;
const MAX_TOTAL_POINTS: usize = 500_000;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_ID_BYTES: usize = 128;
const MAX_COLOR_BYTES: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinProject {
    pub format: String,
    pub format_version: u32,
    pub renderer_version: u32,
    pub created_at: i64,
    pub app_version: String,
    pub source: ProjectSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<ProjectPreview>,
    pub document: ProjectDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectSource {
    pub png_base64: String,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
}

/// IDAT 解码后的像素身份。绑定像素而不是 PNG 文件字节，避免压缩级别、chunk 切分或
/// 无关辅助块变化让同一张合成图失效。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectPreview {
    pub width: u32,
    pub height: u32,
    pub rgba_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectDocument {
    pub annotations: Value,
    pub adjustments: Value,
}

/// 发给贴图窗口的恢复数据。原图字节由 `get_pin_source_image` 按需取，避免 payload 再带一份
/// 几 MB 的 base64。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitialProject {
    pub format: String,
    pub format_version: u32,
    pub renderer_version: u32,
    pub source: InitialProjectSource,
    pub document: ProjectDocument,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitialProjectSource {
    pub width: u32,
    pub height: u32,
    pub sha256: String,
}

impl PinProject {
    pub(super) fn new(
        source_png: &[u8],
        rendered_png: &[u8],
        annotations: Value,
        adjustments: Value,
    ) -> Result<Self, String> {
        let (width, height) = validate_png(source_png, MAX_SOURCE_PNG_BYTES, "工程原图")?;
        let preview = preview_fingerprint(rendered_png)?;
        if (preview.width, preview.height) != (width, height) {
            return Err("合成 PNG 尺寸必须与工程原图一致".to_string());
        }
        let document = ProjectDocument {
            annotations,
            adjustments,
        };
        validate_document(&document, width, height)?;
        Ok(Self {
            format: PROJECT_FORMAT.to_string(),
            format_version: PROJECT_VERSION,
            renderer_version: RENDERER_VERSION,
            created_at: chrono::Local::now().timestamp(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            source: ProjectSource {
                png_base64: STANDARD.encode(source_png),
                width,
                height,
                sha256: sha256_hex(source_png),
            },
            preview: Some(preview),
            document,
        })
    }

    pub(super) fn validate(&self) -> Result<Vec<u8>, String> {
        if self.format != PROJECT_FORMAT {
            return Err("工程格式标识不匹配".to_string());
        }
        if !matches!(
            self.format_version,
            LEGACY_PROJECT_VERSION | PROJECT_VERSION
        ) {
            return Err("工程版本不受支持".to_string());
        }
        if self.renderer_version != RENDERER_VERSION {
            return Err("工程渲染器版本不受支持".to_string());
        }
        if self.app_version.len() > 128 {
            return Err("工程应用版本字段过长".to_string());
        }
        let estimated = self
            .source
            .png_base64
            .len()
            .saturating_div(4)
            .saturating_mul(3);
        if estimated > MAX_SOURCE_PNG_BYTES {
            return Err("工程原图过大".to_string());
        }
        let source = STANDARD
            .decode(&self.source.png_base64)
            .map_err(|_| "工程原图 base64 无效".to_string())?;
        let (width, height) = validate_png(&source, MAX_SOURCE_PNG_BYTES, "工程原图")?;
        if (width, height) != (self.source.width, self.source.height) {
            return Err("工程原图尺寸不匹配".to_string());
        }
        if self.source.sha256.len() != 64
            || !self
                .source
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || !self
                .source
                .sha256
                .eq_ignore_ascii_case(&sha256_hex(&source))
        {
            return Err("工程原图哈希不匹配".to_string());
        }
        match (self.format_version, &self.preview) {
            (LEGACY_PROJECT_VERSION, None) => {}
            (PROJECT_VERSION, Some(preview)) => {
                validate_preview_metadata(preview, width, height)?;
            }
            (LEGACY_PROJECT_VERSION, Some(_)) => {
                return Err("v2 工程不能包含合成图摘要".to_string());
            }
            (PROJECT_VERSION, None) => return Err("v3 工程缺少合成图摘要".to_string()),
            _ => unreachable!("工程版本已在上方校验"),
        }
        validate_document(&self.document, width, height)?;
        Ok(source)
    }

    /// 验证容器当前 IDAT 与工程声明的是同一份合成像素。v2 没有该字段，只维持旧版兼容。
    fn validate_preview_fingerprint(&self, actual: &ProjectPreview) -> Result<(), String> {
        let Some(expected) = &self.preview else {
            return Ok(());
        };
        if expected.width != actual.width
            || expected.height != actual.height
            || !expected
                .rgba_sha256
                .eq_ignore_ascii_case(&actual.rgba_sha256)
        {
            return Err("工程合成图摘要与 IDAT 不匹配".to_string());
        }
        Ok(())
    }

    pub(super) fn initial_payload(&self) -> InitialProject {
        InitialProject {
            format: self.format.clone(),
            format_version: self.format_version,
            renderer_version: self.renderer_version,
            source: InitialProjectSource {
                width: self.source.width,
                height: self.source.height,
                sha256: self.source.sha256.clone(),
            },
            document: self.document.clone(),
        }
    }

    /// 仅供 `extract` 已完成校验后的运行时恢复，避免把原图再次完整解码一遍。
    pub(super) fn decoded_source(&self) -> Result<Vec<u8>, String> {
        STANDARD
            .decode(&self.source.png_base64)
            .map_err(|_| "工程原图 base64 无效".to_string())
    }
}

pub(super) fn validate_rendered_png(png: &[u8]) -> Result<(u32, u32), String> {
    validate_png(png, MAX_RENDERED_PNG_BYTES, "合成 PNG")
}

fn validate_png(png: &[u8], byte_limit: usize, name: &str) -> Result<(u32, u32), String> {
    Ok(decode_png(png, byte_limit, name)?.dimensions())
}

/// 在分配像素缓冲区前先校验文件大小和 IHDR 尺寸，再完成一次完整解码。
/// 调用方需要像素做摘要或重编码时复用返回值，不能为了每一步重新解同一张 4K PNG。
fn decode_png(png: &[u8], byte_limit: usize, name: &str) -> Result<image::RgbaImage, String> {
    if png.len() > byte_limit {
        return Err(format!("{name}超过 {} MiB 上限", byte_limit / 1024 / 1024));
    }
    let sanitized = strip_project_chunks(png).map_err(|_| format!("{name}不是合法 PNG"))?;
    let (width, height) =
        crate::screenshot::png_dimensions(&sanitized).map_err(|_| format!("{name}不是合法 PNG"))?;
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || pixels > MAX_IMAGE_PIXELS
    {
        return Err(format!("{name}尺寸超过安全上限"));
    }
    let image = image::load_from_memory_with_format(&sanitized, image::ImageFormat::Png)
        .map_err(|_| format!("{name}无法完整解码"))?
        .into_rgba8();
    if image.dimensions() != (width, height) {
        return Err(format!("{name}解码尺寸不匹配"));
    }
    Ok(image)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn preview_fingerprint(png: &[u8]) -> Result<ProjectPreview, String> {
    let image = decode_png(png, MAX_RENDERED_PNG_BYTES, "合成 PNG")?;
    Ok(preview_fingerprint_from_image(&image))
}

fn preview_fingerprint_from_image(image: &image::RgbaImage) -> ProjectPreview {
    let (width, height) = image.dimensions();
    let mut digest = Sha256::new();
    digest.update(width.to_be_bytes());
    digest.update(height.to_be_bytes());
    digest.update(image.as_raw());
    ProjectPreview {
        width,
        height,
        rgba_sha256: format!("{:x}", digest.finalize()),
    }
}

fn validate_preview_metadata(
    preview: &ProjectPreview,
    source_width: u32,
    source_height: u32,
) -> Result<(), String> {
    if (preview.width, preview.height) != (source_width, source_height) {
        return Err("工程合成图尺寸与原图不匹配".to_string());
    }
    if preview.rgba_sha256.len() != 64
        || !preview
            .rgba_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("工程合成图摘要无效".to_string());
    }
    Ok(())
}

/// 重新编码合成图并写入真正压缩的 iTXt。
pub(super) fn embed(png: &[u8], project: &PinProject) -> Result<Vec<u8>, String> {
    let image = decode_png(png, MAX_RENDERED_PNG_BYTES, "合成 PNG")?;
    let actual_preview = preview_fingerprint_from_image(&image);
    project.validate()?;
    let project = if project.format_version == LEGACY_PROJECT_VERSION {
        let mut upgraded = project.clone();
        upgraded.format_version = PROJECT_VERSION;
        upgraded.preview = Some(actual_preview.clone());
        upgraded.validate()?;
        upgraded
    } else {
        project.validate_preview_fingerprint(&actual_preview)?;
        project.clone()
    };
    let json =
        serde_json::to_string(&project).map_err(|error| format!("工程序列化失败: {error}"))?;
    if json.len() > MAX_PROJECT_JSON_BYTES {
        return Err("工程 JSON 过大".to_string());
    }
    let (width, height) = image.dimensions();
    let mut out = Vec::with_capacity(png.len().saturating_add(json.len() / 2));
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("PNG 头写入失败: {error}"))?;
        let mut chunk = png::text_metadata::ITXtChunk::new(PROJECT_KEYWORD, json);
        chunk.compressed = true;
        writer
            .write_text_chunk(&chunk)
            .map_err(|error| format!("写入工程数据失败: {error}"))?;
        writer
            .write_image_data(image.as_raw())
            .map_err(|error| format!("PNG 数据写入失败: {error}"))?;
    }
    if out.len() > MAX_CONTAINER_BYTES {
        return Err("可编辑 PNG 容器过大".to_string());
    }
    Ok(out)
}

/// 重新编码为不含任何工程块的普通 PNG。扁平导出不能信任前端传来的 PNG 没有元数据。
pub(super) fn flatten(png: &[u8]) -> Result<Vec<u8>, String> {
    validate_rendered_png(png)?;
    encode_flattened(png)
}

/// 打开可编辑容器时只把 IDAT 重编码后的轻量预览送给 webview，不把内嵌原图再随
/// `imageBase64` 复制一遍。
pub(super) fn flatten_container(png: &[u8]) -> Result<Vec<u8>, String> {
    validate_rendered_png_container(png)?;
    encode_flattened(png)
}

fn encode_flattened(png: &[u8]) -> Result<Vec<u8>, String> {
    let sanitized = strip_project_chunks(png)?;
    let image = image::load_from_memory_with_format(&sanitized, image::ImageFormat::Png)
        .map_err(|_| "合成 PNG 无法完整解码".to_string())?
        .into_rgba8();
    let (width, height) = image.dimensions();
    let flat = crate::screenshot::encode_png(image.as_raw(), width, height)
        .map_err(|error| format!("扁平 PNG 编码失败: {error}"))?;
    if flat.len() > MAX_RENDERED_PNG_BYTES {
        return Err("合成 PNG 超过 64 MiB 上限".to_string());
    }
    Ok(flat)
}

/// 元数据缺失、损坏、v1、未来版本或信任边界校验失败均返回 `Ok(None)`；只有容器本身不是
/// PNG 才返回 `Err`。
pub(super) fn extract(png: &[u8]) -> Result<Option<PinProject>, String> {
    if png.len() > MAX_CONTAINER_BYTES {
        return Err("PNG 文件超过 160 MiB 上限".to_string());
    }
    let actual_preview = validate_rendered_png_container(png)?;
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let reader = match decoder.read_info() {
        Ok(reader) => reader,
        Err(error) => {
            // IDAT 已通过去除工程块后的完整解码。此处失败只可能来自工程辅助块本身，
            // 因而安全降级，不让损坏元数据阻止图片打开。
            log::info!("贴图工程块解析失败，按普通图片处理: {error}");
            return Ok(None);
        }
    };
    for chunk in &reader.info().utf8_text {
        if chunk.keyword != PROJECT_KEYWORD {
            continue;
        }
        let mut chunk = chunk.clone();
        if chunk
            .decompress_text_with_limit(MAX_PROJECT_JSON_BYTES)
            .is_err()
        {
            log::info!("贴图工程解压失败或超过上限，按普通图片处理");
            return Ok(None);
        }
        let Ok(text) = chunk.get_text() else {
            log::info!("贴图工程文本无效，按普通图片处理");
            return Ok(None);
        };
        if text.len() > MAX_PROJECT_JSON_BYTES {
            return Ok(None);
        }
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            return Ok(None);
        };
        // v1 的 `version` 与 v2/v3 的 `formatVersion` 明确区分；不猜测旧坐标语义。
        let format_version = value.get("formatVersion").and_then(Value::as_u64);
        if !matches!(
            format_version,
            Some(version)
                if version == u64::from(LEGACY_PROJECT_VERSION)
                    || version == u64::from(PROJECT_VERSION)
        ) {
            return Ok(None);
        }
        let Ok(project) = serde_json::from_value::<PinProject>(value) else {
            return Ok(None);
        };
        if let Err(error) = project.validate() {
            log::info!("贴图工程校验失败，按普通图片处理: {error}");
            return Ok(None);
        }
        if let Err(error) = project.validate_preview_fingerprint(&actual_preview) {
            log::info!("贴图工程与合成图不匹配，按普通图片处理: {error}");
            return Ok(None);
        }
        return Ok(Some(project));
    }
    Ok(None)
}

fn validate_rendered_png_container(png: &[u8]) -> Result<ProjectPreview, String> {
    let sanitized = strip_project_chunks(png)?;
    let image = decode_png(&sanitized, MAX_RENDERED_PNG_BYTES, "合成 PNG")?;
    Ok(preview_fingerprint_from_image(&image))
}

/// 删除 raw PNG 流里 keyword 为 `clippy-project` 的 iTXt。这里不解析文本、不信任 CRC，
/// 因而即使该块的压缩流/CRC 损坏，也能继续验证并显示其余 IDAT。
fn strip_project_chunks(png: &[u8]) -> Result<Vec<u8>, String> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if png.len() < SIGNATURE.len() || &png[..8] != SIGNATURE {
        return Err("文件不是合法 PNG".to_string());
    }
    let mut out = Vec::with_capacity(png.len());
    out.extend_from_slice(SIGNATURE);
    let mut cursor = 8usize;
    let mut saw_iend = false;
    while cursor < png.len() {
        if png.len().saturating_sub(cursor) < 12 {
            return Err("PNG chunk 被截断".to_string());
        }
        let length = u32::from_be_bytes(
            png[cursor..cursor + 4]
                .try_into()
                .map_err(|_| "PNG chunk 长度无效".to_string())?,
        ) as usize;
        let end = cursor
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .ok_or_else(|| "PNG chunk 长度溢出".to_string())?;
        if end > png.len() {
            return Err("PNG chunk 被截断".to_string());
        }
        let chunk_type = &png[cursor + 4..cursor + 8];
        let data = &png[cursor + 8..cursor + 8 + length];
        let is_project = chunk_type == b"iTXt"
            && data
                .split(|byte| *byte == 0)
                .next()
                .is_some_and(|keyword| keyword == PROJECT_KEYWORD.as_bytes());
        if !is_project {
            out.extend_from_slice(&png[cursor..end]);
        }
        cursor = end;
        if chunk_type == b"IEND" {
            saw_iend = true;
            break;
        }
    }
    if !saw_iend || cursor != png.len() {
        return Err("PNG 缺少有效 IEND".to_string());
    }
    Ok(out)
}

fn validate_document(document: &ProjectDocument, width: u32, height: u32) -> Result<(), String> {
    let annotations = document
        .annotations
        .as_array()
        .ok_or_else(|| "annotations 必须是数组".to_string())?;
    if annotations.len() > MAX_ANNOTATIONS {
        return Err("annotations 数量超过上限".to_string());
    }
    let mut ids = HashSet::with_capacity(annotations.len());
    let mut total_points = 0usize;
    for annotation in annotations {
        validate_annotation(
            annotation,
            &mut ids,
            &mut total_points,
            f64::from(width),
            f64::from(height),
        )?;
    }
    validate_adjustments(&document.adjustments)
}

fn validate_annotation(
    value: &Value,
    ids: &mut HashSet<String>,
    total_points: &mut usize,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "annotation 必须是对象".to_string())?;
    let id = required_string(object, "id", MAX_ID_BYTES)?;
    if id.is_empty() || !ids.insert(id.to_string()) {
        return Err("annotation id 为空或重复".to_string());
    }
    let kind = required_string(object, "type", 32)?;
    match kind {
        "pen" | "marker" => {
            validate_keys(object, &["id", "type", "color", "size", "points"])?;
            validate_color_and_size(object)?;
            let points = object
                .get("points")
                .and_then(Value::as_array)
                .ok_or_else(|| "stroke points 必须是数组".to_string())?;
            if points.len() > MAX_STROKE_POINTS {
                return Err("单个 stroke 点数超过上限".to_string());
            }
            *total_points = total_points.saturating_add(points.len());
            if *total_points > MAX_TOTAL_POINTS {
                return Err("工程总点数超过上限".to_string());
            }
            for point in points {
                validate_point(point, width, height)?;
            }
        }
        "rect" | "ellipse" | "highlight" => {
            validate_keys(object, &["id", "type", "color", "size", "rect"])?;
            validate_color_and_size(object)?;
            validate_rect(required(object, "rect")?, width, height)?;
        }
        "line" | "arrow" | "measure" => {
            validate_keys(object, &["id", "type", "color", "size", "from", "to"])?;
            validate_color_and_size(object)?;
            add_points(total_points, 2)?;
            validate_point(required(object, "from")?, width, height)?;
            validate_point(required(object, "to")?, width, height)?;
        }
        "text" => {
            validate_keys(
                object,
                &["id", "type", "color", "size", "at", "text", "fontFamily"],
            )?;
            validate_color_and_size(object)?;
            add_points(total_points, 1)?;
            validate_point(required(object, "at")?, width, height)?;
            required_string(object, "text", MAX_TEXT_BYTES)?;
            if required(object, "fontFamily")?.as_str() != Some("system-ui") {
                return Err("fontFamily 不受支持".to_string());
            }
        }
        "blur" | "mosaic" | "spotlight" | "magnifier" => {
            validate_keys(object, &["id", "type", "rect", "effect"])?;
            validate_rect(required(object, "rect")?, width, height)?;
            validate_effect(required(object, "effect")?)?;
        }
        _ => return Err(format!("不支持的 annotation 类型: {kind}")),
    }
    Ok(())
}

fn validate_effect(value: &Value) -> Result<(), String> {
    let effect = value
        .as_object()
        .ok_or_else(|| "effect 必须是对象".to_string())?;
    validate_keys(
        effect,
        &["blurRadius", "mosaicCell", "spotlightDim", "magnifierZoom"],
    )?;
    validate_required_number(effect, "blurRadius", 1.0, 100.0)?;
    validate_required_number(effect, "mosaicCell", 1.0, 256.0)?;
    validate_required_number(effect, "spotlightDim", 0.0, 1.0)?;
    validate_required_number(effect, "magnifierZoom", 1.0, 16.0)
}

fn validate_adjustments(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "adjustments 必须是对象".to_string())?;
    validate_keys(
        object,
        &[
            "grayscale",
            "brightness",
            "contrast",
            "saturation",
            "cornerRadius",
        ],
    )?;
    if !matches!(object.get("grayscale"), Some(Value::Bool(_))) {
        return Err("adjustments.grayscale 必须是布尔值".to_string());
    }
    validate_required_number(object, "brightness", -100.0, 100.0)?;
    validate_required_number(object, "contrast", -100.0, 100.0)?;
    validate_required_number(object, "saturation", -100.0, 100.0)?;
    validate_required_number(object, "cornerRadius", 0.0, 120.0)
}

fn validate_color_and_size(object: &Map<String, Value>) -> Result<(), String> {
    let color = required_string(object, "color", MAX_COLOR_BYTES)?;
    if color.is_empty() {
        return Err("annotation color 不能为空".to_string());
    }
    validate_required_number(object, "size", 0.1, 128.0)
}

fn validate_point(value: &Value, width: f64, height: f64) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "point 必须是对象".to_string())?;
    validate_keys(object, &["x", "y"])?;
    validate_required_number(object, "x", 0.0, width)?;
    validate_required_number(object, "y", 0.0, height)
}

fn validate_rect(value: &Value, image_width: f64, image_height: f64) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "rect 必须是对象".to_string())?;
    validate_keys(object, &["x", "y", "width", "height"])?;
    validate_required_number(object, "x", 0.0, image_width)?;
    validate_required_number(object, "y", 0.0, image_height)?;
    validate_required_number(object, "width", 0.0, image_width)?;
    validate_required_number(object, "height", 0.0, image_height)?;
    let x = object["x"].as_f64().unwrap_or(f64::INFINITY);
    let y = object["y"].as_f64().unwrap_or(f64::INFINITY);
    let width = object["width"].as_f64().unwrap_or(f64::INFINITY);
    let height = object["height"].as_f64().unwrap_or(f64::INFINITY);
    if x + width > image_width || y + height > image_height {
        return Err("rect 超出原图边界".to_string());
    }
    Ok(())
}

fn add_points(total: &mut usize, count: usize) -> Result<(), String> {
    *total = total.saturating_add(count);
    if *total > MAX_TOTAL_POINTS {
        return Err("工程总点数超过上限".to_string());
    }
    Ok(())
}

fn required<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a Value, String> {
    object.get(key).ok_or_else(|| format!("缺少字段 {key}"))
}

fn validate_keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("不支持的字段 {key}"));
    }
    Ok(())
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    max_bytes: usize,
) -> Result<&'a str, String> {
    let value = required(object, key)?
        .as_str()
        .ok_or_else(|| format!("{key} 必须是字符串"))?;
    if value.len() > max_bytes {
        return Err(format!("{key} 过长"));
    }
    Ok(value)
}

fn validate_required_number(
    object: &Map<String, Value>,
    key: &str,
    min: f64,
    max: f64,
) -> Result<(), String> {
    let value = required(object, key)?
        .as_f64()
        .ok_or_else(|| format!("{key} 必须是数字"))?;
    if !value.is_finite() || !(min..=max).contains(&value) {
        return Err(format!("{key} 超出允许范围"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_png() -> Vec<u8> {
        crate::screenshot::encode_png(&[40, 80, 120, 255, 9, 8, 7, 255], 2, 1).unwrap()
    }

    fn annotations() -> Value {
        serde_json::json!([
            {"id":"pen-1","type":"pen","color":"#f00","size":4,
             "points":[{"x":0.0,"y":0.0},{"x":1.0,"y":1.0}]},
            {"id":"blur-1","type":"blur","rect":{"x":0,"y":0,"width":1,"height":1},
             "effect":{"blurRadius":8,"mosaicCell":12,"spotlightDim":0.55,"magnifierZoom":2}}
        ])
    }

    fn adjustments() -> Value {
        serde_json::json!({"grayscale":false,"brightness":0,"contrast":0,"saturation":0,"cornerRadius":0})
    }

    fn png_with_project_text(rendered: &[u8], text: String) -> Vec<u8> {
        let image = image::load_from_memory_with_format(rendered, image::ImageFormat::Png)
            .unwrap()
            .into_rgba8();
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, image.width(), image.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_itxt_chunk(PROJECT_KEYWORD.to_string(), text)
            .unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(image.as_raw()).unwrap();
        drop(writer);
        out
    }

    fn project() -> PinProject {
        let png = sample_png();
        PinProject::new(&png, &png, annotations(), adjustments()).unwrap()
    }

    #[test]
    fn v3_round_trip_is_compressed_bound_and_self_contained() {
        let rendered = sample_png();
        let original = project();
        let embedded = embed(&rendered, &original).unwrap();
        crate::screenshot::validate_png(&embedded).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(&embedded));
        let reader = decoder.read_info().unwrap();
        let chunk = reader
            .info()
            .utf8_text
            .iter()
            .find(|chunk| chunk.keyword == PROJECT_KEYWORD)
            .unwrap();
        assert!(chunk.compressed, "工程 iTXt 必须启用压缩");
        assert_eq!(extract(&embedded).unwrap(), Some(original));
    }

    #[test]
    fn legacy_v2_project_is_readable_and_resaves_as_bound_v3() {
        let rendered = sample_png();
        let mut legacy = project();
        legacy.format_version = LEGACY_PROJECT_VERSION;
        legacy.preview = None;

        let old_container =
            png_with_project_text(&rendered, serde_json::to_string(&legacy).unwrap());
        assert_eq!(extract(&old_container).unwrap(), Some(legacy.clone()));

        let upgraded_container = embed(&rendered, &legacy).unwrap();
        let upgraded = extract(&upgraded_container).unwrap().unwrap();
        assert_eq!(upgraded.format_version, PROJECT_VERSION);
        assert!(upgraded.preview.is_some());
        assert_eq!(upgraded.source, legacy.source);
        assert_eq!(upgraded.document, legacy.document);
    }

    #[test]
    fn v3_project_cannot_be_transplanted_onto_another_idat() {
        let rendered = sample_png();
        let project = project();
        let replacement =
            crate::screenshot::encode_png(&[1, 2, 3, 255, 4, 5, 6, 255], 2, 1).unwrap();
        let forged = png_with_project_text(&replacement, serde_json::to_string(&project).unwrap());

        assert_eq!(extract(&forged).unwrap(), None);
        let flat = flatten_container(&forged).unwrap();
        assert_eq!(
            image::load_from_memory_with_format(&flat, image::ImageFormat::Png)
                .unwrap()
                .into_rgba8(),
            image::load_from_memory_with_format(&replacement, image::ImageFormat::Png)
                .unwrap()
                .into_rgba8(),
            "工程不匹配只能降级，不能妨碍安全显示当前 IDAT"
        );
        assert!(embed(&replacement, &project).is_err());
        assert_ne!(rendered, replacement);
    }

    #[test]
    fn preview_binding_follows_pixels_not_png_compression_bytes() {
        let rendered = sample_png();
        let image = image::load_from_memory_with_format(&rendered, image::ImageFormat::Png)
            .unwrap()
            .into_rgba8();
        let mut differently_encoded = Vec::new();
        let mut encoder = png::Encoder::new(&mut differently_encoded, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Best);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(image.as_raw()).unwrap();
        drop(writer);

        assert_ne!(rendered, differently_encoded);
        let embedded = embed(&differently_encoded, &project()).unwrap();
        assert!(extract(&embedded).unwrap().is_some());
    }

    #[test]
    fn plain_corrupt_v1_and_future_metadata_fall_back_to_flat() {
        assert_eq!(extract(&sample_png()).unwrap(), None);
        let rendered = sample_png();
        for text in [
            "{broken".to_string(),
            serde_json::json!({"format":"clippy-pin-project","version":1}).to_string(),
            serde_json::json!({"format":"clippy-pin-project","formatVersion":99}).to_string(),
        ] {
            let out = png_with_project_text(&rendered, text);
            assert_eq!(extract(&out).unwrap(), None);
            crate::screenshot::validate_png(&out).unwrap();
        }
    }

    #[test]
    fn forged_source_and_dangerous_documents_are_rejected() {
        let mut bad_hash = project();
        bad_hash.source.sha256 = "0".repeat(64);
        assert!(bad_hash.validate().is_err());

        let mut bad_dimensions = project();
        bad_dimensions.source.width += 1;
        assert!(bad_dimensions.validate().is_err());

        let mut non_png_source = project();
        non_png_source.source.png_base64 = STANDARD.encode(b"not png");
        assert!(non_png_source.validate().is_err());

        let mut duplicate = project();
        duplicate.document.annotations = serde_json::json!([
            {"id":"same","type":"text","color":"#fff","size":1,"at":{"x":0,"y":0},"text":"a"},
            {"id":"same","type":"text","color":"#fff","size":1,"at":{"x":0,"y":0},"text":"b"}
        ]);
        assert!(duplicate.validate().is_err());

        let mut oversized = project();
        oversized.document.annotations = serde_json::json!([
            {"id":"t","type":"text","color":"#fff","size":1,"at":{"x":0,"y":0},"text":"x".repeat(MAX_TEXT_BYTES + 1)}
        ]);
        assert!(oversized.validate().is_err());

        let mut fake_base64 = project();
        fake_base64.source.png_base64 = "%%%".to_string();
        assert!(fake_base64.validate().is_err());

        let mut bad_effect = project();
        bad_effect.document.annotations[1]["effect"]["magnifierZoom"] = serde_json::json!(1000);
        assert!(bad_effect.validate().is_err());

        let mut missing_preview = project();
        missing_preview.preview = None;
        assert!(missing_preview.validate().is_err());

        let mut bad_preview_dimensions = project();
        bad_preview_dimensions.preview.as_mut().unwrap().width += 1;
        assert!(bad_preview_dimensions.validate().is_err());
    }

    #[test]
    fn oversized_stroke_point_array_is_rejected() {
        let points = (0..=MAX_STROKE_POINTS)
            .map(|_| serde_json::json!({"x":0,"y":0}))
            .collect::<Vec<_>>();
        let result = PinProject::new(
            &sample_png(),
            &sample_png(),
            serde_json::json!([{"id":"pen","type":"pen","color":"#fff","size":1,
                               "points":points}]),
            adjustments(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn flat_export_removes_project_metadata_and_preserves_pixels() {
        let rendered = sample_png();
        let editable = embed(&rendered, &project()).unwrap();
        let flat = flatten(&editable).unwrap();
        assert_eq!(extract(&flat).unwrap(), None);
        assert_eq!(
            image::load_from_memory_with_format(&flat, image::ImageFormat::Png)
                .unwrap()
                .into_rgba8(),
            image::load_from_memory_with_format(&rendered, image::ImageFormat::Png)
                .unwrap()
                .into_rgba8()
        );
    }

    #[test]
    fn corrupt_project_compression_does_not_block_idat_fallback() {
        let rendered = sample_png();
        let mut damaged = embed(&rendered, &project()).unwrap();
        let keyword = PROJECT_KEYWORD.as_bytes();
        let keyword_offset = damaged
            .windows(keyword.len())
            .position(|window| window == keyword)
            .expect("应有工程 keyword");
        let chunk_start = keyword_offset - 8;
        // iTXt data = keyword + NUL + compression flag + method + language NUL + translated NUL
        // + compressed bytes。破坏 zlib 头，模拟工程压缩流损坏。
        let compressed_offset = chunk_start + 8 + keyword.len() + 5;
        damaged[compressed_offset] ^= 0xff;

        assert_eq!(extract(&damaged).unwrap(), None);
        let fallback = flatten_container(&damaged).expect("损坏工程仍应能取出 IDAT");
        assert_eq!(
            crate::screenshot::png_dimensions(&fallback).unwrap(),
            (2, 1)
        );
    }
}
