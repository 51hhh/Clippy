#!/bin/sh
# 卸载时清理 postinst 创建的符号链接

DESKTOP_LINK="/usr/share/applications/com.clippy.app.desktop"
[ -L "$DESKTOP_LINK" ] && rm -f "$DESKTOP_LINK"

for size in 32x32 128x128; do
  ICON_LINK="/usr/share/icons/hicolor/${size}/apps/com.clippy.app.png"
  [ -L "$ICON_LINK" ] && rm -f "$ICON_LINK"
done

gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true
