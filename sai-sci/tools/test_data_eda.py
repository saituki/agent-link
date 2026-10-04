#!/usr/bin/env python3
"""写した EDA と生体信号の CLI のテスト。標準ライブラリだけで動く。

スクリプトを実際に走らせ、出力が読めることを確かめる（終了コード 0 と JSON の形）。
"""

from __future__ import annotations

import json
import math
import os
import random
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
EDA = TOOLS / "eda"
BIOSIGNAL = TOOLS / "biosignal"
TIMEOUT = 120

# 試しの CSV の列と行数。
COLUMNS = ["id", "group", "x", "y", "split"]
ROWS = 200
# id .bukkitTrain と test の両方に出る主体の数。
ENTITIES = 100


def write_csv(path: Path, header: list[str], rows: list[list]) -> Path:
    lines = [",".join(header)]
    for row in rows:
        lines.append(",".join("" if v is None else str(v) for v in row))
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return path


def run_script(script: Path, *args) -> subprocess.CompletedProcess:
    """写した CLI を走らせる。標準出力は取り込まない（返り値だけ見る）。"""
    return subprocess.run(
        [sys.executable, str(script), *[str(a) for a in args]],
        capture_output=True,
        text=True,
        timeout=TIMEOUT,
    )


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="data-eda-test-"))
        self.csv = self.make_table()
        self.signal = self.make_signal()
        self.events = self.make_events()

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def make_table(self) -> Path:
        """列 id,group,x,y,split の 200 行。x に欠損があり、id の一部が train と test の両方に出る。"""
        rng = random.Random(7)
        rows = []
        for i in range(ROWS):
            entity = i % ENTITIES
            x = None if i % 17 == 0 else round(rng.gauss(50.0, 10.0), 3)
            rows.append([
                f"S{entity:03d}",
                rng.choice(["a", "b"]),
                x,
                round(rng.gauss(2.0, 1.0), 3),
                "train" if i < 150 else "test",
            ])
        return write_csv(self.tmp / "data.csv", COLUMNS, rows)

    def make_signal(self) -> Path:
        """列 time,ecg の 2000 行の正弦波（250 Hz で 2 秒）。"""
        rows = []
        for i in range(2000):
            rows.append([f"{i / 250:.5f}", f"{math.sin(i / 12):.6f}"])
        return write_csv(self.tmp / "signal.csv", ["time", "ecg"], rows)

    def make_events(self) -> Path:
        rows = [[onset, f"e{onset}"] for onset in (100, 500, 900, 1400)]
        return write_csv(self.tmp / "events.csv", ["onset", "label"], rows)

    def assert_ok(self, result: subprocess.CompletedProcess, what: str):
        self.assertEqual(
            result.returncode,
            0,
            f"{what} が 0 で終わらなかった。標準エラー: {result.stderr[-800:]}",
        )

    def read_out(self, name: str) -> dict:
        path = self.tmp / name
        self.assertTrue(path.exists(), f"{name} が出力されなかった")
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)


class TestHelp(Base):
    def test_eda_analyzer_help(self):
        for script in sorted(EDA.glob("*.py")):
            with self.subTest(script=script.name):
                self.assertEqual(run_script(script, "--help").returncode, 0)

    def test_biosignal_help(self):
        for script in sorted(BIOSIGNAL.glob("*.py")):
            with self.subTest(script=script.name):
                self.assertEqual(run_script(script, "--help").returncode, 0)


class TestEdaAnalyzer(Base):
    def test_runs_and_reports_five_columns(self):
        out = self.tmp / "eda.json"
        result = run_script(
            EDA / "eda_analyzer.py",
            self.csv, "--root", self.tmp, "--reveal-identifiers",
            "--format", "json", "--output", out,
        )
        self.assert_ok(result, "eda_analyzer.py")
        payload = self.read_out("eda.json")
        analysis = payload["analysis"]
        self.assertEqual(analysis["column_count"], len(COLUMNS))
        self.assertEqual(len(analysis["columns"]), len(COLUMNS))
        self.assertEqual(analysis["rows_scanned"], ROWS)

    def test_capability_manifest(self):
        out = self.tmp / "cap.json"
        result = run_script(
            EDA / "capability_manifest.py", "inspect", self.csv, "--root", self.tmp, "--output", out
        )
        self.assert_ok(result, "capability_manifest.py")
        self.read_out("cap.json")

    def test_tabular_profile(self):
        out = self.tmp / "profile.json"
        result = run_script(
            EDA / "tabular_profile.py", self.csv, "--root", self.tmp,
            "--reveal-identifiers", "--output", out,
        )
        self.assert_ok(result, "tabular_profile.py")
        payload = self.read_out("profile.json")
        self.assertTrue(payload)


class TestMissingnessLeakage(Base):
    """要確認だった実際の出力を確かめる。"""

    def test_detects_split_leakage(self):
        out = self.tmp / "leak.json"
        result = run_script(
            EDA / "missingness_leakage_audit.py", self.csv, "--root", self.tmp,
            "--entity-column", "id", "--split-column", "split",
            "--reveal-identifiers", "--output", out,
        )
        self.assert_ok(result, "missingness_leakage_audit.py")
        audit = self.read_out("leak.json")["analysis"]["leakage_audit"]
        self.assertEqual(audit["status"], "potential_leakage_detected")
        # 同じ id が train と test の両方に出るので、検出数が 0 より大きい
        self.assertGreater(audit["entity_tokens_in_multiple_splits"], 0)
        self.assertEqual(audit["rows_with_missing_split"], 0)

    def test_reports_missingness_by_split(self):
        out = self.tmp / "leak2.json"
        result = run_script(
            EDA / "missingness_leakage_audit.py", self.csv, "--root", self.tmp,
            "--split-column", "split", "--reveal-identifiers", "--output", out,
        )
        self.assert_ok(result, "missingness_leakage_audit.py")
        analysis = self.read_out("leak2.json")["analysis"]
        self.assertTrue(analysis["missingness_by_split"])
        # x に欠損があるので、x の missing_count の合計が 0 より大きい
        totals = sum(
            entry["missing_count"]
            for split in analysis["missingness_by_split"]
            for entry in split["column_missingness"]
            if entry["column_id"] == "x"
        )
        self.assertGreater(totals, 0)

    def test_no_false_alarm_without_roles(self):
        """主体の指定が無くても落ちない（status は found のままでもよい）。"""
        out = self.tmp / "leak3.json"
        result = run_script(
            EDA / "missingness_leakage_audit.py", self.csv, "--root", self.tmp,
            "--reveal-identifiers", "--output", out,
        )
        self.assert_ok(result, "missingness_leakage_audit.py")
        self.assertIn("status", self.read_out("leak3.json")["analysis"]["leakage_audit"])


class TestDistributionSensitivity(Base):
    def test_runs_and_keeps_data_unchanged(self):
        before = self.csv.read_bytes()
        out = self.tmp / "dist.json"
        result = run_script(
            EDA / "distribution_sensitivity.py", self.csv, "--root", self.tmp,
            "--reveal-identifiers", "--output", out,
        )
        self.assert_ok(result, "distribution_sensitivity.py")
        self.assertEqual(self.csv.read_bytes(), before, "元の CSV が変わってしまった")
        analysis = self.read_out("dist.json")["analysis"]
        self.assertGreater(analysis["numeric_columns_reported"], 0)

    def test_with_explicit_column(self):
        out = self.tmp / "dist_x.json"
        result = run_script(
            EDA / "distribution_sensitivity.py", self.csv, "--root", self.tmp,
            "--column", "x", "--reveal-identifiers", "--output", out,
        )
        self.assert_ok(result, "distribution_sensitivity.py")
        self.assertTrue(self.read_out("dist_x.json")["analysis"]["columns"])


class TestReportScaffold(Base):
    """report_scaffold.py は同梱の assets/report_template.md を必要とする。

    書き出し先は scripts/ と同じ階層に置く assets/ だが、写し先の
    assets/report_template.md は scripts/ の外にあるため、今回写した 12 本には
    含まれない。模板が無いときは理由を出して 2 で終える（例外ではない）。
    模板が配られたら 0 で止まることを要求する。
    """

    TEMPLATE = TOOLS.parent / "assets" / "report_template.md"

    def test_runs_or_reports_missing_template(self):
        out = self.tmp / "report.md"
        result = run_script(
            EDA / "report_scaffold.py", "--input", self.csv, "--root", self.tmp, "--output", out
        )
        if self.TEMPLATE.exists():
            self.assert_ok(result, "report_scaffold.py")
            self.assertTrue(out.read_text(encoding="utf-8").strip())
        else:
            # 模板が無いときは理由を出して 2 で終える（例外ではない）
            self.assertEqual(result.returncode, 2)
            self.assertIn("report template is unavailable", result.stderr)


class TestBiosignal(Base):
    def test_inspect_signal(self):
        out = self.tmp / "sig.json"
        result = run_script(
            BIOSIGNAL / "inspect_signal.py", "--input", self.signal, "--root", self.tmp,
            "--columns", "ecg", "--time-column", "time", "--sampling-rate", "250",
            "--deidentified", "--output", out,
        )
        self.assert_ok(result, "inspect_signal.py")
        self.assertTrue(self.read_out("sig.json"))

    def test_plan_epochs_from_events_csv(self):
        out = self.tmp / "epochs.json"
        result = run_script(
            BIOSIGNAL / "plan_epochs.py",
            "--events-csv", self.events, "--deidentified",
            "--onset-column", "onset", "--event-unit", "samples",
            "--sampling-rate", "250", "--recording-samples", "2000",
            "--epoch-start", "0.0", "--epoch-end", "2.0",
            "--root", self.tmp, "--output", out,
        )
        self.assert_ok(result, "plan_epochs.py")
        payload = self.read_out("epochs.json")
        self.assertTrue(payload)

    def test_validate_multimodal(self):
        out = self.tmp / "mm.json"
        result = run_script(
            BIOSIGNAL / "validate_multimodal.py", "--manifest", str(self.csv),
            "--root", self.tmp, "--deidentified", "--output", out,
        )
        # Manifest の形が違っても落ちない。usage の誤りなら 2、実際の検査なら 0。
        self.assertIn(result.returncode, (0, 2))
        if result.returncode == 0:
            self.assertTrue(out.exists())


class TestNoDependencies(Base):
    """写した道具が第三者のパッケージ無しで動くことを確かめる。

    第三者の import を全て失敗させる sitecustomize.py を PYTHONPATH に入れて走らせる。
    """

    BLOCKER = '''import sys
from importlib.abc import MetaPathFinder

BLOCKED = {"numpy", "pandas", "scipy", "neurokit2", "sklearn",
           "matplotlib", "h5py", "statsmodels", "torch"}


class _Blocker(MetaPathFinder):
    def find_spec(self, fullname, path=None, target=None):
        if fullname.split(".")[0] in BLOCKED:
            raise ImportError(fullname + " is blocked by the test")
        return None


sys.meta_path.insert(0, _Blocker())
'''

    def setUp(self):
        super().setUp()
        self.block_dir = Path(tempfile.mkdtemp(prefix="no-deps-"))
        (self.block_dir / "sitecustomize.py").write_text(self.BLOCKER, encoding="utf-8")

    def tearDown(self):
        super().tearDown()
        shutil.rmtree(self.block_dir, ignore_errors=True)

    def run_blocked(self, script: Path, *args) -> subprocess.CompletedProcess:
        """第三者のパッケージを塞いで走らせる。"""
        env = dict(os.environ)
        env["PYTHONPATH"] = str(self.block_dir)
        return subprocess.run(
            [sys.executable, str(script), *[str(a) for a in args]],
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
            env=env,
        )

    def test_blocker_works(self):
        """このテストが本当に塞げていることの確かめ。"""
        probe = self.block_dir / "probe.py"
        probe.write_text(
            "import sys\n"
            "try:\n"
            "    import numpy\n"
            "except ImportError:\n"
            "    print('blocked')\n"
            "    sys.exit(0)\n"
            "print('not blocked')\n"
            "sys.exit(1)\n",
            encoding="utf-8",
        )
        env = dict(os.environ)
        env["PYTHONPATH"] = str(self.block_dir)
        result = subprocess.run(
            [sys.executable, str(probe)], capture_output=True, text=True, timeout=TIMEOUT, env=env
        )
        self.assertEqual(result.returncode, 0, f"sitecustomize.py が効いていない: {result.stdout}")

    def test_eda_scripts_run_without_third_party(self):
        cases = [
            ("capability_manifest.py", [EDA / "capability_manifest.py", "inspect", self.csv,
              "--root", self.tmp, "--output", self.tmp / "c.json"]),
            ("eda_analyzer.py", [EDA / "eda_analyzer.py", self.csv, "--root", self.tmp,
              "--reveal-identifiers", "--format", "json", "--output", self.tmp / "e.json"]),
            ("tabular_profile.py", [EDA / "tabular_profile.py", self.csv, "--root", self.tmp,
              "--reveal-identifiers", "--output", self.tmp / "p.json"]),
            ("missingness_leakage_audit.py", [EDA / "missingness_leakage_audit.py", self.csv,
              "--root", self.tmp, "--entity-column", "id", "--split-column", "split",
              "--reveal-identifiers", "--output", self.tmp / "l.json"]),
            ("distribution_sensitivity.py", [EDA / "distribution_sensitivity.py", self.csv,
              "--root", self.tmp, "--reveal-identifiers", "--output", self.tmp / "d.json"]),
        ]
        for name, argv in cases:
            with self.subTest(script=name):
                self.assert_ok(self.run_blocked(argv[0], *argv[1:]), name)

    def test_biosignal_scripts_run_without_third_party(self):
        cases = [
            ("inspect_signal.py", [BIOSIGNAL / "inspect_signal.py", "--input", self.signal,
              "--root", self.tmp, "--columns", "ecg", "--time-column", "time",
              "--sampling-rate", "250", "--deidentified", "--output", self.tmp / "s.json"]),
            ("plan_epochs.py", [BIOSIGNAL / "plan_epochs.py", "--events-csv", self.events,
              "--deidentified", "--onset-column", "onset", "--event-unit", "samples",
              "--sampling-rate", "250", "--recording-samples", "2000",
              "--epoch-start", "0.0", "--epoch-end", "2.0", "--root", self.tmp,
              "--output", self.tmp / "ep.json"]),
        ]
        for name, argv in cases:
            with self.subTest(script=name):
                self.assert_ok(self.run_blocked(argv[0], *argv[1:]), name)

    def test_neurokit_scripts_declare_their_dependency(self):
        """neurokit2 が要るのは 2 本だけ。"""
        for name in ("biosignal/ecg_hrv_pipeline.py", "biosignal/eda_pipeline.py"):
            text = (TOOLS / name).read_text(encoding="utf-8")
            with self.subTest(script=name):
                self.assertIn("neurokit2", text)


if __name__ == "__main__":
    unittest.main()