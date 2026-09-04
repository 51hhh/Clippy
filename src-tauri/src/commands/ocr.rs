use super::AppState;
use std::future::Future;
use std::sync::Arc;
use tauri::State;

fn load_cached_ocr(
    storage: &Arc<std::sync::Mutex<crate::storage::StorageEngine>>,
    id: i64,
) -> Result<Option<String>, String> {
    storage
        .lock()
        .map_err(|error| error.to_string())?
        .get_ocr_text(id)
        .map_err(|error| error.to_string())
}

async fn cached_or_run<F, Fut>(cached: Option<String>, work: F) -> Result<String, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    match cached {
        Some(text) => Ok(text),
        None => work().await,
    }
}

/// 检查系统是否安装了 tesseract。
#[tauri::command]
pub fn ocr_available() -> bool {
    crate::ocr::is_available()
}

/// 对指定图片条目进行 OCR 识别，返回文字内容。
#[tauri::command]
pub async fn ocr_image(id: i64, state: State<'_, AppState>) -> Result<String, String> {
    let cached = load_cached_ocr(&state.storage, id)?;

    let storage = Arc::clone(&state.storage);
    cached_or_run(cached, move || async move {
        let image_bytes = {
            let storage = storage.lock().map_err(|e| e.to_string())?;
            storage
                .get_clip_image(id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "图片数据为空".to_string())?
        };
        let cache_storage = Arc::clone(&storage);
        crate::ocr::recognize_clip(id, image_bytes, move |text| {
            let result = cache_storage
                .lock()
                .map_err(|error| error.to_string())?
                .set_ocr_text(id, text)
                .map_err(|error| error.to_string());
            if let Err(error) = result {
                // OCR 结果本身已经成功；缓存失败不能让预览与翻译因谁先发起而得到不同结果。
                log::warn!("OCR 缓存写入失败: {error}");
            }
            Ok(())
        })
        .await
    })
    .await
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
        crate::ocr::invalidate_executable_cache();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ContentType;
    use crate::storage::StorageEngine;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn persistent_cache_hit_does_not_start_image_loading_or_ocr() {
        let engine = StorageEngine::new_in_memory().unwrap();
        let clip = engine
            .insert_clip(
                &ContentType::Image,
                None,
                None,
                Some(&[137, 80, 78, 71]),
                "ocr-cache-hit",
                4,
                false,
            )
            .unwrap();
        engine.set_ocr_text(clip.id, "cached").unwrap();
        let storage = Arc::new(std::sync::Mutex::new(engine));
        let calls = AtomicUsize::new(0);
        let cached = load_cached_ocr(&storage, clip.id).unwrap();
        let result = cached_or_run(cached, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok("new".to_string())
        })
        .await;
        assert_eq!(result.unwrap(), "cached");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

/// Windows/macOS 没有安全、统一的系统包管理入口；设置页应隐藏安装按钮，IPC 再做兜底拒绝。
#[tauri::command]
#[cfg(not(target_os = "linux"))]
pub async fn ocr_install() -> Result<String, String> {
    Err("当前平台不支持应用内安装 OCR，请通过系统软件管理方式安装 Tesseract".to_string())
}
