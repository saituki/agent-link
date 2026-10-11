"""組み込みのアルゴリズムが共有する数の部品（U07）。numpy だけ。

スクリプトからは `import _dsp` で読む（スクリプトのフォルダが sys.path の先頭に入るため）。
名前が `_` で始まるので、アルゴリズムの一覧には出ない。
"""

from __future__ import annotations

import numpy as np


def clean(x):
    """有限でない値（NaN・無限大）を 0 に置き換えた float64 の配列と、置き換えた数。"""
    x = np.asarray(x, dtype=np.float64)
    bad = ~np.isfinite(x)
    k = int(np.count_nonzero(bad))
    return (np.where(bad, 0.0, x) if k else x), k


def _gain(f, lo, hi, trans):
    # 通過域 [lo, hi] は 1。外側の trans Hz は余弦で 1→0。lo == 0 なら低域通過、hi >= ナイキストなら高域通過
    g = np.zeros_like(f)
    g[(f >= lo) & (f <= hi)] = 1.0
    if trans > 0:
        up = (f >= lo - trans) & (f < lo)
        g[up] = 0.5 * (1 - np.cos(np.pi * (f[up] - (lo - trans)) / trans))
        dn = (f > hi) & (f <= hi + trans)
        g[dn] = 0.5 * (1 + np.cos(np.pi * (f[dn] - hi) / trans))
    return g


def bandpass_fft(x, sr, lo, hi, trans):
    """0 位相の帯域通過（FFT）。cheby2 の形ではない（数値の一致は求めない）。"""
    # ponytail: 全長を 1 回で FFT する。1 億点を超えるなら overlap-add で区切る
    x = np.asarray(x, dtype=np.float64)
    n = len(x)
    f = np.fft.rfftfreq(n, 1 / sr)
    return np.fft.irfft(np.fft.rfft(x) * _gain(f, lo, hi, trans), n)


def envelope(x):
    """解析信号の絶対値（hilbert と同じ。FFT で作る）。"""
    x = np.asarray(x, dtype=np.float64)
    n = len(x)
    h = np.zeros(n)
    h[0] = 1.0
    if n % 2 == 0:
        h[1:n // 2] = 2.0
        h[n // 2] = 1.0
    else:
        h[1:(n + 1) // 2] = 2.0
    return np.abs(np.fft.ifft(np.fft.fft(x) * h))


def moving_mean(x, k):
    """中心の移動平均（累積和で O(n)）。k は奇数に直す（偶数なら 1 大きくする）。端は窓が欠けた分だけで割る。"""
    x = np.asarray(x, dtype=np.float64)
    k = max(1, int(k))
    if k % 2 == 0:
        k += 1
    h = k // 2
    n = len(x)
    c = np.concatenate(([0.0], np.cumsum(x)))
    i = np.arange(n)
    lo = np.maximum(i - h, 0)
    hi = np.minimum(i + h + 1, n)
    return (c[hi] - c[lo]) / (hi - lo)


def segments(mask):
    """真の連続区間 [(i0, i1), …]。i1 は含まない。"""
    m = np.concatenate(([False], np.asarray(mask, dtype=bool), [False])).astype(np.int8)
    d = np.diff(m)
    return list(zip(np.flatnonzero(d == 1).tolist(), np.flatnonzero(d == -1).tolist()))


def merge(segs, gap):
    """間隔（次の i0 − 前の i1）が gap 以下の区間を結ぶ。"""
    out = []
    for i0, i1 in segs:
        if out and i0 - out[-1][1] <= gap:
            out[-1] = (out[-1][0], i1)
        else:
            out.append((i0, i1))
    return out


def baseline(env, w, n):
    """移動平均（窓 w）の小さい順に、中心どうしが 2w 以上離れた点を最大 n 個取り、その平均。1 つも取れなければ None。"""
    m = moving_mean(env, w)
    picked = []
    # ponytail: 全長を並べ替える。1 億点を超えて重ければ argpartition で小さい方から取る
    for i in np.argsort(m, kind="stable"):
        if len(picked) >= n:
            break
        if all(abs(int(i) - j) >= 2 * w for j in picked):
            picked.append(int(i))
    if not picked:
        return None
    return float(np.mean(m[picked]))
