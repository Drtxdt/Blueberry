import importlib.util
import os
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("snapshot", Path(__file__).parents[1] / "scripts/snapshot-benchmark-path.py")
SNAPSHOT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SNAPSHOT)


class SnapshotTests(unittest.TestCase):
    def test_preserves_command_bytes_names_and_path_priority(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "volatile"
            normal = root / "normal"
            source.mkdir()
            normal.mkdir()
            (source / "tool.EXE").write_bytes(b"full binary")
            (source / "script.cmd").write_bytes(b"script bytes")
            (source / "unrelated.log").write_text("not a command")
            result = SNAPSHOT.freeze(root / "frozen", os.pathsep.join([str(normal), str(source)]),
                                     {".exe", ".cmd"}, lambda p: p == str(source))
            paths = result["path"].split(os.pathsep)
            self.assertEqual(paths[0], str(normal))
            mirror = Path(paths[1])
            self.assertEqual((mirror / "tool.EXE").read_bytes(), b"full binary")
            self.assertEqual((mirror / "script.cmd").read_bytes(), b"script bytes")
            self.assertFalse((mirror / "unrelated.log").exists())
            self.assertEqual(len(result["mappings"][0]["commands"]), 2)
            # Later source-directory churn cannot change the frozen workload.
            (source / "another.log").write_text("directory mtime changes")
            self.assertEqual(SNAPSHOT.sha(mirror / "tool.EXE"), result["mappings"][0]["commands"][1]["sha256"])

    def test_refuses_to_replace_existing_snapshot(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaises(FileExistsError):
                SNAPSHOT.freeze(root, "", {".exe"}, lambda p: False)

    def test_windows_root_churn_is_isolated_without_changing_command_set(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            windows = root / "Windows"
            system = windows / "System32"
            normal = root / "Git"
            system.mkdir(parents=True)
            normal.mkdir()
            (windows / "regedit.exe").write_bytes(b"root command")
            (system / "where.exe").write_bytes(b"system command")
            original = [str(normal), str(windows) + os.sep, str(system)]
            result = SNAPSHOT.freeze(root / "frozen", os.pathsep.join(original), {".exe"},
                                     lambda p: SNAPSHOT.volatile_directory(p, windows))
            paths = result["path"].split(os.pathsep)
            self.assertEqual(paths[0], str(normal))
            self.assertEqual([Path(p).name for p in paths[1:]], ["commands-1", "commands-2"])
            self.assertEqual([row["original"] for row in result["mappings"]], original[1:])
            (windows / "maintenance.log").write_text("unrelated directory change")
            self.assertEqual([p.name for p in Path(paths[1]).iterdir()], ["regedit.exe"])
            self.assertEqual((Path(paths[1]) / "regedit.exe").read_bytes(), b"root command")
            self.assertEqual((Path(paths[2]) / "where.exe").read_bytes(), b"system command")


if __name__ == "__main__":
    unittest.main()
