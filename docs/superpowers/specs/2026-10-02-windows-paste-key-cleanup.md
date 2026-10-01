# WIN-PASTE-CLEANUP-01 — Windows 自动粘贴的部分按键失败清理

## Goal

Windows 自动粘贴的 V Click 未全部发送或展开时，显式尝试释放可能已按下的 V，
不把未进入 Enigo held 列表的按键留给无效的默认 Drop。对应 WIN-NATIVE-01 / W45，
基线 10fd2ea；原协议和受控 Keyboard/Drop 已复现清理缺口，不声称实际系统卡键已观察。

## Requirements

1. 成功路径保留 Control Press → V Click → Control Release，V Click 的按下/释放仍由
   同一次原输入库调用发送，不拆分正常路径，不新增权限、依赖或系统按键状态读取。
2. Windows V Click 返回错误时，先尝试 V Release，然后仍尝试 Control Release。
   V Release 首次失败或 Click 展开时，作用域退出再尝试一次 V Release。清理有界，
   不循环重试；原 Enigo Drop 继续清理其已记录的 modifier，不能假设其记录了 Click。
3. 保留原失败分类和主要 action/detail。清理错误附在主要错误 detail，不掩盖 Click
   错误或虚报成功；持续阻止释放时仅记录/返回失败，不宣称物理键状态已恢复。
4. modifier Press 失败不发送 V；后端/目标校验失败不发送任何按键。原窗口/PID/前台
   复核、完整性边界、copy-only 结果及事件保持。macOS 原协议与 Linux/Wayland 分流保持。
5. 提取原注入决策，仅替换 Keyboard adapter 建立红基线；受控模型按锁定 Enigo 0.6.1
   的 Click 队列、成功 Press 的 held 更新及默认 Drop 合同模拟，不实例化真实 Enigo。
6. 同组新测试原字节、旧六项目标复核正文及相关源码核对；干净 SHA 完整 Windows 默认
   与 recording-windows-av-qa 门禁。新测试计入两图总数，重叠不累加，跳过与未验保留。

## Acceptance Criteria

- [x] 原协议在部分 Click 模型中返回错误后仍留下 V-down，记录失败断言和 Drop 范围。
- [x] 新协议成功清理部分 Click；V Release 首次失败与展开退出时有界重试，modifier 清理保持。
- [x] 永久清理失败不报成功，主要错误保持并包含清理失败；正常成功和早期失败顺序保持。
- [x] 原六项目标复核、错误/command 与 macOS/其它平台正文核对，同组原字节红绿回归。
- [ ] 干净源码完整 Windows 门禁及同 ID 文档同步；系统/其它宿主/当前 SHA CI 未验项保留。

## Out of Scope

不调用 SendInput、Enigo::new、实际键盘/剪贴板/窗口/前台 API，不启动应用或操控桌面。
实际用户同时按住 V、输入竞争/UIPI、持续系统阻塞、其它宿主原生编译、安装与 Wayland
回归不由受控模型证明。没有 Linux/WSL、安装、推送/PR/合入/发布。

## Primary references

[SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)
返回成功插入的事件数，队列按序插入；当前键盘状态不会被该 API 重置。原 Enigo 0.6.1
源码及 Cargo.lock 字节已保存：Click 组合两个事件，返回错误不更新 held；默认 Drop
仅重试已记录的 Press。模型不是实际 Win32 部分发送或物理按键状态证据。

## Verification

Windows V Click 错误后先尝试 V Release，仍释放 Control；首次 V 清理失败或 Click
展开时 RAII guard 再尝试一次。成功路径仍 Control Press/V Click/Control Release，
不拆分正常 Click。原主要 action/detail 和错误分类保持，清理失败追加 detail，持续
阻塞不报成功；原 Enigo held modifier Drop 重试保持。仅 Windows 新协议，macOS
原正文、输入初始化/目标复核、完整性边界、copy-only 与其它平台分流不变。
提取原注入决策只替换 Keyboard adapter，MSVC 红 10 passed / 6 failed，旧六项全绿；
四项模型 V-down 残留、两项释放尝试缺失实际复现（首个断言范围保留）。同十项测试/
Keyboard fixture 原字节绿 16 passed。模型按锁定 Enigo 0.6.1 的 Click/held/默认 Drop
合同构造，展开 panic 被捕获；不是实际 SendInput 部分发送、物理卡键或桌面证据。
原 Windows/macOS 实现和旧六项正文、十二份关联文件、两份锁定 SDK 原字节核对。
没有新依赖、SDK feature 或系统键状态调用。
同组定向回归通过；干净源码完整 Windows 门禁待运行。实际按键/桌面、其它宿主/新 SHA CI、安装和 Wayland 留未验。
