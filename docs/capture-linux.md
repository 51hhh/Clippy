# Linux 截图：窗口几何、坐标空间与覆盖层摆放

这份文档记录截图链路上三件反复踩坑、且**光看代码看不出来**的事：覆盖层窗口为什么不能自己摆位置、
每个窗口的大小到底能不能拿到、以及三套坐标空间怎么换算。改动 `src-tauri/src/capture/`、
`src-tauri/src/screenshot/` 或 `src/react/capture-overlay/` 之前先读这里。

## 1. 覆盖层窗口必须由合成器摆放

**不要用 Tauri 的 `position()` / `set_position()` / `set_size()` 来摆覆盖层。**
Wayland 协议里客户端无权决定自己窗口的位置（`xdg_surface` 只描述内容，摆放是合成器的事），
GNOME 会静默忽略这些调用。表现就是"截屏是黑的"：冻结帧其实是正常的（Rust 侧实测平均亮度 43.7，
全黑像素 0%），但覆盖层窗口落在错误的显示器上、或者尺寸不对，用户看到的是一块没有画面的窗口。

正确做法在 `capture/overlay_windows.rs`：拿到底层 GTK 窗口后设置
Splashscreen 类型提示、去装饰、不进任务栏、置顶、`stick`，然后
`gtk_window.fullscreen_on_monitor(&screen, index)` —— 由**合成器**把窗口铺满指定显示器。
显示器编号用 `OverlayRect` 与 GDK 几何求**最大重叠面积**来选（`best_monitor_index`），
不按索引猜：显示器顺序在 xcap、GDK 和 RandR 之间并不一致。

没走 gtk-layer-shell 是有意的：GNOME 不实现 `wlr-layer-shell`。在 wlroots 系合成器
（sway/Hyprland）上 layer-shell 能给出更可靠的"覆盖整个输出、独占键盘"的语义，
那是未来的升级路径，但它换不来 GNOME 上的任何好处。

非 Linux 平台仍走 `set_position` / `set_size` 兜底。

## 2. 能不能拿到每个窗口的大小

能，但**只在 X11 协议可达时**。调研结论按平台分：

| 环境 | 可用接口 | 能拿到几何吗 |
|---|---|---|
| X11 | `XQueryTree` / `XGetWindowAttributes` / `_NET_CLIENT_LIST_STACKING`（xcb），即 `xcap::Window::all()` 走的路 | 能，含位置与大小 |
| GNOME Wayland | 同上，但 X server 是 XWayland | 只能拿到 **XWayland 客户端**；原生 Wayland 应用列不出来 |
| GNOME Wayland（Shell 接口） | `org.gnome.Shell.Eval` 已被禁用；`org.gnome.Shell.Screenshot` 对普通应用返回 `AccessDenied` | 不能 |
| wlroots（sway/Hyprland） | `wlr-foreign-toplevel-management` | 只有标题 / app_id / 状态，**协议不含几何**；GNOME 也不实现它 |
| xdg-desktop-portal | ScreenCast / Screenshot | 只给画面，不给窗口列表 |

也就是说：Wayland 的安全模型里没有"列出别人的窗口并读它的矩形"这种接口，这不是缺实现，
是刻意不给。所以**窗口速选是能则用的增强，不是必需功能**：`probe_windows` 枚举失败或拿到空列表时
后端记 `log::info`，覆盖层显示 "Window picking unavailable in this session — drag to select an area"
（i18n key `capture.windowPickingUnavailable`），拖拽框选与"点一下取整屏"完全不受影响。

两个必须做的修正：

- **客户端矩形 ≠ 用户看到的窗口。** GTK 的 CSD 把阴影算在窗口里，`_GTK_FRAME_EXTENTS`
  （回退 `_NET_FRAME_EXTENTS`）给出四边要减掉的边距，不减就会得到一个比窗口大一圈、
  边缘全是透明阴影的候选区。见 `window_probe.rs::trim_frame_extents`。
- **小窗口不做候选。** 小于 `MIN_CANDIDATE_SIZE = 20` 逻辑像素的矩形点不准，也挡不住误命中，直接丢掉。

## 3. 三套坐标空间

| 空间 | 谁在用 | 单位 |
|---|---|---|
| 逻辑像素 | 覆盖层 DOM、选区、`WindowCandidate` | 桌面逻辑尺寸（本机 1920×1200） |
| 冻结帧物理像素 | `FrozenFrame.rgba`、标注、`exportPngBase64` 的裁剪矩形 | 帧的真实宽高（本机 2560×1600） |
| X screen 像素 | `xcap::Window::x()/y()/width()/height()` | XWayland 的 X 屏（本机 3840×2400） |

无缩放的 X11 会话里三者恰好相等，所以这段代码长期看着是对的；一有缩放就错得离谱——
本机 scale 1.3333 时一个普通 QQ 窗口被报成 2598 像素宽，比整个逻辑桌面还宽。

换算规则，改代码时不要另立一套：

- **显示器逻辑尺寸 = round(帧像素 / scale_factor)**（`screenshot/backends.rs::normalize_monitor_geometry`）。
  xcap 0.9.6 的 `Monitor::width()` 返回的是 `RandR 像素 ÷ scale_factor`，在本机上给出 2880×1800
  这种既不是逻辑尺寸也不是物理尺寸的数；不归一化的后果是覆盖层里的图"没有正确缩放"。
  原点按同一比例缩放，容差 1 像素。
- **窗口矩形先按 `X screen 像素 / 逻辑像素` 折算**（`window_probe.rs::x11_pixel_ratio`，
  比值钳在 1.0..=4.0），再和帧的逻辑边界求交。
- **选区在逻辑空间，标注在帧像素空间。** 前端 `scale = logicalWidth / pixelWidth`，
  `useCanvasInteractions.pointFromEvent` 把客户端坐标除以 `scale` 得到帧像素坐标；
  导出时用 `geometry.ts::toPixelRect(selection, 1/scale, 1/scaleY, frame)` 把选区换算回帧像素并钳进帧内。

## 4. 快速选区（hover → click）的交互约定

1. 后端把候选窗口按 `xcap::Window::all()` 的顺序下发，**索引 0 是最上层**；
   前端 `windowAt` 取第一个命中的候选，因此重叠时选到的是最上面那个窗口。
2. 鼠标移动时 `hoverCandidate` 只在选区**外面**给高亮预览：选区内部要让位给移动/缩放手势，
   否则随手框过一次之后窗口速选就再也用不上了。
3. 按下到松开的位移小于 `CLICK_SLOP = 4` 逻辑像素算"点击"：
   停在某个窗口上就取那个窗口，停在空地上就取**整个显示器**（参考项目 flashot 的手感）。
   位移超过 slop 就是拖拽，原样落地并钳进屏幕边界；面积不足 2×2 作废。
4. 点击或拖拽**都不结束截图**：工具条贴到选区旁边，选区仍可拖动与缩放，
   点对钩才把"裁剪 + 标注"后的 PNG 提交。铺满全屏的选区靠 `coversBounds` 让内部拖拽回到重新框选。
5. 右键丢掉选区回到 idle，Esc 取消整个截图。

## 5. 真实桌面上仍需人工验收的部分

单元测试覆盖到几何换算、状态机和提交合同，但覆盖不到合成器行为。以下必须在真机上看：
覆盖层是否铺满**当前**显示器（多屏、混合缩放）、原生 Wayland 应用是否如预期地不出现在速选列表里、
Portal 首次确认与撤权。`src-tauri/src/screenshot/backends.rs` 里留了两个 `#[ignore]` 的诊断测试
（`backend_diagnostics`、`window_probe_diagnostics`），用
`cargo test -- --ignored --nocapture` 跑，会打印每个后端的尺寸、平均亮度、全黑像素比例和窗口矩形——
"截图是黑的"这类问题只能靠它定位，不要删。
