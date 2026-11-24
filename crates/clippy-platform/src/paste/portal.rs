use super::Completion;
use crate::private_files::{is_private, replace_private_file, write_private};
use anyhow::{anyhow, bail, Result};
use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::{PersistMode, Session};
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Copy)]
pub(super) enum Operation {
    Authorize,
    Paste,
}

pub(super) struct Worker {
    commands: mpsc::Sender<(Operation, Completion)>,
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
        let thread = thread::Builder::new()
            .name("clippy-paste-portal".into())
            .spawn(move || runtime.block_on(run(path, receiver, stopped)))?;
        Ok(Self {
            commands,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    pub(super) fn dispatch(&self, operation: Operation, complete: Completion) {
        if let Err(error) = self.commands.try_send((operation, complete)) {
            let (_, complete) = error.into_inner();
            complete(Err(anyhow!(
                "The paste permission service is busy or closed"
            )));
        }
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

type Active = (RemoteDesktop, Session<RemoteDesktop>);

async fn run(
    path: PathBuf,
    mut commands: mpsc::Receiver<(Operation, Completion)>,
    mut stop: oneshot::Receiver<()>,
) {
    let mut session: Option<Active> = None;
    let mut attempted_restore = false;
    loop {
        let (operation, complete) = tokio::select! {
            _ = &mut stop => break,
            Some(command) = commands.recv() => command,
        };
        let result = tokio::select! {
            _ = &mut stop => break,
            result = execute(&mut session, &mut attempted_restore, &path, operation) => result,
        };
        if result.is_err() {
            close(&mut session).await;
        }
        complete(result);
    }
    close(&mut session).await;
}

async fn execute(
    session: &mut Option<Active>,
    attempted_restore: &mut bool,
    path: &Path,
    operation: Operation,
) -> Result<()> {
    if session.is_none() {
        let token = read_token(path);
        if matches!(operation, Operation::Paste) && (*attempted_restore || token.is_none()) {
            bail!("Enable Wayland paste permission in Clippy first");
        }
        *attempted_restore = true;
        let proxy = RemoteDesktop::new().await?;
        let active = proxy.create_session(Default::default()).await?;
        *session = Some((proxy, active));
        let (proxy, active) = session.as_ref().expect("New Portal session");
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
        proxy.select_devices(active, selection).await?.response()?;
        let granted = proxy
            .start(active, None, Default::default())
            .await?
            .response()?;
        if !granted.devices().contains(DeviceType::Keyboard) {
            bail!("The desktop did not grant keyboard control");
        }
        if let Some(next) = granted.restore_token().filter(|value| valid_token(value)) {
            let temporary = path.with_extension("tmp");
            write_private(&temporary, next.as_bytes())?;
            replace_private_file(&temporary, path)?;
        }
    }
    if matches!(operation, Operation::Paste) {
        let (proxy, active) = session.as_ref().expect("Authorized Portal session");
        tokio::time::sleep(Duration::from_millis(120)).await;
        let key = |key, state| proxy.notify_keyboard_keysym(active, key, state, Default::default());
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

async fn close(session: &mut Option<Active>) {
    if let Some((_, session)) = session.take() {
        let _ = tokio::time::timeout(Duration::from_secs(1), session.close()).await;
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
