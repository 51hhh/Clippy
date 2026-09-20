//! Linux X11 真实帧源到生产 VP9 会话的显式工程基准。

#[cfg(target_os = "linux")]
use clippy_lib::bench_support::{benchmark_recording_x11_vp9, RecordingX11Vp9BenchmarkOptions};
#[cfg(target_os = "linux")]
use std::path::PathBuf;

fn usage() -> &'static str {
    "用法: recording_x11_vp9_benchmark --output-dir PATH [--duration 60] [--width 1920] [--height 1080] [--fps 30] [--monitor-id ID] [--crop-left 0] [--crop-top 0]"
}

#[cfg(target_os = "linux")]
fn parse_u32(value: Option<String>, name: &str, allow_zero: bool) -> Result<u32, String> {
    let value = value
        .ok_or_else(|| format!("{name} 缺少值"))?
        .parse::<u32>()
        .map_err(|_| format!("{name} 必须是非负整数"))?;
    if !allow_zero && value == 0 {
        return Err(format!("{name} 必须是正整数"));
    }
    Ok(value)
}

#[cfg(target_os = "linux")]
fn run() -> Result<(), String> {
    let mut width = 1920;
    let mut height = 1080;
    let mut frames_per_second = 30;
    let mut duration_seconds = 60;
    let mut crop_left = 0;
    let mut crop_top = 0;
    let mut monitor_id = None;
    let mut output_directory = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--output-dir" => {
                output_directory = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--output-dir 缺少值".to_string())?,
                ));
            }
            "--duration" => {
                duration_seconds = parse_u32(arguments.next(), "--duration", false)?;
            }
            "--width" => width = parse_u32(arguments.next(), "--width", false)?,
            "--height" => height = parse_u32(arguments.next(), "--height", false)?,
            "--fps" => frames_per_second = parse_u32(arguments.next(), "--fps", false)?,
            "--monitor-id" => {
                monitor_id = Some(parse_u32(arguments.next(), "--monitor-id", true)?);
            }
            "--crop-left" => crop_left = parse_u32(arguments.next(), "--crop-left", true)?,
            "--crop-top" => crop_top = parse_u32(arguments.next(), "--crop-top", true)?,
            "--help" => {
                println!("{}", usage());
                return Ok(());
            }
            _ => return Err(format!("未知参数: {argument}")),
        }
    }
    let report = benchmark_recording_x11_vp9(RecordingX11Vp9BenchmarkOptions {
        output_directory: output_directory.ok_or_else(|| "--output-dir 是必需参数".to_string())?,
        monitor_id,
        crop_left,
        crop_top,
        width,
        height,
        frames_per_second,
        duration_seconds,
    })?;
    println!(
        "{}",
        serde_json::to_string(&report).map_err(|error| format!("序列化基准结果失败: {error}"))?
    );
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn run() -> Result<(), String> {
    Err("recording_x11_vp9_benchmark 仅支持 Linux X11".to_string())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("recording-x11-vp9-benchmark: {error}");
        eprintln!("{}", usage());
        std::process::exit(2);
    }
}
