"""組み込み: イベントの特徴量（U07）。仕様は event_features.json。区間の ch から指標を計算し、イベントの metrics に書く。"""

import json
import sys

import numpy as np

from sai_ts import sdk
import _dsp

req = json.load(sys.stdin)
p = req["params"]
ds = req["dataset"]
want = req["target"]["events"]
if want:
    by_id = {e["id"]: e for e in sdk.events(ds)}
    missing = [i for i in want if i not in by_id]
    if missing:
        raise ValueError(f"イベントがありません: {missing[0]}")
    cand = [by_id[i] for i in want]
else:
    recs = set(req["records"])
    cand = [e for e in sdk.events(ds) if not recs or e["record"] in recs]
line_n = sum(e["type"] == "line" for e in cand)
todo = [e for e in cand if e["type"] == "area"]

cache, unit, rows, updates, nan_n = {}, None, [], {}, 0
for e in todo:
    rec = e["record"]
    if rec not in cache:
        cache[rec] = sdk.load(ds, rec)
    meta, data = cache[rec]
    sr = float(meta["srate"])
    n = int(meta["n_samples"])
    ch = e["vars"][0] if e["vars"] else (p["var"] or meta["vars"][0]["name"])
    if ch not in data:
        raise ValueError(f"記録 {rec} に変数がありません: {ch}")
    ch_unit = next(v["unit"] for v in meta["vars"] if v["name"] == ch)
    unit = ch_unit if unit is None else unit

    # 区間の前後の幅を足す。空にならないよう、少なくとも 1 点は取る
    pad = p["pad_ms"] / 1000
    i0 = min(max(0, int(np.floor((e["t0"] - pad) * sr))), n - 1)
    i1 = min(n, max(i0 + 1, int(np.ceil((e["t1"] + pad) * sr))))
    x, bad = _dsp.clean(data[ch][i0:i1])
    nan_n += bad

    c = np.sign(x - x.mean())
    c = c[c != 0]
    feat = {
        "duration_s": e["t1"] - e["t0"],
        "peak_abs": float(np.max(np.abs(x))),
        "rms": float(np.sqrt(np.mean(x * x))),
        "line_length": float(np.sum(np.abs(np.diff(x)))),
        "zero_cross": int(np.count_nonzero(np.diff(c))),
    }
    if len(x) >= 8:  # 点が少ないと周波数が決まらない。書かない
        spec = np.abs(np.fft.rfft((x - x.mean()) * np.hanning(len(x))))
        freq = np.fft.rfftfreq(len(x), 1 / sr)
        feat["peak_hz"] = float(freq[1 + int(np.argmax(spec[1:]))])
    updates[e["id"]] = feat
    rows.append([e["id"], rec, e["t0"], feat["duration_s"], feat["peak_abs"], feat["rms"], feat.get("peak_hz")])

u = unit or ""
sdk.ensure_metrics(ds, [
    {"name": "duration_s", "type": "number", "min": 0, "unit": "s", "desc": "区間の長さ"},
    {"name": "peak_abs", "type": "number", "min": 0, "unit": u, "desc": "区間の最大の絶対値"},
    {"name": "rms", "type": "number", "min": 0, "unit": u, "desc": "区間の二乗平均平方根"},
    {"name": "line_length", "type": "number", "min": 0, "unit": u, "desc": "隣り合う点の差の絶対値の和"},
    {"name": "peak_hz", "type": "number", "min": 0, "unit": "Hz", "desc": "平均を引き hann をかけた後の最大のピーク周波数（0 Hz を除く）"},
    {"name": "zero_cross", "type": "integer", "min": 0, "desc": "平均を引いた値の符号が変わった回数"},
])
if updates:
    sdk.update_metrics(ds, updates)

msg = f"{len(updates)} 件に指標を書きました"
if line_n:
    msg += f"。line のイベント {line_n} 件は飛ばしました"
if nan_n:
    msg += f"。NaN・無限大 {nan_n} 点を 0 にしました"
print(json.dumps({"message": msg,
                  "table": {"columns": ["id", "record", "t0", "duration_s", "peak_abs", "rms", "peak_hz"],
                            "rows": rows}}, ensure_ascii=False))
