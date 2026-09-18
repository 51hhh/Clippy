# OCR 质量基线与优化结论

审阅日期：2026-09-18  
需求标识：`PX-OCR-QUALITY-01`  
关联路线：`docs/reviews/2026-09-18-pixpin-feature-gap-roadmap.md`

## 结论

当前 PP-OCRv6 small + EdgeGNN 增强链在已有 14 张同源合成图上明显优于 Clippy 的 Tesseract
fallback，特别是双栏、旋转和中文场景；简单单行两者都能完全识别。这个结论只适用于已记录的合成
样本，不能外推为真实截图、中日混排、公式或所有设备上的总体精度。

`EdgeGNN` 不是图像边缘增强。它以文字框为节点、相邻关系为边，预测哪些行属于同一段；字符是否
识别正确主要由 det 框、原像素透视 crop、rec 模型、字典和 CTC 决定。Clippy 自研 XY-cut 负责组的
阅读顺序。历史证据还显示 EdgeGNN 会把同一段的普通相邻行拆成 singleton，导致 `result.text` 在每行
之间插入空白行；本轮增加了保守的视觉段落合并来修复这一问题。

## 同源 A/B

14 张图片由原生成脚本重新生成，逐张 SHA-256 与 2026-09-12 增强链证据完全一致。本轮修改前，
`pipeline.py`、`edge_features.py`、`layout_groups.py` 等运行文件的 SHA 也与该次增强链运行一致；随后
`pipeline.py` 只在 Edge 输出后加入视觉段落合并，A/B 使用的检测框、逐行文字、crop、rec 和 CTC
行为没有改变，完整文件 SHA 因该有意改动不再相同。Tesseract 使用当前
`/usr/bin/tesseract 5.5.0` 和产品相同的 `eng+chi_sim` 参数重新运行。比较文本按阅读顺序以 LF 连接，
指标按 Unicode code point 计算。

| 指标 | PP-OCRv6 small + EdgeGNN | Tesseract 5.5.0 |
|---|---:|---:|
| 原始 CER | **0.34%** | 32.70% |
| 去空白 CER | **0.00%** | 30.98% |
| 空白 precision | **1.000** | 0.779 |
| 空白 recall | **0.980** | 0.751 |
| 空白 F1 | **0.990** | 0.765 |
| 14 图总耗时 | 8.56 s | **3.60 s** |

增强链约慢 2.38 倍。Tesseract 的主要失分来自双栏按行交错、8°/-11° 旋转漏字，以及中文字符间
插入空格和少量汉字错误。增强链 13/14 图非空白字符完全正确；唯一混合中文样本的非空白 CER 为 0，
但模型删除了“项目记录 2026 年 9 月”“金额明细 100.00 元”“AABB 112233”中的视觉空格，导致该图
原始 CER 8.05%。因此“增强链更好”与“空格问题已经解决”不能混为一谈。

逐图输入 SHA、期望文本、两引擎输出、指标、时间和运行文件 SHA 保存在
`docs/reviews/evidence/2026-09-18-ocr-ab/ab-results.json`。

## 新 v1 语料与当前 Tesseract 基线

本轮新增 5 张可再分发合成图，使用仓库内 OFL Noto Sans CJK SC 生成，并以 SHA 固定：

- 中英日混排、连续空格、重复字母/数字、货币；
- 数值、全半角标点和常用数学符号；
- 双栏阅读顺序；
- 公式路由与 LaTeX 真值；
- 低对比小字。

当前 Tesseract 在这组 v1 smoke corpus 上的原始 CER 为 27.47%，去空白 CER 19.53%，空白 F1
0.673；它不提供结构化行框、阅读顺序或公式，因此这些能力记录为 `supported=false`，不会用一个
虚构整图框计为失败。预测和报告分别保存在同一 evidence 目录的
`tesseract-predictions.json`、`tesseract-report.json`。

当前机器已经没有增强运行时 manifest 和 Edge 模型，所以没有伪造新 v1 语料的 PP-OCR 结果。新的
`quality_collect.py enhanced` 已能在用户提供有效 manifest 后，用同一 corpus 生成结构化预测；该项
继续保持未验收。

## 本轮实现

1. `quality-corpus.schema.json` 固定图片相对路径、SHA、尺寸、许可、标签、框、原始文字、顺序与公式；
2. `quality-predictions.schema.json` 区分整段文本、行框和结构化公式能力；
3. `quality_metrics.py` 计算四边形检测 Hmean、原始/去空白 CER、精确 Unicode 空白 F1、整行匹配、
   阅读顺序 inversion、公式支持/完全匹配和性能；
4. `quality_collect.py` 用产品相同参数采集 Tesseract，或通过固定 manifest 采集增强 sidecar；
5. `quality-fixtures/v1/` 保存生成器、5 张固定 PNG 和人工真值；
6. 本地门禁加入不依赖模型/第三方 wheel 的质量合同与视觉段落测试；
7. 增强链在 EdgeGNN 分组后合并“同栏、正常行距、近似字号”的相邻片段；大段距、跨栏回跳、
   并排文字和字号突变仍保持边界。历史 singleton 证据重放后，两段文本恢复为 3+3 行，双栏恢复为
   左 3 行 + 右 3 行，已有正确分组保持不变。

## 后续优化顺序

### 1. 空格与重复字符

当前字典含 ASCII space，但 greedy CTC 只保留模型实际发出的空格类别；字典覆盖不能修复模型漏空格。
下一步先在诊断输出记录每个字符的 CTC emission step 和相邻 blank-run 长度，再判断宽 blank 是否能
稳定代表视觉空格。任何补空格规则只允许作用于拉丁/数字/符号边界，并用连续空格、中文、日文、代码、
序列号和金额分层验收；不能在汉字之间普遍插空格。重复字符必须保留 blank 分隔证据，不能用词典
纠错改写编号。

同时 A/B 英语专用识别模型：PaddleOCR 官方明确把英语模型的空格遗漏改善作为能力。先用通用模型
识别并判断行的主要 script，仅对高置信拉丁行重跑专用模型；中英日混排行保留通用模型，除非语料
证明分段重识别更好。

### 2. 方向与裁切

当前 `crop_line` 只在高度远大于宽度时固定旋转 90°，不能判断 90°/180°/270°方向，也没有官方
pipeline 的文字行方向分类。应先比较四方向候选的识别置信与空文本率，并设置总 crop/时间预算；
文档去畸变只在真实拍照语料进入范围后评估，普通截图不承担其包体和延迟。

### 3. 检测与模型档位

det 漏框、错误合框和 rec 错字必须分别统计。先在 v1 加真实 UI、小字、压缩、深浅底、斜体、日文、
表格和长图样本，再比较 small/medium det 与 rec；只替换目标阶段，不能用总 CER 掩盖检测退化。
medium 官方精度更高但包体和 CPU 成本也更高，必须同时报告 P50/P95/RSS。

### 4. 公式

普通 OCR 只能得到线性字符，无法恢复分数、上下标、根式和矩阵结构。公式区域需要独立检测/路由到
PP-FormulaNet 或 UniMERNet 一类公式模型，并输出 LaTeX/MathML；当前通用模型一律报告
`structuredFormula=false`。公式模型体积和 CPU 延迟显著，第一阶段应作为可选组件。

### 5. EdgeGNN 与阅读顺序

保留 EdgeGNN 的模型输出和视觉合并前后分组，分别评测 paragraph membership 与最终阅读顺序。
本轮视觉合并只修正常行距 singleton；阈值、颜色特征或模型替换必须在标题、列表、表格、双栏、
跨栏标题和真实 UI 上验证，不能因为文字 CER 不变就认为版面没有回退。

## 验证边界

已完成 schema/评测/采集器单元测试、固定夹具 SHA 校验、Tesseract v1 基线、14 图同源 A/B 复算和
历史几何的段落合并重放。新 v1 语料尚未重跑增强模型；没有日文专用、结构化公式、真实截图、手写、
相机照片或三平台性能结果。模型权重和 Edge 模型仍不进入仓库。

`./scripts/ci-local.sh` 于 2026-09-18 完整通过：19 项通过、0 项失败；非宿主平台交叉 lint 和
AppImage X11 可视 smoke 共 2 项按配置跳过，不能计为通过。该结果只覆盖 Linux x86_64，仍需同一
提交上的 Windows/macOS 原生 CI。
