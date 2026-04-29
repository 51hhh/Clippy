# 贴图 Pin-to-Desktop + OCR 文字识别

## Goal

为 Clippy 剪贴板管理器新增两大功能：
1. **贴图 (Pin to Desktop)**：将剪贴板历史中的条目一键钉到桌面，作为 always-on-top 浮动窗口
2. **OCR 文字识别**：对剪贴板中的图片提取文字，支持中英文

## 已确认需求

### 贴图 (Pin to Desktop)

**视觉**：无边框 + 阴影 + 圆角（类 PixPin 风格）

**支持内容类型**（MVP）：
- 图片（PNG BLOB → 渲染到 img 标签）
- 纯文本（等宽字体 / 自动换行）

**交互能力**：
- 拖拽移动（自定义 titlebar 区域 / 全窗口可拖拽）
- 滚轮缩放（缩放内容）
- Ctrl+滚轮调节透明度
- 右键菜单（复制 / 关闭 / 锁定位置）
- 双击关闭

**窗口属性**：
- `decorations: false`
- `always_on_top: true`
- `transparent: true`（用于圆角 + 阴影）
- `skip_taskbar: true`
- 多个贴图窗口可共存

### OCR 文字识别

**引擎**：内嵌 Tesseract（`leptess` crate 静态链接 Tesseract + Leptonica）

**语言支持**：英文 (eng) + 中文 (chi_sim)，训练数据内嵌到二进制

**结果处理**（用户可在设置中配置）：
- 模式 1：直接复制到系统剪贴板
- 模式 2：显示在预览面板中，可选择性复制

**触发入口**：
- 剪贴板列表的图片条目 → 行内 OCR 按钮
- 预览面板中 → OCR 按钮

## 技术方案

### Pin to Desktop

**后端**：
- `commands.rs`：新增 `pin_clip(id)` / `close_pin(window_label)` / `list_pins()` 命令
- 动态创建 `WebviewWindow`，label 格式 `pin-{id}`，加载 `pin.html?id={id}`
- 在 `AppState` 中不需要额外跟踪——Tauri 的 `app.webview_windows()` 已可枚举

**前端**：
- 新增 `src/pin.html` + `src/js/pin.js`
- pin.js：从 URL params 取 clip ID → IPC 获取条目 → 渲染内容
- 交互：JS 事件处理（wheel zoom, ctrl+wheel opacity, dblclick close, 右键菜单）
- 拖拽：`data-tauri-drag-region` 或手动 `startDragging()`

**主窗口集成**：
- `clipboard-list.js`：每个条目增加 📌 Pin 按钮
- `api.js`：导出 `pinClip(id)` / `closePin(label)` 函数

### OCR

**后端**：
- Cargo.toml：添加 `leptess` 依赖（静态链接模式）
- 新增 `ocr.rs` 模块：封装 Tesseract 初始化 + 识别逻辑
- `commands.rs`：新增 `ocr_image(id)` 命令，返回识别文本
- 训练数据：`tessdata/` 目录嵌入 `eng.traineddata` + `chi_sim.traineddata`

**前端**：
- `preview-panel.js`：图片预览 tab 增加 OCR 按钮
- `clipboard-list.js`：图片条目行内增加 OCR 图标
- OCR 结果区：可选择文字 + 一键复制

**配置**：
- `AppConfig` 新增 `ocr_result_mode: String`（"clipboard" | "preview"）
- 设置面板增加 OCR 结果处理方式选项

## Acceptance Criteria

- [ ] 点击 Pin 按钮后，桌面出现浮动窗口，正确显示对应内容（文本/图片）
- [ ] Pin 窗口始终置顶，无边框，有阴影和圆角
- [ ] 支持拖拽移动、滚轮缩放、Ctrl+滚轮透明度、双击关闭、右键菜单
- [ ] 可同时存在多个 Pin 窗口
- [ ] 关闭 Pin 窗口后资源正确释放
- [ ] OCR 可正确识别图片中的英文和中文
- [ ] OCR 结果支持配置模式（直接复制 / 预览面板展示）
- [ ] OCR 按钮仅在图片类型条目上显示
- [ ] 编译通过，clippy 无警告
- [ ] i18n 覆盖新增 UI 文字

## Out of Scope

- HTML 类型贴图（后续迭代）
- Pin 窗口持久化（重启恢复位置）
- 图像处理（裁切、旋转等）
- 表格 / 二维码 / 公式识别
- 多语言训练数据动态下载

## Technical Notes

- 现有窗口创建参考：`lib.rs` 中 settings 窗口的动态创建逻辑
- ClipItem 已有 `image_data: Option<Vec<u8>>` 字段（PNG 格式）
- 存储层已有 `get_clip_image(id)` 和 `get_clip_by_id(id)` 方法
- Tauri v2 transparent window 需要 CSS `background: transparent` + `html, body { background: transparent }`
- `leptess` 的 `LEPTESS_TESSDATA_DIR` 环境变量指定训练数据路径
- tessdata 文件体积：eng ~4MB, chi_sim ~45MB（fast 版本 ~2MB / ~17MB）
