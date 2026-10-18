import hashlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location(
    "original_editor", Path(__file__).resolve().parents[1] / "scripts/prepare-original-editor.py")
editor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(editor)


class OriginalEditorTests(unittest.TestCase):
    def test_corrupt_cache_cannot_replace_installed_module(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root / "PSReadLine/2.4.5/PSReadLine.psd1"
            manifest.parent.mkdir(parents=True)
            manifest.write_bytes(b"existing")
            (root / "2.4.5.nupkg").write_bytes(b"corrupt")
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                editor.prepare("2.4.5", root)
            self.assertEqual(manifest.read_bytes(), b"existing")

    def test_all_member_paths_checked_before_extraction(self):
        archive = io.BytesIO()
        with zipfile.ZipFile(archive, "w") as package:
            package.writestr("PSReadLine.psd1", "replacement")
            package.writestr("../outside", "invalid")
        data = archive.getvalue()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "2.4.5.nupkg").write_bytes(data)
            with patch.dict(editor.HASHES, {"2.4.5": hashlib.sha256(data).hexdigest()}):
                with self.assertRaisesRegex(ValueError, "Unsafe package member"):
                    editor.prepare("2.4.5", root)
            self.assertFalse((root / "PSReadLine/2.4.5/PSReadLine.psd1").exists())


if __name__ == "__main__":
    unittest.main()
