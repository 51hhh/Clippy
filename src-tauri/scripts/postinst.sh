#!/bin/sh
# 创建符号链接，让 GTK app ID (com.clippy.app) 能找到对应的 desktop 文件和图标
# 解决 AppImage 运行时桌面环境因找不到 com.clippy.app.desktop 而自动创建重复项

DESKTOP_DIR="/usr/share/applications"
ln -sf "${DESKTOP_DIR}/Clippy.desktop" "${DESKTOP_DIR}/com.clippy.app.desktop"

for size in 32x32 128x128; do
  ICON_DIR="/usr/share/icons/hicolor/${size}/apps"
  if [ -f "${ICON_DIR}/clippy-app.png" ]; then
    ln -sf "${ICON_DIR}/clippy-app.png" "${ICON_DIR}/com.clippy.app.png"
  fi
done

# 刷新图标缓存
gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true
