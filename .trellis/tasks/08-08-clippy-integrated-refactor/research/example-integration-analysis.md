# 示例项目更新与可借鉴设计

## 版本

- Flashot: `23f16b5`, tag `v0.7.1`, MIT。
- Translator: `a8ac6cc`, tag `v0.3.2`, GPL-3.0-only。

## Flashot

可直接参考或按 MIT 条款复用：

- CaptureSession + SessionGuard 生命周期与单会话约束。
- 每显示器冻结覆盖层、智能窗口命中、选区移动和缩放。
- Rust 冻结帧 + 前端轻量参数的输出管线。
- PinManager、缓存文件、隐藏创建、关闭原生阴影、外部控制栏和资源清理。
- Pin 拖动阈值、真实尺寸缩放、hover/focus 控件、编辑和图像调整。
- `frame-source.ts` 对 object URL 的创建、取消和释放。
- v0.7.1 新增整屏画板复用现有 Overlay/Annotation/SessionGuard，证明核心组件可按模式组合。
- 大量纯逻辑和回归测试；当前约 51 个前端测试文件、249 个 Rust test 属性。

不直接照搬：

- 全部 13 种标注工具和滚动截图不是首阶段完成核心融合的必要条件。
- macOS/Windows 私有窗口处理与 Clippy Linux-only 范围无关。

## Translator

仅参考行为和架构，必须独立实现：

- TranslationService 适配器、结构化错误、超时和多服务独立结果。
- request-id 防止陈旧全量/单服务响应覆盖新状态。
- 主窗口定位到鼠标/当前显示器/记忆位置并限制在 work area。
- 非 macOS 运行时关闭原生装饰以避免自绘标题栏重复。
- ResizeObserver + rAF + scaleFactor 的自适应窗口高度。
- v0.3.2 的紧凑结果卡、词典结果分组和每结果 TTS/Copy 工具栏。

不能照搬：

- GPL 源码。
- Google/DeepL 非官方 Web 接口作为稳定默认服务。
- keyring 失败后写明文 `secrets.json` 的 fallback。
- 缺少前端测试的现状。

## 对 Clippy 的组合价值

Clippy 已有剪贴板历史、敏感内容检测、本地 OCR、截图捕获和临时 Pin。目标不是嵌入两个独立应用，而是组合成一条统一工作流：

`clipboard/text/image -> preview -> capture/crop -> edit -> pin -> OCR -> translate -> copy`

截图和 Pin 复用 Flashot 的会话/窗口/图像模型；翻译复用 Translator 的领域边界和请求模型；Clippy 保留自己的存储、敏感检测和轻量弹出式产品形态。

