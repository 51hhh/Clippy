# PX-REC-THUMBNAIL-01 — 录屏结果库持久缩略图

日期：2026-09-21

关联路线：`PX-REC-01` / `PX-REC-PLAYBACK-01` / `PX-REC-MERGE-01`

## Goal

让录屏结果库在不启动播放器、不把真实文件路径交给 WebView 的前提下，用持久首帧缩略图帮助用户
识别完整录屏和可恢复分段；缓存必须继承录屏产物的隐私、完整性和删除语义。

## Requirements

1. 只有显式启用 `recording-vp9-prototype` 的构建，且会话为 `vp9-prototype` + `webm` 时提供缩略图。
   完整会话取最终产物首帧，中断会话取第一个已提交分段首帧；AVI 与无有效产物的会话保持占位图。
2. 前端只提交会话不透明 ID，不提交路径、文件名、SHA 或任意缓存键。后端重新解析代表产物，并在
   blocking pool 中验证普通文件、manifest 长度和 SHA-256 后才允许解码。
3. WebM 读取复用恢复 remux 的受限解析合同：只接受 Clippy 生产的单 VP9 视频轨、无 lacing
   `SimpleBlock`，轨道尺寸必须与 manifest 一致，首个 packet 必须是关键帧，packet 上限为 64 MiB。
4. 首个 VP9 packet 只解码一帧，源画面最多 16,777,216 像素。拒绝高位深、零帧、多帧、尺寸不一致
   和越界 plane/stride；按录制端相同的 BT.709 limited 合同转回 RGBA，再缩放为最长边不超过
   320 px 的 PNG，输出不超过 512 KiB。
5. 缓存位于应用数据目录下独立的私有
   `recording-thumbnails/<session-id>/<artifact-sha256>.png`，不进入录屏会话目录，也不修改恢复
   manifest。命中缓存仍校验普通文件、字节上限、PNG 格式和像素尺寸；失效或损坏缓存原子替换。
6. 每个会话缓存目录最多保留当前代表产物的一张 PNG；同一进程同时只执行一次冷缩略图生成，避免
   StrictMode、快速滚动或重复窗口造成并发解码和内存峰值。
7. 删除录屏会话前先清除该会话由 Clippy 管理的缩略图；恢复合并成功后清除旧代表产物缓存。缓存
   清理拒绝遍历符号链接，缓存损坏或未知文件不能阻止用户删除真实录屏。
8. 结果卡只在接近视口时请求缩略图；离开视口或组件卸载后释放 data URL。生成失败保持中性占位图，
   不能隐藏播放、导出、定位、恢复或删除入口，也不显示全局操作错误。
9. 缩略图命令只允许录屏结果窗口调用，且默认构建必须保持可编译并明确返回不可用，不扩大现有窗口
   IPC 权限。

## Acceptance Criteria

- [x] 完整 VP9/WebM 与中断会话首段都能生成 320 px 内的首帧 PNG；AVI 和默认构建不生成。
- [x] 缓存第二次读取不再打开或解码源视频，并能拒绝符号链接、超限、损坏及尺寸伪造的缓存文件。
- [x] 产物普通文件、长度、SHA、WebM 轨道、关键帧、解码帧数、位深和尺寸异常都有回归测试。
- [x] 同会话重复请求不会并发解码；源 SHA 变化时使用新键并清除旧缓存。
- [x] 删除会话与恢复合并会清除受管缩略图，缓存清理失败不阻塞真实录屏删除。
- [x] 前端测试覆盖接近视口才请求、成功显示、失败占位、卸载后忽略迟到结果和 AVI 不请求。
- [x] IPC 合同、Rust 默认/feature check、clippy、test、前端测试、TypeScript、Vite 与完整本地门禁通过。

## Out of Scope

- 不为 AVI 诊断分段、外部导入视频或任意 Matroska 文件生成缩略图。
- 不自动播放视频，不生成时间轴雪碧图，不提供用户可导出的截图，也不把缩略图加入剪贴板历史。
- 不在本阶段加入音频、剪辑、转码、GPU 解码或开放 Windows、macOS、Wayland 产品入口。
- 不用合成数据或浏览器首帧替代三平台原生录制、系统 WebView 解码和长时间资源真机验收。

## Verification

- `./scripts/ci-local.sh`：Linux x64 完整门禁 25 项通过、0 失败；非宿主平台交叉 lint 与 AppImage
  可视 smoke 2 项按脚本配置跳过，未计入通过。
- `cargo test --features recording-vp9-prototype`：Rust 主测试 1063 通过、0 失败、15 忽略；私有
  X11 剪贴板集成测试 4 项按隔离环境开关忽略。`cargo clippy --all-targets` 的默认构建与 VP9 feature
  构建均以 `-D warnings` 通过。
- 缩略图定向 Rust 回归 8 项通过，覆盖真实 VP9 首帧、BT.709 limited 像素容差、缓存命中、损坏重建、
  SHA/尺寸/像素预算、符号链接、并发单槽、超限与 321 px 伪造缓存；WebM 测试确认快速路径 packet
  与完整受限解析器首 packet 一致。
- 前端全量 Vitest 73 个文件、1233 项通过；其中录屏 API 与结果库定向 20 项通过。TypeScript、IPC
  合同、HTML/Tauri 边界、Vite 构建、DOM/Xvfb、Canvas 像素与主窗口布局 smoke 均通过。
- 开发审阅页已检查深浅主题和约 780 px / 420 px 宽度：缩略图在宽卡片左侧展示，窄窗改为整行，
  占位、恢复提示与动作入口保持可读。
- Windows、macOS 同 SHA Native Check、三平台原生录制缩略图与长时间资源真机验收仍待远程 CI / QA；
  这些结果不由 Linux 本地门禁替代。
