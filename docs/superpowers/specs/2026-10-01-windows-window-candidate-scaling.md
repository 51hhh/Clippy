# WIN-WINDOW-SCALE-01 — Windows 跨屏窗口速选逐帧坐标转换

## Goal

修复 Windows 窗口速选在混合 DPI 跨屏时，用单个显示器缩放转换全部候选的问题。
对应 WIN-NATIVE-01 / W04 的 W29，基于 2bb755a；仅代码/原生离线验证，桌面保持停止。

## Requirements

1. Windows 的 xcap/DWM 窗口矩形保留物理桌面坐标，按每块冻结帧的 scale_x/scale_y 独立
   转换到该覆盖层坐标并求交，不使用窗口所属/主导显示器的缩放处理其它屏幕。
2. 候选保留 f64 分数逻辑坐标，先转换/裁剪再应用既有 20 CSS px 最小边阈值；结果必须在
   该帧逻辑边界内。拒绝非有限/非正缩放与空帧尺寸，不构造猜测候选。
3. 枚举 Z 顺序、窗口标题和既有本进程/最小化过滤保持。Linux Shell/X11 与 macOS 的
   转换、排序不变；将 X11 全局比例诊断限定到适用平台，不再把 Windows 当作 X11。
4. 生产共用入口的确定性回归覆盖 100%/150% 两种左右排布、负坐标、125%/150% 上下排布、
   相同缩放/单屏、分数边界/最小阈值、无效元数据与 Z 顺序。测试不枚举窗口、截图或操控桌面。
5. 原算法用实际 to_logical/append_window_intersections 的抽取调用协议给出红基线，
   Windows MSVC 默认/录屏 QA 两个图执行回归，完整门禁绑定干净源码 SHA。
   文档/CHANGELOG 同 ID，真实多屏/负坐标和新 SHA CI 保留未验证。

## Acceptance Criteria

- [x] 原单一窗口比例在确定性混合 DPI 场景产生丢失/错位的候选。
- [x] 八项生产共用投影回归通过；逐帧边界、阈值与顺序通过，其它平台路径按源码保持。
- [x] 干净源码的完整 Windows 默认/QA 编译、严格 lint、测试与供应链通过。
- [x] 证据分层同步，桌面、Windows 10/多屏、其它宿主和当前 SHA CI 保留未验。

## Out of Scope

更改冻结帧/覆盖层原点模型、窗口建窗定位、Pin/guide/长截图/WGC 几何、DPI 虚拟化模式、
原生窗口枚举时的几何竞争、重启桌面测试、安装包、Linux/WSL、合入 dev 或发布。
这项候选计算修复不能代表 W04 全部通过。

## Review evidence

xcap Windows get_window_bounds 调用 DWMWA_EXTENDED_FRAME_BOUNDS，物理窗口矩形由
window.x/y/width/height 返回。冻结帧则经 normalize_monitor_geometry 逐屏归一化原点/尺寸，
每帧单独保存物理/逻辑尺寸比。原 candidates_from_x11 用 window.current_monitor 的单一
scale_factor 转换整块矩形，然后与所有帧求交；跨屏缩放不同，不存在能满足两帧的单一除数。

例如左屏物理 1920 宽、150%（逻辑宽 1280），右屏起点 1920、100%；窗口物理 x=1800、
宽 400，以右屏 100% 转换会完全丢掉左屏的 120 物理像素。正确左候选为逻辑 x=1200、宽 80，
右候选为局部 x=0、宽 280。该例证明代码计算缺陷，未观察真实双屏桌面。

[Microsoft GetWindowRect](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowrect)
说明 DWM 可见扩展边界不进行 DPI 调整；原生 API 与 vendor 路径已静态核对。
逐帧计算沿用现有冻结帧原点事实；原点归一化的舍入或建窗定位不由本合同证明。

## Validation

实际应用 Cargo 图的 MSVC 红基线：2 passed / 6 failed / 0 ignored，退出码 101；调用实际旧
to_logical/append_window_intersections，用字面帧的物理交集面积抽取主导比例，不调用系统 API。
初版及补齐空像素帧断言后的最终绿回归均为 8 passed / 0 failed / 0 ignored，退出码 0，
1063 项过滤；最终日志 native-green-final.log。
红/绿原始日志、源码快照与哈希位于 windows-window-candidate-red/RESULT.json。
开发中曾因 X11Probe 不可变借用编译失败，已修正并保留日志；不计为测试运行。
一次复跑启动命令缺 PATH 未执行 Cargo，终端返回码不能作为通过证据；最终使用已安装工具的
绝对路径与独立日志检查。

干净被测源码 78bd83fc3547459c523a150faa8a999248c16267 的完整 Windows 默认/QA 门禁 exit 0：
30 passed / 0 failed / 1 skipped（Linux smoke）；默认 Rust 1066 / 5 ignored、QA Rust 1123 /
5 ignored（重叠不累加），两个图分别执行全部八项新回归，已包含在各自 Rust 总数。
前端 75 文件 / 1292 passed，Python 33 + 3；独立 vendor 十八项、剪贴板二十四项通过，
不计入应用 Rust 总数。源码/门禁/CI 后继变化仅四份 Markdown；日志哈希与干净检出已核对。
最终证据 windows-window-candidate-native-qa-78bd83f-exitcode-checked/RESULT.json。

首次完整门禁日志也是 30 passed / 0 failed，但包装器 Start-Process 没有取得子进程退出码；
RESULT exitCode 为 null、status failed，不能计整轮通过。windows-window-candidate-native-qa-78bd83f
原日志与记录保留。新的未纳入 Git 的包装器直接持有 Process，异步排空两输出流，WaitForExit
后要求 HasExited 和非空退出码；无界面退出 0/17、双流探针通过，再对同一 SHA 完整重跑确认 0。
包装器原始字节/探针与前次日志哈希在 windows-window-candidate-exit-code/RESULT.json。

Linux/macOS 转换和排序按源码核对保持，未执行其它宿主；Windows Native 既有 cargo test
自动包含八项模块，无 CI 配置改动。真实桌面、Windows 10/双屏/负坐标、新 SHA CI 和其它
W04 几何仍未验，上一轮门禁和当前离线合同不能替代。未安装新包、合入 dev 或发布。
