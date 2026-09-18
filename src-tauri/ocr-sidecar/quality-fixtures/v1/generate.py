#!/usr/bin/env python3
"""生成 Clippy 自有 OCR v1 合同夹具；输出由 corpus SHA 固定。"""

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
    {
        "id": "mixed-script-spacing",
        "size": [1000, 230],
        "fontSize": 40,
        "background": "#ffffff",
        "foreground": "#111827",
        "tags": ["zh-en-ja", "spaces", "numbers", "currency", "high-contrast"],
        "lines": [
            {"id": "mixed-1", "at": [32, 20], "text": "订单 Order  A  12.50%"},
            {"id": "mixed-2", "at": [32, 88], "text": "中文  OCR 日本語  2026-09-18"},
            {"id": "mixed-3", "at": [32, 156], "text": "AA  11  ll  00  ¥1,234.50"},
        ],
    },
    {
        "id": "numbers-symbols",
        "size": [1000, 170],
        "fontSize": 38,
        "background": "#ffffff",
        "foreground": "#101010",
        "tags": ["numbers", "symbols", "punctuation", "high-contrast"],
        "lines": [
            {"id": "symbol-1", "at": [28, 18], "text": "±0.02  √2≈1.414  ∑  ∫  π  Δ"},
            {"id": "symbol-2", "at": [28, 94], "text": "SN: AA11-BB22 / 98.6%  [OK]"},
        ],
    },
    {
        "id": "two-column-order",
        "size": [1000, 220],
        "fontSize": 36,
        "background": "#ffffff",
        "foreground": "#172033",
        "tags": ["zh-en-ja", "columns", "reading-order"],
        "lines": [
            {"id": "column-left-1", "at": [30, 24], "text": "左栏 01  Alpha"},
            {"id": "column-left-2", "at": [30, 118], "text": "左栏 02  Beta"},
            {"id": "column-right-1", "at": [540, 24], "text": "右欄 03  日本語"},
            {"id": "column-right-2", "at": [540, 118], "text": "右欄 04  Gamma"},
        ],
    },
    {
        "id": "formula-routing",
        "size": [800, 140],
        "fontSize": 48,
        "background": "#ffffff",
        "foreground": "#111111",
        "tags": ["formula", "superscript", "symbols"],
        "lines": [
            {
                "id": "formula-1",
                "at": [40, 34],
                "text": "x² + 1/2 = √2",
                "kind": "formula",
                "formula": {"format": "latex", "value": "x^2 + \\frac{1}{2} = \\sqrt{2}"},
            }
        ],
    },
    {
        "id": "low-contrast-small",
        "size": [900, 150],
        "fontSize": 26,
        "background": "#f1f3f5",
        "foreground": "#777b80",
        "tags": ["zh-en", "low-contrast", "small-text", "numbers"],
        "lines": [
            {"id": "low-1", "at": [24, 22], "text": "Low contrast 中文 OCR 8.5%"},
            {"id": "low-2", "at": [24, 82], "text": "Ref 001122  value ±0.08"},
        ],
    },
]


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def render_case(spec):
    width, height = spec["size"]
    image = Image.new("RGB", (width, height), spec["background"])
    draw = ImageDraw.Draw(image)
    font = ImageFont.truetype(str(FONT_PATH), spec["fontSize"])
    lines = []
    for source in spec["lines"]:
        x, y = source["at"]
        draw.text((x, y), source["text"], font=font, fill=spec["foreground"])
        left, top, right, bottom = draw.textbbox((x, y), source["text"], font=font)
        padding = 4
        left, top = max(0, left - padding), max(0, top - padding)
        right, bottom = min(width, right + padding), min(height, bottom + padding)
        line = {
            "id": source["id"],
            "text": source["text"],
            "quad": [[left, top], [right, top], [right, bottom], [left, bottom]],
            "kind": source.get("kind", "text"),
        }
        if "formula" in source:
            line["formula"] = source["formula"]
        lines.append(line)

    IMAGES.mkdir(parents=True, exist_ok=True)
    image_path = IMAGES / f"{spec['id']}.png"
    image.save(image_path, format="PNG", compress_level=9, optimize=False)
    return {
        "id": spec["id"],
        "tags": spec["tags"],
        "source": {
            "kind": "generated",
            "license": LICENSE,
            "imagePath": f"images/{image_path.name}",
            "sha256": sha256(image_path),
            "width": width,
            "height": height,
        },
        "lines": lines,
    }


def main():
    if not FONT_PATH.is_file():
        raise SystemExit(f"missing bundled font: {FONT_PATH}")
    corpus = {
        "schema": "clippy-ocr-quality-corpus-v1",
        "cases": [render_case(spec) for spec in CASES],
    }
    (ROOT / "corpus.json").write_text(
        json.dumps(corpus, ensure_ascii=False, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
