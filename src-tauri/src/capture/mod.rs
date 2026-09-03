mod action_lifecycle;
/// 几何诊断报告。报障与"补一条回归测试"之间的那一步，见 docs/capture-linux.md §4.2。
pub mod diagnostics;
mod error;
mod frame_protocol;
mod manager;
mod overlay_windows;
#[cfg(target_os = "linux")]
mod shell_extension;
#[cfg(not(target_os = "linux"))]
#[path = "shell_extension_stub.rs"]
mod shell_extension;
mod types;
mod window_probe;

pub use error::CaptureError;
pub(crate) use frame_protocol::handle as frame_protocol;
pub use manager::CaptureManager;
/// 贴图窗口的摆放与置顶也只有这个扩展做得到（Wayland 不许客户端自己来），
/// 所以 `pin/` 借道这里，而不是自己再开一份 D-Bus 契约。
pub(crate) use shell_extension::place_window as shell_extension_place_window;
/// 窗口几何查询。`pin/` 借它问"我这个贴图窗口现在在屏幕的哪儿"——Wayland 下
/// `outer_position()` 是假的（见 `pin::window::known_pin_position`），只有扩展知道真值。
pub(crate) use shell_extension::probe as shell_extension_windows;
/// 截图后端要用扩展这条路取冻结帧。扩展的全部 IPC 都留在 `shell_extension` 里，
/// 这里只把入口露出去，免得契约散成两份。
#[cfg(target_os = "linux")]
pub(crate) use shell_extension::request_screenshot as shell_extension_screenshot;
/// 逐屏原生取像素（协议 v5）。整屏那条路既会把低缩放的屏上采样、又要 gnome-shell
/// 编一张全桌面 PNG（实测 1.8 秒），所以这是首选，上面那个整屏入口只是它的兜底。
#[cfg(target_os = "linux")]
pub(crate) use shell_extension::{
    request_area_captures as shell_extension_area_captures, AreaCapture, CaptureArea,
};
pub use shell_extension::{InstallOutcome, ShellExtensionStatus};
pub use types::{
    CaptureAction, CaptureActionResult, CaptureOverlayPayload, CaptureSelection,
    CaptureTranslationResult,
};

use crate::commands::AppState;
use crate::translation::types::TranslationError;
use tauri::State;

/// renderer v2 输出的 PNG 上限。全屏 4K 的标注 PNG 大约十几 MB，
/// 64 MiB 足够宽松，同时挡住畸形/恶意载荷把内存吃光。
const MAX_COMMIT_PNG_BYTES: usize = 64 * 1024 * 1024;

/// 隐藏自己的窗口之后等合成器真的把它们从屏幕上撤掉的时间。
///
/// 少了这一等会把 Clippy 自己的面板烧进冻结帧。但它**只在真的藏了窗口时才需要**：
/// 快捷键截图的常态是面板本来就没开着（`hide_sources` 返回空），这时白等 140 ms
/// 纯粹是加在用户感知延迟上的。所以按需等待，不要改回无条件 sleep。
const HIDE_SETTLE_MS: u64 = 140;

/// 启动自检：扩展内容过期就静默升级，目录被手工删掉就清理 gsettings 里的孤儿条目。
#[cfg(target_os = "linux")]
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
        crate::pin::restore_pins_after_capture(app_handle, state, &session.lowered_pins);
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
    // **降贴图要在拍完之后。** 贴图该被截进冻结帧（屏幕上有它，画面里就该有它），
    // 而且要按用户看到的那个层级截——先降层再拍，被贴图压着的窗口就会出现在画面里，
    // 拍出来的东西和刚才屏幕上的不一样。降层的唯一目的是别让它盖住选择器。
    let lowered_pins = crate::pin::lower_pins_for_capture(&app_handle, state);
    let probe_hint = take_probe_hint(state);
    let specs = match state.capture_manager.begin(
        frames,
        restore_labels.clone(),
        lowered_pins.clone(),
        probe_hint,
        timings,
    ) {
        Ok(specs) => specs,
        Err(error) => {
            crate::pin::restore_pins_after_capture(&app_handle, state, &lowered_pins);
            overlay_windows::restore(&app_handle, &restore_labels);
            return Err(capture_failure(error));
        }
    };
    if let Err(error) = overlay_windows::create(&app_handle, &specs) {
        if let Some(session) = state.capture_manager.abort() {
            overlay_windows::close(&app_handle, &session.overlay_labels());
        }
        crate::pin::restore_pins_after_capture(&app_handle, state, &lowered_pins);
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
    crate::pin::restore_pins_after_capture(&app_handle, &state, &session.lowered_pins);
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

/// 覆盖层内完成选区与标注之后的唯一提交入口。
///
/// 前端只提交选区和 renderer v2 操作文档；后端从会话中取可信冻结帧，
/// 在 blocking worker 合成完整帧后只输出选区视口。这样既保留跨边界标注/模糊邻域，
/// 又不让 WebView Canvas 成为最终像素事实源。会话在执行动作前仍会被唯一认领。
///
/// `origin` 是选区在桌面逻辑坐标系里的矩形（覆盖层用 payload 的 `logicalX/logicalY`
/// 换算好）。贴图靠它回到原处、按原尺寸显示；复制时也记一份，方便之后从历史里
/// 把同一张图 Pin 回原位。
#[tauri::command]
pub async fn commit_capture_action(
    action: CaptureAction,
    selection: CaptureSelection,
    project: crate::pin::commands::PinCanvasProject,
    origin: Option<crate::pin::PinOrigin>,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<CaptureActionResult, String> {
    let session_id = selection.session_id.clone();
    let capture_manager = state.capture_manager.clone();
    let rendered = tauri::async_runtime::spawn_blocking(move || {
        if project.renderer_version != crate::pin::render_v2::RENDERER_VERSION {
            return Err("截图只接受 renderer v2 文档".to_string());
        }
        let input = capture_manager
            .render_input(&selection)
            .map_err(String::from)?;
        if input.source.dimensions() != (project.source_width, project.source_height) {
            return Err("截图工程尺寸与冻结帧不匹配".to_string());
        }
        let png = crate::pin::render_v2::render_capture(
            input.source,
            input.crop,
            &project.annotations,
            &project.adjustments,
        )?;
        commit_image_from_png(png).map_err(String::from)
    })
    .await
    .map_err(|error| format!("截图合成任务异常: {error}"))
    .and_then(|result| result);
    let result = action_lifecycle::complete_capture_action(
        rendered,
        || {
            state
                .capture_manager
                .finish(&session_id)
                .map_err(String::from)
        },
        |session| overlay_windows::close(&app_handle, &session.overlay_labels()),
        |session| {
            // 贴图的层级要在覆盖层关掉之后才放回去，否则刚恢复置顶的贴图会先盖住
            // 还没消失的覆盖层，闪一下。
            crate::pin::restore_pins_after_capture(&app_handle, &state, &session.lowered_pins);
            overlay_windows::restore(&app_handle, &session.restore_labels);
        },
        |png| execute_action(action, png, origin, &app_handle, &state),
    );
    // 覆盖层在动作之前就关掉了，错误已经没有窗口可以显示——只能留在日志里，
    // 否则"点了对钩什么都没发生"完全无从排查。
    if let Err(error) = &result {
        log::warn!("截图提交失败: {error}");
    }
    result
}

/// renderer v2 产生的那张图，字节和像素各一份。
///
/// 校验这张 PNG 必须整张解码（见 [`commit_image_from_png`]），所以**顺手把像素留下来**：
/// 复制那条路要的正是 RGBA，不留就得再解一遍同一张全屏 PNG（1080p 约 20 ms，
/// 2560x1440 起跳 40 ms 上下，见 docs/bench-baseline.md），而用户此刻正等着对钩生效。
/// 保存与贴图要的是原样的 PNG 字节，它们会当场把 `pixels` 扔掉（十几 MB）。
#[derive(Debug)]
struct CommitImage {
    png: Vec<u8>,
    pixels: image::RgbaImage,
}

/// 不信任编码器的长度或输出：即使像素来自应用自身，也要在副作用前完整解码。
fn commit_image_from_png(png: Vec<u8>) -> Result<CommitImage, CaptureError> {
    if png.len() > MAX_COMMIT_PNG_BYTES {
        return Err(CaptureError::CommitPayloadTooLarge);
    }
    // 必须真的能解成图像：后续 copy/save/pin 都假设手里是合法 PNG。这里是信任边界，
    // 所以整张解码，而不是只看文件头的 `png_dimensions`。
    let pixels = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .map_err(|_| CaptureError::CommitPayloadInvalid)?
        .into_rgba8();
    Ok(CommitImage { png, pixels })
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
    image: CommitImage,
    origin: Option<crate::pin::PinOrigin>,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<CaptureActionResult, String> {
    let CommitImage { png, pixels } = image;
    match action {
        CaptureAction::Copy => {
            // 像素是校验那一步解出来的（见 `CommitImage`），剪贴板和来源登记共用它：
            // 三处各解一遍这张全屏 PNG 要多花上百毫秒，而用户此刻正等着对钩生效。
            drop(png);
            let image = crate::image_io::rgba_to_clipboard_image(pixels);
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
            // 存盘写的是原样的 PNG 字节，校验解出来的那十几 MB 像素当场释放。
            drop(pixels);
            let path = crate::image_io::save_png(&png, "clippy-screenshot", &state.save_target())?;
            Ok(action_result(
                "save",
                Some(path.to_string_lossy().to_string()),
                None,
            ))
        }
        CaptureAction::Pin => {
            // 同上：贴图窗口拿的是 PNG，像素先扔掉再去建窗（建窗那条路本身就要几百毫秒）。
            drop(pixels);
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

    /// 逐屏原生取像素（协议 v5）单独计时，好和整屏那条路直接对比。
    ///
    /// 区域从 Tauri 的显示器几何来而不是 `capture_monitor_frames`：那样这一段就不依赖
    /// 截图链路本身，扩展还是旧版（截图很糊那种状态）时也能量出"新路子能快多少"。
    /// 拿不到几何就跳过——这只是诊断，不该在这里失败。
    ///
    /// 打印里区分 RGBA 与 PNG：扩展那边原始像素走不通时会自动退回 PNG，那时这条日志
    /// 是"为什么还是慢"的第一手证据（另一手在 journal 的 `Clippy raw capture failed`）。
    fn print_area_screenshot_timings() {
        let Ok(monitors) = crate::screenshot::logical_monitor_areas() else {
            println!("逐屏截图：拿不到显示器几何，跳过");
            return;
        };
        let at = Instant::now();
        match super::shell_extension::request_area_captures(&monitors) {
            Ok(captures) => {
                println!(
                    "扩展 CaptureArea × {} 并行往返: {:.1} ms",
                    monitors.len(),
                    ms(at)
                );
                for (area, capture) in monitors.iter().zip(captures.iter()) {
                    let at = Instant::now();
                    let bytes = std::fs::read(capture.path()).unwrap_or_default();
                    let dimensions = match capture {
                        super::shell_extension::AreaCapture::Raw { width, height, .. } => {
                            Some((*width, *height))
                        }
                        super::shell_extension::AreaCapture::Png { .. } => {
                            image::load_from_memory(&bytes)
                                .map(|image| image.to_rgba8().dimensions())
                                .ok()
                        }
                    };
                    println!(
                        "  区域 {}x{}@{},{}×{:.4} → {:?} {:?}，读+解码 {:.1} ms（{} KiB）",
                        area.width,
                        area.height,
                        area.x,
                        area.y,
                        area.scale,
                        dimensions,
                        match capture {
                            super::shell_extension::AreaCapture::Raw { .. } => "RGBA",
                            super::shell_extension::AreaCapture::Png { .. } => "PNG",
                        },
                        ms(at),
                        bytes.len() / 1024,
                    );
                    let _ = std::fs::remove_file(capture.path());
                }
            }
            Err(error) => println!("逐屏取像素不可用（{:.1} ms）: {error}", ms(at)),
        }
    }

    #[test]
    #[ignore = "需要真实桌面会话"]
    fn capture_stage_timings() {
        print_area_screenshot_timings();

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
    fn rendered_capture_must_be_a_real_png() {
        assert_eq!(
            commit_image_from_png(b"not a png".to_vec())
                .unwrap_err()
                .code(),
            "commit_payload_invalid"
        );
    }

    /// 校验解出来的像素必须能直接交给剪贴板：复制那条路就靠它省下第二次整张解码。
    #[test]
    fn rendered_capture_keeps_the_pixels_it_decoded_for_clipboard() {
        let png = crate::screenshot::encode_png(&[9, 8, 7, 255], 1, 1).unwrap();
        let image =
            crate::image_io::rgba_to_clipboard_image(commit_image_from_png(png).unwrap().pixels);
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!(image.bytes.as_ref(), [9, 8, 7, 255]);
    }

    #[test]
    fn capture_ocr_preserves_recognized_text_for_translation() {
        assert_eq!(
            normalize_ocr_text(Ok("local OCR text".to_string())),
            Ok("local OCR text".to_string())
        );
    }
}
