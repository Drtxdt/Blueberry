import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


artifacts = load('materialize-release-artifacts')
guard = load('check-benchmark-artifacts')


class ArtifactTests(unittest.TestCase):
    def test_guard_rejects_new_archives_and_oversized_reports(self):
        self.assertTrue(guard.violation('docs/benchmarks/run.ZIP', 1))
        self.assertTrue(guard.violation('docs/benchmarks/run.json', guard.LIMIT + 1))
        self.assertFalse(guard.violation('docs/benchmarks/summary.json', guard.LIMIT))

    def extract(self, folder, names, wrong_member=False):
        root = Path(folder)
        archive = root / 'artifact.zip'
        with zipfile.ZipFile(archive, 'w') as output:
            for name in names:
                output.writestr(name, b'original evidence')
        descriptor = {'sha256': artifacts.digest(archive), 'files': {
            name: hashlib.sha256(b'wrong' if wrong_member else b'original evidence').hexdigest() for name in names}}
        destination = root / 'evidence'
        destination.mkdir(exist_ok=True)
        artifacts.extract_verified(archive, descriptor, destination)
        return destination, archive, descriptor

    def test_verified_members_can_be_materialized_idempotently(self):
        with tempfile.TemporaryDirectory() as folder:
            root, archive, descriptor = self.extract(folder, ['run/report.json'])
            artifacts.extract_verified(archive, descriptor, root)
            self.assertEqual((root / 'run/report.json').read_bytes(), b'original evidence')
            (root / 'run/report.json').write_bytes(b'other evidence')
            with self.assertRaisesRegex(ValueError, 'replace existing'):
                artifacts.extract_verified(archive, descriptor, root)

    def test_traversal_collision_and_corruption_fail_before_publication(self):
        for names, corrupt in [(['../escape.json'], False), (['C:/escape.json'], False),
                               (['run\\escape.json'], False), (['a.json', 'A.json'], False),
                               (['valid.json'], True)]:
            with self.subTest(names=names), tempfile.TemporaryDirectory() as folder:
                with self.assertRaises(ValueError):
                    self.extract(folder, names, corrupt)
                self.assertFalse(any((Path(folder) / 'evidence').rglob('*')))


if __name__ == '__main__':
    unittest.main()
