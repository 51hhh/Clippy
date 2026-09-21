# 本地增强 OCR 运行时

实际流水线为 PP-OCRv6 small det → 原图文字框 → 四表特征 → EdgeGNN Run/段落 → 保守视觉段落合并 → 原像素透视 crop → 可选文字行方向 → PP-OCRv6 small rec → greedy CTC。可选方向档先按框几何把高窄行旋成横向，再用官方两类模型判断 0°/180°；可选英语空白档会对字典覆盖的行再跑 en_PP-OCRv5 mobile rec。英语结果只有在非空白字符逐码点完全相同且空白增加时才采用，不会借后处理改写编号、货币或符号。EdgeGNN 的 Edge 是文字框节点之间的图边，用于分组，不是图像边缘增强，也不直接提高字符识别率。视觉合并只连接同栏、正常行距和近似字号的相邻片段，避免模型 singleton 让复制文本每行多一个空行。没有配置时应用继续使用系统 Tesseract；不自动下载模型或安装 Python。增强失败后的结果包含 `pipeline.engine=tesseract` 和 `fallbackReason`，不能把几何排序标成一次 EdgeGNN Run。

## 显式配置

已实测 Python 3.12、Linux x64 CPU。此目录是可配置开发/本地运行时，尚未打包为三平台独立安装器；其它平台需具有这些版本的可用 wheel 和绝对 Python 路径。先把五个运行文件保存在受信、稳定的目录；应用不会从 PATH 或研究证据目录猜测运行时位置。

```sh
python3 -m venv /absolute/clippy-ocr/venv
/absolute/clippy-ocr/venv/bin/python -m pip install -r /absolute/Clippy/src-tauri/ocr-sidecar/requirements.lock
/absolute/clippy-ocr/venv/bin/python /absolute/Clippy/src-tauri/ocr-sidecar/setup_models.py \
  --runtime /absolute/clippy-ocr/models \
  --python /absolute/clippy-ocr/venv/bin/python \
  --edge /absolute/existing-local/paragraph-model.onnx \
  --edge-sha256 YOUR_VERIFIED_64_CHARACTER_LOWERCASE_SHA256
```

`setup_models.py` 只在显式执行时下载三个固定的官方公开文件，完整核对 SHA；已有文件不符或 manifest 已存在时拒绝覆盖。EdgeGNN 必须由用户已有本地模型提供，工具只核对并引用该文件，不下载、提取、复制或打包它。用户须自行具备使用该模型的权利。

需要优先保留英文视觉空格时，在同一命令增加 `--english-spacing`。它额外下载固定的 7.5 MiB
`en_PP-OCRv5_mobile_rec` ONNX，并生成 `ppocrv6-edgegnn-en-v2` manifest。该档位按行懒加载；含中日文
或英语模型改变任一非空白码点时保留 PP-OCRv6 结果。固定语料显示它能补回部分公式/序列号空格，
但不能还原所有连续双空格，并会增加端到端耗时，所以不作为无条件默认值。

需要识别倒置或旋转截图时增加 `--line-orientation`。它额外下载固定的 0.96 MiB
`PP-LCNet_x0_25_textline_ori` ONNX；也可以与 `--english-spacing` 同时使用。官方模型只分类 0°/180°，
90°/270°由透视裁切后的高窄框几何归一化覆盖。只有 180°类别分数达到 0.8 才翻转，低置信结果
维持几何方向。该档位不做整页方向、拍照去畸变或竖排语言版式重排。

使用生成的 manifest 启动应用即可：

```sh
CLIPPY_OCR_MANIFEST=/absolute/clippy-ocr/models/manifest.json cargo tauri dev
```

发布版也可在 **Settings → OCR → Enhanced OCR Manifest** 选择生成的 `manifest.json`，先执行与实际
识别完全相同的运行文件/模型哈希检查，再保存设置。状态卡会显示当前引擎、pipeline ID、4–7 个模型
SHA 前缀、Tesseract fallback 以及坏配置的稳定原因；选择文件尚未按 Save 时不会切换后台识别。
清除路径后恢复 Tesseract（开发启动显式设置了 `CLIPPY_OCR_MANIFEST` 时仍以该环境变量兜底）。

Windows manifest 的 `python` 应为 venv 的 `Scripts/python.exe`；路径仍是 JSON 字符串中的绝对路径。Tesseract fallback 沿用 `CLIPPY_TESSERACT_PATH` 或系统探测设置。清除设置页 manifest，并在开发环境中同时删除 `CLIPPY_OCR_MANIFEST`，即可恢复单独 Tesseract；命令行工具不会修改应用设置。

当前安装包不包含 Python、wheel 或模型权重；随源码保存的英语字典不足 2 KiB。Linux、Windows 和 macOS
均使用相同 manifest 合同，但目前只有 Linux x64 + Python 3.12 CPU 运行时完成实测；其它平台只有在
用户准备的 Python、wheel、脚本和必需资产全部通过真实校验后才显示 ready，不能因为选过路径就显示
“已安装”。

## 模型来源与许可

| 资产 | 固定官方来源 | 全文件 SHA-256 |
| --- | --- | --- |
| det | [PaddlePaddle 5994b4f](https://huggingface.co/PaddlePaddle/PP-OCRv6_small_det_onnx/blob/5994b4f1a827310b6d38f6b13716af6381847be0/inference.onnx) | `d73e0058b7a8086bbd57f3d10b8bcd4ff95363f67e06e2762b5e814fe9c9410e` |
| rec | [PaddlePaddle 3d2d345](https://huggingface.co/PaddlePaddle/PP-OCRv6_small_rec_onnx/blob/3d2d345e6a299891174f1397a72cdd81331359c7/inference.onnx) | `5435fd747c9e0efe15a96d0b378d5bd157e9492ed8fd80edf08f30d02fa24634` |
| dictionary | [PaddleOCR 2661c7c](https://github.com/PaddlePaddle/PaddleOCR/blob/2661c7c0ef5c613e8f93c6e93b2e052399f0f854/ppocr/utils/dict/ppocrv6_dict.txt) | `b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d` |
| optional English rec | [PaddlePaddle 3fafbc3](https://huggingface.co/PaddlePaddle/en_PP-OCRv5_mobile_rec_onnx/blob/3fafbc3b5dcf93dd72add9f48368be8a3a2cd33b/inference.onnx) | `b5f833dfc5d0eb71da397b4efa06ebeee9b431b690a47d6af40d77d8eabc557f` |
| optional English dictionary | 同一固定模型的 `inference.yml` character list，本仓库仅保存字典文本 | `e025a66d31f327ba0c232e03f407ae8d105e1e709e7ccb3f408aa778c24e70d6` |
| optional line orientation | [PaddlePaddle aea1f18](https://huggingface.co/PaddlePaddle/PP-LCNet_x0_25_textline_ori_onnx/blob/aea1f18d97338aaf84b463f90192f2820524a00a/inference.onnx) | `94a6a0a0425f2b5f08b5df72086f2d72abe40f1d22f6d12d2cd83674f11f2ff3` |

官方模型卡标为 Apache-2.0；[固定 PaddleOCR LICENSE](https://github.com/PaddlePaddle/PaddleOCR/blob/2661c7c0ef5c613e8f93c6e93b2e052399f0f854/LICENSE)。本仓库不包含这些权重。重新分发模型时应随附适用的许可证和 NOTICE，不能把本地 Edge 的可加载性理解成可重新分发许可。

## 输入、结果与边界

Rust 调用 `python -I main.py --manifest PATH --manifest-sha256 SHA`。脚本只显式加入自身目录，忽略外部 PYTHONPATH。stdin 是一行 ≤16KiB JSON 然后紧随恰好 N 字节 PNG：

```json
{"version":1,"requestId":"example","pngBytes":12345,"deadlineMs":60000}
```

stdout 只返回 ≤4MiB 的 `{version,requestId,result}` JSON；stderr ≤64KiB。可选 CLI `--diagnostics /new/file.json` 记录阶段 shape/hash、真实 logits、分组和原始文字；仅用于本地合成图验收，应用不会传该参数或持久保存结构化 OCR。诊断路径必须不存在。

- 全局 1 active + 2 queued；准备/配置校验另限 3 个有界工作。相同 clip+配置请求合并，快照共享 `Arc<Vec<u8>>`，按图片 hash+完整运行身份复用；取得许可后才加载数据库 BLOB。
- 64MiB PNG、32Mi 像素、最大边16384；det 最多128个960 tile，最多1000 contours/512行/每源15边；rec H48、W≤4096，方向输入固定 160×80，两者共用总 crop 像素预算 48×4096×128，CTC T≤2048。超过预算明确失败，不静默截断。
- 60s 统一识别期限覆盖启动、模型加载、I/O、推理及剩余预算内的 fallback。超时或输出上限不再启动第二引擎。子进程 kill/wait 完成后才释放许可；没有创建后代进程的运行文件。
- 缓存身份覆盖协议、完整 manifest/options/预期资产 SHA 与五个运行 Python 文件。命中缓存前核对真实资产 SHA；运行期间配置改变时拒绝写入旧身份缓存。显式增强配置绕过旧无版本 String 缓存（包括图片翻译）；fallback 结果不缓存为增强成功。
- `lines` 保留原始文字、逐字置信度与 `accepted`；`result.text` 只拼接接受的行。空白/单行明确 `layoutExecuted=false`，多行必须有真实 Edge logits。Tesseract 本身无这些结构化框。

方向/尺度采用 `clippy-edge-features-v1`；GNN 分组后的阅读顺序采用 Clippy XY-cut：先列间隙，跨栏标题先按段切，再行内左到右。大图用960/96 detail与单次1280 overview协调完整行框，按高度/方向/法向中心及源框交集去掉接缝碎片；完整性核对使用unclip前文字core行轴，避免误把短词扩框留白当成缺字，避免吞并临近小字/宽列间隙。这些明确为 Clippy 策略，不能声称与其它产品私有像素处理完全等价。

Edge 原始分组和视觉合并后的分组在显式 diagnostics 中分别记录为 `modelGroups` 与 `groups`。视觉
合并不替换 Edge 推理；大段距、跨栏回跳、并排框和明显字号变化不会被合并。

## 质量评测合同

`quality_metrics.py` 不加载模型，只比较人工标注语料与任一引擎的结构化输出。语料和输出分别遵循
`quality-corpus.schema.json` 与 `quality-predictions.schema.json`。每个 case 的 `lines` 数组顺序就是
阅读顺序；真值行必须带稳定 `id` 和四边形，公式行另带结构化 `formula`。`source` 保存相对图片路径、
SHA-256、尺寸、来源类型与许可，用于确保重复运行读取同一原图，并阻止来源不明的图片混入可再分发
基线。采集器和评测 CLI 都会在工作开始前核对路径边界、普通文件、符号链接、PNG chunk/CRC、尺寸
与 SHA；图片身份不符时不生成预测或报告。

prediction 的 `text` 保存引擎实际返回的整段文本；`engine.capabilities` 明确声明是否提供行框和
结构化公式。Tesseract fallback 没有行框时仍可比较 CER/空白和性能，检测、逐行与阅读顺序会报告
`supported=false`，不会用虚构的整图框把“不支持”伪装成识别失败。

报告同时给出：

- 四边形 IoU 最大基数匹配后的 detection precision/recall/Hmean；
- 原始 Unicode CER 与去空白 CER；
- 经确定性字符对齐后的精确 Unicode 空白 precision/recall/F1；
- 整行完全匹配、匹配框阅读顺序 inversion rate；
- 结构化公式 support/exact rate；
- 引擎提供的 duration median/P95 与 peak RSS 上限。

原始 CER 会保留空格、换行、全角字符和标点；去空白 CER 只是诊断项，不能替代原始分数。普通 OCR
字符序列不能算作结构化公式支持。仓库已经保存 v1 smoke corpus 的 Tesseract 结果，以及 14 张同源
合成图上的 Tesseract/PP-OCRv6 small + EdgeGNN A/B；报告、环境和限制见
[`docs/reviews/2026-09-18-ocr-quality-baseline.md`](../../docs/reviews/2026-09-18-ocr-quality-baseline.md)。
这些结果验证当前管线和评测器，不代表真实截图、全部语言或公式精度；真实扩展语料仍必须保留来源、
固定图片 SHA 并重新采集，不能把合同夹具写成模型精度结果。

`quality-fixtures/browser-ui-v1/` 是 Firefox 实际栅格化的自有 UI 扩展语料，覆盖暗色设置页、代码与
斜体注释、中英日混排、表格和 2400 px 长文档。普通测试只验证已提交基线，不启动浏览器或重写
图片：

```sh
python3 src-tauri/ocr-sidecar/quality-fixtures/browser-ui-v1/capture.py --verify
```

需要有意更新基线时，先审阅 `source.html`，再在固定 Firefox 环境显式执行捕获；已有基线只有同时
传入 `--replace` 才会替换。捕获记录固定浏览器/宿主、视口、DPR、HTML/字体/脚本 SHA 和逐图 SHA。

```sh
python3 src-tauri/ocr-sidecar/quality-fixtures/browser-ui-v1/capture.py --capture --replace
```

Tesseract、增强链预测以及整体/表格分层报告保存在
`docs/reviews/evidence/2026-09-21-ocr-real-ui/`；运行环境文件只记录版本、哈希和资源指标，不记录本机
模型路径。当前结果及表格行优先修复的前后对照见 OCR 质量基线文档。

```sh
python3 src-tauri/ocr-sidecar/quality_metrics.py \
  --corpus /absolute/ocr-corpus.json \
  --predictions /absolute/ppocrv6-small.json \
  --output /absolute/ppocrv6-small-report.json
```

small/medium 模型档位只能通过研究采集器比较，不能直接生成产品配置：

```sh
/absolute/clippy-ocr/venv/bin/python src-tauri/ocr-sidecar/model_tier_ab.py \
  --base-manifest /absolute/clippy-ocr/models/manifest.json \
  --medium-det /absolute/research/PP-OCRv6-medium-det.onnx \
  --medium-rec /absolute/research/PP-OCRv6-medium-rec.onnx \
  --corpus src-tauri/ocr-sidecar/quality-fixtures/v1/corpus.json \
  --corpus src-tauri/ocr-sidecar/quality-fixtures/ui-stress-v1/corpus.json \
  --output-dir /new/private/model-tier-results
```

采集器只接受固定官方 revision、字节数和 SHA，并对 small/small、medium det、medium rec、medium/both
分别采集。每个 case 使用独立进程，并在 Linux、macOS、Windows 分别通过系统进程指标采样峰值 RSS；
报告中的内存包含解释器、运行库、模型和工作区，只能在同一平台与环境中横向比较。它的临时 manifest
带 `researchModelProfile`；Rust 产品 manifest 明确拒绝该未知字段。
当前证据显示 medium rec 只在部分英语/日文非空白字符上改善，medium det 会把表格行拆成 cell 并
破坏现有阅读顺序，整体延迟明显增加，因此设置页没有 medium 档位，64 MiB 产品单模型预算也不提高。

表格样本使用独立 row/cell 合同复核，避免把整行框和 cell 框直接当成互斥答案：

```sh
python3 src-tauri/ocr-sidecar/quality_table.py \
  --corpus src-tauri/ocr-sidecar/quality-fixtures/ui-stress-v1/table-corpus.json \
  --predictions /absolute/model-tier-results/ui-stress-v1-medium-det-predictions.json \
  --output /new/table-report.json
```

报告分别给出 raw cell Hmean、几何聚合 row Hmean、产品原始顺序 inversion，以及只按框的 y/x
重建文本。几何重建只用于诊断可恢复性；产品排序仍由 `layout_groups.py` 执行。

增强链采集时可显式增加 `--diagnostics-dir /new/directory`。采集器只创建全新目录，并为每个 case
保存有界诊断：det/Edge shape 与分组、逐行文字、CTC 接受字符的 class、发射时间步、前置 blank-run
和相邻发射间距。诊断不进入产品 IPC，也不保存完整分类张量；单次请求最多记录 4096 个字符发射。

## 验证

```sh
/usr/bin/python3 -m unittest discover -s src-tauri/ocr-sidecar -p 'test_quality*.py' -v
/absolute/clippy-ocr/venv/bin/python -m unittest discover -s src-tauri/ocr-sidecar -p test_pipeline.py -v
cargo test --manifest-path src-tauri/Cargo.toml --lib ocr:: -- --test-threads=2
CLIPPY_OCR_MANIFEST=/absolute/clippy-ocr/models/manifest.json \
CLIPPY_OCR_TEST_PNG=/absolute/synthetic-ocr.png \
cargo test --manifest-path src-tauri/Cargo.toml --lib configured_real_pipeline_through_rust_supervisor -- --ignored --nocapture
```

公式单元测试不等同于模型准确率。真实验收需另记原图文字/阅读顺序/真实多行分组/CER/耗时/RSS，并覆盖亮暗底、中英、旋转、双列、4K/8K、空图及坏模型回退。固定四向合成语料只验证方向链合同，不代表拍照、曲面或竖排语言准确率。模型可能省略中文视觉空格，GNN 也可能选择 singleton；保留这些结果，不插入期望文本使测试通过。
