"""スペクトルの計算（spectrum.py・G2-C7）の試験。
実行: cd apps/ts && PYTHONPATH=src .venv/bin/python tests/test_spectrum.py
"""

import base64
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

import numpy as np  # noqa: E402

from sai_ts import dataset, demo, spectrum  # noqa: E402


class SpectrumTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        self._saved = os.environ.pop("SAI_PROJECT_ROOT", None)
        os.environ["SAI_PROJECT_ROOT"] = str(self.tmp)
        demo.make(self.tmp / "ts" / "datasets" / "demo", records=1, nvars=1, seconds=10.0, srate=500.0)
        # 0〜5 秒は 50 Hz、5〜10 秒は 150 Hz の正弦
        t = np.arange(5000) / 500.0
        sig = np.where(t < 5.0, np.sin(2 * np.pi * 50 * t), np.sin(2 * np.pi * 150 * t)).astype(np.float32)
        dataset.add_var("demo", "r01", "sig", sig, "uV", {"script": "t", "sha256": "0" * 12, "params": {}})

    def tearDown(self):
        os.environ.pop("SAI_PROJECT_ROOT", None)
        if self._saved is not None:
            os.environ["SAI_PROJECT_ROOT"] = self._saved
        shutil.rmtree(self.tmp, ignore_errors=True)

    def spec(self, **kw):
        args = dict(ds="demo", rec="r01", var="sig", t0=0.0, t1=10.0, cols=40,
                    fmin=0.0, fmax=250.0, win_ms=250.0)
        args.update(kw)
        return spectrum.spectrogram(**args)

    def image(self, r):
        return np.frombuffer(base64.b64decode(r["data"]), dtype=np.uint8).reshape(r["rows"], r["cols"])

    def test_peak_follows_the_tone(self):
        r = self.spec(cols=40)
        img = self.image(r)
        freqs = r["fmin"] + (np.arange(r["rows"]) + 0.5) * (r["fmax"] - r["fmin"]) / r["rows"]
        # 列 0〜9 は 0〜2.5 秒（50 Hz）、列 30〜39 は 7.5〜10 秒（150 Hz）。行 0 が fmin の側
        front = freqs[img[:, 2].argmax()]
        back = freqs[img[:, 37].argmax()]
        self.assertLess(abs(front - 50), 4, f"前半の最大 {front} Hz")
        self.assertLess(abs(back - 150), 4, f"後半の最大 {back} Hz")

    def test_shape_and_size(self):
        r = self.spec(cols=40, fmin=0, fmax=250)
        self.assertEqual((r["cols"], r["t0"], r["t1"]), (40, 0.0, 10.0))
        self.assertLessEqual(r["rows"], spectrum.MAX_ROWS)
        self.assertEqual(len(base64.b64decode(r["data"])), r["rows"] * r["cols"])
        self.assertLess(r["db_min"], r["db_max"])

    def test_rows_capped_at_max_rows(self):
        # 窓を長くすると周波数の数が多い。256 行を超えたら平均して 256 にする
        r = self.spec(cols=8, fmin=0, fmax=250, win_ms=4000)
        self.assertEqual(r["rows"], spectrum.MAX_ROWS)
        self.assertEqual(self.image(r).shape, (256, 8))

    def test_range_clamped_to_record(self):
        r = self.spec(t0=-5.0, t1=99.0, cols=4)
        self.assertEqual((r["t0"], r["t1"]), (0.0, 10.0))
        r = self.spec(fmax=9999.0)
        self.assertEqual(r["fmax"], 250.0)

    def test_errors(self):
        bad = [
            dict(var="nope"),
            dict(cols=0), dict(cols=spectrum.MAX_COLS + 1),
            dict(fmin=-1.0), dict(fmin=100.0, fmax=100.0), dict(fmin=300.0, fmax=400.0),
            dict(win_ms=0.0), dict(win_ms=10001.0), dict(win_ms=float("nan")),
            dict(t0=5.0, t1=5.0), dict(t0=float("inf")),
        ]
        for kw in bad:
            with self.subTest(kw=kw), self.assertRaises(ValueError):
                self.spec(**kw)


if __name__ == "__main__":
    unittest.main()
