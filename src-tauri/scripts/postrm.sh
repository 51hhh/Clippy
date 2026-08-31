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

# purge 时清掉窗口速选用的 GNOME Shell 扩展。
#
# 该扩展由应用在运行时装到用户目录（见 capture/shell_extension.rs），不在 dpkg
# 文件清单里，所以 dpkg 自己清不掉。正常路径是用户在设置页点"卸载服务"；这里只是
# 兜底，避免 purge 之后用户的 GNOME 里还留着 Clippy 的代码。
#
# 两点已知的不完美，都是刻意接受的：
# 1. 按 Debian policy，维护者脚本本不该动用户主目录。但这些文件是本包的程序放进去的，
#    purge 语义就是"连配置一起清干净"，因此只在 purge（不在 remove）时删，且只删
#    这一个 uuid 目录，绝不递归删别的东西。
# 2. 用户 dconf 里 org.gnome.shell enabled-extensions 的那条 uuid 删不掉——改它需要
#    对应用户的会话总线，root 手里没有。gnome-shell 遇到不存在的 uuid 只是记一条日志，
#    无副作用；用户下次装回 Clippy 时这条记录还能直接复用。
if [ "$1" = "purge" ]; then
  EXT_UUID="clippy-windows@clippy.local"
  for home in /root /home/*; do
    EXT_DIR="${home}/.local/share/gnome-shell/extensions/${EXT_UUID}"
    if [ -d "$EXT_DIR" ] && [ ! -L "$EXT_DIR" ]; then
      rm -rf "$EXT_DIR" 2>/dev/null || true
    fi
  done
fi
