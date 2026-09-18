import hashlib
from pathlib import Path
import sys
import tempfile
import unittest

import quality_collect as collect
import quality_metrics as quality
import quality_table


def minimal_png(width, height):
    # 采集器只读签名/IHDR 身份；像素解码由实际 OCR 引擎负责。
    return (
        b"\x89PNG\r\n\x1a\n"
        + (13).to_bytes(4, "big")
        + b"IHDR"
        + width.to_bytes(4, "big")
        + height.to_bytes(4, "big")
        + b"\x08\x02\x00\x00\x00"
        + b"\x00\x00\x00\x00"
    )


class QualityCollectTests(unittest.TestCase):
    def test_peak_memory_metric_names_supported_desktop_source(self):
        if sys.platform.startswith("linux"):
            self.assertEqual(collect.peak_memory_metric(), "linux-proc-vmhwm")
        elif sys.platform == "darwin":
            self.assertEqual(collect.peak_memory_metric(), "macos-ps-rss-sampled")
        elif sys.platform == "win32":
            self.assertEqual(collect.peak_memory_metric(), "windows-peak-working-set")

    def test_measured_process_records_peak_rss_without_changing_output(self):
        process, peak_memory = collect.run_measured(
            [
                sys.executable,
                "-c",
                "import sys,time; data=bytearray(8*1024*1024); "
                "sys.stdout.buffer.write(sys.stdin.buffer.read()); time.sleep(0.08)",
            ],
            b"measured-output",
            2,
        )
        self.assertEqual(process.returncode, 0)
        self.assertEqual(process.stdout, b"measured-output")
        if sys.platform.startswith("linux") or sys.platform == "darwin" or sys.platform == "win32":
            self.assertIsInstance(peak_memory, int)
            self.assertGreater(peak_memory, 0)

    def test_checked_in_corpora_and_png_identities_are_valid(self):
        fixtures = Path(__file__).resolve().parent / "quality-fixtures"
        corpora = sorted(fixtures.glob("*/corpus.json"))
        self.assertGreaterEqual(len(corpora), 2)
        for corpus_path in corpora:
            corpus = quality.validate_corpus(quality.load_json(corpus_path))
            for case in corpus["cases"]:
                self.assertTrue(collect.read_case_png(corpus_path, case))
        table_corpora = sorted(fixtures.glob("*/table-corpus.json"))
        self.assertGreaterEqual(len(table_corpora), 1)
        for corpus_path in table_corpora:
            corpus = quality_table.validate_table_corpus(quality.load_json(corpus_path))
            for case in corpus["cases"]:
                self.assertTrue(collect.read_case_png(corpus_path, case))

    def test_case_image_must_match_relative_path_hash_and_dimensions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = minimal_png(20, 10)
            (root / "image.png").write_bytes(payload)
            case = {
                "id": "case",
                "source": {
                    "imagePath": "image.png",
                    "sha256": hashlib.sha256(payload).hexdigest(),
                    "width": 20,
                    "height": 10,
                },
            }
            self.assertEqual(collect.read_case_png(root / "corpus.json", case), payload)

            case["source"]["sha256"] = "0" * 64
            with self.assertRaisesRegex(quality.ContractError, "SHA-256"):
                collect.read_case_png(root / "corpus.json", case)

    def test_enhanced_adapter_uses_explicit_reading_order_and_keeps_plain_formula(self):
        text, lines = collect.enhanced_prediction(
            {
                "text": "first\nsecond",
                "lines": [
                    {
                        "text": "second",
                        "quad": [[0, 20], [40, 20], [40, 30], [0, 30]],
                        "readingOrder": 1,
                    },
                    {
                        "text": "first",
                        "quad": [[0, 0], [30, 0], [30, 10], [0, 10]],
                        "readingOrder": 0,
                    },
                ],
            }
        )
        self.assertEqual(text, "first\nsecond")
        self.assertEqual([line["text"] for line in lines], ["first", "second"])
        self.assertTrue(all(line["kind"] == "text" for line in lines))
        self.assertTrue(all("formula" not in line for line in lines))

    def test_output_is_create_only(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "prediction.json"
            collect.write_new(path, {"value": "中英日"})
            self.assertIn("中英日", path.read_text(encoding="utf-8"))
            with self.assertRaisesRegex(quality.ContractError, "不会覆盖"):
                collect.write_new(path, {"value": "replacement"})

    def test_diagnostics_directory_is_private_and_create_only(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "diagnostics"
            self.assertEqual(collect.create_diagnostics_directory(path), path.resolve())
            self.assertTrue(path.is_dir())
            self.assertEqual(path.stat().st_mode & 0o777, 0o700)
            with self.assertRaises(FileExistsError):
                collect.create_diagnostics_directory(path)


if __name__ == "__main__":
    unittest.main()
