#!/bin/sh
# 修复 GNOME 双启动问题：
# enableGTKAppId=true 时 GTK 用 com.clippy.app 注册 D-Bus，GNOME shell 会
# 同时把 Clippy.desktop（按 productName 生成）和 com.clippy.app（按 app-id）
# 视为两个 app，导致单击启动后并发拉起两份进程。
#
# 解法：把 Clippy.desktop 重命名为 com.clippy.app.desktop —— GNOME 按
# basename 与 GTK app-id 匹配，从此只识别一项。

DESKTOP_DIR="/usr/share/applications"
SRC="${DESKTOP_DIR}/Clippy.desktop"
DST="${DESKTOP_DIR}/com.clippy.app.desktop"

if [ -L "$DST" ]; then
  # 旧版本（v0.1.6）创建过软链，先清掉
  rm -f "$DST"
fi

# 用新解压的 Clippy.desktop 覆盖（或创建）com.clippy.app.desktop
# 升级 0.1.7 → 后续版本时，dpkg 会重新解压 Clippy.desktop，旧的 DST 是 postinst
# 创建的孤儿（不在 dpkg 文件清单内），必须强制覆盖以保证内容是最新的。
if [ -f "$SRC" ]; then
  mv -f "$SRC" "$DST"
fi

# 图标也按 app-id 命名链接一次（部分主题按文件名查图标）
for size in 32x32 128x128; do
  ICON_DIR="/usr/share/icons/hicolor/${size}/apps"
  if [ -f "${ICON_DIR}/clippy-app.png" ] && [ ! -e "${ICON_DIR}/com.clippy.app.png" ]; then
    ln -sf "${ICON_DIR}/clippy-app.png" "${ICON_DIR}/com.clippy.app.png"
  fi
done

# 刷新桌面项缓存（让 GNOME 重新扫描）
update-desktop-database -q /usr/share/applications 2>/dev/null || true
gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true
