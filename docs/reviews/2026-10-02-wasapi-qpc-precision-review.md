# WIN-NATIVE-01 / W67 — WASAPI QPC 精度审查与修复

需求 `REC-WASAPI-QPC-PRECISION-01`；基线 `a02c35fdb4f5f29aad8997e031dfe073801ed2a8`；先行规格 `9fcc1e3f3d78838261c62b00076252ef604b4cfc`；源码 `aac5e0a728d46adc7bd7603188f41b9380138650`。
分支 `codex/windows-wasapi-qpc-precision`；桌面操作保持暂停。

## 问题与修复

[Microsoft GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)
规定第一音频帧 QPC 为100ns单位，未规定整数舍入方式/固定packet帧数。
构造640帧48kHz连续包：时长13,333,333ns，下一原始起点13,333,300ns，差33ns。
原Windows copy_packet检查和AudioTimeline均严格拒绝重叠；离线诊断直接执行原mapper、packet、pipeline、worker与完整owner，
确认后两者返回PresentationOverlap，实际清单interrupted。没有构造或调用原生COM源，未测设备packet节奏/漂移。
原混音器已按音频帧取整，同组输入保留1280帧；它的输出继续使用Exact。

Windows源以内部精度类型声明100ns；平台enum传递，worker激活前配置队列。
packet guard容忍一个刻度；队列只把媒体区间起点对齐到上一媒体末尾，同时处理由此产生的暂停/封尾末尾差。
原captured_at_ns、样本、序号、会话原点、源控制时间和公共暂停扣时保持。
超过100ns、重复/倒退/迟到或早于起点仍拒绝，真空洞保留；精度不能在PCM/控制后改变。
背压原子性、溢出与原错误/已接受前缀/源析构保持；没有重采样、丢帧或另建时钟。

## 验证层级

- 原API诊断4/0，在原生产代码上执行；同字节夹具在修复后仍4/0，仍证明Exact错误路径，
  不将精度新API的绿色保护伪称为原API接受路径红绿证明。
- 修复领域459/0；新增17项=原诊断4+新精度绿色保护13，15项两图/2项仅QA。
  八个旧完整测试模块/Git内容与原期限保持，三个原诊断文件输入SHA保持。
- 原worker成功接收1280帧并封尾；精度更改错误在Start前中止，原PCM前缀仍可读、源析构一次。
- 原完整双轨owner、真实VP9/Opus和journal输出complete。实际ffprobe退出0：4视频包/3音频包，
  initial_padding=312、discard_padding=648，有效PCM1920帧、容器40ms；两源各析构一次。
  文件SHA256 `38eef325bc3d4f5721bb9d226278e1f74b115a0647c3bd970c3d268ae2056e58`，未播放。
- 干净源码完整Windows门禁33 passed / 0 failed / 1 Linux smoke skipped，实际native/包装器/终端退出0。
  默认Rust1281/QA1425各5ignored，前端81文件/1403；严格clippy、check、边界/供应链/前端构建等原门禁通过。
  WASAPI纯合同25项已含Rust总数，领域/新项不额外累加。旧断言/期限和源码保持，没有自动重试。
  初版完整门禁32/1/1，默认1281/0/5、QA1424/1/5；旧测试
  `transient_partial_sharing_does_not_terminate_original_periodic_session` 返回清单原子写入os error 5。
  同SHA隔离1/0后完整复查通过；初版日志/退出码保留，原TempDir失败媒体已移除，根因未证明。
  不据此推断Defender、索引、共享、调度或本次QPC修复的影响，也不视复查成功为根因修复。

证据在 `src-tauri/target/wasapi-qpc-precision-contract/`、初版 `wasapi-qpc-native-qa-aac5e0a/` 与复查 `wasapi-qpc-native-qa-aac5e0a-recheck/`，
保存冻结输入、PID/原生退出码、UTF-8日志、原实际interrupted/complete文件、ffprobe与核对记录。
前置56d750c默认/QA release与其它历史EXE/已安装包/39项桌面记录原字节保持；它们不含本修复。

## 保留边界

Windows原生源与平台路由有编译/lint证据；合成source/真实codec/文件不能证明WGC/WASAPI真机采集或同步。
当前SHA release/跨平台CI、其它宿主、设备切换/漂移/长时同步、桌面、安装器/updater/无CRT启动、
Win10/多屏/负坐标/混合DPI仍未验。默认录屏入口关闭。W53/W59/W63历史失败根因保持未明。
未启动应用/设备/桌面、未安装/推送/PR/合入/发布；整体任务与最后复合验收仍未完成。
