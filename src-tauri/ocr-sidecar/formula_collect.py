#!/usr/bin/env python3
"""验证官方模型身份并采集固定公式 crop 的真实预测与资源数据。"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import formula_quality
from quality_collect import MAX_OUTPUT_BYTES, MAX_STDERR_BYTES, run_measured
import quality_metrics

MODEL_PROFILES = {
    "s": {
        "inference.pdiparams": (231_675_001, "b6392296d16e2a9f414c0a751d7ccbd1bd9d8272b68aab72df1d3875f35a7489"),
        "inference.json": (504_505, "2c8fc31a94d461a7325fe06e8cbaa5db6c5658c490f74d3d6116defe8c859d6f"),
        "inference.yml": (2_244_559, "08b21c2d255041008266b8fb947758e9731eafff191568af21da5a4b9c9dd5a0"),
        "config.json": (3_951_671, "ea32742b976ba34711042cac4e46206f114067949448c2bd8dec60a44d3de1fb"),
    },
    "plus-s": {
        "inference.pdiparams": (256_845_006, "e464f94412feaa98f8791eacc84684f887b3569e30e80c52b8112e9cf7d4069b"),
        "inference.json": (506_956, "01238434e33df83588e2627f350559b576e34551d2b2ffea148345032de56c00"),
        "inference.yml": (2_244_564, "96062655d94c21d39274328dbc82c1a487e66addb8425f5a7fd5b7dfb2421ec3"),
        "config.json": (3_951_676, "ddf1f951ceeb10c2b9de3ae255b62de59299b1028f87878177e86c9604fe3610"),
    },
}
TIMEOUT_SECONDS = 180


def file_digest(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate_model(model_dir: Path, profile: str) -> Path:
    if not model_dir.is_absolute() or model_dir.is_symlink() or not model_dir.is_dir():
        raise quality_metrics.ContractError("公式模型目录必须是绝对普通目录")
    for name, (expected_size, expected_sha) in MODEL_PROFILES[profile].items():
        path = model_dir / name
        try:
            stat = path.stat(follow_symlinks=False)
        except OSError as error:
            raise quality_metrics.ContractError(f"公式模型缺少 {name}") from error
        if path.is_symlink() or not path.is_file() or stat.st_size != expected_size or file_digest(path) != expected_sha:
            raise quality_metrics.ContractError(f"公式模型 {name} 身份不符")
    return model_dir.resolve()


def collect(python: Path, model_dir: Path, corpus_path: Path, profile: str) -> dict[str, object]:
    corpus = formula_quality.validate_formula_corpus(corpus_path)
    # Python venv 的入口通常是指向基础解释器的符号链接；保留入口路径才能加载该 venv 的 site-packages。
    if not python.is_absolute() or not python.is_file():
        raise quality_metrics.ContractError("公式隔离 Python 无效")
    runner = Path(__file__).resolve().with_name("formula_runner.py")
    with tempfile.TemporaryDirectory(prefix="clippy-formula-paddlex-") as cache:
        environment = os.environ.copy()
        environment.update({
            "PADDLE_PDX_CACHE_HOME": cache,
            "OMP_NUM_THREADS": "2",
            "OPENBLAS_NUM_THREADS": "2",
            "MKL_NUM_THREADS": "2",
        })
        process, peak_memory = run_measured(
            [
                str(python), "-I", str(runner),
                "--model-dir", str(model_dir),
                "--corpus", str(corpus_path),
                "--profile", profile,
            ],
            b"",
            TIMEOUT_SECONDS,
            env=environment,
        )
    if len(process.stdout) > MAX_OUTPUT_BYTES or len(process.stderr) > MAX_STDERR_BYTES:
        raise quality_metrics.ContractError("公式模型输出超出预算")
    if process.returncode != 0:
        message = process.stderr.decode("utf-8", errors="replace")[-4000:]
        raise quality_metrics.ContractError(f"公式模型失败，exit={process.returncode}: {message}")
    try:
        value = json.loads(process.stdout)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise quality_metrics.ContractError("公式模型输出不是有效 JSON") from error
    if peak_memory is not None:
        for case in value.get("cases", []):
            case["peakMemoryBytes"] = peak_memory
    return formula_quality.validate_predictions(value, {case["id"] for case in corpus["cases"]})


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--python", required=True, type=Path)
    parser.add_argument("--model-dir", required=True, type=Path)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--profile", required=True, choices=MODEL_PROFILES)
    arguments = parser.parse_args(argv)
    try:
        model_dir = validate_model(arguments.model_dir, arguments.profile)
        result = collect(arguments.python.absolute(), model_dir, arguments.corpus.resolve(), arguments.profile)
        formula_quality.write_new(arguments.output, result)
    except (OSError, subprocess.SubprocessError, quality_metrics.ContractError) as error:
        print(f"ocr-formula-collect: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
