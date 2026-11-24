"""用独立 X11 server、SNI 主机和合成数据验收 QA 二进制。"""
import ctypes as c
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
PIN_TITLE = "Clippy Pin"
WORKSPACE_TITLE = "Clippy Pinned Workspace"
LONGSHOT_TITLE = "Clippy Scrolling Capture"
RECORDING_TITLE = "Clippy Recordings"
TEXT = "needle 自动剪贴板 QA"
OTHER = "unrelated fixture"
HTML = "<b>rich-qa</b>".encode()
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
        self.set_background = bind(self.lib, "XSetWindowBackground", (c.c_ulong, c.c_ulong))
        self.clear_window = bind(self.lib, "XClearWindow", (c.c_ulong,))
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

    def focus(self, window):
        self.focus_window(self.display, window, 2, 0)
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
    return result.stdout if result.returncode == 0 else None


def decode_png(data):
    pixbuf = pixbuf_png(data)
    return (pixbuf.get_width(), pixbuf.get_height()), pixbuf.get_pixels()[:3]


def pixbuf_png(data):
    loader = GdkPixbuf.PixbufLoader.new_with_type("png")
    loader.write(data)
    loader.close()
    return loader.get_pixbuf()


def save_failure_frame():
    directory = os.environ.get("CLIPPY_SLINT_QA_ARTIFACTS")
    if directory:
        root = Gdk.get_default_root_window()
        pixbuf = Gdk.pixbuf_get_from_window(root, 0, 0, root.get_width(), root.get_height())
        pixbuf.savev(str(Path(directory) / "slint-x11-failure.png"), "png", [], [])


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
    # 自动粘贴在全链路用例中显式开启，普通 QA 安装仍使用首次启动默认值。
    (data_dir / "config.json").write_text(json.dumps({
        "auto_paste": True, "max_history": 200, "storage_mode": "disk", "theme": "system",
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

    def search(box, word):
        x, y, _, _ = box
        desktop.click(x + 95, y + 83)
        desktop.keys("Control_L", "a")
        for letter in word:
            desktop.keys(letter)
        # 输入事件和 SQLite 返回跨两个线程，等查询结果进入页面再点击。
        time.sleep(0.25)

    def close(box):
        x, y, width, _ = box
        desktop.click(x + width - 44, y + 37)
        wait_for(lambda: not titled_window(TITLE), "panel never closed")

    def select_area():
        _, (x, y, width, _) = wait_for(
            lambda: titled_window(CAPTURE_TITLE), "screenshot overlay never appeared",
        )
        desktop.drag(x + 440, y + 200, x + 580, y + 300)
        return x + width // 2, y

    try:
        wait_for(lambda: tray.address, "no tray registration")
        assert not titled_window(TITLE), "tray-only startup opened a window"
        tray.activate("Open Clipboard")
        _, box = panel()
        close(box)
        print("PASS tray registration, menu and window lifecycle", flush=True)

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

        desktop.focus(target_id)
        desktop.keys("Control_L", "Alt_L", "Shift_L", "j")
        panel_id, box = panel()
        desktop.focus(panel_id)
        search(box, "needle")
        x, y, _, height = box
        desktop.click(x + 60, y + height - 102)
        try:
            wait_for(lambda: get_clipboard("UTF8_STRING").decode() == TEXT,
                     "search and copy did not select Unicode")
        except AssertionError:
            print(f"Clipboard fixture output: {get_clipboard('UTF8_STRING')!r}", file=sys.stderr)
            raise
        assert entry.get_text() == "", "Copy injected paste input"
        close(box)
        print("PASS global hotkey, Unicode/HTML/PNG history, search and copy-only", flush=True)

        desktop.focus(target_id)
        desktop.keys("Control_L", "Alt_L", "Shift_L", "j")
        panel_id, box = panel()
        desktop.focus(panel_id)
        search(box, "needle")
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
        search(box, "needle")
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
        middle, top = select_area()
        desktop.click(middle - 101, top + 40)
        png = wait_for(clipboard_png, "screenshot copy did not publish PNG")
        expected = ((140, 100), bytes.fromhex("224466"))
        assert decode_png(png) == expected, decode_png(png)
        wait_for(lambda: not titled_window(CAPTURE_TITLE), "overlay stayed open after copy")

        tray.activate("Screenshot")
        middle, top = select_area()
        desktop.click(middle, top + 40)
        output = root / "shots" / "fixture.png"
        wait_for(output.is_file, "screenshot file was not committed")
        assert decode_png(output.read_bytes()) == expected
        assert list(output.parent.iterdir()) == [output]
        print("PASS screenshot hotkey and tray, native pixels, crop, clipboard and atomic file", flush=True)

        tray.activate("Screenshot")
        middle, top = select_area()
        desktop.click(middle + 100, top + 40)
        wait_for(lambda: not titled_window(CAPTURE_TITLE), "cancel did not close overlay")
        assert list(output.parent.iterdir()) == [output]
        print("PASS screenshot cancellation leaves the existing file untouched", flush=True)

        tray.activate("Screenshot")
        middle, top = select_area()
        desktop.click(middle, top + 85)
        _, (x, y, width, height) = wait_for(lambda: titled_window(EDITOR_TITLE), "editor never opened")
        desktop.drag(x + width // 3, y + height // 3, x + width * 2 // 3, y + height * 2 // 3)
        # 一次标注会在工作线程合成，更改入画后才能提交。
        time.sleep(0.3)
        desktop.click(x + width - 253, y + height - 55)

        def edited_clipboard():
            png = clipboard_png()
            if png:
                pixbuf = pixbuf_png(png)
                if b"\xff\x3b\x30" in pixbuf.get_pixels():
                    return pixbuf
            return None

        edited_copy = wait_for(edited_clipboard, "editor copy did not contain the red rectangle")
        desktop.click(x + width - 155, y + height - 55)
        edited_file = output.with_name("fixture-2.png")
        wait_for(edited_file.is_file, "edited PNG was not committed")
        assert pixbuf_png(edited_file.read_bytes()).get_pixels() == edited_copy.get_pixels()
        assert (edited_copy.get_width(), edited_copy.get_height()) == expected[0]
        desktop.click(x + width - 67, y + height - 26)
        _, (pin_x, pin_y, pin_width, pin_height) = wait_for(
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
        desktop.click(pin_x + pin_width - 43, pin_y + pin_height - 25)
        wait_for(lambda: not titled_window(PIN_TITLE), "pin did not hide")
        tray.activate("Pinned Workspace")
        _, (work_x, work_y, work_width, work_height) = wait_for(
            lambda: titled_window(WORKSPACE_TITLE), "workspace did not open",
        )
        time.sleep(0.2)
        desktop.click(work_x + 61, work_y + work_height - 54)
        wait_for(lambda: titled_window(PIN_TITLE), "workspace did not restore the stored pin")
        print("PASS pin, persistent revision and workspace restore", flush=True)

        desktop.scroll_fixture(0)
        tray.activate("Screenshot")
        middle, top = select_area()
        desktop.click(middle + 113, top + 85)
        _, (long_x, long_y, _, long_height) = wait_for(
            lambda: titled_window(LONGSHOT_TITLE), "scrolling capture did not open",
        )
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
        stitched = output.with_name("fixture-3.png")
        wait_for(stitched.is_file, "long screenshot was not exported")
        assert decode_png(stitched.read_bytes())[0] == (140, 132), decode_png(stitched.read_bytes())
        desktop.click(x + width - 57, y + height - 55)
        wait_for(lambda: not titled_window(EDITOR_TITLE), "long screenshot editor stayed open")
        print("PASS scrolling capture uses the selected region and stitches a second frame", flush=True)

        tray.activate("Screenshot")
        middle, top = select_area()
        desktop.click(middle - 110, top + 85)

        def recording_manifest(state):
            for path in (data_dir / "recordings").glob("*/manifest.json"):
                manifest = json.loads(path.read_text())
                if manifest["state"] == state:
                    return path.parent, manifest
            return None

        wait_for(lambda: recording_manifest("recording"), "region recording never began")
        time.sleep(0.75)
        tray.activate("Pause / Resume recording")
        time.sleep(1.5)
        tray.activate("Pause / Resume recording")
        time.sleep(0.75)
        tray.activate("Stop recording")
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
        _, (rec_x, rec_y, rec_w, rec_h) = wait_for(
            lambda: titled_window(RECORDING_TITLE), "recordings library never opened",
        )
        time.sleep(0.25)
        desktop.click(rec_x + rec_w * 3 // 8, rec_y + rec_h - 55)
        exported = data_dir / "Exported Recordings" / f"Clippy-{manifest['sessionId']}.webm"
        wait_for(exported.is_file, "recording export did not commit")
        assert exported.read_bytes() == video.read_bytes()
        desktop.click(rec_x + rec_w * 7 // 8, rec_y + rec_h - 80)
        wait_for(lambda: not titled_window(RECORDING_TITLE), "recordings library never closed")
        print("PASS region recording, pause/resume, VP9 finalization and exact library export", flush=True)

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
