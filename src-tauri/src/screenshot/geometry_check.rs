//! 截图几何的不变量自检。
//!
//! **为什么需要这一层。** 多屏几何的组合是打不完的（合成器 × 缩放模式 × 屏数 ×
//! 每屏缩放 × 排布 × 旋转 × 截图后端），逐个环境去测必然漏。但这些数据本身是**过定义**的：
//! 同一套显示器几何既决定了逻辑并集，也决定了舞台图应有的尺寸，还决定了每块屏该切多大。
//! 任意两处对不上就说明有一处算错了，**而这件事不需要知道用户在什么环境**。
//!
//! 所以这里全是纯函数：输入几个数，输出"哪条不变量不成立、差多少"。真机上把它们记进日志，
//! 测试里用 `tests/fixtures/monitor-layouts/*.json` 里的真实配置逐行跑。
//!
//! 历史教训见 [`classify_stage`]：舞台图的倍率和"某一块屏自己的缩放"不是一回事，
//! 混用过一次，代价是插上第二块屏后覆盖层整体错位。

#[cfg(any(test, target_os = "linux"))]
use super::ImageRect;
use super::MonitorInfo;

/// 比值判等的容差。舞台图尺寸是取整后的整数，`round()` 最多带来半像素误差，
/// 换成比值后在 4K 量级上是 1e-4 的数量级，1e-3 足够松也足够严。
#[cfg(any(test, target_os = "linux"))]
const RATIO_EPSILON: f32 = 1e-3;

/// 整张舞台图和显示器几何处于哪个坐标空间的关系。
///
/// 判定依据只有一个：**舞台图 ÷ 逻辑并集**这个比值。
#[cfg(any(test, target_os = "linux"))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum StageClass {
    /// 几何是逻辑像素，舞台图 = 逻辑并集 × 各视图里最大的缩放。
    ///
    /// 这是 Mutter 抓整屏的正常形态（`clutter_stage_get_capture_final_size`）。
    /// 几何可信，**不要修正**；低缩放的屏在图里是被放大过的，那是舞台的性质，不是错误。
    ///
    /// **也不要把它缩回原生分辨率。** 量过：2880x1800 → 2560x1600 的重采样要 200 ms
    /// 以上，全加在"用户等覆盖层出现"的时间上，换来的只是一次进程内传输少几 MB；
    /// 画质上也没好处（浏览器按 devicePixelRatio 缩那一步是 GPU 采样，先在 CPU 缩一遍
    /// 反而多一次重采样）。数字见 docs/bench-baseline.md。
    Logical { stage_scale: f32 },
    /// 几何带着物理味：并集和舞台图一样大，但显示器自称有缩放。
    ///
    /// xcap 在 XWayland 上就是这样（把 RandR 尺寸除以自己探测的缩放当"逻辑尺寸"，
    /// 实测 1920x1200 被报成 2880x1800）。这时候要按**每块屏自己的**缩放反推逻辑尺寸。
    Physical,
    /// 两条都不像。多半是枚举漏了显示器、几何是热插拔前的陈数据，或者哪一步单位错了。
    Unknown { stage_scale: f32, max_scale: f32 },
}

/// 整台桌面的最大缩放系数，也就是"舞台图 ÷ 逻辑并集"应有的那个倍率。
///
/// Mutter 抓整屏时把舞台按**各视图里最大的**缩放渲染成一张图，所以这个值既是舞台图的
/// 放大倍率，也是切图时唯一可用的"像素 → 逻辑"除数。
/// 拿不到任何可信缩放时退回 1.0（等于不做修正），绝不让截图整体失败。
pub(super) fn desktop_max_scale_factor(monitors: &[MonitorInfo]) -> f32 {
    monitors
        .iter()
        .map(|monitor| monitor.scale_factor)
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .fold(1.0f32, f32::max)
}

/// **不变量 I1：舞台图尺寸 = 逻辑并集 × max(缩放)。**
///
/// 这一条同时区分了"几何可信"与"几何是物理味"，因此切图前必须先问它走哪个分支，
/// 而不是像以前那样无条件调用修正函数、靠"差值 ≤ 1 像素就提前返回"这个护栏碰运气——
/// 混合缩放的多屏上护栏正好被绕过，非最大缩放的那块屏被改写成 1.125 倍。
#[cfg(any(test, target_os = "linux"))]
pub(super) fn classify_stage(
    monitors: &[MonitorInfo],
    union_width: u32,
    union_height: u32,
    image_width: u32,
    image_height: u32,
) -> StageClass {
    let max_scale = desktop_max_scale_factor(monitors);
    let stage_scale = if union_width == 0 {
        0.0
    } else {
        image_width as f32 / union_width as f32
    };
    let stage_scale_y = if union_height == 0 {
        0.0
    } else {
        image_height as f32 / union_height as f32
    };

    // x/y 两个方向的倍率必须一致：不一致说明并集或图有一个方向算错了，
    // 这种情况下任何"修正"都是在错上加错。
    if !ratios_match(stage_scale, stage_scale_y) {
        return StageClass::Unknown {
            stage_scale,
            max_scale,
        };
    }
    if ratios_match(stage_scale, max_scale) {
        return StageClass::Logical { stage_scale };
    }
    // 并集就是图本身，而显示器自称有缩放 → 几何用的是物理像素。
    if ratios_match(stage_scale, 1.0) && max_scale > 1.0 + RATIO_EPSILON {
        return StageClass::Physical;
    }
    StageClass::Unknown {
        stage_scale,
        max_scale,
    }
}

/// **不变量 I2a：任意两块屏的裁剪矩形不得重叠。**
///
/// 抓得到的问题：枚举到了已经拔掉的屏、热插拔后的陈几何——也就是**部分**重叠这一类，
/// 没有任何正常配置能解释它。
///
/// **镜像屏不算。** 投影时两块屏共用同一个矩形，裁剪完全相同，那是配置本身如此；
/// 调用方先用 [`find_mirror_sources`] 把它们摘出去再进来（以前不摘，代价是
/// "一接投影仪就报几何错"）。
///
/// **注意这里不检查"铺满"。** 显示器并集经常不是矩形（本机就是：2560x1440 的外接屏
/// 配一块下移 408 像素的 1920x1200 笔记本屏），舞台图是并集的**外接矩形**，那些空出来的
/// 区域根本没有显示器对应。"面积之和等于图面积"只在恰好平铺时成立，拿它当不变量会在
/// 完全正常的布局上天天误报——覆盖率是诊断报告里的一个数字，不是判据。
#[cfg(any(test, target_os = "linux"))]
pub(super) fn verify_crops_do_not_overlap(crops: &[ImageRect]) -> Result<(), String> {
    for (index, crop) in crops.iter().enumerate() {
        for other in crops.iter().skip(index + 1) {
            let overlap = image_rect_overlap(*crop, *other);
            if overlap > 0 {
                return Err(format!(
                    "裁剪矩形重叠 {overlap} 像素：{crop:?} 与 {other:?}；\
                     已拔掉的屏或者热插拔前的陈几何？"
                ));
            }
        }
    }
    Ok(())
}

/// 找出**镜像**：裁剪矩形和前面某一块屏**完全相同**的屏，返回各自对应的下标。
///
/// **完全相同和部分重叠是两件不同的事，必须分开。** 投影时两块屏共用同一个逻辑矩形，
/// 于是两次裁剪从舞台图的同一处取像素——这是配置本身如此，不是几何算错；而**部分**重叠
/// 没有任何正常配置能解释，它意味着枚举到了已拔掉的屏或者陈几何。以前两者都走 I2a，
/// 结果是"投影一接就报错"，而报错内容对真正的陈几何毫无指向性。
///
/// 返回 `(镜像屏下标, 源屏下标)`，按镜像屏下标升序。源屏只取**第一个**相同的，
/// 所以三屏镜像会得到 `[(1,0),(2,0)]` 而不是链式的 `(2,1)`。
#[cfg(any(test, target_os = "linux"))]
pub(super) fn find_mirror_sources(crops: &[ImageRect]) -> Vec<(usize, usize)> {
    let mut mirrors = Vec::new();
    for (index, crop) in crops.iter().enumerate() {
        if let Some(source) = crops[..index].iter().position(|earlier| earlier == crop) {
            mirrors.push((index, source));
        }
    }
    mirrors
}

/// **不变量 I2b：裁剪尺寸必须等于"逻辑尺寸 × 舞台倍率"。**
///
/// `scaled_monitor_rect` 会把边界钳进图内，所以"几何声称的显示器伸到舞台图外面"这件事
/// 不会报错，只会**静默切出一块偏小的图**——覆盖层于是拿到一张不完整的底图。
/// 常见成因：枚举到了已经拔掉的屏、几何是分辨率切换前的陈数据。
///
/// 返回两个方向上偏差的最大像素数，0 表示通过。
#[cfg(any(test, target_os = "linux"))]
pub(super) fn verify_crop_not_clamped(
    rect_width: u32,
    rect_height: u32,
    crop: ImageRect,
    scale_x: f32,
    scale_y: f32,
) -> f32 {
    if !scale_x.is_finite() || !scale_y.is_finite() {
        return 0.0;
    }
    let expected_width = rect_width as f32 * scale_x;
    let expected_height = rect_height as f32 * scale_y;
    // 两条边各自取整，所以最坏情况下宽高各差 1 像素，这是噪声不是错误。
    let dx = (crop.width as f32 - expected_width).abs();
    let dy = (crop.height as f32 - expected_height).abs();
    let worst = dx.max(dy);
    if worst <= 1.0 {
        0.0
    } else {
        worst
    }
}

/// 裁剪覆盖了舞台图的多大比例。**这是诊断报告里的一个数字，不是判据**（见
/// [`verify_crops_do_not_overlap`] 里为什么）。前提是裁剪互不重叠。
#[cfg(any(test, target_os = "linux"))]
pub(super) fn crop_coverage_ratio(crops: &[ImageRect], image_width: u32, image_height: u32) -> f32 {
    let image_area = image_width as u64 * image_height as u64;
    if image_area == 0 {
        return 0.0;
    }
    let covered: u64 = crops
        .iter()
        .map(|crop| crop.width as u64 * crop.height as u64)
        .sum();
    covered as f32 / image_area as f32
}

/// 由"物理尺寸 ÷ 逻辑尺寸"求这块屏的缩放，**先按旋转对齐坐标轴**。
///
/// **`axes_swapped` 不是可选的精细化，少了它算出来的是废数。** Wayland 的
/// `physical_size` 是面板自己的原始分辨率（不含旋转），而 `logical_region` 已经是
/// 旋转后的桌面坐标。竖着摆的 1920x1080 面板于是报成 physical 1920x1080 +
/// logical 1080x1920：不换轴直接除，宽比 1.7778、高比 0.5625，取谁都不对——
/// 而这个值会一路传成帧缩放和坐标换算的除数。换轴之后两个方向都是 1.0。
///
/// 拿不到可信比值（逻辑边长为 0、比值非有限或非正）时返回 `None`，由调用方决定退路，
/// 而不是在这里悄悄兜一个 1.0。
#[cfg(any(test, target_os = "linux"))]
pub(super) fn output_scale_from_sizes(
    physical: (u32, u32),
    logical: (u32, u32),
    axes_swapped: bool,
) -> Option<f32> {
    let (physical_width, physical_height) = if axes_swapped {
        (physical.1, physical.0)
    } else {
        physical
    };
    let (logical_width, logical_height) = logical;
    if logical_width == 0 || logical_height == 0 {
        return None;
    }
    // 分数缩放下两个方向会因为取整差一点，取较大的那个和历史行为一致；
    // 换轴之后它们本来就该几乎相等，这里只是兜取整误差，不再兜"轴错了"。
    let scale = (physical_width as f32 / logical_width as f32)
        .max(physical_height as f32 / logical_height as f32);
    (scale.is_finite() && scale > 0.0).then_some(scale)
}

/// **不变量 I3：每块屏的帧/逻辑比值在两个方向上一致。**
///
/// 抓得到的问题：x/y 缩放不一致；几何是热插拔前的陈数据。
///
/// **旋转屏不该在这里报错。** 舞台图是合成器合出来的桌面，旋转已经烤进去了，
/// 旋转屏的逻辑矩形本身就是旋转后的（1080x1920），裁剪也是，两个方向的比值一致。
/// 真正需要处理旋转的地方是 [`output_scale_from_sizes`]。
#[cfg(any(test, target_os = "linux"))]
pub(super) fn verify_frame_isotropy(rect_width: u32, rect_height: u32, crop: ImageRect) -> f32 {
    if rect_width == 0 || rect_height == 0 {
        return 0.0;
    }
    let x = crop.width as f32 / rect_width as f32;
    let y = crop.height as f32 / rect_height as f32;
    if ratios_match(x, y) {
        0.0
    } else {
        (x - y).abs()
    }
}

#[cfg(any(test, target_os = "linux"))]
fn image_rect_overlap(a: ImageRect, b: ImageRect) -> u64 {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    if right <= left || bottom <= top {
        return 0;
    }
    (right - left) as u64 * (bottom - top) as u64
}

/// 两个比值是否可以认为相等。除数为 0 或非有限一律判否，免得 NaN 顺着往下传。
#[cfg(any(test, target_os = "linux"))]
fn ratios_match(a: f32, b: f32) -> bool {
    if !a.is_finite() || !b.is_finite() || a <= 0.0 || b <= 0.0 {
        return false;
    }
    (a - b).abs() <= RATIO_EPSILON * a.max(b)
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

    fn crop(x: u32, y: u32, width: u32, height: u32) -> ImageRect {
        ImageRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn mixed_scale_stage_is_classified_as_logical_at_the_max_scale() {
        // 真机实测：HDMI 2560x1440@1.5 + 笔记本 1920x1200@1.3333，舞台图 6720x2412。
        let monitors = vec![
            monitor(1, 0, 0, 2560, 1440, 1.5),
            monitor(2, 2560, 408, 1920, 1200, 1.333_333_4),
        ];
        assert_eq!(
            classify_stage(&monitors, 4480, 1608, 6720, 2412),
            StageClass::Logical { stage_scale: 1.5 }
        );
    }

    #[test]
    fn bogus_xwayland_geometry_is_classified_as_physical() {
        // xcap 把 1920x1200 报成 2880x1800，而舞台图就是那么大 → 倍率 1.0。
        let monitors = vec![monitor(1, 0, 0, 2880, 1800, 1.5)];
        assert_eq!(
            classify_stage(&monitors, 2880, 1800, 2880, 1800),
            StageClass::Physical
        );
    }

    #[test]
    fn unscaled_single_monitor_is_logical_not_physical() {
        // 没有缩放时两种形态数值上无法区分，必须落在"可信"那一侧（不修正）。
        let monitors = vec![monitor(1, 0, 0, 1920, 1080, 1.0)];
        assert_eq!(
            classify_stage(&monitors, 1920, 1080, 1920, 1080),
            StageClass::Logical { stage_scale: 1.0 }
        );
    }

    #[test]
    fn a_missing_monitor_makes_the_stage_unclassifiable() {
        // 枚举漏了一块屏：并集偏小，倍率既不等于 max(scale) 也不等于 1。
        let monitors = vec![monitor(1, 0, 0, 2560, 1440, 2.0)];
        assert!(matches!(
            classify_stage(&monitors, 2560, 1440, 6720, 2412),
            StageClass::Unknown { .. }
        ));
    }

    #[test]
    fn anisotropic_stage_is_unclassifiable() {
        let monitors = vec![monitor(1, 0, 0, 1000, 1000, 2.0)];
        assert!(matches!(
            classify_stage(&monitors, 1000, 1000, 2000, 1000),
            StageClass::Unknown { .. }
        ));
    }

    #[test]
    fn max_scale_ignores_bogus_values_and_defaults_to_one() {
        // NaN / 0 得被跳过而不是污染 max，空列表退回 1.0（等于不做修正）。
        let mut monitors = vec![
            monitor(1, 0, 0, 100, 50, 1.5),
            monitor(2, 100, 0, 100, 50, f32::NAN),
        ];
        assert!((desktop_max_scale_factor(&monitors) - 1.5).abs() < 1e-6);
        monitors[0].scale_factor = 0.0;
        assert!((desktop_max_scale_factor(&monitors) - 1.0).abs() < 1e-6);
        assert!((desktop_max_scale_factor(&[]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn side_by_side_crops_do_not_overlap() {
        assert!(verify_crops_do_not_overlap(&[crop(0, 0, 100, 50), crop(100, 0, 100, 50)]).is_ok());
    }

    /// I2a 留给**部分**重叠：没有任何正常配置能解释它（已拔掉的屏、陈几何）。
    #[test]
    fn a_partial_overlap_is_still_reported() {
        let error =
            verify_crops_do_not_overlap(&[crop(0, 0, 100, 50), crop(50, 0, 100, 50)]).unwrap_err();
        assert!(error.contains("重叠"), "{error}");
        // 措辞里不能再把镜像当首要嫌疑：镜像已经在上一步被摘走了。
        assert!(!error.contains("镜像"), "{error}");
    }

    /// **完全相同的裁剪是镜像（投影），不是错误。** 以前它走 I2a，等于一接投影仪就报
    /// 几何错；现在先摘成镜像，切图时还能共享源屏那份缓冲。
    #[test]
    fn identical_crops_are_mirrors_not_overlaps() {
        let crops = [crop(0, 0, 100, 50), crop(0, 0, 100, 50)];
        assert_eq!(find_mirror_sources(&crops), vec![(1, 0)]);
        // 摘掉镜像之后剩下的那一份当然不重叠。
        assert!(verify_crops_do_not_overlap(&crops[..1]).is_ok());
    }

    /// 三屏镜像：都指向第一块，不要串成 2→1→0 那样的链，否则共享缓冲得递归找源头。
    #[test]
    fn every_mirror_points_at_the_first_one() {
        let crops = [
            crop(0, 0, 100, 50),
            crop(0, 0, 100, 50),
            crop(0, 0, 100, 50),
        ];
        assert_eq!(find_mirror_sources(&crops), vec![(1, 0), (2, 0)]);
    }

    /// 只差一个像素就不是镜像了——那属于"几何有点不对"，该让 I2a 去说。
    #[test]
    fn a_nearly_identical_crop_is_not_a_mirror() {
        let crops = [crop(0, 0, 100, 50), crop(0, 0, 100, 51)];
        assert!(find_mirror_sources(&crops).is_empty());
        assert!(verify_crops_do_not_overlap(&crops).is_err());
    }

    #[test]
    fn an_uncovered_stage_region_is_not_an_error() {
        // 高度不同的两块屏并排：并集不是矩形，舞台图右下角那块空白没有任何显示器对应。
        // 这是完全正常的布局（本机就是这个形状），绝不能报错。
        let crops = [crop(0, 0, 100, 50), crop(100, 0, 100, 30)];
        assert!(verify_crops_do_not_overlap(&crops).is_ok());
        assert!((crop_coverage_ratio(&crops, 200, 50) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn a_crop_clamped_by_the_image_edge_is_reported() {
        // 几何声称这块屏 1000 宽、舞台倍率 2，但图里只切到 1500——少了 500 像素，
        // 底图会缺一条。`scaled_monitor_rect` 自己是静默钳边的，只有这条抓得到。
        assert_eq!(
            verify_crop_not_clamped(1000, 500, crop(0, 0, 1500, 1000), 2.0, 2.0),
            500.0
        );
        assert_eq!(
            verify_crop_not_clamped(1000, 500, crop(0, 0, 2000, 1000), 2.0, 2.0),
            0.0
        );
        // 取整噪声（1 像素以内）不算。
        assert_eq!(
            verify_crop_not_clamped(1001, 500, crop(0, 0, 1501, 750), 1.5, 1.5),
            0.0
        );
    }

    /// 竖着摆的屏：不换轴算出来的是 1.7778，一路传下去当"缩放"用，帧尺寸和坐标全错。
    #[test]
    fn a_rotated_output_scale_needs_its_axes_swapped() {
        // 1920x1080 面板转 90 度、无缩放：physical 还是 1920x1080，logical 是 1080x1920。
        assert_eq!(
            output_scale_from_sizes((1920, 1080), (1080, 1920), true),
            Some(1.0)
        );
        // 同一组数不换轴：得到一个纯属虚构的 1.7778。这就是这个参数存在的理由。
        let wrong = output_scale_from_sizes((1920, 1080), (1080, 1920), false).unwrap();
        assert!((wrong - 1.7777778).abs() < 1e-4, "{wrong}");
    }

    /// 竖屏 + 2 倍缩放：3840x2160 的面板转 90 度后逻辑是 1080x1920。
    #[test]
    fn a_rotated_output_keeps_its_scale() {
        assert_eq!(
            output_scale_from_sizes((3840, 2160), (1080, 1920), true),
            Some(2.0)
        );
    }

    #[test]
    fn an_unrotated_output_is_unaffected() {
        assert_eq!(
            output_scale_from_sizes((2560, 1600), (1920, 1200), false),
            Some(1.3333334)
        );
        assert_eq!(
            output_scale_from_sizes((1920, 1080), (1920, 1080), false),
            Some(1.0)
        );
    }

    /// 逻辑边长为 0 时不能兜一个 1.0 了事——那等于把"不知道"写成"没缩放"。
    #[test]
    fn a_degenerate_output_has_no_scale_at_all() {
        assert_eq!(
            output_scale_from_sizes((1920, 1080), (0, 1080), false),
            None
        );
        assert_eq!(output_scale_from_sizes((0, 0), (1920, 1080), false), None);
    }

    /// I3 抓的是"裁剪的朝向和逻辑矩形对不上"，**不是**"这块屏被旋转了"。
    /// 正常的竖屏两边都是竖的，比值一致，这条不该响；见 `output_scale_from_sizes`。
    #[test]
    fn isotropy_flags_a_crop_whose_orientation_disagrees_with_the_geometry() {
        // 逻辑 1200x1920（竖屏）却按横屏切出 2880x1800：两个方向比值差很远。
        assert!(verify_frame_isotropy(1200, 1920, crop(0, 0, 2880, 1800)) > 0.5);
        // 真正的竖屏：逻辑 1080x1920，舞台倍率 1.5 → 裁剪 1620x2880，比值一致，不报。
        assert_eq!(
            verify_frame_isotropy(1080, 1920, crop(0, 0, 1620, 2880)),
            0.0
        );
        assert_eq!(
            verify_frame_isotropy(1920, 1200, crop(0, 0, 2880, 1800)),
            0.0
        );
    }
}
