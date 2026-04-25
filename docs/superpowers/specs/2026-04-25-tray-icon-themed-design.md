# Tray icon themed design

- Date: 2026-04-25
- Status: Approved

## 背景

`src-tauri/icons/file.svg` 是单色描边图（仅 `stroke="#000000"`、无背景）。现状托盘只用一份预生成 PNG（透明背景 + 黑描边），在深色任务栏背景下几乎不可见。

目标：托盘图标的背景 = 当前应用主题的 `bg-primary`，描边 = 与背景对比的 `#000` 或 `#fff`，主题切换时自动重绘。

## 决策

- **源**：`src-tauri/icons/file.svg`（保留，唯一来源）。
- **rasterize**：Rust 内用 `resvg` + `tiny-skia` + `usvg` 运行时点阵，零外部依赖。
- **形状**：64×64 RGBA，圆角矩形背景（radius 14 px），SVG 居中渲染留 ~12.5% 内边距。
- **描边色判定**：基于 `bg` 的 sRGB 相对亮度（WCAG 公式），>0.5 用 `#000000`，否则 `#ffffff`。
- **主题来源**：`AppConfig.theme`（与 `themes.css` 一一对应）；启动 + `config-changed` 事件触发刷新。

## 模块

- 新模块 `src-tauri/src/tray_icon.rs`，纯函数：
  ```rust
  pub fn render_themed_tray_icon(theme: &str) -> tauri::image::Image<'static>;
  fn bg_for(theme: &str) -> [u8; 4];
  fn stroke_hex_for(bg: [u8; 4]) -> &'static str;
  ```
  无副作用，可单测。
- `lib.rs`：
  - `mod tray_icon;`
  - `build_tray` 改为 `TrayIconBuilder::icon(tray_icon::render_themed_tray_icon(&theme))`，并把 tray id 设为 `"main"`。
  - `setup` 末尾 `app.listen("config-changed", ...)`，回调里 `app.tray_by_id("main").set_icon(Some(render_themed_tray_icon(theme)))`。`update_config` 已经 emit `config-changed`，无需改 commands.rs。

## 主题色表

```rust
const THEMES: &[(&str, [u8; 4])] = &[
    ("light",            [0xfb, 0xfb, 0xfd, 0xff]),
    ("dark",             [0x16, 0x18, 0x1d, 0xff]),
    ("nord",             [0x2e, 0x34, 0x40, 0xff]),
    ("solarized-light",  [0xfd, 0xf6, 0xe3, 0xff]),
    ("rose",             [0xff, 0xf7, 0xf7, 0xff]),
    ("midnight",         [0x14, 0x13, 0x2b, 0xff]),
];
// 缺省回退 "light"
```

## 数据流

```
启动:  AppConfig.theme ──► render_themed_tray_icon ──► TrayIconBuilder::icon
                                                          │
config-changed event ──► load 新 theme ──► tray.set_icon(...)
```

## 错误处理

- SVG 解析失败：`log::error!` + 回退 `app.default_window_icon().clone()`，不 panic。
- `set_icon` / `tray_by_id` 失败：`log::warn!`，沿用旧图标。
- 主题字符串不在表里：默认 `light` 配色。
- 后台事件 listener 在锁失败 / 无 tray 时仅记录，不回环重试。

## 测试

`src-tauri/src/tray_icon.rs` 内嵌 `#[cfg(test)]`：

- `bg_for_known_theme` / `bg_for_unknown_theme_falls_back_to_light`
- `stroke_picks_dark_for_light_bg` / `stroke_picks_light_for_dark_bg`
- `render_themed_tray_icon_returns_64x64_image`（断言 width/height = 64，alpha 通道部分非透明）

不做图像快照（避免大文件入库）。

## 依赖

`src-tauri/Cargo.toml` 增加：
- `resvg = "0.43"`
- `tiny-skia = "0.11"`
- `usvg = "0.43"`（若 resvg 已 reexport，则只用 resvg）

`sharp` Node 包仅用于一次性发布图标生成，本任务不涉及。

## 不做

- 不实时探测系统任务栏背景（跨 DE 不可靠）。
- 不做 macOS template image / Windows AccentColor 适配（当前只构建 Linux）。
- 不实现自定义主题用户色 → 留给后续可拓展（接口不阻塞）。

## 影响面

- `src-tauri/Cargo.toml`：+3 依赖。
- `src-tauri/src/lib.rs`：注册 module、tray id、config-changed 监听。
- `src-tauri/src/tray_icon.rs`：新增 ~120 行 + 测试。
- 不动前端。

## 验证清单

- `cargo fmt --check`、`cargo clippy -- -D warnings`、`cargo test` 全过。
- 手动：6 套主题在浅色与深色任务栏背景各看一遍，描边/背景对比清晰。
- `cargo tauri build` 成功。
