# Verification — 2026-09-04

## Automated gates

- `./scripts/ci-local.sh --quick`：11 项通过、0 失败、1 项按 `--quick` 规则跳过。
  - `cargo fmt --check`
  - `cargo check --all-targets`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test`：455 passed、0 failed、10 ignored（真实桌面/剪贴板诊断探针）
  - GNOME 扩展静态检查
  - `npm ci`
  - `npx tsc --noEmit`
  - `npx vitest run`：47 files、862 tests passed
  - DOM/Xvfb smoke：9 tests passed
  - Canvas 导出像素 smoke：通过，像素 `0 208 0`
  - 主窗口布局像素 smoke：通过，像素 `0 208 0`
- `cd src && npx vite build`：生产构建通过，1890 modules transformed。
- `git diff --check`：通过。

## Acceptance coverage

- 异步预览：覆盖延迟图片成功/失败/onload、OCR、配置、复制、HTML 详情失败、首次库加载和 A→B→A 取消切换。
- OCR：覆盖成功/失败路径缓存与失效、探测期间失效、持久缓存短路、同 clip single-flight，以及 keyed/unkeyed 全局单并发。
- 可编辑 PNG：覆盖有效工程二次编辑、原图/预览分离、普通/损坏/未来工程拒绝、超限/不可读文件和建窗失败回滚。
- 剪贴板图片：覆盖 4K/8K 原尺寸、像素预算边界、溢出/尺寸/RGBA 长度拒绝，以及异常状态后的两级去重失效。
- 侧栏布局：真实浏览器像素 smoke 证明图片与 OCR 位于同一外层滚动区域；常态主界面没有打开图片按钮或 `Ctrl/Cmd+O`。

## Manual follow-up

- 拖放与 Pin 原生建窗仍需在 Linux、Windows、macOS 的发布包上按 `docs/native-qa.md` 执行一次真实窗口验证。
- 4K renderer 和真实桌面截图性能探针按仓库约定保持 ignored，不在普通 CI 中运行。
