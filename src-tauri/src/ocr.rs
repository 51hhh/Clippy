//! OCR facade：组合增强管线、Tesseract 协议和受限运行时。
//! 外部调用只通过本模块进入，子模块分别拥有探测、进程、调度和解析职责。

use std::sync::Arc;

mod enhanced;
mod executable;
mod process;
mod protocol;
mod runtime;
mod tesseract;

pub(crate) use enhanced::OcrHealthStatus;
pub use protocol::StructuredOcr;
use runtime::ocr_runtime;

pub(super) type OcrResult = Result<String, String>;
pub(super) type StructuredResult = Result<StructuredOcr, String>;

/// 查看器等不可变图像快照的结构化 OCR；与旧字符串入口共用同一全局预算。
pub async fn recognize_snapshot(png: Vec<u8>) -> StructuredResult {
    recognize_snapshot_shared(Arc::new(png)).await
}

/// 重复查看器请求共享冻结 PNG；排队/配置阶段不复制整张图片。
pub async fn recognize_snapshot_shared(png: Arc<Vec<u8>>) -> StructuredResult {
    enhanced::image_dimensions(&png)?;
    let preparation = runtime::prepare_permit()?;
    let input = Arc::clone(&png);
    let (configuration, key) = tauri::async_runtime::spawn_blocking(move || {
        // 调用者取消时 blocking 工作仍持有小准备许可，不产生无限后台配置探测。
        let _preparation = preparation;
        let configuration = enhanced::configuration();
        let key = enhanced::request_key(&input, &configuration);
        (configuration, key)
    })
    .await
    .map_err(|_| "OCR 配置读取失败".to_string())?;
    let runtime = ocr_runtime();
    let cancellation_key = key.clone();
    runtime
        .run_structured(key, move || {
            enhanced::recognize_owned(png, configuration, move || {
                !runtime.has_consumers(&cancellation_key)
            })
        })
        .await
}

async fn prepare_configuration() -> Result<enhanced::Configuration, String> {
    let permit = runtime::prepare_permit()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        enhanced::configuration()
    })
    .await
    .map_err(|_| "OCR 配置读取失败".to_string())
}

/// 旧的无版本 String 缓存无法证明来自当前增强模型，配置增强时必须绕过。
pub(crate) fn uses_enhanced_configuration() -> bool {
    enhanced::configured()
}

/// 设置值变更后立即切换识别配置；空值仍允许开发环境变量显式兜底。
pub(crate) fn set_manifest_setting(value: &str) {
    enhanced::set_manifest_setting(value);
}

pub(crate) fn health_status(candidate: &str) -> OcrHealthStatus {
    enhanced::health_status(candidate)
}

/// 安装流程改变了外部工具状态，下一次查询必须重新探测。
#[cfg(target_os = "linux")]
pub(crate) fn invalidate_executable_cache() {
    executable::invalidate_executable_cache();
}

/// 检查增强管线或系统 Tesseract 是否可用。
pub fn is_available() -> bool {
    enhanced::configuration().is_enhanced() || executable::tesseract_executable().is_some()
}

/// 让运行中的进程拥有许可直到 kill/wait 完成，即使唯一调用者取消等待。
pub(crate) async fn recognize_image(png_bytes: Vec<u8>) -> OcrResult {
    recognize_snapshot(png_bytes)
        .await
        .map(|result| result.text)
}

/// 兼容已经持有冻结图像的调用；队列入口限制持有图片的任务总数。
pub(crate) async fn recognize_clip<F>(id: i64, png_bytes: Vec<u8>, cache: F) -> OcrResult
where
    F: FnOnce(&str) -> Result<(), String> + Send + 'static,
{
    recognize_clip_lazy(id, move || Ok(png_bytes), cache).await
}

/// 预览先加入 single-flight/排队，取得许可后才加载 PNG，等待者不持有 BLOB。
pub(crate) async fn recognize_clip_lazy<L, F>(id: i64, load: L, cache: F) -> OcrResult
where
    L: FnOnce() -> Result<Vec<u8>, String> + Send + 'static,
    F: FnOnce(&str) -> Result<(), String> + Send + 'static,
{
    recognize_clip_result_lazy(id, load, cache)
        .await
        .map(|result| result.text)
}

/// 结构化侧栏入口同样先入队再读数据库，所有 String 调用适配同一结果。
pub(crate) async fn recognize_clip_result_lazy<L, F>(id: i64, load: L, cache: F) -> StructuredResult
where
    L: FnOnce() -> Result<Vec<u8>, String> + Send + 'static,
    F: FnOnce(&str) -> Result<(), String> + Send + 'static,
{
    let configuration = prepare_configuration().await?;
    let runtime = ocr_runtime();
    let key = format!("clip:{id}:{}", configuration.identity);
    let cancellation_key = key.clone();
    runtime
        .run_structured(key, move || async move {
            let png = tauri::async_runtime::spawn_blocking(load)
                .await
                .map_err(|error| format!("OCR 图片读取线程异常: {error}"))??;
            let result = enhanced::recognize_owned(Arc::new(png), configuration, move || {
                !runtime.has_consumers(&cancellation_key)
            })
            .await?;
            cache(&result.text)?;
            Ok(result)
        })
        .await
}

#[cfg(all(test, unix))]
mod process_tests;
