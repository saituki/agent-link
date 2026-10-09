"""読み込み・M4・派生変数（AC2・AC6）。"""

import math
import os
import shutil
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

import numpy as np  # noqa: E402

from sai_ts import dataset, demo, paths  # noqa: E402


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


def r6(v):
    return None if math.isnan(v) else float(f"{v:.6g}")


def naive_m4(vals, i0, i1, px):
    """素朴な計算: 列ごとに最初・最小・最大・最後を Python で求める。"""
    n_in = i1 - i0
    out = {"first": [], "min": [], "max": [], "last": []}
    for k in range(px):
        a = i0 + k * n_in // px
        b = i0 + (k + 1) * n_in // px
        seg = vals[a:b]
        ok = [v for v in seg if not math.isnan(v)]
        out["first"].append(r6(seg[0]))
        out["last"].append(r6(seg[-1]))
        out["min"].append(r6(min(ok)) if ok else None)
        out["max"].append(r6(max(ok)) if ok else None)
    return out


class DatasetTest(TempProject, unittest.TestCase):
    def setUp(self):
        super().setUp()
        self.ds = self.tmp / "ts" / "datasets" / "demo"
        demo.make(self.ds, records=2, nvars=2, seconds=10.0, srate=500.0)
        # NaN を含む変数: 列 10（[1000, 1100)）は全部 NaN。サンプル 2050 は一部だけ NaN。
        vals = np.asarray(dataset.open_var("demo", "r01", "ch01"), dtype=np.float32).copy()
        vals[1000:1100] = np.nan
        vals[2050] = np.nan
        vals[0] = np.nan
        dataset.add_var("demo", "r01", "nanvar", vals, "uV", {"script": "t", "sha256": "0" * 12, "params": {}})

    def vals(self, var, rec="r01"):
        return np.asarray(dataset.open_var("demo", rec, var), dtype=np.float32).tolist()

    def check_m4(self, var, t0, t1, px):
        res = dataset.m4("demo", "r01", [var], t0, t1, px)["vars"][var]
        srate = 500.0
        i0 = max(0, math.floor(t0 * srate))
        i1 = min(5000, math.ceil(t1 * srate))
        exp = naive_m4(self.vals(var), i0, i1, px)
        self.assertEqual(res["mode"], "m4")
        for key in ("first", "min", "max", "last"):
            self.assertEqual(res[key], exp[key], msg=f"{var} {key}")

    def test_m4_full_range_matches_naive(self):
        self.check_m4("ch01", 0.0, 10.0, 50)

    def test_m4_with_nan_and_all_nan_column(self):
        # 列 10 は全部 NaN（min/max は null）、列 0 は最初が NaN（first は null）
        res = dataset.m4("demo", "r01", ["nanvar"], 0.0, 10.0, 50)["vars"]["nanvar"]
        self.assertIsNone(res["min"][10])
        self.assertIsNone(res["max"][10])
        self.assertIsNone(res["first"][0])
        self.check_m4("nanvar", 0.0, 10.0, 50)

    def test_m4_partial_range_not_divisible(self):
        self.check_m4("ch01", 1.234, 8.777, 37)

    def test_raw_mode_when_few_samples(self):
        res = dataset.m4("demo", "r01", ["ch01"], 1.0, 1.5, 200)
        self.assertEqual((res["t0"], res["t1"]), (1.0, 1.5))
        v = res["vars"]["ch01"]
        self.assertEqual(v["mode"], "raw")
        self.assertEqual(v["start"], 1.0)
        self.assertEqual(v["dt"], 0.002)
        self.assertEqual(v["y"], [r6(x) for x in self.vals("ch01")[500:750]])

    def test_clip_to_record_range(self):
        res = dataset.m4("demo", "r01", ["ch01"], -5.0, 100.0, 10)
        self.assertEqual((res["t0"], res["t1"]), (0.0, 10.0))

    def test_errors(self):
        cases = [
            (dict(t0=2.0, t1=2.0, px=10), ValueError),
            (dict(t0=3.0, t1=2.0, px=10), ValueError),
            (dict(t0=0.0, t1=10.0, px=0), ValueError),
            (dict(t0=0.0, t1=10.0, px=8193), ValueError),
            (dict(t0=float("nan"), t1=10.0, px=10), ValueError),
        ]
        for kw, exc in cases:
            with self.assertRaises(exc, msg=str(kw)):
                dataset.m4("demo", "r01", ["ch01"], **kw)
        with self.assertRaises(ValueError):
            dataset.m4("demo", "r01", ["ch01"] * 65, 0.0, 10.0, 10)
        with self.assertRaises(KeyError):
            dataset.m4("demo", "r01", ["zz"], 0.0, 10.0, 10)
        with self.assertRaises(KeyError):
            dataset.m4("demo", "r09", ["ch01"], 0.0, 10.0, 10)

    def test_add_var_overwrites_and_keeps_meta_valid(self):
        n = 5000
        meta = dataset.add_var("demo", "r02", "bp", np.arange(n, dtype=np.float32) * 2, "uV",
                               {"script": "bandpass", "sha256": "abcdef012345", "params": {"lo": 40}})
        self.assertEqual([v["name"] for v in meta["vars"]], ["ch01", "ch02", "bp"])
        self.assertEqual(meta["vars"][2]["source"], "derived")
        self.assertIn("created_at", meta["vars"][2]["derived"])
        self.assertEqual(self.vals("bp", "r02")[3], 6.0)
        self.assertEqual((self.ds / "records" / "r02" / "bp.f32").stat().st_size, n * 4)

        meta = dataset.add_var("demo", "r02", "bp", np.zeros(n), "uV",
                               {"script": "bandpass", "sha256": "111111111111", "params": {"lo": 50}})
        self.assertEqual(len(meta["vars"]), 3)
        self.assertEqual(meta["vars"][2]["derived"]["params"], {"lo": 50})
        self.assertEqual(dataset.load_meta("demo", "r02")["vars"][2]["derived"]["sha256"], "111111111111")

        with self.assertRaises(ValueError):
            dataset.add_var("demo", "r02", "bad", np.zeros(7), "uV", {"params": {}})
        self.assertFalse((self.ds / "records" / "r02" / "bad.f32").exists())

    def test_list_datasets(self):
        self.assertEqual(dataset.list_datasets(), [{"name": "demo", "records": ["r01", "r02"]}])


class PerfTest(TempProject, unittest.TestCase):
    """AC6: 100 万サンプル × 20 変数、全範囲・px=1200 の m4 を 500 ms 未満で返す。"""

    def test_m4_full_1m_x20_under_500ms(self):
        demo.make(self.tmp / "ts" / "datasets" / "big", records=1, nvars=20, seconds=2000.0, srate=500.0)
        names = [f"ch{v:02d}" for v in range(1, 21)]
        dataset.m4("big", "r01", names, 0.0, 2000.0, 1200)  # ファイルを読み込んでおく
        t = time.perf_counter()
        res = dataset.m4("big", "r01", names, 0.0, 2000.0, 1200)
        ms = (time.perf_counter() - t) * 1000
        print(f"\nAC6: m4 1M x 20 px=1200 = {ms:.1f} ms")
        self.assertEqual(len(res["vars"]["ch01"]["min"]), 1200)
        self.assertLess(ms, 500, f"m4 が {ms:.0f} ms かかった（目標 500 ms 未満）")


if __name__ == "__main__":
    unittest.main()
