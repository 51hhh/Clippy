#!/usr/bin/env bash
# 本地质量预检脚本 — 提交前运行
#
# 只验证当前宿主平台。本脚本通过不等于三平台 CI 通过：cargo 只编译当前 target，
# `#[cfg(target_os = ...)]` 挡住的 Windows / macOS 代码和 vendor 目录里的对应平台文件
# 在这里根本不进编译图，其告警与测试也就无从触发。发布门禁仍以 build.yml 的
# Check (ubuntu) + Native Check (windows-latest) + Native Check (macos-latest) 为准。
#
# 用法: ./scripts/ci-local.sh [--quick]
#   --quick: 跳过构建检查，仅运行 lint/test
#
# 可选环境变量:
#   CLIPPY_CROSS_CHECK=1   额外跑非宿主平台的交叉 lint（见下方步骤）。只能发现"编译期"
#                          问题，发现不了只在目标平台运行时才失败的断言，因此在任何情况下
#                          都不能替代真实 Windows / macOS runner。

set -euo pipefail

QUICK=false
[[ "${1:-}" == "--quick" ]] && QUICK=true

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

PASS=0
FAIL=0
SKIP=0

run_step() {
  local name="$1"
  shift
  printf "${YELLOW}▸ %s${NC}\n" "$name"
  if "$@"; then
    printf "${GREEN}  ✓ %s${NC}\n" "$name"
    PASS=$((PASS + 1))
  else
    printf "${RED}  ✗ %s${NC}\n" "$name"
    FAIL=$((FAIL + 1))
  fi
}

skip_step() {
  local name="$1"
  printf "${YELLOW}▸ %s (skipped)${NC}\n" "$name"
  SKIP=$((SKIP + 1))
}

# 缺依赖要在第一步就明确报错，而不是让某个步骤在中途以难以归因的方式失败。
MISSING_COMMANDS=()

require_cmd() {
  local command_name="$1"
  local hint="$2"
  if ! command -v "$command_name" >/dev/null 2>&1; then
    MISSING_COMMANDS+=("$command_name — $hint")
  fi
}

check_prerequisites() {
  require_cmd cargo "Rust toolchain: https://rustup.rs"
  require_cmd node "Node.js >= 22.12: https://nodejs.org"
  require_cmd npm "Node.js >= 20.19: https://nodejs.org"
  require_cmd npx "随 Node.js 一同安装"
  require_cmd python3 "Python 3（OCR 质量合同单元测试）"
  # DOM/Canvas smoke 在无头环境下依赖 Xvfb，缺失时整条前端 smoke 都无法执行。
  require_cmd xvfb-run "sudo apt install -y xvfb"
  if [[ "$(uname -s)" == "Linux" ]]; then
    require_cmd xauth "sudo apt install -y xauth"
    require_cmd xclip "sudo apt install -y xclip"
    require_cmd timeout "sudo apt install -y coreutils"
  fi

  if [[ ${#MISSING_COMMANDS[@]} -gt 0 ]]; then
    printf "${RED}缺少以下依赖，无法运行本地门禁：${NC}\n"
    local entry
    for entry in "${MISSING_COMMANDS[@]}"; do
      printf "${RED}  - %s${NC}\n" "$entry"
    done
    printf "完整环境搭建步骤见 CLAUDE.md「开发环境搭建」。\n"
    exit 1
  fi
}

HOST_PLATFORM="$(uname -s) $(uname -m)"

echo "=========================================="
echo " Clippy 本地质量预检"
echo "=========================================="
printf "${YELLOW} 仅验证当前宿主平台：%s${NC}\n" "$HOST_PLATFORM"
printf "${YELLOW} Windows / macOS 的条件编译代码不在本机编译图内，通过此门禁不代表三平台 CI 通过。${NC}\n"
echo ""

check_prerequisites

# --- OCR quality contract (pure stdlib; no model or third-party wheel) ---
run_step "OCR 质量合同" \
  python3 -m unittest discover -s src-tauri/ocr-sidecar -p 'test_quality*.py' -v
run_step "OCR 视觉段落回退" \
  python3 -m unittest discover -s src-tauri/ocr-sidecar -p test_visual_paragraphs.py -v
run_step "智能擦除可行性证据" \
  python3 scripts/smart-erase/verify_evidence.py

# --- Rust ---
run_step "cargo fmt --check" bash -c "cd src-tauri && cargo fmt -- --check"
run_step "vendor/xcap fmt --check" \
  bash -c "cd src-tauri && cargo fmt --manifest-path vendor/xcap/Cargo.toml -- --check"
run_step "cargo check" bash -c "cd src-tauri && cargo check --all-targets"
run_step "cargo clippy" bash -c "cd src-tauri && cargo clippy --all-targets -- -D warnings"
run_step "cargo test" bash -c "cd src-tauri && cargo test"
if [[ "$(uname -s)" == "Linux" ]]; then
  run_step "X11 录屏到可恢复 AVI 闭环" \
    bash -c "cd src-tauri && xvfb-run -a cargo test recording::session::tests::x11_source_records_a_complete_private_avi_session -- --ignored --exact"
  run_step "X11 剪贴板隔离协议回归" ./scripts/test-x11-clipboard.sh
else
  skip_step "X11 录屏到可恢复 AVI 闭环 (仅 Linux)"
  skip_step "X11 剪贴板隔离协议回归 (仅 Linux)"
fi

# 可选交叉 lint：只覆盖"能不能编译过"，不执行任何测试。
#
# vendor/arboard 是 [patch.crates-io] 的无条件覆盖，三平台都编译它，但它的 macOS / Wayland
# 源码在 Linux 上根本不进编译图——2026-09-16 的 macOS 门禁就是这样被 19 个弃用 / unused_unsafe
# 告警打红的。这个 crate 的 macOS 依赖全是纯 Rust 的 objc2 绑定，`rustup target add` 之后即可
# 交叉 lint，成本几秒，值得作为附加检查。
if [[ "${CLIPPY_CROSS_CHECK:-0}" == "1" ]]; then
  if rustup target list --installed 2>/dev/null | grep -qx 'aarch64-apple-darwin'; then
    run_step "交叉 clippy: vendor/arboard @ aarch64-apple-darwin" \
      bash -c "cd src-tauri && cargo clippy -p arboard --target aarch64-apple-darwin --all-targets -- -D warnings"
  else
    skip_step "交叉 clippy: vendor/arboard @ aarch64-apple-darwin — 先 rustup target add aarch64-apple-darwin"
  fi

  # 主 crate 的 Windows 交叉 lint 需要 MSVC 兼容工具链：依赖里的 C build script 会调用
  # lib.exe，只装 rustup target 会在 cc-rs 阶段失败，与代码无关。有 cargo-xwin 或
  # VCINSTALLDIR 时才尝试。
  if rustup target list --installed 2>/dev/null | grep -qx 'x86_64-pc-windows-msvc'; then
    run_step "交叉 clippy: vendor/xcap WGC @ x86_64-pc-windows-msvc" \
      bash -c "cd src-tauri && cargo clippy --manifest-path vendor/xcap/Cargo.toml --lib --features wgc --target x86_64-pc-windows-msvc -- -D warnings"
    if command -v cargo-xwin >/dev/null 2>&1 || [[ -n "${VCINSTALLDIR:-}" ]]; then
      run_step "交叉 clippy: clippy-app @ x86_64-pc-windows-msvc" \
        bash -c "cd src-tauri && cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings"
    else
      skip_step "交叉 clippy: clippy-app @ x86_64-pc-windows-msvc — 需 MSVC 工具链（cargo-xwin 或 VCINSTALLDIR）"
    fi
  else
    skip_step "交叉 clippy: vendor/xcap WGC @ x86_64-pc-windows-msvc — 先 rustup target add x86_64-pc-windows-msvc"
    skip_step "交叉 clippy: clippy-app @ x86_64-pc-windows-msvc — 先安装 target 和 MSVC 工具链"
  fi
else
  skip_step "非宿主平台交叉 lint (设置 CLIPPY_CROSS_CHECK=1 启用)"
fi

# --- GNOME Shell 扩展 ---
run_step "GNOME 扩展静态检查" ./scripts/check-gnome-extension.sh
run_step "前端 HTML 与 Tauri 边界" node scripts/check-html-sinks.mjs
run_step "IPC 合同一致性" node scripts/check-ipc-contract.mjs
run_step "录屏编码基准脚本语法" node --check scripts/benchmark-recording-encoders.mjs
run_step "录屏编码供应链" node scripts/verify-recording-codec-supply-chain.mjs
run_step "Windows WGC 光标补丁" node scripts/verify-xcap-patch.mjs

# --- Frontend ---
run_step "npm ci" bash -c "cd src && npm ci --prefer-offline"
run_step "vanilla JS 静态检查" bash -c "cd src && npm run lint:js"
run_step "typecheck" bash -c "cd src && npx tsc --noEmit"
run_step "vitest" bash -c "cd src && npx vitest run"
run_step "DOM/Xvfb smoke" ./scripts/smoke-dom.sh
run_step "Canvas 导出像素 smoke" ./scripts/smoke-canvas-export.sh
run_step "主窗口布局像素 smoke" ./scripts/smoke-layout.sh

if [[ "$QUICK" == false ]]; then
  run_step "vite build" bash -c "cd src && npx vite build"
  run_step "built main entry" node scripts/check-built-main.mjs
  if [[ "${CLIPPY_APPIMAGE_SMOKE:-0}" == "1" ]]; then
    if [[ -n "${CLIPPY_APPIMAGE_PATH:-}" ]]; then
      run_step "AppImage X11 可视 smoke" ./scripts/smoke-appimage-x11.sh "${CLIPPY_APPIMAGE_PATH}"
    else
      run_step "AppImage X11 可视 smoke" ./scripts/smoke-appimage-x11.sh
    fi
  else
    skip_step "AppImage X11 可视 smoke (设置 CLIPPY_APPIMAGE_SMOKE=1 启用)"
  fi
else
  skip_step "vite build"
fi

# --- Summary ---
echo ""
echo "=========================================="
printf " 结果: ${GREEN}%d 通过${NC}, ${RED}%d 失败${NC}, ${YELLOW}%d 跳过${NC}\n" "$PASS" "$FAIL" "$SKIP"
printf " 覆盖范围: 仅 %s；跳过的步骤不计为通过。\n" "$HOST_PLATFORM"
echo "=========================================="

if [[ "$FAIL" -eq 0 ]]; then
  printf "${YELLOW}提醒：Windows / macOS 只能由远程 Native Check 判定，合入前请确认三平台在同一 SHA 上为 success。${NC}\n"
fi

if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi
