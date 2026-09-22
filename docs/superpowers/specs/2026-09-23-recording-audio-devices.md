# PX-REC-AUDIO-DEVICE-01 — 录屏音频设备选择与固定

## Goal

让显式录屏 QA 构建在开始录制前选择非默认系统音频输出或麦克风，并把选择解析为本次会话固定的
原生设备。设备身份继续留在 Rust 信任边界内；默认构建、正式 release 和既有默认设备行为不变。

## Requirements

1. 后端按平台枚举当前可用设备：Windows 枚举活动 render/capture endpoint；Linux 枚举 PipeWire
   `Audio/Sink` 与非 monitor 的 `Audio/Source` node；macOS 15+ 枚举 `AVCaptureDevice` 音频输入。
   ScreenCaptureKit 系统声没有单输出选择能力，因此 macOS 系统声只保留系统默认语义。
2. 原生 endpoint ID、PipeWire node name/serial 和 macOS device unique ID 不得进入 WebView。后端为
   每次能力查询建立绑定 caller 的有界目录快照，只返回显示名、默认标记和不透明 token。
3. 单个目录每类最多 64 个设备；显示名去除控制字符、压缩空白并限制 160 个 Unicode scalar。
   新查询替换同 caller 的旧目录，存活目录总数最多 32 个；开始请求只能消费当前目录一次。未知、
   重复、跨类、跨 caller、过期或与音频模式无关的 token 必须在消费冻结截图会话前拒绝。
4. “跟随系统默认”不使用设备 token，保持当前行为。显式选择会在 source plan 中保存原生身份：
   Windows 用 `IMMDeviceEnumerator::GetDevice`，Linux 设置 `target.object`，macOS 设置
   `SCStreamConfiguration.microphoneCaptureDeviceID`。连接时设备消失必须明确失败，不能回退默认。
5. 系统声、麦克风和双源模式分别只接受所需类别。双源允许两类独立选择，继续进入现有共享时钟、
   48 kHz 归一化、100 ms 水位、固定 −6 dB/路和单 Opus 音轨链。
6. 录屏选区工具条保留音频模式按钮；存在可选设备时提供紧凑设备面板，分别选择系统声和麦克风。
   模式切换会隐藏无关字段，但不把任意字符串传给后端。目录加载失败时仍可使用系统默认设备。
7. 能力查询与平台枚举不得阻塞 WebView 事件线程。IPC caller 继续只允许本次 Recording 覆盖层，
   普通截图、控制窗、结果库和主窗口不能读取目录或提交选择。
8. 人工 QA 增加默认/非默认设备、同名设备、目录刷新、开始前拔出、录制中拔出、双源独立选择和
   30 分钟漂移。设备拔出仍按当前 source 错误中止会话并保留已提交恢复分段。

## Acceptance Criteria

- [x] 纯 Rust 目录测试覆盖 caller 绑定、一次消费、刷新失效、类别约束、模式约束、数量/标签预算和
  原生身份不序列化。
- [x] 前端测试覆盖能力校验、默认选择、模式切换、两类设备选择、目录失败回退和伪造 token 拒绝。
- [ ] Linux 原生测试证明选中 node 写入 `target.object`；Windows/macOS 原生编译与合同测试证明显式
  ID 进入对应 source plan，且连接失败不会回退默认。
- [ ] 默认、单音源、双源混音、暂停/恢复、异常恢复和结果库行为不变；本地完整门禁与同一 SHA
  Ubuntu、Windows、macOS、macOS Intel 录屏原型 CI 通过。
- [ ] Windows 10/11、macOS Intel/Apple Silicon、Linux X11/Wayland 真机记录默认与非默认设备、
  拔出、同名设备、听感、暂停、强杀恢复和至少 30 分钟 A/V 漂移。

## 当前验证边界

- Linux x86_64 已执行完整 `./scripts/ci-local.sh`：25 项通过、0 失败；按脚本配置跳过非宿主平台
  交叉 lint 与 AppImage 可视 smoke，跳过项不计为通过。Rust 全量 1139 项与前端 73 个文件、
  1274 项通过，Canvas/布局像素 smoke 与 X11 录屏、4K/8K 剪贴板回归通过。
- 设备目录纯 Rust 测试 8 项通过；录屏 API/覆盖层定向测试 61 项通过，包含启动失败后换发一次性
  目录再开放重试。
- Linux `recording-linux-av-qa` 编译通过；PipeWire 合同测试证明显式 node 写入 `target.object`，
  default metadata 解析已有测试，初始化往返设有五秒上限。
- Windows 设备枚举、属性读取、`GetDevice` API 以同版本 `windows` crate 在
  `x86_64-pc-windows-msvc` target 通过独立编译；完整应用交叉检查受本机缺少 MSVC SDK 阻塞，仍由
  Windows Native Check 判定。
- AVFoundation 非弃用发现 API 以同版本 objc2 crate 在 `aarch64-apple-darwin` 与
  `x86_64-apple-darwin` target 通过独立编译；完整 macOS feature 与 selector 行为仍由原生 CI/真机判定。

## Out of Scope

- 录制中的无缝设备热切换、自动回退新默认设备或跨设备交叉淡化；
- 保存跨重启设备偏好、按应用选择系统声来源、每路增益 UI、独立多音轨或后期调音；
- macOS 指定系统输出设备、macOS 14 麦克风后备、摄像头、直播推流；
- 把录屏或音频 QA feature 加入默认/release 构建。
