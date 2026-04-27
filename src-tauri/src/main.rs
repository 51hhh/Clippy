#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 限制 glibc malloc arena 数量，减少内存碎片（Linux 专用）
    #[cfg(target_os = "linux")]
    {
        std::env::set_var("MALLOC_ARENA_MAX", "2");
    }

    clippy_lib::run()
}
