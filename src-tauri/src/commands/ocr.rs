use super::AppState;
use tauri::State;

/// 检查系统是否安装了 tesseract。
#[tauri::command]
pub fn ocr_available() -> bool {
    crate::ocr::is_available()
}

/// 对指定图片条目进行 OCR 识别，返回文字内容。
#[tauri::command]
pub async fn ocr_image(id: i64, state: State<'_, AppState>) -> Result<String, String> {
    {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        if let Some(cached) = storage.get_ocr_text(id).map_err(|e| e.to_string())? {
            return Ok(cached);
        }
    }

    let image_bytes = {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        storage
            .get_clip_image(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "图片数据为空".to_string())?
    };
    let text = tauri::async_runtime::spawn_blocking(move || crate::ocr::recognize(&image_bytes))
        .await
        .map_err(|e| format!("OCR 线程异常: {}", e))??;

    {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        let _ = storage.set_ocr_text(id, &text);
    }
    Ok(text)
}

/// Linux 下通过 pkexec 安装发行版提供的 Tesseract。
#[tauri::command]
#[cfg(target_os = "linux")]
pub async fn ocr_install() -> Result<String, String> {
    let output = tauri::async_runtime::spawn_blocking(|| {
        std::process::Command::new("pkexec")
            .args([
                "apt-get",
                "install",
                "-y",
                "tesseract-ocr",
                "tesseract-ocr-chi-sim",
            ])
            .output()
    })
    .await
    .map_err(|e| format!("线程异常: {}", e))?
    .map_err(|e| format!("启动 pkexec 失败: {}", e))?;

    if output.status.success() {
        Ok("ok".to_string())
    } else {
        let code = output.status.code().unwrap_or(-1);
        if code == 126 {
            Err("cancelled".to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("安装失败: {}", stderr.trim()))
        }
    }
}

/// Windows/macOS 没有安全、统一的系统包管理入口；设置页应隐藏安装按钮，IPC 再做兜底拒绝。
#[tauri::command]
#[cfg(not(target_os = "linux"))]
pub async fn ocr_install() -> Result<String, String> {
    Err("当前平台不支持应用内安装 OCR，请通过系统软件管理方式安装 Tesseract".to_string())
}
