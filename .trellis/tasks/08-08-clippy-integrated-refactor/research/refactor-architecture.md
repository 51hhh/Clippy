# 重构目标架构

## Rust

```text
src-tauri/src/
  app/                 # builder、managed state、生命周期
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

