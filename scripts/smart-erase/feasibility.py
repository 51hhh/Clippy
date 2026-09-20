#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import random
import resource
import shutil
import statistics
import sys
import time
from pathlib import Path

import cv2
import numpy as np
import onnxruntime as ort
from PIL import Image, ImageDraw, ImageFilter, ImageFont


ROOT = Path(__file__).resolve().parent
CORPUS = ROOT / "corpus"
EVIDENCE = ROOT / "evidence"
OUTPUT = EVIDENCE / "outputs"
FONT_PATH = Path("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")
MODEL_URL = "https://huggingface.co/opencv/inpainting_lama/resolve/main/inpainting_lama_2025jan.onnx"
MODEL_SHA256 = "7df918ac3921d3daf0aae1d219776cf0dc4e4935f035af81841b40adcf74fdf2"
METHODS = ("telea", "navier_stokes", "lama_roi")
BLIND_SEED = 20260922


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def cpu_name() -> str:
    if platform.processor():
        return platform.processor()
    try:
        for line in Path("/proc/cpuinfo").read_text(encoding="utf-8").splitlines():
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return "unknown"


def peak_rss_bytes() -> int:
    value = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return int(value if sys.platform == "darwin" else value * 1024)


def save_case(case_id: str, category: str, reference: Image.Image, input_image: Image.Image, mask: Image.Image) -> dict:
    directory = CORPUS / case_id
    directory.mkdir(parents=True, exist_ok=True)
    paths = {
        "reference": directory / "reference.png",
        "input": directory / "input.png",
        "mask": directory / "mask.png",
    }
    reference.save(paths["reference"], optimize=True)
    input_image.save(paths["input"], optimize=True)
    mask.save(paths["mask"], optimize=True)
    return {
        "id": case_id,
        "category": category,
        "width": reference.width,
        "height": reference.height,
        "files": {name: str(path.relative_to(ROOT)) for name, path in paths.items()},
        "sha256": {name: sha256(path) for name, path in paths.items()},
    }


def overlay_object(reference: Image.Image, draw_object, mask_object) -> tuple[Image.Image, Image.Image]:
    input_image = reference.copy()
    draw_object(ImageDraw.Draw(input_image))
    mask = Image.new("L", reference.size, 0)
    mask_object(ImageDraw.Draw(mask))
    return input_image, mask.filter(ImageFilter.MaxFilter(9))


def build_text_case() -> dict:
    width, height = 768, 512
    yy, xx = np.mgrid[:height, :width]
    array = np.empty((height, width, 3), np.uint8)
    array[..., 0] = 242 - (xx * 15 // width)
    array[..., 1] = 246 - (yy * 12 // height)
    array[..., 2] = 250 - ((xx + yy) * 10 // (width + height))
    reference = Image.fromarray(array, "RGB")
    draw = ImageDraw.Draw(reference)
    for y in range(46, height, 34):
        draw.line((28, y, width - 28, y), fill=(186, 206, 225), width=1)
    draw.line((92, 20, 92, height - 20), fill=(228, 160, 160), width=2)
    font = ImageFont.truetype(str(FONT_PATH), 44)
    text = "Invoice #2026-091"
    bbox = ImageDraw.Draw(Image.new("RGB", (1, 1))).textbbox((0, 0), text, font=font)
    x, y = 146, 212
    rect = (x - 8, y - 6, x + bbox[2] + 8, y + bbox[3] + 10)
    return save_case(
        "text_ruled_paper", "text",
        reference,
        *overlay_object(
            reference,
            lambda d: d.text((x, y), text, font=font, fill=(28, 35, 46)),
            lambda d: d.rounded_rectangle(rect, radius=8, fill=255),
        ),
    )


def build_grid_case() -> dict:
    width, height = 768, 512
    reference = Image.new("RGB", (width, height), (239, 240, 234))
    draw = ImageDraw.Draw(reference)
    for x in range(0, width, 32):
        draw.line((x, 0, x, height), fill=(83, 125, 139), width=3 if x % 128 == 0 else 1)
    for y in range(0, height, 32):
        draw.line((0, y, width, y), fill=(139, 105, 83), width=3 if y % 128 == 0 else 1)
    rect = (294, 166, 474, 346)
    return save_case(
        "regular_grid", "regular-background",
        reference,
        *overlay_object(
            reference,
            lambda d: (d.rounded_rectangle(rect, radius=24, fill=(214, 67, 76)), d.ellipse((334, 206, 434, 306), fill=(255, 223, 92))),
            lambda d: d.rounded_rectangle((286, 158, 482, 354), radius=30, fill=255),
        ),
    )


def fractal_noise(width: int, height: int, seed: int) -> np.ndarray:
    rng = np.random.default_rng(seed)
    accum = np.zeros((height, width), np.float32)
    weight = 0.0
    for scale, factor in ((8, 1.0), (18, 0.65), (42, 0.35), (96, 0.18)):
        small = rng.random((max(2, height // scale), max(2, width // scale)), dtype=np.float32)
        layer = cv2.resize(small, (width, height), interpolation=cv2.INTER_CUBIC)
        accum += layer * factor
        weight += factor
    return np.clip(accum / weight, 0, 1)


def build_natural_case() -> dict:
    width, height = 768, 512
    noise = fractal_noise(width, height, 419)
    yy, xx = np.mgrid[:height, :width]
    sky = np.clip(0.2 + yy / height * 0.8, 0, 1)
    array = np.empty((height, width, 3), np.uint8)
    array[..., 0] = np.clip(42 + 95 * noise + 32 * sky, 0, 255)
    array[..., 1] = np.clip(84 + 105 * noise - 18 * sky, 0, 255)
    array[..., 2] = np.clip(48 + 58 * noise + 12 * np.sin(xx / 31), 0, 255)
    reference = Image.fromarray(array, "RGB")
    draw = ImageDraw.Draw(reference, "RGBA")
    rng = random.Random(77)
    for _ in range(48):
        x = rng.randrange(width)
        y = rng.randrange(height)
        r = rng.randrange(6, 24)
        draw.ellipse((x - r, y - r // 2, x + r, y + r // 2), fill=(24, 92 + rng.randrange(50), 40, 120))
    box = (290, 160, 478, 356)
    return save_case(
        "natural_foliage", "natural-image",
        reference,
        *overlay_object(
            reference,
            lambda d: (d.rectangle(box, fill=(45, 82, 185)), d.polygon(((290, 160), (478, 160), (384, 98)), fill=(238, 225, 195))),
            lambda d: (d.rectangle((282, 152, 486, 364), fill=255), d.polygon(((282, 158), (486, 158), (384, 86)), fill=255)),
        ),
    )


def build_edge_case() -> dict:
    width, height = 768, 512
    reference = Image.new("RGB", (width, height), (245, 204, 93))
    draw = ImageDraw.Draw(reference)
    points = [(0, 380), (130, 318), (280, 342), (430, 225), (570, 250), (768, 120), (768, 512), (0, 512)]
    draw.polygon(points, fill=(37, 86, 137))
    draw.line(points[:6], fill=(248, 248, 246), width=9, joint="curve")
    box = (322, 192, 454, 330)
    return save_case(
        "strong_edge", "edge",
        reference,
        *overlay_object(
            reference,
            lambda d: (d.ellipse(box, fill=(206, 47, 51)), d.line((344, 214, 432, 308), fill=(255, 255, 255), width=18)),
            lambda d: d.ellipse((310, 180, 466, 342), fill=255),
        ),
    )


def build_large_case() -> dict:
    width, height = 2048, 1536
    yy, xx = np.mgrid[:height, :width]
    array = np.empty((height, width, 3), np.uint8)
    array[..., 0] = 228 + ((xx // 24 + yy // 24) % 2) * 8
    array[..., 1] = 232 + ((xx // 24 + yy // 24) % 2) * 7
    array[..., 2] = 236 + ((xx // 24 + yy // 24) % 2) * 6
    reference = Image.fromarray(array, "RGB")
    draw = ImageDraw.Draw(reference)
    for column in range(3):
        left = 120 + column * 640
        draw.rounded_rectangle((left, 110, left + 530, 1420), radius=18, fill=(250, 250, 248), outline=(148, 156, 167), width=4)
        for y in range(180, 1360, 58):
            length = 370 + ((y // 58 + column) % 4) * 35
            draw.rounded_rectangle((left + 54, y, left + 54 + length, y + 10), radius=5, fill=(80, 91, 103))
    font = ImageFont.truetype(str(FONT_PATH), 104)
    text = "REVIEW"
    box = (770, 590, 1300, 930)
    return save_case(
        "large_document", "large-image",
        reference,
        *overlay_object(
            reference,
            lambda d: (d.rounded_rectangle(box, radius=44, fill=(186, 38, 58), outline=(121, 16, 29), width=14), d.text((818, 690), text, font=font, fill=(255, 236, 235))),
            lambda d: d.rounded_rectangle((748, 568, 1322, 952), radius=58, fill=255),
        ),
    )


def generate() -> None:
    CORPUS.mkdir(parents=True, exist_ok=True)
    cases = [build_text_case(), build_grid_case(), build_natural_case(), build_edge_case(), build_large_case()]
    manifest = {
        "schemaVersion": 1,
        "id": "PX-SMART-01-corpus-v1",
        "source": "deterministically generated by feasibility.py",
        "license": "covered by the repository license",
        "generator": {"python": platform.python_version(), "pillow": Image.__version__, "numpy": np.__version__},
        "font": {"name": "DejaVu Sans", "sha256": sha256(FONT_PATH)},
        "cases": cases,
    }
    (CORPUS / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")


def percentile(values: list[float], q: float) -> float:
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    position = (len(ordered) - 1) * q
    lower = math.floor(position)
    upper = math.ceil(position)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def lama_roi(session: ort.InferenceSession, image: np.ndarray, mask: np.ndarray) -> np.ndarray:
    coords = cv2.findNonZero(mask)
    if coords is None:
        return image.copy()
    x, y, width, height = cv2.boundingRect(coords)
    padding = max(32, int(round(max(width, height) * 0.5)))
    left, top = max(0, x - padding), max(0, y - padding)
    right, bottom = min(image.shape[1], x + width + padding), min(image.shape[0], y + height + padding)
    crop, crop_mask = image[top:bottom, left:right], mask[top:bottom, left:right]
    side = max(crop.shape[:2])
    pad_top = (side - crop.shape[0]) // 2
    pad_bottom = side - crop.shape[0] - pad_top
    pad_left = (side - crop.shape[1]) // 2
    pad_right = side - crop.shape[1] - pad_left
    square = cv2.copyMakeBorder(crop, pad_top, pad_bottom, pad_left, pad_right, cv2.BORDER_REFLECT_101)
    square_mask = cv2.copyMakeBorder(crop_mask, pad_top, pad_bottom, pad_left, pad_right, cv2.BORDER_CONSTANT, value=0)
    resized = cv2.resize(square, (512, 512), interpolation=cv2.INTER_AREA)
    resized_mask = cv2.resize(square_mask, (512, 512), interpolation=cv2.INTER_NEAREST)
    image_blob = resized.astype(np.float32).transpose(2, 0, 1)[None] * np.float32(0.00392)
    mask_blob = (resized_mask > 0).astype(np.float32)[None, None]
    raw = session.run(None, {"image": image_blob, "mask": mask_blob})[0][0]
    restored = np.clip(raw.transpose(1, 2, 0), 0, 255).astype(np.uint8)
    restored = cv2.resize(restored, (side, side), interpolation=cv2.INTER_CUBIC)
    restored = restored[pad_top:pad_top + crop.shape[0], pad_left:pad_left + crop.shape[1]]
    result = image.copy()
    region = result[top:bottom, left:right]
    region[crop_mask > 0] = restored[crop_mask > 0]
    return result


def run_method(method: str, session: ort.InferenceSession | None, image: np.ndarray, mask: np.ndarray) -> np.ndarray:
    if method == "telea":
        return cv2.inpaint(image, mask, 3, cv2.INPAINT_TELEA)
    if method == "navier_stokes":
        return cv2.inpaint(image, mask, 3, cv2.INPAINT_NS)
    assert session is not None
    return lama_roi(session, image, mask)


def create_session(model: Path) -> tuple[ort.InferenceSession, float]:
    options = ort.SessionOptions()
    options.log_severity_level = 3
    options.intra_op_num_threads = max(1, min(4, os.cpu_count() or 1))
    options.inter_op_num_threads = 1
    started = time.perf_counter()
    session = ort.InferenceSession(str(model), sess_options=options, providers=["CPUExecutionProvider"])
    return session, (time.perf_counter() - started) * 1000


def metrics(result: np.ndarray, reference: np.ndarray, mask: np.ndarray) -> dict:
    selected = mask > 0
    difference = np.abs(result.astype(np.float32) - reference.astype(np.float32))
    mse = float(np.mean(np.square(difference[selected])))
    boundary = cv2.dilate(mask, np.ones((7, 7), np.uint8)) > 0
    boundary &= cv2.erode(mask, np.ones((7, 7), np.uint8)) == 0
    result_gray = cv2.cvtColor(result, cv2.COLOR_BGR2GRAY).astype(np.float32)
    reference_gray = cv2.cvtColor(reference, cv2.COLOR_BGR2GRAY).astype(np.float32)
    grad_result = cv2.magnitude(cv2.Sobel(result_gray, cv2.CV_32F, 1, 0), cv2.Sobel(result_gray, cv2.CV_32F, 0, 1))
    grad_reference = cv2.magnitude(cv2.Sobel(reference_gray, cv2.CV_32F, 1, 0), cv2.Sobel(reference_gray, cv2.CV_32F, 0, 1))
    return {
        "maskedMae": round(float(np.mean(difference[selected])), 4),
        "maskedPsnr": round(99.0 if mse == 0 else 10 * math.log10((255 * 255) / mse), 4),
        "boundaryMae": round(float(np.mean(difference[boundary])), 4),
        "maskedGradientMae": round(float(np.mean(np.abs(grad_result[selected] - grad_reference[selected]))), 4),
        "outsideExact": bool(np.array_equal(result[~selected], reference[~selected])),
    }


def benchmark(model: Path, repeats: int) -> None:
    if repeats < 1 or repeats > 20:
        raise SystemExit("repeats must be between 1 and 20")
    if sha256(model) != MODEL_SHA256:
        raise SystemExit("model sha256 mismatch")
    manifest = json.loads((CORPUS / "manifest.json").read_text())
    session, session_ms = create_session(model)
    OUTPUT.mkdir(parents=True, exist_ok=True)
    results = []
    for case in manifest["cases"]:
        image = cv2.imread(str(ROOT / case["files"]["input"]), cv2.IMREAD_COLOR)
        mask = cv2.imread(str(ROOT / case["files"]["mask"]), cv2.IMREAD_GRAYSCALE)
        reference = cv2.imread(str(ROOT / case["files"]["reference"]), cv2.IMREAD_COLOR)
        for method in METHODS:
            if method == "lama_roi":
                run_method(method, session, image, mask)
            timings = []
            output = None
            for _ in range(repeats):
                start = time.perf_counter()
                output = run_method(method, session, image, mask)
                timings.append((time.perf_counter() - start) * 1000)
            assert output is not None
            out_path = OUTPUT / f"{case['id']}--{method}.png"
            cv2.imwrite(str(out_path), output, [cv2.IMWRITE_PNG_COMPRESSION, 9])
            results.append({
                "caseId": case["id"], "category": case["category"], "method": method,
                "timingMs": {
                    "samples": [round(value, 3) for value in timings],
                    "p50": round(statistics.median(timings), 3),
                    "p95": round(percentile(timings, 0.95), 3),
                },
                "metrics": metrics(output, reference, mask),
                "output": str(out_path.relative_to(ROOT)),
                "sha256": sha256(out_path),
            })
    report = {
        "schemaVersion": 1,
        "corpusId": manifest["id"],
        "corpusManifestSha256": sha256(CORPUS / "manifest.json"),
        "host": {
            "platform": platform.platform(), "cpu": cpu_name(), "logicalCpus": os.cpu_count(),
            "python": platform.python_version(), "opencv": cv2.__version__, "onnxruntime": ort.__version__,
        },
        "candidate": {
            "name": "OpenCV Zoo quantized LaMa", "source": MODEL_URL, "license": "Apache-2.0",
            "modelSha256": MODEL_SHA256,
            "modelBytes": model.stat().st_size, "sessionLoadMs": round(session_ms, 3),
        },
        "processPeakRssBytes": peak_rss_bytes(),
        "repeats": repeats,
        "results": results,
    }
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    (EVIDENCE / "benchmark.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def resource_probe(model: Path) -> None:
    if sha256(model) != MODEL_SHA256:
        raise SystemExit("model sha256 mismatch")
    case = json.loads((CORPUS / "manifest.json").read_text(encoding="utf-8"))["cases"][0]
    image = cv2.imread(str(ROOT / case["files"]["input"]), cv2.IMREAD_COLOR)
    mask = cv2.imread(str(ROOT / case["files"]["mask"]), cv2.IMREAD_GRAYSCALE)
    session, session_ms = create_session(model)
    started = time.perf_counter()
    result = lama_roi(session, image, mask)
    infer_ms = (time.perf_counter() - started) * 1000
    if np.array_equal(result, image):
        raise SystemExit("candidate returned an unchanged masked image")
    report = {
        "schemaVersion": 1,
        "caseId": case["id"],
        "host": {"platform": platform.platform(), "cpu": cpu_name(), "logicalCpus": os.cpu_count()},
        "runtime": {"onnxruntime": ort.__version__, "threads": max(1, min(4, os.cpu_count() or 1))},
        "modelSha256": MODEL_SHA256,
        "sessionLoadMs": round(session_ms, 3),
        "inferenceMs": round(infer_ms, 3),
        "processPeakRssBytes": peak_rss_bytes(),
    }
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    (EVIDENCE / "resource-probe.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def blind_pack() -> None:
    manifest = json.loads((CORPUS / "manifest.json").read_text())
    rng = random.Random(BLIND_SEED)
    key = {}
    rows = []
    assets = EVIDENCE / "blind-assets"
    assets.mkdir(parents=True, exist_ok=True)
    for case in manifest["cases"]:
        shuffled = list(METHODS)
        rng.shuffle(shuffled)
        labels = ["A", "B", "C"]
        key[case["id"]] = dict(zip(labels, shuffled, strict=True))
        cells = []
        for label, method in zip(labels, shuffled, strict=True):
            anonymous_name = f"{case['id']}--{label}.png"
            shutil.copyfile(OUTPUT / f"{case['id']}--{method}.png", assets / anonymous_name)
            cells.append(f'<figure><img src="blind-assets/{anonymous_name}"><figcaption>{label}</figcaption></figure>')
        rows.append(f'<section><h2>{case["category"]}</h2><div>{"".join(cells)}</div></section>')
    html = """<!doctype html><html><meta charset=\"utf-8\"><title>PX-SMART-01 blind review</title>
<style>body{font:16px system-ui;margin:24px;background:#15171c;color:#eee}section{margin:28px 0;padding:18px;background:#20242c;border-radius:12px}section>div{display:grid;grid-template-columns:repeat(3,1fr);gap:14px}figure{margin:0}img{width:100%;max-height:420px;object-fit:contain;background:#fff}figcaption{text-align:center;font-size:22px;font-weight:700;margin-top:8px}</style>
<h1>PX-SMART-01 anonymous outputs</h1><p>Rank A/B/C for structure continuity, texture plausibility and absence of seams. The method key is stored separately.</p>""" + "".join(rows) + "</html>"
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    (EVIDENCE / "blind-review.html").write_text(html, encoding="utf-8")
    (EVIDENCE / "blind-key.json").write_text(json.dumps(key, indent=2) + "\n", encoding="utf-8")


def blind_sheets() -> None:
    manifest = json.loads((CORPUS / "manifest.json").read_text(encoding="utf-8"))
    key = json.loads((EVIDENCE / "blind-key.json").read_text(encoding="utf-8"))
    font = ImageFont.truetype(str(FONT_PATH), 22)
    directory = EVIDENCE / "sheets"
    directory.mkdir(parents=True, exist_ok=True)
    sheet_records = []
    for case in manifest["cases"]:
        reference = Image.open(ROOT / case["files"]["reference"]).convert("RGB")
        scale = min(620 / reference.width, 340 / reference.height, 1)
        size = (max(1, int(reference.width * scale)), max(1, int(reference.height * scale)))
        panels = [("Reference", reference.resize(size))]
        for label in ("A", "B", "C"):
            method = key[case["id"]][label]
            path = OUTPUT / f"{case['id']}--{method}.png"
            panels.append((label, Image.open(path).convert("RGB").resize(size)))
        sheet = Image.new("RGB", (size[0] * 2 + 48, size[1] * 2 + 100), (24, 27, 33))
        draw = ImageDraw.Draw(sheet)
        for index, (label, panel) in enumerate(panels):
            x = 16 + (index % 2) * (size[0] + 16)
            y = 42 + (index // 2) * (size[1] + 46)
            sheet.paste(panel, (x, y))
            draw.text((x, y - 30), label, font=font, fill=(245, 245, 245))
        sheet_path = directory / f"{case['id']}.png"
        sheet.save(sheet_path, optimize=True)
        sheet_records.append({
            "caseId": case["id"],
            "path": str(sheet_path.relative_to(ROOT)),
            "sha256": sha256(sheet_path),
            "width": sheet.width,
            "height": sheet.height,
        })
    (EVIDENCE / "sheets-manifest.json").write_text(json.dumps({
        "schemaVersion": 1,
        "corpusId": manifest["id"],
        "sheets": sheet_records,
    }, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("generate", "benchmark", "resource-probe", "blind-pack", "blind-sheets"))
    parser.add_argument("--model", type=Path, default=ROOT / "inpainting_lama_2025jan.onnx")
    parser.add_argument("--repeats", type=int, default=5)
    args = parser.parse_args()
    if args.command == "generate": generate()
    elif args.command == "benchmark": benchmark(args.model, args.repeats)
    elif args.command == "resource-probe": resource_probe(args.model)
    elif args.command == "blind-pack": blind_pack()
    else: blind_sheets()


if __name__ == "__main__":
    main()
