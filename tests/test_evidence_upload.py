"""Sealed local evidence must not gain files, change, or escape through links."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('upload', Path(__file__).resolve().parents[1] / 'scripts/verify-evidence-upload.py')
UPLOAD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(UPLOAD)


class EvidenceUploadTests(unittest.TestCase):
    def fixture(self, root):
        (root / 'raw').mkdir()
        sample = root / 'raw/samples.json'
        sample.write_text('{"samples":[1,2,100]}')
        manifest = root / 'upload.json'
        manifest.write_text(json.dumps({'schema': 1, 'files': {'raw/samples.json': UPLOAD.sha(sample)}}))
        return manifest, UPLOAD.sha(manifest)

    def test_accepts_exact_inventory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest, digest = self.fixture(root)
            self.assertEqual(UPLOAD.verify(root, manifest.name, digest), root.absolute())

    def test_rejects_tampered_file_or_manifest(self):
        for changed in ('raw/samples.json', 'upload.json'):
            with self.subTest(changed=changed), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                manifest, digest = self.fixture(root)
                (root / changed).write_text('{}')
                with self.assertRaises(ValueError):
                    UPLOAD.verify(root, manifest.name, digest)

    def test_rejects_added_and_missing_files(self):
        for added in (True, False):
            with self.subTest(added=added), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                manifest, digest = self.fixture(root)
                if added:
                    (root / 'unlisted.json').write_text('{}')
                else:
                    (root / 'raw/samples.json').unlink()
                with self.assertRaises(ValueError):
                    UPLOAD.verify(root, manifest.name, digest)

    def test_rejects_manifest_path_traversal(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest, digest = self.fixture(root)
            with self.assertRaises(ValueError):
                UPLOAD.verify(root, '../upload.json', digest)

    def test_rejects_linked_directory(self):
        with tempfile.TemporaryDirectory() as directory, tempfile.TemporaryDirectory() as outside:
            root = Path(directory)
            manifest, digest = self.fixture(root)
            try:
                (root / 'external').symlink_to(outside, target_is_directory=True)
            except OSError as error:
                self.skipTest('Symbolic links unavailable: ' + str(error))
            with self.assertRaises(ValueError):
                UPLOAD.verify(root, manifest.name, digest)


if __name__ == '__main__':
    unittest.main()
