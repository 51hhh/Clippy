use super::model::{
    validate_label, PinEntry, PinOrigin, PinPayload, PinSource, PinState, PinUpdate, SharpenSlot,
};
use super::window::{
    content_buffer_scale, content_device_scale, create_pin_window, fit_content_size,
    keep_pin_above, origin_content_size, resize_pin_window, reveal_pin_window,
};
use crate::commands::AppState;
use crate::models::ContentType;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::sync::Arc;
use tauri::{Emitter, Manager, State};

#[tauri::command]
pub fn pin_clip(
    id: i64,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let label = format!("pin-clip-{id}");
    if let Some(window) = app_handle.get_webview_window(&label) {
        if state.pin_manager.get(&label).is_ok() {
            // 同一个条目只对应一个贴图窗口，这是刻意的：label 是 GNOME Shell 扩展唯一的
            // 查找键（`window_marker` = 标题 + pid），同名开两个的话扩展的查找只会命中
            // 第一个，第二张贴图从此摆不了位也置不了顶。
            //
            // 但"什么都不发生"是个坏反馈——那张贴图可能正被别的窗口压着、或在另一个
            // 工作区，`set_focus` 的效果用户根本看不见。所以让它闪一下外围蓝框说明
            // "它已经在这儿了"。
            window.show().map_err(|error| error.to_string())?;
            let _ = window.set_focus();
            if let Err(error) = window.emit(PIN_ALREADY_OPEN, ()) {
                log::debug!("提醒既有贴图窗口失败: {error}");
            }
            return Ok(label);
        }
        // A previous creation failed after the native window was built. Do not
        // reuse an orphaned window with no payload entry.
        let _ = window.close();
        return Err("贴图窗口状态不完整，请重试".to_string());
    }

    let (item, image) = {
        let storage = state.storage.lock().map_err(|error| error.to_string())?;
        let mut item = storage
            .get_clip_by_id(id)
            .map_err(|error| error.to_string())?;
        // `get_clip_by_id` 已经把整张图读出来了，从条目里**拿走**它而不是再查一遍库：
        // 全屏截图是几 MB，多读一遍就是多一次几 MB 的 blob 拷贝，而且贴图窗口活着的
        // 期间条目里那份会一直占着内存（`pin/` 只用 `image`，从不看 `item.image_data`）。
        // 只有图片条目有 blob，所以 take 出来的东西和按 content_type 判断是一回事。
        let image = item.image_data.take();
        (item, image)
    };
    let (width, height) = image
        .as_deref()
        .and_then(|png| crate::screenshot::png_dimensions(png).ok())
        .map(|(width, height)| (width as f64, height as f64))
        .unwrap_or((420.0, 280.0));
    // 这张图是不是我们自己截下来复制进剪贴板的？是的话它带着原始矩形，
    // 贴图就该回到那块屏幕、按那个尺寸；别处来的图片查不到，走常规缩放与光标定位。
    let origin = image
        .as_deref()
        .and_then(|png| state.pin_origins.lookup(png));
    let (content_width, content_height) = match origin {
        Some(origin) => origin_content_size(&app_handle, origin),
        None => fit_content_size(&app_handle, width, height),
    };
    state.pin_manager.insert(PinEntry {
        label: label.clone(),
        source: Arc::new(PinSource::Clip { item, image }),
        content_width,
        content_height,
        scale: 1.0,
        opacity: 1.0,
        locked: false,
        above: false,
        position: None,
        origin,
        device_scale: content_device_scale(&app_handle, origin),
        buffer_scale: content_buffer_scale(&app_handle, origin),
        sharpen: Arc::new(SharpenSlot::default()),
    })?;
    // 建窗之前就把清晰度补偿放出去跑，好和 WebKit 起步那几百毫秒重叠。
    spawn_sharpen(&app_handle, &state.pin_manager.get(&label)?);
    if let Err(error) =
        create_pin_window(&app_handle, &label, content_width, content_height, origin)
    {
        let _ = state.pin_manager.remove(&label);
        return Err(crate::error::report("创建剪贴板贴图窗口失败", error));
    }
    Ok(label)
}

/// `origin` 是这张图在屏幕上原本占的矩形（逻辑像素）。截图覆盖层知道选区落在哪，
/// 于是贴图能贴回原处、原尺寸；不知道来源的图片传 `None`，落回光标附近。
pub(crate) fn create_screenshot_pin(
    png: Vec<u8>,
    origin: Option<PinOrigin>,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<String, String> {
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let (width, height) =
        crate::screenshot::png_dimensions(&png).map_err(|error| error.to_string())?;
    let label = format!("pin-image-{}", crate::image_io::unique_image_id());
    let origin = origin.and_then(PinOrigin::sanitized);
    let (content_width, content_height) = match origin {
        Some(origin) => origin_content_size(app_handle, origin),
        None => fit_content_size(app_handle, width as f64, height as f64),
    };
    state.pin_manager.insert(PinEntry {
        label: label.clone(),
        source: Arc::new(PinSource::Screenshot { png }),
        content_width,
        content_height,
        scale: 1.0,
        opacity: 1.0,
        locked: false,
        above: false,
        position: None,
        origin,
        device_scale: content_device_scale(app_handle, origin),
        buffer_scale: content_buffer_scale(app_handle, origin),
        sharpen: Arc::new(SharpenSlot::default()),
    })?;
    // 同上：补偿与开窗并行，抢在前端来取 payload 之前算完。
    spawn_sharpen(app_handle, &state.pin_manager.get(&label)?);
    if let Err(error) = create_pin_window(app_handle, &label, content_width, content_height, origin)
    {
        let _ = state.pin_manager.remove(&label);
        return Err(crate::error::report("创建截图贴图窗口失败", error));
    }
    Ok(label)
}

/// 截图期间让置顶的贴图暂时退出置顶层，返回被降下来的那些 label。
///
/// **贴图仍然会被截进冻结帧，这是刻意的**：它在屏幕上就是一块内容，用户看到的画面里有它，
/// 截出来就该有它。要避免的只是"它浮在截图选择器上面"——覆盖层在 Wayland 下进不了置顶层
/// （Mutter 忽略客户端的 `always_on_top`，见 `pin::window::keep_pin_above`），所以一张开着
/// 图钉的贴图会盖住选择器，挡住选区和工具条。
///
/// 只动**当前确实在置顶层**的那些：没开图钉的贴图本来就是普通窗口，覆盖层刚映射、
/// 刚拿到焦点，天然压在它上面，不需要碰。
pub(crate) fn lower_pins_for_capture(
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Vec<String> {
    let mut lowered = Vec::new();
    for label in state.pin_manager.labels_above() {
        if let Some(window) = app_handle.get_webview_window(&label) {
            keep_pin_above(&window, None, false);
            lowered.push(label);
        }
    }
    lowered
}

/// 截图结束后把刚才降下来的贴图放回置顶层。
///
/// 按 label 逐个查条目而不是无条件置顶：这几百毫秒里用户可能已经关掉了那张贴图，
/// 或者（更细的情形）在别的窗口里把图钉关掉了，那就该尊重现在的状态。
pub(crate) fn restore_pins_after_capture(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    labels: &[String],
) {
    for label in labels {
        let Ok(entry) = state.pin_manager.get(label) else {
            continue;
        };
        if !entry.above {
            continue;
        }
        if let Some(window) = app_handle.get_webview_window(label) {
            keep_pin_above(&window, None, true);
        }
    }
}

/// 置顶的贴图拿到焦点：在置顶层内重新抬到最前。
///
/// 只对开着图钉的那些做。没开图钉的贴图是普通窗口，合成器自己会把它抬上来；对它调
/// `make_above` 反而会把它塞进置顶层，等于偷偷替用户开了图钉。
///
/// 不动位置（`keep_pin_above` 传 `None`）：这一刻用户可能正拖着这张贴图，
/// 顺手摆位就会把它拽回旧坐标。
pub(crate) fn raise_focused_pin(app_handle: &tauri::AppHandle, state: &AppState, label: &str) {
    let Ok(entry) = state.pin_manager.get(label) else {
        return;
    };
    if !entry.above {
        return;
    }
    if let Some(window) = app_handle.get_webview_window(label) {
        keep_pin_above(&window, None, true);
    }
}

#[tauri::command]
pub fn get_pin_payload(label: String, state: State<'_, AppState>) -> Result<PinPayload, String> {
    validate_label(&label)?;
    payload_from_entry(state.pin_manager.get(&label)?)
}

#[tauri::command]
pub fn pin_ready(
    label: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    validate_label(&label)?;
    let entry = state.pin_manager.get(&label)?;
    let window = app_handle
        .get_webview_window(&label)
        .ok_or_else(|| "贴图窗口不存在".to_string())?;
    // 平台适配（置顶 + 缩放锁）在建窗时就做过了，这里只负责显示与摆位；
    // 重复调用会给 zoom-level 挂上第二个回调。
    reveal_pin_window(&app_handle, &window, &entry).map_err(|error| error.to_string())?;
    // 图已经在屏上了，补偿结果（最大十几 MB）没人会再来取，扔掉它——贴图窗口可以开好几个，
    // 留着就是每个窗口白占十几 MB 到关闭为止。见 `SharpenSlot::release`。
    entry.sharpen.release();
    Ok(())
}

/// 改缩放/不透明度/锁定，应答只带变了的那几个字段（见 `PinState`）。
///
/// **这是每帧都会走的路**：滚轮缩放时前端按 rAF 合并后仍是一帧一次。同步命令跑在
/// 主线程上，所以这条路上不能有整张图的 base64、也不能有多余的文件读与 D-Bus 握手
/// （摆位那侧的缓存见 `capture::shell_extension::place_window`）。
#[tauri::command]
pub fn update_pin(
    label: String,
    update: PinUpdate,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<PinState, String> {
    validate_label(&label)?;
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let previous = state.pin_manager.get(&label)?;
    let entry = state.pin_manager.update(&label, &update)?;
    // 图钉开关：只改层级、不动位置和尺寸。关掉就是 `unmake_above`，贴图从此是个普通窗口。
    // 缩放那条路自己会把层级重新表态一次，所以这里只处理"只按了图钉"的情况。
    if update.above.is_some() && update.scale.is_none() {
        if let Some(window) = app_handle.get_webview_window(&label) {
            keep_pin_above(&window, None, entry.above);
        }
    }
    if update.scale.is_some() {
        if let Err(error) = resize_pin_window(&app_handle, &entry) {
            if let Err(rollback_error) = resize_pin_window(&app_handle, &previous) {
                log::warn!("回滚贴图窗口尺寸失败: {rollback_error}");
            }
            state.pin_manager.replace(previous)?;
            // 缩放途中窗口被关掉属于正常竞争，不是故障。
            let context = "缩放贴图窗口失败";
            return Err(if error.is_gone() {
                crate::error::note(context, error)
            } else {
                crate::error::report(context, error)
            });
        }
    }
    Ok(state_from_entry(&entry))
}

#[tauri::command]
pub fn copy_pin(label: String, state: State<'_, AppState>) -> Result<(), String> {
    validate_label(&label)?;
    let entry = state.pin_manager.get(&label)?;
    match &*entry.source {
        PinSource::Clip { item, .. } => crate::commands::write_clip_to_clipboard(item.id, &state),
        // 这里**不设** skip hash：watcher 哈希的是它自己从剪贴板 RGBA 重新编出来的 PNG，
        // 和我们手上这串字节几乎不可能相同，设了也永远匹配不上（历史上就是这样白算了一次
        // 全图 sha256）。后果只是这张图重新进一次历史——`insert_clip` 按哈希去重，
        // 已有的那条只会被顶到最前面，没有重复存储。
        PinSource::Screenshot { png } => crate::image_io::copy_png_to_clipboard(png),
    }
}

#[tauri::command]
pub fn save_pin(label: String, state: State<'_, AppState>) -> Result<String, String> {
    validate_label(&label)?;
    let png = image_bytes(&state.pin_manager.get(&label)?)?;
    let path = crate::image_io::save_png(&png, "clippy-pin", &state.save_target())?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn close_pin(
    label: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_label(&label)?;
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    if let Some(window) = app_handle.get_webview_window(&label) {
        window.close().map_err(|error| error.to_string())?;
    }
    let _ = state.pin_manager.remove(&label)?;
    Ok(())
}

fn state_from_entry(entry: &PinEntry) -> PinState {
    PinState {
        label: entry.label.clone(),
        content_width: entry.content_width,
        content_height: entry.content_height,
        scale: entry.scale,
        opacity: entry.opacity,
        locked: entry.locked,
        above: entry.above,
        position: entry.position,
    }
}

fn payload_from_entry(entry: PinEntry) -> Result<PinPayload, String> {
    let (kind, text, image, can_save) = match &*entry.source {
        PinSource::Clip { item, image } => match item.content_type {
            ContentType::Image => ("image", None, image.as_deref(), true),
            ContentType::Text | ContentType::Html => {
                ("text", item.text_content.clone(), None, false)
            }
        },
        PinSource::Screenshot { png } => ("image", None, Some(png.as_slice()), true),
    };
    // 补偿赶上了就直接发清晰版，这样第一帧就是清楚的；没赶上发原图，
    // 后台线程算完会走 `pin-image-sharpened` 补上（见 `SharpenSlot`）。
    let sharpened = image.and(entry.sharpen.take_for_payload());
    let image_base64 = match (&sharpened, image) {
        (Some(sharp), _) => Some(STANDARD.encode(sharp.as_slice())),
        (None, Some(png)) => Some(STANDARD.encode(png)),
        (None, None) => None,
    };
    Ok(PinPayload {
        label: entry.label,
        kind,
        text,
        image_base64,
        content_width: entry.content_width,
        content_height: entry.content_height,
        scale: entry.scale,
        opacity: entry.opacity,
        locked: entry.locked,
        above: entry.above,
        can_save,
        position: entry.position,
        device_scale: entry.device_scale,
        buffer_scale: entry.buffer_scale,
    })
}

/// 清晰版图片就绪的事件名。与 `src/js/api.ts` 的 `onPinImageSharpened` 是一份契约。
const PIN_IMAGE_SHARPENED: &str = "pin-image-sharpened";

/// "这张图已经贴出来了"。与 `src/js/api.ts` 的 `onPinAlreadyOpen` 是一份契约。
/// 发给那个既有窗口自己，它闪一下外围蓝框。
const PIN_ALREADY_OPEN: &str = "pin-already-open";

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PinImageSharpened {
    label: String,
    image_base64: String,
}

/// 在后台把贴图重新渲染成"缓冲区分辨率 + 已补偿"的版本。
///
/// 为什么要补偿、以及不补偿的话糊在哪里，见 `super::resample`。
///
/// **在建条目时就开跑，不等窗口。** 补偿在 release 本机实测要 250 ms 上下（屏上 1200x900）
/// 到 800 ms 上下（2560x1440），而建窗 + WebKit 起步 + React 挂载本来也要几百毫秒。两件事
/// 并行之后，前端来取 payload 时清晰版多半已经躺在 `SharpenSlot` 里了，于是**第一帧
/// 就是清楚的**；没赶上才退回"原图先上屏、事件补送"，用户最多看到一次变清楚。
///
/// **为什么不在 `get_pin_payload` 里同步等**：那条命令跑在 GTK 主线程上，同步等于把
/// 整个界面卡住几百毫秒，而"慢"正是这个功能一直在修的另一个毛病。
///
/// 复制（`copy_pin`）与保存/编辑（`save_pin`）不受影响：它们走 `image_bytes`，永远是原图。
fn spawn_sharpen(app_handle: &tauri::AppHandle, entry: &PinEntry) {
    let Some(geometry) = super::resample::display_geometry(
        entry.content_width,
        entry.content_height,
        entry.device_scale,
        entry.buffer_scale,
    ) else {
        return;
    };
    let source = Arc::clone(&entry.source);
    let slot = Arc::clone(&entry.sharpen);
    let label = entry.label.clone();
    let app_handle = app_handle.clone();
    let (device_scale, buffer_scale) = (entry.device_scale, entry.buffer_scale);
    // 补偿是"锦上添花"的一步，失败只影响清晰度，所以线程里所有错误都只记日志。
    std::thread::spawn(move || {
        let Some(png) = source_png(&source) else {
            return;
        };
        let started = std::time::Instant::now();
        match super::resample::compensated_png(png, geometry) {
            Ok(bytes) => {
                let bytes = Arc::new(bytes);
                let late = slot.finish(&bytes);
                // 记到 info：两个缩放是**按机器不同**的那两个数，一旦有人报"贴图还是糊"
                // 或者"过锐"，这一行就是第一手证据。`late` 说明这一张没赶上第一帧，
                // 用户会看见一次"由糊变清"——报这种现象时也是看这一行。
                log::info!(
                    "{label} 清晰度补偿完成：真实缩放 {device_scale} / 缓冲区缩放 {buffer_scale}，\
                     屏上 {:?} → 缓冲区 {:?}，耗时 {:?}{}",
                    geometry.panel,
                    geometry.buffer,
                    started.elapsed(),
                    if late {
                        "（没赶上首帧，改用事件换图）"
                    } else {
                        ""
                    }
                );
                if !late {
                    return;
                }
                let payload = PinImageSharpened {
                    label: label.clone(),
                    image_base64: STANDARD.encode(bytes.as_slice()),
                };
                if let Err(error) = app_handle.emit_to(label.as_str(), PIN_IMAGE_SHARPENED, payload)
                {
                    log::warn!("推送贴图清晰版失败: {error}");
                }
            }
            Err(error) => log::warn!("贴图清晰度补偿失败，保留原图: {error}"),
        }
    });
}

/// 贴图内容里的 PNG 字节，文本贴图没有。
fn source_png(source: &PinSource) -> Option<&[u8]> {
    match source {
        PinSource::Clip { image, .. } => image.as_deref(),
        PinSource::Screenshot { png } => Some(png),
    }
}

fn image_bytes(entry: &PinEntry) -> Result<Vec<u8>, String> {
    match &*entry.source {
        PinSource::Clip {
            image: Some(png), ..
        }
        | PinSource::Screenshot { png } => Ok(png.clone()),
        _ => Err("文本贴图不能保存或编辑为图片".to_string()),
    }
}
