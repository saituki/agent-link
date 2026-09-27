#!/usr/bin/env python3
"""cc の起動時フックから呼ばれるセッション登録スクリプト．

SessionStart で呼ばれ，どのセッションがどの pane で動いているかを
サーバーに登録する（01 の対応づけ用）．cc_question.py と同じく軽く保ち，
標準出力に何も書かず，失敗しても終了コード 0 で終える．
"""

from __future__ import annotations

import json
import os
import sys
import urllib.request


def main() -> int:
    try:
        raw = sys.stdin.read()
        data = json.loads(raw)
    except Exception:
        return 0
    try:
        payload = {
            "agent": "claude",
            "session_id": data.get("session_id") or "",
            "transcript_path": data.get("transcript_path") or "",
            "cwd": data.get("cwd") or "",
            "pane_id": os.environ.get("HERDR_PANE_ID") or "",
        }
        if not payload["session_id"] or not payload["pane_id"]:
            return 0
        body = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(
            "http://127.0.0.1:8200/api/register",
            data=body,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=2) as res:
            res.read()
    except Exception:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
