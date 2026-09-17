use super::model::{
    validate_label, PinEntry, PinOrigin, PinPayload, PinSource, PinState, PinUpdate, SharpenFinish,
    SharpenSlot,
};
#[cfg(test)]
use super::output::decode_canvas_png;
use super::output::{
    copy_source, display_png, image_bytes, prepare_pin_copy, prepare_pin_save, source_png,
    PinCanvasProject, PinCanvasSaveMode, PinCanvasSaveResult,
};
use super::project_file::prepare_pin_project_file;
#[cfg(test)]
use super::project_file::{read_png_file, PreparedPinImage};
use super::window::{
    content_buffer_scale, content_device_scale, create_pin_window, fit_content_size,
    keep_pin_above, origin_content_size, resize_pin_window, reveal_pin_window,
};
use crate::commands::AppState;
use crate::models::ContentType;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use tauri::{Emitter, Manager, State};

/// 截图贴图在跨过原生建窗边界后的失败不能安全重试：窗口 API 的失败与销毁成功
/// 都不能证明原生窗口没有留下。普通截图 IPC 仍只见到原先的字符串，这个类型只供
/// 需要决定重试策略的 crate 内调用方使用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScreenshotPinCreateError {
    /// 尚未调用原生窗口 builder，确认没有创建贴图窗口。
    NotCreated { message: String },
    /// 已调用 builder；即使尽力销毁了已知窗口，结果仍不能确认。
    Uncertain {
        attempted_label: String,
        message: String,
    },
}

impl ScreenshotPinCreateError {
    fn not_created(error: impl fmt::Display) -> Self {
        Self::NotCreated {
            message: error.to_string(),
        }
    }

    fn uncertain(attempted_label: String, message: String) -> Self {
        Self::Uncertain {
            attempted_label,
            message,
        }
    }

    /// 原生窗口可能已创建，调用方不得自动重试 Pin。
    pub(crate) fn is_uncertain(&self) -> bool {
        matches!(self, Self::Uncertain { .. })
    }

    /// 不确定失败所尝试创建的窗口 label，供诊断和后续保守收敛使用。
    #[cfg(test)]
    pub(crate) fn attempted_label(&self) -> Option<&str> {
        match self {
            Self::NotCreated { .. } => None,
            Self::Uncertain {
                attempted_label, ..
            } => Some(attempted_label),
        }
    }
}

impl fmt::Display for ScreenshotPinCreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotCreated { message } | Self::Uncertain { message, .. } => {
                formatter.write_str(message)
            }
        }
    }
}

/// 窗口查询会同步等待主事件循环；持 transition 的入口统一离开 UI 线程执行。
async fn run_pin_window_work<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| format!("贴图窗口任务失败: {error}"))?
}

pub(super) async fn pin_clip(id: i64, app_handle: tauri::AppHandle) -> Result<String, String> {
    run_pin_window_work(move || {
        let state = app_handle.state::<AppState>();
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
            let _ = window.destroy();
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
    })
    .await
}

fn screenshot_entry(
    label: String,
    png: Arc<Vec<u8>>,
    content_width: f64,
    content_height: f64,
    origin: Option<PinOrigin>,
    device_scale: f64,
    buffer_scale: f64,
) -> PinEntry {
    PinEntry {
        label,
        source: Arc::new(PinSource::Screenshot { png }),
        content_width,
        content_height,
        scale: 1.0,
        opacity: 1.0,
        locked: false,
        above: false,
        position: None,
        origin,
        device_scale,
        buffer_scale,
        sharpen: Arc::new(SharpenSlot::default()),
    }
}

fn with_validated_screenshot_png<T, F>(
    png: Arc<Vec<u8>>,
    next: F,
) -> Result<T, ScreenshotPinCreateError>
where
    F: FnOnce(Arc<Vec<u8>>, u32, u32) -> Result<T, ScreenshotPinCreateError>,
{
    let (width, height) = super::image_validation::validate_strict_png(
        png.as_slice(),
        super::project::MAX_RENDERED_PNG_BYTES,
        "截图 PNG",
    )
    .map_err(ScreenshotPinCreateError::not_created)?;
    next(png, width, height)
}

/// 与普通截图入口共用同一条建条目/建窗路径，但接管调用方已经持有的 PNG `Arc`。
/// `origin` 是这张图在屏幕上原本占的矩形（逻辑像素）。截图覆盖层知道选区落在哪，
/// 于是贴图能贴回原处、原尺寸；不知道来源的图片传 `None`，落回光标附近。
pub(crate) fn create_screenshot_pin_shared(
    png: Arc<Vec<u8>>,
    origin: Option<PinOrigin>,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<String, ScreenshotPinCreateError> {
    with_validated_screenshot_png(png, |png, width, height| {
        // 截图/查看器每次使用唯一 label，manager 自身原子插入足够。
        // 不持固定 clip 复用窗口的全局 transition：平台尺寸查询/build 会等 UI，
        // 而 UI 上已有 Pin 的同步 ready/update 命令也要该锁，持有会形成 ABBA。
        let label = format!("pin-image-{}", crate::image_io::unique_image_id());
        let origin = origin.and_then(PinOrigin::sanitized);
        let (content_width, content_height) = match origin {
            Some(origin) => origin_content_size(app_handle, origin),
            None => fit_content_size(app_handle, width as f64, height as f64),
        };
        let entry = screenshot_entry(
            label.clone(),
            png,
            content_width,
            content_height,
            origin,
            content_device_scale(app_handle, origin),
            content_buffer_scale(app_handle, origin),
        );
        state
            .pin_manager
            .insert(entry)
            .map_err(ScreenshotPinCreateError::not_created)?;
        // 同上：补偿与开窗并行，抢在前端来取 payload 之前算完。
        let inserted = match state.pin_manager.get(&label) {
            Ok(entry) => entry,
            Err(error) => {
                let _ = state.pin_manager.remove(&label);
                return Err(ScreenshotPinCreateError::not_created(error));
            }
        };
        spawn_sharpen(app_handle, &inserted);
        if let Err(error) =
            create_pin_window(app_handle, &label, content_width, content_height, origin)
        {
            return Err(screenshot_window_failure(&state.pin_manager, label, error));
        }
        Ok(label)
    })
}

/// 原生 builder 一旦被调用，Tauri/Wry 只能给我们操作是否投递成功，不能给出窗口已经
/// 消失的确认。因此 manager 的清理只是释放 payload 与取消清晰化，不能把失败降为可重试。
fn screenshot_window_failure(
    manager: &super::manager::PinManager,
    label: String,
    error: super::error::PinError,
) -> ScreenshotPinCreateError {
    let message = crate::error::report("创建截图贴图窗口失败", error);
    let rollback_label = label.clone();
    uncertain_after_window_attempt(label, message, || {
        manager.remove(&rollback_label).map(|_| ())
    })
}

/// builder 调用后的任意清理都只是 best effort。即使清理动作返回 Ok，也不能证明窗口已被
/// 事件循环销毁，所以这个纯 helper 始终保留 Uncertain 分类。
fn uncertain_after_window_attempt<E>(
    label: String,
    message: String,
    rollback: impl FnOnce() -> Result<(), E>,
) -> ScreenshotPinCreateError {
    let _ = rollback();
    ScreenshotPinCreateError::uncertain(label, message)
}

/// 从文件恢复出的可编辑工程创建贴图。所有读取和工程校验在调用前完成，因此插入 manager
/// 后唯一可能失败的是建窗；失败路径会尽力移除 manager entry（释放 payload、取消清晰化）。
/// 原生窗口的 destroy 只是 best effort，不能据此断言没有遗留的半初始化窗口。
fn create_opened_project_pin(
    preview_png: Vec<u8>,
    project: (Vec<u8>, super::project::RuntimeProject),
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
    let (source_png, project) = project;
    let source = PinSource::Project {
        source_png,
        preview_png,
        project,
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

/// PinManager entry 的插入与回滚是原子的：后续步骤失败时尽力删除刚插入的 payload。
/// 这不是 manager 与原生建窗的全局事务；原生窗口 destroy 只能 best effort，调用方不能
/// 用这里的清理结果推断窗口已消失。抽成纯状态 helper 后，窗口系统不可用的单元测试环境
/// 也能钉住 payload rollback 不变量。
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
pub(super) async fn get_pin_toolbar_bounds(
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

pub(super) fn get_pin_payload(
    label: String,
    state: State<'_, AppState>,
) -> Result<PinPayload, String> {
    validate_label(&label)?;
    payload_from_entry(state.pin_manager.get(&label)?)
}

pub(super) async fn pin_ready(label: String, app_handle: tauri::AppHandle) -> Result<(), String> {
    run_pin_window_work(move || {
        let state = app_handle.state::<AppState>();
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
        Ok(())
    })
    .await
}

/// 改缩放/不透明度/锁定，应答只带变了的那几个字段（见 `PinState`）。
///
/// **这是每帧都会走的路**：滚轮缩放时前端按 rAF 合并后仍是一帧一次。命令在
/// blocking pool 中执行，但这条路仍不能有整张图的 base64、或多余文件读与 D-Bus 握手
/// （摆位那侧的缓存见 `capture::shell_extension::place_window`）。
pub(super) async fn update_pin(
    label: String,
    update: PinUpdate,
    app_handle: tauri::AppHandle,
) -> Result<PinState, String> {
    run_pin_window_work(move || {
        let state = app_handle.state::<AppState>();
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
    })
    .await
}

pub(super) fn copy_pin(label: String, state: State<'_, AppState>) -> Result<(), String> {
    validate_label(&label)?;
    let entry = state.pin_manager.get(&label)?;
    copy_source(
        &entry.source,
        |item, image| crate::commands::write_clip_snapshot_to_clipboard(item, image, &state),
        crate::image_io::copy_png_to_clipboard,
    )
}

pub(super) fn save_pin(label: String, state: State<'_, AppState>) -> Result<String, String> {
    validate_label(&label)?;
    let png = image_bytes(&state.pin_manager.get(&label)?)?;
    let path = crate::image_io::save_png(&png, "clippy-pin", &state.save_target())?;
    Ok(path.to_string_lossy().to_string())
}

/// 贴图的**原图**（base64 PNG），只供前端 Canvas 交互预览；v2 最终导出由后端读取同一可信原图。
///
/// **为什么不能用屏上那张。** `pin-frame` 给前端的显示图优先是清晰度补偿版
/// （见 `spawn_sharpen`）：它按**缓冲区分辨率**渲染（2560x1440 的贴图会是
/// 3413x1920），而且为"随后被合成器缩小 0.75"预先做了反投影锐化。那串字节只适合
/// 贴到那一个窗口的那一块缓冲区里，单独看是偏大且过冲的。拿它当导出底图，存出来的
/// 就是一张大一圈、发硬的图——这违反 `super::resample` 模块头写的
/// "复制与保存永远用原图"。
///
/// 所以导出时单独来取一次。**按需取而不是常驻**：导出是低频动作，而贴图窗口可以开
/// 好几个，让每个窗口长期多驻一份原图和刚做的"上屏后释放补偿结果"正好相反。
pub(super) fn get_pin_source_image(
    label: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    validate_label(&label)?;
    let entry = state.pin_manager.get(&label)?;
    Ok(source_png(&entry.source).map(|png| STANDARD.encode(png)))
}

/// 把最新合成图保存为可编辑工程或安全扁平 PNG。文件先原子落盘，剪贴板随后写入；后者
/// 失败不会谎报文件失败，而是通过结构化结果单独告知调用方。
pub(super) async fn save_pin_canvas(
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
        let (png, to_disk) = prepare_pin_save(&entry, png_base64.as_deref(), mode, project)?;
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

/// 已编辑贴图的 Copy/Ctrl+C：只把最新合成像素送入剪贴板，不携带 iTXt。
pub(super) async fn copy_pin_canvas(
    label: String,
    png_base64: Option<String>,
    project: Option<PinCanvasProject>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_label(&label)?;
    let entry = state.pin_manager.get(&label)?;
    tauri::async_runtime::spawn_blocking(move || {
        let png = prepare_pin_copy(&entry, png_base64.as_deref(), project)?;
        crate::image_io::copy_png_to_clipboard(&png)
    })
    .await
    .map_err(|error| format!("画布复制线程异常: {error}"))?
}

/// 从主窗口拖入的单个文件恢复 Clippy 可编辑贴图工程。
/// 普通 PNG、非 PNG 和不兼容工程返回 `None`，绝不作为普通贴图绕过剪贴板队列。
pub(super) async fn open_pin_project_file(
    path: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let prepared =
        tauri::async_runtime::spawn_blocking(move || prepare_pin_project_file(Path::new(&path)))
            .await
            .map_err(|error| format!("工程读取线程异常: {error}"))??;
    let Some(prepared) = prepared else {
        return Ok(None);
    };
    create_opened_project_pin(prepared.preview_png, prepared.project, &app_handle, &state).map(Some)
}

pub(super) async fn close_pin(label: String, app_handle: tauri::AppHandle) -> Result<(), String> {
    run_pin_window_work(move || {
        let state = app_handle.state::<AppState>();
        validate_label(&label)?;
        let _transition = state
            .pin_transition
            .lock()
            .map_err(|error| error.to_string())?;
        if let Some(window) = app_handle.get_webview_window(&label) {
            // 此命令只由已经确认保存/放弃的前端调用。destroy 不再发 CloseRequested，
            // 避免原生保护把最终关闭再送回确认框。
            window.destroy().map_err(|error| error.to_string())?;
        }
        let _ = state.pin_manager.remove(&label)?;
        Ok(())
    })
    .await
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
    let (kind, text, color, can_save, initial_project) = match &*entry.source {
        PinSource::Clip { item, .. } => match item.content_type {
            ContentType::Image => ("image", None, None, true, None),
            ContentType::Text => {
                let text = item.text_content.clone();
                let color = text.as_deref().and_then(super::color::parse_pin_color);
                let kind = if color.is_some() { "color" } else { "text" };
                (kind, text, color, false, None)
            }
            ContentType::Html => ("text", item.text_content.clone(), None, false, None),
        },
        PinSource::Screenshot { .. } => ("image", None, None, true, None),
        PinSource::Project { project, .. } => {
            ("image", None, None, true, Some(project.initial_payload()))
        }
    };
    Ok(PinPayload {
        label: entry.label,
        kind,
        text,
        color,
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
    /// 前端把版本号放进 `pin-frame` URL，既绕过 WebKit 缓存，也让协议知道这是补偿图请求。
    revision: u8,
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
        match super::resample::compensated_png_after_wait(png, geometry, || slot.is_cancelled()) {
            Ok(Some(bytes)) => {
                let finish = slot.finish(bytes);
                if finish == SharpenFinish::Cancelled {
                    log::debug!("{label} 在补偿期间已关闭，丢弃计算结果");
                    return;
                }
                let late = finish == SharpenFinish::NeedsEvent;
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
                    revision: 1,
                };
                match slot.publish_if_active(|| {
                    app_handle.emit_to(label.as_str(), PIN_IMAGE_SHARPENED, payload)
                }) {
                    None => log::debug!("{label} 在事件发布前已关闭，丢弃旧结果"),
                    Some(Err(error)) => log::warn!("推送贴图清晰版失败: {error}"),
                    Some(Ok(())) => {}
                }
            }
            Ok(None) => log::debug!("{label} 已关闭，取消排队中的清晰度补偿"),
            Err(error) => log::warn!("贴图清晰度补偿失败，保留原图: {error}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn pin_canvas_save_result_matches_the_shared_json_fixture() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../src/tests/fixtures/ipc-contract/pin-canvas-save-result.json"
        ));
        let result: PinCanvasSaveResult = serde_json::from_str(source).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(source).unwrap();
        assert_eq!(serde_json::to_value(result).unwrap(), fixture);
    }

    fn clip_entry(content_type: ContentType, text: Option<&str>) -> PinEntry {
        PinEntry {
            label: "pin-clip-color-contract".to_string(),
            source: Arc::new(PinSource::Clip {
                item: crate::models::ClipItem {
                    id: 1,
                    content_type,
                    text_content: text.map(str::to_owned),
                    html_content: None,
                    image_data: None,
                    content_hash: "color-contract".to_string(),
                    is_favorite: false,
                    is_sensitive: false,
                    created_at: 0,
                    byte_size: 0,
                },
                image: None,
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
        }
    }

    #[test]
    fn deleted_history_does_not_remove_the_pin_copy_snapshot() {
        let storage = crate::storage::StorageEngine::new_in_memory().unwrap();
        for (content_type, text, html, image) in [
            (ContentType::Text, Some("snapshot"), None, None),
            (
                ContentType::Html,
                Some("html text"),
                Some("<b>html text</b>"),
                None,
            ),
            (ContentType::Image, None, None, Some(sample_png())),
        ] {
            let mut item = storage
                .insert_clip(
                    &content_type,
                    text,
                    html,
                    image.as_deref(),
                    &format!("snapshot-{content_type:?}"),
                    1,
                    false,
                )
                .unwrap();
            let png = item.image_data.take();
            let id = item.id;
            let source = PinSource::Clip { item, image: png };
            storage.delete_clip(id).unwrap();
            assert!(storage.get_clip_by_id(id).is_err());
            let called = AtomicBool::new(false);
            copy_source(
                &source,
                |snapshot, bytes| {
                    called.store(true, Ordering::SeqCst);
                    assert_eq!(snapshot.text_content.as_deref(), text);
                    assert_eq!(snapshot.html_content.as_deref(), html);
                    assert_eq!(bytes, image.as_deref());
                    Ok(())
                },
                |_| panic!("历史 Pin 应直接复制快照"),
            )
            .unwrap();
            assert!(called.load(Ordering::SeqCst));
        }
    }

    type CreateScreenshotPinSharedFn = fn(
        Arc<Vec<u8>>,
        Option<PinOrigin>,
        &tauri::AppHandle,
        &crate::commands::AppState,
    ) -> Result<String, super::ScreenshotPinCreateError>;

    const _: CreateScreenshotPinSharedFn = super::create_screenshot_pin_shared;

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
        super::screenshot_entry(
            label.to_string(),
            Arc::new(sample_png()),
            1.0,
            1.0,
            None,
            1.0,
            1.0,
        )
    }

    #[test]
    fn color_payload_classifies_only_valid_plain_text_and_keeps_original_text() {
        let color = payload_from_entry(clip_entry(ContentType::Text, Some("\t#AbC\t")))
            .expect("payload 应成功构建");
        assert_eq!(color.kind, "color");
        assert_eq!(color.text.as_deref(), Some("\t#AbC\t"));
        assert!(!color.can_save);
        assert!(color.initial_project.is_none());
        let parsed = color.color.expect("color payload 必须携带已规范化颜色");
        assert_eq!(
            (parsed.red, parsed.green, parsed.blue, parsed.alpha),
            (170, 187, 204, 255)
        );
        assert_eq!(parsed.canonical, "#aabbccff");
        assert_eq!(
            serde_json::to_value(parsed).expect("颜色 IPC 值必须可序列化"),
            serde_json::json!({
                "red": 170,
                "green": 187,
                "blue": 204,
                "alpha": 255,
                "canonical": "#aabbccff",
            })
        );

        let invalid = payload_from_entry(clip_entry(ContentType::Text, Some("rgb(256,0,0)")))
            .expect("无效颜色应降级为普通文本");
        assert_eq!(invalid.kind, "text");
        assert_eq!(invalid.text.as_deref(), Some("rgb(256,0,0)"));
        assert!(invalid.color.is_none());
        assert!(!invalid.can_save);
        assert!(invalid.initial_project.is_none());

        let missing = payload_from_entry(clip_entry(ContentType::Text, None))
            .expect("无文本条目应保持普通文本 payload");
        assert_eq!(missing.kind, "text");
        assert!(missing.text.is_none());
        assert!(missing.color.is_none());
    }

    #[test]
    fn non_text_payloads_never_expose_color() {
        let image = payload_from_entry(clip_entry(ContentType::Image, Some("#abc")))
            .expect("图片 payload 应成功构建");
        assert_eq!(image.kind, "image");
        assert!(image.color.is_none());

        let html = payload_from_entry(clip_entry(ContentType::Html, Some("#abc")))
            .expect("HTML payload 应成功构建");
        assert_eq!(html.kind, "text");
        assert_eq!(html.text.as_deref(), Some("#abc"));
        assert!(html.color.is_none());
        assert!(!html.can_save);
    }

    #[test]
    fn shared_screenshot_source_keeps_arc_identity_in_manager() {
        let png = Arc::new(sample_png());
        let manager = super::super::manager::PinManager::new();
        manager
            .insert(super::screenshot_entry(
                "pin-image-shared-source".to_string(),
                png.clone(),
                1.0,
                1.0,
                None,
                1.0,
                1.0,
            ))
            .unwrap();

        let entry = manager.get("pin-image-shared-source").unwrap();
        let PinSource::Screenshot { png: stored } = &*entry.source else {
            panic!("应保留截图来源");
        };
        assert!(Arc::ptr_eq(stored, &png));
        assert_eq!(stored.as_slice(), png.as_slice());
    }

    #[test]
    fn successful_preflight_forwards_the_same_arc_and_dimensions() {
        let png = Arc::new(sample_png());
        let forwarded = super::with_validated_screenshot_png(Arc::clone(&png), |received, w, h| {
            assert!(Arc::ptr_eq(&received, &png));
            Ok((w, h))
        })
        .expect("合法截图应通过预检");
        assert_eq!(forwarded, (1, 1));
    }

    #[test]
    fn preflight_failures_are_not_created_and_never_start_side_effects() {
        let side_effect_started = AtomicBool::new(false);
        let error =
            super::with_validated_screenshot_png(Arc::new(b"invalid png".to_vec()), |_, _, _| {
                side_effect_started.store(true, Ordering::SeqCst);
                Ok(())
            })
            .unwrap_err();
        assert!(!error.is_uncertain());
        assert_eq!(error.attempted_label(), None);
        assert!(error.to_string().contains("PNG"));
        assert!(!side_effect_started.load(Ordering::SeqCst));
    }

    #[test]
    fn post_builder_failure_stays_uncertain_after_successful_rollback() {
        let manager = super::super::manager::PinManager::new();
        let label = "pin-image-window-failure".to_string();
        manager.insert(screenshot_entry(&label)).unwrap();

        let error = super::screenshot_window_failure(
            &manager,
            label.clone(),
            super::super::error::PinError::window("position failed"),
        );
        assert!(error.is_uncertain());
        assert_eq!(error.attempted_label(), Some(label.as_str()));
        assert_eq!(error.to_string(), "position failed");
        assert!(manager.get(&label).is_err(), "entry 必须尽力回滚");
    }

    #[test]
    fn cleanup_outcome_never_downgrades_a_builder_boundary_failure() {
        for (stage, cleanup) in [("build", Ok(())), ("position", Err("destroy failed"))] {
            let label = format!("pin-image-{stage}");
            let error = super::uncertain_after_window_attempt(
                label.clone(),
                format!("{stage} failed"),
                || cleanup,
            );
            assert!(error.is_uncertain(), "{stage} 失败不能自动重试");
            assert_eq!(error.attempted_label(), Some(label.as_str()));
        }
    }

    #[test]
    fn legacy_and_shared_error_text_stay_compatible() {
        let error = super::ScreenshotPinCreateError::uncertain(
            "pin-image-legacy-text".to_string(),
            "创建截图贴图窗口失败".to_string(),
        );
        assert_eq!(error.to_string(), "创建截图贴图窗口失败");
    }

    #[test]
    fn legacy_vec_source_path_and_image_bytes_remain_owned() {
        let png = sample_png();
        let entry = super::screenshot_entry(
            "pin-image-legacy-source".to_string(),
            Arc::new(png.clone()),
            1.0,
            1.0,
            None,
            1.0,
            1.0,
        );
        assert_eq!(super::source_png(&entry.source), Some(png.as_slice()));

        let mut copied = super::image_bytes(&entry).unwrap();
        assert_eq!(copied, png);
        copied[0] ^= 0xff;
        assert_eq!(super::source_png(&entry.source), Some(png.as_slice()));
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
        let (runtime_source, project) = project.into_runtime().unwrap();
        assert_eq!(runtime_source, source);
        let entry = PinEntry {
            label: "pin-image-project-test".to_string(),
            source: Arc::new(PinSource::Project {
                source_png: runtime_source,
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
        assert_eq!(payload.kind, "image");
        assert!(payload.can_save);
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
        let (source, project) = project.into_runtime().unwrap();
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
            prepare_pin_save(&entry, None, PinCanvasSaveMode::Editable, None).unwrap();
        assert_eq!(clipboard, preview);
        assert!(super::super::project::extract(&editable).unwrap().is_some());
        let (clipboard, flat) =
            prepare_pin_save(&entry, None, PinCanvasSaveMode::Flat, None).unwrap();
        assert_eq!(clipboard, preview);
        assert_eq!(flat, preview);
        assert_eq!(super::super::project::extract(&flat).unwrap(), None);
    }

    #[test]
    fn ordinary_images_cannot_omit_the_latest_canvas_render() {
        let entry = screenshot_entry("pin-image-missing-render");
        assert!(prepare_pin_save(&entry, None, PinCanvasSaveMode::Flat, None).is_err());
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

        let (clipboard, editable) = prepare_pin_save(
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

        let copied = prepare_pin_copy(&entry, None, Some(project.clone())).unwrap();
        assert_eq!(copied, clipboard);

        let uploaded = STANDARD.encode(&clipboard);
        assert!(prepare_pin_save(
            &entry,
            Some(&uploaded),
            PinCanvasSaveMode::Editable,
            Some(project.clone())
        )
        .is_err());
        assert!(prepare_pin_copy(&entry, Some(&uploaded), Some(project)).is_err());
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
        let (first_preview, first_editable) = prepare_pin_save(
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
        } = prepare_pin_project_file(&first_path).unwrap().unwrap();
        assert_eq!(
            image::load_from_memory_with_format(&preview_png, image::ImageFormat::Png)
                .unwrap()
                .into_rgba8(),
            image::load_from_memory_with_format(&first_preview, image::ImageFormat::Png)
                .unwrap()
                .into_rgba8()
        );

        let (source_png, restored_project) = project;
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
        let (second_preview, second_editable) = prepare_pin_save(
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
        let reopened_again = prepare_pin_project_file(&second_path).unwrap().unwrap();
        let (_, reopened_project) = reopened_again.project;
        assert_eq!(
            reopened_project.initial_payload().document.adjustments,
            second_document.adjustments
        );

        let (flat_clipboard, flat_file) = prepare_pin_save(
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
    fn plain_v1_corrupt_and_future_projects_are_not_opened() {
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
            assert!(prepare_pin_project_file(&path).unwrap().is_none());
        }

        let non_png = directory.path().join("project.txt");
        std::fs::write(&non_png, b"not read").unwrap();
        assert!(prepare_pin_project_file(&non_png).unwrap().is_none());
    }

    #[test]
    fn invalid_oversized_and_unreadable_files_never_insert_entries() {
        let directory = tempfile::tempdir().unwrap();

        let assert_prepare_failure = |path: std::path::PathBuf| {
            assert!(prepare_pin_project_file(&path).is_err());
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

    #[test]
    fn concurrent_screenshot_labels_insert_without_replacing_another_entry() {
        let manager = Arc::new(super::super::manager::PinManager::new());
        let barrier = Arc::new(std::sync::Barrier::new(16));
        let workers: Vec<_> = (0..16)
            .map(|_| {
                let manager = Arc::clone(&manager);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let label = format!("pin-image-{}", crate::image_io::unique_image_id());
                    let png = Arc::new(sample_png());
                    super::with_validated_screenshot_png(png, |png, width, height| {
                        let entry = super::screenshot_entry(
                            label.clone(),
                            png,
                            width as f64,
                            height as f64,
                            None,
                            1.0,
                            1.0,
                        );
                        manager
                            .insert(entry)
                            .map_err(super::ScreenshotPinCreateError::not_created)?;
                        Ok(())
                    })
                    .unwrap();
                    assert!(manager.get(&label).is_ok());
                    label
                })
            })
            .collect();
        let labels: std::collections::HashSet<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(labels.len(), 16);
        assert_eq!(manager.len(), 16);
        let duplicate = labels.iter().next().unwrap();
        assert!(manager.insert(screenshot_entry(duplicate)).is_err());
        assert_eq!(manager.len(), 16);
    }

    #[tokio::test]
    async fn window_transition_waiters_do_not_block_the_ui_task_pump() {
        use std::sync::{mpsc, Mutex};
        use std::time::Duration;
        let transition = Arc::new(Mutex::new(()));
        let entered = Arc::new(tokio::sync::Notify::new());
        let (ui_ack, wait_for_ui) = mpsc::channel();
        let first = {
            let transition = Arc::clone(&transition);
            let entered = Arc::clone(&entered);
            tokio::spawn(super::run_pin_window_work(move || {
                let _guard = transition.lock().unwrap();
                entered.notify_one();
                wait_for_ui
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|_| "ui_deadlock".to_string())?;
                Ok("getter_completed")
            }))
        };
        entered.notified().await;
        let second = tokio::spawn(super::run_pin_window_work(move || {
            let _guard = transition.lock().unwrap();
            Ok("next_pin_command")
        }));
        // 当前单线程执行器模拟 UI 派发：另一条命令等待同锁时仍能响应 getter。
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        ui_ack.send(()).unwrap();
        assert_eq!(first.await.unwrap().unwrap(), "getter_completed");
        assert_eq!(second.await.unwrap().unwrap(), "next_pin_command");
    }
}
