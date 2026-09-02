#!/usr/bin/env bash
# 在隔离的 X11/DBus 会话中启动最终 AppImage，检查主窗口的几何、装饰和首帧。
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
BUNDLE_DIR="${ROOT_DIR}/src-tauri/target/release/bundle/appimage"
APPIMAGE="${1:-}"
SMOKE_REQUIRED="${CLIPPY_APPIMAGE_SMOKE_REQUIRED:-0}"

skip_or_fail() {
    local reason="$1"
    if [[ "$SMOKE_REQUIRED" == "1" ]]; then
        printf 'AppImage X11 smoke failed: %s\n' "$reason" >&2
        exit 1
    fi
    printf 'AppImage X11 smoke skipped: %s\n' "$reason"
    exit 0
}

if [[ "${APPIMAGE}" == "--help" || "${APPIMAGE}" == "-h" ]]; then
    printf '用法: %s [Clippy_*_amd64.AppImage]\n' "$0"
    exit 0
fi
if [[ -z "$APPIMAGE" ]]; then
    APPIMAGE="$(find "$BUNDLE_DIR" -maxdepth 1 -type f -name '*_amd64.AppImage' -print 2>/dev/null | sort | tail -n 1 || true)"
fi
if [[ -z "$APPIMAGE" || ! -f "$APPIMAGE" ]]; then
    skip_or_fail "release AppImage not found"
fi

for command_name in xvfb-run xauth xwininfo xprop ffmpeg dbus-run-session unsquashfs busctl; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        skip_or_fail "required command is unavailable: ${command_name}"
    fi
done
if [[ ! -x "$APPIMAGE" ]]; then
    printf 'AppImage X11 smoke failed: AppImage is not executable: %s\n' "$APPIMAGE" >&2
    exit 1
fi

ARTIFACT_DIR="$(mktemp -d "${ROOT_DIR}/src-tauri/target/appimage-x11-smoke.XXXXXX")"
RUN_LOG="${ARTIFACT_DIR}/runner.log"
mkdir -p "$ARTIFACT_DIR"/{home,runtime,config,data,cache,tmp}
chmod 700 "$ARTIFACT_DIR/runtime"
unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET

set +e
xvfb-run -a -s '-screen 0 1280x800x24' \
    env XDG_SESSION_TYPE=x11 WAYLAND_DISPLAY= WAYLAND_SOCKET= \
    HOME="${ARTIFACT_DIR}/home" XDG_RUNTIME_DIR="${ARTIFACT_DIR}/runtime" \
    XDG_CONFIG_HOME="${ARTIFACT_DIR}/config" XDG_DATA_HOME="${ARTIFACT_DIR}/data" \
    XDG_CACHE_HOME="${ARTIFACT_DIR}/cache" TMPDIR="${ARTIFACT_DIR}/tmp" \
    APPIMAGE_EXTRACT_AND_RUN=1 GDK_BACKEND=x11 LIBGL_ALWAYS_SOFTWARE=1 \
    WEBKIT_DISABLE_DMABUF_RENDERER=1 dbus-run-session -- bash -s -- "$APPIMAGE" "$ARTIFACT_DIR" \
    >"$RUN_LOG" 2>&1 <<'INNER_SMOKE'
set -euo pipefail
APPIMAGE="$1"
ARTIFACT_DIR="$2"
EXTRACT_DIR="$ARTIFACT_DIR/appdir"
SQUASH_OFFSET=""
while IFS=: read -r offset magic; do
    if unsquashfs -s -o "$offset" "$APPIMAGE" >/dev/null 2>&1; then
        SQUASH_OFFSET="$offset"
        break
    fi
done < <(grep -abo 'hsqs' "$APPIMAGE" || true)
[[ -n "$SQUASH_OFFSET" ]] || { printf 'valid SquashFS payload was not found\n' >&2; exit 1; }
unsquashfs -quiet -o "$SQUASH_OFFSET" -d "$EXTRACT_DIR" "$APPIMAGE"
APP_RUN="$EXTRACT_DIR/AppRun"
[[ -x "$APP_RUN" ]] || { printf 'extracted AppRun is missing or not executable\n' >&2; exit 1; }
FIRST_LOG="$ARTIFACT_DIR/first-instance.log"
CALL_LOG="$ARTIFACT_DIR/single-instance-call.log"
WINDOW_INFO="$ARTIFACT_DIR/window-info.txt"
XPROP_INFO="$ARTIFACT_DIR/window-xprop.txt"
SCREENSHOT="$ARTIFACT_DIR/main-window.png"
SIGNALSTATS="$ARTIFACT_DIR/signalstats.txt"
FFMPEG_LOG="$ARTIFACT_DIR/ffmpeg.log"
FIRST_PID=""

cleanup() {
    set +e
    for pid in "$FIRST_PID"; do
        if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null
            for _ in {1..20}; do
                kill -0 "$pid" 2>/dev/null || break
                sleep 0.1
            done
            kill -KILL "$pid" 2>/dev/null
        fi
    done
    wait "$FIRST_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

export XDG_SESSION_TYPE=x11
unset WAYLAND_DISPLAY WAYLAND_SOCKET
"$APP_RUN" >"$FIRST_LOG" 2>&1 &
FIRST_PID=$!
for _ in {1..20}; do
    kill -0 "$FIRST_PID" 2>/dev/null || { printf 'first AppImage exited early\n' >&2; exit 1; }
    if xwininfo -root -tree 2>/dev/null | grep -qi 'Clippy'; then
        break
    fi
    sleep 0.25
done

# 通过插件自己的 D-Bus 接口触发真实 on_second_instance，避免 AppImageLauncher 劫持第二进程。
for _ in {1..20}; do
    if busctl --user list 2>/dev/null | awk '{print $1}' | grep -qx 'com.clippy.app.SingleInstance'; then
        break
    fi
    sleep 0.25
done
busctl --user list 2>/dev/null | awk '{print $1}' | grep -qx 'com.clippy.app.SingleInstance' || {
    printf 'single-instance D-Bus name was not registered\n' >&2
    exit 1
}
busctl --user call com.clippy.app.SingleInstance \
    /com/clippy/app/SingleInstance org.SingleInstance.DBus ExecuteCallback \
    'ass' 1 clippy-smoke "$PWD" >"$CALL_LOG" 2>&1 || {
    printf 'single-instance callback failed\n' >&2
    exit 1
}

find_main_window() {
    while read -r window_id; do
        [[ -n "$window_id" ]] || continue
        local properties
        properties="$(xprop -id "$window_id" _NET_WM_NAME WM_NAME WM_CLASS 2>/dev/null || true)"
        grep -Eq '(_NET_WM_NAME|WM_NAME).*"Clippy"' <<<"$properties" || continue
        grep -qi 'clippy-app' <<<"$properties" || continue
        xwininfo -id "$window_id" 2>/dev/null | grep -q 'Map State: IsViewable' || continue
        printf '%s\n' "$window_id"
        return 0
    done < <(xwininfo -root -tree 2>/dev/null | awk '/"Clippy"/ { print $1 }')
    return 1
}

WINDOW_ID=""
for _ in {1..120}; do
    WINDOW_ID="$(find_main_window || true)"
    [[ -n "$WINDOW_ID" ]] && break
    sleep 0.25
done
[[ -n "$WINDOW_ID" ]] || { printf 'visible Clippy main window was not found\n' >&2; exit 1; }
xwininfo -id "$WINDOW_ID" >"$WINDOW_INFO"
xprop -id "$WINDOW_ID" WM_NORMAL_HINTS _MOTIF_WM_HINTS _NET_WM_NAME WM_NAME WM_CLASS >"$XPROP_INFO"
WINDOW_X="$(awk '/Absolute upper-left X:/ { print $4; exit }' "$WINDOW_INFO")"
WINDOW_Y="$(awk '/Absolute upper-left Y:/ { print $4; exit }' "$WINDOW_INFO")"
WINDOW_WIDTH="$(awk '/Width:/ { print $2; exit }' "$WINDOW_INFO")"
WINDOW_HEIGHT="$(awk '/Height:/ { print $2; exit }' "$WINDOW_INFO")"
[[ -n "$WINDOW_X" && -n "$WINDOW_Y" && -n "$WINDOW_WIDTH" && -n "$WINDOW_HEIGHT" ]] || {
    printf 'failed to read main window geometry\n' >&2
    exit 1
}
(( WINDOW_WIDTH == 380 && WINDOW_HEIGHT == 500 )) || {
    printf 'unexpected main window size: %sx%s (expected 380x500)\n' "$WINDOW_WIDTH" "$WINDOW_HEIGHT" >&2
    exit 1
}
(( WINDOW_X >= 0 && WINDOW_Y >= 0 && WINDOW_X + WINDOW_WIDTH <= 1280 && WINDOW_Y + WINDOW_HEIGHT <= 800 )) || {
    printf 'main window is outside the 1280x800 virtual screen\n' >&2
    exit 1
}
grep -Eq 'minimum size: 380 by 500' "$XPROP_INFO" || { printf 'missing WM_NORMAL_HINTS minimum size\n' >&2; exit 1; }
grep -Eq 'maximum size: 380 by 500' "$XPROP_INFO" || { printf 'missing WM_NORMAL_HINTS maximum size\n' >&2; exit 1; }
MOTIF_HINTS="$(grep '_MOTIF_WM_HINTS' "$XPROP_INFO" || true)"
if [[ -n "$MOTIF_HINTS" ]]; then
    DECORATIONS="$(sed -E 's/.*= *[^,]+, *[^,]+, *([^,]+),.*/\1/' <<<"$MOTIF_HINTS" | tr -d '[:space:]')"
    [[ "$DECORATIONS" == "0" || "$DECORATIONS" == "0x0" ]] || {
        printf 'Motif decorations are enabled: %s\n' "$MOTIF_HINTS" >&2
        exit 1
    }
else
    grep -qi 'Border width: 0' "$WINDOW_INFO" || { printf 'window has no no-decoration hint\n' >&2; exit 1; }
fi

# 直接按窗口 ID 捕获，避免 root 截图混入其他进程的像素。
ffmpeg -hide_banner -loglevel warning -f x11grab -window_id "$WINDOW_ID" \
    -framerate 1 -draw_mouse 0 -i "$DISPLAY" \
    -vf "signalstats,metadata=print:file=${SIGNALSTATS}" -frames:v 1 -y "$SCREENSHOT" \
    >"$FFMPEG_LOG" 2>&1
awk -F= '
    /lavfi.signalstats.YMIN=/ { ymin = $2 }
    /lavfi.signalstats.YMAX=/ { ymax = $2 }
    /lavfi.signalstats.YAVG=/ { yavg = $2 }
    END { if (ymin == "" || ymax == "" || yavg == "" || ymin >= ymax || yavg <= 5 || yavg >= 250) exit 1 }
' "$SIGNALSTATS" || { printf 'main window frame is blank, single-color, or outside brightness range\n' >&2; exit 1; }
printf 'AppImage X11 smoke passed: window=%s geometry=%sx%s+%s+%s artifacts=%s\n' \
    "$WINDOW_ID" "$WINDOW_WIDTH" "$WINDOW_HEIGHT" "$WINDOW_X" "$WINDOW_Y" "$ARTIFACT_DIR"
INNER_SMOKE
STATUS=$?
set -e
if [[ "$STATUS" -ne 0 ]]; then
    printf 'AppImage X11 smoke failed; artifacts: %s\n' "$ARTIFACT_DIR" >&2
    exit "$STATUS"
fi
printf 'AppImage X11 smoke passed; artifacts: %s\n' "$ARTIFACT_DIR"
