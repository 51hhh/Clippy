use super::types::OverlaySpec;
use tauri::Manager;

pub(super) fn create(app: &tauri::AppHandle, specs: &[OverlaySpec]) -> Result<(), String> {
    for spec in specs {
        let window = tauri::WebviewWindowBuilder::new(
            app,
            &spec.label,
            tauri::WebviewUrl::App(format!("capture-overlay.html?label={}", spec.label).into()),
        )
        .title("")
        .position(spec.x as f64, spec.y as f64)
        .inner_size(spec.width as f64, spec.height as f64)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .resizable(false)
        .focused(false)
        .visible(false)
        .build()
        .map_err(|error| format!("创建截图覆盖层失败: {error}"))?;
        window
            .set_position(tauri::LogicalPosition::new(spec.x as f64, spec.y as f64))
            .map_err(|error| error.to_string())?;
        window
            .set_size(tauri::LogicalSize::new(
                spec.width as f64,
                spec.height as f64,
            ))
            .map_err(|error| error.to_string())?;
    }

    let cursor = app.cursor_position().ok();
    let focused = cursor.and_then(|cursor| {
        specs.iter().find(|spec| {
            cursor.x >= spec.x as f64
                && cursor.x < spec.x as f64 + spec.width as f64
                && cursor.y >= spec.y as f64
                && cursor.y < spec.y as f64 + spec.height as f64
        })
    });
    for spec in specs {
        if let Some(window) = app.get_webview_window(&spec.label) {
            window.show().map_err(|error| error.to_string())?;
        }
    }
    if let Some(spec) = focused.or_else(|| specs.first()) {
        if let Some(window) = app.get_webview_window(&spec.label) {
            let _ = window.set_focus();
        }
    }
    Ok(())
}

pub(super) fn hide_sources(app: &tauri::AppHandle) -> Vec<String> {
    ["main", "capture"]
        .into_iter()
        .filter_map(|label| {
            let window = app.get_webview_window(label)?;
            if window.is_visible().unwrap_or(false) {
                let _ = window.hide();
                Some(label.to_string())
            } else {
                None
            }
        })
        .collect()
}

pub(super) fn restore(app: &tauri::AppHandle, labels: &[String]) {
    for label in labels {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

pub(super) fn close(app: &tauri::AppHandle, labels: &[String]) {
    for label in labels {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.close();
        }
    }
}
