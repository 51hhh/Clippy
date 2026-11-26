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
    sel, AllocAnyThread, DefinedClass,
};
use objc2_av_foundation::AVCaptureDevice;
use objc2_core_audio_types::{
    kAudioFormatFlagIsBigEndian, kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved,
    kAudioFormatFlagIsPacked, kAudioFormatLinearPCM, AudioBuffer, AudioBufferList,
};
use objc2_core_media::{
    CMAudioFormatDescriptionGetStreamBasicDescription, CMClock, CMSampleBuffer, CMSyncConvertTime,
    CMTimeFlags,
};
use objc2_foundation::{NSError, NSObject, NSObjectProtocol, NSString};
use std::{
    ptr::NonNull,
    sync::{
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Mutex,
    },
    time::{Duration, Instant},
};

mod microphone;
pub use microphone::{enumerate_microphones, microphone_available, request_microphone_permission};

const AUDIO: isize = 1;
const MICROPHONE: isize = 2;
const SYSTEM_SAMPLE_RATE: u32 = 48_000;
const QUEUE_CAPACITY: usize = 8;
const CHANNELS: u32 = 2;
const SAMPLE_BYTES: u32 = size_of::<f32>() as u32;

pub fn system_audio_available() -> bool {
    os_at_least(13, 0)
}

pub enum AudioInput {
    System,
    Microphone(Option<String>),
}

pub struct AudioPacket {
    pub captured_at: Instant,
    pub microphone_pts: Option<(u64, u32)>,
    pub pcm: StereoPcm,
}

pub struct StereoPcm {
    pub frames: u32,
    pub rate_hz: u32,
    pub samples: Box<[f32]>,
}

struct OutputState {
    packets: SyncSender<AudioPacket>,
    failure: Mutex<Option<String>>,
    stream_clock: Mutex<Option<Retained<CMClock>>>,
    host_clock: NativeClock,
    kind: isize,
}

impl OutputState {
    fn fail(&self, message: String) {
        self.failure.lock().unwrap().get_or_insert(message);
        self.stream_clock.lock().unwrap().take();
    }

    fn capture(&self, stream_clock: &CMClock, sample: &CMSampleBuffer) -> Result<()> {
        let observed = Instant::now();
        let native_pts = unsafe { sample.presentation_time_stamp() };
        ensure!(
            native_pts.flags.contains(CMTimeFlags::Valid)
                && !native_pts
                    .flags
                    .intersects(CMTimeFlags::ImpliedValueFlagsMask)
                && native_pts.epoch == 0
                && native_pts.value >= 0
                && native_pts.timescale > 0,
            "Invalid ScreenCaptureKit audio presentation timestamp"
        );
        let host_clock = unsafe { CMClock::host_time_clock() };
        let host_time = unsafe { CMSyncConvertTime(native_pts, stream_clock, &host_clock) };
        let FrameTime::Native(captured_at) =
            self.host_clock.timestamp(host_time_ns(host_time), observed)
        else {
            bail!("Invalid ScreenCaptureKit audio presentation timestamp");
        };
        let pcm = copy_stereo_pcm(sample, (self.kind == AUDIO).then_some(SYSTEM_SAMPLE_RATE))?;
        if let Err(TrySendError::Full(_)) = self.packets.try_send(AudioPacket {
            captured_at,
            microphone_pts: (self.kind == MICROPHONE)
                .then_some((native_pts.value as u64, native_pts.timescale as u32)),
            pcm,
        }) {
            bail!("ScreenCaptureKit audio queue is full");
        }
        Ok(())
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ClippySlintAudioOutput"]
    #[ivars = OutputState]
    struct AudioOutput;

    unsafe impl SCStreamOutput for AudioOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn did_output(&self, _stream: &NSObject, sample: &CMSampleBuffer, kind: isize) {
            let state = self.ivars();
            if kind == state.kind {
                let clock = state.stream_clock.lock().unwrap().clone();
                let Some(clock) = clock else { return };
                autoreleasepool(|_| {
                    if let Err(error) = state.capture(&clock, sample) {
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

pub struct AudioStream {
    stream: Retained<NSObject>,
    output: Retained<AudioOutput>,
    queue: DispatchRetained<DispatchQueue>,
    packets: Receiver<AudioPacket>,
    microphone: Option<Retained<AVCaptureDevice>>,
    running: bool,
}

impl AudioStream {
    pub fn open(output: &str, geometry: DesktopRect, input: AudioInput) -> Result<Self> {
        autoreleasepool(|_| {
            let microphone = match &input {
                AudioInput::System => {
                    ensure!(
                        system_audio_available(),
                        "System audio capture needs macOS 13 or later"
                    );
                    None
                }
                AudioInput::Microphone(id) => Some(microphone::resolve(id.as_deref())?),
            };
            let monitor = resolve_native_monitor(output, geometry)?;
            let (filter, configuration) =
                display_configuration(monitor.id()?, Instant::now() + FIRST_FRAME_TIMEOUT)?;
            let device_id = microphone
                .as_ref()
                .map(|device| unsafe { device.uniqueID() });
            let kind = configure_input(&configuration, device_id.as_deref())?;
            let (sender, packets) = mpsc::sync_channel(QUEUE_CAPACITY);
            let output = AudioOutput::alloc().set_ivars(OutputState {
                packets: sender,
                failure: Mutex::new(None),
                stream_clock: Mutex::new(None),
                host_clock: correlate_host_clock()?,
                kind,
            });
            let output: Retained<AudioOutput> = unsafe { msg_send![super(output), init] };
            let queue =
                DispatchQueue::new("com.clippy.slint.capture-audio", DispatchQueueAttr::SERIAL);
            let stream = stream_with_output(&filter, &configuration, &output, kind, &queue)?;
            Ok(Self {
                stream,
                output,
                queue,
                packets,
                microphone,
                running: false,
            })
        })
    }

    pub fn start(&mut self) -> Result<Instant> {
        self.check_failure()?;
        if self.microphone.is_some() {
            microphone::check_permission()?;
        }
        self.running = true;
        let clock = autoreleasepool(|_| {
            request_running(&self.stream, true, Instant::now() + FIRST_FRAME_TIMEOUT)?;
            let clock: Option<Retained<CMClock>> =
                unsafe { msg_send![&*self.stream, synchronizationClock] };
            clock.context("ScreenCaptureKit audio clock unavailable")
        })?;
        let started = Instant::now();
        *self.output.ivars().stream_clock.lock().unwrap() = Some(clock);
        self.check_failure()?;
        Ok(started)
    }

    pub fn pause(&mut self) -> Result<Instant> {
        self.output.ivars().stream_clock.lock().unwrap().take();
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
            self.output.ivars().stream_clock.lock().unwrap().take();
            self.running = false;
        }
        self.check_failure()
    }

    pub fn read(&self, timeout: Duration) -> Result<Option<AudioPacket>> {
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
        if let Some(device) = &self.microphone {
            ensure!(
                unsafe { device.isConnected() },
                "The selected microphone was disconnected"
            );
        }
        Ok(())
    }
}

impl Drop for AudioStream {
    fn drop(&mut self) {
        self.output.ivars().stream_clock.lock().unwrap().take();
        let _ = self.stop();
    }
}

fn configure_input(configuration: &NSObject, microphone_id: Option<&NSString>) -> Result<isize> {
    if let Some(id) = microphone_id {
        ensure!(
            configuration.respondsToSelector(sel!(setCaptureMicrophone:))
                && configuration.respondsToSelector(sel!(setMicrophoneCaptureDeviceID:)),
            "ScreenCaptureKit microphone capture is unavailable"
        );
        unsafe {
            let _: () = msg_send![configuration, setMicrophoneCaptureDeviceID: id];
            let _: () = msg_send![configuration, setCaptureMicrophone: true];
        }
        Ok(MICROPHONE)
    } else {
        unsafe {
            let _: () = msg_send![configuration, setCapturesAudio: true];
            let _: () = msg_send![configuration, setSampleRate: SYSTEM_SAMPLE_RATE as isize];
            let _: () = msg_send![configuration, setChannelCount: CHANNELS as isize];
            let _: () = msg_send![configuration, setExcludesCurrentProcessAudio: true];
        }
        Ok(AUDIO)
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

fn copy_stereo_pcm(sample: &CMSampleBuffer, expected_rate: Option<u32>) -> Result<StereoPcm> {
    let format = unsafe { sample.format_description() }.context("Missing audio format")?;
    let description =
        unsafe { CMAudioFormatDescriptionGetStreamBasicDescription(&format).as_ref() }
            .context("Missing linear PCM format")?;
    let sample_rate = description.mSampleRate;
    ensure!(
        (8_000.0..=192_000.0).contains(&sample_rate) && sample_rate.fract() == 0.0,
        "Unsupported ScreenCaptureKit audio sample rate"
    );
    let rate_hz = sample_rate as u32;
    ensure!(
        expected_rate.is_none_or(|expected| expected == rate_hz),
        "Unexpected ScreenCaptureKit system audio sample rate"
    );
    let frames = u32::try_from(unsafe { sample.num_samples() })?;
    ensure!(
        (1..=rate_hz / 5).contains(&frames),
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
            && description.mBitsPerChannel == 32
            && description.mFramesPerPacket == 1
            && description.mBytesPerPacket == buffer_bytes_per_frame
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
    ensure!(
        stereo.iter().all(|sample| sample.is_finite()),
        "Non-finite ScreenCaptureKit audio sample"
    );
    Ok(StereoPcm {
        frames,
        rate_hz,
        samples: stereo.into_boxed_slice(),
    })
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
        rate_hz: u32,
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
            mSampleRate: f64::from(rate_hz),
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
                    CMTime::new(0, rate_hz as i32),
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
            let sample = core_media_sample(&mut samples, 48_000, 2, planar);
            let copied = copy_stereo_pcm(&sample, Some(48_000)).unwrap();
            assert_eq!(copied.frames, 2);
            assert_eq!(copied.samples.as_ref(), expected);
            assert!(copy_stereo_pcm(&sample, Some(24_000)).is_err());
        }
        let mono = core_media_sample(&mut [0.25, -0.5], 44_100, 1, false);
        let copied = copy_stereo_pcm(&mono, None).unwrap();
        assert_eq!(copied.rate_hz, 44_100);
        assert_eq!(copied.samples.as_ref(), &[0.25, 0.25, -0.5, -0.5]);
        assert!(copy_stereo_pcm(&mono, Some(48_000)).is_err());
        let invalid = core_media_sample(&mut [f32::NAN], 96_000, 1, false);
        assert!(copy_stereo_pcm(&invalid, None).is_err());
    }

    #[test]
    fn microphone_sets_only_the_independent_output_and_device() {
        if !microphone_available() {
            return;
        }
        autoreleasepool(|_| {
            super::super::framework().unwrap();
            let class = super::super::class(c"SCStreamConfiguration").unwrap();
            let config: Retained<NSObject> = unsafe { msg_send![class, new] };
            let id = NSString::from_str("clippy-test-device");
            assert_eq!(configure_input(&config, Some(&id)).unwrap(), MICROPHONE);
            let microphone: bool = unsafe { msg_send![&*config, captureMicrophone] };
            let system: bool = unsafe { msg_send![&*config, capturesAudio] };
            let selected: Option<Retained<NSString>> =
                unsafe { msg_send![&*config, microphoneCaptureDeviceID] };
            assert!(microphone);
            assert!(!system);
            assert_eq!(selected.as_deref(), Some(&*id));
        });
    }
}
