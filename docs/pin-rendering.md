# Pin 分数缩放与清晰度补偿

本文记录 Pin 在 Linux GTK/WebKit 分数缩放下的屏显边界、实现选择和环境测量。它是专题验证记录；
当前模块所有权与调用链以 [`architecture.md`](architecture.md) 为准。

## 问题边界

GTK3 不支持 `wp_fractional_scale_v1`。桌面真实缩放为 150% 时，WebKit 可能只拿到整数缓冲区缩放
200%，于是图片先由 WebView 放大，再由合成器缩回桌面大小。即使最终 CSS 尺寸正确，第一次平滑
重采样也会丢失细线和文字边缘。

这里有三种不同单位，不能互换：

- canonical image pixels：复制、保存、标注和工程文件使用的权威原图像素；
- logical/CSS pixels：Pin 内容区在桌面上的逻辑尺寸；
- buffer pixels：WebView 提交给合成器的整数缩放缓冲区尺寸。

`screenshot::desktop_scale_at` 提供真实桌面缩放，窗口/WebView 提供缓冲区缩放。这两个值由后端随
Pin payload 下发；`devicePixelRatio` 只能反映 WebView 的整数缓冲区缩放，不能代替真实桌面缩放。

## 当前实现

`src-tauri/src/pin/resample.rs` 按缓冲区分辨率生成仅用于屏显的补偿预览：

1. 以 Lanczos3 预放大作为初值；
2. 按实测的双线性缩小核预测合成器输出；
3. 将残差以 Q7 定点精度回投影，固定执行四轮；
4. Lanczos、前向缩放和回投影都按行处理，避免保留多份全图 `f32 RGBA`。

任务在后台全局串行，避免多张 Pin 同时叠加峰值工作集。窗口关闭后，尚未开始解码的排队任务会
取消。第一帧完成前得到结果时由 `SharpenSlot` 随 payload 下发，否则通过
`pin-image-sharpened` 事件替换显示图。

每个 RGBA 平面的硬预算为 64 MiB。完整工作集还包括 Q7 缓冲、i16 残差、解码原图、可选目标图和
最终 PNG，所以 64 MiB 不是进程总内存上限。超出预算会记录失败并回退 canonical 原图，不会阻止
Pin 开窗。

`src/react/pin/rendering.ts` 只保留两条 CSS 滤镜兜底：

- `isPixelExact`：显示尺寸乘真实缩放等于图片像素，允许最近邻显示；
- `isBufferExact`：输入已经是缓冲区分辨率，要求 1:1 搬入。

判据不能写成 `scale == 1`。全屏或超大图片可能在 `origin_content_size` 阶段被缩小，而用户缩放值仍
是 1；此时强制最近邻会降低质量。

## 字节与编辑语义

补偿图只是显示缓存，不进入标注坐标系、可编辑工程或输出：

- Canvas 操作文档始终使用 canonical source pixels；
- Copy 和扁平保存由后端对 canonical 原图执行权威合成；
- 可编辑 PNG 同时保存 canonical 原图、操作文档和可验证的 flattened preview；
- `update_pin` 返回轻量 `PinState`，不会在滚轮热路径重复传整张图片。

因此屏显补偿失败只影响清晰度，不能改变复制、保存或二次编辑像素。

## 环境测量

以下数字来自当时的 Linux 分数缩放测试环境，只用于比较方案和监测明显退化，不是跨平台 SLA：

| 方案 | 屏上 PSNR |
|---|---:|
| WebKit 默认平滑 | 30.28 dB |
| CSS `pixelated` | 33.95 dB |
| Lanczos3 预放大 | 34.73 dB |
| Lanczos3 + 四轮反投影 | 43.02 dB |

release 构建中，3413×1920 补偿约 614 ms，5120×2880 约 1.28 s。直接测试进程的瞬时总峰值 RSS
约 354 MiB，空测试壳基线约 6.6 MiB，补偿增量约 347 MiB。这些值受 CPU、分配器、图像内容和
缩放组合影响；修改算法后应在相同输入和环境下重测。

## 修改门禁

涉及 Pin 显示清晰度时至少验证：

1. canonical、preview、buffer 三种尺寸没有混用；
2. Copy、flat save、editable save 的字节语义没有变化；
3. 超预算、后台取消和延迟事件都能安全回退；
4. `isPixelExact` / `isBufferExact` 使用后端下发的两个缩放值；
5. 自动测试通过后，仍在目标 GTK/Wayland 分数缩放桌面进行真机对比。
