# Tauri v2 + WebKitGTK 内存优化方案研究报告

> 目标：将 Clippy 应用从 ~190MB PSS 降至 ~80-90MB  
> 技术栈：Tauri v2 / wry 0.55 / webkit2gtk 2.0.2 / WebKitGTK 4.1 / Linux  
> 3 进程架构：主进程 + WebKit Web 进程 + WebKit 网络进程

---

## 一、环境变量调优（难度：低 / 影响：中-高 / 风险：低）

### 1.1 MALLOC_ARENA_MAX（glibc malloc 竞技场）

**原理**：glibc ptmalloc2 默认为每个线程创建独立 arena（64-bit 最多 `8 × CPU核数`）。每个 arena 至少预留约 1MB 虚拟内存。WebKitGTK 的多线程架构会创建大量 arena。

**方案**：限制 arena 数量为 2，减少内存池碎片。

```rust
// src-tauri/src/main.rs - 在 main() 最前面设置
fn main() {
    // 限制 glibc malloc arena 数量，减少多线程内存池开销
    // 默认值: 8 × CPU核数（64-bit），设为 2 可节省 20-60MB
    std::env::set_var("MALLOC_ARENA_MAX", "2");
    
    clippy_lib::run();
}
```

**预期收益**：20-60MB（取决于 CPU 核数，8核机器效果最明显）  
**代价**：高并发场景下 malloc 竞争略增，但对 GUI 应用几乎无影响  
**参考**：`man 3 mallopt` — `M_ARENA_MAX`

### 1.2 MALLOC_MMAP_THRESHOLD_（mmap 阈值）

**原理**：默认 128KB 以上的分配使用 `mmap()`。降低阈值让更多中型分配走 mmap，内存释放后可立即归还 OS（`munmap`），而非留在 heap 空闲列表。

```rust
// 降低 mmap 阈值到 64KB，让 64KB+ 分配直接 mmap
// 释放后内存立即归还 OS，减少常驻集
std::env::set_var("MALLOC_MMAP_THRESHOLD_", "65536");
```

**预期收益**：10-30MB  
**代价**：频繁小型 mmap/munmap 有少量系统调用开销  
**注意**：此变量名末尾有下划线 `_`

### 1.3 MALLOC_TRIM_THRESHOLD_（堆顶修剪阈值）

**原理**：控制 `free()` 后何时调用 `sbrk()` 收缩堆顶。默认 128KB，降低后更积极地将内存归还 OS。

```rust
std::env::set_var("MALLOC_TRIM_THRESHOLD_", "32768");  // 32KB
```

**预期收益**：5-15MB  
**代价**：几乎没有

### 1.4 完整 main.rs 设置

```rust
fn main() {
    // === 内存优化：在一切初始化之前设置 ===
    // 这些环境变量会被 WebKit 子进程继承
    std::env::set_var("MALLOC_ARENA_MAX", "2");
    std::env::set_var("MALLOC_MMAP_THRESHOLD_", "65536");
    std::env::set_var("MALLOC_TRIM_THRESHOLD_", "32768");
    
    clippy_lib::run();
}
```

---

## 二、WebKit 环境变量（难度：低 / 影响：中 / 风险：中）

来源：[WebKit Environment Variables (trac.webkit.org)](https://trac.webkit.org/wiki/EnvironmentVariables)

### 2.1 WEBKIT_DISABLE_COMPOSITING_MODE

**原理**：禁用 GPU 加速合成模式，WebKit 改用纯 CPU 软件渲染。GPU 合成需要分配 GPU 缓冲区和共享内存，这是 WebProcess 内存的一大消耗来源。

**实测**:禁用 GPU 合成/硬件加速导致 llvmpipe 软件渲染 fallback：

libLLVM: 2.5→24 MB（+21 MB，用于 shader 编译）
libgtk/libwebkit/libgallium 等全面暴涨
需要策略调整：保留 MALLOC_ARENA_MAX（heap -17 MB 已验证有效），撤销 GPU 相关设置。

```rust
std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
```

**预期收益**：30-60MB（GPU 纹理缓冲 + 共享内存）  
**风险**：CSS 动画/transform 性能下降，无 GPU 加速。对于 Clippy 这种简单列表 UI 完全可接受  
**适用**：WebKitGTK ≥ 2.10.9

> ⚠️ **对比 WEBKIT_DISABLE_DMABUF_RENDERER**：你之前测试过 DMABUF 变量反而增加内存。COMPOSITING_MODE 是不同的机制——它关闭整个合成层，而非仅切换渲染后端。

### 2.2 WEBKIT_DISABLE_MEMORY_PRESSURE_MONITOR

**注意**：此变量禁用内存压力监视器。**不建议设置**，因为 WebKit 的内存压力响应机制有助于在系统低内存时释放缓存。

---

## 三、WebKitSettings API 调优（难度：中 / 影响：中 / 风险：低）

通过 Tauri 的 `with_webview()` API 访问底层 WebKitGTK WebView，然后修改 Settings。

### 3.1 API 调用路径

```rust
use tauri::Manager;

// 在 setup 闭包中，或 webview 创建后
let main_webview = app.get_webview_window("main").unwrap();
main_webview.with_webview(|webview| {
    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::{WebViewExt, SettingsExt};
        let wk_view = webview.inner();
        if let Some(settings) = wk_view.settings() {
            // ... 设置各项属性
        }
    }
});
```

### 3.2 hardware-acceleration-policy

`WebKit2.HardwareAccelerationPolicy` 枚举值：
- `ALWAYS`（默认）— 始终使用 GPU 加速
- `ON_DEMAND` — 按需使用
- `NEVER` — 完全禁用

```rust
use webkit2gtk::HardwareAccelerationPolicy;
settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
```

**预期收益**：与 WEBKIT_DISABLE_COMPOSITING_MODE 类似，30-50MB  
**优势**：比环境变量更精确，可按 webview 控制  
**注意**：`NEVER` 策略下 WebGL/CSS 3D transform 不工作

### 3.3 禁用不需要的功能

Clippy 是纯文本剪贴板管理器，可安全禁用大量 Web 功能：

```rust
// 禁用 WebGL（默认 true）— 节省 GPU 上下文初始化内存
settings.set_enable_webgl(false);

// 禁用 WebAudio（默认 true）
settings.set_enable_webaudio(false);

// 禁用 MediaSource（默认 true）
settings.set_enable_mediasource(false);

// 禁用 MediaStream（默认 true）
settings.set_enable_media_stream(false);

// 禁用媒体播放（默认 true）— 彻底禁用 <audio>/<video>/<track>
settings.set_enable_media(false);

// 禁用 Page Cache（默认 true）— 减少后退/前进缓存的内存占用
// Clippy 只有单页面，不需要页面缓存
settings.set_enable_page_cache(false);

// 禁用平滑滚动（默认 true）— 减少合成层
settings.set_enable_smooth_scrolling(false);

// 禁用 2D Canvas 加速（默认 true, since 2.46）
settings.set_enable_2d_canvas_acceleration(false);
```

**预期收益（合计）**：10-30MB  
**风险**：极低，这些功能 Clippy 完全不使用

### 3.4 完整 setup 代码

```rust
// 在 lib.rs 的 .setup() 闭包中
.setup(|app| {
    let main_window = app.get_webview_window("main").unwrap();
    
    main_window.with_webview(|webview| {
        #[cfg(target_os = "linux")]
        {
            use webkit2gtk::{WebViewExt, SettingsExt};
            use webkit2gtk::HardwareAccelerationPolicy;
            
            let wk_view = webview.inner();
            if let Some(settings) = wk_view.settings() {
                // 核心：禁用 GPU 加速
                settings.set_hardware_acceleration_policy(
                    HardwareAccelerationPolicy::Never
                );
                
                // 禁用不需要的 Web 功能
                settings.set_enable_webgl(false);
                settings.set_enable_webaudio(false);
                settings.set_enable_mediasource(false);
                settings.set_enable_media_stream(false);
                settings.set_enable_media(false);
                settings.set_enable_page_cache(false);
                settings.set_enable_smooth_scrolling(false);
            }
        }
    })?;
    
    Ok(())
})
```

---

## 四、WebKitWebContext 层面优化（难度：中 / 影响：中 / 风险：中）

### 4.1 WebKitWebContext cache-model

WebKit 的 `WebKitCacheModel` 枚举：
- `DOCUMENT_VIEWER` — 最小缓存，适合不导航的单文档查看器
- `WEB_BROWSER` — 最大缓存（默认）
- `DOCUMENT_BROWSER` — 中等缓存

```rust
use webkit2gtk::{WebViewExt, WebContextExt};

let wk_view = webview.inner();
let context = wk_view.context().unwrap();

use webkit2gtk::CacheModel;
context.set_cache_model(CacheModel::DocumentViewer);
```

**预期收益**：10-20MB（减少磁盘和内存缓存）  
**风险**：低。Clippy 只加载本地 HTML，不需要网络缓存

### 4.2 内存压力手动触发

可在窗口隐藏时主动释放 WebKit 缓存：

```rust
// 在窗口隐藏时执行 JavaScript 触发 GC
webview.eval("if (window.gc) window.gc();").ok();

// 或通过 WebKitWebContext API（如果可用）
// webkit_web_context_clear_cache() 在某些版本可用
```

---

## 五、进程模型优化（难度：高 / 影响：高 / 风险：高）

### 5.1 共享 WebProcess

Tauri 的 `WebviewBuilder::with_related_view()` 可让多个 webview 共享同一个 Web 进程。

```rust
// WebviewBuilder 上的方法（Linux only, requires wry feature）
pub fn with_related_view(self, related_view: WebView) -> Self
```

**场景**：如果未来 Clippy 有多个 webview 窗口（main + settings），可共享 WebProcess 节省一整个进程的内存。

**预期收益**：30-50MB（一个 WebProcess 的开销）  
**风险**：一个 webview 崩溃会影响另一个；JS 全局作用域隔离不如独立进程

---

## 六、前端优化（难度：低 / 影响：低-中 / 风险：低）

### 6.1 减少 DOM 节点

当前 Clippy 可能一次渲染所有历史条目。实现虚拟滚动（只渲染可视区域 DOM）可减少 WebProcess 内存。

```javascript
// 只渲染可见区域的 ~20 个条目，而非全部 500+
// 使用 IntersectionObserver 或手动计算 scrollTop
```

**预期收益**：5-15MB（取决于历史条目数量）

### 6.2 图片/Blob 及时释放

如果存储了图片类剪贴板内容，确保不用时调用 `URL.revokeObjectURL()`。

---

## 七、优先级排序总结

| 序号 | 措施 | 难度 | 预期收益 | 风险 | 建议 |
|------|------|------|----------|------|------|
| 1 | MALLOC_ARENA_MAX=2 | ⭐ | 20-60MB | 极低 | **立即实施** |
| 2 | WEBKIT_DISABLE_COMPOSITING_MODE=1 | ⭐ | 30-60MB | 低 | **立即实施** |
| 3 | HardwareAccelerationPolicy::Never | ⭐⭐ | 30-50MB | 低 | **优先实施**（与 #2 二选一） |
| 4 | 禁用 WebGL/Audio/Media 等 | ⭐⭐ | 10-30MB | 极低 | **优先实施** |
| 5 | MALLOC_MMAP_THRESHOLD_=65536 | ⭐ | 10-30MB | 极低 | **立即实施** |
| 6 | CacheModel::DocumentViewer | ⭐⭐ | 10-20MB | 低 | **优先实施** |
| 7 | MALLOC_TRIM_THRESHOLD_=32768 | ⭐ | 5-15MB | 极低 | 立即实施 |
| 8 | Page Cache 禁用 | ⭐⭐ | 5-10MB | 极低 | 优先实施 |
| 9 | 前端虚拟滚动 | ⭐⭐⭐ | 5-15MB | 低 | 后续迭代 |
| 10 | 共享 WebProcess | ⭐⭐⭐⭐ | 30-50MB | 中 | 谨慎评估 |

> **综合预期**：实施 #1 + #2/#3 + #4 + #5 + #6 + #7 + #8 后，PSS 应从 ~190MB 降至 ~80-100MB 范围。

---

## 八、验证方法

```bash
# 查看 PSS（Proportional Set Size）
sudo smem -t -k -P clippy

# 或使用 /proc 直接查看
grep -i pss /proc/$(pgrep -f "com.clippy")/smaps_rollup

# WebKit 内存采样（调试用）
WEBKIT_SAMPLE_MEMORY=1 cargo tauri dev
# 查看 /tmp/WebKit* 生成的内存统计文件

# 对比前后
# 1. 启动应用，等待 10 秒稳定
# 2. 执行 smem 采集基线
# 3. 应用环境变量/代码修改
# 4. 重新采集对比
```

---

## 九、参考资料

- [mallopt(3) man page](https://man7.org/linux/man-pages/man3/mallopt.3.html) — MALLOC_ARENA_MAX, M_MMAP_THRESHOLD 等
- [WebKit Environment Variables](https://trac.webkit.org/wiki/EnvironmentVariables) — WEBKIT_DISABLE_COMPOSITING_MODE 等
- [WebKit2.Settings (PyGObject)](https://lazka.github.io/pgi-docs/WebKit2-4.1/classes/Settings.html) — hardware-acceleration-policy, enable-* 属性
- [Tauri Webview::with_webview()](https://docs.rs/tauri/2.10.2/tauri/webview/struct.Webview.html#method.with_webview) — 访问平台底层 webview
- [wry WebViewExtUnix](https://docs.rs/wry/latest/wry/trait.WebViewExtUnix.html) — `.webview()` 获取 webkit2gtk WebView
- [webkit2gtk crate](https://docs.rs/webkit2gtk/2.0.2/webkit2gtk/) — Rust 绑定
