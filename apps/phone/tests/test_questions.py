"""質問・承認の状態遷移と，承認フックの振る舞いの小さなテスト（assertだけ）・05 §6．

実行:
    python3 tests/test_questions.py
    （または PYTHONPATH=src python3 -m pytest tests/test_questions.py）
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

# リポジトリ直下から実行しなくても import できるようにしておく
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_phone import approvals, herdr, questions, store  # noqa: E402

# 本物の herdr を呼ばない（一覧が空＝herdr 不明なので「ペインが消えた」扱いにならない）
herdr.list_panes = lambda: []  # type: ignore

HOOK_PATH = Path(__file__).resolve().parent.parent / "hooks" / "cc_permission.py"


def fresh_db(tmp: str) -> None:
    """テスト用の一時 DB に差し替える（開発用の DB を汚さない）．

    同じ tmp を2回使ったときの中身が混ざらないよう，前回分は消す．
    """
    path = Path(tmp) / "test.db"
    if path.exists():
        path.unlink()
    store.DB_PATH = path
    store.init_db()


def make_question(agent: str, n: int = 1) -> dict:
    qs = [
        {
            "question": f"質問{i + 1}は？",
            "header": f"見出し{i + 1}",
            "multiSelect": False,
            "options": [{"label": "あ"}, {"label": "い"}],
        }
        for i in range(n)
    ]
    return store.save_question(agent, "sess-1", "pane-1", f"tu-{agent}-{n}", qs)


def state_of(qid: str) -> str:
    q = store.get_question(qid)
    assert q is not None
    return q["state"]


def test_pi_question_stays_open_when_pane_not_blocked() -> None:
    """pi の質問はペインが blocked でなくても open のまま（05 §6 の必須項目）．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("pi")["id"]
        # ペインは idle（blocked にならない＝pi の実際の状態）
        herdr.get_agent = lambda pane_id: {"agent_status": "idle"}  # type: ignore
        questions.refresh_question_state("sess-1", "pane-1", "pi")
        assert state_of(qid) == "open"

        # pi への回答はキーを打ち込まず，保存するだけ（herdr が死んでいても成功）
        herdr.get_agent = lambda pane_id: None  # type: ignore
        ok, _reason = questions.answer_question(qid, [{"choice_index": 1}])
        assert ok is True
        assert state_of(qid) == "answered"
        q = store.get_question(qid)
        assert q["answer"]["answers"] == [{"choice_index": 1}]


def test_claude_question_resolved_elsewhere() -> None:
    """cc の質問はペインが blocked でなくなったら「PC で回答済み」になる．

    1 回の読み違いで決めつけないため，2 回続けて見えないときだけ
    「PC で回答済み」になる（計画書 B U08）．
    """
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("claude")["id"]
        herdr.get_agent = lambda pane_id: {"agent_status": "idle"}  # type: ignore
        herdr.read_pane = lambda pane_id, lines=60: "普通の画面です"  # type: ignore
        # 1 回だけでは決めつけない（再描画の隙を取り違えない）
        questions.refresh_question_state("sess-1", "pane-1", "claude")
        assert state_of(qid) == "open"
        # 2 回続けて質問が見えなかったら「PC で回答済み」
        questions.refresh_question_state("sess-1", "pane-1", "claude")
        assert state_of(qid) == "resolved_elsewhere"
        # 画面には「PC で回答済み」として残る（消さない・05 §1 の2番）
        rows = store.list_questions_for_session("sess-1")
        assert [r["state"] for r in rows] == ["resolved_elsewhere"]

        # 拡張からの「PC で答えた」も同じ状態にする（05 §3.1）
        fresh_db(tmp)
        qid = make_question("pi")["id"]
        assert questions.resolve_on_terminal(qid) is True
        assert state_of(qid) == "resolved_elsewhere"
        # 2回目は変更しない（回答済みを上書きしないため）
        assert questions.resolve_on_terminal(qid) is False


# 質問画面のフッター（実機で確認した文言）
QUESTION_SCREEN = """
   1. A1
   2. A2
   3. A3
   4. A4
   5. Type something.
   6. Chat about this

 Enter to select · Tab/Arrow keys to navigate · Esc to cancel
"""

# 2 問以上でのレビュー画面（実機で確認した文言）
REVIEW_SCREEN = """
 Review your answers

 ← 1. 質問1は？ A1
   2. 質問2は？ B2

   1. Submit answers
   2. Edit answers

 Enter to select
"""


def test_one_miss_does_not_resolve_but_two_do() -> None:
    """1 回の読み違いでは「回答済み」にせず，2 回続くと「回答済み」にする．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("claude", n=2)["id"]
        herdr.get_agent = lambda pane_id: {"agent_status": "blocked"}  # type: ignore
        screen = {"text": QUESTION_SCREEN}
        herdr.read_pane = lambda pane_id, lines=60: screen["text"]  # type: ignore

        # 質問が出ている間は open のまま
        questions.refresh_question_state("sess-1", "pane-1", "claude")
        assert state_of(qid) == "open"

        # 1 回だけ読めなかった（再描画の隙に相当する読み違い）→ open のまま
        screen["text"] = ""
        questions.refresh_question_state("sess-1", "pane-1", "claude")
        assert state_of(qid) == "open"

        # もう一度見えたら数え直しは消える（2 回続かなければ解決にしない）
        screen["text"] = QUESTION_SCREEN
        questions.refresh_question_state("sess-1", "pane-1", "claude")
        screen["text"] = ""
        questions.refresh_question_state("sess-1", "pane-1", "claude")
        assert state_of(qid) == "open"

        # 2 回続けて見えなかったら「PC で回答済み」
        questions.refresh_question_state("sess-1", "pane-1", "claude")
        assert state_of(qid) == "resolved_elsewhere"


def test_review_screen_is_not_resolved_elsewhere() -> None:
    """レビュー画面（回答処理の途中）は「PC で回答済み」にしない．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("claude", n=2)["id"]
        herdr.get_agent = lambda pane_id: {"agent_status": "blocked"}  # type: ignore
        screen = {"text": REVIEW_SCREEN}
        herdr.read_pane = lambda pane_id, lines=60: screen["text"]  # type: ignore
        for _ in range(5):
            questions.refresh_question_state("sess-1", "pane-1", "claude")
        assert state_of(qid) == "open"


def test_concurrent_answers_run_only_once() -> None:
    """同じ質問への同時の回答は 1 つだけ処理し，もう 1 つは「送信中です」．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("claude", n=2)["id"]
        herdr.get_agent = lambda pane_id: {"agent_status": "blocked"}  # type: ignore
        herdr.read_pane = lambda pane_id, lines=60: QUESTION_SCREEN  # type: ignore
        started = threading.Event()
        results: list[tuple[bool, str]] = []
        real_send = questions._send_answers_cc  # noqa: SLF001

        def slow_send(pane_id, qs, answers, qid=""):  # type: ignore[no-untyped-def]
            # キーを打つ代わりに「送信中」を真似する（ロックの検証だけが必要）
            started.set()
            time.sleep(1.0)
            return (True, "")

        questions._send_answers_cc = slow_send  # type: ignore[assignment]
        try:
            def worker() -> None:
                results.append(
                    questions.answer_question(qid, [{"choice_index": 0}, {"choice_index": 1}])
                )

            threads = [threading.Thread(target=worker) for _ in range(2)]
            for t in threads:
                t.start()
            assert started.wait(5), "1 つ目の送信が始まらなかった"
            for t in threads:
                t.join(30)
        finally:
            questions._send_answers_cc = real_send  # type: ignore[assignment]
        # 1 つだけ「送信中です」で弾かれ、1 つだけ実際に送る
        assert sorted(r[1] for r in results) == ["", "送信中です"]
        assert state_of(qid) == "answered"


def test_answer_requires_all_questions() -> None:
    """未回答の問が残っている間は送らない（05 §1 の2番の保険）．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("pi", n=2)["id"]
        ok, reason = questions.answer_question(qid, [{"choice_index": 0}, {}])
        assert ok is False
        assert "未回答" in reason
        assert state_of(qid) == "open"


def test_pi_question_expires_after_10_minutes() -> None:
    """pi の open な質問は10分経ったら expired（05 §3.1）．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("pi")["id"]
        # 登録時刻を11分前に遅らせる
        with store._connect() as conn:  # noqa: SLF001
            conn.execute(
                "UPDATE questions SET created_at = ? WHERE id = ?",
                (time.time() - 11 * 60, qid),
            )
        assert questions.expire_stale_questions() == 1
        assert state_of(qid) == "expired"
        # expired は画面に出さない
        assert store.list_questions_for_session("sess-1") == []
        # 2回目は何もしない
        assert questions.expire_stale_questions() == 0


APPROVAL_SCREEN_4 = """
 Bash command
   rm -v /tmp/x/victim.txt
   Delete the confirmed target file

 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and always allow access to
      /tmp/x from this project
   3. Yes, and switch to auto mode · auto mode handles
      these prompts for you
   4. No

 Esc to cancel · Tab to amend
"""

APPROVAL_SCREEN_3 = """
 Bash command
   cat /tmp/x/victim.txt

 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and switch to auto mode
   3. No

 Esc to cancel
"""


def make_approval(suggestions: list | None = None) -> dict:
    return store.save_approval(
        session_id="sess-1",
        pane_id="pane-1",
        tool_use_id="appr-1",
        tool_name="Bash",
        tool_input={"command": "rm -v /tmp/x/victim.txt"},
        permission_suggestions=suggestions if suggestions is not None else [
            {"type": "addRules", "rules": [{"toolName": "Read", "ruleContent": "/tmp/x/**"}]}
        ],
        match_text="rm -v /tmp/x/victim.txt",
    )


def test_approval_state_transitions() -> None:
    """承認の状態: open → answered / resolved_elsewhere（05 §6）．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        sent: list[str] = []
        typed: list[str] = []
        # 画面は「キーを送ると閉じる」ものを再現（実機と同じ動き）
        screen = {"text": APPROVAL_SCREEN_4}
        herdr.get_agent = lambda pane_id: {"agent_status": "blocked"}  # type: ignore
        herdr.read_pane = lambda pane_id, lines=60: screen["text"]  # type: ignore
        def fake_keys(pane_id, *keys):
            sent.append(keys[0])
            screen["text"] = ""  # 決定すると画面が閉じる（観察済み）
            return True
        def fake_text(pane_id, text):
            typed.append(text)
            return True
        herdr.send_keys = fake_keys  # type: ignore
        herdr.send_text = fake_text  # type: ignore

        a = make_approval()
        aid = a["id"]
        assert a["state"] == "open"

        # 許可 → 番号キー 1 で確定（実機で観察したキー・05 §4）
        ok, reason = approvals.answer(aid, "allow")
        assert ok is True, reason
        assert sent == ["1"]
        assert store.get_approval(aid)["state"] == "answered"

        # 回答済みに再度答えることはできない
        ok, _ = approvals.answer(aid, "deny", "理由")
        assert ok is False

        # 拒否は「No」の番号を画面から数える（選択肢の数が違う画面でも動く）
        # かつ拒否理由は通常の入力欄へ alt+enter で送る（観察済み・05 §4）
        fresh_db(tmp)
        sent.clear()
        typed.clear()
        screen["text"] = APPROVAL_SCREEN_3
        a2 = make_approval()
        ok, reason = approvals.answer(a2["id"], "deny", "やめておけ")
        assert ok is True, reason
        assert sent == ["3", "alt+enter"]
        assert typed == ["やめておけ"]
        assert store.get_approval(a2["id"])["state"] == "answered"

        # 「今後も許可」は permission_suggestions があるときだけ（05 §1 の3番）
        fresh_db(tmp)
        sent.clear()
        screen["text"] = APPROVAL_SCREEN_4
        a3 = make_approval(suggestions=[])
        ok, _ = approvals.answer(a3["id"], "always")
        assert ok is False
        assert store.get_approval(a3["id"])["state"] == "open"
        ok, reason = approvals.answer(a3["id"], "allow")
        assert ok is True, reason
        assert sent == ["1"]


def test_approval_resolved_elsewhere_and_mismatch() -> None:
    """PC で先に答えた場合は resolved_elsewhere，違う画面には答えない（05 §3.3）．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        a = make_approval()

        # ペインが blocked でなくなった → PC で回答済み
        herdr.get_agent = lambda pane_id: {"agent_status": "idle"}  # type: ignore
        approvals.refresh("sess-1", "pane-1")
        assert store.get_approval(a["id"])["state"] == "resolved_elsewhere"

        # 別の承認画面（内容が違う）の前では送らない
        fresh_db(tmp)
        a2 = make_approval()
        sent: list[str] = []
        herdr.get_agent = lambda pane_id: {"agent_status": "blocked"}  # type: ignore
        herdr.read_pane = lambda pane_id, lines=60: "別の画面です"  # type: ignore
        herdr.send_keys = lambda pane_id, *keys: (sent.append(keys[0]) or True)  # type: ignore
        ok, reason = approvals.answer(a2["id"], "allow")
        assert ok is False
        assert sent == []
        assert store.get_approval(a2["id"])["state"] == "open"


def _run_hook(stdin_text: str, port: str) -> tuple[int, str]:
    """フックを実行し（終了コード, 標準出力）を返す．"""
    env = {
        "PATH": "/usr/bin:/bin",
        "SAI_PHONE_PORT": port,
    }
    proc = subprocess.run(
        [sys.executable, str(HOOK_PATH)],
        input=stdin_text,
        capture_output=True,
        text=True,
        timeout=15,
        env=env,
    )
    return proc.returncode, proc.stdout + proc.stderr


def test_approval_hook_is_silent() -> None:
    """承認フックは何も出力せず 0 で終わる（サーバー起動中・停止中の両方）・05 §6．"""
    payload = json.dumps(
        {
            "session_id": "sess-1",
            "hook_event_name": "PermissionRequest",
            "tool_name": "Bash",
            "tool_input": {"command": "rm -rf build/"},
        }
    )
    received: list[bytes] = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:  # noqa: N802
            length = int(self.headers.get("Content-Length") or 0)
            received.append(self.rfile.read(length))
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"ok": true}')

        def log_message(self, *args) -> None:  # 騒がしいログは出さない
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    port = str(server.server_address[1])
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        # 1) サーバー起動中: 出力なし・0 で終わり，登録が届く
        code, out = _run_hook(payload, port)
        assert code == 0, code
        assert out == "", repr(out)
        # 少し待つ（POST の完了をフックは待ちに来ないため）
        time.sleep(0.3)
        assert len(received) == 1
        body = json.loads(received[0])
        assert body["tool_name"] == "Bash"
        assert body["pane_id"] == ""  # HERDR_PANE_ID が無い環境では空
    finally:
        server.shutdown()
        server.server_close()

    # 2) サーバー停止中: それでも出力なし・0（cc を止めない・05 §3.3）
    code, out = _run_hook(payload, port)
    assert code == 0, code
    assert out == "", repr(out)

    # 3) JSON が壊れているときも出力なし・0
    code, out = _run_hook("this is not json", port)
    assert code == 0, code
    assert out == "", repr(out)


def test_gone_pane_expires_cards() -> None:
    """ペインが消えたら（agent 終了）質問・承認は expired になり画面から消える．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("pi")["id"]
        a = make_approval()
        herdr.list_panes = lambda: [{"pane_id": "other"}]  # type: ignore
        try:
            questions.refresh_question_state("sess-1", "pane-1", "pi")
            approvals.refresh("sess-1", "pane-1")
        finally:
            herdr.list_panes = lambda: []  # type: ignore
        assert state_of(qid) == "expired"
        assert store.get_approval(a["id"])["state"] == "expired"
        assert store.list_questions_for_session("sess-1") == []
        assert store.list_approvals_for_session("sess-1") == []


def test_poll_does_not_resolve_while_sending() -> None:
    """送信中（レビュー画面のあと画面が閉じた瞬間）は「PC で回答済み」にしない．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("claude", n=2)["id"]
        herdr.get_agent = lambda pane_id: {"agent_status": "blocked"}  # type: ignore
        screen = {"text": REVIEW_SCREEN}
        herdr.read_pane = lambda pane_id, lines=60: screen["text"]  # type: ignore
        started = threading.Event()
        real_send = questions._send_answers_cc  # noqa: SLF001

        def slow_send(pane_id, qs, answers, qid=""):  # type: ignore[no-untyped-def]
            started.set()
            # 送信の途中で画面が閉じる（レビューが消えた状態）
            screen["text"] = "ふつうの画面"
            time.sleep(1.0)
            return (True, "")

        questions._send_answers_cc = slow_send  # type: ignore[assignment]
        results: list[tuple[bool, str]] = []
        try:
            th = threading.Thread(
                target=lambda: results.append(
                    questions.answer_question(
                        qid, [{"choice_index": 0}, {"choice_index": 1}]
                    )
                )
            )
            th.start()
            assert started.wait(5), "送信が始まらなかった"
            # 送信中に何度問い合わせても open のまま
            for _ in range(4):
                questions.refresh_question_state("sess-1", "pane-1", "claude")
            assert state_of(qid) == "open"
            th.join(30)
        finally:
            questions._send_answers_cc = real_send  # type: ignore[assignment]
        assert results == [(True, "")]
        assert state_of(qid) == "answered"


def test_wrapped_question_text_is_found_on_screen() -> None:
    """折り返した質問文（行頭に枠の縦線 `│`）も「次の質問が出た」と判定できる．"""
    qtext = "sai-moon-mlflow の最初の版に入れる機能はどれですか？（実験一覧・入れ子の run 表・run 詳細・sai の run と node への往復リンクは必ず入れます）"
    screen = (
        "←  ☒ 既存の木  ☒ 往復の役  ☒ 置き場  ☐ 範囲  ✔ Submit  →\n\n"
        "│ sai-moon-mlflow の最初の版に入れる機能はどれですか？（実験一覧・入れ子の run 表・run 詳細・sai の run と node\n"
        "│ への往復リンクは必ず入れます）\n\n"
        "❯ 1. [ ] 指標の履歴グラフ\n\nEnter to select · Tab/Arrow keys to navigate · Esc to cancel\n"
    )
    qs = [{"question": "a"}, {"question": "b"}, {"question": "c"}, {"question": qtext}]
    assert questions._expected_after(screen, qs, 3)
    # 別の質問文なら一致しない
    assert not questions._expected_after(screen, [{"question": "全然ちがう質問"}] * 4, 3)


def main() -> None:
    test_pi_question_stays_open_when_pane_not_blocked()
    test_claude_question_resolved_elsewhere()
    test_one_miss_does_not_resolve_but_two_do()
    test_review_screen_is_not_resolved_elsewhere()
    test_concurrent_answers_run_only_once()
    test_poll_does_not_resolve_while_sending()
    test_answer_requires_all_questions()
    test_pi_question_expires_after_10_minutes()
    test_approval_state_transitions()
    test_approval_resolved_elsewhere_and_mismatch()
    test_approval_hook_is_silent()
    test_gone_pane_expires_cards()
    test_wrapped_question_text_is_found_on_screen()
    print("test_questions: すべて通りました")


if __name__ == "__main__":
    main()
