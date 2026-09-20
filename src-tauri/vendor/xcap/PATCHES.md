# Clippy 对 xcap 的补丁

基于 crates.io **xcap 0.9.6**，对应上游提交
`d4ff80928a9758043595f6d9bbda0597efce9e51`，许可证为 Apache-2.0。保留发布包的完整三平台源码、
示例和 normalized `Cargo.toml`。除下述 macOS 区域录制入口外，不改变上游公开 API。

发布包中 `.editorconfig` 与 `.gitignore` 的 CRLF 统一为仓库 LF，以满足 `git diff --check`；内容不变。

- 原始 `.crate` SHA-256：`b6ad471d5ba232bc276382d26a9d3b837d6853b7df389058b5bb1e94dcdd248c`
- 原始 `src/windows/wgc_video_recorder.rs` SHA-256：
  `920b0a9a1bb13929387f9fe6a14ced1d8f8d8222aba2742449e1fe0ebeb52d45`
- 原始 `src/windows/utils.rs` SHA-256：
  `e3bb190867dfcaeb09e1558b0aa08d1e04b8ba9d8fb5f2fe3a4e12d0c22fea53`
- 原始 `src/macos/capture.rs` SHA-256：
  `f93bdd34be02414e2b6ad6ef33387093e3d2f05ef0cb507c150c7653a096ad46`
- 原始 `src/macos/impl_window.rs` SHA-256：
  `256ae02b832e2be956a55578272a0fa7ebd3819b0d5401e193800d3761ddcc96`
- 原始 `src/macos/impl_video_recorder.rs` SHA-256：
  `332b10d02307519d40c156cca67b63961c5b2856edd1cb2a2b6836cde33090c1`
- 原始 `src/macos/impl_monitor.rs` SHA-256：
  `0c932e63a93a7e7ee888c303de18762376a3f0c5b4464951198d018eab66a462`
- 原始 `src/monitor.rs` SHA-256：
  `f5bd15e96b8605431cadc4e75b55b1fc9fe06a6ea4f87d5a902f3d4abc5f8328`

## Windows WGC 录制光标

上游 WGC `VideoRecorder` 在创建 `GraphicsCaptureSession` 后固定调用
`SetIsCursorCaptureEnabled(false)`，因此录制帧一定缺少光标，不能满足 Clippy `PX-REC-01`
第一阶段“画面包含光标”的验收。

行为补丁只把录制会话的参数和对应日志从 `false` 改为 `true`，没有改变一次性截图路径、D3D11
纹理读回、帧格式、通道容量、开始/停止或 macOS/Linux 行为。该属性从 Windows 10 2004
（build 19041）开始提供；上游保留 best-effort 语义，旧系统调用失败时沿用 WGC 会话原有行为。
因此代码合同已经要求包含光标，但仍必须在 Windows 10 2004+ 原生 CI/真机上用移动光标像素场景
确认，不能用 Linux 交叉编译代替。

## 当前工具链 lint 对齐

`src/windows/utils.rs` 将固定四字节 BGRA 像素循环从 `chunks_exact_mut(4)` 改为等价的
`as_chunks_mut::<4>().0`，满足当前 Rust 的 `clippy::chunks_exact_to_as_chunks`。输入是平台生成的
BGRA 帧，长度本来就是四的倍数；原实现也忽略 remainder，因此像素和越界行为不变。

macOS `capture.rs` 与 `impl_video_recorder.rs` 的定长像素循环做相同替换。`CGWindowListCreateImage` 和
`NSWorkspace::activeApplication` 保留原行为并只在精确调用处允许 deprecated：前者迁移到
ScreenCaptureKit 属于独立平台帧源工作；后者现有注释记录了与 `frontmostApplication` 的实时语义差异。
没有对整个 crate 放宽 lint。

## macOS AVFoundation 区域录制

上游 `VideoRecorder` 固定产出整块显示器，Clippy 若在 Rust 中再裁剪，会先为 6K/8K 显示器分配完整
RGBA 帧，违反 64 MiB 单帧预算。补丁增加 macOS 专用的 `Monitor::video_recorder_region`，只接收有限
浮点值、屏幕点矩形和正倍率，并在进入 Objective-C 前核对矩形没有越过显示器边界。内部把参数写入
`AVCaptureScreenInput.cropRect` 与 `scaleFactor`；前者使用显示器左下角为原点的屏幕点，后者把
区域恢复为 backing-pixel 输出。原有 `video_recorder` 继续走无裁剪默认值，其他平台不暴露该入口。

Clippy 仍会核对首帧和后续帧恰好等于冻结选区的物理像素尺寸；尺寸变化会终止会话，不做拉伸或补齐。
该补丁只解决区域输出和整屏内存问题，不提供窗口排除。控制窗若要位于录制区域内，仍须迁移到
ScreenCaptureKit 并完成原生权限、Retina、旋转屏和混合 DPI 真机验收。

## 维护门禁

`scripts/verify-xcap-patch.mjs` 固定版本、来源哈希说明、七个实际修改文件的完整 SHA-256、Cargo path
override、独立 vendor 包归属、锁文件 path 解析和许可证。Windows/macOS 原生 CI 会分别用独立
manifest lint WGC feature 与 macOS 库，避免把上游示例开发依赖并入主锁文件。
vendor 自带 `Cargo.lock` 仅供独立 lint 重现上游依赖，不参与 Clippy 主工程解析。
升级 xcap 时必须重新核对 WGC 光标、同步回调、Stop/Drop 回收及三平台
`cargo clippy --all-targets -- -D warnings`，再决定移除或重放本补丁。
