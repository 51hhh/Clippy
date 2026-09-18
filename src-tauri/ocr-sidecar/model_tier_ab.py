#!/usr/bin/env python3
"""用固定官方 small/medium det/rec 组合采集研究 A/B；不生成产品 manifest。"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import platform
from pathlib import Path
import tempfile

import cv2
import numpy
import onnxruntime

from quality_collect import collect_enhanced, read_case_png, write_new
from quality_metrics import evaluate, load_json, validate_corpus, validate_predictions


MEDIUM = {
    "det": {
        "sha256": "eb13b44b25bb36f89528b68720af8a61d9cf381176107f465db1757b65d086e1",
        "bytes": 62_032_837,
        "revision": "61323801669c338b7891481ec7bac61ce31b576a",
    },
    "rec": {
        "sha256": "9c09abf0957f7968c7586464b7397b84ad2387a0497a351af40e9acc71b673ba",
        "bytes": 76_554_979,
        "revision": "50c7eacafc52fa7bcf4194e8cd08e46f8558504b",
    },
}
PROFILES = {
    "small-small": (False, False),
    "medium-det": (True, False),
    "medium-rec": (False, True),
    "medium-both": (True, True),
}


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def verify_model(path: Path, role: str) -> Path:
    resolved = path.resolve()
    expected = MEDIUM[role]
    if not resolved.is_file() or resolved.stat().st_size != expected["bytes"] or digest(resolved) != expected["sha256"]:
        raise ValueError(f"medium {role} 文件身份不符")
    return resolved


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-manifest", required=True, type=Path)
    parser.add_argument("--medium-det", required=True, type=Path)
    parser.add_argument("--medium-rec", required=True, type=Path)
    parser.add_argument("--corpus", required=True, action="append", type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    arguments = parser.parse_args()

    medium_det = verify_model(arguments.medium_det, "det")
    medium_rec = verify_model(arguments.medium_rec, "rec")
    base_bytes = arguments.base_manifest.read_bytes()
    base = json.loads(base_bytes)
    if set(base.get("models", {})) != {"det", "rec", "dictionary", "edge"}:
        raise ValueError("A/B base manifest 必须只含 small det/rec、字典和 Edge")
    output = arguments.output_dir
    output.mkdir(mode=0o700, parents=False, exist_ok=False)

    environment = {
        "schema": "clippy-ocr-model-tier-ab-v1",
        "python": platform.python_version(),
        "numpy": numpy.__version__,
        "opencv": cv2.__version__,
        "onnxruntime": onnxruntime.__version__,
        "baseManifestSha256": hashlib.sha256(base_bytes).hexdigest(),
        "medium": {
            role: {**identity, "pathRecorded": False}
            for role, identity in MEDIUM.items()
        },
        "profiles": list(PROFILES),
        "corpora": [path.resolve().parent.name for path in arguments.corpus],
    }
    write_new(output / "environment.json", environment)

    with tempfile.TemporaryDirectory(prefix="clippy-ocr-model-tier-") as temporary:
        temporary_root = Path(temporary)
        for corpus_path in arguments.corpus:
            corpus = validate_corpus(load_json(corpus_path))
            cases = [(case, read_case_png(corpus_path, case)) for case in corpus["cases"]]
            corpus_name = corpus_path.resolve().parent.name
            for profile, (use_medium_det, use_medium_rec) in PROFILES.items():
                manifest = copy.deepcopy(base)
                manifest["pipelineId"] = f"ppocrv6-edgegnn-ab-{profile}"
                manifest["researchModelProfile"] = profile
                if use_medium_det:
                    manifest["models"]["det"] = {"path": str(medium_det), "sha256": MEDIUM["det"]["sha256"]}
                if use_medium_rec:
                    manifest["models"]["rec"] = {"path": str(medium_rec), "sha256": MEDIUM["rec"]["sha256"]}
                manifest_path = temporary_root / f"{corpus_name}-{profile}.json"
                manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
                predictions = collect_enhanced(manifest_path, cases)
                predictions = validate_predictions(predictions, {case["id"] for case in corpus["cases"]})
                write_new(output / f"{corpus_name}-{profile}-predictions.json", predictions)
                write_new(output / f"{corpus_name}-{profile}-report.json", evaluate(corpus, predictions))


if __name__ == "__main__":
    main()
