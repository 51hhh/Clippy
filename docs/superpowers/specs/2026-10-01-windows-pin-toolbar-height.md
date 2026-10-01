# WIN-PIN-TOOLBAR-01 — 小图贴图工具栏高度修复

## Goal

修复 Windows 11 真机发现的小图 Pin 工具栏被窗口底部裁切的问题，使保存、复制与关闭按钮可访问。
本修复属于 `WIN-NATIVE-01` 的独立产品问题分支。

## Requirements

1. 原生窗口创建与缩放共用的高度下限须容纳完整图片工具栏及上下各 8 px 间隙。
2. 前端首帧或无法测量时的尺寸兜底须覆盖当前最长工具栏；正常运行仍以实际测量为准。
3. 只增加透明窗口留白，保持图片内容尺寸、原始贴图偏移与既有显示器坐标规则。
4. 回归测试须结合实际渲染的按钮、真实 CSS 和 Rust 窗口下限检查底部可访问性，避免仅复述常量。
5. 使用 Windows 本机工具链验证；自动测试不能替代修复后桌面复测。

## Acceptance Criteria

- [x] 完整图片工具栏在最小原生窗口内上下至少留 8 px（真实 CSS/原生尺寸自动合同）。
- [x] 小选区及缩小后的窗口遵守高度下限；高图片仍使用按内容计算的高度（Rust 合同）。
- [x] Windows 原生完整门禁（含录屏 QA 条件编译图）通过，记录来源 SHA。
- [ ] 修复后 Windows 11 桌面复测保存、复制与关闭入口。

## Out of Scope

Windows 10、双屏、混合 DPI、负坐标、极小显示器工作区和画布横排工具栏布局；本轮不修改录屏、
截图坐标、工作区持久化或安装包，也不合入 dev。

## Review Evidence

- 已安装 QA 包源码：`45769c958e9d4311a7d61ee53ed6707cc9a11ac4`。
- Windows 11 单屏、125% DPI，256×128 RGBA 夹具产生 308×280 逻辑像素 Pin 窗口；
  完整工具栏底部的保存、复制、关闭按钮被裁切，打开/关闭画布后仍然如此。
- 原生观察记录及截图保存于 `src-tauri/target/windows-native-qa/45769c958e9d4311a7d61ee53ed6707cc9a11ac4/operator-evidence/`
  的 `pin-native-live-observation.json`、`pin-small-image-toolbar-clipped.png`。
- 新前端回归在修复前失败：实际 CSS 占用与定位合计底部 359 px，旧窗口允许的底部为 272 px。
- 用户要求停止桌面操作，转为代码 review 与修复；桌面复测保留未完成。

## Reviewed Paths

- `pin/window.rs`：创建、缩放与目标位置共用 `outer_size`，更新下限覆盖最长工具栏。
- `pin/workspace.rs`：工作区恢复位置同样使用 `outer_size` 计算完整窗口的工作区约束。
- `react/pin/App.tsx` / `pin.css`：内容尺寸独立由 `contentWidth/contentHeight × scale` 决定，
  原生高度下限只增加透明留白。
- `PinToolbar.tsx` / `toolbarPlacement.ts` / `useToolbarDrag.ts`：保留实际测量、自动定位和拖动钳制，
  更新首帧尺寸兜底；定位函数不会自动缩小 CSS 内容，不能用改变落点替代窗口高度修复。

## Verification

修复源码 `7aa6cf65e394c3986df45894c60bf08eb408f031`，Windows 11 x64、Windows PowerShell 5.1、
Rust 1.98.1 MSVC、Node 24.21.0；完整 `scripts/ci-windows.ps1 -RecordingQa` exit 0，
23 passed / 0 failed / 1 skipped（Linux smoke）。验证后检出仍干净，stdout/stderr 哈希均核对。

| 层级 | 结果 |
|---|---|
| 本机默认 Rust | 1040 passed / 5 ignored；check、clippy、fmt 通过。 |
| 本机录屏 QA Rust | 1093 passed / 5 ignored；check、clippy 通过。与默认图重叠，不累加。 |
| 本机前端 | 75 文件 / 1292 passed；边界、IPC、静态检查、类型检查、生产构建与产物入口检查通过。 |
| 同 SHA 原生 CI | 未运行。其他平台编译不能由本机 Windows 门禁替代。 |
| 修复后桌面 QA | 未运行，用户要求停止桌面操作；当前安装包仍为旧源码 45769c9。 |

证据位于本工作区主检出的 `src-tauri/target/windows-pin-toolbar-native-qa-7aa6cf6-direct-wait/`，
`RESULT.json` 绑定源码、工具链、日志哈希、exit 0 与完整检查汇总。
首次运行输出相同通过汇总，但外层脚本的进程树等待未返回，退出码未知；原日志保留在
`windows-pin-toolbar-native-qa-7aa6cf6/`，不计为一次成功运行。
仅调整本地证据收集脚本为直接等待 gate 进程退出后重跑；仓库门禁内容保持既有实现。

本记录作为后继文档提交更新；门禁证据绑定 7aa6cf6，不冒称后继文档 SHA 已运行测试。
Windows 10、双屏/混合 DPI、负坐标、极小工作区与修复后桌面行为保留未验证；没有安装新包、合入 dev 或发布。
