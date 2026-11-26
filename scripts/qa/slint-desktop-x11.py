"""在两块真实 RandR 逻辑输出上验证跨屏拖拽和所有静态输出。"""
import json
import os
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tempfile
import time


qa = runpy.run_path(str(Path(__file__).with_name("slint-x11.py")))
codes = runpy.run_path(str(Path(__file__).with_name("slint-code-scan-x11.py")))
Gdk, wait_for = qa["Gdk"], qa["wait_for"]
LEFT, RIGHT = (0, 40, 640, 760), (640, 0, 640, 800)
OUTPUTS = (LEFT, RIGHT)
AREA = (230, 20, 780, 480)
START, END = (230, 500), (1010, 20)
SCALE = float(os.environ.get("SLINT_SCALE_FACTOR", "1"))


def capture_windows():
    tree = subprocess.run(["xwininfo", "-root", "-tree"], capture_output=True, text=True)
    if tree.returncode != 0:
        return {}
    windows = {}
    for name in re.findall(r'\b(0x[\da-f]+)\s+"Clippy Screenshot"', tree.stdout):
        info = subprocess.run(["xwininfo", "-id", name], capture_output=True, text=True)
        if info.returncode or "Map State: IsViewable" not in info.stdout:
            continue
        bounds = tuple(int(re.search(key + r":\s*(-?\d+)", info.stdout)[1])
                       for key in ("Absolute upper-left X", "Absolute upper-left Y", "Width", "Height"))
        windows[bounds] = int(name, 16)
    return windows


def screen(bounds):
    return Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), *bounds)


def paint_scene(desktop):
    desktop.background(0x435567)
    window = desktop.root_window(desktop.display)
    gc = desktop.create_gc(desktop.display, window, 0, None)
    for x, y, width, height, color in (
        (*LEFT, 0x568A2C), (*RIGHT, 0xBE4A69),
        (250, 100, 199, 172, 0x467C9A), (543, 282, 72, 90, 0x77CD2A),
        (649, 187, 164, 74, 0x9E662A), (867, 342, 153, 58, 0xCC7743),
    ):
        desktop.foreground(desktop.display, gc, color)
        desktop.fill(desktop.display, window, gc, x, y, width, height)
    desktop.free_gc(desktop.display, gc)
    desktop.sync(desktop.display, 0)


def expected_pixels(box):
    x, y, width, height = box
    pixels = bytearray(screen(box).add_alpha(False, 0, 0, 0).get_pixels())
    # 左屏上方的物理桌面没有可选输出，合成图在该角落应透明。
    gap_columns = max(0, min(width, LEFT[2] - x))
    if gap_columns and y < LEFT[1]:
        for row in range(LEFT[1] - y):
            offset = row * width * 4
            pixels[offset:offset + gap_columns * 4] = bytes(gap_columns * 4)
    return pixels


def assert_png(data, box, expected):
    actual = qa["pixbuf_png"](data)
    size = (actual.get_width(), actual.get_height())
    assert size == box[2:], (size, box[2:])
    assert actual.add_alpha(False, 0, 0, 0).get_pixels() == expected, "PNG pixels differ from frozen scene"


def toolbar_click(desktop, output, index):
    x, y, width, height = output
    left = max(x + 8 * SCALE, min(AREA[0] + AREA[2] - 176 * SCALE, x + width - 184 * SCALE))
    bottom = AREA[1] + AREA[3]
    assert bottom + 62 * SCALE < y + height
    top = bottom + 8 * SCALE
    wait_for(lambda: screen((round(left + 4), round(top + 23 * SCALE), 1, 1)).get_pixels()[:3]
             == bytes([32, 33, 36]), "cross-screen toolbar did not appear on the destination display")
    desktop.click(round(left + (8 + (index + .5) * 32) * SCALE), round(top + 23 * SCALE))


def run(binary, root):
    tray, desktop = qa["Tray"](), qa["Desktop"]()
    subprocess.run(["xrandr", "--setmonitor", "QA-Left", "640/169x760/201+0+40", "screen"], check=True)
    subprocess.run(["xrandr", "--setmonitor", "QA-Right", "640/169x800/212+640+0", "none"], check=True)
    print(subprocess.check_output(["xrandr", "--listactivemonitors"], text=True), end="", flush=True)
    paint_scene(desktop)
    expected = expected_pixels(AREA)
    local = (800, 200, 160, 200)
    local_pixels = expected_pixels(local)
    display_probes = [(490, 315, 1, 1), (825, 295, 1, 1), (170, 620, 1, 1)]
    bright = [screen(box).get_pixels() for box in display_probes]

    data, shots = root / "data/com.clippy.qa.slint", root / "shots"
    data.mkdir(parents=True, mode=0o700)
    (data / "config.json").write_text(json.dumps({
        "theme": "dark", "auto_paste": False, "max_history": 200, "storage_mode": "memory",
        "screenshot_save_dir": str(shots), "screenshot_filename_template": "desktop.png",
    }))
    env = dict(os.environ, XDG_DATA_HOME=str(root / "data"), XDG_CONFIG_HOME=str(root / "config"))
    app_log = root / "app.log"
    with app_log.open("w") as log:
        app = subprocess.Popen([binary], env=env, stderr=log)

    def open_capture(extra_paint=None):
        assert app.poll() is None
        qa["set_clipboard"]("UTF8_STRING", b"before cross-screen capture")
        # 覆盖层关闭时 Xvfb 会用根背景重绘暴露区域，重新画出本轮已知底图。
        paint_scene(desktop)
        if extra_paint:
            extra_paint()
        before = app_log.read_text().count("capture_overlay output=")
        tray.activate("Screenshot")
        windows = wait_for(lambda: (found := capture_windows()) and set(found) == set(OUTPUTS) and found,
                           "two overlays did not map to the RandR outputs")
        wait_for(lambda: app_log.read_text().count("capture_overlay output=") == before + 2,
                 "both frozen overlays did not draw")
        return windows

    def move(point):
        desktop.move(desktop.display, 0, *point, 0)
        desktop.sync(desktop.display, 0)

    def drag_across(start, end, evidence=None):
        move(start)
        time.sleep(.05)
        desktop.button(desktop.display, 1, 1, 0)
        desktop.sync(desktop.display, 0)
        time.sleep(.05)
        move(end)
        wait_for(lambda: all(screen(box).get_pixels() == expected_pixel
                             for box, expected_pixel in zip(display_probes[:2], bright))
                 and screen(display_probes[2]).get_pixels() != bright[2],
                 "holding the pointer across the seam did not reveal both panes")
        if evidence:
            qa["save_qa_window"](evidence, (0, 0, 1280, 800))
        desktop.button(desktop.display, 1, 0, 0)
        desktop.sync(desktop.display, 0)

    def wait_closed():
        wait_for(lambda: not capture_windows(), "capture overlays did not close")

    def clipboard(box, pixels):
        assert_png(wait_for(qa["clipboard_png"], "nothing was copied"), box, pixels)

    try:
        wait_for(lambda: tray.address, "no tray registration")
        desktop.focus(open_capture()[LEFT])
        drag_across(START, END, "slint-desktop-drag-x11.png")
        toolbar_click(desktop, RIGHT, 1)
        saved = shots / "desktop.png"
        wait_for(saved.is_file, "cross-screen PNG did not save")
        assert_png(saved.read_bytes(), AREA, expected)
        wait_closed()
        print("PASS two-pane preview, right-side toolbar, exact pixels and transparent display gap", flush=True)

        desktop.focus(open_capture()[RIGHT])
        drag_across(END, START)
        desktop.keys("Return")
        clipboard(AREA, expected)
        wait_closed()
        print("PASS reverse drag and Enter copied the same desktop selection", flush=True)

        desktop.focus(open_capture()[LEFT])
        drag_across(START, END)
        toolbar_click(desktop, RIGHT, 0)
        editor, (x, y, width, height) = wait_for(lambda: qa["titled_window"](qa["EDITOR_TITLE"]),
                                                 "composite image did not open in the editor")
        wait_closed()
        desktop.focus(editor)
        desktop.click(round(x + width - 253 * SCALE), round(y + height - 55 * SCALE))
        clipboard(AREA, expected)
        desktop.request_close(editor)
        wait_for(lambda: not qa["titled_window"](qa["EDITOR_TITLE"]), "editor did not close")
        print("PASS the editor received the same composed pixels", flush=True)

        composed_codes = None

        def paint_scan_scene():
            nonlocal composed_codes
            codes["paint_symbols"](desktop, (250, 70), (12, 6, 170, 166), codes["qr_rectangles"]())
            codes["paint_symbols"](desktop, (695, 180), (0, 190, 300, 93), codes["code_39_rectangles"]())
            composed_codes = expected_pixels(AREA)

        windows = open_capture(paint_scan_scene)
        desktop.focus(windows[LEFT])
        drag_across(START, END)
        toolbar_click(desktop, RIGHT, 3)
        wait_for(lambda: "capture_scan finished codes=Some(2)" in app_log.read_text(),
                 "cross-screen scan did not recognize both symbols")
        qr_area, bar_area = (268, 87, 158, 140), (733, 379, 200, 71)
        blue, gold = bytes.fromhex("29a7ef"), bytes.fromhex("ffd158")
        find = codes["find_color"]
        marker = wait_for(lambda: find(qr_area, blue), "QR location is missing on the left screen")
        wait_for(lambda: find(bar_area, blue), "Code 39 location is missing on the right screen")
        desktop.click(*marker)
        wait_for(lambda: find(qr_area, gold), "left-screen marker could not select the result")
        assert not find(bar_area, gold)
        qa["save_qa_window"]("slint-capture-scan-cross-display.png", (0, 0, 1280, 800))
        desktop.keys("Return")
        clipboard(AREA, composed_codes)
        wait_closed()
        print("PASS cross-screen scan marked both original displays and preserved the composed screenshot", flush=True)

        windows = open_capture()
        desktop.focus(windows[LEFT])
        drag_across(START, END)
        # 先等旧拖拽释放，再由右屏发起单屏框选。
        desktop.focus(windows[RIGHT])
        move((local[0], local[1]))
        time.sleep(.07)
        desktop.button(desktop.display, 1, 1, 0)
        desktop.sync(desktop.display, 0)
        time.sleep(.07)
        move((local[0] + local[2], local[1] + local[3]))
        time.sleep(.07)
        desktop.button(desktop.display, 1, 0, 0)
        desktop.sync(desktop.display, 0)
        desktop.keys("Return")
        clipboard(local, local_pixels)
        wait_closed()
        print("PASS a new single-screen drag replaced the previous cross-screen selection", flush=True)

        desktop.focus(open_capture()[LEFT])
        drag_across(START, END)
        desktop.keys("Escape")
        wait_closed()
        assert not qa["clipboard_png"]()
        desktop.focus(open_capture()[RIGHT])
        drag_across(END, START)
        toolbar_click(desktop, LEFT, 4)
        wait_closed()
        assert not qa["clipboard_png"]()
        print("PASS Escape and the destination toolbar cancelled separate capture generations", flush=True)
        tray.activate("Quit")
        assert app.wait(timeout=5) == 0
    except Exception:
        qa["save_failure_frame"]()
        raise
    finally:
        if app.poll() is None:
            app.terminate()
            app.wait(timeout=5)
        print(app_log.read_text(), end="", flush=True)
        desktop.close_display(desktop.display)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="clippy-desktop-capture-") as directory:
        run(str(Path(sys.argv[1]).resolve()), Path(directory))
