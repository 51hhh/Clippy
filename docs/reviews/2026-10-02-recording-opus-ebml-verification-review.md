# WIN-NATIVE-01 / W60 — Opus 元数据验证边界

需求 `REC-OPUS-EBML-VERIFY-01`；基线 `678a817abe073fc1b27f33eeb38657bf1f04c9f1`；
分支 `codex/recording-opus-ebml-verification`。规格在测试修改前建立。本轮没有生产修改。

## 缺陷与修复

旧测试辅助函数在整个文件裸搜字段 ID，把 TrackUID、Void、Block 载荷中的相同字节
当作元数据。真实 mux 输出只等长替换非零视频 UID，旧读取器就分别得到 CodecDelay=None、
SeekPreRoll=None/1、DiscardPadding=None，而实际字段仍为6500000/80000000/12666666 ns。
同文件原严格恢复读取器完整遍历通过，独立 EBML 审计确认字段/偏移与所有其它字节保持，
已有 ffprobe 的三包内容哈希/时间、VP9+Opus 轨道与原文件一致；四份干扰媒体和原文件保留。

修复只在 cfg(test) 读取器按四个既有字段的父路径、ID 和尺寸遍历；非 Master 载荷不递归。
越过父边界、缺失字段、截断/无效 VINT、未知大小标量和同一路径重复字段返回 None。
只接受顶层 Segment 的未知长度，正负整数继续按原逻辑读取。原六个双轨测试函数/
断言/ffprobe 检查及全部生产前缀字节保持；没有改变 mux、严格读取器、帧率或预算。

## 验证

最终六项新夹具和五份实际媒体同字节：原读取器1 passed / 5 failed；修复录屏领域407/0，
包含原401与新6；新六项仅在QA图，已含在领域数。四字段的Void/块载荷干扰、缺失字段、
错误父路径、越界/截断/未知大小保护与正负padding、已知/未知Segment长度均核对。
首轮夹具误假设八字节UID，实际当前writer为七字节；该次失败及日志保留，修正夹具后
再重放原读取器。没有把该夹具失败冒充媒体缺陷。后续对照显式重放已保存的真实媒体；
普通门禁仍由原mux现场生成媒体，不修改随机UID生成器。

干净 `a5e03f8d075cd09023a824e26d51ff80f9466014` 完整Windows门禁33 passed / 0 failed / 1 Linux smoke skipped；
默认Rust1247/QA1373各5ignored，两图重叠不累加；前端81文件/1403passed。
新六项各在QA图实际运行一次，已含在总数；原双轨合同、生产代码与旧期限保持。

## 保留边界

W59那次SeekPreRoll=None的失败媒体未保留，独立可复现的测试缺陷不证明该次根因。
W53历史AVI一次30秒超时亦未归因。桌面停止；真实设备/长时同步、其它宿主本地门禁、
当前SHA三宿主/codec CI、Win10/多屏/安装器与当前release未验。旧830b12b release仅属历史。
生产修复总数仍40，本轮测试验证改进不计作部署修复，也没有用户可见CHANGELOG条目。
证据在 `src-tauri/target/opus-ebml-verification-contract/`。
