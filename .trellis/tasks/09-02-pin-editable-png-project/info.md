# 可编辑 PNG 画布工程技术方案

## 结论

采用单个标准 PNG 作为可移植工程文件：IDAT 保存最新完整合成图，压缩 iTXt `clippy-project` 保存自包含的可编辑工程。运行时继续以 `Arc` 共享源图字节；持久化不能依赖内存指针、临时文件或剪贴板记录。

所有文档对象统一使用原图像素坐标。清晰度补偿图仅用于未编辑贴图的屏幕显示，不参与画布交互、标注、效果或导出。

## 用户可见契约

1. 未进入编辑且没有文档修改时，现有贴图显示与快速复制行为保持不变。
2. 已编辑图片提供两个明确动作：
   - **保存可编辑 PNG**：保存合成 IDAT、压缩工程 iTXt，并把同一合成像素写入剪贴板；界面明确提示文件包含未打码原图。
   - **导出扁平 PNG**：只保存合成 IDAT，不含工程元数据，适合分享。
3. 主窗口增加“打开图片”入口以及 `Ctrl/Cmd+O`；普通 PNG 和不可恢复工程按扁平图片打开。
4. 保存继续沿用当前“不覆盖已有文件、生成新文件”的语义。
5. 现有 v1 工程无法证明其坐标空间，按普通扁平 PNG 打开；本次写入 v2。

## 持久化模型 v2

```text
PinProjectV2 {
  format: "clippy-pin-project",
  formatVersion: 2,
  rendererVersion: 1,
  createdAt,
  appVersion,
  source: {
    pngBase64,
    width,
    height,
    sha256
  },
  document: {
    annotations,
    adjustments
  }
}
```

- PNG 合成像素与工程 JSON 分别编码，iTXt compression flag 必须为 true。
- 工程数据在 Rust 文件信任边界完成结构、数值、资源数量、PNG、尺寸和哈希校验；TypeScript 再做防御性 schema 校验。
- 当前版本固定“图像调整/像素效果先于矢量对象”的渲染顺序，并持久化影响外观的效果参数。

## 运行时模型

- `PinSource` 增加工程来源，持有 canonical source PNG、保存时 flattened preview PNG 和已验证 document。
- 显示路径可以直接使用 preview；打开画布和渲染必须使用 source；文档未变化时复制可复用 preview，变化后必须重新渲染。
- `PinPayload` 增加可选初始工程；前端构造 `EditorDocument` 后一次性初始化 history 和 adjustments。
- `EditorDocument` 至少包含 rendererVersion、sourceWidth、sourceHeight、annotations、adjustments。

## 资源预算

- 合成 PNG：64 MiB。
- 内嵌原图 PNG：64 MiB。
- 解压后工程 JSON：96 MiB。
- PNG 容器总大小：160 MiB。
- annotations：最多 10,000 个；单 stroke 最多 100,000 点；总点数最多 500,000；文本最多 16 KiB；对象 ID 最多 128 字符且必须唯一。

保存侧和读取侧共用同一组常量；在 base64 和 iTXt 开销计入后，应用不得生成超过读取预算的文件。

## 原子性与失败语义

- 保存先写同目录临时文件，完整写入并刷新后再原子提交；失败清理临时文件，不破坏既有目标。
- 工程块添加失败时，“保存可编辑 PNG”整体失败，禁止静默降级成扁平成功。
- 先成功落盘，再写剪贴板；返回结构化结果，以便 UI 区分文件成功但剪贴板失败等状态。
- 文件选择取消不创建窗口或 `PinManager` 条目；解析/校验失败同样不留下半初始化状态。

## 实施阶段

### A. 统一文档与坐标

- 写出补偿尺寸与原图尺寸不同的坐标/像素回归测试。
- 画布按需加载 canonical source，交互和 renderer 只使用 source dimensions。
- 引入 document revision 与 savedRevision；关闭画布不能让已有标注消失。

### B. 工程格式与后端信任边界

- 定义 v2、压缩 iTXt、共享限制常量和完整验证器。
- 增加工程型 `PinSource`、打开 PNG 命令、文件对话框和原子保存。
- ordinary/corrupt/v1/future 项目均保留 IDAT 并安全降级为 flat。

### C. 保存、复制和隐私 UI

- 拆分 editable save 与 flat export；editable 操作显示包含原图警告。
- 编辑后 Copy/Ctrl+C 渲染并复制当前 composition，不回退原图/旧 preview。
- 保存成功推进 checkpoint；后续修改重新变 dirty。

### D. 打开与恢复

- 主窗口增加打开入口和键盘快捷键。
- 合法 v2 恢复 source/document/history baseline；继续编辑可撤销回导入基线。
- 错误和取消路径做窗口/manager 无残留测试。

### E. 闭环

- 增补格式、信任边界、坐标、导入、隐私、dirty、copy/save 的 Rust/TS/DOM/Canvas 测试。
- 更新架构文档、规格与 CHANGELOG。
- 跑真实二进制链接、quick CI、完整前端构建；独立 code review、QA 和 gate review，修复全部 P0/P1/P2。

## 可执行 QA 矩阵

| 阶段 | 工具/位置 | 操作 | 必须观察到的结果 |
| --- | --- | --- | --- |
| A 坐标 | `src/tests/fixtures/canvas-export-smoke.ts` + `./scripts/smoke-canvas-export.sh`（Firefox + ffmpeg/Pillow 像素断言） | 用 2560×1440 source 和 3413×1920 compensation，分别在边角、中心绘制矩形与 blur，再以 source 尺寸导出 | 预览和导出对象覆盖相同 source 像素；坐标不乘 3413/2560；关/开画布与贴图缩放后像素断言不变 |
| A history | Vitest `src/tests/pin-react-app.test.js` | 导入含标注文档，新增一笔，执行 undo/redo，保存后关闭，再修改并关闭 | 首次 undo 回到导入基线而不是空白；保存后无提示；新修改后出现保存提示 |
| B 格式 | Rust `src-tauri/src/pin/project.rs` 单测 | 写入 v2，检查 iTXt compression flag，抽取并逐字段比较 source/document/hash/dimensions | 标准 PNG 解码成功；iTXt 确实压缩；数据完整往返 |
| B 降级 | Rust project/commands 单测 | 构造无块、v1、损坏 JSON/压缩流、未来版本、伪 base64、尺寸/哈希不符、重复 ID、非有限数值和超限数组 | 元数据不可恢复时保留 IDAT 并按 flat 打开；信任边界拒绝危险 document；进程不 panic |
| B 预算 | Rust project/commands 边界单测 | 在 source/render/project/container 各上限与上限+1 保存；将成功生成的最大允许工程立即重新读取 | 所有 writer 接受的文件 reader 接受；+1 在写入前或读取前明确失败；读取先做 metadata 检查 |
| B 原子性 | Rust image I/O/commands 单测 | 注入临时写、rename、工程 embed 失败 | 目标不存在半文件，已有目标不被破坏，临时文件被清理，可编辑保存不得报告 flat 成功 |
| C 保存/复制 | Vitest `src/tests/pin-react-app.test.js` + `src/tests/ipc-api.test.js` | 修改后触发 toolbar/menu/Ctrl+C、editable save、flat export；mock 保存成功但 clipboard 失败 | Copy 发送最新 composition 且无元数据；editable/flat 调用不同 mode；文件已成功时 checkpoint 保持 clean，并单独报告 clipboard 失败，不诱导重复保存 |
| C 隐私 | PNG chunk 检查 + `cargo tauri dev` 手测 | 对含 blur/mosaic 的图分别保存 editable 与 flat，用普通查看器打开，并搜索 `clippy-project` | 两者可立即查看且像素一致；editable 明示包含原图且可恢复；flat 无 keyword/原图且可安全分享 |
| D 打开 | Rust commands 单测 + DOM/Vitest | 从主窗口打开合法 v2、普通 PNG、损坏/未来工程、非 PNG；取消 chooser | 合法 v2 创建工程型 PinSource；可降级项创建 flat pin；失败/取消不创建窗口、不增加 `PinManager` 条目 |
| D 恢复 | `cargo tauri dev` 真实手测 | 保存 editable，关闭，通过主窗口 `Ctrl/Cmd+O` 重开，继续编辑、undo/redo、再次保存重开 | 显示 IDAT 快速预览；画布加载内嵌 source；全部 annotations/adjustments 恢复，新增操作可撤销到导入基线，第二次往返一致 |
| E 门禁 | CLI | 运行 `cargo fmt --check`、`cargo clippy -- -D warnings`、Rust tests、Vitest、DOM/Canvas/Layout smoke、Vite build、`cargo build --bin clippy-app`、`./scripts/ci-local.sh --quick` | 全部退出码为 0；独立 code review、QA、gate review 无未解决 P0/P1/P2 |

真实 Tauri 手测要保存操作记录与可核对的 PNG 样本；若当前桌面环境不能启动 GUI，必须明确记录阻塞，并以命令/DOM/像素证据覆盖可自动化部分，不能把未执行手测写成通过。

## 必须保持的不变量

- annotation 坐标始终是 source pixels。
- compensation image 永不成为工程 source 或导出底图。
- flat 文件没有 `clippy-project`；editable 文件必有可验证的 v2 工程。
- writer 接受的文件 reader 必须接受。
- clean 当且仅当 `savedRevision === currentRevision`。
- 不支持或损坏的工程元数据不能妨碍 PNG IDAT 作为普通图片使用。
