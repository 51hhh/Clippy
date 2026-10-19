# PX-OCR-FORMULA-01 — 独立公式识别可行性门禁

状态：已完成（产品接入 no-go）
关联路线：`PX-OCR-QUALITY-01` / `PX-ROADMAP-2026-09`

## Goal

验证 Clippy 是否能在不把普通 OCR 线性文本冒充结构化结果的前提下，把明确的公式区域交给独立模型，
输出可复制的 LaTeX。结论必须同时覆盖识别质量、公式区域来源、模型供应链、包体、CPU 延迟和峰值内存，
并给出进入产品、保持显式可选或暂缓的明确门禁。

## Requirements

1. 公式识别使用 PaddleOCR 官方 PP-FormulaNet 系列的固定版本与 SHA；第一候选为体积和 CPU 延迟较低的
   `PP-FormulaNet-S`。模型权重和 Paddle 运行时不进入仓库，也不绕过现有 64 MiB 产品单模型预算。
2. 新增不含个人数据的固定公式 crop 语料，至少覆盖上标/下标、分数、根式、求和/积分、希腊字母、
   矩阵或分段表达。图像必须由浏览器 MathML/字体真实栅格化并记录源、许可、尺寸与 SHA-256；已有
   `v1/formula-routing` 继续作为普通 OCR 不具备结构化能力的回归样本。
3. 公式评测器只接受显式公式 crop 或独立版面模型给出的 `formula` 区域。PP-OCR 文字检测框、正则表达式、
   Unicode 数学符号和普通 OCR 置信度都不能被当作可靠的自动公式检测器。
4. 报告至少保存原始 LaTeX、保守词法归一化后的严格匹配、LaTeX token 编辑距离、独立解析器可解析率、
   单样本耗时、P50/P95、峰值内存、模型与运行时版本。归一化只能处理模型常见的无语义空白与外层数学
   定界符，不能重写公式、猜测缺失结构或把渲染相似当成等价。
5. 自动整图路由单独评估官方版面模型对 `formula` 类别的定位。只有检测召回/误报、增加的模型体积与资源
   都有本地证据时，才能接入剪贴板 OCR；否则产品候选仅限查看器/截图选区中的显式“识别公式”。
6. 运行器必须限制输入 PNG、像素、输出长度、单次 deadline 和子进程输出；模型路径、版本与 SHA 必须
   显式记录。失败应返回“公式识别不可用/失败”，并保留普通 OCR 文本，不能静默生成伪 LaTeX。

## Acceptance Criteria

- [x] 固定公式 crop 语料、生成记录、SHA/尺寸校验和负向资产测试落库。
- [x] 官方 PP-FormulaNet-S 在隔离环境完成同一语料实跑，预测、逐层指标和环境证据落库，权重不入库。
- [x] 评测测试覆盖严格匹配、只允许的词法归一化、token 编辑距离、输出预算和不支持能力。
- [x] 报告明确给出显式公式识别与自动整图路由的两个独立 go/no-go 结论，并记录包体、延迟、峰值内存及
  三平台尚未验证的边界。
- [x] OCR sidecar 单元测试和仓库本地完整门禁通过；用户可见结论同步到路线与 CHANGELOG。

## Out of Scope

- 本切片不把 PP-FormulaNet/Paddle 运行时打包进默认安装包，不提高现有 64 MiB 单模型上限。
- 不把 PP-OCR 的 `x² + 1/2 = √2` 线性输出标成 LaTeX/MathML，不用正则表达式自动判断任意文本是否为公式。
- 不承诺 LaTeX 的数学语义等价判定、公式编辑器、MathML 转换、手写公式总体精度或相机文档总体精度。
- 自动公式区域检测未通过独立门禁前，不改变剪贴板 OCR 默认路径和 UI。

## 官方依据与当前约束

- PaddleOCR 的公式 pipeline 明确由可选版面检测与独立公式识别组成；公式识别模块接收公式图像并输出
  LaTeX：<https://github.com/PaddlePaddle/PaddleOCR/blob/main/docs/version3.x/pipeline_usage/formula_recognition.en.md>
- 官方模型表给出的 `PP-FormulaNet-S` 存储大小约 224 MB、CPU 参考耗时约 254 ms；Hugging Face 官方
  仓库当前文件总量约 238 MB，许可证为 Apache-2.0：
  <https://huggingface.co/PaddlePaddle/PP-FormulaNet-S>
- 官方轻量版面模型 `PP-DocLayout-S` 约 4.834 MB，包含 formula 类别，但其公开的总体 mAP 不能直接代表
  Clippy 截图中的公式召回与误报：
  <https://github.com/PaddlePaddle/PaddleOCR/blob/main/docs/version3.x/module_usage/layout_detection.en.md>
