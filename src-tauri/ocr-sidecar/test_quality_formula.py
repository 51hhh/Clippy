import copy
from pathlib import Path
import tempfile
import unittest

import formula_quality as formula
import formula_layout_probe as layout_probe
import quality_metrics


def corpus():
    return {
        "schema": quality_metrics.CORPUS_SCHEMA,
        "cases": [
            {
                "id": "basic",
                "tags": ["fraction"],
                "source": {
                    "kind": "test",
                    "license": "CC0-1.0",
                    "imagePath": "basic.png",
                    "sha256": "0" * 64,
                    "width": 100,
                    "height": 40,
                },
                "lines": [{
                    "id": "basic-formula",
                    "kind": "formula",
                    "text": "x² + 1/2",
                    "quad": [[0, 0], [100, 0], [100, 40], [0, 40]],
                    "formula": {"format": "latex", "value": "x^{2}+\\frac{1}{2}"},
                }],
            },
            {
                "id": "greek",
                "tags": ["greek"],
                "source": {
                    "kind": "test",
                    "license": "CC0-1.0",
                    "imagePath": "greek.png",
                    "sha256": "1" * 64,
                    "width": 100,
                    "height": 40,
                },
                "lines": [{
                    "id": "greek-formula",
                    "kind": "formula",
                    "text": "α+β",
                    "quad": [[0, 0], [100, 0], [100, 40], [0, 40]],
                    "formula": {"format": "latex", "value": "\\alpha+\\beta"},
                }],
            },
        ],
    }


def predictions():
    return {
        "schema": formula.PREDICTION_SCHEMA,
        "engine": {
            "id": "PP-FormulaNet-S",
            "version": "test",
            "runtime": "paddle test",
            "modelSha256": "a" * 64,
            "modelBytes": 123,
            "modelLoadMs": 456,
            "license": "Apache-2.0",
        },
        "cases": [
            {"id": "basic", "latex": "$$ x ^ { 2 } + \\frac {1}{2} $$", "latexParseable": True, "durationMs": 10, "peakMemoryBytes": 1000},
            {"id": "greek", "latex": "\\alpha+\\gamma", "latexParseable": False, "durationMs": 30, "peakMemoryBytes": 2000},
        ],
    }


class FormulaQualityTests(unittest.TestCase):
    def test_normalization_only_removes_delimiters_and_math_whitespace(self):
        self.assertEqual(
            formula.latex_tokens("x^{2}+\\frac{1}{2}"),
            formula.latex_tokens(" $$ x ^ { 2 } + \\frac { 1 } { 2 } $$ "),
        )
        self.assertNotEqual(formula.latex_tokens("\\frac{1}{2}"), formula.latex_tokens("\\dfrac{1}{2}"))
        self.assertNotEqual(formula.latex_tokens("x\\ y"), formula.latex_tokens("xy"))

    def test_report_separates_raw_normalized_and_token_error(self):
        report = formula.evaluate(corpus(), formula.validate_predictions(predictions(), {"basic", "greek"}))
        self.assertEqual(report["summary"]["rawExact"], 0)
        self.assertEqual(report["summary"]["normalizedExact"], 1)
        self.assertEqual(report["summary"]["tokenDistance"], 1)
        self.assertEqual(report["summary"]["latexParseableRate"], 0.5)
        self.assertEqual(report["summary"]["durationMsP50"], 20)
        self.assertEqual(report["summary"]["durationMsP95"], 29)
        self.assertEqual(report["summary"]["peakMemoryBytes"], 2000)

    def test_prediction_contract_rejects_unknown_duplicate_and_oversize(self):
        value = predictions()
        value["cases"][1]["id"] = "unknown"
        with self.assertRaises(quality_metrics.ContractError):
            formula.validate_predictions(value, {"basic", "greek"})
        value = predictions()
        value["cases"][1]["id"] = "basic"
        with self.assertRaises(quality_metrics.ContractError):
            formula.validate_predictions(value, {"basic", "greek"})
        value = predictions()
        value["cases"][0]["latex"] = "x" * (formula.MAX_LATEX_BYTES + 1)
        with self.assertRaises(quality_metrics.ContractError):
            formula.validate_predictions(value, {"basic", "greek"})

    def test_formula_corpus_is_fixed_and_asset_validated(self):
        root = Path(__file__).resolve().parent / "quality-fixtures" / "formula-browser-v1"
        clean = formula.validate_formula_corpus(root / "corpus.json")
        self.assertEqual(len(clean["cases"]), 6)
        self.assertTrue(all(case["lines"][0]["kind"] == "formula" for case in clean["cases"]))

    def test_cli_does_not_overwrite_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "report.json"
            output.write_text("kept", encoding="utf-8")
            with self.assertRaises(quality_metrics.ContractError):
                formula.write_new(output, {"schema": formula.REPORT_SCHEMA})
            self.assertEqual(output.read_text(encoding="utf-8"), "kept")

    def test_layout_probe_separates_recall_from_false_positives(self):
        positive = corpus()
        negative = {
            "schema": quality_metrics.CORPUS_SCHEMA,
            "cases": [{
                "id": "plain-ui",
                "tags": ["ui"],
                "source": positive["cases"][0]["source"],
                "lines": [{
                    "id": "plain-line",
                    "kind": "text",
                    "text": "Settings",
                    "quad": [[0, 0], [100, 0], [100, 40], [0, 40]],
                }],
            }],
        }
        raw = {
            "schema": layout_probe.SCHEMA,
            "engine": {"id": "layout"},
            "cases": [
                {
                    "id": "positive/basic",
                    "durationMs": 10,
                    "boxes": [{"label": "formula", "score": 0.9, "coordinate": [0, 0, 100, 40]}],
                },
                {"id": "positive/greek", "durationMs": 20, "boxes": []},
                {
                    "id": "negative/plain-ui",
                    "durationMs": 30,
                    "boxes": [{"label": "formula", "score": 0.8, "coordinate": [0, 0, 100, 40]}],
                },
            ],
        }
        report = layout_probe.evaluate(positive, negative, raw)
        self.assertEqual(report["summary"]["positiveRecall"], 0.5)
        self.assertEqual(report["summary"]["falsePositiveCases"], 1)
        self.assertEqual(report["summary"]["falsePositiveBoxes"], 1)


if __name__ == "__main__":
    unittest.main()
