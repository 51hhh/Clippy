# Clippy 内存优化深度方案 v2

> **目标**：将 Clippy 应用 PSS 从 ~64MB 进一步降至 ~40-50MB，同时保持剪贴板监听的快速响应  
> **原则**：只有剪贴板监听需要快速响应，其他组件（设置窗口、预览面板、Pin 窗口）不必常驻内存  
> **技术栈**：Tauri v2 / wry / webkit2gtk 2.0.2 / WebKitGTK 4.1 / Linux  
> **3 进程架构**：主进程（Core Process）+ WebKit Web 进程 + WebKit 网络进程  
> **状态**：方案文档，尚未执行

---

## 〇、当前内存基线（v0.1.12 Release Build）

```
测量方法: /proc/<PID>/smaps_rollup + /proc/<PID>/status
环境: Ubuntu 26.04, 启动后等待 10 秒稳定

RSS:       165,580 KB  (~162 MB)  ← 表面数字，包含大量共享库
PSS:        64,044 KB  (~63 MB)  ← 真实唯一内存代价
Pss_Anon:   25,876 KB  (~25 MB)  ← 堆 + 栈（匿名内存）
Pss_File:   37,915 KB  (~37 MB)  ← 文件映射的按比例份额
RssAnon:    25,876 KB  (~25 MB)  ← 匿名页（堆+栈）
RssFile:   139,448 KB  (~136 MB) ← 文件映射页（共享库），大部分与系统共享
Shared_Clean: 128,316 KB (~125 MB) ← 只读共享页
VmPeak: 135,186,732 KB (~72 GB)  ← 虚拟内存峰值（WebKit mmap 映射伪影）
```

### 关键洞察

| 维度 | 值 | 含义 |
|------|------|------|
| **RSS** | 162 MB | 包含共享库映射，不反映真实独占代价 |
| **PSS** | 63 MB | 按比例分摊共享库后的真实内存占用 |
| **Pss_Anon** | 25 MB | 堆上内存，是我们可以直接优化的部分 |
| **RssFile** | 136 MB | 几乎全是 GTK/WebKit 共享库（`libwebkit2gtk`、`libgtk-4`、`libglib` 等） |
| **Shared_Clean** | 125 MB | 共享只读页，在系统中只存一份，其他 GTK 应用也共用 |

**结论**：
1. **RSS 162MB 是假象**——136MB 是共享库映射，系统中只存一份
2. **真实唯一代价 PSS 63MB**——已经处于 Tauri 应用的正常范围
3. **可优化空间 ~25MB 在匿名内存（堆+栈）**
4. 要进一步降低 PSS，核心方向是：减少 WebKit 进程内存 + 延迟加载非关键组件 + 窗口隐藏时释放资源

### 已实施的优化措施

| 措施 | 位置 | 状态 |
|------|------|------|
| `MALLOC_ARENA_MAX=2` | main.rs | ✅ 已生效 |
| `MALLOC_MMAP_THRESHOLD_=65536` | main.rs | ✅ 已生效 |
| `MALLOC_TRIM_THRESHOLD_=32768` | main.rs | ✅ 已生效 |
| `HardwareAccelerationPolicy::Never` | lib.rs setup() | ✅ 已生效 |
| 禁用 WebGL/WebAudio/Media/PageCache | lib.rs setup() | ✅ 已生效 |
| SQLite `PRAGMA cache_size = 128` | storage.rs | ✅ 已生效 |
| `[profile.release] strip/lto/codegen-units` | Cargo.toml | ✅ 已生效 |
| `CacheModel::DocumentViewer` | lib.rs setup() | ✅ 已生效 |
| 窗口隐藏时 `clear_cache()` | lib.rs on_window_event | ✅ 已生效 |
| 前端 blur 时释放 DOM + 预览内容 | app.js / preview-panel.js | ✅ 已生效 |
| `lto=true` + `opt-level="s"` + `panic="abort"` | Cargo.toml | ✅ 已生效 |

---

## 一、WebKit 层深度优化

### 1.1 CacheModel::DocumentViewer（难度：低 / 预期收益：5-15MB）

**原理**：WebKit 的 `CacheModel` 决定了缓存策略。当前使用默认的 `WEB_BROWSER` 模式，会缓存大量网络资源。Clippy 只加载本地 HTML，完全不需要网络缓存。

**官方描述**（来源：webkitgtk.org/reference/webkit2gtk/stable/enum.CacheModel.html）：
- `DOCUMENT_VIEWER`(0)：完全禁用缓存，大幅减少内存。适合只访问单个本地文件、不导航的应用
- `WEB_BROWSER`(1)：最大缓存（默认值）
- `DOCUMENT_BROWSER`(2)：中等缓存

```rust
// lib.rs setup() 中，通过 with_webview 获取 context
use webkit2gtk::{WebViewExt, WebContextExt, CacheModel};
let context = wk.context().unwrap();
context.set_cache_model(CacheModel::DocumentViewer);
```

**收益**：减少 WebKit 进程的磁盘缓存和内存缓存占用  
**风险**：极低，Clippy 加载的是本地 `tauri://localhost` 资源  
**优先级**：🔴 P0

### 1.2 WebKitMemoryPressureSettings（难度：中 / 预期收益：10-25MB）

**原理**：WebKitGTK 2.34+ 提供了 `MemoryPressureSettings`，可以精确控制 Web 进程的内存上限和回收策略。默认内存上限是系统 RAM（最高 3GB），对 Clippy 这种轻量应用完全过剩。

**官方文档**（来源：webkitgtk.org/reference/webkit2gtk/stable/struct.MemoryPressureSettings.html）：

| API | 默认值 | 说明 |
|-----|--------|------|
| `set_memory_limit(mb)` | 系统RAM（max 3GB） | Web 进程允许使用的内存上限(MB) |
| `set_conservative_threshold(f)` | 0.33 | 达到 limit×f 时开始释放非关键内存 |
| `set_strict_threshold(f)` | 0.5 | 达到 limit×f 时积极释放内存 |
| `set_kill_threshold(f)` | 0.0（禁用） | 达到 limit×f 时终止进程 |
| `set_poll_interval(ms)` | 未公开 | 内存用量检查间隔 |

**推荐配置**：

```rust
// 限制 Web 进程内存为 80MB，并设置积极的回收阈值
// 注意：MemoryPressureSettings 是 WebContext 的 construct-only 属性
// 需要通过 g_object_new 或自定义 WebContext 设置
// 在 Tauri/wry 层面需要评估是否有 API 通道

// 理想设置（如果 API 可达）：
// memory_limit = 80  (MB)
// conservative_threshold = 0.25  (20MB 时开始温和回收)
// strict_threshold = 0.5  (40MB 时积极回收)
// kill_threshold = 0.0  (不杀进程，避免白屏)
// poll_interval = 30000  (30秒检查一次，低频减少开销)
```

**限制**：`memory-pressure-settings` 是 `WebContext` 的 **construct-only** 属性，只能在创建 WebContext 时设置。Tauri/wry 目前不暴露 WebContext 创建前的 hook，这个优化需要评估实现路径：
1. 通过 `with_webview` 获取 context 后尝试设置（可能已来不及）
2. 通过环境变量间接影响（WebKit 内部读取）
3. 向 wry 上游提 PR 支持自定义 WebContext 选项

**风险**：内存限制设太低会导致频繁 GC 影响性能  
**优先级**：🟡 P1（需要验证 API 可达性）

### 1.3 窗口隐藏时清理 WebKit 缓存（难度：低 / 预期收益：3-8MB）

**原理**：Clippy 主窗口大部分时间是隐藏的（失焦自动隐藏）。隐藏时可以主动清理 WebKit 缓存和触发 JS GC。

```rust
// on_window_event 中，窗口隐藏时执行清理
tauri::WindowEvent::Focused(false) if window.label() == "main" => {
    // 延迟隐藏后，通知前端释放资源
    let _ = window.eval("
        if (window.gc) window.gc();
        // 清理 blob URL 缓存等
    ");
}
```

同时可以通过 `WebContext::clear_cache()` 清理 WebKit 层缓存：

```rust
// 窗口隐藏时
main_window.with_webview(|webview| {
    use webkit2gtk::{WebViewExt, WebContextExt};
    let wk = webview.inner();
    if let Some(ctx) = wk.context() {
        ctx.clear_cache();
    }
});
```

**风险**：下次显示时有微小延迟（重建缓存）  
**优先级**：🔴 P0

---

## 二、组件常驻分析与延迟加载策略

### 2.1 当前组件常驻情况

| 组件 | 创建方式 | 生命周期 | 是否需要常驻 | 内存特征 |
|------|----------|----------|------------|----------|
| **剪贴板监听器** | setup() 中 `thread::spawn` | 全程运行 | ✅ 必须常驻 | ~2-3MB（线程栈+arboard） |
| **SQLite 存储引擎** | setup() 中初始化 | 全程持有 | ✅ 必须常驻 | ~1-2MB（连接+cache 512KB） |
| **主窗口 WebView** | tauri.conf.json 声明 | 全程存在 | ⚠️ 可优化 | ~40-50MB（WebKit 进程） |
| **系统托盘** | setup() 中 build_tray() | 全程存在 | ✅ 必须常驻 | ~1MB |
| **全局快捷键** | setup() 中注册 | 全程存在 | ✅ 必须常驻 | ~0.5MB |
| **D-Bus 服务** | setup() 中 spawn | 全程运行 | ✅ 必须常驻 | ~1MB |
| **设置窗口** | 托盘菜单动态创建 | 按需创建/关闭 | ❌ 已优化 | 创建时 +15-20MB |
| **Pin 窗口** | API 动态创建 | 按需创建/关闭 | ❌ 已优化 | 每个 +10-15MB |
| **预览面板** | 前端 JS 模块 | 主窗口加载即存在 | ⚠️ 可延迟 | ~3-5MB（hljs+marked+DOMPurify） |

### 2.2 主窗口 WebView 优化策略

主窗口是内存消耗的主体（WebKit Web 进程 ~40-50MB）。大部分时间窗口是隐藏的，但 WebKit 进程仍在运行。

**策略 A：隐藏时最小化 WebView 内存**（推荐）

```
- 窗口隐藏时：清除 DOM 缓存、暂停动画、释放 blob URL
- 窗口显示时：按需重建列表 DOM
- 不销毁 WebView，保持 IPC 通道
```

这是最安全的方案：保持 WebView 进程不变，但释放非必要的 JS/DOM 内存。

**策略 B：销毁 + 重建 WebView**（激进，不推荐）

```
- 窗口隐藏超过 N 分钟后，销毁 WebView
- 下次显示时重建
- 可节省 ~40MB，但重建耗时 300-500ms
```

对于 Clippy 这种弹出式工具，500ms 的重建延迟是不可接受的。剪贴板管理器最重要的体验是**即时响应**。

**策略 C：WebView 回收池**（中等复杂度）

如果未来有多窗口（设置、Pin），可以考虑 `WebviewBuilder::with_related_view()` 让它们共享同一个 Web 进程，减少进程数量。

```rust
// Tauri/wry API（Linux only）
WebviewBuilder::new().with_related_view(main_webview)
```

**收益**：避免每个窗口创建独立 Web 进程（每个 ~15-20MB）  
**限制**：共享进程意味着一个 webview 崩溃影响全部  
**优先级**：🟡 P1（当 Pin 功能频繁使用时再考虑）

### 2.3 前端延迟加载策略

当前 `preview-panel.js` 在主窗口加载时立即导入了 21 个 highlight.js 语言包 + marked + DOMPurify：

```javascript
// 当前（preview-panel.js）—— 启动时全部加载
import hljs from "highlight.js/lib/core";
import javascript from "highlight.js/lib/languages/javascript";
import typescript from "highlight.js/lib/languages/typescript";
// ... 19 more languages
import DOMPurify from "dompurify";
import { marked } from "marked";
```

**优化方案：动态 import()**

```javascript
// 优化后：首次打开预览面板时才加载
let _hljs = null;
let _marked = null;
let _DOMPurify = null;

async function ensureLibs() {
  if (!_hljs) {
    const [hljsMod, markedMod, purifyMod] = await Promise.all([
      import("highlight.js/lib/core"),
      import("marked"),
      import("dompurify"),
    ]);
    _hljs = hljsMod.default;
    _marked = markedMod.marked;
    _DOMPurify = purifyMod.default;
    // 按需注册语言
    const langs = await import("./preview-languages.js");
    langs.registerAll(_hljs);
  }
}
```

**收益**：
- 主窗口首次加载更快（减少 ~200KB JS 解析）
- 如果用户从不打开预览面板，这些库永远不会加载
- WebKit 进程 JS 堆内存减少 ~2-4MB

**风险**：首次打开预览面板有 ~100ms 延迟  
**优先级**：🟡 P1

### 2.4 图片缩略图缓存控制

当前 `clipboard-list.js` 使用 `Map` 缓存图片缩略图：

```javascript
const _thumbCache = new Map();
```

这个缓存永远不会清理。对于图片密集的用户，可能累积大量 base64 数据。

**优化方案**：
```javascript
const MAX_THUMB_CACHE = 50;
const _thumbCache = new Map();

function setThumbCache(id, data) {
  if (_thumbCache.size >= MAX_THUMB_CACHE) {
    // 删除最早的条目
    const first = _thumbCache.keys().next().value;
    _thumbCache.delete(first);
  }
  _thumbCache.set(id, data);
}
```

**收益**：限制缩略图缓存内存（每张 ~10-50KB，50 张封顶 ~2.5MB）  
**优先级**：🟢 P2

---

## 三、Rust 侧内存优化

### 3.1 替代 glibc malloc（难度：低 / 预期收益：3-10MB）

**现状**：已通过 `MALLOC_ARENA_MAX=2` 等环境变量调优 glibc malloc。但环境变量只影响主进程，**WebKit 子进程也会继承这些变量**（因为是 fork+exec 产生的），所以效果已覆盖全部进程。

**替代方案评估**：

| 分配器 | 优势 | 劣势 | 适合 Clippy？ |
|--------|------|------|-------------|
| **glibc malloc + ARENA_MAX** | 零额外依赖，环境变量生效 | 碎片化管理不如专用分配器 | ✅ 当前方案 |
| **jemalloc** (`tikv-jemallocator`) | 低碎片、自动归还内存、线程级缓存 | 增加 ~200KB 二进制大小，**只影响 Rust 主进程** | ⚠️ 可选 |
| **mimalloc** (Microsoft) | 极低碎片、高性能、安全模式可选 | 同上，**只影响 Rust 主进程** | ⚠️ 可选 |

**关键限制**：Rust `#[global_allocator]` 只替换主进程（Core Process）的堆分配器。WebKit 的 Web 进程和网络进程是独立 ELF 可执行文件（`/usr/lib/webkit2gtk-4.1/WebKitWebProcess`），仍然使用系统 glibc malloc，不受 Rust 全局分配器影响。

**结论**：主进程堆内存仅 ~5MB，换分配器收益有限。**保持现有 MALLOC_ARENA_MAX 方案更合理**。

**优先级**：🟢 P2（仅在主进程堆内存成为瓶颈时考虑）

### 3.2 Cargo Release Profile 优化（难度：低 / 预期收益：降低二进制加载内存）

**现状**：

```toml
# 已有配置
[profile.release]
strip = true
lto = "thin"
codegen-units = 1
```

**可以加强的配置**：

```toml
[profile.release]
strip = true
lto = true          # 从 "thin" 升级为 full LTO，更深度的跨 crate 内联和死代码消除
codegen-units = 1
opt-level = "s"     # 优化二进制大小（而非速度），减少加载到内存的代码页
panic = "abort"     # 不包含 unwind 表，减少 ~200KB 二进制大小
```

**Tauri 官方推荐**（来源：v2.tauri.app/concept/size/）完整配置：
```toml
[profile.release]
codegen-units = 1
lto = true
opt-level = "s"
panic = "abort"
strip = true
```

**收益**：
- `lto = true`（full LTO）vs `"thin"`：更多死代码消除，二进制更小 ~5-15%
- `opt-level = "s"`：减小 `.text` 段大小，加载到 RSS 的代码页更少
- `panic = "abort"`：去掉 unwind 表和 panic 处理栈，减少 ~200-500KB

**代价**：编译速度变慢（full LTO 比 thin LTO 慢 2-3x）  
**注意**：`opt-level = "s"` 可能微小降低运行速度（剪贴板轮询不受影响）  
**优先级**：🔴 P0

### 3.3 removeUnusedCommands（难度：低 / 预期收益：减少二进制 + 少量内存）

**来源**：Tauri 2.4+ 新增功能（v2.tauri.app/concept/size/）

```json
// tauri.conf.json
{
  "build": {
    "removeUnusedCommands": true
  }
}
```

移除 capability ACL 中未声明的 IPC 命令处理器，减少 `invoke_handler` 中不使用的代码。

**前提**：需要在 `capabilities/default.json` 中精确声明使用的权限（不能用 `defaults`）  
**优先级**：🟡 P1

---

## 四、前端 JS 内存管理

### 4.1 虚拟滚动（难度：中-高 / 预期收益：2-8MB）

**现状**：`clipboard-list.js` 采用分页加载（`PAGE_SIZE = 30`），滚动到底部追加。当用户滚动多页后，DOM 中可能有 300+ 个列表项。

**优化方案**：只渲染可视区域内的 ~15-20 个条目，滚动时动态回收/创建 DOM 节点。

```javascript
// 可视区域 15 行 × 每行 ~50px = 750px 视口
// 总数据量可以有数千条，但 DOM 始终只有 15-20 个节点
class VirtualScroller {
  constructor(container, itemHeight, renderItem) { ... }
  scrollTo(index) { ... }
  // 使用 requestAnimationFrame 节流渲染
}
```

**收益**：
- DOM 节点从 300+ 降至 20，WebKit 渲染树内存减少
- 减少 Layout/Paint 开销
- 对大量剪贴板历史的用户效果显著

**代价**：实现复杂度中等，需要重构 `clipboard-list.js` 的渲染逻辑  
**优先级**：🟡 P1（列表超过 100 项时效果明显）

### 4.2 Blob URL 及时释放

确保图片预览关闭时释放 Blob URL：

```javascript
// 当前可能遗漏的场景
URL.revokeObjectURL(oldBlobUrl);
```

**优先级**：🟢 P2

### 4.3 主窗口隐藏时的内存释放协议

设计一套前端 「内存释放协议」，在主窗口隐藏时执行：

```javascript
// 窗口隐藏事件
window.__TAURI__.event.listen('tauri://blur', () => {
  // 1. 清空非首屏列表项的 innerHTML（保留数据结构）
  clipboardList.trimDom(10); // 只保留前 10 条 DOM

  // 2. 释放预览面板内容
  previewPanel.clearContent();

  // 3. 释放图片 Blob URL
  thumbCache.releaseAll();
});

// 窗口显示事件
window.__TAURI__.event.listen('tauri://focus', () => {
  // 恢复完整列表
  clipboardList.refresh();
});
```

**收益**：窗口隐藏期间 WebView JS 堆内存大幅减少  
**风险**：下次显示需要 ~50-100ms 重建列表  
**优先级**：🔴 P0

---

## 五、进程架构优化

### 5.1 WebKit 进程模型

**现状**：WebKitGTK 4.1+ 强制使用 `MULTIPLE_SECONDARY_PROCESSES` 模型（`set_process_model` 已在 2.40 废弃），每个 WebView 有独立 Web 进程。

这意味着：
- **主窗口**：1 个 Web 进程（常驻）
- **设置窗口**：打开时 +1 个 Web 进程（关闭后释放）  
- **每个 Pin 窗口**：+1 个 Web 进程

**优化方向**：`WebviewBuilder::with_related_view()` 可让多个 webview 共享 Web 进程。

```rust
// Pin 窗口共享主窗口的 Web 进程
let main_webview = app.get_webview_window("main").unwrap();
let pin_builder = WebviewWindowBuilder::new(...)
    .with_related_view(&main_webview);  // 需要验证 Tauri API
```

**收益**：每少一个 Web 进程节省 ~15-20MB  
**限制**：
1. Tauri 是否暴露 `with_related_view` 给 WebviewWindowBuilder（需验证 API）
2. 共享进程 → 一个 webview 崩溃影响全部
3. JS 全局作用域仍然隔离（不同 origin）

**优先级**：🟡 P1（当 Pin 功能高频使用时）

### 5.2 设置窗口延迟创建（已实现）

当前设置窗口已经是按需创建的（托盘菜单 "Settings" 点击时 `WebviewWindowBuilder::new()`，关闭后销毁），无需额外优化。

---

## 六、全部优化措施优先级矩阵

| # | 措施 | 层级 | 难度 | 预期 PSS 收益 | 风险 | 优先级 | 状态 |
|---|------|------|------|-------------|------|--------|------|
| 1 | CacheModel::DocumentViewer | WebKit | ⭐ | 5-15MB | 极低 | 🔴 P0 | ✅ 已实施 |
| 2 | 窗口隐藏时清理 WebKit 缓存 | WebKit | ⭐ | 3-8MB | 低 | 🔴 P0 | ✅ 已实施 |
| 3 | 前端隐藏时释放 DOM/资源 | 前端 | ⭐⭐ | 3-8MB | 低 | 🔴 P0 | ✅ 已实施 |
| 4 | Cargo `lto=true` + `opt-level="s"` + `panic="abort"` | 构建 | ⭐ | 1-3MB | 极低 | 🔴 P0 | ✅ 已实施 |
| 5 | 预览面板 lazy import() | 前端 | ⭐⭐ | 2-4MB | 低 | 🟡 P1 | ✅ 已实施 |
| 6 | WebKitMemoryPressureSettings | WebKit | ⭐⭐⭐ | 10-25MB | 中 | 🟡 P1 | 📋 需验证 API |
| 7 | removeUnusedCommands | 构建 | ⭐ | 0.5-2MB | 极低 | 🟡 P1 | ✅ 已实施 |
| 8 | 虚拟滚动 | 前端 | ⭐⭐⭐ | 2-8MB | 低 | 🟡 P1 | 📋 待实施 |
| 9 | Pin 窗口共享 Web 进程 | 架构 | ⭐⭐⭐ | 15-20MB/窗口 | 中 | 🟡 P1 | 📋 需验证 API |
| 10 | 缩略图缓存上限 | 前端 | ⭐ | 0.5-2.5MB | 极低 | 🟢 P2 | ✅ 已实施 |
| 11 | 替代分配器 (jemalloc/mimalloc) | Rust | ⭐⭐ | 1-3MB | 低 | 🟢 P2 | 📋 ROI 低 |

### 收益预估

| 场景 | 当前 PSS | 实施 P0 后 | 实施 P0+P1 后 |
|------|---------|-----------|-------------|
| **常驻（主窗口隐藏）** | ~63MB | ~45-52MB | ~35-45MB |
| **活跃（主窗口显示）** | ~63MB | ~55-60MB | ~48-55MB |
| **多窗口（主+设置+2Pin）** | ~100MB+ | ~85-95MB | ~65-80MB |

---

## 七、实施路线图

### 阶段 1：低垂果实（P0，1-2 天）

1. ✏️ `CacheModel::DocumentViewer` — 在 lib.rs setup() 的 with_webview 中添加
2. ✏️ Cargo profile 加强 — `lto=true`, `opt-level="s"`, `panic="abort"`
3. ✏️ 窗口隐藏时 clear_cache — 在 on_window_event Focused(false) 分支添加
4. ✏️ 前端隐藏释放协议 — 监听 blur 事件，trim DOM + 清理预览面板

### 阶段 2：结构优化（P1，3-5 天）

5. ✏️ preview-panel.js 改为 lazy import
6. ✏️ `removeUnusedCommands: true` — 同时精确化 capabilities
7. 🔬 评估 MemoryPressureSettings API 可达性
8. ✏️ 虚拟滚动初版

### 阶段 3：长期架构（P2，按需）

9. 🔬 Pin 窗口 with_related_view 验证
10. ✏️ 缩略图缓存上限
11. 🔬 jemalloc/mimalloc 对比测试

---

## 八、验证方法

### 8.1 内存测量脚本

```bash
#!/bin/bash
# measure-memory.sh — 测量 Clippy 各进程内存
PID=$(pgrep -f "com.clippy" | head -1)
if [ -z "$PID" ]; then
  echo "Clippy 未运行"
  exit 1
fi

echo "=== 主进程 PID=$PID ==="
echo "--- smaps_rollup ---"
sudo cat /proc/$PID/smaps_rollup | grep -E "Rss|Pss|Shared|Private|Swap"

echo ""
echo "--- status ---"
grep -E "VmRSS|VmSize|VmPeak|RssAnon|RssFile" /proc/$PID/status

echo ""
echo "=== WebKit 子进程 ==="
for WP in $(pgrep -f "WebKitWebProcess"); do
  echo "--- WebProcess PID=$WP ---"
  sudo cat /proc/$WP/smaps_rollup | grep -E "Rss|Pss" 2>/dev/null
done

for NP in $(pgrep -f "WebKitNetworkProcess"); do
  echo "--- NetworkProcess PID=$NP ---"
  sudo cat /proc/$NP/smaps_rollup | grep -E "Rss|Pss" 2>/dev/null
done
```

### 8.2 A/B 对比方法

1. 构建当前版本作为基线 → 运行 10 秒 → 采集内存
2. 应用一项优化 → 重新构建 → 运行 10 秒 → 采集内存
3. 记录差异到 changelog

### 8.3 关键指标

| 指标 | 目标 | 当前 |
|------|------|------|
| PSS（主窗口隐藏） | < 50MB | 63MB |
| PSS（主窗口显示） | < 60MB | 63MB |
| Pss_Anon（堆+栈） | < 20MB | 25MB |
| 首次显示延迟 | < 100ms | ~50ms |
| 设置窗口创建时间 | < 300ms | ~200ms |

---

## 九、参考资料

### 官方文档
- [WebKitGTK CacheModel 枚举](https://webkitgtk.org/reference/webkit2gtk/stable/enum.CacheModel.html) — DOCUMENT_VIEWER / WEB_BROWSER / DOCUMENT_BROWSER
- [WebKitGTK MemoryPressureSettings](https://webkitgtk.org/reference/webkit2gtk/stable/struct.MemoryPressureSettings.html) — memory_limit / thresholds / poll_interval
- [WebKitGTK WebContext](https://webkitgtk.org/reference/webkit2gtk/stable/class.WebContext.html) — set_cache_model / clear_cache / memory-pressure-settings 属性
- [WebKitGTK Settings](https://webkitgtk.org/reference/webkit2gtk/stable/class.Settings.html) — hardware-acceleration-policy / enable-* 系列属性
- [Tauri App Size 优化](https://v2.tauri.app/concept/size/) — Cargo profile / removeUnusedCommands
- [Tauri Process Model](https://v2.tauri.app/concept/process-model/) — Core Process + WebView Process 架构
- [mallopt(3)](https://man7.org/linux/man-pages/man3/mallopt.3.html) — MALLOC_ARENA_MAX / M_MMAP_THRESHOLD / M_TRIM_THRESHOLD

### Rust 分配器
- [tikv-jemallocator](https://crates.io/crates/tikv-jemallocator) — jemalloc Rust 绑定，#[global_allocator] 替换
- [mimalloc](https://docs.rs/mimalloc/latest/mimalloc/) — Microsoft mimalloc Rust 绑定

### 关键认知
- `#[global_allocator]` 只替换 Rust 主进程堆，不影响 WebKit 子进程（独立 ELF）
- `MALLOC_ARENA_MAX` 通过 `set_var` 设置后会被 fork+exec 的子进程继承
- WebKit `memory-pressure-settings` 是 **construct-only**，只能在 WebContext 创建时设置
- WebKit `set_process_model` 在 2.40+ 已废弃，强制多进程模型
- RSS 中 ~80% 是共享库映射，看 PSS 才是真实内存代价
