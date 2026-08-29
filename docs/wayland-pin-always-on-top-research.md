# Wayland 贴图窗口置顶能力调研

调研日期：2026-08-29
对应 TODO：`docs/reference-todo.md` P0
参考实现：`example/flashot`（`23f16b5` / v0.7.1 / MIT）`src-tauri/src/overlay_window.rs`

## 结论（先行）

**不为 Pin 窗口引入 gtk-layer-shell，保持现状。** 现状是 `pin/window.rs` 建窗时
`always_on_top(true)`，`pin_window.rs::configure_pin_window` 与前端 `setAlwaysOnTop` 作为兜底。
理由见下方「为什么 Pin 不适用」——代价是重写整套位置模型，收益只在部分合成器上成立。

## Wayland 的实际约束

`always_on_top` 在 Wayland 上是**不可靠的**：`xdg-shell` 协议根本没有"置顶"请求，
窗口层级完全由合成器决定，客户端无法要求。Tauri/tao 的 `set_always_on_top` 在 Wayland
下等于空操作（不报错，也不生效）。所以 Wayland 上想真正置顶只有两条路：

1. **`wlr-layer-shell`（通过 `gtk-layer-shell`）**：把窗口提到 `OVERLAY` 层。
   仅 wlroots 系（sway、Hyprland、river 等）和 KDE/KWin 支持；
   **GNOME/Mutter 至今不实现该协议**，而 GNOME 是 Clippy 的主要目标环境。
2. **XWayland 回退**：以 X11 客户端运行，`_NET_WM_STATE_ABOVE` 生效，但会失去
   Wayland 原生的缩放与输入行为，属于降级而非增强。

## flashot 是怎么做的

flashot 只对**截图覆盖层**用 layer-shell，且刻意做成可选运行时增强：

- `libloading::Library::new("libgtk-layer-shell.so.0")`（回退 `.so`）运行时 dlopen，
  `once_cell::OnceCell` 缓存；加载失败或 `is_supported()` 为假就走回退路径，
  **绝不作为编译期或打包期硬依赖**（它甚至有单测断言 deb 不得依赖 `libgtk-layer-shell0`）。
- 三分支：Wayland + layer-shell 可用 → `set_layer(OVERLAY)` + 四边 anchor + margin 0 +
  `KEYBOARD_MODE_EXCLUSIVE` + `set_monitor()`；Wayland 但无 layer-shell →
  `fullscreen_on_monitor` 回退；X11 → `set_type_hint(Splashscreen)` + `set_keep_above(true)` + `stick()`。
- **flashot 自己的 Pin 窗口没有用 layer-shell**，走的也是 `always_on_top(true)`。

这个 dlopen + 三分支 + 单测锁死可选性的模式值得记下来，但适用对象是覆盖层，不是 Pin。

## 为什么 Pin 不适用

layer-shell 表面的位置由 **anchor + margin** 表达，合成器拥有摆放权，客户端拿不到也设不了
绝对坐标。Clippy 的 Pin 恰好依赖绝对坐标：

- `pin/model.rs::PinPosition { x: i32, y: i32 }` 记录每个 Pin 的位置，
  `PinEntry.position` 参与生命周期与 `PinPayload` 回传前端；
- `pin/window.rs::position_new_pin_window` / `resize_pin_window` 用
  `PhysicalPosition`、`current_monitor()` 做定位与越界收敛；
- 用户拖动 Pin 是核心交互。

改用 layer-shell 意味着：拖动要改写成"顶左 anchor + 实时 margin 运算"，
`position` 的读写要全部换成 margin ↔ 逻辑坐标互转并自行处理多显示器与缩放，
`set_position`/`outer_position` 这类 Tauri API 在该路径下全部失效。
换来的是**在 GNOME 上依然无效**的置顶——投入产出明显不成立。

## 若将来仍要做，前置条件

1. 目标环境明确包含 KDE Wayland 或 wlroots 系，且 GNOME 用户接受不置顶。
2. 照 flashot 的模式 dlopen，加单测断言 deb/AppImage 不产生
   `libgtk-layer-shell0` 依赖。
3. 位置模型抽象成 trait（绝对坐标后端 / anchor-margin 后端），
   `PinManager` 不感知具体实现，避免两套坐标逻辑散落在命令层。
4. 必须补真实合成器手动验证（KWin、sway 至少各一），单测无法覆盖层级行为。

## 当前处置

- X11 / XWayland：`always_on_top` 生效，保持现状。
- GNOME Wayland：置顶不生效属于**已知平台限制**，不是缺陷；
  Pin 仍可用（可见、可拖、可复制），只是会被其他窗口盖住。
- 建议在设置页或文档中如实说明该限制，不要让用户以为是 bug。
