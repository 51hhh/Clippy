//! 托盘图标按主题动态点阵：源 SVG 改色 + resvg 渲染 + 圆角矩形背景。
//! 设计文档：docs/superpowers/specs/2026-04-25-tray-icon-themed-design.md

use tauri::image::Image;

const SVG_SOURCE: &str = include_str!("../icons/file.svg");

/// 输出尺寸（托盘 64×64 足够清晰且不占内存）
const SIZE: u32 = 64;
/// 圆角矩形背景半径
const BG_RADIUS: f32 = 14.0;
/// SVG 描边渲染区域内边距（像素）：64 × 12.5% = 8
const SVG_INSET: f32 = 8.0;

/// 主题 → bg-primary RGBA（与 src/styles/themes.css 对齐）
const THEME_BG: &[(&str, [u8; 4])] = &[
    ("light", [0xfb, 0xfb, 0xfd, 0xff]),
    ("dark", [0x16, 0x18, 0x1d, 0xff]),
    ("nord", [0x2e, 0x34, 0x40, 0xff]),
    ("solarized-light", [0xfd, 0xf6, 0xe3, 0xff]),
    ("rose", [0xff, 0xf7, 0xf7, 0xff]),
    ("midnight", [0x14, 0x13, 0x2b, 0xff]),
];

fn bg_for(theme: &str) -> [u8; 4] {
    THEME_BG
        .iter()
        .find(|(name, _)| *name == theme)
        .map(|(_, c)| *c)
        .unwrap_or([0xfb, 0xfb, 0xfd, 0xff])
}

/// 基于 sRGB 相对亮度（WCAG）选取与背景对比的描边色。
fn stroke_hex_for(bg: [u8; 4]) -> &'static str {
    fn channel(c: u8) -> f32 {
        let v = c as f32 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    let l = 0.2126 * channel(bg[0]) + 0.7152 * channel(bg[1]) + 0.0722 * channel(bg[2]);
    if l > 0.5 {
        "#000000"
    } else {
        "#ffffff"
    }
}

/// 把源 SVG 中的黑色描边替换为指定 hex（如 "#ffffff"）。
fn recolor_svg(stroke: &str) -> String {
    SVG_SOURCE.replace("stroke=\"#000000\"", &format!("stroke=\"{}\"", stroke))
}

/// 生成对应主题的托盘图标；失败时返回 None，调用方自行回退。
pub fn render_themed_tray_icon(theme: &str) -> Option<Image<'static>> {
    use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

    let bg = bg_for(theme);
    let stroke = stroke_hex_for(bg);

    let mut pixmap = Pixmap::new(SIZE, SIZE)?;

    // 1. 圆角矩形背景
    let bg_path = {
        let rect = Rect::from_xywh(0.0, 0.0, SIZE as f32, SIZE as f32)?;
        let mut pb = PathBuilder::new();
        pb.push_rect(rect);
        // tiny-skia 没有原生圆角矩形，自己拼一条
        let r = BG_RADIUS;
        let w = SIZE as f32;
        let h = SIZE as f32;
        let mut pb = PathBuilder::new();
        pb.move_to(r, 0.0);
        pb.line_to(w - r, 0.0);
        pb.quad_to(w, 0.0, w, r);
        pb.line_to(w, h - r);
        pb.quad_to(w, h, w - r, h);
        pb.line_to(r, h);
        pb.quad_to(0.0, h, 0.0, h - r);
        pb.line_to(0.0, r);
        pb.quad_to(0.0, 0.0, r, 0.0);
        pb.close();
        pb.finish()?
    };
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(bg[0], bg[1], bg[2], bg[3]));
    paint.anti_alias = true;
    pixmap.fill_path(
        &bg_path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );

    // 2. SVG 渲染到内层（留 SVG_INSET 像素边距）
    let svg_text = recolor_svg(stroke);
    let opt = usvg::Options::default();
    let tree = match usvg::Tree::from_str(&svg_text, &opt) {
        Ok(t) => t,
        Err(e) => {
            log::error!("托盘 SVG 解析失败: {}", e);
            return None;
        }
    };
    let inner = SIZE as f32 - SVG_INSET * 2.0;
    let svg_size = tree.size();
    let scale = inner / svg_size.width().max(svg_size.height());
    let tx = (SIZE as f32 - svg_size.width() * scale) / 2.0;
    let ty = (SIZE as f32 - svg_size.height() * scale) / 2.0;
    resvg::render(
        &tree,
        Transform::from_translate(tx, ty).pre_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // 3. 转 tauri::image::Image
    Image::new_owned(pixmap.take(), SIZE, SIZE).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bg_for_known_theme() {
        assert_eq!(bg_for("dark"), [0x16, 0x18, 0x1d, 0xff]);
        assert_eq!(bg_for("midnight"), [0x14, 0x13, 0x2b, 0xff]);
    }

    #[test]
    fn bg_for_unknown_theme_falls_back_to_light() {
        assert_eq!(bg_for("not-a-real-theme"), [0xfb, 0xfb, 0xfd, 0xff]);
    }

    #[test]
    fn stroke_picks_dark_for_light_bg() {
        assert_eq!(stroke_hex_for([0xff, 0xff, 0xff, 0xff]), "#000000");
        assert_eq!(stroke_hex_for([0xfd, 0xf6, 0xe3, 0xff]), "#000000");
    }

    #[test]
    fn stroke_picks_light_for_dark_bg() {
        assert_eq!(stroke_hex_for([0x00, 0x00, 0x00, 0xff]), "#ffffff");
        assert_eq!(stroke_hex_for([0x14, 0x13, 0x2b, 0xff]), "#ffffff");
    }

    #[test]
    fn render_returns_64x64() {
        let img = render_themed_tray_icon("light").expect("应返回图像");
        assert_eq!(img.width(), 64);
        assert_eq!(img.height(), 64);
        // 中心至少有一些非透明像素（背景填满画布即可保证）
        assert!(img.rgba().iter().any(|b| *b != 0));
    }

    #[test]
    fn render_handles_all_known_themes() {
        for (name, _) in THEME_BG {
            assert!(render_themed_tray_icon(name).is_some(), "{name} 渲染失败");
        }
    }
}
