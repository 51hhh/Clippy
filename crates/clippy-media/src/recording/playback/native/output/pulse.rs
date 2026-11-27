use super::{OutputCallback, AUDIO_SAMPLE_RATE_HZ, QUEUE_FRAMES};
use libpulse_sys as pa;
use std::{
    cell::UnsafeCell,
    ffi::{c_void, CStr, CString},
    mem::size_of,
    ptr::{self, NonNull},
    sync::atomic::{AtomicI32, Ordering},
    thread,
    time::{Duration, Instant},
};

const CHANNELS: usize = 2;
const FRAME_BYTES: usize = CHANNELS * size_of::<f32>();
const PERIOD_BYTES: u32 = 480 * FRAME_BYTES as u32;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) struct OutputDevice(Option<CString>);

pub(crate) fn open_device(target: Option<&str>) -> Result<OutputDevice, String> {
    target
        .map(CString::new)
        .transpose()
        .map(OutputDevice)
        .map_err(|_| "播放设备名称包含空字节".into())
}

struct Mainloop {
    raw: NonNull<pa::pa_threaded_mainloop>,
}

impl Mainloop {
    fn start() -> Result<Self, String> {
        // 新建的主循环尚未发布给其他线程。
        unsafe {
            let raw = NonNull::new(pa::pa_threaded_mainloop_new())
                .ok_or_else(|| "不能创建 PulseAudio 主循环".to_string())?;
            if pa::pa_threaded_mainloop_start(raw.as_ptr()) != 0 {
                pa::pa_threaded_mainloop_free(raw.as_ptr());
                return Err("不能启动 PulseAudio 主循环".into());
            }
            Ok(Self { raw })
        }
    }

    fn lock(&self) -> MainloopLock<'_> {
        unsafe { pa::pa_threaded_mainloop_lock(self.raw.as_ptr()) };
        MainloopLock(self)
    }

    fn wait(
        &self,
        deadline: Instant,
        phase: &str,
        mut ready: impl FnMut() -> Result<bool, String>,
    ) -> Result<(), String> {
        loop {
            {
                let _guard = self.lock();
                if ready()? {
                    return Ok(());
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("{phase}超时"));
            }
            // 仅等待启动和控制指令，写入由服务器请求触发。
            thread::sleep(remaining.min(Duration::from_millis(2)));
        }
    }
}

impl Drop for Mainloop {
    fn drop(&mut self) {
        // 停止主循环时不能持有它的锁。
        unsafe {
            pa::pa_threaded_mainloop_stop(self.raw.as_ptr());
            pa::pa_threaded_mainloop_free(self.raw.as_ptr());
        }
    }
}

struct MainloopLock<'a>(&'a Mainloop);

impl Drop for MainloopLock<'_> {
    fn drop(&mut self) {
        unsafe { pa::pa_threaded_mainloop_unlock(self.0.raw.as_ptr()) };
    }
}

struct Operation<'a> {
    loop_: &'a Mainloop,
    raw: NonNull<pa::pa_operation>,
}

impl Drop for Operation<'_> {
    fn drop(&mut self) {
        let _guard = self.loop_.lock();
        unsafe {
            pa::pa_operation_cancel(self.raw.as_ptr());
            pa::pa_operation_unref(self.raw.as_ptr());
        }
    }
}

extern "C" fn operation_result(_: *mut pa::pa_stream, success: i32, data: *mut c_void) {
    // 等待方先取消操作，再释放回调使用的结果。
    unsafe { &*data.cast::<AtomicI32>() }.store(success, Ordering::Release);
}

pub(super) struct OutputStream {
    loop_: Mainloop,
    context: *mut pa::pa_context,
    stream: *mut pa::pa_stream,
    writer: Box<UnsafeCell<Writer>>,
}

impl OutputStream {
    fn operation(
        &self,
        deadline: Instant,
        phase: &str,
        init: impl FnOnce(*mut pa::pa_stream, *mut c_void) -> *mut pa::pa_operation,
    ) -> Result<(), String> {
        let result = AtomicI32::new(-1);
        let operation = {
            let _guard = self.loop_.lock();
            let raw = init(self.stream, ptr::from_ref(&result).cast_mut().cast());
            Operation {
                loop_: &self.loop_,
                raw: NonNull::new(raw).ok_or_else(|| context_error(self.context, phase))?,
            }
        };
        self.loop_
            .wait(deadline, phase, || match result.load(Ordering::Acquire) {
                1 => Ok(true),
                -1 if unsafe { pa::pa_operation_get_state(operation.raw.as_ptr()) }
                    == pa::PA_OPERATION_RUNNING =>
                {
                    Ok(false)
                }
                _ => Err(context_error(self.context, phase)),
            })
    }
}

impl Drop for OutputStream {
    fn drop(&mut self) {
        let _guard = self.loop_.lock();
        // 先撤下回调并断开连接，才能释放 Writer。
        unsafe {
            if !self.stream.is_null() {
                pa::pa_stream_set_write_callback(self.stream, None, ptr::null_mut());
                pa::pa_stream_set_state_callback(self.stream, None, ptr::null_mut());
                pa::pa_stream_disconnect(self.stream);
                pa::pa_stream_unref(self.stream);
            }
            if !self.context.is_null() {
                pa::pa_context_disconnect(self.context);
                pa::pa_context_unref(self.context);
            }
        }
    }
}

// 调用方持有主循环锁，或正在该主循环的回调中。
fn context_error(context: *const pa::pa_context, phase: &str) -> String {
    let error = unsafe { CStr::from_ptr(pa::pa_strerror(pa::pa_context_errno(context))) };
    format!("{phase}失败: {}", error.to_string_lossy())
}

struct Writer {
    callback: OutputCallback,
    pcm: [f32; QUEUE_FRAMES * CHANNELS],
}

impl Writer {
    fn write(&mut self, stream: *mut pa::pa_stream, bytes: usize) -> Result<(), String> {
        if bytes == 0 {
            return Ok(());
        }
        if !bytes.is_multiple_of(FRAME_BYTES) || bytes / FRAME_BYTES > QUEUE_FRAMES {
            return Err(format!("PulseAudio 请求的样本数超出缓冲范围: {bytes} 字节"));
        }
        let at = Instant::now();
        let (mut latency, mut negative) = (0, 0);
        // 只写服务器已授予的字节数。PulseAudio 在返回前复制本次缓冲。
        unsafe {
            if pa::pa_stream_get_latency(stream, &mut latency, &mut negative) != 0 || negative != 0
            {
                return Err("PulseAudio 未提供有效的播放延迟".into());
            }
            let samples = &mut self.pcm[..bytes / size_of::<f32>()];
            self.callback
                .write(samples, at, Duration::from_micros(latency));
            if pa::pa_stream_write(
                stream,
                samples.as_ptr().cast(),
                bytes,
                None,
                0,
                pa::PA_SEEK_RELATIVE,
            ) != 0
            {
                return Err(context_error(
                    pa::pa_stream_get_context(stream),
                    "写入 PulseAudio 流",
                ));
            }
        }
        Ok(())
    }

    fn report(&self, error: String) {
        let mut slot = self
            .callback
            .shared
            .error
            .lock()
            .expect("播放设备错误锁被污染");
        slot.get_or_insert(error);
    }
}

extern "C" fn write_requested(stream: *mut pa::pa_stream, bytes: usize, data: *mut c_void) {
    // libpulse 在持有主循环锁时调用回调。预填和 Drop 也使用同一把锁。
    let writer = unsafe { &mut *data.cast::<Writer>() };
    if let Err(error) = writer.write(stream, bytes) {
        writer.report(error);
    }
}

extern "C" fn stream_state_changed(stream: *mut pa::pa_stream, data: *mut c_void) {
    unsafe {
        if !pa::pa_stream_is_good(pa::pa_stream_get_state(stream)) {
            let writer = &*data.cast::<Writer>();
            writer.report(context_error(
                pa::pa_stream_get_context(stream),
                "PulseAudio 播放中断",
            ));
        }
    }
}

pub(super) fn prepare_stream(
    device: &OutputDevice,
    callback: OutputCallback,
) -> Result<OutputStream, String> {
    let mut output = OutputStream {
        loop_: Mainloop::start()?,
        context: ptr::null_mut(),
        stream: ptr::null_mut(),
        writer: Box::new(UnsafeCell::new(Writer {
            callback,
            pcm: [0.0; QUEUE_FRAMES * CHANNELS],
        })),
    };
    let deadline = Instant::now() + CONTROL_TIMEOUT;
    {
        let _guard = output.loop_.lock();
        unsafe {
            let api = pa::pa_threaded_mainloop_get_api(output.loop_.raw.as_ptr());
            output.context = pa::pa_context_new(api, c"Clippy".as_ptr());
            if output.context.is_null() {
                return Err("不能创建 PulseAudio 连接".into());
            }
            if pa::pa_context_connect(
                output.context,
                ptr::null(),
                pa::PA_CONTEXT_NOAUTOSPAWN,
                ptr::null(),
            ) != 0
            {
                return Err(context_error(output.context, "连接 PulseAudio"));
            }
        }
    }
    output.loop_.wait(deadline, "连接 PulseAudio", || {
        let state = unsafe { pa::pa_context_get_state(output.context) };
        if !pa::pa_context_is_good(state) {
            return Err(context_error(output.context, "连接 PulseAudio"));
        }
        Ok(state == pa::PA_CONTEXT_READY)
    })?;
    {
        let _guard = output.loop_.lock();
        let spec = pa::pa_sample_spec {
            format: pa::PA_SAMPLE_FLOAT32NE,
            rate: AUDIO_SAMPLE_RATE_HZ,
            channels: CHANNELS as u8,
        };
        let attr = pa::pa_buffer_attr {
            maxlength: 2 * PERIOD_BYTES,
            tlength: 2 * PERIOD_BYTES,
            prebuf: u32::MAX,
            minreq: PERIOD_BYTES,
            fragsize: u32::MAX,
        };
        let mut flags = pa::PA_STREAM_START_CORKED
            | pa::PA_STREAM_ADJUST_LATENCY
            | pa::PA_STREAM_AUTO_TIMING_UPDATE
            | pa::PA_STREAM_INTERPOLATE_TIMING;
        let target = device.0.as_ref().map_or(ptr::null(), |target| {
            flags |= pa::PA_STREAM_DONT_MOVE;
            target.as_ptr()
        });
        unsafe {
            output.stream = pa::pa_stream_new(
                output.context,
                c"Clippy recording playback".as_ptr(),
                &spec,
                ptr::null(),
            );
            if output.stream.is_null()
                || pa::pa_stream_connect_playback(
                    output.stream,
                    target,
                    &attr,
                    flags,
                    ptr::null(),
                    ptr::null_mut(),
                ) != 0
            {
                return Err(context_error(output.context, "打开 PulseAudio 播放流"));
            }
        }
    }
    output.loop_.wait(deadline, "打开 PulseAudio 播放流", || {
        let state = unsafe { pa::pa_stream_get_state(output.stream) };
        if !pa::pa_stream_is_good(state) {
            return Err(context_error(output.context, "打开 PulseAudio 播放流"));
        }
        Ok(state == pa::PA_STREAM_READY)
    })?;
    output.operation(
        deadline,
        "初始化 PulseAudio 播放时钟",
        |stream, data| unsafe {
            pa::pa_stream_update_timing_info(stream, Some(operation_result), data)
        },
    )?;
    {
        let _guard = output.loop_.lock();
        unsafe {
            if let Some(target) = &device.0 {
                let actual = pa::pa_stream_get_device_name(output.stream);
                if actual.is_null() || CStr::from_ptr(actual) != target.as_c_str() {
                    return Err(format!(
                        "指定的播放设备不可用: {}",
                        target.to_string_lossy()
                    ));
                }
            }
            let writer = output.writer.get();
            pa::pa_stream_set_write_callback(output.stream, Some(write_requested), writer.cast());
            pa::pa_stream_set_state_callback(
                output.stream,
                Some(stream_state_changed),
                writer.cast(),
            );
            let writable = pa::pa_stream_writable_size(output.stream);
            (&mut *writer).write(output.stream, writable)?;
        }
    }
    Ok(output)
}

pub(super) fn start_stream(stream: OutputStream) -> Result<OutputStream, String> {
    stream.operation(
        Instant::now() + CONTROL_TIMEOUT,
        "启动 PulseAudio 播放流",
        |stream, data| unsafe { pa::pa_stream_cork(stream, 0, Some(operation_result), data) },
    )?;
    Ok(stream)
}
