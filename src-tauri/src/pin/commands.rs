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
use serde::{Deserialize, Serialize};
use std::path::Path;
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

/// 从文件打开后的扁平图/工程创建贴图。所有读取和工程校验在调用前完成，因此插入 manager
/// 后唯一可能失败的是建窗；失败路径会立刻移除条目，不留下半初始化状态。
fn create_opened_image_pin(
    preview_png: Vec<u8>,
    project: Option<(Vec<u8>, super::project::PinProject)>,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<String, String> {
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let (width, height) = crate::screenshot::png_dimensions(&preview_png)
        .map_err(|_| "所选文件不是合法 PNG".to_string())?;
    let label = format!("pin-image-{}", crate::image_io::unique_image_id());
    let (content_width, content_height) =
        fit_content_size(app_handle, f64::from(width), f64::from(height));
    let source = match project {
        Some((source_png, project)) => PinSource::Project {
            source_png,
            preview_png,
            project,
        },
        None => PinSource::Screenshot { png: preview_png },
    };
    let entry = PinEntry {
        label: label.clone(),
        source: Arc::new(source),
        content_width,
        content_height,
        scale: 1.0,
        opacity: 1.0,
        locked: false,
        above: false,
        position: None,
        origin: None,
        device_scale: content_device_scale(app_handle, None),
        buffer_scale: content_buffer_scale(app_handle, None),
        sharpen: Arc::new(SharpenSlot::default()),
    };
    insert_pin_with_rollback(&state.pin_manager, entry, |inserted| {
        spawn_sharpen(app_handle, inserted);
        create_pin_window(app_handle, &label, content_width, content_height, None)
            .map_err(|error| crate::error::report("创建图片贴图窗口失败", error))
    })?;
    Ok(label)
}

/// PinManager 插入与原生建窗是一笔事务：后续步骤失败时必须删除刚插入的 entry。
/// 抽成纯状态 helper 后，窗口系统不可用的单元测试环境也能钉住 rollback 不变量。
fn insert_pin_with_rollback<T>(
    manager: &super::manager::PinManager,
    entry: PinEntry,
    after_insert: impl FnOnce(&PinEntry) -> Result<T, String>,
) -> Result<T, String> {
    let label = entry.label.clone();
    manager.insert(entry)?;
    let inserted = match manager.get(&label) {
        Ok(entry) => entry,
        Err(error) => {
            let _ = manager.remove(&label);
            return Err(error.into());
        }
    };
    match after_insert(&inserted) {
        Ok(value) => Ok(value),
        Err(error) => {
            let _ = manager.remove(&label);
            Err(error)
        }
    }
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

/// 工具条能待的范围：贴图窗口里"还落在屏幕工作区内"的那块，窗口局部逻辑坐标。
///
/// **前端算不了这个。** 它只有 `window.innerWidth/innerHeight`，而贴图窗口的外框恒等于
/// 「内容 + 阴影 + 控件栏」，永远给工具条留够了位置——拿窗口自己当边界，"超出屏幕自动
/// 调整"一次都不会触发。真正超出的是窗口在屏幕上的位置，那要问合成器
/// （见 `super::window::pin_toolbar_bounds`）。
///
/// **异步**：Wayland 下要走一次 D-Bus 问扩展（本机实测 1~3 ms），不能压在 GTK 主线程上。
/// 前端只在"窗口位置或尺寸可能变了"之后问一次，不是每帧——见 `usePinToolbarBounds`。
#[tauri::command]
pub async fn get_pin_toolbar_bounds(
    label: String,
    app_handle: tauri::AppHandle,
) -> Result<super::window::ToolbarBounds, String> {
    validate_label(&label)?;
    // 查询里有阻塞的 D-Bus 调用与显示器枚举，挪出运行时线程。
    tauri::async_runtime::spawn_blocking(move || {
        super::window::pin_toolbar_bounds(&app_handle, &label)
    })
    .await
    .map_err(|error| error.to_string())
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
        PinSource::Project { preview_png, .. } => {
            crate::image_io::copy_png_to_clipboard(preview_png)
        }
    }
}

#[tauri::command]
pub fn save_pin(label: String, state: State<'_, AppState>) -> Result<String, String> {
    validate_label(&label)?;
    let png = image_bytes(&state.pin_manager.get(&label)?)?;
    let path = crate::image_io::save_png(&png, "clippy-pin", &state.save_target())?;
    Ok(path.to_string_lossy().to_string())
}

/// 贴图的**原图**（base64 PNG），只供前端 Canvas 交互预览；v2 最终导出由后端读取同一可信原图。
///
/// **为什么不能用屏上那张。** `get_pin_payload` 给前端的 `image_base64` 优先是清晰度
/// 补偿版（见 `payload_from_entry`）：它按**缓冲区分辨率**渲染（2560x1440 的贴图会是
/// 3413x1920），而且为"随后被合成器缩小 0.75"预先做了反投影锐化。那串字节只适合
/// 贴到那一个窗口的那一块缓冲区里，单独看是偏大且过冲的。拿它当导出底图，存出来的
/// 就是一张大一圈、发硬的图——这违反 `super::resample` 模块头写的
/// "复制与保存永远用原图"。
///
/// 所以导出时单独来取一次。**按需取而不是常驻**：导出是低频动作，而贴图窗口可以开
/// 好几个，让每个窗口长期多驻一份原图和刚做的"上屏后释放补偿结果"正好相反。
#[tauri::command]
pub fn get_pin_source_image(
    label: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    validate_label(&label)?;
    let entry = state.pin_manager.get(&label)?;
    Ok(source_png(&entry.source).map(|png| STANDARD.encode(png)))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PinCanvasSaveMode {
    Editable,
    Flat,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinCanvasSaveResult {
    pub path: String,
    pub clipboard_written: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clipboard_error: Option<String>,
}

/// 把最新合成图保存为可编辑工程或安全扁平 PNG。文件先原子落盘，剪贴板随后写入；后者
/// 失败不会谎报文件失败，而是通过结构化结果单独告知调用方。
#[tauri::command]
pub async fn save_pin_canvas(
    label: String,
    png_base64: Option<String>,
    to_clipboard: bool,
    mode: PinCanvasSaveMode,
    project: Option<PinCanvasProject>,
    state: State<'_, AppState>,
) -> Result<PinCanvasSaveResult, String> {
    validate_label(&label)?;
    let entry = state.pin_manager.get(&label)?;
    let save_target = state.save_target();
    tauri::async_runtime::spawn_blocking(move || {
        let (png, to_disk) = prepare_canvas_save(&entry, png_base64.as_deref(), mode, project)?;
        let path = crate::image_io::save_png(&to_disk, "clippy-pin", &save_target)?;
        let clipboard_error = if to_clipboard {
            crate::image_io::copy_png_to_clipboard(&png).err()
        } else {
            None
        };
        Ok(PinCanvasSaveResult {
            path: path.to_string_lossy().to_string(),
            clipboard_written: to_clipboard && clipboard_error.is_none(),
            clipboard_error,
        })
    })
    .await
    .map_err(|error| format!("画布保存线程异常: {error}"))?
}

/// 生成剪贴板合成图与落盘容器。
///
/// `png_base64 = None` 只表示“导入工程尚未发生本地编辑”：这时后端持有的 IDAT 预览
/// 比任一平台重新跑 Canvas 更权威。普通图片或已编辑文档必须提交最新渲染结果。
fn prepare_canvas_save(
    entry: &PinEntry,
    png_base64: Option<&str>,
    mode: PinCanvasSaveMode,
    project: Option<PinCanvasProject>,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let Some(png_base64) = png_base64 else {
        if let Some(project) = project {
            if project.renderer_version != super::render_v2::RENDERER_VERSION {
                return Err("只有 renderer v2 文档可以由后端生成合成图".to_string());
            }
            let png = render_canvas_project(entry, &project)?;
            let to_disk = match mode {
                PinCanvasSaveMode::Flat => super::project::flatten(&png)?,
                PinCanvasSaveMode::Editable => embed_canvas_project(&png, project, entry)?,
            };
            return Ok((png, to_disk));
        }
        let PinSource::Project {
            preview_png,
            project,
            ..
        } = &*entry.source
        else {
            return Err("只有未修改的导入工程可以复用合成预览".to_string());
        };
        let to_disk = match mode {
            PinCanvasSaveMode::Flat => preview_png.clone(),
            PinCanvasSaveMode::Editable => super::project::embed(preview_png, project)?,
        };
        return Ok((preview_png.clone(), to_disk));
    };

    let png = decode_canvas_png(png_base64)?;
    if project
        .as_ref()
        .is_some_and(|project| project.renderer_version == super::render_v2::RENDERER_VERSION)
    {
        return Err("renderer v2 不接受 WebView 上传的合成 PNG".to_string());
    }
    let to_disk = match mode {
        PinCanvasSaveMode::Flat => super::project::flatten(&png)?,
        PinCanvasSaveMode::Editable => {
            let project = project.ok_or_else(|| "保存可编辑 PNG 缺少工程数据".to_string())?;
            embed_canvas_project(&png, project, entry)?
        }
    };
    Ok((png, to_disk))
}

/// 已编辑贴图的 Copy/Ctrl+C：只把最新合成像素送入剪贴板，不携带 iTXt。
#[tauri::command]
pub async fn copy_pin_canvas(
    label: String,
    png_base64: Option<String>,
    project: Option<PinCanvasProject>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_label(&label)?;
    let entry = state.pin_manager.get(&label)?;
    tauri::async_runtime::spawn_blocking(move || {
        let png = prepare_canvas_copy(&entry, png_base64.as_deref(), project)?;
        crate::image_io::copy_png_to_clipboard(&png)
    })
    .await
    .map_err(|error| format!("画布复制线程异常: {error}"))?
}

fn prepare_canvas_copy(
    entry: &PinEntry,
    png_base64: Option<&str>,
    project: Option<PinCanvasProject>,
) -> Result<Vec<u8>, String> {
    match project {
        Some(project) if project.renderer_version == super::render_v2::RENDERER_VERSION => {
            if png_base64.is_some() {
                return Err("renderer v2 不接受 WebView 上传的合成 PNG".to_string());
            }
            render_canvas_project(entry, &project)
        }
        Some(_) => {
            let encoded = png_base64.ok_or_else(|| "renderer v1 复制缺少合成 PNG".to_string())?;
            decode_canvas_png(encoded)
        }
        None => {
            let encoded = png_base64.ok_or_else(|| "复制画布缺少工程文档".to_string())?;
            decode_canvas_png(encoded)
        }
    }
}

fn decode_canvas_png(png_base64: &str) -> Result<Vec<u8>, String> {
    // 在 base64 分配前先做保守长度判断；精确字节与完整 PNG 随后再验证。
    if png_base64.len() > super::project::MAX_RENDERED_PNG_BYTES.saturating_mul(4) / 3 + 64 {
        return Err("画布内容过大".to_string());
    }
    let png = crate::screenshot::decode_png_base64(png_base64)
        .map_err(|_| "画布内容 base64 无效".to_string())?;
    super::project::validate_rendered_png(&png)?;
    Ok(png)
}

/// 读一个 PNG 文件里的贴图工程数据。
///
/// 返回 `None` 表示"这是张普通图片"——没有工程块、块坏了、版本比当前新，三种情况对用户
/// 都是同一件事：能看，不能继续编辑（见 `super::project::extract`）。
///
/// **异步**：要读盘 + 解一次 PNG（工程块里还有一张 base64 原图），不能压在 GTK 主线程上。
///
/// 路径来自用户（文件对话框 / 命令行 / 文件关联），所以这里是信任边界：只按文件读，
/// 大小超限、不是 PNG、元数据坏掉都只是"读不出工程"，不会让调用方崩。
#[tauri::command]
pub async fn read_pin_project(path: String) -> Result<Option<super::project::PinProject>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (_, project) = read_png_file_with_project(Path::new(&path))?;
        Ok(project)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// 从主窗口选择 PNG 并创建贴图。取消返回 `None`；读取/验证/建窗任一步失败都不会遗留
/// manager 条目或孤儿窗口。
#[tauri::command]
pub async fn open_pin_image_dialog(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let start = state.save_target().directory;
    let dialog_handle = app_handle.clone();
    let selected = tauri::async_runtime::spawn_blocking(move || {
        crate::dialogs::choose_png(&dialog_handle, &start)
    })
    .await
    .map_err(|error| format!("图片选择线程异常: {error}"))?;
    let prepared = tauri::async_runtime::spawn_blocking(move || prepare_open_selection(selected))
        .await
        .map_err(|error| format!("图片读取线程异常: {error}"))?;
    complete_open_selection(prepared, |prepared| {
        create_opened_image_pin(prepared.preview_png, prepared.project, &app_handle, &state)
    })
}

struct PreparedPinImage {
    preview_png: Vec<u8>,
    project: Option<(Vec<u8>, super::project::PinProject)>,
}

/// chooser 取消时直接返回 `None`，不读盘、不创建窗口、不接触 PinManager。
fn prepare_open_selection(
    selected: Option<std::path::PathBuf>,
) -> Result<Option<PreparedPinImage>, String> {
    let Some(path) = selected else {
        return Ok(None);
    };
    prepare_opened_image(&path).map(Some)
}

/// 打开流程的提交边界：取消或准备失败时绝不调用创建步骤；只有完整、已验证的图片才能
/// 进入 PinManager 插入和原生窗口创建。把这一层收成纯函数，测试才能观察同一个 manager，
/// 避免用一个从未传给被测函数的空 manager 写出恒真的“无残留”断言。
fn complete_open_selection<F>(
    prepared: Result<Option<PreparedPinImage>, String>,
    create: F,
) -> Result<Option<String>, String>
where
    F: FnOnce(PreparedPinImage) -> Result<String, String>,
{
    let Some(prepared) = prepared? else {
        return Ok(None);
    };
    create(prepared).map(Some)
}

fn prepare_opened_image(path: &Path) -> Result<PreparedPinImage, String> {
    let (container, extracted) = read_png_file_with_project(path)?;
    prepare_opened_png(container, extracted)
}

fn prepare_opened_png(
    container: Vec<u8>,
    extracted: Option<super::project::PinProject>,
) -> Result<PreparedPinImage, String> {
    let project = match extracted {
        Some(project) => {
            // `extract` 已做完整验证，这里只解 base64，不重复解码整张原图。
            let source_png = project.decoded_source()?;
            Some((source_png, project))
        }
        None => None,
    };
    let preview_png = super::project::flatten_container(&container)?;
    Ok(PreparedPinImage {
        preview_png,
        project,
    })
}

#[cfg(test)]
fn read_png_file(path: &Path) -> Result<Vec<u8>, String> {
    read_png_file_with_project(path).map(|(bytes, _)| bytes)
}

fn read_png_file_with_project(
    path: &Path,
) -> Result<(Vec<u8>, Option<super::project::PinProject>), String> {
    use std::io::Read;

    let file = std::fs::File::open(path).map_err(|error| format!("打开文件失败: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("读取文件信息失败: {error}"))?;
    if !metadata.is_file() {
        return Err("所选路径不是普通文件".to_string());
    }
    if metadata.len() > super::project::MAX_CONTAINER_BYTES as u64 {
        return Err("PNG 文件超过 160 MiB 上限".to_string());
    }
    // 文件可能在 metadata 后被别的进程替换/增长；take(+1) 保证竞争下也不会无界读取。
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(super::project::MAX_CONTAINER_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取文件失败: {error}"))?;
    if bytes.len() > super::project::MAX_CONTAINER_BYTES {
        return Err("PNG 文件超过 160 MiB 上限".to_string());
    }
    // `extract` 同时验证整张 PNG；损坏工程只返回 None，损坏 IDAT 会报错。
    let project = super::project::extract(&bytes)?;
    Ok((bytes, project))
}

/// 前端送来的工程操作层。原图一律由**后端**从 Pin 条目或 Capture 会话取。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinCanvasProject {
    pub renderer_version: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub annotations: serde_json::Value,
    pub adjustments: serde_json::Value,
}

/// 给落盘的 PNG 加上工程块；任何一步失败都让 editable save 明确失败，禁止伪装成
/// 扁平保存成功。
///
/// 原图从条目里取而不是让前端回传：前端手上那张可能是补偿版（这正是 `get_pin_source_image`
/// 存在的理由），而且让前端把几 MB 原图再回传一趟纯属浪费——后端本来就有。
fn embed_canvas_project(
    png: &[u8],
    project: PinCanvasProject,
    entry: &PinEntry,
) -> Result<Vec<u8>, String> {
    if !matches!(
        project.renderer_version,
        super::project::LEGACY_RENDERER_VERSION | super::project::RENDERER_VERSION
    ) {
        return Err("工程渲染器版本不受支持".to_string());
    }
    let source =
        source_png(&entry.source).ok_or_else(|| "文本贴图不能保存为可编辑 PNG".to_string())?;
    let dimensions =
        crate::screenshot::png_dimensions(source).map_err(|_| "贴图原图无效".to_string())?;
    if dimensions != (project.source_width, project.source_height) {
        return Err("工程 sourceWidth/sourceHeight 与原图不匹配".to_string());
    }
    let rendered_dimensions =
        crate::screenshot::png_dimensions(png).map_err(|_| "合成 PNG 无效".to_string())?;
    if rendered_dimensions != dimensions {
        return Err("合成 PNG 尺寸必须与工程原图一致".to_string());
    }
    let document = super::project::PinProject::new(
        source,
        png,
        project.renderer_version,
        project.annotations,
        project.adjustments,
    )?;
    super::project::embed(png, &document)
}

/// renderer v2 的原图和最终像素都留在后端；前端只提交经过 schema 限制的操作文档。
fn render_canvas_project(entry: &PinEntry, project: &PinCanvasProject) -> Result<Vec<u8>, String> {
    let source = source_png(&entry.source).ok_or_else(|| "文本贴图不能渲染画布工程".to_string())?;
    super::render_v2::render(
        source,
        project.source_width,
        project.source_height,
        &project.annotations,
        &project.adjustments,
    )
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
    let (kind, text, image, can_save, initial_project) = match &*entry.source {
        PinSource::Clip { item, image } => match item.content_type {
            ContentType::Image => ("image", None, image.as_deref(), true, None),
            ContentType::Text | ContentType::Html => {
                ("text", item.text_content.clone(), None, false, None)
            }
        },
        PinSource::Screenshot { png } => ("image", None, Some(png.as_slice()), true, None),
        PinSource::Project {
            preview_png,
            project,
            ..
        } => (
            "image",
            None,
            Some(preview_png.as_slice()),
            true,
            Some(project.initial_payload()),
        ),
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
        initial_project,
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
/// 普通贴图的复制/保存不受影响；工程贴图则以保存时合成预览为显示与快速复制来源。
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
        let Some(png) = display_png(&source) else {
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
        PinSource::Project { source_png, .. } => Some(source_png),
    }
}

/// 屏幕清晰度补偿必须基于用户当前看到的合成预览；工程 canonical source 只供画布使用。
fn display_png(source: &PinSource) -> Option<&[u8]> {
    match source {
        PinSource::Clip { image, .. } => image.as_deref(),
        PinSource::Screenshot { png } => Some(png),
        PinSource::Project { preview_png, .. } => Some(preview_png),
    }
}

fn image_bytes(entry: &PinEntry) -> Result<Vec<u8>, String> {
    match &*entry.source {
        PinSource::Clip {
            image: Some(png), ..
        }
        | PinSource::Screenshot { png } => Ok(png.clone()),
        PinSource::Project { preview_png, .. } => Ok(preview_png.clone()),
        _ => Err("文本贴图不能保存或编辑为图片".to_string()),
    }
}

#[cfg(test)]
mod project_command_tests {
    use super::*;

    fn adjustments() -> serde_json::Value {
        serde_json::json!({"grayscale":false,"brightness":0,"contrast":0,
                           "saturation":0,"cornerRadius":0})
    }

    fn sample_png() -> Vec<u8> {
        crate::screenshot::encode_png(&[10, 20, 30, 255], 1, 1).unwrap()
    }

    fn png_with_project_text(text: &str) -> Vec<u8> {
        let image = image::load_from_memory_with_format(&sample_png(), image::ImageFormat::Png)
            .unwrap()
            .into_rgba8();
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_itxt_chunk(
                super::super::project::PROJECT_KEYWORD.to_string(),
                text.to_string(),
            )
            .unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(image.as_raw()).unwrap();
        drop(writer);
        out
    }

    fn screenshot_entry(label: &str) -> PinEntry {
        PinEntry {
            label: label.to_string(),
            source: Arc::new(PinSource::Screenshot { png: sample_png() }),
            content_width: 1.0,
            content_height: 1.0,
            scale: 1.0,
            opacity: 1.0,
            locked: false,
            above: false,
            position: None,
            origin: None,
            device_scale: 1.0,
            buffer_scale: 1.0,
            sharpen: Arc::new(SharpenSlot::default()),
        }
    }

    #[test]
    fn oversized_file_is_rejected_from_metadata_before_reading_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oversized.png");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(super::super::project::MAX_CONTAINER_BYTES as u64 + 1)
            .unwrap();
        drop(file);
        assert_eq!(
            read_png_file(&path).unwrap_err(),
            "PNG 文件超过 160 MiB 上限"
        );
    }

    #[test]
    fn canvas_base64_is_bounded_and_fully_validated() {
        assert!(decode_canvas_png("not base64").is_err());
        let truncated = STANDARD.encode([137, 80, 78, 71, 13, 10, 26, 10]);
        assert!(decode_canvas_png(&truncated).is_err());
        let png = crate::screenshot::encode_png(&[1, 2, 3, 255], 1, 1).unwrap();
        assert_eq!(decode_canvas_png(&STANDARD.encode(&png)).unwrap(), png);
    }

    #[test]
    fn project_entry_separates_preview_source_and_restore_payload() {
        let source = crate::screenshot::encode_png(&[255, 0, 0, 255], 1, 1).unwrap();
        let preview = crate::screenshot::encode_png(&[0, 0, 255, 255], 1, 1).unwrap();
        let project = super::super::project::PinProject::new(
            &source,
            &preview,
            super::super::project::RENDERER_VERSION,
            serde_json::json!([]),
            adjustments(),
        )
        .unwrap();
        let entry = PinEntry {
            label: "pin-image-project-test".to_string(),
            source: Arc::new(PinSource::Project {
                source_png: source.clone(),
                preview_png: preview.clone(),
                project,
            }),
            content_width: 1.0,
            content_height: 1.0,
            scale: 1.0,
            opacity: 1.0,
            locked: false,
            above: false,
            position: None,
            origin: None,
            device_scale: 1.0,
            buffer_scale: 1.0,
            sharpen: Arc::new(SharpenSlot::default()),
        };

        assert_eq!(source_png(&entry.source), Some(source.as_slice()));
        assert_eq!(display_png(&entry.source), Some(preview.as_slice()));
        let payload = payload_from_entry(entry).unwrap();
        assert_eq!(payload.image_base64, Some(STANDARD.encode(preview)));
        assert!(payload.initial_project.is_some());
    }

    #[test]
    fn pristine_project_save_reuses_the_stored_composite() {
        let source = crate::screenshot::encode_png(&[255, 0, 0, 255], 1, 1).unwrap();
        let preview = crate::screenshot::encode_png(&[0, 0, 255, 255], 1, 1).unwrap();
        let project = super::super::project::PinProject::new(
            &source,
            &preview,
            super::super::project::RENDERER_VERSION,
            serde_json::json!([]),
            adjustments(),
        )
        .unwrap();
        let entry = PinEntry {
            label: "pin-image-pristine-project".to_string(),
            source: Arc::new(PinSource::Project {
                source_png: source,
                preview_png: preview.clone(),
                project,
            }),
            content_width: 1.0,
            content_height: 1.0,
            scale: 1.0,
            opacity: 1.0,
            locked: false,
            above: false,
            position: None,
            origin: None,
            device_scale: 1.0,
            buffer_scale: 1.0,
            sharpen: Arc::new(SharpenSlot::default()),
        };

        let (clipboard, editable) =
            prepare_canvas_save(&entry, None, PinCanvasSaveMode::Editable, None).unwrap();
        assert_eq!(clipboard, preview);
        assert!(super::super::project::extract(&editable).unwrap().is_some());
        let (clipboard, flat) =
            prepare_canvas_save(&entry, None, PinCanvasSaveMode::Flat, None).unwrap();
        assert_eq!(clipboard, preview);
        assert_eq!(flat, preview);
        assert_eq!(super::super::project::extract(&flat).unwrap(), None);
    }

    #[test]
    fn ordinary_images_cannot_omit_the_latest_canvas_render() {
        let entry = screenshot_entry("pin-image-missing-render");
        assert!(prepare_canvas_save(&entry, None, PinCanvasSaveMode::Flat, None).is_err());
    }

    #[test]
    fn renderer_v2_generates_one_authoritative_png_for_clipboard_and_container() {
        let entry = screenshot_entry("pin-image-renderer-v2");
        let project = PinCanvasProject {
            renderer_version: super::super::render_v2::RENDERER_VERSION,
            source_width: 1,
            source_height: 1,
            annotations: serde_json::json!([]),
            adjustments: serde_json::json!({
                "grayscale": false,
                "brightness": 100,
                "contrast": 0,
                "saturation": 0,
                "cornerRadius": 0
            }),
        };

        let (clipboard, editable) = prepare_canvas_save(
            &entry,
            None,
            PinCanvasSaveMode::Editable,
            Some(project.clone()),
        )
        .unwrap();
        let pixels = image::load_from_memory_with_format(&clipboard, image::ImageFormat::Png)
            .unwrap()
            .into_rgba8();
        assert_eq!(pixels.as_raw(), &[20, 40, 60, 255]);
        let restored = super::super::project::extract(&editable).unwrap().unwrap();
        assert_eq!(restored.renderer_version, project.renderer_version);
        assert_eq!(restored.document.annotations, project.annotations);
        assert_eq!(restored.document.adjustments, project.adjustments);

        let copied = prepare_canvas_copy(&entry, None, Some(project.clone())).unwrap();
        assert_eq!(copied, clipboard);

        let uploaded = STANDARD.encode(&clipboard);
        assert!(prepare_canvas_save(
            &entry,
            Some(&uploaded),
            PinCanvasSaveMode::Editable,
            Some(project.clone())
        )
        .is_err());
        assert!(prepare_canvas_copy(&entry, Some(&uploaded), Some(project)).is_err());
    }

    #[test]
    fn editable_project_reopens_for_second_edit_and_flat_export() {
        let original_entry = screenshot_entry("pin-image-round-trip-source");
        let first_document = PinCanvasProject {
            renderer_version: super::super::render_v2::RENDERER_VERSION,
            source_width: 1,
            source_height: 1,
            annotations: serde_json::json!([]),
            adjustments: serde_json::json!({
                "grayscale": false,
                "brightness": 100,
                "contrast": 0,
                "saturation": 0,
                "cornerRadius": 0
            }),
        };
        let (first_preview, first_editable) = prepare_canvas_save(
            &original_entry,
            None,
            PinCanvasSaveMode::Editable,
            Some(first_document),
        )
        .unwrap();

        let directory = tempfile::tempdir().unwrap();
        let first_path = directory.path().join("first-editable.png");
        std::fs::write(&first_path, first_editable).unwrap();
        let PreparedPinImage {
            preview_png,
            project,
        } = prepare_open_selection(Some(first_path)).unwrap().unwrap();
        assert_eq!(
            image::load_from_memory_with_format(&preview_png, image::ImageFormat::Png)
                .unwrap()
                .into_rgba8(),
            image::load_from_memory_with_format(&first_preview, image::ImageFormat::Png)
                .unwrap()
                .into_rgba8()
        );

        let (source_png, restored_project) = project.unwrap();
        assert_eq!(source_png, sample_png());
        let reopened_entry = PinEntry {
            label: "pin-image-round-trip-reopened".to_string(),
            source: Arc::new(PinSource::Project {
                source_png,
                preview_png,
                project: restored_project,
            }),
            content_width: 1.0,
            content_height: 1.0,
            scale: 1.0,
            opacity: 1.0,
            locked: false,
            above: false,
            position: None,
            origin: None,
            device_scale: 1.0,
            buffer_scale: 1.0,
            sharpen: Arc::new(SharpenSlot::default()),
        };
        let second_document = PinCanvasProject {
            renderer_version: super::super::render_v2::RENDERER_VERSION,
            source_width: 1,
            source_height: 1,
            annotations: serde_json::json!([]),
            adjustments: serde_json::json!({
                "grayscale": false,
                "brightness": 50,
                "contrast": 0,
                "saturation": 0,
                "cornerRadius": 0
            }),
        };
        let (second_preview, second_editable) = prepare_canvas_save(
            &reopened_entry,
            None,
            PinCanvasSaveMode::Editable,
            Some(second_document.clone()),
        )
        .unwrap();
        assert_ne!(
            image::load_from_memory_with_format(&second_preview, image::ImageFormat::Png)
                .unwrap()
                .into_rgba8(),
            image::load_from_memory_with_format(&first_preview, image::ImageFormat::Png)
                .unwrap()
                .into_rgba8()
        );

        let second_path = directory.path().join("second-editable.png");
        std::fs::write(&second_path, &second_editable).unwrap();
        let reopened_again = prepare_open_selection(Some(second_path)).unwrap().unwrap();
        let (_, reopened_project) = reopened_again.project.unwrap();
        assert_eq!(
            reopened_project.document.adjustments,
            second_document.adjustments
        );

        let (flat_clipboard, flat_file) = prepare_canvas_save(
            &reopened_entry,
            None,
            PinCanvasSaveMode::Flat,
            Some(second_document),
        )
        .unwrap();
        assert_eq!(flat_clipboard, second_preview);
        assert_eq!(super::super::project::extract(&flat_file).unwrap(), None);
        assert_eq!(
            image::load_from_memory_with_format(&flat_file, image::ImageFormat::Png)
                .unwrap()
                .into_rgba8(),
            image::load_from_memory_with_format(&second_preview, image::ImageFormat::Png)
                .unwrap()
                .into_rgba8()
        );
    }

    #[test]
    fn cancelled_open_has_zero_manager_side_effects() {
        let manager = super::super::manager::PinManager::new();
        let result = complete_open_selection(prepare_open_selection(None), |_| {
            manager.insert(screenshot_entry("pin-image-cancel-should-not-insert"))?;
            Ok("pin-image-cancel-should-not-insert".to_string())
        });
        assert_eq!(result.unwrap(), None);
        assert_eq!(manager.len(), 0);
    }

    #[test]
    fn plain_v1_corrupt_and_future_projects_prepare_as_flat_images() {
        let directory = tempfile::tempdir().unwrap();
        let variants = [
            sample_png(),
            png_with_project_text(
                &serde_json::json!({"format":"clippy-pin-project","version":1}).to_string(),
            ),
            png_with_project_text("{broken"),
            png_with_project_text(
                &serde_json::json!({
                    "format":"clippy-pin-project",
                    "formatVersion":super::super::project::PROJECT_VERSION + 1
                })
                .to_string(),
            ),
        ];

        for (index, container) in variants.into_iter().enumerate() {
            let path = directory.path().join(format!("variant-{index}.png"));
            std::fs::write(&path, container).unwrap();
            let prepared = prepare_open_selection(Some(path)).unwrap().unwrap();
            assert!(prepared.project.is_none());
            assert_eq!(
                super::super::project::extract(&prepared.preview_png).unwrap(),
                None,
                "flat 预览不能残留工程块"
            );
        }
    }

    #[test]
    fn invalid_oversized_and_unreadable_files_never_insert_entries() {
        let manager = super::super::manager::PinManager::new();
        let directory = tempfile::tempdir().unwrap();

        let assert_prepare_failure = |path: std::path::PathBuf| {
            let result = complete_open_selection(prepare_open_selection(Some(path)), |_| {
                manager.insert(screenshot_entry("pin-image-invalid-should-not-insert"))?;
                Ok("pin-image-invalid-should-not-insert".to_string())
            });
            assert!(result.is_err());
            assert_eq!(manager.len(), 0);
        };

        let invalid = directory.path().join("invalid.png");
        std::fs::write(&invalid, b"not png").unwrap();
        assert_prepare_failure(invalid);

        let oversized = directory.path().join("oversized.png");
        let file = std::fs::File::create(&oversized).unwrap();
        file.set_len(super::super::project::MAX_CONTAINER_BYTES as u64 + 1)
            .unwrap();
        drop(file);
        assert_prepare_failure(oversized);

        let missing = directory.path().join("missing.png");
        assert_prepare_failure(missing);
    }

    #[test]
    fn post_insert_window_failure_rolls_back_manager_entry() {
        let manager = super::super::manager::PinManager::new();
        let label = "pin-image-rollback-test";
        let result = insert_pin_with_rollback(&manager, screenshot_entry(label), |_| {
            Err::<(), String>("simulated native window failure".to_string())
        });
        assert_eq!(result.unwrap_err(), "simulated native window failure");
        assert_eq!(manager.len(), 0);
        assert!(manager.get(label).is_err());
    }
}
