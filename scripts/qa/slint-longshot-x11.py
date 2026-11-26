from pathlib import Path
import json
import os
import runpy
import subprocess
import sys
import tempfile
import time


qa = runpy.run_path(str(Path(__file__).with_name("slint-x11.py")))
Gdk, GdkPixbuf, GLib, Gtk = (qa[name] for name in ("Gdk", "GdkPixbuf", "GLib", "Gtk"))
wait_for, window = qa["wait_for"], qa["titled_window"]
LONGSHOT, CAPTURE, EDITOR, PIN = (
    qa[name] for name in ("LONGSHOT_TITLE", "CAPTURE_TITLE", "EDITOR_TITLE", "PIN_TITLE")
)
VIEW_WIDTH, VIEW_HEIGHT = 320, 240
PAGE_END, STEP, HEADER_HEIGHT = 72, 24, 24
HEADER = (24, 52, 86)
DIRECTIONS = ("down", "up", "left", "right")
TRAY_STOP = "Stop scrolling capture"


def texture(x, y):
    x, y = x // 4 * 4, y // 4 * 4
    value = ((y * 10007 + x * x * 31337) ^ (x * y * 46273)) & 0xffffff
    return tuple((value >> bit) & 255 for bit in (16, 8, 0))


def display_pixel(x, y):
    pixel = Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), x, y, 1, 1)
    return pixel.get_pixels()[:3] if pixel else None


class Page:
    def __init__(self):
        self.window = Gtk.Window(title="Clippy QA scrollable page")
        self.window.set_decorated(False)
        self.window.set_default_size(VIEW_WIDTH, VIEW_HEIGHT)
        self.window.move(400, 180)
        self.screen = Gtk.Image()
        self.window.add(self.screen)
        self.window.add_events(Gdk.EventMask.SCROLL_MASK)
        self.window.connect("scroll-event", self.scroll)
        self.position = (0, 0)
        self.events = []
        self.header, self.flat, self.delay = False, False, 0
        self.paint_position(self.position)
        self.window.show_all()
        _, self.x, self.y = self.window.get_window().get_origin()

    def paint_position(self, position):
        self.position = position
        if self.flat:
            gray = 64 + position[1] // 2
            pixels = bytes([gray]) * VIEW_WIDTH * VIEW_HEIGHT * 3
        else:
            offset_x, offset_y = position
            rows = []
            if self.header:
                rows.append(bytes(HEADER) * VIEW_WIDTH * HEADER_HEIGHT)
            for y in range(HEADER_HEIGHT if self.header else 0, VIEW_HEIGHT, 4):
                line = b"".join(bytes(texture(offset_x + x, offset_y + y)) * 4
                                for x in range(0, VIEW_WIDTH, 4))
                rows.append(line * 4)
            pixels = b"".join(rows)
        image = GdkPixbuf.Pixbuf.new_from_bytes(
            GLib.Bytes.new(pixels), GdkPixbuf.Colorspace.RGB, False, 8,
            VIEW_WIDTH, VIEW_HEIGHT, VIEW_WIDTH * 3)
        self.screen.set_from_pixbuf(image)
        return GLib.SOURCE_REMOVE

    def prepare(self, position=(0, 0), header=False, flat=False, delay=0):
        self.window.present()
        self.events.clear()
        self.header, self.flat, self.delay = header, flat, delay
        self.paint_position(position)
        expected = texture(24 + position[0], 60 + position[1]) if not flat else (64, 64, 64)
        wait_for(lambda: display_pixel(self.x + 24, self.y + 60) == bytes(expected), "source page did not paint")

    def scroll(self, _, event):
        direction, axis = {
            Gdk.ScrollDirection.DOWN: ("down", (0, STEP)),
            Gdk.ScrollDirection.UP: ("up", (0, -STEP)),
            Gdk.ScrollDirection.LEFT: ("left", (-STEP, 0)),
            Gdk.ScrollDirection.RIGHT: ("right", (STEP, 0)),
        }[event.direction]
        self.events.append(direction)
        next_position = tuple(max(0, min(PAGE_END, value + delta))
                              for value, delta in zip(self.position, axis))
        if self.delay:
            GLib.timeout_add(self.delay, self.paint_position, next_position)
        else:
            self.paint_position(next_position)
        return True

def tray_contains(tray, title):
    service, path = tray.address
    (menu_path,) = tray.call(service, path, "org.freedesktop.DBus.Properties", "Get",
                             GLib.Variant("(ss)", (qa["ITEM"], "Menu")))
    _, (_, _, children) = tray.call(service, menu_path, qa["MENU"], "GetLayout",
                                   GLib.Variant("(iias)", (0, 1, ["label"])))
    return any(props.get("label") == title for _, props, _ in children)


def open_longshot(tray, desktop, page):
    tray.activate("Screenshot")
    capture, (screen_x, screen_y, screen_width, _) = wait_for(
        lambda: window(CAPTURE), "screenshot overlay did not open")
    desktop.focus(capture)
    left, top = page.x, page.y
    right, bottom = left + VIEW_WIDTH, top + VIEW_HEIGHT
    desktop.drag(left, top, right, bottom)
    toolbar_width = min(560, screen_width - 16)
    toolbar_x = max(screen_x + 8, min(right - toolbar_width,
                                      screen_x + screen_width - toolbar_width - 8))
    toolbar_y = bottom + 8
    wait_for(lambda: display_pixel(toolbar_x + 4, toolbar_y + 22) == b"\x20\x21\x24",
             "capture toolbar did not appear")
    desktop.click(qa["capture_command_x"](toolbar_x, toolbar_width, 1), toolbar_y + 19)
    return ready_control()


def ready_control():
    def ready():
        found = window(LONGSHOT)
        if found:
            _, (x, y, _, height) = found
            if display_pixel(x + 23, y + height - 82) in (b"\x2b\x2d\x33", b"\x26\x3c\x56"):
                return found
        return None
    return wait_for(ready, "automatic scrolling controls did not become ready")


def control_button(desktop, bounds, index, automatic=False):
    x, y, width, height = bounds
    step = (width - 40 - 24) / 4
    point = (round(x + 20 + (step + 8) * index + step / 2),
             y + height - (82 if automatic else 36))
    desktop.click(*point)
    return point


def pointer_position():
    _, x, y = Gdk.Display.get_default().get_default_seat().get_pointer().get_position()
    return x, y


def start_auto(tray, desktop, page, direction):
    _, bounds = open_longshot(tray, desktop, page)
    origin = control_button(desktop, bounds, DIRECTIONS.index(direction), automatic=True)
    wait_for(lambda: not window(LONGSHOT), "long capture controls stayed in the capture area")
    wait_for(lambda: page.events, "native target received no scroll input")
    return origin


def save_result(desktop, shots, undo=False):
    _, bounds = ready_control()
    if undo:
        control_button(desktop, bounds, 1)
        _, bounds = ready_control()
    control_button(desktop, bounds, 2)
    _, (x, y, width, height) = wait_for(lambda: window(EDITOR), "stitched image did not open")
    before = set(shots.glob("*.png"))
    desktop.click(x + width - 155, y + height - 55)
    output = wait_for(lambda: next(iter(set(shots.glob("*.png")) - before), None),
                      "could not save stitched PNG")
    desktop.click(x + width - 57, y + height - 55)
    wait_for(lambda: not window(EDITOR), "editor stayed open")
    return qa["pixbuf_png"](output.read_bytes())


def assert_pixels(image, width, height, header=False, flat=False):
    assert (image.get_width(), image.get_height()) == (width, height), (
        image.get_width(), image.get_height(), width, height)
    channels, stride, pixels = image.get_n_channels(), image.get_rowstride(), image.get_pixels()
    for y in range(height):
        for x in range(width):
            expected = (64, 64, 64) if flat else (
                HEADER if header and y < HEADER_HEIGHT else texture(x, y))
            base = y * stride + x * channels
            assert pixels[base:base + 3] == bytes(expected), (x, y, pixels[base:base+3], expected)


def verify_direct_outputs(desktop, shots):
    _, (x, y, width, height) = ready_control()
    button_width = (width - 40 - 16) / 3
    def center(index):
        return round(x + 20 + (button_width + 8) * index + button_width / 2)
    row = y + height - 128
    before = set(shots.glob("*.png"))
    desktop.click(center(1), row)
    saved = wait_for(lambda: next(iter(set(shots.glob("*.png")) - before), None),
                     "direct longshot save did not create PNG")
    assert_pixels(qa["pixbuf_png"](saved.read_bytes()), VIEW_WIDTH, VIEW_HEIGHT + PAGE_END, header=True)
    desktop.click(center(0), row)
    copied = wait_for(qa["clipboard_png"], "direct longshot copy did not publish PNG")
    assert_pixels(qa["pixbuf_png"](copied), VIEW_WIDTH, VIEW_HEIGHT + PAGE_END, header=True)
    desktop.click(center(2), row)
    pin_id, _ = wait_for(lambda: window(PIN), "direct longshot pin did not open")
    desktop.request_close(pin_id)
    wait_for(lambda: not window(PIN), "pin did not close")
    assert window(LONGSHOT) and not window(EDITOR)
    print("PASS direct longshot copy, atomic PNG and pinned image without opening the editor", flush=True)


def assert_no_late_scroll(page):
    count = len(page.events)
    end = time.monotonic() + .55
    while time.monotonic() < end:
        while GLib.MainContext.default().pending():
            GLib.MainContext.default().iteration(False)
        time.sleep(.02)
    assert len(page.events) == count, page.events


def run(binary, root):
    tray, desktop, page = qa["Tray"](), qa["Desktop"](), Page()
    data, shots = root / "data" / "com.clippy.qa.slint", root / "shots"
    data.mkdir(parents=True, mode=0o700)
    (data / "config.json").write_text(json.dumps({
        "auto_paste": False, "max_history": 200, "storage_mode": "memory", "theme": "dark",
        "screenshot_save_dir": str(shots), "screenshot_filename_template": "scroll.png",
    }))
    env = dict(os.environ, XDG_DATA_HOME=str(root / "data"), XDG_CONFIG_HOME=str(root / "config"))
    app = subprocess.Popen([binary], env=env)
    try:
        wait_for(lambda: tray.address, "no tray registration")
        assert not tray_contains(tray, TRAY_STOP)
        for direction, start, fixed, undo in (
            ("down", (0, 0), True, False), ("up", (0, PAGE_END), False, False),
            ("left", (PAGE_END, 0), False, False), ("right", (0, 0), False, True),
        ):
            page.prepare(start, header=fixed)
            origin = start_auto(tray, desktop, page, direction)
            ready_control()
            assert pointer_position() == origin, pointer_position()
            assert page.events == [direction] * (PAGE_END // STEP + 1), page.events
            assert not tray_contains(tray, TRAY_STOP)
            if direction == "down":
                verify_direct_outputs(desktop, shots)
            long = save_result(desktop, shots, undo=undo)
            extension = PAGE_END - STEP * int(undo)
            width = VIEW_WIDTH + (extension if direction in ("left", "right") else 0)
            height = VIEW_HEIGHT + (extension if direction in ("down", "up") else 0)
            assert_pixels(long, width, height, header=fixed)
            print(f"PASS automatic {direction}: exact {width}×{height} PNG and stopped at the page edge",
                  flush=True)

        page.prepare(flat=True)
        start_auto(tray, desktop, page, "down")
        ready_control()
        assert page.events == ["down"], page.events
        assert_pixels(save_result(desktop, shots), VIEW_WIDTH, VIEW_HEIGHT, flat=True)
        print("PASS low-texture scroll preserved the unmodified first frame", flush=True)

        page.prepare(delay=140)
        start_auto(tray, desktop, page, "down")
        wait_for(lambda: tray_contains(tray, TRAY_STOP), "no tray stop action during auto scroll")
        tray.activate(TRAY_STOP)
        identifier, _ = ready_control()
        assert_no_late_scroll(page)
        assert page.events == ["down"], page.events
        desktop.focus(identifier)
        desktop.keys("Escape")
        wait_for(lambda: not window(LONGSHOT), "cancelling the stopped capture kept the controls")
        print("PASS tray stop prevented delayed input and Escape cancelled the stopped session", flush=True)

        page.prepare(delay=140)
        start_auto(tray, desktop, page, "down")
        desktop.move(desktop.display, 0, 42, 62, 0)
        desktop.sync(desktop.display, 0)
        _, bounds = ready_control()
        assert_no_late_scroll(page)
        assert page.events == ["down"], page.events
        assert pointer_position() == (42, 62), pointer_position()
        control_button(desktop, bounds, 3)
        wait_for(lambda: not window(LONGSHOT), "pointer-interrupted capture did not close")
        print("PASS pointer takeover stopped the loop and preserved the user's pointer", flush=True)

        page.prepare(delay=140)
        start_auto(tray, desktop, page, "down")
        cover = Gtk.Window(title="Clippy QA replacement")
        cover.set_decorated(False)
        cover.set_default_size(VIEW_WIDTH, VIEW_HEIGHT)
        cover.move(page.x, page.y)
        received_by_cover = []
        cover.add_events(Gdk.EventMask.SCROLL_MASK)
        cover.connect("scroll-event", lambda _, event: received_by_cover.append(event.direction) or True)
        cover.show_all()
        try:
            _, bounds = ready_control()
            assert_no_late_scroll(page)
            assert page.events == ["down"], page.events
            assert not received_by_cover, received_by_cover
            control_button(desktop, bounds, 3)
            wait_for(lambda: not window(LONGSHOT), "interrupted capture did not close")
        finally:
            cover.destroy()
        print("PASS replacement window stopped automatic scrolling before the next input", flush=True)

        page.prepare(delay=140)
        start_auto(tray, desktop, page, "down")
        tray.activate("Quit")
        assert app.wait(timeout=6) == 0
        assert_no_late_scroll(page)
        assert page.events == ["down"], page.events
        print("PASS quitting during automatic capture revoked input before shutdown", flush=True)
    except Exception:
        qa["save_failure_frame"]()
        raise
    finally:
        if app.poll() is None:
            app.terminate()
            app.wait(timeout=5)
        page.window.destroy()
        desktop.close_display(desktop.display)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="clippy-slint-x11-longshot-") as temporary:
        run(str(Path(sys.argv[1]).resolve(strict=True)), Path(temporary))
