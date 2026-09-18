#!/usr/bin/env python3
"""生成四向文字行夹具；图片和 corpus SHA 一起固定。"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parent
IMAGES = ROOT / "images"
FONT_PATH = ROOT.parents[2] / "assets" / "fonts" / "NotoSansCJKsc-Medium.otf"
LICENSE = "Generated Clippy fixture text CC0-1.0; Noto Sans CJK SC font OFL-1.1"
TEXT = "方向 OCR 90° SN-A1"
SIZE = (720, 150)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def rotate_point(point, angle):
    x, y = point
    width, height = SIZE
    if angle == 0:
        return [x, y]
    if angle == 90:
        return [height - y, x]
    if angle == 180:
        return [width - x, height - y]
    return [y, width - x]


def main():
    if not FONT_PATH.is_file():
        raise SystemExit(f"missing bundled font: {FONT_PATH}")
    font = ImageFont.truetype(str(FONT_PATH), 44)
    source = Image.new("RGB", SIZE, "#ffffff")
    draw = ImageDraw.Draw(source)
    at = (32, 34)
    draw.text(at, TEXT, font=font, fill="#111827")
    left, top, right, bottom = draw.textbbox(at, TEXT, font=font)
    quad = [[left - 4, top - 4], [right + 4, top - 4], [right + 4, bottom + 4], [left - 4, bottom + 4]]

    IMAGES.mkdir(parents=True, exist_ok=True)
    cases = []
    rotations = {
        0: source,
        90: source.transpose(Image.Transpose.ROTATE_270),
        180: source.transpose(Image.Transpose.ROTATE_180),
        270: source.transpose(Image.Transpose.ROTATE_90),
    }
    for angle, image in rotations.items():
        path = IMAGES / f"line-{angle}.png"
        image.save(path, format="PNG", compress_level=9, optimize=False)
        cases.append({
            "id": f"line-{angle}",
            "tags": ["zh-en", "orientation", f"rotate-{angle}", "high-contrast"],
            "source": {
                "kind": "generated",
                "license": LICENSE,
                "imagePath": f"images/{path.name}",
                "sha256": sha256(path),
                "width": image.width,
                "height": image.height,
            },
            "lines": [{
                "id": f"line-{angle}-1",
                "kind": "text",
                "quad": [rotate_point(point, angle) for point in quad],
                "text": TEXT,
            }],
        })
    (ROOT / "corpus.json").write_text(
        json.dumps({"schema": "clippy-ocr-quality-corpus-v1", "cases": cases}, ensure_ascii=False, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
