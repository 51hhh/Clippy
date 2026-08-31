//! 截图几何诊断报告：一份可以直接贴进 issue 的纯文本。
//!
//! **它解决的是一个信息不对称。** 几何算错时用户看到的是"覆盖层错位""画面溢到隔壁屏"，
//! 而判断根因需要的数字（每块屏上报了什么、舞台图多大、切出来的裁剪是什么、
//! 覆盖层实际被摆成多大）全在进程里。没有这份报告，一次报障要来回问五轮还问不准；
//! 有了它，第一条消息里通常就已经带着答案。
//!
//! **三条硬约束，改这个文件时不要破：**
//!
//! 1. **不含截图像素。** 舞台图只读 PNG 头取尺寸，文件当场删掉。
//! 2. **不含窗口标题。** 标题泄露用户正在做什么，和扩展 `GetWindows` 那个令牌是同一套
//!    威胁模型（docs/capture-linux.md §2.1）。窗口候选整段不进报告。
//! 3. **绝不自动上传。** 只写本地缓存目录并原样回给前端显示，发不发由用户自己决定。
//!
//! 报告末尾的 `monitor-layout` 段落**就是** `tests/fixtures/monitor-layouts/` 的格式：
//! 存成一个 json 丢进那个目录，PR 就完整了，不需要写一行 Rust（见 CONTRIBUTING.md）。

use super::manager::ViewportObservation;
use super::shell_extension::{self, ShellExtensionStatus};
use crate::commands::AppState;
use serde::Serialize;
use tauri::{Manager, State};

/// fixture 里的 `name`。回归测试要求它非空，而合适的名字只有提 PR 的人知道，
/// 所以这里给一个中性的占位，让用户改文件名时顺手改掉。
const FIXTURE_NAME: &str = "monitor-layout";

/// 没填备注时的占位。fixture 的 `note` 不允许为空（回归测试会拒），所以必须是一句
/// 有信息量的话，而不是空字符串。
const DEFAULT_NOTE: &str = "由 Clippy 截图诊断自动生成；请补充这台机器的排布与症状";

/// 报告文件名。固定名字、每次覆盖：诊断报告攒一堆没意义，而"上次那份在哪"要好找。
const REPORT_FILE: &str = "capture-diagnostics.txt";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDiagnosticsReport {
    /// 完整报告文本。前端原样显示——用户看得到自己要发出去的每一个字。
    pub text: String,
    /// 落盘位置；写不进去时是 `None`，报告本身照样有效。
    pub path: Option<String>,
    /// 可以直接存成 fixture 的 json；拿不到舞台图时为 `None`。
    pub fixture_json: Option<String>,
}

/// 只报告环境变量**在不在**，不报告值。
///
/// `WAYLAND_DISPLAY` / `DISPLAY` 的值本身没有诊断价值（`wayland-0`、`:0`），
/// 但它们在不在决定了走哪条后端。
fn env_presence(key: &str) -> String {
    match std::env::var(key) {
        Ok(value) if !value.is_empty() => format!("{key} = 已设置"),
        _ => format!("{key} = 未设置"),
    }
}

fn env_value(key: &str) -> String {
    match std::env::var(key) {
        Ok(value) if !value.is_empty() => format!("{key} = {value}"),
        _ => format!("{key} = 未设置"),
    }
}

/// 这台机器的桌面环境，一行。也用作 fixture 的 `compositor` 字段。
fn compositor_line() -> String {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "未知".to_string());
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "未知".to_string());
    format!("{desktop} ({session})")
}

/// 扩展状态一行。装没装、跑没跑、是不是等注销——这三件事直接决定截图走哪条后端。
fn extension_line(status: &ShellExtensionStatus) -> String {
    if !status.supported {
        return "窗口速选扩展：当前会话用不上（只有 GNOME Wayland 需要）".to_string();
    }
    let state = if status.active && status.stale {
        "在应答，但跑的是旧版（磁盘已升级，等注销一次）"
    } else if status.active {
        "在应答，已就绪"
    } else if status.installed && !status.user_extensions_enabled {
        "已安装，但系统层面关掉了全部扩展"
    } else if status.installed {
        "已安装，等注销一次生效"
    } else {
        "未安装"
    };
    format!(
        "窗口速选扩展：{state}（installed={} enabled={} active={} stale={}）",
        status.installed, status.enabled, status.active, status.stale,
    )
}

/// I4 那一行。**"没观测过"和"通过了"必须分开说**——把没查过写成通过，
/// 会让排障的人绕开唯一一条闭环自检。
fn invariant_i4_line(observation: Option<ViewportObservation>) -> String {
    let Some(observation) = observation else {
        return "I4   覆盖层视口   未观测（这个进程还没截过图；先截一次再回来看）".to_string();
    };
    match observation.mismatch {
        None => format!(
            "I4   覆盖层视口   PASS（实测 {}x{} == 逻辑尺寸 {}x{}）",
            observation.actual.0,
            observation.actual.1,
            observation.expected.0,
            observation.expected.1,
        ),
        Some((dx, dy)) => format!(
            "I4   覆盖层视口   FAIL 实测 {}x{}，逻辑尺寸 {}x{}，差 {dx}x{dy}——几何算错了，界面正在错位",
            observation.actual.0,
            observation.actual.1,
            observation.expected.0,
            observation.expected.1,
        ),
    }
}

/// 一次 I5 观测：Shell 扩展报的"自己那个窗口"，和 Tauri 自己知道的同一个窗口。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OwnWindowObservation {
    /// Tauri 侧：`outer_position` / `outer_size` 换算出的逻辑矩形。
    pub expected: (i32, i32, u32, u32),
    /// 扩展侧：`GetWindows` 里 `pid == 自己` 那一条的 frame_rect。
    pub actual: (i32, i32, u32, u32),
}

/// I5 允许的相对误差。
///
/// **I5 按比例判，不按像素差判。** 要抓的是坐标空间错位（1.125×、1.5×、2× 这种量级），
/// 而扩展给的 frame_rect 不含 CSD 阴影、Tauri 的 `outer_size` 含不含随 GTK 版本而异，
/// 两者天然差一圈几十像素。按像素判会天天误报，按比例判只在真的错了空间时才响。
const OWN_WINDOW_RATIO_TOLERANCE: f64 = 0.08;

/// **不变量 I5：扩展报出的窗口坐标必须和覆盖层用的逻辑坐标在同一个空间。**
///
/// I4 闭合的是"覆盖层被摆成多大"，I5 闭合的是"窗口候选画在哪"——两条独立的错法。
/// 唯一能同时被两边看到的参照物是 **Clippy 自己的窗口**：扩展按 pid 报出它，Tauri 自己
/// 也知道它的逻辑几何，两个数字必须一致。这个判据不需要知道用户有几块屏、怎么缩放。
///
/// 返回横纵各自的比例（扩展 ÷ Tauri），都在容差内时返回 `None`。
fn own_window_ratio(observation: OwnWindowObservation) -> Option<(f64, f64)> {
    let (_, _, expected_width, expected_height) = observation.expected;
    let (_, _, actual_width, actual_height) = observation.actual;
    if expected_width == 0 || expected_height == 0 {
        return None;
    }
    let rx = actual_width as f64 / expected_width as f64;
    let ry = actual_height as f64 / expected_height as f64;
    let off = |ratio: f64| !ratio.is_finite() || (ratio - 1.0).abs() > OWN_WINDOW_RATIO_TOLERANCE;
    (off(rx) || off(ry)).then_some((rx, ry))
}

fn format_rect(rect: (i32, i32, u32, u32)) -> String {
    format!("{},{} {}x{}", rect.0, rect.1, rect.2, rect.3)
}

/// I5 那一行。拿不到参照物时说"未检查"并给出原因——它比 I1–I4 更容易缺（要扩展在跑、
/// 而且刚好只有一个自己的窗口开着），所以原因必须写出来，否则用户不知道怎么补。
fn invariant_i5_line(observation: &Result<OwnWindowObservation, String>) -> String {
    match observation {
        Err(reason) => format!("I5   窗口坐标空间 未检查（{reason}）"),
        Ok(observation) => match own_window_ratio(*observation) {
            None => format!(
                "I5   窗口坐标空间 PASS（扩展 {} ≈ Tauri {}）",
                format_rect(observation.actual),
                format_rect(observation.expected),
            ),
            Some((rx, ry)) => format!(
                "I5   窗口坐标空间 FAIL 扩展 {}，Tauri {}，比例 {rx:.4}x{ry:.4}\
                 ——窗口候选会整体偏移或缩放",
                format_rect(observation.actual),
                format_rect(observation.expected),
            ),
        },
    }
}

/// I1–I3 每条一行。**"没检查"要和"通过"分开**，理由同 I4。
fn invariant_lines(warnings: &[String], has_stage: bool) -> Vec<String> {
    [
        ("I1", "舞台图比例  "),
        ("I2a", "裁剪不重叠  "),
        ("I2b", "裁剪未被夹  "),
        // 标签宽度按 CJK 字形对齐（"各屏同向缩放"已占满，只补一个空格与结论分开）
        ("I3", "各屏同向缩放 "),
    ]
    .into_iter()
    .map(|(tag, label)| {
        let failures: Vec<&str> = warnings
            .iter()
            .filter(|warning| warning.split_whitespace().next() == Some(tag))
            .map(|warning| warning.as_str())
            .collect();
        let verdict = if !has_stage {
            "未检查（拿不到舞台图）".to_string()
        } else if failures.is_empty() {
            "PASS".to_string()
        } else {
            failures.join(" / ")
        };
        format!("{tag:<5}{label}{verdict}")
    })
    .collect()
}

/// 拼出整份报告。
///
/// 刻意做成纯函数：不碰 D-Bus、不碰文件，全部输入由调用方传进来，于是排版可以单测，
/// 而排版恰恰是最容易悄悄退化的部分——报障的人读的就是这几行。
fn render(
    version: &str,
    extension: &ShellExtensionStatus,
    geometry: &crate::screenshot::diagnostics::GeometryDiagnostics,
    viewport: Option<ViewportObservation>,
    own_window: &Result<OwnWindowObservation, String>,
) -> String {
    let mut out = String::new();
    out.push_str("=== Clippy 截图几何诊断 ===\n");
    out.push_str(&format!("应用版本：{version}\n"));
    out.push_str(&format!("{}\n", env_value("XDG_CURRENT_DESKTOP")));
    out.push_str(&format!("{}\n", env_value("XDG_SESSION_TYPE")));
    out.push_str(&format!("{}\n", env_presence("WAYLAND_DISPLAY")));
    out.push_str(&format!("{}\n", env_presence("DISPLAY")));
    out.push_str(&format!("{}\n", extension_line(extension)));

    out.push_str("\n--- 显示器几何（逐来源）---\n");
    for source in &geometry.sources {
        out.push_str(&format!("[{}]\n", source.source));
        match &source.lines {
            Ok(lines) => {
                for line in lines {
                    out.push_str("  ");
                    out.push_str(line);
                    out.push('\n');
                }
            }
            Err(error) => out.push_str(&format!("  枚举失败：{error}\n")),
        }
    }

    out.push_str("\n--- 舞台图与切分 ---\n");
    match &geometry.stage {
        Ok(stage) => out.push_str(&format!(
            "后端：{}，几何来源：{}\n舞台图：{}x{}\n{}\n",
            stage.backend, stage.geometry_source, stage.width, stage.height, stage.summary,
        )),
        Err(error) => out.push_str(&format!(
            "拿不到整张舞台图：{error}\n\
             （wlroots 系合成器逐输出抓图，本来就不走切分这条路，这里为空是正常的）\n"
        )),
    }

    out.push_str("\n--- 不变量自检 ---\n");
    let warnings = geometry
        .stage
        .as_ref()
        .map(|stage| stage.warnings.as_slice())
        .unwrap_or_default();
    for line in invariant_lines(warnings, geometry.stage.is_ok()) {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(&invariant_i4_line(viewport));
    out.push('\n');
    out.push_str(&invariant_i5_line(own_window));
    out.push('\n');

    if let Ok(stage) = &geometry.stage {
        out.push_str(
            "\n--- monitor-layout ---\n\
             存成 src-tauri/tests/fixtures/monitor-layouts/<名字>.json 就是一条回归测试，\
             改一下 name/note 直接提 PR（见 CONTRIBUTING.md）：\n",
        );
        out.push_str(&stage.fixture_json);
        out.push('\n');
    }

    out.push_str(
        "\n本报告不含任何截图像素，也不含任何窗口标题；文件只写在本机缓存目录，不会自动上传。\n",
    );
    out
}

/// 多个自己的窗口同时开着时的说法。两边都可能撞上，措辞统一一处。
const AMBIGUOUS_OWN_WINDOW: &str = "同时开着多个 Clippy 窗口，分不清哪个对哪个；只留一个再试";

/// I5 的 Tauri 侧参照物：自己那个窗口的**逻辑**外框。
///
/// **必须在 async / 主线程侧调用。** 读窗口属性要走 GTK 主循环，在 `spawn_blocking`
/// 的工作线程上调是未定义行为——这也是它和 [`own_window_observation`] 分成两半的原因，
/// 后者反过来只能在阻塞线程上跑（里面是 D-Bus）。
fn own_window_logical_rect(app_handle: &tauri::AppHandle) -> Result<(i32, i32, u32, u32), String> {
    // 只认可见窗口：藏起来的窗口在扩展的列表里根本不出现，配对时会张冠李戴。
    let mut visible = app_handle
        .webview_windows()
        .into_values()
        .filter(|window| window.is_visible().unwrap_or(false));
    let own = visible.next().ok_or("没有可见的 Clippy 窗口")?;
    if visible.next().is_some() {
        return Err(AMBIGUOUS_OWN_WINDOW.to_string());
    }

    let scale = own.scale_factor().map_err(|error| error.to_string())?;
    if !scale.is_finite() || scale <= 0.0 {
        return Err(format!("窗口缩放比例不可用（{scale}）"));
    }
    let position = own.outer_position().map_err(|error| error.to_string())?;
    let size = own.outer_size().map_err(|error| error.to_string())?;
    Ok((
        (position.x as f64 / scale).round() as i32,
        (position.y as f64 / scale).round() as i32,
        (size.width as f64 / scale).round() as u32,
        (size.height as f64 / scale).round() as u32,
    ))
}

/// 把 Tauri 侧的参照物和扩展侧的同一个窗口凑成一次 I5 观测。**阻塞**（D-Bus）。
///
/// **配对靠"两边各只有一个"这个条件，不靠猜。** 扩展只给 pid，同一个 pid 下的多个窗口
/// 分不出谁是谁。只有"恰好一个自己的窗口可见"且"扩展也只报出一条自己的"时两者必然
/// 是同一个窗口。用户点这个按钮时设置窗口正开着、主面板一般是关的，所以这个条件在实际
/// 场景里成立；配不上就诚实地记未检查。
///
/// 顺带解释了为什么 I5 只能是按需诊断、不能是截图时的自检：截图前 `hide_sources`
/// 已经把自己的窗口藏了，藏起来的窗口既不在扩展的列表里、也没有可信的几何。
fn own_window_observation(
    expected: Result<(i32, i32, u32, u32), String>,
) -> Result<OwnWindowObservation, String> {
    let expected = expected?;
    let own_pid = std::process::id();
    let windows = shell_extension::probe().ok_or("扩展没在应答，拿不到窗口几何")?;
    let mut mine = windows.iter().filter(|window| window.pid == own_pid);
    // 只取 x/y/width/height；`title` 绝不能进报告（会泄露用户在做什么）。
    let shell = mine
        .next()
        .ok_or("扩展的窗口列表里没有 Clippy 自己的窗口")?;
    if mine.next().is_some() {
        return Err(AMBIGUOUS_OWN_WINDOW.to_string());
    }
    Ok(OwnWindowObservation {
        expected,
        actual: (
            shell.x,
            shell.y,
            shell.width.max(0) as u32,
            shell.height.max(0) as u32,
        ),
    })
}

/// 采集一份报告。**阻塞**：里面有 D-Bus 往返和一次真实的舞台图请求（约 550 ms）。
///
/// 不要求 `AppHandle`，所以命令行入口（`--capture-diagnose`）可以在 Tauri 起来之前就用它。
pub(crate) fn collect_report(
    version: &str,
    viewport: Option<ViewportObservation>,
    own_window_rect: Result<(i32, i32, u32, u32), String>,
    note: Option<String>,
) -> (String, Option<String>) {
    let own_window = own_window_observation(own_window_rect);
    let extension = shell_extension::status();
    let compositor = compositor_line();
    let note = note
        .map(|note| note.trim().to_string())
        .filter(|note| !note.is_empty())
        .unwrap_or_else(|| DEFAULT_NOTE.to_string());
    let geometry = crate::screenshot::diagnostics::collect(FIXTURE_NAME, &note, &compositor);
    let text = render(version, &extension, &geometry, viewport, &own_window);
    let fixture_json = geometry
        .stage
        .as_ref()
        .ok()
        .map(|stage| stage.fixture_json.clone());
    (text, fixture_json)
}

/// 命令行请求的诊断模式。
///
/// 有命令行入口是因为**几何算错的时候图形界面本身就不可信**：覆盖层错位、面板跑到隔壁屏，
/// 让用户去点设置页里的按钮是在最不合适的时候要求他操作 GUI。一条 `clippy --capture-diagnose`
/// 走的是同一份 [`collect_report`]，结论完全一样。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliMode {
    /// 整份人读报告。
    Report,
    /// 只吐 fixture json，可以直接 `> tests/fixtures/monitor-layouts/xxx.json`。
    TestCase,
}

/// 环境变量开关。和 `--capture-diagnose` 等价，给"从桌面图标启动、加不了参数"的场景用。
pub const DIAGNOSE_ENV: &str = "CLIPPY_CAPTURE_DIAGNOSE";

/// 从参数与环境变量里解析诊断模式。纯函数，故意不碰 `std::env`，这样可以单测。
///
/// `--emit-test-case` 优先：同时给了两个的时候，用户显然是想要那个能重定向进文件的。
pub fn cli_mode(args: &[String], diagnose_env: Option<&str>) -> Option<CliMode> {
    if args.iter().any(|arg| arg == "--emit-test-case") {
        return Some(CliMode::TestCase);
    }
    if args.iter().any(|arg| arg == "--capture-diagnose") {
        return Some(CliMode::Report);
    }
    // "0" / "false" / 空串都当没开：环境变量常被 systemd 之类的东西设成 0 来表示关闭。
    match diagnose_env {
        Some(value) if !matches!(value.trim(), "" | "0" | "false") => Some(CliMode::Report),
        _ => None,
    }
}

/// `--note=<文本>` 的值。fixture 的 `note` 是给下一个人读的，只有用户知道该写什么。
pub fn cli_note(args: &[String]) -> Option<String> {
    args.iter()
        .find_map(|arg| arg.strip_prefix("--note=").map(|note| note.to_string()))
}

/// 命令行入口。**在 Tauri 起来之前调用**——这里全是阻塞 D-Bus，而且不该拉起窗口、
/// 不该撞上 single-instance 的 name 抢占。返回进程退出码。
pub fn run_cli(mode: CliMode, version: &str, note: Option<String>) -> i32 {
    // 命令行模式下自己没有窗口可比对，I5 只能记成"未检查"——写成 PASS 会让人绕开它。
    let own_window = Err("命令行模式没有窗口可比对；从设置页运行可以查这一条".to_string());
    let (text, fixture_json) = collect_report(version, None, own_window, note);
    match mode {
        CliMode::Report => {
            println!("{text}");
            0
        }
        CliMode::TestCase => match fixture_json {
            Some(json) => {
                println!("{json}");
                0
            }
            None => {
                // 报告本身仍然有诊断价值，所以走 stderr 一并给出，别让用户拿到一片空白。
                eprintln!(
                    "拿不到整张舞台图，生成不了 fixture（wlroots 系合成器逐输出抓图，本来就没有舞台图）。\n\
                     完整报告：\n{text}"
                );
                1
            }
        },
    }
}

/// 把报告写进缓存目录。**写失败不算失败**：报告已经在内存里，前端照样能显示与复制。
fn write_report(app_handle: &tauri::AppHandle, text: &str) -> Option<String> {
    let dir = app_handle.path().app_cache_dir().ok()?;
    if let Err(error) = std::fs::create_dir_all(&dir) {
        log::warn!("诊断报告目录建不出来 {}: {error}", dir.display());
        return None;
    }
    let path = dir.join(REPORT_FILE);
    match std::fs::write(&path, text) {
        Ok(()) => Some(path.display().to_string()),
        Err(error) => {
            log::warn!("诊断报告写不进去 {}: {error}", path.display());
            None
        }
    }
}

/// 设置页的"运行诊断"按钮。
///
/// 走 `spawn_blocking`：里面有阻塞 D-Bus 调用（扩展状态、扩展截图），在 tokio async
/// worker 上直接调必然 panic（`Cannot start a runtime from within a runtime`，见 `dbus.rs`）。
#[tauri::command]
pub async fn run_capture_diagnostics(
    note: Option<String>,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<CaptureDiagnosticsReport, String> {
    // `State` 借在这个 async fn 的栈上，跨不进 `spawn_blocking`，所以先把要用的那一份取出来。
    let viewport = state.capture_manager.last_viewport();
    let version = app_handle.package_info().version.to_string();
    // I5 的 Tauri 侧参照物也得在这儿取：读窗口属性要走 GTK 主循环，挪进
    // `spawn_blocking` 的工作线程是未定义行为。扩展那一半反过来只能在阻塞线程上跑。
    let own_window_rect = own_window_logical_rect(&app_handle);
    let handle = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (text, fixture_json) = collect_report(&version, viewport, own_window_rect, note);
        let path = write_report(&handle, &text);
        CaptureDiagnosticsReport {
            text,
            path,
            fixture_json,
        }
    })
    .await
    .map_err(|error| format!("诊断线程异常退出：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(active: bool, stale: bool) -> ShellExtensionStatus {
        ShellExtensionStatus {
            supported: true,
            installed: true,
            enabled: true,
            active,
            stale,
            user_extensions_enabled: true,
        }
    }

    /// "装了但要注销"和"已就绪"在报障里是完全不同的结论，不能都写成"已安装"。
    #[test]
    fn the_extension_line_separates_pending_logout_from_ready() {
        assert!(extension_line(&status(true, false)).contains("已就绪"));
        assert!(extension_line(&status(true, true)).contains("旧版"));
        assert!(extension_line(&status(false, false)).contains("等注销"));
        assert!(extension_line(&ShellExtensionStatus {
            supported: false,
            ..status(false, false)
        })
        .contains("用不上"));
    }

    /// **没检查 ≠ 通过。** 拿不到舞台图时把 I1–I3 写成 PASS，会让人以为几何没问题，
    /// 从而绕开真正的根因。
    #[test]
    fn invariants_without_a_stage_image_are_reported_as_unchecked() {
        let lines = invariant_lines(&[], false);
        assert_eq!(lines.len(), 4);
        for line in &lines {
            assert!(line.contains("未检查"), "{line}");
            assert!(!line.contains("PASS"), "{line}");
        }
    }

    /// 告警要落在对应那一行上，别的行仍是 PASS——否则一条 I2a 会污染整张表。
    #[test]
    fn a_warning_lands_on_its_own_invariant_row() {
        let warnings = vec!["I2a 显示器 2 的裁剪和显示器 1 重叠".to_string()];
        let lines = invariant_lines(&warnings, true);
        assert!(lines[0].starts_with("I1"), "{}", lines[0]);
        assert!(lines[0].contains("PASS"), "{}", lines[0]);
        assert!(lines[1].contains("重叠"), "{}", lines[1]);
        assert!(!lines[1].contains("PASS"), "{}", lines[1]);
        assert!(lines[2].contains("PASS"), "{}", lines[2]);
        assert!(lines[3].contains("PASS"), "{}", lines[3]);
    }

    /// I2a 和 I2b 都以 "I2" 开头，前缀匹配会把它们混成一行。这里钉住按整词比。
    #[test]
    fn i2a_and_i2b_do_not_bleed_into_each_other() {
        let warnings = vec!["I2b 显示器 1 的裁剪被舞台图边界钳掉 12 像素".to_string()];
        let lines = invariant_lines(&warnings, true);
        assert!(lines[1].contains("PASS"), "I2a 被 I2b 污染了：{}", lines[1]);
        assert!(lines[2].contains("钳掉"), "{}", lines[2]);
    }

    /// 那次真实事故的数字。报障里最有价值的一行就是它。
    #[test]
    fn the_i4_row_spells_out_the_real_incident() {
        let line = invariant_i4_line(Some(ViewportObservation {
            expected: (2160, 1350),
            actual: (1920, 1200),
            mismatch: Some((-240, -150)),
        }));
        assert!(line.contains("FAIL"), "{line}");
        assert!(line.contains("1920x1200"), "{line}");
        assert!(line.contains("2160x1350"), "{line}");
        assert!(line.contains("-240x-150"), "{line}");
    }

    #[test]
    fn an_unobserved_viewport_is_not_reported_as_passing() {
        let line = invariant_i4_line(None);
        assert!(line.contains("未观测"), "{line}");
        assert!(!line.contains("PASS"), "{line}");
    }

    /// I5 要抓的就是这一类：扩展报的坐标整体比 Tauri 的大一个缩放倍数。
    /// 1.125× 是那次真实事故的倍数（把 1.5 的舞台缩放套到 4/3 的屏上）。
    #[test]
    fn the_i5_row_catches_a_whole_coordinate_space_being_off() {
        for factor in [1.125_f64, 1.5, 2.0] {
            let line = invariant_i5_line(&Ok(OwnWindowObservation {
                expected: (100, 200, 800, 600),
                actual: (
                    (100.0 * factor) as i32,
                    (200.0 * factor) as i32,
                    (800.0 * factor) as u32,
                    (600.0 * factor) as u32,
                ),
            }));
            assert!(line.contains("FAIL"), "{factor}: {line}");
            // 比例本身要印出来：它直接告诉看报障的人错了几倍。
            assert!(line.contains(&format!("{factor:.4}")), "{factor}: {line}");
        }
    }

    /// **这条是 I5 按比例判而不按像素差判的全部理由。** 扩展的 frame_rect 不含 CSD 阴影，
    /// Tauri 的 `outer_size` 含不含随 GTK 版本而异，两者天然差一圈几十像素。
    /// 按像素判会天天误报，那样的检查很快就没人看了。
    #[test]
    fn a_csd_shadow_sized_difference_is_not_a_failure() {
        let line = invariant_i5_line(&Ok(OwnWindowObservation {
            expected: (74, 174, 852, 652),
            actual: (100, 200, 800, 600),
        }));
        assert!(line.contains("PASS"), "{line}");
    }

    /// 同上，但差到 8% 容差之外就必须响——否则 1.125× 也会被当成阴影糊过去。
    #[test]
    fn a_difference_beyond_the_tolerance_still_fails() {
        let line = invariant_i5_line(&Ok(OwnWindowObservation {
            expected: (100, 200, 800, 600),
            actual: (100, 200, 880, 600),
        }));
        assert!(line.contains("FAIL"), "{line}");
    }

    /// I5 比 I1–I4 更容易缺（要扩展在跑、还得刚好只有一个自己的窗口开着），
    /// 所以"未检查"必须带原因，而且绝不能写成 PASS。
    #[test]
    fn an_unpaired_i5_says_why_it_was_skipped_and_never_says_passing() {
        let line = invariant_i5_line(&Err("扩展没在应答，拿不到窗口几何".to_string()));
        assert!(line.contains("未检查"), "{line}");
        assert!(line.contains("扩展没在应答"), "{line}");
        assert!(!line.contains("PASS"), "{line}");
    }

    /// 宽或高为 0 时比例没有意义，不能拿 inf/NaN 去判——那会变成一条读不懂的 FAIL。
    #[test]
    fn a_degenerate_rect_does_not_produce_a_nonsense_ratio() {
        assert_eq!(
            own_window_ratio(OwnWindowObservation {
                expected: (0, 0, 0, 0),
                actual: (0, 0, 800, 600),
            }),
            None
        );
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    /// 正常启动**绝不能**被误判成诊断——那会让应用只打印一段文字就退出。
    #[test]
    fn a_normal_launch_is_not_mistaken_for_a_diagnostic_run() {
        assert_eq!(cli_mode(&args(&["clippy"]), None), None);
        assert_eq!(cli_mode(&args(&["clippy", "--autostart"]), None), None);
        // 环境变量被设成"关闭"的那几种写法
        for off in ["", " ", "0", "false"] {
            assert_eq!(cli_mode(&args(&["clippy"]), Some(off)), None, "{off:?}");
        }
    }

    #[test]
    fn emit_test_case_wins_so_the_output_can_be_redirected_into_a_file() {
        assert_eq!(
            cli_mode(
                &args(&["clippy", "--capture-diagnose", "--emit-test-case"]),
                None
            ),
            Some(CliMode::TestCase)
        );
        assert_eq!(
            cli_mode(&args(&["clippy", "--capture-diagnose"]), None),
            Some(CliMode::Report)
        );
        assert_eq!(
            cli_mode(&args(&["clippy"]), Some("1")),
            Some(CliMode::Report)
        );
    }

    #[test]
    fn the_note_comes_from_the_command_line_and_falls_back_to_a_useful_sentence() {
        assert_eq!(
            cli_note(&args(&["clippy", "--note=双屏混合缩放，覆盖层错位"])).as_deref(),
            Some("双屏混合缩放，覆盖层错位")
        );
        assert_eq!(cli_note(&args(&["clippy", "--note"])), None);
        // fixture 的 note 不允许为空，占位必须是一句有信息量的话
        assert!(!DEFAULT_NOTE.trim().is_empty());
    }
}
