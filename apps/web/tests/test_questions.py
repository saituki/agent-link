"""質問の状態遷移の小さなテスト（assertだけ）・05 §6・U03．

headless では pane の画面が無いので，回答は記録するだけになった（共有契約 C6）。
確かめること:
- pi の質問は答えるまで open のまま（画面を読まない）
- cc の質問に答えると，キーを送らずに answered になり答えが保存される（AC4）
- 未回答の問が残っている間は記録しない
- pi は 10 分・cc は 24 時間で期限切れになる
- 拡張からの「PC で答えた」の通知は resolved_elsewhere にする

実行:
    PYTHONPATH=src .venv/bin/python tests/test_questions.py
    （または PYTHONPATH=src python3 -m pytest tests/test_questions.py）
"""

from __future__ import annotations

import sqlite3
import sys
import tempfile
import time
from pathlib import Path

# リポジトリ直下から実行しなくても import できるようにしておく
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_web import questions, store  # noqa: E402


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
    """pi の質問は答えるまで open のまま（画面を読まない・05 §6）．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("pi")["id"]
        # 画面を見ないので，何度 refresh しても open のまま
        questions.refresh_question_state("sess-1", "pane-1", "pi")
        assert state_of(qid) == "open"

        # pi への回答はキーを打ち込まず，保存するだけ
        ok, _reason = questions.answer_question(qid, [{"choice_index": 1}])
        assert ok is True
        assert state_of(qid) == "answered"
        q = store.get_question(qid)
        assert q["answer"]["answers"] == [{"choice_index": 1}]


def test_cc_question_answer_records_without_keys() -> None:
    """cc の質問もキーを送らず，記録するだけ（AC4・共有契約 C6）．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("claude")["id"]
        ok, reason = questions.answer_question(qid, [{"choice_index": 0}])
        assert ok is True, reason
        assert state_of(qid) == "answered"
        q = store.get_question(qid)
        assert q["answer"]["answers"] == [{"choice_index": 0}]
        # 回答済みに再度答えることはできない
        ok, reason = questions.answer_question(qid, [{"choice_index": 1}])
        assert ok is False
        assert "回答済み" in reason


def test_answer_requires_all_questions() -> None:
    """未回答の問が残っている間は記録しない（05 §1 の2番の保険）．"""
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


def test_cc_question_expires_after_24_hours() -> None:
    """cc の open な質問は 24 時間で expired（フックの待ち時間に合わせる・U02）．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("claude")["id"]
        # 登録時刻を 23 時間前に遅らせてもまだ expired にならない
        with store._connect() as conn:  # noqa: SLF001
            conn.execute(
                "UPDATE questions SET created_at = ? WHERE id = ?",
                (time.time() - 23 * 60 * 60, qid),
            )
        assert questions.expire_stale_questions() == 0
        assert state_of(qid) == "open"
        # 25 時間前なら expired
        with store._connect() as conn:  # noqa: SLF001
            conn.execute(
                "UPDATE questions SET created_at = ? WHERE id = ?",
                (time.time() - 25 * 60 * 60, qid),
            )
        assert questions.expire_stale_questions() == 1
        assert state_of(qid) == "expired"


def test_resolve_on_terminal() -> None:
    """拡張からの「PC で答えた」は resolved_elsewhere にする（05 §3.1）．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("pi")["id"]
        assert questions.resolve_on_terminal(qid) is True
        assert state_of(qid) == "resolved_elsewhere"
        # 2回目は変更しない（回答済みを上書きしないため）
        assert questions.resolve_on_terminal(qid) is False


def test_answer_records_answered_by() -> None:
    """回答者を記録する（C8）: 単独モードは user，端末で答えたのは terminal．"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("claude")["id"]
        ok, reason = questions.answer_question(qid, [{"choice_index": 0}])
        assert ok is True, reason
        # store.get_question の dict をそのまま返すので，GET /api/questions/{qid} にも出る
        assert store.get_question(qid)["answered_by"] == "user"

        # 端末で答えた（PC で回答済み）は terminal
        qid_pi = make_question("pi")["id"]
        assert questions.resolve_on_terminal(qid_pi) is True
        assert store.get_question(qid_pi)["answered_by"] == "terminal"


def test_old_db_gets_answered_by_column() -> None:
    """`answered_by` の列が無い古い DB に足され，既存の行は空文字になる（DB を壊さない）。"""
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "old.db"
        conn = sqlite3.connect(path)
        conn.executescript(
            "CREATE TABLE questions (id TEXT PRIMARY KEY, agent TEXT NOT NULL, "
            "session_id TEXT NOT NULL, pane_id TEXT NOT NULL, tool_use_id TEXT NOT NULL, "
            "questions_json TEXT NOT NULL, created_at REAL NOT NULL, "
            "state TEXT NOT NULL DEFAULT 'open', answer_json TEXT NOT NULL DEFAULT '', "
            "owner TEXT NOT NULL DEFAULT '');"
            "INSERT INTO questions (id, agent, session_id, pane_id, tool_use_id, "
            "questions_json, created_at, state, answer_json) "
            "VALUES ('old1', 'claude', 's', 'p', 'tu-old', '[]', 0, 'answered', '{\"answers\": []}');"
        )
        conn.commit()
        conn.close()
        store.DB_PATH = path
        store.init_db()
        q = store.get_question("old1")
        assert q is not None
        assert q["answered_by"] == "", q
        assert q["state"] == "answered", q
        # 列を足したあとも，2 回目の init_db は何もせず通る
        store.init_db()


def test_free_text_answer_is_recorded() -> None:
    """自由記述（Other）の答えは text のまま保存され、回答済みになる（AC3）。"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("claude")["id"]
        ok, reason = questions.answer_question(qid, [{"text": "緑"}])
        assert (ok, reason) == (True, ""), reason
        assert state_of(qid) == "answered"
        assert store.get_question(qid)["answer"]["answers"] == [{"text": "緑"}]


def test_withdraw_changes_only_open() -> None:
    """取り下げは open の行だけを変え，送った文を答えに残す。画面にも出る（U02）。"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        qid = make_question("claude")["id"]
        assert store.mark_withdrawn(qid, "次は表を作って") is True
        assert state_of(qid) == "withdrawn"
        q = store.get_question(qid)
        assert q["answer"] == {"text": "次は表を作って"}, q["answer"]
        assert q["answered_by"] == "user", q["answered_by"]
        # 締め切られた質問は変えない（2 回目・回答済み）
        assert store.mark_withdrawn(qid, "別の文") is False
        assert store.get_question(qid)["answer"] == {"text": "次は表を作って"}
        ok, reason = questions.answer_question(qid, [{"choice_index": 0}])
        assert ok is False and "回答済み" in reason, reason
        # 画面に出す状態に withdrawn が入る（open の一覧からは外れる）
        assert [q["state"] for q in store.list_questions_for_session("sess-1")] == ["withdrawn"]
        assert store.list_open_questions() == []

        # 文なしの取り下げ（interrupt と質問カードの「質問を止める」）
        qid2 = make_question("claude", n=2)["id"]
        assert questions.withdraw_question(qid2, "") is True
        assert store.get_question(qid2)["answer"] == {"text": ""}
        assert questions.withdraw_question(qid2, "") is False

        # 回答済みの質問は取り下げできない（上書きしない）
        qid3 = make_question("pi")["id"]
        assert questions.resolve_on_terminal(qid3) is True
        assert store.mark_withdrawn(qid3, "x") is False
        assert state_of(qid3) == "resolved_elsewhere"


def main() -> None:
    test_withdraw_changes_only_open()
    test_pi_question_stays_open_when_pane_not_blocked()
    test_cc_question_answer_records_without_keys()
    test_answer_requires_all_questions()
    test_pi_question_expires_after_10_minutes()
    test_cc_question_expires_after_24_hours()
    test_resolve_on_terminal()
    test_answer_records_answered_by()
    test_old_db_gets_answered_by_column()
    test_free_text_answer_is_recorded()
    print("test_questions: すべて通りました")


if __name__ == "__main__":
    main()
