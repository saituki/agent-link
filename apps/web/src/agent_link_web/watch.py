"""状態の監視ループ．5秒おきに headless の実行の `state.json` を見て通知する（U03）。

通知の対象は **agent-link が書き出した headless の実行だけ**
（`herdr_pane_id` が `hl-` で始まる物・共有契約 C1）。

通知の条件は2つだけ（増やすと無視されるようになるため増やさない）:
1. 判断が必要になった（質問の登録）— 質問を登録したときに `questions.py` が出す
2. 長い作業が終わった（3分以上 working / blocked が続いた後に idle）

新しい仕組みは増やさない．実行の状態は `runs.headless_state`（C1 の `state.json`）から読む。
"""

from __future__ import annotations

import asyncio
import logging
import time

from agent_link_web import notify

logger = logging.getLogger("agent_link_web")

# 「長い作業」の基準（秒）．変えたくなったらこの定数を書き換える．
_LONG_WORK_SEC = 3 * 60
# 監視の間隔（秒）
_POLL_SEC = 5.0

# 擬似 pane ごとの前回状態と，working が続いている開始時刻
_prev_state: dict[str, str] = {}
_working_since: dict[str, float] = {}


def notify_status_change(run: dict, state: str, prev: str, now: float) -> None:
    """状態が変わったときの通知を出す（見出しはプロジェクト名・計画書 A の U05）．

    監視ループから切り出したもの．テストから引数を確かめられる．
    """
    if prev in ("working", "blocked") and state == "idle":
        # 「長い作業」の基準：3分以上 working（質問待ちの blocked を含む）が続いた後のみ
        if should_notify_done(run.get("herdr_pane_id") or "", now):
            notify.broadcast(
                notify.notify_heading(run),
                f"{notify.run_title(run)} has finished.",
                notify.run_url(run),
            )


def watch_once(now: float | None = None) -> None:
    """1回分の見回り（テストから呼べるように切り出す）。"""
    from agent_link_web import runs

    now = time.time() if now is None else now
    seen: set[str] = set()
    for run in runs.load_runs():
        st = runs.headless_state(run)
        if st is None:
            continue
        pane_id = run.get("herdr_pane_id") or ""
        if not pane_id:
            continue
        seen.add(pane_id)
        state = st["state"]
        prev = _prev_state.get(pane_id)
        if prev is not None and prev != state:
            notify_status_change(run, state, prev, now)
        # working / blocked を入れた時刻を覚える／抜けたら消す（質問待ちも作業の時間に数える）
        if state in ("working", "blocked"):
            _working_since.setdefault(pane_id, now)
        else:
            _working_since.pop(pane_id, None)
        _prev_state[pane_id] = state
    # 対象から外れた pane（実行終了・消えた）の記録を掃除する
    for pane_id in list(_prev_state):
        if pane_id not in seen:
            _prev_state.pop(pane_id, None)
            _working_since.pop(pane_id, None)


async def watch_loop() -> None:
    """サーバー起動中ずっと回る監視ループ．失敗しても止めない．"""
    while True:
        try:
            watch_once()
        except Exception as e:  # 監視の失敗でサーバーを落とさない
            logger.warning(f"監視ループでエラー: {e}")
        await asyncio.sleep(_POLL_SEC)


def should_notify_done(pane_id: str, now: float) -> bool:
    """3分以上 working が続いていたか．テスト用の小さな関数．"""
    started = _working_since.get(pane_id)
    if started is None:
        return False
    return now - started >= _LONG_WORK_SEC
