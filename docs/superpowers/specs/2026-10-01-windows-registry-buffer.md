# WIN-REGISTRY-BUFFER-01 — Windows 构建号读取的内存与长度边界

## Goal

消除截图依赖 xcap 在读取 Windows 构建号时违反 Vec 初始化合同的问题。
对应 WIN-NATIVE-01 / W28，基于 f84e6be；桌面操作保持停止。
只修复本地注册表读取边界，不声称已观察到实际崩溃或版本误判。

## Requirements

1. RegGetValueW 的 pcbData 是字节数；读取前提供完全初始化、正确对齐的 2048 字节缓冲区，
   不再用返回字节数设置 Vec<u16> 长度。原生读取仍使用既有键、值和 RRF_RT_REG_SZ。
2. 仅在读取成功后解析返回范围；拒绝奇数字节数、超出缓冲区、缺失范围内 NUL、无效 UTF-16
   或无法解析为 u32 的内容，沿用失败返回 0 的既有行为。不得读取返回范围以外的尾部。
3. 生产与离线测试共用读取/解析入口；验证 Win10/Win11 字面样本、初始化/容量、短返回与尾部、
   错误长度、失败写入以及文本错误。测试不访问注册表、屏幕、捕获对象或输入设备。
4. 不改变版本阈值、BGRA 转换、WGC 生命周期、公开 API、其它平台或依赖锁。
   修改/新增 vendor 文件的 LF 原始字节继续固定，独立篡改负例必须失败。
5. 本机/Windows Native CI 显式执行独立 vendor 合同；完整默认/录屏 QA 门禁绑定干净源码 SHA。
   文档与 CHANGELOG 引用同一 ID，实际桌面和新 SHA CI 保留未验证。

## Acceptance Criteria

- [x] 源码/API 合同证明原 set_len 将字节误作 u16 数量；不运行原未初始化内存路径。
- [x] 安全的旧单位协议辅助红基线失败，修复后生产共用入口的边界回归通过。
- [x] vendor 字节正例/独立篡改负例、严格原生编译与完整 Windows 默认/QA 门禁通过。
- [x] 本机/CI 入口、补丁说明与证据同步，桌面、新 SHA CI 和其它宿主保留未验。

## Out of Scope

重现未定义行为、Miri/ASan 证明、修改系统注册表、重构版本检测、实际截图/录屏、安装包、
恢复桌面操作、Windows 10 与多屏真机验收、Linux/WSL、合入 dev 或发布。

## Review evidence

旧 get_build_number 以 Vec::with_capacity(2048) 分配未初始化 u16 内存，让 API 最多写入
2048 字节，再以返回字节数调用 set_len。普通 `26100\0` 是 12 字节，仅初始化 6 个 u16，
set_len(12) 却声明 12 个元素有效，违反 Rust 的初始化前提；实际崩溃未观察。

- [Microsoft RegGetValueW](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-reggetvaluew)：
  输入/返回长度单位均为字节，字符串长度包括终止 NUL。
- [Rust Vec::set_len](https://doc.rust-lang.org/std/vec/struct.Vec.html#method.set_len)：
  新长度以内的元素必须已初始化，且不能超过容量。

## Validation

完整生产纯模块的 MSVC Rust 1.98 harness：八项绿合同通过；安全旧单位模型 4 passed / 4 failed，
退出 101。红模型将内存初始化并以 truncate 代替 set_len，保留错误单位，故不声称执行或复现 UB。
无类型 stub，不访问实际注册表/捕获对象。日志和原始哈希见 ignored target 的
windows-registry-buffer-contract/RESULT.json。

产品修复提交 326af9e5afbc3d099d886003a78ec05a7ae053ea，独立 CI 接线
8c6fbfe4f111db6562ce38cf740342afd7ee8f5d。该 SHA 首次完整门禁 29 passed / 1 failed /
1 skipped、exit 1，失败为遗留 U16CString 导入触发 vendor 严格 lint；八项回归实际通过，
整轮不计为通过。windows-registry-buffer-native-qa-8c6fbfe/RESULT.json 与原日志保留。

移除导入并同步原始哈希后的干净被测源码 bb38cc6c98a141d95f67834f4aeb8f98f2318881：
完整 ./scripts/ci-windows.ps1 -RecordingQa exit 0，30 passed / 0 failed / 1 skipped（Linux smoke）。
默认 Rust 1058 / 5 ignored、QA Rust 1115 / 5 ignored，两个图重叠不相加；前端 75 文件 /
1292 passed，Python 33 + 3。vendor 八项构建号、六项关闭、四项初始化合同均通过，十八项
独立于应用 Rust 总数；构建号组过滤十一项，关闭组十三项，初始化组十五项，其中仅一项上游
显示器测试，其余为其它离线组，filtered 不计为通过。默认/QA API 图、vendor 严格 lib/tests
clippy、原始字节校验、生产构建通过，验证后检出干净，stdout/stderr 原始哈希已核对。
证据 windows-registry-buffer-native-qa-bb38cc6/RESULT.json。

最终供应链隔离夹具复制实际 verifier 与十一项 pins：正例 exit 0，在 registry_build/utils/mod
各追加一个 LF 的独立负例均 exit 1 且指明文件，恢复后 exit 0，生产文件未篡改。
来源记录、锁文件和许可证保留，不归一化原始字节；证据 windows-registry-buffer-supply-chain-final/RESULT.json。
Windows Native CI 的条件、命令与工作目录经 YAML 解析核对，Windows PowerShell 5.1 解析零错误；
远程当前 SHA CI、真实注册表/截图/录屏、桌面、其它宿主仍未验。未安装新包、合入 dev 或发布。

本轮亦静态检查 Windows 持久化调用锁、自启动开发版保护和 UIPI 边界，未确认新增产品缺陷。
跨屏混合缩放坐标仍保留 WIN-NATIVE-01 / W04，不能由本轮离线边界合同替代真机验收。
