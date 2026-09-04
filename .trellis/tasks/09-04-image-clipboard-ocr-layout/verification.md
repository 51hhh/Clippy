# 验证记录

日期：2026-09-04

## 结果

- 主窗口恢复到 `11bacf8^` 的入口设计：不再渲染“打开图片”按钮，也不再拦截
  `Ctrl/Cmd+O`。仅删除前端入口；Rust 可编辑 PNG 工程读写实现保留。
- `clipboard_watcher.rs` 的图片路径保持不变：`get_image` → PNG 编码 →
  `insert_clip(ContentType::Image)` → `emit("clip-added")`，复制图片会自动进入队列。
- 图片和 `.preview-ocr-result` 都是 `#preview-content` 的直接子节点；图片在前，OCR 在后。
  OCR 不再设置 `max-height` 或 `overflow`，外层 `.preview-content` 是唯一纵向滚动容器。

## 自动验证

- `npx vitest run`：45 个文件、852 项测试通过。
- `npx tsc --noEmit`：通过。
- `npx vite build`：1889 modules，生产构建通过。
- `./scripts/smoke-layout.sh`：真实 Firefox 布局 smoke 通过，结果像素 `0 208 0`。
- `cargo fmt --check`：通过；本次没有修改 Rust 源码，图片 watcher 与工程 PNG 命令保持原样。
- `git diff --check`：通过。

## 浏览器几何证据

在 1200×900 图片和 80 行 OCR 文本的真实布局中：

- 外层预览：`overflow-y: auto`，`clientHeight=304`，`scrollHeight=2065`。
- OCR：`overflow-y: visible`，`max-height: none`，`clientHeight=scrollHeight=1680`。
- 图片节点位于 OCR 节点之前；外层滚至底部后 OCR 末尾进入可视范围。

## Review

- 用户内容仍由 `createElement`/`textContent` 渲染，未增加 XSS 面。
- 没有新增 IPC；删除的 `openPinImageDialog` 包装只服务于已移除的主界面入口。
- 未改动图片监听、OCR 调用、Pin 保存、iTXt 工程格式、列表虚拟化或翻译面板。
- CSS 由普通文档流和单一滚动所有权解决问题，不增加滚动事件监听或运行时布局计算。
