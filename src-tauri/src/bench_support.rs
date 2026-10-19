//! criterion 基准用的内部入口。
//!
//! 这里是 `benches/` 唯一能碰到的内部实现：模块本身在 crate 里是私有的，
//! 基准通过这些 re-export/wrapper 调用真实生产代码，而不是在 bench 文件里
//! 复制一份实现——复制出来的基准量的是副本，和生产路径分叉后毫无意义。
//!
//! **不是稳定 API**：这里出现什么完全由基准需要决定，随时可以改。

pub use crate::models::{ClipItem, ContentType};
pub use crate::screenshot::{decode_png_base64, encode_png, png_dimensions, validate_png};
pub use crate::storage::StorageEngine;

/// 剪贴板去重哈希。轮询线程每次拿到新内容都要跑一遍全量内容。
pub fn compute_hash(data: &[u8]) -> String {
    crate::clipboard_watcher::content::compute_hash(data)
}

/// 图片轮询之间的"还是上一张吗"指纹。挡掉的是一整次 PNG 编码。
pub fn rgba_fingerprint(width: usize, height: usize, bytes: &[u8]) -> u64 {
    crate::clipboard_watcher::content::rgba_fingerprint(width, height, bytes)
}

/// 敏感内容判定。命中前缀会提前返回，最坏情况是整段文本转小写后多次 contains。
pub fn is_sensitive_text(text: &str) -> bool {
    crate::clipboard_watcher::content::is_sensitive_text(text)
}

/// HTML 转可搜索纯文本，逐字符扫描。
pub fn strip_html_tags(html: &str) -> String {
    crate::clipboard_watcher::content::strip_html_tags(html)
}

#[cfg(feature = "recording-vp9-prototype")]
#[derive(Debug, Clone, Copy)]
pub struct RecordingVp9BenchmarkReport {
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u32,
    pub duration_ns: u64,
    pub encoded_frames: u64,
    pub output_bytes: u64,
    pub elapsed_ns: u64,
    pub fixture_preparation_ns: u64,
    pub color_conversion_ns: u64,
    pub encode_mux_ns: u64,
    pub sync_ns: u64,
}

/// Linux X11 真实帧源 → 生产 VP9 会话的显式工程基准参数。
#[cfg(all(feature = "recording-vp9-prototype", target_os = "linux"))]
#[derive(Debug, Clone)]
pub struct RecordingX11Vp9BenchmarkOptions {
    pub output_directory: std::path::PathBuf,
    pub monitor_id: Option<u32>,
    pub crop_left: u32,
    pub crop_top: u32,
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u32,
    pub duration_seconds: u32,
}

#[cfg(all(feature = "recording-vp9-prototype", target_os = "linux"))]
pub use crate::recording::X11Vp9BenchmarkReport as RecordingX11Vp9BenchmarkReport;

/// 运行真实 X11 帧源、三槽 pipeline、周期分段和连续 VP9 最终输出。
#[cfg(all(feature = "recording-vp9-prototype", target_os = "linux"))]
pub fn benchmark_recording_x11_vp9(
    options: RecordingX11Vp9BenchmarkOptions,
) -> Result<RecordingX11Vp9BenchmarkReport, String> {
    crate::recording::run_x11_vp9_benchmark(crate::recording::X11Vp9BenchmarkOptions {
        output_directory: options.output_directory,
        monitor_id: options.monitor_id,
        crop_left: options.crop_left,
        crop_top: options.crop_top,
        width: options.width,
        height: options.height,
        frames_per_second: options.frames_per_second,
        duration_seconds: options.duration_seconds,
    })
}

/// 用生产 RGBA → I420 → libvpx → WebM writer 生成确定性屏幕样本。
///
/// 该入口只供 `src/bin/recording_vp9_benchmark.rs` 使用；它刻意不经过 FFmpeg，也不复制编码逻辑，
/// 用于区分系统 FFmpeg 方向性数据与 Clippy 实际嵌入式 writer 的成本。
#[cfg(feature = "recording-vp9-prototype")]
pub fn benchmark_recording_vp9(
    output_path: &std::path::Path,
    width: u32,
    height: u32,
    frames_per_second: u32,
    duration_seconds: u32,
) -> Result<RecordingVp9BenchmarkReport, String> {
    use crate::recording::Vp9WebmWriter;
    use std::fs::OpenOptions;
    use std::time::Instant;

    let frame_count = u64::from(frames_per_second)
        .checked_mul(u64::from(duration_seconds))
        .filter(|frames| *frames > 0 && *frames <= 18_000)
        .ok_or_else(|| "录屏 VP9 基准帧数超出预算".to_string())?;
    let _ = recording_fixture_byte_len(width, height)?;
    if width < 8
        || height < 8
        || !width.is_multiple_of(2)
        || !height.is_multiple_of(2)
        || !(1..=120).contains(&frames_per_second)
    {
        return Err("录屏 VP9 基准配置无效".to_string());
    }
    let output_path = output_path.to_path_buf();
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .map_err(|error| format!("创建 VP9 基准输出失败: {error}"))?;
    let result = (|| {
        let mut writer = Vp9WebmWriter::new(file, width, height, frames_per_second, 1)
            .map_err(|error| format!("创建 VP9 基准 writer 失败: {error}"))?;
        let base = recording_fixture_rgba(width, height)?;
        let mut frame = base.clone();
        let square = (width.min(height) / 14).clamp(8, 160);
        let started = Instant::now();
        let mut previous = None;
        let mut fixture_preparation_ns = 0_u64;
        let mut color_conversion_ns = 0_u64;
        let mut encode_mux_ns = 0_u64;
        for index in 0..frame_count {
            let fixture_started = Instant::now();
            if let Some((x, y)) = previous {
                copy_recording_fixture_rect(&base, &mut frame, width, x, y, square);
            }
            let x = u32::try_from(
                (u128::from(index) * 13) % u128::from(width.saturating_sub(square).max(1)),
            )
            .map_err(|_| "录屏 VP9 基准横坐标溢出".to_string())?;
            let y = u32::try_from(
                (u128::from(index) * 7) % u128::from(height.saturating_sub(square).max(1)),
            )
            .map_err(|_| "录屏 VP9 基准纵坐标溢出".to_string())?;
            paint_recording_fixture_rect(&mut frame, width, x, y, square, index);
            previous = Some((x, y));
            fixture_preparation_ns =
                fixture_preparation_ns.saturating_add(elapsed_benchmark_ns(fixture_started));
            let presentation_at_ns = u128::from(index)
                .checked_mul(1_000_000_000)
                .and_then(|value| value.checked_div(u128::from(frames_per_second)))
                .and_then(|value| u64::try_from(value).ok())
                .ok_or_else(|| "录屏 VP9 基准时间戳溢出".to_string())?;
            let (frame_color_ns, frame_encode_mux_ns) = writer
                .push_rgba_profiled(&frame, presentation_at_ns)
                .map_err(|error| format!("VP9 基准编码失败: {error}"))?;
            color_conversion_ns = color_conversion_ns.saturating_add(frame_color_ns);
            encode_mux_ns = encode_mux_ns.saturating_add(frame_encode_mux_ns);
        }
        let duration_ns = u128::from(frame_count)
            .checked_mul(1_000_000_000)
            .and_then(|value| value.checked_div(u128::from(frames_per_second)))
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| "录屏 VP9 基准时长溢出".to_string())?;
        let finalize_started = Instant::now();
        let output = writer
            .finish_with_stats(duration_ns)
            .map_err(|error| format!("VP9 基准封尾失败: {error}"))?;
        encode_mux_ns = encode_mux_ns.saturating_add(elapsed_benchmark_ns(finalize_started));
        let sync_started = Instant::now();
        output
            .writer
            .sync_all()
            .map_err(|error| format!("同步 VP9 基准输出失败: {error}"))?;
        let sync_ns = elapsed_benchmark_ns(sync_started);
        let elapsed_ns = elapsed_benchmark_ns(started);
        let output_bytes = output
            .writer
            .metadata()
            .map_err(|error| format!("读取 VP9 基准输出失败: {error}"))?
            .len();
        Ok(RecordingVp9BenchmarkReport {
            width,
            height,
            frames_per_second,
            duration_ns,
            encoded_frames: output.frame_count,
            output_bytes,
            elapsed_ns,
            fixture_preparation_ns,
            color_conversion_ns,
            encode_mux_ns,
            sync_ns,
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(output_path);
    }
    result
}

#[cfg(feature = "recording-vp9-prototype")]
fn elapsed_benchmark_ns(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(feature = "recording-vp9-prototype")]
fn recording_fixture_rgba(width: u32, height: u32) -> Result<Vec<u8>, String> {
    let bytes = recording_fixture_byte_len(width, height)?;
    let mut rgba = vec![0_u8; bytes];
    for y in 0..height {
        for x in 0..width {
            let offset = usize::try_from((u64::from(y) * u64::from(width) + u64::from(x)) * 4)
                .map_err(|_| "录屏 VP9 基准像素坐标溢出".to_string())?;
            let grid = if x % 96 == 0 || y % 64 == 0 { 28 } else { 0 };
            rgba[offset] = (20_u32 + x.saturating_mul(140) / width + grid).min(255) as u8;
            rgba[offset + 1] = (32_u32 + y.saturating_mul(120) / height + grid).min(255) as u8;
            rgba[offset + 2] = (52_u32 + (x + y) % 120 + grid).min(255) as u8;
            rgba[offset + 3] = 255;
        }
    }
    Ok(rgba)
}

#[cfg(feature = "recording-vp9-prototype")]
fn recording_fixture_byte_len(width: u32, height: u32) -> Result<usize, String> {
    const MAX_FRAME_BYTES: u64 = 64 * 1024 * 1024;

    u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes <= MAX_FRAME_BYTES)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| "录屏 VP9 基准帧超过 64 MiB 预算".to_string())
}

#[cfg(feature = "recording-vp9-prototype")]
fn copy_recording_fixture_rect(
    source: &[u8],
    destination: &mut [u8],
    width: u32,
    x: u32,
    y: u32,
    size: u32,
) {
    let row_bytes = usize::try_from(u64::from(size) * 4).expect("矩形行字节已受帧预算约束");
    for row in y..y + size {
        let start = usize::try_from((u64::from(row) * u64::from(width) + u64::from(x)) * 4)
            .expect("矩形坐标已受帧预算约束");
        destination[start..start + row_bytes].copy_from_slice(&source[start..start + row_bytes]);
    }
}

#[cfg(feature = "recording-vp9-prototype")]
fn paint_recording_fixture_rect(
    rgba: &mut [u8],
    width: u32,
    x: u32,
    y: u32,
    size: u32,
    frame: u64,
) {
    for row in y..y + size {
        for column in x..x + size {
            let offset =
                usize::try_from((u64::from(row) * u64::from(width) + u64::from(column)) * 4)
                    .expect("矩形坐标已受帧预算约束");
            rgba[offset] = 248;
            rgba[offset + 1] = 80_u8.wrapping_add(frame as u8);
            rgba[offset + 2] = 64;
            rgba[offset + 3] = 255;
        }
    }
}

#[cfg(all(test, feature = "recording-vp9-prototype"))]
mod recording_vp9_benchmark_tests {
    use super::benchmark_recording_vp9;

    #[test]
    fn rejects_frame_over_product_budget_before_creating_output() {
        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let output = directory.path().join("oversized.webm");

        let error = benchmark_recording_vp9(&output, 20_000, 2_000, 1, 1)
            .expect_err("超过 64 MiB 的 RGBA 帧应被拒绝");

        assert!(error.contains("64 MiB"));
        assert!(!output.exists());
    }

    #[test]
    fn never_overwrites_existing_output() {
        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let output = directory.path().join("existing.webm");
        std::fs::write(&output, b"keep").expect("写入占位文件失败");

        let error = benchmark_recording_vp9(&output, 8, 8, 1, 1).expect_err("基准不应覆盖已有输出");

        assert!(error.contains("创建 VP9 基准输出失败"));
        assert_eq!(std::fs::read(output).expect("读取占位文件失败"), b"keep");
    }
}
