"""実行の記録 ts/datasets/<ds>/runs.jsonl（G2-C3）。追記だけ。読むときに start と end を id でまとめる。"""

from __future__ import annotations

import fcntl
import json
import logging
import math
import secrets
import time
from datetime import datetime, timezone
from pathlib import Path

from . import paths

FILE = "runs.jsonl"
STARTED_AT = time.time()  # このプロセスが始まった時刻（running と lost を分ける）
_FMT = "%Y-%m-%dT%H:%M:%SZ"


def new_id() -> str:
    return "a-" + secrets.token_hex(6)


def _path(ds: str) -> Path:
    return paths.dataset_dir(ds) / FILE


def append(ds: str, row: dict) -> None:
    """1 行足す。排他ロックの中で、途中で切れた行の後ろに続けないよう改行を確かめる。"""
    line = json.dumps(row, ensure_ascii=False).encode("utf-8") + b"\n"
    # ponytail: 書くたびに末尾 1 バイトだけ見る。記録が大きくなって遅ければ、別の索引を足す
    with open(_path(ds), "a+b") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        f.seek(0, 2)
        size = f.tell()
        if size:
            f.seek(size - 1)
            if f.read(1) != b"\n":
                line = b"\n" + line
            f.seek(0, 2)
        f.write(line)
        f.flush()


def _rows(ds: str) -> list[dict]:
    path = _path(ds)
    if not path.is_file():
        return []
    with open(path, encoding="utf-8") as f:
        fcntl.flock(f, fcntl.LOCK_SH)
        data = f.read()
    out, bad = [], 0
    for line in data.split("\n"):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except ValueError:
            row = None
        if isinstance(row, dict) and row.get("op") in ("start", "end") and isinstance(row.get("id"), str):
            out.append(row)
        else:
            bad += 1
    if bad:
        logging.warning("runs.jsonl の壊れた行を %d 件飛ばしました", bad)
    return out


def _ts(s) -> float | None:
    try:
        return datetime.strptime(s, _FMT).replace(tzinfo=timezone.utc).timestamp()
    except (TypeError, ValueError):
        return None


def list_runs(ds: str, limit: int = 50) -> list[dict]:
    """start と end をまとめ、新しい順に limit 件返す。end が無いものは running（このプロセスの分）か lost。"""
    starts: dict[str, dict] = {}
    ends: dict[str, dict] = {}
    for row in _rows(ds):
        (starts if row["op"] == "start" else ends).setdefault(row["id"], row)
    floor = math.floor(STARTED_AT)  # started_at は秒の精度なので、切り捨てて比べる
    out = []
    for id, s in starts.items():
        row = {k: v for k, v in s.items() if k != "op"}
        e = ends.get(id)
        if e is not None:
            row.update(status=e.get("status", ""), result=e.get("result"), error=e.get("error", ""),
                       ended_at=e.get("ended_at", ""), elapsed_s=e.get("elapsed_s"))
        else:
            t = _ts(s.get("started_at"))
            row.update(status="running" if t is not None and t >= floor else "lost",
                       result=None, error="", ended_at="", elapsed_s=None)
        out.append(row)
    out.sort(key=lambda r: (r.get("started_at") or "", r["id"]), reverse=True)
    return out[:limit]
