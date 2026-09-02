# Renderer v2 跨平台合成实施方案

## 要解决的真实缺陷

工程 v3 已把原图、操作文档和 IDAT 合成像素绑定在一起，也会在未修改时复用 IDAT。但一旦用户继续
编辑，当前 `rendererVersion=1` 仍由宿主 WebView Canvas 生成最终 PNG：Linux 是 WebKitGTK，Windows
是 WebView2，macOS 是 WKWebView。系统字体、CSS filter、路径抗锯齿和图像采样都可能不同，因此同一
原图与同一操作文档不能证明会得到同一合成像素。

## v2 保证

`rendererVersion=2` 的稳定输入为：解码后的 canonical 原图 RGBA、原图宽高、规范化 annotations、
规范化 adjustments 和固定渲染器版本。Copy、扁平保存和可编辑保存必须调用同一个 Rust CPU 渲染入口；
不再接受 WebView 上传的 PNG 作为权威结果。

保证目标是相同输入得到相同宽高与 RGBA。PNG chunk 切分或压缩字节不是像素合同；工程 v3 继续用
`preview.rgbaSha256` 校验最终 RGBA。原生 CI 应对同一 fixture 比较 RGBA sha256，而不是只检查“能构建”。

交互期间仍可用 Canvas 做低延迟预览，但它只是预览。保存/复制结果以及再次打开时看到的 IDAT，统一来自
后端渲染器。若预览与最终像素的视觉差异超过允许阈值，应继续让前端复用同一固定字体和 v2 参数，而不能
退回让 WebView 决定文件内容。

## 数据版本与迁移

- PNG `formatVersion` 保持 3；容器结构不因更换渲染器再升版。
- 新建文档使用 `rendererVersion=2`。
- v1 工程保持可读；未修改时继续逐字节复用已验证 IDAT。
- v1 工程发生首次真实编辑后，以同一 annotations/adjustments 交给 v2 重绘并保存为 v2。升级只发生在
  用户修改并触发 Copy/Save 时，不能在单纯打开时悄悄改变像素。
- 未来渲染器版本仍按普通 PNG 安全降级，不能猜测未知语义。

## 单一渲染管线

1. 后端从 `PinEntry` 取得 canonical 原图；前端不回传原图或补偿预览。
2. 复用工程信任边界校验尺寸、schema、坐标、颜色、点数和效果参数。
3. 以固定点整数顺序执行 grayscale → brightness → contrast → saturation；alpha 不变。
4. 效果层始终先于矢量层，并按文档顺序处理：
   - blur：固定半径的整数可复现卷积/盒模糊，不调用平台图形 API；
   - mosaic：固定缩小采样与最近邻放大；
   - spotlight：偶奇遮罩，只压暗选区外像素；
   - magnifier：固定采样、椭圆裁剪和白色描边。
5. pen、marker、rect、highlight、ellipse、line、arrow、measure、text 使用锁定版本的纯 Rust 软件
   光栅器；线帽、连接、透明度和混合顺序写成版本语义。
6. text 与 measure 标签只使用仓库内固定版本的 Noto Sans SC 字体，不查询系统字体。字体与 SIL OFL
   许可证一起分发，版本和 sha256 记录在仓库。
7. 最后应用圆角 alpha mask，再由同一 PNG 编码器输出 RGBA8。

底图/效果的像素处理优先直接在 Rust RGBA 缓冲区完成；矢量与文字可由锁定的 `resvg`/`tiny-skia`
软件路径完成。不得通过 SVG 外部 URL、系统字体数据库、GPU、WebView 或 OS 图形 API读取不稳定资源。

## IPC 调整

- `copy_pin_canvas(label, project)`：后端渲染并写剪贴板。
- `save_pin_canvas`：已编辑文档只提交 `project`；后端渲染一次，同时用于剪贴板、扁平文件或工程 IDAT。
- 未修改导入工程仍以 `project=null` 走已有 IDAT 复用路径。
- v2 路径拒绝同时上传 `pngBase64`，避免两个互相矛盾的“最终图”来源。
- 兼容期保留 v1 的旧参数解析，但前端发布版本不再产生新的 v1 文档。

## 安全与性能预算

- 沿用 64 MiB 原图、64 MiB 合成图、160 MiB 容器、最大尺寸与总像素限制。
- 渲染前增加效果数量和累计处理像素预算，防止恶意工程用一万个全屏 blur 放大 CPU 成本。
- 所有 SVG/XML 文本、若采用中间 SVG，必须由结构化值生成并转义；不允许文档提供 URL、font-family、
  filter id 或任意 markup。
- 同一次渲染只解码一次原图；相同 blur 半径允许复用结果，但缓存必须受像素预算约束。
- 渲染放入 blocking worker，不能占用 GTK/WebView 事件线程。

## 验证门禁

- 每一种效果、矢量工具、文字、测量标签、调整和圆角均有 RGBA 金图或关键像素断言。
- 组合 fixture 覆盖图层顺序、半透明混合、边缘裁剪和中文/ASCII 文本。
- 同一 fixture 连续渲染两次的 RGBA sha256 必须相同。
- Linux、Windows x86_64、macOS x86_64/arm64 原生 CI 使用同一 fixture 和同一预期摘要。
- v1 未修改复用、v1 首次编辑升级、v2 工程往返、v3 IDAT 绑定、损坏/移植降级均有回归测试。
- 4K 组合文档记录 release 模式耗时与峰值内存；超过交互可接受阈值时优化算法，不退回 WebView 导出。

## 小步提交顺序

1. `docs(canvas): 制定 renderer v2 实施方案`
2. `chore(canvas): 固定跨平台渲染字体`
3. `feat(canvas): 实现确定性底图与效果渲染`
4. `feat(canvas): 实现确定性矢量与文字渲染`
5. `refactor(canvas): 统一复制与保存渲染入口`
6. `test(canvas): 锁定跨平台合成像素`
7. `docs(canvas): 更新 renderer v2 验证结论`

每一步提交前运行对应窄测试；接线完成后运行 `./scripts/ci-local.sh`。原生平台摘要只有在对应 runner
实际产出证据后才能标记通过。
