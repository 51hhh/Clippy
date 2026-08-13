#!/usr/bin/env bash
# 在真实浏览器 Canvas 中校验截图编辑器导出像素。
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
FIREFOX="$(command -v firefox || true)"
FFMPEG="$(command -v ffmpeg || true)"
if [[ -z "$FIREFOX" || -z "$FFMPEG" ]]; then
  printf '%s\n' 'Canvas export smoke skipped: firefox and ffmpeg are required'
  exit 0
fi

ARTIFACT_DIR="$(mktemp -d "${ROOT_DIR}/src-tauri/target/canvas-export-smoke.XXXXXX")"
PROFILE_DIR="${ARTIFACT_DIR}/firefox-profile"
SCREENSHOT="${ARTIFACT_DIR}/result.png"
VITE_LOG="${ARTIFACT_DIR}/vite.log"
FIREFOX_LOG="${ARTIFACT_DIR}/firefox.log"
mkdir -p "$PROFILE_DIR"
VITE_PID=""

cleanup() {
  if [[ -n "$VITE_PID" ]] && kill -0 "$VITE_PID" 2>/dev/null; then
    kill "$VITE_PID" 2>/dev/null || true
    wait "$VITE_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

find_port() {
  local port
  for port in $(seq 14520 14540); do
    if ! (exec 3<>"/dev/tcp/127.0.0.1/${port}") 2>/dev/null; then
      printf '%s\n' "$port"
      return 0
    fi
  done
  return 1
}

PORT="$(find_port)" || { printf '%s\n' 'Canvas export smoke failed: no local port' >&2; exit 1; }
(
  cd "${ROOT_DIR}/src"
  exec npx vite --host 127.0.0.1 --port "$PORT" --strictPort
) >"$VITE_LOG" 2>&1 &
VITE_PID=$!

for _ in {1..50}; do
  kill -0 "$VITE_PID" 2>/dev/null || {
    printf 'Canvas export smoke failed: Vite exited; log: %s\n' "$VITE_LOG" >&2
    exit 1
  }
  if (exec 3<>"/dev/tcp/127.0.0.1/${PORT}") 2>/dev/null; then
    break
  fi
  sleep 0.1
done

HOME="$PROFILE_DIR" timeout 20 "$FIREFOX" --headless \
  --window-size 320,240 --screenshot "$SCREENSHOT" \
  "http://127.0.0.1:${PORT}/tests/fixtures/canvas-export-smoke.html" \
  >"$FIREFOX_LOG" 2>&1 || {
  printf 'Canvas export smoke failed: Firefox failed; artifacts: %s\n' "$ARTIFACT_DIR" >&2
  exit 1
}

[[ -s "$SCREENSHOT" ]] || {
  printf 'Canvas export smoke failed: screenshot is missing; artifacts: %s\n' "$ARTIFACT_DIR" >&2
  exit 1
}
PIXEL="$($FFMPEG -v error -i "$SCREENSHOT" -vf 'crop=1:1:160:120,format=rgb24' -f rawvideo - 2>/dev/null | od -An -tu1 -N3)"
read -r RED GREEN BLUE <<<"$PIXEL"
if (( GREEN < 180 || RED > 40 || BLUE > 40 )); then
  printf 'Canvas export smoke failed: browser pixel check failed (%s); artifacts: %s\n' "$PIXEL" "$ARTIFACT_DIR" >&2
  exit 1
fi

printf 'Canvas export smoke passed; pixel=%s artifacts=%s\n' "$PIXEL" "$ARTIFACT_DIR"
