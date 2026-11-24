"""在已登录的 Linux 桌面上测量 Slint QA 进程树的空闲 PSS。"""
import argparse
from contextlib import contextmanager
import hashlib
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

from gi.repository import Gio, GLib


def call(bus, service, path, interface, method, arguments):
    return bus.call_sync(
        service, path, interface, method, arguments, None,
        Gio.DBusCallFlags.NO_AUTO_START, 2000, None,
    ).unpack()


def tray_item(bus, pid):
    try:
        (items,) = call(
            bus, "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher",
            "org.freedesktop.DBus.Properties", "Get",
            GLib.Variant("(ss)", ("org.kde.StatusNotifierWatcher", "RegisteredStatusNotifierItems")),
        )
    except GLib.Error:
        return None
    for item in items:
        service, separator, path = item.partition("/")
        service = service.rstrip("@")
        try:
            (owner,) = call(
                bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                "GetConnectionUnixProcessID", GLib.Variant("(s)", (service,)),
            )
            if owner == pid:
                return service, f"/{path}" if separator else "/StatusNotifierItem"
        except GLib.Error:
            continue
    return None


def tray_ready(bus, pid):
    return tray_item(bus, pid) is not None


def menu_quit(bus, pid):
    item = tray_item(bus, pid)
    if not item:
        return False
    service, path = item
    try:
        (menu,) = call(
            bus, service, path, "org.freedesktop.DBus.Properties", "Get",
            GLib.Variant("(ss)", ("org.kde.StatusNotifierItem", "Menu")),
        )
        _, layout = call(
            bus, service, menu, "com.canonical.dbusmenu", "GetLayout",
            GLib.Variant("(iias)", (0, -1, ["label"])),
        )
        nodes = [layout]
        while nodes:
            node = nodes.pop()
            item_id, properties, children = node.unpack() if isinstance(node, GLib.Variant) else node
            label = properties.get("label", "").replace("_", "").strip().lower()
            if label in {"quit", "exit", "退出"}:
                call(bus, service, menu, "com.canonical.dbusmenu", "Event",
                     GLib.Variant("(isvu)", (item_id, "clicked", GLib.Variant("i", 0), 0)))
                return True
            nodes.extend(children)
    except GLib.Error:
        pass
    return False


def running_executable(executable):
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            if entry.stat().st_uid == os.getuid() and (entry / "exe").readlink() == executable:
                found.append(int(entry.name))
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
    return found


@contextmanager
def suspend_legacy(executable):
    if executable is None:
        yield
        return
    executable = executable.resolve(strict=True)
    if executable.name != "clippy-app":
        raise RuntimeError("The legacy executable must be clippy-app")
    candidates = running_executable(executable)
    if len(candidates) != 1:
        raise RuntimeError(f"Expected one running legacy process for {executable}, found {len(candidates)}")
    pid, = candidates
    bus = Gio.bus_get_sync(Gio.BusType.SESSION)
    descriptor = os.pidfd_open(pid)
    try:
        if menu_quit(bus, pid):
            deadline = time.monotonic() + 5
            while pid in running_executable(executable) and time.monotonic() < deadline:
                time.sleep(0.1)
        if pid in running_executable(executable):
            signal.pidfd_send_signal(descriptor, signal.SIGTERM)
        deadline = time.monotonic() + 10
        while pid in running_executable(executable):
            if time.monotonic() >= deadline:
                raise RuntimeError("The legacy process did not exit")
            time.sleep(0.1)
        print(f"legacy_stopped pid={pid}", flush=True)
        yield
    finally:
        os.close(descriptor)
        if not running_executable(executable):
            restored = subprocess.Popen(
                [executable], cwd=Path.home(), stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
            )
            deadline = time.monotonic() + 18
            while not tray_ready(bus, restored.pid):
                if restored.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError(f"Legacy restart did not register a tray item (PID {restored.pid})")
                time.sleep(0.1)
            print(f"legacy_restored pid={restored.pid} tray_ready=yes", flush=True)


def process_tree_pss(pid):
    pss = 0
    processes = {pid}
    pending = [pid]
    while pending:
        entry = Path("/proc") / str(pending.pop())
        try:
            pss += int(next(line.split()[1] for line in (entry / "smaps_rollup").read_text().splitlines()
                            if line.startswith("Pss:")))
        except (FileNotFoundError, ProcessLookupError):
            if entry.name == str(pid):
                raise
        for task in (entry / "task").glob("*/children"):
            try:
                children = map(int, task.read_text().split())
                for child in children:
                    if child not in processes:
                        processes.add(child)
                        pending.append(child)
            except (FileNotFoundError, ProcessLookupError):
                continue
    return pss, len(processes)


def require_isolated_clippy():
    names = {"clippy-app", "clippy-slint", "clippy-slint-qa"}
    for process in Path("/proc").iterdir():
        if not process.name.isdecimal():
            continue
        try:
            if process.stat().st_uid != os.getuid():
                continue
            name = (process / "comm").read_text().strip()
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
        if name in names:
            raise SystemExit(f"Close the running {name} (PID {process.name}) before measuring QA idle PSS.")


def measure(binary, repeats, seconds):
    require_isolated_clippy()
    bus = Gio.bus_get_sync(Gio.BusType.SESSION)
    sha256 = hashlib.sha256(binary.read_bytes()).hexdigest()
    print(f"binary_sha256={sha256}", flush=True)
    passed = True
    for run in range(1, repeats + 1):
        require_isolated_clippy()
        with tempfile.TemporaryDirectory(prefix="clippy-slint-pss-") as temporary:
            directory = Path(temporary)
            env = dict(os.environ, XDG_DATA_HOME=str(directory / "data"),
                       XDG_CONFIG_HOME=str(directory / "config"), GSETTINGS_BACKEND="keyfile")
            log = directory / "app.log"
            with log.open("w") as output, subprocess.Popen(
                [binary], env=env, stderr=output,
            ) as process:
                try:
                    deadline = time.monotonic() + 10
                    while not tray_ready(bus, process.pid):
                        if process.poll() is not None or time.monotonic() >= deadline:
                            raise RuntimeError("QA tray item did not register on the real desktop")
                        time.sleep(0.1)
                    print(f"run={run} tray_ready pid={process.pid}", flush=True)
                    time.sleep(seconds)
                    pss, count = process_tree_pss(process.pid)
                    panels = log.read_text().count("panel_shown")
                    passed &= panels == 0 and pss < 50 * 1024
                    print(f"run={run} idle_seconds={seconds} processes={count} pss_kib={pss} "
                          f"pss_mib={pss / 1024:.2f} panels_opened={panels}",
                          flush=True)
                finally:
                    if process.poll() is None:
                        process.terminate()
                        process.wait(timeout=5)
    return passed


def interrupt(signum, _frame):
    raise SystemExit(128 + signum)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--repeats", default=5, type=int)
    parser.add_argument("--seconds", default=60, type=int)
    parser.add_argument("--temporarily-stop-legacy", type=Path)
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, interrupt)
    binary = args.binary.resolve(strict=True)
    if args.repeats < 1 or args.seconds < 1:
        parser.error("repeats and seconds must be positive")
    with suspend_legacy(args.temporarily_stop_legacy):
        passed = measure(binary, args.repeats, args.seconds)
    raise SystemExit(0 if passed else 1)
