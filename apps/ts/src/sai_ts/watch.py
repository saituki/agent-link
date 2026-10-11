"""変化の知らせ（SSE・共有契約 C7・C8）。1 秒ごとにファイルの mtime と大きさを見る。"""

from __future__ import annotations

import asyncio
import hashlib
import json
from pathlib import Path

from . import algos, paths


def ext_version(ext_dir: Path) -> str:
    h = hashlib.sha256()
    for p in sorted(f for f in ext_dir.rglob("*") if f.is_file()):
        h.update(p.relative_to(ext_dir).as_posix().encode("utf-8") + b"\0")
        h.update(p.read_bytes())
    return h.hexdigest()[:12]


def _stat(p: Path):
    try:
        st = p.stat()
    except FileNotFoundError:
        return None
    return (st.st_mtime_ns, st.st_size)


def _view_state():
    st = _stat(paths.view_path())
    if st is None:
        return None
    try:
        by = json.loads(paths.view_path().read_text(encoding="utf-8")).get("updated_by")
    except (ValueError, OSError):
        by = None
    return (*st, by)


def _algos_state() -> list:
    # スクリプトと仕様（ts/scripts/ と組み込み）の (置き場, 名前, mtime, 大きさ) の一覧
    out = []
    for d in (paths.scripts_dir(), algos.BUILTIN_DIR):
        for p in sorted(d.glob("*")) if d.is_dir() else []:
            if p.suffix in (".py", ".json") and p.is_file():
                st = _stat(p)
                if st is not None:
                    out.append((d.name, p.name, *st))
    return out


def snapshot() -> dict:
    # ponytail: 1 秒ごとに全部見る（inotify 無し）。記録や拡張が増えて重くなったら変える
    snap: dict = {"view": _view_state(), "algos": _algos_state()}
    root = paths.datasets_dir()
    for d in root.glob("*") if root.is_dir() else []:
        if not d.is_dir() or not paths.valid_name(d.name):
            continue
        snap[f"events:{d.name}"] = _stat(d / "events.jsonl")
        snap[f"schema:{d.name}"] = _stat(d / "schema.json")
        snap[f"channels:{d.name}"] = _stat(d / "channels.json")
        snap[f"runs:{d.name}"] = _stat(d / "runs.jsonl")
        for r in (d / "records").glob("*") if (d / "records").is_dir() else []:
            if r.is_dir():
                snap[f"vars:{d.name}/{r.name}"] = _stat(r / "meta.json")
    ext = paths.ext_dir()
    for e in ext.glob("*") if ext.is_dir() else []:
        if e.is_dir():
            snap[f"ext:{e.name}"] = ext_version(e)
    sroot = paths.snapshots_dir()
    for s in sroot.glob("*") if sroot.is_dir() else []:
        if s.is_dir() and paths.valid_name(s.name):
            snap[f"snapshots:{s.name}"] = (_stat(s / "manifest.json"), _stat(s / "links.json"))
    return snap


def diff(old: dict, new: dict) -> list[dict]:
    out = []
    for key in sorted(set(old) | set(new)):
        a, b = old.get(key), new.get(key)
        if a == b:
            continue
        if key == "view":
            # 画面からの変更は知らせない（画面が自分で知っている）
            if b is not None and b[2] == "agent":
                out.append({"kind": "view"})
        elif key.startswith("events:"):
            out.append({"kind": "events", "dataset": key.split(":", 1)[1]})
        elif key.startswith("schema:"):
            out.append({"kind": "schema", "dataset": key.split(":", 1)[1]})
        elif key.startswith("channels:"):
            out.append({"kind": "channels", "dataset": key.split(":", 1)[1]})
        elif key == "algos":
            out.append({"kind": "algos"})
        elif key.startswith("runs:"):
            out.append({"kind": "runs", "dataset": key.split(":", 1)[1]})
        elif key.startswith("vars:"):
            ds, rec = key.split(":", 1)[1].split("/", 1)
            out.append({"kind": "vars", "dataset": ds, "record": rec})
        elif key.startswith("ext:"):
            out.append({"kind": "ext", "name": key.split(":", 1)[1], "version": b})
        elif key.startswith("snapshots:"):
            # スナップショットは 1 件ずつ増えるので、同じ知らせは 1 回の差分に 1 つだけ出す
            if {"kind": "snapshots"} not in out:
                out.append({"kind": "snapshots"})
    return out


async def stream():
    old = await asyncio.to_thread(snapshot)
    ticks = 0
    while True:
        await asyncio.sleep(1)
        new = await asyncio.to_thread(snapshot)
        for ev in diff(old, new):
            yield f"data: {json.dumps(ev, ensure_ascii=False)}\n\n"
        old = new
        ticks += 1
        if ticks % 15 == 0:
            yield ": ping\n\n"
