# REC-ARTIFACT-SHARING-01 — Windows 已提交录屏产物提升

## Goal

基于 W56 文档 `b2f6e95` / 源码 `d7c66bb`，检查清单已提交后的分段、最终输出和
恢复提升是否会被短暂 Windows 删除共享冲突中断。保持
`PX-REC-AV-MANIFEST-01` 的 fsync → 清单原子提交 → 产物提升 → 目录同步合同。
W53 历史 AVI 超时缺少 worker 原错，本轮独立复现不能作为其根因证明。

## Requirements

1. 先使用真实不共享删除的文件句柄和原 journal/session/MJPEG/VP9 文件路径取得
   原生错误；不替换生产文件 I/O，不操作桌面、接入设备或改旧测试/等待期限。
   会话仍使用原 10 FPS/200 ms 配置，核对原 encoder 原错及失败保留的已提交前缀。
   生产修改前基线取得 2 passed / 6 failed，六条短暂冲突路径全部为实际错误 32，
   包含五个 journal/恢复提升点和周期 session 的 encoder 根因，运行期约 0.31 秒。
2. 如复现，Windows 已进入清单的产物提升复用 W56 已验证的共享冲突合同：原始
   32/33，或错误 5 且原生删除访问探针证实共享拒绝 32，最多 21 次/500 ms；
   单次原生 I/O 自身阻塞不受该窗口承诺。源消失、权限拒绝和其它错误仍返回原错。
3. 范围为分段提交、最终输出提交、恢复合并输出提交，以及已提交 partial 的分段/
   最终输出恢复提升。导出、其它文件保存及泛用私有权限工具不改；其它平台沿用
   原调用。清单、fsync、ACL、哈希/长度/轨道统计、恢复前缀和保留/清理顺序不改。
4. 相同旧 API 运行期夹具在原 Git 源码和修复源码执行，原阶段/失败保留。所有新
   产物均核对实际文件、清单和哈希；不将合成原生文件或 codec 测试算成设备 QA。
   永久共享冲突仍失败且保留已提交 partial；损坏/未声明 partial 不得变为合法产物。

## Acceptance Criteria

- [x] 原 session/journal/恢复路径在真实句柄下取得错误或否定证据。
- [x] 如复现，五个已提交产物提升路径有界恢复，原权限与提交/恢复保护保持。
- [x] 同字节旧实现与新实现对照、原完整测试模块与预算/期限保持。
- [x] 干净 source SHA 完整 Windows 默认/录屏 QA 门禁，重叠/ignored/skip 单列。
- [ ] 当前 SHA 三宿主/codec CI、真实设备/安装器/多屏和历史 AVI 根因核实。

## Out of Scope

桌面、设备、工具安装、证书、推送/PR/合入/发布继续停止。其它保存/导出、Linux/
macOS 行为不变。本 SHA release 未构建，Win10/混合 DPI/负坐标/多屏未验。
历史 W53 根因未证明；永久错误不能以忽略异常、增加旧超时或扩大队列预算通过。

## Verified source

同字节八项旧API夹具原实现2/6（六条实际错误32）、修复领域393/0=原385+新8。
干净 `830b12b43051efa7eebad36a3d1a4434975785b2` 完整 Windows 门禁33 passed / 0 failed / 1 Linux smoke skipped；
默认Rust1240/QA1359各5ignored，两图重叠不累加；前端81文件/1403passed。
新5在默认/QA两图、新3仅QA，已含在Rust总数；两个旧完整测试模块与原权限/恢复保护保持。
三份运行期夹具字节相同，全部使用原API，无接口stub；原retry/probe算法与导出保持，十四份Rust输入绑定干净SHA。
旧实现重放及finally恢复、最终领域与完整门禁的实际日志/输入哈希保留。
证据：`recording-artifact-sharing-native-qa-830b12b/RESULT.json`、
`recording-artifact-sharing-contract/COMMITTED-CONTRACT-AUDIT.json`、`REVIEW-CLOSURE.json`。
设备/跨宿主/当前CI/安装器/多屏/发布及W53 AVI旧超时根因仍未验。
