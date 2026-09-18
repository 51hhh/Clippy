# Pin renderer v2 金图

`combined-tools.png` 是 `pin::render_v2` 权威软件渲染器的 64×48 RGBA 审阅基线。测试内用固定
源图、13 种像素工具、图像调整和透明圆角生成实际输出，当前登记容差为：

- 单通道差：`0`；
- 允许超差像素：`0`。

失败时会在 `src-tauri/target/test-artifacts/pin-render-v2/` 写入
`*-expected.png`、`*-actual.png` 和 `*-diff.png`。也可用 `CLIPPY_TEST_ARTIFACT_DIR` 指定证据目录。
差异图用红色标出超差像素，尺寸缺失区域用洋红色标出。

只有确认渲染变化符合需求、人工查看三张证据并同步更新 CHANGELOG 后，才能替换金图或调整容差；
不能以让测试通过为理由自动接受新输出。
