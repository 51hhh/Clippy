#!/usr/bin/env python3
"""显式捕获并验证浏览器栅格化 OCR 语料；普通测试只运行 --verify。"""

from __future__ import annotations

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import platform
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
from urllib.parse import parse_qs, urlparse

ROOT = Path(__file__).resolve().parent
SIDECAR = ROOT.parents[1]
FONT_PATH = ROOT.parents[2] / "assets" / "fonts" / "NotoSansCJKsc-Medium.otf"
SOURCE_PATH = ROOT / "source.html"
CORPUS_PATH = ROOT / "corpus.json"
TABLE_CORPUS_PATH = ROOT / "table-corpus.json"
CAPTURE_PATH = ROOT / "capture.json"
IMAGES = ROOT / "images"
LICENSE = "Clippy browser fixture text CC0-1.0; Noto Sans CJK SC font OFL-1.1"
SCENES = {
    "settings-dark": (1280, 720),
    "code-review": (1280, 720),
    "invoice-table": (1280, 760),
    "long-document": (1000, 2400),
}
MAX_CONTRACT_BYTES = 2 * 1024 * 1024


def digest_bytes(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def digest(path: Path) -> str:
    return digest_bytes(path.read_bytes())


def write_json(path: Path, value: object) -> None:
    path.write_text(
        json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )


class ContractState:
    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.event = threading.Event()
        self.expected_scene: str | None = None
        self.payload: dict[str, object] | None = None

    def reset(self, scene: str) -> None:
        with self.lock:
            self.expected_scene = scene
            self.payload = None
            self.event.clear()

    def accept(self, payload: object) -> bool:
        if not isinstance(payload, dict) or payload.get("scene") != self.expected_scene:
            return False
        with self.lock:
            self.payload = payload
            self.event.set()
        return True

    def take(self) -> dict[str, object] | None:
        with self.lock:
            return self.payload


def handler_for(state: ContractState):
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
            request = urlparse(self.path)
            if request.path == "/source.html":
                scene = parse_qs(request.query).get("scene", [""])[0]
                if scene not in SCENES:
                    self.send_error(404)
                    return
                payload = SOURCE_PATH.read_bytes()
                content_type = "text/html; charset=utf-8"
            elif request.path == "/font.otf":
                payload = FONT_PATH.read_bytes()
                content_type = "font/otf"
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(payload)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(payload)

        def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
            if urlparse(self.path).path != "/contract":
                self.send_error(404)
                return
            try:
                length = int(self.headers.get("Content-Length", "0"))
            except ValueError:
                length = 0
            if length <= 0 or length > MAX_CONTRACT_BYTES:
                self.send_error(413)
                return
            try:
                payload = json.loads(self.rfile.read(length))
            except (UnicodeDecodeError, json.JSONDecodeError):
                self.send_error(400)
                return
            if not state.accept(payload):
                self.send_error(409)
                return
            self.send_response(204)
            self.end_headers()

        def log_message(self, _format: str, *_args: object) -> None:
            return

    return Handler


def firefox_version(firefox: str) -> str:
    result = subprocess.run(
        [firefox, "--version"],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=10,
        text=True,
    )
    lines = [line.strip() for line in result.stdout.splitlines() if "Firefox" in line]
    if not lines:
        raise RuntimeError("无法读取 Firefox 版本")
    return lines[-1]


def validate_contract(scene: str, value: object, expected_size: tuple[int, int]) -> dict[str, object]:
    if not isinstance(value, dict) or set(value) != {
        "scene", "tags", "viewport", "fontReady", "lines", "tables"
    }:
        raise RuntimeError(f"{scene} 页面合同字段无效")
    if value["scene"] != scene or value["fontReady"] is not True:
        raise RuntimeError(f"{scene} 字体或页面身份未就绪")
    viewport = value["viewport"]
    if not isinstance(viewport, dict) or set(viewport) != {"width", "height", "devicePixelRatio"}:
        raise RuntimeError(f"{scene} viewport 合同无效")
    if (viewport["width"], viewport["height"]) != expected_size or viewport["devicePixelRatio"] != 1:
        raise RuntimeError(f"{scene} viewport/DPR 与固定捕获合同不符: {viewport}")
    tags = value["tags"]
    lines = value["lines"]
    tables = value["tables"]
    if not isinstance(tags, list) or not tags or len(tags) != len(set(tags)):
        raise RuntimeError(f"{scene} tags 无效")
    if not isinstance(lines, list) or not lines:
        raise RuntimeError(f"{scene} 没有 OCR 行")
    ids: set[str] = set()
    for line in lines:
        if not isinstance(line, dict) or set(line) - {"id", "text", "quad", "kind", "paragraphId"}:
            raise RuntimeError(f"{scene} 行合同无效")
        identifier = line.get("id")
        if not isinstance(identifier, str) or not identifier or identifier in ids:
            raise RuntimeError(f"{scene} 行 id 缺失或重复")
        ids.add(identifier)
        if not isinstance(line.get("text"), str) or line.get("kind") != "text":
            raise RuntimeError(f"{scene}/{identifier} 文字合同无效")
    if not isinstance(tables, list):
        raise RuntimeError(f"{scene} tables 无效")
    return value


def capture_scene(
    firefox: str,
    server: ThreadingHTTPServer,
    state: ContractState,
    scene: str,
    size: tuple[int, int],
    profile: Path,
    destination: Path,
) -> dict[str, object]:
    try:
        from PIL import Image
    except ModuleNotFoundError as error:
        raise RuntimeError("捕获 browser-ui-v1 需要 Pillow") from error
    width, height = size
    url = f"http://127.0.0.1:{server.server_port}/source.html?scene={scene}"
    log_path = destination.with_suffix(".firefox.log")
    for attempt in range(1, 4):
        state.reset(scene)
        command = [
            firefox,
            "--headless",
            "--profile", str(profile),
            "--window-size", f"{width},{height}",
            "--screenshot", str(destination),
            url,
        ]
        result = subprocess.run(
            command,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=40,
        )
        log_path.write_bytes(result.stdout)
        if result.returncode != 0 or not destination.is_file():
            continue
        state.event.wait(timeout=1)
        contract = state.take()
        try:
            with Image.open(destination) as image:
                image.load()
                rendered = image.convert("RGB")
                ready = rendered.getpixel((1, 1)) == (0, 200, 0)
                dimensions = image.size
        except OSError:
            ready = False
            dimensions = (0, 0)
        if ready and dimensions == size and contract is not None:
            # 就绪像素只负责拒绝过早截图；基线图恢复该场景自身的左上角背景，不留下测试标记。
            background = rendered.getpixel((5, 1))
            for y in range(4):
                for x in range(4):
                    rendered.putpixel((x, y), background)
            rendered.save(destination, format="PNG", compress_level=9, optimize=False)
            log_path.unlink(missing_ok=True)
            return validate_contract(scene, contract, size)
        if attempt == 3:
            raise RuntimeError(
                f"{scene} 捕获在 3 次尝试后仍未就绪: return={result.returncode}, "
                f"size={dimensions}, ready={ready}, contract={contract is not None}, log={log_path}"
            )
    raise RuntimeError(f"{scene} Firefox 捕获失败")


def source_for(scene: str, size: tuple[int, int], image: Path) -> dict[str, object]:
    return {
        "kind": "browser-rendered",
        "license": LICENSE,
        "imagePath": f"images/{scene}.png",
        "sha256": digest(image),
        "width": size[0],
        "height": size[1],
    }


def capture(replace: bool) -> None:
    firefox = shutil.which("firefox")
    if firefox is None:
        raise RuntimeError("捕获 browser-ui-v1 需要 Firefox")
    if not SOURCE_PATH.is_file() or not FONT_PATH.is_file():
        raise RuntimeError("浏览器语料源页面或固定字体不存在")
    outputs = [CORPUS_PATH, TABLE_CORPUS_PATH, CAPTURE_PATH, *(IMAGES / f"{scene}.png" for scene in SCENES)]
    if not replace and any(path.exists() for path in outputs):
        raise RuntimeError("浏览器语料已经存在；显式传 --replace 才能重建基线")

    state = ContractState()
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler_for(state))
    thread = threading.Thread(target=server.serve_forever, name="ocr-corpus-http", daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix=".capture-", dir=ROOT) as temporary:
            staging = Path(temporary)
            profile = staging / "firefox-profile"
            staged_images = staging / "images"
            profile.mkdir()
            staged_images.mkdir()
            cases = []
            table_cases = []
            scene_records = {}
            for scene, size in SCENES.items():
                image = staged_images / f"{scene}.png"
                contract = capture_scene(firefox, server, state, scene, size, profile, image)
                source = source_for(scene, size, image)
                cases.append({
                    "id": scene,
                    "tags": contract["tags"],
                    "source": source,
                    "lines": contract["lines"],
                })
                if contract["tables"]:
                    table_cases.append({
                        "id": scene,
                        "tags": contract["tags"],
                        "source": source,
                        "tables": contract["tables"],
                    })
                scene_records[scene] = {
                    "viewport": contract["viewport"],
                    "imageSha256": source["sha256"],
                    "lineCount": len(contract["lines"]),
                    "tableCount": len(contract["tables"]),
                }

            corpus = {"schema": "clippy-ocr-quality-corpus-v1", "cases": cases}
            table_corpus = {"schema": "clippy-ocr-table-corpus-v1", "cases": table_cases}
            capture_record = {
                "schema": "clippy-ocr-browser-capture-v1",
                "firefox": firefox_version(firefox),
                "host": {
                    "system": platform.system(),
                    "release": platform.release(),
                    "machine": platform.machine(),
                },
                "sourceHtmlSha256": digest(SOURCE_PATH),
                "fontSha256": digest(FONT_PATH),
                "captureScriptSha256": digest(Path(__file__)),
                "scenes": scene_records,
            }
            staged_corpus = staging / "corpus.json"
            staged_table = staging / "table-corpus.json"
            staged_capture = staging / "capture.json"
            write_json(staged_corpus, corpus)
            write_json(staged_table, table_corpus)
            write_json(staged_capture, capture_record)

            import sys
            sys.path.insert(0, str(SIDECAR))
            import quality_metrics
            import quality_table

            clean = quality_metrics.validate_corpus(corpus)
            quality_metrics.validate_corpus_assets(staged_corpus, clean)
            clean_table = quality_table.validate_table_corpus(table_corpus)
            for case in clean_table["cases"]:
                quality_metrics.read_case_png(staged_table, case)

            IMAGES.mkdir(exist_ok=True)
            for scene in SCENES:
                (staged_images / f"{scene}.png").replace(IMAGES / f"{scene}.png")
            staged_corpus.replace(CORPUS_PATH)
            staged_table.replace(TABLE_CORPUS_PATH)
            staged_capture.replace(CAPTURE_PATH)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


def verify() -> None:
    import sys
    sys.path.insert(0, str(SIDECAR))
    import quality_metrics
    import quality_table

    corpus = quality_metrics.validate_corpus(quality_metrics.load_json(CORPUS_PATH))
    quality_metrics.validate_corpus_assets(CORPUS_PATH, corpus)
    table_corpus = quality_table.validate_table_corpus(quality_metrics.load_json(TABLE_CORPUS_PATH))
    for case in table_corpus["cases"]:
        quality_metrics.read_case_png(TABLE_CORPUS_PATH, case)
    record = quality_metrics.load_json(CAPTURE_PATH)
    if not isinstance(record, dict) or set(record) != {
        "schema", "firefox", "host", "sourceHtmlSha256", "fontSha256", "captureScriptSha256", "scenes"
    } or record.get("schema") != "clippy-ocr-browser-capture-v1":
        raise RuntimeError("capture.json 合同无效")
    if record["sourceHtmlSha256"] != digest(SOURCE_PATH):
        raise RuntimeError("source.html 已变化但浏览器语料未重新捕获")
    if record["fontSha256"] != digest(FONT_PATH):
        raise RuntimeError("固定字体已变化但浏览器语料未重新捕获")
    if record["captureScriptSha256"] != digest(Path(__file__)):
        raise RuntimeError("capture.py 已变化但捕获记录未更新")
    cases = {case["id"]: case for case in corpus["cases"]}
    if set(cases) != set(SCENES) or set(record["scenes"]) != set(SCENES):
        raise RuntimeError("捕获场景集合不完整")
    for scene, size in SCENES.items():
        case = cases[scene]
        scene_record = record["scenes"][scene]
        if (
            (case["source"]["width"], case["source"]["height"]) != size
            or scene_record.get("imageSha256") != case["source"]["sha256"]
            or scene_record.get("lineCount") != len(case["lines"])
        ):
            raise RuntimeError(f"{scene} 捕获记录与 corpus 不一致")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--capture", action="store_true")
    mode.add_argument("--verify", action="store_true")
    parser.add_argument("--replace", action="store_true")
    arguments = parser.parse_args()
    if arguments.replace and not arguments.capture:
        parser.error("--replace 只能与 --capture 一起使用")
    try:
        if arguments.capture:
            capture(arguments.replace)
        else:
            verify()
    except (OSError, RuntimeError, subprocess.SubprocessError, ValueError) as error:
        print(f"ocr-browser-corpus: {error}")
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
