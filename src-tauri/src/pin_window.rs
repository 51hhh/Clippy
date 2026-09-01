//! Pin 窗口平台适配
//!
//! Pin 是可拖动小窗，不能复用截图 overlay 的 layer-shell 四边锚定逻辑；
//! 那会把窗口拉伸成全屏层。这里做两件平台相关的事：吃掉触控板捏合手势，
//! 以及把 WebKit 的页面缩放锁死在 100%。
//!
//! **置顶不在这里做。** 它是用户可开关的选项、默认关，由 `pin::window::keep_pin_above`
//! 按条目状态表态（见 `pin::model::PinEntry::above`）。这里以前无条件
//! `set_always_on_top(true)`，那会把默认值悄悄改回"总是置顶"。

/// 配置 pin 窗口的平台特定属性。
pub fn configure_pin_window(window: &tauri::WebviewWindow) {
    swallow_touchpad_pinch(window);
    lock_pin_zoom(window);
}

/// 在 GTK 事件层把触控板捏合手势吃掉。
///
/// 这是拦捏合的**正门**，`lock_pin_zoom` 只是兜底。探过 WebKitWebView 的事件掩码：
///
/// ```text
/// EventMask(BUTTON_MOTION | BUTTON_PRESS | BUTTON_RELEASE | TOUCH | TOUCHPAD_GESTURE)
/// ```
///
/// 也就是说 WebKit 自己订阅了 `GDK_TOUCHPAD_PINCH`，捏合是作为**原生手势事件**进去的，
/// 既不是 ctrl+滚轮（前端的 wheel 监听拦不到），也不改 `zoom-level` 属性
/// （WebKit 内部走的是 page scale，`lock_pin_zoom` 的 notify 回调根本不会被叫醒）——
/// 这就是"触控板缩放手势还是没修好"的原因：之前两道防线都没在捏合的那条路上。
///
/// GTK3 里 `::event` 信号的处理器跑在 widget 类自己的默认处理器**之前**，
/// 返回 `Stop` 就终止这次信号发射，WebKit 那边收不到事件、也就无从缩放。
/// 只吃 `TouchpadPinch` 一种类型：按键、移动、触摸都要照常放过去，
/// 否则贴图的拖动和按钮就全废了。
fn swallow_touchpad_pinch(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "linux")]
    if let Err(error) = window.with_webview(|webview| {
        use gtk::glib::Propagation;
        use gtk::prelude::WidgetExt;

        webview.inner().connect_event(|_, event| {
            if event.event_type() == gtk::gdk::EventType::TouchpadPinch {
                Propagation::Stop
            } else {
                Propagation::Proceed
            }
        });
    }) {
        log::warn!("pin 窗口捏合手势拦截失败: {error}");
    }
}

/// 把贴图窗口的 WebKit 页面缩放钉在 1.0。
///
/// 贴图自己有一套缩放（滚轮改 `scale`，窗口尺寸随之变化），WebKit 的页面缩放是**另一套**
/// 东西：它只放大 DOM 而不改窗口，于是内容溢出到窗口外、工具栏错位。
///
/// 这道锁管的是**改 `zoom-level` 属性**的那些入口：ctrl+滚轮、ctrl +/-、WebKit 自己的
/// 快捷键。捏合手势不走这条路（见 `swallow_touchpad_pinch`），别指望这里能拦住它。
/// WebKitGTK 没有"禁用页面缩放"的开关（`webkit_settings_*` 里只有 `zoom-text-only`），
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
