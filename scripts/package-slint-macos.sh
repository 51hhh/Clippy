#!/usr/bin/env bash
set -euo pipefail

config=$1
app=$2
repo=$(cd -- "$(dirname -- "$0")/.." && pwd)
export CLIPPY_MACOS_PACKAGE_APP="$PWD/$app"
export CLIPPY_MACOS_PACKAGE_ENTITLEMENTS="$repo/apps/slint/Entitlements.macos.plist"

temporary=$(mktemp "${config%.toml}.macos.XXXXXX")
trap 'rm -f -- "$temporary" "$temporary.toml"' EXIT
{
  # Packager 会对资源内的 Mach-O 一起签名。完成 app 后只给主应用加权限，再将其装入 DMG。
  cat <<'HOOK'
before-each-package-command = '''
if [ "$CARGO_PACKAGER_FORMAT" = dmg ]; then
  codesign --force --sign - --options runtime --timestamp=none \
    --entitlements "$CLIPPY_MACOS_PACKAGE_ENTITLEMENTS" "$CLIPPY_MACOS_PACKAGE_APP"
fi
'''
HOOK
  cat -- "$config"
} > "$temporary.toml"

cargo-packager --config "$temporary.toml" --formats app,dmg
