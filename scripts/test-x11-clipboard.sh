#!/usr/bin/env bash
# 仅合成数据与脚本新建的私有 Xvfb；不连接调用者桌面，也不启动 Clippy。
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "X11 剪贴板协议测试仅支持 Linux。" >&2
  exit 1
fi
for command_name in cargo xvfb-run Xvfb xauth xclip timeout; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "缺少 $command_name；需要 Rust、xvfb、xauth、xclip 和 coreutils。" >&2
    exit 1
  fi
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$REPO_ROOT/src-tauri/Cargo.toml"
UNIT_FILTER='platform::linux::x11::incr::tests::'

# 编译与纯状态测试不需要桌面。明确过滤掉上游会操作系统剪贴板的测试。
cargo test --locked --manifest-path "$MANIFEST" -p arboard --lib "$UNIT_FILTER"
cargo test --locked --manifest-path "$MANIFEST" -p clippy-app --test x11_clipboard_transfer --no-run
cargo test --locked --manifest-path "$MANIFEST" -p clippy-app --lib --no-run

# 移除桌面地址/认证及夹具角色变量，由 xvfb-run 分配自己的 display 和临时 Xauthority。
# 外层截止也覆盖夹具意外死锁；xvfb-run 负责退出时回收私有 server。
env -u DISPLAY -u WAYLAND_DISPLAY -u XAUTHORITY \
  -u CLIPPY_INCR_PROVIDER -u CLIPPY_TEST_X11_PROVIDER \
  XDG_SESSION_TYPE=x11 CLIPPY_TEST_X11_ISOLATED=1 \
  timeout --kill-after=10s 180s \
  xvfb-run -a --server-args='-screen 0 1280x800x24 -nolisten tcp' \
  bash -euo pipefail -c '
    cargo test --locked --manifest-path "$1" -p clippy-app --test x11_clipboard_transfer -- --ignored --nocapture --test-threads=1
    cargo test --locked --manifest-path "$1" -p arboard --lib platform::linux::x11::incr::tests::private_x11_rejects_oversized_property_before_reading_or_deleting_it -- --exact --ignored --nocapture --test-threads=1
    cargo test --locked --manifest-path "$1" -p clippy-app --lib clipboard_watcher::tests::x11_internal_writes_are_settled_before_read_and_external_recopies_are_kept -- --exact --ignored --nocapture --test-threads=1
  ' _ "$MANIFEST"
