//! 把"用户的显示器配置"变成数据：`tests/fixtures/monitor-layouts/*.json` 一个文件一种环境。
//!
//! **为什么是这个形状。** 多屏几何的组合打不完（合成器 × 缩放模式 × 屏数 × 每屏缩放 ×
//! 排布 × 旋转 × 截图后端），一人一台机器逐个手测必然漏。但每种配置需要的信息只有几个整数，
//! 于是"收到一份报障"和"补一条回归测试"可以是**同一件事**：用户把诊断报告里的
//! `monitor-layout` 段落存成一个 json 丢进那个目录，PR 就完整了，不需要写一行 Rust。
//!
//! 因此这里刻意不长成一堆手写 `#[test]`：新增环境的成本必须是一个文件，否则没人会补。
//!
//! 驱动的是**真正在跑的** [`plan_stage_split`]（它一个像素都不碰，所以不用凑几十兆的假图），
//! 不是测试里另抄一份算式——抄的那份只能证明抄对了。格式定义在 `layout_format.rs`，
//! 和诊断工具的输出侧共用。

use super::backends::plan_stage_split;
use super::layout_format::{stage_class_name, Expect, Fixture};
use super::{MonitorInfo, Rect};
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/monitor-layouts")
}

fn load_fixtures() -> Vec<(String, Fixture)> {
    let dir = fixture_dir();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("读不到 fixture 目录 {}: {error}", dir.display()))
        .map(|entry| entry.expect("读取 fixture 目录项失败").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    entries.sort();
    entries
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("读不到 {}: {error}", path.display()));
            let fixture: Fixture = serde_json::from_str(&text)
                .unwrap_or_else(|error| panic!("{} 解析失败: {error}", path.display()));
            let file = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("?")
                .to_string();
            (file, fixture)
        })
        .collect()
}

/// 断言里所有缩放比较都用这个容差：舞台尺寸是取整后的整数，反推出来的比值带半像素噪声。
const SCALE_EPSILON: f32 = 1e-3;

#[test]
fn every_monitor_layout_fixture_matches_the_pipeline() {
    let fixtures = load_fixtures();
    assert!(
        !fixtures.is_empty(),
        "fixture 目录是空的——这条测试会变成永远通过的空壳"
    );

    for (file, fixture) in fixtures {
        // 报障里带的是文件名，所以每条断言都要能指回它。
        let at = format!("{file}（{}，{}）", fixture.session.compositor, fixture.name);
        let monitors: Vec<MonitorInfo> = fixture
            .monitors
            .iter()
            .map(|monitor| monitor.to_monitor_info())
            .collect();

        let plan = plan_stage_split(&monitors, fixture.stage.width, fixture.stage.height)
            .unwrap_or_else(|error| panic!("{at}: 切分失败 {error}"));

        // 舞台分类是所有后续换算的前提，先钉它。
        assert_eq!(
            stage_class_name(plan.stage),
            fixture.expect.stage_class,
            "{at}: 舞台分类不符（实测 {:?}）",
            plan.stage
        );

        // 不变量：预期失败的那几条必须失败，没预期的一条都不许冒出来。
        let tags = super::layout_format::invariant_tags(&plan.warnings);
        assert_eq!(
            tags, fixture.expect.invariants,
            "{at}: 不变量结果不符，实测告警：{:#?}",
            plan.warnings
        );

        assert_eq!(
            plan.tiles.len(),
            fixture.expect.monitors.len(),
            "{at}: 屏数不符"
        );
        for (tile, expected) in plan.tiles.iter().zip(&fixture.expect.monitors) {
            let where_ = format!("{at} 显示器 {}", expected.id);
            assert_eq!(tile.monitor.id, expected.id, "{where_}: id 顺序不符");
            assert_eq!(
                tile.monitor.rect,
                Rect {
                    x: expected.x,
                    y: expected.y,
                    width: expected.width,
                    height: expected.height,
                },
                "{where_}: 修正后的逻辑几何不符"
            );
            assert!(
                (tile.monitor.scale_factor - expected.scale).abs() <= SCALE_EPSILON,
                "{where_}: 帧缩放不符，实测 {}",
                tile.monitor.scale_factor
            );
            assert_eq!(
                (tile.crop.x, tile.crop.y, tile.crop.width, tile.crop.height),
                (
                    expected.crop.x,
                    expected.crop.y,
                    expected.crop.width,
                    expected.crop.height
                ),
                "{where_}: 裁剪矩形不符"
            );
        }

        // 摘要行是报障的起点，每种布局都要能格式化出来，并且**每块屏都得出现在里面**——
        // 少一块屏的摘要会把人引向错误的方向。
        let summary = plan.summary_line(monitors.len(), fixture.stage.width, fixture.stage.height);
        for expected in &fixture.expect.monitors {
            assert!(
                summary.contains(&format!("#{}@", expected.id)),
                "{at}: 摘要行少了显示器 {}：{summary}",
                expected.id
            );
        }

        // 后端字段目前只给人读；写错一个词比留空更容易误导，所以至少要求它非空。
        assert!(
            !fixture.session.backend.is_empty(),
            "{at}: backend 不能为空"
        );
        assert!(!fixture.note.is_empty(), "{at}: note 不能为空——写清症状");
    }
}

/// 诊断工具 `--emit-test-case` 吐出来的东西必须能被上面那条测试**原样**接受。
///
/// 这是"报障即回归测试"这条路的关键一环：如果生成侧和断言侧对期望值的理解差一点，
/// 用户提上来的 fixture 会当场变红，而红的原因跟他的环境毫无关系。所以拿每个现成
/// fixture 的输入重跑一遍生成，再和文件里的 `expect` 逐字段比。
#[test]
fn the_diagnostic_emits_exactly_what_the_fixture_test_asserts() {
    for (file, fixture) in load_fixtures() {
        let monitors: Vec<MonitorInfo> = fixture
            .monitors
            .iter()
            .map(|monitor| monitor.to_monitor_info())
            .collect();
        let emitted = Fixture::capture(
            fixture.name.clone(),
            fixture.note.clone(),
            super::layout_format::Session {
                compositor: fixture.session.compositor.clone(),
                backend: fixture.session.backend.clone(),
            },
            &monitors,
            fixture.stage.width,
            fixture.stage.height,
        )
        .unwrap_or_else(|error| panic!("{file}: 生成 fixture 失败 {error}"));

        // 先过一遍 json 序列化再比：字段名写错、驼峰漏了，都得在这里暴露。
        let text = serde_json::to_string_pretty(&emitted).expect("fixture 序列化失败");
        let round_tripped: Fixture =
            serde_json::from_str(&text).unwrap_or_else(|error| panic!("{file}: 回读失败 {error}"));

        assert_eq!(
            round_tripped.expect, fixture.expect,
            "{file}: 诊断生成的期望段落和文件里的不一致——报障提上来的 fixture 会当场变红"
        );
        assert_eq!(round_tripped.stage.width, fixture.stage.width, "{file}");
        assert_eq!(round_tripped.stage.height, fixture.stage.height, "{file}");
        assert_eq!(
            round_tripped.monitors.len(),
            fixture.monitors.len(),
            "{file}"
        );
    }
}

/// `Expect` 的 `PartialEq` 是逐位比 f32 的，所以生成侧必须原样搬运缩放值而不是
/// 自己再算一遍——这条测试钉住"同一个 plan 生成两次结果相同"。
#[test]
fn expectations_generated_from_the_same_plan_are_identical() {
    let monitors = vec![
        MonitorInfo {
            id: 1,
            rect: Rect {
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
            },
            scale_factor: 1.5,
        },
        MonitorInfo {
            id: 2,
            rect: Rect {
                x: 2560,
                y: 408,
                width: 1920,
                height: 1200,
            },
            scale_factor: 4.0 / 3.0,
        },
    ];
    let plan = plan_stage_split(&monitors, 6720, 2412).expect("切分失败");
    assert_eq!(Expect::from_plan(&plan), Expect::from_plan(&plan));
}
