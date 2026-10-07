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
    updated_at REAL NOT NULL
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
    answer_json TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS subscriptions (
    endpoint TEXT PRIMARY KEY,
    subscription_json TEXT NOT NULL,
    created_at REAL NOT NULL
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
"""


def _connect() -> sqlite3.Connection:
    conn = sqlite3.connect(DB_PATH)
    return conn


def init_db() -> None:
    with _connect() as conn:
        conn.executescript(_SCHEMA)


def save_registration(
    agent: str, session_id: str, transcript_path: str, cwd: str, pane_id: str
) -> None:
    """エージェントからの登録を保存する（同じセッションは上書き）．"""
    with _connect() as conn:
        conn.execute(
            "INSERT OR REPLACE INTO registrations "
            "(session_id, agent, transcript_path, cwd, pane_id, updated_at) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            (session_id, agent, transcript_path, cwd, pane_id, time.time()),
        )


def get_registration(session_id: str) -> dict | None:
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            "SELECT * FROM registrations WHERE session_id = ?", (session_id,)
        ).fetchone()
    return dict(row) if row else None


def list_registrations() -> list[dict]:
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute("SELECT * FROM registrations").fetchall()
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
            "(id, agent, session_id, pane_id, tool_use_id, questions_json, created_at, state, answer_json) "
            "VALUES (?, ?, ?, ?, ?, ?, ?, 'open', '')",
            (
                qid,
                agent,
                session_id,
                pane_id,
                tool_use_id,
                json.dumps(questions, ensure_ascii=False),
                time.time(),
            ),
        )
        row = conn.execute("SELECT * FROM questions WHERE id = ?", (qid,)).fetchone()
        return _row_to_question(row)


def get_question(qid: str) -> dict | None:
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute("SELECT * FROM questions WHERE id = ?", (qid,)).fetchone()
    return _row_to_question(row) if row else None


def list_open_questions(session_id: str | None = None) -> list[dict]:
    """未解決の質問を新しい順に返す．session_id があれば絞る．"""
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        if session_id:
            rows = conn.execute(
                "SELECT * FROM questions WHERE state = 'open' AND session_id = ? "
                "ORDER BY created_at DESC",
                (session_id,),
            ).fetchall()
        else:
            rows = conn.execute(
                "SELECT * FROM questions WHERE state = 'open' ORDER BY created_at DESC"
            ).fetchall()
    return [_row_to_question(r) for r in rows]


def mark_answered(qid: str, answer: dict) -> None:
    """回答済みにする．"""
    with _connect() as conn:
        conn.execute(
            "UPDATE questions SET state = 'answered', answer_json = ? WHERE id = ?",
            (json.dumps(answer, ensure_ascii=False), qid),
        )


def list_questions_for_session(session_id: str) -> list[dict]:
    """そのセッションの質問を，画面に出す3つの状態だけで新しい順に返す．

    - open: 未回答（カードを操作できる）
    - answered: iPhone で回答済み（「回答済み: …」の1行）
    - resolved_elsewhere: PC で先に回答済み（「PC で回答済み」の1行）
    expired は表示しない（待っていた時間が過ぎた）。
    """
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute(
            "SELECT * FROM questions WHERE session_id = ? "
            "AND state IN ('open', 'answered', 'resolved_elsewhere') "
            "ORDER BY created_at DESC",
            (session_id,),
        ).fetchall()
    return [_row_to_question(r) for r in rows]


def mark_resolved_elsewhere_qid(qid: str) -> bool:
    """1つの質問だけ「PC で解決済み」にする（拡張からの知らせ用）．

    open のときだけ書き換える（既に回答済みを上書きしない）。
    戻り値は書き換えできたか。
    """
    with _connect() as conn:
        cur = conn.execute(
            "UPDATE questions SET state = 'resolved_elsewhere' "
            "WHERE id = ? AND state = 'open'",
            (qid,),
        )
        return cur.rowcount > 0


def expire_stale_questions(agent: str, older_than_sec: float, now: float | None = None) -> int:
    """指定エージェントの open な質問のうち，古すぎるものを expired にする．

    pi の拡張は最大5分しか取りに来ない（05 §3.1）ため，10分経ったら切る。
    件数を返す。
    """
    base = time.time() if now is None else now
    with _connect() as conn:
        cur = conn.execute(
            "UPDATE questions SET state = 'expired' "
            "WHERE agent = ? AND state = 'open' AND created_at < ?",
            (agent, base - older_than_sec),
        )
        return cur.rowcount


def mark_resolved_elsewhere(session_id: str) -> int:
    """そのセッションの未解決質問を「端末側で解決済み」にする．件数を返す．"""
    with _connect() as conn:
        cur = conn.execute(
            "UPDATE questions SET state = 'resolved_elsewhere' "
            "WHERE session_id = ? AND state = 'open'",
            (session_id,),
        )
        return cur.rowcount


def save_subscription(endpoint: str, subscription: dict) -> None:
    """iPhone の通知登録を保存する（同じ端末は上書き）．"""
    import json as _json
    import time as _time

    with _connect() as conn:
        conn.execute(
            "INSERT OR REPLACE INTO subscriptions "
            "(endpoint, subscription_json, created_at) VALUES (?, ?, ?)",
            (endpoint, _json.dumps(subscription, ensure_ascii=False), _time.time()),
        )


def list_subscriptions() -> list[dict]:
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute("SELECT * FROM subscriptions").fetchall()
    return [dict(r) for r in rows]


def delete_subscription(endpoint: str) -> None:
    """届かなくなった登録を消す．"""
    with _connect() as conn:
        conn.execute("DELETE FROM subscriptions WHERE endpoint = ?", (endpoint,))
