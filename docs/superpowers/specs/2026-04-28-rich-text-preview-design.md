# 富文本预览窗口设计文档

## 1. 概述

为 Clippy 剪贴板管理器增加富文本预览功能，当用户在列表中聚焦某个条目时，在主面板右侧悬浮显示该条目的渲染效果。

支持内容类型：
- **HTML 富文本**：安全渲染 HTML（禁止 JS/iframe/object/embed，允许 img/video）
- **代码语法高亮**：highlight.js（默认）/ Prism.js（可配置）
- **Markdown 渲染**：marked（默认）/ markdown-it（可配置）

---

## 2. 预览窗口 UI 方案分析

### 方案 A：独立 WebviewWindow

在主面板右侧创建一个独立的 Tauri `WebviewWindow`。

**优势：**
- Tauri 2.x 原生支持 `WebviewWindowBuilder`，已有 settings 窗口先例
- 完全独立的 WebView 进程，安全隔离好（HTML 渲染中的恶意内容不影响主面板）
- 独立 CSP 策略，可以为预览窗口设置更宽松的 img-src 规则
- 生命周期独立，关闭预览不影响主面板

**劣势：**
- **Wayland 定位限制**：Wayland 协议不允许客户端主动设置窗口绝对位置，`window.set_position()` 在 Wayland 上无效
- 两个窗口间有 gap，视觉不如一体化
- 窗口焦点管理复杂（点击预览窗口会导致主面板失去焦点）
- 创建/销毁 WebviewWindow 有开销

**Wayland 兼容性：** ❌ 严重问题。主面板本身用 `center: true`，Wayland 下也无法精确定位，所以预览窗口无法保证紧贴主面板右侧。

---

### 方案 B：扩展原窗口 + 内部分区

将主窗口从 380px 扩展到 380+400=780px（预览开启时），右半部分渲染预览内容。

**优势：**
- **单窗口**：无焦点争抢问题
- **定位一致**：预览区天然紧贴列表区域，无 gap
- **Wayland 完全兼容**：不涉及跨窗口定位
- **性能**：无需创建额外 WebviewWindow
- 动画流畅：窗口宽度过渡动画自然

**劣势：**
- 窗口宽度动态变化需要处理
- HTML 预览和主面板在同一个 WebView 中，安全需要在 JS 层通过 sanitizer 保证
- 扩大的窗口可能超出屏幕边缘（需要检测屏幕边界）

**Wayland 兼容性：** ✅ 完全兼容

---

### ✅ 推荐方案：方案 B（扩展原窗口）

**核心原因：**
1. Wayland 兼容性是硬约束（项目已支持 Wayland + gsettings 快捷键）
2. 单窗口焦点管理简单，UX 更流畅
3. 实现复杂度更低
4. 安全性通过前端 HTML sanitizer（如 DOMPurify）保证

---

## 3. 实现细节设计

### 3.1 窗口布局

```
┌─────────────────┬──────────────────────┐
│  剪贴板列表      │  富文本预览           │
│  (380px, 固定)   │  (400px, 可折叠)     │
│                  │                      │
│  [条目列表...]   │  [渲染后的内容...]    │
│                  │                      │
└─────────────────┴──────────────────────┘
```

- 窗口默认宽度 380px（预览关闭）
- 预览开启时窗口扩展到 780px
- 需要 Rust 端 `window.set_size()` 动态调整
- 新增 IPC 命令：`set_preview_visible(bool)` → 调整窗口尺寸

### 3.2 预览区 HTML 结构

```html
<div id="preview-panel" class="preview-panel hidden">
  <div class="preview-header">
    <span class="preview-type-badge">HTML</span>
    <span class="preview-size">1.2 KB</span>
  </div>
  <div class="preview-content">
    <!-- sanitized HTML / highlighted code / rendered markdown -->
  </div>
</div>
```

### 3.3 内容类型检测与渲染策略

| content_type | 渲染方式 |
|---|---|
| `html` | DOMPurify 消毒后插入 innerHTML |
| `text`（代码检测） | highlight.js / Prism.js 语法高亮 |
| `text`（Markdown 检测） | marked / markdown-it 渲染 |
| `text`（纯文本） | 保持 textContent，等宽字体 |
| `image` | 直接显示大图预览 |

**代码检测启发式：**
- 检查是否以 `#!` shebang 开头
- 检查常见代码模式：`function`, `def`, `class`, `import`, `{`, `<`, `=>` 等
- 或使用 highlight.js 的 `highlightAuto()` 自动检测

**Markdown 检测启发式：**
- 以 `# ` 开头，或包含 `## `、`- `、`* `、`1. `、`` ``` `` 等 Markdown 标记
- 在 highlight.js 检测之前优先检测 Markdown

### 3.4 安全架构

#### DOMPurify 配置
```javascript
const PURIFY_CONFIG = {
  ALLOWED_TAGS: ['h1','h2','h3','h4','h5','h6','p','br','hr','ul','ol','li',
                  'strong','em','u','s','del','ins','a','img','video','source',
                  'table','thead','tbody','tr','th','td','blockquote','pre','code',
                  'span','div','sub','sup','mark','abbr','details','summary'],
  ALLOWED_ATTR: ['href','src','alt','title','width','height','class','style',
                  'target','rel','controls','autoplay','loop','muted','type'],
  FORBID_TAGS: ['script','iframe','object','embed','form','input','textarea',
                'select','button','meta','link','base','applet'],
  ALLOW_DATA_ATTR: false,
};
```

#### 外部资源白/黑名单（设置页面配置）
- **默认允许**：`img[src]`, `video[src]` 的外部 URL
- **默认禁止**：`script`, `iframe`, `object`, `embed`, `link[rel=stylesheet]`
- 用户可在设置中配置域名白名单/黑名单
- CSP 更新：预览区需要 `img-src *`（用户可控范围内）

#### Tauri CSP 调整
当前：`img-src 'self' data: blob:`
需要：`img-src 'self' data: blob: https: http:`（允许外部图片）

### 3.5 依赖库

| 库 | 用途 | 大小 |
|---|---|---|
| DOMPurify | HTML 消毒 | ~7KB gzip |
| highlight.js | 代码高亮（默认） | ~40KB gzip (core+常用语言) |
| Prism.js | 代码高亮（可选） | ~6KB gzip (core+常用语言) |
| marked | Markdown 渲染（默认） | ~10KB gzip |
| markdown-it | Markdown 渲染（可选） | ~30KB gzip |

这些库通过 npm 安装，Vite 打包时 tree-shake。

### 3.6 配置项扩展（AppConfig）

```rust
// models.rs 新增字段
pub struct AppConfig {
    // ... 现有字段 ...
    #[serde(default = "default_preview_enabled")]
    pub preview_enabled: bool,           // 预览是否开启
    #[serde(default = "default_code_highlighter")]
    pub code_highlighter: String,        // "highlightjs" | "prismjs"
    #[serde(default = "default_markdown_renderer")]
    pub markdown_renderer: String,       // "marked" | "markdown-it"
    #[serde(default)]
    pub resource_whitelist: Vec<String>, // 外部资源域名白名单
    #[serde(default)]
    pub resource_blacklist: Vec<String>, // 外部资源域名黑名单
}
```

### 3.7 IPC 命令新增

- `set_preview_visible(visible: bool)` — 调整窗口宽度（380 vs 780）
- 不需要新的数据获取命令，前端已有 `getClips` 返回 `html_content`

---

## 4. 实现阶段

### Phase A：基础预览框架
1. 主窗口右侧预览区 HTML/CSS 布局
2. Rust `set_preview_visible` 命令（动态窗口宽度）
3. 前端聚焦条目时更新预览区内容
4. 纯文本预览（等宽字体）
5. 图片大图预览

### Phase B：HTML 安全渲染
1. 添加 DOMPurify 依赖
2. HTML 内容消毒 + 渲染
3. 外部资源过滤逻辑
4. CSP 调整

### Phase C：代码语法高亮
1. 添加 highlight.js + Prism.js 依赖
2. 代码检测启发式
3. 渲染引擎切换逻辑
4. 高亮主题适配 light/dark

### Phase D：Markdown 渲染
1. 添加 marked + markdown-it 依赖
2. Markdown 检测启发式
3. 渲染引擎切换逻辑
4. Markdown CSS 样式

### Phase E：设置页面 + 资源管理
1. 设置页面新增配置项：预览开关、高亮引擎、Markdown 引擎
2. 白名单/黑名单 UI
3. AppConfig 扩展 + 持久化
4. 运行时引擎切换

---

## 5. 安全清单

- [ ] DOMPurify 消毒所有 HTML 内容
- [ ] 禁止 script/iframe/object/embed 标签
- [ ] 外部图片/视频允许但可配置白黑名单
- [ ] 不执行任何剪贴板中的 JavaScript
- [ ] CSP 限制预览区域的能力
- [ ] URL sanitize（防止 javascript: 协议）
