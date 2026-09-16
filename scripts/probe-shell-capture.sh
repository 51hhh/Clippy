#!/bin/bash
# 在**当前会话**里直接量一遍"逐屏取像素"这条路，不用注销重登。
#
# 为什么需要它：gnome-shell 只在登录时加载扩展，`ReloadExtension` 已废弃，所以每改一行
# 扩展的 JS 就要注销一次——一轮验证十几分钟，而且失败了只能从 journal 里猜。
# 但 `org.gnome.Shell.Eval` 可以在活着的会话里跑任意 GJS，于是把扩展里那段取像素的代码
# 原样贴进去，就能当场知道"这套 GI 编排在这台机器上通不通、快不快"。
#
# 前提：Eval 默认是关的（返回 `(false, '')`）。打开方式是按 Alt+F2 输入 `lg` 回车打开
# Looking Glass，在里面执行一行 `global.context.unsafe_mode = true`。
# 这是**临时**的：注销就恢复，也只有坐在机器前的人能开（Looking Glass 需要物理在场）。
# 量完记得在 Looking Glass 里执行 `global.context.unsafe_mode = false` 关掉。
set -uo pipefail

eval_js() {
    gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
        --method org.gnome.Shell.Eval "$1"
}

probe=$(eval_js "1+1")
case "$probe" in
    "(true, '2')"*) ;;
    *)
        echo "org.gnome.Shell.Eval 关着（返回 $probe）。"
        echo
        echo "打开办法（不需要注销）："
        echo "  1. 按 Alt+F2，输入 lg 回车 —— 打开 Looking Glass"
        echo "  2. 在它的输入框里执行： global.context.unsafe_mode = true"
        echo "  3. 按 Esc 关掉 Looking Glass，重新跑本脚本"
        echo
        echo "量完请在 Looking Glass 里执行 global.context.unsafe_mode = false 关掉它。"
        exit 1
        ;;
esac

# 扩展里 _captureAreaToRawFile 的等价实现，逐屏跑一遍并分段计时。
# 用 imports.gi（GJS 的老式同步导入）——Eval 是同步的，进不去 ESM 的 import。
SNIPPET=$(cat <<'JS'
(() => {
    const {Clutter, Cogl, GLib, Mtk} = imports.gi;
    const now = () => GLib.get_monotonic_time() / 1000;
    const out = {shell: imports.misc.config.PACKAGE_VERSION, monitors: []};
    const display = global.display;
    for (let i = 0; i < display.get_n_monitors(); i++) {
        const geometry = display.get_monitor_geometry(i);
        const scale = display.get_monitor_scale(i);
        const record = {index: i, x: geometry.x, y: geometry.y,
            logical: `${geometry.width}x${geometry.height}`, scale};
        try {
            let t = now();
            const content = global.stage.paint_to_content(
                new Mtk.Rectangle({x: geometry.x, y: geometry.y,
                    width: geometry.width, height: geometry.height}),
                scale, global.stage.get_color_state?.() ?? null,
                Clutter.PaintFlag.NO_CURSORS);
            record.paintMs = Math.round(now() - t);
            const texture = content.get_texture();
            record.pixels = `${texture.get_width()}x${texture.get_height()}`;
            record.readBackSupported = texture.is_get_data_supported();

            const stride = texture.get_width() * 4;
            const data = new Uint8Array(stride * texture.get_height());
            const probes = [];
            const step = Math.max(1, Math.floor(data.length / 32));
            for (let o = 0; o < data.length; o += step) {
                data[o] = 0xCD;
                probes.push(o);
            }
            t = now();
            record.copied = texture.get_data(Cogl.PixelFormat.RGBA_8888, stride, data);
            record.getDataMs = Math.round(now() - t);
            record.bufferReachedCogl = !probes.every(o => data[o] === 0xCD);

            const path = `${GLib.get_user_runtime_dir()}/clippy-probe-${i}.rgba`;
            t = now();
            GLib.file_set_contents(path, data);
            record.writeMs = Math.round(now() - t);
            record.path = path;
            record.bytes = data.length;
            record.stride = stride;
        } catch (error) {
            record.error = `${error}`;
        }
        out.monitors.push(record);
    }
    return JSON.stringify(out);
})()
JS
)

echo "== 逐屏原始像素（paint_to_content + get_data）=="
eval_js "$SNIPPET" | python3 -c '
import sys, json, re
raw = sys.stdin.read()
m = re.match(r"^\(true, .(.*).,\)\s*$", raw, re.S)
if not m:
    print("Eval 没有返回结果:", raw); sys.exit(1)
data = json.loads(m.group(1).encode().decode("unicode_escape"))
print(f"gnome-shell {data[\"shell\"]}")
for r in data["monitors"]:
    if "error" in r:
        print(f"  屏 {r[\"index\"]} {r[\"logical\"]}×{r[\"scale\"]:.4f} → 失败: {r[\"error\"]}")
        continue
    print(f"  屏 {r[\"index\"]} @{r[\"x\"]},{r[\"y\"]} {r[\"logical\"]}×{r[\"scale\"]:.4f} → {r[\"pixels\"]} 原生像素")
    print(f"      paint_to_content {r[\"paintMs\"]} ms | get_data {r[\"getDataMs\"]} ms "
          f"(copied={r[\"copied\"]}, 缓冲区真的交给了 Cogl={r[\"bufferReachedCogl\"]}) | 写文件 {r[\"writeMs\"]} ms")
    print(f"      合计 {r[\"paintMs\"] + r[\"getDataMs\"] + r[\"writeMs\"]} ms，{r[\"bytes\"]/1048576:.1f} MiB → {r[\"path\"]}")
'
