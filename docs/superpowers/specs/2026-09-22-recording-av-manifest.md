# PX-REC-AV-MANIFEST-01 — 双轨录屏清单与恢复协议

## Goal

在音频 worker 接入产品录屏 session 前，把恢复清单升级为能明确描述 VP9 + Opus 的版本化协议。
旧的纯视频录像继续按 schema v1 读取；带音轨的录像只能写 schema v2，并在启动恢复、结果库和产物
解析入口中保留清楚的能力边界，避免双轨 WebM 被现有单轨 remux 或缩略图解析器误处理。

## Requirements

1. `clippy-recording` schema v1 保持可读且序列化形状不变。只有 journal 配置了音轨时才写 schema
   v2；未知版本继续拒绝，不能把未来版本降级解释。
2. v2 顶层必须包含一条音轨，固定 48 kHz、mono/stereo、`opus` encoder、非零 pre-skip、由
   `pre-skip × 1e9 / 48000` 得到的 codec delay 和 80 ms seek pre-roll。v1 不得携带任何音轨字段。
3. v2 每个已提交分段和最终输出必须记录 Opus packet 数与真实 PCM frame 数；两个计数均非零。
   最终输出的真实 PCM frame 数必须等于已提交分段之和。由于每段独立封尾会产生额外 packet，最终
   packet 数不要求等于分段 packet 数之和。
4. journal 继续使用 `partial fsync → manifest 原子提交 → 产物提升 → 目录同步` 的提交顺序。音轨
   元数据必须与文件长度、SHA-256 一起进入同一个 manifest 提交点，错误时不能留下半个 v2 记录。
5. 启动恢复继续只认可连续、大小与 SHA-256 完全匹配的已提交前缀。v2 的坏尾段可截断，保留下来的
   每个分段仍必须具有完整音轨统计；未知字段、缺失统计或 v1/v2 混合形状必须拒绝。
6. 结果库可显示完整或中断的双轨产物，并暴露只读音轨摘要。现有单轨 VP9 remux 与首帧缩略图
   解析器尚未支持双轨，因此 v2 中断会话不得显示“合并”，所有 v2 会话暂不显示持久缩略图入口；
   播放、导出、定位和安全删除继续复用经过哈希验证的不透明产物能力。
7. 新 API 使用显式 `RecordingTrackStats` 提交视频帧、音频 packet 与真实 PCM frame。旧的纯视频
   调用继续走兼容包装，避免默认构建和既有 QA 会话被迫链接 Opus。

## Acceptance Criteria

- v1 fixture 能继续加载、恢复和进入结果库，序列化后没有 `audio` 或音频统计字段。
- v2 fixture 与 journal 成功路径精确核对音轨格式、pre-skip、codec delay、seek pre-roll、分段/最终统计、
  文件长度和 SHA-256。
- v1 携带音频、v2 缺音轨、非法采样率/声道/编码器/时延、缺失或零音频统计、最终 PCM 统计不一致
  均被确定拒绝。
- v2 活动会话强制中断后能恢复完整已提交前缀；篡改尾段只截断尾部，不影响此前双轨分段。
- 结果库 v2 投影包含音轨摘要，同时 `canMerge = false`、`canThumbnail = false`；产物导出与删除安全
  测试保持通过。
- 默认构建、`recording-vp9-prototype` 与 `recording-opus-webm` 测试和 clippy 通过；完整
  `./scripts/ci-local.sh` 通过。同一 SHA 的三平台 CI 仍是跨平台门槛。

## Out of Scope

- 启动音频 source、修改录屏 UI 或把音频设为默认能力；
- 把 PCM/Opus packet 真正送入周期分段 writer；该接线使用本协议的显式统计 API作为下一提交；
- 双轨异常分段 packet remux、双轨首帧解析、波形缩略图、音频设备选择、混音和重采样；
- 用 schema v2 的存在替代 Windows/macOS/Linux 真机 A/V 同步、播放器兼容或强杀恢复验收。

## Compatibility Decision

schema 版本描述的是清单语义，不直接跟随应用版本。v1 只表达一条视频轨；v2 才允许 `audio` 和每个
产物的音频统计。解析器先按统一的严格结构反序列化，再按版本验证字段组合，因此旧录像无需迁移，
未来版本也不会被当前程序静默误读。

## Verification

- `cargo test --features recording-opus-webm recording::manifest::tests --lib`：29 项通过。
- `cargo test --features recording-opus-webm --lib`：1118 项通过、0 失败、15 项按环境忽略；需要
  loopback 的本地 HTTP 测试在允许本机监听后通过。
- `cargo clippy --all-targets -- -D warnings` 与
  `cargo clippy --features recording-opus-webm --all-targets -- -D warnings`：通过。
- 前端 `tsc --noEmit` 通过；录屏结果库与 API 定向测试 21 项通过。
- `./scripts/ci-local.sh`：25 项通过、0 失败、2 项按环境跳过；其中 Vitest 73 个文件、1252 项
  测试通过，DOM smoke 14 项通过，X11 剪贴板隔离和录屏闭环通过。
- Windows / macOS / Linux 同一提交的远程 CI：推送后填写，不能由本地门禁替代。
