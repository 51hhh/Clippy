use super::{
    correlate_host_clock, display_configuration, host_time_ns, os_at_least, request_running,
    stream_with_output, NativeClock, SCStreamDelegate, SCStreamOutput, FIRST_FRAME_TIMEOUT,
    STOP_TIMEOUT,
};
use crate::capture::{resolve_native_monitor, DesktopRect, FrameTime};
use anyhow::{bail, ensure, Context, Result};
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::{
    define_class, msg_send,
    rc::{autoreleasepool, Retained},
    AllocAnyThread, DefinedClass,
};
use objc2_core_audio_types::{
    kAudioFormatFlagIsBigEndian, kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved,
    kAudioFormatFlagIsPacked, kAudioFormatLinearPCM, AudioBuffer, AudioBufferList,
};
use objc2_core_media::{
    CMAudioFormatDescriptionGetStreamBasicDescription, CMClock, CMSampleBuffer, CMSyncConvertTime,
};
use objc2_foundation::{NSError, NSObject, NSObjectProtocol};
use std::{
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Mutex,
    },
    time::{Duration, Instant},
};

const AUDIO: isize = 1;
const QUEUE_CAPACITY: usize = 8;
const CHANNELS: u32 = 2;
const SAMPLE_BYTES: u32 = size_of::<f32>() as u32;

pub fn system_audio_available() -> bool {
    os_at_least(13, 0)
}

pub struct SystemAudioPacket {
    pub captured_at: Instant,
    pub frames: u32,
    pub samples: Box<[f32]>,
}

struct OutputState {
    packets: SyncSender<SystemAudioPacket>,
    failure: Mutex<Option<String>>,
    active: AtomicBool,
    clock: NativeClock,
    sample_rate: u32,
}

impl OutputState {
    fn fail(&self, message: String) {
        self.active.store(false, Ordering::Release);
        self.failure.lock().unwrap().get_or_insert(message);
    }

    fn capture(&self, stream: &NSObject, sample: &CMSampleBuffer) -> Result<()> {
        let observed = Instant::now();
        let stream_clock: Option<Retained<CMClock>> =
            unsafe { msg_send![stream, synchronizationClock] };
        let stream_clock = stream_clock.context("ScreenCaptureKit audio clock unavailable")?;
        let host_clock = unsafe { CMClock::host_time_clock() };
        let host_time = unsafe {
            CMSyncConvertTime(sample.presentation_time_stamp(), &stream_clock, &host_clock)
        };
        let FrameTime::Native(captured_at) =
            self.clock.timestamp(host_time_ns(host_time), observed)
        else {
            bail!("Invalid ScreenCaptureKit audio presentation timestamp");
        };
        let (frames, samples) = copy_stereo_pcm(sample, self.sample_rate)?;
        if let Err(TrySendError::Full(_)) = self.packets.try_send(SystemAudioPacket {
            captured_at,
            frames,
            samples,
        }) {
            bail!("ScreenCaptureKit audio queue is full");
        }
        Ok(())
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ClippySlintSystemAudioOutput"]
    #[ivars = OutputState]
    struct AudioOutput;

    unsafe impl SCStreamOutput for AudioOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn did_output(&self, stream: &NSObject, sample: &CMSampleBuffer, kind: isize) {
            let state = self.ivars();
            if kind == AUDIO && state.active.load(Ordering::Acquire) {
                autoreleasepool(|_| {
                    if let Err(error) = state.capture(stream, sample) {
                        state.fail(format!("{error:#}"));
                    }
                });
            }
        }
    }

    unsafe impl SCStreamDelegate for AudioOutput {
        #[unsafe(method(stream:didStopWithError:))]
        unsafe fn did_stop(&self, _stream: &NSObject, error: &NSError) {
            autoreleasepool(|_| self.ivars().fail(error.localizedDescription().to_string()));
        }
    }
);

unsafe impl NSObjectProtocol for AudioOutput {}

pub struct SystemAudioStream {
    stream: Retained<NSObject>,
    output: Retained<AudioOutput>,
    queue: DispatchRetained<DispatchQueue>,
    packets: Receiver<SystemAudioPacket>,
    running: bool,
}

impl SystemAudioStream {
    pub fn open(output: &str, geometry: DesktopRect, sample_rate: u32) -> Result<Self> {
        ensure!(
            system_audio_available(),
            "System audio capture needs macOS 13 or later"
        );
        ensure!(
            matches!(sample_rate, 8_000 | 16_000 | 24_000 | 48_000),
            "Unsupported system audio sample rate"
        );
        autoreleasepool(|_| {
            let monitor = resolve_native_monitor(output, geometry)?;
            let (filter, configuration) =
                display_configuration(monitor.id()?, Instant::now() + FIRST_FRAME_TIMEOUT)?;
            unsafe {
                let _: () = msg_send![&*configuration, setCapturesAudio: true];
                let _: () = msg_send![&*configuration, setSampleRate: sample_rate as isize];
                let _: () = msg_send![&*configuration, setChannelCount: CHANNELS as isize];
                let _: () = msg_send![&*configuration, setExcludesCurrentProcessAudio: true];
            }
            let (sender, packets) = mpsc::sync_channel(QUEUE_CAPACITY);
            let output = AudioOutput::alloc().set_ivars(OutputState {
                packets: sender,
                failure: Mutex::new(None),
                active: AtomicBool::new(false),
                clock: correlate_host_clock()?,
                sample_rate,
            });
            let output: Retained<AudioOutput> = unsafe { msg_send![super(output), init] };
            let queue =
                DispatchQueue::new("com.clippy.slint.system-audio", DispatchQueueAttr::SERIAL);
            let stream = stream_with_output(&filter, &configuration, &output, AUDIO, &queue)?;
            Ok(Self {
                stream,
                output,
                queue,
                packets,
                running: false,
            })
        })
    }

    pub fn start(&mut self) -> Result<Instant> {
        self.check_failure()?;
        self.running = true;
        autoreleasepool(|_| {
            request_running(&self.stream, true, Instant::now() + FIRST_FRAME_TIMEOUT)
        })?;
        let started = Instant::now();
        self.output.ivars().active.store(true, Ordering::Release);
        Ok(started)
    }

    pub fn pause(&mut self) -> Result<Instant> {
        self.output.ivars().active.store(false, Ordering::Release);
        let paused = Instant::now();
        self.stop()?;
        self.packets.try_iter().for_each(drop);
        Ok(paused)
    }

    pub fn stop(&mut self) -> Result<()> {
        if self.running {
            autoreleasepool(|_| {
                request_running(&self.stream, false, Instant::now() + STOP_TIMEOUT)
            })?;
            // 停止确认后排空已提交的原生回调，正常结束才能消费最后一批 PCM。
            self.queue.exec_sync(|| {});
            self.output.ivars().active.store(false, Ordering::Release);
            self.running = false;
        }
        self.check_failure()
    }

    pub fn read(&self, timeout: Duration) -> Result<Option<SystemAudioPacket>> {
        self.check_failure()?;
        match self.packets.recv_timeout(timeout) {
            Ok(packet) => Ok(Some(packet)),
            Err(RecvTimeoutError::Timeout) => {
                self.check_failure()?;
                Ok(None)
            }
            Err(RecvTimeoutError::Disconnected) => bail!("ScreenCaptureKit audio output closed"),
        }
    }

    fn check_failure(&self) -> Result<()> {
        if let Some(message) = self.output.ivars().failure.lock().unwrap().as_ref() {
            bail!("ScreenCaptureKit audio: {message}");
        }
        Ok(())
    }
}

impl Drop for SystemAudioStream {
    fn drop(&mut self) {
        self.output.ivars().active.store(false, Ordering::Release);
        let _ = self.stop();
    }
}

// AudioBufferList 的尾部是可变长数组。双声道平面数据需要两个 AudioBuffer。
#[repr(C)]
struct StereoBufferList {
    count: u32,
    buffers: [AudioBuffer; 2],
}

impl StereoBufferList {
    fn new(samples: &mut [f32], frames: u32, channels: u32, planar: bool) -> Self {
        let channels_per_buffer = if planar { 1 } else { channels };
        let buffer_samples = frames * channels_per_buffer;
        let mut list = Self {
            count: channels / channels_per_buffer,
            buffers: [AudioBuffer {
                mNumberChannels: 0,
                mDataByteSize: 0,
                mData: std::ptr::null_mut(),
            }; 2],
        };
        for (buffer, samples) in list
            .buffers
            .iter_mut()
            .zip(samples.chunks_exact_mut(buffer_samples as usize))
        {
            *buffer = AudioBuffer {
                mNumberChannels: channels_per_buffer,
                mDataByteSize: buffer_samples * SAMPLE_BYTES,
                mData: samples.as_mut_ptr().cast(),
            };
        }
        list
    }

    fn as_audio_buffer_list(&mut self) -> NonNull<AudioBufferList> {
        NonNull::from(self).cast()
    }
}

fn copy_stereo_pcm(sample: &CMSampleBuffer, sample_rate: u32) -> Result<(u32, Box<[f32]>)> {
    let format = unsafe { sample.format_description() }.context("Missing audio format")?;
    let description =
        unsafe { CMAudioFormatDescriptionGetStreamBasicDescription(&format).as_ref() }
            .context("Missing linear PCM format")?;
    let frames = u32::try_from(unsafe { sample.num_samples() })?;
    ensure!(
        (1..=sample_rate / 5).contains(&frames),
        "Invalid ScreenCaptureKit audio packet size"
    );
    let channels = description.mChannelsPerFrame;
    ensure!(
        (1..=CHANNELS).contains(&channels),
        "Unsupported ScreenCaptureKit audio channel count"
    );
    let flags = description.mFormatFlags;
    let planar = flags & kAudioFormatFlagIsNonInterleaved != 0;
    let channels_per_buffer = if planar { 1 } else { channels };
    let buffer_bytes_per_frame = channels_per_buffer * SAMPLE_BYTES;
    ensure!(
        description.mFormatID == kAudioFormatLinearPCM
            && flags & kAudioFormatFlagIsFloat != 0
            && flags & kAudioFormatFlagIsPacked != 0
            && flags & kAudioFormatFlagIsBigEndian == 0
            && description.mSampleRate == f64::from(sample_rate)
            && description.mBitsPerChannel == 32
            && description.mFramesPerPacket == 1
            && description.mBytesPerFrame == buffer_bytes_per_frame,
        "Unsupported ScreenCaptureKit audio format"
    );
    let mut samples = vec![0.0_f32; (frames * channels) as usize];
    let mut list = StereoBufferList::new(&mut samples, frames, channels, planar);
    let status = unsafe {
        sample.copy_pcm_data_into_audio_buffer_list(0, frames as i32, list.as_audio_buffer_list())
    };
    ensure!(
        status == 0,
        "Copying ScreenCaptureKit audio failed ({status})"
    );
    let stereo = if channels == 1 {
        samples
            .into_iter()
            .flat_map(|sample| [sample, sample])
            .collect()
    } else if planar {
        let (left, right) = samples.split_at(frames as usize);
        left.iter()
            .zip(right)
            .flat_map(|(&left, &right)| [left, right])
            .collect()
    } else {
        samples
    };
    Ok((frames, stereo.into_boxed_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_core_audio_types::AudioStreamBasicDescription;
    use objc2_core_foundation::CFRetained;
    use objc2_core_media::{
        CMAudioFormatDescriptionCreate, CMAudioSampleBufferCreateWithPacketDescriptions, CMTime,
    };
    use std::ptr;

    fn core_media_sample(
        samples: &mut [f32],
        channels: u32,
        planar: bool,
    ) -> CFRetained<CMSampleBuffer> {
        let mut flags = kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked;
        let bytes_per_frame = if planar {
            flags |= kAudioFormatFlagIsNonInterleaved;
            SAMPLE_BYTES
        } else {
            channels * SAMPLE_BYTES
        };
        let mut format = AudioStreamBasicDescription {
            mSampleRate: 48_000.0,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: flags,
            mBytesPerPacket: bytes_per_frame,
            mFramesPerPacket: 1,
            mBytesPerFrame: bytes_per_frame,
            mChannelsPerFrame: channels,
            mBitsPerChannel: 32,
            mReserved: 0,
        };
        let frames = samples.len() as u32 / channels;
        let mut format_ptr = ptr::null();
        let mut sample_ptr = ptr::null_mut();
        unsafe {
            assert_eq!(
                CMAudioFormatDescriptionCreate(
                    None,
                    NonNull::from(&mut format),
                    0,
                    ptr::null(),
                    0,
                    ptr::null(),
                    None,
                    NonNull::from(&mut format_ptr)
                ),
                0
            );
            let format = CFRetained::from_raw(NonNull::new(format_ptr.cast_mut()).unwrap());
            assert_eq!(
                CMAudioSampleBufferCreateWithPacketDescriptions(
                    None,
                    None,
                    false,
                    None,
                    ptr::null_mut(),
                    &format,
                    frames as isize,
                    CMTime::new(0, 48_000),
                    ptr::null(),
                    NonNull::from(&mut sample_ptr)
                ),
                0
            );
            let sample = CFRetained::from_raw(NonNull::new(sample_ptr).unwrap());
            let mut list = StereoBufferList::new(samples, frames, channels, planar);
            assert_eq!(
                sample.set_data_buffer_from_audio_buffer_list(
                    None,
                    None,
                    0,
                    list.as_audio_buffer_list()
                ),
                0
            );
            assert_eq!(sample.set_data_ready(), 0);
            sample
        }
    }

    #[test]
    fn copies_native_planar_interleaved_and_mono_pcm() {
        let expected = &[0.25, -0.25, 0.5, -0.5];
        for (planar, mut samples) in [(false, *expected), (true, [0.25, 0.5, -0.25, -0.5])] {
            let sample = core_media_sample(&mut samples, 2, planar);
            let (frames, copied) = copy_stereo_pcm(&sample, 48_000).unwrap();
            assert_eq!(frames, 2);
            assert_eq!(copied.as_ref(), expected);
            assert!(copy_stereo_pcm(&sample, 24_000).is_err());
        }
        let mono = core_media_sample(&mut [0.25, -0.5], 1, false);
        assert_eq!(
            copy_stereo_pcm(&mono, 48_000).unwrap().1.as_ref(),
            &[0.25, 0.25, -0.5, -0.5]
        );
    }
}
