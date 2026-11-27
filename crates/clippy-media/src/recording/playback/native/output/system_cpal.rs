use super::{OutputCallback, AUDIO_SAMPLE_RATE_HZ};
pub(crate) use cpal::Device as OutputDevice;
pub(super) use cpal::Stream as OutputStream;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    BufferSize, DeviceId, HostId, StreamConfig,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const DEVICE_BUFFER_FRAMES: u32 = 480;

pub(crate) fn open_device(target: Option<&str>) -> Result<OutputDevice, String> {
    let host_id = HostId::Wasapi;
    let host = cpal::host_from_id(host_id)
        .map_err(|error| format!("无法连接 {} 输出: {error}", host_id.name()))?;
    match target {
        Some(target) => host
            .device_by_id(&DeviceId::new(host_id, target))
            .ok_or_else(|| format!("指定的播放设备不可用: {target}")),
        None => host
            .default_output_device()
            .ok_or_else(|| "系统默认播放设备不可用".to_string()),
    }
}

pub(super) fn prepare_stream(
    device: &OutputDevice,
    mut callback: OutputCallback,
) -> Result<OutputStream, String> {
    let shared = Arc::clone(&callback.shared);
    device
        .build_output_stream(
            StreamConfig {
                channels: 2,
                sample_rate: AUDIO_SAMPLE_RATE_HZ,
                buffer_size: BufferSize::Fixed(DEVICE_BUFFER_FRAMES),
            },
            move |output: &mut [f32], info| {
                let at = Instant::now();
                let timestamp = info.timestamp();
                let delay = timestamp.playback.duration_since(timestamp.callback);
                callback.write(output, at, delay);
            },
            move |error| {
                *shared.error.lock().expect("播放设备错误锁被污染") = Some(error.to_string());
            },
            Some(Duration::from_secs(2)),
        )
        .map_err(|error| format!("打开播放音频流失败: {error}"))
}

pub(super) fn start_stream(stream: OutputStream) -> Result<OutputStream, String> {
    stream
        .play()
        .map_err(|error| format!("启动播放音频流失败: {error}"))?;
    Ok(stream)
}
