use super::{editor, get_artifact, manifest, App, RecordingLibrary};
use anyhow::{bail, Context, Result};
use clippy_media::recording::playback::{PlaybackUpdate, RecordingPlayer};
use slint::{ComponentHandle, Timer, TimerMode};
use std::{
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Commands {
    paused: bool,
    seek_ms: Option<u64>,
}

#[derive(Default)]
struct Mailbox {
    update: Option<(PlaybackUpdate, bool)>,
    finished: Option<Result<()>>,
}

pub(super) struct Playback {
    stop: Arc<AtomicBool>,
    worker: JoinHandle<()>,
    commands: Arc<Mutex<Commands>>,
    timer: Rc<Timer>,
}

impl Playback {
    fn open(window: &RecordingLibrary, root: PathBuf, key: String) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let commands = Arc::new(Mutex::new(Commands::default()));
        let latest = Arc::new(Mutex::new(Mailbox::default()));
        let worker = {
            let stop = Arc::clone(&stop);
            let commands = Arc::clone(&commands);
            let latest = Arc::clone(&latest);
            thread::Builder::new()
                .name("clippy-recording-player".into())
                .spawn(move || {
                    let result = run(&root, &key, &stop, &commands, &latest);
                    latest.lock().expect("回放状态锁被污染").finished = Some(result);
                })?
        };
        window.set_playback_frame(slint::Image::default());
        window.set_playback_position(0.0);
        window.set_playback_duration(0.0);
        window.set_playback_time("".into());
        window.set_playback_status("Opening recording…".into());
        window.set_playback_paused(false);
        window.set_playback_ready(false);
        window.set_playback_visible(true);
        let timer = Rc::new(Timer::default());
        let weak_window = window.as_weak();
        let weak_timer = Rc::downgrade(&timer);
        timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            let Some(window) = weak_window.upgrade() else {
                return;
            };
            let mut current = {
                let mut latest = latest.lock().expect("回放状态锁被污染");
                std::mem::take(&mut *latest)
            };
            if let Some((update, paused)) = current.update.take() {
                if let Some(frame) = update.frame {
                    window.set_playback_frame(editor::to_slint(&frame));
                    window.set_playback_ready(true);
                }
                if window.get_playback_ready() {
                    window.set_playback_paused(paused);
                    window.set_playback_status(if paused { "Paused" } else { "Playing" }.into());
                }
                if update.duration_ms > 0 {
                    window.set_playback_duration(update.duration_ms as f32);
                    window.set_playback_position(update.position_ms.min(update.duration_ms) as f32);
                    window.set_playback_time(
                        format!(
                            "{} / {}",
                            time_label(update.position_ms),
                            time_label(update.duration_ms)
                        )
                        .into(),
                    );
                }
            }
            if let Some(finished) = current.finished {
                window.set_playback_ready(false);
                let text = match finished {
                    Ok(()) => "Playback finished".into(),
                    Err(error) => format!("Could not play recording: {error:#}"),
                };
                window.set_playback_status(text.into());
                if let Some(timer) = weak_timer.upgrade() {
                    timer.stop();
                }
            }
        });
        Ok(Self {
            stop,
            worker,
            commands,
            timer,
        })
    }

    pub(super) fn toggle(&self) {
        let mut commands = self.commands.lock().expect("回放控制锁被污染");
        commands.paused = !commands.paused;
    }

    pub(super) fn seek(&self, millis: u64) {
        self.commands.lock().expect("回放控制锁被污染").seek_ms = Some(millis);
    }

    pub(super) fn stop(self) {
        self.timer.stop();
        self.stop.store(true, Ordering::Release);
        let _ = self.worker.join();
    }
}

fn run(
    root: &Path,
    key: &str,
    stop: &AtomicBool,
    commands: &Mutex<Commands>,
    latest: &Mutex<Mailbox>,
) -> Result<()> {
    let artifact = get_artifact(root, key)?;
    manifest::verify_library_artifact_with_checkpoint(&artifact, || {
        if stop.load(Ordering::Acquire) {
            Err("Playback cancelled".into())
        } else {
            Ok(())
        }
    })
    .map_err(anyhow::Error::msg)?;
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut player = RecordingPlayer::open(&artifact.path).map_err(anyhow::Error::msg)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut started = false;
    let mut duration_ms: u64 = 0;
    while !stop.load(Ordering::Acquire) {
        let (paused, seek_ms) = {
            let mut commands = commands.lock().expect("回放控制锁被污染");
            (commands.paused, commands.seek_ms.take())
        };
        player.set_paused(paused).map_err(anyhow::Error::msg)?;
        if let Some(millis) = seek_ms {
            player
                .seek(millis.min(duration_ms.saturating_sub(1)))
                .map_err(anyhow::Error::msg)?;
        }
        let mut update = player.poll().map_err(anyhow::Error::msg)?;
        started |= update.frame.is_some();
        if !started && (update.ended || Instant::now() >= deadline) {
            bail!("No video frame arrived");
        }
        let ended = update.ended;
        if update.duration_ms > 0 {
            duration_ms = update.duration_ms;
        }
        {
            let mut latest = latest.lock().expect("回放状态锁被污染");
            if seek_ms.is_none() && update.frame.is_none() {
                update.frame = latest.update.take().and_then(|(old, _)| old.frame);
            }
            latest.update = Some((update, paused));
        }
        if ended {
            break;
        }
    }
    Ok(())
}

fn time_label(millis: u64) -> String {
    let seconds = millis / 1000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

impl App {
    pub(super) fn play_recording(&mut self, key: String) -> Result<()> {
        self.recordings.stop_playback();
        let window = self
            .recordings
            .window
            .as_ref()
            .context("Open the recording library")?;
        self.recordings.playback = Some(Playback::open(window, self.paths.data_dir.clone(), key)?);
        Ok(())
    }
}
