"""組み込み: 振幅の閾値でイベントを検出する（HFO・U07）。仕様は hfo_detect.json。

RIPPLELAB の HFOL の手順（make_HFOLine.m）を numpy で。包絡の基準から閾値の区間を取り、結び、長さで捨て、振幅と長さで雑音にする。
"""

import json
import sys

from sai_ts import sdk
import _dsp

req = json.load(sys.stdin)
p = req["params"]
ds = req["dataset"]
lo, hi = p["low_hz"], p["high_hz"]
if not 0 < lo < hi:
    sys.stderr.write("low_hz は 0 より大きく、high_hz より小さくしてください\n")
    sys.exit(2)
cat, noise_cat = p["category"], p["noise_category"]

events, table, skipped = [], [], []
n_det = n_noise = nan_n = 0
for rec in req["records"] or sdk.records(ds):
    meta, data = sdk.load(ds, rec)
    sr = float(meta["srate"])
    k = round(p["smooth_ms"] * sr / 1000)
    w = max(1, round(p["base_win_s"] * sr))
    gap = round(p["merge_ms"] * sr / 1000)
    for var in req["target"]["vars"]:
        if var not in data:
            raise ValueError(f"記録 {rec} に変数がありません: {var}")
        x, bad = _dsp.clean(data[var])
        nan_n += bad
        env = _dsp.moving_mean(_dsp.envelope(_dsp.bandpass_fft(x, sr, lo, hi, p["trans_hz"])), k)
        base = _dsp.baseline(env, w, p["base_n"])
        if base is None or base <= 0:
            skipped.append(f"{rec}/{var}")  # 基準が取れない（または 0）。この ch は飛ばす
            continue
        segs = _dsp.merge(_dsp.segments(env >= base * p["thr_min"]), gap)
        rec_det = rec_noise = 0
        for i0, i1 in segs:
            dur = (i1 - i0) / sr
            if dur < p["min_cycles"] / lo:
                continue
            seg = env[i0:i1]
            amax = float(seg.max())
            noise = amax >= base * p["thr_max"] or dur >= p["max_cycles"] / lo
            if noise:
                rec_noise += 1
                if not noise_cat:
                    continue
            else:
                rec_det += 1
            events.append({"type": "area", "record": rec, "vars": [var], "t0": i0 / sr, "t1": i1 / sr,
                           "category": noise_cat if noise else cat, "text": "",
                           "metrics": {"amp_max": amax, "amp_mean": float(seg.mean()),
                                       "ratio_max": amax / base, "base": base}})
        n_det += rec_det
        n_noise += rec_noise
        table.append([rec, var, rec_det, rec_noise, round(base, 4)])

sdk.ensure_category(ds, cat, "#f59e0b", type="area", ch="single")
if noise_cat:
    sdk.ensure_category(ds, noise_cat, "#f87171", type="area", ch="single")
sdk.ensure_metrics(ds, [
    {"name": "amp_max", "type": "number", "min": 0, "unit": "", "desc": "区間の最大の包絡（帯域通過後の ch の単位）"},
    {"name": "amp_mean", "type": "number", "min": 0, "unit": "", "desc": "区間の平均の包絡"},
    {"name": "ratio_max", "type": "number", "min": 0, "unit": "倍", "desc": "最大の包絡 ÷ 基準振幅"},
    {"name": "base", "type": "number", "min": 0, "unit": "", "desc": "基準振幅（移動平均の小さい点の平均）"},
])
rows = sdk.add_events(ds, events)

msg = f"{n_det} 件を検出しました（雑音 {n_noise} 件{'' if noise_cat else 'は捨てました'}）"
if skipped:
    msg += f"。基準が取れず飛ばした: {', '.join(skipped)}"
if nan_n:
    msg += f"。NaN・無限大 {nan_n} 点を 0 にしました"
print(json.dumps({"message": msg, "events": len(rows),
                  "table": {"columns": ["record", "var", "events", "noise", "base"], "rows": table}},
                 ensure_ascii=False))
