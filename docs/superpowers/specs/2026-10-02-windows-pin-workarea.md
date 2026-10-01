# WIN-PIN-WORKAREA-01 — Windows Pin 原生工作区几何

## Goal

修复 Windows Pin 保存工作区、恢复窗口与工具条可见区域共用全局逻辑点猜屏的问题。
对应 WIN-NATIVE-01 / W04 的 W33，基线 7017562。桌面操作保持停止。

## Requirements

1. 保存时优先使用窗口当前原生显示器的物理工作区与外框物理位置；同一目标屏比例转换保存
   位置和参考工作区，不再把窗口位置用当前 DPI 转全局逻辑后按枚举顺序猜屏。
   缺失 owner 时只用有效原生快照的物理交集择屏；缺失位置或可靠显示器不捏造来源。
2. 保持 StoredPinPlacement 的现有字段、SQLite/备份格式与相对位置策略。新保存记录准确保留
   owner 名称和参考工作区；旧正常记录按名称、保存参考几何、主屏兜底映射，不重写旧数据。
   已被旧 bug 写错的来源名称/参考矩形无法可靠反推出真实来源，不能伪称迁移恢复。
3. 恢复时目标原生显示器身份、工作区、DPI 和物理 anchor 一直进入 PinEntry 缓存布局，
   创建/reveal 使用同一物理请求，禁止把已选屏的恢复位置再次交给全局逻辑选屏。
   图片/文字/HTML 工作区均保留所存内容 CSS 尺寸与用户 scale/opacity/locked/above；
   按当前工作区钳窗口，当前 device/buffer scale 与目标一致。缺失记录/目标或无有效快照正常回退。
4. Windows 工具条从客户端物理位置/尺寸与原生 owner 工作区求交，最后只按窗口实际 DPI
   转成窗口局部 CSS 坐标；不把外框当 WebView 客户区。未知位置/owner 回退整个客户端，
   客户端尺寸/DPI 不可用返回现有 UNKNOWN，IPC 四字段、前端刷新和非 Windows 路径保持。
5. 离线回归直接调用生产纯数据入口，覆盖混合 DPI/枚举顺序、负/上下/非整除原点、任务栏、
   客户区/外框偏移、跨屏窗口、窗口 DPI 与显示器 DPI 不同时的单位、保存→真实 SQLite→恢复、
   旧记录与屏幕改名/缩放/移除、无效/缺失快照、两阶段物理请求和原展示参数。
   不创建窗口、调用显示器/光标 API 或发送输入；实际 Tauri/dpi 类型与 MSVC 默认/QA 图编译。
6. 保留旧生产保存/恢复映射及提取旧工具条单位协议的红基线，同组断言修复后通过；
   干净 SHA 完整 Windows 默认/QA 门禁、源码/日志哈希和文档同 ID 同步；证据不替代真实桌面。

## Acceptance Criteria

- [x] 保存位置与参考工作区从同一可靠原生 owner 产生，旧存储合同保留。
- [x] 工作区恢复保留目标屏，物理创建/reveal 和 payload DPI 一致，旧正常记录可映射。
- [x] 工具条求交使用真实客户区和物理工作区，局部 CSS 与窗口 DPI 一致，未知安全回退。
- [x] 旧协议红基线、同组回归、原断言与完整 Windows 默认/QA 门禁及哈希核对完成。
- [x] 文档同步；真实窗口/DPI/热插拔、多屏/Windows 10、当前 SHA CI 与其它宿主仍保留未验。

## Out of Scope

无法恢复的旧错误来源推断、数据库/备份格式重设计、WGC 源身份、实际 OS owner/DPI/热插拔
结果、桌面操控、安装新包、Linux/WSL、推送/新 PR、合入 dev 或发布。W04 真机矩阵不关闭。

## Review evidence

右屏物理 x=2560 @150% 的窗口位置除以 1.5 后可落在主屏 @100% 的逻辑工作区内。
capture_workspace_placement 与 pin_toolbar_bounds 都由 logical_work_area 取首个命中，
因此保存错屏或算错可见区域。map_workspace_position 虽按名称选到目标屏，却只返回逻辑点；
随后 scale_origin、position_new_pin_window 和 reveal 再猜屏，目标身份丢失。
锁定 Tao inner_position 使用 ClientToScreen，inner_size 读 client_rect；outer_position/size
读 GetWindowRect，工具条应使用前者。窗口定位/尺寸 API 是异步请求，合同通过不等于最终 OS 结果。

## Validation

Windows MSVC 提取的旧生产保存/恢复/工具条协议红基线 exit 101：1 passed / 15 failed /
0 ignored / 1113 filtered。原 map_workspace_position 与 visible_window_part 正文保留；不是
未经改动的原窗口函数或真实桌面结果。修复后同一份新回归文件十六项通过，包含真实内存 SQLite
保存/加载与生产几何应用，用户内容与展示参数保留。新回归不调用窗口、显示器、光标或输入 API。

初绿记录保留；Windows 不再使用的扩展导出清理后，干净源码
`4a5810117722c35f48382da31617b595769bbcda` 完整默认/录屏 QA 门禁确认原生子进程 exit 0，
30 passed / 0 failed / 1 skipped（Linux smoke）。默认 Rust 1124 / 5 ignored，QA Rust 1181 / 5 ignored；
重叠不累加，每图本次十六项及此前 Pin 来源十八项包含在各自总数。前端 75 文件 / 1292 passed，
Python 33 + 3；独立 vendor 十八项及剪贴板二十四项通过。check、严格 clippy、供应链、构建/入口
通过，日志原始哈希和门禁前后干净检出已核对；后继仅四份 Markdown。
5 个既有修改文件 96 个 assert 宏 token 保留（只归一化空白），新回归原始字节不变；
SQLite/备份生产文件和此前 Pin 回归源码保持，锁定原生库四份源码哈希一致。
完整结果与审计目录 `C:\win\Clippy\src-tauri\target\windows-pin-workarea-native-qa-4a58101`。
证据目录 `C:\win\Clippy\src-tauri\target\windows-pin-workarea-contract`。
真实原生 owner/DPI/窗口事件和热插拔、多屏/Windows 10、新 SHA CI 与其它宿主仍未验。
