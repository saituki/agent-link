#!/usr/bin/env python3
"""cc の PreToolUse（AskUserQuestion）フック — 対話モード（tmux）の cc の質問の中継．

cc が AskUserQuestion を呼ぶ直前に呼ばれる．cc は tmux の対話モードで動くので，
Web の質問カードに登録し，利用者が答えるまで待つ．答えが届いたら
`permissionDecision: "allow"` と `updatedInput`（`questions` は cc の物をそのまま，
`answers` は `{質問文: 答え}`）を返す．cc はこれを AskUserQuestion の結果として
モデルに渡す．答えが来ないまま打ち切ったときは「利用者は答えませんでした。」を
`permissionDecisionReason` に入れて拒否する．

- `AGENT_LINK_CC_QUESTION_MODE=deny` のときは，答えがあっても今の拒否の形（理由文に答え）で返す
  （`updatedInput` が cc で効かなかったときに，コードを変えずに戻すため）．
- 利用者が質問を取り下げたとき（`state` が `withdrawn`）は，入力欄から送った文を理由文に入れて
  `deny` する。
- Web の場所: `AGENT_LINK_WEB_URL` → `http://127.0.0.1:${AGENT_LINK_WEB_PORT:-8400}`
- agent-link の run の中（`AGENT_LINK_PARENT_RUN_ID` がある）でだけ動く．
  同じプロジェクトで sai-moon の cc が動いても，何もせず終える
- 認証: 付けない（Web は localhost からの呼び出しを通す）
- 待つ間は `state.json` が `blocked`（Rust の PreToolUse フックが書く）．待ちを抜けたら `working` に戻す
  （取り下げ・打ち切りでは cc がツールを動かさず PostToolUse が来ないため）．
- 答えが届いたら `agent-link ext hook question` に渡す（拡張の hook。失敗しても答えは返す）．
- 標準ライブラリだけで書く．登録できなければ何も出さずに終了コード 0
  （cc はそのままツールを動かす）．
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import time
import urllib.request

# 答えを待つ間隔（秒）と上限回数．24 時間（cc の hooks の `timeout` 86400 秒とそろえる）．
POLL_INTERVAL_SEC = 1.0
POLL_MAX_COUNT = 86400


def server_base() -> str:
    """Web の場所（既定は 127.0.0.1:8400）．"""
    url = os.environ.get("AGENT_LINK_WEB_URL")
    if url:
        return url.rstrip("/")
    port = os.environ.get("AGENT_LINK_WEB_PORT") or "8400"
    return f"http://127.0.0.1:{port}"


def post_json(url: str, body: dict, timeout: float = 5.0) -> dict | None:
    """JSON を POST し，応答の JSON（dict）を返す（失敗は None）．"""
    data = json.dumps(body).encode("utf-8")
    headers = {"Content-Type": "application/json"}
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
    req = urllib.request.Request(url, method="GET")
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
        # 鍵は cc の質問文（`updatedInput.answers` は cc の文で引く）。並びの順で対応づける
        cc_q = questions[i] if i < len(questions) and isinstance(questions[i], dict) else q
        key = str(cc_q.get("question", ""))
        if item.get("text"):
            answers[key] = str(item["text"])
        elif isinstance(item.get("choice_indices"), list) and item["choice_indices"]:
            labels = []
            for n in item["choice_indices"]:
                if isinstance(n, int) and 0 <= n < len(options) and isinstance(options[n], dict):
                    labels.append(str(options[n].get("label", "")))
            answers[key] = ", ".join([label for label in labels if label])
        elif isinstance(item.get("choice_index"), int):
            n = item["choice_index"]
            label = ""
            if 0 <= n < len(options) and isinstance(options[n], dict):
                label = str(options[n].get("label", ""))
            answers[key] = label
    return answers


def reason_text(answers: dict | None) -> str:
    """`permissionDecisionReason` に出す文を作る．"""
    if not answers:
        return "利用者は答えませんでした。"
    lines = ["利用者が Web で答えました。"]
    for question, answer in answers.items():
        lines.append(f"{question}: {answer}")
    return "\n".join(lines)


def deny(reason: str) -> dict:
    """`deny` と理由文の PreToolUse の出力。"""
    return {
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    }


def withdrawn_output(text: str) -> dict:
    """利用者が質問を取り下げたときの deny（承認の記録はしない）。

    送った文（`text`）があれば、それを次の指示として理由文に入れる。空なら、このターンを終えさせる。
    """
    if text:
        return deny(
            "利用者は質問に答えず、質問を取り下げて次の文を送りました。質問への答えはありません。次の文に従ってください。\n"
            + text
        )
    return deny(
        "利用者は質問を取り下げました。質問への答えはありません。このターンはここで終え、利用者の次の文を待ってください。"
    )


def hook_output(questions: list, answers: dict | None) -> dict:
    """cc に返す PreToolUse の出力を作る．

    答えがあれば `allow` と `updatedInput`（質問と答え）を返す．
    `AGENT_LINK_CC_QUESTION_MODE=deny` のときと答えが無いときは `deny` と理由文を返す．
    """
    if answers and os.environ.get("AGENT_LINK_CC_QUESTION_MODE") != "deny":
        return {
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "updatedInput": {"questions": questions, "answers": answers},
            }
        }
    return deny(reason_text(answers))


def mark_working() -> None:
    """state.json を working に戻す（agent-link cc-state-hook working）。失敗は無視する．"""
    exe = shutil.which("agent-link")
    if not exe:
        return
    try:
        subprocess.run(
            [exe, "cc-state-hook", "working"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=5,
            check=False,
        )
    except Exception:
        pass


def pass_answer_to_extension(qid: str, questions: list, answers: dict) -> None:
    """答えを `agent-link ext hook question` に渡す。持ち主の拡張が無ければ何も起きない。失敗は無視する．"""
    exe = shutil.which("agent-link")
    run_id = os.environ.get("AGENT_LINK_PARENT_RUN_ID", "")
    block = os.environ.get("AGENT_LINK_BLOCK_NAME", "")
    if not exe or not run_id.isdigit() or not block:
        return
    body = {
        "run_id": int(run_id),
        "block": block,
        "qid": qid,
        "questions": questions,
        "answers": answers,
        "answered_by": "user",
    }
    try:
        # 標準出力は cc が JSON として読むので、子の出力は捨てる
        subprocess.run(
            [exe, "ext", "hook", "question", "--json"],
            input=json.dumps(body, ensure_ascii=False),
            text=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=15,
            check=False,
        )
    except Exception:
        pass


def main() -> int:
    if not os.environ.get("AGENT_LINK_PARENT_RUN_ID"):
        return 0
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
    withdrawn: str | None = None
    for _ in range(POLL_MAX_COUNT):
        time.sleep(POLL_INTERVAL_SEC)
        state = get_json(f"{base}/api/questions/{qid}")
        if state is None:
            continue
        answers = answers_from_state(state, questions)
        if answers is not None:
            break
        # 利用者が質問を取り下げた（送った文を次の指示として返す）
        if state.get("state") == "withdrawn":
            answer = state.get("answer")
            withdrawn = str((answer.get("text") if isinstance(answer, dict) else "") or "")
            break
        # `open` 以外で答えが無ければ打ち切る
        if state.get("state") and state.get("state") != "open":
            break

    # 答え無しで終わると cc はツールを動かさず PostToolUse が来ない。blocked を残さない
    mark_working()
    if withdrawn is not None:
        out = withdrawn_output(withdrawn)
    else:
        if answers:
            pass_answer_to_extension(qid, questions, answers)
        out = hook_output(questions, answers)
    sys.stdout.write(json.dumps(out, ensure_ascii=False))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
