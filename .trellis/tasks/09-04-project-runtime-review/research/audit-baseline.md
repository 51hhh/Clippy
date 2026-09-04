# 全项目审计基线（2026-09-04）

## 范围与事实基线

- 基线提交：`c9e3d68`（`release: v0.1.20`），分支 `dev`。
- 发布基线自动化：Rust 441 项通过、10 项忽略；前端 839 项通过；DOM、Canvas 像素与布局
  smoke 通过；Linux/Windows/macOS 发布流水线通过。当前 `f8ac057` 本机回归为 Rust 447 项通过、
  10 项忽略，前端 848 项通过；核心代码 `d702407` 的 CI Check 与全平台 Native QA 均成功。
- 已通过的实机路径：GNOME Wayland、两块混合缩放显示器连续截图；覆盖层首帧约
  0.56–0.87 秒，没有黑屏、缺帧或 WebKit 崩溃。
- 当前构建产物已清理，`target/` 与前端 `dist/` 均不存在；后续验证会重新产生，结束时再清理。

## 功能—实现—证据矩阵

| 功能 | 主要实现 | 当前证据 | 仍需验证 |
| --- | --- | --- | --- |
| 剪贴板监听/写回 | `clipboard_watcher*`、`clipboard_content*`、`clipboard_writer*` | Rust 单测、前端 IPC 测试 | 长时间图片轮询 CPU/RSS |
| 历史、搜索、收藏、删除 | `storage*`、`commands/clipboard.rs`、React `main/` | SQLite/FTS 单测、React 测试 | 大库查询延迟 |
| 图片缩略图/预览 | `get_clip_thumbnail`、`ClipboardRow.tsx`、`clipboardVirtualization.ts` | 两路冷解码上限、10,000 条窗口化、屏外释放、Firefox 行高 | WebKit 大图库滚动 PSS/帧时间 |
| 快捷键/托盘/窗口 | `gsettings_shortcuts*`、`portal_shortcuts*`、`tray_icon*`、`window*` | Rust 单测、Linux 实机、移动事件压力守卫 | Windows/macOS 实机边界 |
| 自动粘贴 | `paste*`、主窗口 React facade | Rust/前端测试 | 各桌面目标应用实机 |
| 截图/选区/编辑 | `screenshot*`、`capture*`、React `capture/` | 像素测试、Wayland 双屏实机 | X11、Windows、macOS 实机 |
| Pin/画布/保存 | `pin*`、React `pin/` | Rust 往返/像素测试、React 测试、GNOME 五图早到/晚到协议实测 | Pin 关闭后的 PSS 回收曲线 |
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

### P2：拖动主窗口按事件创建 debounce 线程

`WindowEvent::Moved` 每次都创建一个 OS 线程并睡眠 300 ms，只靠 generation 让旧线程醒来后退出。
60 Hz 连续拖动会稳定叠加约 18 个睡眠线程及其栈保留。改为所有事件只覆盖最新坐标并推进代次，
整个拖动周期由唯一 worker 等待静默窗口、保存最终坐标；停止与新事件交界处用原子 CAS 交接。

### 已修复：Pin 显示图的 base64 常驻与瞬时复制

第一步已让 Blob URL 建好后立即从 React 状态丢弃 `PinPayload.imageBase64`；后续 `e7c693b` 又把
首图与迟到补偿图都迁到按 WebView label 隔离的 `pin-frame` 协议。revision=0 直接取首图，revision=1
只取晚到补偿图，响应接管 `Vec<u8>` 所有权，不再经过 Rust base64、JSON、JS 解码与 Blob URL。
画布、复制、保存继续单独向后端取 canonical source，不使用屏上补偿图。

### 已修复：10,000 条历史全部常驻 DOM

仅用 `content-visibility` 会跳过屏外绘制，却仍让 React 和 DOM 保留全部行；压力探针中一次挂载
10,000 行约 5.76 秒并增加约 2.03 GiB jsdom 堆。现在根据文本/图片 77/87 px 行高计算前后 padding，
只挂载视口前后 320 px 的窗口；中段滚动、末行键盘焦点、无限分页和 ARIA 总数/序号均有回归覆盖。
真实 Firefox 已确认固定高度与实际内容不溢出，WebKit PSS/帧时间仍作为实机验证边界保留。

## 已检查且有界的生命周期

- 后端缩略图缓存固定 64 条；Pin 原始位置注册表固定 16 条。
- Pin 条目内容位于 `Arc<PinSource>`，滚轮缩放不复制大 PNG；窗口销毁时 manager、清晰化槽位和
  placement generation 均会清理。
- React Pin 只持有 `pin-frame` URL；主列表失焦会清空数据快照与搜索定时器，窗口化列表只挂载
  视口附近行，屏外图片行同时释放缩略图 base64 与解码纹理。
- 截图会话与后台清晰化任务带 generation/cancel 防护，不会把旧结果写入新窗口。
- 链接预览缓存原先只在读取时忽略 7 天前的记录，却不删除旧行；现改为写入时清除过期行并保留
  最近 512 条，避免长期复制不同链接造成 SQLite 文件单向增长。

## 已评估、暂不在本轮冒险改动

- Pin 的 label/revision 本地资源协议已按上述约束落地，并由真实 GNOME 五图验证覆盖早到/晚到
  两条补偿路径；这项不再是剩余风险。
- `CaptureManager::render_input` 会在提交动作时复制被选显示器的完整 RGBA，随后才在 blocking worker
  裁选区并合成；4K 单屏的短暂重复约 31.6 MiB。直接提前 `take` 会破坏“提交失败仍能唯一认领并
  恢复会话”以及并发新截图的竞态约束，因此需要新增 finalizing 状态机和压力测试后再改。它不在
  覆盖层首帧路径上，也不是常驻泄漏，本轮不以未经验证的生命周期重构换取峰值数字。

以上结论已覆盖主要缓存、线程、监听器和大对象生命周期，并已补测当前提交的真实 GNOME 双屏
截图、0/5/10 轮 PSS 与 10,000 条窗口化列表。剩余风险集中在 WebKit 实机长时间图片滚动与
Pin 关闭后 PSS 曲线；不把 CI 构建成功等同于 Windows/macOS 实际桌面交互已经通过。
