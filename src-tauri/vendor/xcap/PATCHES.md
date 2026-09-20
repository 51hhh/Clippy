# Clippy 对 xcap 的补丁

基于 crates.io **xcap 0.9.6**，对应上游提交
`d4ff80928a9758043595f6d9bbda0597efce9e51`，许可证为 Apache-2.0。保留发布包的完整三平台源码、
示例和 normalized `Cargo.toml`，没有修改公开 API。

发布包中 `.editorconfig` 与 `.gitignore` 的 CRLF 统一为仓库 LF，以满足 `git diff --check`；内容不变。

- 原始 `.crate` SHA-256：`b6ad471d5ba232bc276382d26a9d3b837d6853b7df389058b5bb1e94dcdd248c`
- 原始 `src/windows/wgc_video_recorder.rs` SHA-256：
  `920b0a9a1bb13929387f9fe6a14ced1d8f8d8222aba2742449e1fe0ebeb52d45`
- 原始 `src/windows/utils.rs` SHA-256：
  `e3bb190867dfcaeb09e1558b0aa08d1e04b8ba9d8fb5f2fe3a4e12d0c22fea53`

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

## 维护门禁

`scripts/verify-xcap-patch.mjs` 固定版本、来源哈希说明、五个实际修改文件的完整 SHA-256、Cargo path
override、独立 vendor 包归属、锁文件 path 解析和许可证。Windows/macOS 原生 CI 会分别用独立
manifest lint WGC feature 与 macOS 库，避免把上游示例开发依赖并入主锁文件。
vendor 自带 `Cargo.lock` 仅供独立 lint 重现上游依赖，不参与 Clippy 主工程解析。
升级 xcap 时必须重新核对 WGC 光标、同步回调、Stop/Drop 回收及三平台
`cargo clippy --all-targets -- -D warnings`，再决定移除或重放本补丁。
