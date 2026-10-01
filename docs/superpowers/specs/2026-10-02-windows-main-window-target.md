# WIN-MAIN-TARGET-01 — 主窗口保存位置优先于后备查询

## Goal

修复已有有效保存位置时仍执行后备显示器查询、后备错误阻止主窗口显示的问题。
对应 WIN-NATIVE-01 / W36，基线 301c782；桌面操作保持停止。

## Requirements

1. remembered_target 成功返回目标时直接采用该目标，不执行 cursor/current/primary 后备查询。
   保存物理位置、选中的显示器及既有尺寸/工作区钳位不变。
2. remembered_target 返回 None 时只执行一次既有后备逻辑；后备目标、None 与原错误完整传递。
   remembered_target 自身错误仍直接返回，不由后备查询掩盖。
3. 从实际 show_main_window 提取目标选择入口，用真实生产数据/closure 验证调用次数与错误；
   提取红基线保留原先 remembered?.or(fallback()?) 求值顺序。不得声称未改原 show 函数或桌面复现。
4. 既有七项主窗口保存/几何断言与配置格式保持。只改变后备执行时机，不新增功能/权限/依赖。
   共享路径对其它平台同样适用，但本机只能证明 Windows 原生编译和数据合同。
5. 干净 SHA Windows 默认/录屏 QA 完整门禁、同组测试字节、源码/日志哈希和既有合同核对；
   spec、总计划、审查与 CHANGELOG 引用同一 ID，保留真实窗口/多屏/DPI/当前 SHA CI 未验边界。

## Acceptance Criteria

- [x] 有保存目标时，成功和失败后备均不执行，保存位置不变。
- [x] 无保存目标时，后备仅执行一次，目标/None/原错误保持。
- [x] 保存查询自身错误不执行后备，旧断言与配置字段保持。
- [ ] 提取旧求值协议红基线、同组回归和干净 SHA Windows 默认/QA 门禁已核对。
- [ ] 同 ID 文档同步，桌面及跨平台缺少的证据保留未验证。

## Out of Scope

不修改保存坐标格式或主窗口显示/尺寸/焦点行为，不重构配置 debounce；不根据源码推断已通过
真实 Windows DPI/多屏/热插拔验收。原生 DPI 消息与窗口约束时序没有真机证据，保持待验。
不操控桌面、不安装新包、不运行 Linux/WSL、不推送/合入/发布。

## Review evidence

show_main_window 先计算 remembered_target，再用 Option::or 接收 target_monitor(app, window)?。
or 的参数会先求值，即使保存目标为 Some，后备查询的错误仍在窗口 show/focus 前返回。
固定 AppHandle 的 monitor_from_point 将 runtime Option 包成 Ok，不能宣称该调用返回原生错误；
cursor 错误由既有 .ok() 忽略。未选到光标屏后的 window.current_monitor/primary_monitor 通过
消息接收返回 Result，接收失败会传递。回归用可控后备错误验证生产选择协议，没有观测实际
原生失败或窗口显示失败，不调用实际原生 API。

MainWindowPosition 接收 Moved 的物理 i32 坐标；WorkArea 与恢复 Position 都是物理坐标，
布局尺寸才按目标 DPI 换算。目前未确认 Pin 式全局逻辑猜屏缺陷，不修改这部分。

## Validation

Windows MSVC 提取原 eager 选择协议红基线 exit 101：13 项中 11 passed / 2 failed；既有七项
全通过，新增六项为 4 passed / 2 failed。不是未改的原 show_main_window 或桌面复现。
同组新测试原始字节不变，修复后 exit 0：13 passed / 0 failed / 0 ignored / 1135 filtered。
生产选择函数接收真实 Result/Option/PhysicalPosition 数据与可控 closure；没有创建窗口/显示器
或查询光标。既有七项测试模块、十八段生产/测试/后备布局正文与十一份关联文件保持，五份
固定原生库源码哈希核对。完整干净 SHA Windows 默认/QA 门禁待执行，其它宿主/桌面/新 SHA CI 未验。
证据：C:\win\Clippy\src-tauri\target\windows-main-window-target-contract。
