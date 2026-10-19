# PX-LS-WAYLAND-AUTO-01 — Wayland 授权自动长截图

## Goal

在原生 Wayland 会话中，通过用户明确授权的 XDG RemoteDesktop + ScreenCast 会话驱动长截图
上下左右滚动。输入只绑定本次长截图代次和冻结选区所在显示器；授权、滚动或几何核验失败时保留
已提交画布并回到手动追加。

## Requirements

1. Wayland 只有在 `RemoteDesktop` 与 `ScreenCast` Portal 都可用时才显示授权入口。未授权状态必须
   是 `permission_required`，不能提前宣称自动滚动可用；缺少接口时保持 `unsupported`。
2. 授权由长截图控制窗显式发起。后端从调用窗口取得 xdg-foreign parent，组合请求单一 Monitor
   stream 与 Pointer 权限，并要求返回且只返回一个显示器流。不得借 `DISPLAY`、XWayland 或 Enigo
   注入原生 Wayland 窗口。
3. Portal 返回的 stream 必须与冻结选区的显示器逻辑位置和尺寸一致；多屏时缺失 position/size、
   选错显示器、未授予 Pointer、拒绝或关闭授权窗都必须失败并关闭临时 session。
4. 授权 session 只属于当前长截图代次，finish、cancel、窗口销毁或被新会话替代时关闭。授权期间
   不改变画布；并发 append、finish、cancel 与重复授权继续由现有 registry 串行化。
5. 每个自动步骤使用后端冻结的选区中心换算 stream 内绝对坐标，先发送绝对指针移动，再发送一个
   有界离散滚轮事件。方向和步数由后端固定，前端不能提交坐标、stream ID 或滚轮幅度。
6. Wayland 无法读取全局窗口句柄或物理指针位置。为避免目标切换后误滚动，每一步输入前必须重新
   捕获固定选区，并与上次已提交帧的视觉身份一致；不一致时在输入前返回
   `longshot_auto_target_lost`。动态内容可保守暂停，用户仍可手动追加。
7. 输入后仍复用现有重叠、相似度、歧义、位移和资源预算门禁。Portal 调用设置总截止时间，失败
   不更新视觉身份；只有帧成功提交后才把新帧登记为下一步基线。

## Acceptance Criteria

- [x] 能力测试覆盖 Wayland Portal 可用、接口缺失、未授权与授权后四向可用状态。
- [x] Portal 合同测试覆盖 Pointer/Monitor 请求、单 stream、显示器匹配、错误 stream、拒绝与关闭。
- [x] 状态测试证明授权绑定 exact handle，重复/并发/取消/窗口销毁不会泄漏 session 或开放旧代次。
- [x] 自动步骤测试证明坐标、方向与离散步数由后端生成，视觉身份不符时不会发送滚轮事件。
- [x] 前端测试覆盖授权按钮、授权中禁用、成功后出现四向控制、拒绝后保留手动追加与安全错误文案。
- [x] Rust check/clippy/test、前端测试、TypeScript 和完整 `./scripts/ci-local.sh` 通过。
- [ ] 同一 SHA 的 Ubuntu/Windows/macOS CI 证明非 Wayland 平台没有条件编译回退。
- [ ] GNOME、KDE 与 wlroots 真机分别记录允许、拒绝、选错显示器、Stop/Esc、目标变化、四方向、
  页面到底与输出闭环；自动化不能替代该矩阵。

## Out of Scope

- 绕过系统 Portal、后台控制浏览器 DOM 或保存跨长截图会话的永久 RemoteDesktop 权限；
- 在 Wayland 查询全局窗口列表、窗口 PID 或物理指针位置；
- 把一次 Portal 授权解释为 GNOME、KDE 与 wlroots 已完成真机验收；
- 修改现有 X11、Windows、macOS 自动滚动输入实现或放宽拼接质量门禁。
