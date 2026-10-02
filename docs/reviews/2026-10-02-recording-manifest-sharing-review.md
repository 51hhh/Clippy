# WIN-NATIVE-01 / W56 — Windows 清单临时共享冲突

需求 `REC-MANIFEST-SHARING-01`；基线 `9a7a6e2738c7014e747eff973b812c02d54006a4`，
独立分支 `codex/recording-manifest-sharing`。规格
[`2026-10-02-recording-manifest-sharing.md`](../superpowers/specs/2026-10-02-recording-manifest-sharing.md)
在生产修改前建立。本轮仅源码与已有 Windows 原生 CLI；桌面继续停止。

## 已复现的问题

使用原 DiagnosticRecordingSession、MJPEG encoder、worker/journal，以及原
10 FPS/200 ms 分段配置。初始 manifest.json 用真实原生文件句柄打开，允许读写，
不共享删除；第三帧后继续短暂持有 75 ms。旧实现约 0.30 秒退出，取得实际 encoder
Journal 错误：`原子写入恢复清单失败: 拒绝访问。 (os error 5)`。初始 native exit 101，
0 passed / 1 failed；没有将实际错误 5 写成错误 32。

原 W53 AVI 30 秒失败只保存等待断言，没有保留 worker 原错，不能据新受控复现
认定其原因。原测试、期限和失败证据保持；新夹具在 worker 退出后收集实际结果。

## 修复行为

只有 Windows 录屏清单替换启用有界重试。原始错误 32/33 可重试；错误 5 必须用
原生删除访问探针确认源或目标返回共享冲突 32，才进入同样的重试。最多 21 次尝试，
500 ms 重试窗口，每次等待最多 25 ms；原生单次调用自身的阻塞不声称受该窗口限制。
真正的权限拒绝、其它错误、永久冲突仍返回原错误；源已提升而消失时不再重试。

探针只打开现有文件并关闭句柄，使用全部共享及不跟随 reparse point 的标志，不
实际删除或改变 ACL。Windows 的访问/共享兼容检查与删除共享对重命名的影响见
[CreateFileW 文档](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)；
替换继续使用原 [MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
权限工具。探针成功或权限拒绝不作为错误 5 的共享证明。

私有 temporary、原子替换、ACL 与 sync 顺序保持；清单提交后分段提升、恢复哈希、
统计和采集预算不改。其它文件保存、原权限模块与 Linux/macOS 路径保持原行为。

## 验证证据

首轮领域 385 passed / 0 failed，原 377 项与新增 8 项均在内。新增运行期一项使用
真实句柄和原完整录屏路径，核对周期提交、Stop、AVI 文件、清单哈希/长度/帧数/时长。
新增七项 API 合同覆盖实际暂时/永久共享冲突、只读权限拒绝、未确认错误 5、其它错误、
已提升源、32/33 分支与次数预算；API 合同只在修复图执行。

<!-- W56_GATE_PENDING -->

证据目录 `src-tauri/target/recording-manifest-sharing-contract/` 保存 W55 原状态、
初始红、阶段绿、旧 Git 源码重放/恢复、最终领域及冻结 SHA 门禁，附输入/日志哈希。
首次旧实现重放因沙箱跨目录写入限制在启动原生测试前退出，finally 已恢复源码；
经自动审批仅放行项目内证据写入后执行，不计作测试失败或通过。

## 未完成项

实际设备/桌面/安装器/无 CRT 启动、当前 SHA 三宿主/codec CI、其它宿主本地门禁、
Windows 10/混合 DPI/多屏/负坐标与发布未验。本 SHA release 未构建，旧安装
`45769c9` 与历史 QA EXE `1c66112` 不含本修复。W53 原 AVI 超时原因仍未知；
不能用本轮受控共享错误或后续通过替代其根因证明。旧桌面记录仍为
2 pass / 1 fail / 36 not_run，整体目标未完成。
