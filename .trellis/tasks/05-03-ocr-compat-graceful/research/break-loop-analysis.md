# Bug 分析：OCR 动态链接导致跨发行版启动失败

## 1. 根因类别
- **类别**: E — 隐式假设 (Implicit Assumption)
- **具体原因**: `leptess = "0.14"` 引入了对 `tesseract-sys` 的编译时依赖。`tesseract-sys` 的 `build.rs` 通过 `pkg-config` 检测系统 tesseract 并 `cargo:rustc-link-lib=tesseract`（无 static/dylib 前缀），由链接器自行决定动态还是静态链接。CI (Ubuntu 22.04) 上链接了 `libtesseract.so.4` + `liblept.so.5`，而目标用户系统 (Ubuntu 24.04/26.04) 只有 `libtesseract.so.5` + `libleptonica.so.6`。
- **隐式假设**: "编译环境的动态库 SONAME 与部署环境一致"

## 2. 修复过程
- **第一次尝试（表面修复）**: 将 CI runner 升级到 ubuntu-24.04 — 解决了 SONAME 版本问题，但仍是编译时硬链接，未来仍可能断裂
- **最终修复（根因修复）**: 移除 `leptess` crate，改用 `std::process::Command` 调用系统 `tesseract` CLI — 完全消除编译时动态链接依赖，运行时按需检测

## 3. 预防机制

| 类型 | 措施 | 状态 |
|------|------|------|
| **架构** | 系统工具通过 CLI 子进程调用而非 -sys crate 链接 | ✅ 已实施 |
| **运行时** | `ocr::is_available()` 检查 + 前端友好提示 | ✅ 已实施 |
| **文档** | Release Notes 中标注 OCR 需额外安装 tesseract | ✅ 已实施 |
| **CI** | `readelf -d` 检查 NEEDED 列表无意外动态库 | 建议 |
| **Spec** | 新增"外部工具集成"规范：优先 CLI 调用 > -sys crate | 建议 |

## 4. 系统性扩展

### 类似问题
- 当前无其他 `-sys` crate（`rusqlite` 用 `bundled` feature 静态编译 SQLite）
- 未来如需 PDF 渲染、图片处理等外部能力，应遵循同一模式

### 设计缺陷
- Rust 生态的 `-sys` crate 默认动态链接系统库，这在跨发行版分发时是定时炸弹
- 正确做法：a) 使用 `bundled` feature（如 rusqlite）或 b) 改用 CLI 子进程

### 流程改进
- CI release 阶段增加一步 `readelf -d` 审计，确认无意外 NEEDED 条目
- 引入新依赖时强制检查：是否引入了动态链接？是否有 bundled 选项？

## 5. 知识固化

### Spec 更新建议

新增 `.trellis/spec/backend/external-tools.md`：

```
# 外部工具集成规范

## 原则
- 优先通过 CLI 子进程调用系统工具（如 tesseract、ffmpeg）
- 避免使用 -sys crate 动态链接系统库（跨发行版 SONAME 不兼容）
- 唯一例外：有 `bundled` feature 的 crate（如 rusqlite）

## 模式
1. 运行时检测工具是否可用：`Command::new("tool").arg("--version").status()`
2. 不可用时返回友好错误，不影响应用其他功能
3. deb depends 不硬依赖可选工具，通过应用内提示引导安装

## 检查清单
- [ ] `readelf -d` 确认无意外动态链接
- [ ] 工具缺失时应用仍可启动
- [ ] 前端有不可用提示 + 安装指引
```
