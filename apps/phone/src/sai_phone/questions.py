"""質問の保管と回答．cc のフックが登録し，iPhone が取り出して答える．

質問の中身は cc の AskUserQuestion と同じ書式
（質問文・見出し・選択肢・説明・複数選択可否・最大4問）．

方針（05 §3.2・§3.1）:
- 質問・回答は **PC と iPhone の両方に同時に出し，先に答えた方を採用する**．
  iPhone の回答待ちで PC の画面を止めない（フックで待たない）．
- cc への回答は，実機（cc 2.1.280）で観察したキー順で画面を操作する．
  **キー操作は推測で作らない**（§4 の確認作業で画面を読みながら確かめた）．
- pi の質問はキー操作をしない（pi の拡張がサーバーから答えを取りに来る）．
  ペインが blocked でなくても解決済みにしない（pi の質問は blocked にならない）．
"""

from __future__ import annotations

import logging
import re
import threading
import time

from sai_phone import herdr, store

logger = logging.getLogger("sai_phone")

# pi の open な質問を期限切れにするまでの秒数（05 §3.1: 拡張は最大5分待つ）
_PI_EXPIRE_SEC = 10 * 60

# 画面の何行まで読むか（質問・承認のウィジェットは画面下寄りに出る）
_SCREEN_LINES = 60

# 画面確認の待ち（キーを送って画面が切り替わるまで少し待つ）
_KEY_WAIT = 0.35
# 画面が想定どおりに変わるまで読み直す回数（0.35 秒 × 10 で 3.5 秒）。
# 質問が多いほど 1 問ごとの待ちが積もるため、1 問あたり 6 回（2.1 秒）では
# 端末が読み出しに時間がかかるとき「送信できませんでした」になりうる。
# 10 回まで待てるようにした（実機で「送信できませんでした」を確認した
# わけではないが、待ちだけを長くするのは他の動作を壊さない）。
_SCREEN_TRIES = 10

# 「画面に質問が見えていない」ことを何回続けて数えるか。
# 1 回では「回答を送った直後」や「再描画の途中」などを取り違えるので、
# 2 回続けて見えなかったときだけ「PC で回答済み」にする（計画書 B U08）。
_MISS_LIMIT = 2

# 質問ごとの「連続して見えなかった回数」（質問 ID が鍵）。
# 画面が質問に戻ってきたら消す。
_miss_lock = threading.Lock()
_misses: dict[str, int] = {}

# 同じ質問への回答を二重に処理しないためのロック（質問 ID が鍵）。
# 処理中に来た 2 件目には「送信中です」と返す。
_answer_lock_guard = threading.Lock()
_answer_locks: dict[str, threading.Lock] = {}
# 使い終わったロックを溜め込まないための上限（回答済みの質問は二度と使わない）
_ANSWER_LOCK_MAX = 512

# 回答ロックと同じ寿命で「送信中の質問」を持つ（ポーリングで「解決済み」にしないため）
_sending_lock = threading.Lock()
_sending: set[str] = set()


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


def _bump_miss(qid: str) -> int:
    """その質問の「連続して見えなかった回数」を 1 増やして返す．"""
    with _miss_lock:
        n = _misses.get(qid, 0) + 1
        _misses[qid] = n
        return n


def _forget_miss(qid: str) -> None:
    """画面が見えたので「見えなかった」を消す．"""
    with _miss_lock:
        _misses.pop(qid, None)


def _screen_tail(pane_id: str, lines: int = 20) -> str:
    """失敗したときの記録用に，画面の末尾を抜く（読めなければその旨）．"""
    if not pane_id:
        return "(pane なし)"
    screen = herdr.read_pane(pane_id, _SCREEN_LINES)
    if screen is None:
        return "(画面を読めなかった)"
    tail = screen.splitlines()[-lines:]
    return "\n".join(tail)


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
    return q


def refresh_question_state(session_id: str, pane_id: str | None, agent: str | None = None) -> None:
    """端末側で先に答えられていないか確かめ，解決済みならカードを消せるようにする．

    判定は原則「質問画面がまだ出ているか」を画面から直接読む
    （herdr の blocked 判定は質問画面中でも done になる場合が
    実機で発生したため）。画面が読めないときだけ blocked 判定に頼る。
    ただし **pi は対象外**（05 §3.1）．pi の質問は herdr が blocked にならず，
    登録直後に解決済みにしてしまう不具合が実際に起きていたため．


    実機（cc 2.1.283）で分かったこと（計画書 B U08）:
    - 「Review your answers」（2 問以上でのレビュー画面）には
      `Enter to select` が無い．iPhone からの回答処理の最後
      （_send_answers_cc の中）でこの画面が出るため，2 秒ごとの問い合わせが
      この瞬間を捉えると，一方通行の resolved_elsewhere に書き換えられて
      しまう．実際に 4 問の質問への回答送信の途中（12:20:03 の問い合わせ）で
      質問が resolved_elsewhere になるのを実機で確認した．
      1 問だけのときはレビュー画面が出ないため，この症状が出ない．
    - そのためレビュー画面は「まだ回答が送られていない」状態として扱い，
      解決済みにしない．
    - さらに，質問が見えないのが 1 回だけなら再描画の隙と決めつけず，
      質問ごとに数えて _MISS_LIMIT 回続けてから解決済みにする．
    """
    if not pane_id:
        return
    # ペインが消えた＝agent は終了した。答えの届け先が無いので片付ける（pi も同じ）
    if herdr.pane_gone(pane_id):
        store.expire_open_for_pane("questions", session_id, pane_id)
        return
    if agent == "pi":
        return
    screen = herdr.read_pane(pane_id, _SCREEN_LINES)
    if screen is not None:
        if _question_open(screen):
            _clear_misses(session_id)  # 質問画面はそのまま（先に PC で答えてはいない）
            return
        if _review_shown(screen):
            # レビュー画面は回答処理の途中。まだ誰も答えていない
            _clear_misses(session_id)
            return
        _resolve_after_misses(session_id, pane_id, screen)
        return
    state = herdr.get_agent(pane_id)
    if state is None:
        return
    if state.get("agent_status") != "blocked":
        _resolve_after_misses(session_id, pane_id, None)


def _clear_misses(session_id: str) -> None:
    """そのセッションの「見えなかった回数」を全部消す．"""
    with _miss_lock:
        for qid in [q["id"] for q in store.list_open_questions(session_id)]:
            _misses.pop(qid, None)


def _resolve_after_misses(session_id: str, pane_id: str, screen: str | None) -> None:
    """質問が見えないのが _MISS_LIMIT 回続いたら「PC で回答済み」にする．

    1 回では再描画の隙・レビュー画面と取り違えるので，数えるだけにする．
    2 回続いたら記録を logger.warning に出してから解決済みにする．
    """
    for q in store.list_open_questions(session_id):
        qid = q["id"]
        with _sending_lock:
            if qid in _sending:
                # いま送信中の質問は「PC で回答済み」にしない
                # （レビュー画面のあとで画面が閉じるので，瞬間にこうなる）
                continue
        n = _bump_miss(qid)
        if n < _MISS_LIMIT:
            logger.info(
                "質問が見えない（%d 回目）：まだ open のままにする qid=%s 質問数=%d",
                n,
                qid,
                len(q.get("questions") or []),
            )
            continue
        logger.warning(
            "質問が画面から %d 回続けて見えなかったので「PC で回答済み」にする: "
            "qid=%s 質問数=%d 画面末尾:\n%s",
            n,
            qid,
            len(q.get("questions") or []),
            screen if screen is not None else _screen_tail(pane_id),
        )
        store.mark_resolved_elsewhere_qid(qid)
        _forget_miss(qid)


def expire_stale_questions() -> int:
    """期限切れの pi の質問を expired にする（05 §3.1）．件数を返す．"""
    return store.expire_stale_questions("pi", _PI_EXPIRE_SEC)


def resolve_on_terminal(qid: str) -> bool:
    """pi の拡張から「PC で答えた」の知らせが来た（05 §3.1）．

    その質問だけ「PC で回答済み」にする（既に回答済みなら変えない）．
    """
    return store.mark_resolved_elsewhere_qid(qid)


def answer_question(qid: str, answers: list[dict]) -> tuple[bool, str]:
    """質問に答える．(成功かどうか, 理由) を返す．

    answers は質問と同じ並びの配列（各要素は {choice_index} か
    {choice_indices}（複数選択）か {text}）．
    iPhone は全問まとめて1回で送ってくる．

    同じ質問へ同時に 2 つ処理しないよう，質問 ID ごとのロックを取る
    （2 件目には「送信中です」と返す・計画書 B U08）．失敗したときは
    qid・理由・質問数・画面の末尾を logger.warning に残す．
    """
    lock = _answer_lock(qid)
    if not lock.acquire(blocking=False):
        logger.warning("回答の送信中被り: qid=%s", qid)
        return (False, "送信中です")
    with _sending_lock:
        _sending.add(qid)
    try:
        return _answer_question_locked(qid, answers)
    finally:
        with _sending_lock:
            _sending.discard(qid)
        lock.release()


def _fail(qid: str, reason: str, questions: list[dict], pane_id: str) -> tuple[bool, str]:
    """失敗の理由をログに残してから返す（画面末尾つき）．"""
    logger.warning(
        "質問に答えられない: qid=%s 理由=%s 質問数=%d 画面末尾:\n%s",
        qid,
        reason,
        len(questions or []),
        _screen_tail(pane_id),
    )
    return (False, reason)


def _answer_question_locked(qid: str, answers: list[dict]) -> tuple[bool, str]:
    """ロックを取った後の本体（answer_question から呼ばれる）．"""
    q = store.get_question(qid)
    if q is None:
        return (False, "質問が見つかりません")
    questions = q["questions"] or []
    if q["state"] != "open":
        return _fail(qid, "この質問は回答済みです", questions, q.get("pane_id") or "")
    # 質問数に answers を合わせる（欠けている分は空扱い）
    answers = list(answers) + [{}] * max(0, len(questions) - len(answers))
    # 未選択の問が残っていたら送らない（画面側で全問 answered になるまで
    # 送信ボタンを押せなくしているので，ここは保険）
    for i, one in enumerate(questions):
        a = answers[i] if i < len(answers) else {}
        if not a or (
            a.get("choice_index") is None
            and not a.get("choice_indices")
            and not (a.get("text") or "").strip()
        ):
            return _fail(qid, "未回答の質問があります", questions, q.get("pane_id") or "")

    # pi: ヘルダーでキーを打ち込まない．保存するだけ（05 §3.1）
    # 拡張（ask_user_phone.ts）が1秒おきに取りに来る．
    if q.get("agent") == "pi":
        store.mark_answered(qid, {"answers": answers})
        return (True, "")

    # cc: まだ選択画面が出ているかを画面から確かめる
    # （先に PC で答えられていたら負け。blocked 判定は当てにならないため画面を見る）
    pane_id = q.get("pane_id") or ""
    if not pane_id:
        return (False, "端末に繋がりませんでした")
    # 解決済みの書き換え（resolved_elsewhere 化）は行わない
    # （それはポーリング側 refresh_question_state の仕事。回答の失敗を
    #  直後にカードが消えてしまわせないため）。
    screen = _read_screen_retry(pane_id)
    if screen is not None:
        # レビュー画面（Review your answers）は前の送信が最後まで進んでいる
        # 途中．「端末で回答済み」とは言わない（この場合はロックで弾かれる）．
        if not _question_open(screen) and not _review_shown(screen):
            return _fail(qid, "端末側で回答済みです", questions, pane_id)
    else:
        # 画面が読めないときは blocked 判定に頼る（従来どおり）
        agent = herdr.get_agent(pane_id)
        if agent is None:
            return (False, "端末に繋がりませんでした")
        if agent.get("agent_status") != "blocked":
            return _fail(qid, "端末側で回答済みです", questions, pane_id)

    ok, reason = _send_answers_cc(pane_id, questions, answers, qid=qid)
    if not ok:
        return (False, reason)
    store.mark_answered(qid, {"answers": answers})
    return (True, "")


def _read_screen_retry(pane_id: str) -> str | None:
    """画面を読む．1 回失敗（読めない）なら 0.5 秒待って読み直す．

    一瞬 herdr が読めないことがあるため，1 回で「端末で回答済み」と
    決めつけない（計画書 B U08）．
    """
    screen = herdr.read_pane(pane_id, _SCREEN_LINES)
    if screen is None:
        time.sleep(0.5)
        screen = herdr.read_pane(pane_id, _SCREEN_LINES)
    return screen


# ---------------------------------------------------------------------------
# cc の質問画面の操作（05 §3.2・実機で観察した手順）
# ---------------------------------------------------------------------------
#
# 観察した画面（cc 2.1.280 / AskUserQuestion）:
#
#   ←  ☒ 色  ☐ 技  ✔ Submit  →
#
#   使う技は？
#
#     1. [ ] 探索
#     2. [ ] 集中
#     …
#     5. [ ] Type something
#        Submit
#
#   Enter to select · Tab/Arrow keys to navigate · Esc to cancel
#
# ここから分かったキー（推測せず，送るごとに画面を読んで確かめた）:
# - 単一選択: カーソルは最初の選択肢。down で移動し **Enter で決定**．
#   決定すると次の質問へ進む（これが最後の質問なら「Review your answers」）．
# - 複数選択: Enter は選択のオン/オフ（その場に残る）．全部選んだら
#   リスト内の `Submit` まで down で移動して Enter → 次の質問 / レビューへ．
# - 自由入力: 「Type something.」の行は入力欄そのもの．そこまで移動してから
#   **文字を打ち込む**（`pane send-text`）．Enter で確定．
#   空のまま Enter を押すと「回答を拒否した」扱いになる（観察済み）ため注意．
# - 2問以上のときだけ最後に「Review your answers」が出て，カーソルは
#   「1. Submit answers」．Enter で送信．1問だけのときは即送信される．
# - 送信後は画面が閉じる（入力欄に戻る）．
#
# 各キーの後で画面を読み，期待どおり（次の質問 / レビュー / 画面が閉じた）かを
# 確かめる．違えば打ち切って「送信できませんでした。PC で答えてください」を返す
# （途中まで間違った答えを入れ続けない・05 §3.2）．


def _norm(text: str) -> str:
    """画面の折り返しで文が切れないように，空白と枠の縦線を全部落として比べる用．

    質問文は枠の中に出て，折り返した行の頭に縦線 `│` が付く。
    縦線を残すと長い質問文が画面と一致せず「送信できませんでした」になる。
    """
    return re.sub(r"[\s│┃|]+", "", text or "")


def _read_screen(pane_id: str) -> str:
    return herdr.read_pane(pane_id, _SCREEN_LINES) or ""


def _wait_screen(pane_id: str, predicate) -> str | None:
    """画面が predicate を満たすまで少し待って読み直す．満たせなければ None．"""
    for _ in range(_SCREEN_TRIES):
        screen = _read_screen(pane_id)
        if predicate(screen):
            return screen
        time.sleep(_KEY_WAIT)
    return None


def _review_shown(screen: str) -> bool:
    """レビュー画面（Review your answers）が出ているか．"""
    return "Reviewyouranswers" in _norm(screen)


def _question_open(screen: str) -> bool:
    """まだ質問画面（選択肢リスト）が出ているか．"""
    return "Entertoselect" in _norm(screen)


def _expected_after(screen: str, questions: list[dict], next_index: int) -> bool:
    """質問を1つ処理した直後の画面が期待どおりか．"""
    if _review_shown(screen):
        return True
    if next_index < len(questions):
        # 次の質問の本文（タブの見出しではなく，中央に出る質問文）が出ているか．
        # 見出し（header）はタブに常に並ぶので判定に使わない．
        one = questions[next_index]
        qtext = _norm(one.get("question") or "")
        if qtext and qtext in _norm(screen):
            return True
        return False
    # 最後の質問を処理した後: レビューか，画面が閉じた（1問のとき）かのどちらか
    return not _question_open(screen)


def _move_to(pane_id: str, target: int, current: int) -> tuple[bool, int]:
    """リスト内を target 番目まで移動する（0 が先頭）．"""
    steps = target - current
    if steps < 0:
        # 上移動は使わない（常に先頭から下へ進むため），安全のため戻せない
        return (False, current)
    for _ in range(steps):
        if not herdr.send_keys(pane_id, "down"):
            return (False, current)
        time.sleep(_KEY_WAIT)
    return (True, target)


def _answer_single(pane_id: str, options: list[dict], index: int) -> tuple[bool, int]:
    """単一選択の1問を答える．(成功, 移動後の位置) を返す．"""
    if index < 0 or index >= len(options):
        return (False, 0)
    ok, pos = _move_to(pane_id, index, 0)
    if not ok:
        return (False, pos)
    if not herdr.send_keys(pane_id, "enter"):
        return (False, pos)
    time.sleep(_KEY_WAIT)
    return (True, pos)


def _answer_multi(
    pane_id: str, options: list[dict], indices: list[int]
) -> tuple[bool, int]:
    """複数選択の1問を答える．最後にリスト内の Submit まで移動して Enter する．"""
    wanted = sorted({int(i) for i in indices})
    if not wanted or wanted[0] < 0 or wanted[-1] >= len(options):
        return (False, 0)
    pos = 0
    for target in wanted:
        ok, pos = _move_to(pane_id, target, pos)
        if not ok:
            return (False, pos)
        if not herdr.send_keys(pane_id, "enter"):  # 選択のオン/オフ
            return (False, pos)
        time.sleep(_KEY_WAIT)
    # 選択肢 → Type something → Submit の順に並ぶ（観察した並び）
    ok, pos = _move_to(pane_id, len(options) + 1, pos)
    if not ok:
        return (False, pos)
    if not herdr.send_keys(pane_id, "enter"):
        return (False, pos)
    time.sleep(_KEY_WAIT)
    return (True, pos)


def _answer_text(pane_id: str, options: list[dict], text: str) -> tuple[bool, int]:
    """自由入力の1問を答える（Type something の行に文字を入れて Enter）．"""
    target = len(options)  # 選択肢の次が Type something の行
    ok, pos = _move_to(pane_id, target, 0)
    if not ok:
        return (False, pos)
    # その行が入力欄になっていることを確認してから打ち込む
    screen = _wait_screen(
        pane_id, lambda s: "Typesomething" in _norm(s)
    )
    if screen is None:
        return (False, pos)
    if not herdr.send_text(pane_id, text):
        return (False, pos)
    # 打ち込んだ文字が画面に出たか確かめる（折り返し対策で空白は落として比べる）
    if _wait_screen(pane_id, lambda s: _norm(text) in _norm(s)) is None:
        return (False, pos)
    if not herdr.send_keys(pane_id, "enter"):
        return (False, pos)
    time.sleep(_KEY_WAIT)
    return (True, pos)


def _send_answers_cc(
    pane_id: str,
    questions: list[dict],
    answers: list[dict],
    qid: str = "",
) -> tuple[bool, str]:
    """cc の質問画面に全問分の回答を入れて送る（観察したキー順・05 §3.2）．

    途中で期待どおりに進まなかったときは，どの問で止まったかを
    logger.warning に残す（画面の末尾つき・計画書 B U08）．
    """
    for i, one in enumerate(questions):
        answer = answers[i] if i < len(answers) else {}
        options = one.get("options") or []
        text = (answer.get("text") or "").strip()
        indices = answer.get("choice_indices")
        if text:
            ok, _ = _answer_text(pane_id, options, text)
        elif indices:
            ok, _ = _answer_multi(pane_id, options, list(indices))
        elif answer.get("choice_index") is not None:
            ok, _ = _answer_single(pane_id, options, int(answer["choice_index"]))
        else:
            return _fail(qid, "未回答の質問があります", questions, pane_id)
        if not ok:
            return _fail(qid, f"第{i + 1}問のキーを送れなかった", questions, pane_id)
        # 次の質問 / レビュー / 送信完了 のどこへ進んだかを画面で確かめる
        screen = _wait_screen(pane_id, lambda s, idx=i + 1: _expected_after(s, questions, idx))
        if screen is None:
            return _fail(
                qid,
                f"第{i + 1}問のあとが画面に出なかった。"
                "（送信できませんでした。PC で答えてください）",
                questions,
                pane_id,
            )

    # レビュー画面（2問以上のとき）で確定する
    screen = _wait_screen(pane_id, lambda s: _review_shown(s) or not _question_open(s))
    if screen is None:
        return _fail(
            qid, "レビュー画面が待った時間内に出なかった", questions, pane_id
        )
    if _review_shown(screen):
        if not herdr.send_keys(pane_id, "enter"):
            return _fail(qid, "レビューの Enter が送れなかった", questions, pane_id)
        # まだレビューが残っていたら送信できていない（間違った画面なら止めない）
        closed = _wait_screen(pane_id, lambda s: not _review_shown(s))
        if closed is None:
            return _fail(
                qid,
                "送信できませんでした。PC で答えてください（レビューが消えなかった）",
                questions,
                pane_id,
            )
    return (True, "")
