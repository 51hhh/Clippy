//! Pin 窗口平台适配
//!
//! Pin 是可拖动小窗，不能复用截图 overlay 的 layer-shell 四边锚定逻辑；
//! 那会把窗口拉伸成全屏层。这里仅在创建后再次确认 always-on-top。

/// 配置 pin 窗口的平台特定属性。
pub fn configure_pin_window(window: &tauri::WebviewWindow) {
    if let Err(e) = window.set_always_on_top(true) {
        log::warn!("pin 窗口置顶确认失败: {e}");
    }
}
