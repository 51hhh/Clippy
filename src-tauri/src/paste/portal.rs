use super::token_store::{read_restore_token, write_restore_token};
use super::PastePhase;
use std::path::Path;
use std::time::Duration;

pub(super) struct PortalState {
    phase: PastePhase,
    session: Option<PortalSession>,
    implicit_attempted: bool,
    detail: Option<String>,
}

impl PortalState {
    pub(super) fn new(phase: PastePhase) -> Self {
        Self {
            phase,
            session: None,
            implicit_attempted: false,
            detail: None,
        }
    }

    pub(super) fn phase(&self) -> PastePhase {
        self.phase
    }

    pub(super) fn can_request_permission(&self) -> bool {
        self.session.is_none()
    }

    pub(super) fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    pub(super) fn allow_explicit_retry(&mut self) {
        self.implicit_attempted = false;
    }

    fn begin_attempt(&mut self, explicit: bool) -> Result<(), String> {
        if self.implicit_attempted && !explicit {
            return Err(self.detail.clone().unwrap_or_else(|| {
                "自动粘贴授权本次运行中已尝试，需在设置中手动重试".to_string()
            }));
        }
        self.implicit_attempted = true;
        self.phase = PastePhase::Initializing;
        self.detail = None;
        Ok(())
    }
}

struct PortalSession {
    proxy: ashpd::desktop::remote_desktop::RemoteDesktop,
    session: ashpd::desktop::Session<ashpd::desktop::remote_desktop::RemoteDesktop>,
}

pub(super) async fn paste(state: &mut PortalState, token_path: &Path) -> Result<(), String> {
    ensure_session(state, token_path, false).await?;
    tokio::time::sleep(Duration::from_millis(120)).await;

    let result = match state.session.as_ref() {
        Some(session) => press_ctrl_v(session).await,
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
    Ok(())
}

pub(super) async fn ensure_session(
    state: &mut PortalState,
    token_path: &Path,
    explicit: bool,
) -> Result<(), String> {
    use ashpd::desktop::remote_desktop::{DeviceType, RemoteDesktop, SelectDevicesOptions};
    use ashpd::desktop::PersistMode;

    if state.session.is_some() {
        state.phase = PastePhase::Ready;
        return Ok(());
    }
    state.begin_attempt(explicit)?;

    let restore_token = read_restore_token(token_path);
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
                if let Err(error) = write_restore_token(token_path, token) {
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
                let _ = std::fs::remove_file(token_path);
            }
            Err(error)
        }
    }
}

async fn press_ctrl_v(session: &PortalSession) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implicit_attempt_runs_once_until_explicit_retry() {
        let mut state = PortalState::new(PastePhase::PermissionRequired);

        assert!(state.begin_attempt(false).is_ok());
        state.detail = Some("denied".to_string());
        assert_eq!(state.begin_attempt(false).unwrap_err(), "denied");
        assert!(state.begin_attempt(true).is_ok());
        assert_eq!(state.phase, PastePhase::Initializing);
    }
}
