use super::Completion;
use crate::private_files::{is_private, replace_private_file, write_private};
use anyhow::{anyhow, bail, Result};
use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::{PersistMode, Session};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

mod clipboard;
pub(crate) use clipboard::Client as ClipboardClient;

#[derive(Clone, Copy)]
pub(super) enum Operation {
    Authorize,
    AuthorizeClipboard,
    Paste,
}

enum Command {
    Input(Operation, Completion),
    Clipboard(clipboard::Request),
}

pub(super) struct Worker {
    commands: mpsc::Sender<Command>,
    clipboard: ClipboardClient,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub(super) fn start(path: PathBuf) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (commands, receiver) = mpsc::channel(2);
        let (stop, stopped) = oneshot::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let clipboard = ClipboardClient::new(commands.clone(), generation.clone());
        let thread = thread::Builder::new()
            .name("clippy-paste-portal".into())
            .spawn(move || runtime.block_on(run(path, receiver, stopped, generation)))?;
        Ok(Self {
            commands,
            clipboard,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    pub(super) fn dispatch(&self, operation: Operation, complete: Completion) {
        if let Err(error) = self.commands.try_send(Command::Input(operation, complete)) {
            let Command::Input(_, complete) = error.into_inner() else {
                unreachable!()
            };
            complete(Err(anyhow!(
                "The paste permission service is busy or closed"
            )));
        }
    }

    pub(super) fn clipboard(&self) -> Option<ClipboardClient> {
        self.clipboard.generation().map(|_| self.clipboard.clone())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Active {
    remote: RemoteDesktop,
    session: Session<RemoteDesktop>,
    clipboard: Option<clipboard::Binding>,
}

async fn next_clipboard_event(session: &mut Option<Active>) -> clipboard::Event {
    match session
        .as_mut()
        .and_then(|session| session.clipboard.as_mut())
    {
        Some(binding) => binding.next().await,
        None => std::future::pending().await,
    }
}

async fn run(
    path: PathBuf,
    mut commands: mpsc::Receiver<Command>,
    mut stop: oneshot::Receiver<()>,
    generation: Arc<AtomicU64>,
) {
    let mut session: Option<Active> = None;
    let mut attempted_restore = false;
    loop {
        let command = tokio::select! {
            biased;
            _ = &mut stop => break,
            event = next_clipboard_event(&mut session) => {
                if matches!(event, clipboard::Event::Closed) {
                    close(&mut session, &generation).await;
                } else if let Some(active) = session.as_mut() {
                    let binding = active.clipboard.as_mut().expect("Active clipboard event");
                    tokio::select! {
                        _ = &mut stop => break,
                        () = binding.observe(&active.session, event) => (),
                    }
                }
                continue;
            }
            Some(command) = commands.recv() => command,
        };
        match command {
            Command::Input(operation, complete) => {
                let result = tokio::select! {
                    _ = &mut stop => break,
                    result = execute(&mut session, &mut attempted_restore, &path, operation, &generation) => result,
                };
                if result.is_err() {
                    close(&mut session, &generation).await;
                }
                complete(result);
            }
            Command::Clipboard(request) => {
                if let Some(active) = session.as_mut() {
                    if let Some(binding) = &mut active.clipboard {
                        tokio::select! {
                            _ = &mut stop => break,
                            () = binding.command(&active.session, request) => (),
                        }
                        if generation.load(Ordering::Acquire) == 0 {
                            close(&mut session, &generation).await;
                        }
                        continue;
                    }
                }
                request.closed();
            }
        }
    }
    close(&mut session, &generation).await;
}

async fn execute(
    session: &mut Option<Active>,
    attempted_restore: &mut bool,
    path: &Path,
    operation: Operation,
    generation: &Arc<AtomicU64>,
) -> Result<()> {
    let with_clipboard = matches!(operation, Operation::AuthorizeClipboard);
    if with_clipboard
        && session
            .as_ref()
            .is_some_and(|active| active.clipboard.is_none())
    {
        close(session, generation).await;
    }
    if session.is_none() {
        let token = read_token(path);
        if matches!(operation, Operation::Paste) && (*attempted_restore || token.is_none()) {
            bail!("Enable Wayland paste permission in Clippy first");
        }
        *attempted_restore = true;
        // 清除授权会关闭 worker。新会话需要当前 runtime 可以驱动的 D-Bus 连接。
        let connection = zbus::Connection::session().await?;
        let proxy = RemoteDesktop::with_connection(connection.clone()).await?;
        let active = proxy.create_session(Default::default()).await?;
        *session = Some(Active {
            remote: proxy,
            session: active,
            clipboard: None,
        });
        let active = session.as_mut().expect("New Portal session");
        let proxy = &active.remote;
        let mut selection =
            SelectDevicesOptions::default().set_devices(Some(DeviceType::Keyboard.into()));
        if proxy.version() >= 2 {
            selection = selection
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(token.as_deref());
            // 令牌只能用一次。提交过程失败时也由下一次显式授权重新获取。
            if token.is_some() {
                std::fs::remove_file(path)?;
            }
        }
        proxy
            .select_devices(&active.session, selection)
            .await?
            .response()?;
        if with_clipboard {
            active.clipboard = Some(
                clipboard::Binding::prepare(connection, &active.session, generation.clone())
                    .await?,
            );
        }
        let granted = proxy
            .start(&active.session, None, Default::default())
            .await?
            .response()?;
        if !granted.devices().contains(DeviceType::Keyboard) {
            bail!("The desktop did not grant keyboard control");
        }
        if with_clipboard && !granted.is_clipboard_enabled() {
            bail!("The desktop did not grant clipboard access");
        }
        if let Some(next) = granted.restore_token().filter(|value| valid_token(value)) {
            let temporary = path.with_extension("tmp");
            write_private(&temporary, next.as_bytes())?;
            replace_private_file(&temporary, path)?;
        }
        if let Some(binding) = &active.clipboard {
            binding.advance();
        }
    }
    if matches!(operation, Operation::Paste) {
        let active = session.as_ref().expect("Authorized Portal session");
        tokio::time::sleep(Duration::from_millis(120)).await;
        let key = |key, state| {
            active
                .remote
                .notify_keyboard_keysym(&active.session, key, state, Default::default())
        };
        key(0xffe3, KeyState::Pressed).await?;
        let pressed = key(0x76, KeyState::Pressed).await;
        let release_v = key(0x76, KeyState::Released).await;
        let release_control = key(0xffe3, KeyState::Released).await;
        pressed?;
        release_v?;
        release_control?;
    }
    Ok(())
}

async fn close(session: &mut Option<Active>, generation: &AtomicU64) {
    generation.store(0, Ordering::Release);
    if let Some(active) = session.take() {
        drop(active.clipboard);
        let _ = tokio::time::timeout(Duration::from_secs(1), active.session.close()).await;
    }
}

fn read_token(path: &Path) -> Option<String> {
    if !is_private(path) || std::fs::metadata(path).ok()?.len() > 4096 {
        return None;
    }
    std::fs::read_to_string(path)
        .ok()
        .filter(|value| valid_token(value))
}

fn valid_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_whitespace)
}
