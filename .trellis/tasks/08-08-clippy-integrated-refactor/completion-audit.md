# 综合重构完成度审计

审计日期：2026-08-11

状态定义：

- **自动化通过**：当前代码和可重复测试直接覆盖验收点。
- **部分通过**：实现与自动化证据存在，但真实桌面、外部服务或视觉结果尚未确认。
- **待人工**：必须由真实环境操作，不能从单元测试或 Xvfb 间接推断。

## Acceptance Criteria

| # | 验收项 | 当前状态 | 权威证据 / 剩余缺口 |
|---|---|---|---|
| 1 | X11 恢复原窗口并自动粘贴，无授权弹窗 | 待人工 | `paste/x11.rs` 实现 `_NET_ACTIVE_WINDOW` 恢复、500ms 确认及 Ctrl+V；仍需 GNOME X11 对原窗口、真实按键和无 Portal 弹窗验收。 |
| 2 | Wayland 首次授权、会话复用、重启 token 恢复、失效仅提示一次 | 待人工 | `paste/portal.rs` 使用 `persist_mode=2`、滚动 token、单次隐式尝试和显式重试；状态机/token 权限测试通过，仍需 GNOME Wayland Portal 首授、重启和撤权。 |
| 3 | 自动粘贴不可用时仍复制且不注入按键 | 自动化通过 | `PasteBackend::CopyOnly` 与 `select_clip` 错误回退均返回 copied-only；后端选择测试覆盖 X11/Wayland/TTY。 |
| 4 | X11 主页面 deb/AppImage 非空白、非黑屏、不越界、无双标题栏 | 待人工 | 当前 HEAD 的 deb/AppImage 已构建并检查；release Xvfb 启动无早退，但视觉、缩放和多显示器必须在 GNOME X11 实装确认。 |
| 5 | 冻结覆盖层、区域/窗口选择、移动和八向缩放 | 部分通过 | CaptureSession、多显示器帧、窗口候选和 React geometry 已实现；Rust 混合缩放及前端八向 geometry 测试通过，仍需 GNOME/KDE 视觉与输入验收。 |
| 6 | 截图直接 Copy/Save/Pin/Edit | 自动化通过 | `CaptureAction` IPC 和 Overlay 四动作已接通，typed IPC contract 与入口构建测试通过。 |
| 7 | 编辑器基础标注、模糊、撤销/重做、调整和一致输出 | 自动化通过 | React 对象模型、画笔/矩形/箭头/文字/模糊、历史栈和图像调整均已实现；document model、调整和 rAF 交互测试通过。 |
| 8 | 文本/图片/截图统一 Pin，控件、缩放、透明度、锁定、拖动、清理可靠 | 部分通过 | 统一 `PinManager` 和 React Pin 已实现，尺寸、乱序响应和生命周期逻辑有测试；始终置顶、拖动和视觉仍需 GNOME/KDE 实机。 |
| 9 | Pin Copy 无自动粘贴，关闭后无缓存/管理残留 | 自动化通过 | `copy_pin` 只调用 clipboard 写入，不调用 `select_clip`；关闭路径移除 manager entry 并关闭窗口，截图 Pin 数据由 entry 生命周期释放。 |
| 10 | 文本/OCR/截图可显式翻译，敏感条目不自动发送 | 自动化通过 | translation content selector 在 Rust 拒绝敏感条目；截图只发送本地 OCR 文本且 IPC 无图片 payload；前端仅点击后请求。 |
| 11 | 翻译超时、重试、request-id 和安全密钥存储 | 部分通过 | 15s 超时、一次可重试错误重试、陈旧结果保护和 Secret Service-only 实现/测试通过；真实 Secret Service 保存/读取/删除待验收。 |
| 12 | 原剪贴板、搜索、收藏、预览、OCR、设置、托盘、更新、快捷键无回归 | 部分通过 | 363 项前端测试、79 项沙箱内 Rust 测试、五入口构建和既有 Xvfb smoke 通过；真实托盘、更新器和桌面快捷键仍需实机。 |
| 13 | Rust/前端/quick/full CI 全部通过 | 部分通过 | fmt/check/clippy、79 项非回环 Rust、363 项前端、typecheck/build 和 audit 通过；2 项 localhost mock 在已授权环境曾通过，但当前沙箱禁止 bind；Xvfb 当前沙箱跳过。 |
| 14 | X11、Wayland 和 Portal 人工矩阵有结果 | 待人工 | `qa-matrix.md` 已列出完整矩阵，但真实 GNOME X11、GNOME Wayland、KDE Wayland 结果仍为空。 |

## 发布产物

- deb SHA-256：`a3da37ab7c30c97b88e6fab0e17c1904fbd636cb0da8d15e9747050882850053`
- AppImage SHA-256：`2d5234dd28c16b8169319d59190ab8c9db8f48b3bb499eb5729fcaf84bc9082d`
- AppImage SquashFS、主二进制、GTK/WebKit 动态库和相对 `.DirIcon` 已检查。

## 完成阻断项

Trellis task 只有在下列证据写回 `qa-matrix.md` 后才能标记 `completed`：

1. GNOME X11 的窗口恢复、Ctrl+V、主页面视觉和 deb/AppImage 实装。
2. GNOME Wayland 的首次授权、同进程复用、重启 token 恢复及撤权。
3. KDE Wayland 的 Portal、截图覆盖层和 Pin 置顶。
4. 真实 Secret Service 及至少一个 LibreTranslate-compatible/OpenAI-compatible 测试服务。
