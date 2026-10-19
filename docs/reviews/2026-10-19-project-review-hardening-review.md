# 项目审查安全与资源加固复审

- Requirement ID：`PROJECT-REVIEW-HARDENING-01`
- 日期：2026-10-19
- 范围：根目录 Clippy；不含独立项目 `fxxkDJTU/`
- 基线：`460c3f88e9aecc1bace3d62c5bff613206e5ec2c` 上的工作树改动

## 结论

本轮审查确认的可直接修复项已经闭环：外部 URL/favicon 不再绕过实际连接地址校验，tmux
不再覆盖后猜测恢复用户配置，敏感内容检测和去重标记变为单向安全，剪贴板列表 IPC 与正文入口
都有明确预算，重复的旧列表实现已删除，已知 npm/Rust vulnerability 已清零，workflow action
全部固定到可验证的提交 SHA。

这不是三平台发布批准。当前证据只证明 Linux 宿主的源码、自动化合同与本地 smoke；尤其是
Tauri/updater 依赖收敛后的 Windows/macOS 条件编译、安装器和真实桌面仍必须由同一提交的原生
CI/QA 证明。

## 发现与处置

| 风险 | 原问题 | 处置与合同 |
|---|---|---|
| 高 | URL 只做字符串阶段私网判断，DNS 重新解析和 favicon 外链可绕开同一安全边界 | ureq 自定义 resolver 校验实际交给 connector 的完整地址组；任一私网/保留地址即拒绝。关闭环境代理和重定向，限制全局并发 2、总超时 5 秒、HTML 1 MiB、URL 8 KiB；favicon 后端二次请求、64 KiB 上限、受限 raster MIME、data URL 输出，前端再验证类型与长度 |
| 高 | tmux 启用覆盖六个用户按键，禁用时写死“默认值”，命令失败被忽略，全局 hook 可能覆盖用户配置 | 首次启用前以 0600 文件保存精确 `list-keys` 指令；所有命令检查退出状态，应用失败回滚，禁用用 `source-file` 恢复。新实现不创建全局 hook，只按索引清理精确匹配的旧 Clippy hook；状态目录 0700，文件 owner/type/权限和 shell 引号均验证 |
| 高 | workflow 使用浮动 action tag；DOMPurify/source-map-js/undici 与 Rust quick-xml/rustls/zip 链路存在公告 | 三份 workflow 的全部 `uses:` 固定到 40 位 SHA，逐个远端 `action.yml` 返回 HTTP 200；Dependabot 覆盖 Actions/npm/Cargo。升级前端依赖；Rust 链路升级到修复版本，updater 临时固定到上游已切换 zip 9 的提交 `11211c30…` |
| 中 | 只识别文本开头的少数 token，重复内容可把敏感标记降回普通 | 检测嵌入式服务 token、JWT、Bearer、赋值/JSON、AWS secret 与常见私钥头；SQLite UPSERT 使用单向 `MAX`，已有敏感标记不会降级 |
| 中 | 文本/HTML 无入库上限，列表查询和事件携带完整正文；图片插入后又从 SQLite 物化一次 BLOB | 新内容单表示按 UTF-8 限制为 1 MiB；列表 SQL/事件只返回 2048 Unicode 字符摘要且不含 HTML/图片。HTML或真正截断的焦点文本才从 blocking worker 取详情，小文本不增加 IPC；图片详情和插入结果不物化 BLOB，原图继续走显式图片接口 |
| 中 | Vanilla 与 React 各有一套列表/行渲染代码和重复测试，实际入口只用 React，修复容易落错实现 | 删除 `src/js/clipboard-list.js`、`src/js/clipboard/row-renderer.js` 及其旧测试；当前文档和回归守卫只认 React store/row |

## 依赖审计

`npm audit --omit=dev --audit-level=low` 与 `npm audit --audit-level=high` 均为 0
vulnerability。`cargo-audit 0.22.2` 扫描当前 `Cargo.lock`（779 个 crate dependency）为 0
vulnerability，仍报告 10 个默认允许的 warning：

- unmaintained：`RUSTSEC-2024-0370`、`RUSTSEC-2026-0206`、`RUSTSEC-2026-0192`、
  `RUSTSEC-2026-0370`、`RUSTSEC-2026-0371`、`RUSTSEC-2026-0372`；
- unsound：`RUSTSEC-2026-0190`、`RUSTSEC-2026-0221`、`RUSTSEC-2024-0429`、
  `RUSTSEC-2026-0186`。

这些是仍在上游传递依赖中的未完成边界，当前 RustSec action 不以 warning 失败。本轮不通过
忽略 ID 或降低审计级别掩盖它们，也不把跨 GTK/渲染/Portal 依赖栈替换混入本修复。

## 验证证据

- `./scripts/ci-local.sh`：27 passed、0 failed、2 skipped；跳过非宿主平台交叉 lint 与
  AppImage X11 可视 smoke。
- `cargo test`：主测试 1341 passed、0 failed、14 ignored；隔离 X11 集成测试另由门禁显式执行并
  通过。URL、tmux、敏感内容、列表投影、归档 ZIP 9 兼容合同包含在全量结果中。
- `cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、
  `cargo fmt -- --check`：通过。
- 前端定向复核：preview race/renderers/regression guards 共 90 项通过；完整 Vitest 为
  1335 passed、0 failed、36 skipped，typecheck、lint、Vite build 与 DOM/Canvas/layout smoke
  包含在本地完整门禁中。
- `cargo audit --no-fetch`：0 vulnerability、10 allowed warning；两类 `npm audit`：0 vulnerability。
- 十个不同 GitHub Action 固定提交的远端 `action.yml` 均验证为 HTTP 200；文本回归守卫同时拒绝
  任何非 40 位 SHA 的 workflow `uses:`。

## 资源影响

本轮建立的是可证明的上限，不是新的 PSS 基准：新文本/HTML 单表示最多 1 MiB，列表正文最多
2048 字符，favicon 最多 64 KiB，URL 元数据并发最多 2；图片插入、列表、详情不再隐式复制
SQLite BLOB。删除未使用列表实现会减少维护面，但它原本未进入产品入口，不能把删除行数换算为
运行内存收益。没有重新启动真实桌面做 PSS/RSS 采样，因此不宣称百分比改善。

## 尚未验证与后续门槛

- 未运行同一提交的 `Check (ubuntu-22.04)`、`Native Check (windows-latest)`、
  `Native Check (macos-latest)`；当前工作树尚未形成可供远端验证的提交 SHA。
- 未执行 Windows/macOS 真实编译、安装、升级或卸载；Tauri 2.12/updater 固定提交尤其需要原生
  runner 复核。发布包、签名、公证和 AppImage 可视 smoke 均未计为通过。
- 未在用户真实 tmux server 上改写按键，也未对公网网站发起功能 QA；单元测试和 resolver
  合同不能替代真实配置/网络环境。
- 历史数据库中已经存在的超大正文不追溯删除；选中历史大条目仍可能按需读取，前端渲染预算
  继续负责最终显示上限。
- RustSec 的 10 个 warning 保持显式记录，需随上游可用修复由 Dependabot/后续专项收敛。
- 用户原有 `.gitignore` 修改和未跟踪的 `native-ci-evidence.md` 未被本任务改动或纳入结论。
