"""用已安装的 legacy 二进制和解包的生产版二进制演练数据升级。"""
import os
from pathlib import Path
import runpy
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile

fixture = runpy.run_path(str(Path(__file__).with_name("slint-x11.py")))
TEXT = "legacy-upgrade-fixture 剪贴板"


def contains_clip(path):
    if not path.is_file():
        return False
    with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as connection:
        return connection.execute(
            "SELECT 1 FROM clips WHERE text_content=?", (TEXT,),
        ).fetchone() is not None


def run_app(binary, env, body):
    with subprocess.Popen([binary], env=env, start_new_session=True) as process:
        try:
            body(process)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=10)


def exercise(legacy, native, root):
    tray = fixture["Tray"]()
    env = dict(
        os.environ, XDG_DATA_HOME=str(root / "data"), XDG_CONFIG_HOME=str(root / "config"),
        GSETTINGS_BACKEND="keyfile",
    )
    data_dir = root / "data" / "com.clippy.app"
    database = data_dir / "clips.db"
    config = data_dir / "config.json"
    backup = data_dir / "before-slint-v0.2"

    def seed_legacy(process):
        fixture["wait_for"](lambda: tray.address, "legacy tray never registered", timeout=12)
        assert process.poll() is None
        fixture["set_clipboard"]("UTF8_STRING", TEXT.encode())
        fixture["wait_for"](lambda: contains_clip(database), "legacy did not store external clipboard")
        assert config.is_file(), "legacy did not create its configuration"

    def open_native(process):
        def ready():
            return tray.address and (backup / "clips.db").is_file()

        fixture["wait_for"](ready, "native app could not open the legacy installation")
        assert process.poll() is None
        assert contains_clip(database)
        assert contains_clip(backup / "clips.db")
        assert (backup / "config.json").read_bytes() == settings_bytes
        with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
            assert connection.execute("PRAGMA quick_check").fetchone() == ("ok",)

    run_app(legacy, env, seed_legacy)
    settings_bytes = config.read_bytes()
    tray.address = None
    run_app(native, env, open_native)

    restored = root / "restored"
    shutil.copytree(backup, restored)
    assert contains_clip(restored / "clips.db")
    assert (restored / "config.json").read_bytes() == settings_bytes
    print("PASS legacy clipboard insertion, native data migration, consistent backup and restore")


if __name__ == "__main__":
    legacy, native = (Path(path).resolve(strict=True) for path in sys.argv[1:])
    with tempfile.TemporaryDirectory(prefix="clippy-slint-upgrade-") as temporary:
        exercise(legacy, native, Path(temporary))
