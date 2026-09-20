//! PX-REC-01 的 Linux X11 → VP9 端到端工程基准。
//!
//! 该模块只在显式 VP9 原型 feature 下编译。它复用生产帧源、采集 worker、三槽 pipeline、周期恢复
//! 分段和连续最终 mux；不会把基准入口接进产品 IPC，也不会把 Xwayland 数据写成原生 X11 证据。

use super::platform::x11::X11RegionFrameSource;
use super::segmenting::{RecordingEncoder, DEFAULT_SEGMENT_DURATION_NS};
use super::session::{DiagnosticRecordingConfig, DiagnosticRecordingSession};
use crate::capture::RecordingCaptureSpec;
use crate::private_files::restrict_directory;
use serde::Serialize;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

const MAX_BENCHMARK_SECONDS: u32 = 300;

pub(crate) struct X11Vp9BenchmarkOptions {
    pub output_directory: PathBuf,
    pub monitor_id: Option<u32>,
    pub crop_left: u32,
    pub crop_top: u32,
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u32,
    pub duration_seconds: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct X11Vp9BenchmarkReport {
    pub source_id: String,
    pub monitor_id: u32,
    pub physical_x: i32,
    pub physical_y: i32,
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u32,
    pub requested_duration_ns: u64,
    pub session_duration_ns: u64,
    pub wall_elapsed_ns: u64,
    pub stop_finalize_ns: u64,
    pub requested_capture_frames: u64,
    pub target_frame_slots: u64,
    pub captured_frames: u64,
    pub capture_shortfall_frames: u64,
    pub accepted_frames: u64,
    pub encoder_input_frames: u64,
    pub encoded_frames: u64,
    pub dropped_by_backpressure: u64,
    pub segment_count: usize,
    pub final_output_bytes: u64,
    pub peak_rss_kib: Option<u64>,
    pub manifest_path: PathBuf,
    pub final_output_path: PathBuf,
}

pub(crate) fn run_x11_vp9_benchmark(
    options: X11Vp9BenchmarkOptions,
) -> Result<X11Vp9BenchmarkReport, String> {
    validate_options(&options)?;
    reject_xwayland_session()?;
    let (monitor_id, selection) = resolve_selection(&options)?;
    let source = X11RegionFrameSource::connect(selection)
        .map_err(|error| format!("连接 X11 录屏帧源失败: {error}"))?;
    let descriptor = source.descriptor().clone();
    reserve_output_directory(&options.output_directory)?;

    let requested_duration_ns = u64::from(options.duration_seconds)
        .checked_mul(1_000_000_000)
        .ok_or_else(|| "X11 VP9 基准时长溢出".to_string())?;
    let started = Instant::now();
    let session = DiagnosticRecordingSession::start(
        &options.output_directory,
        DiagnosticRecordingConfig {
            session_id: "x11-vp9-benchmark".to_string(),
            source_id: descriptor.source_id.clone(),
            physical_x: descriptor.physical_x,
            physical_y: descriptor.physical_y,
            width: descriptor.width,
            height: descriptor.height,
            frames_per_second: options.frames_per_second,
            include_cursor: true,
            encoder: RecordingEncoder::Vp9Prototype,
            segment_duration_ns: DEFAULT_SEGMENT_DURATION_NS,
        },
        source,
    )
    .map_err(|error| format!("启动 X11 VP9 基准失败: {error}"))?;
    let session_directory = session.session_directory().to_path_buf();
    let capture_deadline =
        Instant::now() + Duration::from_secs(u64::from(options.duration_seconds));
    while session
        .is_running()
        .map_err(|error| format!("读取 X11 VP9 基准状态失败: {error}"))?
    {
        let remaining = capture_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        std::thread::sleep(remaining.min(Duration::from_millis(100)));
    }
    let stop_started = Instant::now();
    let report = session.stop().map_err(|error| {
        format!(
            "停止 X11 VP9 基准失败，诊断保留在 {}: {error}",
            session_directory.display()
        )
    })?;
    let stop_finalize_ns = elapsed_ns(stop_started);
    let wall_elapsed_ns = elapsed_ns(started);
    let final_output_path = report
        .final_output_path
        .ok_or_else(|| "X11 VP9 基准没有生成连续最终文件".to_string())?;
    let final_output_bytes = final_output_path
        .metadata()
        .map_err(|error| format!("读取 X11 VP9 最终文件失败: {error}"))?
        .len();
    let target_frame_slots = frame_slots(report.duration_ns, options.frames_per_second)?;
    let requested_capture_frames = u64::from(options.duration_seconds)
        .checked_mul(u64::from(options.frames_per_second))
        .ok_or_else(|| "X11 VP9 基准请求采样数溢出".to_string())?;

    Ok(X11Vp9BenchmarkReport {
        source_id: descriptor.source_id,
        monitor_id,
        physical_x: descriptor.physical_x,
        physical_y: descriptor.physical_y,
        width: descriptor.width,
        height: descriptor.height,
        frames_per_second: options.frames_per_second,
        requested_duration_ns,
        session_duration_ns: report.duration_ns,
        wall_elapsed_ns,
        stop_finalize_ns,
        requested_capture_frames,
        target_frame_slots,
        captured_frames: report.captured_frames,
        capture_shortfall_frames: requested_capture_frames.saturating_sub(report.captured_frames),
        accepted_frames: report.accepted_frames,
        encoder_input_frames: report.encoder_input_frames,
        encoded_frames: report.encoded_frames,
        dropped_by_backpressure: report.dropped_by_backpressure,
        segment_count: report.segment_paths.len(),
        final_output_bytes,
        peak_rss_kib: linux_peak_rss_kib(),
        manifest_path: session_directory.join("manifest.json"),
        final_output_path,
    })
}

fn validate_options(options: &X11Vp9BenchmarkOptions) -> Result<(), String> {
    let frame_bytes = u64::from(options.width)
        .checked_mul(u64::from(options.height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "X11 VP9 基准帧尺寸溢出".to_string())?;
    if options.output_directory.as_os_str().is_empty()
        || options.width == 0
        || options.height == 0
        || !options.width.is_multiple_of(2)
        || !options.height.is_multiple_of(2)
        || frame_bytes > super::frame::MAX_FRAME_BYTES as u64
        || !(1..=120).contains(&options.frames_per_second)
        || !(1..=MAX_BENCHMARK_SECONDS).contains(&options.duration_seconds)
    {
        return Err("X11 VP9 基准配置无效".to_string());
    }
    Ok(())
}

fn reject_xwayland_session() -> Result<(), String> {
    let xdg_reports_wayland =
        std::env::var("XDG_SESSION_TYPE").is_ok_and(|value| value.eq_ignore_ascii_case("wayland"));
    let wayland_display_is_present =
        std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty());
    if xdg_reports_wayland || wayland_display_is_present {
        return Err("当前是 Wayland/Xwayland 会话，不能记作原生 X11 录屏性能证据".to_string());
    }
    Ok(())
}

fn resolve_selection(
    options: &X11Vp9BenchmarkOptions,
) -> Result<(u32, RecordingCaptureSpec), String> {
    let (connection, screen_number) = RustConnection::connect(None)
        .map_err(|error| format!("连接 X11 基准显示器失败: {error}"))?;
    let screen = connection
        .setup()
        .roots
        .get(screen_number)
        .ok_or_else(|| "X11 基准 screen 索引无效".to_string())?;
    let xwayland = connection
        .query_extension(b"XWAYLAND")
        .map_err(|error| format!("查询 XWAYLAND 扩展失败: {error}"))?
        .reply()
        .map_err(|error| format!("读取 XWAYLAND 扩展失败: {error}"))?;
    if xwayland.present {
        return Err("目标 X 服务器是 Xwayland，不能记作原生 X11 录屏性能证据".to_string());
    }
    connection
        .randr_query_version(1, 5)
        .map_err(|error| format!("查询 X11 RandR 版本失败: {error}"))?
        .reply()
        .map_err(|error| format!("读取 X11 RandR 版本失败: {error}"))?;
    let monitors = connection
        .randr_get_monitors(screen.root, true)
        .map_err(|error| format!("查询 X11 RandR 显示器失败: {error}"))?
        .reply()
        .map_err(|error| format!("读取 X11 RandR 显示器失败: {error}"))?;
    let monitor = if let Some(monitor_id) = options.monitor_id {
        let mut matches = monitors
            .monitors
            .iter()
            .filter(|monitor| monitor.outputs.contains(&monitor_id));
        let monitor = matches
            .next()
            .ok_or_else(|| "X11 VP9 基准指定的 RandR output 不存在".to_string())?;
        if matches.next().is_some() {
            return Err("X11 VP9 基准指定的 RandR output 映射不唯一".to_string());
        }
        monitor
    } else {
        monitors
            .monitors
            .iter()
            .find(|monitor| !monitor.outputs.is_empty())
            .ok_or_else(|| "X11 VP9 基准找不到 RandR output".to_string())?
    };
    let monitor_id = options.monitor_id.unwrap_or(monitor.outputs[0]);
    let right = options
        .crop_left
        .checked_add(options.width)
        .ok_or_else(|| "X11 VP9 基准横向裁剪溢出".to_string())?;
    let bottom = options
        .crop_top
        .checked_add(options.height)
        .ok_or_else(|| "X11 VP9 基准纵向裁剪溢出".to_string())?;
    if right > u32::from(monitor.width) || bottom > u32::from(monitor.height) {
        return Err("X11 VP9 基准裁剪区域超出目标显示器".to_string());
    }
    Ok((
        monitor_id,
        RecordingCaptureSpec {
            monitor_id,
            monitor_pixel_width: u32::from(monitor.width),
            monitor_pixel_height: u32::from(monitor.height),
            crop_left: options.crop_left,
            crop_top: options.crop_top,
            crop_width: options.width,
            crop_height: options.height,
        },
    ))
}

fn reserve_output_directory(path: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir(path).map_err(|error| format!("创建 X11 VP9 基准输出目录失败: {error}"))?;
    restrict_directory(path).map_err(|error| format!("收紧 X11 VP9 基准目录权限失败: {error}"))
}

fn frame_slots(duration_ns: u64, frames_per_second: u32) -> Result<u64, String> {
    let numerator = u128::from(duration_ns)
        .checked_mul(u128::from(frames_per_second))
        .ok_or_else(|| "X11 VP9 基准帧槽数量溢出".to_string())?;
    u64::try_from(numerator.div_ceil(1_000_000_000))
        .map_err(|_| "X11 VP9 基准帧槽数量溢出".to_string())
}

fn linux_peak_rss_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))?
        .split_ascii_whitespace()
        .next()?
        .parse()
        .ok()
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(output_directory: PathBuf) -> X11Vp9BenchmarkOptions {
        X11Vp9BenchmarkOptions {
            output_directory,
            monitor_id: None,
            crop_left: 0,
            crop_top: 0,
            width: 64,
            height: 48,
            frames_per_second: 30,
            duration_seconds: 1,
        }
    }

    #[test]
    fn validates_resource_budgets_and_uses_ceiling_frame_slots() {
        let mut invalid = options(PathBuf::from("result"));
        invalid.width = 63;
        assert!(validate_options(&invalid).is_err());
        invalid.width = 64;
        invalid.duration_seconds = MAX_BENCHMARK_SECONDS + 1;
        assert!(validate_options(&invalid).is_err());
        assert_eq!(frame_slots(1_000_000_001, 30).unwrap(), 31);
    }

    #[test]
    fn output_directory_is_create_only_and_private() {
        let temporary = tempfile::tempdir().unwrap();
        let output = temporary.path().join("result");
        reserve_output_directory(&output).unwrap();
        assert!(crate::private_files::is_private(&output));
        std::fs::write(output.join("keep"), b"keep").unwrap();

        assert!(reserve_output_directory(&output).is_err());
        assert_eq!(std::fs::read(output.join("keep")).unwrap(), b"keep");
    }

    #[test]
    #[ignore = "由录屏原型 CI 在隔离 Xvfb 中显式运行"]
    fn x11_vp9_benchmark_runs_the_production_capture_pipeline() {
        let temporary = tempfile::tempdir().unwrap();
        let output = temporary.path().join("result");
        let report = run_x11_vp9_benchmark(X11Vp9BenchmarkOptions {
            frames_per_second: 10,
            ..options(output)
        })
        .unwrap();

        assert!(report.captured_frames >= 5);
        assert_eq!(report.accepted_frames, report.captured_frames);
        assert_eq!(report.requested_capture_frames, 10);
        assert_eq!(report.capture_shortfall_frames, 0);
        assert_eq!(report.encoded_frames, report.target_frame_slots);
        assert_eq!(report.segment_count, 1);
        assert!(report.final_output_bytes > 0);
        assert!(report.manifest_path.is_file());
        assert!(report.final_output_path.is_file());
        assert!(crate::private_files::is_private(&report.final_output_path));
    }
}
