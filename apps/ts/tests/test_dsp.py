"""組み込みの共通の部品（AC1）: バンドパス・包絡・移動平均・区間・結合・基準を、小さい手計算の例と数で確かめる。"""

import sys
import unittest
from pathlib import Path

import numpy as np

SRC = Path(__file__).resolve().parent.parent / "src"
sys.path.insert(0, str(SRC))
sys.path.insert(0, str(SRC / "sai_ts" / "algos"))

import _dsp  # noqa: E402


class BandpassTest(unittest.TestCase):
    def test_passes_band_and_removes_outside(self):  # AC1（bandpass_fft）
        sr = 500.0
        t = np.arange(10000) / sr  # 20 秒。10 Hz と 100 Hz は整数周期
        x = np.sin(2 * np.pi * 10 * t) + np.sin(2 * np.pi * 100 * t)
        y = _dsp.bandpass_fft(x, sr, 80, 150, 5)
        X, Y = np.fft.rfft(x), np.fft.rfft(y)
        self.assertLessEqual(abs(Y[200]) / abs(X[200]), 0.01)  # 10 Hz は 40 dB 以上下がる
        self.assertAlmostEqual(abs(Y[2000]) / abs(X[2000]), 1.0, delta=0.05)  # 100 Hz は残る

    def test_low_pass_high_pass_and_full_band(self):
        sr = 500.0
        t = np.arange(10000) / sr  # 10 Hz と 100 Hz は整数周期
        x = np.sin(2 * np.pi * 10 * t) + np.sin(2 * np.pi * 100 * t)
        low = _dsp.bandpass_fft(x, sr, 0, 50, 5)  # lo == 0 は低域通過
        self.assertLess(np.max(np.abs(low - np.sin(2 * np.pi * 10 * t))), 1e-6)
        high = _dsp.bandpass_fft(x, sr, 50, 250, 5)  # hi がナイキスト以上は高域通過
        self.assertLess(np.max(np.abs(high - np.sin(2 * np.pi * 100 * t))), 1e-6)
        full = _dsp.bandpass_fft(x, sr, 0, 250, 0)  # 0〜ナイキストは全部通る
        self.assertLess(np.max(np.abs(full - x)), 1e-9)


class EnvelopeTest(unittest.TestCase):
    def test_envelope_of_sine_is_amplitude(self):  # AC1（envelope）
        sr = 500.0
        t = np.arange(5000) / sr  # 10 秒。100 Hz は整数周期
        env = _dsp.envelope(3 * np.sin(2 * np.pi * 100 * t))
        n = len(env)
        mid = env[n // 20: n - n // 20]  # 端の 5% を除く
        self.assertLess(np.max(np.abs(mid - 3)), 0.1)


class MovingMeanTest(unittest.TestCase):
    def test_edges_divide_by_available_count(self):  # AC1（moving_mean）
        self.assertEqual(_dsp.moving_mean([1, 2, 3, 4, 5], 3).tolist(), [1.5, 2, 3, 4, 4.5])

    def test_even_window_becomes_odd(self):
        self.assertEqual(_dsp.moving_mean([1, 2, 3, 4, 5], 2).tolist(), _dsp.moving_mean([1, 2, 3, 4, 5], 3).tolist())

    def test_empty_input(self):
        self.assertEqual(_dsp.moving_mean([], 3).size, 0)


class SegmentsTest(unittest.TestCase):
    def test_runs_are_half_open(self):  # AC1（segments・merge）
        self.assertEqual(_dsp.segments([0, 1, 1, 0, 1, 1, 1]), [(1, 3), (4, 7)])
        self.assertEqual(_dsp.segments([False, False]), [])

    def test_merge_joins_gaps_up_to_gap(self):
        segs = [(1, 3), (4, 7), (9, 10)]
        self.assertEqual(_dsp.merge(segs, 1), [(1, 7), (9, 10)])  # 間隔 1 は結ぶ。2 は結ばない
        self.assertEqual(_dsp.merge(segs, 0), segs)


class BaselineTest(unittest.TestCase):
    def test_picks_spaced_low_points(self):  # AC1（baseline）
        env = np.array([1, 1, 9, 9, 9, 9, 9, 9, 9, 9.0])
        # 窓 1 は移動平均を使わない。小さい 1 の点は 0 と 1（間隔 1 < 2 なので 1 は取らない）。2 点目は 2 の 9
        self.assertEqual(_dsp.baseline(env, 1, 2), 5.0)

    def test_none_when_nothing_to_pick(self):
        self.assertIsNone(_dsp.baseline(np.zeros(0), 1, 10))


class CleanTest(unittest.TestCase):
    def test_nonfinite_become_zero_and_are_counted(self):
        y, k = _dsp.clean([1.0, np.nan, np.inf, 2.0])
        self.assertEqual((y.tolist(), k), ([1.0, 0.0, 0.0, 2.0], 2))


if __name__ == "__main__":
    unittest.main()
