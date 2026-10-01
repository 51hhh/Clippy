# WIN-PIN-LIVE-DPI-01 — Windows Pin 实时渲染 DPI

## Goal

修复 Windows 图片 Pin 跨 DPI 后仍按创建时比例选择最近邻滤镜的问题。
对应 WIN-NATIVE-01 / W04 的 W34，基线 c6075e2。桌面操作保持停止。

## Requirements

1. Windows 图片渲染使用当前原生窗口的 DPI；首读之前先订阅当前窗口的原生 scale 事件。
   事件已经更新比例时，迟到的初始查询不能覆盖它；payload 或用户状态应答也不能覆盖实时比例。
2. 比例必须有限且为正。等待、读取失败、订阅失败或无效值时使用 auto；后续有效事件可恢复。
   不借助旧 payload 或浏览器默认比例猜测 Windows 当前 DPI。异步订阅晚于卸载完成时仍清理监听。
3. 非 Windows 继续使用既有 device/buffer scale 与补偿语义，不查询或订阅 Windows 实时路径。
   内容尺寸、用户 zoom、窗口摆放、源像素/工程/复制/保存及现有 payload/SQLite 格式保持。
4. Tauri SDK 操作只位于登记的 api/pin.ts，经 api.ts facade 和 Pin service 进入业务代码。
   首读经只读业务命令注入原生调用者窗口，不接受目标 label；校验仍存在的安全 Pin 条目、
   拒绝其它调用者。不新增通用 core 窗口权限，既有 capability/业务命令 gate 保持。
5. 新回归运行真实 React App 和 DOM 滤镜判据，验证 100%/125%/150% 切换、没有 resize 的事件、
   首读/事件/初始 payload 竞争、无效/失败、安全回退、卸载清理及其它宿主不访问原生新路径。
   SDK 适配器合同验证 current-window 操作和 callback 值，不将 mocked API/jsdom 计为真机结果。
6. 保存未改生产 App 的红基线与同组绿色回归；完成干净 SHA Windows 默认/QA 门禁和源码/日志
   哈希核对，同 ID 同步审查、计划与 CHANGELOG。真实 WebView2 像素/窗口事件与其它宿主仍未验。

## Acceptance Criteria

- [x] Windows 使用实时原生 DPI 选择滤镜，迟到首读/旧 payload 不回退比例。
- [x] 未知/无效/失败回退 auto，有效事件恢复，异步卸载无监听泄漏。
- [x] 非 Windows 既有行为、来源/内容/展示/持久化合同保持；首读绑定原生 Pin 调用者。
- [ ] 旧 App 红基线、同组绿色回归、适配器和干净 SHA Windows 默认/QA 门禁核对完成。
- [ ] 文档同 ID 同步，真机与当前 SHA CI 未验证边界保留。

## Out of Scope

不改变跨屏 CSS 尺寸或用户 zoom，不重采样/修改图片，不重构 Linux GTK 补偿，
不改录屏 monitor-local crop 语义，不操控桌面、不安装包、不运行 Linux/WSL、不推送/合并/发布。
W04 的真实 WM_DPICHANGED、多屏、热插拔和 Windows 10/11 矩阵不由离线合同关闭。

## Review evidence

现有 App.tsx 传 pin.deviceScale 到 pinImageRendering；payload 只在创建时读取，PinState 不包含
实时 DPI。pinImageRendering 以 isPixelExact 选择 pixelated，没有实时比例订阅。800×600 CSS、
1200×900 像素、创建比例 1.5 在当前比例变为 1 时不再像素对齐，旧判据仍返回 pixelated。
锁定 Tao 0.34.8 的 WM_DPICHANGED 更新 window_state.scale_factor、保留逻辑尺寸并发送
ScaleFactorChanged；Tauri 2.10.3 将 scaleFactor 和 size 发送给当前窗口。锁定 JS SDK 的
scaleFactor 调用 plugin:window|scale_factor，onScaleChanged 订阅当前窗口 scale 事件。
项目既有权限合同禁止通用 scale-factor grant，最终首读改用注入原生 WebviewWindow 的业务
命令；锁定 Tauri 的 CommandArg 实现直接从 command.message.webview() 取得该窗口。
 capability 原始字节保持，前端只有现有事件监听权限。
这是源码与数值合同证据，尚未观测真实 WebView2 成像或原生事件到达。

录屏续审：RecordingCaptureSpec 明确只接受显示器 ID 与屏内像素 crop，平台复核冻结尺寸；
Windows prepare/connect 重新查询，并比较实际 descriptor 的物理 crop 原点/尺寸。冻结到 prepare
的原点不一致本身不足以证明 bug，不擅自改变现有 monitor-local 语义。真实热插拔/身份重用仍未验。

## Validation

原生产 App 和 renderer 与 c6075e2 原文一致：十二项真实 React/DOM 滤镜合同在 Windows Node
上红基线 exit 1，2 passed / 10 failed；同一测试文件修复后全部通过。新增适配器三项、既有
权限和 Pin 回归共五文件 74 passed，TS 检查 exit 0。首次 jsx 扩展名不在 Vitest include 的
发现失败保留为单独日志，不计为产品红基线。MSVC 六项生产 pure-data read_for_pin 合同 exit 0，
0 failed / 0 ignored / 1129 filtered；回归不调用原生窗口/显示器/光标/输入 API。
完整干净 SHA 默认/QA 门禁待执行；真实成像、事件、硬件矩阵和当前 SHA CI 未验证。
红绿/源码目录 C:\win\Clippy\src-tauri\target\windows-pin-live-dpi-contract。
