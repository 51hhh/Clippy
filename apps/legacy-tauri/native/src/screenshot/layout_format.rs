//! `tests/fixtures/monitor-layouts/*.json` 的格式定义。
//!
//! **为什么这些结构体不在测试里。** 诊断工具（`--emit-test-case`）输出的必须**正好**是
//! 回归测试读的那个格式，否则"用户报障 → 补一条测试"之间会多出一步人工翻译，
//! 而那一步就是这条链路断掉的地方。让两边共用同一组结构体，格式漂移在编译期就会暴露。
//!
//! 生成侧见 [`Fixture::from_plan`]，消费侧见 `layout_fixtures.rs`。

#[cfg(test)]
use super::backends::plan_stage_split;
use super::backends::StageSplitPlan;
use super::geometry_check::StageClass;
use super::MonitorInfo;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Fixture {
    /// 文件名之外再写一遍，方便报障时对齐。
    pub name: String,
    /// 这套配置是怎么来的、当时的症状是什么。写给下一个人看。
    pub note: String,
    pub session: Session,
    /// 整张舞台图的像素尺寸。
    pub stage: Size,
    pub monitors: Vec<FixtureMonitor>,
    pub expect: Expect,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Session {
    /// 例如 "GNOME Shell 50.1 (Wayland)"。只用于人读，不参与断言。
    pub compositor: String,
    /// 冻结帧来自哪个后端：gnome-shell-extension / wlroots / portal / xcap。
    pub backend: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FixtureMonitor {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// 显示器自己上报的缩放。混合缩放时它和舞台倍率不是一回事，这正是要钉住的东西。
    pub scale: f32,
}

/// 预期结果。**这一段由 [`Fixture::from_plan`] 从真实管线算出来，不是人手填的**——
/// 手填的期望值只能证明填的人当时怎么想，而这里要钉住的是"这套配置下管线的行为"。
/// 因此收到报障时的正确读法是：先看 `invariants` 是不是空的、`monitors` 是不是用户屏幕
/// 的真实样子，再决定这是一条"钉住正确行为"的 fixture 还是一条"钉住已知症状"的 fixture。
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Expect {
    /// "logical" / "physical" / "unknown"。
    pub stage_class: String,
    /// 预期没通过的不变量标签（"I1" / "I2a" / "I2b" / "I3"），空表示全过。
    /// 已知有问题的配置照样可以进来——把症状钉住比把它挡在门外有用。
    pub invariants: Vec<String>,
    pub monitors: Vec<ExpectMonitor>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
// 其余字段都是单个小写词，加 camelCase 只影响 `mirror_of` → `mirrorOf`，与别处一致。
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ExpectMonitor {
    pub id: u32,
    /// 修正**之后**的逻辑几何。这四个数就是覆盖层开窗和窗口候选换算用的那四个。
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// 按帧算出的缩放。
    pub scale: f32,
    /// 这块屏在舞台图里的位置。
    pub crop: CropRect,
    /// 裁剪和哪块屏完全相同（镜像/投影）。**这一条要单独钉住**：否则"镜像不再报 I2a"
    /// 这个期望，在有人把镜像识别整段删掉之后照样成立。
    /// 非镜像时字段整个不写，老 fixture 不用改。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror_of: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[cfg(test)]
impl FixtureMonitor {
    pub(super) fn to_monitor_info(&self) -> MonitorInfo {
        MonitorInfo {
            id: self.id,
            rect: super::Rect {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
            },
            scale_factor: self.scale,
        }
    }
}

/// 舞台分类的字符串名。fixture 里写的是这三个词，别在别处重新拼。
pub(super) fn stage_class_name(class: StageClass) -> &'static str {
    match class {
        StageClass::Logical { .. } => "logical",
        StageClass::Physical => "physical",
        StageClass::Unknown { .. } => "unknown",
    }
}

/// 告警文本的不变量标签（第一个空白分隔的词）。fixture 的 `invariants` 就是这个列表。
pub(super) fn invariant_tags(warnings: &[String]) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| {
            warning
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

impl Expect {
    /// 从真实管线的结论生成期望段落。**测试的断言和诊断的输出共用这一个函数**，
    /// 所以"诊断给出的 fixture 一定能被测试接受"是结构上成立的，不靠人去核对。
    pub(super) fn from_plan(plan: &StageSplitPlan) -> Self {
        Self {
            stage_class: stage_class_name(plan.stage).to_string(),
            invariants: invariant_tags(&plan.warnings),
            monitors: plan
                .tiles
                .iter()
                .map(|tile| ExpectMonitor {
                    id: tile.monitor.id,
                    x: tile.monitor.rect.x,
                    y: tile.monitor.rect.y,
                    width: tile.monitor.rect.width,
                    height: tile.monitor.rect.height,
                    scale: tile.monitor.scale_factor,
                    crop: CropRect {
                        x: tile.crop.x,
                        y: tile.crop.y,
                        width: tile.crop.width,
                        height: tile.crop.height,
                    },
                    mirror_of: tile.mirror_of,
                })
                .collect(),
        }
    }
}

impl Fixture {
    /// 把"当前这台机器的显示器配置"整理成一条可以直接提 PR 的 fixture。
    ///
    /// `name` 与 `note` 由调用方给，因为只有用户知道这台机器是怎么摆的、症状是什么；
    /// 其余全部来自实测与真实管线。
    pub(super) fn from_plan(
        name: String,
        note: String,
        session: Session,
        monitors: &[MonitorInfo],
        image_width: u32,
        image_height: u32,
        plan: &StageSplitPlan,
    ) -> Self {
        Self {
            name,
            note,
            session,
            stage: Size {
                width: image_width,
                height: image_height,
            },
            monitors: monitors
                .iter()
                .map(|monitor| FixtureMonitor {
                    id: monitor.id,
                    x: monitor.rect.x,
                    y: monitor.rect.y,
                    width: monitor.rect.width,
                    height: monitor.rect.height,
                    scale: monitor.scale_factor,
                })
                .collect(),
            expect: Expect::from_plan(plan),
        }
    }

    /// 直接从显示器列表与舞台图尺寸算一条 fixture（跑一遍真实管线）。
    #[cfg(test)]
    pub(super) fn capture(
        name: String,
        note: String,
        session: Session,
        monitors: &[MonitorInfo],
        image_width: u32,
        image_height: u32,
    ) -> anyhow::Result<Self> {
        let plan = plan_stage_split(monitors, image_width, image_height)?;
        Ok(Self::from_plan(
            name,
            note,
            session,
            monitors,
            image_width,
            image_height,
            &plan,
        ))
    }
}
