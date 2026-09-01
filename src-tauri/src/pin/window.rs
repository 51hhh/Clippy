use super::error::PinError;
use super::model::{window_marker, PinEntry, PinOrigin};
use tauri::{LogicalPosition, Manager, PhysicalPosition, PhysicalSize, Position, Size};

/// 内容区四周留给投影的空隙。同时也是内容区相对窗口原点的左/上偏移
/// （见 `src/react/pin/pin.css` 的 `.pin-media { inset: 12px 56px 60px 12px }`），
/// 所以"把内容区盖在原始矩形上"就是把窗口摆到原始矩形减去这个偏移的位置。
const SHADOW_GUTTER: f64 = 12.0;
const CONTROLS_GUTTER: f64 = 44.0;
const TOOLBAR_GUTTER: f64 = 48.0;
const MIN_IMAGE_WIDTH: f64 = 180.0;
const MIN_IMAGE_HEIGHT: f64 = 120.0;

/// 竖排工具条要的最小窗口高度。
///
/// 工具条钉在窗口右上角、按钮 28 px 竖着排：放大 / 比例 / 缩小 / 分隔线 / 锁定 /
/// 不透明度 / 保存 / 复制 / 关闭，连内边距和 8 px 上边距约 249 px。而窗口高度是
/// `内容高 × scale + 72`，于是内容不到 180 px 高的贴图（随手框一个小按钮就是这样）
/// 会把工具条切掉一截——按钮点不到，贴图只能靠 Esc 关。
///
/// 因此给窗口一个高度下限。**多出来的高度不许改变内容的位置**：`.pin-media` 为此
/// 按内容尺寸显式定宽高、贴在左上角，而不是用 inset 撑满窗口（撑满会让图片在变高的
/// 框里居中，"贴回原处"当场对不上）。多出来的那块是透明的，看不见。
const MIN_OUTER_HEIGHT: f64 = 252.0;

pub(super) fn create_pin_window(
    app: &tauri::AppHandle,
    label: &str,
    content_width: f64,
    content_height: f64,
    origin: Option<PinOrigin>,
) -> Result<(), PinError> {
    let (outer_width, outer_height) = outer_size(content_width, content_height, 1.0);
    let window = tauri::WebviewWindowBuilder::new(
        app,
        label,
        tauri::WebviewUrl::App(format!("pin.html?label={label}").into()),
    )
    // 标题只做 GNOME Shell 扩展的查找键，界面上看不到（无装饰 + 不进任务栏）。
    .title(window_marker(label))
    .inner_size(outer_width, outer_height)
    .decorations(false)
    // 置顶是可选项、默认关（见 `PinEntry::above`）。建窗时一律不置顶，
    // 开着图钉的贴图由 `reveal_pin_window` 那一步进 above 层。
    .always_on_top(false)
    .transparent(true)
    .shadow(false)
    .skip_taskbar(true)
    .resizable(false)
    .visible(false)
    .center()
    .build()
    .map_err(PinError::window)?;
    if let Err(error) = position_new_pin_window(app, &window, outer_width, outer_height, origin) {
        if let Err(close_error) = window.close() {
            log::warn!("关闭定位失败的贴图窗口失败: {close_error}");
        }
        return Err(error);
    }
    crate::pin_window::configure_pin_window(&window);
    Ok(())
}

/// 显示贴图窗口，并让它落到该去的位置、压在别的窗口上面。
///
/// 顺序不能换：Wayland 下只有窗口真的映射之后 Shell 里才有对应的 MetaWindow，
/// 扩展才找得到它，所以摆放必须在 `show()` 之后。但 `show()` 返回**不等于**映射完成
/// （见 `PLACEMENT_RETRY_DELAYS_MS`），所以第一次摆放几乎必然落空，要重试。
pub(super) fn reveal_pin_window(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    entry: &PinEntry,
) -> Result<(), PinError> {
    window.show().map_err(PinError::window)?;
    window.set_focus().map_err(PinError::window)?;
    let logical = pin_target_position(app, entry);
    if let Placement::NotMappedYet { generation } = keep_pin_above(window, logical, entry.above) {
        retry_placement(
            window.label().to_string(),
            shell_target(logical),
            generation,
            entry.above,
        );
    }
    Ok(())
}

/// 一次摆放的结果，调用方据此决定要不要等窗口出现再试。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Placement {
    /// 扩展已经把窗口摆好、压到最上层了。
    Done,
    /// 扩展在，但 Shell 里还没有这个窗口。`generation` 是这次请求的代次，
    /// 重试要带着它，好在有更新的请求出现时自己作废。
    NotMappedYet { generation: u64 },
    /// 没有扩展这条路（X11、非 GNOME、扩展装了还没生效），已退回 Tauri 自己那套。
    NoExtension,
}

/// Shell 认出一个刚显示的窗口要等多久：实测 GTK `show()` 之后 **+0 ms 时
/// `PlaceWindow` 返回 false，几十毫秒后才返回 true**（本机 GNOME 50 多次采样 28~137 ms，
/// 系统忙时偏后）。MetaWindow 是合成器那边建的，客户端的 `show()` 返回时它还不存在——
/// 这正是"贴图不回原位、也不置顶"的根因：唯一一次摆放尝试恰好落在那个空窗里，
/// 于是 `move_frame` 与 `make_above` 两个动作全被跳过，窗口留在 Mutter 给的居中位置。
///
/// 于是改成退避重试。等待必须在后台线程上：`create_pin` / `update_pin` 都是同步命令，
/// 跑在 GTK 主线程，在这里睡几十毫秒就是把整个界面卡住几十毫秒。
const PLACEMENT_RETRY_DELAYS_MS: [u64; 8] = [30, 50, 80, 120, 200, 300, 500, 800];

/// 后台等窗口在 Shell 里出现，出现了就摆好。只在 `NotMappedYet` 时起一条线程，
/// 线程最多活 `PLACEMENT_RETRY_DELAYS_MS` 之和（约 2 秒）。
///
/// 摆成功之后**再补摆一次**（隔一个退避步）。实测 Mutter 自己的初始摆放在窗口刚出现在
/// Shell 里时就已经定稿（+28 ms 读到居中坐标，此后不再变），所以正常情况下一次就够；
/// 但有一次采样里摆放成功后窗口仍回到了居中位置，补摆是针对这种竞争的兜底。
/// 只补一次、且紧跟着收工：拖久了会跟用户抢——刚出现就被拖走的贴图会被拽回原位。
fn retry_placement(label: String, target: Option<(i32, i32)>, generation: u64, above: bool) {
    std::thread::spawn(move || {
        let marker = window_marker(&label);
        let mut placed = false;
        for delay in PLACEMENT_RETRY_DELAYS_MS {
            std::thread::sleep(std::time::Duration::from_millis(delay));
            if !placement_is_current(&label, generation) {
                // 这几百毫秒里用户缩放或换了位置，旧坐标已经作废。
                return;
            }
            match crate::capture::shell_extension_place_window(&marker, target, above) {
                // 补摆也成功了，收工。
                Ok(true) if placed => return,
                Ok(true) => placed = true,
                // 窗口还没在 Shell 里出现，继续等。
                Ok(false) => placed = false,
                Err(reason) => {
                    log::debug!("贴图窗口 {marker} 重试摆放中止: {reason}");
                    return;
                }
            }
        }
        if !placed {
            log::info!("贴图窗口 {marker} 等到超时也没被 GNOME Shell 认出来，位置与层级交给合成器");
        }
    });
}

/// 把逻辑坐标折成扩展要的整数像素。
fn shell_target(logical: Option<LogicalPosition<f64>>) -> Option<(i32, i32)> {
    logical.map(|position| (position.x.round() as i32, position.y.round() as i32))
}

/// 每个贴图窗口最近一次摆放请求的代次。
///
/// 摆放要重试（刚 `show()` 的窗口在 Shell 里还不存在），而重试是异步的：万一用户在这
/// 几百毫秒里缩放或拖动了贴图，一个迟到的重试就会把它拽回旧坐标。所以每次新请求把代次
/// +1，重试线程每轮先确认自己那一代还是最新的。窗口关掉时 `forget_placement` 抹掉记录，
/// 于是待命的重试也一并作废（查不到就是不是最新）。
fn placement_generations() -> &'static std::sync::Mutex<std::collections::HashMap<String, u64>> {
    static GENERATIONS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, u64>>,
    > = std::sync::OnceLock::new();
    GENERATIONS.get_or_init(Default::default)
}

fn next_placement_generation(label: &str) -> u64 {
    let Ok(mut generations) = placement_generations().lock() else {
        return 0;
    };
    let slot = generations.entry(label.to_string()).or_default();
    *slot += 1;
    *slot
}

fn placement_is_current(label: &str, generation: u64) -> bool {
    let Ok(generations) = placement_generations().lock() else {
        return false;
    };
    generations.get(label) == Some(&generation)
}

/// 窗口关闭时丢掉它的代次记录，同时让还在等的重试线程停下来。
pub(super) fn forget_placement(label: &str) {
    if let Ok(mut generations) = placement_generations().lock() {
        generations.remove(label);
    }
}

/// 把窗口摆到 `logical`（逻辑像素，窗口左上角），并按 `above` 决定进不进置顶层。
/// `logical` 传 `None` 表示只管层级、不动位置。
///
/// Wayland 协议里客户端既无权决定自己窗口的位置、也无权置顶，Mutter 把
/// `set_position` / `set_always_on_top` 静默忽略——这正是"贴图出现在屏幕中间"和
/// "贴图被别的窗口盖住"的原因。只有 GNOME Shell 扩展进得去 Shell 里调
/// `MetaWindow.move_frame()` / `make_above()` / `unmake_above()`。扩展不可用（没装、
/// 装了还没注销生效、不是 GNOME）时退回 Tauri 自己那套：在 X11 上它本来就管用。
///
/// **`above == false` 不是"什么都不做"，是明确地退出置顶层**（`unmake_above`）：
/// 用户关掉图钉、以及截图期间临时让路，靠的都是这一条。退出之后贴图就是个普通窗口，
/// 层内顺序交回合成器——"谁最后拿到焦点谁在上面"因此是自动成立的，不需要我们插手。
///
/// 两条路都失败只意味着"位置或层级不理想"，绝不能让贴图本身失败。
///
/// 只试一次，不等待——调用方里有热路径（缩放的每一帧）。需要等窗口映射的只有
/// `reveal_pin_window`，它拿返回值自己去排重试。
pub(super) fn keep_pin_above(
    window: &tauri::WebviewWindow,
    logical: Option<LogicalPosition<f64>>,
    above: bool,
) -> Placement {
    // 无论走哪条分支都先推进代次：这次请求的坐标就是最新的，
    // 还在后台等待的旧重试从此作废。
    let generation = next_placement_generation(window.label());
    let marker = window_marker(window.label());
    let outcome =
        crate::capture::shell_extension_place_window(&marker, shell_target(logical), above);
    let placement = match outcome {
        Ok(true) => return Placement::Done,
        // 刚 show() 的窗口在 Shell 里还不存在（实测 28~137 ms 才出现），所以这是**常态**，
        // 不是故障——真正该报的是重试也等不到，那条日志在 `retry_placement` 里。
        Ok(false) => {
            log::debug!("贴图窗口 {marker} 还没在 Shell 里出现，安排重试");
            Placement::NotMappedYet { generation }
        }
        // 非 GNOME Wayland、未安装、未注销生效都走到这里，是常态而不是故障。
        Err(reason) => {
            log::debug!("贴图窗口不经扩展摆放: {reason}");
            Placement::NoExtension
        }
    };
    if let Err(error) = window.set_always_on_top(above) {
        log::warn!("贴图窗口置顶状态设置失败: {error}");
    }
    if let Some(position) = logical {
        if let Err(error) = window.set_position(Position::Logical(position)) {
            log::warn!("贴图窗口定位失败: {error}");
        }
    }
    placement
}

/// 贴图窗口该待的逻辑坐标：让内容区正好盖住图片原本所在的那块屏幕。
///
/// 没有原始矩形（从剪贴板历史贴的图、别的程序复制来的图）时返回 `None`，
/// 位置交给创建时的光标定位与合成器自己的摆放。
fn pin_target_position(app: &tauri::AppHandle, entry: &PinEntry) -> Option<LogicalPosition<f64>> {
    let origin = entry.origin?;
    let (outer_width, outer_height) =
        outer_size(entry.content_width, entry.content_height, entry.scale);
    let target = LogicalPosition::new(origin.x - SHADOW_GUTTER, origin.y - SHADOW_GUTTER);
    Some(clamp_logical_position(
        app,
        target,
        outer_width,
        outer_height,
    ))
}

/// 把逻辑坐标钳进"包含它的那块显示器"的逻辑工作区。找不到显示器就原样返回——
/// 摆得不完美也比因为查不到几何而放弃摆放要好。
fn clamp_logical_position(
    app: &tauri::AppHandle,
    position: LogicalPosition<f64>,
    width: f64,
    height: f64,
) -> LogicalPosition<f64> {
    let Some(area) = logical_work_area(app, position) else {
        return position;
    };
    LogicalPosition::new(
        clamp_span(position.x, area.x, area.width, width),
        clamp_span(position.y, area.y, area.height, height),
    )
}

/// 把 `value` 钳进 `[start, start + span - size]`，窗口比工作区还大时退化为贴边。
pub(super) fn clamp_span(value: f64, start: f64, span: f64, size: f64) -> f64 {
    value.clamp(start, (start + span - size).max(start))
}

/// 逻辑坐标系里的显示器工作区。
///
/// Tauri 的显示器几何是物理像素，而原始矩形与扩展给的窗口坐标都是逻辑像素；
/// 多屏混合缩放时不能用同一个系数换算整个桌面，所以逐屏折算再挑包含目标点的那块。
/// 一块都不包含时返回第一块（目标点在屏幕外，钳一下总比不管要好）。
struct LogicalWorkArea {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
}

fn logical_work_area(
    app: &tauri::AppHandle,
    position: LogicalPosition<f64>,
) -> Option<LogicalWorkArea> {
    let monitors = app.available_monitors().ok()?;
    let mut fallback = None;
    for monitor in &monitors {
        let scale = monitor.scale_factor().max(0.1);
        let work = monitor.work_area();
        let area = LogicalWorkArea {
            x: work.position.x as f64 / scale,
            y: work.position.y as f64 / scale,
            width: work.size.width as f64 / scale,
            height: work.size.height as f64 / scale,
            scale,
        };
        if position.x >= area.x
            && position.x < area.x + area.width
            && position.y >= area.y
            && position.y < area.y + area.height
        {
            return Some(area);
        }
        fallback = fallback.or(Some(area));
    }
    fallback
}

/// 贴图窗口里"还落在屏幕工作区内"的那块矩形，**窗口局部坐标**、逻辑像素。
///
/// 工具条要靠它决定翻边还是进内部。为什么必须由后端算：前端只知道
/// `window.innerWidth/innerHeight`，而贴图窗口的外框恒等于「内容 + 阴影 + 控件栏」，
/// 也就是永远给工具条留够了位置——拿窗口自己当边界，右侧候选永远装得下，
/// "超出屏幕自动调整"一次都不会触发。真正会超出的是**窗口在屏幕上**的位置，
/// 而 Wayland 下客户端连自己窗口在哪都不知道（见 `known_pin_position`），
/// 只有合成器/扩展知道。
///
/// 拿不到窗口位置时返回整个窗口（等于"假定完全在屏内"），也就是这个功能之前的行为：
/// 宁可不调整，也不能因为查不到几何就把工具条摆到奇怪的地方。
pub(super) fn pin_toolbar_bounds(app: &tauri::AppHandle, label: &str) -> ToolbarBounds {
    let window = app.get_webview_window(label);
    let scale = window
        .as_ref()
        .and_then(|window| window.scale_factor().ok())
        .unwrap_or(1.0)
        .max(0.1);
    let size = window
        .as_ref()
        .and_then(|window| window.outer_size().ok())
        .map(|outer| (outer.width as f64 / scale, outer.height as f64 / scale));
    let Some((width, height)) = size else {
        return ToolbarBounds::UNKNOWN;
    };
    let whole = ToolbarBounds {
        x: 0.0,
        y: 0.0,
        width,
        height,
    };
    let position = visible_window_origin(label).or_else(|| {
        window
            .as_ref()
            .and_then(|window| x11_window_origin(window, scale))
    });
    let Some(position) = position else {
        return whole;
    };
    let Some(area) = logical_work_area(app, position) else {
        return whole;
    };
    visible_window_part(
        (position.x, position.y),
        (width, height),
        (area.x, area.y, area.width, area.height),
    )
}

/// 窗口矩形与工作区的交集，换算成**窗口局部坐标**。
///
/// 纯函数，好让"窗口挂在屏幕边缘外"这件事能被测到——上一版把这段算式和 `AppHandle`
/// 缠在一起，于是唯一能测的是 placement 那个函数，而真正决定行为的输入（可用范围）
/// 反而没人验，"超出屏幕自动调整"因此一次都没生效过还全绿。
///
/// 交集退化（多屏热插拔的瞬间、窗口整个在屏外）时返回整个窗口：摆得不完美也比算出
/// 空矩形、把工具条挤成一条线要好。
fn visible_window_part(
    origin: (f64, f64),
    size: (f64, f64),
    work: (f64, f64, f64, f64),
) -> ToolbarBounds {
    let (window_x, window_y) = origin;
    let (width, height) = size;
    let (area_x, area_y, area_width, area_height) = work;
    let whole = ToolbarBounds {
        x: 0.0,
        y: 0.0,
        width,
        height,
    };
    let left = (area_x - window_x).max(0.0);
    let top = (area_y - window_y).max(0.0);
    let right = (area_x + area_width - window_x).min(width);
    let bottom = (area_y + area_height - window_y).min(height);
    if right - left < 1.0 || bottom - top < 1.0 {
        return whole;
    }
    ToolbarBounds {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

/// 工具条可用范围，窗口局部逻辑坐标。宽或高为 0 表示"查不到，前端自己用整个窗口"。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolbarBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl ToolbarBounds {
    /// 连窗口尺寸都问不到（窗口刚关掉）。前端看到 0 宽高就退回 `window.innerWidth`。
    const UNKNOWN: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };
}

/// 贴图窗口左上角的逻辑坐标（桌面全局）。
///
/// **不能用 `outer_position()`**：GNOME Wayland 上它是假的，协议不把窗口位置告诉客户端，
/// GTK 只会回它自己最后一次 move 的值（也就是 0,0）——这个坑在 `known_pin_position`
/// 里已经栽过一次，缩放时拿它算"保持中心"会把窗口传送到屏幕角上。
///
/// 唯一知道真值的是合成器，所以问扩展。贴图窗口的标题是扩展的查找键
/// （`window_marker`），而且上一轮已经让贴图出现在 `GetWindows` 里了。
/// 扩展不可用（X11、非 GNOME、没装）时退回 `outer_position()`——X11 上它是真的。
fn visible_window_origin(label: &str) -> Option<LogicalPosition<f64>> {
    let marker = window_marker(label);
    let own_pid = std::process::id();
    if let Some(windows) = crate::capture::shell_extension_windows() {
        if let Some(window) = windows
            .iter()
            .find(|candidate| candidate.pid == own_pid && candidate.title == marker)
        {
            return Some(LogicalPosition::new(window.x as f64, window.y as f64));
        }
        // 扩展在但没报出这个窗口（刚建好还没映射进 Shell，实测 28~137 ms）：
        // 只能当作"位置未知"，这一轮不做边界调整。下一次询问就有了。
        return None;
    }
    None
}

/// X11 与非 GNOME 会话上的位置来源：那里 `outer_position()` 是真的。
///
/// 单独一个函数是为了让"哪条路是可信的"留在明面上——Wayland 与 X11 在这件事上
/// 完全不同，混在一个表达式里下次一定被误改。
fn x11_window_origin(window: &tauri::WebviewWindow, scale: f64) -> Option<LogicalPosition<f64>> {
    if crate::gsettings_shortcuts::is_wayland() {
        return None;
    }
    let physical = window.outer_position().ok()?;
    Some(LogicalPosition::new(
        physical.x as f64 / scale,
        physical.y as f64 / scale,
    ))
}

pub(super) fn resize_pin_window(app: &tauri::AppHandle, entry: &PinEntry) -> Result<(), PinError> {
    let window = app
        .get_webview_window(&entry.label)
        .ok_or(PinError::WindowMissing)?;
    let monitor = window
        .current_monitor()
        .map_err(PinError::window)?
        .or(window.primary_monitor().map_err(PinError::window)?);
    let (logical_width, logical_height) =
        outer_size(entry.content_width, entry.content_height, entry.scale);
    let Some(monitor) = monitor else {
        return window
            .set_size(tauri::LogicalSize::new(logical_width, logical_height))
            .map_err(PinError::window);
    };
    let scale_factor = monitor.scale_factor().max(0.1);
    let work = monitor.work_area();
    let requested = PhysicalSize::new(
        (logical_width * scale_factor).round() as u32,
        (logical_height * scale_factor).round() as u32,
    );
    let size = PhysicalSize::new(
        requested
            .width
            .min(work.size.width.saturating_sub(16).max(1)),
        requested
            .height
            .min(work.size.height.saturating_sub(16).max(1)),
    );
    let old_size = window.outer_size().unwrap_or(size);
    // 缩放本来想让窗口"从中心长大"，那需要知道它现在在哪。Wayland 上不知道，见
    // `known_pin_position`；以前那句 `.unwrap_or(work.position)` 把"不知道"当成了
    // "在工作区原点"，于是每一格滚轮都被算成一次"移到左上角"——这就是缩放时贴图
    // 跳走的原因。不知道就别动它：Wayland 上改尺寸时表面左上角本来是钉住的，
    // 贴图往右下长大，比每格都传送到屏幕角上好得多。
    let position = known_pin_position(&window, entry).map(|old_position| {
        let centered = PhysicalPosition::new(
            old_position.x + (old_size.width as i32 - size.width as i32) / 2,
            old_position.y + (old_size.height as i32 - size.height as i32) / 2,
        );
        clamp_pin_position(centered, size, work)
    });
    window
        .set_size(Size::Physical(size))
        .map_err(PinError::window)?;
    if let Some(position) = position {
        window
            .set_position(Position::Physical(position))
            .map_err(PinError::window)?;
    }
    // 改尺寸有可能把窗口带回普通层，所以每次都要把层级重新表态一次。位置未知时只管层级、
    // 不摆位（`keep_pin_above` 的 `None`）。
    //
    // **表态的内容是 `entry.above`，不是无条件置顶。** 以前这里写死 `true`，于是缩放一张
    // 没开图钉的贴图会把它弹到所有窗口上面——"谁最后拿到焦点谁在上"的语义被缩放这条路
    // 破坏掉了，而且用户没有任何办法让它退回去。
    keep_pin_above(
        &window,
        position.map(|position| {
            LogicalPosition::new(
                position.x as f64 / scale_factor,
                position.y as f64 / scale_factor,
            )
        }),
        entry.above,
    );
    Ok(())
}

/// 缩放时能不能相信"窗口现在在哪"这个数。
///
/// X11 与其它平台上 `WindowEvent::Moved` 和 `outer_position()` 都是真的。
/// GNOME Wayland 上两个都不是：协议根本不把窗口位置告诉客户端，GTK 只能回它自己
/// 最后一次 `move` 的值（也就是 0,0）。拿这种假位置去算"保持中心"就是把窗口传送走，
/// 所以 Wayland 上一律返回 `None`——含义是"别动位置"，而不是"位置是原点"。
fn known_pin_position(
    window: &tauri::WebviewWindow,
    entry: &PinEntry,
) -> Option<PhysicalPosition<i32>> {
    if crate::gsettings_shortcuts::is_wayland() {
        return None;
    }
    entry
        .position
        .map(|position| PhysicalPosition::new(position.x, position.y))
        .or_else(|| window.outer_position().ok())
}

fn position_new_pin_window(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    logical_width: f64,
    logical_height: f64,
    origin: Option<PinOrigin>,
) -> Result<(), PinError> {
    let cursor = app.cursor_position().ok();
    // 有原始矩形就照它摆（截图贴回原处），否则跟着光标——两种情况都要先找到
    // 目标点所在的显示器，因为工作区与缩放都是按屏算的。
    let anchor = origin
        .map(|origin| {
            let target = LogicalPosition::new(origin.x - SHADOW_GUTTER, origin.y - SHADOW_GUTTER);
            let scale = logical_scale_near(app, target);
            PhysicalPosition::new((target.x * scale).round(), (target.y * scale).round())
        })
        .or(cursor.map(|position| {
            PhysicalPosition::new(position.x.round() + 12.0, position.y.round() + 12.0)
        }));
    let monitor = anchor
        .and_then(|position| {
            app.monitor_from_point(position.x, position.y)
                .ok()
                .flatten()
        })
        .or(app.primary_monitor().map_err(PinError::window)?);
    let Some(monitor) = monitor else {
        return Ok(());
    };
    let scale = monitor.scale_factor().max(0.1);
    let size = PhysicalSize::new(
        (logical_width * scale).round().max(1.0) as u32,
        (logical_height * scale).round().max(1.0) as u32,
    );
    let work = monitor.work_area();
    let raw = anchor
        .map(|position| PhysicalPosition::new(position.x as i32, position.y as i32))
        .unwrap_or_else(|| {
            PhysicalPosition::new(
                work.position.x + (work.size.width.saturating_sub(size.width) / 2) as i32,
                work.position.y + (work.size.height.saturating_sub(size.height) / 2) as i32,
            )
        });
    window
        .set_position(Position::Physical(clamp_pin_position(raw, size, work)))
        .map_err(PinError::window)
}

/// 目标逻辑点所在显示器的缩放系数。逻辑坐标要换成物理坐标才能喂给
/// `monitor_from_point` / `set_position`。
fn logical_scale_near(app: &tauri::AppHandle, position: LogicalPosition<f64>) -> f64 {
    logical_work_area(app, position)
        .map(|area| area.scale)
        .unwrap_or(1.0)
}

pub(super) fn clamp_pin_position(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    work: &tauri::PhysicalRect<i32, u32>,
) -> PhysicalPosition<i32> {
    let max_x = work.position.x + work.size.width.saturating_sub(size.width) as i32;
    let max_y = work.position.y + work.size.height.saturating_sub(size.height) as i32;
    PhysicalPosition::new(
        position
            .x
            .clamp(work.position.x, max_x.max(work.position.x)),
        position
            .y
            .clamp(work.position.y, max_y.max(work.position.y)),
    )
}

/// 有原始矩形时的内容区尺寸：就用原始尺寸，**不做** `fit_content_size` 的放大。
///
/// "贴回原尺寸"意味着一个 60×30 的小选区就该显示成 60×30；`fit_dimensions` 会把它撑到
/// 至少 180×120 以保证界面可用，那正好破坏了原尺寸。只在窗口连工作区都装不下时
/// 才按比例缩小——否则贴图会有一部分永远在屏幕外。
pub(super) fn origin_content_size(app: &tauri::AppHandle, origin: PinOrigin) -> (f64, f64) {
    let Some(area) = logical_work_area(app, LogicalPosition::new(origin.x, origin.y)) else {
        return (origin.width, origin.height);
    };
    let max_width = (area.width - SHADOW_GUTTER * 2.0 - CONTROLS_GUTTER).max(1.0);
    let max_height = (area.height - SHADOW_GUTTER * 2.0 - TOOLBAR_GUTTER).max(1.0);
    // 上限 1.0：只缩不放，"原尺寸"就是原尺寸。下限 0.01 防止极端矩形算出 0。
    let shrink = (max_width / origin.width)
        .min(max_height / origin.height)
        .clamp(0.01, 1.0);
    (origin.width * shrink, origin.height * shrink)
}

/// 没有原始矩形时的内容区尺寸。入参是**图片像素**，出参是 CSS 像素。
///
/// 两者不是一回事，这一步以前漏了：在缩放 1.3333 的屏上把 1052 像素宽的图当成
/// 1052 CSS 像素显示，它在屏幕上就占 1403 个设备像素——图片被拉大 1.3333 倍，
/// 于是"贴出来的截图比原来大一圈而且发糊"。按真实缩放折算之后一个图片像素正好落在
/// 一个设备像素上，和它在屏幕上原来的样子一致（有原始矩形那条路本来就是这个效果，
/// 因为选区尺寸本身就是逻辑像素）。
///
/// 真实缩放要问合成器，不能用 `scale_factor()`，理由见
/// `crate::screenshot::desktop_scale_at`。拿不到就退回 GDK 那个数——X11 与其它平台上
/// 它就是真的，Wayland 上退化成"像素当 CSS 像素"，也就是修这个 bug 之前的行为。
pub(super) fn fit_content_size(app: &tauri::AppHandle, width: f64, height: f64) -> (f64, f64) {
    let (max_width, max_height, device_scale) = cursor_monitor(app)
        .map(|monitor| {
            let work = monitor.work_area();
            let scale = monitor.scale_factor().max(0.1);
            (
                work.size.width as f64 / scale * 0.72 - CONTROLS_GUTTER,
                work.size.height as f64 / scale * 0.72 - TOOLBAR_GUTTER,
                monitor_device_scale(&monitor, scale),
            )
        })
        .unwrap_or((900.0, 700.0, 1.0));
    fit_image_content_size(width, height, device_scale, max_width, max_height)
}

/// `fit_content_size` 的纯函数内核：图片像素 → CSS 像素 → 钳进上限。
pub(super) fn fit_image_content_size(
    pixel_width: f64,
    pixel_height: f64,
    device_scale: f64,
    max_width: f64,
    max_height: f64,
) -> (f64, f64) {
    let device_scale = if device_scale > 0.0 {
        device_scale
    } else {
        1.0
    };
    fit_dimensions(
        pixel_width / device_scale,
        pixel_height / device_scale,
        max_width,
        max_height,
    )
}

/// 没有原始矩形时按光标那块屏定尺寸。单独拿出来是为了让 `content_device_scale`
/// 问的是**同一块屏**：内容尺寸与它用的缩放必须配套，否则前端按 payload 里那两个数
/// 复原出来的显示尺寸就是错的。
fn cursor_monitor(app: &tauri::AppHandle) -> Option<tauri::Monitor> {
    app.cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
}

/// 贴图内容所在那块屏的真实缩放，跟着 payload 交给前端。
///
/// 前端要判断"屏上一个图片像素是不是正好一个设备像素"——只有相等时才该让 WebKit
/// 用最近邻把图搬进缓冲区（理由与实测见 `src/react/pin/rendering.ts`）。选屏规则必须
/// 和算内容尺寸时一致：有原始矩形就问那块屏，没有就问光标那块。
pub(super) fn content_device_scale(app: &tauri::AppHandle, origin: Option<PinOrigin>) -> f64 {
    match origin {
        // 矩形左上角可能正好压在屏幕边界上，+1 保证落在这块屏里面。
        Some(origin) => {
            let inside = LogicalPosition::new(origin.x + 1.0, origin.y + 1.0);
            crate::screenshot::desktop_scale_at(inside.x, inside.y)
                .unwrap_or_else(|| logical_scale_near(app, inside))
        }
        None => cursor_monitor(app)
            .map(|monitor| monitor_device_scale(&monitor, monitor.scale_factor().max(0.1)))
            .unwrap_or(1.0),
    }
}

/// 贴图窗口的**缓冲区缩放**：GTK/GDK 给这块屏报的那个整数。
///
/// 它和 `content_device_scale` 的真实缩放常常不相等（1.5 倍缩放的桌面上是 2 对 1.5），
/// 差出来的那一趟放大就是"贴出来发糊"的根源。两个数一起交给
/// `super::resample`，那边据此把显示用的图预先渲染成缓冲区分辨率。
/// 选屏规则必须和 `content_device_scale` 逐字一致，否则两个缩放不配套。
pub(super) fn content_buffer_scale(app: &tauri::AppHandle, origin: Option<PinOrigin>) -> f64 {
    match origin {
        Some(origin) => {
            logical_scale_near(app, LogicalPosition::new(origin.x + 1.0, origin.y + 1.0))
        }
        None => cursor_monitor(app)
            .map(|monitor| monitor.scale_factor().max(0.1))
            .unwrap_or(1.0),
    }
}

/// 这块屏上一个逻辑像素等于几个设备像素。取屏幕自己的原点去问，不用光标位置——
/// 光标可能正好停在屏幕边界上，原点加一像素一定落在这块屏里面。
fn monitor_device_scale(monitor: &tauri::Monitor, gdk_scale: f64) -> f64 {
    let position = monitor.position();
    let x = position.x as f64 / gdk_scale + 1.0;
    let y = position.y as f64 / gdk_scale + 1.0;
    crate::screenshot::desktop_scale_at(x, y).unwrap_or(gdk_scale)
}

pub(super) fn fit_dimensions(
    width: f64,
    height: f64,
    max_width: f64,
    max_height: f64,
) -> (f64, f64) {
    let width = width.max(1.0);
    let height = height.max(1.0);
    let maximum_scale = (max_width.max(1.0) / width).min(max_height.max(1.0) / height);
    let desired_scale = 1.0_f64
        .max(MIN_IMAGE_WIDTH / width)
        .max(MIN_IMAGE_HEIGHT / height);
    let scale = desired_scale.min(maximum_scale).max(0.01);
    (width * scale, height * scale)
}

pub(super) fn outer_size(content_width: f64, content_height: f64, scale: f64) -> (f64, f64) {
    (
        content_width * scale + SHADOW_GUTTER * 2.0 + CONTROLS_GUTTER,
        (content_height * scale + SHADOW_GUTTER * 2.0 + TOOLBAR_GUTTER).max(MIN_OUTER_HEIGHT),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 代次登记表是进程级的静态量，测试之间必须用不同的 label 才互不干扰。
    #[test]
    fn a_newer_placement_request_invalidates_the_pending_retry() {
        let label = "pin-generation-newer";
        let first = next_placement_generation(label);
        assert!(placement_is_current(label, first));

        // 用户缩放/拖动贴图 → 又一次摆放请求：上一代作废，只有最新那代能继续摆放。
        // 少了这一层，一个迟到的重试会把贴图拽回它刚出现时的坐标。
        let second = next_placement_generation(label);
        assert!(!placement_is_current(label, first));
        assert!(placement_is_current(label, second));
    }

    /// 窗口关掉之后，还在后台等它出现的重试必须停下来（查不到记录就不是最新）。
    #[test]
    fn forgetting_a_window_stops_its_retry() {
        let label = "pin-generation-forget";
        let generation = next_placement_generation(label);
        forget_placement(label);
        assert!(!placement_is_current(label, generation));
    }

    /// 两个贴图各自记代次：先后开两张图，第二张不能把第一张还没落地的摆放作废。
    #[test]
    fn generations_are_tracked_per_window() {
        let (first_label, second_label) = ("pin-generation-a", "pin-generation-b");
        let first = next_placement_generation(first_label);
        next_placement_generation(second_label);
        assert!(placement_is_current(first_label, first));
    }

    /// 重试的等待总额要够覆盖实测的映射延迟（~65 ms），又不能长到让用户看着贴图
    /// 自己跳位置；同时必须是递增退避，头几次快、后面稀。
    #[test]
    fn the_retry_schedule_covers_the_measured_map_delay() {
        let total: u64 = PLACEMENT_RETRY_DELAYS_MS.iter().sum();
        assert!((300..=3000).contains(&total), "重试总时长 {total}ms 不合理");
        assert!(PLACEMENT_RETRY_DELAYS_MS
            .windows(2)
            .all(|pair| pair[0] <= pair[1]));
        assert!(
            PLACEMENT_RETRY_DELAYS_MS[0] < 65,
            "第一次重试必须早于实测的映射时刻"
        );
    }

    /// 窗口完全在屏内：可用范围就是整个窗口，工具条行为和以前一致。
    #[test]
    fn a_fully_visible_window_yields_its_whole_area() {
        let bounds =
            visible_window_part((100.0, 100.0), (868.0, 672.0), (0.0, 0.0, 1920.0, 1080.0));
        assert_eq!(
            (bounds.x, bounds.y, bounds.width, bounds.height),
            (0.0, 0.0, 868.0, 672.0)
        );
    }

    /// **这条是"超出屏幕自动调整"的真正入口。** 窗口右边挂在屏幕外面时，可用范围要窄，
    /// 工具条才会翻边——上一版没有这一步，于是那个功能一次都没生效过。
    #[test]
    fn a_window_hanging_off_the_right_edge_reports_a_narrower_area() {
        // 屏幕宽 1920，窗口左上角在 x=1600，宽 868 → 只有 320 可见。
        let bounds =
            visible_window_part((1600.0, 100.0), (868.0, 672.0), (0.0, 0.0, 1920.0, 1080.0));
        assert_eq!((bounds.x, bounds.width), (0.0, 320.0));
        // 纵向没超，高度不该被动。
        assert_eq!((bounds.y, bounds.height), (0.0, 672.0));
    }

    /// 窗口左上角在屏幕外（拖到左上角）：可用范围要带偏移，工具条不能摆到偏移之前。
    #[test]
    fn a_window_past_the_top_left_reports_an_offset_area() {
        let bounds =
            visible_window_part((-200.0, -150.0), (868.0, 672.0), (0.0, 0.0, 1920.0, 1080.0));
        assert_eq!((bounds.x, bounds.y), (200.0, 150.0));
        assert_eq!((bounds.width, bounds.height), (668.0, 522.0));
    }

    /// 工作区本身带偏移（顶栏、副屏在负坐标上）时同样成立。
    #[test]
    fn the_work_area_offset_is_respected() {
        // 顶栏占 32 px：工作区从 y=32 开始，窗口贴在 y=0。
        let bounds = visible_window_part((0.0, 0.0), (868.0, 672.0), (0.0, 32.0, 1920.0, 1048.0));
        assert_eq!((bounds.y, bounds.height), (32.0, 640.0));
        // 副屏在主屏左边：工作区原点是负数，窗口整体可见。
        let left_screen = visible_window_part(
            (-1800.0, 24.0),
            (868.0, 672.0),
            (-1920.0, 24.0, 1920.0, 1056.0),
        );
        assert_eq!(
            (left_screen.x, left_screen.y, left_screen.width),
            (0.0, 0.0, 868.0)
        );
    }

    /// 交集退化（窗口整个在屏外、热插拔瞬间）时退回整个窗口，绝不返回空矩形——
    /// 空矩形会让工具条被钳成一条线，一个按钮都点不到。
    #[test]
    fn a_degenerate_intersection_falls_back_to_the_whole_window() {
        let off_screen =
            visible_window_part((5000.0, 5000.0), (868.0, 672.0), (0.0, 0.0, 1920.0, 1080.0));
        assert_eq!((off_screen.width, off_screen.height), (868.0, 672.0));
        // 只剩一条缝也算退化。
        let sliver =
            visible_window_part((1919.5, 100.0), (868.0, 672.0), (0.0, 0.0, 1920.0, 1080.0));
        assert_eq!((sliver.width, sliver.height), (868.0, 672.0));
    }

    #[test]
    fn shell_targets_are_rounded_and_optional() {
        assert_eq!(shell_target(None), None);
        assert_eq!(
            shell_target(Some(LogicalPosition::new(10.4, -20.6))),
            Some((10, -21))
        );
    }
}
