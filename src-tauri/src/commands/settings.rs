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

/// 更新应用配置并持久化到磁盘。
#[tauri::command]
pub fn update_config(
    new_config: AppConfig,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    let global_changed = config.global_shortcut != new_config.global_shortcut;
    let pin_changed = config.pin_shortcut != new_config.pin_shortcut;
    let capture_changed = config.capture_shortcut != new_config.capture_shortcut;
    *config = new_config;
    save_config(&state.config_path, &config);
    let _ = app_handle.emit("config-changed", &*config);

    // 保存后的注册失败必须回到界面上，否则设置页显示新键位而系统里根本没绑上。
    if global_changed || pin_changed || capture_changed {
        if crate::gsettings_shortcuts::is_wayland() {
            if global_changed {
                crate::record_register_result(
                    &app_handle,
                    &["global"],
                    &config.global_shortcut,
                    true,
                    crate::gsettings_shortcuts::update_binding(&config.global_shortcut),
                );
            }
            if pin_changed {
                crate::record_register_result(
                    &app_handle,
                    &["pin"],
                    &config.pin_shortcut,
                    true,
                    crate::gsettings_shortcuts::update_pin_binding(&config.pin_shortcut),
                );
            }
            if capture_changed {
                crate::record_register_result(
                    &app_handle,
                    &["capture"],
                    &config.capture_shortcut,
                    true,
                    crate::gsettings_shortcuts::update_capture_binding(&config.capture_shortcut),
                );
            }
        } else if let Err(error) = crate::register_x11_shortcuts(&app_handle, &config) {
            // 逐个动作的失败已在注册内部记账，这里只记录整体失败
            log::warn!("X11 快捷键全部注册失败: {error}");
        }
    }
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
/// GNOME/Wayland 下枚举 gsettings 里已声明的绑定做精确比较；X11 下只能看到
/// Clippy 自己的注册（X 服务器不提供他人 grab 的枚举），此时结果的
/// `enumerable = false`，前端据此不把"没查到"说成"没有冲突"。
#[tauri::command]
pub fn check_shortcut_conflict(
    shortcut: String,
    app_handle: tauri::AppHandle,
) -> Result<crate::shortcut_conflict::ShortcutConflict, String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let probe = shortcut.clone();
    Ok(crate::shortcut_conflict::detect_with(
        &shortcut,
        crate::gsettings_shortcuts::is_wayland(),
        || app_handle.global_shortcut().is_registered(probe.as_str()),
        crate::shortcut_conflict::scan_gnome_bindings,
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

/// 打开设置窗口，并确保载入最新页面状态。
#[tauri::command]
pub fn show_settings(app_handle: tauri::AppHandle) -> Result<(), String> {
    crate::window_controller::open_settings_window(&app_handle)
}

/// 暂停全局快捷键，供快捷键录制使用。
#[tauri::command]
pub fn pause_shortcuts(app_handle: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    let mut pause_result = Ok(());
    {
        let _transition = state
            .shortcut_transition
            .lock()
            .map_err(|e| e.to_string())?;
        if !state.shortcuts_paused.load(Ordering::Acquire) {
            // 先记录暂停意图：Wayland 由多次 gsettings 写入组成，部分失败时
            // 仍必须让销毁兜底尝试恢复，不能把状态留在“未暂停”。
            state.shortcuts_paused.store(true, Ordering::Release);
            pause_result = pause_shortcuts_for_platform(&app_handle);
        }
    }

    // 原生关闭可能早于 pause command 完成；窗口已销毁时在这里补偿恢复。
    if app_handle.get_webview_window("settings").is_none() {
        resume_shortcuts_for_app(&app_handle, &state)?;
    }
    pause_result
}

fn pause_shortcuts_for_platform(app_handle: &tauri::AppHandle) -> Result<(), String> {
    if crate::gsettings_shortcuts::is_wayland() {
        crate::gsettings_shortcuts::pause().map_err(|e| e.to_string())
    } else {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        app_handle
            .global_shortcut()
            .unregister_all()
            .map_err(|e| e.to_string())
    }
}

/// 幂等恢复全局快捷键，供 IPC 与设置窗口销毁兜底共用。
pub(crate) fn resume_shortcuts_for_app(
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), String> {
    let _transition = state
        .shortcut_transition
        .lock()
        .map_err(|e| e.to_string())?;
    if !claim_shortcuts_resume(&state.shortcuts_paused) {
        return Ok(());
    }

    let config = {
        match state.config.lock() {
            Ok(config) => config.clone(),
            Err(error) => {
                state.shortcuts_paused.store(true, Ordering::Release);
                return Err(error.to_string());
            }
        }
    };

    // 恢复同样要记账：录制结束后没绑回去，用户按键就没反应，而设置页显示得好好的。
    let result = if crate::gsettings_shortcuts::is_wayland() {
        let outcomes = crate::gsettings_shortcuts::resume_with_results(
            &config.global_shortcut,
            &config.pin_shortcut,
            &config.capture_shortcut,
        );
        let failed = outcomes
            .iter()
            .filter(|(_, _, result)| result.is_err())
            .count();
        let mut first_error = None;
        for (action, shortcut, outcome) in outcomes.iter() {
            if let Err(reason) = outcome {
                first_error.get_or_insert_with(|| reason.clone());
            }
            crate::record_register_result(app_handle, &[action], shortcut, true, outcome.clone());
        }
        // 只有全都失败才算"仍处于暂停"，部分成功的键位已经生效，状态不能再说暂停
        match first_error {
            Some(reason) if failed == outcomes.len() => Err(reason),
            Some(reason) => {
                log::warn!("恢复 GNOME 快捷键部分失败: {reason}");
                Ok(())
            }
            None => Ok(()),
        }
    } else {
        crate::register_x11_shortcuts(app_handle, &config)
    };
    if result.is_err() {
        state.shortcuts_paused.store(true, Ordering::Release);
    }
    result
}

fn claim_shortcuts_resume(shortcuts_paused: &std::sync::atomic::AtomicBool) -> bool {
    shortcuts_paused.swap(false, Ordering::AcqRel)
}

/// 恢复全局快捷键。
#[tauri::command]
pub fn resume_shortcuts(
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    resume_shortcuts_for_app(&app_handle, &state)
}

/// 检测安装类型：AppImage 支持自动更新，deb/dev 不支持。
#[tauri::command]
pub fn get_install_type() -> String {
    if std::env::var("APPIMAGE").is_ok() {
        "appimage".into()
    } else {
        "deb".into()
    }
}

/// 当前可执行文件是否位于 cargo target 产物目录。
#[tauri::command]
pub fn is_dev_binary() -> bool {
    match std::env::current_exe() {
        Ok(path) => {
            let path = path.to_string_lossy();
            path.contains("/target/debug/") || path.contains("/target/release/")
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::claim_shortcuts_resume;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn resume_claim_is_idempotent() {
        let paused = AtomicBool::new(true);

        assert!(claim_shortcuts_resume(&paused));
        assert!(!claim_shortcuts_resume(&paused));
        assert!(!paused.load(Ordering::Acquire));
    }
}
