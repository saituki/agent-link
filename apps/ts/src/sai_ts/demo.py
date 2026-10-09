"""合成データ（sai-ts demo）。試す・テストするための物。"""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np

from . import paths

SCHEMA = {
    "version": 1,
    "categories": [
        {"name": "burst", "color": "#d9534f", "visible": True, "desc": "短いバースト"},
        {"name": "noise", "color": "#6c757d", "visible": True, "desc": "雑音"},
    ],
    "metrics": [{"name": "amp", "type": "number", "min": 0, "max": 100, "unit": "", "desc": "振幅の倍率"}],
}


def make(ds_dir: Path, records=3, nvars=4, seconds=60.0, srate=500.0, seed=0) -> None:
    paths.check_name(ds_dir.name)
    ds_dir = paths.inside(ds_dir)
    if ds_dir.exists():
        raise FileExistsError(f"既にあります: {ds_dir.name}")

    rng = np.random.default_rng(seed)
    n = int(round(seconds * srate))
    t = np.arange(n) / srate
    base = np.sin(2 * np.pi * 10 * t)  # 10 Hz の正弦
    names = [f"ch{v:02d}" for v in range(1, nvars + 1)]

    for r in range(1, records + 1):
        rdir = ds_dir / "records" / f"r{r:02d}"
        rdir.mkdir(parents=True)
        # 記録ごとに 5 か所のバースト（80 Hz・振幅 5 倍・0.2 秒）。位置は全変数で共通
        y_base = base.copy()
        for s in rng.uniform(0, max(seconds - 0.2, 0), size=5):
            m = (t >= s) & (t < s + 0.2)
            y_base[m] = 5 * np.sin(2 * np.pi * 80 * t[m])
        vars_meta = []
        for name in names:
            y = y_base + 0.2 * rng.standard_normal(n)
            y.astype("<f4").tofile(rdir / f"{name}.f32")
            vars_meta.append({"name": name, "unit": "uV", "file": f"{name}.f32", "source": "raw"})
        meta = {"version": 1, "srate": float(srate), "n_samples": n, "vars": vars_meta}
        (rdir / "meta.json").write_text(json.dumps(meta, ensure_ascii=False, indent=2) + "\n",
                                        encoding="utf-8")

    (ds_dir / "schema.json").write_text(json.dumps(SCHEMA, ensure_ascii=False, indent=2) + "\n",
                                        encoding="utf-8")
