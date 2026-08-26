use crate::capture;
use crate::commands::{self, AppState};
use crate::window_controller;
use tauri::Manager;

pub(crate) fn handle(window: &tauri::Window, event: &tauri::WindowEvent) {
    match event {
        tauri::WindowEvent::Moved(position) if window.label() == "main" => {
            window_controller::remember_main_window_position(window, *position);
        }
        tauri::WindowEvent::Moved(position) if window.label().starts_with("pin-") => {
            if let Some(state) = window.app_handle().try_state::<AppState>() {
                crate::pin::remember_pin_window_position(
                    &state.pin_manager,
                    window.label(),
                    *position,
                );
            }
        }
        tauri::WindowEvent::CloseRequested { api, .. }
            if hides_instead_of_closing(window.label()) =>
        {
            api.prevent_close();
            let _ = window.hide();
            if window.label() == "capture" {
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    commands::release_capture_window(window.app_handle(), &state);
                }
            }
        }
        tauri::WindowEvent::Focused(false) if window.label() == "main" => {
            hide_main_after_focus_loss(window.clone());
        }
        tauri::WindowEvent::Destroyed if window.label().starts_with("pin-") => {
            if let Some(state) = window.app_handle().try_state::<AppState>() {
                // A fixed-label clip pin can be recreated before an older
                // Destroyed event arrives. Preserve the replacement entry.
                if window
                    .app_handle()
                    .get_webview_window(window.label())
                    .is_none()
                {
                    state.pin_manager.remove_window(window.label());
                }
            }
        }
        tauri::WindowEvent::Destroyed if window.label().starts_with("capture-overlay-") => {
            if let Some(state) = window.app_handle().try_state::<AppState>() {
                capture::handle_overlay_destroyed(window.app_handle(), &state, window.label());
            }
        }
        tauri::WindowEvent::Destroyed if window.label() == "settings" => {
            if let Some(state) = window.app_handle().try_state::<AppState>() {
                if let Err(error) = commands::resume_shortcuts_for_app(window.app_handle(), &state)
                {
                    log::warn!("设置窗口销毁后恢复全局快捷键失败: {}", error);
                }
            }
        }
        _ => {}
    }
}

fn hides_instead_of_closing(label: &str) -> bool {
    matches!(label, "main" | "capture")
}

fn hide_main_after_focus_loss(window: tauri::Window) {
    let app_handle = window.app_handle().clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if window.is_focused().unwrap_or(true) {
            return;
        }
        if let Some(state) = app_handle.try_state::<AppState>() {
            if state
                .preview_visible
                .lock()
                .is_ok_and(|preview_visible| *preview_visible)
            {
                return;
            }
        }
        let _ = window.hide();
    });
}

#[cfg(test)]
mod tests {
    use super::hides_instead_of_closing;

    #[test]
    fn reusable_windows_hide_instead_of_entering_a_destroy_race() {
        assert!(hides_instead_of_closing("main"));
        assert!(hides_instead_of_closing("capture"));
        assert!(!hides_instead_of_closing("settings"));
        assert!(!hides_instead_of_closing("pin-1"));
    }
}
