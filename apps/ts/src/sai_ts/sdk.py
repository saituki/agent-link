"""スクリプトと拡張の server.py が使う sdk（C9・G2-C6）。読むのは自由。書くのは events と schema と派生変数だけ。

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

import json
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


def _script_name() -> str:
    # SAI_TS_SOURCE は script:<name>@<sha8> の形（スクリプトから呼ばれたとき）
    src = os.environ.get("SAI_TS_SOURCE", "")
    if src.startswith("script:") and "@" in src:
        return src[len("script:"):].rsplit("@", 1)[0]
    return "agent"


def events(name: str, **filters) -> list[dict]:
    return _ev.list_events(name, **filters)


def add_events(name: str, evs: list[dict]) -> list[dict]:
    return _ev.add_events(name, evs, source=_source())


def add_var(name: str, record: str, var: str, values: np.ndarray, unit: str,
            params: dict | None = None, inputs: list[str] | None = None) -> dict:
    inputs = [] if inputs is None else inputs
    if not isinstance(inputs, list) or not all(isinstance(x, str) for x in inputs):
        raise ValueError("inputs は ch の名前の配列にしてください")
    derived = {"script": _script_name(), "sha256": os.environ.get("SAI_TS_SHA256", ""),
               "params": params or {}, "inputs": inputs, "run": os.environ.get("SAI_TS_RUN", "")}
    return _data.add_var(name, record, var, values, unit, derived)


def ensure_category(name: str, cat: str, color: str, type: str | None = None, ch: str | None = None) -> None:
    s = _ev.load_schema(name)
    if any(c["name"] == cat for c in s["categories"]):
        return
    item = {"name": cat, "color": color}
    if type is not None:
        item["type"] = type
    if ch is not None:
        item["ch"] = ch
    # ponytail: 読んでから書くので、同時に 2 人が足すと片方が消える。スクリプトは順に動くので今は受け入れる
    _ev.save_schema(name, {**s, "categories": [*s["categories"], item]})


def ensure_metrics(name: str, defs: list[dict]) -> None:
    s = _ev.load_schema(name)
    have = {m["name"] for m in s["metrics"]}
    new = [d for d in defs if d.get("name") not in have]
    if new:
        _ev.save_schema(name, {**s, "metrics": [*s["metrics"], *new]})


def update_metrics(name: str, values: dict[str, dict]) -> int:
    """{イベントの ID: {メトリクス: 値}} を、各イベントの metrics に重ねて書く。直した件数を返す。"""
    if not isinstance(values, dict) or not all(isinstance(v, dict) for v in values.values()):
        raise ValueError("values は {id: {メトリクス: 値}} の object にしてください")
    cur = {r["id"]: r for r in _ev.list_events(name)}
    patches = {}
    for id, mets in values.items():
        if id not in cur:
            raise KeyError(f"イベントがありません: {id}")
        patches[id] = {"metrics": {**cur[id]["metrics"], **mets}}
    return len(_ev.update_events(name, patches, source=_source()))


def delete_events(name: str, ids: list[str]) -> int:
    """イベントをまとめて消す。消した件数を返す。source は sdk.add_events と同じ決まり。"""
    return len(_ev.delete_events(name, ids, source=_source()))


def view() -> dict:
    return _view.load_view()


def target() -> dict:
    # 標準入力は script が読み切るので、対象は環境変数で受け取る（大きすぎると空。req["target"] を読むこと）
    return json.loads(os.environ.get("SAI_TS_TARGET") or "{}")
