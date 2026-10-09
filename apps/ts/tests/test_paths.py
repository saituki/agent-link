"""置き場と名前の検査（AC3）。"""

import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_ts import paths  # noqa: E402


class TempProject:
    """setUp で一時フォルダを作って SAI_PROJECT_ROOT に入れ、tearDown で戻して消す。"""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        self._saved = {k: os.environ.pop(k, None) for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN")}
        os.environ["SAI_PROJECT_ROOT"] = str(self.tmp)

    def tearDown(self):
        for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN"):
            os.environ.pop(k, None)
            if self._saved[k] is not None:
                os.environ[k] = self._saved[k]
        shutil.rmtree(self.tmp, ignore_errors=True)


class NameTest(unittest.TestCase):
    def test_ok(self):
        for n in ["r01", "a.b-c_d", "A", "x" * 64]:
            self.assertEqual(paths.check_name(n), n)

    def test_ng(self):
        for n in ["", "..", "a..b", "a/b", "x" * 65, "-a", ".a", "a b", "a\n"]:
            with self.assertRaises(ValueError, msg=repr(n)):
                paths.check_name(n)


class ProjectTest(TempProject, unittest.TestCase):
    def setUp(self):
        super().setUp()
        self.cwd = os.getcwd()

    def tearDown(self):
        os.chdir(self.cwd)
        super().tearDown()

    def test_env_wins(self):
        self.assertEqual(paths.project_root(), self.tmp)

    def test_cwd_walks_up_to_dot_sai(self):
        del os.environ["SAI_PROJECT_ROOT"]
        (self.tmp / ".sai").mkdir()
        (self.tmp / "a" / "b").mkdir(parents=True)
        os.chdir(self.tmp / "a" / "b")
        self.assertEqual(paths.project_root(), self.tmp)

    def test_not_found(self):
        del os.environ["SAI_PROJECT_ROOT"]
        os.chdir(self.tmp)
        with self.assertRaises(paths.ProjectNotFound):
            paths.project_root()

    def test_inside(self):
        (self.tmp / "ts").mkdir()
        self.assertEqual(paths.inside(self.tmp / "ts" / "x"), (self.tmp / "ts" / "x").resolve())
        with self.assertRaises(ValueError):
            paths.inside(self.tmp / "ts" / ".." / "x")

    def test_dataset_and_record_lookup(self):
        (self.tmp / "ts" / "datasets" / "demo").mkdir(parents=True)
        with self.assertRaises(KeyError):
            paths.dataset_dir("nope")
        with self.assertRaises(KeyError):
            paths.record_dir("demo", "r01")
        with self.assertRaises(ValueError):
            paths.dataset_dir("..")


if __name__ == "__main__":
    unittest.main()
