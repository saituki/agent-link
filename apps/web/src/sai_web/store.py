"""pane とセッションの対応表・質問の保管場所．SQLite を1ファイル使うだけ．

保存するのは2つ:
- registrations: エージェント自身が送ってきた登録
  （どのセッションがどの pane で動いているか）
- questions: cc のフック等から受け取った質問と回答
  （電波が切れている間の質問も残し続け，復帰時にまとめて届ける）
"""

from __future__ import annotations

import json
import sqlite3
import time
import uuid

from sai_web.config import DB_PATH

_SCHEMA = """
CREATE TABLE IF NOT EXISTS registrations (
    session_id TEXT PRIMARY KEY,
    agent TEXT NOT NULL,
    transcript_path TEXT NOT NULL,
    cwd TEXT NOT NULL,
    pane_id TEXT NOT NULL,
    updated_at REAL NOT NULL,
    owner TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS questions (
    id TEXT PRIMARY KEY,
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    pane_id TEXT NOT NULL,
    tool_use_id TEXT NOT NULL,
    questions_json TEXT NOT NULL,
    created_at REAL NOT NULL,
    state TEXT NOT NULL DEFAULT 'open',
    answer_json TEXT NOT NULL DEFAULT '',
    owner TEXT NOT NULL DEFAULT '',
    answered_by TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS subscriptions (
    endpoint TEXT PRIMARY KEY,
    subscription_json TEXT NOT NULL,
    created_at REAL NOT NULL,
    owner TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS approvals (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    pane_id TEXT NOT NULL,
    tool_use_id TEXT NOT NULL DEFAULT '',
    tool_name TEXT NOT NULL,
    tool_input_json TEXT NOT NULL DEFAULT '',
    suggestions_json TEXT NOT NULL DEFAULT '',
    match_text TEXT NOT NULL DEFAULT '',
    created_at REAL NOT NULL,
    state TEXT NOT NULL DEFAULT 'open',
    answer_json TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS users (
    name TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    is_admin INTEGER NOT NULL DEFAULT 0,
    must_change_password INTEGER NOT NULL DEFAULT 1,
    disabled INTEGER NOT NULL DEFAULT 0,
    created_at REAL NOT NULL
);
CREATE TABLE IF NOT EXISTS web_sessions (
    token_sha256 TEXT PRIMARY KEY,
    user_name TEXT NOT NULL,
    created_at REAL NOT NULL,
    expires_at REAL NOT NULL
);
CREATE TABLE IF NOT EXISTS api_tokens (
    token_sha256 TEXT PRIMARY KEY,
    user_name TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at REAL NOT NULL,
    last_used_at REAL
);
CREATE TABLE IF NOT EXISTS project_tokens (
    token_sha256 TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    project TEXT NOT NULL,
    created_at REAL NOT NULL
);
"""


def _connect() -> sqlite3.Connection:
    conn = sqlite3.connect(DB_PATH)
    return conn


def _current_owner() -> str:
    """保存する行の持ち主。多人数モードで今の利用者がいればその名前、いなければ空文字。"""
    from sai_web import registry

    if not registry.multi_user():
        return ""
    user = registry.current_user.get()
    if isinstance(user, dict):
        return str(user.get("name") or "")
    return ""


def _owner_filter() -> str | None:
    """読むときに絞る持ち主。単独モードは None（絞らない）。

    多人数モードでは今の利用者の名前（未ログインなら空文字＝持ち主の無い行だけ）。
    絞り込みを store の中で行うのは、読む場所（Home・Chats・質問・通知）が多いため。
    """
    from sai_web import registry

    if not registry.multi_user():
        return None
    user = registry.current_user.get()
    if isinstance(user, dict):
        return str(user.get("name") or "")
    return ""


def _ensure_owner_column(conn: sqlite3.Connection, table: str) -> None:
    """古い DB に `owner` の列が無ければ足す（既存の行は空文字・DB を壊さない）。"""
    columns = {row[1] for row in conn.execute(f"PRAGMA table_info({table})")}
    if "owner" not in columns:
        conn.execute(f"ALTER TABLE {table} ADD COLUMN owner TEXT NOT NULL DEFAULT ''")


def _ensure_answered_by_column(conn: sqlite3.Connection) -> None:
    """古い DB に `answered_by` の列が無ければ足す（既存の行は空文字）。"""
    columns = {row[1] for row in conn.execute("PRAGMA table_info(questions)")}
    if "answered_by" not in columns:
        conn.execute("ALTER TABLE questions ADD COLUMN answered_by TEXT NOT NULL DEFAULT ''")


def init_db() -> None:
    with _connect() as conn:
        conn.executescript(_SCHEMA)
        # 既存の DB（`owner` の列が無い）をそのまま使えるようにする
        for table in ("registrations", "questions", "subscriptions"):
            _ensure_owner_column(conn, table)
        _ensure_answered_by_column(conn)


def save_registration(
    agent: str, session_id: str, transcript_path: str, cwd: str, pane_id: str
) -> None:
    """エージェントからの登録を保存する（同じセッションは上書き）．"""
    with _connect() as conn:
        conn.execute(
            "INSERT OR REPLACE INTO registrations "
            "(session_id, agent, transcript_path, cwd, pane_id, updated_at, owner) "
            "VALUES (?, ?, ?, ?, ?, ?, ?)",
            (session_id, agent, transcript_path, cwd, pane_id, time.time(), _current_owner()),
        )


def get_registration(session_id: str) -> dict | None:
    owner = _owner_filter()
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        if owner is None:
            row = conn.execute(
                "SELECT * FROM registrations WHERE session_id = ?", (session_id,)
            ).fetchone()
        else:
            row = conn.execute(
                "SELECT * FROM registrations WHERE session_id = ? AND owner = ?",
                (session_id, owner),
            ).fetchone()
    return dict(row) if row else None


def list_registrations() -> list[dict]:
    owner = _owner_filter()
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        if owner is None:
            rows = conn.execute("SELECT * FROM registrations").fetchall()
        else:
            rows = conn.execute(
                "SELECT * FROM registrations WHERE owner = ?", (owner,)
            ).fetchall()
    return [dict(r) for r in rows]


def _row_to_question(row: sqlite3.Row) -> dict:
    d = dict(row)
    try:
        d["questions"] = json.loads(d.pop("questions_json"))
    except (json.JSONDecodeError, KeyError):
        d["questions"] = []
    try:
        d["answer"] = json.loads(d["answer_json"]) if d.get("answer_json") else None
    except json.JSONDecodeError:
        d["answer"] = None
    d.pop("answer_json", None)
    return d


def save_question(
    agent: str,
    session_id: str,
    pane_id: str,
    tool_use_id: str,
    questions: list[dict],
) -> dict:
    """質問を保管する．同じ tool_use_id があれば上書きしない（重複登録の防止）．"""
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            "SELECT * FROM questions WHERE tool_use_id = ?", (tool_use_id,)
        ).fetchone()
        if row is not None:
            return _row_to_question(row)
        qid = uuid.uuid4().hex[:12]
        conn.execute(
            "INSERT INTO questions "
            "(id, agent, session_id, pane_id, tool_use_id, questions_json, created_at, state, answer_json, owner) "
            "VALUES (?, ?, ?, ?, ?, ?, ?, 'open', '', ?)",
            (
                qid,
                agent,
                session_id,
                pane_id,
                tool_use_id,
                json.dumps(questions, ensure_ascii=False),
                time.time(),
                _current_owner(),
            ),
        )
        row = conn.execute("SELECT * FROM questions WHERE id = ?", (qid,)).fetchone()
        return _row_to_question(row)


def get_question(qid: str) -> dict | None:
    owner = _owner_filter()
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        if owner is None:
            row = conn.execute("SELECT * FROM questions WHERE id = ?", (qid,)).fetchone()
        else:
            row = conn.execute(
                "SELECT * FROM questions WHERE id = ? AND owner = ?", (qid, owner)
            ).fetchone()
    return _row_to_question(row) if row else None


def list_open_questions(session_id: str | None = None) -> list[dict]:
    """未解決の質問を新しい順に返す．session_id があれば絞る．

    多人数モードでは今の利用者の質問だけを返す（`_owner_filter`）。
    """
    owner = _owner_filter()
    clauses = ["state = 'open'"]
    values: list = []
    if session_id:
        clauses.append("session_id = ?")
        values.append(session_id)
    if owner is not None:
        clauses.append("owner = ?")
        values.append(owner)
    sql = (
        "SELECT * FROM questions WHERE "
        + " AND ".join(clauses)
        + " ORDER BY created_at DESC"
    )
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute(sql, values).fetchall()
    return [_row_to_question(r) for r in rows]


def mark_answered(qid: str, answer: dict, answered_by: str) -> None:
    """回答済みにする．回答した人の名前（端末なら `terminal`）も残す．"""
    owner = _owner_filter()
    with _connect() as conn:
        if owner is None:
            conn.execute(
                "UPDATE questions SET state = 'answered', answer_json = ?, answered_by = ? "
                "WHERE id = ?",
                (json.dumps(answer, ensure_ascii=False), answered_by, qid),
            )
        else:
            conn.execute(
                "UPDATE questions SET state = 'answered', answer_json = ?, answered_by = ? "
                "WHERE id = ? AND owner = ?",
                (json.dumps(answer, ensure_ascii=False), answered_by, qid, owner),
            )


def list_questions_for_session(session_id: str) -> list[dict]:
    """そのセッションの質問を，画面に出す3つの状態だけで新しい順に返す．

    - open: 未回答（カードを操作できる）
    - answered: iPhone で回答済み（「回答済み: …」の1行）
    - resolved_elsewhere: PC で先に回答済み（「PC で回答済み」の1行）
    expired は表示しない（待っていた時間が過ぎた）。
    多人数モードでは今の利用者の質問だけを返す。
    """
    owner = _owner_filter()
    clauses = [
        "session_id = ?",
        "state IN ('open', 'answered', 'resolved_elsewhere')",
    ]
    values: list = [session_id]
    if owner is not None:
        clauses.append("owner = ?")
        values.append(owner)
    sql = (
        "SELECT * FROM questions WHERE "
        + " AND ".join(clauses)
        + " ORDER BY created_at DESC"
    )
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute(sql, values).fetchall()
    return [_row_to_question(r) for r in rows]


def mark_resolved_elsewhere_qid(qid: str) -> bool:
    """1つの質問だけ「PC で解決済み」にする（拡張からの知らせ用）．

    open のときだけ書き換える（既に回答済みを上書きしない）。
    戻り値は書き換えできたか。
    """
    owner = _owner_filter()
    with _connect() as conn:
        if owner is None:
            cur = conn.execute(
                "UPDATE questions SET state = 'resolved_elsewhere', answered_by = 'terminal' "
                "WHERE id = ? AND state = 'open'",
                (qid,),
            )
        else:
            cur = conn.execute(
                "UPDATE questions SET state = 'resolved_elsewhere', answered_by = 'terminal' "
                "WHERE id = ? AND state = 'open' AND owner = ?",
                (qid, owner),
            )
        return cur.rowcount > 0


def expire_stale_questions(agent: str, older_than_sec: float, now: float | None = None) -> int:
    """指定エージェントの open な質問のうち，古すぎるものを expired にする．

    pi の拡張は最大5分しか取りに来ない（05 §3.1）ため，10分経ったら切る。
    件数を返す。多人数モードでは今の利用者の質問だけを切る。
    """
    base = time.time() if now is None else now
    owner = _owner_filter()
    with _connect() as conn:
        if owner is None:
            cur = conn.execute(
                "UPDATE questions SET state = 'expired' "
                "WHERE agent = ? AND state = 'open' AND created_at < ?",
                (agent, base - older_than_sec),
            )
        else:
            cur = conn.execute(
                "UPDATE questions SET state = 'expired' "
                "WHERE agent = ? AND state = 'open' AND created_at < ? AND owner = ?",
                (agent, base - older_than_sec, owner),
            )
        return cur.rowcount


def mark_resolved_elsewhere(session_id: str) -> int:
    """そのセッションの未解決質問を「端末側で解決済み」にする．件数を返す．"""
    owner = _owner_filter()
    with _connect() as conn:
        if owner is None:
            cur = conn.execute(
                "UPDATE questions SET state = 'resolved_elsewhere' "
                "WHERE session_id = ? AND state = 'open'",
                (session_id,),
            )
        else:
            cur = conn.execute(
                "UPDATE questions SET state = 'resolved_elsewhere' "
                "WHERE session_id = ? AND state = 'open' AND owner = ?",
                (session_id, owner),
            )
        return cur.rowcount


def save_subscription(endpoint: str, subscription: dict) -> None:
    """iPhone の通知登録を保存する（同じ端末は上書き）．"""
    import json as _json
    import time as _time

    with _connect() as conn:
        conn.execute(
            "INSERT OR REPLACE INTO subscriptions "
            "(endpoint, subscription_json, created_at, owner) VALUES (?, ?, ?, ?)",
            (
                endpoint,
                _json.dumps(subscription, ensure_ascii=False),
                _time.time(),
                _current_owner(),
            ),
        )


def list_subscriptions() -> list[dict]:
    """通知の購読。多人数モードでは今の利用者の購読だけを返す。"""
    owner = _owner_filter()
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        if owner is None:
            rows = conn.execute("SELECT * FROM subscriptions").fetchall()
        else:
            rows = conn.execute(
                "SELECT * FROM subscriptions WHERE owner = ?", (owner,)
            ).fetchall()
    return [dict(r) for r in rows]


def delete_subscription(endpoint: str) -> None:
    """届かなくなった登録を消す．"""
    owner = _owner_filter()
    with _connect() as conn:
        if owner is None:
            conn.execute("DELETE FROM subscriptions WHERE endpoint = ?", (endpoint,))
        else:
            conn.execute(
                "DELETE FROM subscriptions WHERE endpoint = ? AND owner = ?",
                (endpoint, owner),
            )


# --- アカウント・ログイン・個人のトークン（U06） ---
# 値そのもの（パスワード・Cookie・トークン）は保存しない。
# パスワードは `auth.hash_password` の scrypt 文字列、Cookie とトークンは
# `auth.sha256_hex` の SHA-256 だけを置く。


def _row_to_user(row: sqlite3.Row | None) -> dict | None:
    """users の 1 行を画面に渡す形にする（0/1 は真偽にする）。"""
    if row is None:
        return None
    d = dict(row)
    d["is_admin"] = bool(d.get("is_admin"))
    d["must_change_password"] = bool(d.get("must_change_password"))
    d["disabled"] = bool(d.get("disabled"))
    return d


def create_user(
    name: str,
    email: str,
    password_hash: str,
    is_admin: bool = False,
    must_change_password: bool = True,
    disabled: bool = False,
) -> dict:
    """利用者を作る（email は小文字にして保存する）。"""
    with _connect() as conn:
        conn.execute(
            "INSERT INTO users "
            "(name, email, password_hash, is_admin, must_change_password, disabled, created_at) "
            "VALUES (?, ?, ?, ?, ?, ?, ?)",
            (
                name,
                email.lower(),
                password_hash,
                1 if is_admin else 0,
                1 if must_change_password else 0,
                1 if disabled else 0,
                time.time(),
            ),
        )
    return get_user(name) or {}


def get_user(name: str) -> dict | None:
    """名前で利用者を引く。"""
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute("SELECT * FROM users WHERE name = ?", (name,)).fetchone()
    return _row_to_user(row)


def get_user_by_email(email: str) -> dict | None:
    """メールアドレスで利用者を引く（小文字にして照合）。"""
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            "SELECT * FROM users WHERE email = ?", (email.lower(),)
        ).fetchone()
    return _row_to_user(row)


def list_users() -> list[dict]:
    """利用者の一覧（名前の順）。"""
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute("SELECT * FROM users ORDER BY name").fetchall()
    return [u for u in (_row_to_user(r) for r in rows) if u is not None]


def update_user(
    name: str,
    *,
    email: str | None = None,
    is_admin: bool | None = None,
    disabled: bool | None = None,
    password_hash: str | None = None,
    must_change_password: bool | None = None,
) -> bool:
    """渡された項目だけを書き換える。書き換えられたら真。"""
    sets: list[str] = []
    values: list = []
    if email is not None:
        sets.append("email = ?")
        values.append(email.lower())
    if is_admin is not None:
        sets.append("is_admin = ?")
        values.append(1 if is_admin else 0)
    if disabled is not None:
        sets.append("disabled = ?")
        values.append(1 if disabled else 0)
    if password_hash is not None:
        sets.append("password_hash = ?")
        values.append(password_hash)
    if must_change_password is not None:
        sets.append("must_change_password = ?")
        values.append(1 if must_change_password else 0)
    if not sets:
        return False
    values.append(name)
    with _connect() as conn:
        cur = conn.execute(f"UPDATE users SET {', '.join(sets)} WHERE name = ?", values)
        return cur.rowcount > 0


def create_session(token_sha256: str, user_name: str, expires_at: float) -> None:
    """ログインのセッションを保存する（Cookie の値そのものは置かない）。"""
    with _connect() as conn:
        conn.execute(
            "INSERT OR REPLACE INTO web_sessions "
            "(token_sha256, user_name, created_at, expires_at) VALUES (?, ?, ?, ?)",
            (token_sha256, user_name, time.time(), expires_at),
        )


def get_session_user(token_sha256: str) -> dict | None:
    """セッションの利用者を引く。期限切れは消して None を返す。"""
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            "SELECT * FROM web_sessions WHERE token_sha256 = ?", (token_sha256,)
        ).fetchone()
        if row is None:
            return None
        if float(row["expires_at"]) < time.time():
            conn.execute(
                "DELETE FROM web_sessions WHERE token_sha256 = ?", (token_sha256,)
            )
            return None
    return get_user(str(row["user_name"]))


def delete_session(token_sha256: str) -> None:
    """セッションを 1 つ消す（ログアウト）。"""
    with _connect() as conn:
        conn.execute(
            "DELETE FROM web_sessions WHERE token_sha256 = ?", (token_sha256,)
        )


def delete_sessions_of(user_name: str) -> None:
    """その人のセッションを全部消す（パスワード変更・無効化）。"""
    with _connect() as conn:
        conn.execute("DELETE FROM web_sessions WHERE user_name = ?", (user_name,))


def create_api_token(token_sha256: str, user_name: str, label: str) -> None:
    """個人のトークンを保存する（値そのものは置かない）。"""
    with _connect() as conn:
        conn.execute(
            "INSERT INTO api_tokens "
            "(token_sha256, user_name, label, created_at, last_used_at) "
            "VALUES (?, ?, ?, ?, NULL)",
            (token_sha256, user_name, label, time.time()),
        )


def find_api_token(token_sha256: str) -> dict | None:
    """個人のトークンを引く（見つけたら `last_used_at` を今にする）。"""
    now = time.time()
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            "SELECT * FROM api_tokens WHERE token_sha256 = ?", (token_sha256,)
        ).fetchone()
        if row is None:
            return None
        conn.execute(
            "UPDATE api_tokens SET last_used_at = ? WHERE token_sha256 = ?",
            (now, token_sha256),
        )
    d = dict(row)
    d["last_used_at"] = now
    return d


def list_api_tokens(user_name: str) -> list[dict]:
    """その人の個人のトークンの一覧（新しい順）。"""
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute(
            "SELECT * FROM api_tokens WHERE user_name = ? ORDER BY created_at DESC",
            (user_name,),
        ).fetchall()
    return [dict(r) for r in rows]


def delete_api_token(token_id: str, user_name: str) -> bool:
    """自分の個人のトークンを消す。`token_id` は SHA-256 の先頭 12 文字（画面の id）。

    他人のトークンは消せない（`user_name` で絞る）。消せたら真。
    """
    with _connect() as conn:
        cur = conn.execute(
            "DELETE FROM api_tokens "
            "WHERE user_name = ? AND substr(token_sha256, 1, 12) = ?",
            (user_name, token_id),
        )
        return cur.rowcount > 0


# --- プロジェクトの内部の合言葉（U09・U10 が使う） ---
# 値そのものは保存しない（`auth.sha256_hex` の SHA-256 だけ）。
# 同じプロジェクトを作り直すと古い行は消す（コンテナごとに 1 つだけ有効）。


def save_project_token(token_sha256: str, owner: str, project: str) -> None:
    """プロジェクトの合言葉を保存する（同じ project の古い行は消す）。"""
    with _connect() as conn:
        conn.execute("DELETE FROM project_tokens WHERE project = ?", (project,))
        conn.execute(
            "INSERT INTO project_tokens "
            "(token_sha256, owner, project, created_at) VALUES (?, ?, ?, ?)",
            (token_sha256, owner, project, time.time()),
        )


def find_project_token(token_sha256: str) -> dict | None:
    """プロジェクトの合言葉を引く（無ければ None）。"""
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            "SELECT * FROM project_tokens WHERE token_sha256 = ?", (token_sha256,)
        ).fetchone()
    return dict(row) if row else None
