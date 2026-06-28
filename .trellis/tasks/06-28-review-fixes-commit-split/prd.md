# Review fixes and commit split

## Goal

修复最近两次提交 review 中发现的回归风险，并把本地领先提交重写为按功能分类的提交历史。

## Requirements

- 修复 `scripts/ci-local.sh` 在 `set -e` 下成功一步后提前退出的问题。
- 修复 pin 窗口误用截图 overlay layer-shell 全屏锚定逻辑的问题。
- 保留 pin 窗口创建后的置顶兜底。
- 移除未接入的预备抽象，避免无调用方代码进入历史。
- 重写 `1/dev..dev` 的两个本地提交，按功能分类重新提交。

## Acceptance Criteria

- [x] `./scripts/ci-local.sh --quick` 能完整执行并给出汇总。
- [x] `cargo check`、`cargo test`、`cargo clippy -- -D warnings` 通过。
- [x] `npm test`、`npx vite build` 通过。
- [x] `git log 1/dev..dev` 显示新的按功能分类提交。
- [x] 工作区最终干净。

## Out of Scope

- 不新增截图编辑器或翻译功能。
- 不修改已发布版本号。
