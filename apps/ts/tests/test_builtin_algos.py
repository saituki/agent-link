"""組み込みの 3 本（AC2・AC3）: 合成データで bandpass・hfo_detect・event_features を scripts.run_script から動かす。"""

import hashlib
import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from fastapi.testclient import TestClient  # noqa: E402

from sai_ts import algos, events, scripts  # noqa: E402
from sai_ts.server import app  # noqa: E402

SR = 500.0
SECONDS = 20
NOISE = 0.2  # 雑音の標準偏差
BURST_S = 0.2  # バーストの長さ（秒）
AMP = 3.0  # バーストの振幅（100 Hz の正弦）
HUGE = 300.0  # 雑音の扱いを見る大きなバースト（r02 だけ）
BUILTIN = algos.BUILTIN_DIR


def make_record(ds_dir: Path, rec: str, bursts: list[tuple[float, float]], seed: int) -> None:
    rdir = ds_dir / "records" / rec
    rdir.mkdir(parents=True)
    rng = np.random.default_rng(seed)
    n = int(SECONDS * SR)
    t = np.arange(n) / SR
    x = NOISE * rng.standard_normal(n)
    for start, amp in bursts:
        m = (t >= start) & (t < start + BURST_S)
        x[m] += amp * np.sin(2 * np.pi * 100 * t[m])
    x.astype("<f4").tofile(rdir / "ch01.f32")
    meta = {"version": 1, "srate": SR, "n_samples": n,
            "vars": [{"name": "ch01", "unit": "uV", "file": "ch01.f32", "source": "raw"}]}
    (rdir / "meta.json").write_text(json.dumps(meta, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


class BuiltinTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        (self.tmp / ".sai").mkdir()
        self._saved = {k: os.environ.pop(k, None) for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN", "SAI_TS_SOURCE")}
        os.environ["SAI_PROJECT_ROOT"] = str(self.tmp)
        ds = self.tmp / "ts" / "datasets" / "syn"
        # r01 は 3 か所のバースト。r02 は同じ 3 か所に加えて、雑音になるほど大きい 1 か所
        make_record(ds, "r01", [(3.0, AMP), (9.0, AMP), (15.0, AMP)], seed=1)
        make_record(ds, "r02", [(3.0, AMP), (9.0, AMP), (15.0, AMP), (17.0, HUGE)], seed=2)

    def tearDown(self):
        for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN", "SAI_TS_SOURCE"):
            os.environ.pop(k, None)
        for k, v in self._saved.items():
            if v is not None:
                os.environ[k] = v
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_algo(self, name, params=None, records=None, target=None):
        # 画面と同じく、検査と既定の埋めを通してから動かす
        spec = algos.find(name)[1]
        body = algos.check_request(spec, {"records": records or [], "params": params or {},
                                          "target": target or {"vars": ["ch01"], "events": []}})
        return scripts.run_script(name, "syn", body["records"], body["params"],
                                  target=body["target"], by="human")

    def test_bandpass_adds_bandpass_and_envelope(self):  # AC2（bandpass）
        res = self.run_algo("bandpass")
        self.assertEqual(res["result"]["message"], "2 ch を足しました（2 記録）")
        self.assertEqual(res["result"]["vars"], ["ch01_bp80-250", "ch01_env80-250"])
        meta = json.loads((self.tmp / "ts" / "datasets" / "syn" / "records" / "r01" / "meta.json").read_text())
        added = {v["name"]: v for v in meta["vars"] if v["source"] == "derived"}
        self.assertEqual(set(added), {"ch01_bp80-250", "ch01_env80-250"})
        d = added["ch01_bp80-250"]["derived"]
        self.assertEqual((d["script"], d["inputs"], d["run"]), ("bandpass", ["ch01"], res["run"]))
        self.assertEqual(d["sha256"], hashlib.sha256((BUILTIN / "bandpass.py").read_bytes()).hexdigest())
        self.assertEqual(len(d["sha256"]), 64)
        self.assertEqual(added["ch01_env80-250"]["unit"], "uV")
        # 包絡は 0 以上。バーストの中心で、雑音のときより大きい
        env = np.fromfile(self.tmp / "ts" / "datasets" / "syn" / "records" / "r01" / "ch01_env80-250.f32",
                          dtype="<f4")
        center = int(3.1 * SR)  # 最初のバーストは 3.0〜3.2 秒
        self.assertGreater(env[center], 3 * np.median(env[: int(2 * SR)]))

    def test_bandpass_refuses_inverted_band(self):  # AC2（終了コード 2 に当たる失敗）
        with self.assertRaises(RuntimeError) as cm:
            self.run_algo("bandpass", {"low_hz": 300, "high_hz": 250})
        self.assertIn("low_hz は high_hz より小さくしてください", str(cm.exception))

    def test_hfo_detect_finds_each_burst_once(self):  # AC2（hfo_detect）
        res = self.run_algo("hfo_detect", {"noise_category": "noise"}, target={"vars": ["ch01"], "events": []})
        table = {(r[0], r[1]): r for r in res["result"]["table"]["rows"]}
        self.assertEqual((table[("r01", "ch01")][2], table[("r01", "ch01")][3]), (3, 0))
        self.assertEqual((table[("r02", "ch01")][2], table[("r02", "ch01")][3]), (3, 1))
        self.assertEqual(res["result"]["events"], 7)

        ds = "syn"
        hfo = events.list_events(ds, category="HFO")
        for rec in ("r01", "r02"):
            rows = [e for e in hfo if e["record"] == rec]
            for center in (3.1, 9.1, 15.1):
                hit = [e for e in rows if e["t0"] <= center <= e["t1"]]
                self.assertEqual(len(hit), 1, (rec, center))
        noise = events.list_events(ds, category="noise")
        self.assertEqual([e["record"] for e in noise], ["r02"])
        self.assertLessEqual(noise[0]["t0"], 17.1)
        self.assertGreaterEqual(noise[0]["t1"], 17.1)
        self.assertEqual(hfo[0]["source"], "script:hfo_detect@" + hashlib.sha256(
            (BUILTIN / "hfo_detect.py").read_bytes()).hexdigest()[:8])

    def test_event_features_write_peak_frequency(self):  # AC2（event_features）
        self.run_algo("hfo_detect", {"noise_category": "noise"}, target={"vars": ["ch01"], "events": []})
        res = self.run_algo("event_features", records=["r01"], target={"vars": [], "events": []})
        rows = res["result"]["table"]["rows"]
        self.assertEqual(res["result"]["table"]["columns"],
                         ["id", "record", "t0", "duration_s", "peak_abs", "rms", "peak_hz"])
        self.assertEqual(len(rows), 3)
        r01 = [e for e in events.list_events("syn", category="HFO") if e["record"] == "r01"]
        self.assertEqual(len(r01), 3)
        for e in r01:
            self.assertLessEqual(abs(e["metrics"]["peak_hz"] - 100), 5)
            self.assertGreater(e["metrics"]["duration_s"], 0)
            self.assertGreater(e["metrics"]["zero_cross"], 0)
        names = {m["name"] for m in events.load_schema("syn")["metrics"]}
        self.assertTrue({"duration_s", "peak_abs", "rms", "line_length", "peak_hz", "zero_cross"} <= names)

    def test_list_over_http_shows_three_builtins(self):  # AC3
        c = TestClient(app)
        body = c.get("/api/algos").json()["algos"]
        by = {a["name"]: a for a in body}
        for name in ("bandpass", "hfo_detect", "event_features"):
            with self.subTest(name):
                self.assertEqual((by[name]["origin"], by[name]["error"]), ("builtin", ""))
        self.assertNotIn("_dsp", by)


if __name__ == "__main__":
    unittest.main()
