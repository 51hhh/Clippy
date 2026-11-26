#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

if [[ $# -eq 0 ]]; then
  cargo build --locked -p clippy-slint --bin clippy-slint-qa --features qa-build
  set -- "${CARGO_TARGET_DIR:-target}/debug/clippy-slint-qa"
fi

runtime_dir="$(mktemp -d)"
trap 'rm -rf -- "$runtime_dir"' EXIT
run_fixture() {
  env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET -u XAUTHORITY -u DBUS_SESSION_BUS_ADDRESS \
    -u XDG_CURRENT_DESKTOP XDG_RUNTIME_DIR="$runtime_dir" XDG_SESSION_TYPE=x11 \
    GDK_BACKEND=x11 SLINT_BACKEND=winit-femtovg NO_AT_BRIDGE=1 \
    xvfb-run -a -s '-screen 0 1280x800x24 -nolisten tcp' \
    dbus-run-session -- timeout --kill-after=5s 90s /usr/bin/python3 "$@"
}

run_fixture scripts/qa/slint-x11.py "$1"
run_fixture scripts/qa/slint-longshot-x11.py "$1"
SLINT_SCALE_FACTOR=1.25 run_fixture scripts/qa/slint-window-pick-x11.py "$1"
