# 现状审查与设计约束

## 已确认问题

1. `usePinCanvas` 用当前 `<img>.naturalWidth` 计算交互 scale；该图片可能是缓冲区补偿版，而导出改用原图，标注坐标会按两者尺寸比错位。
2. 当前 Save image 无条件把完整原图写入普通 PNG iTXt，模糊/马赛克后的文件若分享可恢复敏感像素，必须提供清晰的可编辑/扁平语义。
3. 保存只限制合成 PNG 为 64 MiB，加入 base64 原图后可超过读取侧 64 MiB 总文件上限，可能生成自己无法读取的文件。
4. `read_pin_project` 与前端 API 已存在，但没有打开入口、调用方、PinEntry 恢复或 React history hydration。
5. `add_itxt_chunk` 创建的 iTXt 默认未压缩，当前实现没有把 `compressed` 设为 true。
6. `sourcePngBase64`、annotations、adjustments 没有完整运行时校验。
7. 工程版本只检查 `<= current`，没有迁移；效果常量和字体策略也未进入工程语义。
8. `dirty = annotations.length > 0` 不是保存基线，保存后仍会重复提示。

## 当前正确基础

- `PinEntry.source: Arc<PinSource>` 已避免缩放热路径复制原图。
- `get_pin_source_image` 能从后端条目按需返回真正原图。
- `renderExport` 可以从原图 + annotations + adjustments 生成合成 PNG。
- `save_pin_canvas` 已把剪贴板合成像素与落盘 PNG 分开处理。
- `PinProject` 已有格式/版本字段和损坏元数据降级测试。
- quick CI、Canvas/Layout 像素 smoke 与真实 binary link 在 2026-09-02 审查时通过。

## 决策

- 保留单 PNG：IDAT 是合成预览/分享载体，压缩 iTXt 是可编辑工程。
- 运行时共享原图指针，持久化内嵌原图字节，保证文件可移植。
- 文档坐标固定为原图像素；补偿图只属于显示优化层。
- 可编辑保存与安全扁平导出必须在 UI 上可区分。
- 不升级为完整 PSD 图层系统；annotations 是可编辑操作对象列表。

