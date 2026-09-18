import json
from pathlib import Path
import tempfile
import unittest

import quality_metrics as quality
import quality_table as table_quality


def quad(left, top, right, bottom):
    return [[left, top], [right, top], [right, bottom], [left, bottom]]


def cell(identifier, text, column, bounds):
    return {"id": identifier, "text": text, "columnIndex": column, "quad": bounds}


def corpus():
    return {
        "schema": table_quality.TABLE_CORPUS_SCHEMA,
        "cases": [
            {
                "id": "table",
                "tags": ["table", "numbers"],
                "source": {
                    "kind": "synthetic",
                    "license": "CC0-1.0",
                    "imagePath": "images/table.png",
                    "sha256": "0" * 64,
                    "width": 100,
                    "height": 40,
                },
                "tables": [
                    {
                        "id": "main",
                        "rows": [
                            {
                                "id": "r1",
                                "cells": [
                                    cell("a", "A", 0, quad(0, 0, 20, 10)),
                                    cell("b", "B", 1, quad(30, 0, 50, 10)),
                                    cell("c", "C", 2, quad(60, 0, 80, 10)),
                                ],
                            },
                            {
                                "id": "r2",
                                "cells": [
                                    cell("d", "D", 0, quad(0, 20, 20, 30)),
                                    cell("e", "E", 1, quad(30, 20, 50, 30)),
                                    cell("f", "F", 2, quad(60, 20, 80, 30)),
                                ],
                            },
                        ],
                    }
                ],
            }
        ],
    }


def predictions(lines, text):
    return {
        "schema": quality.PREDICTION_SCHEMA,
        "engine": {
            "id": "fixture",
            "version": "1",
            "capabilities": {"lineGeometry": True, "structuredFormula": False},
        },
        "cases": [{"id": "table", "text": text, "lines": lines}],
    }


def predicted(text, bounds):
    return {"text": text, "quad": bounds, "kind": "text"}


class TableQualityTests(unittest.TestCase):
    def test_row_and_cell_granularity_are_scored_separately(self):
        rows = predictions(
            [
                predicted("A B C", quad(0, 0, 80, 10)),
                predicted("D E F", quad(0, 20, 80, 30)),
            ],
            "A B C\nD E F",
        )
        summary = table_quality.evaluate(corpus(), rows)["summary"]
        self.assertEqual(summary["rowDetection"]["hmean"], 1)
        self.assertEqual(summary["cellDetection"]["hmean"], 0)
        self.assertEqual(summary["text"]["geometryNonWhitespaceCer"], 0)
        self.assertEqual(summary["text"]["exactGeometryRowRecall"], 1)

        cells = predictions(
            [
                predicted("A", quad(0, 0, 20, 10)),
                predicted("B", quad(30, 0, 50, 10)),
                predicted("C", quad(60, 0, 80, 10)),
                predicted("D", quad(0, 20, 20, 30)),
                predicted("E", quad(30, 20, 50, 30)),
                predicted("F", quad(60, 20, 80, 30)),
            ],
            "ABC\nDEF",
        )
        summary = table_quality.evaluate(corpus(), cells)["summary"]
        self.assertEqual(summary["rowDetection"]["hmean"], 1)
        self.assertEqual(summary["cellDetection"]["hmean"], 1)

    def test_raw_order_failure_is_visible_and_geometry_reconstruction_is_independent(self):
        shuffled = predictions(
            [
                predicted("A", quad(0, 0, 20, 10)),
                predicted("B", quad(30, 0, 50, 10)),
                predicted("C", quad(60, 0, 80, 10)),
                predicted("F", quad(60, 20, 80, 30)),
                predicted("D", quad(0, 20, 20, 30)),
                predicted("E", quad(30, 20, 50, 30)),
            ],
            "ABC\nFDE",
        )
        summary = table_quality.evaluate(corpus(), shuffled)["summary"]
        self.assertEqual(summary["rawReadingOrder"]["inversions"], 2)
        self.assertGreater(summary["text"]["rawNonWhitespaceCer"], 0)
        self.assertEqual(summary["text"]["geometryNonWhitespaceCer"], 0)
        self.assertEqual(summary["text"]["exactGeometryRowRecall"], 1)

    def test_contract_rejects_duplicate_columns_unknown_fields_and_outside_cells(self):
        duplicate_column = corpus()
        duplicate_column["cases"][0]["tables"][0]["rows"][0]["cells"][1]["columnIndex"] = 0
        with self.assertRaisesRegex(quality.ContractError, "严格递增"):
            table_quality.validate_table_corpus(duplicate_column)

        unknown = corpus()
        unknown["cases"][0]["tables"][0]["rows"][0]["cells"][0]["span"] = 2
        with self.assertRaisesRegex(quality.ContractError, "只允许"):
            table_quality.validate_table_corpus(unknown)

        outside = corpus()
        outside["cases"][0]["tables"][0]["rows"][0]["cells"][0]["quad"][1][0] = 101
        with self.assertRaisesRegex(quality.ContractError, "原图范围"):
            table_quality.validate_table_corpus(outside)

    def test_full_prediction_file_can_contain_non_table_cases_and_cli_is_create_only(self):
        value = predictions(
            [predicted("ABC", quad(0, 0, 80, 10)), predicted("DEF", quad(0, 20, 80, 30))],
            "ABC\nDEF",
        )
        value["cases"].append({"id": "other", "text": "other", "lines": []})
        self.assertEqual(table_quality.evaluate(corpus(), value)["summary"]["caseCount"], 1)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            corpus_path = root / "corpus.json"
            predictions_path = root / "predictions.json"
            output_path = root / "report.json"
            corpus_path.write_text(json.dumps(corpus()), encoding="utf-8")
            predictions_path.write_text(json.dumps(value), encoding="utf-8")
            self.assertEqual(
                table_quality.main(
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
            self.assertEqual(json.loads(output_path.read_text())["schema"], table_quality.TABLE_REPORT_SCHEMA)
            with self.assertRaisesRegex(quality.ContractError, "不会覆盖"):
                table_quality.main(
                    [
                        "--corpus",
                        str(corpus_path),
                        "--predictions",
                        str(predictions_path),
                        "--output",
                        str(output_path),
                    ]
                )


if __name__ == "__main__":
    unittest.main()
