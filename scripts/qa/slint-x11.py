"""用独立 X11 server、SNI 主机和合成数据验收 QA 二进制。"""
import ctypes as c
from contextlib import contextmanager
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import struct
import subprocess
import sys
import tempfile
import time
import zlib

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
gi.require_version("GdkX11", "3.0")
from gi.repository import Gdk, GdkPixbuf, GdkX11, Gio, GLib, Gtk


TITLE = "Clippy Slint QA"
CAPTURE_TITLE = "Clippy Screenshot"
EDITOR_TITLE = "Clippy Image Editor"
VIEWER_TITLE = "Clippy Image Viewer"
PIN_TITLE = "Clippy Pin"
WORKSPACE_TITLE = "Clippy Pinned Workspace"
LONGSHOT_TITLE = "Clippy Scrolling Capture"
RECORDING_TITLE = "Clippy Recordings"
RECORDING_CONTROL_TITLE = "Clippy Recording Control"
SETTINGS_TITLE = "Clippy Settings"
LAUNCHER_TITLE = "Clippy Launcher"
TOOLS_TITLE = "Clippy Actions"
IMAGE_TEXT_TITLE = "Clippy Image Text"
TEXT = "needle 自动剪贴板 QA"
OTHER = "unrelated fixture"
HTML = "<b>rich-qa</b>".encode()
SENSITIVE = "password=synthetic-clipboard-preview-qa"
WATCHER = "org.kde.StatusNotifierWatcher"
ITEM = "org.kde.StatusNotifierItem"
MENU = "com.canonical.dbusmenu"
WATCHER_XML = """
<node><interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
</interface></node>
"""


def wait_for(predicate, description, timeout=8):
    end = time.monotonic() + timeout
    context = GLib.MainContext.default()
    while time.monotonic() < end:
        while context.pending():
            context.iteration(False)
        value = predicate()
        if value:
            return value
        time.sleep(0.025)
    raise AssertionError(description)


class ClientMessage(c.Structure):
    _fields_ = [
        ("type", c.c_int), ("serial", c.c_ulong), ("send_event", c.c_int),
        ("display", c.c_void_p), ("window", c.c_ulong), ("message_type", c.c_ulong),
        ("format", c.c_int), ("data", c.c_long * 5),
    ]


class Desktop:
    def __init__(self):
        self.lib = c.CDLL("libX11.so.6")
        xtst = c.CDLL("libXtst.so.6")
        self.lib.XOpenDisplay.restype = c.c_void_p
        self.lib.XOpenDisplay.argtypes = (c.c_char_p,)
        self.display = self.lib.XOpenDisplay(None)
        assert self.display

        def bind(lib, name, args, result=c.c_int):
            function = getattr(lib, name)
            function.argtypes = (c.c_void_p, *args)
            function.restype = result
            return function

        self.keysym = self.lib.XStringToKeysym
        self.keysym.argtypes = (c.c_char_p,)
        self.keysym.restype = c.c_ulong
        self.keycode = bind(self.lib, "XKeysymToKeycode", (c.c_ulong,), c.c_ubyte)
        self.focus_window = bind(self.lib, "XSetInputFocus", (c.c_ulong, c.c_int, c.c_ulong))
        self.sync = bind(self.lib, "XSync", (c.c_int,))
        self.close_display = bind(self.lib, "XCloseDisplay", ())
        self.root_window = bind(self.lib, "XDefaultRootWindow", (), c.c_ulong)
        self.grab_key = bind(self.lib, "XGrabKey", (c.c_int, c.c_uint, c.c_ulong, c.c_int, c.c_int, c.c_int))
        self.ungrab_key = bind(self.lib, "XUngrabKey", (c.c_int, c.c_uint, c.c_ulong))
        self.set_background = bind(self.lib, "XSetWindowBackground", (c.c_ulong, c.c_ulong))
        self.clear_window = bind(self.lib, "XClearWindow", (c.c_ulong,))
        self.atom = bind(self.lib, "XInternAtom", (c.c_char_p, c.c_int), c.c_ulong)
        self.send_event = bind(self.lib, "XSendEvent", (c.c_ulong, c.c_int, c.c_long, c.c_void_p))
        self.create_gc = bind(self.lib, "XCreateGC", (c.c_ulong, c.c_ulong, c.c_void_p), c.c_void_p)
        self.free_gc = bind(self.lib, "XFreeGC", (c.c_void_p,))
        self.foreground = bind(self.lib, "XSetForeground", (c.c_void_p, c.c_ulong))
        self.fill = bind(self.lib, "XFillRectangle",
                         (c.c_ulong, c.c_void_p, c.c_int, c.c_int, c.c_uint, c.c_uint))
        self.key = bind(xtst, "XTestFakeKeyEvent", (c.c_uint, c.c_int, c.c_ulong))
        self.move = bind(xtst, "XTestFakeMotionEvent", (c.c_int, c.c_int, c.c_int, c.c_ulong))
        self.button = bind(xtst, "XTestFakeButtonEvent", (c.c_uint, c.c_int, c.c_ulong))

    def keys(self, *keys):
        codes = [self.keycode(self.display, self.keysym(key.encode())) for key in keys]
        assert all(codes), keys
        for code in codes:
            self.key(self.display, code, 1, 0)
        for code in reversed(codes):
            self.key(self.display, code, 0, 0)
        self.sync(self.display, 0)

    @contextmanager
    def occupy_control_key(self, key):
        code = self.keycode(self.display, self.keysym(key.encode()))
        root = self.root_window(self.display)
        self.grab_key(self.display, code, 4, root, 0, 1, 1)
        self.sync(self.display, 0)
        try:
            yield
        finally:
            self.ungrab_key(self.display, code, 4, root)
            self.sync(self.display, 0)

    def focus(self, window):
        self.focus_window(self.display, window, 2, 0)
        self.sync(self.display, 0)

    def request_close(self, window):
        storage = (c.c_long * 24)()
        message = ClientMessage.from_buffer(storage)
        message.type = 33
        message.display = self.display
        message.window = window
        message.message_type = self.atom(self.display, b"WM_PROTOCOLS", 0)
        message.format = 32
        message.data[0] = self.atom(self.display, b"WM_DELETE_WINDOW", 0)
        assert self.send_event(self.display, window, 0, 0, c.byref(storage))
        self.sync(self.display, 0)

    def click(self, x, y):
        self.move(self.display, 0, x, y, 0)
        self.sync(self.display, 0)
        time.sleep(0.04)
        self.button(self.display, 1, 1, 0)
        self.sync(self.display, 0)
        time.sleep(0.04)
        self.button(self.display, 1, 0, 0)
        self.sync(self.display, 0)

    def drag(self, left, top, right, bottom):
        self.move(self.display, 0, left, top, 0)
        self.button(self.display, 1, 1, 0)
        self.sync(self.display, 0)
        self.move(self.display, 0, right, bottom, 0)
        self.sync(self.display, 0)
        self.button(self.display, 1, 0, 0)
        self.sync(self.display, 0)

    def scroll_down(self, x, y):
        self.move(self.display, 0, x, y, 0)
        for _ in range(16):
            self.button(self.display, 5, 1, 0)
            self.button(self.display, 5, 0, 0)
        self.sync(self.display, 0)
        time.sleep(0.25)

    def background(self, rgb):
        root = self.root_window(self.display)
        self.set_background(self.display, root, rgb)
        self.clear_window(self.display, root)
        self.sync(self.display, 0)

    def scroll_fixture(self, offset):
        root = self.root_window(self.display)
        gc = self.create_gc(self.display, root, 0, None)
        for y in range(0, 100, 4):
            for x in range(0, 140, 4):
                row = (y + offset) // 4
                color = ((row * 10007 + x * x * 31337) ^ (x * row * 46273)) & 0xFFFFFF
                self.foreground(self.display, gc, color)
                self.fill(self.display, root, gc, 440 + x, 200 + y, 4, 4)
        self.free_gc(self.display, gc)
        self.sync(self.display, 0)


class Tray:
    def __init__(self):
        self.connection = Gio.bus_get_sync(Gio.BusType.SESSION)
        self.address = None
        self.connection.register_object(
            "/StatusNotifierWatcher", Gio.DBusNodeInfo.new_for_xml(WATCHER_XML).interfaces[0],
            self.register_item, self.get_property, None,
        )
        assert self.call(
            "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
            "RequestName", GLib.Variant("(su)", (WATCHER, 0)),
        ) == (1,)

    def call(self, service, path, interface, method, args):
        return self.connection.call_sync(
            service, path, interface, method, args, None, Gio.DBusCallFlags.NONE, 3000, None,
        ).unpack()

    def register_item(self, connection, sender, path, interface, name, params, invocation):
        assert name == "RegisterStatusNotifierItem"
        (item,) = params.unpack()
        self.address = (sender, item) if item.startswith("/") else (item, "/StatusNotifierItem")
        invocation.return_value(GLib.Variant("()", ()))

    def get_property(self, connection, sender, path, interface, prop):
        if prop == "IsStatusNotifierHostRegistered":
            return GLib.Variant("b", True)
        return GLib.Variant("i", 0)

    def activate(self, label):
        service, path = self.address
        (menu_path,) = self.call(service, path, "org.freedesktop.DBus.Properties", "Get",
                                 GLib.Variant("(ss)", (ITEM, "Menu")))
        _, (_, _, children) = self.call(service, menu_path, MENU, "GetLayout",
                                       GLib.Variant("(iias)", (0, 1, ["label"])))
        item, = [number for number, props, _ in children if props.get("label") == label]
        self.call(service, menu_path, MENU, "Event",
                  GLib.Variant("(isvu)", (item, "clicked", GLib.Variant("i", 0), 0)))


def titled_window(title):
    # X11 窗口在枚举子树时可能刚好销毁，此时重新读取整棵树。
    for _ in range(4):
        tree = subprocess.run(["xwininfo", "-root", "-tree"], capture_output=True, text=True)
        if tree.returncode == 0 or not any(
            error in tree.stderr for error in ("Bad Drawable", "Bad Window")
        ):
            break
        time.sleep(0.025)
    tree.check_returncode()
    match = re.search(r'\b(0x[\da-f]+)\s+"' + re.escape(title) + '"', tree.stdout)
    if match is None:
        return None
    window = int(match[1], 16)
    info = subprocess.run(["xwininfo", "-id", str(window)], capture_output=True, text=True)
    if info.returncode != 0:
        return None
    geometry = info.stdout
    if "Map State: IsViewable" not in geometry:
        return None
    bounds = tuple(int(re.search(key + r":\s*(-?\d+)", geometry)[1])
                   for key in ("Absolute upper-left X", "Absolute upper-left Y", "Width", "Height"))
    return window, bounds


def set_clipboard(target, data):
    subprocess.run(["xclip", "-selection", "clipboard", "-in", "-target", target],
                   input=data, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)


def get_clipboard(target):
    return subprocess.check_output(
        ["xclip", "-selection", "clipboard", "-out", "-target", target], timeout=3,
    )


def clipboard_png():
    result = subprocess.run(
        ["xclip", "-selection", "clipboard", "-out", "-target", "image/png"],
        capture_output=True, timeout=3,
    )
    return result.stdout if result.returncode == 0 and result.stdout.startswith(b"\x89PNG\r\n\x1a\n") else None


def decode_png(data):
    pixbuf = pixbuf_png(data)
    return (pixbuf.get_width(), pixbuf.get_height()), pixbuf.get_pixels()[:3]


def pixbuf_png(data):
    loader = GdkPixbuf.PixbufLoader.new_with_type("png")
    loader.write(data)
    loader.close()
    return loader.get_pixbuf()

def capture_button_width(toolbar_width):
    return min(32, (toolbar_width - 24) / 12)


def capture_command_x(left, width, index):
    return round(left + 15 + (9 + index + 0.5) * capture_button_width(width))



def save_failure_frame():
    directory = os.environ.get("CLIPPY_SLINT_QA_ARTIFACTS")
    if directory:
        root = Gdk.get_default_root_window()
        pixbuf = Gdk.pixbuf_get_from_window(root, 0, 0, root.get_width(), root.get_height())
        pixbuf.savev(str(Path(directory) / "slint-x11-failure.png"), "png", [], [])


def save_qa_window(name, bounds):
    directory = os.environ.get("CLIPPY_SLINT_QA_ARTIFACTS")
    if directory:
        pixbuf = Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), *bounds)
        pixbuf.savev(str(Path(directory) / name), "png", [], [])


def rgba_png():
    def chunk(name, data):
        return struct.pack(">I", len(data)) + name + data + struct.pack(">I", zlib.crc32(name + data))

    pixels = bytes((255, 0, 0, 128, 0, 128, 255, 0))
    header = struct.pack(">IIBBBBB", 2, 1, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(b"\0" + pixels)) + chunk(b"IEND", b"")


def run(binary, root):
    tray = Tray()
    desktop = Desktop()
    target = Gtk.Window(title="Slint Paste Target")
    target.set_default_size(360, 80)
    entry = Gtk.Entry()
    target.add(entry)
    target.show_all()
    entry.grab_focus()
    target_id = GdkX11.X11Window.get_xid(target.get_window())
    env = dict(os.environ, XDG_DATA_HOME=str(root / "data"), XDG_CONFIG_HOME=str(root / "config"),
               XDG_PICTURES_DIR=str(root / "Pictures"), NO_AT_BRIDGE="1")
    data_dir = root / "data" / "com.clippy.qa.slint"
    data_dir.mkdir(parents=True, mode=0o700)
    theme = os.environ.get("CLIPPY_SLINT_QA_THEME", "light")
    # 自动粘贴在全链路用例中显式开启，普通 QA 安装仍使用首次启动默认值。
    (data_dir / "config.json").write_text(json.dumps({
        "auto_paste": True, "max_history": 200, "storage_mode": "disk", "theme": theme,
        "global_shortcut": "Ctrl+Alt+Shift+J", "capture_shortcut": "Ctrl+Alt+Shift+K",
        "screenshot_save_dir": str(root / "shots"), "screenshot_filename_template": "fixture.png",
    }))
    process = subprocess.Popen([binary], env=env)

    def rows():
        assert process.poll() is None, "QA application exited"
        database = data_dir / "clips.db"
        if not database.exists():
            return []
        with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
            try:
                return connection.execute(
                    "SELECT content_type, text_content, html_content FROM clips ORDER BY use_order"
                ).fetchall()
            except sqlite3.OperationalError as error:
                if "no such table" not in str(error):
                    raise
                return []

    def panel():
        return wait_for(lambda: titled_window(TITLE), "clipboard panel never appeared")

    def panel_width(width):
        def match():
            window = titled_window(TITLE)
            return window if window and window[1][2:] == (width, 500) else None

        return wait_for(match, f"clipboard panel did not resize to {width}×500")

    def preview_pixels(box):
        x, y, width, _ = box
        root_window = Gdk.get_default_root_window()
        image = Gdk.pixbuf_get_from_window(root_window, x + width - 308, y + 152, 218, 1)
        if image is None:
            return False
        pixels = image.get_pixels()
        background = (38, 42, 50) if theme == "dark" else (241, 241, 246)
        red_sample = tuple((bg + rgba) // 2 for bg, rgba in zip(background, (255, 0, 0)))
        return all(abs(actual - expected) < 19 for actual, expected in zip(pixels[:3], red_sample)) and all(
            abs(actual - expected) < 12 for actual, expected in zip(pixels[-3:], background)
        )

    def row_thumbnail(box):
        x, y, _, _ = box
        pixel = Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), x + 22, y + 119, 1, 1)
        if pixel:
            red, green, blue = pixel.get_pixels()[:3]
            return red - max(green, blue) >= 90 and red > 120
        return False

    def check_titlebar(bounds, name, color=None):
        x, y, width, _ = bounds
        expected = bytes.fromhex(color or ("20232a" if theme == "dark" else "ffffff"))

        def correct_color():
            pixel = Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), x + width // 2, y + 8, 1, 1)
            return pixel and pixel.get_pixels()[:3] == expected

        wait_for(correct_color, f"{name} did not use the shared {theme} titlebar")
        save_qa_window(f"{name}-{theme}.png", bounds)

    def search(word):
        desktop.keys("Control_L", "f")
        desktop.keys("Control_L", "a")
        for letter in word:
            desktop.keys(letter)
        # 输入事件和 SQLite 返回跨两个线程，等查询结果进入页面再点击。
        time.sleep(0.25)

    def launcher_search(word):
        desktop.keys("Control_L", "a")
        for letter in word:
            desktop.keys("space" if letter == " " else letter)

    def close():
        panel_id, _ = panel()
        desktop.focus(panel_id)
        desktop.keys("Escape")
        time.sleep(0.1)
        if titled_window(TITLE):
            desktop.keys("Escape")
        wait_for(lambda: not titled_window(TITLE), "panel never closed")

    def select_area(near_bottom=False):
        overlay, (x, y, width, height) = wait_for(
            lambda: titled_window(CAPTURE_TITLE), "screenshot overlay never appeared",
        )
        desktop.focus(overlay)
        top = height - 120 if near_bottom else 200
        desktop.drag(x + 440, y + top, x + 580, y + top + 100)
        toolbar_width = min(560, width - 16)
        toolbar_left = x + max(8, min(580 - toolbar_width, width - toolbar_width - 8))
        toolbar_top = y + (top - 84 if near_bottom else top + 108)

        def toolbar_visible():
            pixel = Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), toolbar_left + 4, toolbar_top + 22, 1, 1)
            return pixel and pixel.get_pixels()[:3] == bytes.fromhex("202124")

        wait_for(toolbar_visible, "selection toolbar did not appear beside the selected region")
        chip_top = toolbar_top - 26 if near_bottom else y + top - 26

        def dimensions_visible():
            pixel = Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), x + 444, chip_top + 11, 1, 1)
            return pixel and pixel.get_pixels()[:3] == bytes.fromhex("202124")

        wait_for(dimensions_visible, "the toolbar obscured the selection dimensions")
        if near_bottom:
            save_qa_window("capture-bottom-toolbar.png", (x, y, width, height))
        return toolbar_left, toolbar_top, toolbar_width

    def click_capture_action(toolbar, index):
        x, y, width = toolbar
        button = capture_button_width(width)
        if index in (2, 3, 4):
            desktop.click(capture_command_x(x, width, index - 2), y + 19)
        elif index == 5:
            desktop.click(round(x + 6 + 1.5 * button), y + 19)
        else:
            right = {0: 1, 1: 2, 6: 5, 7: 0, 8: 4, 9: 3}[index]
            desktop.click(round(x + width - 6 - (right + 0.5) * button), y + 57)

    try:
        wait_for(lambda: tray.address, "no tray registration")
        assert not titled_window(TITLE), "tray-only startup opened a window"
        tray.activate("Open Clipboard")
        _, box = panel()
        assert box[2:] == (380, 500), box
        close()
        print("PASS tray registration, menu and window lifecycle", flush=True)

        tray.activate("Settings")
        _, (x, y, width, height) = wait_for(lambda: titled_window(SETTINGS_TITLE), "settings window never appeared")
        assert (width, height) == (720, 560), (width, height)
        desktop.click(x + width - 28, y + 26)
        wait_for(lambda: not titled_window(SETTINGS_TITLE), "settings titlebar close did not close the window")
        print("PASS settings window has its own size and native titlebar close", flush=True)

        tray.activate("Actions")
        launcher_id, bounds = wait_for(lambda: titled_window(LAUNCHER_TITLE), "tray launcher never appeared")
        assert bounds[2:] == (640, 520), bounds
        desktop.focus(launcher_id)
        launcher_search("missingaction")
        desktop.keys("Return")
        assert titled_window(LAUNCHER_TITLE), "an empty search ran a command"
        launcher_search("settings")
        desktop.keys("Return")
        settings_id, _ = wait_for(lambda: titled_window(SETTINGS_TITLE), "launcher did not open settings")
        wait_for(lambda: not titled_window(LAUNCHER_TITLE), "launcher stayed open after choosing settings")
        desktop.request_close(settings_id)
        wait_for(lambda: not titled_window(SETTINGS_TITLE), "launcher settings did not close")

        set_clipboard("UTF8_STRING", TEXT.encode())
        assert get_clipboard("UTF8_STRING").decode() == TEXT
        try:
            wait_for(lambda: ("text", TEXT, None) in rows(), "Unicode did not reach the history")
        except AssertionError:
            print(f"History fixture rows: {rows()!r}", file=sys.stderr)
            raise
        set_clipboard("text/html", HTML)
        wait_for(lambda: ("html", "rich-qa", HTML.decode()) in rows(), "HTML did not reach the history")
        set_clipboard("image/png", rgba_png())
        wait_for(lambda: ("image", None, None) in rows(), "transparent PNG did not reach the history")
        with sqlite3.connect(data_dir / "clips.db") as connection:
            png, = connection.execute("SELECT image_data FROM clips WHERE content_type='image'").fetchone()
            loader = GdkPixbuf.PixbufLoader.new_with_type("png")
            loader.write(png)
            loader.close()
            assert loader.get_pixbuf().get_pixels()[:8] == bytes((255, 0, 0, 128, 0, 128, 255, 0))
        set_clipboard("UTF8_STRING", OTHER.encode())
        wait_for(lambda: ("text", OTHER, None) in rows(), "second text did not reach the history")

        tray.activate("Open Clipboard")
        panel_id, compact_bounds = panel_width(380)
        x, y, _, _ = compact_bounds
        desktop.focus(panel_id)
        wait_for(lambda: row_thumbnail(compact_bounds), "transparent PNG thumbnail did not appear in the list")
        save_qa_window("history-list-thumbnail.png", compact_bounds)
        desktop.click(x + 356, y + 42)
        desktop.click(x + 247, y + 42)
        panel_width(780)
        desktop.keys("Tab")
        panel_width(380)
        desktop.keys("Tab")
        _, expanded = panel_width(780)
        desktop.keys("Down")
        wait_for(lambda: preview_pixels(expanded), "preview did not render the transparent PNG")
        if os.environ.get("CLIPPY_SLINT_QA_ARTIFACTS"):
            x, y, width, height = expanded
            preview = Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), x, y, width, height)
            preview.savev(str(Path(os.environ["CLIPPY_SLINT_QA_ARTIFACTS"]) / "slint-history-image-preview.png"), "png", [], [])

        desktop.keys("Down")
        x, y, width, height = expanded
        time.sleep(0.2)
        desktop.click(x + width - 330, y + 57)
        desktop.keys("Control_L", "a")
        desktop.keys("Control_L", "c")
        wait_for(lambda: get_clipboard("UTF8_STRING").decode() == "rich-qa",
                 "rich preview did not allow selecting and copying text")
        save_qa_window("slint-history-rich-preview.png", expanded)
        wait_for(lambda: ("text", "rich-qa", None) in rows(), "rich selection did not reach clipboard history")
        time.sleep(0.1)
        desktop.keys("Escape")
        panel_width(380)
        desktop.keys("Tab")
        panel_width(780)
        desktop.click(x + width - 330, y + 57)
        desktop.keys("Control_L", "k")
        launcher_id, _ = wait_for(lambda: titled_window(LAUNCHER_TITLE), "Ctrl+K did not reach the launcher from preview text")
        desktop.focus(launcher_id)
        desktop.keys("Escape")
        wait_for(lambda: not titled_window(LAUNCHER_TITLE), "launcher did not close from preview")
        assert panel_width(780)[0] == panel_id, "opening the launcher replaced the preview"
        desktop.focus(panel_id)
        time.sleep(0.1)
        desktop.keys("Escape")
        panel_width(380)
        desktop.keys("Tab")
        _, (x, y, width, height) = panel_width(780)
        # 历史可能因上一次复制重排。搜索固定夹具来核对侧栏复制仍跟随当前选项。
        search("needle")
        desktop.click(x + 52, y + 79)
        assert not row_thumbnail((x, y, width, height)), "image thumbnail followed a text-only search"
        time.sleep(0.2)
        desktop.click(x + width - 150, y + height - 25)
        wait_for(lambda: get_clipboard("UTF8_STRING").decode() == TEXT,
                 "side panel Copy did not use the selected Unicode row")
        assert entry.get_text() == "", "Preview Copy injected paste input"

        set_clipboard("UTF8_STRING", SENSITIVE.encode())
        wait_for(lambda: ("text", SENSITIVE, None) in rows(), "sensitive fixture did not reach the history")
        search("synthetic")
        desktop.click(x + 52, y + 79)
        set_clipboard("UTF8_STRING", OTHER.encode())
        time.sleep(0.2)
        desktop.click(x + width - 150, y + height - 25)
        assert get_clipboard("UTF8_STRING").decode() == OTHER, "protected preview exposed a Copy action"
        desktop.click(x + width - 24, y + 25)
        panel_width(380)
        close()
        print("PASS history preview resize, Unicode/HTML/image selection, protected row and close", flush=True)
        print("PASS visible history PNG thumbnail and text-only search", flush=True)

        tray.activate("Actions")
        launcher_id, _ = wait_for(lambda: titled_window(LAUNCHER_TITLE), "clipboard image launcher never appeared")
        desktop.focus(launcher_id)
        launcher_search("image")
        desktop.keys("Down")
        desktop.keys("Up")
        desktop.keys("Return")
        _, image_text_bounds = wait_for(lambda: titled_window(IMAGE_TEXT_TITLE), "launcher did not open image analysis")
        wait_for(lambda: not titled_window(LAUNCHER_TITLE), "launcher stayed open after beginning image analysis")
        check_titlebar(image_text_bounds, "image-text")
        image_x, image_y, image_width, _ = image_text_bounds
        desktop.click(image_x + image_width - 28, image_y + 26)
        wait_for(lambda: not titled_window(IMAGE_TEXT_TITLE), "image analysis did not close")

        tray.activate("Open Clipboard")
        history_id, (x, y, _, _) = panel_width(380)
        desktop.focus(history_id)
        search("needle")
        desktop.click(x + 48, y + 81)
        desktop.keys("Control_L", "k")
        launcher_id, _ = wait_for(lambda: titled_window(LAUNCHER_TITLE), "Ctrl+K did not open the launcher")
        desktop.focus(launcher_id)
        launcher_search("ocr")
        desktop.keys("Return")
        assert not titled_window(IMAGE_TEXT_TITLE), "text selection unexpectedly loaded a different image"
        desktop.keys("Escape")
        wait_for(lambda: not titled_window(LAUNCHER_TITLE), "Escape did not dismiss the launcher")
        assert panel()[0] == history_id, "dismissing the launcher replaced clipboard history"
        desktop.focus(history_id)
        desktop.keys("Control_L", "k")
        launcher_id, _ = wait_for(lambda: titled_window(LAUNCHER_TITLE), "launcher did not reopen")
        desktop.focus(launcher_id)
        launcher_search("codec")
        desktop.keys("Return")
        tools_id, tools_bounds = wait_for(lambda: titled_window(TOOLS_TITLE), "launcher did not open the text tools")
        x, y, width, height = tools_bounds
        desktop.focus(tools_id)
        check_titlebar(tools_bounds, "text-tools")

        def select_text_pane(left, expected):
            desktop.click(x + (80 if left else width // 2 + 80), y + 159)
            desktop.keys("Control_L", "a")
            desktop.keys("Control_L", "c")
            wait_for(lambda: get_clipboard("UTF8_STRING").decode() == expected,
                     f"text tools did not expose the expected {'source' if left else 'result'}")

        select_text_pane(True, TEXT)
        sample = '{"mode":"local","version":2}'
        set_clipboard("UTF8_STRING", sample.encode())
        desktop.keys("Control_L", "a")
        desktop.keys("Control_L", "v")
        desktop.click(x + 81, y + height - 55)
        desktop.keys("Down")
        desktop.keys("Down")
        desktop.keys("Return")
        desktop.keys("Tab")
        desktop.keys("Return")
        select_text_pane(False, json.dumps(json.loads(sample), indent=2))

        set_clipboard("UTF8_STRING", b"# Native markup\n\n**bold QA** and `code`.")
        desktop.click(x + 80, y + 159)
        desktop.keys("Control_L", "a")
        desktop.keys("Control_L", "v")
        desktop.click(x + 81, y + height - 55)
        desktop.keys("Up")
        desktop.keys("Return")
        desktop.keys("Tab")
        desktop.keys("Return")
        select_text_pane(False, "Native markup\nbold QA and code.")
        save_qa_window("text-tools-markdown.png", tools_bounds)

        with sqlite3.connect(data_dir / "clips.db") as connection:
            connection.execute(
                "INSERT INTO translation_history "
                "(provider, source_language, target_language, source_hash, source_text, translated_text, created_at) "
                "VALUES ('libretranslate', 'en', 'zh-CN', 'qa-translation', ?, ?, ?)",
                ("History source", "历史译文", int(time.time())),
            )
        desktop.click(x + width - 78, y + 78)
        time.sleep(0.25)
        desktop.click(x + 100, y + 130)
        select_text_pane(True, "History source")
        select_text_pane(False, "历史译文")
        save_qa_window("text-tools-translation-history.png", tools_bounds)
        desktop.click(x + width - 28, y + 26)
        wait_for(lambda: not titled_window(TOOLS_TITLE), "text tools did not close")
        print("PASS launcher tray and keyboard, search, selection, image analysis and selected text", flush=True)
        print("PASS side-by-side text panes, local formatting and restored translation history", flush=True)
        print("PASS native HTML and Markdown typography with selectable text", flush=True)

        desktop.focus(target_id)
        desktop.keys("Control_L", "Alt_L", "Shift_L", "j")
        panel_id, box = panel()
        desktop.focus(panel_id)
        search("needle")
        x, y, _, _ = box
        desktop.click(x + 50, y + 82)
        desktop.keys("Control_L", "c")
        try:
            wait_for(lambda: get_clipboard("UTF8_STRING").decode() == TEXT,
                     "search and copy did not select Unicode")
        except AssertionError:
            print(f"Clipboard fixture output: {get_clipboard('UTF8_STRING')!r}", file=sys.stderr)
            raise
        assert entry.get_text() == "", "Copy injected paste input"
        close()
        print("PASS global hotkey, Unicode/HTML/PNG history, search and copy-only", flush=True)

        desktop.focus(target_id)
        desktop.keys("Control_L", "Alt_L", "Shift_L", "j")
        panel_id, box = panel()
        desktop.focus(panel_id)
        search("needle")
        desktop.keys("Return")
        wait_for(lambda: not titled_window(TITLE), "panel stayed open after paste")
        # Xvfb 没有窗口管理器，夹具在面板撤下后恢复焦点。
        desktop.focus(target_id)
        wait_for(lambda: entry.get_text() == TEXT, "target did not receive Unicode via Ctrl+V")
        print("PASS automatic paste into the focused GTK application", flush=True)

        third = Gtk.Window(title="Slint Unrelated Window")
        third_entry = Gtk.Entry()
        third.add(third_entry)
        third.show_all()
        third_entry.grab_focus()
        third_id = GdkX11.X11Window.get_xid(third.get_window())
        desktop.focus(target_id)
        desktop.keys("Control_L", "Alt_L", "Shift_L", "j")
        panel_id, box = panel()
        desktop.focus(panel_id)
        search("needle")
        desktop.keys("Return")
        wait_for(lambda: not titled_window(TITLE), "panel stayed open after requesting paste")
        desktop.focus(third_id)
        # 超过应用等待焦点的截止，确认两个输入框都没有收到追加的粘贴。
        deadline = time.monotonic() + 0.6
        while time.monotonic() < deadline:
            while GLib.MainContext.default().pending():
                GLib.MainContext.default().iteration(False)
            time.sleep(0.025)
        assert third_entry.get_text() == "" and entry.get_text() == TEXT
        assert get_clipboard("UTF8_STRING").decode() == TEXT
        third.destroy()
        print("PASS changed focus preserved the copy and suppressed input to both windows", flush=True)

        desktop.background(0x224466)
        desktop.keys("Control_L", "Alt_L", "Shift_L", "k")
        select_area()
        desktop.keys("Return")
        png = wait_for(clipboard_png, "screenshot copy did not publish PNG")
        expected = ((140, 100), bytes.fromhex("224466"))
        assert decode_png(png) == expected, decode_png(png)
        plain_pixels = pixbuf_png(png).get_pixels()
        wait_for(lambda: not titled_window(CAPTURE_TITLE), "overlay stayed open after copy")

        tray.activate("Screenshot")
        select_area()
        overlay, (screen_x, screen_y, _, _) = titled_window(CAPTURE_TITLE)
        screen = Gdk.get_default_root_window()
        inside = Gdk.pixbuf_get_from_window(screen, screen_x + 500, screen_y + 250, 1, 1)
        outside = Gdk.pixbuf_get_from_window(screen, screen_x + 500, screen_y + 150, 1, 1)
        assert inside.get_pixels()[:3] == expected[1]
        assert all(0 < value < original for value, original in zip(outside.get_pixels()[:3], expected[1]))
        desktop.drag(screen_x + 580, screen_y + 300, screen_x + 614, screen_y + 321)
        desktop.drag(screen_x + 508, screen_y + 251, screen_x + 549, screen_y + 266)
        desktop.focus(overlay)
        desktop.keys("Return")
        wait_for(lambda: (data := clipboard_png()) and decode_png(data)[0] == (174, 121),
                 "resized and moved capture did not preserve the new pixel bounds")
        print("PASS capture shade, resize handle, selection drag and exact pixel bounds", flush=True)

        tray.activate("Screenshot")
        click_capture_action(select_area(), 1)
        output = root / "shots" / "fixture.png"
        wait_for(output.is_file, "screenshot file was not committed")
        assert decode_png(output.read_bytes()) == expected
        assert list(output.parent.iterdir()) == [output]
        print("PASS screenshot hotkey and tray, native pixels, crop, clipboard and atomic file", flush=True)

        tray.activate("Screenshot")
        click_capture_action(select_area(near_bottom=True), 7)
        wait_for(lambda: not titled_window(CAPTURE_TITLE), "cancel did not close overlay")
        assert list(output.parent.iterdir()) == [output]
        print("PASS selection toolbar and dimensions stay visible near the bottom edge; cancel preserves the file", flush=True)

        def edited_clipboard():
            png = clipboard_png()
            if png:
                pixbuf = pixbuf_png(png)
                if b"\xff\x3b\x30" in pixbuf.get_pixels():
                    return pixbuf
            return None

        def mark_selection(undo=False):
            tray.activate("Screenshot")
            toolbar = select_area()
            _, (left, top, width, height) = titled_window(CAPTURE_TITLE)
            click_capture_action(toolbar, 5)

            def draw_rectangle():
                desktop.drag(left + 460, top + 220, left + 530, top + 274)

            def border_pixel():
                pixel = Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), left + 475, top + 221, 1, 1)
                return pixel.get_pixels()[:3] if pixel else b""

            draw_rectangle()
            wait_for(lambda: border_pixel() == bytes.fromhex("ff3b30"), "selection did not display its rectangle")
            if undo:
                click_capture_action(toolbar, 6)
                wait_for(lambda: border_pixel() == expected[1], "undo did not remove the selection rectangle")
                desktop.keys("Control_L", "Shift_L", "z")
                wait_for(lambda: border_pixel() == bytes.fromhex("ff3b30"), "Ctrl+Shift+Z did not redo the rectangle")
                desktop.keys("Control_L", "z")
                wait_for(lambda: border_pixel() == expected[1], "Ctrl+Z did not undo the redone rectangle")
                desktop.keys("Control_L", "y")
                wait_for(lambda: border_pixel() == bytes.fromhex("ff3b30"), "Ctrl+Y did not redo the rectangle")
                desktop.keys("Control_L", "z")
                wait_for(lambda: border_pixel() == expected[1], "rectangle stayed visible after undo")
                tx, ty, _ = toolbar
                desktop.click(tx + 150, ty + 19)
                desktop.click(tx + 35, ty + 57)
                desktop.drag(left + 522, top + 248, left + 466, top + 248)

                def yellow_arrow():
                    band = Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), left + 493, top + 246, 5, 6)
                    return band and bytes.fromhex("ffcf36") in band.get_pixels()

                wait_for(yellow_arrow, "selection did not display the horizontal arrow with the chosen color")
                click_capture_action(toolbar, 6)
                wait_for(lambda: not yellow_arrow(), "undo did not remove the arrow")
                click_capture_action(toolbar, 8)
                wait_for(yellow_arrow, "toolbar redo did not restore the arrow")
                desktop.keys("Control_L", "y")
                time.sleep(0.1)
                assert border_pixel() == expected[1], "new arrow did not replace the old redo history"
                click_capture_action(toolbar, 6)
                wait_for(lambda: not yellow_arrow(), "redone arrow stayed visible after undo")
                click_capture_action(toolbar, 9)
                desktop.keys("Control_L", "y")
                time.sleep(0.1)
                assert not yellow_arrow(), "clearing the selection marks left a redo entry"
                desktop.click(tx + 16, ty + 57)
                click_capture_action(toolbar, 5)
                draw_rectangle()
                wait_for(lambda: border_pixel() == bytes.fromhex("ff3b30"), "selection did not accept another rectangle")
                click_capture_action(toolbar, 6)
                wait_for(lambda: border_pixel() == expected[1], "undo did not remove the replacement rectangle")
                click_capture_action(toolbar, 8)
                wait_for(lambda: border_pixel() == bytes.fromhex("ff3b30"), "toolbar redo did not restore the replacement rectangle")
            save_qa_window("capture-rectangle.png", (left, top, width, height))
            return toolbar

        click_capture_action(mark_selection(undo=True), 0)
        marked_copy = wait_for(edited_clipboard, "selection copy lost the rectangle")
        assert (marked_copy.get_width(), marked_copy.get_height()) == expected[0]
        click_capture_action(mark_selection(), 1)
        marked_file = output.with_name("fixture-2.png")
        wait_for(marked_file.is_file, "marked PNG was not committed")
        assert pixbuf_png(marked_file.read_bytes()).get_pixels() == marked_copy.get_pixels()

        click_capture_action(mark_selection(), 2)
        _, (x, y, width, height) = wait_for(lambda: titled_window(EDITOR_TITLE), "editor never opened")
        check_titlebar((x, y, width, height), "editor")
        set_clipboard("UTF8_STRING", OTHER.encode())
        desktop.click(x + width - 253, y + height - 55)
        assert wait_for(edited_clipboard, "editor did not restore the selection markup").get_pixels() == marked_copy.get_pixels()
        desktop.click(x + width - 110, y + 80)
        time.sleep(0.3)
        desktop.click(x + width - 253, y + height - 55)

        def plain_clipboard():
            png = clipboard_png()
            return png and pixbuf_png(png).get_pixels() == plain_pixels

        wait_for(plain_clipboard, "editor could not undo the inherited rectangle")
        print("PASS selection undo/redo shortcuts and toolbar, matching clipboard/PNG pixels and editable transfer", flush=True)

        desktop.drag(x + width // 3, y + height // 3, x + width * 2 // 3, y + height * 2 // 3)
        # 一次标注会在工作线程合成，更改入画后才能提交。
        time.sleep(0.3)
        desktop.click(x + width - 253, y + height - 55)
        edited_copy = wait_for(edited_clipboard, "editor copy did not contain the red rectangle")
        desktop.click(x + width - 155, y + height - 55)
        edited_file = output.with_name("fixture-3.png")
        wait_for(edited_file.is_file, "edited PNG was not committed")
        assert pixbuf_png(edited_file.read_bytes()).get_pixels() == edited_copy.get_pixels()
        assert (edited_copy.get_width(), edited_copy.get_height()) == expected[0]
        desktop.click(x + width - 67, y + height - 26)
        pin_id, (pin_x, pin_y, pin_width, pin_height) = wait_for(
            lambda: titled_window(PIN_TITLE), "edited image did not open as a pin",
        )
        with sqlite3.connect(data_dir / "clips.db") as connection:
            annotations, = connection.execute(
                "SELECT r.annotations_json FROM image_revisions r "
                "JOIN pin_workspace_items p ON p.revision_id = r.id",
            ).fetchone()
            assert json.loads(annotations)[0]["type"] == "rect"
        desktop.click(x + width - 57, y + height - 55)
        wait_for(lambda: not titled_window(EDITOR_TITLE), "editor stayed open after close")
        print("PASS screenshot editor and identical copy/save pixels", flush=True)

        def pin_presentation():
            with sqlite3.connect(data_dir / "clips.db") as conn:
                return conn.execute("SELECT scale, opacity, locked, above FROM pin_workspace_items").fetchone()

        desktop.focus(pin_id)
        desktop.click(pin_x + pin_width - 26, pin_y + 138)
        locked = wait_for(lambda: pin_presentation()[2] == 1 and pin_presentation(),
                          "pin size lock was not saved")
        desktop.click(pin_x + pin_width - 26, pin_y + 25)
        assert pin_presentation()[0] == locked[0], "locked pin accepted zoom"
        desktop.click(pin_x + pin_width - 26, pin_y + 198)
        desktop.click(pin_x + pin_width - 185, pin_y + 197)
        wait_for(lambda: pin_presentation()[1] < locked[1], "pin opacity was not persisted")
        desktop.click(pin_x + pin_width - 26, pin_y + 198)
        set_clipboard("UTF8_STRING", OTHER.encode())
        desktop.click(pin_x + pin_width - 26, pin_y + 288)
        copied_pin = wait_for(edited_clipboard, "pin toolbar did not copy the stored image")
        assert copied_pin.get_pixels() == edited_copy.get_pixels()
        save_qa_window("pin-toolbar.png", (pin_x, pin_y, pin_width, pin_height))
        desktop.click(pin_x + pin_width - 26, pin_y + 318)
        wait_for(lambda: not titled_window(PIN_TITLE), "pin did not hide")
        print("PASS pin toolbar stores opacity and size lock and copies unchanged pixels", flush=True)
        tray.activate("Pinned Workspace")
        workspace_id, (work_x, work_y, work_width, work_height) = wait_for(
            lambda: titled_window(WORKSPACE_TITLE), "workspace did not open",
        )
        assert (work_width, work_height) == (820, 640)
        desktop.focus(workspace_id)
        desktop.click(work_x + 78, work_y + 202)
        for letter in "review":
            desktop.keys(letter)
        desktop.keys("Return")

        def workspace_group(moved):
            with sqlite3.connect(data_dir / "clips.db") as connection:
                groups = connection.execute("SELECT id FROM pin_groups WHERE name='review'").fetchall()
                if not groups:
                    return False
                return not moved or bool(connection.execute(
                    "SELECT 1 FROM pin_workspace_items WHERE group_id=?", groups[0],
                ).fetchone())

        wait_for(lambda: workspace_group(False), "workspace did not create the group")
        time.sleep(0.2)
        desktop.click(work_x + 80, work_y + 193)
        def edited_thumbnail():
            pixels = Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), work_x + 240, work_y + 105, 235, 140)
            return pixels and bytes.fromhex("ff3b30") in pixels.get_pixels()

        wait_for(edited_thumbnail, "workspace did not render the annotated thumbnail")
        desktop.click(work_x + 315, work_y + 370)
        assert not titled_window(PIN_TITLE), "empty group opened a pin from a different group"
        desktop.click(work_x + 80, work_y + 111)
        desktop.click(work_x + 315, work_y + 326)
        desktop.keys("Down")
        desktop.keys("Return")
        wait_for(lambda: workspace_group(True), "workspace did not persist the pin group")
        desktop.request_close(workspace_id)
        wait_for(lambda: not titled_window(WORKSPACE_TITLE), "workspace did not close before reopening")
        tray.activate("Pinned Workspace")
        workspace_id, (work_x, work_y, _, work_height) = wait_for(lambda: titled_window(WORKSPACE_TITLE), "workspace did not reopen")
        time.sleep(0.2)
        desktop.click(work_x + 80, work_y + 193)
        save_qa_window("workspace-group.png", (work_x, work_y, work_width, work_height))
        desktop.click(work_x + 315, work_y + 370)
        restored_pin, _ = wait_for(lambda: titled_window(PIN_TITLE), "workspace did not restore the stored pin")
        desktop.request_close(restored_pin)
        desktop.request_close(workspace_id)
        wait_for(lambda: not titled_window(PIN_TITLE) and not titled_window(WORKSPACE_TITLE),
                 "workspace windows still cover the scrolling fixture")
        print("PASS pin, persistent revision, workspace group creation, filter, move and restore", flush=True)

        desktop.scroll_fixture(0)
        tray.activate("Screenshot")
        click_capture_action(select_area(), 3)
        _, (long_x, long_y, long_width, long_height) = wait_for(
            lambda: titled_window(LONGSHOT_TITLE), "scrolling capture did not open",
        )
        check_titlebar((long_x, long_y, long_width, long_height), "longshot", "17191f")
        time.sleep(0.25)
        desktop.scroll_fixture(32)
        desktop.click(long_x + 49, long_y + long_height - 29)
        time.sleep(0.45)
        _, (long_x, long_y, _, long_height) = wait_for(
            lambda: titled_window(LONGSHOT_TITLE), "scrolling capture controls did not return",
        )
        desktop.click(long_x + 244, long_y + long_height - 29)
        _, (x, y, width, height) = wait_for(lambda: titled_window(EDITOR_TITLE), "stitched image did not open")
        desktop.click(x + width - 155, y + height - 55)
        stitched = output.with_name("fixture-4.png")
        wait_for(stitched.is_file, "long screenshot was not exported")
        assert decode_png(stitched.read_bytes())[0] == (140, 132), decode_png(stitched.read_bytes())
        desktop.click(x + width - 57, y + height - 55)
        wait_for(lambda: not titled_window(EDITOR_TITLE), "long screenshot editor stayed open")
        tray.activate("Screenshot")
        click_capture_action(select_area(), 3)
        longshot_id, _ = wait_for(lambda: titled_window(LONGSHOT_TITLE), "scrolling cancellation fixture did not open")
        desktop.focus(longshot_id)
        desktop.keys("Escape")
        wait_for(lambda: not titled_window(LONGSHOT_TITLE), "Escape did not cancel scrolling capture")
        print("PASS scrolling capture uses the selected region, stitches and cancels from the keyboard", flush=True)

        tray.activate("Screenshot")
        click_capture_action(select_area(), 4)

        def recording_manifest(state):
            for path in (data_dir / "recordings").glob("*/manifest.json"):
                manifest = json.loads(path.read_text())
                if manifest["state"] == state:
                    return path.parent, manifest
            return None

        wait_for(lambda: recording_manifest("recording"), "region recording never began")
        _, (control_x, control_y, control_w, control_h) = wait_for(
            lambda: titled_window(RECORDING_CONTROL_TITLE), "recording control never opened",
        )
        assert (control_w, control_h) == (280, 56)
        assert control_x + control_w <= 440 or control_x >= 580 or control_y + control_h <= 200 or control_y >= 300
        save_qa_window("recording-control.png", (control_x, control_y, control_w, control_h))
        time.sleep(0.62)
        desktop.click(control_x + control_w - 74, control_y + 28)
        time.sleep(1.5)
        desktop.click(control_x + control_w - 74, control_y + 28)
        time.sleep(0.62)
        desktop.click(control_x + control_w - 28, control_y + 28)
        directory, manifest = wait_for(
            lambda: recording_manifest("complete"), "region recording never finalized",
        )
        video = directory / manifest["finalOutput"]["fileName"]
        assert video.stat().st_size == manifest["finalOutput"]["byteLength"]
        assert (manifest["video"]["width"], manifest["video"]["height"]) == (140, 100)
        duration = manifest["finalOutput"]["durationNs"] / 1e9
        assert 0.75 < duration < 2.7, (duration, manifest)
        if shutil.which("ffprobe"):
            probe = subprocess.check_output(
                ["ffprobe", "-v", "error", "-show_streams", "-of", "json", video], timeout=4,
            )
            stream, = json.loads(probe)["streams"]
            assert (stream["codec_name"], stream["width"], stream["height"]) == ("vp9", 140, 100)
        tray.activate("Recordings")
        recording_window, (rec_x, rec_y, rec_w, rec_h) = wait_for(
            lambda: titled_window(RECORDING_TITLE), "recordings library never opened",
        )
        assert (rec_w, rec_h) == (780, 620)
        check_titlebar((rec_x, rec_y, rec_w, rec_h), "recordings")
        wait_for(lambda: not titled_window(RECORDING_CONTROL_TITLE), "recording control did not close")

        def export_button():
            y = rec_y + 240
            pixels = Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), rec_x + rec_w - 97, y, 1, 180)
            if pixels:
                data, stride = pixels.get_pixels(), pixels.get_rowstride()
                accent = bytes.fromhex("8aa4ff" if theme == "dark" else "5b6cff")
                for index in range(pixels.get_height()):
                    if data[index * stride:index * stride + 3] == accent:
                        return y + index + 12
            return None

        button_y = wait_for(export_button, "recording card did not offer export")
        desktop.click(rec_x + rec_w - 69, button_y)
        exported = data_dir / "Exported Recordings" / f"Clippy-{manifest['sessionId']}.webm"
        wait_for(exported.is_file, "recording export did not commit")
        assert exported.read_bytes() == video.read_bytes()
        desktop.request_close(recording_window)
        wait_for(lambda: not titled_window(RECORDING_TITLE), "recordings library never closed")
        print("PASS region recording, excluded controls pause/resume/stop, VP9 finalization and exact card export", flush=True)

        set_clipboard("UTF8_STRING", OTHER.encode())
        wait_for(lambda: rows()[-1:] == [("text", OTHER, None)], "launcher clipboard change did not settle")
        set_clipboard("image/png", rgba_png())
        wait_for(lambda: rows()[-1:] == [("image", None, None)], "launcher PNG fixture did not reach the history")
        tray.activate("Actions")
        launcher_id, _ = wait_for(lambda: titled_window(LAUNCHER_TITLE), "save action launcher never appeared")
        desktop.focus(launcher_id)
        launcher_search("save png")
        desktop.keys("Return")
        saved = output.with_name("fixture-5.png")
        wait_for(saved.is_file, "launcher did not save the clipboard image")
        pixels = pixbuf_png(saved.read_bytes()).get_pixels()[:8]
        assert pixels == bytes((255, 0, 0, 128, 0, 128, 255, 0)), pixels
        time.sleep(0.15)
        launcher_search("pin image")
        desktop.keys("Return")
        launcher_pin, _ = wait_for(lambda: titled_window(PIN_TITLE), "launcher did not pin the clipboard image")
        wait_for(lambda: not titled_window(LAUNCHER_TITLE), "launcher did not close after pinning")
        desktop.request_close(launcher_pin)
        wait_for(lambda: not titled_window(PIN_TITLE), "launcher pin did not close")
        print("PASS launcher PNG export preserves alpha and pin creation opens the stored image", flush=True)

        tray.activate("Open Clipboard")
        preview_id, _ = panel_width(380)
        desktop.focus(preview_id)
        desktop.keys("Tab")
        _, (panel_x, panel_y, panel_w, panel_h) = panel_width(780)
        wait_for(lambda: preview_pixels((panel_x, panel_y, panel_w, panel_h)), "viewer fixture is not in the sidebar")
        desktop.click(panel_x + panel_w - 49, panel_y + panel_h - 45)
        viewer_id, viewer_bounds = wait_for(lambda: titled_window(VIEWER_TITLE), "image viewer did not open from preview")
        assert viewer_bounds[2:] == (1000, 720)
        wait_for(lambda: not titled_window(TITLE), "clipboard panel covered the viewer")
        check_titlebar(viewer_bounds, "viewer")
        desktop.request_close(viewer_id)
        wait_for(lambda: not titled_window(VIEWER_TITLE), "image viewer did not close")
        print("PASS image preview opens the 1000×720 native viewer and releases its window", flush=True)

        tray.activate("Open Clipboard")
        panel_id, (panel_x, panel_y, _, _) = panel_width(380)
        desktop.focus(panel_id)
        search("needle")
        desktop.click(panel_x + 48, panel_y + 80)
        desktop.keys("Control_L", "2")
        text_pin, pin_bounds = wait_for(lambda: titled_window(PIN_TITLE), "pin shortcut did not open the selected text")
        with sqlite3.connect(data_dir / "clips.db") as connection:
            assert connection.execute(
                "SELECT text_content FROM pin_workspace_items WHERE content_type='text'",
            ).fetchone() == (TEXT,)
        save_qa_window("pin-shortcut-text.png", pin_bounds)
        desktop.request_close(text_pin)
        wait_for(lambda: not titled_window(PIN_TITLE), "text pin did not close")
        close()
        print("PASS pin shortcut uses the selected clipboard item and persists a text pin", flush=True)

        def rgb(pixels, x, y):
            offset = y * pixels.get_rowstride() + x * pixels.get_n_channels()
            return pixels.get_pixels()[offset:offset + 3]

        def annotate_selection(check_history=False, accept_caption=True):
            desktop.scroll_fixture(0)
            root_window = Gdk.get_default_root_window()
            original = Gdk.pixbuf_get_from_window(root_window, 440, 200, 140, 100)
            tray.activate("Screenshot")
            toolbar = select_area()
            left, top, width = toolbar

            def select_tool(index):
                desktop.click(round(left + 6 + (index + 0.5) * capture_button_width(width)), top + 19)

            def screen_pixels(x, y, width=1, height=1):
                pixels = Gdk.pixbuf_get_from_window(root_window, 440 + x, 200 + y, width, height)
                return pixels.get_pixels() if pixels else b""

            select_tool(6)
            desktop.move(desktop.display, 0, 448, 224, 0)
            desktop.button(desktop.display, 1, 1, 0)
            desktop.sync(desktop.display, 0)
            for x, y in [(462, 208), (474, 220), (480, 230)]:
                desktop.move(desktop.display, 0, x, y, 0)
                desktop.sync(desktop.display, 0)
                time.sleep(0.045)
            desktop.button(desktop.display, 1, 0, 0)
            desktop.sync(desktop.display, 0)
            red = bytes.fromhex("ff3b30")
            pen_turn = lambda: red in screen_pixels(20, 6, 5, 5)
            wait_for(pen_turn, "the pen turn did not stay at the drawn coordinates")
            if check_history:
                desktop.keys("Control_L", "z")
                wait_for(lambda: not pen_turn(), "pen undo left the stroke visible")
                desktop.keys("Control_L", "y")
                wait_for(pen_turn, "pen redo did not restore the bend")

            select_tool(8)
            sample_tiles = {}
            for cell_x, cell_y in [(70, 25), (94, 25), (94, 73)]:
                cell = [rgb(original, px, py)
                        for px in range(cell_x, min(cell_x + 12, 105))
                        for py in range(cell_y, min(cell_y + 12, 75))]
                sample_tiles[cell_x + 2, cell_y + 1] = bytes(
                    (sum(pixel[channel] for pixel in cell) + len(cell) // 2) // len(cell)
                    for channel in range(3))

            def mosaic_visible():
                return all(screen_pixels(*point)[:3] == color for point, color in sample_tiles.items())

            desktop.drag(510, 225, 545, 275)
            wait_for(mosaic_visible, "mosaic preview did not keep source cell colors and partial edges")
            if check_history:
                desktop.keys("Control_L", "z")
                wait_for(lambda: all(screen_pixels(*point)[:3] == rgb(original, *point) for point in sample_tiles),
                         "mosaic undo did not restore the source colors")
                desktop.keys("Control_L", "y")
                wait_for(mosaic_visible, "mosaic redo did not restore the preview")

            select_tool(7)
            if check_history:
                desktop.click(448, 252)
                desktop.keys("q")
                desktop.keys("Escape")
                assert titled_window(CAPTURE_TITLE), "Escape closed the capture instead of the text field"
                wait_for(lambda: screen_pixels(11, 54)[:3] == rgb(original, 11, 54),
                         "a cancelled text field left pixels on the selection")
            desktop.click(448, 252)
            set_clipboard("UTF8_STRING", "Label 中".encode())
            desktop.keys("Control_L", "v")
            time.sleep(0.2)
            if accept_caption:
                desktop.keys("Return")
                label_visible = lambda: red in screen_pixels(8, 52, 61, 20)
                wait_for(label_visible, "confirmed caption did not appear at the chosen position")
                if check_history:
                    desktop.keys("Control_L", "z")
                    wait_for(lambda: not label_visible(), "caption undo left letters on the image")
                    desktop.keys("Control_L", "y")
                    wait_for(label_visible, "caption redo did not restore the letters")
                    save_qa_window("capture-inline-tools.png", (12, 170, 600, 225))
            return toolbar, sample_tiles

        toolbar, sample_tiles = annotate_selection(check_history=True)
        click_capture_action(toolbar, 0)
        inline_copy = wait_for(lambda: pixbuf_png(png) if (png := clipboard_png()) else None,
                               "inline annotations were not copied")
        assert (inline_copy.get_width(), inline_copy.get_height()) == (140, 100)
        assert all(rgb(inline_copy, *point) == color for point, color in sample_tiles.items())
        assert bytes.fromhex("ff3b30") in inline_copy.get_pixels()

        known_files = set(output.parent.glob("fixture*.png"))
        toolbar, _ = annotate_selection(accept_caption=False)
        click_capture_action(toolbar, 1)
        saved = wait_for(lambda: next(iter(set(output.parent.glob("fixture*.png")) - known_files), None),
                         "PNG output did not commit the open text field")
        assert pixbuf_png(saved.read_bytes()).get_pixels() == inline_copy.get_pixels()

        toolbar, _ = annotate_selection()
        click_capture_action(toolbar, 2)
        editor_id, (x, y, width, height) = wait_for(lambda: titled_window(EDITOR_TITLE),
                                                  "inline annotations did not open in the editor")
        set_clipboard("UTF8_STRING", OTHER.encode())
        desktop.click(x + width - 253, y + height - 55)
        wait_for(lambda: (png := clipboard_png()) and pixbuf_png(png).get_pixels() == inline_copy.get_pixels(),
                 "editor did not inherit the three inline tools")
        desktop.request_close(editor_id)
        wait_for(lambda: not titled_window(EDITOR_TITLE), "inline annotation editor did not close")
        print("PASS inline pen path, text input and mosaic undo/redo, matching clipboard/PNG/editor", flush=True)

        def configured(key):
            return json.loads((data_dir / "config.json").read_text())[key]

        tray.activate("Settings")
        settings_id, (x, y, width, height) = wait_for(
            lambda: titled_window(SETTINGS_TITLE), "shortcut settings did not open",
        )
        desktop.focus(settings_id)
        desktop.click(x + width - 157, y + 206)
        time.sleep(0.1)
        save_qa_window("shortcut-recording.png", (x, y, width, height))
        desktop.keys("Control_L", "Alt_L", "Shift_L", "j")
        time.sleep(0.15)
        assert not titled_window(TITLE), "the active hotkey interrupted recording"
        desktop.keys("Escape")
        desktop.click(x + width - 157, y + 206)
        desktop.keys("Control_L", "F8")
        time.sleep(0.08)
        save_qa_window("shortcut-recorded.png", (x, y, width, height))
        desktop.click(x + width - 157, y + 206)
        desktop.keys("Escape")
        desktop.scroll_down(x + width - 94, y + height - 67)
        desktop.click(x + width // 2, y + height - 50)
        wait_for(lambda: configured("global_shortcut") == "Ctrl+F8", "clipboard shortcut was not saved after cancel")

        desktop.click(x + 279, y + 109)
        desktop.click(x + width - 157, y + 444)
        desktop.keys("Control_L", "F8")
        desktop.scroll_down(x + width - 94, y + height - 67)
        desktop.click(x + width // 2, y + height - 50)
        time.sleep(0.2)
        assert configured("capture_shortcut") == "Ctrl+Alt+Shift+K", "duplicate shortcut was accepted"
        desktop.click(x + 126, y + 109)
        desktop.click(x + 279, y + 109)
        desktop.click(x + width - 157, y + 444)
        desktop.keys("Control_L", "Shift_L", "F9")
        desktop.scroll_down(x + width - 94, y + height - 67)
        desktop.click(x + width // 2, y + height - 50)
        wait_for(lambda: configured("capture_shortcut") == "Ctrl+Shift+F9", "recorded screenshot shortcut was not saved")
        desktop.request_close(settings_id)
        wait_for(lambda: not titled_window(SETTINGS_TITLE), "shortcut settings did not close")
        print("PASS settings record native keys, cancel, reject duplicates and persist both shortcuts", flush=True)

        desktop.keys("Control_L", "Alt_L", "Shift_L", "j")
        desktop.keys("Control_L", "Alt_L", "Shift_L", "k")
        time.sleep(0.25)
        assert not titled_window(TITLE) and not titled_window(CAPTURE_TITLE), "retired shortcuts still triggered"
        desktop.keys("Control_L", "F8")
        close()
        desktop.keys("Control_L", "Shift_L", "F9")
        overlay, _ = wait_for(lambda: titled_window(CAPTURE_TITLE), "new screenshot shortcut did not trigger")
        desktop.focus(overlay)
        desktop.keys("Escape")
        wait_for(lambda: not titled_window(CAPTURE_TITLE), "screenshot from the new shortcut stayed open")
        print("PASS new clipboard and screenshot shortcuts work without restart and old keys are released", flush=True)

        def open_shortcut_settings():
            tray.activate("Settings")
            window, bounds = wait_for(lambda: titled_window(SETTINGS_TITLE), "shortcut settings did not reopen")
            desktop.focus(window)
            return window, bounds

        def fill_shortcut(row, shortcut):
            desktop.click(x + 160, y + row)
            desktop.keys("Control_L", "a")
            for char in shortcut:
                if char == "+":
                    desktop.keys("Shift_L", "equal")
                elif char.isupper():
                    desktop.keys("Shift_L", char.lower())
                else:
                    desktop.keys(char)

        def save_shortcut_settings():
            desktop.scroll_down(x + width - 94, y + height - 67)
            desktop.click(x + width // 2, y + height - 50)

        def footer():
            return Gdk.pixbuf_get_from_window(
                Gdk.get_default_root_window(), x, y + height - 25, width, 25,
            ).get_pixels()

        config_path = data_dir / "config.json"
        saved = config_path.read_bytes()
        settings_id, (x, y, width, height) = open_shortcut_settings()
        fill_shortcut(206, "Ctrl+F10")
        fill_shortcut(368, "Ctrl+F11")
        before = footer()
        with desktop.occupy_control_key("F11"):
            save_shortcut_settings()
            wait_for(lambda: footer() != before, "conflicting shortcut did not return a save result")
            save_qa_window("native-shortcut-conflict.png", (x, y, width, height))
            assert config_path.read_bytes() == saved, "a conflicting change overwrote the current config"
            with desktop.occupy_control_key("F10"):
                pass
            desktop.keys("Control_L", "F8")
            close()
        print("PASS another X11 client's grab rejects the batch and releases the first new registration", flush=True)

        settings_id, (x, y, width, height) = open_shortcut_settings()
        fill_shortcut(206, "Ctrl+F10")
        before = footer()
        temporary_config = config_path.with_suffix(".tmp")
        temporary_config.mkdir()
        try:
            save_shortcut_settings()
            wait_for(lambda: footer() != before, "failed config write did not return a save result")
            save_qa_window("native-shortcut-write-error.png", (x, y, width, height))
            assert config_path.read_bytes() == saved, "failed write changed the previous config"
            with desktop.occupy_control_key("F10"):
                pass
            desktop.keys("Control_L", "F8")
            close()
        finally:
            temporary_config.rmdir()
        settings_id, (x, y, width, height) = open_shortcut_settings()
        fill_shortcut(206, "Ctrl+F10")
        save_qa_window("native-shortcut-retry-draft.png", (x, y, width, height))
        save_shortcut_settings()
        wait_for(lambda: configured("global_shortcut") == "Ctrl+F10", "retry did not save the shortcut")
        desktop.request_close(settings_id)
        wait_for(lambda: not titled_window(SETTINGS_TITLE), "shortcut settings did not close after retry")
        desktop.keys("Control_L", "F10")
        close()
        print("PASS failed config write restored the old shortcut and a subsequent save applied the new one", flush=True)

        settings_id, (x, y, width, height) = open_shortcut_settings()
        fill_shortcut(206, "Ctrl+2")
        fill_shortcut(368, "Ctrl+F10")
        save_shortcut_settings()
        wait_for(lambda: (configured("global_shortcut"), configured("pin_shortcut")) == ("Ctrl+2", "Ctrl+F10"),
                 "two active shortcuts could not be swapped")
        desktop.request_close(settings_id)
        wait_for(lambda: not titled_window(SETTINGS_TITLE), "swapped shortcuts left settings open")
        desktop.keys("Control_L", "2")
        panel_id, (panel_x, panel_y, _, _) = panel_width(380)
        desktop.focus(panel_id)
        search("needle")
        desktop.click(panel_x + 48, panel_y + 80)
        desktop.keys("Control_L", "F10")
        pin, _ = wait_for(lambda: titled_window(PIN_TITLE), "swapped pin shortcut did not trigger")
        desktop.request_close(pin)
        wait_for(lambda: not titled_window(PIN_TITLE), "pin from the swapped shortcut did not close")
        close()
        print("PASS exchanging active clipboard and pin chords routes both keys to the new actions", flush=True)

        second = subprocess.run([binary, "--open"], env=env, timeout=4)
        assert second.returncode == 0
        panel()
        tray.activate("Quit")
        assert process.wait(timeout=5) == 0
        print("PASS second launch forwarded to the running instance and tray quit exited", flush=True)
    except Exception:
        save_failure_frame()
        raise
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
        target.destroy()
        desktop.close_display(desktop.display)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="clippy-slint-x11-") as temporary:
        run(str(Path(sys.argv[1]).resolve(strict=True)), Path(temporary))
