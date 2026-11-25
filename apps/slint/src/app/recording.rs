use super::{editor, post, settings, App, Event};
use crate::{ClippyTheme, RecordingFile, RecordingLibrary, RecordingRow};
use anyhow::{anyhow, Context, Result};
use clippy_core::image_output::{expand_user_path, unique_image_id};
#[cfg(all(
    feature = "native-recording-audio",
    any(target_os = "linux", target_os = "windows")
))]
use clippy_media::recording::av_session::{AvRecordingConfig, AvRecordingSession};
use clippy_media::recording::{
    clock::RecordingSessionClock,
    frame::CapturedFrame,
    manifest::{self, RecordingLibraryItem, ResolvedRecordingArtifact},
    recover_interrupted_sessions,
    segmenting::{RecordingEncoder, DEFAULT_SEGMENT_DURATION_NS},
    session::{DiagnosticRecordingConfig, DiagnosticRecordingSession},
    worker::RecordingFrameSource,
};
use clippy_platform::capture::{Region, RegionCancellation, RegionReader};
use image::RgbaImage;
use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, SharedString, Timer, VecModel};
use std::{
    collections::VecDeque,
    fs, io,
    path::{Path, PathBuf},
    process::{Command as Process, Stdio},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

mod control;

#[derive(Clone, Copy)]
pub(super) enum Control {
    PauseResume,
    Stop,
}

#[derive(Clone, Copy, Default)]
pub(super) enum AudioTrack {
    #[default]
    Silent,
    #[cfg(all(
        feature = "native-recording-audio",
        any(target_os = "linux", target_os = "windows")
    ))]
    System,
    #[cfg(all(
        feature = "native-recording-audio",
        any(target_os = "linux", target_os = "windows")
    ))]
    Microphone,
}

impl AudioTrack {
    fn from_index(index: i32) -> Self {
        match index {
            #[cfg(all(
                feature = "native-recording-audio",
                any(target_os = "linux", target_os = "windows")
            ))]
            1 => Self::System,
            #[cfg(all(
                feature = "native-recording-audio",
                any(target_os = "linux", target_os = "windows")
            ))]
            2 => Self::Microphone,
            _ => Self::Silent,
        }
    }

    const fn names() -> &'static [&'static str] {
        #[cfg(all(
            feature = "native-recording-audio",
            any(target_os = "linux", target_os = "windows")
        ))]
        {
            &["No audio", "Default system audio", "Default microphone"]
        }
        #[cfg(not(all(
            feature = "native-recording-audio",
            any(target_os = "linux", target_os = "windows")
        )))]
        {
            &["No audio"]
        }
    }
}

pub(super) enum Message {
    Show,
    Refresh,
    Close,
    Begin(Region, (u32, u32)),
    HideControl,
    Control(Control),
    Audio(i32),
    List(u64, Result<Vec<RecordingLibraryItem>>),
    Preview(String),
    PreviewReady(u64, String, Result<Option<RgbaImage>>),
    Started(u64),
    Toggled(u64, Result<bool>),
    Finished(u64, Result<String>),
    Play(String),
    Export(String, String),
    Delete(String),
    Merge(String),
    Update(Result<String>),
    Mutation(Result<String>),
}

#[derive(Default)]
pub(super) struct Recordings {
    window: Option<RecordingLibrary>,
    control: Option<control::View>,
    job: Option<Job>,
    playback: Option<Playback>,
    editing_library: bool,
    list_generation: u64,
    rows: Rc<VecModel<RecordingRow>>,
    preview_queue: VecDeque<String>,
    preview_cached: VecDeque<String>,
    preview_running: bool,
    recovered: Arc<Mutex<bool>>,
    audio_selection: i32,
    status: String,
    paused: bool,
    toggle_pending: bool,
    finishing: bool,
}

struct Job {
    id: u64,
    control: Option<SyncSender<Control>>,
    worker: Option<JoinHandle<()>>,
    cancellation: RegionCancellation,
}

struct Playback {
    stop: Arc<AtomicBool>,
    worker: JoinHandle<()>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.control.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Recordings {
    fn drop(&mut self) {
        self.stop_playback();
        if let Some(window) = self.window.take() {
            let _ = window.hide();
        }
    }
}

struct Source {
    clock: RecordingSessionClock,
    region: RegionReader,
    sequence: u64,
}

impl RecordingFrameSource for Source {
    type Error = io::Error;

    fn capture_next(&mut self) -> io::Result<CapturedFrame> {
        let image = self.region.read().map_err(io::Error::other)?;
        let (width, height) = image.dimensions();
        let sequence = self.sequence;
        self.sequence += 1;
        Ok(CapturedFrame {
            sequence,
            captured_at_ns: self.clock.now_ns(),
            width,
            height,
            stride: width * 4,
            rgba: image.into_raw().into_boxed_slice(),
        })
    }

    fn control_timestamp_ns(&mut self) -> io::Result<u64> {
        Ok(self.clock.now_ns())
    }
}

enum Session {
    Video(DiagnosticRecordingSession),
    #[cfg(all(
        feature = "native-recording-audio",
        any(target_os = "linux", target_os = "windows")
    ))]
    WithAudio(AvRecordingSession),
}

impl Session {
    fn start(
        base: &Path,
        region: Region,
        config: DiagnosticRecordingConfig,
        track: AudioTrack,
        cancellation: RegionCancellation,
    ) -> Result<Self> {
        let video = move |clock| {
            let region =
                RegionReader::open(region, cancellation).map_err(|error| format!("{error:#}"))?;
            Ok(Source {
                clock,
                region,
                sequence: 0,
            })
        };
        match track {
            AudioTrack::Silent => Ok(Self::Video(DiagnosticRecordingSession::start_with_factory(
                base, config, video,
            )?)),
            #[cfg(all(
                feature = "native-recording-audio",
                any(target_os = "linux", target_os = "windows")
            ))]
            AudioTrack::System | AudioTrack::Microphone => {
                #[cfg(target_os = "linux")]
                let plan = {
                    use clippy_media::recording::platform::linux_audio::LinuxPipeWireAudioSourcePlan as Plan;
                    match track {
                        AudioTrack::System => Plan::system_audio(None),
                        AudioTrack::Microphone => Plan::microphone(None),
                        AudioTrack::Silent => unreachable!(),
                    }
                };
                #[cfg(target_os = "windows")]
                let plan = {
                    use clippy_media::recording::platform::windows_audio::WindowsWasapiAudioSourcePlan as Plan;
                    match track {
                        AudioTrack::System => Plan::system_loopback(None),
                        AudioTrack::Microphone => Plan::microphone(None),
                        AudioTrack::Silent => unreachable!(),
                    }
                };
                let config = AvRecordingConfig {
                    session_id: config.session_id,
                    source_id: config.source_id,
                    physical_x: config.physical_x,
                    physical_y: config.physical_y,
                    width: config.width,
                    height: config.height,
                    frames_per_second: config.frames_per_second,
                    include_cursor: config.include_cursor,
                    audio_channels: plan.channels(),
                    segment_duration_ns: config.segment_duration_ns,
                };
                Ok(Self::WithAudio(AvRecordingSession::start_with_factories(
                    base,
                    config,
                    video,
                    |clock| plan.connect(clock).map_err(|error| error.to_string()),
                )?))
            }
        }
    }

    fn toggle(&self, paused: bool) -> Result<()> {
        match self {
            Self::Video(session) => {
                if paused {
                    session.resume()?
                } else {
                    session.pause()?
                }
            }
            #[cfg(all(
                feature = "native-recording-audio",
                any(target_os = "linux", target_os = "windows")
            ))]
            Self::WithAudio(session) => {
                if paused {
                    session.resume()?
                } else {
                    session.pause()?
                }
            }
        }
        Ok(())
    }

    fn terminated(&self) -> bool {
        match self {
            Self::Video(session) => session.has_terminated_worker(),
            #[cfg(all(
                feature = "native-recording-audio",
                any(target_os = "linux", target_os = "windows")
            ))]
            Self::WithAudio(session) => session.has_terminated_worker(),
        }
    }

    fn stop(self) -> Result<(u64, u64)> {
        match self {
            Self::Video(session) => session
                .stop()
                .map(|report| (report.duration_ns, report.encoded_frames))
                .map_err(Into::into),
            #[cfg(all(
                feature = "native-recording-audio",
                any(target_os = "linux", target_os = "windows")
            ))]
            Self::WithAudio(session) => session
                .stop()
                .map(|report| (report.duration_ns, report.video_encoded_frames))
                .map_err(Into::into),
        }
    }
}

fn send(message: Message) {
    post(Event::Recordings(message));
}

fn recover_once(root: &Path, recovered: &Mutex<bool>) -> io::Result<()> {
    let mut recovered = recovered.lock().expect("录屏恢复锁被污染");
    if !*recovered {
        recover_interrupted_sessions(root)?;
        *recovered = true;
    }
    Ok(())
}

impl Job {
    fn open(
        id: u64,
        base: PathBuf,
        region: Region,
        size: (u32, u32),
        recovered: Arc<Mutex<bool>>,
        track: AudioTrack,
    ) -> io::Result<Self> {
        let (control, commands) = mpsc::sync_channel(4);
        let cancellation = RegionCancellation::default();
        let worker_cancellation = cancellation.clone();
        let worker = thread::Builder::new()
            .name("clippy-recording".into())
            .spawn(move || {
                let result = (|| {
                    recover_once(&base, &recovered)?;
                    let bounds = control::recorded_rectangle(&region, size);
                    let config = DiagnosticRecordingConfig {
                        session_id: unique_image_id(),
                        source_id: region.output.clone(),
                        physical_x: bounds.x,
                        physical_y: bounds.y,
                        width: bounds.width,
                        height: bounds.height,
                        frames_per_second: 15,
                        include_cursor: false,
                        encoder: recording_encoder(),
                        segment_duration_ns: DEFAULT_SEGMENT_DURATION_NS,
                    };
                    let session =
                        Session::start(&base, region, config, track, worker_cancellation)?;
                    send(Message::Started(id));
                    let mut paused = false;
                    loop {
                        match commands.recv_timeout(Duration::from_millis(250)) {
                            Ok(Control::PauseResume) => {
                                if let Err(error) = session.toggle(paused) {
                                    send(Message::Toggled(id, Err(error)));
                                    if session.terminated() {
                                        break;
                                    }
                                    continue;
                                }
                                paused = !paused;
                                send(Message::Toggled(id, Ok(paused)));
                            }
                            Ok(Control::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                            Err(RecvTimeoutError::Timeout) if session.terminated() => {
                                anyhow::bail!(
                                    "Capture source stopped. Committed segments can be recovered."
                                );
                            }
                            Err(RecvTimeoutError::Timeout) => {}
                        }
                    }
                    let (duration, frames) = session.stop()?;
                    Ok(format!(
                        "Saved recording · {:.1} s · {} frames",
                        duration as f64 / 1e9,
                        frames
                    ))
                })();
                send(Message::Finished(id, result));
            })?;
        Ok(Self {
            id,
            control: Some(control),
            worker: Some(worker),
            cancellation,
        })
    }
}

fn recording_encoder() -> RecordingEncoder {
    #[cfg(feature = "vp9-recording")]
    {
        RecordingEncoder::Vp9Prototype
    }
    #[cfg(not(feature = "vp9-recording"))]
    {
        RecordingEncoder::MjpegDiagnostic { jpeg_quality: 85 }
    }
}

impl Recordings {
    fn show(&mut self, theme: i32) -> Result<()> {
        if let Some(window) = self.window.as_ref() {
            window.global::<ClippyTheme>().set_index(theme);
            return Ok(window.show()?);
        }
        let window = RecordingLibrary::new()?;
        let appearance = window.global::<ClippyTheme>();
        appearance.set_index(theme);
        settings::connect_titlebar(&window, RecordingLibrary::invoke_titlebar_contains);
        window.set_audio_options(ModelRc::new(VecModel::from(
            AudioTrack::names()
                .iter()
                .copied()
                .map(Into::into)
                .collect::<Vec<_>>(),
        )));
        window.set_audio_index(self.audio_selection);
        window.on_audio_changed(|index| send(Message::Audio(index)));
        window.on_dismiss(|| send(Message::Close));
        window.on_reload(|| send(Message::Refresh));
        window.on_pause(|| send(Message::Control(Control::PauseResume)));
        window.on_stop(|| send(Message::Control(Control::Stop)));
        window.on_play(|key| send(Message::Play(key.to_string())));
        window.on_export_recording(|key, path| {
            send(Message::Export(key.to_string(), path.to_string()))
        });
        window.on_remove_recording(|key| send(Message::Delete(key.to_string())));
        window.on_merge_recording(|key| send(Message::Merge(key.to_string())));
        window.on_preview_recording(|session| send(Message::Preview(session.to_string())));
        window.window().on_close_requested(|| {
            send(Message::Close);
            CloseRequestResponse::KeepWindowShown
        });
        window.show()?;
        self.window = Some(window);
        self.sync_controls();
        Ok(())
    }

    fn note(&mut self, result: Result<String>) {
        self.status = result.unwrap_or_else(|error| error.to_string());
        self.sync_controls();
    }

    fn sync_controls(&self) {
        if let Some(window) = self.window.as_ref() {
            window.set_recording(self.job.is_some());
            window.set_paused(self.paused);
            window.set_toggle_pending(self.toggle_pending);
            window.set_finishing(self.finishing);
            window.set_status(self.status.clone().into());
        }
        if let Some(control) = self.control.as_ref() {
            control.sync(self.paused, self.toggle_pending, self.finishing);
        }
    }

    fn stop_playback(&mut self) {
        if let Some(Playback { stop, worker }) = self.playback.take() {
            stop.store(true, Ordering::Release);
            worker.thread().unpark();
            let _ = worker.join();
        }
    }

    fn clear_previews(&mut self) {
        self.rows = Rc::new(VecModel::default());
        self.preview_queue.clear();
        self.preview_cached.clear();
    }

    fn store_preview(&mut self, session: String, result: Result<Option<RgbaImage>>) {
        const MAX_PREVIEWS: usize = 8;
        let Some(index) = self.rows.iter().position(|row| row.session == session) else {
            return;
        };
        let mut row = self.rows.row_data(index).expect("现有录屏行");
        row.can_preview = false;
        match result {
            Ok(Some(image)) => {
                if self.preview_cached.len() == MAX_PREVIEWS {
                    let removed = self.preview_cached.pop_front().expect("缩略图缓存已满");
                    if let Some(stale) = self.rows.iter().position(|row| row.session == removed) {
                        let mut entry = self.rows.row_data(stale).expect("现有录屏行");
                        entry.thumbnail = slint::Image::default();
                        entry.can_preview = true;
                        self.rows.set_row_data(stale, entry);
                    }
                }
                row.thumbnail = editor::to_slint(&image);
                self.preview_cached.push_back(session);
            }
            Ok(None) => {}
            Err(error) => eprintln!("Recording thumbnail: {error}"),
        }
        self.rows.set_row_data(index, row);
    }
}

fn get_artifact(root: &Path, key: &str) -> Result<ResolvedRecordingArtifact> {
    let (session, artifact) = key.split_once(':').context("Select a recording")?;
    manifest::resolve_library_artifact(root, session, artifact).map_err(anyhow::Error::msg)
}

fn library_rows(items: Vec<RecordingLibraryItem>) -> Vec<RecordingRow> {
    items
        .into_iter()
        .map(|item| {
            let files = item
                .artifacts
                .into_iter()
                .map(|artifact| RecordingFile {
                    key: format!("{}:{}", item.session_id, artifact.artifact_id).into(),
                    title: artifact.display_name.into(),
                    detail: format!(
                        "{:.1} s · {:.1} MiB",
                        artifact.duration_ms as f64 / 1000.0,
                        artifact.byte_length as f64 / 1024.0 / 1024.0
                    )
                    .into(),
                })
                .collect::<Vec<_>>();
            let state = match item.state {
                "complete" => "Complete",
                "interrupted" => "Interrupted",
                state => state,
            };
            RecordingRow {
                session: item.session_id.into(),
                title: SharedString::from("Screen recording"),
                detail: format!(
                    "{} × {} · {:.1} s · {} frames · {}",
                    item.width,
                    item.height,
                    item.duration_ms as f64 / 1000.0,
                    item.frame_count,
                    item.container.to_ascii_uppercase()
                )
                .into(),
                state: state.into(),
                complete: item.state == "complete",
                can_merge: cfg!(feature = "vp9-recording") && item.can_merge,
                can_preview: cfg!(feature = "vp9-recording") && item.can_thumbnail,
                thumbnail: slint::Image::default(),
                files: ModelRc::new(VecModel::from(files)),
            }
        })
        .collect()
}

impl App {
    pub(super) fn start_recording(&mut self, input: Result<(Region, (u32, u32))>) {
        if self.recordings.job.is_some() || self.recordings.control.is_some() {
            self.complete("A recording is already in progress");
            return;
        }
        self.recordings.stop_playback();
        if let Some(window) = self.recordings.window.take() {
            let _ = window.hide();
        }
        self.recordings.clear_previews();
        let (region, size) = match input {
            Ok(input) => input,
            Err(error) => return self.capture_error(error),
        };
        self.recordings.control = control::View::open(&region, size).unwrap_or_else(|error| {
            eprintln!("Recording controls: {error}");
            None
        });
        if self.recordings.control.is_some() {
            // WM 摆位有异步阶段，在首帧采集前检查实际坐标。
            Timer::single_shot(Duration::from_millis(90), move || {
                send(Message::Begin(region, size))
            });
        } else {
            self.begin_recording(region, size);
        }
    }

    fn begin_recording(&mut self, region: Region, size: (u32, u32)) {
        self.capture_task_id += 1;
        let result = Job::open(
            self.capture_task_id,
            self.paths.data_dir.clone(),
            region,
            size,
            self.recordings.recovered.clone(),
            AudioTrack::from_index(self.recordings.audio_selection),
        );
        match result {
            Ok(job) => {
                self.recordings.job = Some(job);
                self.recordings.note(Ok("Opening recording source…".into()));
            }
            Err(error) => {
                self.recordings.control.take();
                self.capture_error(error.into());
            }
        }
    }

    fn refresh_recordings(&mut self) -> io::Result<()> {
        if self.recordings.window.is_none() {
            return Ok(());
        }
        let root = self.paths.data_dir.clone();
        let recovered = self.recordings.recovered.clone();
        self.recordings.list_generation += 1;
        let generation = self.recordings.list_generation;
        thread::Builder::new()
            .name("clippy-recording-library".into())
            .spawn(move || {
                let result = (|| {
                    recover_once(&root, &recovered)?;
                    manifest::list_library(&root).map_err(anyhow::Error::msg)
                })();
                send(Message::List(generation, result));
            })?;
        Ok(())
    }

    pub(super) fn recording_message(&mut self, message: Message) {
        if let Err(error) = self.handle_recording(message) {
            self.status(error.to_string());
            self.recordings.note(Err(error));
        }
    }

    fn handle_recording(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Show => {
                self.recordings
                    .show(settings::theme_index(&self.config.theme))?;
                self.refresh_recordings()?;
            }
            Message::Refresh => self.refresh_recordings()?,
            Message::Begin(region, size) => {
                if self
                    .recordings
                    .control
                    .as_ref()
                    .is_some_and(|control| !control.arm())
                {
                    self.recordings.control.take();
                    Timer::single_shot(Duration::from_millis(32), move || {
                        send(Message::Begin(region, size))
                    });
                } else {
                    self.begin_recording(region, size);
                }
            }
            Message::HideControl => {
                self.recordings.control.take();
            }
            Message::Audio(index) => self.recordings.audio_selection = index,
            Message::Close => {
                self.recordings.stop_playback();
                if let Some(window) = self.recordings.window.take() {
                    window.hide()?;
                }
                self.recordings.clear_previews();
            }
            Message::Started(id)
                if self.recordings.job.as_ref().is_some_and(|job| job.id == id) =>
            {
                if let Some(control) = self.recordings.control.as_ref() {
                    control.start();
                }
                self.complete(if self.recordings.control.is_some() {
                    "Recording"
                } else {
                    "Recording · use the tray to pause or stop"
                });
                if !self.recordings.finishing {
                    self.recordings.note(Ok("Recording".into()));
                }
            }
            Message::Toggled(id, result)
                if self.recordings.job.as_ref().is_some_and(|job| job.id == id) =>
            {
                self.recordings.toggle_pending = false;
                let result = result.map(|paused| {
                    self.recordings.paused = paused;
                    if paused {
                        "Recording paused"
                    } else {
                        "Recording resumed"
                    }
                    .into()
                });
                if self.recordings.finishing {
                    self.recordings.sync_controls();
                } else {
                    self.recordings.note(result);
                }
            }
            Message::Finished(id, result)
                if self.recordings.job.as_ref().is_some_and(|job| job.id == id) =>
            {
                self.recordings.job.take();
                self.recordings.control.take();
                self.recordings.paused = false;
                self.recordings.toggle_pending = false;
                self.recordings.finishing = false;
                self.set_busy(false);
                self.recordings.note(result);
                self.status(self.recordings.status.clone());
                self.refresh_recordings()?;
            }
            Message::Update(result) => self.recordings.note(result),
            Message::Mutation(result) => {
                self.recordings.editing_library = false;
                self.recordings.note(result);
                self.refresh_recordings()?;
            }
            Message::Control(control) => {
                if self.recordings.finishing
                    || (matches!(control, Control::PauseResume) && self.recordings.toggle_pending)
                {
                    return Ok(());
                }
                let job = self
                    .recordings
                    .job
                    .as_ref()
                    .context("No recording is in progress")?;
                job.control
                    .as_ref()
                    .expect("录屏控制器处于运行状态")
                    .try_send(control)?;
                match control {
                    Control::PauseResume => {
                        self.recordings.toggle_pending = true;
                        self.recordings.sync_controls();
                    }
                    Control::Stop => {
                        job.cancellation.cancel();
                        self.recordings.finishing = true;
                        self.recordings.note(Ok("Finalizing recording…".into()));
                    }
                }
            }
            Message::List(generation, result) if generation == self.recordings.list_generation => {
                if self.recordings.window.is_some() {
                    self.recordings.clear_previews();
                    self.recordings.rows = Rc::new(VecModel::from(library_rows(result?)));
                    let window = self.recordings.window.as_ref().expect("录屏库仍打开");
                    window.set_recordings(ModelRc::from(self.recordings.rows.clone()));
                    window.set_loaded(true);
                }
            }
            Message::Preview(session) => {
                if self
                    .recordings
                    .rows
                    .iter()
                    .any(|row| row.session == session && row.can_preview)
                    && !self.recordings.preview_queue.contains(&session)
                {
                    if self.recordings.preview_queue.len() == 12 {
                        self.recordings.preview_queue.pop_front();
                    }
                    self.recordings.preview_queue.push_back(session);
                    self.begin_recording_preview()?;
                }
            }
            Message::PreviewReady(generation, session, result) => {
                self.recordings.preview_running = false;
                if generation == self.recordings.list_generation {
                    self.recordings.store_preview(session, result);
                }
                self.begin_recording_preview()?;
            }
            Message::Play(key) => self.play_recording(key)?,
            Message::Export(key, directory) if !self.recordings.editing_library => {
                let root = self.paths.data_dir.clone();
                let destination = if directory.trim().is_empty() {
                    root.join("Exported Recordings")
                } else {
                    expand_user_path(directory.trim())
                };
                self.recordings.editing_library = true;
                if let Err(error) = thread::Builder::new()
                    .name("clippy-recording-export".into())
                    .spawn(move || {
                        let result = (|| {
                            let artifact = get_artifact(&root, &key)?;
                            fs::create_dir_all(&destination)?;
                            let output = destination.join(&artifact.suggested_file_name);
                            manifest::export_new_library_artifact(&artifact, &output)
                                .map_err(anyhow::Error::msg)?;
                            Ok(format!("Exported to {}", output.display()))
                        })();
                        send(Message::Mutation(result));
                    })
                {
                    self.recordings.editing_library = false;
                    return Err(error.into());
                }
            }
            mutation @ (Message::Delete(_) | Message::Merge(_))
                if self.recordings.job.is_none() && !self.recordings.editing_library =>
            {
                let (key, merging) = match mutation {
                    Message::Delete(key) => (key, false),
                    Message::Merge(key) => (key, true),
                    _ => unreachable!(),
                };
                let (session, _) = key.split_once(':').context("Select a recording")?;
                let session = session.to_owned();
                let root = self.paths.data_dir.clone();
                self.recordings.stop_playback();
                self.recordings.editing_library = true;
                if let Err(error) = thread::Builder::new()
                    .name("clippy-recording-library-mutation".into())
                    .spawn(move || {
                        let result = if merging {
                            #[cfg(feature = "vp9-recording")]
                            {
                                manifest::merge_interrupted_vp9_session(&root, &session)
                                    .map(|()| "Recording recovered".into())
                            }
                            #[cfg(not(feature = "vp9-recording"))]
                            {
                                Err("Recording recovery requires VP9 support".into())
                            }
                        } else {
                            manifest::delete_library_session(&root, &session)
                                .map(|()| "Recording deleted".into())
                        };
                        send(Message::Mutation(result.map_err(anyhow::Error::msg)));
                    })
                {
                    self.recordings.editing_library = false;
                    return Err(error.into());
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn begin_recording_preview(&mut self) -> Result<()> {
        if self.recordings.preview_running {
            return Ok(());
        }
        let Some(session) = self.recordings.preview_queue.pop_front() else {
            return Ok(());
        };
        let generation = self.recordings.list_generation;
        let root = self.paths.data_dir.clone();
        thread::Builder::new()
            .name("clippy-recording-thumbnail".into())
            .spawn(move || {
                #[cfg(feature = "vp9-recording")]
                let result = manifest::resolve_library_thumbnail_source(&root, &session)
                    .and_then(|source| {
                        source
                            .as_ref()
                            .map(clippy_media::recording::thumbnail::decode_thumbnail)
                            .transpose()
                    })
                    .map_err(anyhow::Error::msg);
                #[cfg(not(feature = "vp9-recording"))]
                let result = {
                    let _ = root;
                    Ok(None)
                };
                send(Message::PreviewReady(generation, session, result));
            })?;
        self.recordings.preview_running = true;
        Ok(())
    }

    fn play_recording(&mut self, key: String) -> Result<()> {
        self.recordings.stop_playback();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let root = self.paths.data_dir.clone();
        let worker = thread::Builder::new()
            .name("clippy-recording-player".into())
            .spawn(move || {
                let result = (|| {
                    let artifact = get_artifact(&root, &key)?;
                    manifest::verify_library_artifact_with_checkpoint(&artifact, || {
                        if worker_stop.load(Ordering::Acquire) {
                            Err("Playback cancelled".into())
                        } else {
                            Ok(())
                        }
                    })
                    .map_err(anyhow::Error::msg)?;
                    if worker_stop.load(Ordering::Acquire) {
                        return Ok(String::new());
                    }
                    let mut child = Process::new("ffplay")
                        .args(["-autoexit", "-loglevel", "error", "-i"])
                        .arg(&artifact.path)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                        .context("Native playback requires ffplay")?;
                    while !worker_stop.load(Ordering::Acquire) {
                        if let Some(status) = child.try_wait()? {
                            return if status.success() {
                                Ok("Playback finished".into())
                            } else {
                                Err(anyhow!("Native player failed to open this recording"))
                            };
                        }
                        thread::park_timeout(Duration::from_millis(100));
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    Ok(String::new())
                })();
                if !worker_stop.load(Ordering::Acquire) {
                    send(Message::Update(result));
                }
            })?;
        self.recordings.playback = Some(Playback { stop, worker });
        self.recordings.note(Ok("Opening native player…".into()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_recordings_waits_for_playback_cleanup() {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (cleaned, result) = mpsc::channel();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                thread::park();
            }
            cleaned.send(()).unwrap();
        });
        let mut recordings = Recordings::default();
        let _ = recordings.playback.insert(Playback { stop, worker });
        drop(recordings);
        assert_eq!(result.try_recv(), Ok(()));
    }
}
