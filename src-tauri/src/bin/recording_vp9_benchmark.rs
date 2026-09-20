//! 录屏嵌入式 VP9 writer 的显式工程基准。

use clippy_lib::bench_support::benchmark_recording_vp9;
use std::path::PathBuf;

fn usage() -> &'static str {
    "用法: recording_vp9_benchmark --output PATH [--duration 60] [--width 1920] [--height 1080] [--fps 30]"
}

fn parse_u32(value: Option<String>, name: &str) -> Result<u32, String> {
    value
        .ok_or_else(|| format!("{name} 缺少值"))?
        .parse::<u32>()
        .map_err(|_| format!("{name} 必须是正整数"))
        .and_then(|value| {
            if value == 0 {
                Err(format!("{name} 必须是正整数"))
            } else {
                Ok(value)
            }
        })
}

fn run() -> Result<(), String> {
    let mut width = 1920;
    let mut height = 1080;
    let mut frames_per_second = 30;
    let mut duration_seconds = 60;
    let mut output = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--output" => {
                output = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--output 缺少值".to_string())?,
                ));
            }
            "--duration" => {
                duration_seconds = parse_u32(arguments.next(), "--duration")?;
            }
            "--width" => width = parse_u32(arguments.next(), "--width")?,
            "--height" => height = parse_u32(arguments.next(), "--height")?,
            "--fps" => frames_per_second = parse_u32(arguments.next(), "--fps")?,
            "--help" => {
                println!("{}", usage());
                return Ok(());
            }
            _ => return Err(format!("未知参数: {argument}")),
        }
    }
    let output = output.ok_or_else(|| "--output 是必需参数".to_string())?;
    let report =
        benchmark_recording_vp9(&output, width, height, frames_per_second, duration_seconds)?;
    println!(
        "{{\"width\":{},\"height\":{},\"framesPerSecond\":{},\"durationNs\":{},\"encodedFrames\":{},\"outputBytes\":{},\"elapsedNs\":{}}}",
        report.width,
        report.height,
        report.frames_per_second,
        report.duration_ns,
        report.encoded_frames,
        report.output_bytes,
        report.elapsed_ns,
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("recording-vp9-benchmark: {error}");
        eprintln!("{}", usage());
        std::process::exit(2);
    }
}
