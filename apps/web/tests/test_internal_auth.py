"""内部の要求（localhost だけ）・質問・通知・Usage の形・古い DB（単独モード）のテスト。

確かめること:
- 内部の住所（`POST /api/register`・`POST /api/questions`・`GET /api/questions/{qid}` など）は
  127.0.0.1 / ::1 からだけ通り、ほかの住所からは 403
- 合言葉は無い（Authorization を付けても 403 が変わらない・内部でない住所は何も付けずに通る）
- 答えた記録の `answered_by` は `user`
- 通知は購読をすべてに送る
- `GET /api/usage` は `{"cc", "saved_at"}` だけを返す（`activity`・`logs` の住所は無い）
- `owner` の列・`answered_by` の列が無い古い DB を開いても動く
- 消した住所（`/api/auth/me`・`/api/customer`・`/api/questions/mine`・`/api/paper-projects`・
  `/api/mlflow/…`・`/api/radio/…`）は 404、`/api/days` に `customer` は無い
- 質問が開いている run に送る・停止すると質問を取り下げる（取り下げの住所も）

実行:
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_internal_auth.py
"""

from __future__ import annotations

import asyncio
import json
import os
import sqlite3
import sys
import tempfile
from pathlib import Path

# import より前に一時フォルダを決める（config.DB_PATH は import 時に決まるため）
_TMP = Path(tempfile.mkdtemp(prefix="agent-link-web-internal-"))
os.environ["AGENT_LINK_HOME"] = str(_TMP / "sai-home")
os.environ.pop("AGENT_LINK_WEB_RUNS_DIR", None)

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from agent_link_web import control, notify, runs, store, usage  # noqa: E402
from agent_link_web.main import app  # noqa: E402

HOST = "testserver"
LOCAL = ("127.0.0.1", 40000)
REMOTE = ("192.168.0.9", 40000)


# --- ASGI を直接叩く呼び出し係（httpx を使わない） ---


async def _run_asgi(
    method: str, path: str, body: bytes, headers: list[tuple[bytes, bytes]], client: tuple[str, int]
):
    if "?" in path:
        path_only, _, query = path.partition("?")
    else:
        path_only, query = path, ""
    scope = {
        "type": "http",
        "asgi": {"version": "3.0"},
        "http_version": "1.1",
        "method": method,
        "scheme": "http",
        "path": path_only,
        "raw_path": path_only.encode(),
        "query_string": query.encode(),
        "root_path": "",
        "headers": headers,
        "client": client,
        "server": (HOST, 80),
    }
    messages: list[dict] = []
    state = {"sent": False}

    async def receive():
        if state["sent"]:
            return {"type": "http.disconnect"}
        state["sent"] = True
        return {"type": "http.request", "body": body, "more_body": False}

    async def send(message):
        messages.append(message)

    await app(scope, receive, send)
    return messages


def call(
    method: str,
    path: str,
    *,
    json_body: dict | None = None,
    headers: dict | None = None,
    client: tuple[str, int] = LOCAL,
) -> tuple[int, object]:
    """1 つの住所を呼び、`(状態コード, 本文(JSON なら dict))` を返す。"""
    hdrs: list[tuple[bytes, bytes]] = [(b"host", HOST.encode())]
    body = b""
    if json_body is not None:
        body = json.dumps(json_body, ensure_ascii=False).encode("utf-8")
        hdrs.append((b"content-type", b"application/json"))
    for key, value in (headers or {}).items():
        hdrs.append((key.lower().encode(), value.encode()))
    messages = asyncio.run(_run_asgi(method, path, body, hdrs, client))
    start = next(m for m in messages if m["type"] == "http.response.start")
    raw = b"".join(m.get("body", b"") for m in messages if m["type"] == "http.response.body")
    try:
        parsed = json.loads(raw.decode("utf-8")) if raw else {}
    except (json.JSONDecodeError, UnicodeDecodeError):
        parsed = raw
    return start["status"], parsed


def question_body(session_id: str, tool_use_id: str) -> dict:
    return {
        "agent": "pi",
        "session_id": session_id,
        "pane_id": "pane-1",
        "tool_use_id": tool_use_id,
        "questions": [
            {
                "question": "どれにしますか？",
                "header": "選択",
                "multiSelect": False,
                "options": [{"label": "あ"}, {"label": "い"}],
            }
        ],
    }


def question_state(qid: str) -> tuple[str, str | None]:
    """DB を直接見て質問の状態と回答者を返す。"""
    with store._connect() as conn:  # noqa: SLF001
        row = conn.execute(
            "SELECT state, answered_by FROM questions WHERE id = ?", (qid,)
        ).fetchone()
    return (row[0], row[1] or None) if row else ("missing", None)


def test_internal_addresses_localhost_only() -> None:
    """内部の住所は localhost からだけ通る。合言葉（Authorization）は意味を持たない。"""
    status, body = call("POST", "/api/questions", json_body=question_body("s1", "tu-1"))
    assert status == 200, (status, body)
    qid = body["id"]

    status, body = call("POST", "/api/questions", json_body=question_body("s1", "tu-2"), client=REMOTE)
    assert status == 403 and body["detail"] == "内部専用です", (status, body)
    # Authorization を付けても、localhost でなければ 403 のまま
    status, body = call(
        "POST",
        "/api/questions",
        json_body=question_body("s1", "tu-3"),
        headers={"authorization": "Bearer any-token"},
        client=REMOTE,
    )
    assert status == 403 and body["detail"] == "内部専用です", (status, body)
    # IPv6 の localhost も通る
    status, body = call(
        "POST", "/api/questions", json_body=question_body("s1", "tu-4"), client=("::1", 40000)
    )
    assert status == 200, (status, body)

    # 質問 1 件の読み出し（拡張のポーリング用）も同じ
    status, body = call("GET", f"/api/questions/{qid}")
    assert status == 200 and body["id"] == qid, (status, body)
    status, body = call("GET", f"/api/questions/{qid}", client=REMOTE)
    assert status == 403 and body["detail"] == "内部専用です", (status, body)

    # 登録も同じ
    reg = {"agent": "claude", "session_id": "s1", "transcript_path": "", "cwd": "/tmp", "pane_id": "p"}
    status, body = call("POST", "/api/register", json_body=reg)
    assert status == 200 and body == {"ok": True}, (status, body)
    status, body = call("POST", "/api/register", json_body=reg, client=REMOTE)
    assert status == 403, (status, body)

    # 一覧（pending）は内部の住所ではない（合言葉は無いので誰でも通る）
    status, body = call("GET", "/api/questions/pending", client=REMOTE)
    assert status == 200 and qid in [q["id"] for q in body["questions"]], (status, body)


def test_answer_records_user() -> None:
    """答えた記録の `answered_by` は `user`。"""
    status, body = call("POST", "/api/questions", json_body=question_body("s-ans", "tu-ans"))
    assert status == 200, (status, body)
    qid = body["id"]
    answer = {"answers": [{"choice_index": 0}]}
    status, body = call("POST", f"/api/questions/{qid}/answer", json_body=answer, client=REMOTE)
    assert status == 200 and body == {"ok": True}, (status, body)
    assert question_state(qid) == ("answered", "user"), question_state(qid)


def test_removed_addresses() -> None:
    """消した住所は 404 になる。`/api/days` に `customer` は無い。"""
    for path in (
        "/api/auth/me",
        "/api/customer",
        "/api/paper-projects",
        "/api/mlflow/experiments",
        "/api/radio/health",
        "/api/prep/download",
        "/api/projects/x/container",
        "/api/projects/x/papers/library",
    ):
        status, body = call("GET", path)
        assert status == 404, (path, status, body)

    original = runs.load_runs
    runs.load_runs = lambda *a, **k: []  # noqa: E731
    try:
        status, body = call("GET", "/api/days")
        assert status == 200 and "customer" not in body and body["days"] == [], (status, body)
        status, body = call("GET", "/api/home")
        assert status == 200 and "papers" not in body, (status, body)
    finally:
        runs.load_runs = original


def test_notify_all_subscriptions() -> None:
    """通知は購読のすべてに送られる。"""
    sent: list[str] = []

    def fake_send_to(subscription, title, body, url):
        sent.append(subscription["endpoint"])
        return "ok"

    original = notify.send_to
    notify.send_to = fake_send_to
    try:
        store.save_subscription("a-endpoint", {"endpoint": "a-endpoint", "keys": {}})
        store.save_subscription("b-endpoint", {"endpoint": "b-endpoint", "keys": {}})
        count = notify.broadcast("t", "b", "/")
        assert count == 2 and sorted(sent) == ["a-endpoint", "b-endpoint"], (count, sent)
    finally:
        notify.send_to = original


def test_usage_shape() -> None:
    """`GET /api/usage` は `cc` と `saved_at` だけを返す（Authorization なしで通る・消した住所は 404）。"""
    status, body = call("GET", "/api/usage", client=REMOTE)
    assert status == 200, (status, body)
    assert set(body) == {"cc", "saved_at"}, body
    assert body["cc"] == usage.cc_card(), body
    assert set(body["cc"]["cost"]) == {"today", "month", "sessions"}, body
    assert isinstance(body["saved_at"], int), body
    for path in ("/api/usage/openrouter/main/activity", "/api/usage/openrouter/main/logs"):
        status, _ = call("GET", path)
        assert status == 404, (path, status)


def test_old_db() -> None:
    """`owner`・`answered_by` の列が無い古い DB を開いても動く。"""
    saved_db = store.DB_PATH
    tmp = Path(tempfile.mkdtemp(prefix="agent-link-web-olddb-"))
    db = tmp / "old.db"
    try:
        with sqlite3.connect(db) as conn:
            conn.executescript(
                """
                CREATE TABLE registrations (
                    session_id TEXT PRIMARY KEY, agent TEXT NOT NULL,
                    transcript_path TEXT NOT NULL, cwd TEXT NOT NULL,
                    pane_id TEXT NOT NULL, updated_at REAL NOT NULL);
                CREATE TABLE questions (
                    id TEXT PRIMARY KEY, agent TEXT NOT NULL, session_id TEXT NOT NULL,
                    pane_id TEXT NOT NULL, tool_use_id TEXT NOT NULL,
                    questions_json TEXT NOT NULL, created_at REAL NOT NULL,
                    state TEXT NOT NULL DEFAULT 'open',
                    answer_json TEXT NOT NULL DEFAULT '');
                CREATE TABLE subscriptions (
                    endpoint TEXT PRIMARY KEY, subscription_json TEXT NOT NULL,
                    created_at REAL NOT NULL);
                """
            )
        store.DB_PATH = db
        store.init_db()
        with store._connect() as conn:  # noqa: SLF001
            columns = {r[1] for r in conn.execute("PRAGMA table_info(questions)")}
        assert "answered_by" in columns, columns
        # 保存・読み出し・回答の記録ができる
        q = store.save_question("pi", "s", "p", "tu", [{"question": "?"}])
        assert store.get_question(q["id"]) is not None
        store.mark_answered(q["id"], {"answers": []}, "user")
        assert store.get_question(q["id"])["answered_by"] == "user"
        store.save_registration("pi", "s", "", "", "p")
        assert store.get_registration("s") is not None
        store.save_subscription("e", {"endpoint": "e"})
        assert len(store.list_subscriptions()) == 1
    finally:
        store.DB_PATH = saved_db


def test_send_and_interrupt_withdraw_questions() -> None:
    """質問が開いている run に送ると質問を取り下げて文を残す。停止は文なしで取り下げ、esc は送らない。

    取り下げの住所（`POST /api/questions/{qid}/withdraw`）も確かめる。
    """
    live = {
        "project_name": "demo", "project_root": "/tmp/a", "run_id": 7,
        "parent_run_id": None, "block_name": "main", "description": "実験",
        "runner": "cc", "status": "running", "model": None, "started_at": "2026-10-15T01:00:00Z",
        "finished_at": None, "duration_ms": None, "agent_session_id": "sess-live",
        "transcript_path": None, "updated_at": "2026-10-15T01:00:00Z", "stats": None,
    }
    sent: list[tuple] = []
    interrupted: list[tuple] = []
    saved = (runs.load_runs, control.run_send, control.run_interrupt)
    runs.load_runs = lambda *a, **k: [live]  # noqa: E731
    control.run_send = lambda name, run_id, text: sent.append((name, run_id, text)) or {  # noqa: E731
        "run_id": run_id, "pane_id": "p", "status": "sent",
    }
    control.run_interrupt = lambda name, run_id: interrupted.append((name, run_id)) or {  # noqa: E731
        "run_id": run_id, "status": "interrupted", "agent_status": "idle",
    }

    def ask(tool_use_id: str) -> str:
        status, body = call("POST", "/api/questions", json_body=question_body("sess-live", tool_use_id))
        assert status == 200, (status, body)
        return body["id"]

    try:
        # --- 送信: 質問が開いている間は tmux に送らず、取り下げて文を残す
        q1 = ask("tu-live-1")
        status, body = call("POST", "/api/runs/demo/7/send", json_body={"text": "表も作って"})
        assert status == 200 and body == {"run_id": 7, "status": "withdrawn", "question_ids": [q1]}, (status, body)
        assert sent == [], sent
        assert question_state(q1) == ("withdrawn", "user"), question_state(q1)
        status, body = call("GET", f"/api/questions/{q1}")
        assert body["state"] == "withdrawn" and body["answer"] == {"text": "表も作って"}, body

        # --- 質問が無ければ今までどおり送る
        status, body = call("POST", "/api/runs/demo/7/send", json_body={"text": "次"})
        assert status == 200 and body == {"run_id": 7, "pane_id": "p", "status": "sent"}, (status, body)
        assert sent == [("demo", 7, "次")], sent

        # --- 停止: 質問が開いていれば文なしで取り下げ、esc（control.run_interrupt）は送らない
        q2 = ask("tu-live-2")
        status, body = call("POST", "/api/runs/demo/7/interrupt")
        assert status == 200 and body == {"run_id": 7, "status": "interrupted", "agent_status": "blocked"}, (status, body)
        assert interrupted == [], interrupted
        assert question_state(q2) == ("withdrawn", "user"), question_state(q2)
        status, body = call("GET", f"/api/questions/{q2}")
        assert body["answer"] == {"text": ""}, body
        # 質問が無ければ esc を送る
        status, body = call("POST", "/api/runs/demo/7/interrupt")
        assert status == 200 and body["agent_status"] == "idle", (status, body)
        assert interrupted == [("demo", 7)], interrupted

        # --- 取り下げの住所: 開いていなければ 409、無ければ 404
        status, body = call("POST", f"/api/questions/{q2}/withdraw", json_body={"text": "x"})
        assert status == 409 and body["detail"] == "この質問はもう締め切られています", (status, body)
        q3 = ask("tu-live-3")
        status, body = call("POST", f"/api/questions/{q3}/withdraw", json_body={"text": "表は要らない"})
        assert status == 200 and body == {"ok": True}, (status, body)
        assert question_state(q3) == ("withdrawn", "user"), question_state(q3)
        status, body = call("POST", f"/api/questions/{q3}/withdraw", json_body={"text": "x" * 8001})
        assert status == 400, (status, body)
        status, body = call("POST", "/api/questions/no-such/withdraw")
        assert status == 404, (status, body)
    finally:
        runs.load_runs, control.run_send, control.run_interrupt = saved


def main() -> None:
    store.init_db()

    test_internal_addresses_localhost_only()
    test_answer_records_user()
    test_removed_addresses()
    test_notify_all_subscriptions()
    test_usage_shape()
    test_old_db()
    test_send_and_interrupt_withdraw_questions()

    print("test_internal_auth: すべて通りました")


if __name__ == "__main__":
    main()
