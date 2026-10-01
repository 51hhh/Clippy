# OCR 子进程取消合同同步修复

需求 ID：`OCR-PROC-CANCEL-01`；父任务：`WIN-NATIVE-01 / W15`。
基线：`42e52c064aba36bb7e93a5e68a0cfb54f65c5b7b`。

## Goal

让实际子进程启动、消费者离开与监督器回收按可观测阶段发生，修复 macOS 原生 CI
暴露的测试预算竞态，保留子进程回收先于许可复用的生产合同。

## Requirements

1. 只修改进程测试夹具与合同；不改变 OCR 生产超时、取消策略、管道上限或许可生命周期。
2. 慢启动夹具在写 PID 前延迟 750 ms，超过原测试的 250 ms 执行预算，必须实际进入测试阶段。
3. PID 就绪后确认任务未完成、消费者取消且许可仍占用；后继任务在子进程回收前不能进入工作。
4. 使用夹具控制标记触发超过既有 stdout 上限的输出，走真实监督器 kill/wait 清理路径，
   不靠随意 sleep、重试、忽略或删断言制造通过；启动/清理等待仍有硬截止时间。
5. Linux 保留 /proc 存活/僵尸检查；macOS 对当前用户的已启动子进程用 signal 0 验证回收。
6. 原有 250 ms 超时测试保持不变；区分本机 Windows 监督器探针与 Unix 原生合同测试证据。

## Acceptance Criteria

- [x] 原 macOS CI failure 的 SHA/job/断言已记录，Windows 实际生产监督器探针复现 750 ms 慢启动被 250 ms 预算提前终止；不声称复现 CI 的具体调度耗时。
- [ ] 无延迟与 750 ms 慢启动两组均实际启动，取消/占用/排队/回收/恢复断言成立。
- [ ] macOS 回收断言实际执行；原超时/非读输入/输出上限合同保持通过。
- [ ] 格式、受影响平台原生测试与同 SHA 三平台/四原型 CI 通过；未执行项保留。
- [ ] PR、CHANGELOG 与本 spec 引用相同需求 ID，桌面未验证项不改成通过。

## Out of Scope

- 更改 OCR 产品取消语义、生产识别预算、Tauri runtime、队列或 IPC。
- 混入 WebM 参数补丁，或把 Windows 监督器探针算作 Unix/桌面 QA。
- 合入 dev、发布、安装软件或更改系统信任。

## Verification

初始 failure：[CI run 36819267902](https://github.com/51hhh/Clippy/actions/runs/36819267902)，
SHA `437949bd74a7d4b852581500834f8cb646cf1895`，macOS native job `110231086906`。
Rust 1054 passed / 1 failed / 5 ignored；`process_tests.rs:211` 的“假进程必须实际启动”失败。
WebM 补丁未修改该模块。原测试等待启动最多 10 s，但实际进程监督器只允许 250 ms；
启动晚于执行预算会先被回收，再等待已无法生成的 PID。具体 CI 解释器调度时长未输出，
不能把静态竞态解释写成已观测到的启动耗时。

Windows 本机不会编译 `cfg(all(test, unix))` 的进程测试；本机格式/探针不替代 Unix 原生 CI。
Windows 11 单屏桌面、Windows 10、多屏、音频与安装升级仍未执行。

本机 Windows 探针直接编译未修改的 `src-tauri/src/ocr/process.rs`，Tokio 1.52.1 与仓库锁文件一致。
750 ms 慢启动在 250 ms 原预算下返回超时，尚未写出启动标记；夹具阶段控制的 20 s 兜底下
进程实际就绪且监督器未完成，释放控制标记后按输出上限触发 kill/wait，Windows PID 查询确认
子进程不再存在。该探针只验证竞态前提与监督器清理路径，不含 Tauri runtime/消费者合同，
不计为 Unix 原生测试或桌面 QA。日志/记录位于父 checkout 的 ignored target：
`ocr-process-budget-probe.log`、`ocr-process-budget-probe.json`。Rust fmt 与 whitespace 检查通过。
