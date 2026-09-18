#!/usr/bin/env python3
"""在固定 OCR 语料上采集 Tesseract 或增强链预测。"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time
from typing import Any

from quality_metrics import (
    PREDICTION_SCHEMA,
    ContractError,
    load_json,
    validate_corpus,
    validate_predictions,
)


MAX_PNG_BYTES = 64 * 1024 * 1024
MAX_OUTPUT_BYTES = 4 * 1024 * 1024
MAX_STDERR_BYTES = 64 * 1024
TIMEOUT_SECONDS = 65


def file_sha256(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def png_dimensions(payload: bytes) -> tuple[int, int]:
    if (
        len(payload) < 33
        or len(payload) > MAX_PNG_BYTES
        or payload[:8] != b"\x89PNG\r\n\x1a\n"
        or payload[12:16] != b"IHDR"
    ):
        raise ContractError("语料图片必须是预算内 PNG")
    return int.from_bytes(payload[16:20], "big"), int.from_bytes(payload[20:24], "big")


def read_case_png(corpus_path: Path, case: dict[str, Any]) -> bytes:
    source = case["source"]
    root = corpus_path.resolve().parent
    image_path = (root / source["imagePath"]).resolve()
    try:
        image_path.relative_to(root)
    except ValueError as error:
        raise ContractError(f"{case['id']} 图片逃逸语料目录") from error
    payload = image_path.read_bytes()
    if file_sha256(payload) != source["sha256"]:
        raise ContractError(f"{case['id']} 图片 SHA-256 不符")
    if png_dimensions(payload) != (source["width"], source["height"]):
        raise ContractError(f"{case['id']} 图片尺寸不符")
    return payload


def command_version(executable: Path) -> str:
    output = subprocess.run(
        [str(executable), "--version"],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=10,
    ).stdout
    first_line = output.decode("utf-8", errors="replace").splitlines()
    return first_line[0].strip() if first_line else executable.name


def collect_tesseract(
    executable: Path, cases: list[tuple[dict[str, Any], bytes]]
) -> dict[str, Any]:
    version = command_version(executable)
    predictions = []
    for case, png in cases:
        started = time.perf_counter()
        process = subprocess.run(
            [str(executable), "stdin", "stdout", "-l", "eng+chi_sim"],
            input=png,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=TIMEOUT_SECONDS,
        )
        duration = (time.perf_counter() - started) * 1000
        if len(process.stdout) > MAX_OUTPUT_BYTES or len(process.stderr) > MAX_STDERR_BYTES:
            raise ContractError(f"{case['id']} Tesseract 输出超出预算")
        if process.returncode != 0:
            raise ContractError(f"{case['id']} Tesseract 失败，exit={process.returncode}")
        try:
            text = process.stdout.decode("utf-8").strip()
        except UnicodeDecodeError as error:
            raise ContractError(f"{case['id']} Tesseract 输出不是 UTF-8") from error
        predictions.append(
            {"id": case["id"], "text": text, "lines": [], "durationMs": duration}
        )
    return {
        "schema": PREDICTION_SCHEMA,
        "engine": {
            "id": "tesseract",
            "version": version,
            "capabilities": {"lineGeometry": False, "structuredFormula": False},
        },
        "cases": predictions,
    }


def _read_manifest(path: Path) -> tuple[dict[str, Any], str]:
    payload = path.read_bytes()
    if not payload or len(payload) > 65_536:
        raise ContractError("增强 OCR manifest 超出预算")
    try:
        manifest = json.loads(payload)
    except json.JSONDecodeError as error:
        raise ContractError("增强 OCR manifest 不是 JSON") from error
    if not isinstance(manifest, dict):
        raise ContractError("增强 OCR manifest 必须是 object")
    return manifest, file_sha256(payload)


def enhanced_prediction(result: Any) -> tuple[str, list[dict[str, Any]]]:
    if not isinstance(result, dict) or not isinstance(result.get("text"), str):
        raise ContractError("增强 OCR 返回无效 result")
    raw_lines = result.get("lines")
    if not isinstance(raw_lines, list):
        raise ContractError("增强 OCR 返回无效 lines")
    try:
        ordered = sorted(raw_lines, key=lambda line: line["readingOrder"])
        lines = [
            {
                "text": line["text"],
                "quad": line["quad"],
                "kind": "text",
            }
            for line in ordered
        ]
    except (KeyError, TypeError) as error:
        raise ContractError("增强 OCR 行合同无效") from error
    return result["text"], lines


def collect_enhanced(
    manifest_path: Path,
    cases: list[tuple[dict[str, Any], bytes]],
    diagnostics_dir: Path | None = None,
) -> dict[str, Any]:
    manifest, manifest_sha = _read_manifest(manifest_path)
    python = Path(str(manifest.get("python", "")))
    script = Path(str(manifest.get("script", "")))
    pipeline_id = manifest.get("pipelineId")
    if (
        not python.is_absolute()
        or not python.is_file()
        or not script.is_absolute()
        or not script.is_file()
        or not isinstance(pipeline_id, str)
        or not pipeline_id
    ):
        raise ContractError("增强 OCR manifest 缺少有效绝对运行路径或 pipelineId")

    predictions = []
    for index, (case, png) in enumerate(cases):
        request_id = f"quality-{index}"
        header = json.dumps(
            {
                "version": 1,
                "requestId": request_id,
                "pngBytes": len(png),
                "deadlineMs": 60_000,
            },
            separators=(",", ":"),
        ).encode("utf-8") + b"\n"
        started = time.perf_counter()
        command = [
            str(python),
            "-I",
            str(script),
            "--manifest",
            str(manifest_path.resolve()),
            "--manifest-sha256",
            manifest_sha,
        ]
        if diagnostics_dir is not None:
            command.extend(
                [
                    "--diagnostics",
                    str(diagnostics_dir / f"{index:03d}-{case['id']}.json"),
                ]
            )
        process = subprocess.run(
            command,
            input=header + png,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=TIMEOUT_SECONDS,
        )
        duration = (time.perf_counter() - started) * 1000
        if len(process.stdout) > MAX_OUTPUT_BYTES or len(process.stderr) > MAX_STDERR_BYTES:
            raise ContractError(f"{case['id']} 增强 OCR 输出超出预算")
        if process.returncode != 0:
            raise ContractError(f"{case['id']} 增强 OCR 失败，exit={process.returncode}")
        try:
            reply = json.loads(process.stdout)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ContractError(f"{case['id']} 增强 OCR 输出不是有效 JSON") from error
        if (
            not isinstance(reply, dict)
            or reply.get("version") != 1
            or reply.get("requestId") != request_id
        ):
            raise ContractError(f"{case['id']} 增强 OCR 请求身份不符")
        text, lines = enhanced_prediction(reply.get("result"))
        predictions.append(
            {
                "id": case["id"],
                "text": text,
                "lines": lines,
                "durationMs": duration,
            }
        )
    return {
        "schema": PREDICTION_SCHEMA,
        "engine": {
            "id": "ppocrv6+edgegnn",
            "version": f"{pipeline_id}@{manifest_sha}",
            "capabilities": {"lineGeometry": True, "structuredFormula": False},
        },
        "cases": predictions,
    }


def write_new(path: Path, value: dict[str, Any]) -> None:
    encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
    try:
        with path.open("x", encoding="utf-8") as handle:
            handle.write(encoded)
    except FileExistsError as error:
        raise ContractError(f"输出已存在，不会覆盖: {path}") from error


def create_diagnostics_directory(path: Path) -> Path:
    path.mkdir(mode=0o700, parents=False, exist_ok=False)
    return path.resolve()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    subparsers = parser.add_subparsers(dest="engine", required=True)
    tesseract = subparsers.add_parser("tesseract")
    tesseract.add_argument("--executable", default="tesseract")
    enhanced = subparsers.add_parser("enhanced")
    enhanced.add_argument("--manifest", required=True, type=Path)
    enhanced.add_argument("--diagnostics-dir", type=Path)
    arguments = parser.parse_args(argv)
    try:
        corpus = validate_corpus(load_json(arguments.corpus))
        cases = [
            (case, read_case_png(arguments.corpus, case)) for case in corpus["cases"]
        ]
        if arguments.engine == "tesseract":
            executable = shutil.which(arguments.executable)
            if executable is None:
                raise ContractError(f"找不到 Tesseract: {arguments.executable}")
            result = collect_tesseract(Path(executable).resolve(), cases)
        else:
            diagnostics_dir = arguments.diagnostics_dir
            if diagnostics_dir is not None:
                diagnostics_dir = create_diagnostics_directory(diagnostics_dir)
            result = collect_enhanced(arguments.manifest.resolve(), cases, diagnostics_dir)
        validated = validate_predictions(result, {case["id"] for case in corpus["cases"]})
        write_new(arguments.output, validated)
    except (
        OSError,
        subprocess.SubprocessError,
        json.JSONDecodeError,
        ContractError,
    ) as error:
        print(f"ocr-quality-collect: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
