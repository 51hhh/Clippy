#!/usr/bin/env python3
"""在固定 OCR 语料上采集 Tesseract 或增强链预测。"""

from __future__ import annotations

import argparse
import ctypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import threading
import time
from typing import Any

from quality_metrics import (
    PREDICTION_SCHEMA,
    ContractError,
    file_sha256,
    load_json,
    read_case_png,
    validate_corpus,
    validate_predictions,
)


MAX_OUTPUT_BYTES = 4 * 1024 * 1024
MAX_STDERR_BYTES = 64 * 1024
TIMEOUT_SECONDS = 65
RSS_SAMPLE_INTERVAL_SECONDS = 0.01


def _linux_peak_rss_bytes(pid: int) -> int | None:
    try:
        lines = Path(f"/proc/{pid}/status").read_text(encoding="ascii").splitlines()
    except (OSError, UnicodeError):
        return None
    values: dict[str, int] = {}
    for line in lines:
        name, separator, raw = line.partition(":")
        if separator and name in {"VmHWM", "VmRSS"}:
            fields = raw.split()
            if fields and fields[0].isdigit():
                values[name] = int(fields[0]) * 1024
    return values.get("VmHWM", values.get("VmRSS"))


def _macos_rss_bytes(pid: int) -> int | None:
    try:
        result = subprocess.run(
            ["ps", "-o", "rss=", "-p", str(pid)],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            timeout=0.5,
        )
        value = result.stdout.decode("ascii", errors="strict").strip()
        return int(value) * 1024 if result.returncode == 0 and value.isdigit() else None
    except (OSError, UnicodeError, subprocess.SubprocessError):
        return None


def _windows_peak_rss_bytes(pid: int) -> int | None:
    # PROCESS_MEMORY_COUNTERS 的 PeakWorkingSetSize 是进程自启动以来的峰值驻留集。
    class ProcessMemoryCounters(ctypes.Structure):
        _fields_ = [
            ("cb", ctypes.c_ulong),
            ("page_fault_count", ctypes.c_ulong),
            ("peak_working_set_size", ctypes.c_size_t),
            ("working_set_size", ctypes.c_size_t),
            ("quota_peak_paged_pool_usage", ctypes.c_size_t),
            ("quota_paged_pool_usage", ctypes.c_size_t),
            ("quota_peak_non_paged_pool_usage", ctypes.c_size_t),
            ("quota_non_paged_pool_usage", ctypes.c_size_t),
            ("pagefile_usage", ctypes.c_size_t),
            ("peak_pagefile_usage", ctypes.c_size_t),
        ]

    try:
        kernel32 = ctypes.windll.kernel32
        psapi = ctypes.windll.psapi
        kernel32.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
        kernel32.OpenProcess.restype = ctypes.c_void_p
        kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
        kernel32.CloseHandle.restype = ctypes.c_int
        psapi.GetProcessMemoryInfo.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ProcessMemoryCounters),
            ctypes.c_ulong,
        ]
        psapi.GetProcessMemoryInfo.restype = ctypes.c_int
        handle = kernel32.OpenProcess(0x0400 | 0x0010, False, pid)
        if not handle:
            return None
        try:
            counters = ProcessMemoryCounters()
            counters.cb = ctypes.sizeof(counters)
            if not psapi.GetProcessMemoryInfo(
                handle, ctypes.byref(counters), ctypes.sizeof(counters)
            ):
                return None
            return int(counters.peak_working_set_size)
        finally:
            kernel32.CloseHandle(handle)
    except (AttributeError, OSError, ValueError):
        return None


def process_peak_rss_bytes(pid: int) -> int | None:
    if sys.platform.startswith("linux"):
        return _linux_peak_rss_bytes(pid)
    if sys.platform == "darwin":
        return _macos_rss_bytes(pid)
    if os.name == "nt":
        return _windows_peak_rss_bytes(pid)
    return None


def peak_memory_metric() -> str | None:
    if sys.platform.startswith("linux"):
        return "linux-proc-vmhwm"
    if sys.platform == "darwin":
        return "macos-ps-rss-sampled"
    if os.name == "nt":
        return "windows-peak-working-set"
    return None


def run_measured(
    command: list[str], payload: bytes, timeout: float, env: dict[str, str] | None = None
) -> tuple[subprocess.CompletedProcess[bytes], int | None]:
    """运行一次独立 OCR 进程，并采样该进程自己的峰值 RSS。"""
    process = subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
    )
    stopped = threading.Event()
    peak_memory: list[int] = []

    def sample() -> None:
        while not stopped.is_set():
            value = process_peak_rss_bytes(process.pid)
            if value is not None:
                peak_memory.append(value)
            stopped.wait(RSS_SAMPLE_INTERVAL_SECONDS)

    monitor = threading.Thread(target=sample, name="ocr-quality-rss", daemon=True)
    monitor.start()
    try:
        stdout, stderr = process.communicate(input=payload, timeout=timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        process.communicate()
        raise
    finally:
        stopped.set()
        monitor.join(timeout=1)
    return (
        subprocess.CompletedProcess(command, process.returncode, stdout, stderr),
        max(peak_memory) if peak_memory else None,
    )


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
        process, peak_memory = run_measured(
            [str(executable), "stdin", "stdout", "-l", "eng+chi_sim"],
            png,
            TIMEOUT_SECONDS,
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
        prediction = {
            "id": case["id"],
            "text": text,
            "lines": [],
            "durationMs": duration,
        }
        if peak_memory is not None:
            prediction["peakMemoryBytes"] = peak_memory
        predictions.append(prediction)
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
        process, peak_memory = run_measured(
            command,
            header + png,
            TIMEOUT_SECONDS,
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
        prediction = {
            "id": case["id"],
            "text": text,
            "lines": lines,
            "durationMs": duration,
        }
        if peak_memory is not None:
            prediction["peakMemoryBytes"] = peak_memory
        predictions.append(prediction)
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
