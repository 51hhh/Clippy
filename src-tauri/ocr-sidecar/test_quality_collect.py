import hashlib
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib

import quality_collect as collect
import quality_metrics as quality
import quality_table


def minimal_png(width, height):
    def chunk(kind, data):
        checksum = zlib.crc32(kind + data) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", checksum)

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    rows = b"".join(b"\x00" + b"\xff\xff\xff" * width for _ in range(height))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(
        b"IDAT", zlib.compress(rows)
    ) + chunk(b"IEND", b"")


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

        for capture in (
            fixtures / "browser-ui-v1" / "capture.py",
            fixtures / "formula-browser-v1" / "capture.py",
        ):
            result = subprocess.run(
                [sys.executable, str(capture), "--verify"],
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                timeout=10,
            )
            self.assertEqual(result.returncode, 0, result.stdout.decode("utf-8", errors="replace"))

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

    def test_case_image_rejects_symlink_missing_non_png_and_wrong_dimensions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = minimal_png(20, 10)
            target = root / "target.png"
            target.write_bytes(payload)
            case = {
                "id": "case",
                "source": {
                    "imagePath": "target.png",
                    "sha256": hashlib.sha256(payload).hexdigest(),
                    "width": 20,
                    "height": 10,
                },
            }

            case["source"]["width"] = 21
            with self.assertRaisesRegex(quality.ContractError, "尺寸"):
                collect.read_case_png(root / "corpus.json", case)
            case["source"]["width"] = 20

            link = root / "link.png"
            link.symlink_to(target.name)
            case["source"]["imagePath"] = link.name
            with self.assertRaisesRegex(quality.ContractError, "符号链接"):
                collect.read_case_png(root / "corpus.json", case)

            case["source"]["imagePath"] = "missing.png"
            with self.assertRaisesRegex(quality.ContractError, "不存在"):
                collect.read_case_png(root / "corpus.json", case)

            invalid = b"not-a-png"
            (root / "invalid.png").write_bytes(invalid)
            case["source"].update(
                imagePath="invalid.png", sha256=hashlib.sha256(invalid).hexdigest()
            )
            with self.assertRaisesRegex(quality.ContractError, "PNG 预算"):
                collect.read_case_png(root / "corpus.json", case)

            corrupt = bytearray(payload)
            corrupt[24] ^= 1
            corrupt_payload = bytes(corrupt)
            (root / "corrupt.png").write_bytes(corrupt_payload)
            case["source"].update(
                imagePath="corrupt.png",
                sha256=hashlib.sha256(corrupt_payload).hexdigest(),
            )
            with self.assertRaisesRegex(quality.ContractError, "CRC"):
                collect.read_case_png(root / "corpus.json", case)

            (root / "directory").mkdir()
            case["source"]["imagePath"] = "directory"
            with self.assertRaisesRegex(quality.ContractError, "普通文件"):
                collect.read_case_png(root / "corpus.json", case)

            nested = root / "nested"
            nested.mkdir()
            (root / "outside.png").write_bytes(payload)
            case["source"].update(
                imagePath="../outside.png",
                sha256=hashlib.sha256(payload).hexdigest(),
            )
            with self.assertRaisesRegex(quality.ContractError, "逃逸"):
                collect.read_case_png(nested / "corpus.json", case)

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
