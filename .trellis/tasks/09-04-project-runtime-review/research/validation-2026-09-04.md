# 验证记录（2026-09-04）

## 自动化门禁

- `cargo fmt --check`：通过。
- `cargo check --all-targets`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`（沙箱外，允许 loopback）：442 通过、0 失败、10 个真实桌面/手动性能探针忽略。
- `npx tsc --noEmit`：通过。
- `npx vitest run`：44 个文件、839 项通过。
- GNOME 扩展静态检查：Shell 45–51 通过。
- DOM/Xvfb：9 项通过。
- Canvas 导出像素 smoke：通过，探针像素 `0 208 0`。
- 主窗口布局像素 smoke：通过，探针像素 `0 208 0`。
- Vite production build：通过（1888 modules，约 433 ms）。

受限沙箱内的 16 个翻译测试因无法绑定本机 loopback 失败，Canvas/Layout smoke 也因无法连接
Xvfb/Vite 本地端口失败；同一提交在沙箱外全部通过，因此这些不是产品回归。

## 性能基线

本机 Criterion `--quick`：

| 路径 | 时间 |
| --- | ---: |
| 剪贴板首屏 50 条 / 2000 条库 | 23.05–23.28 µs |
| FTS 前缀搜索 | 308.65–314.48 µs |
| 两字符 LIKE 回退 | 38.67–38.95 µs |
| FTS 无命中 | 282.13–282.48 µs |
| 1 MiB SHA-256 | 1.55–1.56 ms |
| 100 KiB 最坏敏感词检测 | 29.90–30.60 µs |
| 1 MiB HTML 去标签 | 1.90–1.92 ms |
| 1080p RGBA 指纹 | 1.53–1.57 ms |
| 1080p PNG 编码 | 79.04–79.36 ms |
| 1080p PNG 完整校验 | 20.49–21.12 ms |
| 1080p base64 解码 | 1.273–1.274 ms |
| 1440p PNG 完整校验 | 36.32–38.10 ms |

结论：列表/搜索 SQLite 路径远低于 1 ms，不是当前交互瓶颈；图片全尺寸解码与编码才应隔离出
async worker 和 UI 热路径。连续切换不同 Criterion target 时 Cargo 偶发复用到 release
`panic=abort` 产物并报 unwind 不兼容；对单个包执行 `cargo clean --profile bench -p clippy-app`
后可复现地运行。这是开发工具链低优先级问题，不影响应用或 CI 构建。

## 内存基线与静态生命周期

- 已安装 v0.1.20 在 GNOME Wayland 连续运行约 4 小时后：主进程 PSS 109.0 MiB、NetworkProcess
  11.6 MiB、主 WebProcess 94.2 MiB，总 PSS 约 214.8 MiB。
- 后端缩略图缓存固定 64 条；原点注册表固定 16 条；代码高亮结论缓存固定 200 条。
- 冷缩略图使用异步 Semaphore 限制为两路全尺寸 PNG 解码；等待任务不占 Tauri async worker，
  从而同时约束响应线程饥饿与 RGBA/CPU 峰值。
- 主列表失焦清空条目数组和搜索 timer；Pin 关闭会移除 manager 条目、placement generation 和
  清晰化结果；对象 URL 在替换/卸载时撤销。
- 本轮把导入工程的常驻表示从“原图 PNG + 同源 base64 + 合成预览”改为“原图 PNG + 轻量元数据
  + 合成预览”。节省量精确为 `4 * ceil(source_png_len / 3)` 附近的 base64 字符串容量，具体取决
  于分配器容量取整；磁盘工程仍保持自包含。

## 仍需实机覆盖

- 当前机器已有 v0.1.20 单实例运行，隔离 dev 实例被单实例保护正常拒绝；未中断用户的正式实例，
  因此本轮未取得“本次未提交二进制”连续 Pin/截图后的独立 PSS 曲线。
- v0.1.20 的 GNOME Wayland 双屏截图首帧已有上一轮连续三次 0.56–0.87 秒证据；本轮没有修改
  capture、frame protocol、画布比例或合成路径，相关 452 项 Rust 与 839 项前端回归仍通过。
- Windows/macOS 行为以同代码 CI 为证，实际桌面交互仍应在下一次 native QA 构建中复核。
