# OCR 质量夹具 v1

这组图片由 `generate.py` 使用仓库内的 Noto Sans CJK SC 字体生成，只用于验证评测合同和建立最小
引擎基线。文本为 Clippy 自有 CC0 测试内容，字体遵循仓库已有 OFL-1.1；每张 PNG 的 SHA-256、尺寸、
行框、原始空白、阅读顺序和公式真值保存在 `corpus.json`。

覆盖范围：中英日混排、连续空格、重复字符、数字/货币/标点/数学符号、双栏阅读顺序、结构化公式
路由和低对比小字。这是合成 smoke corpus，不能代表真实截图总体精度，也不能单独决定默认模型。

重新生成会更新 PNG 和 SHA，因此只应在有意修改夹具时执行：

```sh
python3 src-tauri/ocr-sidecar/quality-fixtures/v1/generate.py
```
