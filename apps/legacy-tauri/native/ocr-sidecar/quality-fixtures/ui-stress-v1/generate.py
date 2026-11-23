#!/usr/bin/env python3
"""生成小字号、暗色、压缩和表格式 UI OCR 压力语料。"""

from __future__ import annotations

import hashlib
import io
import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parent
IMAGES = ROOT / "images"
FONT_PATH = ROOT.parents[2] / "assets" / "fonts" / "NotoSansCJKsc-Medium.otf"
LICENSE = "Generated Clippy fixture text CC0-1.0; Noto Sans CJK SC font OFL-1.1"

CASES = [
    {
        "id": "dark-settings",
        "size": [900, 250],
        "scale": 1.25,
        "background": "#15171c",
        "foreground": "#edf0f7",
        "fontSize": 22,
        "tags": ["ui", "dark", "zh-en-ja", "fractional-scale"],
        "lines": [
            [28, 24, "设置 · OCR 模型"],
            [28, 92, "中文  English  日本語"],
            [28, 160, "版本 v0.1.20  ·  状态 Ready"],
        ],
    },
    {
        "id": "compressed-status",
        "size": [900, 210],
        "scale": 2,
        "background": "#f4f5f7",
        "foreground": "#596273",
        "fontSize": 18,
        "jpegQuality": 28,
        "tags": ["ui", "small-text", "jpeg-artifacts", "numbers", "currency"],
        "lines": [
            [24, 20, "CPU 23.7%  RAM 1,024 MB"],
            [24, 82, "SN: 00Il1lO0  ¥1,280.50"],
            [24, 144, "2026-09-18  08:05:09 UTC+8"],
        ],
    },
    {
        "id": "source-code",
        "size": [1000, 220],
        "scale": 1.25,
        "background": "#0f172a",
        "foreground": "#dbeafe",
        "fontSize": 20,
        "tags": ["ui", "code", "punctuation", "identifiers", "zh-ja"],
        "lines": [
            [24, 18, "const total = price * 0.85;"],
            [24, 82, "if (id === \"AA11-BB22\") return null;"],
            [24, 146, "路径 /tmp/demo  状態 OK"],
        ],
    },
    {
        "id": "table-values",
        "size": [1000, 260],
        "scale": 1.5,
        "background": "#ffffff",
        "foreground": "#172033",
        "fontSize": 20,
        "tags": ["ui", "table", "spaces", "numbers", "currency", "zh-en-ja"],
        "lines": [
            [28, 22, "商品    数量    单价       小计"],
            [28, 88, "Coffee  12      ¥19.90     ¥238.80"],
            [28, 154, "日本茶  3       ¥88.00     ¥264.00"],
        ],
        "tableCells": [
            ["商品", "数量", "单价", "小计"],
            ["Coffee", "12", "¥19.90", "¥238.80"],
            ["日本茶", "3", "¥88.00", "¥264.00"],
        ],
    },
]


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def render(spec):
    logical_width, logical_height = spec["size"]
    scale = spec["scale"]
    width, height = round(logical_width * scale), round(logical_height * scale)
    image = Image.new("RGB", (width, height), spec["background"])
    draw = ImageDraw.Draw(image)
    font = ImageFont.truetype(str(FONT_PATH), round(spec["fontSize"] * scale))
    lines = []
    for index, (x, y, text) in enumerate(spec["lines"]):
        at = (round(x * scale), round(y * scale))
        draw.text(at, text, font=font, fill=spec["foreground"])
        left, top, right, bottom = draw.textbbox(at, text, font=font)
        padding = round(3 * scale)
        lines.append({
            "id": f"{spec['id']}-{index + 1}",
            "kind": "text",
            "text": text,
            "quad": [
                [(left - padding) / scale, (top - padding) / scale],
                [(right + padding) / scale, (top - padding) / scale],
                [(right + padding) / scale, (bottom + padding) / scale],
                [(left - padding) / scale, (bottom + padding) / scale],
            ],
        })
    if scale != 1:
        image = image.resize((logical_width, logical_height), Image.Resampling.LANCZOS)
    if "jpegQuality" in spec:
        encoded = io.BytesIO()
        image.save(encoded, format="JPEG", quality=spec["jpegQuality"], subsampling=2)
        image = Image.open(io.BytesIO(encoded.getvalue())).convert("RGB")
    IMAGES.mkdir(parents=True, exist_ok=True)
    path = IMAGES / f"{spec['id']}.png"
    image.save(path, format="PNG", compress_level=9, optimize=False)
    return {
        "id": spec["id"],
        "tags": spec["tags"],
        "source": {
            "kind": "generated",
            "license": LICENSE,
            "imagePath": f"images/{path.name}",
            "sha256": sha256(path),
            "width": logical_width,
            "height": logical_height,
        },
        "lines": lines,
    }


def table_case(spec, rendered):
    logical_width, logical_height = spec["size"]
    scale = spec["scale"]
    image = Image.new("RGB", (round(logical_width * scale), round(logical_height * scale)))
    draw = ImageDraw.Draw(image)
    font = ImageFont.truetype(str(FONT_PATH), round(spec["fontSize"] * scale))
    rows = []
    for row_index, ((x, y, line_text), cell_texts) in enumerate(
        zip(spec["lines"], spec["tableCells"], strict=True)
    ):
        cursor = 0
        cells = []
        for column_index, text in enumerate(cell_texts):
            start = line_text.find(text, cursor)
            if start < 0:
                raise ValueError(f"{text!r} 不在表格行 {line_text!r} 中")
            at = (
                round(x * scale) + draw.textlength(line_text[:start], font=font),
                round(y * scale),
            )
            left, top, right, bottom = draw.textbbox(at, text, font=font)
            padding = round(2 * scale)
            cells.append(
                {
                    "id": f"table-values-r{row_index + 1}c{column_index + 1}",
                    "text": text,
                    "columnIndex": column_index,
                    "quad": [
                        [(left - padding) / scale, (top - padding) / scale],
                        [(right + padding) / scale, (top - padding) / scale],
                        [(right + padding) / scale, (bottom + padding) / scale],
                        [(left - padding) / scale, (bottom + padding) / scale],
                    ],
                }
            )
            cursor = start + len(text)
        rows.append({"id": f"table-values-r{row_index + 1}", "cells": cells})
    return {
        "id": rendered["id"],
        "tags": rendered["tags"],
        "source": rendered["source"],
        "tables": [{"id": "table-values-main", "rows": rows}],
    }


def main():
    if not FONT_PATH.is_file():
        raise SystemExit(f"missing bundled font: {FONT_PATH}")
    rendered = [render(spec) for spec in CASES]
    corpus = {"schema": "clippy-ocr-quality-corpus-v1", "cases": rendered}
    (ROOT / "corpus.json").write_text(
        json.dumps(corpus, ensure_ascii=False, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )
    table_spec = next(spec for spec in CASES if spec["id"] == "table-values")
    table_rendered = next(case for case in rendered if case["id"] == "table-values")
    table_corpus = {
        "schema": "clippy-ocr-table-corpus-v1",
        "cases": [table_case(table_spec, table_rendered)],
    }
    (ROOT / "table-corpus.json").write_text(
        json.dumps(table_corpus, ensure_ascii=False, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
