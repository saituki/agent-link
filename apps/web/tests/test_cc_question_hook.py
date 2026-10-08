"""cc_question.py（cc の PreToolUse フック）のテスト。

標準ライブラリの `http.server` で偽の Web を立て、`cc_question.py` を走らせて
次を確かめる。

- 登録 → 答え待ち → 答えが入った `allow` + `updatedInput` の JSON を出す
- `SAI_CC_QUESTION_MODE=deny` のときは答えを `permissionDecisionReason` に入れた `deny` を出す
- `SAI_WEB_INTERNAL_TOKEN` を付けると `Authorization: Bearer` が届く
- `SAI_WEB_URL` の値に送る
- 答えが来ないまま `open` でなくなったら「利用者は答えませんでした。」
- 答えが取れたら，`SAI_PARENT_RUN_ID` があるときだけ `sai-sci approval record` を呼ぶ（U04・C8）
  失敗しても allow を返す

実行:
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_cc_question_hook.py
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
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
ANSWERED_BY_STATE = {**ANSWERED_STATE, "answered_by": "saituki"}


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
        "SAI_WEB_URL": f"http://127.0.0.1:{port}",
        "HERDR_PANE_ID": "hl-0123456789ab",
    }
    if token is not None:
        env["SAI_WEB_INTERNAL_TOKEN"] = token
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
    # 合言葉があれば Authorization: Bearer が届く
    assert headers.get("Authorization") == "Bearer secret-token"
    # SAI_WEB_URL の値（偽の Web の住所）に送る
    assert web.gets and web.gets[0][0] == "/api/questions/q-test"
    # GET にも合言葉が届く
    assert web.gets[0][1].get("Authorization") == "Bearer secret-token"


def test_deny_mode_returns_reason_with_answer() -> None:
    """SAI_CC_QUESTION_MODE=deny なら、答えがあっても今の deny と理由文の形。"""
    web = FakeWeb([OPEN_STATE, ANSWERED_STATE])
    try:
        code, out = run_hook(web.port, token=None, extra_env={"SAI_CC_QUESTION_MODE": "deny"})
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
    """合言葉が無ければ Authorization を付けない（単独モードは今のまま）。"""
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


def test_registration_failure_is_silent() -> None:
    """登録できないときは何も出さずに終了コード 0（cc を止めない）。"""
    # 誰も listen していないポートへ送る
    code, out = run_hook(1, token="secret-token")
    assert code == 0, code
    assert out == "", repr(out)


def make_fake_sai_sci(dir: Path, exit_code: int) -> tuple[Path, Path]:
    """渡された引数を 1 行ずつ log に書き，exit_code で終わる偽の sai-sci を作る。"""
    log = dir / "args.log"
    script = dir / "sai-sci"
    script.write_text(f'#!/bin/sh\nprintf \'%s\\n\' "$@" > "{log}"\nexit {exit_code}\n', encoding="utf-8")
    script.chmod(0o755)
    return script, log


def test_answered_calls_approval_record() -> None:
    """答えが取れたら approval record に qid・質問・答え・回答者が渡る（AC2・C8）。"""
    with tempfile.TemporaryDirectory() as tmp:
        script, log = make_fake_sai_sci(Path(tmp), exit_code=0)
        web = FakeWeb([OPEN_STATE, ANSWERED_BY_STATE])
        try:
            code, out = run_hook(
                web.port,
                token=None,
                extra_env={"SAI_PARENT_RUN_ID": "61", "SAI_SCI_BIN": str(script)},
            )
        finally:
            web.close()
        assert code == 0, code
        # allow はそのまま返る（記録は答えを返す前に終わっている）
        assert json.loads(out)["hookSpecificOutput"]["permissionDecision"] == "allow", out
        args = log.read_text(encoding="utf-8").splitlines()
        assert args == [
            "approval",
            "record",
            "--qid",
            "q-test",
            "--question",
            "好きな色は？",
            "--answer",
            "青",
            "--answered-by",
            "saituki",
        ], args


def test_no_run_id_skips_record() -> None:
    """SAI_PARENT_RUN_ID が無ければ approval record を呼ばない（手動のエージェント）。"""
    with tempfile.TemporaryDirectory() as tmp:
        script, log = make_fake_sai_sci(Path(tmp), exit_code=0)
        web = FakeWeb([OPEN_STATE, ANSWERED_BY_STATE])
        try:
            code, out = run_hook(web.port, token=None, extra_env={"SAI_SCI_BIN": str(script)})
        finally:
            web.close()
        assert code == 0, code
        assert json.loads(out)["hookSpecificOutput"]["permissionDecision"] == "allow", out
        assert not log.exists(), "親の run が無いのに approval record を呼びました"


def test_record_failure_is_ignored() -> None:
    """approval record が失敗しても（終了コード 4・呼べない）allow は返す。"""
    with tempfile.TemporaryDirectory() as tmp:
        script, log = make_fake_sai_sci(Path(tmp), exit_code=4)
        web = FakeWeb([OPEN_STATE, ANSWERED_BY_STATE])
        try:
            code, out = run_hook(
                web.port,
                token=None,
                extra_env={"SAI_PARENT_RUN_ID": "61", "SAI_SCI_BIN": str(script)},
            )
        finally:
            web.close()
        assert code == 0, code
        assert json.loads(out)["hookSpecificOutput"]["permissionDecision"] == "allow", out
        assert log.exists(), "approval record が呼ばれていません"

        # 実行できない場所を指しても同じ（allow のまま）
        web = FakeWeb([OPEN_STATE, ANSWERED_BY_STATE])
        try:
            code, out = run_hook(
                web.port,
                token=None,
                extra_env={"SAI_PARENT_RUN_ID": "61", "SAI_SCI_BIN": str(Path(tmp) / "no-such-sai-sci")},
            )
        finally:
            web.close()
        assert code == 0, code
        assert json.loads(out)["hookSpecificOutput"]["permissionDecision"] == "allow", out


if __name__ == "__main__":
    test_answered_returns_allow_with_updated_input()
    test_deny_mode_returns_reason_with_answer()
    test_no_token_has_no_authorization()
    test_unanswered_reports_not_answered()
    test_registration_failure_is_silent()
    test_answered_calls_approval_record()
    test_no_run_id_skips_record()
    test_record_failure_is_ignored()
    print("test_cc_question_hook: すべて通りました")
