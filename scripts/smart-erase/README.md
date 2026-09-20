# PX-SMART-01 智能擦除可行性工具

这个目录只用于复现 `PX-SMART-01` 的离线质量与资源评估，不进入 Clippy 产品运行时。
固定语料覆盖文字、规则背景、自然纹理、强边缘和大图；所有图片由
`feasibility.py generate` 生成并由 manifest 哈希锁定。

候选模型使用 OpenCV Zoo 发布的量化 LaMa ONNX。模型不进入仓库，运行前必须下载并校验：

```bash
curl -L --fail --silent --show-error \
  -o /tmp/inpainting_lama_2025jan.onnx \
  https://huggingface.co/opencv/inpainting_lama/resolve/main/inpainting_lama_2025jan.onnx
echo "7df918ac3921d3daf0aae1d219776cf0dc4e4935f035af81841b40adcf74fdf2  /tmp/inpainting_lama_2025jan.onnx" \
  | sha256sum --check
```

在隔离 Python 3.13 环境安装 `requirements.txt` 后运行：

```bash
python scripts/smart-erase/feasibility.py benchmark \
  --model /tmp/inpainting_lama_2025jan.onnx --repeats 5
python scripts/smart-erase/feasibility.py resource-probe \
  --model /tmp/inpainting_lama_2025jan.onnx
python scripts/smart-erase/feasibility.py blind-pack
python scripts/smart-erase/feasibility.py blind-sheets
python scripts/smart-erase/verify_evidence.py
```

`benchmark` 在运行任何引擎前先校验模型 SHA-256；输入语料只读，结果只写入
`evidence/`。匿名评审时只打开 `blind-review.html` 或 `sheets/`，接触表哈希记录在
`sheets-manifest.json`；提交
`blind-review.json` 后再查看 `blind-key.json`。

这轮证据不能证明不同 CPU、Windows 或 macOS 的性能，也不能替代多评审真实照片语料。当前门控
结论是不接入编辑器；未来若更换模型、运行时、裁切策略或阈值，必须生成新的 corpus/report ID，
不能覆盖本轮结果。
