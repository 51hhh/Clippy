from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import private_diagnostics as private


class PrivateDiagnosticsTests(unittest.TestCase):
    def test_nul_path_is_rejected_without_creating_truncated_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory) / "diagnostics"
            with self.assertRaisesRegex(ValueError, "NUL"):
                private.create_private_directory(Path(str(prefix) + "\0suffix"))
            self.assertFalse(prefix.exists())

    def test_existing_directory_and_contents_are_never_replaced(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            marker = path / "existing.txt"
            marker.write_text("keep", encoding="utf-8")
            with self.assertRaises(FileExistsError):
                private.create_private_directory(path)
            self.assertEqual(marker.read_text(encoding="utf-8"), "keep")

    def test_missing_parent_is_not_created(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "missing" / "diagnostics"
            with self.assertRaises(FileNotFoundError):
                private.create_private_directory(path)
            self.assertFalse(path.parent.exists())

    @unittest.skipUnless(sys.platform == "win32", "Windows DACL 原生检查")
    def test_unicode_directory_has_only_current_user_protected_dacl(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "中文 诊断"
            self.assertEqual(private.create_private_directory(path), path.resolve())
            api = private._windows_api()
            sid = private._current_user_sid(api)
            self.assertEqual(private._read_windows_dacl(path, api), f"D:P(A;OICI;FA;;;{sid})")

    @unittest.skipUnless(sys.platform == "win32", "Windows DACL 失败关闭检查")
    def test_unexpected_dacl_removes_new_empty_directory_and_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "diagnostics"
            with patch.object(private, "_read_windows_dacl", return_value="D:(A;;FA;;;WD)"):
                with self.assertRaisesRegex(OSError, "DACL"):
                    private.create_private_directory(path)
            self.assertFalse(path.exists())

    @unittest.skipUnless(sys.platform == "win32", "Windows ACL 查询失败关闭检查")
    def test_dacl_query_error_does_not_leave_output_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "diagnostics"
            with patch.object(private, "_read_windows_dacl", side_effect=OSError("query failed")):
                with self.assertRaisesRegex(OSError, "query failed"):
                    private.create_private_directory(path)
            self.assertFalse(path.exists())


if __name__ == "__main__":
    unittest.main()
