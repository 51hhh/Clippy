"""用混合实码检查查看器和贴图的原图定位与复制。"""

import json
import os
from pathlib import Path
import runpy
import sqlite3
import struct
import subprocess
import sys
import tempfile
import zlib


qa = runpy.run_path(str(Path(__file__).with_name("slint-x11.py")))
Gdk, wait_for = qa["Gdk"], qa["wait_for"]
SCAN_TITLE = "Clippy Code Scan"
SCALE = float(os.environ.get("SLINT_SCALE_FACTOR", "1"))
QR = b"CLIPPY-QR-1D"
QR_ROWS = (
    "#######...#.#.#######", "#.....#.#.#.#.#.....#", "#.###.#.#.##..#.###.#",
    "#.###.#.....#.#.###.#", "#.###.#.#####.#.###.#", "#.....#.###...#.....#",
    "#######.#.#.#.#######", "........#............", "##.#..##..###.###.##.",
    "..###..##.##.#....##.", "##.#####...#..##..#.#", "..#..#.##.#.#.#..#...",
    "#####.#...##.###..#.#", "........#..#...#.##..", "#######.#.###.#.#.##.",
    "#.....#..#.#.#.....#.", "#.###.#..#.###..###..", "#.###.#.###........##",
    "#.###.#....######.#.#", "#.....#.###....##....", "#######.#..####.#.##.",
)


def fixture_png(has_codes):
    width, height = 320, 300
    rows = [bytearray([255] * width) for _ in range(height)]

    def rectangle(x, y, w, h):
        for row in rows[y:y + h]:
            row[x:x + w] = bytes(w)

    if has_codes:
        for y, row in enumerate(QR_ROWS):
            for x, cell in enumerate(row):
                if cell == "#":
                    rectangle(30 + x * 6, 24 + y * 6, 6, 6)
        x = 45
        for symbol in ("nwnnwnwnn", "wnnnnwnnw", "nwnnwnwnn"):
            for index, bar in enumerate(symbol):
                width_px = 12 if bar == "w" else 4
                if index % 2 == 0:
                    rectangle(x, 203, width_px, 64)
                x += width_px
            x += 4

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    header = struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0)
    data = zlib.compress(b"".join(b"\0" + row for row in rows))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", data) + chunk(b"IEND", b"")


def marker(bounds, color, lower=False):
    x, y, width, height = bounds
    split = height * 2 // 3
    offset, rows = (split, height - split) if lower else (0, split)
    snapshot = Gdk.pixbuf_get_from_window(
        Gdk.get_default_root_window(), x, y + offset, width - round(372 * SCALE), rows,
    )
    if snapshot is not None:
        pixels = snapshot.get_pixels()
        stride = snapshot.get_rowstride()
        for row in range(rows):
            line = pixels[row * stride:row * stride + snapshot.get_width() * 3]
            match = line.find(color)
            if match >= 0 and match % 3 == 0:
                return x + match // 3, y + offset + row
    return None


def run(binary, root):
    tray, desktop = qa["Tray"](), qa["Desktop"]()
    data_dir = root / "data/com.clippy.qa.slint"
    data_dir.mkdir(parents=True, mode=0o700)
    (data_dir / "config.json").write_text(json.dumps({
        "theme": "light", "language": "en", "max_history": 200, "storage_mode": "disk",
    }))
    env = dict(os.environ, XDG_DATA_HOME=str(root / "data"), XDG_CONFIG_HOME=str(root / "config"))
    process = subprocess.Popen([binary], env=env)
    blue, gold = bytes.fromhex("29a7ef"), bytes.fromhex("ffd158")

    def window(title):
        return wait_for(lambda: qa["titled_window"](title), f"{title} did not open")

    def close(title, xid):
        desktop.request_close(xid)
        wait_for(lambda: not qa["titled_window"](title), f"{title} did not close")

    def image_count():
        with sqlite3.connect(data_dir / "clips.db") as db:
            count, = db.execute("SELECT count(*) FROM clips WHERE content_type = 'image'").fetchone()
            return count

    def run_action(query):
        tray.activate("Actions")
        xid, _ = window(qa["LAUNCHER_TITLE"])
        desktop.focus(xid)
        desktop.keys("Control_L", "a")
        for letter in query:
            desktop.keys("space" if letter == " " else letter)
        desktop.keys("Return")

    def clipboard(expected):
        def matches():
            try:
                return qa["get_clipboard"]("UTF8_STRING") == expected
            except subprocess.CalledProcessError:
                return False

        wait_for(matches, f"clipboard did not contain {expected!r}")

    try:
        wait_for(lambda: tray.address, "no tray registration")
        qa["set_clipboard"]("image/png", fixture_png(True))
        wait_for(image_count, "fixture image did not reach the clipboard history")
        tray.activate("Open Clipboard")
        history_id, _ = window(qa["TITLE"])
        desktop.focus(history_id)
        desktop.keys("Tab")
        x, y, w, h = wait_for(
            lambda: (found := qa["titled_window"](qa["TITLE"])) and found[1][2] >= 775 * SCALE and found[1],
            "history preview did not expand",
        )
        desktop.click(x + w - round(53 * SCALE), y + h - round(24 * SCALE))
        viewer_id, (x, y, w, h) = window(qa["VIEWER_TITLE"])
        desktop.focus(viewer_id)
        desktop.click(round(x + w / 2 + 12 * SCALE), y + h - round(93 * SCALE))
        scan_id, bounds = window(SCAN_TITLE)
        first = wait_for(lambda: marker(bounds, blue), "QR source marker missing")
        wait_for(lambda: marker(bounds, blue, lower=True), "barcode source marker missing")
        desktop.click(*first)
        wait_for(lambda: marker(bounds, gold), "clicking a QR marker did not select its result")
        assert not marker(bounds, gold, lower=True), "QR selection highlighted the barcode"
        qa["save_qa_window"]("slint-code-scan-mixed.png", bounds)
        x, y, w, h = bounds
        desktop.click(x + w - round(45 * SCALE), y + round(80 * SCALE))
        clipboard(QR)
        desktop.click(x + w - round(323 * SCALE), y + h - round(24 * SCALE))
        clipboard(QR + b"\nA")
        close(SCAN_TITLE, scan_id)
        close(qa["VIEWER_TITLE"], viewer_id)
        print("PASS viewer recognized two codes, selected a source marker and copied exact text", flush=True)

        run_action("pin image")
        pin_id, (x, y, w, _) = window(qa["PIN_TITLE"])
        desktop.focus(pin_id)
        desktop.click(x + w - round(26 * SCALE), y + round(318 * SCALE))
        scan_id, bounds = window(SCAN_TITLE)
        lower = wait_for(lambda: marker(bounds, blue, lower=True), "pin barcode marker missing")
        desktop.click(*lower)
        wait_for(lambda: marker(bounds, gold, lower=True), "pin barcode marker did not select its result")
        assert not marker(bounds, gold), "selecting the barcode highlighted the QR result"
        x, y, w, _ = bounds
        desktop.click(x + w - round(45 * SCALE), y + round(138 * SCALE))
        clipboard(b"A")
        close(SCAN_TITLE, scan_id)
        close(qa["PIN_TITLE"], pin_id)
        print("PASS pin scan uses the displayed image and independent barcode coordinates", flush=True)

        qa["set_clipboard"]("image/png", fixture_png(False))
        wait_for(lambda: image_count() == 2, "blank image did not reach the clipboard history")
        run_action("scan")
        blank_id, blank_bounds = window(SCAN_TITLE)
        qa["set_clipboard"]("UTF8_STRING", b"unchanged after empty scan")
        x, y, w, h = blank_bounds
        desktop.click(x + w - round(323 * SCALE), y + h - round(24 * SCALE))
        clipboard(b"unchanged after empty scan")
        assert not marker(blank_bounds, blue) and not marker(blank_bounds, blue, lower=True)
        qa["save_qa_window"]("slint-code-scan-empty.png", blank_bounds)
        close(SCAN_TITLE, blank_id)
        print("PASS empty scan leaves the clipboard and source markers untouched", flush=True)
        tray.activate("Quit")
        wait_for(lambda: process.poll() is not None, "tray quit did not exit")
        assert process.returncode == 0
    except Exception:
        qa["save_failure_frame"]()
        raise
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="clippy-code-scan-x11-") as directory:
        run(sys.argv[1], Path(directory))
