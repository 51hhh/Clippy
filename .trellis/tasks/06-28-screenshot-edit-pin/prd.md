# 截图编辑与贴图融合

## Goal

在 Clippy 中融合 flashot 的截图能力，提供区域截图、基础图像编辑、复制、保存和贴图工作流，并保持现有剪贴板管理功能稳定。

## Requirements

- 引入 React/TypeScript 功能岛，仅用于截图编辑入口，不重写现有剪贴板主界面。
- 新增截图入口页面，可从前端直接打开截图编辑器。
- 后端支持 Linux 下捕获当前屏幕并返回 PNG/base64 数据。
- 前端支持在截图上进行基础区域选择和图像编辑。
- 图像编辑至少包含亮度、对比度、饱和度、灰度调节。
- 编辑后的图片可复制到系统剪贴板。
- 编辑后的图片可保存为 PNG 文件，默认写入 `~/Pictures/Clippy`。
- 编辑后的图片可作为临时图片贴到桌面，复用现有 pin 窗口交互能力。
- 可直接复制 flashot 的 MIT 代码或实现思路；不引入 translator 的 GPL 代码。
- 新增依赖和构建配置必须纳入本地 CI。

## Acceptance Criteria

- [x] `src/capture.html` 可以通过 Vite 构建。
- [x] 截图页面能加载当前屏幕截图。
- [x] 用户可以框选截图区域，未框选时默认使用整张截图。
- [x] 用户可以调整灰度、亮度、对比度、饱和度并预览结果。
- [x] Copy 可把编辑后的 PNG 写入系统剪贴板。
- [x] Save 可将编辑后的 PNG 保存到 `~/Pictures/Clippy` 并返回路径。
- [x] Pin 可创建 always-on-top 的临时图片贴图窗口。
- [x] 原有 `pin_clip` 剪贴板贴图仍可工作。
- [x] `cargo check`、`cargo test`、`cargo clippy -- -D warnings` 通过。
- [x] `npm test`、`npx vite build` 通过。

## Out of Scope

- 不实现滚动截图。
- 不实现活动窗口自动识别截图。
- 不实现完整 flashot 标注工具集（箭头、文本、马赛克等可后续追加）。
- 不做现有主界面 React 全量迁移。
- 不复制 translator GPL 代码。
