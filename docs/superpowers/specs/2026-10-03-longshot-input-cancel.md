# WIN-LONGSHOT-INPUT-CANCEL-01 — 长截图取消撤销旧线程输入

## Goal

补齐 WIN-NATIVE-01 / W04、PX-LS-NATIVE-AUTO-01 的受控输入边界。取消旧长截图后，旧 append
线程仍可能处于输入准备、settle 或抓帧中；它不能继续发起指针移动、窗口激活、滚轮或恢复移动。
已进入的原生输入调用需要结算，不声称撤销已提交给操作系统的事件。

## Requirements

1. 输入许可按后端会话创建，owner 与锁外 lease 的 target 克隆共享；新一代获得独立许可，
   旧/错误 token 的取消不得撤销新会话。拒绝使用前端提供的坐标或新增系统授权。
2. 匹配取消先不可逆撤销许可，再按原 manager 规则使 lease 失效。输入 mutation 与许可检查
   串行；取消完成前等待已经进入的输入调用结算，不等待抓帧、settle、拼接或 PNG 编码。
3. 原生指针移动、Windows/macOS 激活、Enigo 滚轮和 RAII 恢复均使用同一许可。后台初始化、
   settle/目标检查和抓帧边界复核许可；取消后迟到的恢复不得移动用户指针。
4. 保留物理坐标、窗口/PID 锁、UIPI/权限检查、四方向、3像素容差、原45/360/500毫秒等待，
   原业务错误及图像质量门。不强杀线程、不自动重试、不放宽期限，不把输入返回成功当正确捕获。
5. Wayland 原 Portal cancel 保持；共享原生路径影响 Windows/X11/macOS，不能用本机 Windows
   构建与纯副作用夹具冒称其它原生图、真实用户接管/设备或系统调度已经验证。
6. 原生产输入边界提取的无撤销协议以受控指针/回调和真实线程复现，明确提取范围；修复后的
   同一回归正文覆盖真实 controller 取消、克隆/后继隔离、已进入调用顺序和 RAII 恢复。
   不调用本机输入 API；旧测试正文、完整 include、断言与等待期限保持。
7. 独立分支，规范/CHANGELOG/计划使用同 RID；完整 Windows 默认/QA 门禁绑定干净源码与日志。
   产品修复累计从49增加此一项，测试诊断修复另列；历史失败及未验边界完整保留。

## Acceptance Criteria

- [x] 提取的旧无撤销协议在原生受控回归失败，源码、差异、输入、日志与非零退出保存。
- [x] 同正文证明真实 owner 取消使克隆许可失效，旧 token 不能撤销后继会话。
- [x] 已进入输入调用先结算，取消返回后不再执行旧输入；恢复 guard 不移动，正常旧合同保持。
- [x] 旧正文/期限与固定来源保留，干净 Windows 默认/QA 完整门禁通过，记录同步。
- [ ] 当前源码同 SHA 七项 CI、其它宿主、真实取消/输入/DPI/权限及完整交付验收完成。

## Out of Scope

- 桌面控制、安装、真实截图/鼠标/录屏/音频、推送、PR、合入或发布。
- 撤回 OS 队列事件，保证原生 API 不阻塞，改变 Windows 前台锁/路由或 Wayland Portal 实现。
- 用户移动后又回到原点的全局输入跟踪、外部程序竞争、跨窗口继续或绕过高完整性窗口。
- 用此修复解释 W53/W59/W63/W67 历史失败，重写旧证据或宣称旧 release 包含新修复。

## Review Evidence

基线文档 `64e5abfd77e97ac164a9801a7618c5e4190f4eca`。原 controller.cancel 只调用
cancel_wayland，然后使 manager lease 失效；该方法在 Windows 为空。原 with_scroll 没有会话
取消检查，旧 lease 的输入与 CursorRestore 仍可继续，最终 manager 拒绝结果不能撤回这些调用。
这是源码/受控副作用协议缺口，真实 OS 用户场景仍须单独验收。

## Verification（W81）

先定义合同`6d49204`，修复源码`00f40cc5cf8b3cf3c332dc7cce6be47cd07aefcc`。
原无撤销 mutation 边界提取为不检查/不撤销的转发协议，使用真实controller、克隆、线程和RAII：
0 passed / 4 failed，Cargo101 / 包装器终端1；修复后同三份回归文件原字节4/0、全部exit0。
这是提取协议、受控副作用的红绿，不是未修改原应用或真实OS输入场景复现。

许可先以原子标记撤销，manager立即失效，再等待已有输入临界区；后继许可独立。Mutex只围住
指针移动、窗口激活、滚轮和恢复的同步mutation，不包含聚焦poll/settle/抓帧/像素处理。
23项原父测试正文和其它1066份完整输入保持，原常量/权限/质量门保持；原Wayland分支及Portal cancel保持。

干净源码完整Windows PowerShell5.1门禁33 passed / 0 failed / 1 Linux smoke skipped；默认Rust1334、
QA1486各5ignored、前端81文件/1405，QA录屏516包含于1486。新4项两图共8次执行已含总数，
旧W80通过全名仍在相同阶段通过；1072份冻结输入、Git tree和原日志/实际退出绑定。
当前SHA release/七项CI、其它宿主与真实取消/输入/DPI/权限仍未验；产品修复累计50，测试诊断另列。
已进入的输入临界区须先完成；不能撤回OS队列事件或保证原生API不会阻塞。末项全局AC未勾选。
见 [审查记录](../../reviews/2026-10-03-longshot-input-cancel-review.md)。

W82后续验证：同一00f40cc源码默认/显式QA release编译及PE/CRT文件核对完成，751编译输入/1072门禁输入绑定；只验证文件，不计测试或真实启动。原门禁保持，末项复合AC、同SHA CI/其它宿主/真实桌面与完整交付仍未完成。
见 [当前release文件验证](../../reviews/2026-10-03-windows-longshot-input-cancel-release-review.md)。
