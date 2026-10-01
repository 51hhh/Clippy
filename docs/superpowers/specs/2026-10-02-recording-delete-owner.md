# REC-DELETE-OWNER-01 — 录屏恢复合并与删除的会话所有权

## Goal

录屏恢复合并持有会话时，结果库删除不能撤销播放/缓存或删除其已提交产物；删除持有会话时，
同会话合并和重复删除不能进入。对应 WIN-NATIVE-01 / W06 的 W38，基线 ab13a3a。

## Requirements

1. 既有 VP9 feature 图内，合并和删除在同一个进程级 registry 原子取得会话所有权。
   保持全进程同一时间只合并一个会话；同会话删除互斥，不限制其它会话的合并/删除。
2. 删除所有权覆盖原后台 worker 的整个播放撤销、缩略图清理和 manifest 删除过程。遇到冲突
   在这些操作前返回 recording_library_delete_busy；同会话删除占用时合并返回既有 merge_busy。
3. 成功、原文件/存储错误和 callback panic 都通过 RAII 释放所有权；错误不被清理错误覆盖。
   不取消或中断正在合并的任务，不用前端窗口的 busyKey 作为后端所有权。
4. 从实际删除 worker 提取可测操作入口；旧无所有权删除协议红基线使用生产 registry、真实
   journal/VP9 writer/manifest 文件和受控线程验证。不是未经改动的 Tauri IPC 或桌面复现。
5. 原 caller 检查、IPC 名称/参数、默认 feature 删除路径、播放/缩略图清理顺序、manifest
   文件/完整性边界、合并/恢复算法保持；不添加依赖或改权限/录屏默认门控。
6. 干净 SHA 完整 Windows 默认与录屏 QA 门禁、原断言/源码/日志核对；同 ID 同步审查、计划、
   CHANGELOG。新所有权测试仅进入 VP9/QA 图，不重复计入默认图，图谱重叠不累加。

## Acceptance Criteria

- [x] 已持有合并所有权时删除不执行任何 worker 操作，已提交文件和 manifest 原始字节保持。
- [x] 删除持有期间同会话合并/重复删除失败，其它会话操作保持可用；真实线程顺序可验证。
- [x] 删除成功/错误/panic 后可重新取得所有权，原错误保持；真实恢复后可正常删除。
- [ ] 旧协议红基线与同组回归、旧单槽/manifest/前端合同及干净 SHA Windows 门禁核对。
- [ ] 同 ID 文档同步，真实重开窗口/录屏/强杀/其它宿主与当前 SHA CI 保留未验。

## Out of Scope

桌面操作保持停止。不安装新包、不运行 Linux/WSL、不推送/新 PR、合入或发布。
不改变跨进程文件访问策略、导出路径身份、媒体租约/缩略图生成并发合同、manifest 提交协议
或恢复算法。关闭/重新打开真实窗口、系统强杀恢复和设备/最终播放尚未验收，不能由纯文件/
线程合同代替。共享代码本机只编译 Windows，其它宿主和当前 SHA CI 保留未验。

## Review evidence

恢复合并在 spawn_blocking 前取得 RecordingMergeRegistry guard，后台任务和窗口寿命独立。
delete_recording_session 没有同会话所有权检查。结果库 React App 的 busyKey 限制当前挂载，
关闭按钮不因合并禁用，重新挂载从 null 开始；这不能替代后端保护。需用生产文件和 registry
复现，再将确认的问题写成修复结论。红基线确认生产文件确被删除，修复已针对该所有权缺口。

## Validation

Windows MSVC 提取原删除 worker 无所有权协议红基线：原生 child exit 101，终端 wrapper
exit 1，3 passed / 3 failed / 0 ignored / 1212 filtered。一个失败实际删除了 merge guard
持有期间由真实 journal/VP9 writer 生成并提交的两个 WebM 分段与清单；其它两个失败确认
同会话合并/重复删除可进入，包括受控真实 worker 线程。不是未经改动的原 IPC 或桌面复现。
同组六项原始测试字节不变，修复后原生 child/终端 exit 0，6 passed / 0 failed / 0 ignored /
1212 filtered。实际恢复 remux 后删除、错误保留/重试和 panic 释放已覆盖，panic 日志来自
显式 catch_unwind 注入，并非用例失败。既有全局合并单槽一项红/绿均通过。
十八段原结果库函数、原 worker 正文（归一化空白）、旧单槽测试模块原字节和十一份关联文件
核对；默认分支仍直接执行原 worker callback，新所有权和六项测试仅进入 VP9/QA feature。
首次启动因子进程目录未找到 Cargo.toml，未执行任何测试，错误日志独立保留，不计红基线。
完整 Windows 默认/QA 门禁待执行，真实重开窗口/系统强杀/设备/最终播放、其它宿主与当前
SHA CI 留未验。证据目录 C:\win\Clippy\src-tauri\target\recording-delete-owner-contract。
