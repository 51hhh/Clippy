//! 截图编解码基线。截图动作的墙钟时间几乎都花在 PNG 编码上，
//! 所以这里量的是真实导出路径用的 `encode_png`，以及前端回传时的 base64 解码。

use clippy_lib::bench_support::{decode_png_base64, encode_png, png_dimensions, validate_png};
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

/// 本机 HDMI-1 的逻辑分辨率，也就是"整屏截图提交回来"的常见最坏尺寸。
/// 只量整张解码：截图提交的信任边界必须解一次，省下的第二次就是这个数
/// （见 `capture::CommitImage`）。
const COMMIT_WIDTH: u32 = 2560;
const COMMIT_HEIGHT: u32 = 1440;

/// 渐变而不是纯色：纯色会被 PNG 过滤器压到极小，量不出真实截图的编码成本。
fn gradient_rgba(width: u32, height: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            rgba.push((x % 256) as u8);
            rgba.push((y % 256) as u8);
            rgba.push(((x + y) % 256) as u8);
            rgba.push(255);
        }
    }
    rgba
}

fn bench(c: &mut Criterion) {
    let rgba = gradient_rgba(WIDTH, HEIGHT);
    let png = encode_png(&rgba, WIDTH, HEIGHT).expect("编码基准数据失败");
    let base64 = format!(
        "data:image/png;base64,{}",
        base64_encode_for_bench(&png).as_str()
    );

    c.bench_function("encode_png_1080p", |b| {
        b.iter(|| encode_png(black_box(&rgba), WIDTH, HEIGHT).expect("编码失败"))
    });
    // 这两条要并排看：差值就是"只读头"省下来的钱，也是把整张解码留在信任边界上的理由。
    c.bench_function("png_dimensions_1080p", |b| {
        b.iter(|| png_dimensions(black_box(&png)).expect("读取尺寸失败"))
    });
    c.bench_function("validate_png_1080p", |b| {
        b.iter(|| validate_png(black_box(&png)).expect("校验失败"))
    });
    c.bench_function("decode_png_base64_1080p", |b| {
        b.iter(|| decode_png_base64(black_box(&base64)).expect("解码失败"))
    });

    let commit_rgba = gradient_rgba(COMMIT_WIDTH, COMMIT_HEIGHT);
    let commit_png =
        encode_png(&commit_rgba, COMMIT_WIDTH, COMMIT_HEIGHT).expect("编码基准数据失败");
    c.bench_function("validate_png_1440p", |b| {
        b.iter(|| validate_png(black_box(&commit_png)).expect("校验失败"))
    });
}

/// 基准自己造 data URL 输入，不依赖生产代码里没有的编码方向。
fn base64_encode_for_bench(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

criterion_group!(benches, bench);
criterion_main!(benches);
