import copy
import itertools
import json
from pathlib import Path
import random
import tempfile
import unittest

import quality_metrics as quality


def quad(left, top, right, bottom):
    return [[left, top], [right, top], [right, bottom], [left, bottom]]


def line(identifier, text, bounds, *, kind="text", formula=None):
    value = {"id": identifier, "text": text, "quad": bounds, "kind": kind}
    if formula is not None:
        value["formula"] = formula
    return value


def predicted(text, bounds, *, kind="text", formula=None):
    value = {"text": text, "quad": bounds, "kind": kind}
    if formula is not None:
        value["formula"] = formula
    return value


def corpus():
    return {
        "schema": quality.CORPUS_SCHEMA,
        "cases": [
            {
                "id": "mixed-script-spacing-symbols",
                "tags": ["zh-en-ja", "spaces", "numbers", "symbols", "formula"],
                "source": {
                    "kind": "synthetic",
                    "license": "CC0-1.0",
                    "imagePath": "fixtures/mixed-script-spacing-symbols.png",
                    "sha256": "0" * 64,
                    "width": 300,
                    "height": 72,
                },
                "lines": [
                    line(
                        "l1",
                        "中文  OCR 日本語  12.50% ± √x",
                        quad(0, 0, 300, 16),
                    ),
                    line("l2", "AA  11", quad(0, 24, 90, 40)),
                    line(
                        "l3",
                        "x²+1/2",
                        quad(0, 48, 100, 72),
                        kind="formula",
                        formula={"format": "latex", "value": "x^2+\\frac{1}{2}"},
                    ),
                ],
            }
        ],
    }


def predictions():
    return {
        "schema": quality.PREDICTION_SCHEMA,
        "engine": {
            "id": "enhanced",
            "version": "PP-OCRv6 small / 中英日",
            "capabilities": {"lineGeometry": True, "structuredFormula": True},
        },
        "cases": [
            {
                "id": "mixed-script-spacing-symbols",
                "durationMs": 120.5,
                "peakMemoryBytes": 1_048_576,
                "text": "中文 OCR 日本語 12.50% ± √x\nx²+1/2\nAA  11\nextra",
                "lines": [
                    predicted(
                        "中文 OCR 日本語 12.50% ± √x",
                        quad(0, 0, 300, 16),
                    ),
                    predicted(
                        "x²+1/2",
                        quad(0, 48, 100, 72),
                        kind="formula",
                        formula={"format": "latex", "value": "x^2+\\frac{1}{2}"},
                    ),
                    predicted("AA  11", quad(0, 24, 90, 40)),
                    predicted("extra", quad(400, 0, 450, 16)),
                ],
            }
        ],
    }


def naive_distance(left, right):
    previous = list(range(len(right) + 1))
    for left_index, left_character in enumerate(left, 1):
        current = [left_index]
        for right_index, right_character in enumerate(right, 1):
            current.append(
                min(
                    current[-1] + 1,
                    previous[right_index] + 1,
                    previous[right_index - 1] + (left_character != right_character),
                )
            )
        previous = current
    return previous[-1]


class QualityMetricsTests(unittest.TestCase):
    def test_bit_vector_levenshtein_matches_reference_for_unicode(self):
        alphabet = ["", "a", "中", "日", " ", "±"]
        samples = alphabet + ["中文  OCR", "日本語", "AA  11", "x²+½"]
        randomizer = random.Random(0xC11F)
        samples.extend(
            "".join(randomizer.choice(alphabet[1:]) for _ in range(length))
            for length in range(1, 12)
            for _ in range(4)
        )
        for left, right in itertools.product(samples, repeat=2):
            self.assertEqual(
                quality.levenshtein_distance(left, right),
                naive_distance(left, right),
                (left, right),
            )

    def test_rotated_and_axis_aligned_quad_iou_is_geometric(self):
        self.assertAlmostEqual(
            quality.quad_iou(quad(0, 0, 10, 10), quad(5, 0, 15, 10)),
            1 / 3,
        )
        diamond = [[5, 0], [10, 5], [5, 10], [0, 5]]
        self.assertAlmostEqual(
            quality.quad_iou(diamond, quad(0, 0, 10, 10)),
            0.5,
        )
        self.assertEqual(
            quality.quad_iou(quad(0, 0, 10, 10), quad(20, 20, 30, 30)),
            0,
        )

    def test_report_keeps_detection_text_spaces_order_formula_and_resources_separate(self):
        report = quality.evaluate(corpus(), predictions())
        summary = report["summary"]

        self.assertEqual(report["schema"], quality.REPORT_SCHEMA)
        self.assertEqual(summary["detection"]["truePositive"], 3)
        self.assertEqual(summary["detection"]["falsePositive"], 1)
        self.assertEqual(summary["detection"]["falseNegative"], 0)
        self.assertEqual(summary["detection"]["precision"], 0.75)
        self.assertEqual(summary["detection"]["recall"], 1.0)
        self.assertEqual(summary["text"]["exactLineRecall"], 2 / 3)
        self.assertGreater(summary["text"]["rawCer"], 0)
        self.assertGreater(summary["whitespace"]["falseNegative"], 0)
        self.assertEqual(summary["readingOrder"]["inversions"], 1)
        self.assertEqual(summary["readingOrder"]["comparablePairs"], 3)
        self.assertEqual(summary["formula"]["expected"], 1)
        self.assertEqual(summary["formula"]["structuredPredictions"], 1)
        self.assertEqual(summary["formula"]["exact"], 1)
        self.assertEqual(summary["performance"]["durationMsP95"], 120.5)
        self.assertEqual(summary["performance"]["peakMemoryBytesMax"], 1_048_576)
        self.assertEqual(report["byTag"]["zh-en-ja"]["caseCount"], 1)

    def test_missing_prediction_counts_as_detection_and_text_failure(self):
        empty = {
            "schema": quality.PREDICTION_SCHEMA,
            "engine": {
                "id": "missing",
                "version": "1",
                "capabilities": {"lineGeometry": True, "structuredFormula": True},
            },
            "cases": [],
        }
        summary = quality.evaluate(corpus(), empty)["summary"]
        self.assertEqual(summary["detection"]["falseNegative"], 3)
        self.assertEqual(summary["detection"]["recall"], 0)
        self.assertEqual(summary["text"]["exactCaseRate"], 0)
        self.assertEqual(summary["formula"]["supportRate"], 0)
        self.assertGreater(summary["whitespace"]["falseNegative"], 0)

    def test_unstructured_tesseract_text_is_scored_without_fake_boxes(self):
        tesseract = {
            "schema": quality.PREDICTION_SCHEMA,
            "engine": {
                "id": "tesseract",
                "version": "5.5.0",
                "capabilities": {"lineGeometry": False, "structuredFormula": False},
            },
            "cases": [
                {
                    "id": "mixed-script-spacing-symbols",
                    "text": "中文 OCR 日本語 12.50% ± √x\nAA 11\nx²+1/2",
                    "lines": [],
                }
            ],
        }
        summary = quality.evaluate(corpus(), tesseract)["summary"]
        self.assertFalse(summary["detection"]["supported"])
        self.assertIsNone(summary["text"]["exactLineRecall"])
        self.assertFalse(summary["readingOrder"]["supported"])
        self.assertIsNone(summary["readingOrder"]["errorRate"])
        self.assertFalse(summary["formula"]["supportedByEngine"])
        self.assertIsNone(summary["formula"]["exactRate"])
        self.assertGreater(summary["text"]["rawCer"], 0)
        self.assertGreater(summary["whitespace"]["falseNegative"], 0)

    def test_contract_rejects_duplicate_ids_unknown_cases_and_formula_without_truth(self):
        duplicate = corpus()
        duplicate["cases"].append(copy.deepcopy(duplicate["cases"][0]))
        with self.assertRaisesRegex(quality.ContractError, "重复 case id"):
            quality.evaluate(duplicate, predictions())

        unknown = predictions()
        unknown["cases"][0]["id"] = "not-in-corpus"
        with self.assertRaisesRegex(quality.ContractError, "未知 case"):
            quality.evaluate(corpus(), unknown)

        missing_formula = corpus()
        del missing_formula["cases"][0]["lines"][2]["formula"]
        with self.assertRaisesRegex(quality.ContractError, "公式真值"):
            quality.evaluate(missing_formula, predictions())

        escaped_source = corpus()
        escaped_source["cases"][0]["source"]["imagePath"] = "../private.png"
        with self.assertRaisesRegex(quality.ContractError, "相对 POSIX 路径"):
            quality.evaluate(escaped_source, predictions())

        outside = corpus()
        outside["cases"][0]["lines"][0]["quad"][1][0] = 301
        with self.assertRaisesRegex(quality.ContractError, "原图范围"):
            quality.evaluate(outside, predictions())

    def test_cli_writes_deterministic_utf8_report(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            corpus_path = root / "corpus.json"
            predictions_path = root / "predictions.json"
            output_path = root / "report.json"
            corpus_path.write_text(
                json.dumps(corpus(), ensure_ascii=False), encoding="utf-8"
            )
            predictions_path.write_text(
                json.dumps(predictions(), ensure_ascii=False), encoding="utf-8"
            )
            self.assertEqual(
                quality.main(
                    [
                        "--corpus",
                        str(corpus_path),
                        "--predictions",
                        str(predictions_path),
                        "--output",
                        str(output_path),
                    ]
                ),
                0,
            )
            rendered = output_path.read_text(encoding="utf-8")
            self.assertIn("中英日", rendered)
            self.assertEqual(json.loads(rendered)["schema"], quality.REPORT_SCHEMA)


if __name__ == "__main__":
    unittest.main()
