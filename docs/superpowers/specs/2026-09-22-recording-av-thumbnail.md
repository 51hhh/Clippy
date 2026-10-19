# PX-REC-AV-THUMBNAIL-01 — 双轨录屏首帧缩略图

日期：2026-09-22

关联路线：`PX-REC-01` / `PX-REC-THUMBNAIL-01` / `PX-REC-WINDOWS-AV-QA-01`

## Goal

让带 VP9 + Opus 音轨的完整录屏和异常恢复分段复用现有结果库首帧缩略图，同时保持 WebView
只提交不透明会话 ID、后端完整校验产物和私有缓存的安全边界。

## Requirements

1. 只有显式启用 `recording-opus-webm` 的构建，且清单为合法 schema v2、视频为
   `vp9-prototype + webm`、音频为 48 kHz mono/stereo Opus 时，双轨结果才声明可生成缩略图。
   完整会话使用最终产物，中断会话使用首个已提交分段。
2. 缩略图源必须同时携带经过清单校验的 Opus 轨参数和代表产物音频统计；缺失、混用 v1/v2 或
   packet/PCM 统计无效时拒绝，不能仅因文件中出现 `A_OPUS` 字符串而接受。
3. 受限 WebM 解析器只接受 Clippy 生产的精确轨道形状：1 号 VP9 视频轨、2 号 Opus 音频轨；视频
   尺寸、音频声道、48 kHz 采样率、`OpusHead`、`CodecDelay` 和 `SeekPreRoll` 必须与清单一致。
   重复、未知或额外轨道一律拒绝。
4. 首帧扫描允许在首个视频 packet 以前出现经过校验的无 lacing 音频 `SimpleBlock`，但只把首个
   VP9 关键帧交给解码器。未知轨道、视频非关键首帧、越界 packet 和首帧以前的 `BlockGroup`
   明确失败；不会为了生成图片解码 Opus 或遍历完整媒体文件。
5. 缓存键、PNG 上限、单冷任务、删除/失效和前端懒加载继续复用 `PX-REC-THUMBNAIL-01`，不创建
   第二套缓存或新的 IPC 参数。
6. 单轨 schema v1 行为保持不变；双轨会话仍不显示异常合并入口。本切片不能暗示独立 Opus 编码
   分段已经可以无损拼成一条连续音轨。

## Acceptance Criteria

- [x] 真实生产 writer 生成的完整双轨 WebM 和首个已提交双轨恢复分段都能生成尺寸受限的 PNG。
- [x] 结果库对合法双轨 complete/interrupted 会话返回 `canThumbnail=true`，仍返回
  `canMerge=false`；默认构建继续返回不可用。
- [x] 自动化覆盖单轨兼容、错误声道/采样率/Opus 私有头/延迟、额外轨道、未知 block 轨道和非关键
  首视频帧；失败不写入缓存。
- [x] Rust 默认与 `recording-opus-webm` feature 的 fmt、check、clippy、test 通过，前端和完整本地
  门禁保持通过；同一 SHA 的四平台录制原型 CI 继续作为跨平台门槛。

## Out of Scope

- 双轨异常分段合并、音频解码/重编码、波形图、静音检测和缩略图导出。
- 改变录制音频模式、平台采集、缓存格式或结果库交互。
- 用缩略图成功替代 Windows 系统声/麦克风、设备拔出、系统播放器兼容和 30 分钟 A/V 漂移真机
  验收。

## Verification

- `cargo test --features recording-opus-webm --lib`：1138 项通过、15 项真实桌面测试忽略；包含真实
  VP9 + Opus 完整文件和恢复分段首帧解码、伪造音轨合同与单轨回归。
- `cargo clippy --features recording-opus-webm --all-targets -- -D warnings`：通过。
- `npx vitest run tests/recording-library-app.test.tsx tests/recording-api.test.ts`：2 个文件、24 项通过，
  覆盖有声中断卡片请求缩略图且不显示单轨恢复动作。
- `./scripts/ci-local.sh`：25 步通过、0 失败、2 项按环境配置跳过；默认 Rust 1090 项通过、14 项
  真实桌面测试忽略，前端 73 个文件 / 1260 项、DOM 14 项以及 X11 录屏/剪贴板、Canvas、布局和
  生产构建 smoke 通过。
- 本提交同一 SHA 的远程 Ubuntu、Windows、macOS 与录屏原型 CI：推送后填写；本地 Linux 门禁
  不能替代条件编译平台结果。
