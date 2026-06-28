# 截图性能与内存优化

## Goal

降低截图功能引入后的常驻内存和交互卡顿，使 Clippy 保持轻量小工具定位。

## Requirements

- 分析当前截图实现的主要内存来源和卡顿路径。
- 降低截图数据在后端、IPC、前端中的峰值体积。
- 优化截图编辑画布拖拽、缩放、选区和绘制的重绘频率。
- 避免隐藏或关闭截图窗口后长期保留大图数据。
- 保持现有截图快捷键、选区、Copy/Save/Pin 功能可用。

## Acceptance Criteria

- [x] 截图 PNG 编码不再使用明显膨胀的数据格式。
- [x] 画布交互通过 rAF 合并高频 pointer/wheel 更新。
- [x] 关闭截图窗口时释放待编辑截图缓存。
- [x] 截图窗口关闭路径清理前端大对象引用。
- [x] Rust/前端测试、typecheck、Vite build 和 quick CI 通过。

## Out of Scope

- 不引入全量 React 迁移。
- 不实现翻译/OCR 截图。
- 不重写底层截图后端。
