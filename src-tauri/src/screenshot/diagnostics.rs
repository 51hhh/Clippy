//! 截图几何诊断：把这台机器的显示器配置、舞台图尺寸与切分结论摊开给人看。
//!
//! **为什么要有它。** 多屏几何算错时用户看到的是"覆盖层错位""画面溢到隔壁屏"，
//! 而真正的数字（每块屏上报了什么、舞台图多大、切出来的裁剪是什么）全在进程里、
//! 一条日志都不一定开着。没有这份报告，一次报障要来回问五轮还问不准。
//!
//! **报告里没有像素、没有窗口标题。** 舞台图只读 PNG 头取尺寸就把文件删掉（见
//! [`probe_stage_image_size`]），窗口候选整段不进报告——标题会泄露用户在做什么，
//! 和扩展 `GetWindows` 那个令牌是同一套威胁模型（docs/capture-linux.md §2.1）。
//! 所以这份报告可以直接贴进 issue。
//!
//! 附带的 `monitor-layout` 段落**就是** `tests/fixtures/monitor-layouts/` 的格式
//! （共用 `layout_format.rs` 的结构体），存成文件丢进那个目录就是一条回归测试。

use super::backends::{enumerate_xcap_monitors, monitor_union};
use super::geometry_check::desktop_max_scale_factor;
use super::MonitorInfo;
#[cfg(target_os = "linux")]
use {
    super::backends::{enumerate_wayland_monitors, plan_stage_split, probe_stage_image_size},
    super::layout_format::{Fixture, Session},
};

/// 一份几何诊断。所有字段都是排好版的文本：`MonitorInfo` 之类的内部类型不出这个模块。
pub(crate) struct GeometryDiagnostics {
    /// 逐来源的显示器几何。多个来源互相对不上，本身就是最有价值的线索。
    pub sources: Vec<MonitorSourceReport>,
    /// 舞台图与切分结论；拿不到舞台图时是失败原因。
    pub stage: Result<StageDiagnostics, String>,
}

pub(crate) struct MonitorSourceReport {
    pub source: &'static str,
    /// 每块屏一行，或者枚举失败的原因。
    pub lines: Result<Vec<String>, String>,
}

pub(crate) struct StageDiagnostics {
    /// 舞台图是哪条后端给的。
    pub backend: &'static str,
    pub width: u32,
    pub height: u32,
    /// 几何用的是哪个来源（和真实截图链路一致：Wayland 优先、xcap 兜底）。
    pub geometry_source: &'static str,
    /// `StageSplitPlan::summary_line`，和每次截图打进日志的那一行是同一个函数。
    pub summary: String,
    /// 没通过的不变量。空表示 I1–I3 全过。
    pub warnings: Vec<String>,
    /// 可以直接存成 fixture 提 PR 的 json。
    pub fixture_json: String,
}

/// 一块屏一行：`#id x,y WxH ×scale`。
fn format_monitor(monitor: &MonitorInfo) -> String {
    format!(
        "#{} {},{} {}x{} ×{:.4}",
        monitor.id,
        monitor.rect.x,
        monitor.rect.y,
        monitor.rect.width,
        monitor.rect.height,
        monitor.scale_factor,
    )
}

/// 一个来源的全部行：每块屏一行，末尾补一行并集与最大缩放。
///
/// 并集和 max(scale) 单独列出来是有原因的：舞台图尺寸应当正好是
/// `并集 × max(scale)`，报障时把这两个数摆在舞台图旁边，对不上一眼就看出来。
fn describe_monitors(monitors: &[MonitorInfo]) -> Vec<String> {
    let mut lines: Vec<String> = monitors.iter().map(format_monitor).collect();
    match monitor_union(monitors) {
        Ok(union) => lines.push(format!(
            "并集 {}x{}@{},{}，max(scale)={:.4}，预期舞台图 {}x{}",
            union.width,
            union.height,
            union.x,
            union.y,
            desktop_max_scale_factor(monitors),
            (union.width as f32 * desktop_max_scale_factor(monitors)).round() as u32,
            (union.height as f32 * desktop_max_scale_factor(monitors)).round() as u32,
        )),
        Err(error) => lines.push(format!("并集算不出来：{error:#}")),
    }
    lines
}

/// 报告里的一块屏，**逻辑几何**。
///
/// 给 `screenshot` 之外的来源用：`MonitorInfo` / `Rect` 是这个模块的私有类型，不出去，
/// 但"第三方来源"必须能用**同一套算式**排版和求并集——两个来源的并集若各算一遍，
/// 差异就可能来自算法而不是数据，那这份对比就白做了。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReportedMonitor {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

/// 把外部来源的逻辑几何排成和内建来源一模一样的几行（含并集与预期舞台图）。
pub(crate) fn describe_reported_monitors(monitors: &[ReportedMonitor]) -> Vec<String> {
    let monitors: Vec<MonitorInfo> = monitors
        .iter()
        .map(|monitor| MonitorInfo {
            id: monitor.id,
            rect: crate::screenshot::Rect {
                x: monitor.x,
                y: monitor.y,
                width: monitor.width,
                height: monitor.height,
            },
            scale_factor: monitor.scale,
        })
        .collect();
    describe_monitors(&monitors)
}

#[cfg(target_os = "linux")]
fn monitor_sources() -> Vec<MonitorSourceReport> {
    vec![
        MonitorSourceReport {
            source: "wl_output（libwayshot）",
            lines: enumerate_wayland_monitors()
                .map(|monitors| describe_monitors(&monitors))
                .map_err(|error| format!("{error:#}")),
        },
        MonitorSourceReport {
            source: "xcap（XRandR / XWayland）",
            lines: enumerate_xcap_monitors()
                .map(|monitors| describe_monitors(&monitors))
                .map_err(|error| format!("{error:#}")),
        },
    ]
}

#[cfg(not(target_os = "linux"))]
fn monitor_sources() -> Vec<MonitorSourceReport> {
    vec![MonitorSourceReport {
        source: "xcap",
        lines: enumerate_xcap_monitors()
            .map(|monitors| describe_monitors(&monitors))
            .map_err(|error| format!("{error:#}")),
    }]
}

/// 采集几何诊断。
///
/// `name` / `note` / `compositor` 只影响附带的 fixture 段落——只有用户知道自己这台机器
/// 是怎么摆的、症状是什么，这三样代码猜不出来。
#[cfg(target_os = "linux")]
pub(crate) fn collect(name: &str, note: &str, compositor: &str) -> GeometryDiagnostics {
    GeometryDiagnostics {
        sources: monitor_sources(),
        stage: collect_stage(name, note, compositor),
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn collect(_name: &str, _note: &str, _compositor: &str) -> GeometryDiagnostics {
    GeometryDiagnostics {
        sources: monitor_sources(),
        stage: Err("整张舞台图的切分只发生在 Linux 上".to_string()),
    }
}

#[cfg(target_os = "linux")]
fn collect_stage(name: &str, note: &str, compositor: &str) -> Result<StageDiagnostics, String> {
    let (backend, width, height) =
        probe_stage_image_size().map_err(|error| format!("{error:#}"))?;

    // 几何来源的优先级和真实截图链路保持一致，否则诊断出来的计划不是用户遇到的那个。
    let (geometry_source, monitors) = match enumerate_wayland_monitors() {
        Ok(monitors) => ("wl_output", monitors),
        Err(wayland_error) => {
            let monitors = enumerate_xcap_monitors().map_err(|xcap_error| {
                format!("两个几何来源都失败了：wl_output {wayland_error:#}；xcap {xcap_error:#}")
            })?;
            ("xcap", monitors)
        }
    };

    let plan = plan_stage_split(&monitors, width, height)
        .map_err(|error| format!("切分失败：{error:#}"))?;
    let fixture = Fixture::from_plan(
        name.to_string(),
        note.to_string(),
        Session {
            compositor: compositor.to_string(),
            backend: backend.to_string(),
        },
        &monitors,
        width,
        height,
        &plan,
    );
    let fixture_json = serde_json::to_string_pretty(&fixture)
        .map_err(|error| format!("fixture 序列化失败：{error}"))?;

    Ok(StageDiagnostics {
        backend,
        width,
        height,
        geometry_source,
        summary: plan.summary_line(monitors.len(), width, height),
        warnings: plan.warnings.clone(),
        fixture_json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screenshot::Rect;

    fn monitor(id: u32, x: i32, y: i32, width: u32, height: u32, scale: f32) -> MonitorInfo {
        MonitorInfo {
            id,
            rect: Rect {
                x,
                y,
                width,
                height,
            },
            scale_factor: scale,
        }
    }

    /// 报障读的就是这几行，格式跑偏了没人看得懂。
    #[test]
    fn a_monitor_line_carries_the_four_numbers_and_the_scale() {
        assert_eq!(
            format_monitor(&monitor(3, -1920, 408, 1920, 1200, 1.3333334)),
            "#3 -1920,408 1920x1200 ×1.3333"
        );
    }

    /// 并集那行的意义全在"预期舞台图"上：它就是 `并集 × max(scale)`，
    /// 和实测的舞台图尺寸一比，混合缩放算错时当场露馅。
    #[test]
    fn the_union_line_predicts_the_stage_image_size() {
        let monitors = [
            monitor(1, 0, 0, 2560, 1440, 1.5),
            monitor(2, 2560, 408, 1920, 1200, 4.0 / 3.0),
        ];
        let lines = describe_monitors(&monitors);
        let union_line = lines.last().expect("并集那行没了");
        assert!(
            union_line.contains("并集 4480x1608@0,0"),
            "并集不对：{union_line}"
        );
        assert!(
            union_line.contains("max(scale)=1.5000"),
            "最大缩放不对：{union_line}"
        );
        // 4480×1.5 = 6720，1608×1.5 = 2412，正是那次真实事故里的舞台图尺寸。
        assert!(
            union_line.contains("预期舞台图 6720x2412"),
            "预测的舞台图不对：{union_line}"
        );
    }

    /// 外部来源（Tauri/GTK）必须走**同一套算式**排版求并集。各算一遍的话，两个来源的
    /// 差异就可能来自算法而不是数据，而这份并排对比的全部价值就是"数据谁不一样"。
    #[test]
    fn an_external_source_is_formatted_by_the_very_same_arithmetic() {
        let external = [
            ReportedMonitor {
                id: 1,
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
                scale: 1.5,
            },
            ReportedMonitor {
                id: 2,
                x: 2560,
                y: 408,
                width: 1920,
                height: 1200,
                scale: 4.0 / 3.0,
            },
        ];
        let builtin = [
            monitor(1, 0, 0, 2560, 1440, 1.5),
            monitor(2, 2560, 408, 1920, 1200, 4.0 / 3.0),
        ];
        assert_eq!(
            describe_reported_monitors(&external),
            describe_monitors(&builtin)
        );
    }

    /// 一块屏都枚举不到时不能崩，也不能假装算出了并集——报告要说清是哪一步断的。
    #[test]
    fn an_empty_monitor_list_reports_the_failure_instead_of_panicking() {
        let lines = describe_monitors(&[]);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("并集算不出来"), "{:?}", lines[0]);
    }
}
