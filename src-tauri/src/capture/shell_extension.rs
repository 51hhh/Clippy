//! GNOME Shell 扩展：窗口几何与冻结帧截图的唯一可靠来源。
//!
//! GNOME Wayland 下客户端拿不到任何窗口的屏幕坐标，逐一实测排除过 Shell.Introspect、
//! Shell.Screenshot、ext/wlr-foreign-toplevel、AT-SPI、X11 枚举（详见
//! `docs/capture-linux.md`）。唯一持有这份数据的进程是 gnome-shell 自己，所以只能以
//! 扩展的身份进去取。这个模块负责扩展的安装、卸载、状态查询，以及通过私有 D-Bus
//! 取窗口列表和截图。
//!
//! 截图为什么也走这条路：xdg-desktop-portal 的非交互截图要先弹一个系统授权对话框，
//! 而 gnome-shell 只允许当前聚焦的应用弹（实测 "Only the focused app is allowed to
//! show a system access dialog"）。截图由全局快捷键触发，那一刻 Clippy 没有窗口聚焦，
//! 对话框永远弹不出来，非交互截图就永远失败。走扩展则完全不碰 Portal。
//!
//! 安装策略：扩展的两个文件用 `include_str!` 编进二进制，运行时装到用户目录。
//! 不走 deb 的 `/usr/share/gnome-shell/extensions/`——postrm 跑在 root 下没法清理每个
//! 用户的目录与 dconf，AppImage 又没有安装钩子，而 `/usr/share` 下自动启用是 Ubuntu 的
//! 补丁行为、上游 GNOME 并不会。单一来源放用户目录，三种分发方式行为一致，也才能由
//! 应用自己卸载干净。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 扩展 uuid，必须与 `gnome-extension/` 下目录名和 metadata.json 一致。
pub(crate) const UUID: &str = "clippy-windows@clippy.local";

const METADATA_JSON: &str =
    include_str!("../../../gnome-extension/clippy-windows@clippy.local/metadata.json");
const EXTENSION_JS: &str =
    include_str!("../../../gnome-extension/clippy-windows@clippy.local/extension.js");

/// 令牌文件名，必须与 extension.js 里的 TOKEN_FILE_NAME 一致。
const TOKEN_FILE_NAME: &str = "token";

/// 令牌长度下限，和 extension.js 的 MIN_TOKEN_LENGTH 一致。
const MIN_TOKEN_LENGTH: usize = 16;

/// 内嵌扩展声明的协议版本，也是能解析的上限。跑着的扩展报出更大的值说明磁盘上的扩展
/// 比当前二进制新（用户装了新版又跑了旧版），此时宁可退化也不要错解析。
const EMBEDDED_PROTOCOL_VERSION: u32 = 5;

/// `Screenshot` 方法从这个协议版本起存在。低于它的扩展只能提供窗口几何：
/// gnome-shell 只在登录时加载扩展（ReloadExtension 实测已废弃，直接报
/// "is deprecated and does not work"），所以升级完到下次注销之前跑的仍是旧版。
const SCREENSHOT_PROTOCOL_VERSION: u32 = 2;

/// 逐屏原生取像素的 `CaptureArea` 从这个协议版本起存在。低于它只能走整屏那条路，
/// 而整屏图是按**全桌面最大缩放**渲染的，混合缩放时低缩放的屏会被上采样、画面发糊
/// （见 `screenshot/backends.rs::capture_all_shell_extension_monitor_areas`）。
/// 所以这是"糊"与"不糊"的分界线，也是"快"与"慢"的分界线（整屏那条路要 gnome-shell
/// 编一张全桌面 PNG，实测 1.8 秒），两者都意味着升级完必须注销一次才真的生效。
const RAW_CAPTURE_PROTOCOL_VERSION: u32 = 5;

/// `PlaceWindow` 方法从这个协议版本起存在。低于它的扩展照样能截图与速选，
/// 只是贴图窗口回不到原位、也压不住别的窗口——退化，不是故障。
const PLACEMENT_PROTOCOL_VERSION: u32 = 3;

/// 扩展写截图的目录名，位于 XDG_RUNTIME_DIR 下，必须与 extension.js 的
/// SCREENSHOT_DIR_NAME 一致。只接受这个目录里的路径——读完就删，不能删错地方。
const SCREENSHOT_DIR_NAME: &str = "clippy-shots";

const SHELL_BUS_NAME: &str = "org.gnome.Shell";
const WINDOWS_OBJECT_PATH: &str = "/org/gnome/Shell/Extensions/ClippyWindows";
const WINDOWS_INTERFACE: &str = "org.gnome.Shell.Extensions.ClippyWindows";
const EXTENSIONS_OBJECT_PATH: &str = "/org/gnome/Shell";
const EXTENSIONS_INTERFACE: &str = "org.gnome.Shell.Extensions";

const SHELL_SCHEMA: &str = "org.gnome.shell";
const ENABLED_EXTENSIONS_KEY: &str = "enabled-extensions";

/// 扩展的服务状态，直接下发给设置页。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellExtensionStatus {
    /// 当前会话是否用得上这个扩展（只有 GNOME Wayland 有意义，见 `is_relevant`）。
    pub supported: bool,
    /// 文件已就位且内容与当前二进制内嵌的一致。
    pub installed: bool,
    /// uuid 已写进 `org.gnome.shell enabled-extensions`。
    pub enabled: bool,
    /// gnome-shell 里的扩展真的在应答 D-Bus——这是"功能可用"的唯一判据。
    pub active: bool,
    /// 在应答，但版本比内嵌的旧：文件已经升级过，跑着的还是上次登录时加载的那份。
    /// 窗口速选照样可用，扩展截图要等下次登录。
    pub stale: bool,
    /// 用户是否在系统层面关掉了全部扩展（关掉时装了也不会跑）。
    pub user_extensions_enabled: bool,
}

/// 安装结果。`needs_logout` 为真时窗口速选要等下次登录才生效。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    pub needs_logout: bool,
    pub status: ShellExtensionStatus,
}

/// 扩展下发的单个窗口。坐标是逻辑像素、且已排除 CSD 阴影。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(super) struct ShellWindow {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub wm_class: String,
    #[serde(default)]
    pub pid: u32,
}

/// 取当前工作区的窗口列表，数组顺序即堆叠顺序（索引 0 最上层）。
///
/// 不可用时返回 `None`，调用方退回 X11 枚举。非 GNOME 桌面只付一次 stat 的代价，
/// 不会白跑一趟 D-Bus。
pub(super) fn probe() -> Option<Vec<ShellWindow>> {
    if !is_installed() {
        log::debug!("GNOME Shell 窗口扩展未安装，窗口几何退回 X11 枚举");
        return None;
    }
    let token = read_token()?;
    match call_get_windows(&token) {
        Ok(json) => match parse_windows(&json) {
            Ok(windows) => Some(windows),
            Err(error) => {
                log::warn!("GNOME Shell 窗口扩展返回的 JSON 无法解析: {error}");
                None
            }
        },
        Err(error) => {
            // 装了却调不通，最常见的原因是装完还没注销过。
            log::info!("GNOME Shell 窗口扩展未应答（可能尚未注销生效）: {error}");
            None
        }
    }
}

/// 让扩展截一张整屏（含全部显示器）的 PNG，返回它落地的私有路径。
///
/// 调用方读完必须删掉那个文件。拿不到就返回 `Err`，由截图链路退到下一个后端——
/// 非 GNOME 桌面、没装扩展、装完还没注销，都属于这一类，只付一次 stat 的代价。
pub(crate) fn request_screenshot() -> Result<PathBuf, String> {
    let token = screenshot_token(SCREENSHOT_PROTOCOL_VERSION)?;
    let path: String = shell_call(
        WINDOWS_OBJECT_PATH,
        WINDOWS_INTERFACE,
        "Screenshot",
        &(token.as_str(),),
    )
    .map_err(|error| format!("扩展截图调用失败: {error}"))?;

    let path = PathBuf::from(path);
    validate_screenshot_path(&screenshot_dir()?, &path)?;
    Ok(path)
}

/// 要向扩展索取的一块屏。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CaptureArea {
    /// 这块屏的逻辑矩形（stage 坐标，与 `GetWindows` 同一坐标系）。
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// 这块屏自己的缩放。扩展按它把区域画成原生像素——**不是**由相交视图推出来的，
    /// 所以混合缩放的多屏上也不会被邻屏的缩放带偏。
    pub scale: f64,
}

/// 扩展交回来的一块屏的画面。文件都在 XDG_RUNTIME_DIR（tmpfs）里，调用方读完必须删。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AreaCapture {
    /// 原始 RGBA8，行优先。`stride` 是一行的字节数，可能大于 `width * 4`。
    Raw {
        path: PathBuf,
        width: u32,
        height: u32,
        stride: usize,
    },
    /// 扩展那边原始像素这条路没走通（journal 里有一行 `Clippy raw capture failed`），
    /// 退回了 PNG。尺寸由调用方从文件头读——反正它本来就要解这张图。
    Png { path: PathBuf },
}

impl AreaCapture {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Raw { path, .. } | Self::Png { path } => path,
        }
    }
}

/// 逐屏取画面：每块屏一次 `CaptureArea`，**同时发起**，返回与入参同序的结果。
///
/// 为什么是原始像素而不是 PNG：实测（GNOME 50.1，双屏，Intel Arc iGPU）同样 4.1 Mpx 的
/// 一块区域，内容简单的 eDP 要 124 ms（PNG 640 KiB），内容复杂的 4K 屏要 884 ms
/// （PNG 3217 KiB）——像素数一样、时间差 7 倍，差额全在 gnome-shell 的 deflate 上，
/// 而 `Shell.Screenshot` 没有调压缩级别的入口。所以整条链路上最贵的一步是"编一张没人
/// 需要的 PNG"，扩展改用 `paint_to_content` + `Cogl.Texture.get_data` 直接落原始字节，
/// 我们这侧连解码都省了。扩展那边万一 GI 编排不通会自动退回 PNG，于是有 `AreaCapture::Png`。
///
/// 区域必须正好是那块屏的逻辑矩形：虽然像素尺寸现在由 `scale` 决定、不再依赖
/// "相交视图的最大缩放"，但画面内容仍然是按这个矩形裁的，多一个像素就会带进邻屏的内容。
///
/// 并行是有意的：读回像素在 gnome-shell 主循环上会串行，但每块屏只要几十到两百毫秒，
/// 而 D-Bus 往返本身是能重叠的。调用方读完每个文件都必须删。
pub(crate) fn request_area_captures(areas: &[CaptureArea]) -> Result<Vec<AreaCapture>, String> {
    if areas.is_empty() {
        return Err("没有要截的显示器区域".to_string());
    }
    let token = screenshot_token(RAW_CAPTURE_PROTOCOL_VERSION)?;
    let directory = screenshot_dir()?;
    let token = token.as_str();
    let directory = directory.as_path();
    let shots: Vec<Result<AreaCapture, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = areas
            .iter()
            .map(|&area| scope.spawn(move || request_one_area(token, area, directory)))
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err("逐屏截图线程 panic".to_string()))
            })
            .collect()
    });
    collect_area_captures(shots)
}

/// 一块屏的 `CaptureArea`。
fn request_one_area(
    token: &str,
    area: CaptureArea,
    directory: &Path,
) -> Result<AreaCapture, String> {
    // D-Bus 签名是四个 i32，超范围的宽高在这里就挡住——转进 D-Bus 之后只会得到
    // 一个"参数类型不匹配"，看不出是哪块屏的几何有问题。
    let width =
        i32::try_from(area.width).map_err(|_| format!("显示器宽度 {} 超出 i32", area.width))?;
    let height =
        i32::try_from(area.height).map_err(|_| format!("显示器高度 {} 超出 i32", area.height))?;
    let (path, pixel_width, pixel_height, stride, format): (String, i32, i32, i32, String) =
        shell_call(
            WINDOWS_OBJECT_PATH,
            WINDOWS_INTERFACE,
            "CaptureArea",
            &(token, area.x, area.y, width, height, area.scale),
        )
        .map_err(|error| format!("扩展逐屏取像素调用失败: {error}"))?;

    let path = PathBuf::from(path);
    // 先校验路径再谈内容：校验不过的路径不是我们的文件，绝不能去删。
    validate_screenshot_path(directory, &path)?;
    let capture = interpret_capture_reply(path.clone(), pixel_width, pixel_height, stride, &format);
    if capture.is_err() {
        // 应答自相矛盾时文件已经落地了，谁也不会再去读它。
        let _ = std::fs::remove_file(&path);
    }
    capture
}

/// 把扩展的应答翻译成 `AreaCapture`。单独拆出来是为了能直接钉住这份格式契约——
/// 尺寸/stride 一旦对不上，画面会整张斜掉，而那种错很难从像素里反推。
fn interpret_capture_reply(
    path: PathBuf,
    pixel_width: i32,
    pixel_height: i32,
    stride: i32,
    format: &str,
) -> Result<AreaCapture, String> {
    match format {
        "PNG" => Ok(AreaCapture::Png { path }),
        "RGBA" => {
            let width = u32::try_from(pixel_width)
                .ok()
                .filter(|width| *width > 0)
                .ok_or_else(|| format!("扩展报的像素宽 {pixel_width} 不是正数"))?;
            let height = u32::try_from(pixel_height)
                .ok()
                .filter(|height| *height > 0)
                .ok_or_else(|| format!("扩展报的像素高 {pixel_height} 不是正数"))?;
            let stride = usize::try_from(stride)
                .map_err(|_| format!("扩展报的 stride {stride} 不是正数"))?;
            if stride < width as usize * 4 {
                return Err(format!(
                    "扩展报的 stride {stride} 装不下 {width} 像素的一行 RGBA"
                ));
            }
            Ok(AreaCapture::Raw {
                path,
                width,
                height,
                stride,
            })
        }
        other => Err(format!("扩展报了未知的画面格式 {other}")),
    }
}

/// 全成才算成。缺一块屏的冻结帧比整体退回整屏那条路糟得多（覆盖层会有一块屏是空的），
/// 所以有任何一块失败就把已经落地的文件删掉、整体报错——那些文件再没人会去读，
/// 留着就是 XDG_RUNTIME_DIR 里的垃圾。
fn collect_area_captures(
    shots: Vec<Result<AreaCapture, String>>,
) -> Result<Vec<AreaCapture>, String> {
    if let Some(reason) = shots.iter().find_map(|shot| shot.as_ref().err()).cloned() {
        for shot in shots.iter().flatten() {
            let _ = std::fs::remove_file(shot.path());
        }
        return Err(reason);
    }
    shots.into_iter().collect()
}

/// 两条截图路子共同的前置条件：会话对不对、扩展装了没、跑着的协议够不够新。
/// 过了就返回令牌。
fn screenshot_token(minimum_version: u32) -> Result<String, String> {
    if !is_relevant() {
        return Err("当前会话不是 GNOME Wayland，不走扩展截图".to_string());
    }
    if !is_installed() {
        return Err("GNOME Shell 截图扩展未安装".to_string());
    }
    match running_protocol_version() {
        Some(version) if version >= minimum_version => {}
        Some(version) => {
            return Err(format!(
                "跑着的扩展是协议 v{version}，这次截图需要 v{minimum_version}（注销一次生效）"
            ))
        }
        None => return Err("GNOME Shell 扩展未应答（可能尚未注销生效）".to_string()),
    }
    read_token().ok_or_else(|| "缺少可用的扩展令牌".to_string())
}

/// 扩展写截图的目录。和 extension.js 里的 `GLib.get_user_runtime_dir()` 同一个位置。
fn screenshot_dir() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "XDG_RUNTIME_DIR 不可用，无法校验扩展截图路径".to_string())?;
    Ok(runtime.join(SCREENSHOT_DIR_NAME))
}

/// 扩展写出来的两种后缀：`png` 是编码过的，`rgba` 是原始像素。
const SCREENSHOT_EXTENSIONS: [&str; 2] = ["png", "rgba"];

/// 只接受约定目录里后缀在册的普通文件。读完就删，所以宁可错拒也不能删错文件；
/// 而且路径来自 D-Bus 应答，即便发送方只能是 gnome-shell 也不该无条件相信。
fn validate_screenshot_path(directory: &Path, path: &Path) -> Result<(), String> {
    if path.parent() != Some(directory) {
        return Err(format!(
            "扩展截图路径 {} 不在 {} 内",
            path.display(),
            directory.display()
        ));
    }
    let extension = path.extension().and_then(|ext| ext.to_str());
    if !extension.is_some_and(|ext| SCREENSHOT_EXTENSIONS.contains(&ext)) {
        return Err(format!(
            "扩展截图路径 {} 的后缀不是 {}",
            path.display(),
            SCREENSHOT_EXTENSIONS.join(" / ")
        ));
    }
    Ok(())
}

/// 让扩展把我们自己的某个窗口摆到 `(x, y)` 并/或置顶，返回它有没有找到那个窗口。
///
/// 只有 GNOME Wayland 需要这条路：Wayland 客户端无权决定自己窗口的位置、也无权置顶，
/// `set_position` / `set_always_on_top` 在 Mutter 下是静默空操作。别的会话（X11、
/// 其它合成器）直接返回 `Err`，调用方退回 Tauri 自己那套——在 X11 上它本来就管用。
///
/// `marker` 是窗口标题：贴图窗口无装饰、不进任务栏，标题不出现在界面上，只做查找键。
///
/// **这是热路径**：贴图窗口缩放时每一帧都要重新摆位（改尺寸会把窗口带回普通层），
/// 而调用方 `update_pin` 是同步命令、跑在主线程上。所以前置检查的结果缓存在
/// `placement_token()` 里，一帧的成本只剩一次 D-Bus 往返。
pub(crate) fn place_window(
    marker: &str,
    position: Option<(i32, i32)>,
    above: bool,
) -> Result<bool, String> {
    let token = placement_token()?;
    let (x, y) = position.unwrap_or((0, 0));
    let outcome: Result<bool, zbus::Error> = shell_call(
        WINDOWS_OBJECT_PATH,
        WINDOWS_INTERFACE,
        "PlaceWindow",
        &(
            token.as_str(),
            std::process::id(),
            marker,
            x,
            y,
            position.is_some(),
            above,
        ),
    );
    if outcome.is_err() {
        // 缓存的前置条件已经不成立了（扩展被关掉、令牌换过）：丢掉，下次重新探。
        invalidate_placement_probe();
    }
    outcome.map_err(|error| format!("PlaceWindow 调用失败: {error}"))
}

/// 失败的探测结果只缓存这么久。用户可以在同一个会话里用 GNOME 的扩展开关把扩展重新打开，
/// 永久记住"用不了"会让摆位在应用重启前一直失效；而缩放贴图那一串帧只有几百毫秒，
/// 这个窗口足够把它们全挡在探测之外。成功的结果不过期，由调用失败与安装/卸载来失效。
const PLACEMENT_PROBE_NEGATIVE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

struct PlacementProbe {
    outcome: Result<String, String>,
    at: std::time::Instant,
}

fn placement_probe_cache() -> &'static std::sync::Mutex<Option<PlacementProbe>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<PlacementProbe>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

/// 成功的探测永久有效，失败的过 TTL 就要重探一次。
fn placement_probe_is_fresh(succeeded: bool, age: std::time::Duration) -> bool {
    succeeded || age < PLACEMENT_PROBE_NEGATIVE_TTL
}

/// 摆窗的前置条件：会话对不对、扩展装了没、跑着的协议够不够新、令牌是什么。
///
/// 探一次要两次整文件读（extension.js 有 13 KB）加一次 `GetVersion` 往返，所以结果缓存起来。
fn placement_token() -> Result<String, String> {
    if let Ok(slot) = placement_probe_cache().lock() {
        if let Some(probe) = slot.as_ref() {
            if placement_probe_is_fresh(probe.outcome.is_ok(), probe.at.elapsed()) {
                return probe.outcome.clone();
            }
        }
    }
    let outcome = probe_placement();
    if let Ok(mut slot) = placement_probe_cache().lock() {
        *slot = Some(PlacementProbe {
            outcome: outcome.clone(),
            at: std::time::Instant::now(),
        });
    }
    outcome
}

fn probe_placement() -> Result<String, String> {
    if !is_relevant() {
        return Err("当前会话不是 GNOME Wayland，窗口摆放不走扩展".to_string());
    }
    if !is_installed() {
        return Err("GNOME Shell 扩展未安装".to_string());
    }
    match running_protocol_version() {
        Some(version) if version >= PLACEMENT_PROTOCOL_VERSION => {}
        Some(version) => {
            return Err(format!(
                "跑着的扩展是协议 v{version}，窗口摆放需要 v{PLACEMENT_PROTOCOL_VERSION}（注销一次生效）"
            ))
        }
        None => return Err("GNOME Shell 扩展未应答（可能尚未注销生效）".to_string()),
    }
    read_token().ok_or_else(|| "缺少可用的扩展令牌".to_string())
}

fn invalidate_placement_probe() {
    if let Ok(mut slot) = placement_probe_cache().lock() {
        *slot = None;
    }
}

/// 这个扩展在当前会话里有没有意义。
///
/// 只有 GNOME Wayland 需要它：GNOME X11 下 xcap 能直接枚举窗口几何，
/// 而别的合成器根本不认 GNOME Shell 扩展，装了也不会跑。
pub(super) fn is_relevant() -> bool {
    crate::gsettings_shortcuts::is_gnome_desktop() && crate::gsettings_shortcuts::is_wayland()
}

/// 覆盖层要不要提示"装个服务才能窗口速选"。
///
/// 判据是扩展有没有在应答 D-Bus，而不是文件在不在：装完还没注销同样是不可用状态，
/// 这时候恰恰最需要提示（提示文案里就写着要注销）。
pub(super) fn hint_needed() -> bool {
    is_relevant() && !(is_installed() && running_protocol_version().is_some())
}

pub fn status() -> ShellExtensionStatus {
    let supported = is_relevant();
    let installed = is_installed();
    let running = if installed {
        running_protocol_version()
    } else {
        None
    };
    ShellExtensionStatus {
        supported,
        installed,
        enabled: read_enabled_extensions()
            .map(|entries| entries.iter().any(|entry| entry == UUID))
            .unwrap_or(false),
        active: running.is_some(),
        stale: running.is_some_and(|version| version < EMBEDDED_PROTOCOL_VERSION),
        user_extensions_enabled: user_extensions_enabled(),
    }
}

/// 安装并尽力当场启用。
pub fn install() -> Result<InstallOutcome, String> {
    let directory = extension_dir()?;
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("创建 {} 失败: {error}", directory.display()))?;
    write_if_changed(&directory.join("metadata.json"), METADATA_JSON)?;
    write_if_changed(&directory.join("extension.js"), EXTENSION_JS)?;
    ensure_token(&directory)?;
    write_enabled_extensions(&with_uuid(&read_enabled_extensions().unwrap_or_default()))?;
    // 装完能力就变了（哪怕之前探到的是"没装"），缓存必须作废。
    invalidate_placement_probe();

    // Shell 不会热扫描新目录，所以首装时这一步注定失败；但"卸载后又装回来"的场景里
    // Shell 本次会话已经认识这个 uuid，这时就能立刻生效，省用户一次注销。
    let enabled_now = call_enable_extension().unwrap_or(false);
    let status = status();
    if !enabled_now && !status.active {
        log::info!("GNOME Shell 窗口扩展已写入 {}，需注销一次生效", UUID);
    }
    Ok(InstallOutcome {
        needs_logout: !status.active,
        status,
    })
}

/// 卸载：先让扩展停止服务，再清 gsettings 与文件。全程即时生效，不需要注销。
pub fn uninstall() -> Result<ShellExtensionStatus, String> {
    invalidate_placement_probe();
    let _ = call_disable_extension();
    write_enabled_extensions(&without_uuid(
        &read_enabled_extensions().unwrap_or_default(),
    ))?;
    let directory = extension_dir()?;
    if directory.exists() {
        std::fs::remove_dir_all(&directory)
            .map_err(|error| format!("删除 {} 失败: {error}", directory.display()))?;
    }
    Ok(status())
}

/// 启动自检：内嵌内容更新了就静默重写，目录被手工删掉就清掉 gsettings 里的孤儿 uuid。
///
/// 只在"用户已经装过"的前提下动手——从不擅自替用户安装 GNOME 扩展。
pub fn reconcile_on_startup() {
    if !crate::gsettings_shortcuts::is_gnome_desktop() {
        return;
    }
    let Ok(directory) = extension_dir() else {
        return;
    };
    let listed = read_enabled_extensions()
        .map(|entries| entries.iter().any(|entry| entry == UUID))
        .unwrap_or(false);

    if !directory.exists() {
        if listed {
            log::info!("扩展目录已被移除，清理 enabled-extensions 里的孤儿条目");
            let _ = write_enabled_extensions(&without_uuid(
                &read_enabled_extensions().unwrap_or_default(),
            ));
        }
        return;
    }
    if !listed {
        // 目录在但没登记（用户手动删过 gsettings 项），补回去比装着不生效更符合预期。
        let _ =
            write_enabled_extensions(&with_uuid(&read_enabled_extensions().unwrap_or_default()));
    }
    if !is_installed() {
        log::info!("GNOME Shell 窗口扩展内容已过期，静默升级");
        if let Err(error) = write_if_changed(&directory.join("metadata.json"), METADATA_JSON)
            .and_then(|()| write_if_changed(&directory.join("extension.js"), EXTENSION_JS))
        {
            log::warn!("升级 GNOME Shell 窗口扩展失败: {error}");
        }
    }
    if let Err(error) = ensure_token(&directory) {
        log::warn!("准备 GNOME Shell 窗口扩展令牌失败: {error}");
    }
}

// ---------------------------------------------------------------- 文件与令牌

fn data_home() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_DATA_HOME") {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            return Some(path);
        }
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
}

fn extension_dir() -> Result<PathBuf, String> {
    data_home()
        .map(|base| base.join("gnome-shell").join("extensions").join(UUID))
        .ok_or_else(|| "无法定位用户数据目录（XDG_DATA_HOME 与 HOME 都不可用）".to_string())
}

/// 两个文件都在、且内容与内嵌副本完全一致，才算"装好了"。内容不一致按未安装处理，
/// 由 `reconcile_on_startup` 重写，避免旧版扩展配新版协议。
fn is_installed() -> bool {
    let Ok(directory) = extension_dir() else {
        return false;
    };
    file_matches(&directory.join("metadata.json"), METADATA_JSON)
        && file_matches(&directory.join("extension.js"), EXTENSION_JS)
}

fn file_matches(path: &Path, expected: &str) -> bool {
    std::fs::read_to_string(path)
        .map(|actual| actual == expected)
        .unwrap_or(false)
}

fn write_if_changed(path: &Path, contents: &str) -> Result<(), String> {
    if file_matches(path, contents) {
        return Ok(());
    }
    std::fs::write(path, contents).map_err(|error| format!("写入 {} 失败: {error}", path.display()))
}

fn token_path(directory: &Path) -> PathBuf {
    directory.join(TOKEN_FILE_NAME)
}

/// 已有可用令牌就沿用（不打断正在跑的 Shell 侧缓存），否则生成一个新的。
fn ensure_token(directory: &Path) -> Result<String, String> {
    let path = token_path(directory);
    if let Some(existing) = read_token_at(&path) {
        return Ok(existing);
    }
    let token = random_token()?;
    let temp = path.with_extension("tmp");
    crate::private_files::write_private(&temp, token.as_bytes())
        .map_err(|error| format!("写入令牌失败: {error}"))?;
    std::fs::rename(&temp, &path).map_err(|error| format!("落地令牌文件失败: {error}"))?;
    Ok(token)
}

fn read_token() -> Option<String> {
    read_token_at(&token_path(&extension_dir().ok()?))
}

fn read_token_at(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    if !crate::private_files::is_private(path) {
        log::warn!("拒绝使用权限过宽的窗口扩展令牌");
        return None;
    }
    let token = std::fs::read_to_string(path).ok()?.trim().to_string();
    (token.len() >= MIN_TOKEN_LENGTH).then_some(token)
}

fn random_token() -> Result<String, String> {
    use std::io::Read;

    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| format!("读取 /dev/urandom 失败: {error}"))?;
    Ok(to_hex(&bytes))
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

// ---------------------------------------------------------------- gsettings

fn read_enabled_extensions() -> Option<Vec<String>> {
    let output = std::process::Command::new("gsettings")
        .args(["get", SHELL_SCHEMA, ENABLED_EXTENSIONS_KEY])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(crate::gsettings_shortcuts::parse_string_list(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

fn write_enabled_extensions(entries: &[String]) -> Result<(), String> {
    let status = std::process::Command::new("gsettings")
        .args([
            "set",
            SHELL_SCHEMA,
            ENABLED_EXTENSIONS_KEY,
            &crate::gsettings_shortcuts::format_string_list(entries),
        ])
        .status()
        .map_err(|error| format!("gsettings set {ENABLED_EXTENSIONS_KEY} 失败: {error}"))?;
    if !status.success() {
        return Err(format!(
            "gsettings set {ENABLED_EXTENSIONS_KEY} 返回非零退出码"
        ));
    }
    Ok(())
}

pub(super) fn with_uuid(entries: &[String]) -> Vec<String> {
    let mut result = entries.to_vec();
    if !result.iter().any(|entry| entry == UUID) {
        result.push(UUID.to_string());
    }
    result
}

pub(super) fn without_uuid(entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| entry.as_str() != UUID)
        .cloned()
        .collect()
}

// ---------------------------------------------------------------- D-Bus

/// 一次面向 gnome-shell 的阻塞方法调用。全部 D-Bus 都走 `crate::dbus`：
/// 直接用 `zbus::blocking` 会在 async 任务里 panic，理由见那个模块的注释。
fn shell_call<Args, Ret>(
    path: &'static str,
    interface: &'static str,
    method: &str,
    args: &Args,
) -> Result<Ret, zbus::Error>
where
    Args: serde::Serialize + zbus::zvariant::DynamicType + Sync,
    Ret: for<'d> zbus::zvariant::DynamicDeserialize<'d> + Send,
{
    crate::dbus::call(SHELL_BUS_NAME, path, interface, method, args)
}

/// 正在跑的那份扩展报出的协议版本。免鉴权：只暴露"装了没"，不含窗口信息。
///
/// 注意是"正在跑的"而不是"磁盘上的"：升级完文件到下次登录之间两者会不一致。
fn running_protocol_version() -> Option<u32> {
    let version: u32 =
        shell_call(WINDOWS_OBJECT_PATH, WINDOWS_INTERFACE, "GetVersion", &()).ok()?;
    (version <= EMBEDDED_PROTOCOL_VERSION).then_some(version)
}

fn call_get_windows(token: &str) -> Result<String, String> {
    shell_call(
        WINDOWS_OBJECT_PATH,
        WINDOWS_INTERFACE,
        "GetWindows",
        &(token,),
    )
    .map_err(|error| format!("GetWindows 调用失败: {error}"))
}

fn call_enable_extension() -> Result<bool, String> {
    call_extension_system("EnableExtension")
}

fn call_disable_extension() -> Result<bool, String> {
    call_extension_system("DisableExtension")
}

fn call_extension_system(method: &str) -> Result<bool, String> {
    shell_call(
        EXTENSIONS_OBJECT_PATH,
        EXTENSIONS_INTERFACE,
        method,
        &(UUID,),
    )
    .map_err(|error| format!("{method} 调用失败: {error}"))
}

/// 用户可以在 GNOME 里一键关掉所有扩展，关掉时装了也不会跑，状态卡片需要说明白。
fn user_extensions_enabled() -> bool {
    crate::dbus::property::<bool>(
        SHELL_BUS_NAME,
        EXTENSIONS_OBJECT_PATH,
        EXTENSIONS_INTERFACE,
        "UserExtensionsEnabled",
    )
    .unwrap_or(true)
}

pub(super) fn parse_windows(json: &str) -> Result<Vec<ShellWindow>, serde_json::Error> {
    serde_json::from_str(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 摆窗探测缓存的两条规则：成功的结果一直用（失效靠调用失败与安装/卸载），
    /// 失败的结果只压住一小段时间，否则用户在会话里重新打开扩展之后摆位再也不会恢复。
    #[test]
    fn failed_placement_probes_expire_but_successful_ones_do_not() {
        let long = PLACEMENT_PROBE_NEGATIVE_TTL + std::time::Duration::from_secs(1);
        assert!(placement_probe_is_fresh(true, long));
        assert!(placement_probe_is_fresh(
            false,
            std::time::Duration::from_millis(200)
        ));
        assert!(!placement_probe_is_fresh(false, long));
    }

    #[test]
    fn parses_the_measured_shell_payload_in_stacking_order() {
        // 实测载荷（GNOME Shell 50，全部为原生 Wayland 窗口，索引 0 是当时聚焦的终端）。
        let json = r#"[
          {"x":848,"y":37,"width":924,"height":1157,"title":"Ptyxis","wm_class":"org.gnome.Ptyxis","pid":4242},
          {"x":67,"y":270,"width":940,"height":700,"title":"Clash Verge","wm_class":"clash-verge","pid":1337}
        ]"#;
        let windows = parse_windows(json).expect("解析实测载荷失败");
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].title, "Ptyxis");
        assert_eq!(
            (
                windows[0].x,
                windows[0].y,
                windows[0].width,
                windows[0].height
            ),
            (848, 37, 924, 1157)
        );
        assert_eq!(windows[1].pid, 1337);
    }

    #[test]
    fn parse_tolerates_missing_optional_fields_but_not_missing_geometry() {
        let windows =
            parse_windows(r#"[{"x":1,"y":2,"width":3,"height":4}]"#).expect("解析缺省字段失败");
        assert_eq!(windows[0].title, "");
        assert_eq!(windows[0].pid, 0);
        assert!(parse_windows(r#"[{"x":1,"y":2}]"#).is_err());
        assert!(parse_windows("not json").is_err());
    }

    #[test]
    fn uuid_list_edits_are_idempotent() {
        let existing = vec!["ding@rastersoft.com".to_string()];
        let added = with_uuid(&existing);
        assert_eq!(added, vec!["ding@rastersoft.com", UUID]);
        // 重复安装不该写出两条。
        assert_eq!(with_uuid(&added), added);
        assert_eq!(without_uuid(&added), existing);
        // 卸载两次也不该动到别人的扩展。
        assert_eq!(without_uuid(&existing), existing);
    }

    #[test]
    fn embedded_extension_matches_the_uuid_and_token_contract() {
        // 三处 uuid（目录名、metadata、Rust 常量）必须一致，否则装进去 Shell 会拒载。
        assert!(METADATA_JSON.contains(&format!("\"uuid\": \"{UUID}\"")));
        // 令牌文件名与长度下限在两侧各写了一份，漂移会变成"永远鉴权失败"。
        assert!(EXTENSION_JS.contains(&format!("TOKEN_FILE_NAME = '{TOKEN_FILE_NAME}'")));
        assert!(EXTENSION_JS.contains(&format!("MIN_TOKEN_LENGTH = {MIN_TOKEN_LENGTH}")));
        assert!(EXTENSION_JS.contains(&format!("PROTOCOL_VERSION = {EMBEDDED_PROTOCOL_VERSION}")));
        assert!(EXTENSION_JS.contains(WINDOWS_INTERFACE));
        assert!(EXTENSION_JS.contains(WINDOWS_OBJECT_PATH));
        // 截图落地目录名两侧各写一份，漂移会让 Rust 拒收扩展给出的路径。
        assert!(EXTENSION_JS.contains(&format!("SCREENSHOT_DIR_NAME = '{SCREENSHOT_DIR_NAME}'")));
        // 六个方法都必须在内嵌的接口 XML 里声明，否则 wrapJSObject 根本不导出它们。
        for method in [
            "GetVersion",
            "GetWindows",
            "Screenshot",
            "ScreenshotArea",
            "CaptureArea",
            "PlaceWindow",
        ] {
            assert!(
                EXTENSION_JS.contains(&format!("<method name=\"{method}\">")),
                "内嵌扩展缺少 {method} 方法声明"
            );
        }
        // 截图是异步实现的，wrapJSObject 只认 `<Method>Async` 这个命名。
        assert!(EXTENSION_JS.contains("ScreenshotAsync(params, invocation)"));
        assert!(EXTENSION_JS.contains("ScreenshotAreaAsync(params, invocation)"));
        assert!(EXTENSION_JS.contains("CaptureAreaAsync(params, invocation)"));
        // 原始像素那条路的三个 GI 调用：少一个就只会在 journal 里报错并退回 PNG，
        // 而"退回 PNG"的症状就是慢回 1.8 秒——用户看得见，测试看不见，所以钉在这里。
        for api in [
            "global.stage.paint_to_content(",
            "content.get_texture()",
            "texture.get_data(Cogl.PixelFormat.RGBA_8888, stride, data)",
        ] {
            assert!(EXTENSION_JS.contains(api), "内嵌扩展缺少 {api}");
        }
        // 原始像素这条路唯一无法在本机测出的风险是 GJS 怎么编排那个没有长度标注的
        // `array<u8>`：万一它把 Uint8Array **复制**一份给 Cogl，Cogl 写的是副本，
        // 我们手里仍是原样——那是一张**全黑的图**，不是一个能被 catch 的异常，也就不会
        // 退回 PNG。所以两道自检必须在：纹理声明自己能不能读回，以及哨兵字节有没有被覆盖。
        assert!(EXTENSION_JS.contains("texture.is_get_data_supported()"));
        assert!(EXTENSION_JS.contains("the pixel buffer never reached Cogl"));
        // 出参签名两侧各写一份，漂了就是"路径能拿到、像素解释全错"。
        assert!(EXTENSION_JS.contains("new GLib.Variant('(siiis)'"));
        // 原始像素这条路失败时必须自动退回 PNG：慢，但画面仍然是对的。
        assert!(EXTENSION_JS.contains("falling back to PNG"));
        // 后缀是 Rust 侧 validate_screenshot_path 的白名单，两侧必须一致。
        for suffix in SCREENSHOT_EXTENSIONS {
            assert!(
                EXTENSION_JS.contains(&format!("'{suffix}'")),
                "内嵌扩展没写 .{suffix} 这个后缀"
            );
        }
        // 逐屏截图的 Shell API 与它的 finish 必须成对出现：只写一半会在运行时才炸，
        // 而那时错误只进 journal。参数顺序也钉住——错了会截到别处去。
        for api in [
            "screenshot_area(x, y, width, height, stream, done)",
            "screenshot_area_finish(result)",
        ] {
            assert!(EXTENSION_JS.contains(api), "内嵌扩展缺少 {api}");
        }
        // 每次都新建 ShellScreenshot 实例，否则同时发起的逐屏截图会互相顶掉
        // （同一个实例第二次直接 G_IO_ERROR_PENDING）。
        assert!(EXTENSION_JS.contains("start(new Shell.Screenshot(), stream,"));
        // 摆放是同步实现，签名顺序必须和 Rust 侧 place_window 的入参一致，
        // 顺序漂了 D-Bus 只会报参数类型不匹配，排查起来极其费劲。
        assert!(EXTENSION_JS.contains("PlaceWindow(token, pid, marker, x, y, reposition, above)"));
        // 置顶与摆放各自对应的 Mutter API 必须都在，少一个就只有一半功能。
        for api in ["make_above()", "unmake_above()", "move_frame(false, x, y)"] {
            assert!(EXTENSION_JS.contains(api), "内嵌扩展缺少 {api}");
        }
    }

    /// 读完就删，所以路径校验必须严：只认约定目录里的 .png / .rgba。
    #[test]
    fn screenshot_path_must_stay_in_the_agreed_directory() {
        let directory = Path::new("/run/user/1000").join(SCREENSHOT_DIR_NAME);
        assert!(validate_screenshot_path(&directory, &directory.join("frame-abc.png")).is_ok());
        // 原始像素那条路写的是 .rgba，同样必须放行，否则 v5 一上来就整体退回。
        assert!(validate_screenshot_path(&directory, &directory.join("frame-abc.rgba")).is_ok());
        // 别的目录、上跳一级、后缀不在册的一律拒收。
        for rejected in [
            PathBuf::from("/run/user/1000/frame-abc.png"),
            directory.join("nested").join("frame-abc.png"),
            directory.join("frame-abc.png.txt"),
            directory.join("frame-abc.rgba.sh"),
            directory.join("frame-abc"),
        ] {
            assert!(
                validate_screenshot_path(&directory, &rejected).is_err(),
                "{} 本该被拒",
                rejected.display()
            );
        }
    }

    /// 应答里的格式与尺寸就是像素的解释方式，错一点整张画面就斜掉，所以逐项钉住。
    #[test]
    fn capture_replies_are_interpreted_strictly() {
        let path = PathBuf::from("/run/user/1000/clippy-shots/frame-a.rgba");
        assert_eq!(
            interpret_capture_reply(path.clone(), 2560, 1600, 10240, "RGBA").unwrap(),
            AreaCapture::Raw {
                path: path.clone(),
                width: 2560,
                height: 1600,
                stride: 10240,
            }
        );
        // stride 大于一行是合法的（Cogl 可以按自己的对齐给），小于一行不可能是真的。
        assert!(interpret_capture_reply(path.clone(), 2560, 1600, 10496, "RGBA").is_ok());
        assert!(interpret_capture_reply(path.clone(), 2560, 1600, 10236, "RGBA").is_err());
        // 退回 PNG 时宽高与 stride 是 0，由调用方从文件头读，不该被当成非法。
        assert_eq!(
            interpret_capture_reply(path.clone(), 0, 0, 0, "PNG").unwrap(),
            AreaCapture::Png { path: path.clone() }
        );
        // 空/负尺寸与没听说过的格式一律拒收：宁可退回整屏那条路，也不要拿它当像素解。
        for (width, height, stride, format) in [
            (0, 1600, 10240, "RGBA"),
            (2560, 0, 10240, "RGBA"),
            (-1, 1600, 10240, "RGBA"),
            (2560, 1600, -1, "RGBA"),
            (2560, 1600, 10240, "JPEG"),
        ] {
            assert!(
                interpret_capture_reply(path.clone(), width, height, stride, format).is_err(),
                "{width}x{height} stride {stride} {format} 本该被拒"
            );
        }
    }

    /// 逐屏取画面是"全成才算成"：任何一块屏失败，已经落地的文件都要删掉，
    /// 否则 XDG_RUNTIME_DIR 里会攒下再没人读的整屏画面。
    #[test]
    fn a_failed_area_screenshot_cleans_up_the_ones_that_landed() {
        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let landed = directory.path().join("frame-a.rgba");
        std::fs::write(&landed, b"pixels").expect("写入临时画面失败");

        let error = collect_area_captures(vec![
            Ok(AreaCapture::Raw {
                path: landed.clone(),
                width: 1,
                height: 1,
                stride: 4,
            }),
            Err("第二块屏取像素失败".to_string()),
        ])
        .expect_err("有失败就该整体报错");
        assert_eq!(error, "第二块屏取像素失败");
        assert!(!landed.exists(), "失败路径上的临时画面没有被删掉");
    }

    #[test]
    fn area_screenshots_keep_the_order_they_were_requested_in() {
        let captures = vec![
            AreaCapture::Png {
                path: PathBuf::from("/a.png"),
            },
            AreaCapture::Raw {
                path: PathBuf::from("/b.rgba"),
                width: 2,
                height: 1,
                stride: 8,
            },
        ];
        assert_eq!(
            collect_area_captures(captures.iter().cloned().map(Ok).collect()).unwrap(),
            captures
        );
    }

    #[test]
    fn token_round_trip_is_private_and_reused() {
        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let first = ensure_token(directory.path()).expect("生成令牌失败");
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        // 再次安装沿用同一个令牌，不打断已经在跑的 Shell 侧。
        assert_eq!(ensure_token(directory.path()).expect("复用令牌失败"), first);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(token_path(directory.path()))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_world_readable_token() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let path = token_path(directory.path());
        std::fs::write(&path, "0123456789abcdef0123").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(read_token_at(&path), None);
    }

    #[test]
    fn refuses_truncated_token() {
        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let path = token_path(directory.path());
        crate::private_files::write_private(&path, b"short").unwrap();
        assert_eq!(read_token_at(&path), None);
    }

    /// 真机诊断：从 async 任务里跑一遍全部 D-Bus 调用。
    ///
    /// 这三个函数在生产里就是被 `#[tauri::command] async fn` 的函数体直接调的，
    /// 而阻塞 D-Bus 调用在那种线程上曾经必 panic（见 `crate::dbus`）。CI 里没有
    /// session bus，所以默认忽略；`cargo test -- --ignored --nocapture` 手动跑。
    #[test]
    #[ignore]
    fn shell_extension_diagnostics() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("创建诊断 runtime 失败");
        runtime.block_on(async {
            println!("status  = {:?}", status());
            println!("running = {:?}", running_protocol_version());
            println!("hint    = {}", hint_needed());
            println!(
                "windows = {:?}",
                probe().map(|windows| windows.len()).unwrap_or(0)
            );
            let shot = request_screenshot();
            println!("shot    = {shot:?}");
            // 截图的处置权在调用方，诊断也不例外：不删的话 XDG_RUNTIME_DIR 里会攒垃圾。
            if let Ok(path) = shot {
                let _ = std::fs::remove_file(path);
            }
        });
    }

    #[test]
    fn write_if_changed_skips_identical_content() {
        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let path = directory.path().join("extension.js");
        write_if_changed(&path, "one").expect("首次写入失败");
        let first = std::fs::metadata(&path).unwrap().modified().unwrap();
        write_if_changed(&path, "one").expect("重复写入失败");
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), first);
        write_if_changed(&path, "two").expect("升级写入失败");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
    }
}
