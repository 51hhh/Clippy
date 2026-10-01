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
- [ ] vendor 字节正例/独立篡改负例、严格原生编译与完整 Windows 默认/QA 门禁通过。
- [ ] 本机/CI 入口、补丁说明与证据同步，桌面、新 SHA CI 和其它宿主保留未验。

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
windows-registry-buffer-contract/RESULT.json；真实 vendor Cargo、供应链与完整门禁待记录。
此前 61d6823 的门禁不替代本修复，新 SHA 远程 CI、真实注册表与桌面测试未运行。
