# 重构目标架构

## Rust

```text
src-tauri/src/
  app/
    mod.rs             # builder 与 managed state 组装
    startup.rs         # 启动恢复与后台任务
    tray.rs            # 托盘构建与菜单事件
    shortcuts.rs       # X11/Portal 快捷键注册与分发
    window_events.rs   # 主窗口和功能岛窗口生命周期
  platform/
    session.rs         # X11 / Wayland 探测
    x11.rs             # 活动窗口、焦点、XTest
    wayland.rs         # Portal 能力
  paste/
    mod.rs             # PasteBackend trait + coordinator
    x11.rs
    portal.rs
    copy_only.rs
  window/
    mod.rs             # WindowController
    geometry.rs        # work area 与 logical/physical 换算
    main.rs
  clipboard/           # watcher、写入、skip hash
  storage/
  capture/             # capture backend、session、commands
  pin/                 # manager、models、commands、cache
  ocr/
  translation/         # models、service、coordinator、secrets
  commands/            # 按 feature 暴露薄 IPC 层
```

## Frontend

```text
src/react/
  app/                 # 路由、主题、i18n、window shell
  features/
    clipboard/
    preview/
    capture/
    editor/
    pin/
    translation/
    settings/
  shared/
    ipc/
    components/
    hooks/
    types/
```

## 迁移策略

1. 先抽后端接口并保持旧前端可用。
2. Pin 是第一个 React 迁移岛，用于验证 WindowController、typed IPC 和资源管理。
3. 截图 Overlay 与现有 CaptureEditor 并行接入，稳定后删除旧入口。
4. 主 clipboard/preview 最后迁移；按行为测试逐项替换，不进行无验收依据的大爆炸重写。
5. 翻译领域层先在旧预览入口可调用，再切换到新 React 结果视图。

## 核心状态所有权

- Rust 拥有系统资源、窗口、授权会话、冻结帧、Pin 缓存、密钥和网络请求。
- React 拥有短期 UI 状态、选区交互、编辑对象和展示状态。
- IPC payload 使用稳定 ID 和 request ID；大图通过受控文件/object URL 传输。

## 当前落地（2026-08-11）

- 后端已形成 `app/`、`paste/`、`capture/`、`pin/`、`translation/` 和按职责拆分的 `commands/`；`window_controller.rs` 统一主窗口 work area 几何。
- `app/` 将 Tauri builder 保留为薄组装层，启动恢复、托盘、快捷键和窗口事件分别拥有独立模块。
- `paste/` 已进一步按协调器、Portal 状态机、X11 输入和 token 存储拆分；原始截图入口、平台 fallback 与测试也已分离。
- `clipboard_watcher` 已按主轮询、内容分类、写入重试和 tmux/inotify 监听拆分，tmux poll 使用安全封装而非业务代码中的 unsafe。
- 前端完成 typed `api.ts`/`ipc-types.ts`，Pin、Capture Overlay、Editor 为 React/TS 功能岛；主 clipboard 保持 vanilla，preview 已拆分调度器与五类 renderer。
- 当前未进行主窗口 React 大爆炸迁移，符合“保留可工作入口、按行为逐项替换”的迁移策略。
- 完整实现与状态所有权见 `docs/architecture.md`，验证边界见任务目录 `qa-matrix.md`。
