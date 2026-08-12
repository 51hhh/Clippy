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

rm -f "${TARGET}.sig"
if [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" || -n "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" ]]; then
  (cd "${ROOT_DIR}" && cargo tauri signer sign "${TARGET}")
elif [[ "${REQUIRE_SIGNATURE}" == "true" ]]; then
  printf '%s\n' 'Updater signing key is required for release AppImage finalization' >&2
  exit 1
fi

printf 'Finalized AppImage: %s\n' "${TARGET}"
printf 'Portable .DirIcon: %s\n' "${ICON_NAME}"
