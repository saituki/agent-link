"""チャット（C11）。相手は sai-link の cc の run 1 本（データセットごと）。会話は sai-link の記録から読む。"""

from __future__ import annotations

import json
import os
import re
from datetime import datetime, timezone

from . import linkcli, paths

BLOCK = "ts-agent"
MAX_TEXT = 20000
TASK = ("あなたは sai-ts のチャット相手である。データセットは {ds}。"
        "利用者の発言を待ち、sai-ts の CLI で画面の状態とイベントを読んでから答えよ。")
_DELIM = re.compile(r"# (user|assistant) \((\d+)\)")


def _load(ds: str) -> dict | None:
    p = paths.dataset_dir(ds) / "chat.json"
    return json.loads(p.read_text(encoding="utf-8")) if p.is_file() else None


def _status(run_id: int) -> str | None:
    for r in linkcli.run(["run", "list", "--json", "--limit", "0"]).get("runs", []):
        if r.get("run_id") == run_id:
            return r.get("status")
    return None


def start(ds: str) -> dict:
    paths.dataset_dir(ds)
    cur = _load(ds)
    if cur and _status(cur["run_id"]) == "running":
        return {"run_id": cur["run_id"], "started": False}
    card = {"runner": "cc", "block": BLOCK, "description": f"sai-ts チャット: {ds}",
            "task_text": TASK.format(ds=ds)}
    run_id = linkcli.run(["card", "create", "--json"], stdin=json.dumps(card, ensure_ascii=False))["run_id"]
    linkcli.run(["card", "run", str(run_id), "--json"])
    body = {"run_id": run_id, "created_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")}
    p = paths.dataset_dir(ds) / "chat.json"
    tmp = p.with_name(p.name + ".tmp")
    tmp.write_text(json.dumps(body, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(tmp, p)
    return {"run_id": run_id, "started": True}


def send(ds: str, text: str) -> dict:
    if not isinstance(text, str) or not 1 <= len(text) <= MAX_TEXT:
        raise ValueError(f"本文は 1〜{MAX_TEXT} 字にしてください")
    cur = _load(ds)
    if cur is None:
        raise linkcli.LinkError(409, "チャットが始まっていません")
    linkcli.run(["run", "send", str(cur["run_id"]), "--json"], stdin=text)
    return {"run_id": cur["run_id"]}


def split_messages(text: str) -> list[dict]:
    """sai-link の message.md を発言に分ける。最初の区切りより前は捨てる。"""
    out: list[dict] = []
    cur = None
    buf: list[str] = []

    def close():
        if cur is not None:
            out.append({"role": cur[0], "n": cur[1], "text": "\n".join(buf).strip("\n")})

    for line in text.splitlines():
        m = _DELIM.fullmatch(line)
        if m:
            close()
            cur = (m.group(1), int(m.group(2)))
            buf = []
        elif cur is not None:
            buf.append(line)
    close()
    return out


def messages(ds: str) -> dict:
    cur = _load(ds)
    if cur is None:
        return {"run_id": None, "status": None, "messages": []}
    rid = cur["run_id"]
    status = _status(rid)
    try:
        md = linkcli.run(["run", "md", str(rid), "--kind", "message", "--json"])
    except linkcli.LinkError as e:
        if e.status != 404:  # 404 は「まだありません」
            raise
        return {"run_id": rid, "status": status, "messages": []}
    return {"run_id": rid, "status": status, "messages": split_messages(md["text"])}
