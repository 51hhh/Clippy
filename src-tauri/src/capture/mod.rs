mod action_lifecycle;
mod error;
mod manager;
mod overlay_windows;
mod shell_extension;
mod types;
mod window_probe;

pub use error::CaptureError;
pub use manager::CaptureManager;
/// 贴图窗口的摆放与置顶也只有这个扩展做得到（Wayland 不许客户端自己来），
/// 所以 `pin/` 借道这里，而不是自己再开一份 D-Bus 契约。
pub(crate) use shell_extension::place_window as shell_extension_place_window;
/// 截图后端要用扩展这条路取冻结帧。扩展的全部 IPC 都留在 `shell_extension` 里，
/// 这里只把入口露出去，免得契约散成两份。
pub(crate) use shell_extension::request_screenshot as shell_extension_screenshot;
pub use shell_extension::{InstallOutcome, ShellExtensionStatus};
pub use types::{
    CaptureAction, CaptureActionResult, CaptureOverlayPayload, CaptureSelection,
    CaptureTranslationResult,
};

use crate::commands::AppState;
use crate::translation::types::TranslationError;
use tauri::State;

/// 覆盖层提交回来的 PNG 上限。全屏 4K 的标注 PNG 大约十几 MB，
/// 64 MiB 足够宽松，同时挡住畸形/恶意载荷把内存吃光。
const MAX_COMMIT_PNG_BYTES: usize = 64 * 1024 * 1024;

/// 隐藏自己的窗口之后等合成器真的把它们从屏幕上撤掉的时间。
///
/// 少了这一等会把 Clippy 自己的面板烧进冻结帧。但它**只在真的藏了窗口时才需要**：
/// 快捷键截图的常态是面板本来就没开着（`hide_sources` 返回空），这时白等 140 ms
/// 纯粹是加在用户感知延迟上的。所以按需等待，不要改回无条件 sleep。
const HIDE_SETTLE_MS: u64 = 140;

/// 启动自检：扩展内容过期就静默升级，目录被手工删掉就清理 gsettings 里的孤儿条目。
pub(crate) fn reconcile_window_probe_extension() {
    shell_extension::reconcile_on_startup();
}

pub(crate) fn handle_overlay_destroyed(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    label: &str,
) {
    if let Some(session) = state.capture_manager.abort_if_overlay(label) {
        overlay_windows::close(app_handle, &session.overlay_labels());
        overlay_windows::restore(app_handle, &session.restore_labels);
    }
}

#[tauri::command]
pub async fn show_capture_overlay(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    show_capture_overlay_for_app(app_handle, &state).await
}

/// 本次截图要不要提示"窗口速选需要安装服务"，并且当场把"已提示过"落盘。
///
/// 提示只出现一次：没有这个服务照样能自由框选，反复提示纯属打扰。取一次就消耗掉，
/// 所以叫 take。任何环节出错都返回 false——提示是可选的锦上添花，绝不能因此挡住截图。
fn take_probe_hint(state: &AppState) -> bool {
    if !shell_extension::hint_needed() {
        return false;
    }
    let Ok(mut config) = state.config.lock() else {
        return false;
    };
    if config.capture_probe_hint_shown {
        return false;
    }
    config.capture_probe_hint_shown = true;
    crate::config::save_config(&state.config_path, &config);
    true
}

pub(crate) async fn show_capture_overlay_for_app(
    app_handle: tauri::AppHandle,
    state: &AppState,
) -> Result<(), String> {
    let mut timings = manager::StageTimings::start();
    let restore_labels = overlay_windows::hide_sources(&app_handle);
    let settle = if restore_labels.is_empty() {
        0
    } else {
        HIDE_SETTLE_MS
    };
    let frames = match tauri::async_runtime::spawn_blocking(move || {
        if settle > 0 {
            std::thread::sleep(std::time::Duration::from_millis(settle));
        }
        crate::screenshot::capture_monitor_frames()
    })
    .await
    {
        Ok(Ok(frames)) => frames,
        Ok(Err(error)) => {
            overlay_windows::restore(&app_handle, &restore_labels);
            return Err(capture_failure(CaptureError::Screenshot(error.to_string())));
        }
        Err(error) => {
            overlay_windows::restore(&app_handle, &restore_labels);
            return Err(capture_failure(CaptureError::ThreadPanic(
                error.to_string(),
            )));
        }
    };
    timings.frames_ms = timings.started.elapsed().as_secs_f64() * 1000.0;
    let probe_hint = take_probe_hint(state);
    let specs =
        match state
            .capture_manager
            .begin(frames, restore_labels.clone(), probe_hint, timings)
        {
            Ok(specs) => specs,
            Err(error) => {
                overlay_windows::restore(&app_handle, &restore_labels);
                return Err(capture_failure(error));
            }
        };
    if let Err(error) = overlay_windows::create(&app_handle, &specs) {
        if let Some(session) = state.capture_manager.abort() {
            overlay_windows::close(&app_handle, &session.overlay_labels());
        }
        overlay_windows::restore(&app_handle, &restore_labels);
        return Err(capture_failure(error));
    }
    Ok(())
}

#[tauri::command]
pub fn get_capture_overlay(
    label: String,
    state: State<'_, AppState>,
) -> Result<CaptureOverlayPayload, String> {
    validate_overlay_label(&label)?;
    Ok(state.capture_manager.payload(&label)?)
}

/// 冻结帧像素：原始 RGBA，以二进制 IPC（`ArrayBuffer`）送进覆盖层。
///
/// 走 `tauri::ipc::Response` 而不是 payload 里的字符串字段，是因为像素一旦进 JSON 就得
/// PNG + base64 两次编码、webview 那头再 atob + 解码两次；2560×1600 实测这四段占掉了
/// 覆盖层出现前的一半时间。原始 RGBA 反而更省：后端**零编码**，前端一次 `putImageData`
/// 就是底图。字节序约定 RGBA8，行优先、无 padding，尺寸取 payload 的
/// `pixelWidth`/`pixelHeight`。
///
/// 零编码不等于零拷贝：`InvokeResponseBody::Raw` 要的是 `Vec<u8>`，所以这里还有一次
/// 16 MB 的 memcpy（实测 2 ms 上下，与那 215 ms 的 PNG 编码不是一个量级）。帧还留在
/// 会话里给选区翻译用，因此不能把 `Arc` 里的缓冲区直接交出去。
#[tauri::command]
pub fn get_capture_frame(
    label: String,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    validate_overlay_label(&label)?;
    let rgba = state.capture_manager.frame_rgba(&label)?;
    Ok(tauri::ipc::Response::new(rgba.to_vec()))
}

/// 覆盖层画完首帧后调用，后端这才把窗口显示出来。
///
/// 之前是建窗就 `show()`，于是 webview 加载 + 取 payload + 铺底图的整段时间里
/// 用户盯着一整屏白色（webview 默认底色），画面才姗姗出现。现在窗口先隐藏，
/// 由前端决定显示时机；兜底定时器见 `overlay_windows::READY_FALLBACK_MS`。
///
/// 顺带捎上前端实测的可见视口，闭合不变量 I4（见 `manager::viewport_mismatch`）：
/// 这是唯一一条能看到"合成器最终摆成什么样"的自检，而这里恰好是它天然的时机——
/// 首帧画完意味着窗口已经布局完成。为它单开一个 IPC 命令纯属多余。
#[tauri::command]
pub fn mark_capture_overlay_ready(
    label: String,
    viewport_width: Option<u32>,
    viewport_height: Option<u32>,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_overlay_label(&label)?;
    let cursor = app_handle
        .cursor_position()
        .ok()
        .map(|cursor| (cursor.x, cursor.y));
    let viewport = viewport_width.zip(viewport_height);
    let plan = state.capture_manager.reveal(&label, cursor, viewport)?;
    overlay_windows::reveal(&app_handle, &label, plan.take_focus)?;
    Ok(())
}

#[tauri::command]
pub fn cancel_capture_overlay(
    session_id: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let session = state.capture_manager.finish(&session_id)?;
    overlay_windows::close(&app_handle, &session.overlay_labels());
    overlay_windows::restore(&app_handle, &session.restore_labels);
    Ok(())
}

/// 窗口速选所需的 GNOME Shell 扩展的服务状态。
#[tauri::command]
pub fn get_window_probe_status() -> ShellExtensionStatus {
    shell_extension::status()
}

/// 安装并尽力当场启用扩展。`needsLogout` 为真时前端要提示"注销一次后生效"。
///
/// 装 GNOME Shell 扩展是很打扰的动作，只能由用户在设置页显式点击触发，
/// 应用绝不擅自安装。
#[tauri::command]
pub fn install_window_probe_extension() -> Result<InstallOutcome, String> {
    shell_extension::install()
}

#[tauri::command]
pub fn uninstall_window_probe_extension() -> Result<ShellExtensionStatus, String> {
    shell_extension::uninstall()
}

/// 覆盖层内完成裁剪与标注之后的唯一提交入口。
///
/// PNG 由覆盖层的画布渲染（`renderExport` 已经把裁剪、图像调整和矢量标注合成进去），
/// 后端不再自己裁一遍——否则标注会被丢掉。会话在执行动作前先被认领，
/// 并发取消或连点两次都不会产生两份副作用。
///
/// `origin` 是选区在桌面逻辑坐标系里的矩形（覆盖层用 payload 的 `logicalX/logicalY`
/// 换算好）。贴图靠它回到原处、按原尺寸显示；复制时也记一份，方便之后从历史里
/// 把同一张图 Pin 回原位。
#[tauri::command]
pub fn commit_capture_action(
    action: CaptureAction,
    session_id: String,
    png_base64: String,
    origin: Option<crate::pin::PinOrigin>,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<CaptureActionResult, String> {
    let png = decode_commit_png(&png_base64);
    let result = action_lifecycle::complete_capture_action(
        png,
        || state.capture_manager.finish(&session_id),
        |session| overlay_windows::close(&app_handle, &session.overlay_labels()),
        |session| overlay_windows::restore(&app_handle, &session.restore_labels),
        |png| execute_action(action, png, origin, &app_handle, &state),
    );
    // 覆盖层在动作之前就关掉了，错误已经没有窗口可以显示——只能留在日志里，
    // 否则"点了对钩什么都没发生"完全无从排查。
    if let Err(error) = &result {
        log::warn!("截图提交失败: {error}");
    }
    result
}

/// 解码并校验覆盖层提交的 PNG。先看 base64 长度再解码，避免为一个畸形载荷先分配几百 MB。
fn decode_commit_png(png_base64: &str) -> Result<Vec<u8>, CaptureError> {
    // base64 每 4 个字符出 3 字节，用这个上界提前拒绝。
    if png_base64.len() / 4 * 3 > MAX_COMMIT_PNG_BYTES {
        return Err(CaptureError::CommitPayloadTooLarge);
    }
    let png = crate::screenshot::decode_png_base64(png_base64)
        .map_err(|_| CaptureError::CommitPayloadInvalid)?;
    if png.len() > MAX_COMMIT_PNG_BYTES {
        return Err(CaptureError::CommitPayloadTooLarge);
    }
    // 必须真的能解成图像：后续 copy/save/pin 都假设手里是合法 PNG。
    crate::screenshot::png_dimensions(&png).map_err(|_| CaptureError::CommitPayloadInvalid)?;
    Ok(png)
}

/// 显式执行“选区 -> 本地 OCR -> 文本翻译”。裁剪帧只进入 Tesseract，永不发送给 provider。
/// 此命令不会结束 CaptureSession，用户可继续复制、保存、Pin 或编辑同一选区。
#[tauri::command]
pub async fn translate_capture_selection(
    selection: CaptureSelection,
    source_language: Option<String>,
    target_language: Option<String>,
    request_id: Option<u64>,
    state: State<'_, AppState>,
) -> Result<CaptureTranslationResult, String> {
    let request_id =
        crate::translation::commands::reserve_request_id(&state.translation, request_id);
    let capture_manager = state.capture_manager.clone();
    let png = run_translation_blocking(move || {
        capture_manager
            .crop(&selection)
            .map_err(|_| TranslationError::CaptureUnavailable)
    })
    .await
    .map_err(translation_ipc_error)?;

    let source_text = run_translation_blocking(move || recognize_capture_text(&png))
        .await
        .map_err(translation_ipc_error)?;
    let translated = crate::translation::commands::translate_configured_text(
        state.translation.clone(),
        state.config.clone(),
        source_text.clone(),
        source_language,
        target_language,
        request_id,
    )
    .await
    .map_err(translation_ipc_error)?;

    // 选区译文不属于任何剪贴板条目，历史里以 clip_id = 0 记录。
    crate::translation::commands::record_translations(
        state.storage.clone(),
        None,
        source_text.clone(),
        vec![crate::translation::types::ServiceTranslation::from_result(
            translated.provider,
            Ok(translated.clone()),
        )],
    )
    .await;

    Ok(CaptureTranslationResult::from_translation(
        source_text,
        translated,
    ))
}

async fn run_translation_blocking<T, F>(task: F) -> Result<T, TranslationError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, TranslationError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| TranslationError::Internal)?
}

fn recognize_capture_text(png: &[u8]) -> Result<String, TranslationError> {
    normalize_ocr_text(crate::ocr::recognize(png))
}

fn normalize_ocr_text(result: Result<String, String>) -> Result<String, TranslationError> {
    let text = result.map_err(|_| TranslationError::OcrFailed)?;
    if text.trim().is_empty() {
        Err(TranslationError::OcrFailed)
    } else {
        Ok(text)
    }
}

/// 选区翻译与文本翻译共用同一个错误出口，保证日志标识和对外文案不分叉。
fn translation_ipc_error(error: TranslationError) -> String {
    crate::translation::commands::ipc_error(error)
}

fn execute_action(
    action: CaptureAction,
    png: Vec<u8>,
    origin: Option<crate::pin::PinOrigin>,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<CaptureActionResult, String> {
    match action {
        CaptureAction::Copy => {
            // 解码一次，剪贴板和来源登记共用同一份像素：两边各解一遍这张全屏 PNG
            // 要多花几十毫秒，而用户此刻正等着对钩生效。
            let image = crate::image_io::png_to_clipboard_image(&png)?;
            let fingerprint = origin.map(|origin| {
                (
                    crate::pin::PinFingerprint::of(
                        image.width as u32,
                        image.height as u32,
                        &image.bytes,
                    ),
                    origin,
                )
            });
            crate::clipboard_watcher::clipboard_set_image_with_retry(image)?;
            // 复制成功之后再登记：复制失败的话这张图根本没进剪贴板，
            // 记下来只会让将来某张碰巧一样的图错位。
            if let Some((fingerprint, origin)) = fingerprint {
                state.pin_origins.remember(fingerprint, origin);
            }
            Ok(action_result("copy", None, None))
        }
        CaptureAction::Save => {
            let path = crate::image_io::save_png(&png, "clippy-screenshot", &state.save_target())?;
            Ok(action_result(
                "save",
                Some(path.to_string_lossy().to_string()),
                None,
            ))
        }
        CaptureAction::Pin => {
            let label = crate::pin::create_screenshot_pin(png, origin, app_handle, state)?;
            Ok(action_result("pin", None, Some(label)))
        }
    }
}

fn action_result(
    action: &'static str,
    path: Option<String>,
    pin_label: Option<String>,
) -> CaptureActionResult {
    CaptureActionResult {
        action,
        path,
        pin_label,
    }
}

/// 覆盖层无法建立时源窗口已经恢复，用户只会看到"截图没打开"，
/// 所以失败原因必须留在日志里才能排障。
fn capture_failure(error: CaptureError) -> String {
    let context = "打开截图覆盖层失败";
    // 快捷键连按会撞上仍在进行的会话，这是正常竞争而不是故障。
    if matches!(error, CaptureError::SessionBusy) {
        crate::error::note(context, error)
    } else {
        crate::error::report(context, error)
    }
}

fn validate_overlay_label(label: &str) -> Result<(), CaptureError> {
    if label.starts_with("capture-overlay-")
        && label.len() <= 128
        && label
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        Ok(())
    } else {
        Err(CaptureError::OverlayLabelInvalid)
    }
}

/// 真机截图耗时诊断。默认 `#[ignore]`，要有真实桌面会话才有意义：
/// `cargo test --lib capture_stage_timings -- --ignored --nocapture`
///
/// "按下快捷键到覆盖层出现要好几秒"这类问题只能靠逐段计时定位——每一环都返回 Ok，
/// 只是加起来太久。这里按真实顺序把每段单独跑一遍并打印毫秒数，不要删。
#[cfg(all(test, target_os = "linux"))]
mod timing_diagnostics {
    use std::time::Instant;

    fn ms(at: Instant) -> f64 {
        at.elapsed().as_secs_f64() * 1000.0
    }

    #[test]
    #[ignore = "需要真实桌面会话"]
    fn capture_stage_timings() {
        let at = Instant::now();
        let shot = super::shell_extension::request_screenshot();
        println!("扩展 Screenshot D-Bus 往返: {:.1} ms", ms(at));
        match &shot {
            Ok(path) => {
                let at = Instant::now();
                let bytes = std::fs::read(path).unwrap_or_default();
                println!("读文件 {} 字节: {:.1} ms", bytes.len(), ms(at));
                let at = Instant::now();
                let decoded = image::load_from_memory(&bytes).map(|image| image.to_rgba8());
                println!(
                    "PNG 解码 {:?}: {:.1} ms",
                    decoded.as_ref().map(|image| image.dimensions()).ok(),
                    ms(at)
                );
                let _ = std::fs::remove_file(path);
            }
            Err(error) => println!("扩展截图不可用: {error}"),
        }

        let at = Instant::now();
        let frames = crate::screenshot::capture_monitor_frames();
        println!("capture_monitor_frames 全程: {:.1} ms", ms(at));
        let Ok(frames) = frames else {
            println!("拿不到冻结帧，后面几段跳过");
            return;
        };

        let at = Instant::now();
        let windows = super::shell_extension::probe();
        println!(
            "扩展 GetWindows: {:.1} ms（{} 个窗口）",
            ms(at),
            windows.map(|list| list.len()).unwrap_or(0)
        );
        let at = Instant::now();
        let candidates = super::window_probe::probe_windows(&frames);
        println!(
            "probe_windows 全程: {:.1} ms（{} 块屏有候选）",
            ms(at),
            candidates.len()
        );

        // 下面这两段量的是**已经下线**的旧交付路径（payload 里带 pngBase64）。
        // 留着是为了随时复核"原始 RGBA 直传值不值"这个结论：一旦有人想把像素塞回 JSON，
        // 先跑一遍这里看看要付多少代价。
        for frame in &frames {
            let at = Instant::now();
            let png =
                crate::screenshot::encode_png(&frame.rgba, frame.pixel_width, frame.pixel_height)
                    .unwrap();
            let encode = ms(at);
            let at = Instant::now();
            let base64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png);
            println!(
                "payload 显示器 {} {}x{}: PNG 编码 {:.1} ms（{} KiB）+ base64 {:.1} ms（{} KiB）",
                frame.monitor_id,
                frame.pixel_width,
                frame.pixel_height,
                encode,
                png.len() / 1024,
                ms(at),
                base64.len() / 1024,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_ocr_rejects_failures_and_blank_output_with_safe_error() {
        for result in [
            Err("private tesseract detail".to_string()),
            Ok("  \n".to_string()),
        ] {
            let error = normalize_ocr_text(result).unwrap_err();
            assert_eq!(error, TranslationError::OcrFailed);
            assert_eq!(
                error.ipc_message(),
                "translation.ocr_failed: Local OCR could not extract text from the image"
            );
        }
    }

    #[test]
    fn commit_payload_must_be_a_real_png_within_the_size_limit() {
        assert_eq!(
            decode_commit_png("not base64!!").unwrap_err().code(),
            "commit_payload_invalid"
        );
        // 合法 base64 但不是 PNG
        assert_eq!(
            decode_commit_png("aGVsbG8=").unwrap_err().code(),
            "commit_payload_invalid"
        );
        // 长度上界在解码之前就拦住，不为畸形载荷分配内存
        let oversized = "A".repeat(MAX_COMMIT_PNG_BYTES / 3 * 4 + 8);
        assert_eq!(
            decode_commit_png(&oversized).unwrap_err().code(),
            "commit_payload_too_large"
        );
    }

    #[test]
    fn commit_payload_accepts_the_data_url_form_the_canvas_produces() {
        let png = crate::screenshot::encode_png(&[255, 0, 0, 255], 1, 1).unwrap();
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png);
        assert_eq!(decode_commit_png(&encoded).unwrap(), png);
        assert_eq!(
            decode_commit_png(&format!("data:image/png;base64,{encoded}")).unwrap(),
            png
        );
    }

    #[test]
    fn capture_ocr_preserves_recognized_text_for_translation() {
        assert_eq!(
            normalize_ocr_text(Ok("local OCR text".to_string())),
            Ok("local OCR text".to_string())
        );
    }
}
