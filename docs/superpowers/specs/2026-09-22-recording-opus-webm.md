# PX-REC-OPUS-WEBM-01 — Opus 编码与 WebM 双轨封装合同

## Goal

在不接产品录屏 session 的前提下，把规范化 48 kHz mono/stereo `f32` PCM 编码为 Opus，并与现有
VP9 packet 写入同一个合规 WebM。该切片固定编码延迟、首尾裁切、轨道元数据和跨轨写入顺序，避免
后续平台接线各自实现一套近似时间线。

## Requirements

1. Opus 只进入显式 `recording-opus-webm` feature；该 feature 复用 `recording-vp9-prototype`，默认
   产品构建、现有纯视频 WebM 和 UI 保持关闭。编码器使用精确固定的 `opusic-c 1.6.1` 与
   `opusic-sys 0.7.5`，后者随 crate 携带 libopus 1.6.1 源码并静态构建，构建期间不能下载代码。
2. 输入必须是 48 kHz、1 或 2 声道、有限值、交错 `f32` PCM。编码块固定 20 ms / 960 frames，
   `Application::Audio`、VBR 和约束 VBR 保持显式配置；连续输入可跨任意合法 PCM chunk 边界重组，
   时间倒退、重叠或内部空洞必须报错，不能静默压缩或补齐时间。
3. `OpusHead` 必须为版本 1、mapping family 0，channel count 与轨道一致，input rate 为 48 kHz，
   output gain 为 0；pre-skip 必须来自同一 libopus encoder 的 lookahead，不能写死猜测值。
4. WebM 音轨必须使用 `A_OPUS`、48 kHz、相同 channel count 和完整 `OpusHead`。`CodecDelay` 必须为
   `pre_skip * 1_000_000_000 / 48_000`，`SeekPreRoll` 必须为 80 ms。为此仅给 vendored `webm 2.2.1`
   / `webm-sys 2.2.1` 增加 libwebm 已支持但 Rust API 未暴露的 setter，并记录补丁范围。
5. 编码器结束时必须追加足够的零样本排空 lookahead，再把总解码样本超出 `pre_skip + 真实输入`
   的部分写为最后一个音频 `BlockGroup/DiscardPadding`；不得丢失尾部真实 PCM，也不得把 padding
   计入媒体时长。
6. 双轨 mux 接受已经编码的 VP9/Opus packet。音频 Block timestamp 必须在对应 PCM presentation
   timestamp 上增加 `CodecDelay`，使播放器按 Matroska 规则减去 delay 后回到共同 A/V 时间点；
   Segment 使用 0.5 ms TimecodeScale，精确表示本固定 libopus 的 6.5 ms lookahead，同时避免极短
   Cluster。所有 packet 的 WebM timestamp 必须全局单调；重复
   时间只允许不同轨道使用。结束时显式写两轨实际最大时长，视频帧数和音频 packet/真实 sample
   数分别保留，不能用 packet 数冒充 PCM 时长。
7. 现有纯 VP9 writer、恢复 remux 和缩略图解析继续只接受单 VP9 轨。本切片生成的双轨文件必须有
   独立结构解析测试；在恢复协议正式升级前，双轨产物不能伪装成旧 manifest schema。
8. 依赖锁文件、许可证资源、Tauri bundle 和录屏供应链检查必须覆盖新增两个 Opus crate 与两个
   vendored WebM path package；Windows、macOS 与 Linux 在同一 SHA 的原型 CI 编译并运行合同测试。

## Acceptance Criteria

- mono/stereo 编码均产生可由 libopus 解码的 packet；确定性测试核对 20 ms frame、lookahead、
  `OpusHead` 字段、真实 sample 数和非法格式/时间线错误。
- 非 20 ms 结尾能完整排空；解码后应用 pre-skip 与 DiscardPadding，样本数与真实输入完全相等。
- 真实双轨 WebM 包含一条 `V_VP9` 与一条 `A_OPUS`，并精确核对 CodecPrivate、CodecDelay、
  SeekPreRoll、最后块 DiscardPadding、轨道时间戳及 Duration。
- 现有纯视频 WebM 回归、恢复合并与缩略图测试保持通过；默认构建不解析或链接 Opus。
- `./scripts/ci-local.sh` 完整通过；同一提交的 Ubuntu、Windows、macOS 原生/原型 CI 均为 success。

## Out of Scope

- 把音频 worker、A/V epoch coordinator 或平台音源接入 `DiagnosticRecordingSession`。
- 系统声与麦克风混音、重采样、自动增益、降噪、回声消除、漂移拉伸或丢包隐藏。
- 给已有纯视频恢复 remux、缩略图或 manifest schema 增加双轨支持。
- macOS ScreenCaptureKit audio、Linux PipeWire 音频节点、录音设备 UI 和默认发布能力。

## Format Decisions

- 编码实现使用 Xiph libopus 1.6.1 参考实现。当前新出现的纯 Rust 编码器尚缺少本项目的三平台、
  长时录屏和浏览器互操作证据，不在这一质量基线上替换参考实现。
- Opus packet 的 WebM timestamp 使用对应输入 PCM frame 的 presentation 时间加 `CodecDelay`；
  播放器减去 delay 后回到共同 A/V epoch，pre-skip 负责丢弃编码器起始无效样本。结束时追加 lookahead 并用
  `DiscardPadding` 精确裁掉尾部补零。
- 后续 session 接线必须在 mux 前按两轨 timestamp 合并 packet。本切片只接受全局有序输入并拒绝
  顺序错误，不在容器层建立无界重排队列。

## References

- Xiph libopus 1.6.1：<https://opus-codec.org/>
- RFC 7845 pre-skip、OpusHead 与 80 ms pre-roll：<https://www.rfc-editor.org/rfc/rfc7845>
- WebM container codecs：<https://www.webmproject.org/docs/container/>
- Matroska CodecDelay / SeekPreRoll：<https://www.matroska.org/technical/notes.html>
- Matroska Opus codec mapping draft：<https://datatracker.ietf.org/doc/draft-ietf-cellar-codec/>

## Verification

- `cargo test --features recording-opus-webm --lib recording::mux::opus_webm::tests`：6 项通过；
  覆盖 mono/stereo 真编码解码、任意 chunk、pre-skip、尾裁切、非法时间线、双轨结构和跨轨排序。
  本机存在 `ffprobe`，同一真实双轨文件被识别为 VP9 + Opus，首个音频 PTS 为非负且小于 1 ms。
- `cargo test --features recording-opus-webm --lib`：1114 项通过、15 项真实桌面测试忽略；现有 VP9、
  WebM 恢复 remux、缩略图和默认业务回归保持通过。
- `cargo clippy --features recording-opus-webm --all-targets -- -D warnings`、
  `node scripts/verify-recording-codec-supply-chain.mjs` 与格式检查通过。
- `./scripts/ci-local.sh`：25 项通过、0 失败、2 项按环境配置跳过；其中默认 Rust 1082 项通过、
  14 项真实桌面测试忽略，前端 73 个文件 / 1251 项测试通过，X11 隔离协议与生产构建通过。
- 提交 `dfbc8a45090b2238a61ac97f946a26d793b8c1a1` 的
  [CI Check 35690688905](https://github.com/51hhh/Clippy/actions/runs/35690688905) 已在同一 SHA 通过
  Ubuntu 主检查、Windows/macOS 原生检查，以及 Ubuntu、Windows、macOS ARM/Intel 四个录制原型
  job；这关闭了该原型的跨平台编译、链接和合同测试门禁，仍不替代平台音源与真机 A/V 验收。
