# 全项目审计基线（2026-09-04）

## 范围与事实基线

- 基线提交：`c9e3d68`（`release: v0.1.20`），分支 `dev`。
- 已通过的自动化：Rust 441 项通过、10 项忽略；前端 839 项通过；DOM、Canvas 像素与布局
  smoke 通过；Linux/Windows/macOS 发布流水线通过。
- 已通过的实机路径：GNOME Wayland、两块混合缩放显示器连续截图；覆盖层首帧约
  0.56–0.87 秒，没有黑屏、缺帧或 WebKit 崩溃。
- 当前构建产物已清理，`target/` 与前端 `dist/` 均不存在；后续验证会重新产生，结束时再清理。

## 功能—实现—证据矩阵

| 功能 | 主要实现 | 当前证据 | 仍需验证 |
| --- | --- | --- | --- |
| 剪贴板监听/写回 | `clipboard_watcher*`、`clipboard_content*`、`clipboard_writer*` | Rust 单测、前端 IPC 测试 | 长时间图片轮询 CPU/RSS |
| 历史、搜索、收藏、删除 | `storage*`、`commands/clipboard.rs`、React `main/` | SQLite/FTS 单测、React 测试 | 大库查询延迟 |
| 图片缩略图/预览 | `get_clip_thumbnail`、`get_clip_image`、`ClipboardRow.tsx` | 缓存边界与回归守卫 | 并发冷缓存的 runtime 阻塞 |
| 快捷键/托盘/窗口 | `gsettings_shortcuts*`、`portal_shortcuts*`、`tray_icon*`、`window*` | Rust 单测、Linux 实机 | Windows/macOS 实机边界 |
| 自动粘贴 | `paste*`、主窗口 React facade | Rust/前端测试 | 各桌面目标应用实机 |
| 截图/选区/编辑 | `screenshot*`、`capture*`、React `capture/` | 像素测试、Wayland 双屏实机 | X11、Windows、macOS 实机 |
| Pin/画布/保存 | `pin*`、React `pin/` | Rust 往返/像素测试、React 测试 | 重复开关 Pin 的 PSS 曲线 |
| OCR/翻译 | `ocr*`、`translation*` | Rust/前端测试 | 外部服务与本机 OCR 可用性 |
| 设置/更新/发布 | `config*`、`updater*`、CI workflow | 单测、v0.1.20 全平台发布 | 各平台安装与更新实机 |

## 已确认问题

### P1：冷缩略图在异步执行线程内同步解码

`get_clip_thumbnail` 虽声明为 `async`，但数据库读取、全尺寸 PNG 解码、缩放和编码都直接在
Tauri async runtime worker 上执行。一次 2560×1600 解码约几十毫秒，打开含多张图片的列表时
会并发占满 worker，影响其他异步命令响应。应将整段阻塞工作移入 `spawn_blocking`，同时限制
冷解码并行度，避免宽 blocking pool 把几十张全屏 PNG 同时展开成 RGBA；命中缓存继续零调度返回。

### P1：导入可编辑 PNG 后常驻两份同源压缩数据

`PinSource::Project` 同时保留 `source_png: Vec<u8>` 和 `PinProject.source.png_base64: String`。
base64 比原始 PNG 多约 33%，因此每张打开的工程在预览图和 WebView 之外又常驻一份重复原图。
运行时只需要工程元数据与文档；保存时可从不可变 `source_png` 临时重建完整工程。应删除常驻
base64 副本，并以往返、原图哈希与合成像素测试证明不损失画质或工程数据。

### P1：未编辑覆盖层仍常驻同尺寸 Canvas 副本

资源协议已把冻结帧解码为原生 `<img>`，但首帧随后仍通过 `drawScene` 复制进同尺寸 Canvas 才显示。
3840×2160 与 2560×1600 两屏的 RGBA Canvas 合计约 49 MiB，而且复制位于用户等待覆盖层出现的
路径。未编辑时直接挂载已解码图片，只有首次标注或调色才显示 Canvas；Rust 权威合成与帧数据不变。

## 已检查且有界的生命周期

- 后端缩略图缓存固定 64 条；Pin 原始位置注册表固定 16 条。
- Pin 条目内容位于 `Arc<PinSource>`，滚轮缩放不复制大 PNG；窗口销毁时 manager、清晰化槽位和
  placement generation 均会清理。
- React Pin 图片对象 URL 在替换和卸载时撤销；主列表失焦会清空数据快照与搜索定时器。
- 截图会话与后台清晰化任务带 generation/cancel 防护，不会把旧结果写入新窗口。
- 链接预览缓存原先只在读取时忽略 7 天前的记录，却不删除旧行；现改为写入时清除过期行并保留
  最近 512 条，避免长期复制不同链接造成 SQLite 文件单向增长。

以上只是第一轮静态审计结论；压力实验与其余模块 findings 在验证后继续补充。
