use super::{
    latest::LatestFrames,
    pipewire_frame::{
        enum_format_pod, init_pipewire, parse_video_format, request_frame_header, ScreenBuffer,
    },
    FrozenPixels,
};
use anyhow::{anyhow, ensure, Context, Result};
use image::RgbaImage;
use nix::{
    sys::time::TimeValLike,
    time::{clock_gettime, ClockId},
};
use pipewire as pw;
use pw::{
    main_loop::MainLoopRc,
    spa::{self, param::video::VideoInfoRaw, pod::Pod},
    stream::StreamState,
};
use std::{
    cell::{Cell, RefCell},
    os::fd::OwnedFd,
    rc::Rc,
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

const UPDATE_WAIT: Duration = Duration::from_millis(40);
type FrameResult = Result<FrozenPixels>;
type FirstFrame = Rc<RefCell<Option<oneshot::Sender<FrameResult>>>>;

#[derive(Clone, Copy)]
pub(super) enum StreamMode {
    Freeze(i64),
    AwaitingSelection,
    Region([u32; 4]),
    RegionAfter([u32; 4], i64),
}

enum Command {
    Resume([u32; 4], Option<i64>, oneshot::Sender<FrameResult>),
    Quit,
}

pub(super) struct ScreenCastStream {
    frames: Arc<LatestFrames<FrozenPixels>>,
    control: pw::channel::Sender<Command>,
    worker: Option<JoinHandle<()>>,
    seen: u64,
}

impl ScreenCastStream {
    pub fn start(
        remote: OwnedFd,
        node: u32,
        source_size: (u32, u32),
        mode: StreamMode,
    ) -> Result<(Self, oneshot::Receiver<FrameResult>)> {
        let frames = Arc::new(LatestFrames::new());
        let worker_frames = frames.clone();
        let (control, commands) = pw::channel::channel();
        let (ready, first_frame) = oneshot::channel();
        let worker = thread::Builder::new()
            .name("clippy-screencast".into())
            .spawn(move || {
                let ready = Rc::new(RefCell::new(Some(ready)));
                let result = run(
                    remote,
                    node,
                    source_size,
                    mode,
                    worker_frames.clone(),
                    ready.clone(),
                    commands,
                );
                let error = result
                    .err()
                    .unwrap_or_else(|| anyhow!("ScreenCast stream closed"));
                worker_frames.publish(Err(format!("{error:#}")));
                if let Some(sender) = ready.borrow_mut().take() {
                    let _ = sender.send(Err(error));
                };
            })?;
        Ok((
            Self {
                frames,
                control,
                worker: Some(worker),
                seen: 0,
            },
            first_frame,
        ))
    }

    pub fn resume(
        &self,
        bounds: [u32; 4],
        fresh_from: Option<i64>,
    ) -> Result<oneshot::Receiver<FrameResult>> {
        let (ready, first_frame) = oneshot::channel();
        self.control
            .send(Command::Resume(bounds, fresh_from, ready))
            .map_err(|_| anyhow!("Cannot resume the shared screen"))?;
        Ok(first_frame)
    }

    pub fn read(&mut self) -> Result<RgbaImage> {
        let (sequence, frame) = self.frames.wait(self.seen, UPDATE_WAIT)?;
        let rgba = frame.rgba.to_vec();
        self.frames.check_active()?;
        self.seen = sequence;
        Ok(RgbaImage::from_raw(frame.width, frame.height, rgba)
            .expect("PipeWire 已转换为紧排 RGBA"))
    }

    pub fn check_active(&self) -> Result<()> {
        self.frames.check_active()
    }

    pub fn matches_view(&mut self, expected: &RgbaImage) -> Result<bool> {
        let (sequence, frame) = self.frames.wait(self.seen, UPDATE_WAIT)?;
        let matches = (frame.width, frame.height) == expected.dimensions()
            && frame.rgba.as_ref() == expected.as_raw();
        self.frames.check_active()?;
        self.seen = sequence;
        Ok(matches)
    }
}

impl Drop for ScreenCastStream {
    fn drop(&mut self) {
        let _ = self.control.send(Command::Quit);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(
    remote: OwnedFd,
    node: u32,
    source_size: (u32, u32),
    mode: StreamMode,
    frames: Arc<LatestFrames<FrozenPixels>>,
    ready: FirstFrame,
    commands: pw::channel::Receiver<Command>,
) -> Result<()> {
    init_pipewire();
    let mainloop = MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context
        .connect_fd_rc(remote, None)
        .context("Connecting to the authorized PipeWire remote")?;
    let publish = Rc::new(Publisher {
        mainloop: mainloop.clone(),
        frames,
        ready,
        source_size,
        mode: Cell::new(mode),
        fresh_from: Cell::new(match mode {
            StreamMode::Freeze(authorized_at) | StreamMode::RegionAfter(_, authorized_at) => {
                Some(authorized_at)
            }
            _ => None,
        }),
        last_frame_time: Cell::new(None),
    });
    let core_publisher = publish.clone();
    let _core_listener = core
        .add_listener_local()
        .error(move |id, _, _, message| {
            if id == pw::core::PW_ID_CORE {
                core_publisher.fail(anyhow!(
                    "ScreenCast PipeWire remote disconnected: {message}"
                ));
            }
        })
        .register();
    let stream = pw::stream::StreamRc::new(
        core.clone(),
        "clippy-screencast-region",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;
    let _listener = stream
        .add_local_listener_with_user_data(StreamInput {
            publish: publish.clone(),
            format: None,
            request_header: matches!(mode, StreamMode::Freeze(_) | StreamMode::RegionAfter(..)),
        })
        .state_changed(|_, input, previous, state| match state {
            StreamState::Error(error) => input
                .publish
                .fail(anyhow!("ScreenCast PipeWire stream: {error}")),
            StreamState::Unconnected if previous != StreamState::Unconnected => {
                input
                    .publish
                    .fail(anyhow!("ScreenCast PipeWire stream disconnected"));
            }
            StreamState::Paused if previous == StreamState::Streaming => {
                input.publish.frames.publish(Ok(None));
            }
            _ => {}
        })
        .param_changed(|stream, input, id, param| {
            if id == spa::param::ParamType::Format.as_raw() {
                input.publish.frames.publish(Ok(None));
                input.format = None;
                match param
                    .map(|value| {
                        let format = video_format(value, input.publish.source_size)?;
                        if input.request_header {
                            request_frame_header(stream)?;
                        }
                        Ok(format)
                    })
                    .transpose()
                {
                    Ok(info) => input.format = info,
                    Err(error) => input.publish.fail(error),
                }
            }
        })
        .process(|stream, input| {
            if let Some(info) = input.format {
                if let Err(error) = input.publish.process(stream, info) {
                    input.publish.fail(error);
                }
            }
        })
        .register()?;
    let controlled_stream = stream.clone();
    let _control = commands.attach(mainloop.loop_(), move |command| match command {
        Command::Resume(bounds, fresh_from, ready) => {
            publish.resume(&controlled_stream, bounds, fresh_from, ready)
        }
        Command::Quit => publish.mainloop.quit(),
    });
    let format_pod = enum_format_pod()?;
    let mut params = [Pod::from_bytes(&format_pod).context("Invalid ScreenCast format")?];
    stream.connect(
        spa::utils::Direction::Input,
        Some(node),
        pw::stream::StreamFlags::AUTOCONNECT,
        &mut params,
    )?;
    mainloop.run();
    Ok(())
}

struct StreamInput {
    publish: Rc<Publisher>,
    format: Option<VideoInfoRaw>,
    request_header: bool,
}

struct Publisher {
    mainloop: MainLoopRc,
    frames: Arc<LatestFrames<FrozenPixels>>,
    ready: FirstFrame,
    source_size: (u32, u32),
    mode: Cell<StreamMode>,
    fresh_from: Cell<Option<i64>>,
    last_frame_time: Cell<Option<i64>>,
}

impl Publisher {
    fn resume(
        &self,
        stream: &pw::stream::Stream,
        bounds: [u32; 4],
        fresh_from: Option<i64>,
        ready: oneshot::Sender<FrameResult>,
    ) {
        let reusable = matches!(
            self.mode.get(),
            StreamMode::Region(current) | StreamMode::RegionAfter(current, _) if current == bounds
        ) && fresh_from
            .zip(self.last_frame_time.get())
            .is_some_and(|(after, pts)| pts >= after);
        if fresh_from.is_none() {
            while stream.dequeue_buffer().is_some() {}
        }
        self.mode.set(StreamMode::Region(bounds));
        self.fresh_from.set(fresh_from);
        // 静态画面可能只在控制窗消失时提交一帧，复用已验证时间的同选区新帧。
        if reusable {
            if let Ok((_, frame)) = self.frames.wait(0, Duration::ZERO) {
                let _ = ready.send(Ok(frame));
                return;
            }
        }
        self.ready.replace(Some(ready));
    }

    fn fail(&self, error: anyhow::Error) {
        self.frames.publish(Err(format!("{error:#}")));
        if let Some(sender) = self.ready.borrow_mut().take() {
            let _ = sender.send(Err(error));
        }
        self.mainloop.quit();
    }

    fn process(&self, stream: &pw::stream::Stream, format: VideoInfoRaw) -> Result<()> {
        let mode = self.mode.get();
        let bounds = match mode {
            StreamMode::Freeze(_) => [0, 0, self.source_size.0, self.source_size.1],
            StreamMode::Region(bounds) | StreamMode::RegionAfter(bounds, _) => bounds,
            StreamMode::AwaitingSelection => {
                while stream.dequeue_buffer().is_some() {}
                return Ok(());
            }
        };
        let mut latest = None;
        while let Some(mut buffer) = ScreenBuffer::dequeue(stream) {
            if buffer.has_frame()? {
                let frame_time = if let Some(fresh_from) = self.fresh_from.get() {
                    let header = buffer.header().context(
                        "The screen sharing service did not provide a video frame timestamp",
                    )?;
                    ensure!(
                        header.pts > 0,
                        "The screen sharing service did not provide a usable presentation timestamp"
                    );
                    // 授权返回或覆盖层关闭后仍可能收到关闭前采集的画面。
                    if header.pts < fresh_from
                        || header.flags & spa::sys::SPA_META_HEADER_FLAG_CORRUPTED != 0
                    {
                        continue;
                    }
                    Some(header.pts)
                } else {
                    None
                };
                latest = Some((buffer, frame_time));
            }
        }
        if let Some((mut buffer, frame_time)) = latest {
            let frame = buffer.region(format, bounds)?;
            drop(buffer);
            let frame = FrozenPixels {
                width: frame.width,
                height: frame.height,
                rgba: frame.rgba,
            };
            if matches!(mode, StreamMode::Freeze(_)) {
                // 快速重启可能打断 wlr 仍在进行的帧请求，框选期间只停止转换。
                self.mode.set(StreamMode::AwaitingSelection);
            } else {
                self.last_frame_time.set(frame_time);
                self.frames.publish(Ok(Some(frame.clone())));
            }
            if let Some(sender) = self.ready.borrow_mut().take() {
                let _ = sender.send(Ok(frame));
            }
        }
        Ok(())
    }
}

pub(super) fn monotonic_ns() -> Result<i64> {
    Ok(clock_gettime(ClockId::CLOCK_MONOTONIC)?.num_nanoseconds())
}

pub(super) fn monotonic_ns_at(instant: Instant) -> Result<i64> {
    // Linux 的 Instant 和帧 PTS 都使用 CLOCK_MONOTONIC。
    Ok(monotonic_ns()? - i64::try_from(instant.elapsed().as_nanos())?)
}

fn video_format(param: &Pod, source_size: (u32, u32)) -> Result<VideoInfoRaw> {
    let format = parse_video_format(param)?;
    let actual = format.size();
    ensure!(
        (actual.width, actual.height) == source_size,
        "ScreenCast resolution {}×{} differs from the frozen screen {}×{}",
        actual.width,
        actual.height,
        source_size.0,
        source_size.1
    );
    Ok(format)
}
