#!/bin/sh
set -eu

# v0.1.20 及更早版本的 postrm 会在升级解包后删除这三个路径。新包把它们纳入
# dpkg 文件清单，并额外保存同字节副本；postinst 只在旧脚本删过文件或残留软链时恢复。
# 写回内容与包内 md5sums 完全一致，因此 `dpkg -V clippy` 不再产生伪损坏报告。
ASSET_ROOT="/usr/lib/Clippy/package-assets"

restore_owned_file() {
  source_path="$1"
  destination_path="$2"

  if [ -L "$destination_path" ]; then
    rm -f "$destination_path"
  fi
  if [ ! -f "$destination_path" ] || ! cmp -s "$source_path" "$destination_path"; then
    install -m 0644 "$source_path" "$destination_path"
  fi
}

restore_owned_file \
  "${ASSET_ROOT}/com.clippy.app.desktop" \
  "/usr/share/applications/com.clippy.app.desktop"
restore_owned_file \
  "${ASSET_ROOT}/com.clippy.app-32.png" \
  "/usr/share/icons/hicolor/32x32/apps/com.clippy.app.png"
restore_owned_file \
  "${ASSET_ROOT}/com.clippy.app-128.png" \
  "/usr/share/icons/hicolor/128x128/apps/com.clippy.app.png"

# 刷新桌面项缓存（让 GNOME 重新扫描）。Tauri 生成的 Clippy.desktop 保留为
# dpkg 管理的 NoDisplay 兼容条目，唯一可见入口按 GTK app-id 命名。
update-desktop-database -q /usr/share/applications 2>/dev/null || true
gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true
