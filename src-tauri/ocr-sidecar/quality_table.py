#!/usr/bin/env python3
"""表格 OCR 双层评测：原始 cell、几何聚合 row、顺序与文本分别计分。"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any

import quality_metrics as quality


TABLE_CORPUS_SCHEMA = "clippy-ocr-table-corpus-v1"
TABLE_REPORT_SCHEMA = "clippy-ocr-table-report-v1"
MAX_TABLES_PER_CASE = 32
MAX_ROWS_PER_TABLE = 512
MAX_CELLS_PER_ROW = 128
EPSILON = 1e-9


def _object(value: Any, where: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise quality.ContractError(f"{where} 必须是 object")
    return value


def _list(value: Any, where: str) -> list[Any]:
    if not isinstance(value, list):
        raise quality.ContractError(f"{where} 必须是 array")
    return value


def _string(value: Any, where: str, *, empty: bool = False) -> str:
    if not isinstance(value, str) or (not empty and not value):
        raise quality.ContractError(f"{where} 必须是{'可为空' if empty else '非空'} string")
    return value


def validate_table_corpus(value: Any) -> dict[str, Any]:
    """验证表格层级，并复用普通语料合同校验图片、坐标和文本预算。"""
    corpus = _object(value, "tableCorpus")
    if set(corpus) != {"schema", "cases"} or corpus.get("schema") != TABLE_CORPUS_SCHEMA:
        raise quality.ContractError(f"tableCorpus 必须且只允许 schema={TABLE_CORPUS_SCHEMA} 与 cases")
    cases = _list(corpus["cases"], "tableCorpus.cases")
    if len(cases) > quality.MAX_CASES:
        raise quality.ContractError("tableCorpus.cases 超出数量预算")
    normalized = []
    ordinary_cases = []
    case_ids: set[str] = set()
    for case_index, raw_case in enumerate(cases):
        where = f"tableCorpus.cases[{case_index}]"
        case = _object(raw_case, where)
        if set(case) != {"id", "tags", "source", "tables"}:
            raise quality.ContractError(f"{where} 必须且只允许 id/tags/source/tables")
        case_id = _string(case["id"], f"{where}.id")
        if case_id in case_ids:
            raise quality.ContractError(f"重复 table case id: {case_id}")
        case_ids.add(case_id)
        tables = _list(case["tables"], f"{where}.tables")
        if not tables or len(tables) > MAX_TABLES_PER_CASE:
            raise quality.ContractError(f"{where}.tables 数量无效")
        clean_tables = []
        table_ids: set[str] = set()
        ordinary_lines = []
        all_ids: set[str] = set()
        for table_index, raw_table in enumerate(tables):
            table_where = f"{where}.tables[{table_index}]"
            table = _object(raw_table, table_where)
            if set(table) != {"id", "rows"}:
                raise quality.ContractError(f"{table_where} 必须且只允许 id/rows")
            table_id = _string(table["id"], f"{table_where}.id")
            if table_id in table_ids:
                raise quality.ContractError(f"{where} 的 table id 不能重复")
            table_ids.add(table_id)
            rows = _list(table["rows"], f"{table_where}.rows")
            if not rows or len(rows) > MAX_ROWS_PER_TABLE:
                raise quality.ContractError(f"{table_where}.rows 数量无效")
            clean_rows = []
            row_ids: set[str] = set()
            for row_index, raw_row in enumerate(rows):
                row_where = f"{table_where}.rows[{row_index}]"
                row = _object(raw_row, row_where)
                if set(row) != {"id", "cells"}:
                    raise quality.ContractError(f"{row_where} 必须且只允许 id/cells")
                row_id = _string(row["id"], f"{row_where}.id")
                if row_id in row_ids:
                    raise quality.ContractError(f"{table_where} 的 row id 不能重复")
                row_ids.add(row_id)
                cells = _list(row["cells"], f"{row_where}.cells")
                if not cells or len(cells) > MAX_CELLS_PER_ROW:
                    raise quality.ContractError(f"{row_where}.cells 数量无效")
                clean_cells = []
                columns: list[int] = []
                for cell_index, raw_cell in enumerate(cells):
                    cell_where = f"{row_where}.cells[{cell_index}]"
                    cell = _object(raw_cell, cell_where)
                    if set(cell) != {"id", "text", "quad", "columnIndex"}:
                        raise quality.ContractError(
                            f"{cell_where} 必须且只允许 id/text/quad/columnIndex"
                        )
                    cell_id = _string(cell["id"], f"{cell_where}.id")
                    if cell_id in all_ids:
                        raise quality.ContractError(f"{where} 的 cell id 不能重复")
                    all_ids.add(cell_id)
                    column = cell["columnIndex"]
                    if isinstance(column, bool) or not isinstance(column, int) or column < 0:
                        raise quality.ContractError(f"{cell_where}.columnIndex 必须是非负 integer")
                    columns.append(column)
                    clean_cell = {
                        "id": cell_id,
                        "text": _string(cell["text"], f"{cell_where}.text", empty=True),
                        "quad": cell["quad"],
                        "columnIndex": column,
                    }
                    clean_cells.append(clean_cell)
                    ordinary_lines.append(
                        {
                            "id": cell_id,
                            "text": clean_cell["text"],
                            "quad": clean_cell["quad"],
                            "kind": "text",
                        }
                    )
                if columns != sorted(columns) or len(columns) != len(set(columns)):
                    raise quality.ContractError(f"{row_where}.columnIndex 必须严格递增且唯一")
                clean_rows.append({"id": row_id, "cells": clean_cells})
            clean_tables.append({"id": table_id, "rows": clean_rows})
        ordinary_cases.append(
            {
                "id": case_id,
                "tags": case["tags"],
                "source": case["source"],
                "lines": ordinary_lines,
            }
        )
        normalized.append(
            {
                "id": case_id,
                "tags": case["tags"],
                "source": case["source"],
                "tables": clean_tables,
            }
        )
    validated = quality.validate_corpus(
        {"schema": quality.CORPUS_SCHEMA, "cases": ordinary_cases}
    )
    for clean_case, validated_case in zip(normalized, validated["cases"], strict=True):
        clean_case["tags"] = validated_case["tags"]
        clean_case["source"] = validated_case["source"]
        validated_cells = iter(validated_case["lines"])
        for table in clean_case["tables"]:
            for row in table["rows"]:
                for cell in row["cells"]:
                    cell["quad"] = next(validated_cells)["quad"]
    return {"schema": TABLE_CORPUS_SCHEMA, "cases": normalized}


def _bounds(quad: list[list[float]]) -> tuple[float, float, float, float]:
    return (
        min(point[0] for point in quad),
        min(point[1] for point in quad),
        max(point[0] for point in quad),
        max(point[1] for point in quad),
    )


def _union_quad(lines: list[dict[str, Any]]) -> list[list[float]]:
    bounds = [_bounds(line["quad"]) for line in lines]
    left = min(value[0] for value in bounds)
    top = min(value[1] for value in bounds)
    right = max(value[2] for value in bounds)
    bottom = max(value[3] for value in bounds)
    return [[left, top], [right, top], [right, bottom], [left, bottom]]


def _vertical_overlap(left: dict[str, Any], right: dict[str, Any]) -> float:
    _, left_top, _, left_bottom = _bounds(left["quad"])
    _, right_top, _, right_bottom = _bounds(right["quad"])
    overlap = max(0.0, min(left_bottom, right_bottom) - max(left_top, right_top))
    return overlap / max(min(left_bottom - left_top, right_bottom - right_top), EPSILON)


def cluster_rows(lines: list[dict[str, Any]]) -> list[list[dict[str, Any]]]:
    """只按可解释几何聚合同行框，不读取真值或文字。"""
    ordered = sorted(lines, key=lambda line: ((_bounds(line["quad"])[1] + _bounds(line["quad"])[3]) / 2,
                                              _bounds(line["quad"])[0]))
    groups: list[list[dict[str, Any]]] = []
    for line in ordered:
        candidates = [
            (index, max(_vertical_overlap(line, existing) for existing in group))
            for index, group in enumerate(groups)
        ]
        candidates = [candidate for candidate in candidates if candidate[1] >= 0.5]
        if candidates:
            groups[max(candidates, key=lambda candidate: candidate[1])[0]].append(line)
        else:
            groups.append([line])
    for group in groups:
        group.sort(key=lambda line: _bounds(line["quad"])[0])
    groups.sort(key=lambda group: _bounds(_union_quad(group))[1])
    return groups


def _inside_scope(line: dict[str, Any], scope: list[list[float]]) -> bool:
    left, top, right, bottom = _bounds(line["quad"])
    scope_left, scope_top, scope_right, scope_bottom = _bounds(scope)
    center_x = (left + right) / 2
    center_y = (top + bottom) / 2
    return scope_left <= center_x <= scope_right and scope_top <= center_y <= scope_bottom


def _compact(text: str) -> str:
    return "".join(character for character in text if not character.isspace())


def _empty_counts() -> dict[str, Any]:
    return {
        "caseCount": 0,
        "expectedRows": 0,
        "predictedRows": 0,
        "matchedRows": 0,
        "rowIou": 0.0,
        "expectedCells": 0,
        "predictedCells": 0,
        "matchedCells": 0,
        "cellIou": 0.0,
        "orderInversions": 0,
        "orderPairs": 0,
        "expectedText": 0,
        "rawText": 0,
        "rawEdits": 0,
        "geometryText": 0,
        "geometryEdits": 0,
        "exactGeometryRows": 0,
    }


def _add_detection(
    counts: dict[str, Any],
    prefix: str,
    expected: list[dict[str, Any]],
    predicted: list[dict[str, Any]],
    threshold: float,
) -> None:
    matches = quality.match_quads(expected, predicted, threshold)
    counts[f"expected{prefix}"] += len(expected)
    counts[f"predicted{prefix}"] += len(predicted)
    counts[f"matched{prefix}"] += len(matches)
    counts[f"{prefix.lower()[:-1]}Iou"] += sum(match[2] for match in matches)


def _assignment_key(line: dict[str, Any], rows: list[dict[str, Any]]) -> int | None:
    row_candidates = [
        (index, _vertical_overlap(line, row)) for index, row in enumerate(rows)
    ]
    row_index, overlap = max(row_candidates, key=lambda candidate: candidate[1])
    if overlap < 0.5:
        return None
    line_left, _, line_right, _ = _bounds(line["quad"])
    cells = rows[row_index]["cells"]
    overlapping = []
    for index, cell in enumerate(cells):
        cell_left, _, cell_right, _ = _bounds(cell["quad"])
        horizontal = max(0.0, min(line_right, cell_right) - max(line_left, cell_left))
        if horizontal / max(min(line_right - line_left, cell_right - cell_left), EPSILON) >= 0.25:
            overlapping.append(index)
    if overlapping:
        column = min(overlapping)
    else:
        center = (line_left + line_right) / 2
        column = min(
            range(len(cells)),
            key=lambda index: abs(center - sum(_bounds(cells[index]["quad"])[::2]) / 2),
        )
    return row_index * MAX_CELLS_PER_ROW + column


def _add_case(
    counts: dict[str, Any],
    expected_case: dict[str, Any],
    predicted_case: dict[str, Any],
    threshold: float,
    line_geometry: bool,
) -> None:
    counts["caseCount"] += 1
    expected_text = _compact(
        "\n\n".join(
            "\n".join(
                "\t".join(cell["text"] for cell in row["cells"])
                for row in table["rows"]
            )
            for table in expected_case["tables"]
        )
    )
    raw_text = _compact(predicted_case["text"])
    counts["expectedText"] += len(expected_text)
    counts["rawText"] += len(raw_text)
    counts["rawEdits"] += quality.levenshtein_distance(expected_text, raw_text)
    geometry_parts = []
    for table in expected_case["tables"]:
        cells = [cell for row in table["rows"] for cell in row["cells"]]
        expected_rows = [
            {"quad": _union_quad(row["cells"]), "cells": row["cells"]}
            for row in table["rows"]
        ]
        if not line_geometry:
            continue
        scope = _union_quad(cells)
        predicted_lines = [line for line in predicted_case["lines"] if _inside_scope(line, scope)]
        predicted_groups = cluster_rows(predicted_lines)
        predicted_rows = [{"quad": _union_quad(group)} for group in predicted_groups]
        _add_detection(counts, "Rows", expected_rows, predicted_rows, threshold)
        _add_detection(counts, "Cells", cells, predicted_lines, threshold)

        keys = [key for line in predicted_lines if (key := _assignment_key(line, expected_rows)) is not None]
        inversions, pairs = quality.inversion_counts(keys)
        counts["orderInversions"] += inversions
        counts["orderPairs"] += pairs

        geometry_rows = ["".join(_compact(line["text"]) for line in group) for group in predicted_groups]
        geometry_parts.append("".join(geometry_rows))
        expected_row_texts = ["".join(_compact(cell["text"]) for cell in row["cells"]) for row in table["rows"]]
        counts["exactGeometryRows"] += sum(
            expected == predicted
            for expected, predicted in zip(expected_row_texts, geometry_rows)
        )
    if line_geometry:
        geometry_text = "".join(geometry_parts)
        counts["geometryText"] += len(geometry_text)
        counts["geometryEdits"] += quality.levenshtein_distance(expected_text, geometry_text)


def _detection(counts: dict[str, Any], prefix: str, threshold: float, supported: bool) -> dict[str, Any]:
    if not supported:
        return {"supported": False, "iouThreshold": threshold}
    expected = counts[f"expected{prefix}"]
    predicted = counts[f"predicted{prefix}"]
    matched = counts[f"matched{prefix}"]
    precision = matched / predicted if predicted else 1.0
    recall = matched / expected if expected else 1.0
    return {
        "supported": True,
        "iouThreshold": threshold,
        "truePositive": matched,
        "falsePositive": predicted - matched,
        "falseNegative": expected - matched,
        "precision": precision,
        "recall": recall,
        "hmean": 2 * precision * recall / (precision + recall) if precision + recall else 0.0,
        "meanMatchedIou": counts[f"{prefix.lower()[:-1]}Iou"] / matched if matched else 1.0,
    }


def _finalize(counts: dict[str, Any], threshold: float, line_geometry: bool) -> dict[str, Any]:
    expected_text = counts["expectedText"]
    return {
        "caseCount": counts["caseCount"],
        "rowDetection": _detection(counts, "Rows", threshold, line_geometry),
        "cellDetection": _detection(counts, "Cells", threshold, line_geometry),
        "rawReadingOrder": {
            "supported": line_geometry,
            "inversions": counts["orderInversions"],
            "comparablePairs": counts["orderPairs"],
            "errorRate": counts["orderInversions"] / counts["orderPairs"]
            if counts["orderPairs"]
            else (0.0 if line_geometry else None),
        },
        "text": {
            "expectedNonWhitespaceCodepoints": expected_text,
            "rawNonWhitespaceCodepoints": counts["rawText"],
            "rawNonWhitespaceEditDistance": counts["rawEdits"],
            "rawNonWhitespaceCer": counts["rawEdits"] / expected_text if expected_text else 0.0,
            "geometryNonWhitespaceCodepoints": counts["geometryText"] if line_geometry else None,
            "geometryNonWhitespaceEditDistance": counts["geometryEdits"] if line_geometry else None,
            "geometryNonWhitespaceCer": counts["geometryEdits"] / expected_text
            if line_geometry and expected_text
            else (0.0 if line_geometry else None),
            "exactGeometryRowRecall": counts["exactGeometryRows"] / counts["expectedRows"]
            if line_geometry and counts["expectedRows"]
            else (0.0 if line_geometry else None),
        },
    }


def evaluate(table_corpus: Any, predictions: Any, *, iou_threshold: float = 0.5) -> dict[str, Any]:
    if not math.isfinite(iou_threshold) or not 0 < iou_threshold <= 1:
        raise quality.ContractError("iou threshold 必须位于 (0, 1]")
    corpus = validate_table_corpus(table_corpus)
    raw_prediction = _object(predictions, "predictions")
    raw_cases = _list(raw_prediction.get("cases"), "predictions.cases")
    accepted_case_ids = {
        case.get("id") for case in raw_cases if isinstance(case, dict) and isinstance(case.get("id"), str)
    } | {case["id"] for case in corpus["cases"]}
    prediction = quality.validate_predictions(predictions, accepted_case_ids)
    capabilities = prediction["engine"]["capabilities"]
    predicted_by_id = {case["id"]: case for case in prediction["cases"]}
    total = _empty_counts()
    by_tag: dict[str, dict[str, Any]] = {}
    for expected_case in corpus["cases"]:
        predicted_case = predicted_by_id.get(
            expected_case["id"], {"id": expected_case["id"], "text": "", "lines": []}
        )
        _add_case(total, expected_case, predicted_case, iou_threshold, capabilities["lineGeometry"])
        for tag in expected_case["tags"]:
            counts = by_tag.setdefault(tag, _empty_counts())
            _add_case(counts, expected_case, predicted_case, iou_threshold, capabilities["lineGeometry"])
    return {
        "schema": TABLE_REPORT_SCHEMA,
        "engine": prediction["engine"],
        "summary": _finalize(total, iou_threshold, capabilities["lineGeometry"]),
        "byTag": {
            tag: _finalize(counts, iou_threshold, capabilities["lineGeometry"])
            for tag, counts in sorted(by_tag.items())
        },
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--predictions", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--iou-threshold", type=float, default=0.5)
    arguments = parser.parse_args(argv)
    raw_corpus = quality.load_json(arguments.corpus)
    clean_corpus = validate_table_corpus(raw_corpus)
    for case in clean_corpus["cases"]:
        quality.read_case_png(arguments.corpus, case)
    report = evaluate(
        clean_corpus,
        quality.load_json(arguments.predictions),
        iou_threshold=arguments.iou_threshold,
    )
    encoded = json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
    if arguments.output.exists():
        raise quality.ContractError(f"输出已存在，不会覆盖: {arguments.output}")
    arguments.output.write_text(encoded, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
