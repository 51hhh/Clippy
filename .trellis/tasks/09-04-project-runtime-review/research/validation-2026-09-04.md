# 验证记录（2026-09-04）

## 自动化门禁

- `cargo fmt --check`：通过。
- `cargo check --all-targets`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- 主窗口位置 debounce 专项：7 项通过；500 次连续 worker 申请只允许首个创建线程，释放后才允许
  下一轮接管。
- Pin React/手势专项及回归守卫：3 个文件、87 项通过；显示图改走带 revision 的原生资源 URL，
  后续缩放不重建 URL，早到/晚到清晰度补偿的交接状态均有单测。
- `cargo test`（沙箱外，允许 loopback）：447 通过、0 失败、10 个真实桌面/手动性能探针忽略；
  新增 `pin-frame` revision 解析 2 项与补偿槽所有权交接回归覆盖。
- `npx tsc --noEmit`：通过。
- 列表行为与窗口化专项：2 个文件、9 项通过；图片行在进入 `160px` 预取区前不请求缩略图，
  进入后加载、离开后释放；10,000 条混合行高列表只挂载不足 20 行，滚到中段与键盘跳到末行
  均保留真实索引及无障碍位置。
- `npx vitest run`：45 个文件、848 项通过。
- GNOME 扩展静态检查：Shell 45–51 通过。
- DOM/Xvfb：9 项通过。
- Canvas 导出像素 smoke：通过，探针像素 `0 208 0`。
- 主窗口布局像素 smoke：通过，探针像素 `0 208 0`；真实 Firefox 同时确认文本行 77 px、
  图片行 87 px，完整内容没有溢出固定行高。
- Vite production build：通过（1889 modules）。
- `449571e` 三平台 CI：Windows 与 macOS 原生 `check/clippy/test` 通过；Ubuntu 22.04 的格式、
  PipeWire 快路径基线、`check/clippy/test`、GNOME 扩展、前端测试、DOM/Xvfb、类型检查与生产构建
  全部通过（Actions `33850790082`）。
- `449571e` 原生 QA：Linux x64 的 deb/AppImage 构建、PipeWire 动态链接与校验通过；Windows x64
  自签名 NSIS/MSI 通过；macOS Intel 与 Apple Silicon 的 Ad-Hoc 签名包通过；Ubuntu 24.04
  下载同一份 Ubuntu 22 AppImage 并完成 X11 可视运行 smoke（Actions `33850955167`）。
- `158da7f` 文档收口提交的 CI Check 已通过（Actions `33853135642`）。
- 核心代码提交为 `e7c693b`（Pin 原生资源协议）与 `d702407`（屏外缩略图释放）；本机完整预检
  已通过。CI Check `33854814443` 的 Ubuntu 22.04、Windows、macOS 三组门禁全部成功；Native QA
  `33854884030` 的 Linux x64、Windows x64、macOS Intel、macOS Apple Silicon 构建，以及
  Ubuntu 24.04 已打包 AppImage 可视运行 smoke 全部成功。
- 本地验证结束后执行 `cargo clean`，删除 18,059 个文件、14.5 GiB；根目录 `dist/` 同步删除，
  两个构建输出路径均确认不存在。

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

长列表专项（同机 Node/jsdom，用于比较 React/DOM 规模而非替代 WebKit 实机数字）：旧实现一次挂载
10,000 个完整 `ClipboardRow` 约 5.76 秒，堆增量约 2.03 GiB；窗口化后同一 10,000 条组件测试只
挂载不足 20 行，单项测试约 72 ms。后者还覆盖中段滚动和焦点直接跳到第 10,000 条。

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
- Pin 显示图不再进入 JSON/JS：`pin-frame` 响应直接接管补偿 PNG 的 `Vec` 所有权；初始 URL
  revision=0，补偿晚到事件只带 revision=1。由此同时去掉 Rust base64、JSON 字符串、JS
  `atob`/`Uint8Array` 与 Blob URL 的瞬时链路；复制、保存和画布仍读取 canonical source。
- 列表允许配置到 10,000 条，但 React 现在只挂载视口前后 320 px 的窗口；前后 padding 保留完整
  滚动几何，文本/图片 77/87 px 行高由真实 Firefox 锁定。屏外图片行卸载时会取消/释放缩略图
  base64 与解码纹理，`content-visibility:auto` 继续减少 overscan 行的布局/绘制。
- 本轮把导入工程的常驻表示从“原图 PNG + 同源 base64 + 合成预览”改为“原图 PNG + 轻量元数据
  + 合成预览”。节省量精确为 `4 * ceil(source_png_len / 3)` 附近的 base64 字符串容量，具体取决
  于分配器容量取整；磁盘工程仍保持自包含。
- 截图覆盖层的原生资源协议已经返回浏览器解码后的冻结帧。无标注/调色的首帧现在直接挂载该
  `<img>`，Canvas 保持浏览器默认 300×150 backing store；首次真正合成时才按冻结帧原生像素分配。
  3840×2160 + 2560×1600 两屏避免提前分配 49,111,040 字节（约 46.84 MiB）的 Canvas 像素，
  同时省掉首次 `drawImage`。切入合成使用 `useLayoutEffect`，避免先隐藏原图再出现空白帧。
- 当前 `449571e` QA AppImage 在本机 Ubuntu 26.04/GNOME Wayland 的第二次独立启动中，进程组空闲
  PSS 为 235,870 kB；连续 5 轮双屏截图后为 332,754 kB，连续 10 轮后为 334,242 kB。前五轮
  建立约 94.6 MiB 的全屏 allocator 高水位，后五轮只增加约 1.45 MiB，没有按截图次数线性增长；
  主进程、WebProcess 与 NetworkProcess 均持续存活且 Swap 为 0。
- 当前 `d702407` 本地 release 后端配合当前 Vite 前端，在真实 GNOME Wayland 依次 Pin 2730×1535、
  760×1000、1776×1410、1560×1000、1560×1000 五张仓库图片。第一张 Pin 后进程组 PSS
  450,861 kB，五轮完成时 542,271 kB，静置后回落到 524,845 kB；补偿 PNG 被协议取走后没有继续
  增长。该曲线包含用户仍保留的解码图与 WebView 工作集，不能当作关闭后泄漏曲线。

## 现场截图时序

- 已安装 v0.1.20 的 GNOME Wayland 冷启动双屏首轮仍为 1.72/2.39 秒，第二轮 1.40/2.12 秒；
  其中冻结帧 375–576 ms、建窗/WebView 561–696 ms，4K 前端首绘 1.11–1.18 秒，是本次继续优化
  原生图片直显路径的直接依据。
- 同一进程预热约 5 分钟后，双屏覆盖层降到约 0.55/0.81 秒（冻结帧 195 ms、建窗 127 ms、
  前端绘制 226/487 ms）。日志未出现新崩溃，但扩展屏冷首帧成本依旧显著。
- 当前安装包确认为 `0.1.20`；14:44 重启后的第一轮双屏为 712/990 ms，仍使用逐屏 PipeWire
  且没有缺帧或兜底。15:38 的 `GTK_IS_WIDGET` 断言在 Clippy、`update-notifier` 与 `cc-switch`
  同一秒共同出现；内核日志没有 OOM、segfault 或 systemd-coredump 记录，因此只能归为桌面级
  GTK 状态切换线索，不能据此宣称 Clippy 崩溃或已经定位黑屏根因。
- 从 Actions 下载并校验 `449571e` QA AppImage 后，在本机 Ubuntu 26.04/GNOME Wayland 两次启动
  共连续测 15 轮。2560×1600 外接屏总时延中位数 504 ms、样本 P95 591 ms；3840×2160 主屏
  中位数 622 ms、样本 P95 719 ms。全部使用逐屏 PipeWire，前端绘制 141–372 ms，没有缺帧、
  首帧超时、黑屏或 WebKit 崩溃。临时解包运行漏传 Xwayland `XAUTHORITY`，因此动作提交到剪贴板
  会失败；这不影响已完整执行的冻结帧、建窗、资源协议与首绘计时，测试后系统 deb 已恢复运行。
- 隔离 Xvfb 中默认快捷键可以注册并触发；虚拟服务器没有 Mutter/wlroots/Portal 输出，最终在
  `xcap` 捕获前置阶段返回“无法捕获显示器”，因此它不能替代真实 GNOME Wayland 的整链路截图。
  覆盖层显示、Canvas 切换及像素导出由 843 项前端测试与三组 Xvfb 冒烟继续兜底。
- 当前 Pin 真机覆盖：2730×1535 原图在 1.5 倍真实缩放/2 倍 WebKit 缓冲区下显示为
  1827×1061 逻辑窗口，截取该窗口确认实际内容完整、非黑屏/空图；后台生成 3517×1978 补偿图
  用时 801 ms，晚到 revision=1 路径成功。其余四张同时覆盖“赶上首帧”与“事件换图”，耗时
  156–398 ms，日志无协议、解码或 WebKit 错误。测试结束已停止 QA/Vite 并恢复 `/usr/bin/clippy-app`。

## 仍需实机覆盖

- 当前提交已取得真实 GNOME Wayland 连续截图 0/5/10 轮与连续 Pin 1/5 轮 PSS 曲线，并完成
  10,000 条 React 窗口化压力与 Firefox 行高实测；尚未完成 WebKit 实机长时间图片滚动和连续
  Pin **关闭后**的 PSS 回收曲线。
- Rust 权威冻结帧、裁剪、标注、保存与 Pin 渲染器都未修改；协议尺寸、原图哈希、工程往返和
  合成像素测试继续通过，因此首帧直显不改变保存/复制/Pin 的原生分辨率。
- Windows/macOS 已生成并验证签名测试包；截图、粘贴、权限提示等实际桌面交互仍需对应机器手工复核。
