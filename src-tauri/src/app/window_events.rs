use crate::capture;
use crate::commands::{self, AppState};
use tauri::Manager;

pub(crate) fn handle(window: &tauri::Window, event: &tauri::WindowEvent) {
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
            api.prevent_close();
            let _ = window.hide();
        }
        tauri::WindowEvent::Focused(false) if window.label() == "main" => {
            hide_main_after_focus_loss(window.clone());
        }
        tauri::WindowEvent::Destroyed if window.label().starts_with("pin-") => {
            if let Some(state) = window.app_handle().try_state::<AppState>() {
                state.pin_manager.remove_window(window.label());
            }
        }
        tauri::WindowEvent::Destroyed if window.label().starts_with("capture-overlay-") => {
            if let Some(state) = window.app_handle().try_state::<AppState>() {
                capture::handle_overlay_destroyed(window.app_handle(), &state, window.label());
            }
        }
        tauri::WindowEvent::Destroyed if window.label() == "capture" => {
            if let Some(state) = window.app_handle().try_state::<AppState>() {
                commands::clear_latest_capture(&state);
            }
        }
        _ => {}
    }
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
