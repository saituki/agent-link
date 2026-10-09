"""画面の状態（C6・view.json）。人の画面が書き、agent が CLI で読んで書き換える。"""

from __future__ import annotations

import json

from . import paths
from .events import is_num, now, write_json

VIEW_KEYS = ("dataset", "record", "t0", "t1", "vars", "hidden_categories",
             "selection", "selected_events")
_SERVER_KEYS = ("updated_at", "updated_by")  # 書き手が決める（本文では無視）


def load_view() -> dict:
    p = paths.view_path()
    return json.loads(p.read_text(encoding="utf-8")) if p.is_file() else {}


def _names(xs, what: str) -> None:
    if not isinstance(xs, list) or not all(paths.valid_name(x) for x in xs):
        raise ValueError(f"{what}: 名前の配列にしてください")


def _span(d, what: str) -> None:
    if not (isinstance(d, dict) and is_num(d.get("t0")) and is_num(d.get("t1"))
            and d["t0"] < d["t1"]):
        raise ValueError(f"{what}: t0・t1 は数で、t0 < t1 にしてください")


def _check(v: dict) -> None:
    for k in ("dataset", "record"):
        if v.get(k) is not None and not paths.valid_name(v[k]):
            raise ValueError(f"{k}: 名前の形が違います: {v[k]}")
    if "t0" in v or "t1" in v:
        _span(v, "t0・t1")
    _names(v.get("vars", []), "vars")
    _names(v.get("hidden_categories", []), "hidden_categories")
    _names(v.get("selected_events", []), "selected_events")
    sel = v.get("selection")
    if sel is not None:
        _span(sel, "selection")
        _names(sel.get("vars", []), "selection.vars")


def save_view(patch: dict, by: str) -> dict:
    if by not in ("human", "agent"):
        raise ValueError("by: human か agent にしてください")
    if not isinstance(patch, dict):
        raise ValueError("本文は object にしてください")
    body = {k: v for k, v in patch.items() if k not in _SERVER_KEYS}
    unknown = sorted(set(body) - set(VIEW_KEYS))
    if unknown:
        raise ValueError(f"未知の項目です: {', '.join(unknown)}")
    old = load_view()
    v = {**old, **body}  # 足りない項目は前の値を残す
    # 記録（や dataset）が変わったら、前の選択は別の記録の区間になるので消す
    if "selection" not in body and any(k in body and body[k] != old.get(k) for k in ("record", "dataset")):
        v["selection"] = None
    _check(v)
    v["updated_at"] = now()
    v["updated_by"] = by
    paths.view_path().parent.mkdir(parents=True, exist_ok=True)
    write_json(paths.view_path(), v)
    return v
