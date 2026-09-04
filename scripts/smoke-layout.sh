#!/usr/bin/env bash
# 在真实浏览器里校验主窗口预览/翻译区的布局几何（jsdom 没有布局引擎，量不出遮挡）。
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
FIREFOX="$(command -v firefox || true)"
FFMPEG="$(command -v ffmpeg || true)"
# 读取截图像素：优先 ffmpeg，其次 python3 的 Pillow，都没有才跳过。
PYTHON_PIL=""
if [[ -z "$FFMPEG" ]] && command -v python3 >/dev/null; then
  python3 -c 'import PIL.Image' 2>/dev/null && PYTHON_PIL="$(command -v python3)"
fi
if [[ -z "$FIREFOX" || ( -z "$FFMPEG" && -z "$PYTHON_PIL" ) ]]; then
  printf '%s\n' 'Layout smoke skipped: firefox plus ffmpeg or python3-pil are required'
  exit 0
fi

mkdir -p "${ROOT_DIR}/src-tauri/target"
ARTIFACT_DIR="$(mktemp -d "${ROOT_DIR}/src-tauri/target/layout-smoke.XXXXXX")"
PROFILE_DIR="${ARTIFACT_DIR}/firefox-profile"
SCREENSHOT="${ARTIFACT_DIR}/result.png"
VITE_LOG="${ARTIFACT_DIR}/vite.log"
FIREFOX_LOG="${ARTIFACT_DIR}/firefox.log"
mkdir -p "$PROFILE_DIR"
VITE_PID=""
SELF_PGID="$(ps -o pgid= -p "$$" | tr -d ' ')"

# npx 在 vite 之上还套着 npm exec / sh / node 三层，只 kill 顶层会留下孤儿 dev server
# 常驻端口（本地一度攒了十几个）。整个进程组一起收，PGID 由下面的 setsid 保证不是本脚本自己的组。
cleanup() {
  [[ -n "$VITE_PID" ]] || return 0
  local pgid
  pgid="$(ps -o pgid= -p "$VITE_PID" 2>/dev/null | tr -d ' ')"
  if [[ -n "$pgid" && "$pgid" != "$SELF_PGID" ]]; then
    kill -- "-${pgid}" 2>/dev/null || true
  else
    kill "$VITE_PID" 2>/dev/null || true
  fi
  wait "$VITE_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

find_port() {
  local port
  for port in $(seq 14541 14560); do
    if ! (exec 3<>"/dev/tcp/127.0.0.1/${port}") 2>/dev/null; then
      printf '%s\n' "$port"
      return 0
    fi
  done
  return 1
}

PORT="$(find_port)" || { printf '%s\n' 'Layout smoke failed: no local port' >&2; exit 1; }
SETSID="$(command -v setsid || true)"
$SETSID bash -c "cd '${ROOT_DIR}/src' && exec npx vite --host 127.0.0.1 --port '${PORT}' --strictPort" \
  >"$VITE_LOG" 2>&1 &
VITE_PID=$!

for _ in {1..50}; do
  kill -0 "$VITE_PID" 2>/dev/null || {
    printf 'Layout smoke failed: Vite exited; log: %s\n' "$VITE_LOG" >&2
    exit 1
  }
  if (exec 3<>"/dev/tcp/127.0.0.1/${PORT}") 2>/dev/null; then
    break
  fi
  sleep 0.1
done

# 780x500 = 预览展开时 window_controller 给出的逻辑尺寸（380 列表 + 400 面板，高度恒定 500）
HOME="$PROFILE_DIR" timeout 25 "$FIREFOX" --headless \
  --window-size 780,500 --screenshot "$SCREENSHOT" \
  "http://127.0.0.1:${PORT}/tests/fixtures/layout-smoke.html" \
  >"$FIREFOX_LOG" 2>&1 || {
  printf 'Layout smoke failed: Firefox failed; artifacts: %s\n' "$ARTIFACT_DIR" >&2
  exit 1
}

[[ -s "$SCREENSHOT" ]] || {
  printf 'Layout smoke failed: screenshot is missing; artifacts: %s\n' "$ARTIFACT_DIR" >&2
  exit 1
}
if [[ -n "$FFMPEG" ]]; then
  PIXEL="$($FFMPEG -v error -i "$SCREENSHOT" -vf 'crop=1:1:390:310,format=rgb24' -f rawvideo - 2>/dev/null | od -An -tu1 -N3)"
else
  PIXEL="$("$PYTHON_PIL" -c '
import sys
from PIL import Image
print(" ".join(str(value) for value in Image.open(sys.argv[1]).convert("RGB").getpixel((390, 310))))
' "$SCREENSHOT")"
fi
read -r RED GREEN BLUE <<<"$PIXEL"
if (( GREEN < 180 || RED > 40 || BLUE > 40 )); then
  printf 'Layout smoke failed: geometry assertions failed (%s); artifacts: %s\n' "$PIXEL" "$ARTIFACT_DIR" >&2
  exit 1
fi

printf 'Layout smoke passed; pixel=%s artifacts=%s\n' "$PIXEL" "$ARTIFACT_DIR"
