"""命令: doctor の表と setup の動き。"""

from __future__ import annotations

import helpers  # noqa: F401  （src を sys.path に入れる）
import io
import os
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

from fakes import write_fake_tools
from sai_prep import cli


class DoctorTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bindir = write_fake_tools(self.root / "bin")
        env = mock.patch.dict(
            os.environ, {"SAI_PREP_CONFIG": str(self.root / "config.json")}
        )
        env.start()
        self.addCleanup(env.stop)

    def _fake_find(self, name, extra_dirs=None):
        candidate = self.bindir / name
        return str(candidate) if candidate.exists() else None

    def test_missing_tools_are_reported(self):
        with mock.patch("sai_prep.cli.find_tool", return_value=None):
            rows = dict(cli.doctor_rows())
        self.assertIn("無い", rows["docling"])
        self.assertIn("sai-prep setup", rows["docling"])
        self.assertIn("未設定", rows["サーバ"])

    def test_present_tools_are_reported(self):
        with mock.patch("sai_prep.cli.find_tool", side_effect=self._fake_find):
            rows = dict(cli.doctor_rows())
        self.assertIn("あり", rows["docling"])
        self.assertIn("2.134.0", rows["docling"])
        self.assertIn("1.9.11", rows["pdf2zh"])
        self.assertIn("あり", rows["mlx_lm.server"])

    def test_cmd_doctor_prints_a_table(self):
        with mock.patch("sai_prep.cli.find_tool", return_value=None):
            buffer = io.StringIO()
            with redirect_stdout(buffer):
                code = cli.cmd_doctor()
        self.assertEqual(code, 0)
        self.assertIn("sai-prep", buffer.getvalue())
        self.assertIn("docling", buffer.getvalue())

    def test_main_doctor(self):
        with mock.patch("sai_prep.cli.find_tool", return_value=None):
            buffer = io.StringIO()
            with redirect_stdout(buffer):
                code = cli.main(["doctor"])
        self.assertEqual(code, 0)


class SetupTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        env = mock.patch.dict(
            os.environ, {"SAI_PREP_CONFIG": str(self.root / "config.json")}
        )
        env.start()
        self.addCleanup(env.stop)

    def test_setup_without_uv_fails(self):
        with mock.patch("sai_prep.cli._uv_binary", return_value=None):
            buffer = io.StringIO()
            with redirect_stdout(buffer):
                code = cli.cmd_setup(mock.Mock(yes=True))
        self.assertEqual(code, 1)

    def test_setup_installs_only_missing(self):
        calls = []

        def fake_run(argv, *a, **k):
            calls.append(argv)
            return mock.Mock(returncode=0)

        args = mock.Mock(yes=True)
        with (
            mock.patch("sai_prep.cli._uv_binary", return_value="/usr/bin/uv"),
            mock.patch("sai_prep.cli.find_tool", return_value=None),
            mock.patch("sai_prep.cli.subprocess.run", side_effect=fake_run),
            mock.patch("sai_prep.cli.cmd_doctor", return_value=0),
        ):
            buffer = io.StringIO()
            with redirect_stdout(buffer):
                code = cli.cmd_setup(args)
        self.assertEqual(code, 0)
        self.assertEqual(
            calls,
            [
                ["/usr/bin/uv", "tool", "install", "--python", "3.12", "docling"],
                ["/usr/bin/uv", "tool", "install", "--python", "3.12", "pdf2zh"],
                ["/usr/bin/uv", "tool", "install", "--python", "3.12", "mlx-lm"],
            ],
        )


if __name__ == "__main__":
    unittest.main()
