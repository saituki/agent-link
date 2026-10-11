"""組み込み: バンドパスと包絡（U07）。仕様は bandpass.json。対象の ch ごとに、帯域通過の ch と包絡の ch を足す。"""

import json
import sys

from sai_ts import paths, sdk
import _dsp

req = json.load(sys.stdin)
p = req["params"]
ds = req["dataset"]
lo, hi, trans = p["low_hz"], p["high_hz"], p["trans_hz"]
if not lo < hi:
    sys.stderr.write("low_hz は high_hz より小さくしてください\n")
    sys.exit(2)
tag = f"bp{lo:g}-{hi:g}"
env_tag = f"env{lo:g}-{hi:g}"

added, skipped, recs, nan_n = {}, [], set(), 0
for rec in req["records"] or sdk.records(ds):
    meta, data = sdk.load(ds, rec)
    sr = float(meta["srate"])
    units = {v["name"]: v["unit"] for v in meta["vars"]}
    k = round(p["smooth_ms"] * sr / 1000)
    for var in req["target"]["vars"]:
        if var not in data:
            raise ValueError(f"記録 {rec} に変数がありません: {var}")
        names = [f"{var}_{tag}"] + ([f"{var}_{env_tag}"] if p["envelope"] else [])
        if not all(paths.valid_name(n) for n in names):
            skipped.append(var)  # 名前が長すぎる（64 字）。この ch は飛ばす
            continue
        x, bad = _dsp.clean(data[var])
        nan_n += bad
        bp = _dsp.bandpass_fft(x, sr, lo, hi, trans)
        values = [bp] + ([_dsp.moving_mean(_dsp.envelope(bp), k)] if p["envelope"] else [])
        for n, v in zip(names, values):
            sdk.add_var(ds, rec, n, v, units[var], params=p, inputs=[var])
            added[n] = True
        recs.add(rec)

msg = f"{len(added)} ch を足しました（{len(recs)} 記録）"
if skipped:
    msg += f"。名前が長すぎるため飛ばした ch: {', '.join(skipped)}"
if nan_n:
    msg += f"。NaN・無限大 {nan_n} 点を 0 にしました"
print(json.dumps({"message": msg, "vars": list(added)}, ensure_ascii=False))
