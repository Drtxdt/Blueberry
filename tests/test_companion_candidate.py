"""Original-package digest and Windows path checks before candidate extraction."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
import zipfile

SPEC=importlib.util.spec_from_file_location('candidate',Path(__file__).parents[1]/'experiments/wt-overlay/prepare-candidate.py')
C=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(C)


class CandidateTests(unittest.TestCase):
    def archive(self, root, names):
        path=root/'package.zip'
        with zipfile.ZipFile(path,'w') as z:
            for name in names: z.writestr(name,b'fixture')
        return path,hashlib.sha256(path.read_bytes()).hexdigest()

    def test_valid_layout_retains_hidden_portable_marker(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            path,digest=self.archive(root,['bin/WindowsTerminal.exe','bin/.portable'])
            C.extract(path,digest,root/'out')
            self.assertEqual((root/'out/bin/.portable').read_bytes(),b'fixture')

    def test_sha_mismatch_cannot_create_destination(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            path,_=self.archive(root,['blueberry.exe'])
            with self.assertRaisesRegex(AssertionError,'SHA-256 mismatch'): C.extract(path,'0'*64,root/'out')
            self.assertFalse((root/'out').exists())

    def test_windows_aliases_traversal_and_duplicates_are_rejected(self):
        for names in (['../outside'],['a/../outside'],['a/./b'],['a//b'],['C:/outside'],
                      ['../outside '],['a/.. /b'],['CON.txt'],['a/LPT1.bin'],['a/file.'],['A.exe','a.exe']):
            with self.subTest(names=names),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp)
                path,digest=self.archive(root,names)
                with self.assertRaises(AssertionError): C.extract(path,digest,root/'out')
                self.assertFalse((root/'out').exists())


if __name__=='__main__': unittest.main()
