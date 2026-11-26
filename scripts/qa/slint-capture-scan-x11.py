"""在隔离 X11 中验证截图覆盖层直接扫码及后续截图操作。"""

import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile


codes = runpy.run_path(str(Path(__file__).with_name("slint-code-scan-x11.py")))
qa = codes["qa"]
Gdk, SCALE, wait_for = qa["Gdk"], codes["SCALE"], qa["wait_for"]
MIXED = (320, 100, 320, 300)
EMPTY = (80, 525, 240, 200)
QR_AREA = (338, 111, 160, 148)
BAR_AREA = (353, 300, 200, 75)
BLUE, GOLD = bytes.fromhex("29a7ef"), bytes.fromhex("ffd158")


def screen(bounds):
    return Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), *bounds)


def toolbar(area, window):
    _, _, width, height = window
    x, y, w, h = area
    gap = 8 * SCALE
    bar_width = min(560 * SCALE, width - 2 * gap)
    left = max(gap, min(x + w - bar_width, width - bar_width - gap))
    top = y + h + gap if y + h + 76 * SCALE + 2 * gap <= height else max(gap, y - 76 * SCALE - gap)
    button = min(32 * SCALE, (bar_width - 24 * SCALE) / 13)
    return left, top, bar_width, button


def run(binary, root):
    tray, desktop = qa["Tray"](), qa["Desktop"]()
    data = root / "data" / "com.clippy.qa.slint"
    data.mkdir(parents=True, mode=0o700)
    (data / "config.json").write_text(json.dumps({
        "theme": "light", "language": "en", "max_history": 100, "storage_mode": "memory",
    }))
    env = dict(os.environ, XDG_DATA_HOME=str(root / "data"), XDG_CONFIG_HOME=str(root / "config"))
    app_log = root / "app.log"
    with app_log.open("w") as log:
        app = subprocess.Popen([binary], env=env, stderr=log)

    def paint():
        desktop.background(0x435567)
        codes["paint_symbols"](desktop, MIXED[:2], (0, 0, *MIXED[2:]),
                               codes["qr_rectangles"]() + codes["code_39_rectangles"]())

    def open_capture():
        paint()
        before = app_log.read_text().count("capture_overlay output=")
        tray.activate("Screenshot")
        overlay = wait_for(lambda: qa["titled_window"](qa["CAPTURE_TITLE"]), "screenshot did not open")
        wait_for(lambda: app_log.read_text().count("capture_overlay output=") > before,
                 "screenshot did not render frozen pixels")
        desktop.focus(overlay[0])
        return overlay

    def drag(area):
        x, y, w, h = area
        desktop.drag(x, y, x + w, y + h)

    def scan(area, overlay, count):
        left, top, _, button = toolbar(area, overlay[1])
        token = f"capture_scan finished codes=Some({count})"
        before = app_log.read_text().count(token)
        desktop.click(round(left + 15 * SCALE + 12.5 * button), round(top + 19 * SCALE))
        wait_for(lambda: app_log.read_text().count(token) > before, f"failed to display {count} codes")
        assert qa["titled_window"](qa["CAPTURE_TITLE"])[0] == overlay[0], "scan closed the screenshot"

    def clipboard(text):
        wait_for(lambda: qa["get_clipboard"]("UTF8_STRING") == text, f"clipboard did not contain {text!r}")

    try:
        wait_for(lambda: tray.address, "no tray registration")
        qa["set_clipboard"]("UTF8_STRING", b"before overlay scan")
        paint()
        expected_empty = screen(EMPTY).add_alpha(False, 0, 0, 0).get_pixels()
        overlay = open_capture()
        drag(MIXED)
        scan(MIXED, overlay, 2)
        first = wait_for(lambda: codes["find_color"](QR_AREA, BLUE), "QR was not marked on the frozen pixels")
        wait_for(lambda: codes["find_color"](BAR_AREA, BLUE), "Code 39 was not marked on the frozen pixels")
        clipboard(b"before overlay scan")
        desktop.click(*first)
        wait_for(lambda: codes["find_color"](QR_AREA, GOLD), "selecting the QR marker did not highlight it")
        assert not codes["find_color"](BAR_AREA, GOLD)
        qa["save_qa_window"]("slint-capture-scan-mixed.png", overlay[1])

        panel_x, panel_y = MIXED[0] + MIXED[2] + 8 * SCALE, MIXED[1]
        desktop.click(round(panel_x + 292 * SCALE), round(panel_y + 29 * SCALE))
        clipboard(codes["QR"])
        desktop.click(round(panel_x + 224 * SCALE), round(panel_y + 270 * SCALE))
        clipboard(codes["QR"] + b"\nA")

        drag(EMPTY)
        wait_for(lambda: not codes["find_color"](QR_AREA, GOLD) and not codes["find_color"](BAR_AREA, BLUE),
                 "editing the selection left old scan markers on screen")
        scan(EMPTY, overlay, 0)
        clipboard(codes["QR"] + b"\nA")
        qa["save_qa_window"]("slint-capture-scan-empty.png", overlay[1])
        left, top, width, button = toolbar(EMPTY, overlay[1])
        desktop.click(round(left + width - 6 * SCALE - 1.5 * button), round(top + 57 * SCALE))
        png = wait_for(qa["clipboard_png"], "an empty scan prevented normal screenshot copy")
        pixbuf = qa["pixbuf_png"](png)
        assert (pixbuf.get_width(), pixbuf.get_height()) == EMPTY[2:]
        assert pixbuf.add_alpha(False, 0, 0, 0).get_pixels() == expected_empty
        wait_for(lambda: not qa["titled_window"](qa["CAPTURE_TITLE"]), "screenshot copy did not close the overlay")
        print("PASS overlay recognized QR and Code 39, copied raw text, cleared markers and copied the next selection", flush=True)

        capture = open_capture()
        drag(MIXED)
        left, top, _, button = toolbar(MIXED, capture[1])
        desktop.click(round(left + 6 * SCALE + 8.5 * button), round(top + 19 * SCALE))
        before = screen(QR_AREA).get_pixels()
        desktop.drag(332, 106, 488, 258)
        wait_for(lambda: screen(QR_AREA).get_pixels() != before, "mosaic was not rendered")
        scan(MIXED, capture, 1)
        assert not codes["find_color"](QR_AREA, BLUE), "scan read pixels hidden by the mosaic"
        wait_for(lambda: codes["find_color"](BAR_AREA, BLUE), "the uncovered Code 39 was lost")
        desktop.keys("Escape")
        wait_for(lambda: not qa["titled_window"](qa["CAPTURE_TITLE"]), "Escape did not close the scanner and capture")
        print("PASS direct scan reads the annotated pixels and leaves hidden QR content out of the results", flush=True)

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
    with tempfile.TemporaryDirectory(prefix="clippy-capture-scan-x11-") as directory:
        run(str(Path(sys.argv[1]).resolve()), Path(directory))
