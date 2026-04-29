# 图片剪贴板支持：监听 + 存储 + 渲染 + 回写

## Goal
让 Clippy 能够捕获、存储、展示和回写图片类型的剪贴板内容。同时增加悬浮预览窗口，支持图片和富文本条目的预览。

## 分期规划

### Phase 1 (P0) — 图片完整链路 ← 本任务
1. **后端：图片捕获** — watcher 中调用 `arboard::Clipboard::get_image()`，RGBA → PNG 编码后存储
2. **后端：图片回写** — `select_clip` 对 image 类型调用 `clipboard.set_image()`
3. **后端：新增 `get_clip_image` 命令** — 按 id 返回图片 base64（列表查询中 image_data 为 NULL 性能优化）
4. **前端：图片行渲染** — 列表中对 image 类型显示缩略图 + 类型图标
5. **前端：分类 tab** — 在 segment-tabs 中增加 All / Text / Image 过滤

### Phase 2 (P1) — 悬浮预览窗口
- 独立 Tauri 窗口，定位在主窗口右侧
- hover 或键盘导航时触发显示
- 图片显示原图预览；文本显示完整内容

### Phase 3 (P2) — 富文本/HTML 支持 → 新 trellis 任务
- HTML 剪贴板捕获（需 platform-specific clipboard API）
- 富文本渲染（代码语法、Markdown、HTML）
- 安全策略（外部资源白黑名单）

## 技术设计

### 图片捕获流程
```
arboard::get_image() → ImageData { width, height, bytes: Vec<u8> (RGBA) }
  → image crate 编码为 PNG bytes
  → SHA-256 哈希（基于 PNG bytes）
  → storage.insert_clip(ContentType::Image, None, None, Some(&png_bytes), &hash, png_bytes.len())
  → app.emit("clip-added", &clip)  // 注意：clip 中 image_data = png bytes，前端序列化为 base64
```

### 图片回写流程
```
select_clip(id) → get_clip_by_id(id) → clip.image_data: Some(png_bytes)
  → image crate 解码 PNG → RGBA raw pixels
  → arboard::Clipboard::set_image(ImageData { width, height, bytes })
```

### 前端图片渲染
- 列表查询（get_clips）不返回 image_data（性能）
- 列表行对 image 类型显示占位缩略图 + "Image · {size}" 标签
- 新增 IPC `get_clip_image(id)` → 返回 base64 PNG，用于预览窗口
- 图片缩略图：后端生成缩略图 or 前端 lazy load 完整图 → Phase 2 优化

### 依赖变更
- 新增 `image` crate（PNG 编解码）

### 预览窗口可行性分析

**方案 A：独立 Tauri 窗口**
- 优点：真正悬浮、可定位在主窗口任意方向、背景透明天然支持、不影响主窗口布局
- 缺点：窗口间通信需要 IPC 事件、快速导航时可能闪烁、窗口管理复杂度更高
- 实现：`WebviewWindowBuilder::new(app, "preview", ...)` + `decorations(false)` + `transparent(true)` + `always_on_top(true)`

**方案 B：扩大主窗口 + 透明区域**
- 优点：单窗口状态管理简单、无窗口间同步
- 缺点：透明区域可能无法点击穿透（Tauri 限制）、Linux WM 行为不一致、窗口尺寸频繁变化体验差
- 结论：在 Linux 上透明区域 click-through 不可靠，不推荐

**推荐：方案 A（独立窗口）**
- 先实现图片核心链路，预览窗口作为 Phase 2 独立实现

## 存储策略
- 图片默认不限大小
- 可在设置中配置 `max_image_size_mb`（0 = 无限制）
- 图片存储在 SQLite BLOB 中（与设计文档一致）
- 超出历史上限时按现有 `cleanup_old_entries` 逻辑清理

## Acceptance Criteria
- [ ] 截图/复制图片后，Clippy 列表中出现图片条目（带缩略图）
- [ ] 选中图片条目后，图片正确写回系统剪贴板
- [ ] 图片条目支持收藏、删除操作
- [ ] 文本条目功能不受影响
- [ ] `cargo test` 通过
- [ ] `cargo clippy -- -D warnings` 通过
