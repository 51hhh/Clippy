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

#[derive(Debug, Clone, PartialEq, Eq)]
enum RestoreTokenAction {
    Preserve,
    Remove,
    Replace(String),
}

#[derive(Debug, Clone, Copy)]
struct RestoreTokenAttempt {
    had_token: bool,
    restore_token_attached: bool,
    stage: PortalAuthorizationStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PortalAuthorizationStage {
    PreparingSelectDevices,
    SelectDevicesSubmitted,
    SelectDevicesAccepted,
    StartSubmitted,
    StartAccepted,
}

impl RestoreTokenAttempt {
    fn new(had_token: bool) -> Self {
        Self {
            had_token,
            restore_token_attached: false,
            stage: PortalAuthorizationStage::PreparingSelectDevices,
        }
    }

    fn attach_restore_token(&mut self, attached: bool) {
        self.restore_token_attached = attached;
    }

    fn advance_to(&mut self, stage: PortalAuthorizationStage) {
        debug_assert!(stage >= self.stage);
        self.stage = stage;
    }

    fn old_token_consumed(self) -> bool {
        self.had_token
            && self.restore_token_attached
            && self.stage >= PortalAuthorizationStage::SelectDevicesSubmitted
    }

    fn after_failure(self) -> RestoreTokenAction {
        if self.old_token_consumed() {
            RestoreTokenAction::Remove
        } else {
            RestoreTokenAction::Preserve
        }
    }

    fn after_success(self, next_token: Option<String>) -> RestoreTokenAction {
        match next_token {
            Some(token) => RestoreTokenAction::Replace(token),
            None if self.old_token_consumed() => RestoreTokenAction::Remove,
            None => RestoreTokenAction::Preserve,
        }
    }
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
    let mut token_attempt = RestoreTokenAttempt::new(restore_token.is_some());
    let result = async {
        let proxy = RemoteDesktop::new()
            .await
            .map_err(|error| format!("连接 RemoteDesktop Portal 失败: {error}"))?;
        let version = proxy.version();
        let session = proxy
            .create_session(Default::default())
            .await
            .map_err(|error| format!("创建 RemoteDesktop 会话失败: {error}"))?;
        let setup_result = async {
            let mut options =
                SelectDevicesOptions::default().set_devices(Some(DeviceType::Keyboard.into()));
            let restore_token_submitted = version >= 2 && restore_token.is_some();
            if version >= 2 {
                options = options
                    .set_persist_mode(PersistMode::ExplicitlyRevoked)
                    .set_restore_token(restore_token.as_deref());
            }
            token_attempt.attach_restore_token(restore_token_submitted);
            let select_request = proxy
                .select_devices(&session, options)
                .await
                .map_err(|error| format!("请求键盘控制失败: {error}"))?;
            // request 已成功提交给 Portal，旧 token 从此可能被消费。
            token_attempt.advance_to(PortalAuthorizationStage::SelectDevicesSubmitted);
            select_request
                .response()
                .map_err(|error| format!("键盘控制请求被拒绝: {error}"))?;
            token_attempt.advance_to(PortalAuthorizationStage::SelectDevicesAccepted);

            let start_request = proxy
                .start(&session, None, Default::default())
                .await
                .map_err(|error| format!("启动 RemoteDesktop 会话失败: {error}"))?;
            token_attempt.advance_to(PortalAuthorizationStage::StartSubmitted);
            let selected = start_request
                .response()
                .map_err(|error| format!("RemoteDesktop 授权未通过: {error}"))?;
            token_attempt.advance_to(PortalAuthorizationStage::StartAccepted);
            if !selected.devices().contains(DeviceType::Keyboard) {
                return Err("RemoteDesktop Portal 未授予键盘控制权限".to_string());
            }
            Ok((version >= 2)
                .then(|| selected.restore_token().map(str::to_string))
                .flatten())
        }
        .await;
        match setup_result {
            Ok(next_token) => Ok((PortalSession { proxy, session }, next_token)),
            Err(error) => {
                if let Err(close_error) = session.close().await {
                    log::warn!("关闭未完成的 RemoteDesktop Portal 会话失败: {close_error}");
                }
                Err(error)
            }
        }
    }
    .await;

    match result {
        Ok((session, next_token)) => {
            apply_restore_token_action(token_path, token_attempt.after_success(next_token));
            state.session = Some(session);
            state.phase = PastePhase::Ready;
            state.detail = None;
            Ok(())
        }
        Err(error) => {
            state.phase = PastePhase::Denied;
            state.detail = Some(error.clone());
            apply_restore_token_action(token_path, token_attempt.after_failure());
            Err(error)
        }
    }
}

fn apply_restore_token_action(path: &Path, action: RestoreTokenAction) {
    match action {
        RestoreTokenAction::Preserve => {}
        RestoreTokenAction::Remove => remove_restore_token(path),
        RestoreTokenAction::Replace(token) => {
            if let Err(error) = write_restore_token(path, &token) {
                log::warn!("保存 Portal restore token 失败，删除已消费的旧 token: {error}");
                remove_restore_token(path);
            }
        }
    }
}

fn remove_restore_token(path: &Path) {
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!("删除失效的 Portal restore token 失败: {error}");
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

    fn token_attempt_at(stage: PortalAuthorizationStage) -> RestoreTokenAttempt {
        let mut attempt = RestoreTokenAttempt::new(true);
        attempt.attach_restore_token(true);
        attempt.advance_to(stage);
        attempt
    }

    #[test]
    fn implicit_attempt_runs_once_until_explicit_retry() {
        let mut state = PortalState::new(PastePhase::PermissionRequired);

        assert!(state.begin_attempt(false).is_ok());
        state.detail = Some("denied".to_string());
        assert_eq!(state.begin_attempt(false).unwrap_err(), "denied");
        assert!(state.begin_attempt(true).is_ok());
        assert_eq!(state.phase, PastePhase::Initializing);
    }

    #[test]
    fn select_request_construction_or_send_failure_preserves_restore_token() {
        let attempt = token_attempt_at(PortalAuthorizationStage::PreparingSelectDevices);
        assert_eq!(attempt.after_failure(), RestoreTokenAction::Preserve);
    }

    #[test]
    fn select_devices_response_failure_removes_consumed_restore_token() {
        let attempt = token_attempt_at(PortalAuthorizationStage::SelectDevicesSubmitted);
        assert_eq!(attempt.after_failure(), RestoreTokenAction::Remove);
    }

    #[test]
    fn start_request_failure_removes_consumed_restore_token() {
        let attempt = token_attempt_at(PortalAuthorizationStage::SelectDevicesAccepted);
        assert_eq!(attempt.after_failure(), RestoreTokenAction::Remove);
    }

    #[test]
    fn start_response_failure_removes_consumed_restore_token() {
        let attempt = token_attempt_at(PortalAuthorizationStage::StartSubmitted);
        assert_eq!(attempt.after_failure(), RestoreTokenAction::Remove);
    }

    #[test]
    fn successful_restore_rolls_token_forward() {
        let attempt = token_attempt_at(PortalAuthorizationStage::StartAccepted);
        assert_eq!(
            attempt.after_success(Some("next-token".to_string())),
            RestoreTokenAction::Replace("next-token".to_string())
        );
    }

    #[test]
    fn first_authorization_persists_returned_restore_token() {
        let mut attempt = RestoreTokenAttempt::new(false);
        attempt.advance_to(PortalAuthorizationStage::StartAccepted);
        assert_eq!(
            attempt.after_success(Some("first-token".to_string())),
            RestoreTokenAction::Replace("first-token".to_string())
        );
    }

    #[test]
    fn successful_restore_without_replacement_removes_consumed_token() {
        let attempt = token_attempt_at(PortalAuthorizationStage::StartAccepted);
        assert_eq!(attempt.after_success(None), RestoreTokenAction::Remove);
    }

    #[test]
    fn portal_without_restore_support_preserves_unsubmitted_token() {
        let mut attempt = RestoreTokenAttempt::new(true);
        attempt.attach_restore_token(false);
        attempt.advance_to(PortalAuthorizationStage::StartAccepted);
        assert_eq!(attempt.after_failure(), RestoreTokenAction::Preserve);
        assert_eq!(attempt.after_success(None), RestoreTokenAction::Preserve);
    }
}
