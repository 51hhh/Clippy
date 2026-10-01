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

- [ ] 完整图片工具栏在最小原生窗口内上下至少留 8 px。
- [ ] 小选区及缩小后的窗口遵守高度下限；高图片仍使用按内容计算的高度。
- [ ] Windows 原生完整门禁（含录屏 QA 条件编译图）通过，记录来源 SHA。
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

## Verification

代码修复与 Windows 本机验证进行中。新 SHA 的原生 CI、其他平台编译与修复后桌面测试尚未执行。
