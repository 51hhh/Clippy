//! 搜索与列表读取的基线。搜索框每次输入都会打一次 `get_clips`，
//! 是用户能直接感知到的延迟；这里用内存库避免磁盘噪声干扰对比。

use clippy_lib::bench_support::{ContentType, StorageEngine};
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

const ROWS: usize = 2_000;
const PAGE: i64 = 50;

fn seeded_engine() -> StorageEngine {
    let engine = StorageEngine::new_in_memory().expect("创建内存库失败");
    for index in 0..ROWS {
        let text = format!(
            "clip {index} rust tauri clipboard manager entry with searchable words sqlite fts5"
        );
        engine
            .insert_clip(
                &ContentType::Text,
                Some(&text),
                None,
                None,
                &format!("hash-{index:08}"),
                text.len() as i64,
                false,
            )
            .expect("插入基准数据失败");
    }
    engine
}

fn bench(c: &mut Criterion) {
    let engine = seeded_engine();

    c.bench_function("get_clips_first_page", |b| {
        b.iter(|| {
            engine
                .get_clips(black_box(None), false, 0, PAGE)
                .expect("读取列表失败")
        })
    });
    // 三字符以上走 FTS5 前缀查询，两字符走 LIKE 回退，两条路径分别量。
    c.bench_function("get_clips_fts_prefix", |b| {
        b.iter(|| {
            engine
                .get_clips(black_box(Some("clipboard")), false, 0, PAGE)
                .expect("FTS 搜索失败")
        })
    });
    c.bench_function("get_clips_like_fallback_short_query", |b| {
        b.iter(|| {
            engine
                .get_clips(black_box(Some("ru")), false, 0, PAGE)
                .expect("LIKE 搜索失败")
        })
    });
    c.bench_function("get_clips_no_match", |b| {
        b.iter(|| {
            engine
                .get_clips(black_box(Some("zzzzznotpresent")), false, 0, PAGE)
                .expect("空结果搜索失败")
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
