#!/usr/bin/env python3
"""固定 PP-DocLayout-S 的公式区域路由探针；权重仅从显式本地目录读取。"""

from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import time
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
import formula_quality
import quality_metrics
from quality_collect import MAX_OUTPUT_BYTES, MAX_STDERR_BYTES, run_measured

SCHEMA = "clippy-ocr-formula-layout-probe-v1"
MODEL_NAME = "PP-DocLayout-S"
MODEL_REVISION = "8ac289e66575bb9bba6e15c53719d8b15cc9b3b2"
MODEL_FILES = {
    "inference.pdiparams": (4_804_904, "491c3382d84ca04d2033afbee0c105942ed82fea392bb4a19170646adebe088a"),
    "inference.json": (339_876, "ac09e931895d4c442e5379ab3b7e9b583baf288e816ab7675ca519da9eb2a9d7"),
    "inference.yml": (1_579, "6f690098c438214c67822239a568f0bc2bbe97a8f02fba93e492d4f2c47523c3"),
    "config.json": (4_715, "45f5757daf01694c4adba0eba7ff2854caf011f02c91bdbd2ef6eb06d7a0f488"),
}
SCORE_THRESHOLD = 0.5
IOU_THRESHOLD = 0.5


def file_digest(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate_model(model_dir: Path) -> Path:
    if not model_dir.is_absolute() or model_dir.is_symlink() or not model_dir.is_dir():
        raise quality_metrics.ContractError("版面模型目录必须是绝对普通目录")
    for name, (size, sha) in MODEL_FILES.items():
        path = model_dir / name
        try:
            stat = path.stat(follow_symlinks=False)
        except OSError as error:
            raise quality_metrics.ContractError(f"版面模型缺少 {name}") from error
        if path.is_symlink() or not path.is_file() or stat.st_size != size or file_digest(path) != sha:
            raise quality_metrics.ContractError(f"版面模型 {name} 身份不符")
    return model_dir.resolve()


def axis_quad(coordinate: list[float]) -> list[list[float]]:
    left, top, right, bottom = coordinate
    return [[left, top], [right, top], [right, bottom], [left, bottom]]


def evaluate(positive: dict[str, Any], negative: dict[str, Any], raw: dict[str, Any]) -> dict[str, Any]:
    expected: dict[str, tuple[bool, list[list[float]] | None]] = {}
    for case in positive["cases"]:
        expected[f"positive/{case['id']}"] = (True, case["lines"][0]["quad"])
    for case in negative["cases"]:
        expected[f"negative/{case['id']}"] = (False, None)
    if not isinstance(raw, dict) or raw.get("schema") != SCHEMA or set(raw) != {"schema", "engine", "cases"}:
        raise quality_metrics.ContractError("版面探针顶层合同无效")
    cases = raw.get("cases")
    if not isinstance(cases, list) or len(cases) != len(expected):
        raise quality_metrics.ContractError("版面探针 case 数量无效")
    seen: set[str] = set()
    reports = []
    positive_detected = false_positive_cases = false_positive_boxes = 0
    durations: list[float] = []
    for index, case in enumerate(cases):
        if not isinstance(case, dict) or set(case) != {"id", "durationMs", "boxes"}:
            raise quality_metrics.ContractError(f"版面探针 cases[{index}] 字段无效")
        identifier = case.get("id")
        if not isinstance(identifier, str) or identifier in seen or identifier not in expected:
            raise quality_metrics.ContractError("版面探针 case id 重复或未知")
        seen.add(identifier)
        duration = case.get("durationMs")
        if isinstance(duration, bool) or not isinstance(duration, (int, float)) or not math.isfinite(duration) or not 0 <= duration <= 600_000:
            raise quality_metrics.ContractError("版面探针耗时无效")
        durations.append(float(duration))
        boxes = case.get("boxes")
        if not isinstance(boxes, list) or len(boxes) > 1000:
            raise quality_metrics.ContractError("版面探针 boxes 无效")
        formula_boxes = []
        for box in boxes:
            if not isinstance(box, dict) or set(box) != {"label", "score", "coordinate"}:
                raise quality_metrics.ContractError("版面探针 box 合同无效")
            score, coordinate = box.get("score"), box.get("coordinate")
            if (
                not isinstance(box.get("label"), str)
                or isinstance(score, bool)
                or not isinstance(score, (int, float))
                or not math.isfinite(score)
                or not 0 <= score <= 1
                or not isinstance(coordinate, list)
                or len(coordinate) != 4
                or any(isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) for value in coordinate)
            ):
                raise quality_metrics.ContractError("版面探针 box 值无效")
            if box["label"] == "formula" and score >= SCORE_THRESHOLD:
                formula_boxes.append(box)
        is_positive, truth = expected[identifier]
        best_iou = max(
            (quality_metrics.quad_iou(truth, axis_quad(box["coordinate"])) for box in formula_boxes),
            default=0.0,
        ) if truth is not None else None
        matched = bool(is_positive and best_iou is not None and best_iou >= IOU_THRESHOLD)
        positive_detected += matched
        if not is_positive and formula_boxes:
            false_positive_cases += 1
            false_positive_boxes += len(formula_boxes)
        reports.append({
            "id": identifier,
            "expectedFormula": is_positive,
            "formulaBoxes": formula_boxes,
            "bestFormulaIou": best_iou,
            "matched": matched,
            "durationMs": duration,
        })
    if seen != set(expected):
        raise quality_metrics.ContractError("版面探针缺少 case")
    positive_count = len(positive["cases"])
    negative_count = len(negative["cases"])
    return {
        "schema": SCHEMA,
        "engine": raw["engine"],
        "cases": reports,
        "summary": {
            "positiveCases": positive_count,
            "positiveDetected": positive_detected,
            "positiveRecall": positive_detected / positive_count,
            "negativeCases": negative_count,
            "falsePositiveCases": false_positive_cases,
            "falsePositiveBoxes": false_positive_boxes,
            "durationMsP50": formula_quality.percentile(durations, 0.5),
            "durationMsP95": formula_quality.percentile(durations, 0.95),
            "scoreThreshold": SCORE_THRESHOLD,
            "iouThreshold": IOU_THRESHOLD,
        },
    }


def worker(model_dir: Path, positive_path: Path, negative_path: Path) -> int:
    positive = formula_quality.validate_formula_corpus(positive_path)
    negative = quality_metrics.validate_corpus(quality_metrics.load_json(negative_path))
    quality_metrics.validate_corpus_assets(negative_path, negative)
    from paddleocr import LayoutDetection

    started = time.perf_counter()
    # Paddle 3.3.1 的 oneDNN runner 无法转换本模型的 Array<Double> PIR 属性。
    model = LayoutDetection(
        model_name=MODEL_NAME,
        model_dir=str(model_dir),
        device="cpu",
        enable_mkldnn=False,
    )
    load_ms = (time.perf_counter() - started) * 1000
    cases = []
    for prefix, corpus_path, corpus in (
        ("positive", positive_path, positive),
        ("negative", negative_path, negative),
    ):
        for case in corpus["cases"]:
            image = (corpus_path.parent / case["source"]["imagePath"]).resolve()
            started = time.perf_counter()
            results = list(model.predict(str(image), batch_size=1, layout_nms=True))
            duration = (time.perf_counter() - started) * 1000
            if len(results) != 1 or not isinstance(results[0].get("boxes"), list):
                raise RuntimeError(f"{prefix}/{case['id']} 版面模型输出无效")
            boxes = [{
                "label": box["label"],
                "score": float(box["score"]),
                "coordinate": [float(value) for value in box["coordinate"]],
            } for box in results[0]["boxes"]]
            cases.append({"id": f"{prefix}/{case['id']}", "durationMs": duration, "boxes": boxes})
    versions = "/".join(
        f"{name} {importlib.metadata.version(name)}"
        for name in ("paddleocr", "paddlepaddle", "paddlex")
    )
    print(json.dumps({
        "schema": SCHEMA,
        "engine": {
            "id": MODEL_NAME,
            "version": MODEL_REVISION,
            "runtime": f"{versions}/Python {platform.python_version()}",
            "modelSha256": MODEL_FILES["inference.pdiparams"][1],
            "modelBytes": MODEL_FILES["inference.pdiparams"][0],
            "modelLoadMs": load_ms,
            "enableMkldnn": False,
            "license": "Apache-2.0",
        },
        "cases": cases,
    }, ensure_ascii=False, separators=(",", ":")))
    return 0


def collect(arguments: argparse.Namespace) -> int:
    model_dir = validate_model(arguments.model_dir)
    positive_path, negative_path = arguments.positive_corpus.resolve(), arguments.negative_corpus.resolve()
    positive = formula_quality.validate_formula_corpus(positive_path)
    negative = quality_metrics.validate_corpus(quality_metrics.load_json(negative_path))
    quality_metrics.validate_corpus_assets(negative_path, negative)
    if not arguments.python.is_absolute() or not arguments.python.is_file():
        raise quality_metrics.ContractError("版面探针隔离 Python 无效")
    with tempfile.TemporaryDirectory(prefix="clippy-formula-layout-") as cache:
        environment = os.environ.copy()
        environment.update({
            "PADDLE_PDX_CACHE_HOME": cache,
            "OMP_NUM_THREADS": "2",
            "OPENBLAS_NUM_THREADS": "2",
            "MKL_NUM_THREADS": "2",
        })
        process, peak_memory = run_measured(
            [
                str(arguments.python.absolute()), "-I", str(Path(__file__).resolve()), "worker",
                "--model-dir", str(model_dir),
                "--positive-corpus", str(positive_path),
                "--negative-corpus", str(negative_path),
            ],
            b"",
            180,
            env=environment,
        )
    if len(process.stdout) > MAX_OUTPUT_BYTES or len(process.stderr) > MAX_STDERR_BYTES:
        raise quality_metrics.ContractError("版面探针输出超出预算")
    if process.returncode != 0:
        message = process.stderr.decode("utf-8", errors="replace")[-4000:]
        raise quality_metrics.ContractError(f"版面探针失败，exit={process.returncode}: {message}")
    try:
        raw = json.loads(process.stdout)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise quality_metrics.ContractError("版面探针输出不是有效 JSON") from error
    raw["engine"]["peakMemoryBytes"] = peak_memory
    formula_quality.write_new(arguments.predictions_output, raw)
    formula_quality.write_new(arguments.report_output, evaluate(positive, negative, raw))
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="mode", required=True)
    collect_parser = subparsers.add_parser("collect")
    collect_parser.add_argument("--python", required=True, type=Path)
    collect_parser.add_argument("--model-dir", required=True, type=Path)
    collect_parser.add_argument("--positive-corpus", required=True, type=Path)
    collect_parser.add_argument("--negative-corpus", required=True, type=Path)
    collect_parser.add_argument("--predictions-output", required=True, type=Path)
    collect_parser.add_argument("--report-output", required=True, type=Path)
    worker_parser = subparsers.add_parser("worker")
    worker_parser.add_argument("--model-dir", required=True, type=Path)
    worker_parser.add_argument("--positive-corpus", required=True, type=Path)
    worker_parser.add_argument("--negative-corpus", required=True, type=Path)
    arguments = parser.parse_args(argv)
    try:
        if arguments.mode == "worker":
            return worker(arguments.model_dir, arguments.positive_corpus, arguments.negative_corpus)
        return collect(arguments)
    except (OSError, subprocess.SubprocessError, quality_metrics.ContractError) as error:
        print(f"ocr-formula-layout-probe: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
