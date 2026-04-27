use std::path::Path;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

const SVG_PATH: &str = "icons/file.svg";
const SIZE: u32 = 64;
const BG_RADIUS: f32 = 14.0;
const SVG_INSET: f32 = 8.0;

const THEMES: &[(&str, [u8; 4])] = &[
    ("light", [0xfb, 0xfb, 0xfd, 0xff]),
    ("dark", [0x16, 0x18, 0x1d, 0xff]),
    ("nord", [0x2e, 0x34, 0x40, 0xff]),
    ("solarized-light", [0xfd, 0xf6, 0xe3, 0xff]),
    ("rose", [0xff, 0xf7, 0xf7, 0xff]),
    ("midnight", [0x14, 0x13, 0x2b, 0xff]),
];

fn luminance(bg: [u8; 4]) -> f32 {
    fn ch(c: u8) -> f32 {
        let v = c as f32 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    }
    0.2126 * ch(bg[0]) + 0.7152 * ch(bg[1]) + 0.0722 * ch(bg[2])
}

fn render_icon(svg_source: &str, bg: [u8; 4]) -> Vec<u8> {
    let stroke = if luminance(bg) > 0.5 { "#000000" } else { "#ffffff" };
    let svg_text = svg_source.replace("stroke=\"#000000\"", &format!("stroke=\"{}\"", stroke));

    let mut pixmap = Pixmap::new(SIZE, SIZE).expect("pixmap");

    // 圆角矩形背景
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
    let bg_path = pb.finish().expect("path");

    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(bg[0], bg[1], bg[2], bg[3]));
    paint.anti_alias = true;
    pixmap.fill_path(&bg_path, &paint, FillRule::Winding, Transform::identity(), None);

    // SVG 渲染
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_str(&svg_text, &opt).expect("svg parse");
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

    pixmap.take()
}

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let svg_source = std::fs::read_to_string(SVG_PATH).expect("读取 SVG 源文件");

    for (name, bg) in THEMES {
        let rgba = render_icon(&svg_source, *bg);
        let path = Path::new(&out_dir).join(format!("tray-{name}.rgba"));
        std::fs::write(&path, &rgba).expect("写入预渲染图标");
    }

    println!("cargo::rerun-if-changed={SVG_PATH}");
    tauri_build::build()
}
