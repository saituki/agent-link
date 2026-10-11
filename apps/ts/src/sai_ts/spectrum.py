"""スペクトル（時間周波数）の図（共有契約 G2-C7）。列の中心のまわりだけを計算し、0〜255 の画素にする。"""

from __future__ import annotations

import base64
import math

import numpy as np

from . import dataset

MAX_COLS = 2048
MAX_ROWS = 256
MAX_WIN_MS = 10000
MIN_NPER = 16
MAX_NPER = 65536
MAX_CELLS = 4_000_000  # 一度に作る行列（列 × 窓の点数）の大きさ。超えたら列を区切って計算する


def spectrogram(ds, rec, var, t0, t1, cols, fmin, fmax, win_ms) -> dict:
    meta = dataset.load_meta(ds, rec)
    if var not in {v["name"] for v in meta["vars"]}:
        raise ValueError(f"変数がありません: {var}")
    if not 1 <= cols <= MAX_COLS:
        raise ValueError(f"cols は 1〜{MAX_COLS} にしてください")
    if not all(math.isfinite(v) for v in (t0, t1, fmin, fmax, win_ms)):
        raise ValueError("t0・t1・fmin・fmax・win_ms は数にしてください")
    if not 0 < win_ms <= MAX_WIN_MS:
        raise ValueError(f"win_ms は 0 より大きく {MAX_WIN_MS} 以下にしてください")
    sr = float(meta["srate"])
    n = int(meta["n_samples"])
    dur = n / sr
    t0 = min(max(t0, 0.0), dur)
    t1 = min(max(t1, 0.0), dur)
    if t1 <= t0:
        raise ValueError("t1 は t0 より大きくしてください")
    fmax = min(fmax, sr / 2)
    if not 0 <= fmin < fmax:
        raise ValueError("fmin・fmax は 0 ≤ fmin < fmax にしてください（fmax は sr/2 まで）")

    nper = min(MAX_NPER, max(MIN_NPER, round(win_ms * sr / 1000)))
    freqs = np.arange(nper // 2 + 1) * (sr / nper)
    band = (freqs >= fmin) & (freqs <= fmax)
    if not band.any():
        raise ValueError("帯域の中に周波数がありません（win_ms を大きくしてください）")

    x = dataset.open_var(ds, rec, var)
    centers = t0 + (np.arange(cols) + 0.5) * (t1 - t0) / cols
    start = np.round(centers * sr).astype(np.int64) - nper // 2
    win = np.hanning(nper)
    db = np.empty((cols, int(band.sum())))
    per = max(1, MAX_CELLS // nper)
    for c0 in range(0, cols, per):
        idx = start[c0:c0 + per, None] + np.arange(nper)[None, :]
        ok = (idx >= 0) & (idx < n)  # 記録の外は 0 で埋める（計算には入れない）
        seg = np.asarray(x[np.clip(idx, 0, n - 1)], dtype=np.float64)
        ok &= np.isfinite(seg)
        seg = np.where(ok, seg, 0.0)
        mean = seg.sum(axis=1, keepdims=True) / np.maximum(ok.sum(axis=1, keepdims=True), 1)
        seg = np.where(ok, seg - mean, 0.0) * win
        p = np.abs(np.fft.rfft(seg, axis=1)) ** 2
        db[c0:c0 + per] = 10 * np.log10(p[:, band] + 1e-20)

    rows = db.shape[1]
    if rows > MAX_ROWS:
        # ponytail: 隣どうしを平均して減らす。細かい周波数の並びは失う
        edges = np.linspace(0, rows, MAX_ROWS + 1).astype(np.int64)
        db = np.stack([db[:, a:b].mean(axis=1) for a, b in zip(edges[:-1], edges[1:])], axis=1)
        rows = MAX_ROWS

    lo, hi = (float(v) for v in np.percentile(db, [5, 99.5]))
    if hi - lo < 1e-12:
        hi = lo + 1
    px = np.clip((db - lo) / (hi - lo) * 255, 0, 255)
    img = np.ascontiguousarray(np.round(px).astype(np.uint8).T)  # 行 0 が fmin
    return {"t0": t0, "t1": t1, "cols": cols, "rows": rows, "fmin": fmin, "fmax": fmax,
            "db_min": lo, "db_max": hi, "data": base64.b64encode(img.tobytes()).decode("ascii")}
