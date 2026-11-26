"""从隔离桌面的文件拖入、原生选择器和剪贴板历史验证查看器修订。"""

import base64
import hashlib
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
Gdk, Gio, GLib, Gtk = (qa[name] for name in ("Gdk", "Gio", "GLib", "Gtk"))
wait = qa["wait_for"]
APP = qa["TITLE"]
VIEWER = qa["VIEWER_TITLE"]
PIN = qa["PIN_TITLE"]
DRAG_SOURCE = "Clippy QA PNG drag source"
RED = bytes.fromhex("ff3b30")


def chunk(name, data):
    return struct.pack(">I", len(data)) + name + data + struct.pack(">I", zlib.crc32(name + data))


def png(width, height, pixels):
    stride = width * 4
    rows = b"".join(b"\0" + pixels[i:i + stride] for i in range(0, len(pixels), stride))
    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


def rgba(data):
    return qa["pixbuf_png"](data).add_alpha(False, 0, 0, 0).get_pixels()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def container(flat, source, annotations, adjustments, version, renderer=2):
    picture = qa["pixbuf_png"](flat)
    width, height = picture.get_width(), picture.get_height()
    document = {
        "format": "clippy-pin-project", "formatVersion": version, "rendererVersion": renderer,
        "createdAt": 0, "appVersion": "0.1.25",
        "source": {"pngBase64": base64.b64encode(source).decode(), "sha256": sha(source),
                   "width": width, "height": height},
        "document": {"annotations": annotations, "adjustments": adjustments},
    }
    if version == 3:
        document["preview"] = {
            "width": width, "height": height, "rgbaSha256": sha(struct.pack(">II", width, height) + rgba(flat)),
        }
    payload = b"clippy-project\x00\x01\x00\x00\x00" + zlib.compress(json.dumps(document).encode())
    return flat[:33] + chunk(b"iTXt", payload) + flat[33:]


class FileChooser:
    path = "/org/freedesktop/portal/desktop"
    service = "org.freedesktop.portal.Desktop"
    interface = "org.freedesktop.portal.FileChooser"

    def __init__(self, tray):
        self.bus = tray.connection
        self.pending = []
        xml = f"""<node><interface name="{self.interface}">
          <property name="version" type="u" access="read"/>
          <method name="OpenFile"><arg type="s" direction="in"/><arg type="s" direction="in"/>
            <arg type="a{{sv}}" direction="in"/><arg type="o" direction="out"/></method>
        </interface></node>"""
        self.bus.register_object(self.path, Gio.DBusNodeInfo.new_for_xml(xml).interfaces[0],
                                 self.open_file, lambda *_: GLib.Variant("u", 4), None)
        assert tray.call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                         "RequestName", GLib.Variant("(su)", (self.service, 0))) == (1,)

    def open_file(self, bus, sender, path, interface, method, params, invocation):
        parent, title, options = params.unpack()
        token = options["handle_token"]
        request = f"{self.path}/request/{sender[1:].replace('.', '_')}/{token}"
        self.pending.append((sender, request, parent, title, options))
        invocation.return_value(GLib.Variant("(o)", (request,)))

    def finish(self, path):
        sender, request, parent, title, options = wait(lambda: self.pending, "OpenFile was not called").pop(0)
        assert parent.startswith("x11:") and title == "Open PNG", (parent, title)
        assert options["filters"] == [("PNG images", [(0, "*.png")])], options
        response = (1, {}) if path is None else (0, {"uris": GLib.Variant("as", [path.as_uri()])})
        self.bus.emit_signal(sender, request, "org.freedesktop.portal.Request", "Response",
                             GLib.Variant("(ua{sv})", response))


class DragSource:
    def __init__(self, desktop):
        self.desktop = desktop
        self.window = Gtk.Window(title=DRAG_SOURCE)
        self.window.set_default_size(192, 100)
        self.window.move(1060, 80)
        content = Gtk.EventBox()
        content.add(Gtk.Label(label="Drag PNG"))
        self.window.add(content)
        content.drag_source_set(Gdk.ModifierType.BUTTON1_MASK,
                                [Gtk.TargetEntry.new("text/uri-list", 0, 0)], Gdk.DragAction.COPY)
        content.connect("drag-begin", self.begin)
        content.connect("drag-data-get", self.provide)
        self.window.show_all()
        self.xid, self.bounds = titled(DRAG_SOURCE)

    def begin(self, widget, context):
        self.context = context

    def provide(self, widget, context, selection, info, time):
        assert selection.set_uris([self.uri])
        self.provided = True

    def move(self, x, y):
        self.desktop.move(self.desktop.display, 0, x, y, 0)
        self.desktop.sync(self.desktop.display, 0)

    def button(self, down):
        self.desktop.button(self.desktop.display, 1, int(down), 0)
        self.desktop.sync(self.desktop.display, 0)

    def drop(self, path, destination):
        self.context = None
        self.provided = False
        self.uri = path.as_uri()
        self.desktop.focus(self.xid)
        x, y, width, height = self.bounds
        self.move(x + width // 2, y + height // 2)
        self.button(True)
        try:
            self.move(x + 22, y + height // 2)
            wait(lambda: self.context, "GTK did not start dragging the file")
            x, y, _, _ = destination
            self.move(x + 50, y + 110)
            wait(lambda: self.context.get_selected_action(), "Clippy did not accept the native file drag")
            wait(lambda: self.provided, "Clippy did not request the file URI before the drop")
        finally:
            self.button(False)


def titled(title):
    return wait(lambda: qa["titled_window"](title), f"{title} did not open")


def run(binary, root):
    tray = qa["Tray"]()
    dialog = FileChooser(tray)
    desktop = qa["Desktop"]()
    drag = DragSource(desktop)
    process = None
    width, height = 120, 88
    root_pixels = bytes.fromhex("263d5bff") * (width * height)
    original = root / "source.png"
    original.write_bytes(png(width, height, root_pixels))

    def start(profile, saves):
        nonlocal process, data
        data = root / profile / "com.clippy.qa.slint"
        data.mkdir(mode=0o700, parents=True, exist_ok=True)
        (data / "config.json").write_text(json.dumps({
            "auto_paste": False, "language": "en", "theme": "light", "screenshot_save_dir": str(saves),
            "screenshot_filename_template": "viewer", "storage_mode": "persistent", "max_history": 100,
        }))
        tray.address = None
        env = {**os.environ, "XDG_DATA_HOME": str(data.parent),
               "XDG_CONFIG_HOME": str(data.parent / "config"), "XDG_CACHE_HOME": str(data.parent / "cache")}
        process = subprocess.Popen([binary], env=env)
        wait(lambda: tray.address, "QA tray did not register")

    def quit_app():
        nonlocal process
        tray.activate("Quit")
        assert process.wait(timeout=8) == 0
        process = None

    def rows(query):
        with sqlite3.connect(data / "clips.db", timeout=1) as db:
            return db.execute(query).fetchall()

    def drop_file(path):
        tray.activate("Open Clipboard")
        _, history = titled(APP)
        drag.drop(path, history)
        return titled(VIEWER)

    def choose_file(path, shortcut=False):
        if shortcut:
            tray.activate("Open Clipboard")
            window_id, _ = titled(APP)
            desktop.focus(window_id)
            desktop.keys("Control_L", "o")
        else:
            tray.activate("Open PNG…")
        dialog.finish(path)
        return titled(VIEWER) if path is not None else None

    def close(title, xid):
        desktop.request_close(xid)
        wait(lambda: not qa["titled_window"](title), f"{title} did not close")

    def red(bounds):
        x, y, w, h = bounds
        preview = Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), x + 140, y + 115, w - 280, h - 310)
        return RED in preview.get_pixels()

    def save(viewer, path):
        xid, (x, y, w, h) = viewer
        desktop.focus(xid)
        qa["set_clipboard"]("UTF8_STRING", b"viewer-before-save")
        desktop.click(x + w - 80, y + h - 14)
        wait(path.is_file, "editable PNG was not committed")
        result = path.read_bytes()
        assert b"clippy-project" not in result

        def matching_clipboard():
            copied = qa["clipboard_png"]()
            return copied and b"clippy-project" not in copied and rgba(copied) == rgba(result)

        wait(matching_clipboard, "editable save clipboard did not match the flat file")
        return result

    data = None
    try:
        saves = root / "saved"
        start("primary", saves)
        choose_file(None)
        assert qa["titled_window"](APP)
        assert not qa["titled_window"](VIEWER)
        view = drop_file(original)
        xid, (x, y, w, h) = view
        desktop.focus(xid)
        desktop.click(x + w // 2 - 56, y + h - 93)
        desktop.drag(x + 330, y + 206, x + 640, y + 403)
        wait(lambda: red(view[1]), "viewer did not draw the rectangle")
        qa["save_qa_window"]("slint-viewer-editable.png", view[1])
        saved_path = saves / "viewer.png"
        saved = save(view, saved_path)
        result, = rows(
            "SELECT source_hash, png, annotations_json, adjustments_json, rendered_hash "
            "FROM image_revisions JOIN image_assets ON image_revisions.asset_id=image_assets.id"
        )
        root_hash, source_png, annotations, adjustments, rendered_hash = result
        assert root_hash == sha(source_png) and rgba(source_png) == root_pixels
        assert rendered_hash == sha(saved) and rgba(saved) != root_pixels
        assert json.loads(annotations)[0]["type"] == "rect"
        wait(lambda: rows("SELECT revision_id FROM clip_image_revisions"), "copied image did not link to its revision")
        second_path = saves / "viewer-2.png"
        assert save(view, second_path) == saved
        assert saved_path.read_bytes() == saved
        assert rows("SELECT count(*) FROM image_revisions") == [(1,)]
        quit_app()
        print(f"PASS editable save used only flat pixels, root={root_hash} rendered={rendered_hash}", flush=True)

        start("primary", saves)
        reopened = drop_file(saved_path)
        wait(lambda: red(reopened[1]), "reopened viewer lost the saved annotation")
        xid, (x, y, w, h) = reopened
        desktop.focus(xid)
        qa["set_clipboard"]("UTF8_STRING", b"viewer-after-restart")
        desktop.click(x + w // 2 + 46, y + h - 93)
        reopened_copy = wait(lambda: qa["clipboard_png"](), "reopened viewer did not copy")
        assert rgba(reopened_copy) == rgba(saved)
        print(f"reopened RGBA sha256={sha(rgba(reopened_copy))}", flush=True)
        desktop.click(x + w // 2 - 56, y + h - 93)
        qa["save_qa_window"]("slint-viewer-reopened.png", reopened[1])
        undo_x, undo_y = x + w // 2 + 192, y + h - 101
        desktop.click(undo_x, undo_y)
        wait(lambda: not red(reopened[1]), "undo after restart did not restore the root image")
        desktop.click(x + w // 2 + 80, y + h - 142)
        flat_path = saves / "viewer-3.png"
        wait(flat_path.is_file, "flat save after undo did not complete")
        assert rgba(flat_path.read_bytes()) == root_pixels
        close(VIEWER, xid)
        tray.activate("Open Clipboard")
        history_id, (hx, hy, _, _) = titled(APP)
        desktop.focus(history_id)
        desktop.click(hx + 60, hy + 35)
        desktop.keys("Control_L", "2")
        pin_id, _ = titled(PIN)
        pinned = rows("SELECT revision_id FROM pin_workspace_items")
        revisions = rows("SELECT id, asset_id, document_hash, rendered_hash, annotations_json FROM image_revisions")
        assert pinned == [(revisions[0][0],)] and len(revisions) == 1, (pinned, revisions)
        close(PIN, pin_id)
        print("PASS restart reopened the actual PNG, undo restored the root and history pinned the stored revision", flush=True)

        mutated = root / "one pixel changed.png"
        changed = bytes([17]) + rgba(saved)[1:]
        mutated.write_bytes(png(width, height, changed))
        view = choose_file(mutated, shortcut=True)
        changed_save = save(view, saves / "viewer-4.png")
        assert rgba(changed_save) == changed
        source_hash, source, operations = rows(
            "SELECT source_hash, png, annotations_json FROM image_revisions "
            "JOIN image_assets ON image_revisions.asset_id=image_assets.id ORDER BY image_revisions.id DESC LIMIT 1"
        )[0]
        assert source_hash == sha(source) and rgba(source) == changed and json.loads(operations) == []
        quit_app()
        print("PASS a one-pixel edit opened as a new plain root through the native picker", flush=True)

        for version, renderer in ((2, 2), (3, 2), (3, 1)):
            legacy = root / f"legacy-format-{version}-renderer-{renderer}.png"
            legacy.write_bytes(container(saved, source_png, json.loads(annotations), json.loads(adjustments),
                                         version, renderer))
            target = root / f"migrated-{version}-{renderer}"
            start(target.name, target)
            view = drop_file(legacy)
            migrated = save(view, target / "viewer.png")
            assert rgba(migrated) == rgba(saved)
            stored_source, operations, stored_renderer = rows(
                "SELECT png, annotations_json, renderer_version FROM image_revisions "
                "JOIN image_assets ON image_revisions.asset_id=image_assets.id"
            )[0]
            assert stored_renderer == 2
            if renderer == 2:
                assert rgba(stored_source) == root_pixels and json.loads(operations) == json.loads(annotations)
            else:
                assert rgba(stored_source) == rgba(saved) and json.loads(operations) == []
            quit_app()
            print(f"PASS legacy iTXt v{version}, renderer {renderer}, saved as one SQLite v2 revision", flush=True)
    except Exception:
        qa["save_failure_frame"]()
        raise
    finally:
        if process is not None:
            process.terminate()
            process.wait(timeout=5)
        drag.window.destroy()
        desktop.close_display(desktop.display)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="clippy-viewer-revisions-") as directory:
        run(str(Path(sys.argv[1]).resolve(strict=True)), Path(directory))
