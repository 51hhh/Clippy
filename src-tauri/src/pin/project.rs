//! 贴图工程：把"原图 + 标注"塞进导出 PNG 自己的元数据块里。
//!
//! # 为什么是单文件而不是两个文件
//!
//! 画布产物存下来之后要能**继续编辑**，而复原标注需要原图（模糊、马赛克、聚光、放大镜
//! 每次渲染都从原图重新采样，见 `src/react/annotation/canvasRenderer.ts`——没有任何工具
//! 依赖"上一步的像素结果"，所以"原图 + 操作"在信息上是无损的）。
//!
//! 存成 `x.png` + `x.json` 两个文件的话，用户拖走一个、重命名、只删一个，工程就散了；
//! 而 PNG 规范要求解码器**忽略不认识的辅助块**，所以把工程数据放进 PNG 自己的
//! iTXt 块里，对任何看图软件、任何粘贴目标都还是一张普通图片，同时 Clippy 打开它就能
//! 接着编辑。这也是 `.psd` / `.kra` 的思路：容器里同时放合成结果与图层。
//!
//! # 为什么是 iTXt 而不是 zTXt
//!
//! zTXt 是 **Latin-1**：`png` crate 的 `encode_iso_8859_1` 要求每个 char ≤ U+00FF，
//! 塞任意字节要构造畸形 `String`，是在滥用文本字段。iTXt 是 **UTF-8**，JSON 天然合规，
//! 内嵌原图走 base64（纯 ASCII）也安全；而且 iTXt 有压缩开关，读侧还有带上限的解压
//! （见 [`MAX_PROJECT_BYTES`]）。
//!
//! # 边界
//!
//! 工程块是**用户可控输入**——任何人都能构造一个 PNG 塞进恶意 iTXt。所以读的那一侧
//! 每一步都要能失败而不炸：解压有上限、内嵌原图仍走整张解码校验、版本不认识就当普通
//! PNG 处理。**读不出工程只意味着"这张图不能继续编辑"，绝不能让打开图片本身失败。**

use serde::{Deserialize, Serialize};

/// 工程块的 iTXt keyword。PNG 规范限制 keyword 为 1~79 字节的 Latin-1。
const PROJECT_KEYWORD: &str = "clippy-project";

/// `format` 字段的固定值。用来在同名 keyword 撞车时确认这真是我们的数据。
const PROJECT_FORMAT: &str = "clippy-pin-project";

/// 当前工程格式版本。
///
/// **读到更大的版本号就当普通 PNG 处理**，不要猜着解析：工具语义会变（箭头画法、
/// 马赛克格子大小），猜出来的复原会让用户看到一张和当初不一样的图，那比"不能编辑"更糟。
const PROJECT_VERSION: u32 = 1;

/// 解压后的工程 JSON 上限。
///
/// 和 `capture::MAX_COMMIT_PNG_BYTES` 同一个数量级、同一个理由：工程块含 base64 的原图
/// （1440p 截图约 1.6 MB），64 MiB 足够宽松，同时挡住"一个几 KB 的 iTXt 解压成几 GB"
/// 这种压缩炸弹。`png` crate 的 `decompress_text_with_limit` 正好接这个数。
const MAX_PROJECT_BYTES: usize = 64 * 1024 * 1024;

/// 贴图工程。字段与前端 `annotation/types.ts` 的 `EditorDocument` 对应。
///
/// **后端不解释 `annotations` 与 `adjustments`**：它们是前端渲染器的数据结构，
/// 后端只负责原样搬运（`serde_json::Value`）。把它们在 Rust 里再定义一遍等于把
/// 16 个工具的形状抄第二份，改一个工具要改两处，而后端一行都不会用到它们。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PinProject {
    /// 固定为 [`PROJECT_FORMAT`]。
    pub format: String,
    pub version: u32,
    /// 落盘时刻（Unix 秒）。排障用：能对上是哪一版应用写的。
    pub created_at: i64,
    /// 写这个工程的应用版本。
    pub app_version: String,
    /// 底图：**条目原图**的 base64 PNG，不是屏上那张补偿版（见 `get_pin_source_image`）。
    pub source_png_base64: String,
    /// 标注数组，原样搬运前端的 `Annotation[]`。
    pub annotations: serde_json::Value,
    /// 图像调整，原样搬运前端的 `ImageAdjustments`。
    pub adjustments: serde_json::Value,
}

impl PinProject {
    /// 组装一份当前版本的工程。
    pub fn new(
        source_png_base64: String,
        annotations: serde_json::Value,
        adjustments: serde_json::Value,
    ) -> Self {
        Self {
            format: PROJECT_FORMAT.to_string(),
            version: PROJECT_VERSION,
            created_at: chrono::Local::now().timestamp(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            source_png_base64,
            annotations,
            adjustments,
        }
    }

    /// 这份工程是当前代码能理解的吗？
    fn is_supported(&self) -> bool {
        self.format == PROJECT_FORMAT && self.version <= PROJECT_VERSION
    }
}

/// 把工程数据写进一张已编码好的 PNG，返回新的 PNG 字节。
///
/// **重新编码一遍，而不是往字节流里插块。** 手工插块要自己算 CRC、找 IEND 的位置、
/// 处理可能存在的其它辅助块——那是在重写一个 PNG 写入器。走 `png` crate 的编码器则由
/// 它保证结构合法；代价是多一次编解码（1440p 约 36 + 77 ms），而这条路只在
/// "用户点保存"时走一次。
pub fn embed(png: &[u8], project: &PinProject) -> Result<Vec<u8>, String> {
    let json =
        serde_json::to_string(project).map_err(|error| format!("工程序列化失败: {error}"))?;
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|error| format!("PNG 解码失败: {error}"))?
        .to_rgba8();
    let (width, height) = image.dimensions();

    let mut out = Vec::with_capacity(png.len() + json.len());
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // 和 `screenshot::encode_png` 一致：这条路上用户在等着，压缩比不如速度重要。
        encoder.set_compression(png::Compression::Fast);
        // iTXt 而不是 zTXt：JSON 是 UTF-8，见模块头。
        encoder
            .add_itxt_chunk(PROJECT_KEYWORD.to_string(), json)
            .map_err(|error| format!("写入工程数据失败: {error}"))?;
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("PNG 头写入失败: {error}"))?;
        writer
            .write_image_data(image.as_raw())
            .map_err(|error| format!("PNG 数据写入失败: {error}"))?;
    }
    Ok(out)
}

/// 从一张 PNG 里读回工程数据。
///
/// 返回 `Ok(None)` 表示"这是张普通 PNG"——没有工程块、块坏了、版本不认识，
/// 三种情况对用户都是同一件事：能看，不能继续编辑。所以不区分，也**不返回 Err**：
/// 这条路的调用方是"打开一张图片"，它不该因为元数据有问题而失败。
///
/// 真正的 `Err` 只留给"连 PNG 都不是"。
pub fn extract(png: &[u8]) -> Result<Option<PinProject>, String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let reader = decoder
        .read_info()
        .map_err(|error| format!("PNG 解析失败: {error}"))?;
    let info = reader.info();

    for chunk in &info.utf8_text {
        if chunk.keyword != PROJECT_KEYWORD {
            continue;
        }
        // 带上限解压：工程块是用户可控输入，不能让一个几 KB 的块解压成几 GB。
        let mut chunk = chunk.clone();
        if chunk.decompress_text_with_limit(MAX_PROJECT_BYTES).is_err() {
            log::info!("贴图工程数据超过 {MAX_PROJECT_BYTES} 字节上限或解压失败，按普通图片处理");
            return Ok(None);
        }
        let Ok(text) = chunk.get_text() else {
            log::info!("贴图工程数据不是合法文本，按普通图片处理");
            return Ok(None);
        };
        let Ok(project) = serde_json::from_str::<PinProject>(&text) else {
            log::info!("贴图工程数据无法解析，按普通图片处理");
            return Ok(None);
        };
        if !project.is_supported() {
            log::info!(
                "贴图工程版本 {} 高于当前支持的 {PROJECT_VERSION}（或格式不符），按普通图片处理",
                project.version
            );
            return Ok(None);
        }
        return Ok(Some(project));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_png() -> Vec<u8> {
        // 4x3 渐变，够小又不是纯色（纯色会被 PNG 过滤器压到极小，掩盖体积问题）。
        let mut image = image::RgbaImage::new(4, 3);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgba([(x * 60) as u8, (y * 80) as u8, 40, 255]);
        }
        crate::screenshot::encode_png(image.as_raw(), 4, 3).unwrap()
    }

    fn sample_project() -> PinProject {
        PinProject::new(
            "c291cmNl".to_string(),
            serde_json::json!([{ "id": "pen-1", "type": "pen", "color": "#ff0000", "size": 4,
                                 "points": [{ "x": 1.0, "y": 2.0 }, { "x": 3.0, "y": 4.0 }] }]),
            serde_json::json!({ "grayscale": false, "brightness": 0, "contrast": 0,
                                "saturation": 0, "cornerRadius": 0 }),
        )
    }

    /// 往返：写进去能原样读回来，而且图像本身仍然是合法 PNG、尺寸不变。
    #[test]
    fn a_project_survives_a_round_trip() {
        let png = sample_png();
        let project = sample_project();
        let embedded = embed(&png, &project).expect("写入工程");

        assert_eq!(
            crate::screenshot::png_dimensions(&embedded).expect("读头"),
            (4, 3)
        );
        // 关键：像素照旧能解出来——工程块绝不能影响图像本身。
        crate::screenshot::validate_png(&embedded).expect("图像仍然合法");

        let read = extract(&embedded).expect("解析").expect("应有工程");
        assert_eq!(read, project);
        assert_eq!(read.source_png_base64, "c291cmNl");
        assert_eq!(read.annotations[0]["type"], "pen");
    }

    /// 普通 PNG（没有工程块）读出来是 `None`，不是错误。
    #[test]
    fn a_plain_png_has_no_project() {
        assert_eq!(extract(&sample_png()).expect("解析"), None);
    }

    /// 连 PNG 都不是才返回 `Err`。
    #[test]
    fn garbage_is_not_a_png() {
        assert!(extract(b"not a png at all").is_err());
        assert!(embed(b"not a png at all", &sample_project()).is_err());
    }

    /// **版本比当前新就当普通图片**，不要猜着解析：工具语义会变，猜出来的复原会让
    /// 用户看到一张和当初不一样的图。
    #[test]
    fn a_newer_version_is_ignored_rather_than_guessed() {
        let png = sample_png();
        let mut future = sample_project();
        future.version = PROJECT_VERSION + 1;
        let embedded = embed(&png, &future).expect("写入");
        assert_eq!(extract(&embedded).expect("解析"), None);
    }

    /// keyword 撞车时靠 `format` 认自己的数据；别人的同名块要被忽略。
    #[test]
    fn a_foreign_chunk_with_the_same_keyword_is_ignored() {
        let png = sample_png();
        let mut alien = sample_project();
        alien.format = "someone-elses-format".to_string();
        let embedded = embed(&png, &alien).expect("写入");
        assert_eq!(extract(&embedded).expect("解析"), None);
    }

    /// 块里是垃圾文本时按普通图片处理，而且**图像照旧能打开**——
    /// 这是"读不出工程绝不让打开图片失败"那条约定的回归测试。
    #[test]
    fn a_corrupt_project_still_leaves_a_usable_image() {
        let png = sample_png();
        let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 4, 3);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .add_itxt_chunk(PROJECT_KEYWORD.to_string(), "{ not json".to_string())
                .unwrap();
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(image.as_raw()).unwrap();
        }
        assert_eq!(extract(&out).expect("解析"), None);
        crate::screenshot::validate_png(&out).expect("图像仍然可用");
    }
}
