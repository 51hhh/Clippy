#!/usr/bin/env python3
"""隔离运行固定 PP-FormulaNet-S；仅由 formula_collect.py 调用。"""

from __future__ import annotations

import argparse
import importlib.metadata
import json
from pathlib import Path
import platform
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import formula_quality

MODEL_PROFILES = {
    "s": {
        "name": "PP-FormulaNet-S",
        "revision": "0572450e501be9eb1b1cdb7e00fccf4b22fab4df",
        "sha256": "b6392296d16e2a9f414c0a751d7ccbd1bd9d8272b68aab72df1d3875f35a7489",
        "bytes": 231_675_001,
    },
    "plus-s": {
        "name": "PP-FormulaNet_plus-S",
        "revision": "3d46f557e3a1752f4bf81202395af3b5ecfadfd2",
        "sha256": "e464f94412feaa98f8791eacc84684f887b3569e30e80c52b8112e9cf7d4069b",
        "bytes": 256_845_006,
    },
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model-dir", required=True, type=Path)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--profile", required=True, choices=MODEL_PROFILES)
    arguments = parser.parse_args()
    profile = MODEL_PROFILES[arguments.profile]

    corpus = formula_quality.validate_formula_corpus(arguments.corpus)
    from paddleocr import FormulaRecognition
    from latex2mathml.converter import convert as latex_to_mathml

    started = time.perf_counter()
    model = FormulaRecognition(
        model_name=profile["name"],
        model_dir=str(arguments.model_dir),
        device="cpu",
    )
    model_load_ms = (time.perf_counter() - started) * 1000
    cases = []
    for case in corpus["cases"]:
        image = (arguments.corpus.parent / case["source"]["imagePath"]).resolve()
        started = time.perf_counter()
        results = list(model.predict(str(image), batch_size=1))
        duration_ms = (time.perf_counter() - started) * 1000
        if len(results) != 1 or not isinstance(results[0].get("rec_formula"), str):
            raise RuntimeError(f"{case['id']} 公式模型输出合同无效")
        latex = results[0]["rec_formula"]
        try:
            latex_to_mathml(latex)
            parseable = True
        except Exception:  # 第三方解析器异常类型较多；合同只记录稳定 boolean。
            parseable = False
        cases.append({
            "id": case["id"],
            "latex": latex,
            "latexParseable": parseable,
            "durationMs": duration_ms,
        })
    versions = "/".join(
        f"{name} {importlib.metadata.version(name)}"
        for name in ("paddleocr", "paddlepaddle", "paddlex")
    )
    output = {
        "schema": formula_quality.PREDICTION_SCHEMA,
        "engine": {
            "id": profile["name"],
            "version": profile["revision"],
            "runtime": f"{versions}/Python {platform.python_version()}",
            "modelSha256": profile["sha256"],
            "modelBytes": profile["bytes"],
            "modelLoadMs": model_load_ms,
            "license": "Apache-2.0",
        },
        "cases": cases,
    }
    print(json.dumps(output, ensure_ascii=False, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
