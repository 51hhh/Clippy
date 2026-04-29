//! ocr.rs — Tesseract OCR 封装
//! 使用 leptess 对图片进行文字识别，支持中英文。

use leptess::LepTess;
use std::sync::Mutex;

/// 全局 Tesseract 实例（初始化较重，复用）
static OCR_ENGINE: Mutex<Option<LepTess>> = Mutex::new(None);

/// 对 PNG 图片字节进行 OCR 识别，返回文字内容
pub fn recognize(png_bytes: &[u8]) -> Result<String, String> {
    let mut guard = OCR_ENGINE.lock().map_err(|e| e.to_string())?;

    let lt = match guard.as_mut() {
        Some(lt) => lt,
        None => {
            // 初始化 Tesseract，使用系统 tessdata
            let engine = LepTess::new(None, "eng+chi_sim")
                .map_err(|e| format!("Tesseract 初始化失败: {}", e))?;
            *guard = Some(engine);
            guard.as_mut().unwrap()
        }
    };

    // 从内存加载图片
    lt.set_image_from_mem(png_bytes)
        .map_err(|e| format!("图片加载失败: {}", e))?;

    lt.get_utf8_text()
        .map(|t| t.trim().to_string())
        .map_err(|e| format!("OCR 识别失败: {}", e))
}
