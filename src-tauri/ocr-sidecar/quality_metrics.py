#!/usr/bin/env python3
"""Clippy OCR 质量评测器。

评测器只消费人工标注合同和引擎输出，不加载 OCR 模型。它刻意保留原始 Unicode、
空白和行顺序，避免用归一化总分掩盖复制文本时可见的退化。
"""

from __future__ import annotations

import argparse
from collections import Counter
from difflib import SequenceMatcher
import hashlib
import json
import math
import stat
from pathlib import Path
import re
import statistics
import sys
from typing import Any, Iterable
import zlib


CORPUS_SCHEMA = "clippy-ocr-quality-corpus-v1"
PREDICTION_SCHEMA = "clippy-ocr-quality-predictions-v1"
REPORT_SCHEMA = "clippy-ocr-quality-report-v1"
MAX_INPUT_BYTES = 16 * 1024 * 1024
MAX_PNG_BYTES = 64 * 1024 * 1024
MAX_CASES = 2_000
MAX_LINES_PER_CASE = 512
MAX_TEXT_CODEPOINTS = 2_000_000
MAX_IMAGE_PIXELS = 32 * 1024 * 1024
MAX_IMAGE_DIMENSION = 16_384
MAX_COORDINATE = 1_000_000_000.0
DEFAULT_IOU_THRESHOLD = 0.5
EPSILON = 1e-9


class ContractError(ValueError):
    """质量合同无效。"""


def _require_object(value: Any, where: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ContractError(f"{where} 必须是 object")
    return value


def _require_list(value: Any, where: str) -> list[Any]:
    if not isinstance(value, list):
        raise ContractError(f"{where} 必须是 array")
    return value


def _require_string(value: Any, where: str, *, allow_empty: bool = False) -> str:
    if not isinstance(value, str) or (not allow_empty and not value):
        raise ContractError(f"{where} 必须是{'可为空的' if allow_empty else '非空'} string")
    return value


def _require_number(value: Any, where: str, *, integer: bool = False) -> float | int:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ContractError(f"{where} 必须是 number")
    if integer and not isinstance(value, int):
        raise ContractError(f"{where} 必须是 integer")
    if not math.isfinite(float(value)) or value < 0:
        raise ContractError(f"{where} 必须是非负有限数")
    return value


def _validate_quad(value: Any, where: str) -> list[list[float]]:
    points = _require_list(value, where)
    if len(points) != 4:
        raise ContractError(f"{where} 必须包含四个点")
    result: list[list[float]] = []
    for index, point in enumerate(points):
        pair = _require_list(point, f"{where}[{index}]")
        if len(pair) != 2:
            raise ContractError(f"{where}[{index}] 必须是 [x, y]")
        coordinates: list[float] = []
        for axis, coordinate in enumerate(pair):
            if isinstance(coordinate, bool) or not isinstance(coordinate, (int, float)):
                raise ContractError(f"{where}[{index}][{axis}] 必须是 number")
            number = float(coordinate)
            if not math.isfinite(number) or abs(number) > MAX_COORDINATE:
                raise ContractError(f"{where}[{index}][{axis}] 超出坐标预算")
            coordinates.append(number)
        result.append(coordinates)
    if polygon_area(convex_hull(result)) <= EPSILON:
        raise ContractError(f"{where} 面积必须大于零")
    return result


def _validate_formula(value: Any, where: str) -> dict[str, str]:
    formula = _require_object(value, where)
    if set(formula) != {"format", "value"}:
        raise ContractError(f"{where} 只允许 format/value")
    return {
        "format": _require_string(formula.get("format"), f"{where}.format"),
        "value": _require_string(formula.get("value"), f"{where}.value", allow_empty=True),
    }


def _validate_line(value: Any, where: str, *, expected: bool) -> dict[str, Any]:
    line = _require_object(value, where)
    allowed = {"text", "quad", "kind", "formula", "paragraphId"}
    if expected:
        allowed.add("id")
    unknown = set(line) - allowed
    if unknown:
        raise ContractError(f"{where} 含未知字段: {sorted(unknown)}")

    result: dict[str, Any] = {
        "text": _require_string(line.get("text"), f"{where}.text", allow_empty=True),
        "quad": _validate_quad(line.get("quad"), f"{where}.quad"),
        "kind": line.get("kind", "text"),
    }
    if result["kind"] not in {"text", "formula"}:
        raise ContractError(f"{where}.kind 必须是 text 或 formula")
    if expected:
        result["id"] = _require_string(line.get("id"), f"{where}.id")
    if "paragraphId" in line:
        result["paragraphId"] = _require_string(
            line["paragraphId"], f"{where}.paragraphId"
        )
    if "formula" in line:
        result["formula"] = _validate_formula(line["formula"], f"{where}.formula")
    if result["kind"] == "formula" and expected and "formula" not in result:
        raise ContractError(f"{where} 的公式真值必须包含 formula")
    return result


def validate_corpus(value: Any) -> dict[str, Any]:
    corpus = _require_object(value, "corpus")
    if corpus.get("schema") != CORPUS_SCHEMA:
        raise ContractError(f"corpus.schema 必须是 {CORPUS_SCHEMA}")
    cases = _require_list(corpus.get("cases"), "corpus.cases")
    if len(cases) > MAX_CASES:
        raise ContractError("corpus.cases 超出数量预算")

    result_cases: list[dict[str, Any]] = []
    case_ids: set[str] = set()
    total_text = 0
    for case_index, raw_case in enumerate(cases):
        where = f"corpus.cases[{case_index}]"
        case = _require_object(raw_case, where)
        allowed = {"id", "tags", "source", "lines"}
        unknown = set(case) - allowed
        if unknown:
            raise ContractError(f"{where} 含未知字段: {sorted(unknown)}")
        case_id = _require_string(case.get("id"), f"{where}.id")
        if case_id in case_ids:
            raise ContractError(f"重复 case id: {case_id}")
        case_ids.add(case_id)

        tags = _require_list(case.get("tags"), f"{where}.tags")
        clean_tags = [_require_string(tag, f"{where}.tags") for tag in tags]
        if len(clean_tags) != len(set(clean_tags)):
            raise ContractError(f"{where}.tags 不能重复")

        source = _require_object(case.get("source"), f"{where}.source")
        source_fields = {"kind", "license", "imagePath", "sha256", "width", "height"}
        if set(source) != source_fields:
            raise ContractError(f"{where}.source 必须且只允许 {sorted(source_fields)}")
        image_path = _require_string(
            source.get("imagePath"), f"{where}.source.imagePath"
        )
        path_parts = Path(image_path).parts
        if Path(image_path).is_absolute() or ".." in path_parts or "\\" in image_path:
            raise ContractError(f"{where}.source.imagePath 必须是无回退段的相对 POSIX 路径")
        sha256 = _require_string(source.get("sha256"), f"{where}.source.sha256")
        if not re.fullmatch(r"[0-9a-f]{64}", sha256):
            raise ContractError(f"{where}.source.sha256 必须是小写 SHA-256")
        width = int(
            _require_number(source.get("width"), f"{where}.source.width", integer=True)
        )
        height = int(
            _require_number(source.get("height"), f"{where}.source.height", integer=True)
        )
        if (
            width == 0
            or height == 0
            or max(width, height) > MAX_IMAGE_DIMENSION
            or width * height > MAX_IMAGE_PIXELS
        ):
            raise ContractError(f"{where}.source 尺寸超出 OCR 输入预算")
        clean_source = {
            "kind": _require_string(source.get("kind"), f"{where}.source.kind"),
            "license": _require_string(source.get("license"), f"{where}.source.license"),
            "imagePath": image_path,
            "sha256": sha256,
            "width": width,
            "height": height,
        }

        lines = _require_list(case.get("lines"), f"{where}.lines")
        if len(lines) > MAX_LINES_PER_CASE:
            raise ContractError(f"{where}.lines 超出数量预算")
        clean_lines = [
            _validate_line(line, f"{where}.lines[{index}]", expected=True)
            for index, line in enumerate(lines)
        ]
        if any(
            coordinate < 0
            or (axis == 0 and coordinate > width)
            or (axis == 1 and coordinate > height)
            for clean_line in clean_lines
            for point in clean_line["quad"]
            for axis, coordinate in enumerate(point)
        ):
            raise ContractError(f"{where}.lines 的 quad 必须位于原图范围内")
        line_ids = [line["id"] for line in clean_lines]
        if len(line_ids) != len(set(line_ids)):
            raise ContractError(f"{where}.lines 的 id 不能重复")
        total_text += sum(len(line["text"]) for line in clean_lines)
        if total_text > MAX_TEXT_CODEPOINTS:
            raise ContractError("corpus 文字总量超出预算")
        result_cases.append(
            {
                "id": case_id,
                "tags": clean_tags,
                "source": clean_source,
                "lines": clean_lines,
            }
        )
    return {"schema": CORPUS_SCHEMA, "cases": result_cases}


def validate_predictions(value: Any, corpus_case_ids: set[str]) -> dict[str, Any]:
    predictions = _require_object(value, "predictions")
    if predictions.get("schema") != PREDICTION_SCHEMA:
        raise ContractError(f"predictions.schema 必须是 {PREDICTION_SCHEMA}")
    engine = _require_object(predictions.get("engine"), "predictions.engine")
    if set(engine) != {"id", "version", "capabilities"}:
        raise ContractError("predictions.engine 必须且只允许 id/version/capabilities")
    capabilities = _require_object(
        engine.get("capabilities"), "predictions.engine.capabilities"
    )
    if set(capabilities) != {"lineGeometry", "structuredFormula"} or any(
        not isinstance(value, bool) for value in capabilities.values()
    ):
        raise ContractError(
            "predictions.engine.capabilities 必须包含 boolean lineGeometry/structuredFormula"
        )
    if capabilities["structuredFormula"] and not capabilities["lineGeometry"]:
        raise ContractError("structuredFormula 需要 lineGeometry")
    clean_engine = {
        "id": _require_string(engine.get("id"), "predictions.engine.id"),
        "version": _require_string(engine.get("version"), "predictions.engine.version"),
        "capabilities": dict(capabilities),
    }

    cases = _require_list(predictions.get("cases"), "predictions.cases")
    if len(cases) > MAX_CASES:
        raise ContractError("predictions.cases 超出数量预算")
    result_cases: list[dict[str, Any]] = []
    seen: set[str] = set()
    total_text = 0
    for case_index, raw_case in enumerate(cases):
        where = f"predictions.cases[{case_index}]"
        case = _require_object(raw_case, where)
        allowed = {"id", "text", "lines", "durationMs", "peakMemoryBytes"}
        unknown = set(case) - allowed
        if unknown:
            raise ContractError(f"{where} 含未知字段: {sorted(unknown)}")
        case_id = _require_string(case.get("id"), f"{where}.id")
        if case_id not in corpus_case_ids:
            raise ContractError(f"预测包含未知 case: {case_id}")
        if case_id in seen:
            raise ContractError(f"预测重复 case: {case_id}")
        seen.add(case_id)
        text = _require_string(case.get("text"), f"{where}.text", allow_empty=True)
        lines = _require_list(case.get("lines"), f"{where}.lines")
        if len(lines) > MAX_LINES_PER_CASE:
            raise ContractError(f"{where}.lines 超出数量预算")
        clean_lines = [
            _validate_line(line, f"{where}.lines[{index}]", expected=False)
            for index, line in enumerate(lines)
        ]
        if not capabilities["lineGeometry"] and clean_lines:
            raise ContractError(f"{where}.lines 在 lineGeometry=false 时必须为空")
        total_text += len(text) + sum(len(line["text"]) for line in clean_lines)
        if total_text > MAX_TEXT_CODEPOINTS:
            raise ContractError("predictions 文字总量超出预算")
        clean: dict[str, Any] = {"id": case_id, "text": text, "lines": clean_lines}
        if "durationMs" in case:
            clean["durationMs"] = float(
                _require_number(case["durationMs"], f"{where}.durationMs")
            )
        if "peakMemoryBytes" in case:
            clean["peakMemoryBytes"] = int(
                _require_number(
                    case["peakMemoryBytes"], f"{where}.peakMemoryBytes", integer=True
                )
            )
        result_cases.append(clean)
    return {"schema": PREDICTION_SCHEMA, "engine": clean_engine, "cases": result_cases}


def load_json(path: Path) -> Any:
    size = path.stat().st_size
    if size > MAX_INPUT_BYTES:
        raise ContractError(f"{path} 超出 {MAX_INPUT_BYTES} 字节预算")
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def file_sha256(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def png_dimensions(payload: bytes) -> tuple[int, int]:
    """校验有界 PNG chunk/CRC，并返回 IHDR 尺寸；不解压像素。"""
    if len(payload) < 57 or len(payload) > MAX_PNG_BYTES or payload[:8] != b"\x89PNG\r\n\x1a\n":
        raise ContractError("语料图片必须是预算内 PNG")
    offset = 8
    width = height = 0
    saw_header = saw_data = saw_end = False
    while offset < len(payload):
        if offset + 12 > len(payload):
            raise ContractError("语料 PNG chunk 被截断")
        length = int.from_bytes(payload[offset : offset + 4], "big")
        chunk_type = payload[offset + 4 : offset + 8]
        data_start = offset + 8
        data_end = data_start + length
        crc_end = data_end + 4
        if crc_end > len(payload):
            raise ContractError("语料 PNG chunk 超出文件")
        expected_crc = int.from_bytes(payload[data_end:crc_end], "big")
        actual_crc = zlib.crc32(chunk_type + payload[data_start:data_end]) & 0xFFFFFFFF
        if actual_crc != expected_crc:
            raise ContractError("语料 PNG chunk CRC 不符")
        if not saw_header:
            if chunk_type != b"IHDR" or length != 13:
                raise ContractError("语料 PNG 首个 chunk 必须是 IHDR")
            width = int.from_bytes(payload[data_start : data_start + 4], "big")
            height = int.from_bytes(payload[data_start + 4 : data_start + 8], "big")
            if width == 0 or height == 0:
                raise ContractError("语料 PNG 尺寸必须大于零")
            saw_header = True
        elif chunk_type == b"IHDR":
            raise ContractError("语料 PNG 不能重复 IHDR")
        if chunk_type == b"IDAT":
            saw_data = True
        if chunk_type == b"IEND":
            if length != 0 or crc_end != len(payload):
                raise ContractError("语料 PNG IEND 必须终止文件")
            saw_end = True
        offset = crc_end
    if not (saw_header and saw_data and saw_end):
        raise ContractError("语料 PNG 缺少 IHDR、IDAT 或 IEND")
    return width, height


def read_case_png(corpus_path: Path, case: dict[str, Any]) -> bytes:
    """按已校验 corpus 身份读取图片；拒绝逃逸、符号链接和非普通文件。"""
    source = case["source"]
    root = corpus_path.resolve().parent
    candidate = root / source["imagePath"]
    current = root
    try:
        for part in Path(source["imagePath"]).parts:
            current = current / part
            if current.is_symlink():
                raise ContractError(f"{case['id']} 图片路径不能包含符号链接")
        image_path = candidate.resolve(strict=True)
        image_path.relative_to(root)
        metadata = image_path.stat()
    except ContractError:
        raise
    except (OSError, ValueError) as error:
        raise ContractError(f"{case['id']} 图片不存在或逃逸语料目录") from error
    if not stat.S_ISREG(metadata.st_mode):
        raise ContractError(f"{case['id']} 图片必须是普通文件")
    if metadata.st_size < 57 or metadata.st_size > MAX_PNG_BYTES:
        raise ContractError(f"{case['id']} 图片大小超出 PNG 预算")
    try:
        payload = image_path.read_bytes()
    except OSError as error:
        raise ContractError(f"{case['id']} 图片读取失败") from error
    if file_sha256(payload) != source["sha256"]:
        raise ContractError(f"{case['id']} 图片 SHA-256 不符")
    if png_dimensions(payload) != (source["width"], source["height"]):
        raise ContractError(f"{case['id']} 图片尺寸不符")
    return payload


def validate_corpus_assets(corpus_path: Path, corpus: dict[str, Any]) -> None:
    for case in corpus["cases"]:
        read_case_png(corpus_path, case)


def levenshtein_distance(left: str, right: str) -> int:
    """Myers bit-vector Levenshtein；按 Unicode code point 计算。"""
    if len(left) > len(right):
        left, right = right, left
    width = len(left)
    if width == 0:
        return len(right)

    masks: dict[str, int] = {}
    for index, character in enumerate(left):
        masks[character] = masks.get(character, 0) | (1 << index)
    all_bits = (1 << width) - 1
    positive = all_bits
    negative = 0
    score = width
    last = 1 << (width - 1)
    for character in right:
        equal = masks.get(character, 0)
        combined = equal | negative
        horizontal = (((equal & positive) + positive) ^ positive) | equal
        positive_horizontal = negative | ~(horizontal | positive)
        negative_horizontal = positive & horizontal
        if positive_horizontal & last:
            score += 1
        elif negative_horizontal & last:
            score -= 1
        positive_horizontal = ((positive_horizontal << 1) | 1) & all_bits
        negative_horizontal = (negative_horizontal << 1) & all_bits
        positive = (negative_horizontal | ~(combined | positive_horizontal)) & all_bits
        negative = positive_horizontal & combined
    return score


def convex_hull(points: Iterable[Iterable[float]]) -> list[tuple[float, float]]:
    unique = sorted({(float(point[0]), float(point[1])) for point in points})
    if len(unique) <= 1:
        return unique

    def cross(
        origin: tuple[float, float],
        first: tuple[float, float],
        second: tuple[float, float],
    ) -> float:
        return (first[0] - origin[0]) * (second[1] - origin[1]) - (
            first[1] - origin[1]
        ) * (second[0] - origin[0])

    lower: list[tuple[float, float]] = []
    for point in unique:
        while len(lower) >= 2 and cross(lower[-2], lower[-1], point) <= EPSILON:
            lower.pop()
        lower.append(point)
    upper: list[tuple[float, float]] = []
    for point in reversed(unique):
        while len(upper) >= 2 and cross(upper[-2], upper[-1], point) <= EPSILON:
            upper.pop()
        upper.append(point)
    return lower[:-1] + upper[:-1]


def polygon_area(points: Iterable[Iterable[float]]) -> float:
    polygon = [(float(point[0]), float(point[1])) for point in points]
    if len(polygon) < 3:
        return 0.0
    return abs(
        sum(
            polygon[index][0] * polygon[(index + 1) % len(polygon)][1]
            - polygon[(index + 1) % len(polygon)][0] * polygon[index][1]
            for index in range(len(polygon))
        )
    ) / 2.0


def _signed_area(points: list[tuple[float, float]]) -> float:
    return sum(
        points[index][0] * points[(index + 1) % len(points)][1]
        - points[(index + 1) % len(points)][0] * points[index][1]
        for index in range(len(points))
    ) / 2.0


def _line_intersection(
    start: tuple[float, float],
    end: tuple[float, float],
    clip_start: tuple[float, float],
    clip_end: tuple[float, float],
) -> tuple[float, float]:
    dx1, dy1 = end[0] - start[0], end[1] - start[1]
    dx2, dy2 = clip_end[0] - clip_start[0], clip_end[1] - clip_start[1]
    denominator = dx1 * dy2 - dy1 * dx2
    if abs(denominator) <= EPSILON:
        return end
    offset_x, offset_y = clip_start[0] - start[0], clip_start[1] - start[1]
    distance = (offset_x * dy2 - offset_y * dx2) / denominator
    return start[0] + distance * dx1, start[1] + distance * dy1


def _polygon_intersection(
    subject: list[tuple[float, float]], clip: list[tuple[float, float]]
) -> list[tuple[float, float]]:
    if _signed_area(clip) < 0:
        clip = list(reversed(clip))
    output = subject
    for index, clip_start in enumerate(clip):
        clip_end = clip[(index + 1) % len(clip)]
        source = output
        output = []
        if not source:
            break

        def inside(point: tuple[float, float]) -> bool:
            return (
                (clip_end[0] - clip_start[0]) * (point[1] - clip_start[1])
                - (clip_end[1] - clip_start[1]) * (point[0] - clip_start[0])
                >= -EPSILON
            )

        previous = source[-1]
        for current in source:
            current_inside = inside(current)
            previous_inside = inside(previous)
            if current_inside:
                if not previous_inside:
                    output.append(
                        _line_intersection(previous, current, clip_start, clip_end)
                    )
                output.append(current)
            elif previous_inside:
                output.append(_line_intersection(previous, current, clip_start, clip_end))
            previous = current
    return output


def quad_iou(left: list[list[float]], right: list[list[float]]) -> float:
    left_hull = convex_hull(left)
    right_hull = convex_hull(right)
    left_area = polygon_area(left_hull)
    right_area = polygon_area(right_hull)
    intersection = polygon_area(_polygon_intersection(left_hull, right_hull))
    union = left_area + right_area - intersection
    return intersection / union if union > EPSILON else 0.0


def match_quads(
    expected: list[dict[str, Any]],
    predicted: list[dict[str, Any]],
    threshold: float,
) -> list[tuple[int, int, float]]:
    """求阈值图的最大基数匹配；候选按 IoU 降序保持确定性。"""
    candidates: list[list[tuple[int, float]]] = []
    for expected_line in expected:
        row = [
            (index, quad_iou(expected_line["quad"], predicted_line["quad"]))
            for index, predicted_line in enumerate(predicted)
        ]
        candidates.append(
            sorted(
                ((index, iou) for index, iou in row if iou + EPSILON >= threshold),
                key=lambda item: (-item[1], item[0]),
            )
        )

    prediction_owner: dict[int, int] = {}

    def augment(expected_index: int, visited: set[int]) -> bool:
        for predicted_index, _ in candidates[expected_index]:
            if predicted_index in visited:
                continue
            visited.add(predicted_index)
            owner = prediction_owner.get(predicted_index)
            if owner is None or augment(owner, visited):
                prediction_owner[predicted_index] = expected_index
                return True
        return False

    order = sorted(range(len(expected)), key=lambda index: (len(candidates[index]), index))
    for expected_index in order:
        augment(expected_index, set())

    result = []
    for predicted_index, expected_index in prediction_owner.items():
        iou = next(
            iou for candidate, iou in candidates[expected_index] if candidate == predicted_index
        )
        result.append((expected_index, predicted_index, iou))
    return sorted(result)


def whitespace_counts(expected: str, predicted: str) -> tuple[int, int, int]:
    """基于确定性文本对齐统计精确 Unicode 空白 TP/FP/FN。"""
    true_positive = false_positive = false_negative = 0
    matcher = SequenceMatcher(None, expected, predicted, autojunk=False)
    for operation, left_start, left_end, right_start, right_end in matcher.get_opcodes():
        left = expected[left_start:left_end]
        right = predicted[right_start:right_end]
        if operation == "equal":
            true_positive += sum(character.isspace() for character in left)
            continue
        left_spaces = Counter(character for character in left if character.isspace())
        right_spaces = Counter(character for character in right if character.isspace())
        matched = sum((left_spaces & right_spaces).values())
        true_positive += matched
        false_negative += sum(left_spaces.values()) - matched
        false_positive += sum(right_spaces.values()) - matched
    return true_positive, false_positive, false_negative


def inversion_counts(sequence: list[int]) -> tuple[int, int]:
    inversions = sum(
        sequence[left] > sequence[right]
        for left in range(len(sequence))
        for right in range(left + 1, len(sequence))
    )
    return inversions, len(sequence) * (len(sequence) - 1) // 2


def _ratio(numerator: int | float, denominator: int | float) -> float:
    return float(numerator) / float(denominator) if denominator else 1.0


def _f1(precision: float, recall: float) -> float:
    return 2 * precision * recall / (precision + recall) if precision + recall else 0.0


def _percentile(values: list[float], percentile: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, math.ceil(percentile * len(ordered)) - 1)
    return ordered[index]


def _empty_counts() -> dict[str, Any]:
    return {
        "caseCount": 0,
        "exactCases": 0,
        "expectedLines": 0,
        "predictedLines": 0,
        "matchedLines": 0,
        "iouTotal": 0.0,
        "exactMatchedLines": 0,
        "rawExpected": 0,
        "rawPredicted": 0,
        "rawEdits": 0,
        "compactExpected": 0,
        "compactPredicted": 0,
        "compactEdits": 0,
        "whitespaceTp": 0,
        "whitespaceFp": 0,
        "whitespaceFn": 0,
        "orderInversions": 0,
        "orderPairs": 0,
        "formulaExpected": 0,
        "formulaSupported": 0,
        "formulaExact": 0,
        "durations": [],
        "peakMemory": [],
    }


def _add_case(
    counts: dict[str, Any],
    expected_case: dict[str, Any],
    predicted_case: dict[str, Any],
    threshold: float,
    capabilities: dict[str, bool],
) -> None:
    expected = expected_case["lines"]
    predicted = predicted_case["lines"]
    matches = (
        match_quads(expected, predicted, threshold)
        if capabilities["lineGeometry"]
        else []
    )
    matched_expected = {left for left, _, _ in matches}
    matched_predicted = {right for _, right, _ in matches}

    expected_text = "\n".join(line["text"] for line in expected)
    predicted_text = predicted_case["text"]
    compact_expected = "".join(character for character in expected_text if not character.isspace())
    compact_predicted = "".join(character for character in predicted_text if not character.isspace())

    counts["caseCount"] += 1
    counts["exactCases"] += expected_text == predicted_text
    counts["expectedLines"] += len(expected)
    counts["predictedLines"] += len(predicted)
    counts["matchedLines"] += len(matches)
    counts["iouTotal"] += sum(iou for _, _, iou in matches)
    counts["rawExpected"] += len(expected_text)
    counts["rawPredicted"] += len(predicted_text)
    counts["rawEdits"] += levenshtein_distance(expected_text, predicted_text)
    counts["compactExpected"] += len(compact_expected)
    counts["compactPredicted"] += len(compact_predicted)
    counts["compactEdits"] += levenshtein_distance(compact_expected, compact_predicted)

    counts["formulaExpected"] += sum(line["kind"] == "formula" for line in expected)
    if capabilities["lineGeometry"]:
        for expected_index, predicted_index, _ in matches:
            expected_line = expected[expected_index]
            predicted_line = predicted[predicted_index]
            counts["exactMatchedLines"] += expected_line["text"] == predicted_line["text"]
            tp, fp, fn = whitespace_counts(expected_line["text"], predicted_line["text"])
            counts["whitespaceTp"] += tp
            counts["whitespaceFp"] += fp
            counts["whitespaceFn"] += fn
            if (
                capabilities["structuredFormula"]
                and expected_line["kind"] == "formula"
                and "formula" in predicted_line
            ):
                counts["formulaSupported"] += 1
                counts["formulaExact"] += (
                    predicted_line["formula"] == expected_line["formula"]
                )

        for expected_index, line in enumerate(expected):
            if expected_index not in matched_expected:
                counts["whitespaceFn"] += sum(
                    character.isspace() for character in line["text"]
                )
        for predicted_index, line in enumerate(predicted):
            if predicted_index not in matched_predicted:
                counts["whitespaceFp"] += sum(
                    character.isspace() for character in line["text"]
                )
    else:
        tp, fp, fn = whitespace_counts(expected_text, predicted_text)
        counts["whitespaceTp"] += tp
        counts["whitespaceFp"] += fp
        counts["whitespaceFn"] += fn

    if capabilities["lineGeometry"]:
        expected_by_predicted_order = [
            expected_index
            for expected_index, _, _ in sorted(matches, key=lambda item: item[1])
        ]
        inversions, pairs = inversion_counts(expected_by_predicted_order)
        counts["orderInversions"] += inversions
        counts["orderPairs"] += pairs
    if "durationMs" in predicted_case:
        counts["durations"].append(predicted_case["durationMs"])
    if "peakMemoryBytes" in predicted_case:
        counts["peakMemory"].append(predicted_case["peakMemoryBytes"])


def _finalize(
    counts: dict[str, Any], threshold: float, capabilities: dict[str, bool]
) -> dict[str, Any]:
    detection_precision = _ratio(counts["matchedLines"], counts["predictedLines"])
    detection_recall = _ratio(counts["matchedLines"], counts["expectedLines"])
    whitespace_precision = _ratio(
        counts["whitespaceTp"], counts["whitespaceTp"] + counts["whitespaceFp"]
    )
    whitespace_recall = _ratio(
        counts["whitespaceTp"], counts["whitespaceTp"] + counts["whitespaceFn"]
    )
    durations = counts["durations"]
    peak_memory = counts["peakMemory"]
    detection = (
        {
            "supported": True,
            "iouThreshold": threshold,
            "truePositive": counts["matchedLines"],
            "falsePositive": counts["predictedLines"] - counts["matchedLines"],
            "falseNegative": counts["expectedLines"] - counts["matchedLines"],
            "precision": detection_precision,
            "recall": detection_recall,
            "hmean": _f1(detection_precision, detection_recall),
            "meanMatchedIou": _ratio(counts["iouTotal"], counts["matchedLines"]),
        }
        if capabilities["lineGeometry"]
        else {"supported": False, "iouThreshold": threshold}
    )
    return {
        "caseCount": counts["caseCount"],
        "detection": detection,
        "text": {
            "rawExpectedCodepoints": counts["rawExpected"],
            "rawPredictedCodepoints": counts["rawPredicted"],
            "rawEditDistance": counts["rawEdits"],
            "rawCer": _ratio(counts["rawEdits"], counts["rawExpected"]),
            "nonWhitespaceExpectedCodepoints": counts["compactExpected"],
            "nonWhitespacePredictedCodepoints": counts["compactPredicted"],
            "nonWhitespaceEditDistance": counts["compactEdits"],
            "nonWhitespaceCer": _ratio(
                counts["compactEdits"], counts["compactExpected"]
            ),
            "exactCaseRate": _ratio(counts["exactCases"], counts["caseCount"]),
            "exactLineRecall": _ratio(
                counts["exactMatchedLines"], counts["expectedLines"]
            )
            if capabilities["lineGeometry"]
            else None,
            "exactLinePrecision": _ratio(
                counts["exactMatchedLines"], counts["predictedLines"]
            )
            if capabilities["lineGeometry"]
            else None,
        },
        "whitespace": {
            "truePositive": counts["whitespaceTp"],
            "falsePositive": counts["whitespaceFp"],
            "falseNegative": counts["whitespaceFn"],
            "precision": whitespace_precision,
            "recall": whitespace_recall,
            "f1": _f1(whitespace_precision, whitespace_recall),
        },
        "readingOrder": {
            "supported": capabilities["lineGeometry"],
            "inversions": counts["orderInversions"],
            "comparablePairs": counts["orderPairs"],
            "errorRate": _ratio(counts["orderInversions"], counts["orderPairs"])
            if counts["orderPairs"]
            else (0.0 if capabilities["lineGeometry"] else None),
        },
        "formula": {
            "supportedByEngine": capabilities["structuredFormula"],
            "expected": counts["formulaExpected"],
            "structuredPredictions": counts["formulaSupported"]
            if capabilities["structuredFormula"]
            else None,
            "exact": counts["formulaExact"]
            if capabilities["structuredFormula"]
            else None,
            "supportRate": _ratio(counts["formulaSupported"], counts["formulaExpected"])
            if capabilities["structuredFormula"]
            else None,
            "exactRate": _ratio(counts["formulaExact"], counts["formulaExpected"])
            if capabilities["structuredFormula"]
            else None,
        },
        "performance": {
            "measuredCases": len(durations),
            "durationMsTotal": sum(durations),
            "durationMsMedian": statistics.median(durations) if durations else None,
            "durationMsP95": _percentile(durations, 0.95),
            "peakMemoryBytesMax": max(peak_memory) if peak_memory else None,
        },
    }


def evaluate(
    corpus: dict[str, Any],
    predictions: dict[str, Any],
    *,
    iou_threshold: float = DEFAULT_IOU_THRESHOLD,
) -> dict[str, Any]:
    if not 0 < iou_threshold <= 1:
        raise ContractError("IoU threshold 必须位于 (0, 1]")
    clean_corpus = validate_corpus(corpus)
    case_ids = {case["id"] for case in clean_corpus["cases"]}
    clean_predictions = validate_predictions(predictions, case_ids)
    predictions_by_id = {case["id"]: case for case in clean_predictions["cases"]}
    capabilities = clean_predictions["engine"]["capabilities"]

    total = _empty_counts()
    tags: dict[str, dict[str, Any]] = {}
    for expected_case in clean_corpus["cases"]:
        predicted_case = predictions_by_id.get(
            expected_case["id"], {"id": expected_case["id"], "text": "", "lines": []}
        )
        _add_case(total, expected_case, predicted_case, iou_threshold, capabilities)
        for tag in expected_case["tags"]:
            counts = tags.setdefault(tag, _empty_counts())
            _add_case(counts, expected_case, predicted_case, iou_threshold, capabilities)

    return {
        "schema": REPORT_SCHEMA,
        "engine": clean_predictions["engine"],
        "summary": _finalize(total, iou_threshold, capabilities),
        "byTag": {
            tag: _finalize(counts, iou_threshold, capabilities)
            for tag, counts in sorted(tags.items())
        },
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="评测 Clippy OCR 结构化输出")
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--predictions", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--iou-threshold", type=float, default=DEFAULT_IOU_THRESHOLD)
    arguments = parser.parse_args(argv)
    try:
        raw_corpus = load_json(arguments.corpus)
        clean_corpus = validate_corpus(raw_corpus)
        validate_corpus_assets(arguments.corpus, clean_corpus)
        report = evaluate(
            clean_corpus,
            load_json(arguments.predictions),
            iou_threshold=arguments.iou_threshold,
        )
    except (OSError, json.JSONDecodeError, ContractError) as error:
        print(f"ocr-quality: {error}", file=sys.stderr)
        return 2
    encoded = json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
    if arguments.output:
        arguments.output.write_text(encoded, encoding="utf-8")
    else:
        sys.stdout.write(encoded)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
