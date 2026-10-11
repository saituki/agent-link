"""アルゴリズム（G2-C1・G2-C4・G2-C5）。仕様（<name>.json）を持つスクリプト（<name>.py）の場所・一覧・検査。

置き場は 2 つ。同じ名前なら、プロジェクト（ts/scripts/）を使う。組み込みは BUILTIN_DIR。
（組み込みのフォルダに __init__.py は置かない。置くと package が algos.py を隠す）
"""

from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path

from . import events, paths

BUILTIN_DIR = Path(__file__).resolve().parent / "algos"
KINDS = ("channel", "event", "measure")
TARGETS = ("vars", "events", "none")
SPEC_KEYS = ("version", "title", "kind", "target", "desc", "params")
PARAM_KEYS = ("name", "type", "values", "min", "max", "unit", "desc", "default")
REQUEST_KEYS = ("records", "params", "target")
TARGET_KEYS = ("vars", "events")
MAX_TITLE = 80
MAX_DESC = 2000
MAX_STR = 2000
EVENT_ID_RE = re.compile(r"e-[0-9a-f]{12}")


class NoSpec(ValueError):
    """仕様（<name>.json）が無い。画面からは動かせない。CLI の sai-ts run は今までどおり動く。"""


def _is_int(v) -> bool:
    return isinstance(v, int) and not isinstance(v, bool)


def _only(d: dict, keys: tuple, what: str) -> None:
    unknown = sorted(set(d) - set(keys))
    if unknown:
        raise ValueError(f"{what}に未知の項目があります: {', '.join(unknown)}")


def _text(v, what: str, hi: int) -> str:
    if not isinstance(v, str) or len(v) > hi:
        raise ValueError(f"{what}: 文字列にしてください（{hi} 字まで）")
    return v


def locate(name: str) -> tuple[Path, str]:
    """スクリプトの場所と出どころ（"project" か "builtin"）。プロジェクトを先に探す。無ければ KeyError。"""
    paths.check_name(name)
    p = paths.inside(paths.scripts_dir() / f"{name}.py")
    if p.is_file():
        return p, "project"
    b = (BUILTIN_DIR / f"{name}.py").resolve()
    if b.is_relative_to(BUILTIN_DIR.resolve()) and b.is_file():
        return b, "builtin"
    raise KeyError(f"スクリプトがありません: {name}")


def check_spec(spec) -> dict:
    """仕様（G2-C1）を検査し、正規化した dict を返す。誤りは ValueError。"""
    if not isinstance(spec, dict):
        raise ValueError("仕様は object にしてください")
    _only(spec, SPEC_KEYS, "仕様")
    if not _is_int(spec.get("version")) or spec["version"] != 1:
        raise ValueError("version: 1 にしてください")
    title = _text(spec.get("title"), "title", MAX_TITLE)
    if not title:
        raise ValueError("title: 1 字以上にしてください")
    if spec.get("kind") not in KINDS:
        raise ValueError(f"kind: {' / '.join(KINDS)} のどれかにしてください")
    if spec.get("target") not in TARGETS:
        raise ValueError(f"target: {' / '.join(TARGETS)} のどれかにしてください")
    desc = _text(spec.get("desc", ""), "desc", MAX_DESC)

    params = spec.get("params", [])
    if not isinstance(params, list):
        raise ValueError("params: 配列にしてください")
    for p in params:
        if not isinstance(p, dict):
            raise ValueError("params: object にしてください")
        _only(p, PARAM_KEYS, "params")
        _text(p.get("desc", ""), "params.desc", MAX_DESC)
        if not isinstance(p.get("unit", ""), str):
            raise ValueError(f"params.unit: 文字列にしてください: {p.get('name')}")
    # 型・values・min/max・名前の重なりは、メトリクスの定義と同じ決まり（events._check_schema）
    events._check_schema({"metrics": params})
    for p in params:
        if "default" in p:
            events._metric(p["name"], p["default"], p)
    return {"version": 1, "title": title, "kind": spec["kind"], "target": spec["target"],
            "desc": desc, "params": params}


def _spec_of(name: str, jp: Path) -> dict:
    if not jp.is_file():
        raise NoSpec(f"仕様（{name}.json）がありません")
    try:
        raw = json.loads(jp.read_text(encoding="utf-8"))
    except (ValueError, OSError):
        raise ValueError(f"仕様（{name}.json）が読めません") from None
    try:
        return check_spec(raw)
    except ValueError as e:
        raise ValueError(f"仕様（{name}.json）: {e}") from None


def find(name: str) -> tuple[Path, dict]:
    """スクリプトのパスと検査済みの仕様。無ければ KeyError。仕様が無い（NoSpec）・読めないなら ValueError。"""
    path, _ = locate(name)
    return path, _spec_of(name, path.with_suffix(".json"))


def _entry(name: str, origin: str, path: Path) -> dict:
    out = {"name": name, "origin": origin, "title": "", "kind": "", "target": "", "desc": "",
           "params": [], "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "error": ""}
    try:
        spec = _spec_of(name, path.with_suffix(".json"))
    except ValueError as e:
        out["error"] = str(e)
        return out
    out.update(title=spec["title"], kind=spec["kind"], target=spec["target"], desc=spec["desc"],
               params=spec["params"])
    return out


def _listed(origin: str, p: Path) -> bool:
    # 置き場の外へ出るリンクは、一覧にも実行にも出さない（locate と同じ決まり）
    if not paths.valid_name(p.stem) or not p.is_file():
        return False
    root = BUILTIN_DIR if origin == "builtin" else paths.ts_root()
    return p.resolve().is_relative_to(root.resolve())


def list_algos() -> list[dict]:
    """組み込みとプロジェクトの一覧（名前の順）。同じ名前はプロジェクトが勝つ。.json の無いものは error を出す。"""
    found: dict[str, tuple[str, Path]] = {}
    for origin, d in (("builtin", BUILTIN_DIR), ("project", paths.scripts_dir())):
        if d.is_dir():
            for p in d.glob("*.py"):
                if _listed(origin, p):
                    found[p.stem] = (origin, p)
    return [_entry(n, o, p) for n, (o, p) in sorted(found.items())]


def _target(kind: str, t) -> dict:
    if not isinstance(t, dict):
        raise ValueError("target: object にしてください")
    _only(t, TARGET_KEYS, "target")
    vs, es = t.get("vars", []), t.get("events", [])
    if not isinstance(vs, list) or not all(paths.valid_name(x) for x in vs):
        raise ValueError("target.vars: 名前の配列にしてください")
    if not isinstance(es, list) or not all(isinstance(x, str) and EVENT_ID_RE.fullmatch(x) for x in es):
        raise ValueError("target.events: イベントの ID の配列にしてください")
    if kind == "vars":
        if not vs:
            raise ValueError("target.vars: ch を 1 つ以上選んでください")
        if es:
            raise ValueError("この仕様は target.events を受け取りません")
    elif kind == "events":
        if vs:
            raise ValueError("この仕様は target.vars を受け取りません")
    elif vs or es:
        raise ValueError("この仕様は対象を受け取りません")
    return {"vars": vs, "events": es}


def check_request(spec: dict, body: dict) -> dict:
    """要求（G2-C5）を仕様で検査する。既定の値で埋めて {records, params, target} を返す。"""
    if not isinstance(body, dict):
        raise ValueError("本文は object にしてください")
    _only(body, REQUEST_KEYS, "本文")
    records = body.get("records", [])
    if not isinstance(records, list) or not all(paths.valid_name(r) for r in records):
        raise ValueError("records: 名前の配列にしてください")
    given = body.get("params", {})
    if not isinstance(given, dict):
        raise ValueError("params: object にしてください")
    defs = {p["name"]: p for p in spec["params"]}
    for k in given:
        if k not in defs:
            raise ValueError(f"params.{k} は仕様にありません")
    out: dict = {}
    for p in spec["params"]:
        k = p["name"]
        if k in given:
            v = given[k]
            events._metric(k, v, p)  # 型・範囲・values（メトリクスと同じ決まり）
            if p["type"] == "string" and len(v) > MAX_STR:
                raise ValueError(f"params.{k}: {MAX_STR} 字までです")
            out[k] = v
        elif "default" in p:
            out[k] = p["default"]
        elif p["type"] == "boolean":
            out[k] = False
    return {"records": records, "params": out, "target": _target(spec["target"], body.get("target", {}))}
