"""sdk（AC4）: 読み書きの関数と、派生変数に出どころ（derived）が残ること。"""

import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_ts import demo, events, sdk  # noqa: E402


class SdkTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        (self.tmp / ".sai").mkdir()
        self._saved = {k: os.environ.pop(k, None) for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN", "SAI_TS_SOURCE", "SAI_TS_SHA256", "SAI_TS_RUN", "SAI_TS_TARGET")}
        os.environ["SAI_PROJECT_ROOT"] = str(self.tmp)
        (self.tmp / "ts" / "datasets").mkdir(parents=True)
        demo.make(self.tmp / "ts" / "datasets" / "demo", records=2, nvars=2, seconds=2.0, srate=100.0)

    def tearDown(self):
        for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN", "SAI_TS_SOURCE", "SAI_TS_SHA256", "SAI_TS_RUN", "SAI_TS_TARGET"):
            os.environ.pop(k, None)
        for k, v in self._saved.items():
            if v is not None:
                os.environ[k] = v
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_read_functions(self):
        self.assertEqual(sdk.dataset("demo"), self.tmp / "ts" / "datasets" / "demo")
        self.assertEqual(sdk.records("demo"), ["r01", "r02"])
        meta, data = sdk.load("demo", "r01")
        self.assertEqual(meta["srate"], 100.0)
        self.assertEqual(sorted(data), ["ch01", "ch02"])
        self.assertEqual(data["ch01"].shape, (200,))
        self.assertEqual(sdk.view(), {})

    def test_add_events_source_from_env(self):
        ev = {"type": "line", "record": "r01", "vars": [], "t": 0.5, "category": "burst",
              "text": "", "metrics": {}}
        sdk.add_events("demo", [ev])
        self.assertEqual(sdk.events("demo", record="r01")[0]["source"], "agent")
        os.environ["SAI_TS_SOURCE"] = "script:burst@0a1b2c3d"
        sdk.add_events("demo", [{**ev, "t": 0.9}])
        self.assertEqual(sdk.events("demo", record="r01")[-1]["source"], "script:burst@0a1b2c3d")

    def test_add_var_records_origin(self):
        full = "0a1b2c3d" * 8
        os.environ["SAI_TS_SOURCE"] = "script:burst@0a1b2c3d"
        os.environ["SAI_TS_SHA256"] = full
        os.environ["SAI_TS_RUN"] = "a-3f9a1c2b7d10"
        meta = sdk.add_var("demo", "r01", "ch01_hp", np.zeros(200), "uV", {"thr": 1}, inputs=["ch01"])
        entry = next(v for v in meta["vars"] if v["name"] == "ch01_hp")
        self.assertEqual(entry["source"], "derived")
        self.assertEqual(entry["derived"]["script"], "burst")
        self.assertEqual(entry["derived"]["sha256"], full)  # AC5: 8 桁ではなく全長
        self.assertEqual(entry["derived"]["run"], "a-3f9a1c2b7d10")
        self.assertEqual(entry["derived"]["inputs"], ["ch01"])
        self.assertEqual(entry["derived"]["params"], {"thr": 1})
        self.assertEqual((self.tmp / "ts/datasets/demo/records/r01/ch01_hp.f32").stat().st_size, 800)
        _, data = sdk.load("demo", "r01")
        self.assertIn("ch01_hp", data)

    def test_add_var_without_script_is_agent(self):
        meta = sdk.add_var("demo", "r02", "ch02_z", np.ones(200, dtype=np.float32), "", None)
        derived = next(v for v in meta["vars"] if v["name"] == "ch02_z")["derived"]
        self.assertEqual((derived["script"], derived["sha256"], derived["params"]), ("agent", "", {}))
        self.assertEqual((derived["inputs"], derived["run"]), ([], ""))
        self.assertIn("created_at", derived)
        json.dumps(meta)  # meta.json に書ける形

    def test_ensure_category_and_metrics_keep_existing(self):  # AC5（demo の既定の schema は残す）
        before = {c["name"]: c for c in events.load_schema("demo")["categories"]}
        sdk.ensure_category("demo", "spike", "#d9534f", ch="single")
        sdk.ensure_category("demo", "spike", "#000000")  # 既にあれば変えない
        sdk.ensure_category("demo", "burst", "#000000")  # 既定の burst も変えない
        cats = {c["name"]: c for c in events.load_schema("demo")["categories"]}
        self.assertEqual((cats["spike"]["color"], cats["spike"]["ch"]), ("#d9534f", "single"))
        self.assertEqual(cats["burst"], before["burst"])
        sdk.ensure_metrics("demo", [{"name": "dur", "type": "number", "unit": "s"}])
        sdk.ensure_metrics("demo", [{"name": "amp", "type": "integer"}, {"name": "dur", "type": "string"}])
        mets = {m["name"]: m for m in events.load_schema("demo")["metrics"]}
        self.assertEqual((mets["amp"]["type"], mets["dur"]["type"], mets["dur"]["unit"]), ("number", "number", "s"))

    def test_update_metrics_merges_and_keeps_others(self):  # AC5
        sdk.ensure_category("demo", "burst", "#d9534f")
        sdk.ensure_metrics("demo", [{"name": "amp", "type": "number"}, {"name": "dur", "type": "number"}])
        a = sdk.add_events("demo", [{"type": "line", "record": "r01", "vars": [], "t": 0.5, "category": "burst",
                                     "text": "", "metrics": {"amp": 3.2}}])[0]
        self.assertEqual(sdk.update_metrics("demo", {a["id"]: {"dur": 0.042}}), 1)
        self.assertEqual(events.list_events("demo")[0]["metrics"], {"amp": 3.2, "dur": 0.042})
        with self.assertRaises(KeyError):
            sdk.update_metrics("demo", {"e-000000000000": {"dur": 1}})
        with self.assertRaises(ValueError):  # 誤り（型）なら何も書かない
            sdk.update_metrics("demo", {a["id"]: {"dur": "x"}})
        self.assertEqual(events.list_events("demo")[0]["metrics"]["dur"], 0.042)

    def test_target_from_env(self):
        self.assertEqual(sdk.target(), {})
        os.environ["SAI_TS_TARGET"] = '{"vars": ["ch01"], "events": []}'
        self.assertEqual(sdk.target(), {"vars": ["ch01"], "events": []})


if __name__ == "__main__":
    unittest.main()
