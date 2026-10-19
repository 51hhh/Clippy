#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import json
import math
import statistics
import struct
from pathlib import Path


ROOT = Path(__file__).resolve().parent
CORPUS = ROOT / "corpus"
EVIDENCE = ROOT / "evidence"
METHODS = {"telea", "navier_stokes", "lama_roi"}
LABELS = {"A", "B", "C"}
MODEL_SHA256 = "7df918ac3921d3daf0aae1d219776cf0dc4e4935f035af81841b40adcf74fdf2"
EXPECTED_THRESHOLDS = {
    "licenseMustExplicitlyCoverCandidateFiles": True,
    "modelBytesMax": 104857600,
    "runtimePackageBudgetBytesMax": 167772160,
    "sessionLoadMsMax": 1500,
    "inferenceP95MsMax": 1000,
    "singleRunPeakRssBytesMax": 536870912,
    "blindBestOrTiedCasesMin": 4,
    "blindUnacceptableCasesMax": 0,
    "outsideMaskMustRemainExact": True,
}


def fail(message: str) -> None:
    raise SystemExit(f"smart-erase evidence invalid: {message}")


def load_json(path: Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read {path.relative_to(ROOT)}: {error}")
    if not isinstance(value, dict):
        fail(f"{path.relative_to(ROOT)} must contain an object")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def owned_path(relative: str) -> Path:
    if not isinstance(relative, str) or not relative or "\\" in relative:
        fail(f"invalid relative path: {relative!r}")
    candidate = ROOT / relative
    try:
        candidate.relative_to(ROOT)
        resolved = candidate.resolve(strict=True)
        resolved.relative_to(ROOT.resolve())
    except (OSError, ValueError):
        fail(f"path escapes evidence root or is missing: {relative}")
    if candidate.is_symlink() or not candidate.is_file():
        fail(f"evidence path must be a regular non-symlink file: {relative}")
    return candidate


def png_size(path: Path) -> tuple[int, int]:
    with path.open("rb") as handle:
        if handle.read(8) != b"\x89PNG\r\n\x1a\n":
            fail(f"not a PNG: {path.relative_to(ROOT)}")
        if handle.read(4) != b"\x00\x00\x00\r" or handle.read(4) != b"IHDR":
            fail(f"PNG has no canonical IHDR: {path.relative_to(ROOT)}")
        return struct.unpack(">II", handle.read(8))


def close_enough(left: float, right: float) -> bool:
    return math.isclose(float(left), float(right), rel_tol=0, abs_tol=0.001)


def percentile(values: list[float], q: float) -> float:
    ordered = sorted(values)
    position = (len(ordered) - 1) * q
    lower = math.floor(position)
    upper = math.ceil(position)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def main() -> None:
    manifest_path = CORPUS / "manifest.json"
    manifest = load_json(manifest_path)
    if manifest.get("schemaVersion") != 1 or manifest.get("id") != "PX-SMART-01-corpus-v1":
        fail("unexpected corpus schema or id")
    cases = manifest.get("cases")
    if not isinstance(cases, list) or len(cases) != 5:
        fail("corpus must contain five cases")
    case_by_id: dict[str, dict] = {}
    for case in cases:
        case_id = case.get("id")
        if not isinstance(case_id, str) or case_id in case_by_id:
            fail("corpus case ids must be unique strings")
        files = case.get("files")
        hashes = case.get("sha256")
        if set(files or {}) != {"reference", "input", "mask"} or set(hashes or {}) != set(files):
            fail(f"{case_id} must declare reference/input/mask and their hashes")
        for name, relative in files.items():
            path = owned_path(relative)
            if sha256(path) != hashes[name]:
                fail(f"corpus hash mismatch: {relative}")
            if png_size(path) != (case.get("width"), case.get("height")):
                fail(f"corpus dimensions mismatch: {relative}")
        case_by_id[case_id] = case

    benchmark = load_json(EVIDENCE / "benchmark.json")
    if benchmark.get("schemaVersion") != 1 or benchmark.get("corpusId") != manifest["id"]:
        fail("benchmark is bound to the wrong corpus")
    if benchmark.get("corpusManifestSha256") != sha256(manifest_path):
        fail("benchmark corpus manifest hash mismatch")
    candidate = benchmark.get("candidate", {})
    if candidate.get("modelSha256") != MODEL_SHA256 or candidate.get("modelBytes") != 92591623:
        fail("benchmark candidate model identity changed")
    if candidate.get("name") != "OpenCV Zoo quantized LaMa" or candidate.get("license") != "Apache-2.0":
        fail("benchmark candidate name or license changed")
    results = benchmark.get("results")
    if not isinstance(results, list) or len(results) != len(cases) * len(METHODS):
        fail("benchmark result matrix is incomplete")
    result_by_key: dict[tuple[str, str], dict] = {}
    for result in results:
        key = (result.get("caseId"), result.get("method"))
        if key[0] not in case_by_id or key[1] not in METHODS or key in result_by_key:
            fail(f"invalid or duplicate benchmark result: {key}")
        path = owned_path(result.get("output"))
        if sha256(path) != result.get("sha256"):
            fail(f"benchmark output hash mismatch: {path.relative_to(ROOT)}")
        case = case_by_id[key[0]]
        if png_size(path) != (case["width"], case["height"]):
            fail(f"benchmark output dimensions mismatch: {path.relative_to(ROOT)}")
        timings = result.get("timingMs", {}).get("samples")
        if not isinstance(timings, list) or len(timings) != benchmark.get("repeats") or any(value <= 0 for value in timings):
            fail(f"invalid timing samples: {key}")
        if not close_enough(result["timingMs"].get("p50"), statistics.median(timings)):
            fail(f"timing p50 does not derive from samples: {key}")
        if not close_enough(result["timingMs"].get("p95"), percentile(timings, 0.95)):
            fail(f"timing p95 does not derive from samples: {key}")
        if result.get("metrics", {}).get("outsideExact") is not True:
            fail(f"method changed pixels outside the mask: {key}")
        result_by_key[key] = result
    if set(result_by_key) != {(case_id, method) for case_id in case_by_id for method in METHODS}:
        fail("benchmark result matrix does not cover every case and method")

    probe = load_json(EVIDENCE / "resource-probe.json")
    if probe.get("modelSha256") != MODEL_SHA256 or probe.get("caseId") not in case_by_id:
        fail("resource probe identity mismatch")
    for field in ("sessionLoadMs", "inferenceMs", "processPeakRssBytes"):
        if not isinstance(probe.get(field), (int, float)) or probe[field] <= 0:
            fail(f"resource probe has invalid {field}")

    key = load_json(EVIDENCE / "blind-key.json")
    review = load_json(EVIDENCE / "blind-review.json")
    if set(key) != set(case_by_id) or review.get("corpusId") != manifest["id"]:
        fail("blind evidence case set mismatch")
    review_cases = review.get("cases")
    if not isinstance(review_cases, list) or len(review_cases) != len(cases):
        fail("blind review must cover every case")
    review_by_id = {}
    for case_id, mapping in key.items():
        if set(mapping) != LABELS or set(mapping.values()) != METHODS:
            fail(f"blind key is not a method permutation: {case_id}")
        for label in LABELS:
            anonymous = owned_path(f"evidence/blind-assets/{case_id}--{label}.png")
            source = result_by_key[(case_id, mapping[label])]
            if sha256(anonymous) != source["sha256"]:
                fail(f"anonymous output does not match its reveal key: {case_id}/{label}")
    for item in review_cases:
        case_id = item.get("caseId")
        ranking = item.get("ranking")
        unacceptable = item.get("unacceptable")
        if case_id not in key or case_id in review_by_id:
            fail(f"invalid or duplicate blind review case: {case_id}")
        if not isinstance(ranking, list) or set(ranking) != LABELS or len(ranking) != 3:
            fail(f"blind ranking must be a label permutation: {case_id}")
        if not isinstance(unacceptable, list) or not set(unacceptable).issubset(LABELS):
            fail(f"blind unacceptable labels are invalid: {case_id}")
        review_by_id[case_id] = item

    winner_counts = {method: 0 for method in METHODS}
    candidate_unacceptable = []
    for case_id, item in review_by_id.items():
        winner_counts[key[case_id][item["ranking"][0]]] += 1
        if any(key[case_id][label] == "lama_roi" for label in item["unacceptable"]):
            candidate_unacceptable.append(case_id)
    summary = review.get("revealedSummary", {})
    if summary.get("winnerCounts") != winner_counts or summary.get("candidateUnacceptableCases") != candidate_unacceptable:
        fail("blind revealed summary does not derive from the key and ratings")

    decision = load_json(EVIDENCE / "decision.json")
    thresholds = decision.get("thresholds", {})
    if thresholds != EXPECTED_THRESHOLDS:
        fail("decision thresholds changed without a new research id")
    observed = decision.get("observed", {})
    expected_p95 = max(result["timingMs"]["p95"] for result in results if result["method"] == "lama_roi")
    expected_outside = sum(result["metrics"]["outsideExact"] for result in results if result["method"] == "lama_roi")
    expected_stack = observed.get("modelBytes", 0) + sum(observed.get("runtimePackageBytes", {}).values())
    comparisons = {
        "license": observed.get("licenseExplicit") is thresholds.get("licenseMustExplicitlyCoverCandidateFiles"),
        "modelSize": observed.get("modelBytes", 0) <= thresholds.get("modelBytesMax", -1),
        "runtimePackageBudget": expected_stack <= thresholds.get("runtimePackageBudgetBytesMax", -1),
        "sessionLoad": observed.get("sessionLoadMs", math.inf) <= thresholds.get("sessionLoadMsMax", -1),
        "interactiveLatency": observed.get("inferenceP95MsWorstCase", math.inf) <= thresholds.get("inferenceP95MsMax", -1),
        "peakMemory": observed.get("singleRunPeakRssBytes", math.inf) <= thresholds.get("singleRunPeakRssBytesMax", -1),
        "blindPreferenceCount": observed.get("blindBestOrTiedCases", -1) >= thresholds.get("blindBestOrTiedCasesMin", math.inf),
        "blindFailureCount": observed.get("blindUnacceptableCases", math.inf) <= thresholds.get("blindUnacceptableCasesMax", -1),
        "outsideMaskIntegrity": observed.get("outsideMaskExactCases", -1) == observed.get("caseCount")
        and thresholds.get("outsideMaskMustRemainExact") is True,
    }
    if observed.get("modelBytes") != candidate["modelBytes"] or observed.get("runtimePackageLowerBoundBytes") != expected_stack:
        fail("decision package measurements are internally inconsistent")
    if not close_enough(observed.get("sessionLoadMs"), probe["sessionLoadMs"]):
        fail("decision session load does not match resource probe")
    if not close_enough(observed.get("inferenceP95MsWorstCase"), expected_p95):
        fail("decision p95 does not match benchmark")
    if observed.get("singleRunPeakRssBytes") != probe["processPeakRssBytes"]:
        fail("decision peak RSS does not match resource probe")
    if observed.get("blindBestOrTiedCases") != winner_counts["lama_roi"]:
        fail("decision blind preference count is stale")
    if observed.get("blindUnacceptableCases") != len(candidate_unacceptable):
        fail("decision blind failure count is stale")
    if observed.get("outsideMaskExactCases") != expected_outside or observed.get("caseCount") != len(cases):
        fail("decision mask integrity count is stale")
    if decision.get("checks") != comparisons:
        fail("decision threshold checks are stale")
    if decision.get("eligibleForProductImplementation") is not all(comparisons.values()):
        fail("decision eligibility does not match threshold checks")

    sheet_manifest = load_json(EVIDENCE / "sheets-manifest.json")
    if sheet_manifest.get("schemaVersion") != 1 or sheet_manifest.get("corpusId") != manifest["id"]:
        fail("review sheet manifest is bound to the wrong corpus")
    sheet_records = sheet_manifest.get("sheets")
    if not isinstance(sheet_records, list) or len(sheet_records) != len(case_by_id):
        fail("review sheet manifest must cover every case")
    seen_sheets = set()
    for record in sheet_records:
        case_id = record.get("caseId")
        if case_id not in case_by_id or case_id in seen_sheets:
            fail(f"invalid or duplicate review sheet: {case_id}")
        path = owned_path(record.get("path"))
        if sha256(path) != record.get("sha256"):
            fail(f"review sheet hash mismatch: {case_id}")
        if png_size(path) != (record.get("width"), record.get("height")):
            fail(f"review sheet dimensions mismatch: {case_id}")
        seen_sheets.add(case_id)
    if seen_sheets != set(case_by_id):
        fail("review sheet case set is incomplete")
    print("smart-erase feasibility evidence: OK (candidate remains gated)")


if __name__ == "__main__":
    main()
