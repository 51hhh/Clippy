#!/usr/bin/env bash
# 在虚拟 X11 显示器中运行 DOM 入口 smoke 测试。
set -euo pipefail

if ! command -v xvfb-run >/dev/null 2>&1 || ! command -v xwininfo >/dev/null 2>&1; then
  printf '%s\n' "Xvfb smoke skipped: xvfb-run and xwininfo are required"
  exit 0
fi

if ! xvfb-run -a -s "-screen 0 1280x800x24" bash -c 'xwininfo -root >/dev/null' >/dev/null 2>&1; then
  printf '%s\n' "Xvfb smoke skipped: the sandbox could not connect to the virtual display"
  exit 0
fi

xvfb-run -a -s "-screen 0 1280x800x24" bash -c '
  set -euo pipefail
  cd src
  npx vitest run tests/entrypoints-smoke.test.js
'
