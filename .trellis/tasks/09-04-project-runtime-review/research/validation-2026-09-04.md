# 验证记录（2026-09-04）

## 自动化门禁

- `cargo fmt --check`：通过。
- `cargo check --all-targets`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- 主窗口位置 debounce 专项：7 项通过；500 次连续 worker 申请只允许首个创建线程，释放后才允许
  下一轮接管。
- `cargo test`（沙箱外，允许 loopback）：444 通过、0 失败、10 个真实桌面/手动性能探针忽略；
  覆盖层变更后又单独复跑冻结帧协议 2 项，全部通过。
- `npx tsc --noEmit`：通过。
- `npx vitest run`：44 个文件、840 项通过。
- GNOME 扩展静态检查：Shell 45–51 通过。
- DOM/Xvfb：9 项通过。
- Canvas 导出像素 smoke：通过，探针像素 `0 208 0`。
- 主窗口布局像素 smoke：通过，探针像素 `0 208 0`。
- Vite production build：通过（1888 modules）。

受限沙箱内的 16 个翻译测试因无法绑定本机 loopback 失败，Canvas/Layout smoke 也因无法连接
Xvfb/Vite 本地端口失败；同一提交在沙箱外全部通过，因此这些不是产品回归。

## 性能基线

本机 Criterion `--quick`：

| 路径 | 时间 |
| --- | ---: |
| 剪贴板首屏 50 条 / 2000 条库 | 23.05–23.28 µs |
| FTS 前缀搜索 | 308.65–314.48 µs |
| 两字符 LIKE 回退 | 38.67–38.95 µs |
| FTS 无命中 | 282.13–282.48 µs |
| 1 MiB SHA-256 | 1.55–1.56 ms |
| 100 KiB 最坏敏感词检测 | 29.90–30.60 µs |
| 1 MiB HTML 去标签 | 1.90–1.92 ms |
| 1080p RGBA 指纹 | 1.53–1.57 ms |
| 1080p PNG 编码 | 79.04–79.36 ms |
| 1080p PNG 完整校验 | 20.49–21.12 ms |
| 1080p base64 解码 | 1.273–1.274 ms |
| 1440p PNG 完整校验 | 36.32–38.10 ms |

结论：列表/搜索 SQLite 路径远低于 1 ms，不是当前交互瓶颈；图片全尺寸解码与编码才应隔离出
async worker 和 UI 热路径。连续切换不同 Criterion target 时 Cargo 偶发复用到 release
`panic=abort` 产物并报 unwind 不兼容；对单个包执行 `cargo clean --profile bench -p clippy-app`
后可复现地运行。这是开发工具链低优先级问题，不影响应用或 CI 构建。

## 内存基线与静态生命周期

- 已安装 v0.1.20 在 GNOME Wayland 连续运行约 4 小时后：主进程 PSS 109.0 MiB、NetworkProcess
  11.6 MiB、主 WebProcess 94.2 MiB，总 PSS 约 214.8 MiB。
- 重新启动后、打开额外 WebView 的现场快照：主进程 96.5 MiB、NetworkProcess 10.3 MiB、主
  WebProcess 131.2 MiB、额外 WebProcess 34.6 MiB，总 PSS 约 272.6 MiB。该快照只能证明当前
  工作集，不把用户主动打开的第二窗口误判为泄漏。
- 后端缩略图缓存固定 64 条；原点注册表固定 16 条；代码高亮结论缓存固定 200 条。
- 冷缩略图使用异步 Semaphore 限制为两路全尺寸 PNG 解码；等待任务不占 Tauri async worker，
  从而同时约束响应线程饥饿与 RGBA/CPU 峰值。
- 主列表失焦清空条目数组和搜索 timer；Pin 关闭会移除 manager 条目、placement generation 和
  清晰化结果；对象 URL 在替换/卸载时撤销。
- 主窗口移动事件共享一份最新坐标与单一 debounce worker；连续拖动不再按事件创建 OS 线程，
  worker 在最后一次移动 300 ms 后落盘，并在退出竞态中用 CAS 把新一轮工作唯一交接出去。
- 本轮把导入工程的常驻表示从“原图 PNG + 同源 base64 + 合成预览”改为“原图 PNG + 轻量元数据
  + 合成预览”。节省量精确为 `4 * ceil(source_png_len / 3)` 附近的 base64 字符串容量，具体取决
  于分配器容量取整；磁盘工程仍保持自包含。
- 截图覆盖层的原生资源协议已经返回浏览器解码后的冻结帧。无标注/调色的首帧现在直接挂载该
  `<img>`，Canvas 保持浏览器默认 300×150 backing store；首次真正合成时才按冻结帧原生像素分配。
  3840×2160 + 2560×1600 两屏避免提前分配 49,111,040 字节（约 46.84 MiB）的 Canvas 像素，
  同时省掉首次 `drawImage`。切入合成使用 `useLayoutEffect`，避免先隐藏原图再出现空白帧。

## 现场截图时序

- 已安装 v0.1.20 的 GNOME Wayland 冷启动双屏首轮仍为 1.72/2.39 秒，第二轮 1.40/2.12 秒；
  其中冻结帧 375–576 ms、建窗/WebView 561–696 ms，4K 前端首绘 1.11–1.18 秒，是本次继续优化
  原生图片直显路径的直接依据。
- 同一进程预热约 5 分钟后，双屏覆盖层降到约 0.55/0.81 秒（冻结帧 195 ms、建窗 127 ms、
  前端绘制 226/487 ms）。日志未出现新崩溃，但扩展屏冷首帧成本依旧显著。
- 隔离 Xvfb 中默认快捷键可以注册并触发；虚拟服务器没有 Mutter/wlroots/Portal 输出，最终在
  `xcap` 捕获前置阶段返回“无法捕获显示器”，因此它不能替代真实 GNOME Wayland 的整链路截图。
  覆盖层显示、Canvas 切换及像素导出由 840 项前端测试与三组 Xvfb 冒烟继续兜底。

## 仍需实机覆盖

- 未中断用户当前运行的正式实例，因此本轮未取得“本次未提交二进制”在真实 GNOME Wayland
  连续 Pin/截图后的独立 PSS 曲线；新路径需要下一份安装包继续采集冷/热时序。
- Rust 权威冻结帧、裁剪、标注、保存与 Pin 渲染器都未修改；协议尺寸、原图哈希、工程往返和
  合成像素测试继续通过，因此首帧直显不改变保存/复制/Pin 的原生分辨率。
- Windows/macOS 行为以同代码 CI 为证，实际桌面交互仍应在下一次 native QA 构建中复核。
