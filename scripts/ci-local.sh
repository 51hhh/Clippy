#!/usr/bin/env bash
# 本地质量预检脚本 — 提交前运行，与 CI 门禁保持一致
# 用法: ./scripts/ci-local.sh [--quick]
#   --quick: 跳过构建检查，仅运行 lint/test

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
  require_cmd npm "Node.js >= 20.19: https://nodejs.org"
  require_cmd npx "随 Node.js 一同安装"
  # DOM/Canvas smoke 在无头环境下依赖 Xvfb，缺失时整条前端 smoke 都无法执行。
  require_cmd xvfb-run "sudo apt install -y xvfb"

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

echo "=========================================="
echo " Clippy 本地质量预检"
echo "=========================================="
echo ""

check_prerequisites

# --- Rust ---
run_step "cargo fmt --check" bash -c "cd src-tauri && cargo fmt -- --check"
run_step "cargo check" bash -c "cd src-tauri && cargo check --all-targets"
run_step "cargo clippy" bash -c "cd src-tauri && cargo clippy --all-targets -- -D warnings"
run_step "cargo test" bash -c "cd src-tauri && cargo test"

# --- Frontend ---
run_step "npm ci" bash -c "cd src && npm ci --prefer-offline"
run_step "typecheck" bash -c "cd src && npx tsc --noEmit"
run_step "vitest" bash -c "cd src && npx vitest run"
run_step "DOM/Xvfb smoke" ./scripts/smoke-dom.sh
run_step "Canvas 导出像素 smoke" ./scripts/smoke-canvas-export.sh

if [[ "$QUICK" == false ]]; then
  run_step "vite build" bash -c "cd src && npx vite build"
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
echo "=========================================="

if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi
