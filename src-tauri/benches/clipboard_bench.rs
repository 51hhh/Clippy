//! 剪贴板轮询路径的基线。轮询线程每 500ms 醒一次，拿到新内容时会顺序跑
//! 哈希去重、敏感判定和（HTML 时）标签剥离，这三步都是全量扫描，
//! 所以大段内容下的成本值得有基线。不碰真实剪贴板，纯函数即可。

use clippy_lib::bench_support::{compute_hash, is_sensitive_text, strip_html_tags};
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

const LARGE: usize = 1024 * 1024;

fn large_text() -> String {
    "clipboard content line for benchmarking\n".repeat(LARGE / 40)
}

fn large_html() -> String {
    "<p class=\"line\">clipboard <b>content</b> line</p>\n".repeat(LARGE / 48)
}

fn bench(c: &mut Criterion) {
    let text = large_text();
    let html = large_html();
    // 不命中任何前缀，也不含 password/secret：走最坏路径（整段转小写 + 多次 contains）。
    let dense = "lorem ipsum dolor sit amet ".repeat(4096);

    c.bench_function("compute_hash_1mib", |b| {
        b.iter(|| compute_hash(black_box(text.as_bytes())))
    });
    c.bench_function("is_sensitive_text_worst_case_100kib", |b| {
        b.iter(|| is_sensitive_text(black_box(&dense)))
    });
    c.bench_function("is_sensitive_text_prefix_hit", |b| {
        b.iter(|| is_sensitive_text(black_box("sk-0123456789abcdef")))
    });
    c.bench_function("strip_html_tags_1mib", |b| {
        b.iter(|| strip_html_tags(black_box(&html)))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
