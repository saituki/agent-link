"""スクリプトと拡張の server.py が使う sdk（C9）。読むのは自由。書くのは add_events と add_var だけ。

スクリプトの最小の例（agent が読む。sai-ts run burst --dataset demo で動く）:

    import json, sys
    import numpy as np
    from sai_ts import sdk
    req = json.load(sys.stdin)
    ds = req["dataset"]
    found = []
    for rec in req["records"] or sdk.records(ds):
        meta, data = sdk.load(ds, rec)
        x = data["ch01"]; sr = meta["srate"]
        idx = np.flatnonzero(np.abs(x) > req["params"].get("thr", 3.0) * np.std(x))
        for i in idx[::int(sr)]:   # 1 秒に 1 件まで
            found.append({"type": "line", "record": rec, "vars": ["ch01"], "t": float(i / sr), "category": "burst", "text": "", "metrics": {}})
    sdk.add_events(ds, found)
    print(json.dumps({"added": len(found)}))
"""

from __future__ import annotations

import os
from pathlib import Path

import numpy as np

from . import dataset as _data, events as _ev, paths, view as _view


def dataset(name: str) -> Path:
    return paths.dataset_dir(name)


def records(name: str) -> list[str]:
    rdir = paths.dataset_dir(name) / "records"
    return sorted(p.name for p in rdir.iterdir() if p.is_dir()) if rdir.is_dir() else []


def load(name: str, record: str) -> tuple[dict, dict]:
    meta = _data.load_meta(name, record)
    data = {v["name"]: _data.open_var(name, record, v["name"]) for v in meta["vars"]}
    return meta, data


def _source() -> str:
    return os.environ.get("SAI_TS_SOURCE", "agent")


def _origin() -> tuple[str, str]:
    # SAI_TS_SOURCE は script:<name>@<sha8> の形（スクリプトから呼ばれたとき）
    src = os.environ.get("SAI_TS_SOURCE", "")
    if src.startswith("script:") and "@" in src:
        name, sha = src[len("script:"):].rsplit("@", 1)
        return name, sha
    return "agent", ""


def events(name: str, **filters) -> list[dict]:
    return _ev.list_events(name, **filters)


def add_events(name: str, evs: list[dict]) -> list[dict]:
    return _ev.add_events(name, evs, source=_source())


def add_var(name: str, record: str, var: str, values: np.ndarray, unit: str,
            params: dict | None = None) -> dict:
    script, sha = _origin()
    derived = {"script": script, "sha256": sha, "params": params or {}}
    return _data.add_var(name, record, var, values, unit, derived)


def view() -> dict:
    return _view.load_view()
