"""cc_question.py（cc の PreToolUse フック）のテスト。

標準ライブラリの `http.server` で偽の Web を立て、`cc_question.py` を走らせて
次を確かめる。

- 登録 → 答え待ち → 答えが入った `allow` + `updatedInput` の JSON を出す
- `AGENT_LINK_CC_QUESTION_MODE=deny` のときは答えを `permissionDecisionReason` に入れた `deny` を出す
- `AGENT_LINK_WEB_INTERNAL_TOKEN` を渡しても `Authorization` は付かない
- `AGENT_LINK_WEB_URL` の値に送る
- 答えが来ないまま `open` でなくなったら「利用者は答えませんでした。」
- `AGENT_LINK_PARENT_RUN_ID` があっても承認の記録は行わず、答えが取れたら allow

実行:
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_cc_question_hook.py
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HOOK_PATH = Path(__file__).resolve().parent.parent / "hooks" / "cc_question.py"

QUESTIONS = [
    {
        "question": "好きな色は？",
        "header": "色",
        "multiSelect": False,
        "options": [{"label": "赤"}, {"label": "青"}],
    },
]
PAYLOAD = {
    "session_id": "sess-1",
    "hook_event_name": "PreToolUse",
    "tool_use_id": "tu-1",
    "tool_input": {"questions": QUESTIONS},
}
OPEN_STATE = {"state": "open", "questions": QUESTIONS}
ANSWERED_STATE = {
    "state": "answered",
    "questions": QUESTIONS,
    "answer": {"answers": [{"choice_index": 1}]},
}
RESOLVED_STATE = {"state": "resolved_elsewhere", "questions": QUESTIONS}
WITHDRAWN_STATE = {"state": "withdrawn", "questions": QUESTIONS, "answer": {"text": ""}}
ANSWERED_BY_STATE = {**ANSWERED_STATE, "answered_by": "saituki"}
# 自由記述（Other）で `-` で始まる答え
ANSWERED_DASH_STATE = {
    "state": "answered",
    "questions": QUESTIONS,
    "answer": {"answers": [{"text": "-緑"}]},
    "answered_by": "saituki",
}


class FakeWeb:
    """`POST /api/questions` と `GET /api/questions/{qid}` に答える偽の Web。"""

    def __init__(self, get_states: list[dict]) -> None:
        self._states = list(get_states)
        self.registrations: list[tuple[dict, bytes]] = []
        self.gets: list[tuple[str, dict]] = []
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def _send(self, obj: dict) -> None:
                body = json.dumps(obj).encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self) -> None:  # noqa: N802
                length = int(self.headers.get("Content-Length") or 0)
                raw = self.rfile.read(length)
                outer.registrations.append((dict(self.headers), raw))
                self._send({"ok": True, "id": "q-test"})

            def do_GET(self) -> None:  # noqa: N802
                outer.gets.append((self.path, dict(self.headers)))
                self._send(outer.next_state())

            def log_message(self, *args) -> None:  # 騒がしいログは出さない
                return

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.port = self._server.server_address[1]
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    def next_state(self) -> dict:
        """GET の応答を順に返す（尽きたら最後のものを返し続ける）。"""
        if len(self._states) > 1:
            return self._states.pop(0)
        return self._states[0]

    def close(self) -> None:
        self._server.shutdown()
        self._server.server_close()


def run_hook(port: int, token: str | None = None, extra_env: dict | None = None) -> tuple[int, str]:
    """フックを走らせて（終了コード, 標準出力）を返す。"""
    env = {
        "PATH": "/usr/bin:/bin",
        "AGENT_LINK_WEB_URL": f"http://127.0.0.1:{port}",
        "HERDR_PANE_ID": "hl-0123456789ab",
        "AGENT_LINK_PARENT_RUN_ID": "7",
    }
    if token is not None:
        env["AGENT_LINK_WEB_INTERNAL_TOKEN"] = token
    if extra_env:
        env.update(extra_env)
    proc = subprocess.run(
        [sys.executable, str(HOOK_PATH)],
        input=json.dumps(PAYLOAD),
        capture_output=True,
        text=True,
        timeout=30,
        env=env,
    )
    return proc.returncode, proc.stdout


def test_answered_returns_allow_with_updated_input() -> None:
    """登録 → 答え待ち → allow + updatedInput（質問と答え）。Authorization も届く。"""
    web = FakeWeb([OPEN_STATE, ANSWERED_STATE])
    try:
        code, out = run_hook(web.port, token="secret-token")
    finally:
        web.close()
    assert code == 0, code
    value = json.loads(out)
    hook = value["hookSpecificOutput"]
    assert hook["hookEventName"] == "PreToolUse"
    assert hook["permissionDecision"] == "allow"
    assert "permissionDecisionReason" not in hook, hook
    assert hook["updatedInput"]["questions"] == QUESTIONS, hook
    assert hook["updatedInput"]["answers"] == {"好きな色は？": "青"}, hook

    # 登録は POST /api/questions に、質問の本文つきで届く
    assert len(web.registrations) == 1
    headers, raw = web.registrations[0]
    body = json.loads(raw)
    assert body["agent"] == "claude"
    assert body["session_id"] == "sess-1"
    assert body["pane_id"] == "hl-0123456789ab"
    assert body["questions"] == QUESTIONS
    # `AGENT_LINK_WEB_INTERNAL_TOKEN` を渡しても Authorization は付けない（localhost だけで通す）
    assert "Authorization" not in headers, headers
    # AGENT_LINK_WEB_URL の値（偽の Web の住所）に送る
    assert web.gets and web.gets[0][0] == "/api/questions/q-test"
    assert "Authorization" not in web.gets[0][1], web.gets[0][1]


def test_deny_mode_returns_reason_with_answer() -> None:
    """AGENT_LINK_CC_QUESTION_MODE=deny なら、答えがあっても今の deny と理由文の形。"""
    web = FakeWeb([OPEN_STATE, ANSWERED_STATE])
    try:
        code, out = run_hook(web.port, token=None, extra_env={"AGENT_LINK_CC_QUESTION_MODE": "deny"})
    finally:
        web.close()
    assert code == 0, code
    hook = json.loads(out)["hookSpecificOutput"]
    assert hook["permissionDecision"] == "deny", hook
    assert "updatedInput" not in hook, hook
    reason = hook["permissionDecisionReason"]
    assert reason.startswith("利用者が Web で答えました。"), repr(reason)
    assert "好きな色は？: 青" in reason, repr(reason)


def test_no_token_has_no_authorization() -> None:
    """合言葉は無いので Authorization を付けない。"""
    web = FakeWeb([ANSWERED_STATE])
    try:
        code, out = run_hook(web.port, token=None)
    finally:
        web.close()
    assert code == 0, code
    json.loads(out)
    headers, _ = web.registrations[0]
    assert "Authorization" not in headers, headers
    assert "Authorization" not in web.gets[0][1], web.gets[0][1]


def test_unanswered_reports_not_answered() -> None:
    """答えが来ないまま open でなくなったら「利用者は答えませんでした。」。"""
    web = FakeWeb([RESOLVED_STATE])
    try:
        code, out = run_hook(web.port, token="secret-token")
    finally:
        web.close()
    assert code == 0, code
    value = json.loads(out)
    reason = value["hookSpecificOutput"]["permissionDecisionReason"]
    assert reason == "利用者は答えませんでした。", repr(reason)


def test_withdrawn_with_text_denies_with_next_message() -> None:
    """取り下げ（文あり）は、送った文を次の指示として理由文に入れて deny する（承認の記録はしない）。"""
    web = FakeWeb([OPEN_STATE, {**WITHDRAWN_STATE, "answer": {"text": "表を作って"}}])
    try:
        code, out = run_hook(web.port, token="secret-token")
    finally:
        web.close()
    assert code == 0, code
    hook = json.loads(out)["hookSpecificOutput"]
    assert hook["permissionDecision"] == "deny", hook
    assert "updatedInput" not in hook, hook
    assert hook["permissionDecisionReason"] == (
        "利用者は質問に答えず、質問を取り下げて次の文を送りました。質問への答えはありません。次の文に従ってください。\n"
        "表を作って"
    ), repr(hook["permissionDecisionReason"])


def test_withdrawn_without_text_ends_turn() -> None:
    """取り下げ（文なし・interrupt）は、このターンを終えて次の文を待つ理由文で deny する。"""
    web = FakeWeb([OPEN_STATE, {**WITHDRAWN_STATE, "answer": {"text": ""}}])
    try:
        code, out = run_hook(web.port, token="secret-token")
    finally:
        web.close()
    assert code == 0, code
    hook = json.loads(out)["hookSpecificOutput"]
    assert hook["permissionDecision"] == "deny", hook
    assert hook["permissionDecisionReason"] == (
        "利用者は質問を取り下げました。質問への答えはありません。このターンはここで終え、利用者の次の文を待ってください。"
    ), repr(hook["permissionDecisionReason"])


def test_registration_failure_is_silent() -> None:
    """登録できないときは何も出さずに終了コード 0（cc を止めない）。"""
    # 誰も listen していないポートへ送る
    code, out = run_hook(1, token="secret-token")
    assert code == 0, code
    assert out == "", repr(out)


def test_outside_agent_link_run_is_silent() -> None:
    """`AGENT_LINK_PARENT_RUN_ID` が無い cc（sai-moon の run など）では Web に送らず何も出さない。"""
    web = FakeWeb([ANSWERED_STATE])
    try:
        code, out = run_hook(web.port, extra_env={"AGENT_LINK_PARENT_RUN_ID": ""})
    finally:
        web.close()
    assert code == 0, code
    assert out == "", repr(out)
    assert web.registrations == [], web.registrations


def load_hook():
    """cc_question.py を import する（`__main__` の外の関数を直接確かめる）。"""
    spec = importlib.util.spec_from_file_location("cc_question", HOOK_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_free_text_answer_becomes_answers() -> None:
    """自由記述（Other）の答えは `{質問: 答え}` の答えになる（`-` で始まっても同じ）。"""
    hook = load_hook()
    data = {"state": "answered", "answer": {"answers": [{"text": "緑"}]}}
    assert hook.answers_from_state(data, QUESTIONS) == {"好きな色は？": "緑"}
    data = {"state": "answered", "answer": {"answers": [{"text": "-緑"}]}}
    assert hook.answers_from_state(data, QUESTIONS) == {"好きな色は？": "-緑"}


def test_parent_run_id_does_not_deny() -> None:
    """`AGENT_LINK_PARENT_RUN_ID` があっても承認の記録は行わず、答えが取れたら allow（`-` で始まる答えも）。"""
    web = FakeWeb([OPEN_STATE, ANSWERED_DASH_STATE])
    try:
        code, out = run_hook(web.port, token=None, extra_env={"AGENT_LINK_PARENT_RUN_ID": "61"})
    finally:
        web.close()
    assert code == 0, code
    hook = json.loads(out)["hookSpecificOutput"]
    assert hook["permissionDecision"] == "allow", out
    assert hook["updatedInput"]["answers"] == {"好きな色は？": "-緑"}, hook


def test_mark_working_is_silent_without_agent_link() -> None:
    """`agent-link` が PATH に無くても `mark_working` は例外を出さない。"""
    import os

    spec = importlib.util.spec_from_file_location("cc_question_hook", HOOK_PATH)
    hook = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(hook)
    old = os.environ.get("PATH")
    os.environ["PATH"] = ""
    try:
        hook.mark_working()
    finally:
        if old is None:
            os.environ.pop("PATH", None)
        else:
            os.environ["PATH"] = old


if __name__ == "__main__":
    test_answered_returns_allow_with_updated_input()
    test_deny_mode_returns_reason_with_answer()
    test_no_token_has_no_authorization()
    test_unanswered_reports_not_answered()
    test_withdrawn_with_text_denies_with_next_message()
    test_withdrawn_without_text_ends_turn()
    test_registration_failure_is_silent()
    test_outside_agent_link_run_is_silent()
    test_free_text_answer_becomes_answers()
    test_parent_run_id_does_not_deny()
    test_mark_working_is_silent_without_agent_link()
    print("test_cc_question_hook: すべて通りました")
