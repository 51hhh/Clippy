# RustSec 依赖安全刷新复审

- Requirement ID：`PROJECT-REVIEW-HARDENING-01`
- 日期：2026-11-24
- 范围：根 Cargo workspace；不含独立项目 `fxxkDJTU/`
- 触发基线：`67aa33ea8e50b72d23747ed02c11ef8aec3a573e` 的远程 CI

## 结论

最新 RustSec 数据库新增的 6 个 vulnerability 已通过锁文件升级清零。bundled SQLite 从
`libsqlite3-sys 0.28.0` 升到 `0.39.1`（SQLite 3.54.1），并由 `rusqlite 0.41.1` 接入；现有
SQLite/FTS、WAL、schema 迁移、归档与 Pin 工作区测试无需兼容分支即可通过。

原 `rustsec/audit-check 2.0.0` 在完成扫描后调用 GitHub Checks API 返回 404，导致主 job 失败并留下
queued 的 `Security audit` check。workflow 已改为安装固定的 `cargo-audit 0.22.2`，直接以审计进程
退出码作为门禁，不再申请 `checks: write`。这不会降低审计级别，也没有加入 advisory ignore。

## 漏洞与处置

| Advisory | 原依赖 | 修复后 |
|---|---|---|
| `RUSTSEC-2026-0594` | `cookie 0.18.1` | `0.18.3` |
| `RUSTSEC-2026-0459` | `libsqlite3-sys 0.28.0` / `rusqlite 0.31.0` | `0.39.1` / `0.41.1` |
| `RUSTSEC-2026-0382` | `rustls-webpki 0.103.15` | `0.103.17` |
| `RUSTSEC-2026-0592` | `url 2.5.8` | `2.5.9` |
| `RUSTSEC-2026-0383` | `webpki-root-certs 1.0.7` | `1.0.12` |
| `RUSTSEC-2026-0381` | `webpki-roots 1.0.7` | `1.0.12` |

同一轮还在既有 semver 约束内更新 anyhow、event-listener、rustix 和 Tokio，消除已有修复版本的
`RUSTSEC-2026-0190`、`RUSTSEC-2026-0221`、`RUSTSEC-2026-0486`、
`RUSTSEC-2026-0386` soundness 警告。没有为仍需跨框架或上游替换的警告进行大版本重构。

## 验证证据

- `cargo-audit 0.22.2`：扫描 997 个 crate dependency，0 vulnerability，13 allowed warning。
- `cargo check --workspace --all-targets`：通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- 存储定向测试：43 passed、0 failed，覆盖 SQLite/FTS、WAL 权限、schema 迁移、事务回滚、归档、
  图片修订和 Pin 工作区。
- workflow 回归合同：77 passed、0 failed，包含固定 audit 版本、直接退出码和最小权限检查。

## 保留边界

审计仍报告 10 个 unmaintained 实例、1 个 glib soundness warning 和 2 个 yanked 依赖，共 13 项。
它们由 Tauri/GTK/Slint/Portal 等上游依赖链带入，当前没有可在补丁版本内完成的统一替换；本次不通过
ignore 或降低 warning 级别隐藏。完整本地主机门禁与同一 SHA 的 Ubuntu/Windows/macOS 原生 CI
必须在提交后重新运行；未运行前不能把本次复审当作三平台发布批准。
