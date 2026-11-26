use super::{OutputCallback, AUDIO_SAMPLE_RATE_HZ};
use coreaudio::audio_unit::{
    audio_format::LinearPcmFlags,
    render_callback::{data::Interleaved, Args},
    AudioUnit, Element, IOType, SampleFormat, Scope, StreamFormat,
};
use objc2_core_audio::{AudioConvertHostTimeToNanos, AudioGetCurrentHostTime};
use objc2_core_audio_types::AudioTimeStampFlags;
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

pub(crate) struct OutputDevice;
pub(super) type OutputStream = AudioUnit;

pub(crate) fn open_device(target: Option<&str>) -> Result<OutputDevice, String> {
    match target {
        None => Ok(OutputDevice),
        Some(target) => Err(format!("macOS 仅支持系统默认播放设备: {target}")),
    }
}

pub(super) fn prepare_stream(
    _device: &OutputDevice,
    mut callback: OutputCallback,
) -> Result<OutputStream, String> {
    let prepare = || -> Result<_, coreaudio::Error> {
        let mut stream = AudioUnit::new_uninitialized(IOType::DefaultOutput)?;
        // 只配置本次播放的客户端格式，保留设备当前的采样率和缓冲大小。
        stream.set_stream_format(
            StreamFormat {
                sample_rate: f64::from(AUDIO_SAMPLE_RATE_HZ),
                sample_format: SampleFormat::F32,
                flags: LinearPcmFlags::IS_FLOAT | LinearPcmFlags::IS_PACKED,
                channels: 2,
            },
            Scope::Input,
            Element::Output,
        )?;
        stream.set_render_callback(move |args: Args<Interleaved<f32>>| {
            let stamp = args.time_stamp;
            if !stamp.mFlags.contains(AudioTimeStampFlags::HostTimeValid) {
                args.data.buffer.fill(0.0);
                callback.shared.invalid_clock.store(true, Ordering::Release);
                return Ok(());
            }
            let at = Instant::now();
            // 系统回调的时间戳和当前宿主时间使用同一时钟，接口只读取和转换时间。
            let delay = unsafe {
                let ticks = stamp.mHostTime.saturating_sub(AudioGetCurrentHostTime());
                Duration::from_nanos(AudioConvertHostTimeToNanos(ticks))
            };
            callback.write(args.data.buffer, at, delay);
            Ok(())
        })?;
        stream.initialize()?;
        Ok(stream)
    };
    prepare().map_err(|error| format!("打开 Core Audio 播放流失败: {error}"))
}

pub(super) fn start_stream(mut stream: OutputStream) -> Result<OutputStream, String> {
    stream
        .start()
        .map_err(|error| format!("启动 Core Audio 播放流失败: {error}"))?;
    Ok(stream)
}
