#!/usr/bin/env python3
"""cc のフックから呼ばれる質問の中継スクリプト．

cc が AskUserQuestion を呼ぶ直前（PreToolUse）に呼ばれ，
標準入力の JSON をサーバーに転送するだけ．軽く保つこと
（cc の動作を止めないため）．

注意:
- 標準出力に何も書かない（cc がフックの出力を解釈してしまうため）
- サーバーに繋がらなくてもエラーを出さず終了コード 0 で終える
  （端末側での回答は従来どおりできるようにするため）
- フックが質問に答えることはできない（cc の仕様上，
  フックはツールの結果を作れない）．答えるのはサーバー経由の別経路．
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
        tool_input = data.get("tool_input") or {}
        payload = {
            "agent": "claude",
            "session_id": data.get("session_id") or "",
            "pane_id": os.environ.get("HERDR_PANE_ID") or "",
            "tool_use_id": data.get("tool_use_id") or "",
            "questions": tool_input.get("questions") or [],
        }
        if not payload["session_id"] or not payload["questions"]:
            return 0
        body = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(
            "http://127.0.0.1:8200/api/questions",
            data=body,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=2) as res:
            res.read()
    except Exception:
        # サーバー停止中でも cc は普通に動かす
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
