use super::AppState;
use crate::config::save_config;
use crate::models::AppConfig;
use std::sync::atomic::Ordering;
use tauri::{Emitter, Manager, State};

/// 切换预览面板并调整主窗口宽度。
#[tauri::command]
pub fn set_preview_visible(
    visible: bool,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let _transition = state
        .main_window_transition
        .lock()
        .map_err(|error| format!("主窗口面板切换状态损坏: {error}"))?;
    let previous = *state
        .preview_visible
        .lock()
        .map_err(|error| format!("读取预览面板状态失败: {error}"))?;
    *state
        .preview_visible
        .lock()
        .map_err(|error| format!("更新预览面板状态失败: {error}"))? = visible;
    if let Err(error) = crate::window_controller::resize_main_window(&app_handle) {
        match state.preview_visible.lock() {
            Ok(mut current) => *current = previous,
            Err(restore_error) => {
                log::error!("恢复预览面板状态失败，保留 resize 原错误: {restore_error}");
            }
        }
        if let Err(compensation_error) = crate::window_controller::resize_main_window(&app_handle) {
            log::error!("恢复预览面板几何失败，保留 resize 原错误: {compensation_error}");
        }
        return Err(error);
    }
    Ok(())
}

/// 切换编解码面板并调整主窗口宽度。
#[tauri::command]
pub fn set_codec_visible(
    visible: bool,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let _transition = state
        .main_window_transition
        .lock()
        .map_err(|error| format!("主窗口面板切换状态损坏: {error}"))?;
    let previous = *state
        .codec_visible
        .lock()
        .map_err(|error| format!("读取编解码面板状态失败: {error}"))?;
    *state
        .codec_visible
        .lock()
        .map_err(|error| format!("更新编解码面板状态失败: {error}"))? = visible;
    if let Err(error) = crate::window_controller::resize_main_window(&app_handle) {
        match state.codec_visible.lock() {
            Ok(mut current) => *current = previous,
            Err(restore_error) => {
                log::error!("恢复编解码面板状态失败，保留 resize 原错误: {restore_error}");
            }
        }
        if let Err(compensation_error) = crate::window_controller::resize_main_window(&app_handle) {
            log::error!("恢复编解码面板几何失败，保留 resize 原错误: {compensation_error}");
        }
        return Err(error);
    }
    Ok(())
}

/// 读取当前应用配置。
#[tauri::command]
pub fn get_config(state: State<AppState>) -> Result<AppConfig, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    Ok(config.clone())
}

#[derive(Debug, serde::Serialize)]
pub struct ConfigUpdateOutcome {
    pub shortcut_status: &'static str,
}

/// GNOME 命令始终离开主线程；原生插件只在主线程执行，不带锁跨线程等待。
async fn run_shortcut_operation<T: Send + 'static>(
    app_handle: tauri::AppHandle,
    operation: impl FnOnce(&tauri::AppHandle) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    #[cfg(target_os = "linux")]
    if crate::platform::uses_gnome_shortcuts() {
        return run_blocking_shortcut_work(move || operation(&app_handle)).await;
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let handle = app_handle.clone();
    app_handle
        .run_on_main_thread(move || {
            let _ = sender.send(operation(&handle));
        })
        .map_err(|error| error.to_string())?;
    receiver.await.map_err(|error| error.to_string())?
}

#[cfg(any(test, target_os = "linux"))]
async fn run_blocking_shortcut_work<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("快捷键工作线程异常: {error}"))?
}

/// 更新配置：同步平台只有全部绑定生效才成功；Portal 明确返回等待系统确认。
#[tauri::command]
pub async fn update_config(
    new_config: AppConfig,
    require_shortcuts_active: Option<bool>,
    app_handle: tauri::AppHandle,
) -> Result<ConfigUpdateOutcome, String> {
    run_shortcut_operation(app_handle, move |app| {
        update_config_for_app(new_config, require_shortcuts_active.unwrap_or(false), app)
    })
    .await
}

fn preserve_runtime_fields(next: &mut AppConfig, current: &AppConfig) {
    next.main_window_position = current.main_window_position;
    next.capture_probe_hint_shown = current.capture_probe_hint_shown;
    next.tmux_capture = current.tmux_capture;
}

fn persist_settings_snapshot(state: &AppState, candidate: &AppConfig) -> Result<(), String> {
    let current = state.config.lock().map_err(|error| error.to_string())?;
    let mut merged = candidate.clone();
    preserve_runtime_fields(&mut merged, &current);
    save_config(&state.config_path, &merged).map_err(|error| format!("配置保存失败: {error}"))
}

fn update_config_for_app(
    mut new_config: AppConfig,
    require_shortcuts_active: bool,
    app_handle: &tauri::AppHandle,
) -> Result<ConfigUpdateOutcome, String> {
    let state = app_handle.state::<AppState>();
    let _transition = state
        .shortcut_transition
        .lock()
        .map_err(|e| e.to_string())?;
    let mut config = state.config.lock().map_err(|e| e.to_string())?.clone();
    let previous = config.clone();
    // 外部命令可能等数秒；不持有 config 锁，主窗口查询与后台位置记录仍可运行。
    preserve_runtime_fields(&mut new_config, &config);
    let shortcuts_changed = config.global_shortcut != new_config.global_shortcut
        || config.pin_shortcut != new_config.pin_shortcut
        || config.capture_shortcut != new_config.capture_shortcut;
    validate_shortcut_save(
        state.shortcuts_paused.load(Ordering::Acquire),
        shortcuts_changed,
        require_shortcuts_active,
    )?;
    let shortcut_status = commit_user_config_change(
        &mut config,
        new_config,
        shortcuts_changed || require_shortcuts_active,
        |value| persist_settings_snapshot(&state, value),
        |value| {
            if shortcuts_changed {
                apply_settings_shortcuts(app_handle, &state, value)
            } else {
                Ok("unchanged")
            }
        },
    )?;
    // 外部命令期间后台可能更新窗口位置或 tmux。最终落盘重新合并，不能回写旧字段。
    let mut current = state.config.lock().map_err(|error| error.to_string())?;
    preserve_runtime_fields(&mut config, &current);
    if let Err(error) = save_config(&state.config_path, &config) {
        drop(current);
        let disk = persist_settings_snapshot(&state, &previous).err();
        let effects = if shortcuts_changed {
            apply_settings_shortcuts(app_handle, &state, &previous).err()
        } else {
            None
        };
        return Err(format!(
            "配置最终保存失败: {error}; 配置恢复: {disk:?}; 快捷键恢复: {effects:?}"
        ));
    }
    *current = config;
    let emitted = current.clone();
    drop(current);
    if let Err(error) = app_handle.emit("config-changed", &emitted) {
        log::warn!("配置已保存，但变更通知发送失败: {error}");
    }
    Ok(ConfigUpdateOutcome { shortcut_status })
}

fn commit_user_config_change<T>(
    current: &mut AppConfig,
    next: AppConfig,
    validate_shortcuts: bool,
    persist: impl FnMut(&AppConfig) -> Result<(), String>,
    apply: impl FnMut(&AppConfig) -> Result<T, String>,
) -> Result<T, String> {
    if validate_shortcuts {
        validate_user_shortcuts(&next)?;
    }
    crate::config::commit_config_change(current, next, persist, apply)
}

/// 用户保存前按原生解析身份判重；旧配置的启动容错注册不受影响。
fn validate_user_shortcuts(config: &AppConfig) -> Result<(), String> {
    use std::str::FromStr;
    let mut seen = std::collections::HashMap::new();
    for (action, raw) in [
        ("global", &config.global_shortcut),
        ("pin", &config.pin_shortcut),
        ("capture", &config.capture_shortcut),
    ] {
        if raw.trim().is_empty() {
            continue;
        }
        let shortcut = tauri_plugin_global_shortcut::Shortcut::from_str(raw.trim())
            .map_err(|error| format!("快捷键格式无效: {error}"))?;
        if let Some(previous) = seen.insert(shortcut.id(), action) {
            return Err(format!("settings.shortcut.duplicate:{previous},{action}"));
        }
    }
    Ok(())
}

fn apply_settings_shortcuts(
    app: &tauri::AppHandle,
    state: &AppState,
    config: &AppConfig,
) -> Result<&'static str, String> {
    #[cfg(target_os = "linux")]
    if crate::platform::uses_gnome_shortcuts() {
        let outcomes = crate::gsettings_shortcuts::update_bindings_confirmed(
            &config.global_shortcut,
            &config.pin_shortcut,
            &config.capture_shortcut,
        );
        return record_gnome_results(app, outcomes).map(|()| "applied");
    }
    #[cfg(target_os = "linux")]
    if crate::platform::uses_portal_shortcuts() {
        state
            .portal_shortcuts
            .as_ref()
            .ok_or_else(|| "GlobalShortcuts Portal manager 未初始化".to_string())?
            .activate(config.clone())?;
        return Ok("pending");
    }
    crate::register_tauri_shortcuts(app, config)?;
    // 启动时允许部分注册成功；用户保存必须将任何一个失败反馈为失败并回滚。
    let failures = state.shortcut_failures.lock().map_err(|e| e.to_string())?;
    match failures.first() {
        Some(failure) => Err(format!(
            "{} 快捷键未生效: {}",
            failure.action, failure.reason
        )),
        None => Ok("applied"),
    }
}

#[cfg(target_os = "linux")]
fn record_gnome_results(
    app: &tauri::AppHandle,
    outcomes: Vec<(&'static str, String, Result<(), String>)>,
) -> Result<(), String> {
    let error = outcomes
        .iter()
        .find_map(|(_, _, result)| result.as_ref().err().cloned());
    for (action, shortcut, result) in outcomes {
        crate::record_register_result(
            app,
            &[action],
            &shortcut,
            crate::platform::DesktopSession::Wayland,
            result,
        );
    }
    error.map_or(Ok(()), Err)
}

/// 用户明确点击更新完成后的重启入口时才调用；从不自动重启应用。
#[tauri::command]
pub fn restart_app(app_handle: tauri::AppHandle) -> Result<(), String> {
    app_handle
        .state::<std::sync::Arc<crate::app_update::AppUpdater>>()
        .ensure_restart_allowed()?;
    app_handle.request_restart();
    Ok(())
}

/// 让用户选择截图保存目录，返回选中的绝对路径；取消返回 None。
/// 只回传路径，是否写进配置由设置页的保存动作决定。
#[tauri::command]
pub async fn pick_screenshot_directory(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let start = state.save_target().directory;
    // 对话框阻塞到用户操作完，必须离开 IPC 的 async 线程。
    tauri::async_runtime::spawn_blocking(move || {
        crate::dialogs::choose_directory(&app_handle, &start)
            .map(|path| path.to_string_lossy().to_string())
    })
    .await
    .map_err(|error| format!("目录选择线程异常: {error}"))
}

/// 检查指定快捷键是否已被桌面或本应用占用。
///
/// GNOME/Wayland 下枚举 gsettings 里已声明的绑定做精确比较；Tauri 原生后端只能看到
/// Clippy 自己的注册（系统不提供他人全局注册的通用枚举），此时结果的
/// `enumerable = false`，前端据此不把"没查到"说成"没有冲突"。
#[tauri::command]
pub fn check_shortcut_conflict(
    shortcut: String,
    app_handle: tauri::AppHandle,
) -> Result<crate::shortcut_conflict::ShortcutConflict, String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let probe = shortcut.clone();
    let gnome_wayland = crate::platform::uses_gnome_shortcuts();
    Ok(crate::shortcut_conflict::detect_with(
        &shortcut,
        crate::platform::is_wayland(),
        || app_handle.global_shortcut().is_registered(probe.as_str()),
        || {
            if gnome_wayland {
                crate::shortcut_conflict::scan_gnome_bindings()
            } else {
                None
            }
        },
    ))
}

/// 读取快捷键注册失败记录。启动阶段的失败早于前端监听，设置页打开时必须能主动查。
#[tauri::command]
pub fn get_shortcut_failures(
    state: State<AppState>,
) -> Result<Vec<crate::app::shortcuts::ShortcutRegisterFailure>, String> {
    let failures = state
        .shortcut_failures
        .lock()
        .map_err(|error| format!("读取快捷键失败记录失败: {error}"))?;
    Ok(failures.clone())
}

/// 打开或聚焦已有设置窗口，保留未保存的表单。
#[tauri::command]
pub fn show_settings(app_handle: tauri::AppHandle) -> Result<(), String> {
    crate::window_controller::open_settings_window(&app_handle)
}

/// 即时非快捷键项允许在录制期间保存；显式 Save 和键位变化必须先完成恢复。
fn validate_shortcut_save(paused: bool, changed: bool, required: bool) -> Result<(), String> {
    if paused && (changed || required) {
        Err("请先恢复快捷键再保存设置".to_string())
    } else {
        Ok(())
    }
}

/// 暂停全局快捷键，供快捷键录制使用。
#[tauri::command]
pub async fn pause_shortcuts(app_handle: tauri::AppHandle) -> Result<(), String> {
    run_shortcut_operation(app_handle, pause_shortcuts_for_app).await
}

fn pause_shortcuts_for_app(app_handle: &tauri::AppHandle) -> Result<(), String> {
    let state = app_handle.state::<AppState>();
    let mut result = Ok(());
    {
        let _transition = state
            .shortcut_transition
            .try_lock()
            .map_err(|error| format!("快捷键正在更新，请稍后重试: {error}"))?;
        if !state.shortcuts_paused.load(Ordering::Acquire) {
            // 暂停也可能只成功了一部分；恢复成功前一直保留暂停意图。
            state.shortcuts_paused.store(true, Ordering::Release);
            result = pause_shortcuts_for_platform(app_handle, &state);
        }
    }
    if app_handle.get_webview_window("settings").is_none() {
        resume_shortcuts_now(app_handle)?;
    }
    result
}

fn pause_shortcuts_for_platform(app: &tauri::AppHandle, _state: &AppState) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if crate::platform::uses_gnome_shortcuts() {
        return record_gnome_results(
            app,
            crate::gsettings_shortcuts::update_bindings_confirmed("", "", ""),
        );
    }
    #[cfg(target_os = "linux")]
    if crate::platform::uses_portal_shortcuts() {
        return _state
            .portal_shortcuts
            .as_ref()
            .ok_or_else(|| "GlobalShortcuts Portal manager 未初始化".to_string())?
            .pause();
    }
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    app.global_shortcut()
        .unregister_all()
        .map_err(|error| error.to_string())
}

/// 只有每项绑定和刷新都成功，才清除待恢复状态；失败仍可再次调用。
fn resume_shortcuts_with(
    paused: &std::sync::atomic::AtomicBool,
    snapshot: impl FnOnce() -> Result<AppConfig, String>,
    apply: impl FnOnce(&AppConfig) -> Result<&'static str, String>,
) -> Result<ConfigUpdateOutcome, String> {
    if !paused.load(Ordering::Acquire) {
        return Ok(ConfigUpdateOutcome {
            shortcut_status: "unchanged",
        });
    }
    let config = snapshot()?;
    let shortcut_status = apply(&config)?;
    paused.store(false, Ordering::Release);
    Ok(ConfigUpdateOutcome { shortcut_status })
}

fn resume_shortcuts_now(app: &tauri::AppHandle) -> Result<ConfigUpdateOutcome, String> {
    resume_shortcuts_guarded(app, false)
}

fn resume_shortcuts_guarded(
    app: &tauri::AppHandle,
    after_destroy: bool,
) -> Result<ConfigUpdateOutcome, String> {
    let state = app.state::<AppState>();
    let _transition = state
        .shortcut_transition
        .lock()
        .map_err(|error| error.to_string())?;
    // 获取转换锁后再检查；旧 Destroyed 不能恢复新窗口刚暂停的录制会话。
    restore_for_settings_window(
        after_destroy,
        app.get_webview_window("settings").is_some(),
        || {
            resume_shortcuts_with(
                &state.shortcuts_paused,
                || {
                    state
                        .config
                        .lock()
                        .map(|config| config.clone())
                        .map_err(|error| error.to_string())
                },
                |config| apply_settings_shortcuts(app, &state, config),
            )
        },
    )
}

fn restore_for_settings_window(
    after_destroy: bool,
    settings_present: bool,
    restore: impl FnOnce() -> Result<ConfigUpdateOutcome, String>,
) -> Result<ConfigUpdateOutcome, String> {
    if after_destroy && settings_present {
        Ok(ConfigUpdateOutcome {
            shortcut_status: "unchanged",
        })
    } else {
        restore()
    }
}

pub(crate) async fn restore_shortcuts_after_settings_destroyed(
    app: tauri::AppHandle,
) -> Result<ConfigUpdateOutcome, String> {
    run_shortcut_operation(app, |app| resume_shortcuts_guarded(app, true)).await
}

/// 录制结束、原生销毁兜底共用同一调度与恢复合同。
pub(crate) async fn resume_shortcuts_for_app(
    app: tauri::AppHandle,
) -> Result<ConfigUpdateOutcome, String> {
    run_shortcut_operation(app, resume_shortcuts_now).await
}

#[tauri::command]
pub async fn resume_shortcuts(app_handle: tauri::AppHandle) -> Result<ConfigUpdateOutcome, String> {
    resume_shortcuts_for_app(app_handle).await
}

/// Save/Cancel/原生关闭都必须先恢复；失败保留窗口及表单以便重试。
#[tauri::command]
pub async fn close_settings(app_handle: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("settings") {
        close_settings_window(window.as_ref().window()).await?;
    }
    Ok(())
}

/// 原生关闭直接调用，不依赖前端 bootstrap；只销毁发起请求时的窗口句柄。
pub(crate) async fn close_settings_window(window: tauri::Window) -> Result<(), String> {
    let app = window.app_handle().clone();
    let state = app.state::<AppState>();
    finish_settings_close_with(
        &state.settings_close_pending,
        resume_shortcuts_for_app(app.clone()),
        || window.destroy().map_err(|error| error.to_string()),
    )
    .await
}

async fn finish_settings_close_with(
    pending: &std::sync::atomic::AtomicBool,
    restore: impl std::future::Future<Output = Result<ConfigUpdateOutcome, String>>,
    destroy: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if pending
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok(());
    }
    struct PendingClose<'a>(&'a std::sync::atomic::AtomicBool);
    impl Drop for PendingClose<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    let _pending = PendingClose(pending);
    restore.await?;
    destroy()
}

/// 检测安装类型；Windows/macOS 不再被误报为 deb。
#[tauri::command]
pub fn get_install_type() -> crate::platform::InstallType {
    crate::platform::current_install_type()
}

/// 当前可执行文件是否位于 cargo target 产物目录。
#[tauri::command]
pub fn is_dev_binary() -> bool {
    crate::platform::is_dev_binary()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc, Mutex};

    #[test]
    fn user_save_rejects_normalized_duplicate_shortcuts() {
        let mut config = AppConfig {
            global_shortcut: "Ctrl+Shift+A".into(),
            pin_shortcut: "Shift+Control+A".into(),
            capture_shortcut: String::new(),
            ..AppConfig::default()
        };
        assert_eq!(
            validate_user_shortcuts(&config).unwrap_err(),
            "settings.shortcut.duplicate:global,pin"
        );
        config.pin_shortcut = String::new();
        assert!(validate_user_shortcuts(&config).is_ok());
        config.capture_shortcut = "Control+Shift+A".into();
        assert_eq!(
            validate_user_shortcuts(&config).unwrap_err(),
            "settings.shortcut.duplicate:global,capture"
        );
        config.capture_shortcut = "Ctrl+Shift+S".into();
        assert!(validate_user_shortcuts(&config).is_ok());
    }

    #[test]
    fn duplicate_user_save_never_persists_or_changes_existing_bindings() {
        let mut current = AppConfig::default();
        let before = current.clone();
        let mut duplicate = current.clone();
        duplicate.pin_shortcut = duplicate.global_shortcut.clone();
        let outcome: Result<(), String> = commit_user_config_change(
            &mut current,
            duplicate,
            true,
            |_| panic!("重复键位不得落盘"),
            |_| panic!("重复键位不得解绑或修改现有快捷键"),
        );
        assert_eq!(
            outcome.unwrap_err(),
            "settings.shortcut.duplicate:global,pin"
        );
        assert_eq!(current.global_shortcut, before.global_shortcut);
        assert_eq!(current.pin_shortcut, before.pin_shortcut);
        assert_eq!(current.capture_shortcut, before.capture_shortcut);
    }

    #[test]
    fn explicit_save_and_shortcut_changes_reject_pause_but_immediate_theme_does_not() {
        assert!(validate_shortcut_save(true, false, true).is_err());
        assert!(validate_shortcut_save(true, true, false).is_err());
        assert!(validate_shortcut_save(true, false, false).is_ok());
        assert!(validate_shortcut_save(false, true, true).is_ok());
    }

    #[test]
    fn restore_failure_retains_intent_and_retries_without_holding_config_lock() {
        let paused = AtomicBool::new(true);
        let config = Mutex::new(AppConfig::default());
        for error in ["partial binding failure", "refresh deadline exceeded"] {
            let outcome = resume_shortcuts_with(
                &paused,
                || Ok(config.lock().unwrap().clone()),
                |_| {
                    assert!(config.try_lock().is_ok(), "外部命令期间不能持有 config 锁");
                    Err(error.into())
                },
            );
            assert_eq!(outcome.unwrap_err(), error);
            assert!(paused.load(Ordering::Acquire));
        }
        let outcome = resume_shortcuts_with(
            &paused,
            || Ok(config.lock().unwrap().clone()),
            |_| Ok("pending"),
        )
        .unwrap();
        assert_eq!(outcome.shortcut_status, "pending");
        assert!(!paused.load(Ordering::Acquire));
        assert_eq!(
            resume_shortcuts_with(
                &paused,
                || panic!("幂等恢复不能再次读取"),
                |_| panic!("不能重复注册")
            )
            .unwrap()
            .shortcut_status,
            "unchanged"
        );
    }

    #[test]
    fn bounded_shortcut_work_runs_off_the_caller_and_keeps_config_readable() {
        let caller = std::thread::current().id();
        let config = Arc::new(Mutex::new(AppConfig::default()));
        let outcome = tauri::async_runtime::block_on(run_blocking_shortcut_work(move || {
            assert_ne!(std::thread::current().id(), caller);
            let paused = AtomicBool::new(true);
            resume_shortcuts_with(
                &paused,
                || Ok(config.lock().unwrap().clone()),
                |_| {
                    assert!(config.try_lock().is_ok());
                    Ok("applied")
                },
            )
        }))
        .unwrap();
        assert_eq!(outcome.shortcut_status, "applied");
    }

    #[test]
    fn native_close_without_javascript_restores_then_destroys_and_failure_can_retry() {
        use std::cell::Cell;
        let pending = AtomicBool::new(false);
        let paused = AtomicBool::new(false);
        let destroyed = Cell::new(0);
        // 没有 JS、也没有录制：不注册快捷键，但后端仍有完整关闭出口。
        tauri::async_runtime::block_on(finish_settings_close_with(
            &pending,
            async {
                resume_shortcuts_with(&paused, || panic!("未暂停不读取配置"), |_| panic!("不注册"))
            },
            || {
                destroyed.set(destroyed.get() + 1);
                Ok(())
            },
        ))
        .unwrap();
        assert_eq!(destroyed.get(), 1);

        // 第一次恢复失败不能销毁；第二次原生关闭可重新恢复并完成。
        paused.store(true, Ordering::Release);
        let failed = tauri::async_runtime::block_on(finish_settings_close_with(
            &pending,
            async {
                resume_shortcuts_with(
                    &paused,
                    || Ok(AppConfig::default()),
                    |_| Err("partial restore failure".into()),
                )
            },
            || {
                destroyed.set(destroyed.get() + 1);
                Ok(())
            },
        ));
        assert!(failed.unwrap_err().contains("partial restore failure"));
        assert_eq!(destroyed.get(), 1);
        assert!(paused.load(Ordering::Acquire));
        assert!(!pending.load(Ordering::Acquire));
        tauri::async_runtime::block_on(finish_settings_close_with(
            &pending,
            async {
                resume_shortcuts_with(&paused, || Ok(AppConfig::default()), |_| Ok("applied"))
            },
            || {
                assert!(!paused.load(Ordering::Acquire));
                destroyed.set(destroyed.get() + 1);
                Ok(())
            },
        ))
        .unwrap();
        assert_eq!(destroyed.get(), 2);
        assert!(!pending.load(Ordering::Acquire));
    }

    #[test]
    fn concurrent_native_and_cancel_close_share_one_restore_and_one_destroy() {
        use std::cell::Cell;
        let pending = AtomicBool::new(false);
        let destroys = Cell::new(0);
        tauri::async_runtime::block_on(finish_settings_close_with(
            &pending,
            async {
                finish_settings_close_with(
                    &pending,
                    async { panic!("第二个关闭请求不能再次恢复") },
                    || panic!("第二个关闭请求不能再次销毁"),
                )
                .await?;
                Ok(ConfigUpdateOutcome {
                    shortcut_status: "applied",
                })
            },
            || {
                destroys.set(destroys.get() + 1);
                Ok(())
            },
        ))
        .unwrap();
        assert_eq!(destroys.get(), 1);
        assert!(!pending.load(Ordering::Acquire));
    }

    #[test]
    fn old_destroyed_event_never_restores_replacement_window_recording() {
        let paused = AtomicBool::new(true);
        let skipped = restore_for_settings_window(true, true, || {
            panic!("不能读取配置、清除暂停意图或恢复新窗口的绑定")
        })
        .unwrap();
        assert_eq!(skipped.shortcut_status, "unchanged");
        assert!(paused.load(Ordering::Acquire));
        let restored = restore_for_settings_window(true, false, || {
            resume_shortcuts_with(&paused, || Ok(AppConfig::default()), |_| Ok("applied"))
        })
        .unwrap();
        assert_eq!(restored.shortcut_status, "applied");
        assert!(!paused.load(Ordering::Acquire));
    }
}
