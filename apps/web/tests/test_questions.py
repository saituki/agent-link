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


def main() -> None:
    test_pi_question_stays_open_when_pane_not_blocked()
    test_cc_question_answer_records_without_keys()
    test_answer_requires_all_questions()
    test_pi_question_expires_after_10_minutes()
    test_cc_question_expires_after_24_hours()
    test_resolve_on_terminal()
    print("test_questions: すべて通りました")


if __name__ == "__main__":
    main()
