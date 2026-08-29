# 参考项目与集成原则

本项目曾对比 `/home/rick/desktop/Clippy/examples` 中的 Flashot 与 Translator。完整证据和版本信息见 [example-integration-analysis.md](../.trellis/tasks/08-08-clippy-integrated-refactor/research/example-integration-analysis.md)。本页保留可执行的结论，避免后续重构重复调研。

## 可借鉴设计

- 截图：以 `CaptureSession`/会话守卫保证一次动作只有一个所有者；冻结多显示器帧，前端只传递选区和编辑参数。
- Pin：由独立 `PinManager` 持有内容、窗口尺寸和生命周期；窗口先隐藏创建，首帧就绪后显示，关闭时统一释放缓存和注册表项。
- 编辑：将视口、标注、调整、撤销/重做和导出拆成小模块；导出只在明确动作时生成 PNG。
- 翻译：provider 适配器、结构化错误、超时/有限重试和 request-id 分层；多个服务的结果互不覆盖。
- 界面：紧凑结果卡、明确的复制/朗读动作、ResizeObserver 配合窗口缩放；高频剪贴板列表保持轻量，复杂功能使用 React 功能岛。

## Clippy 的组合边界

统一工作流为：

```text
clipboard/text/image -> preview -> capture/crop -> edit -> pin -> OCR -> translate -> copy
```

Rust/Tauri 继续拥有剪贴板、截图帧、窗口、Portal 会话、Pin 数据和密钥；前端通过唯一 IPC 边界消费数据。敏感内容在本地阻断，图片翻译只发送本地 OCR 文本，不上传原图。

## 不直接照搬

- Flashot 的 MIT 代码可按许可证评估复用，但跨平台私有窗口实现、滚动截图和完整标注工具不属于 Linux 首阶段范围。
- Translator 为 GPL-3.0-only，只参考行为和模块边界，不复制源码。
- 不在 Secret Service 失败时写入明文密钥。

## 已推翻的原则：非官方接口与在线 TTS

早期原则是"不把非官方 Google/DeepL 接口设为默认"。2026-08-29 由项目所有者决定推翻，改为
**与 translator 全量对齐**：

- 翻译服务实现官方 API 与非官方 web 端点双路径。配置了密钥时优先官方 API，未配置时回退到
  web 端点——也就是说未配置用户实际走的就是非官方路径。
- TTS 与 translator 一致使用在线 `dictvoice` 端点，不做本地合成。

代价已知并被接受：非官方端点没有可用性承诺、可能随时变更或限流，且朗读会把待读文本发往
第三方。实现时必须保证敏感剪贴板内容在本地阻断的规则同样覆盖这些路径。

## 后续实现顺序

优先保持会话、授权 token、Pin 生命周期和翻译请求的可测试性；再逐步迁移 preview、codec、settings 等稳定功能。真实 GNOME X11/Wayland、KDE Wayland、Secret Service 和翻译服务矩阵必须在发布前单独完成，不能以本地单元测试代替。
