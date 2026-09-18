#!/usr/bin/env python3
"""生成英语空白路由质量夹具；输出由 corpus SHA 固定。"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parent
IMAGES = ROOT / "images"
FONT_PATH = ROOT.parents[2] / "assets" / "fonts" / "NotoSansCJKsc-Medium.otf"
LICENSE = "Generated Clippy fixture text CC0-1.0; Noto Sans CJK SC font OFL-1.1"
CASES = [
    ("prose-double-space", "Save  as  editable  copy", ["english", "spaces", "prose"]),
    ("repeated-identifiers", "AA  11  ll  00  AABB", ["english", "spaces", "identifiers"]),
    ("serial-punctuation", "SN: AA11-BB22 / 98.6%  [OK]", ["english", "spaces", "punctuation"]),
    ("currency-guard", "USD 12.50  EUR 9.99  ¥1,234.50", ["english", "spaces", "currency"]),
    ("linear-math", "x² + 1/2 = √2", ["english", "spaces", "symbols"]),
    ("source-code", "const value = foo_bar + 42;", ["english", "spaces", "code"]),
]


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def render(case_id: str, text: str, tags: list[str]) -> dict:
    image = Image.new("RGB", (1000, 112), "#ffffff")
    draw = ImageDraw.Draw(image)
    font = ImageFont.truetype(str(FONT_PATH), 40)
    at = (28, 18)
    draw.text(at, text, font=font, fill="#111827")
    left, top, right, bottom = draw.textbbox(at, text, font=font)
    padding = 4
    quad = [
        [max(0, left - padding), max(0, top - padding)],
        [min(image.width, right + padding), max(0, top - padding)],
        [min(image.width, right + padding), min(image.height, bottom + padding)],
        [max(0, left - padding), min(image.height, bottom + padding)],
    ]
    path = IMAGES / f"{case_id}.png"
    image.save(path, format="PNG", compress_level=9, optimize=False)
    return {
        "id": case_id,
        "tags": tags,
        "source": {
            "kind": "generated",
            "license": LICENSE,
            "imagePath": f"images/{path.name}",
            "sha256": sha256(path),
            "width": image.width,
            "height": image.height,
        },
        "lines": [{"id": f"{case_id}-1", "kind": "text", "quad": quad, "text": text}],
    }


def main() -> None:
    if not FONT_PATH.is_file():
        raise SystemExit(f"missing bundled font: {FONT_PATH}")
    IMAGES.mkdir(parents=True, exist_ok=True)
    corpus = {
        "schema": "clippy-ocr-quality-corpus-v1",
        "cases": [render(*case) for case in CASES],
    }
    (ROOT / "corpus.json").write_text(
        json.dumps(corpus, ensure_ascii=False, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
