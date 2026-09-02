#!/usr/bin/env bash
# 修复 linuxdeploy 生成的绝对 .DirIcon，重新封装并校验 AppImage。
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEFAULT_BUNDLE_DIR="${ROOT_DIR}/src-tauri/target/release/bundle/appimage"
TARGET="${1:-}"
REQUIRE_SIGNATURE="${REQUIRE_TAURI_SIGNATURE:-false}"

if [[ -z "${TARGET}" ]]; then
  TARGET="$(find "${DEFAULT_BUNDLE_DIR}" -maxdepth 1 -type f \
    -name '*_amd64.AppImage' -print -quit)"
fi
if [[ -z "${TARGET}" || ! -f "${TARGET}" ]]; then
  printf 'AppImage not found: %s\n' "${TARGET:-<empty>}" >&2
  exit 1
fi
TARGET="$(cd "$(dirname "${TARGET}")" && pwd)/$(basename "${TARGET}")"
BUNDLE_DIR="$(dirname "${TARGET}")"

mapfile -t APP_DIRS < <(find "${BUNDLE_DIR}" -maxdepth 1 -type d -name '*.AppDir' -print)
if [[ "${#APP_DIRS[@]}" -ne 1 ]]; then
  printf 'Expected one AppDir in %s, found %d\n' "${BUNDLE_DIR}" "${#APP_DIRS[@]}" >&2
  exit 1
fi
APP_DIR="${APP_DIRS[0]}"

mapfile -t ROOT_ICONS < <(find "${APP_DIR}" -maxdepth 1 -type f -name '*.png' -print)
if [[ "${#ROOT_ICONS[@]}" -ne 1 ]]; then
  printf 'Expected one root PNG in %s, found %d\n' "${APP_DIR}" "${#ROOT_ICONS[@]}" >&2
  exit 1
fi
ICON_NAME="$(basename "${ROOT_ICONS[0]}")"
ln -sfn "${ICON_NAME}" "${APP_DIR}/.DirIcon"

# linuxdeploy-plugin-gtk 会把构建机的 Wayland ABI 库带进 AppImage。用 Ubuntu 22
# 构建后，这些旧库会优先于新发行版的系统库加载，并与 Mesa 25+ 混用，导致
# WebKitWebProcess 以 EGL_BAD_PARAMETER 终止。Wayland ABI 是桌面系统基础能力，
# 应由运行机提供；四个相互依赖的库必须作为一个集合移除。
# 上游跟踪：https://github.com/tauri-apps/tauri/issues/15665
mapfile -t BUNDLED_WAYLAND_LIBS < <(
  find "${APP_DIR}/usr/lib" \( -type f -o -type l \) \
    -name 'libwayland-*.so*' -print | sort
)
for library in "${BUNDLED_WAYLAND_LIBS[@]}"; do
  rm -f -- "${library}"
done

PLUGIN="${TAURI_APPIMAGE_PLUGIN:-${HOME}/.cache/tauri/linuxdeploy-plugin-appimage.AppImage}"
if [[ ! -x "${PLUGIN}" ]]; then
  printf 'Tauri AppImage plugin is unavailable: %s\n' "${PLUGIN}" >&2
  exit 1
fi

ARCH="${ARCH:-x86_64}"
GENERATED="${BUNDLE_DIR}/$(basename "${APP_DIR}" .AppDir)-${ARCH}.AppImage"
rm -f "${GENERATED}"
(
  cd "${BUNDLE_DIR}"
  ARCH="${ARCH}" "${PLUGIN}" --appimage-extract-and-run \
    --appdir="$(basename "${APP_DIR}")"
)
if [[ ! -f "${GENERATED}" ]]; then
  printf 'AppImage plugin did not create %s\n' "${GENERATED}" >&2
  exit 1
fi
mv -f "${GENERATED}" "${TARGET}"

if ! command -v unsquashfs >/dev/null 2>&1; then
  printf '%s\n' 'unsquashfs is required to verify the final AppImage' >&2
  exit 1
fi

VALID_OFFSET=""
while IFS=: read -r offset _; do
  if unsquashfs -o "${offset}" -s "${TARGET}" >/dev/null 2>&1; then
    VALID_OFFSET="${offset}"
  fi
done < <(LC_ALL=C grep -aob 'hsqs' "${TARGET}")
if [[ -z "${VALID_OFFSET}" ]]; then
  printf '%s\n' 'Final AppImage has no readable SquashFS payload' >&2
  exit 1
fi

DIR_ICON_ENTRY="$(unsquashfs -o "${VALID_OFFSET}" -lls "${TARGET}" | \
  grep 'squashfs-root/.DirIcon ->' || true)"
if [[ "${DIR_ICON_ENTRY}" != *" -> ${ICON_NAME}" ]]; then
  printf 'Final AppImage contains a non-portable .DirIcon: %s\n' \
    "${DIR_ICON_ENTRY:-<missing>}" >&2
  exit 1
fi

WAYLAND_LIBRARY_ENTRIES="$(unsquashfs -o "${VALID_OFFSET}" -lls "${TARGET}" | \
  grep -E 'squashfs-root/.*/libwayland-[^/]*\.so' || true)"
if [[ -n "${WAYLAND_LIBRARY_ENTRIES}" ]]; then
  printf 'Final AppImage still bundles host Wayland ABI libraries:\n%s\n' \
    "${WAYLAND_LIBRARY_ENTRIES}" >&2
  exit 1
fi

# 签名用的 tauri CLI：优先仓库锁定的 npm 侧 CLI（`src/` 的 devDependency，与 cargo-tauri 同版本），
# 其次才是 cargo-tauri。release runner 上只有 tauri-action 自带的 CLI，没有 cargo-tauri，
# 这里原来写死 `cargo tauri` 会直接 `no such command: tauri`，AppImage 签名步骤必挂。
tauri_cli() {
  local npm_cli="${ROOT_DIR}/src/node_modules/.bin/tauri"
  if [[ -x "${npm_cli}" ]]; then
    (cd "${ROOT_DIR}" && "${npm_cli}" "$@")
  elif cargo tauri --version >/dev/null 2>&1; then
    (cd "${ROOT_DIR}" && cargo tauri "$@")
  else
    printf '%s\n' 'No tauri CLI found for signing (need src/node_modules/.bin/tauri or cargo-tauri)' >&2
    return 1
  fi
}

rm -f "${TARGET}.sig"
if [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" || -n "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" ]]; then
  tauri_cli signer sign "${TARGET}"
  # 签名失败时 CLI 也可能 exit 0（例如密钥密码不对只打印警告），产物缺 .sig 会让
  # updater 静默拿不到签名，所以显式校验。
  if [[ ! -s "${TARGET}.sig" ]]; then
    printf 'Signing produced no signature: %s\n' "${TARGET}.sig" >&2
    exit 1
  fi
elif [[ "${REQUIRE_SIGNATURE}" == "true" ]]; then
  printf '%s\n' 'Updater signing key is required for release AppImage finalization' >&2
  exit 1
fi

printf 'Finalized AppImage: %s\n' "${TARGET}"
printf 'Portable .DirIcon: %s\n' "${ICON_NAME}"
printf 'Removed bundled Wayland ABI libraries: %d\n' "${#BUNDLED_WAYLAND_LIBS[@]}"
