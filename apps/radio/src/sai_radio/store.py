"""論文・音声・ログを保存する場所．SQLite を1ファイル使うだけ．

保存する情報は3つだけ:
- papers: 論文ごとの情報と，対応表(manifest)そのもの(JSON文字列)
- events: 「いつ・どの論文を聞いたか／質問したか」のログ
"""

from __future__ import annotations

import json
import sqlite3
import time
from dataclasses import asdict, dataclass

from sai_radio.config import DB_PATH

_SCHEMA = """
CREATE TABLE IF NOT EXISTS papers (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    mode TEXT NOT NULL,           -- 'narration' か 'podcast'
    audio_path TEXT NOT NULL,     -- data/audio/ からの相対パス，または論文フォルダの絶対パス
    manifest_json TEXT NOT NULL,  -- セクション対応表 (JSON)
    created_at REAL NOT NULL,
    favorite INTEGER NOT NULL DEFAULT 0  -- お気に入り(1=星付き)
);

CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    paper_id TEXT NOT NULL,
    kind TEXT NOT NULL,           -- 'listen' か 'question'
    payload_json TEXT NOT NULL,
    created_at REAL NOT NULL
);
"""


@dataclass
class ManifestSection:
    start: float
    end: float | None
    title: str
    source_text: str


@dataclass
class Paper:
    id: str
    title: str
    mode: str
    audio_path: str
    sections: list[ManifestSection]
    created_at: float
    # お気に入りかどうか．一覧で星マークを付けるために使う．
    favorite: bool = False


def _connect() -> sqlite3.Connection:
    conn = sqlite3.connect(DB_PATH)
    conn.execute("PRAGMA foreign_keys = ON")
    return conn


def init_db() -> None:
    with _connect() as conn:
        conn.executescript(_SCHEMA)
        # 古いDBには favorite 列がないので，あれば追加する．
        # すでにある場合は何もしない(エラーを無視する)．
        try:
            conn.execute("ALTER TABLE papers ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0")
        except sqlite3.OperationalError:
            pass


def save_paper(paper_id: str, title: str, mode: str, audio_path: str, sections: list[ManifestSection]) -> Paper:
    # 同じIDで登録し直しても，星(お気に入り)は消さない．
    old = get_paper(paper_id)
    paper = Paper(
        id=paper_id,
        title=title,
        mode=mode,
        audio_path=audio_path,
        sections=sections,
        created_at=time.time(),
        favorite=old.favorite if old else False,
    )
    manifest_json = json.dumps([asdict(s) for s in sections], ensure_ascii=False)
    with _connect() as conn:
        conn.execute(
            "INSERT OR REPLACE INTO papers (id, title, mode, audio_path, manifest_json, created_at, favorite) "
            "VALUES (?, ?, ?, ?, ?, ?, ?)",
            (paper.id, paper.title, paper.mode, paper.audio_path, manifest_json, paper.created_at, 1 if paper.favorite else 0),
        )
    return paper


def _row_to_paper(row: sqlite3.Row) -> Paper:
    sections = [ManifestSection(**s) for s in json.loads(row["manifest_json"])]
    # keys() で列があるか確かめる．古いDBでは favorite 列がないことがあるため．
    favorite = bool(row["favorite"]) if "favorite" in row.keys() else False
    return Paper(
        id=row["id"],
        title=row["title"],
        mode=row["mode"],
        audio_path=row["audio_path"],
        sections=sections,
        created_at=row["created_at"],
        favorite=favorite,
    )


def set_favorite(paper_id: str, favorite: bool) -> None:
    """お気に入り(星)のON/OFFを切り替える．"""
    with _connect() as conn:
        conn.execute(
            "UPDATE papers SET favorite = ? WHERE id = ?",
            (1 if favorite else 0, paper_id),
        )


def get_paper(paper_id: str) -> Paper | None:
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute("SELECT * FROM papers WHERE id = ?", (paper_id,)).fetchone()
    return _row_to_paper(row) if row else None


def update_audio_path(paper_id: str, audio_path: str) -> None:
    """論文の音声ファイルの場所を更新する．

    音声を圧縮してファイル名が変わったとき(m4a化など)に使う．
    音声の中身や対応表は変わらないので，パスだけ書き換える．
    """
    with _connect() as conn:
        conn.execute(
            "UPDATE papers SET audio_path = ? WHERE id = ?",
            (audio_path, paper_id),
        )


def list_papers() -> list[Paper]:
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute("SELECT * FROM papers ORDER BY created_at DESC").fetchall()
    return [_row_to_paper(r) for r in rows]


def find_section(paper_id: str, position_sec: float) -> ManifestSection | None:
    """再生位置(秒)から対応するセクションを逆引きする．

    end が None のセクションは「そこから最後まで」を意味する．
    """
    paper = get_paper(paper_id)
    if paper is None or not paper.sections:
        return None
    for section in paper.sections:
        if position_sec >= section.start and (section.end is None or position_sec < section.end):
            return section
    return paper.sections[-1]


def add_event(paper_id: str, kind: str, payload: dict) -> None:
    with _connect() as conn:
        conn.execute(
            "INSERT INTO events (paper_id, kind, payload_json, created_at) VALUES (?, ?, ?, ?)",
            (paper_id, kind, json.dumps(payload, ensure_ascii=False), time.time()),
        )


def list_events(paper_id: str | None = None, limit: int = 200) -> list[dict]:
    with _connect() as conn:
        conn.row_factory = sqlite3.Row
        if paper_id:
            rows = conn.execute(
                "SELECT * FROM events WHERE paper_id = ? ORDER BY created_at DESC LIMIT ?",
                (paper_id, limit),
            ).fetchall()
        else:
            rows = conn.execute(
                "SELECT * FROM events ORDER BY created_at DESC LIMIT ?", (limit,)
            ).fetchall()
    return [
        {
            "id": r["id"],
            "paper_id": r["paper_id"],
            "kind": r["kind"],
            "payload": json.loads(r["payload_json"]),
            "created_at": r["created_at"],
        }
        for r in rows
    ]


def stats_by_day(days: int = 84) -> list[dict]:
    """直近days日分の再生回数を，日付ごとに数えて返す．

    聞いていない日も0件で返すので，カレンダーにそのまま並べられる．
    日付は "2026-09-22" の形の文字列．
    """
    from datetime import date, timedelta

    today = date.today()
    start = today - timedelta(days=days - 1)
    counts: dict[str, int] = {}
    with _connect() as conn:
        # created_at はUNIX秒なので，date(..., 'unixepoch') で日付に直す．
        rows = conn.execute(
            "SELECT date(created_at, 'unixepoch', 'localtime') AS day, COUNT(*) AS count "
            "FROM events WHERE kind = 'listen' AND date(created_at, 'unixepoch', 'localtime') >= ? "
            "GROUP BY day",
            (start.isoformat(),),
        ).fetchall()
    for day, count in rows:
        counts[day] = count
    result = []
    for i in range(days):
        day = (start + timedelta(days=i)).isoformat()
        result.append({"date": day, "count": counts.get(day, 0)})
    return result
