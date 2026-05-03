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
    }

    clippy_lib::run()
}
