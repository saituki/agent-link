"""質問の保管と回答．cc のフックと pi の拡張が登録し，iPhone が取り出して答える．

質問の中身は cc の AskUserQuestion と同じ書式
（質問文・見出し・選択肢・説明・複数選択可否・最大4問）．

方針（U03・共有契約 C6）:
- headless では pane の画面が無いので，回答は**記録するだけ**にする。
  cc のフック（U02）と pi の拡張が `GET /api/questions/{qid}` で取りに来る。
- 質問の状態は，答えの記録・拡張からの `resolved` の通知・期限切れでだけ変わる。
  画面を読んで「PC で回答済み」を判定することはしない（pane が無いため）。
- pi の open な質問は 10 分で期限切れ（05 §3.1: 拡張は最大5分待つ）。
  cc のフックは最長 24 時間待つので，cc の質問は 24 時間で期限切れにする。
"""

from __future__ import annotations

import logging
import threading

from sai_web import store

logger = logging.getLogger("sai_web")

# pi の open な質問を期限切れにするまでの秒数（05 §3.1: 拡張は最大5分待つ）
_PI_EXPIRE_SEC = 10 * 60
# cc の open な質問を期限切れにするまでの秒数（フックは最長 24 時間待つ・U02）
_CC_EXPIRE_SEC = 24 * 60 * 60

# 同じ質問への回答を二重に処理しないためのロック（質問 ID が鍵）。
# 処理中に来た 2 件目には「送信中です」と返す。
_answer_lock_guard = threading.Lock()
_answer_locks: dict[str, threading.Lock] = {}
# 使い終わったロックを溜め込まないための上限（回答済みの質問は二度と使わない）
_ANSWER_LOCK_MAX = 512


def _answer_lock(qid: str) -> threading.Lock:
    """質問 ID ごとのロックを返す（無ければ作る）．"""
    with _answer_lock_guard:
        lock = _answer_locks.get(qid)
        if lock is None:
            if len(_answer_locks) > _ANSWER_LOCK_MAX:
                # 終えた質問のロックが溜まっただけなのでまとめて捨てる
                # （今処理中のものは保持したうえで、超過分だけ消す）
                for old in list(_answer_locks)[: len(_answer_locks) - _ANSWER_LOCK_MAX]:
                    _answer_locks.pop(old, None)
            lock = threading.Lock()
            _answer_locks[qid] = lock
        return lock


def register_question(
    agent: str,
    session_id: str,
    pane_id: str,
    tool_use_id: str,
    questions: list[dict],
) -> dict:
    """フック・拡張からの質問登録．登録ついでにセッション対応も保存する．"""
    # どのセッションがどの pane かもここで覚える（01 の対応づけ用）
    q = store.save_question(agent, session_id, pane_id, tool_use_id, questions)
    _notify_if_run(session_id)
    return q


def _notify_if_run(session_id: str) -> None:
    """そのセッションが sai-link の headless 実行なら「質問しています」を通知する．

    手動で起動したエージェントには鳴らさない（00 §5）。通知の失敗で登録は止めない。
    """
    from sai_web import notify, runs

    try:
        for run in runs.load_runs():
            if session_id in runs.run_session_ids(run):
                notify.broadcast(
                    notify.notify_heading(run),
                    f"{notify.run_title(run)} が質問しています",
                    notify.run_url(run),
                )
                return
    except Exception:
        return


def refresh_question_state(session_id: str, pane_id: str | None = None, agent: str | None = None) -> None:
    """headless では画面を読まないので何もしない（U03）．

    質問の状態は，答えの記録・拡張からの `resolved` の通知・期限切れでだけ変わる。
    呼び出し元（`GET /api/questions`）の互換のために残す。
    """
    return


def expire_stale_questions() -> int:
    """期限切れの質問を expired にする．件数を返す．

    - pi は 10 分（`_PI_EXPIRE_SEC`・05 §3.1）。
    - cc は 24 時間（`_CC_EXPIRE_SEC`・フックの待ち時間に合わせる・U02）。
    """
    return store.expire_stale_questions("pi", _PI_EXPIRE_SEC) + store.expire_stale_questions(
        "claude", _CC_EXPIRE_SEC
    )


def resolve_on_terminal(qid: str) -> bool:
    """pi の拡張から「PC で答えた」の知らせが来た（05 §3.1）．

    その質問だけ「PC で回答済み」にする（既に回答済みなら変えない）．
    回答者は `terminal` になる（store.mark_resolved_elsewhere_qid）．
    """
    return store.mark_resolved_elsewhere_qid(qid)


def withdraw_question(qid: str, text: str) -> bool:
    """質問を取り下げる（送った文 `text` を残す・空もある）。open のときだけ変える。

    回答と同じロックを取る（回答の処理と同時に走らないように）。戻り値は変えられたか。
    """
    with _answer_lock(qid):
        return store.mark_withdrawn(qid, text)


def answer_question(
    qid: str, answers: list[dict], answered_by: str | None = None
) -> tuple[bool, str]:
    """質問に答える．(成功かどうか, 理由) を返す．

    answers は質問と同じ並びの配列（各要素は {choice_index} か
    {choice_indices}（複数選択）か {text}）．
    iPhone は全問まとめて1回で送ってくる．
    answered_by は答えた人の記録（None なら今の利用者・単独は user）．

    同じ質問へ同時に 2 つ処理しないよう，質問 ID ごとのロックを取る
    （2 件目には「送信中です」と返す・計画書 B U08）．
    """
    lock = _answer_lock(qid)
    if not lock.acquire(blocking=False):
        logger.warning("回答の送信中被り: qid=%s", qid)
        return (False, "送信中です")
    try:
        return _answer_question_locked(qid, answers, answered_by)
    finally:
        lock.release()


def _answer_question_locked(
    qid: str, answers: list[dict], answered_by: str | None = None
) -> tuple[bool, str]:
    """ロックを取った後の本体（answer_question から呼ばれる）．"""
    q = store.get_question(qid)
    if q is None:
        return (False, "質問が見つかりません")
    questions = q["questions"] or []
    if q["state"] != "open":
        return (False, "この質問は回答済みです")
    # 質問数に answers を合わせる（欠けている分は空扱い）
    answers = list(answers) + [{}] * max(0, len(questions) - len(answers))
    # 未選択の問が残っていたら記録しない（画面側で全問 answered になるまで
    # 送信ボタンを押せなくしているので，ここは保険）
    for i, one in enumerate(questions):
        a = answers[i] if i < len(answers) else {}
        if not a or (
            a.get("choice_index") is None
            and not a.get("choice_indices")
            and not (a.get("text") or "").strip()
        ):
            return (False, "未回答の質問があります")

    # headless ではキーを送れない．記録するだけ（cc のフックと pi の拡張が取りに来る）
    # 回答者は今の利用者（単独モードは user）．承認の記録（approval record）に使う（C8）
    who = answered_by if answered_by is not None else (store._current_owner() or "user")
    store.mark_answered(qid, {"answers": answers}, who)
    return (True, "")
