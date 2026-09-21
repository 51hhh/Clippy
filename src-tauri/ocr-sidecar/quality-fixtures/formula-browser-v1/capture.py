#!/usr/bin/env python3
"""显式捕获并验证浏览器 MathML 公式 crop；普通测试只运行 --verify。"""

from __future__ import annotations

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import math
import platform
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
from urllib.parse import parse_qs, urlparse

ROOT = Path(__file__).resolve().parent
SIDECAR = ROOT.parents[1]
CJK_FONT_PATH = ROOT.parents[2] / "assets" / "fonts" / "NotoSansCJKsc-Medium.otf"
MATH_FONT_PATH = ROOT.parents[2] / "assets" / "fonts" / "NotoSansMath-Regular.ttf"
SOURCE_PATH = ROOT / "source.html"
CORPUS_PATH = ROOT / "corpus.json"
CAPTURE_PATH = ROOT / "capture.json"
IMAGES = ROOT / "images"
LICENSE = "Clippy MathML fixture formulas CC0-1.0; Noto Sans CJK SC and Noto Sans Math fonts OFL-1.1"
SCENES = {
    "fraction-radical": (960, 300),
    "integral-limit": (1100, 340),
    "summation": (960, 340),
    "matrix": (900, 420),
    "piecewise": (1050, 460),
    "greek-subscript": (960, 300),
}
MAX_CONTRACT_BYTES = 64 * 1024
CROP_MARGIN = 28


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


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
        with self.lock:
            if not isinstance(payload, dict) or payload.get("scene") != self.expected_scene:
                return False
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
                payload, content_type = SOURCE_PATH.read_bytes(), "text/html; charset=utf-8"
            elif request.path == "/text-font.otf":
                payload, content_type = CJK_FONT_PATH.read_bytes(), "font/otf"
            elif request.path == "/math-font.ttf":
                payload, content_type = MATH_FONT_PATH.read_bytes(), "font/ttf"
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
            if not 0 < length <= MAX_CONTRACT_BYTES:
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


def validate_contract(scene: str, value: object, viewport: tuple[int, int]) -> dict[str, object]:
    required = {"scene", "viewport", "fontReady", "tags", "line"}
    if not isinstance(value, dict) or set(value) != required or value.get("scene") != scene:
        raise RuntimeError(f"{scene} 页面合同字段无效")
    size = value.get("viewport")
    if not isinstance(size, dict) or (size.get("width"), size.get("height")) != viewport or size.get("devicePixelRatio") != 1:
        raise RuntimeError(f"{scene} viewport/DPR 与固定合同不符")
    if value.get("fontReady") is not True:
        raise RuntimeError(f"{scene} 固定字体未就绪")
    tags = value.get("tags")
    line = value.get("line")
    if not isinstance(tags, list) or not tags or len(tags) != len(set(tags)):
        raise RuntimeError(f"{scene} tags 无效")
    if not isinstance(line, dict) or set(line) != {"id", "kind", "text", "formula", "quad"}:
        raise RuntimeError(f"{scene} 公式行合同无效")
    formula = line.get("formula")
    quad = line.get("quad")
    if (
        line.get("kind") != "formula"
        or not isinstance(line.get("text"), str)
        or not isinstance(formula, dict)
        or formula.get("format") != "latex"
        or not isinstance(formula.get("value"), str)
        or not isinstance(quad, list)
        or len(quad) != 4
    ):
        raise RuntimeError(f"{scene} 公式真值无效")
    return value


def capture_scene(
    firefox: str,
    server: ThreadingHTTPServer,
    state: ContractState,
    scene: str,
    viewport: tuple[int, int],
    profile: Path,
    destination: Path,
) -> tuple[dict[str, object], tuple[int, int]]:
    try:
        from PIL import Image
    except ModuleNotFoundError as error:
        raise RuntimeError("捕获 formula-browser-v1 需要 Pillow") from error
    width, height = viewport
    url = f"http://127.0.0.1:{server.server_port}/source.html?scene={scene}"
    raw = destination.with_suffix(".viewport.png")
    log = destination.with_suffix(".firefox.log")
    for attempt in range(1, 4):
        state.reset(scene)
        result = subprocess.run(
            [firefox, "--headless", "--profile", str(profile), "--window-size", f"{width},{height}", "--screenshot", str(raw), url],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=40,
        )
        log.write_bytes(result.stdout)
        state.event.wait(timeout=1)
        contract = state.take()
        if result.returncode != 0 or not raw.is_file() or contract is None:
            continue
        contract = validate_contract(scene, contract, viewport)
        try:
            with Image.open(raw) as opened:
                image = opened.convert("RGB")
                ready = image.size == viewport and image.getpixel((1, 1)) == (0, 200, 0)
        except OSError:
            ready = False
        if not ready:
            continue
        quad = contract["line"]["quad"]
        xs = [point[0] for point in quad]
        ys = [point[1] for point in quad]
        left = max(0, math.floor(min(xs)) - CROP_MARGIN)
        top = max(0, math.floor(min(ys)) - CROP_MARGIN)
        right = min(width, math.ceil(max(xs)) + CROP_MARGIN)
        bottom = min(height, math.ceil(max(ys)) + CROP_MARGIN)
        if right - left < 64 or bottom - top < 64:
            raise RuntimeError(f"{scene} 公式 crop 尺寸异常")
        cropped = image.crop((left, top, right, bottom))
        cropped.save(destination, format="PNG", compress_level=9, optimize=False)
        contract["line"]["quad"] = [[point[0] - left, point[1] - top] for point in quad]
        raw.unlink(missing_ok=True)
        log.unlink(missing_ok=True)
        return contract, cropped.size
    raise RuntimeError(f"{scene} Firefox 捕获在 3 次尝试后仍未就绪: {log}")


def firefox_version(firefox: str) -> str:
    result = subprocess.run(
        [firefox, "--version"], check=True, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, timeout=10, text=True,
    )
    return next((line.strip() for line in result.stdout.splitlines() if "Firefox" in line), "unknown")


def capture(replace: bool) -> None:
    firefox = shutil.which("firefox")
    if firefox is None:
        raise RuntimeError("捕获 formula-browser-v1 需要 Firefox")
    outputs = [CORPUS_PATH, CAPTURE_PATH, *(IMAGES / f"{scene}.png" for scene in SCENES)]
    if not replace and any(path.exists() for path in outputs):
        raise RuntimeError("公式语料已经存在；显式传 --replace 才能重建基线")
    state = ContractState()
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler_for(state))
    thread = threading.Thread(target=server.serve_forever, name="formula-corpus-http", daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix=".capture-", dir=ROOT) as temporary:
            staging = Path(temporary)
            profile = staging / "firefox-profile"
            staged_images = staging / "images"
            profile.mkdir()
            staged_images.mkdir()
            cases = []
            records = {}
            for scene, viewport in SCENES.items():
                image = staged_images / f"{scene}.png"
                contract, image_size = capture_scene(firefox, server, state, scene, viewport, profile, image)
                source = {
                    "kind": "browser-mathml-crop",
                    "license": LICENSE,
                    "imagePath": f"images/{scene}.png",
                    "sha256": digest(image),
                    "width": image_size[0],
                    "height": image_size[1],
                }
                cases.append({"id": scene, "tags": contract["tags"], "source": source, "lines": [contract["line"]]})
                records[scene] = {
                    "viewport": {"width": viewport[0], "height": viewport[1], "devicePixelRatio": 1},
                    "image": {"width": image_size[0], "height": image_size[1], "sha256": source["sha256"]},
                }
            corpus = {"schema": "clippy-ocr-quality-corpus-v1", "cases": cases}
            record = {
                "schema": "clippy-ocr-formula-browser-capture-v1",
                "firefox": firefox_version(firefox),
                "host": {"system": platform.system(), "release": platform.release(), "machine": platform.machine()},
                "sourceHtmlSha256": digest(SOURCE_PATH),
                "fontSha256": digest(CJK_FONT_PATH),
                "mathFontSha256": digest(MATH_FONT_PATH),
                "captureScriptSha256": digest(Path(__file__)),
                "scenes": records,
            }
            staged_corpus, staged_record = staging / "corpus.json", staging / "capture.json"
            write_json(staged_corpus, corpus)
            write_json(staged_record, record)
            import sys
            sys.path.insert(0, str(SIDECAR))
            import quality_metrics
            clean = quality_metrics.validate_corpus(corpus)
            quality_metrics.validate_corpus_assets(staged_corpus, clean)
            IMAGES.mkdir(exist_ok=True)
            for scene in SCENES:
                (staged_images / f"{scene}.png").replace(IMAGES / f"{scene}.png")
            staged_corpus.replace(CORPUS_PATH)
            staged_record.replace(CAPTURE_PATH)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


def verify() -> None:
    import sys
    sys.path.insert(0, str(SIDECAR))
    import quality_metrics
    corpus = quality_metrics.validate_corpus(quality_metrics.load_json(CORPUS_PATH))
    quality_metrics.validate_corpus_assets(CORPUS_PATH, corpus)
    record = quality_metrics.load_json(CAPTURE_PATH)
    required = {
        "schema", "firefox", "host", "sourceHtmlSha256", "fontSha256", "mathFontSha256",
        "captureScriptSha256", "scenes",
    }
    if not isinstance(record, dict) or set(record) != required or record.get("schema") != "clippy-ocr-formula-browser-capture-v1":
        raise RuntimeError("capture.json 合同无效")
    if (
        record["sourceHtmlSha256"] != digest(SOURCE_PATH)
        or record["fontSha256"] != digest(CJK_FONT_PATH)
        or record["mathFontSha256"] != digest(MATH_FONT_PATH)
    ):
        raise RuntimeError("公式页面或固定字体已变化但语料未重新捕获")
    if record["captureScriptSha256"] != digest(Path(__file__)):
        raise RuntimeError("capture.py 已变化但捕获记录未更新")
    cases = {case["id"]: case for case in corpus["cases"]}
    if set(cases) != set(SCENES) or set(record["scenes"]) != set(SCENES):
        raise RuntimeError("公式捕获场景集合不完整")
    for scene, viewport in SCENES.items():
        case, scene_record = cases[scene], record["scenes"][scene]
        source, image = case["source"], scene_record.get("image", {})
        if (
            len(case["lines"]) != 1
            or case["lines"][0]["kind"] != "formula"
            or scene_record.get("viewport") != {"width": viewport[0], "height": viewport[1], "devicePixelRatio": 1}
            or image.get("sha256") != source["sha256"]
            or (image.get("width"), image.get("height")) != (source["width"], source["height"])
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
        capture(arguments.replace) if arguments.capture else verify()
    except (OSError, RuntimeError, subprocess.SubprocessError, ValueError) as error:
        print(f"ocr-formula-browser-corpus: {error}")
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
