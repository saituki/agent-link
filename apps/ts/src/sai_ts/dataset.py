"""データの読み込み・M4 の間引き・派生変数の書き込み（C2・C3）。"""

from __future__ import annotations

import json
import math
import os
from datetime import datetime, timezone
from pathlib import Path

import numpy as np

from . import paths

MAX_VARS = 64
MAX_PX = 8192


def list_datasets() -> list[dict]:
    root = paths.datasets_dir()
    if not root.is_dir():
        return []
    out = []
    for d in sorted(p for p in root.iterdir() if p.is_dir() and paths.valid_name(p.name)):
        recs = d / "records"
        names = sorted(p.name for p in recs.iterdir() if p.is_dir()) if recs.is_dir() else []
        out.append({"name": d.name, "records": names})
    return out


def load_meta(ds: str, rec: str) -> dict:
    path = paths.record_dir(ds, rec) / "meta.json"
    if not path.is_file():
        raise KeyError(f"meta.json がありません: {rec}")
    meta = json.loads(path.read_text(encoding="utf-8"))
    if meta.get("version") != 1:
        raise ValueError(f"meta.json の version が違います: {meta.get('version')}")
    return meta


def _find_var(meta: dict, var: str) -> dict:
    for v in meta["vars"]:
        if v["name"] == var:
            return v
    raise KeyError(f"変数がありません: {var}")


def open_var(ds: str, rec: str, var: str) -> np.memmap:
    meta = load_meta(ds, rec)
    info = _find_var(meta, var)
    path = paths.inside(paths.record_dir(ds, rec) / info["file"])
    n = int(meta["n_samples"])
    if not path.is_file() or path.stat().st_size != n * 4:
        raise ValueError(f"変数のファイルの大きさが違います: {var}")
    return np.memmap(path, dtype="<f4", mode="r", shape=(n,))


def _round(arr) -> list:
    # 有効数字 6 桁。NaN と無限大は null（JSON にできないため）
    return [float(f"{v:.6g}") if math.isfinite(v) else None
            for v in np.asarray(arr, dtype=np.float64).tolist()]


def m4(ds: str, rec: str, vars: list[str], t0: float, t1: float, px: int) -> dict:
    # ponytail: 毎回ファイルから計算する（タイルのキャッシュ無し）。遅ければ事前計算のピラミッドを足す
    if not 1 <= len(vars) <= MAX_VARS:
        raise ValueError(f"vars は 1〜{MAX_VARS} 個にしてください")
    if not 1 <= px <= MAX_PX:
        raise ValueError(f"px は 1〜{MAX_PX} にしてください")
    if not (math.isfinite(t0) and math.isfinite(t1)):
        raise ValueError("t0・t1 は数にしてください")
    meta = load_meta(ds, rec)
    srate = float(meta["srate"])
    n = int(meta["n_samples"])
    dur = n / srate
    t0 = min(max(t0, 0.0), dur)
    t1 = min(max(t1, 0.0), dur)
    if t1 <= t0:
        raise ValueError("t1 は t0 より大きくしてください")

    i0 = max(0, int(math.floor(t0 * srate)))
    i1 = min(n, int(math.ceil(t1 * srate)))  # [i0, i1)
    n_in = i1 - i0
    res = {"t0": t0, "t1": t1, "px": px, "srate": srate, "vars": {}}

    if n_in <= 4 * px:
        for name in vars:
            data = open_var(ds, rec, name)
            res["vars"][name] = {"mode": "raw", "start": i0 / srate, "dt": 1 / srate,
                                 "y": _round(data[i0:i1])}
        return res

    # 列 k は [edges[k], edges[k+1])。整数で割るので列の境目が正確になる
    edges = i0 + (np.arange(px + 1, dtype=np.int64) * n_in) // px
    starts = edges[:-1] - i0
    nonempty = edges[1:] > edges[:-1]  # n_in > 4 px なので常に真。念のため
    last_idx = np.maximum(edges[1:] - 1, edges[:-1]) - i0
    for name in vars:
        seg = open_var(ds, rec, name)[i0:i1]
        # fmin / fmax は NaN を無視する。全部 NaN の列は NaN のまま → null
        mn = np.fmin.reduceat(seg, starts)
        mx = np.fmax.reduceat(seg, starts)
        first = seg[starts]
        last = seg[last_idx]
        mn, mx, first, last = (np.where(nonempty, a, np.nan) for a in (mn, mx, first, last))
        res["vars"][name] = {"mode": "m4", "first": _round(first), "min": _round(mn),
                             "max": _round(mx), "last": _round(last)}
    return res


def _write_atomic(path: Path, write) -> None:
    tmp = path.with_name(path.name + ".tmp")
    write(tmp)
    os.replace(tmp, path)


def add_var(ds: str, rec: str, var: str, values: np.ndarray, unit: str, derived: dict) -> dict:
    """派生変数を足す（同名は上書き）。返り値は新しい meta。"""
    paths.check_name(var)
    rdir = paths.record_dir(ds, rec)
    meta = load_meta(ds, rec)
    n = int(meta["n_samples"])
    arr = np.asarray(values, dtype="<f4")
    if arr.shape != (n,):
        raise ValueError(f"長さが n_samples（{n}）と違います: {arr.shape}")

    fname = f"{var}.f32"
    _write_atomic(paths.inside(rdir / fname), lambda p: arr.tofile(p))

    derived = {"created_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), **derived}
    entry = {"name": var, "unit": unit, "file": fname, "source": "derived", "derived": derived}
    vs = meta["vars"]
    idx = next((i for i, v in enumerate(vs) if v["name"] == var), None)
    if idx is None:
        vs.append(entry)
    else:
        vs[idx] = entry

    body = json.dumps(meta, ensure_ascii=False, indent=2) + "\n"
    _write_atomic(rdir / "meta.json", lambda p: p.write_text(body, encoding="utf-8"))
    return meta
