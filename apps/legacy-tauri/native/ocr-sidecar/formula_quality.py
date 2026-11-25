#!/usr/bin/env python3
"""独立公式识别的严格合同与 LaTeX token 质量报告。"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import re
import sys
from typing import Any

import quality_metrics

PREDICTION_SCHEMA = "clippy-ocr-formula-predictions-v1"
REPORT_SCHEMA = "clippy-ocr-formula-report-v1"
MAX_CONTRACT_BYTES = 4 * 1024 * 1024
MAX_LATEX_BYTES = 32 * 1024
MAX_TOKENS = 4096
SHA256 = re.compile(r"[0-9a-f]{64}")


def load_json(path: Path) -> Any:
    payload = path.read_bytes()
    if not payload or len(payload) > MAX_CONTRACT_BYTES:
        raise quality_metrics.ContractError(f"JSON 合同为空或超过 {MAX_CONTRACT_BYTES} 字节: {path}")
    try:
        return json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise quality_metrics.ContractError(f"JSON 合同无效: {path}") from error


def _string(value: Any, where: str, *, maximum: int = 256) -> str:
    if not isinstance(value, str) or not value or len(value.encode("utf-8")) > maximum:
        raise quality_metrics.ContractError(f"{where} 必须是预算内非空字符串")
    return value


def _number(value: Any, where: str, *, maximum: float) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise quality_metrics.ContractError(f"{where} 必须是有限数值")
    result = float(value)
    if result < 0 or result > maximum:
        raise quality_metrics.ContractError(f"{where} 超出范围")
    return result


def validate_formula_corpus(corpus_path: Path) -> dict[str, Any]:
    corpus = quality_metrics.validate_corpus(quality_metrics.load_json(corpus_path))
    quality_metrics.validate_corpus_assets(corpus_path, corpus)
    for case in corpus["cases"]:
        if len(case["lines"]) != 1 or case["lines"][0]["kind"] != "formula":
            raise quality_metrics.ContractError(f"{case['id']} 必须只包含一个公式 crop 真值")
        formula = case["lines"][0].get("formula")
        if formula is None or formula["format"] != "latex":
            raise quality_metrics.ContractError(f"{case['id']} 缺少 LaTeX 真值")
    return corpus


def validate_predictions(value: Any, expected_ids: set[str]) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != {"schema", "engine", "cases"}:
        raise quality_metrics.ContractError("公式预测顶层字段无效")
    if value.get("schema") != PREDICTION_SCHEMA:
        raise quality_metrics.ContractError("公式预测 schema 无效")
    engine = value.get("engine")
    required_engine = {"id", "version", "runtime", "modelSha256", "modelBytes", "modelLoadMs", "license"}
    if not isinstance(engine, dict) or set(engine) != required_engine:
        raise quality_metrics.ContractError("公式预测 engine 字段无效")
    clean_engine = {
        "id": _string(engine.get("id"), "engine.id"),
        "version": _string(engine.get("version"), "engine.version"),
        "runtime": _string(engine.get("runtime"), "engine.runtime", maximum=512),
        "modelSha256": _string(engine.get("modelSha256"), "engine.modelSha256", maximum=64),
        "modelBytes": int(_number(engine.get("modelBytes"), "engine.modelBytes", maximum=2 * 1024**3)),
        "modelLoadMs": _number(engine.get("modelLoadMs"), "engine.modelLoadMs", maximum=600_000),
        "license": _string(engine.get("license"), "engine.license"),
    }
    if not SHA256.fullmatch(clean_engine["modelSha256"]) or clean_engine["modelBytes"] <= 0:
        raise quality_metrics.ContractError("公式模型 SHA 或字节数无效")
    cases = value.get("cases")
    if not isinstance(cases, list) or len(cases) != len(expected_ids):
        raise quality_metrics.ContractError("公式预测 case 数量无效")
    clean_cases = []
    seen: set[str] = set()
    for index, case in enumerate(cases):
        where = f"cases[{index}]"
        allowed = {"id", "latex", "latexParseable", "durationMs", "peakMemoryBytes"}
        if not isinstance(case, dict) or set(case) - allowed or not {"id", "latex", "latexParseable", "durationMs"} <= set(case):
            raise quality_metrics.ContractError(f"{where} 字段无效")
        identifier = _string(case.get("id"), f"{where}.id")
        if identifier in seen or identifier not in expected_ids:
            raise quality_metrics.ContractError(f"{where}.id 重复或未知")
        seen.add(identifier)
        latex = _string(case.get("latex"), f"{where}.latex", maximum=MAX_LATEX_BYTES)
        clean = {
            "id": identifier,
            "latex": latex,
            "latexParseable": case.get("latexParseable"),
            "durationMs": _number(case.get("durationMs"), f"{where}.durationMs", maximum=600_000),
        }
        if not isinstance(clean["latexParseable"], bool):
            raise quality_metrics.ContractError(f"{where}.latexParseable 必须是 boolean")
        if "peakMemoryBytes" in case:
            memory = int(_number(case["peakMemoryBytes"], f"{where}.peakMemoryBytes", maximum=16 * 1024**3))
            if memory <= 0:
                raise quality_metrics.ContractError(f"{where}.peakMemoryBytes 必须大于 0")
            clean["peakMemoryBytes"] = memory
        clean_cases.append(clean)
    if seen != expected_ids:
        raise quality_metrics.ContractError("公式预测缺少 case")
    return {"schema": PREDICTION_SCHEMA, "engine": clean_engine, "cases": clean_cases}


def strip_math_delimiters(latex: str) -> str:
    value = latex.strip()
    pairs = (("$$", "$$"), ("\\[", "\\]"), ("\\(", "\\)"), ("$", "$"))
    for opening, closing in pairs:
        if value.startswith(opening) and value.endswith(closing) and len(value) > len(opening) + len(closing):
            return value[len(opening):-len(closing)].strip()
    return value


def latex_tokens(latex: str) -> list[str]:
    """忽略普通数学模式空白；不改写命令、括号或符号。"""
    value = strip_math_delimiters(latex)
    tokens: list[str] = []
    index = 0
    while index < len(value):
        character = value[index]
        if character.isspace():
            index += 1
            continue
        if character == "\\":
            start = index
            index += 1
            if index < len(value) and value[index].isalpha():
                while index < len(value) and value[index].isalpha():
                    index += 1
            elif index < len(value):
                index += 1
            tokens.append(value[start:index])
            continue
        if character.isalpha():
            start = index
            index += 1
            while index < len(value) and value[index].isalpha():
                index += 1
            tokens.append(value[start:index])
            continue
        if character.isdigit():
            start = index
            index += 1
            while index < len(value) and (value[index].isdigit() or value[index] == "."):
                index += 1
            tokens.append(value[start:index])
            continue
        tokens.append(character)
        index += 1
    if not tokens or len(tokens) > MAX_TOKENS:
        raise quality_metrics.ContractError("LaTeX token 为空或超过预算")
    return tokens


def token_distance(expected: list[str], actual: list[str]) -> int:
    if len(expected) > MAX_TOKENS or len(actual) > MAX_TOKENS:
        raise quality_metrics.ContractError("LaTeX token 超过编辑距离预算")
    if len(expected) < len(actual):
        expected, actual = actual, expected
    previous = list(range(len(actual) + 1))
    for row, left in enumerate(expected, 1):
        current = [row]
        for column, right in enumerate(actual, 1):
            current.append(min(current[-1] + 1, previous[column] + 1, previous[column - 1] + (left != right)))
        previous = current
    return previous[-1]


def percentile(values: list[float], ratio: float) -> float:
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    position = (len(ordered) - 1) * ratio
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def evaluate(corpus: dict[str, Any], predictions: dict[str, Any]) -> dict[str, Any]:
    expected_by_id = {case["id"]: case for case in corpus["cases"]}
    prediction_by_id = {case["id"]: case for case in predictions["cases"]}
    reports = []
    raw_exact = normalized_exact = parseable = distance_total = expected_token_total = 0
    durations: list[float] = []
    memories: list[int] = []
    for identifier, case in expected_by_id.items():
        expected = case["lines"][0]["formula"]["value"]
        predicted = prediction_by_id[identifier]["latex"]
        expected_tokens = latex_tokens(expected)
        predicted_tokens = latex_tokens(predicted)
        distance = token_distance(expected_tokens, predicted_tokens)
        raw = expected == predicted
        normalized = expected_tokens == predicted_tokens
        raw_exact += raw
        normalized_exact += normalized
        parseable += prediction_by_id[identifier]["latexParseable"]
        distance_total += distance
        expected_token_total += len(expected_tokens)
        duration = prediction_by_id[identifier]["durationMs"]
        durations.append(duration)
        if "peakMemoryBytes" in prediction_by_id[identifier]:
            memories.append(prediction_by_id[identifier]["peakMemoryBytes"])
        reports.append({
            "id": identifier,
            "tags": case["tags"],
            "expectedLatex": expected,
            "predictedLatex": predicted,
            "rawExact": raw,
            "normalizedExact": normalized,
            "latexParseable": prediction_by_id[identifier]["latexParseable"],
            "expectedTokens": expected_tokens,
            "predictedTokens": predicted_tokens,
            "tokenDistance": distance,
            "tokenErrorRate": distance / max(1, len(expected_tokens)),
            "durationMs": duration,
            **({"peakMemoryBytes": prediction_by_id[identifier]["peakMemoryBytes"]} if "peakMemoryBytes" in prediction_by_id[identifier] else {}),
        })
    count = len(reports)
    return {
        "schema": REPORT_SCHEMA,
        "engine": predictions["engine"],
        "cases": reports,
        "summary": {
            "cases": count,
            "rawExact": raw_exact,
            "rawExactRate": raw_exact / count,
            "normalizedExact": normalized_exact,
            "normalizedExactRate": normalized_exact / count,
            "latexParseable": parseable,
            "latexParseableRate": parseable / count,
            "tokenDistance": distance_total,
            "expectedTokens": expected_token_total,
            "tokenErrorRate": distance_total / max(1, expected_token_total),
            "durationMsP50": percentile(durations, 0.5),
            "durationMsP95": percentile(durations, 0.95),
            "peakMemoryBytes": max(memories) if memories else None,
        },
    }


def write_new(path: Path, value: dict[str, Any]) -> None:
    try:
        with path.open("x", encoding="utf-8") as handle:
            json.dump(value, handle, ensure_ascii=False, sort_keys=True, indent=2)
            handle.write("\n")
    except FileExistsError as error:
        raise quality_metrics.ContractError(f"输出已存在，不会覆盖: {path}") from error


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--predictions", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    arguments = parser.parse_args(argv)
    try:
        corpus = validate_formula_corpus(arguments.corpus)
        predictions = validate_predictions(load_json(arguments.predictions), {case["id"] for case in corpus["cases"]})
        write_new(arguments.output, evaluate(corpus, predictions))
    except (OSError, quality_metrics.ContractError) as error:
        print(f"ocr-formula-quality: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
