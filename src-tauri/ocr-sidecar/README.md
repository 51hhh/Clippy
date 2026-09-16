# 本地增强 OCR 运行时

实际流水线为 PP-OCRv6 small det → 原图文字框 → 四表特征 → EdgeGNN Run/段落 → 原像素透视 crop → PP-OCRv6 small rec → greedy CTC。没有配置时应用继续使用系统 Tesseract；不自动下载模型或安装 Python。增强失败后的结果包含 `pipeline.engine=tesseract` 和 `fallbackReason`，不能把几何排序标成一次 EdgeGNN Run。

## 显式配置

已实测 Python 3.12、Linux x64 CPU。此目录是可配置开发/本地运行时，尚未打包为三平台独立安装器；其它平台需具有这些版本的可用 wheel 和绝对 Python 路径。先把四个运行文件保存在受信、稳定的目录；应用不会从 PATH 或研究证据目录猜测运行时位置。

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

使用生成的 manifest 启动应用即可：

```sh
CLIPPY_OCR_MANIFEST=/absolute/clippy-ocr/models/manifest.json cargo tauri dev
```

Windows manifest 的 `python` 应为 venv 的 `Scripts/python.exe`；路径仍是 JSON 字符串中的绝对路径。Tesseract fallback 沿用 `CLIPPY_TESSERACT_PATH` 或系统探测设置。删除环境变量即恢复单独 Tesseract，工具不会修改应用设置。

## 模型来源与许可

| 资产 | 固定官方来源 | 全文件 SHA-256 |
| --- | --- | --- |
| det | [PaddlePaddle 5994b4f](https://huggingface.co/PaddlePaddle/PP-OCRv6_small_det_onnx/blob/5994b4f1a827310b6d38f6b13716af6381847be0/inference.onnx) | `d73e0058b7a8086bbd57f3d10b8bcd4ff95363f67e06e2762b5e814fe9c9410e` |
| rec | [PaddlePaddle 3d2d345](https://huggingface.co/PaddlePaddle/PP-OCRv6_small_rec_onnx/blob/3d2d345e6a299891174f1397a72cdd81331359c7/inference.onnx) | `5435fd747c9e0efe15a96d0b378d5bd157e9492ed8fd80edf08f30d02fa24634` |
| dictionary | [PaddleOCR 2661c7c](https://github.com/PaddlePaddle/PaddleOCR/blob/2661c7c0ef5c613e8f93c6e93b2e052399f0f854/ppocr/utils/dict/ppocrv6_dict.txt) | `b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d` |

官方模型卡标为 Apache-2.0；[固定 PaddleOCR LICENSE](https://github.com/PaddlePaddle/PaddleOCR/blob/2661c7c0ef5c613e8f93c6e93b2e052399f0f854/LICENSE)。本仓库不包含这些权重。重新分发模型时应随附适用的许可证和 NOTICE，不能把本地 Edge 的可加载性理解成可重新分发许可。

## 输入、结果与边界

Rust 调用 `python -I main.py --manifest PATH --manifest-sha256 SHA`。脚本只显式加入自身目录，忽略外部 PYTHONPATH。stdin 是一行 ≤16KiB JSON 然后紧随恰好 N 字节 PNG：

```json
{"version":1,"requestId":"example","pngBytes":12345,"deadlineMs":60000}
```

stdout 只返回 ≤4MiB 的 `{version,requestId,result}` JSON；stderr ≤64KiB。可选 CLI `--diagnostics /new/file.json` 记录阶段 shape/hash、真实 logits、分组和原始文字；仅用于本地合成图验收，应用不会传该参数或持久保存结构化 OCR。诊断路径必须不存在。

- 全局 1 active + 2 queued；准备/配置校验另限 3 个有界工作。相同 clip+配置请求合并，快照共享 `Arc<Vec<u8>>`，按图片 hash+完整运行身份复用；取得许可后才加载数据库 BLOB。
- 64MiB PNG、32Mi 像素、最大边16384；det 最多128个960 tile，最多1000 contours/512行/每源15边；rec H48、W≤4096、总 crop 像素≤48×4096×128、T≤2048。超过预算明确失败，不静默截断。
- 60s 统一识别期限覆盖启动、模型加载、I/O、推理及剩余预算内的 fallback。超时或输出上限不再启动第二引擎。子进程 kill/wait 完成后才释放许可；没有创建后代进程的运行文件。
- 缓存身份覆盖协议、完整 manifest/options/预期资产 SHA 与四个运行 Python 文件。命中缓存前核对真实资产 SHA；运行期间配置改变时拒绝写入旧身份缓存。显式增强配置绕过旧无版本 String 缓存（包括图片翻译）；fallback 结果不缓存为增强成功。
- `lines` 保留原始文字、逐字置信度与 `accepted`；`result.text` 只拼接接受的行。空白/单行明确 `layoutExecuted=false`，多行必须有真实 Edge logits。Tesseract 本身无这些结构化框。

方向/尺度采用 `clippy-edge-features-v1`；GNN 分组后的阅读顺序采用 Clippy XY-cut：先列间隙，跨栏标题先按段切，再行内左到右。大图用960/96 detail与单次1280 overview协调完整行框，按高度/方向/法向中心及源框交集去掉接缝碎片；完整性核对使用unclip前文字core行轴，避免误把短词扩框留白当成缺字，避免吞并临近小字/宽列间隙。这些明确为 Clippy 策略，不能声称与其它产品私有像素处理完全等价。

## 验证

```sh
/absolute/clippy-ocr/venv/bin/python -m unittest discover -s src-tauri/ocr-sidecar -p test_pipeline.py -v
cargo test --manifest-path src-tauri/Cargo.toml --lib ocr:: -- --test-threads=2
CLIPPY_OCR_MANIFEST=/absolute/clippy-ocr/models/manifest.json \
CLIPPY_OCR_TEST_PNG=/absolute/synthetic-ocr.png \
cargo test --manifest-path src-tauri/Cargo.toml --lib configured_real_pipeline_through_rust_supervisor -- --ignored --nocapture
```

公式单元测试不等同于模型准确率。真实验收需另记原图文字/阅读顺序/真实多行分组/CER/耗时/RSS，并覆盖亮暗底、中英、旋转、双列、4K/8K、空图及坏模型回退。模型可能省略中文视觉空格，GNN 也可能选择 singleton；保留这些结果，不插入期望文本使测试通过。
