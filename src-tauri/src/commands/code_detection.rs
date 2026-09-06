use super::AppState;
use crate::code_detection::{CodeScanError, CodeScanResponse, MAX_PNG_BYTES};
use crate::storage::{BoundedImageData, StorageEngine};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::State;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

fn scan_gate() -> &'static Arc<Semaphore> {
    static GATE: OnceLock<Arc<Semaphore>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(Semaphore::new(1)))
}

fn acquire_scan_permit() -> Result<OwnedSemaphorePermit, CodeScanError> {
    Arc::clone(scan_gate())
        .try_acquire_owned()
        .map_err(|_| CodeScanError::Busy)
}

fn load_image_once_with_limit(
    storage: &Arc<Mutex<StorageEngine>>,
    id: i64,
    byte_limit: usize,
) -> Result<Vec<u8>, CodeScanError> {
    let image = {
        let storage = storage.lock().map_err(|_| CodeScanError::StorageFailed)?;
        storage
            .get_bounded_image_for_code_scan(id, byte_limit)
            .map_err(|_| CodeScanError::StorageFailed)?
    };
    match image {
        BoundedImageData::ClipNotFound => Err(CodeScanError::ClipNotFound),
        BoundedImageData::NotImage => Err(CodeScanError::NotImage),
        BoundedImageData::Missing => Err(CodeScanError::ImageMissing),
        BoundedImageData::TooLarge => Err(CodeScanError::ImageTooLarge),
        BoundedImageData::Bytes(bytes) => Ok(bytes),
    }
}

fn load_image_once(storage: &Arc<Mutex<StorageEngine>>, id: i64) -> Result<Vec<u8>, CodeScanError> {
    load_image_once_with_limit(storage, id, MAX_PNG_BYTES)
}

/// 识别一条图片剪贴历史中的本地二维码/条码；不联网、不缓存结果。
#[tauri::command]
pub async fn detect_image_codes(
    id: i64,
    state: State<'_, AppState>,
) -> Result<CodeScanResponse, CodeScanError> {
    let permit = acquire_scan_permit()?;
    let storage = Arc::clone(&state.storage);
    let png_bytes = load_image_once(&storage, id)?;
    tauri::async_runtime::spawn_blocking(move || {
        // permit 必须跨完整 PNG 解码和条码扫描生命周期持有。
        let _permit = permit;
        crate::code_detection::scan_png(png_bytes)
    })
    .await
    .map_err(|_| CodeScanError::WorkerFailed)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ContentType;

    fn storage_with(
        content_type: ContentType,
        image_data: Option<&[u8]>,
    ) -> (Arc<Mutex<StorageEngine>>, i64) {
        let engine = StorageEngine::new_in_memory().expect("内存存储");
        let clip = engine
            .insert_clip(
                &content_type,
                None,
                None,
                image_data,
                "code-detection-test",
                image_data.map_or(0, |value| value.len() as i64),
                false,
            )
            .expect("插入测试条目");
        (Arc::new(Mutex::new(engine)), clip.id)
    }

    #[test]
    fn image_load_distinguishes_missing_clip_non_image_and_missing_bytes() {
        let (storage, id) = storage_with(ContentType::Text, None);
        assert_eq!(load_image_once(&storage, id), Err(CodeScanError::NotImage));
        assert_eq!(
            load_image_once(&storage, id + 999),
            Err(CodeScanError::ClipNotFound)
        );

        let (storage, id) = storage_with(ContentType::Image, None);
        assert_eq!(
            load_image_once(&storage, id),
            Err(CodeScanError::ImageMissing)
        );

        let (storage, id) = storage_with(ContentType::Image, Some(&[1, 2, 3]));
        assert_eq!(
            load_image_once_with_limit(&storage, id, 2),
            Err(CodeScanError::ImageTooLarge)
        );
    }

    #[test]
    fn only_one_owned_scan_permit_can_be_held() {
        let permit = acquire_scan_permit().expect("首个 permit");
        assert!(matches!(acquire_scan_permit(), Err(CodeScanError::Busy)));
        drop(permit);
        assert!(acquire_scan_permit().is_ok());
    }
}
