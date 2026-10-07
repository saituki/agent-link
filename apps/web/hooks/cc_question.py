#!/usr/bin/env python3
"""cc の PreToolUse（AskUserQuestion）フック — headless 用の質問の中継．

cc が AskUserQuestion を呼ぶ直前に呼ばれる．sai-tree の headless では端末の
選択画面が無いので，Web の質問カードに登録し，利用者が答えるまで待つ．
答えが届いたら「利用者が Web で答えました。」と答えを
`permissionDecisionReason` に入れてツールの実行を拒否し，モデルに答えを返す
（cc の PreToolUse の公式の出力）．答えが来ないまま打ち切ったときは
「利用者は答えませんでした。」を返す．

- Web の場所: `SAI_WEB_URL` → `http://127.0.0.1:${SAI_WEB_PORT:-8200}`
- 合言葉: `SAI_WEB_INTERNAL_TOKEN` があれば `Authorization: Bearer <値>` を付ける
- 標準ライブラリだけで書く．登録できなければ何も出さずに終了コード 0
  （cc はそのままツールを動かし，headless では選択画面が出ないので失敗が返る）．
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.request

# 答えを待つ間隔（秒）と上限回数．24 時間（cc の hooks の `timeout` 86400 秒とそろえる）．
POLL_INTERVAL_SEC = 1.0
POLL_MAX_COUNT = 86400


def server_base() -> str:
    """Web の場所（既定は 127.0.0.1:8200）．"""
    url = os.environ.get("SAI_WEB_URL")
    if url:
        return url.rstrip("/")
    port = os.environ.get("SAI_WEB_PORT") or "8200"
    return f"http://127.0.0.1:{port}"


def auth_headers(extra: dict | None = None) -> dict:
    """`SAI_WEB_INTERNAL_TOKEN` があれば Authorization を足したヘッダを返す．"""
    headers = dict(extra or {})
    token = os.environ.get("SAI_WEB_INTERNAL_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    return headers


def post_json(url: str, body: dict, timeout: float = 5.0) -> dict | None:
    """JSON を POST し，応答の JSON（dict）を返す（失敗は None）．"""
    data = json.dumps(body).encode("utf-8")
    headers = auth_headers({"Content-Type": "application/json"})
    req = urllib.request.Request(url, data=data, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as res:
            raw = res.read()
    except Exception:
        return None
    try:
        value = json.loads(raw)
    except Exception:
        return None
    return value if isinstance(value, dict) else None


def get_json(url: str, timeout: float = 5.0) -> dict | None:
    """JSON を GET し，応答の JSON（dict）を返す（失敗は None）．"""
    req = urllib.request.Request(url, headers=auth_headers(), method="GET")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as res:
            raw = res.read()
    except Exception:
        return None
    try:
        value = json.loads(raw)
    except Exception:
        return None
    return value if isinstance(value, dict) else None


def answers_from_state(data: dict, questions: list) -> dict | None:
    """サーバーが保存した答えを `{質問: 答え}` に変える（`ask_user.ts` と同じ規則）．

    `state` が `answered` のときだけ答えを返す（それ以外は None）．
    """
    if data.get("state") != "answered" or not isinstance(data.get("answer"), dict):
        return None
    qs = data.get("questions")
    if not isinstance(qs, list) or not qs:
        qs = questions
    answer = data["answer"]
    items = answer.get("answers")
    if not isinstance(items, list):
        items = [{"choice_index": answer.get("choice_index"), "text": answer.get("text")}]
    answers: dict[str, str] = {}
    for i, item in enumerate(items):
        if not isinstance(item, dict) or i >= len(qs):
            continue
        q = qs[i] if isinstance(qs[i], dict) else {}
        options = q.get("options") if isinstance(q.get("options"), list) else []
        if item.get("text"):
            answers[str(q.get("question", ""))] = str(item["text"])
        elif isinstance(item.get("choice_indices"), list) and item["choice_indices"]:
            labels = []
            for n in item["choice_indices"]:
                if isinstance(n, int) and 0 <= n < len(options) and isinstance(options[n], dict):
                    labels.append(str(options[n].get("label", "")))
            answers[str(q.get("question", ""))] = ", ".join([label for label in labels if label])
        elif isinstance(item.get("choice_index"), int):
            n = item["choice_index"]
            label = ""
            if 0 <= n < len(options) and isinstance(options[n], dict):
                label = str(options[n].get("label", ""))
            answers[str(q.get("question", ""))] = label
    return answers


def reason_text(answers: dict | None) -> str:
    """`permissionDecisionReason` に出す文を作る．"""
    if not answers:
        return "利用者は答えませんでした。"
    lines = ["利用者が Web で答えました。"]
    for question, answer in answers.items():
        lines.append(f"{question}: {answer}")
    return "\n".join(lines)


def main() -> int:
    try:
        data = json.loads(sys.stdin.read())
    except Exception:
        return 0
    if not isinstance(data, dict):
        return 0
    tool_input = data.get("tool_input") or {}
    questions = tool_input.get("questions") or []
    session_id = data.get("session_id") or ""
    if not session_id or not questions:
        return 0

    base = server_base()
    registered = post_json(
        f"{base}/api/questions",
        {
            "agent": "claude",
            "session_id": session_id,
            "pane_id": os.environ.get("HERDR_PANE_ID") or "",
            "tool_use_id": data.get("tool_use_id") or "",
            "questions": questions,
        },
    )
    if not registered or not registered.get("id"):
        # 登録できなければ何も出さずに終える（cc の動作を止めない）
        return 0
    qid = str(registered["id"])

    answers: dict | None = None
    for _ in range(POLL_MAX_COUNT):
        time.sleep(POLL_INTERVAL_SEC)
        state = get_json(f"{base}/api/questions/{qid}")
        if state is None:
            continue
        answers = answers_from_state(state, questions)
        if answers is not None:
            break
        # `open` 以外で答えが無ければ打ち切る
        if state.get("state") and state.get("state") != "open":
            break

    out = {
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason_text(answers),
        }
    }
    sys.stdout.write(json.dumps(out, ensure_ascii=False))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
