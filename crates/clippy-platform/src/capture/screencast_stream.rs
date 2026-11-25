use super::{
    latest::LatestFrames,
    pipewire_frame::{
        enum_format_pod, has_frame, init_pipewire, parse_video_format, region_from_buffer,
        FrameRegion,
    },
    FrozenPixels,
};
use anyhow::{anyhow, ensure, Context, Result};
use image::RgbaImage;
use pipewire as pw;
use pw::{
    main_loop::MainLoopRc,
    spa::{self, param::video::VideoInfoRaw, pod::Pod},
    stream::StreamState,
};
use std::{
    cell::RefCell,
    os::fd::OwnedFd,
    rc::Rc,
    sync::Arc,
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::oneshot;

const UPDATE_WAIT: Duration = Duration::from_millis(40);
type FirstFrame = Rc<RefCell<Option<oneshot::Sender<Result<()>>>>>;

pub(super) struct ScreenCastStream {
    frames: Arc<LatestFrames<FrozenPixels>>,
    quit: pw::channel::Sender<()>,
    worker: Option<JoinHandle<()>>,
    seen: u64,
}

impl ScreenCastStream {
    pub fn start(
        remote: OwnedFd,
        node: u32,
        region: FrameRegion,
    ) -> Result<(Self, oneshot::Receiver<Result<()>>)> {
        let frames = Arc::new(LatestFrames::new());
        let worker_frames = frames.clone();
        let (quit, shutdown) = pw::channel::channel();
        let (ready, first_frame) = oneshot::channel();
        let worker = thread::Builder::new()
            .name("clippy-screencast".into())
            .spawn(move || {
                let ready = Rc::new(RefCell::new(Some(ready)));
                let result = run(
                    remote,
                    node,
                    region,
                    worker_frames.clone(),
                    ready.clone(),
                    shutdown,
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
                quit,
                worker: Some(worker),
                seen: 0,
            },
            first_frame,
        ))
    }

    pub fn read(&mut self) -> Result<RgbaImage> {
        let (sequence, frame) = self.frames.wait(self.seen, UPDATE_WAIT)?;
        let rgba = frame.rgba.to_vec();
        self.frames.check_active()?;
        self.seen = sequence;
        Ok(RgbaImage::from_raw(frame.width, frame.height, rgba)
            .expect("PipeWire 已转换为紧排 RGBA"))
    }
}

impl Drop for ScreenCastStream {
    fn drop(&mut self) {
        let _ = self.quit.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(
    remote: OwnedFd,
    node: u32,
    region: FrameRegion,
    frames: Arc<LatestFrames<FrozenPixels>>,
    ready: FirstFrame,
    shutdown: pw::channel::Receiver<()>,
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
            publish,
            region,
            format: None,
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
        .param_changed(|_, input, id, param| {
            if id == spa::param::ParamType::Format.as_raw() {
                input.publish.frames.publish(Ok(None));
                input.format = None;
                match param
                    .map(|value| video_format(value, input.region))
                    .transpose()
                {
                    Ok(info) => input.format = info,
                    Err(error) => input.publish.fail(error),
                }
            }
        })
        .process(|stream, input| {
            if let Some(info) = input.format {
                if let Err(error) = input.publish.process(stream, info, input.region) {
                    input.publish.fail(error);
                }
            }
        })
        .register()?;
    let stop = mainloop.clone();
    let _stop = shutdown.attach(mainloop.loop_(), move |_| stop.quit());
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
    region: FrameRegion,
    format: Option<VideoInfoRaw>,
}

struct Publisher {
    mainloop: MainLoopRc,
    frames: Arc<LatestFrames<FrozenPixels>>,
    ready: FirstFrame,
}

impl Publisher {
    fn fail(&self, error: anyhow::Error) {
        self.frames.publish(Err(format!("{error:#}")));
        if let Some(sender) = self.ready.borrow_mut().take() {
            let _ = sender.send(Err(error));
        }
        self.mainloop.quit();
    }

    fn process(
        &self,
        stream: &pw::stream::Stream,
        format: VideoInfoRaw,
        region: FrameRegion,
    ) -> Result<()> {
        let mut latest = None;
        while let Some(mut buffer) = stream.dequeue_buffer() {
            if has_frame(&mut buffer)? {
                latest = Some(buffer);
            }
        }
        if let Some(mut buffer) = latest {
            let frame = region_from_buffer(&mut buffer, format, region)?;
            self.frames.publish(Ok(Some(FrozenPixels {
                width: frame.width,
                height: frame.height,
                rgba: frame.rgba,
            })));
            if let Some(sender) = self.ready.borrow_mut().take() {
                let _ = sender.send(Ok(()));
            }
        }
        Ok(())
    }
}

fn video_format(param: &Pod, region: FrameRegion) -> Result<VideoInfoRaw> {
    let format = parse_video_format(param)?;
    let actual = format.size();
    ensure!(
        (actual.width, actual.height) == region.source_size,
        "ScreenCast resolution {}×{} differs from the frozen screen {}×{}",
        actual.width,
        actual.height,
        region.source_size.0,
        region.source_size.1
    );
    Ok(format)
}
