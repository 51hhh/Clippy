# 独立公式识别与自动路由可行性报告

需求：`PX-OCR-FORMULA-01`
日期：2026-09-21
结论：显式公式 crop 的独立识别链成立，但当前产品接入 no-go；自动整图公式路由 no-go。

## 结论

普通 PP-OCR 只能输出 `x² + 1/2 = √2` 这类线性字符，不能恢复分数、上下标、根式、矩阵或分段
函数的二维结构。独立公式模型方向正确：PP-FormulaNet-S 与 plus-S 都能从浏览器 MathML crop 生成
LaTeX，而且不是把普通 OCR 字符串包装成 LaTeX。

当前实现不能进入默认产品，也暂不增加“识别公式”按钮：

- S 权重 231,675,001 bytes，plus-S 权重 256,845,006 bytes，分别约为现有 64 MiB 单模型预算的
  3.45 倍和 3.83 倍；本次 Paddle OCR 隔离环境实际占用约 1.45 GiB。
- 两个模型的 Linux CPU 峰值 RSS 分别约 899 MiB 和 920 MiB，初始化约 3.5 秒；暖态单 crop
  P50 仍约 325 ms / 360 ms。
- 两个模型都只有 5/6 输出能通过独立 LaTeX 解析器。S 的矩阵环境括号不配对，且积分结果虽可解析
  却生成异常的 `frac/slash/pipi` token；plus-S 的希腊下标输出截断，分段式虽可解析却丢失 `<`。
  这类结构和符号错误不能用普通 OCR 文本后处理安全猜回。
- PP-DocLayout-S 在 6 个孤立公式 crop 中没有产生一个 `formula` 框。该模型面向整页文档，本轮正样本
  不代表其官方文档域总体精度，但足以否定“直接对任意剪贴板图片自动路由”的实现。4 张无公式 UI
  没有误报，不能抵消正样本召回为 0。

因此分两层处理：独立公式识别保留为研究链，后续只有在更小运行时、固定语料 100% 可解析且关键
符号不丢失后，才考虑查看器/截图选区中的显式可选入口；自动整图路由必须先增加包含正文、行内公式、
展示公式和无公式 UI 的整页真值语料，再单独评估版面模型。剪贴板普通 OCR 继续声明
`structuredFormula=false`。

## 固定语料与指标

`quality-fixtures/formula-browser-v1/` 由 Firefox 155.0.1 实际栅格化 MathML，再按 DOM 真值框裁切。
6 个自有公式覆盖分数、根式、上下标、积分/求和极限、希腊字母、矩阵和分段函数；源 HTML、捕获
脚本、Noto Sans CJK SC 正文字体、Noto Sans Math 公式字体、视口、图片尺寸和 SHA-256 均固定。
捕获合同会同时校验两种字体，避免 MathML 静默使用宿主系统字体。它验证当前打印公式 crop，不代表
手写、相机照片、曲面或总体公式精度。

LaTeX 报告同时保留三种互不替代的信号：

- raw exact：原始字符串完全一致；
- normalized exact / token error：只忽略数学模式普通空白和最外层定界符，不改写命令、花括号或符号；
- parseable：第三方 `latex2mathml` 能否解析输出，只证明语法入口可接受，不证明数学语义等价。

严格 token 距离会把多余花括号、`\big`、`\displaylimits` 和不同矩阵环境计为差异，所以不能单独解释
为数学准确率；反过来，可解析也不能掩盖 plus-S 丢失 `<`。两者必须与逐 case 原文一起审阅。

| 模型 | 权重 | load | P50 / P95 | 峰值 RSS | 可解析 | normalized exact | token error |
|---|---:|---:|---:|---:|---:|---:|---:|
| PP-FormulaNet-S | 231.7 MB | 3.49 s | 325 / 385 ms | 942,575,616 B | 5/6 | 0/6 | 56.16% |
| PP-FormulaNet_plus-S | 256.8 MB | 3.50 s | 360 / 472 ms | 964,960,256 B | 5/6 | 1/6 | 45.21% |

plus-S 改善了简单分数、积分与求和的严格 token 距离，但没有形成无条件替换：S 的语法失败集中在
矩阵，plus-S 的语法失败转移到希腊下标，且其分段函数丢失关键关系符号。样本只有 6 个，不能据此
使用平均数掩盖任一阻塞 case。

## 自动公式区域探针

固定 PP-DocLayout-S 权重为 4,804,904 bytes。PaddlePaddle 3.3.1 CPU oneDNN 路径会在本模型上触发
`ConvertPirAttribute2RuntimeAttribute` 未实现错误；探针显式关闭 oneDNN 后完成，避免把崩溃写成
模型精度失败。

关闭 oneDNN 的 Linux 结果为：模型加载约 603 ms，10 张输入 P50/P95 约 198/211 ms，峰值 RSS
640,012,288 bytes；6 个公式 crop 召回 0%，4 个无公式浏览器 UI 的 formula 误报为 0。正样本是
识别器输入 crop，不是 PP-DocLayout-S 的整页训练域，所以结论只适用于 Clippy 当前没有可验证自动
路由链，不能反向宣称该官方模型总体无效。

## 供应链与复现边界

- PP-FormulaNet-S：官方 Hugging Face revision
  `0572450e501be9eb1b1cdb7e00fccf4b22fab4df`，权重 SHA-256
  `b6392296d16e2a9f414c0a751d7ccbd1bd9d8272b68aab72df1d3875f35a7489`。
- PP-FormulaNet_plus-S：revision `3d46f557e3a1752f4bf81202395af3b5ecfadfd2`，权重 SHA-256
  `e464f94412feaa98f8791eacc84684f887b3569e30e80c52b8112e9cf7d4069b`。
- PP-DocLayout-S：revision `8ac289e66575bb9bba6e15c53719d8b15cc9b3b2`，权重 SHA-256
  `491c3382d84ca04d2033afbee0c105942ed82fea392bb4a19170646adebe088a`。
- 运行时固定为 PaddleOCR 3.7.0、PaddlePaddle 3.3.1、PaddleX 3.7.2、Python 3.12.14；模型权重、
  venv 和本机路径未进入仓库。

预测、报告与无路径环境证据位于
[`docs/reviews/evidence/2026-09-21-ocr-formula/`](evidence/2026-09-21-ocr-formula/)；采集器在推理前核对
固定文件字节数与 SHA，输出采用 create-only，输入 PNG 继续走共享路径、chunk/CRC、尺寸与 SHA
校验。

官方资料：

- [PaddleOCR 公式识别 pipeline](https://github.com/PaddlePaddle/PaddleOCR/blob/main/docs/version3.x/pipeline_usage/formula_recognition.en.md)
- [PP-FormulaNet-S 官方模型页](https://huggingface.co/PaddlePaddle/PP-FormulaNet-S)
- [PP-FormulaNet_plus-S 官方模型页](https://huggingface.co/PaddlePaddle/PP-FormulaNet_plus-S)
- [PP-DocLayout-S 官方模型页](https://huggingface.co/PaddlePaddle/PP-DocLayout-S)

## 验证

- OCR sidecar 质量合同：25 项通过，包含语料资产、严格归一化、token 距离、输出预算与版面召回/误报；
- 固定公式捕获记录、两种字体 SHA、全部 PNG 尺寸/SHA 与共享 PNG chunk/CRC 校验通过；
- `./scripts/ci-local.sh`：25 个步骤通过、0 失败、2 个按配置跳过；Rust 1048 项、前端 1250 项通过；
- 本地门禁只覆盖 Linux x86_64。Windows/macOS 公式运行时与模型没有执行；远程原生 CI 只能验证仓库
  编译和既有测试，不能替代三平台同源公式模型基准。

## 下一门禁

1. 建立整页公式路由语料，分别覆盖行内/展示公式、正文、代码、金额、数学符号和无公式 UI；先测
   公式区域 recall/precision，再决定是否需要 PP-DocLayout-S、独立轻量分类器或只保留显式框选。
2. 研究官方 ONNX/静态图裁剪后的实际依赖，不把 1.45 GiB Python 环境直接包装成桌面功能；任何可选
   组件要有独立下载、许可、SHA、磁盘和卸载合同。
3. 扩充真实扫描、压缩、低对比和中英文公式；阻塞集必须达到 100% 可解析且关键关系符号不丢失。
4. 在 Linux 结论满足后再跑 Windows/macOS 同源 CPU、峰值内存和语法结果；三平台不能互相替代。
