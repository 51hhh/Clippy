# OCR 表格同基线排序证据

需求标识：`PX-OCR-TABLE-01`

输入是 `quality-fixtures/ui-stress-v1/table-values.png`，模型是固定官方 PP-OCRv6 medium det 与
small rec。修改前的原始预测和双层报告分别位于：

- `../2026-09-18-ocr-model-tiers/ui-stress-v1-medium-det-predictions.json`
- `../2026-09-18-ocr-model-tiers/ui-stress-v1-medium-det-table-report.json`

修改后使用相同图片、模型、阈值、Edge 权重和评测器重跑，文件为
`medium-det-predictions.json` 与 `medium-det-table-report.json`。`environment.json` 固定语料、运行源码、
官方模型和基础 manifest 的 SHA，不记录本机路径。

| 指标 | 修改前 | 修改后 |
|---|---:|---:|
| row detection Hmean | 1.000 | 1.000 |
| cell detection Hmean | 0.632 | 0.632 |
| raw reading-order inversion | 1 / 21 | 0 / 21 |
| raw non-whitespace CER | 17.39% | 0% |
| geometry reconstruction CER | 0% | 0% |

修改只改变同一基线内的节点顺序。检测框、识别字符和 cell/row 粒度均保持一致。
