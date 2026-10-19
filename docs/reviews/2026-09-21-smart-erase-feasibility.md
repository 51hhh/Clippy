# PX-SMART-01 智能擦除可行性结论

日期：2026-09-21
需求 ID：`PX-SMART-01`
结论：**当前候选不进入产品实现，编辑器继续不展示“智能擦除”。**

## Goal

判断本地内容感知修复在许可、包体、CPU 延迟、内存和视觉质量上是否适合 Clippy，并在任何产品
接入前留下可重复的固定语料、匿名评审和资源证据。

## Requirements

- 固定覆盖文字、规则背景、自然图像、强边缘和大图的语料；
- 对比 Telea、Navier–Stokes 与一个许可明确的本地模型；
- 记录模型身份、许可、运行时、包体、延迟、内存和失败表现；
- 原图、标注和撤销历史不能因候选失败而改变；
- 未通过预设门槛时，不得把工具入口或模型依赖接入产品。

## Acceptance Criteria

- [x] 五类固定样本均有哈希、匿名顺序人工评审和五轮性能数据；
- [x] 研究工具只读固定输入并写独立 evidence 目录，产品标注链没有接入候选，因此失败不能改写
  原图、标注或撤销历史；
- [x] 门槛与观测值由机器校验，当前候选未通过，未进入实现计划。

## Out of Scope

本轮不实现编辑器工具，不下载模型到安装包，不上传图片，不使用云端生成式修复，不把对 PixPin
二进制的观察当成算法来源，也不以单机合成语料代表三平台真实照片表现。

## 候选与许可筛选

传统基线采用 OpenCV `cv::inpaint` 的 Telea 与 Navier–Stokes 路径。OpenCV 文档明确这两个枚举
以及“由区域邻域恢复掩码像素”的合同；OpenCV 4.5 之后本体为 Apache-2.0。

模型候选采用 Open Source Vision Foundation 账号发布的量化 LaMa ONNX：

- 模型：`inpainting_lama_2025jan.onnx`；
- 大小：92,591,623 bytes；
- SHA-256：`7df918ac3921d3daf0aae1d219776cf0dc4e4935f035af81841b40adcf74fdf2`；
- 模型目录声明全部文件为 Apache-2.0；
- 运行时 ONNX Runtime 为 MIT。

研究同时排除了 MI-GAN。它的官方仓库公开了 ONNX 导出与移动端目标，但截至本轮，作者仓库中的
权重许可问题仍在询问权重是否受仓库许可证覆盖、是否受教师模型非商业条款影响。这个不确定性
不满足 Clippy “许可必须明确覆盖候选文件”的门槛，因此没有下载或评测其权重。

一手来源：

- [OpenCV inpaint API](https://docs.opencv.org/3.4.6/d7/d8b/group__photo__inpaint.html)
- [OpenCV 许可说明](https://opencv.org/license/)
- [OpenCV Zoo LaMa 模型卡](https://huggingface.co/opencv/inpainting_lama)
- [OpenCV Zoo LaMa 模型文件与 SHA-256](https://huggingface.co/opencv/inpainting_lama/blob/main/inpainting_lama_2025jan.onnx)
- [ONNX Runtime MIT LICENSE](https://github.com/microsoft/onnxruntime/blob/main/LICENSE)
- [MI-GAN 官方仓库](https://github.com/Picsart-AI-Research/MI-GAN)
- [MI-GAN 权重许可澄清问题](https://github.com/Picsart-AI-Research/MI-GAN/issues/25)

## 固定语料与方法

`scripts/smart-erase/corpus/manifest.json` 锁定五组输入、掩码和无物体参考图：

| 样本 | 类别 | 尺寸 | 观察重点 |
|---|---|---:|---|
| `text_ruled_paper` | 文字 | 768×512 | 删除文字后细横线与纸张渐变能否连续 |
| `regular_grid` | 规则背景 | 768×512 | 横纵网格交点、粗细线是否延续 |
| `natural_foliage` | 自然纹理 | 768×512 | 不规则纹理是否出现重复、辐射或色块 |
| `strong_edge` | 强边缘 | 768×512 | 跨越高反差折线时是否产生断裂和色晕 |
| `large_document` | 大图 | 2048×1536 | 512 模型经过 ROI 缩放后能否恢复重复文档线 |

所有样本由仓库脚本生成，不含第三方照片。LaMa wrapper 使用普通的掩码外接矩形、固定比例 padding、
方形反射填充和 512×512 输入；输出只在掩码内合成。该裁切策略是 Clippy 的研究实现，没有复制
参考软件的私有算法或参数。

每个方法预热后执行五轮。指标同时记录掩码 MAE/PSNR、边界 MAE、梯度 MAE和掩码外逐像素不变。
指标用于发现回归，最终可接受性仍由隐藏方法名的 A/B/C 接触表判断，因为强边缘样本证明更低的
像素误差仍可能对应更明显的色晕。

## 进入产品前的门槛

| 维度 | 门槛 | 理由 |
|---|---:|---|
| 许可 | 模型与运行时文件均有明确可再分发许可 | 安装包不能依赖推断出的权重许可 |
| 模型 | ≤ 100 MiB | 保持可选资产仍能被正常下载和更新 |
| 最低实测运行包子集 | ≤ 160 MiB | 即使不计 Python 解释器与 Pillow，也不能显著放大安装体积 |
| 冷启动 | ≤ 1.5 s | 首次点击不能长时间无结果 |
| CPU p95 | ≤ 1.0 s | 擦除需要接近交互式反馈 |
| 单次峰值 RSS | ≤ 512 MiB | 与剪贴板大图、OCR 和 Pin 并存时仍留有余量 |
| 匿名评审 | 至少 4/5 最佳或并列最佳，且 0 个不可接受失败 | 平均提升不能掩盖结构性坏图 |
| 完整性 | 5/5 掩码外逐像素不变 | 候选只能修改用户明确选择的区域 |

阈值由 `evidence/decision.json` 表达，`verify_evidence.py` 从原始结果重新计算每一项，避免文档结论
与数据漂移。

## 结果

宿主为 Intel Core Ultra 5 125H、18 个逻辑 CPU，Linux x86_64；候选使用 ONNX Runtime 1.30.0、
4 个 CPU 线程。完整原始结果见 `scripts/smart-erase/evidence/benchmark.json`。

| 方法 | 五类样本最慢 p95 | 匿名第一名 | 不可接受样本 |
|---|---:|---:|---:|
| Telea | 138.674 ms | 1/5 | 由接触表逐例记录 |
| Navier–Stokes | 111.995 ms | 0/5 | 由接触表逐例记录 |
| LaMa ROI | 2,221.520 ms | 4/5 | 1/5（强边缘色晕） |

LaMa 的匿名质量明显高于传统方法：它在文字、网格、自然纹理和大文档四项排名第一；Telea 在强
边缘项更好。LaMa 在强边缘中产生宽范围黄蓝色晕，被判为不可接受，因此没有通过“零结构性失败”
门槛。

资源侧同样未通过：冷 session 约 2,959.769 ms，单次推理约 2,160.199 ms，单次进程峰值
680,140,800 bytes（约 648.6 MiB）。模型、ONNX Runtime、NumPy 与 OpenCV 的最低实测包子集共
275,521,386 bytes（约 262.8 MiB）；它尚未计入 Python 解释器与 Pillow，因此是保守下界。
完整五样本进程峰值更高，但不拿它代替单次产品预算。

模型大小与许可通过；当前可运行栈大小、冷启动、交互延迟、内存和零不可接受失败均未通过。

## 安全与失败合同

- 模型哈希在创建输出目录和启动推理前校验；错误模型直接失败；
- corpus 的输入、掩码、参考图与 manifest 哈希由纯标准库 verifier 锁定；
- benchmark 只写 `evidence/outputs/`，不会覆盖 corpus；
- 每个方法的输出均证实掩码外像素与参考图逐像素一致；
- Clippy 的标注模型、撤销栈、PNG 导出和 Pin 修订代码没有接入该候选。

因此本轮失败的实际效果是“没有产品状态变化”。未来若候选通过门槛，产品实现仍必须采用临时
preview → 校验快照身份 → 单次 history commit 的事务结构，并覆盖取消、超时、模型异常和晚到结果；
不能把 benchmark 的输出写法直接搬进编辑器。

## 决策

`PX-SMART-01` 的可行性研究完成，当前候选状态为 **gated / no-go**。Clippy 不新增“智能擦除”
按钮，不捆绑 LaMa、ONNX Runtime 或 Python 栈，也不把 Telea 当成质量等价的替代品。

只有以下任一条件形成新证据后才重开：许可明确且更小的模型；三平台硬件加速并有同预算 CPU
fallback；或扩大真实图片盲评后消除强边缘不可接受失败。重开必须使用新报告 ID，不覆盖本轮证据。
