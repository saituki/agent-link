"""コンテナからの内部の要求・質問の持ち主・Usage の鍵（実装計画 U10）のテスト。

確かめること（AC1〜AC4）:
- プロジェクトの合言葉（`SAI_WEB_INTERNAL_TOKEN`）で `POST /api/questions` すると、
  その質問の `owner` が合言葉の持ち主になる
- 合言葉無しの内部の住所は 403、Cookie で内部の住所を呼んでも 403
- bob の Cookie では alice の質問は見えない（`GET /api/questions`・`/api/home`）
- bob の合言葉で alice の質問の `GET /api/questions/{qid}` は 404
- alice の合言葉で内部でない住所（`GET /api/projects`）は 403
- 通知は持ち主の購読にだけ送られる
- Usage の OpenRouter は多人数モードで `secrets.env` のキーを使う
- `owner` の列が無い古い DB を開いても動く（`init_db` が列を足す）
- 内部の合言葉の agent は、自分のプロジェクトの答え待ちだけを `GET /api/questions/mine` で読める
- 内部の合言葉で、別のプロジェクトの質問に答えると 403（質問は open のまま）
- 内部の合言葉で答えた記録は `answered_by` が `agent` になる

実行:
    cd ~/workdir/Sai/sai-tree
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_internal_auth.py
"""

from __future__ import annotations

import asyncio
import hashlib
import json
import os
import sqlite3
import sys
import tempfile
from pathlib import Path
from urllib.parse import quote

# import より前に一時フォルダを決める（config.DB_PATH は import 時に決まるため）
_TMP = Path(tempfile.mkdtemp(prefix="sai-web-internal-"))
os.environ["SAI_HOME"] = str(_TMP / "sai-home")
os.environ["SAI_WEB_USERS_ROOT"] = str(_TMP / "users")
os.environ["SAI_WEB_COOKIE_SECURE"] = "0"
os.environ.pop("SAI_WEB_TOKEN", None)
os.environ.pop("SAI_WEB_RUNS_DIR", None)

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_web import auth, config, containers, control, home, notify, registry, runs, store, usage  # noqa: E402
from sai_web.main import app  # noqa: E402

HOST = "testserver"
USERS = Path(os.environ["SAI_WEB_USERS_ROOT"])

ALICE_TOKEN = "alice-internal-token-123"
ALICE_TOKEN_BETA = "alice-internal-token-789"
BOB_TOKEN = "bob-internal-token-456"


# --- ASGI を直接叩く呼び出し係（httpx を使わない） ---


async def _run_asgi(method: str, path: str, body: bytes, headers: list[tuple[bytes, bytes]]):
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
        "client": ("127.0.0.1", 40000),
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
    cookie: str | None = None,
) -> tuple[int, object]:
    """1 つの住所を呼び、`(状態コード, 本文(JSON なら dict))` を返す。"""
    hdrs: list[tuple[bytes, bytes]] = [(b"host", HOST.encode())]
    body = b""
    if json_body is not None:
        body = json.dumps(json_body, ensure_ascii=False).encode("utf-8")
        hdrs.append((b"content-type", b"application/json"))
    if cookie:
        hdrs.append((b"cookie", f"{auth.SESSION_COOKIE}={cookie}".encode()))
    for key, value in (headers or {}).items():
        hdrs.append((key.lower().encode(), value.encode()))
    messages = asyncio.run(_run_asgi(method, path, body, hdrs))
    start = next(m for m in messages if m["type"] == "http.response.start")
    raw = b"".join(m.get("body", b"") for m in messages if m["type"] == "http.response.body")
    try:
        parsed = json.loads(raw.decode("utf-8")) if raw else {}
    except (json.JSONDecodeError, UnicodeDecodeError):
        parsed = raw
    return start["status"], parsed


def cookie_of(method: str, path: str, body: dict) -> str:
    hdrs: list[tuple[bytes, bytes]] = [
        (b"host", HOST.encode()),
        (b"content-type", b"application/json"),
    ]
    messages = asyncio.run(
        _run_asgi(method, path, json.dumps(body).encode("utf-8"), hdrs)
    )
    start = next(m for m in messages if m["type"] == "http.response.start")
    for key, value in start["headers"]:
        if key.decode() == "set-cookie" and value.decode().startswith(
            f"{auth.SESSION_COOKIE}="
        ):
            return value.decode().split(";", 1)[0].split("=", 1)[1]
    raise AssertionError("Cookie が付かない")


def login(email: str, password: str) -> str:
    return cookie_of("POST", "/api/auth/login", {"email": email, "password": password})


def add_user(name: str, email: str, password: str) -> None:
    auth.create_user_dirs(name)
    store.create_user(
        name, email, auth.hash_password(password), must_change_password=False
    )


def save_project_token(token: str, owner: str, project: str) -> None:
    store.save_project_token(hashlib.sha256(token.encode()).hexdigest(), owner, project)


def owner_of(qid: str) -> str | None:
    """DB を直接見て質問の持ち主を返す（store の絞り込みを通さない）。"""
    with store._connect() as conn:  # noqa: SLF001
        row = conn.execute("SELECT owner FROM questions WHERE id = ?", (qid,)).fetchone()
    return row[0] if row else None


def as_user(name: str):
    """`registry.current_user` を入れて、戻すためのトークンを返す。"""
    return registry.current_user.set({"name": name, "email": "", "is_admin": False})


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


def test_internal_question_owner() -> None:
    """AC1: 合言葉で登録した質問の持ち主が記録され、他人からは見えない。"""
    alice_bearer = {"authorization": f"Bearer {ALICE_TOKEN}"}
    bob_bearer = {"authorization": f"Bearer {BOB_TOKEN}"}

    # --- alice の合言葉で質問を登録 ---
    status, body = call(
        "POST",
        "/api/questions",
        json_body=question_body("alice-sess", "tu-alice"),
        headers=alice_bearer,
    )
    assert status == 200, (status, body)
    qid = body["id"]
    assert owner_of(qid) == "alice", owner_of(qid)

    # --- 合言葉無しの内部の住所は 403 ---
    status, body = call("POST", "/api/questions", json_body=question_body("x", "tu-x"))
    assert status == 403 and body["detail"] == "内部専用です", (status, body)
    # --- 知らない合言葉も 403 ---
    status, body = call(
        "POST",
        "/api/questions",
        json_body=question_body("x", "tu-y"),
        headers={"authorization": "Bearer no-such-token"},
    )
    assert status == 403 and body["detail"] == "内部専用です", (status, body)

    alice_cookie = login("alice@uni.edu", "alicepass1")
    bob_cookie = login("bob@uni.edu", "bobpass123")

    # --- Cookie で内部の住所を呼んでも 403 ---
    status, body = call(
        "POST",
        "/api/questions",
        json_body=question_body("z", "tu-z"),
        cookie=alice_cookie,
    )
    assert status == 403 and body["detail"] == "内部専用です", (status, body)

    # --- bob の Cookie では alice の質問は見えない ---
    status, body = call(
        "GET", "/api/questions?session_id=alice-sess", cookie=bob_cookie
    )
    if status == 200:
        assert qid not in [q["id"] for q in body["questions"]], body
    else:
        assert status == 404, (status, body)

    status, body = call("GET", "/api/home", cookie=bob_cookie)
    assert status == 200, (status, body)
    assert qid not in [q["id"] for q in body["questions"]], body

    # --- bob の合言葉で alice の質問は 404 ---
    status, body = call("GET", f"/api/questions/{qid}", headers=bob_bearer)
    assert status == 404, (status, body)
    # --- alice の合言葉なら読める ---
    status, body = call("GET", f"/api/questions/{qid}", headers=alice_bearer)
    assert status == 200 and body["id"] == qid, (status, body)

    # --- 内部の合言葉で内部でない住所は 403 ---
    status, body = call("GET", "/api/projects", headers=alice_bearer)
    assert status == 403 and body["detail"] == "内部専用の合言葉では使えません", (
        status,
        body,
    )
    # --- 一覧（pending）も内部の合言葉では使えない ---
    status, body = call("GET", "/api/questions/pending", headers=alice_bearer)
    assert status == 403 and body["detail"] == "内部専用の合言葉では使えません", (
        status,
        body,
    )


def test_agent_questions() -> None:
    """内部の合言葉の agent は、自分のプロジェクトの答え待ちだけを読み、答えられる。

    質問の実行（session_id）のプロジェクトは `home._session_map` の偽物で決める。
    """
    alpha = {"authorization": f"Bearer {ALICE_TOKEN}"}
    beta = {"authorization": f"Bearer {ALICE_TOKEN_BETA}"}
    fake_map = [
        ("alice-alpha", {"agent-alpha-sess"}, 1),
        ("alice-beta", {"agent-beta-sess"}, 2),
    ]
    original_map = home._session_map  # noqa: SLF001
    home._session_map = lambda: fake_map  # noqa: SLF001
    try:
        # --- alice のプロジェクト alpha と beta に、それぞれ質問を登録 ---
        status, body = call(
            "POST", "/api/questions",
            json_body=question_body("agent-alpha-sess", "tu-agent-alpha"), headers=alpha,
        )
        assert status == 200, (status, body)
        qid_alpha = body["id"]
        status, body = call(
            "POST", "/api/questions",
            json_body=question_body("agent-beta-sess", "tu-agent-beta"), headers=beta,
        )
        assert status == 200, (status, body)
        qid_beta = body["id"]

        # --- 一覧は自分のプロジェクトの分だけ（`{qid}` の住所に吸われない）---
        status, body = call("GET", "/api/questions/mine", headers=alpha)
        assert status == 200 and "questions" in body, (status, body)
        ids = [q["id"] for q in body["questions"]]
        assert qid_alpha in ids and qid_beta not in ids, body
        row = next(q for q in body["questions"] if q["id"] == qid_alpha)
        assert row["project"] == "alice-alpha" and row["run_id"] == 1, row
        status, body = call("GET", "/api/questions/mine", headers=beta)
        assert status == 200 and [q["id"] for q in body["questions"]] == [qid_beta], body

        # --- alpha の合言葉で beta の質問に答える → 403・質問は open のまま ---
        answer = {"answers": [{"choice_index": 0}]}
        status, body = call("POST", f"/api/questions/{qid_beta}/answer", json_body=answer, headers=alpha)
        assert status == 403 and body["detail"] == "このプロジェクトの質問ではありません", (status, body)
        assert owner_state(qid_beta) == ("open", None), owner_state(qid_beta)

        # --- alpha の合言葉で alpha の質問に答える → 200・answered_by は agent ---
        status, body = call("POST", f"/api/questions/{qid_alpha}/answer", json_body=answer, headers=alpha)
        assert status == 200 and body == {"ok": True}, (status, body)
        assert owner_state(qid_alpha) == ("answered", "agent"), owner_state(qid_alpha)

        # --- 答えた後は一覧に出ない ---
        status, body = call("GET", "/api/questions/mine", headers=alpha)
        assert status == 200 and qid_alpha not in [q["id"] for q in body["questions"]], body

        # --- 内部の合言葉では、一覧と home は今までどおり使えない ---
        status, body = call("GET", "/api/home", headers=alpha)
        assert status == 403 and body["detail"] == "内部専用の合言葉では使えません", (status, body)
    finally:
        home._session_map = original_map  # noqa: SLF001


def owner_state(qid: str) -> tuple[str, str | None]:
    """DB を直接見て質問の状態と回答者を返す（store の絞り込みを通さない）。"""
    with store._connect() as conn:  # noqa: SLF001
        row = conn.execute(
            "SELECT state, answered_by FROM questions WHERE id = ?", (qid,)
        ).fetchone()
    return (row[0], row[1] or None) if row else ("missing", None)


def test_notify_owner() -> None:
    """AC3: 通知は持ち主の購読にだけ送られる。"""
    sent: list[str] = []

    def fake_send_to(subscription, title, body, url):
        sent.append(subscription["endpoint"])
        return "ok"

    original = notify.send_to
    notify.send_to = fake_send_to
    try:
        token = as_user("alice")
        try:
            store.save_subscription("alice-endpoint", {"endpoint": "alice-endpoint", "keys": {}})
        finally:
            registry.current_user.reset(token)
        token = as_user("bob")
        try:
            store.save_subscription("bob-endpoint", {"endpoint": "bob-endpoint", "keys": {}})
        finally:
            registry.current_user.reset(token)

        token = as_user("alice")
        try:
            count = notify.broadcast("t", "b", "/")
        finally:
            registry.current_user.reset(token)
        assert count == 1 and sent == ["alice-endpoint"], (count, sent)

        sent.clear()
        token = as_user("bob")
        try:
            count = notify.broadcast("t", "b", "/")
        finally:
            registry.current_user.reset(token)
        assert count == 1 and sent == ["bob-endpoint"], (count, sent)
    finally:
        notify.send_to = original


def test_usage_key_from_secrets() -> None:
    """AC4: 多人数モードの OpenRouter は `secrets.env` のキーを使う。"""
    containers.write_keys("alice", {"OPENROUTER_API_KEY": "sk-or-secret"})
    os.environ["OPENROUTER_API_KEY"] = "env-secret"

    calls: list[dict] = []

    def fake_get_json(url, headers):
        calls.append(headers)
        return {"data": {}}

    original = usage._get_json  # noqa: SLF001
    usage._get_json = fake_get_json  # noqa: SLF001
    token = as_user("alice")
    try:
        card = usage.openrouter_card("main", "OPENROUTER_API_KEY")
    finally:
        registry.current_user.reset(token)
        usage._get_json = original  # noqa: SLF001
        os.environ.pop("OPENROUTER_API_KEY", None)

    assert card.get("error") is None, card
    assert calls, "外部 API を呼んでいない"
    assert calls[0]["Authorization"] == "Bearer sk-or-secret", calls[0]

    # --- 単独モードでは今までどおり環境変数から読む ---
    os.environ["OPENROUTER_API_KEY"] = "env-secret"
    saved = os.environ.pop("SAI_WEB_USERS_ROOT")
    try:
        assert usage._key_for("OPENROUTER_API_KEY") == "env-secret"  # noqa: SLF001
    finally:
        os.environ["SAI_WEB_USERS_ROOT"] = saved
        os.environ.pop("OPENROUTER_API_KEY", None)


def test_old_db_without_owner() -> None:
    """AC2: `owner` の列が無い古い DB を開いても動く（単独モード）。"""
    saved_root = os.environ.pop("SAI_WEB_USERS_ROOT")
    saved_db = store.DB_PATH
    tmp = Path(tempfile.mkdtemp(prefix="sai-web-olddb-"))
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
            for table in ("registrations", "questions", "subscriptions"):
                columns = {r[1] for r in conn.execute(f"PRAGMA table_info({table})")}
                assert "owner" in columns, (table, columns)
        # 保存・読み出しができる（単独モードは owner が空文字）
        q = store.save_question("pi", "s", "p", "tu", [{"question": "?"}])
        assert q["owner"] == "", q
        assert store.get_question(q["id"]) is not None
    finally:
        store.DB_PATH = saved_db
        os.environ["SAI_WEB_USERS_ROOT"] = saved_root


def test_feedback_receiver() -> None:
    """AC3: レポートは DATA_DIR/feedback/<kind>/ に frontmatter 付きで 0600 で保存する。

    多人数モードは内部の合言葉が要り、project と利用者は合言葉の持ち主で決まる。
    """
    alice = {"authorization": f"Bearer {ALICE_TOKEN}"}
    body = {"kind": "report", "text": "## 不具合\n表が崩れる", "run_id": 42, "project": "other-proj"}

    status, res = call("POST", "/api/feedback", json_body=body, headers=alice)
    assert status == 200 and res["ok"] is True, (status, res)
    folder = config.DATA_DIR / "feedback" / "report"
    path = folder / f"{res['id']}.md"
    assert path.exists(), path
    # 保存先のパスは返さない
    assert str(config.DATA_DIR) not in json.dumps(res, ensure_ascii=False), res
    text = path.read_text(encoding="utf-8")
    assert text.startswith(
        "---\nkind: report\nuser: alice\nproject: alice-alpha\nrun_id: 42\nreceived_at: "
    ), text[:200]
    assert text.endswith("---\n\n## 不具合\n表が崩れる\n"), text
    assert oct(path.stat().st_mode & 0o777) == "0o600", oct(path.stat().st_mode)
    assert oct(folder.stat().st_mode & 0o777) == "0o700", oct(folder.stat().st_mode)

    # 2 件目も別のファイルになる（同じ名前は上書きしない）
    status, res2 = call("POST", "/api/feedback", json_body=body, headers=alice)
    assert status == 200 and res2["id"] != res["id"], (status, res2)
    assert (folder / f"{res2['id']}.md").exists()

    # 内部の合言葉が無いと 403（Cookie の利用者でも内部の住所は 403）
    status, res = call("POST", "/api/feedback", json_body=body)
    assert status == 403 and res["detail"] == "内部専用です", (status, res)
    bob_cookie = login("bob@uni.edu", "bobpass123")
    status, res = call("POST", "/api/feedback", json_body=body, cookie=bob_cookie)
    assert status == 403 and res["detail"] == "内部専用です", (status, res)

    # 入力の誤りは 400（種類・空の文・run_id・project・200KB 超え）
    for bad in (
        {"kind": "bug", "text": "x"},
        {"kind": "report", "text": "  \n"},
        {"kind": "report", "text": "x", "run_id": 0},
        {"kind": "report", "text": "x", "run_id": True},
        {"kind": "report", "text": "x", "project": "../etc"},
        {"kind": "report", "text": "x" * (200 * 1024 + 1)},
    ):
        status, res = call("POST", "/api/feedback", json_body=bad, headers=alice)
        assert status == 400, (bad.get("kind"), status, res)

    # 単独モードは 127.0.0.1 から、合言葉無しで受け付ける（project は本文の値・利用者は local）
    saved_root = os.environ.pop("SAI_WEB_USERS_ROOT")
    try:
        status, res = call("POST", "/api/feedback", json_body=body)
        assert status == 200 and res["ok"] is True, (status, res)
        text = (folder / f"{res['id']}.md").read_text(encoding="utf-8")
        assert "user: local\nproject: other-proj\n" in text, text[:200]
    finally:
        os.environ["SAI_WEB_USERS_ROOT"] = saved_root


def test_customer_days_and_list() -> None:
    """AC1: /api/days は Customer を数えず `customer` の要約を返す。/api/customer は Customer だけ新しい順。"""
    cookie = login("alice@uni.edu", "alicepass1")

    def run(run_id: int, block: str, status: str, started: str, description: str) -> dict:
        return {
            "project_name": "alice-alpha", "project_root": "/tmp/a", "run_id": run_id,
            "parent_run_id": None, "block_name": block, "description": description,
            "runner": "cc", "status": status, "model": None, "started_at": started,
            "finished_at": None, "duration_ms": None, "agent_session_id": None,
            "transcript_path": None, "updated_at": started, "stats": None,
        }

    fake = [
        run(1, "sai2", "completed", "2026-10-15T01:00:00Z", "通常の実験"),
        run(2, "user-report", "running", "2026-10-15T03:00:00Z", "Customer: バグの報告"),
        run(3, "user-question", "completed", "2026-10-15T04:00:00Z", "Customer: 質問"),
    ]
    original = runs.load_runs
    runs.load_runs = lambda *a, **k: fake  # noqa: E731
    try:
        status, body = call("GET", "/api/days", cookie=cookie)
        assert status == 200, (status, body)
        assert [d["date"] for d in body["days"]] == ["2026-10-15"], body["days"]
        assert body["days"][0]["done"] == {"cc": 1} and body["days"][0]["running"] == 0, body["days"]
        assert body["customer"] == {"count": 2, "running": 1, "has_question": False}, body["customer"]
        status, body = call("GET", "/api/days/2026-10-15", cookie=cookie)
        assert [r["run_id"] for r in body["projects"][0]["runs"]] == [1], body
        # 検索には Customer も出る
        status, body = call("GET", "/api/days?q=" + quote("バグ"), cookie=cookie)
        assert [r["run_id"] for r in body["results"]] == [2], body["results"]

        status, body = call("GET", "/api/customer", cookie=cookie)
        assert status == 200, (status, body)
        assert [r["run_id"] for r in body["runs"]] == [3, 2], body
        assert body["count"] == 2 and body["running"] == 1 and body["has_question"] is False, body
        # 内部の合言葉では使えない（一覧の住所）
        status, body = call("GET", "/api/customer", headers={"authorization": f"Bearer {ALICE_TOKEN}"})
        assert status == 403, (status, body)
    finally:
        runs.load_runs = original


def test_send_and_interrupt_withdraw_questions() -> None:
    """AC5: 質問が開いている run に送ると質問を取り下げて文を残す。停止は文なしで取り下げ、esc は送らない。

    取り下げの住所（`POST /api/questions/{qid}/withdraw`）も確かめる。
    """
    alice = {"authorization": f"Bearer {ALICE_TOKEN}"}
    cookie = login("alice@uni.edu", "alicepass1")
    live = {
        "project_name": "alice-alpha", "project_root": "/tmp/a", "run_id": 7,
        "parent_run_id": None, "block_name": "sai2", "description": "実験",
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
        status, body = call("POST", "/api/questions", json_body=question_body("sess-live", tool_use_id), headers=alice)
        assert status == 200, (status, body)
        return body["id"]

    try:
        # --- 送信: 質問が開いている間は tmux に送らず、取り下げて文を残す
        q1 = ask("tu-live-1")
        status, body = call("POST", "/api/runs/alice-alpha/7/send", json_body={"text": "表も作って"}, cookie=cookie)
        assert status == 200 and body == {"run_id": 7, "status": "withdrawn", "question_ids": [q1]}, (status, body)
        assert sent == [], sent
        assert owner_state(q1) == ("withdrawn", "alice"), owner_state(q1)
        status, body = call("GET", f"/api/questions/{q1}", headers=alice)
        assert body["state"] == "withdrawn" and body["answer"] == {"text": "表も作って"}, body

        # --- 質問が無ければ今までどおり送る
        status, body = call("POST", "/api/runs/alice-alpha/7/send", json_body={"text": "次"}, cookie=cookie)
        assert status == 200 and body == {"run_id": 7, "pane_id": "p", "status": "sent"}, (status, body)
        assert sent == [("alice-alpha", 7, "次")], sent

        # --- 停止: 質問が開いていれば文なしで取り下げ、esc（control.run_interrupt）は送らない
        q2 = ask("tu-live-2")
        status, body = call("POST", "/api/runs/alice-alpha/7/interrupt", cookie=cookie)
        assert status == 200 and body == {"run_id": 7, "status": "interrupted", "agent_status": "blocked"}, (status, body)
        assert interrupted == [], interrupted
        assert owner_state(q2) == ("withdrawn", "alice"), owner_state(q2)
        status, body = call("GET", f"/api/questions/{q2}", headers=alice)
        assert body["answer"] == {"text": ""}, body
        # 質問が無ければ esc を送る
        status, body = call("POST", "/api/runs/alice-alpha/7/interrupt", cookie=cookie)
        assert status == 200 and body["agent_status"] == "idle", (status, body)
        assert interrupted == [("alice-alpha", 7)], interrupted

        # --- 取り下げの住所: 開いていなければ 409、無ければ 404、内部の合言葉は 403
        status, body = call("POST", f"/api/questions/{q2}/withdraw", json_body={"text": "x"}, cookie=cookie)
        assert status == 409 and body["detail"] == "この質問はもう締め切られています", (status, body)
        q3 = ask("tu-live-3")
        status, body = call("POST", f"/api/questions/{q3}/withdraw", json_body={"text": "表は要らない"}, cookie=cookie)
        assert status == 200 and body == {"ok": True}, (status, body)
        assert owner_state(q3) == ("withdrawn", "alice"), owner_state(q3)
        status, body = call("POST", f"/api/questions/{q3}/withdraw", json_body={"text": "x" * 8001}, cookie=cookie)
        assert status == 400, (status, body)
        status, body = call("POST", "/api/questions/no-such/withdraw", cookie=cookie)
        assert status == 404, (status, body)
        status, body = call("POST", f"/api/questions/{q3}/withdraw", headers=alice)
        assert status == 403 and body["detail"] == "内部専用の合言葉では使えません", (status, body)
    finally:
        runs.load_runs, control.run_send, control.run_interrupt = saved


def main() -> None:
    store.init_db()

    add_user("alice", "alice@uni.edu", "alicepass1")
    add_user("bob", "bob@uni.edu", "bobpass123")
    save_project_token(ALICE_TOKEN, "alice", "alice-alpha")
    save_project_token(ALICE_TOKEN_BETA, "alice", "alice-beta")
    save_project_token(BOB_TOKEN, "bob", "bob-beta")

    test_internal_question_owner()
    test_agent_questions()
    test_notify_owner()
    test_usage_key_from_secrets()
    test_old_db_without_owner()
    test_feedback_receiver()
    test_customer_days_and_list()
    test_send_and_interrupt_withdraw_questions()

    print("test_internal_auth: すべて通りました")


if __name__ == "__main__":
    main()
