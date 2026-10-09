"""init（AC5）と block ts-agent の形（AC6）。偽の sai-link で def write を確かめる。"""

import os
import re
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_ts import init  # noqa: E402

FAKE = str(Path(__file__).resolve().parent / "fake_sai_link.sh")
BLOCK = Path(__file__).resolve().parent.parent / "blocks" / "ts-agent.md"


class InitTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        (self.tmp / ".sai").mkdir()
        self.fake = self.tmp / "fake"
        self._saved = {k: os.environ.pop(k, None) for k in
                       ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN", "SAI_LINK_BIN", "FAKE_LINK_DIR")}
        os.environ.update(SAI_PROJECT_ROOT=str(self.tmp), SAI_LINK_BIN=FAKE, FAKE_LINK_DIR=str(self.fake))

    def tearDown(self):
        for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN", "SAI_LINK_BIN", "FAKE_LINK_DIR"):
            os.environ.pop(k, None)
        for k, v in self._saved.items():
            if v is not None:
                os.environ[k] = v
        shutil.rmtree(self.tmp, ignore_errors=True)

    def set_rc(self, rc, err=None):
        out = self.fake / "out"
        out.mkdir(parents=True, exist_ok=True)
        (out / "def_write.rc").write_text(str(rc), encoding="utf-8")
        if err:
            (out / "def_write.err").write_text(err + "\n", encoding="utf-8")

    def calls(self):
        d = self.fake / "calls"
        return [(f.read_text(encoding="utf-8").splitlines(),
                 (d / (f.name + ".stdin")).read_text(encoding="utf-8"))
                for f in sorted(d.iterdir()) if f.suffix != ".stdin"]

    def test_creates_folders_and_block(self):
        res = init.init(update=False)
        self.assertEqual(res, {"ts_root": str(self.tmp / "ts"), "block": "created"})
        for d in ("ts/datasets", "ts/ext", "ts/scripts"):
            self.assertTrue((self.tmp / d).is_dir(), d)
        calls = self.calls()
        self.assertEqual(len(calls), 1)
        args, stdin = calls[0]
        self.assertEqual(args[:2], ["def", "write"])
        self.assertIn("--create", args)
        self.assertIn("ts-agent", args)
        self.assertEqual(stdin, BLOCK.read_text(encoding="utf-8"))

    def test_exists_without_update_is_not_rewritten(self):
        self.set_rc(4, "既にあります")
        self.assertEqual(init.init(update=False)["block"], "exists")
        self.assertEqual(len(self.calls()), 1)

    def test_update_rewrites_without_create(self):
        self.set_rc(4, "既にあります")
        self.assertEqual(init.init(update=True)["block"], "updated")
        calls = self.calls()
        self.assertEqual(len(calls), 2)
        self.assertIn("--create", calls[0][0])
        self.assertNotIn("--create", calls[1][0])
        self.assertEqual(calls[1][1], BLOCK.read_text(encoding="utf-8"))


class BlockTest(unittest.TestCase):
    """ts-agent.md の frontmatter と本文の 7 節（AC6）。"""

    def setUp(self):
        self.text = BLOCK.read_text(encoding="utf-8")

    def test_frontmatter(self):
        lines = self.text.splitlines()
        self.assertEqual(lines[0], "---")
        fm = lines[1:lines.index("---", 1)]
        self.assertEqual(fm[0], "name: 📈 ts-agent")
        self.assertEqual(fm[1], "id: ts-agent")
        self.assertTrue(fm[2].startswith("description: sai-ts のチャット相手。"))
        self.assertEqual(fm[3:], ["model: sonnet", "thinking: high", "templates: sai-base",
                                  "worktree: none", "enable: true"])

    def test_seven_sections_present(self):
        heads = re.findall(r"^## ([1-7])\. ", self.text, flags=re.M)
        self.assertEqual(heads, ["1", "2", "3", "4", "5", "6", "7"])

    def test_body_length(self):
        body = self.text.split("\n---\n", 1)[1]
        self.assertTrue(80 <= len(body.splitlines()) <= 150, len(body.splitlines()))


if __name__ == "__main__":
    unittest.main()
