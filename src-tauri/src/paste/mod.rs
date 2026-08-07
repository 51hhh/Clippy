use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteBackend {
    X11,
    WaylandPortal,
    CopyOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PastePhase {
    Ready,
    PermissionRequired,
    Initializing,
    Denied,
    Unavailable,
}

#[derive(Debug, Clone, Serialize)]
pub struct PasteStatus {
    pub backend: PasteBackend,
    pub phase: PastePhase,
    pub auto_paste_enabled: bool,
    pub can_request_permission: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PasteOutcome {
    pub copied: bool,
    pub pasted: bool,
    pub backend: PasteBackend,
    pub detail: Option<String>,
}

impl PasteOutcome {
    pub fn copied_only(backend: PasteBackend, detail: impl Into<Option<String>>) -> Self {
        Self {
            copied: true,
            pasted: false,
            backend,
            detail: detail.into(),
        }
    }
}

pub struct PasteManager {
    backend: PasteBackend,
    x11_target: Mutex<Option<u32>>,
    portal: tokio::sync::Mutex<PortalState>,
    token_path: PathBuf,
}

struct PortalState {
    phase: PastePhase,
    session: Option<PortalSession>,
    implicit_attempted: bool,
    detail: Option<String>,
}

struct PortalSession {
    proxy: ashpd::desktop::remote_desktop::RemoteDesktop,
    session: ashpd::desktop::Session<ashpd::desktop::remote_desktop::RemoteDesktop>,
}

impl PasteManager {
    pub fn new(app_data_dir: &Path) -> Self {
        let backend = detect_backend();
        let token_path = app_data_dir.join("remote-desktop-restore-token");
        let phase = match backend {
            PasteBackend::X11 => PastePhase::Ready,
            PasteBackend::WaylandPortal => PastePhase::PermissionRequired,
            PasteBackend::CopyOnly => PastePhase::Unavailable,
        };
        Self {
            backend,
            x11_target: Mutex::new(None),
            portal: tokio::sync::Mutex::new(PortalState {
                phase,
                session: None,
                implicit_attempted: false,
                detail: None,
            }),
            token_path,
        }
    }

    pub fn backend(&self) -> PasteBackend {
        self.backend
    }

    /// 在 Clippy 抢占焦点前记录目标窗口，避免 Ctrl+V 注入到自身。
    pub fn capture_target(&self) {
        if self.backend != PasteBackend::X11 {
            return;
        }
        match x11::active_window() {
            Ok(window) => {
                if let Ok(mut target) = self.x11_target.lock() {
                    *target = Some(window);
                }
                log::debug!("记录 X11 粘贴目标窗口: {window}");
            }
            Err(error) => log::warn!("无法记录 X11 活动窗口: {error}"),
        }
    }

    pub async fn status(&self, auto_paste_enabled: bool) -> PasteStatus {
        match self.backend {
            PasteBackend::X11 => PasteStatus {
                backend: self.backend,
                phase: PastePhase::Ready,
                auto_paste_enabled,
                can_request_permission: false,
                detail: None,
            },
            PasteBackend::CopyOnly => PasteStatus {
                backend: self.backend,
                phase: PastePhase::Unavailable,
                auto_paste_enabled,
                can_request_permission: false,
                detail: Some("No supported input injection backend is available".to_string()),
            },
            PasteBackend::WaylandPortal => {
                let state = self.portal.lock().await;
                PasteStatus {
                    backend: self.backend,
                    phase: state.phase,
                    auto_paste_enabled,
                    can_request_permission: state.session.is_none(),
                    detail: state.detail.clone(),
                }
            }
        }
    }

    pub async fn request_permission(&self) -> Result<PasteStatus, String> {
        if self.backend != PasteBackend::WaylandPortal {
            return Ok(self.status(true).await);
        }

        let mut state = self.portal.lock().await;
        // 用户显式点击授权时允许重新尝试；自动粘贴失败不会自行循环弹窗。
        state.implicit_attempted = false;
        self.ensure_portal_session(&mut state, true).await?;
        drop(state);
        Ok(self.status(true).await)
    }

    pub async fn paste(&self) -> Result<PasteOutcome, String> {
        match self.backend {
            PasteBackend::X11 => {
                let target = self.x11_target.lock().ok().and_then(|target| *target);
                let result = tauri::async_runtime::spawn_blocking(move || {
                    let target = target.ok_or_else(|| "没有可恢复的 X11 目标窗口".to_string())?;
                    x11::activate_and_confirm(target)?;
                    simulate_x11_paste()
                })
                .await
                .map_err(|error| format!("X11 粘贴线程异常: {error}"))?;
                result?;
                Ok(PasteOutcome {
                    copied: true,
                    pasted: true,
                    backend: self.backend,
                    detail: None,
                })
            }
            PasteBackend::WaylandPortal => self.paste_via_portal().await,
            PasteBackend::CopyOnly => Ok(PasteOutcome::copied_only(
                self.backend,
                Some("Automatic paste is unavailable in this session".to_string()),
            )),
        }
    }

    async fn paste_via_portal(&self) -> Result<PasteOutcome, String> {
        let mut state = self.portal.lock().await;
        self.ensure_portal_session(&mut state, false).await?;
        tokio::time::sleep(Duration::from_millis(120)).await;

        let result = match state.session.as_ref() {
            Some(session) => portal_ctrl_v(session).await,
            None => Err("RemoteDesktop Portal 会话未建立".to_string()),
        };
        if let Err(error) = result {
            state.phase = PastePhase::Unavailable;
            state.detail = Some(error.clone());
            if let Some(session) = state.session.take() {
                let _ = session.session.close().await;
            }
            return Err(error);
        }

        Ok(PasteOutcome {
            copied: true,
            pasted: true,
            backend: self.backend,
            detail: None,
        })
    }

    async fn ensure_portal_session(
        &self,
        state: &mut PortalState,
        explicit: bool,
    ) -> Result<(), String> {
        use ashpd::desktop::remote_desktop::{DeviceType, RemoteDesktop, SelectDevicesOptions};
        use ashpd::desktop::PersistMode;

        if state.session.is_some() {
            state.phase = PastePhase::Ready;
            return Ok(());
        }
        if state.implicit_attempted && !explicit {
            return Err(state.detail.clone().unwrap_or_else(|| {
                "自动粘贴授权本次运行中已尝试，需在设置中手动重试".to_string()
            }));
        }
        state.implicit_attempted = true;
        state.phase = PastePhase::Initializing;
        state.detail = None;

        let restore_token = read_restore_token(&self.token_path);
        let result = async {
            let proxy = RemoteDesktop::new()
                .await
                .map_err(|error| format!("连接 RemoteDesktop Portal 失败: {error}"))?;
            let version = proxy.version();
            let session = proxy
                .create_session(Default::default())
                .await
                .map_err(|error| format!("创建 RemoteDesktop 会话失败: {error}"))?;
            let mut options =
                SelectDevicesOptions::default().set_devices(Some(DeviceType::Keyboard.into()));
            if version >= 2 {
                options = options
                    .set_persist_mode(PersistMode::ExplicitlyRevoked)
                    .set_restore_token(restore_token.as_deref());
            }
            proxy
                .select_devices(&session, options)
                .await
                .map_err(|error| format!("请求键盘控制失败: {error}"))?
                .response()
                .map_err(|error| format!("键盘控制请求被拒绝: {error}"))?;

            let selected = proxy
                .start(&session, None, Default::default())
                .await
                .map_err(|error| format!("启动 RemoteDesktop 会话失败: {error}"))?
                .response()
                .map_err(|error| format!("RemoteDesktop 授权未通过: {error}"))?;
            if !selected.devices().contains(DeviceType::Keyboard) {
                let _ = session.close().await;
                return Err("RemoteDesktop Portal 未授予键盘控制权限".to_string());
            }
            if version >= 2 {
                if let Some(token) = selected.restore_token() {
                    if let Err(error) = write_restore_token(&self.token_path, token) {
                        log::warn!("保存 Portal restore token 失败: {error}");
                    }
                }
            }
            Ok(PortalSession { proxy, session })
        }
        .await;

        match result {
            Ok(session) => {
                state.session = Some(session);
                state.phase = PastePhase::Ready;
                state.detail = None;
                Ok(())
            }
            Err(error) => {
                state.phase = PastePhase::Denied;
                state.detail = Some(error.clone());
                if restore_token.is_some() {
                    let _ = std::fs::remove_file(&self.token_path);
                }
                Err(error)
            }
        }
    }
}

fn detect_backend() -> PasteBackend {
    let session_type = std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if session_type == "wayland" || std::env::var_os("WAYLAND_DISPLAY").is_some() {
        PasteBackend::WaylandPortal
    } else if session_type == "x11" || std::env::var_os("DISPLAY").is_some() {
        PasteBackend::X11
    } else {
        PasteBackend::CopyOnly
    }
}

async fn portal_ctrl_v(session: &PortalSession) -> Result<(), String> {
    use ashpd::desktop::remote_desktop::{KeyState, NotifyKeyboardKeysymOptions};

    const CONTROL_L: i32 = 0xffe3;
    const V: i32 = 0x0076;
    session
        .proxy
        .notify_keyboard_keysym(
            &session.session,
            CONTROL_L,
            KeyState::Pressed,
            NotifyKeyboardKeysymOptions::default(),
        )
        .await
        .map_err(|error| format!("按下 Control 失败: {error}"))?;

    let press_v = session
        .proxy
        .notify_keyboard_keysym(
            &session.session,
            V,
            KeyState::Pressed,
            NotifyKeyboardKeysymOptions::default(),
        )
        .await;
    let release_v = session
        .proxy
        .notify_keyboard_keysym(
            &session.session,
            V,
            KeyState::Released,
            NotifyKeyboardKeysymOptions::default(),
        )
        .await;
    let release_control = session
        .proxy
        .notify_keyboard_keysym(
            &session.session,
            CONTROL_L,
            KeyState::Released,
            NotifyKeyboardKeysymOptions::default(),
        )
        .await;

    press_v.map_err(|error| format!("按下 V 失败: {error}"))?;
    release_v.map_err(|error| format!("释放 V 失败: {error}"))?;
    release_control.map_err(|error| format!("释放 Control 失败: {error}"))?;
    Ok(())
}

fn simulate_x11_paste() -> Result<(), String> {
    use enigo::{
        Direction::{Click, Press, Release},
        Enigo, Key, Keyboard, Settings,
    };

    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|error| format!("初始化 enigo 失败: {error}"))?;
    enigo
        .key(Key::Control, Press)
        .map_err(|error| format!("按下 Control 失败: {error}"))?;
    let click = enigo.key(Key::Unicode('v'), Click);
    let release = enigo.key(Key::Control, Release);
    click.map_err(|error| format!("按下 V 失败: {error}"))?;
    release.map_err(|error| format!("释放 Control 失败: {error}"))?;
    Ok(())
}

fn read_restore_token(path: &Path) -> Option<String> {
    let token = std::fs::read_to_string(path).ok()?;
    let token = token.trim();
    if token.is_empty() || token.len() > 4096 {
        return None;
    }
    Some(token.to_string())
}

fn write_restore_token(path: &Path, token: &str) -> Result<(), String> {
    if token.is_empty() || token.len() > 4096 {
        return Err("Portal restore token 长度无效".to_string());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "Portal token 路径没有父目录".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temp = path.with_extension("tmp");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|error| error.to_string())?;
        file.write_all(token.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&temp, token).map_err(|error| error.to_string())?;
    std::fs::rename(temp, path).map_err(|error| error.to_string())
}

mod x11 {
    use std::time::{Duration, Instant};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ClientMessageData, ClientMessageEvent, ConnectionExt, EventMask, Window,
    };
    use x11rb::rust_connection::RustConnection;
    use x11rb::CURRENT_TIME;

    pub fn active_window() -> Result<Window, String> {
        let (connection, screen) = RustConnection::connect(None).map_err(|e| e.to_string())?;
        active_window_on(&connection, screen)
    }

    pub fn activate_and_confirm(target: Window) -> Result<(), String> {
        let (connection, screen) = RustConnection::connect(None).map_err(|e| e.to_string())?;
        let root = connection
            .setup()
            .roots
            .get(screen)
            .ok_or_else(|| "X11 screen 不存在".to_string())?
            .root;
        let active_atom = atom(&connection, b"_NET_ACTIVE_WINDOW")?;
        let message = ClientMessageEvent::new(
            32,
            target,
            active_atom,
            ClientMessageData::from([1, CURRENT_TIME, 0, 0, 0]),
        );
        connection
            .send_event(
                false,
                root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                message,
            )
            .map_err(|e| e.to_string())?;
        connection.flush().map_err(|e| e.to_string())?;

        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if active_window_on(&connection, screen).ok() == Some(target) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err("X11 窗口管理器未恢复原活动窗口，已取消按键注入".to_string())
    }

    fn active_window_on(connection: &RustConnection, screen: usize) -> Result<Window, String> {
        let root = connection
            .setup()
            .roots
            .get(screen)
            .ok_or_else(|| "X11 screen 不存在".to_string())?
            .root;
        let active_atom = atom(connection, b"_NET_ACTIVE_WINDOW")?;
        let reply = connection
            .get_property(false, root, active_atom, AtomEnum::WINDOW, 0, 1)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        reply
            .value32()
            .and_then(|mut values| values.next())
            .filter(|window| *window != 0)
            .ok_or_else(|| "X11 活动窗口为空".to_string())
    }

    fn atom(connection: &RustConnection, name: &[u8]) -> Result<u32, String> {
        let atom = connection
            .intern_atom(false, name)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?
            .atom;
        if atom == 0 {
            Err(format!(
                "X11 atom 不存在: {}",
                String::from_utf8_lossy(name)
            ))
        } else {
            Ok(atom)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_store_round_trip_uses_separate_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portal-token");
        write_restore_token(&path, "token-value").unwrap();
        assert_eq!(read_restore_token(&path).as_deref(), Some("token-value"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o077,
                0
            );
        }
    }

    #[test]
    fn token_store_rejects_empty_and_oversized_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portal-token");
        assert!(write_restore_token(&path, "").is_err());
        assert!(write_restore_token(&path, &"x".repeat(4097)).is_err());
    }
}
