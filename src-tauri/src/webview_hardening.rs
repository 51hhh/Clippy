//! 关掉 WebKit 自带的右键菜单与开发者工具。
//!
//! 这个应用的每个窗口都是"界面"而不是"网页"：无边框悬浮面板、贴图、截图覆盖层、设置页。
//! WebKitGTK 默认给它们挂一份网页右键菜单（重新加载、返回、检查元素……），后果有三层：
//!
//! 1. **"重新加载"会把界面弄坏**。贴图窗口重载之后前端要重新取一次 payload，而清晰度补偿
//!    结果在上屏后就被释放了（见 `pin::model::SharpenSlot::release`），重载只能拿回原图；
//!    覆盖层重载更糟——会话还在，但那块冻结帧的画布没了。
//! 2. **"检查元素"打开开发者工具**，在一个无边框、不可缩放的小窗口里根本没法用，
//!    而且用户按不到关闭它的地方。
//! 3. 右键这个手势本身有更好的用途：贴图窗口用它出快速菜单（置顶/画布/保存/关闭），
//!    前端要收到 `contextmenu` 事件才能接管，而 WebKit 的默认菜单会抢在前面弹出来。
//!
//! **拦在 GTK 信号层，不靠前端 `preventDefault`。** 前端那条路要求每个页面各写一遍、
//! 而且 JS 出错或还没加载完时右键就漏出去了；`context-menu` 信号返回 `true` 是"这次菜单
//! 已被处理"，WebKit 于是什么都不弹，这一层对所有页面一次生效。返回 `true` **不会**阻止
//! DOM 的 `contextmenu` 事件，所以前端接管右键的能力不受影响（贴图菜单就靠这个）。
//!
//! **可编辑区域是例外，照旧给默认菜单。** 设置页里有十几个输入框（含翻译 API key 与
//! 密码框），把它们的右键粘贴也拦掉是纯粹的体验回退——没人手打一串长密钥。可编辑区域的
//! 默认菜单只有剪切/复制/粘贴/全选，不含上面那两个会弄坏界面的入口，所以放行是安全的。
//!
//! 另外把 `enable_developer_extras` 关掉：它是开发者工具的总开关，关了之后连快捷键
//! （Ctrl+Shift+I / F12）也进不去。**dev 构建保留它**——调试界面本来就要用，
//! 而 dev 构建不会发给用户。

/// 注册成插件，好让**每一个** webview 都过这一遍。
///
/// 不在建窗的地方逐个调：窗口有四类（main、settings、pin-*、capture-overlay-*），
/// 其中设置窗口和贴图窗口是按需创建的，漏一处就等于那个页面还留着右键菜单。
/// 插件的 `on_webview_ready` 由 Tauri 在每个 webview 建好时调用，天然覆盖全部。
pub(crate) fn plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("clippy-webview-hardening")
        .on_webview_ready(|webview| {
            let label = webview.label().to_string();
            let for_closure = label.clone();
            if let Err(error) = webview.with_webview(move |platform| harden(&for_closure, platform))
            {
                log::warn!("webview {label} 右键菜单与开发者工具未能关闭: {error}");
            }
        })
        .build()
}

#[cfg(target_os = "linux")]
fn harden(label: &str, platform: tauri::webview::PlatformWebview) {
    use webkit2gtk::{HitTestResultExt, SettingsExt, WebViewExt};

    let webkit = platform.inner();
    // 返回 true = "这次右键菜单我处理了"，WebKit 因此不弹任何菜单。
    // DOM 的 contextmenu 事件照旧派发，前端接管右键的能力不受影响。
    //
    // **可编辑区域例外。** 一律拦掉的话，设置页里那些输入框（9 个文本框、2 个密码框、
    // 1 个 URL 框）就没有右键粘贴了——而翻译 API key 恰恰是最需要粘贴的东西，没人手打
    // 一串长密钥。那里的默认菜单只有剪切/复制/粘贴/全选这类编辑项，本来就是用户要的，
    // 也没有"重新加载/检查元素"那两个会弄坏界面的入口。
    webkit.connect_context_menu(|_, _, _, hit| !hit.context_is_editable());
    // debug_assertions 而不是某个环境变量：这是"开发时可用、发布版没有"的开关，
    // 不该让用户能在运行时打开。
    if !cfg!(debug_assertions) {
        match webkit.settings() {
            Some(settings) => settings.set_enable_developer_extras(false),
            None => log::warn!("webview {label} 拿不到 WebKit settings，开发者工具开关未改"),
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn harden(_label: &str, _platform: tauri::webview::PlatformWebview) {}
