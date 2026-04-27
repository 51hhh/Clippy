#!/bin/sh
# 卸载时清理 postinst 重命名后的 desktop 文件和图标符号链接

DESKTOP_FILE="/usr/share/applications/com.clippy.app.desktop"
[ -e "$DESKTOP_FILE" ] && rm -f "$DESKTOP_FILE"

for size in 32x32 128x128; do
  ICON_LINK="/usr/share/icons/hicolor/${size}/apps/com.clippy.app.png"
  [ -L "$ICON_LINK" ] && rm -f "$ICON_LINK"
done

update-desktop-database -q /usr/share/applications 2>/dev/null || true
gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true
