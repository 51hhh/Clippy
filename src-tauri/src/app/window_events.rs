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
        }
        tauri::WindowEvent::Focused(false) if window.label() == "main" => {
            hide_main_after_focus_loss(window.clone());
        }
        // 置顶的贴图拿到焦点：在置顶层内重新抬到最前。
        //
        // 置顶层里可以同时有好几张贴图，它们之间该和普通窗口一样"谁最后拿到焦点谁在上面"。
        // 层内顺序本来由合成器按栈序管，但 Wayland 下客户端点击一张被压住的贴图时我们做不了
        // 任何事——以前只有建窗和缩放两处会 `make_above`，于是缩放一张被压住的贴图会让它突然
        // 跳到最前，而单纯点它却不动。这里补上焦点这条路，让两者一致。
        //
        // 没开图钉的贴图不管：它是普通窗口，合成器自己就会把它抬上来。
        tauri::WindowEvent::Focused(true) if window.label().starts_with("pin-") => {
            if let Some(state) = window.app_handle().try_state::<AppState>() {
                crate::pin::raise_focused_pin(window.app_handle(), &state, window.label());
            }
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

/// 主窗口靠快捷键反复显隐，关闭要退化成隐藏；其余窗口（设置、Pin、覆盖层）
/// 都是用完即销毁，真关掉才对。
fn hides_instead_of_closing(label: &str) -> bool {
    matches!(label, "main")
}

/// 侧栏开着时失焦不隐藏窗口。
///
/// 原生弹窗（编解码面板的下拉、右键菜单、文件对话框）在 WebKitGTK 上是独立的 GTK 窗口，
/// 一打开 webview 就失焦。此时把无边框的主窗口藏掉会让弹窗变成孤儿浮层，视觉上等同崩溃。
/// 判定与前端 `app.js::onWindowBlur` 保持一致：任一侧栏可见就不隐藏。
fn should_hide_on_focus_loss(preview_visible: bool, codec_visible: bool) -> bool {
    !preview_visible && !codec_visible
}

fn hide_main_after_focus_loss(window: tauri::Window) {
    let app_handle = window.app_handle().clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if window.is_focused().unwrap_or(true) {
            return;
        }
        if let Some(state) = app_handle.try_state::<AppState>() {
            // 读不到锁时按"侧栏可见"处理：宁可留着窗口，也不要在状态未知时藏掉它。
            let preview_visible = state
                .preview_visible
                .lock()
                .map_or(true, |preview_visible| *preview_visible);
            let codec_visible = state
                .codec_visible
                .lock()
                .map_or(true, |codec_visible| *codec_visible);
            if !should_hide_on_focus_loss(preview_visible, codec_visible) {
                return;
            }
        }
        let _ = window.hide();
    });
}

#[cfg(test)]
mod tests {
    use super::{hides_instead_of_closing, should_hide_on_focus_loss};

    #[test]
    fn reusable_windows_hide_instead_of_entering_a_destroy_race() {
        assert!(hides_instead_of_closing("main"));
        assert!(!hides_instead_of_closing("settings"));
        // 截图编辑器窗口已删除，不该再有复用它的隐藏路径
        assert!(!hides_instead_of_closing("capture"));
        assert!(!hides_instead_of_closing("pin-1"));
    }

    #[test]
    fn any_open_sidebar_keeps_the_main_window_visible() {
        assert!(should_hide_on_focus_loss(false, false));
        // 编解码面板的原生下拉会让 webview 失焦，这时候不能藏窗口
        assert!(!should_hide_on_focus_loss(false, true));
        assert!(!should_hide_on_focus_loss(true, false));
        assert!(!should_hide_on_focus_loss(true, true));
    }
}
