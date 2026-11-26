"""用源屏幕像素验收 Slint 的窗口速选和自由框选。"""
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import time


qa = runpy.run_path(str(Path(__file__).with_name("slint-x11.py")))
Gdk, GdkPixbuf, GLib, Gtk = (qa[name] for name in ("Gdk", "GdkPixbuf", "GLib", "Gtk"))
wait_for, window = qa["wait_for"], qa["titled_window"]
CAPTURE = qa["CAPTURE_TITLE"]


def fixture(title, bounds, color):
    x, y, width, height = bounds
    pixels = bytes(channel
                   for row in range(height)
                   for col in range(width)
                   for channel in (color[0] + col % 19, color[1] + row % 17, color[2]))
    image = GdkPixbuf.Pixbuf.new_from_bytes(
        GLib.Bytes.new(pixels), GdkPixbuf.Colorspace.RGB, False, 8, width, height, width * 3)
    surface = Gtk.Window(title=title)
    surface.set_decorated(False)
    surface.move(x, y)
    surface.add(Gtk.Image.new_from_pixbuf(image))
    surface.show_all()
    wait_for(lambda: window(title), f"{title} is not visible")
    assert window(title)[1] == bounds
    return surface


def screen(bounds):
    return Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), *bounds)


def rgba_pixels(pixbuf):
    return pixbuf.add_alpha(False, 0, 0, 0).get_pixels()


def assert_png(data, expected):
    actual = qa["pixbuf_png"](data)
    got = (actual.get_width(), actual.get_height())
    want = (expected.get_width(), expected.get_height())
    assert got == want, f"captured {got}, expected {want}"
    assert rgba_pixels(actual) == rgba_pixels(expected)


def run(binary, root):
    tray, desktop = qa["Tray"](), qa["Desktop"]()
    desktop.background(0x28384C)
    lower_box, upper_box = (103, 109, 261, 179), (239, 203, 403, 257)
    lower = fixture("Clippy QA lower window", lower_box, (70, 150, 65))
    upper = fixture("Clippy QA upper window", upper_box, (175, 52, 80))
    dark_point = (180, 190, 1, 1)
    light_point = (380, 280, 1, 1)
    wait_for(lambda: screen(dark_point).get_pixels()[2] == 65
             and screen(light_point).get_pixels()[2] == 80, "fixture surfaces did not paint")
    underlying = screen(dark_point).get_pixels()
    on_top = screen(light_point).get_pixels()

    data, shots = root / "data/com.clippy.qa.slint", root / "shots"
    data.mkdir(parents=True, mode=0o700)
    (data / "config.json").write_text(json.dumps({
        "auto_paste": False, "max_history": 200, "storage_mode": "memory", "theme": "dark",
        "screenshot_save_dir": str(shots), "screenshot_filename_template": "window.png",
    }))
    env = dict(os.environ, XDG_DATA_HOME=str(root / "data"), XDG_CONFIG_HOME=str(root / "config"))
    app_log = root / "app.log"
    with app_log.open("w") as log:
        app = subprocess.Popen([binary], env=env, stderr=log)

    def open_capture(candidates):
        assert app.poll() is None
        qa["set_clipboard"]("UTF8_STRING", b"before window screenshot")
        previous = app_log.read_text().count("capture_windows candidates=")
        tray.activate("Screenshot")
        overlay, bounds = wait_for(lambda: window(CAPTURE), "capture overlay did not open")
        desktop.focus(overlay)

        def ready():
            lines = [line for line in app_log.read_text().splitlines()
                     if line.startswith("capture_windows candidates=")]
            return len(lines) > previous and lines[-1].startswith(f"capture_windows candidates={candidates} ")

        wait_for(ready, "window snapshot did not describe this capture")
        return bounds

    def copy(expected):
        desktop.keys("Return")
        assert_png(wait_for(qa["clipboard_png"], "selection was not copied as PNG"), expected)
        wait_for(lambda: not window(CAPTURE), "copy did not close capture overlay")

    try:
        wait_for(lambda: tray.address, "no tray registration")
        expected = screen(upper_box)
        bounds = open_capture(2)
        desktop.move(desktop.display, 0, 300, 240, 0)
        desktop.sync(desktop.display, 0)
        wait_for(lambda: screen(light_point).get_pixels() == on_top
                 and screen(dark_point).get_pixels() != underlying,
                 "hover did not reveal only the top window")
        qa["save_qa_window"]("capture-window-hover.png", bounds)
        # 手抖越过重叠边缘时沿用按下时的窗口。
        desktop.drag(240, 240, 237, 240)
        copy(expected)
        print("PASS overlap preview and click lock selected the top window at exact source pixels", flush=True)

        expected = screen(lower_box)
        _, _, width, height = open_capture(2)
        desktop.click(145, 155)
        scale = float(env.get("SLINT_SCALE_FACTOR", "1"))
        toolbar_width = min(560, width / scale - 16)
        toolbar_x = max(8, min((lower_box[0] + lower_box[2]) / scale - toolbar_width,
                               width / scale - toolbar_width - 8))
        toolbar_y = (lower_box[1] + lower_box[3]) / scale + 8
        assert toolbar_y + 76 <= height / scale
        save_x = toolbar_x + toolbar_width - 6 - 2.5 * qa["capture_button_width"](toolbar_width)
        desktop.click(round(save_x * scale), round((toolbar_y + 57) * scale))
        output = shots / "window.png"
        wait_for(output.is_file, "window selection was not saved")
        assert_png(output.read_bytes(), expected)
        wait_for(lambda: not window(CAPTURE), "save did not close capture overlay")
        print("PASS uncovered part selected the lower window and saved an exact PNG", flush=True)

        centered = (980, 640)
        expected = screen((0, 0, width, height))
        open_capture(2)
        desktop.click(*centered)
        copy(expected)
        manual = (310, 250, 160, 150)
        expected = screen(manual)
        inside = (manual[0] + 20, manual[1] + 20, 1, 1)
        unshaded = screen(inside).get_pixels()
        open_capture(2)
        desktop.click(*centered)
        wait_for(lambda: screen(inside).get_pixels() == unshaded, "empty click did not reveal the display")
        desktop.move(desktop.display, 0, manual[0], manual[1], 0)
        desktop.sync(desktop.display, 0)
        time.sleep(.04)
        desktop.button(desktop.display, 1, 1, 0)
        desktop.sync(desktop.display, 0)
        wait_for(lambda: screen(inside).get_pixels() != unshaded, "drag press did not begin a new selection")
        desktop.move(desktop.display, 0, manual[0] + manual[2], manual[1] + manual[3], 0)
        desktop.sync(desktop.display, 0)
        wait_for(lambda: screen(inside).get_pixels() == unshaded, "drag motion did not extend the selection")
        desktop.button(desktop.display, 1, 0, 0)
        desktop.sync(desktop.display, 0)
        copy(expected)
        print("PASS empty click selected the display and a new drag could replace that selection", flush=True)

        open_capture(2)
        desktop.keys("Escape")
        wait_for(lambda: not window(CAPTURE), "Escape did not close capture overlay")
        assert not qa["clipboard_png"]()
        upper.destroy()
        wait_for(lambda: not window("Clippy QA upper window"), "closed fixture is still mapped")
        expected = lower.get_child().get_pixbuf()
        expected_pixels = rgba_pixels(expected)
        wait_for(lambda: rgba_pixels(screen(lower_box)) == expected_pixels,
                 "uncovered lower fixture did not repaint")
        open_capture(1)
        desktop.click(300, 240)
        copy(expected)
        print("PASS reopening after cancellation used the new snapshot", flush=True)
        tray.activate("Quit")
        assert app.wait(timeout=5) == 0
    finally:
        if app.poll() is None:
            app.terminate()
            app.wait(timeout=5)
        print(app_log.read_text(), end="", flush=True)
        lower.destroy()
        desktop.close_display(desktop.display)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="clippy-window-pick-") as directory:
        run(str(Path(sys.argv[1]).resolve()), Path(directory))
