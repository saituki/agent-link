"""スクリプト（AC4）: sdk の例のスクリプトを run_script で動かし、出どころ（source）が残ること。"""

import hashlib
import os
import re
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_ts import algos, demo, events, runs, scripts  # noqa: E402

# sdk.py の docstring にある例と同じ（agent が読む物と動きを揃える）
BURST = '''import json, sys
import numpy as np
from sai_ts import sdk
req = json.load(sys.stdin)
ds = req["dataset"]
found = []
for rec in req["records"] or sdk.records(ds):
    meta, data = sdk.load(ds, rec)
    x = data["ch01"]; sr = meta["srate"]
    idx = np.flatnonzero(np.abs(x) > req["params"].get("thr", 3.0) * np.std(x))
    for i in idx[::int(sr)]:   # 1 秒に 1 件まで
        found.append({"type": "line", "record": rec, "vars": ["ch01"], "t": float(i / sr), "category": "burst", "text": "", "metrics": {}})
sdk.add_events(ds, found)
print(json.dumps({"added": len(found)}))
'''


class ScriptsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        (self.tmp / ".sai").mkdir()
        self._saved = {k: os.environ.pop(k, None) for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN", "SAI_TS_SOURCE")}
        os.environ["SAI_PROJECT_ROOT"] = str(self.tmp)
        (self.tmp / "ts" / "datasets").mkdir(parents=True)
        demo.make(self.tmp / "ts" / "datasets" / "demo", records=2, nvars=2, seconds=2.0, srate=100.0)
        (self.tmp / "ts" / "scripts").mkdir()

    def tearDown(self):
        for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN", "SAI_TS_SOURCE"):
            os.environ.pop(k, None)
        for k, v in self._saved.items():
            if v is not None:
                os.environ[k] = v
        shutil.rmtree(self.tmp, ignore_errors=True)

    def write_script(self, name, body):
        (self.tmp / "ts" / "scripts" / f"{name}.py").write_text(body, encoding="utf-8")

    def test_burst_script_adds_events_with_source(self):
        self.write_script("burst", BURST)
        sha = hashlib.sha256(BURST.encode("utf-8")).hexdigest()[:8]
        res = scripts.run_script("burst", "demo", [], {"thr": 1.0})
        self.assertEqual(res["script"], "burst")
        self.assertEqual(res["source"], f"script:burst@{sha}")
        added = res["result"]["added"]
        self.assertGreaterEqual(added, 1)
        evs = events.list_events("demo")
        self.assertEqual(len(evs), added)
        for ev in evs:
            self.assertEqual(ev["source"], res["source"])
            self.assertTrue(events.SOURCE_RE.fullmatch(ev["source"]))
        self.assertGreaterEqual(res["elapsed_s"], 0)

    def test_records_limit_the_run(self):
        self.write_script("burst", BURST)
        res = scripts.run_script("burst", "demo", ["r01"], {"thr": 1.0})
        self.assertGreaterEqual(res["result"]["added"], 1)
        recs = {ev["record"] for ev in events.list_events("demo")}
        self.assertEqual(recs, {"r01"})

    def test_failure_reports_last_stderr_line(self):
        self.write_script("bad", "import sys\nprint('x', file=sys.stderr)\nsys.exit(1)\n")
        with self.assertRaises(RuntimeError) as cm:
            scripts.run_script("bad", "demo", [])
        self.assertEqual(str(cm.exception), "bad が失敗しました: x")

    def test_timeout_raises(self):
        self.write_script("slow", "import time\ntime.sleep(5)\n")
        with self.assertRaises(TimeoutError):
            scripts.run_script("slow", "demo", [], timeout=1)

    def test_unknown_script_and_bad_names(self):
        with self.assertRaises(KeyError):
            scripts.run_script("nope", "demo", [])
        with self.assertRaises(ValueError):
            scripts.run_script("../x", "demo", [])
        self.assertEqual(runs.list_runs("demo"), [])  # 見つからないものは記録に残さない

    def test_run_records_start_and_end(self):  # AC2（記録）
        self.write_script("burst", BURST)
        res = scripts.run_script("burst", "demo", [], {"thr": 1.0})
        self.assertTrue(res["run"].startswith("a-"))
        row = runs.list_runs("demo")[0]
        self.assertEqual((row["id"], row["status"], row["by"]), (res["run"], "ok", "agent"))
        self.assertEqual((row["algo"], row["origin"], row["params"]), ("burst", "project", {"thr": 1.0}))
        self.assertEqual(row["sha256"], hashlib.sha256(BURST.encode("utf-8")).hexdigest())
        self.assertEqual((row["result"]["added"], row["elapsed_s"]), (res["result"]["added"], res["elapsed_s"]))

    def test_failure_and_timeout_are_recorded(self):  # AC3（記録）
        self.write_script("bad", "import sys\nprint('x', file=sys.stderr)\nsys.exit(1)\n")
        self.write_script("slow", "import time\ntime.sleep(5)\n")
        with self.assertRaises(RuntimeError):
            scripts.run_script("bad", "demo", [])
        with self.assertRaises(TimeoutError):
            scripts.run_script("slow", "demo", [], timeout=1)
        by = {r["algo"]: r for r in runs.list_runs("demo")}
        self.assertEqual((by["bad"]["status"], by["bad"]["error"]), ("error", "bad が失敗しました: x"))
        self.assertEqual(by["slow"]["status"], "timeout")

    def test_child_gets_request_and_env(self):  # G2-C2
        self.write_script("echo", ECHO)
        tgt = {"vars": ["ch01"], "events": []}
        res = scripts.run_script("echo", "demo", [], {}, target=tgt, run_id="a-111111111111")
        out = res["result"]
        sha = hashlib.sha256(ECHO.encode("utf-8")).hexdigest()
        self.assertEqual(out["req"], {"dataset": "demo", "records": ["r01", "r02"], "params": {}, "target": tgt,
                                      "run": "a-111111111111"})
        self.assertEqual((out["run"], out["sha"], out["target"]), ("a-111111111111", sha, tgt))
        self.assertEqual((out["src"], res["source"]), (f"script:echo@{sha[:8]}", f"script:echo@{sha[:8]}"))
        self.assertEqual(res["run"], "a-111111111111")

    def test_builtin_runs_without_project_script(self):  # 組み込み（試験の一時の BUILTIN_DIR）
        builtin = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, builtin, True)
        (builtin / "hello.py").write_text('import json, sys\njson.load(sys.stdin)\nprint(json.dumps({"message": "hi"}))\n',
                                          encoding="utf-8")
        with mock.patch.object(algos, "BUILTIN_DIR", builtin):
            res = scripts.run_script("hello", "demo", [])
        self.assertEqual(res["result"], {"message": "hi"})
        self.assertEqual(runs.list_runs("demo")[0]["origin"], "builtin")


ECHO = '''import json, os, sys
req = json.load(sys.stdin)
print(json.dumps({"req": req, "run": os.environ["SAI_TS_RUN"], "sha": os.environ["SAI_TS_SHA256"],
                  "src": os.environ["SAI_TS_SOURCE"], "target": json.loads(os.environ["SAI_TS_TARGET"])}))
'''


if __name__ == "__main__":
    unittest.main()
