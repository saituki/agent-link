"""イベント（C4）・スキーマ・BIDS 互換の書き出し（C5）。正は追記だけの events.jsonl。"""

from __future__ import annotations

import fcntl
import json
import logging
import math
import os
import re
import secrets
import tempfile
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path

from . import dataset, paths

EVENTS = "events.jsonl"
SCHEMA = "schema.json"
TYPES = ("line", "area")
CH_MODES = ("single", "all")
METRIC_TYPES = ("number", "integer", "string", "boolean", "enum")
MAX_TEXT = 20000
SOURCE_RE = re.compile(r"human|agent|script:[A-Za-z0-9][A-Za-z0-9_.-]{0,63}@[0-9a-f]{8}")
COLOR_RE = re.compile(r"#[0-9A-Fa-f]{6}")
_BOOK = ("op", "id", "rev", "source", "created_at")  # 来歴は書き手が決める（本文では無視）
_FIELDS = ("type", "record", "vars", "t", "t0", "t1", "category", "text", "metrics")
# TSV の 1 セルで困る文字（タブ・改行。Unicode の行区切りも含む）
_TSV_BAD = re.compile(r"[\t\r\n\x0b\x0c\x1c-\x1e\x85  ]")


def now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def is_num(v) -> bool:
    if isinstance(v, bool):
        return False
    if isinstance(v, int):
        return True
    return isinstance(v, float) and math.isfinite(v)


def write_json(path: Path, obj) -> None:
    """一時ファイルに書いてから置き換える。一時名は毎回違うので同時の書き込みでも衝突しない。"""
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=path.name + ".", suffix=".tmp")
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        f.write(json.dumps(obj, ensure_ascii=False, indent=2) + "\n")
    os.replace(tmp, path)


def _check_source(source) -> None:
    if not isinstance(source, str) or not SOURCE_RE.fullmatch(source):
        raise ValueError(f"source の形が違います: {source}")


# --- 読み書き（行の解釈・ロック） ---

def _read(ds: str) -> str:
    path = paths.dataset_dir(ds) / EVENTS
    if not path.is_file():
        return ""
    with open(path, encoding="utf-8") as f:
        fcntl.flock(f, fcntl.LOCK_SH)
        return f.read()


@contextmanager
def _journal(ds: str):
    """events.jsonl を排他ロックして開く。yield は (今の状態, 追記の関数)。
    読みも書きもロックの中で行うので、rev の採番が競合しない。"""
    path = paths.dataset_dir(ds) / EVENTS
    with open(path, "a+", encoding="utf-8") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        f.seek(0)
        data = f.read()

        def append(rows: list[dict]) -> None:
            text = "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows)
            if data and not data.endswith("\n"):
                text = "\n" + text  # 途中で切れた行の後ろに続けない
            f.write(text)
            f.flush()

        yield _state(data), append


def _parse(line: str) -> dict | None:
    try:
        row = json.loads(line)
    except ValueError:
        return None
    ok = (isinstance(row, dict) and isinstance(row.get("id"), str)
          and isinstance(row.get("rev"), int) and row.get("op") in ("put", "del"))
    return row if ok else None


def _state(data: str) -> dict[str, dict]:
    # id ごとに rev が最大の行。del も残す（消えたことを示すため）
    # ponytail: 行を split("\n") で割る。splitlines は U+2028 などでも割るので使わない
    cur: dict[str, dict] = {}
    bad = 0
    for line in data.split("\n"):
        if not line.strip():
            continue
        row = _parse(line)
        if row is None:
            bad += 1
            continue
        old = cur.get(row["id"])
        if old is None or row["rev"] > old["rev"]:
            cur[row["id"]] = row
    if bad:
        logging.warning("events.jsonl の壊れた行を %d 件飛ばしました", bad)
    return cur


def _live(state: dict[str, dict], id: str) -> dict:
    cur = state.get(id)
    if cur is None or cur["op"] == "del":
        raise KeyError(f"イベントがありません: {id}")
    return cur


def _onset(r: dict):
    return r["t"] if r["type"] == "line" else r["t0"]


# --- 検査（C4） ---

def _fields(ev) -> dict:
    """本文から書き手の項目（op・id・rev・source・created_at）を外し、知らない項目を拒む。"""
    if not isinstance(ev, dict):
        raise ValueError("イベントは object にしてください")
    body = {k: v for k, v in ev.items() if k not in _BOOK}
    unknown = sorted(set(body) - set(_FIELDS))
    if unknown:
        raise ValueError(f"未知の項目です: {', '.join(unknown)}")
    return body


def _time(ev: dict, key: str, dur: float) -> float:
    v = ev.get(key)
    if not is_num(v) or not 0 <= v <= dur:
        raise ValueError(f"{key}: 0 から {dur:g} 秒の間の数にしてください")
    return float(v)


def _metric(k: str, v, d: dict) -> None:
    t = d.get("type")
    if t == "number":
        ok = is_num(v)
    elif t == "integer":
        ok = isinstance(v, int) and not isinstance(v, bool)
    elif t == "string":
        ok = isinstance(v, str)
    elif t == "boolean":
        ok = isinstance(v, bool)
    elif t == "enum":
        ok = isinstance(v, str) and v in d.get("values", [])
    else:
        ok = False
    if not ok:
        raise ValueError(f"metrics.{k}: {t} の値の形が違います")
    if t in ("number", "integer"):
        lo, hi = d.get("min"), d.get("max")
        if (lo is not None and v < lo) or (hi is not None and v > hi):
            raise ValueError(f"metrics.{k}: 範囲の外です（min {lo}・max {hi}）")


def _validate(ds: str, ev: dict, schema: dict) -> dict:
    """1 件のイベントを検査し、型に合う項目だけを返す（時刻は line なら t、area なら t0・t1）。"""
    typ = ev.get("type")
    if typ not in TYPES:
        raise ValueError("type: line か area にしてください")
    rec = ev.get("record")
    if not paths.valid_name(rec):
        raise ValueError(f"record: 名前の形が違います: {rec}")
    try:
        meta = dataset.load_meta(ds, rec)
    except KeyError as e:
        raise ValueError(f"record: {e.args[0]}") from None
    dur = meta["n_samples"] / meta["srate"]

    vs = ev.get("vars", [])
    if not isinstance(vs, list) or not all(isinstance(x, str) for x in vs):
        raise ValueError("vars: 文字列の配列にしてください")
    names = {v["name"] for v in meta["vars"]}
    for x in vs:
        if x not in names:
            raise ValueError(f"vars: この記録にない変数です: {x}")

    out = {"type": typ, "record": rec, "vars": vs}
    if typ == "line":
        out["t"] = _time(ev, "t", dur)
    else:
        out["t0"] = _time(ev, "t0", dur)
        out["t1"] = _time(ev, "t1", dur)
        if not out["t0"] < out["t1"]:
            raise ValueError("t0: t1 より小さくしてください")

    cats = {c["name"] for c in schema["categories"]}
    cat = ev.get("category")
    if not isinstance(cat, str) or cat not in cats:
        raise ValueError(f"category: 登録のないカテゴリです: {cat}")
    cdef = next(c for c in schema["categories"] if c["name"] == cat)
    if cdef.get("type") and cdef["type"] != typ:
        raise ValueError(f"type: カテゴリ {cat} は {cdef['type']} です")
    if cdef.get("ch") == "single" and len(vs) != 1:
        raise ValueError(f"vars: カテゴリ {cat} は ch を 1 つだけ選びます")
    if cdef.get("ch") == "all" and vs:
        raise ValueError(f"vars: カテゴリ {cat} は記録全体（vars は空）です")

    text = ev.get("text", "")
    if not isinstance(text, str) or len(text) > MAX_TEXT:
        raise ValueError(f"text: 文字列にしてください（{MAX_TEXT} 字まで）")

    mets = ev.get("metrics", {})
    if not isinstance(mets, dict):
        raise ValueError("metrics: object にしてください")
    defs = {m["name"]: m for m in schema["metrics"]}
    for k, v in mets.items():
        if k not in defs:
            raise ValueError(f"metrics.{k}: 登録のないメトリクスです")
        _metric(k, v, defs[k])

    out.update(category=cat, text=text, metrics=mets)
    return out


def _put(ds: str, id: str, rev: int, ev: dict, schema: dict, source: str) -> dict:
    return {"op": "put", "id": id, "rev": rev, **_validate(ds, ev, schema),
            "source": source, "created_at": now()}


# --- イベント（C4） ---

def list_events(ds: str, record: str | None = None, category: str | None = None,
                type: str | None = None) -> list[dict]:
    if type is not None and type not in TYPES:
        raise ValueError("type: line か area にしてください")
    evs = [r for r in _state(_read(ds)).values() if r["op"] == "put"]
    evs = [r for r in evs
           if (record is None or r["record"] == record)
           and (category is None or r["category"] == category)
           and (type is None or r["type"] == type)]
    return sorted(evs, key=lambda r: (r["record"], _onset(r), r["id"]))


def add_events(ds: str, evs: list, source: str) -> list[dict]:
    """まとめて検査し、1 件でも誤りがあれば何も書かない。"""
    _check_source(source)
    if not isinstance(evs, list):
        raise ValueError("イベントは配列にしてください")
    # ponytail: 追記のたびに全行を読む。件数が増えて遅くなったら id の索引を足す
    with _journal(ds) as (_, append):
        schema = load_schema(ds)
        rows = [_put(ds, "e-" + secrets.token_hex(6), 1, _fields(ev), schema, source)
                for ev in evs]
        append(rows)
    return rows


def add_event(ds: str, ev: dict, source: str) -> dict:
    return add_events(ds, [ev], source)[0]


def update_event(ds: str, id: str, patch: dict, source: str) -> dict:
    _check_source(source)
    body = _fields(patch)
    with _journal(ds) as (state, append):
        cur = _live(state, id)
        merged = {**{k: v for k, v in cur.items() if k not in _BOOK}, **body}
        row = _put(ds, id, cur["rev"] + 1, merged, load_schema(ds), source)
        append([row])
    return row


def update_events(ds: str, patches: dict[str, dict], source: str) -> list[dict]:
    """{id: 変える項目} をまとめて書く。1 回のロックの中で、1 件でも誤りがあれば何も書かない。"""
    _check_source(source)
    if not isinstance(patches, dict):
        raise ValueError("patches は object にしてください")
    with _journal(ds) as (state, append):
        schema = load_schema(ds)
        rows = []
        for id, patch in patches.items():
            cur = _live(state, id)
            merged = {**{k: v for k, v in cur.items() if k not in _BOOK}, **_fields(patch)}
            rows.append(_put(ds, id, cur["rev"] + 1, merged, schema, source))
        append(rows)
    return rows


def delete_event(ds: str, id: str, source: str) -> dict:
    _check_source(source)
    with _journal(ds) as (state, append):
        cur = _live(state, id)
        row = {"op": "del", "id": id, "rev": cur["rev"] + 1, "source": source, "created_at": now()}
        append([row])
    return row


def delete_events(ds: str, ids: list[str], source: str) -> list[dict]:
    """まとめて消す。1 回のロックの中で行い、1 件でも無い ID があれば何も書かない（KeyError）。返り値は書いた del の行。"""
    _check_source(source)
    if not isinstance(ids, list) or not all(isinstance(x, str) for x in ids):
        raise ValueError("ids は文字列の配列にしてください")
    with _journal(ds) as (state, append):
        rows = [{"op": "del", "id": id, "rev": _live(state, id)["rev"] + 1,
                 "source": source, "created_at": now()}
                for id in dict.fromkeys(ids)]  # 同じ ID が重なっても 1 件
        if rows:
            append(rows)
    return rows


# --- スキーマ（C4） ---

def load_schema(ds: str) -> dict:
    p = paths.dataset_dir(ds) / SCHEMA
    s = json.loads(p.read_text(encoding="utf-8")) if p.is_file() else {}
    return {"version": 1, "categories": s.get("categories", []), "metrics": s.get("metrics", [])}


def _names(items, what: str) -> None:
    if not isinstance(items, list):
        raise ValueError(f"{what} は配列にしてください")
    seen = set()
    for it in items:
        if not isinstance(it, dict):
            raise ValueError(f"{what}: object にしてください")
        n = it.get("name")
        if not paths.valid_name(n):
            raise ValueError(f"{what}.name: 名前の形が違います: {n}")
        if n in seen:
            raise ValueError(f"{what}.name: 重なっています: {n}")
        seen.add(n)


def _check_schema(s) -> dict:
    if not isinstance(s, dict):
        raise ValueError("schema は object にしてください")
    cats, mets = s.get("categories", []), s.get("metrics", [])
    _names(cats, "categories")
    for c in cats:
        if not isinstance(c.get("color"), str) or not COLOR_RE.fullmatch(c["color"]):
            raise ValueError(f"categories.color: #RRGGBB の形にしてください: {c['name']}")
        if "visible" in c and not isinstance(c["visible"], bool):
            raise ValueError(f"categories.visible: true か false にしてください: {c['name']}")
        if "type" in c and c["type"] not in TYPES:
            raise ValueError(f"categories.type: area か line にしてください: {c['name']}")
        if "ch" in c and c["ch"] not in CH_MODES:
            raise ValueError(f"categories.ch: single か all にしてください: {c['name']}")
    _names(mets, "metrics")
    for m in mets:
        if m.get("type") not in METRIC_TYPES:
            raise ValueError(f"metrics.type: {' / '.join(METRIC_TYPES)} のどれかにしてください: {m['name']}")
        if m["type"] == "enum":
            vals = m.get("values")
            if not isinstance(vals, list) or not vals or not all(isinstance(x, str) and x for x in vals):
                raise ValueError(f"metrics.values: 空でない文字列の配列にしてください: {m['name']}")
        lo, hi = m.get("min"), m.get("max")
        if any(x is not None and not is_num(x) for x in (lo, hi)):
            raise ValueError(f"metrics.min・max: 数にしてください: {m['name']}")
        if lo is not None and hi is not None and lo > hi:
            raise ValueError(f"metrics.min: max より大きくできません: {m['name']}")
    return {"version": 1, "categories": cats, "metrics": mets}


def save_schema(ds: str, schema: dict) -> dict:
    new = _check_schema(schema)
    # ロックの中で検査する。検査と書き込みの間に、使われ始めたカテゴリを消さないため
    with _journal(ds) as (state, _):
        live = [r for r in state.values() if r["op"] == "put"]
        kept = {c["name"] for c in new["categories"]}
        for c in load_schema(ds)["categories"]:
            n = sum(1 for r in live if r["category"] == c["name"])
            if c["name"] not in kept and n:
                raise ValueError(f"カテゴリ {c['name']} は {n} 件のイベントが使っています")
        write_json(paths.dataset_dir(ds) / SCHEMA, new)
    return new


# --- BIDS 互換の書き出し（C5） ---

def _cell(v) -> str:
    if isinstance(v, bool):
        return "true" if v else "false"
    return str(v)


def export_tsv(ds: str) -> str:
    metrics = [m["name"] for m in load_schema(ds)["metrics"]]
    head = ["onset", "duration", "trial_type", "channel", "record", "text", "source", "id"]
    lines = ["\t".join(head + [f"m_{m}" for m in metrics])]
    for r in list_events(ds):
        on = _onset(r)
        dur = 0.0 if r["type"] == "line" else r["t1"] - r["t0"]
        cells = [f"{on:.6f}", f"{dur:.6f}", r["category"], "|".join(r["vars"]) or "n/a",
                 r["record"], r["text"] or "n/a", r["source"], r["id"]]
        for m in metrics:
            v = r["metrics"].get(m)
            cells.append("n/a" if v is None else _cell(v))
        lines.append("\t".join(_TSV_BAD.sub(" ", str(c)) for c in cells))
    return "\n".join(lines) + "\n"
