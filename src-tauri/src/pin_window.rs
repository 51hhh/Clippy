//! Pin 窗口平台适配
//!
//! Pin 是可拖动小窗，不能复用截图 overlay 的 layer-shell 四边锚定逻辑；
//! 那会把窗口拉伸成全屏层。这里做两件平台相关的事：再次确认 always-on-top，
//! 以及把 WebKit 的页面缩放锁死在 100%。

/// 配置 pin 窗口的平台特定属性。
pub fn configure_pin_window(window: &tauri::WebviewWindow) {
    if let Err(e) = window.set_always_on_top(true) {
        log::warn!("pin 窗口置顶确认失败: {e}");
    }
    lock_pin_zoom(window);
}

/// 把贴图窗口的 WebKit 页面缩放钉在 1.0。
///
/// 贴图自己有一套缩放（滚轮改 `scale`，窗口尺寸随之变化），WebKit 的页面缩放是**另一套**
/// 东西：它只放大 DOM 而不改窗口，于是内容溢出到窗口外、工具栏错位。触控板的捏合手势
/// 会被 WebKitGTK 合成成 ctrl+滚轮，直接触发页面缩放——这就是"用触控板缩放导致异常"的来源。
/// 前端拦不住它：React 把 wheel 注册成 passive 监听器，`preventDefault()` 在那里是空操作，
/// 而且捏合还可能根本不经过 DOM 事件。所以在这一层兜底。
///
/// WebKitGTK 没有"禁用捏合缩放"的开关（`webkit_settings_*` 里只有 `zoom-text-only`），
/// 唯一能拿到的抓手就是 `zoom-level` 属性。于是监听它的变更、每次被改动就打回 1.0。
fn lock_pin_zoom(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "linux")]
    if let Err(error) = window.with_webview(|webview| {
        use webkit2gtk::WebViewExt;
        let webkit = webview.inner();
        webkit.set_zoom_level(1.0);
        webkit.connect_zoom_level_notify(|view| {
            // 比较要留容差：属性通知里的浮点值不一定精确等于我们写进去的 1.0，
            // 严格比较会让这个回调自己触发自己，死循环刷 GTK 主循环。
            if (view.zoom_level() - 1.0).abs() > f64::EPSILON {
                view.set_zoom_level(1.0);
            }
        });
    }) {
        log::warn!("pin 窗口缩放锁定失败: {error}");
    }
}
