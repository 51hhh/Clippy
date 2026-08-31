#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // glibc malloc 调优 — 减少内存碎片和常驻集（Linux 专用）
    #[cfg(target_os = "linux")]
    {
        unsafe {
            // 限制 malloc arena 数量（默认 8×核数），减少线程池碎片
            std::env::set_var("MALLOC_ARENA_MAX", "2");
            // 降低 mmap 阈值：64KB+ 分配直接 mmap，释放后立即归还 OS
            std::env::set_var("MALLOC_MMAP_THRESHOLD_", "65536");
            // 降低堆顶修剪阈值：free() 后更积极地收缩堆
            std::env::set_var("MALLOC_TRIM_THRESHOLD_", "32768");
        }

        // AppImage 的 linuxdeploy-plugin-gtk 强制设置 GDK_BACKEND=x11，
        // 导致 Wayland 下托盘图标消失和页面渲染异常。
        // 在 GTK 初始化前移除此变量，使 WebKit2GTK 使用原生 Wayland 后端。
        if std::env::var("WAYLAND_DISPLAY").is_ok()
            || std::env::var("XDG_SESSION_TYPE")
                .map(|v| v == "wayland")
                .unwrap_or(false)
        {
            unsafe { std::env::remove_var("GDK_BACKEND") };
        }
    }

    // 截图诊断在 Tauri 之前就结束：几何算错时 GUI 本身不可信，而且这条路不该抢
    // single-instance 的 D-Bus name 把用户正在用的实例顶掉。见 docs/capture-linux.md §4.2。
    if let Some(code) = clippy_lib::capture_diagnostics_cli() {
        std::process::exit(code);
    }

    clippy_lib::run()
}
