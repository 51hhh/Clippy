# Renderer v2 固定字体

- 文件：`NotoSansCJKsc-Medium.otf`
- 字体：Noto Sans CJK SC Medium
- 版本：2.004（固定 tag `Sans2.004`）
- 上游：<https://github.com/notofonts/noto-cjk>
- 原始路径：`Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Medium.otf`
- SHA-256：`ca094f6b0001fb048ca39ddd797a0cdb0179e1e55c6561e111c49c3e6a61d7b7`
- 许可证：SIL Open Font License 1.1，见 `OFL-1.1.txt`

该文件是 renderer v2 的持久化格式依赖。不得在不升级 `rendererVersion`、更新金图摘要和迁移说明的
情况下替换、子集化或重新生成；系统字体只可用于普通 UI，不能参与工程 PNG 的最终合成。

## 公式 OCR 固定数学字体

- 文件：`NotoSansMath-Regular.ttf`
- 字体：Noto Sans Math Regular
- 版本：2.001
- 上游：<https://github.com/notofonts/math>
- SHA-256：`8242bd1e55368b27e32455260754cf9aa58f3ad7ea80664b66c21f1b09910d6c`
- 许可证：SIL Open Font License 1.1，见 `OFL-1.1.txt`

该文件只用于 `formula-browser-v1` 质量语料的 MathML 栅格化，使公式像素不依赖宿主系统的数学字体。
替换它必须显式重建公式语料、更新捕获记录和重新运行公式识别基准。
