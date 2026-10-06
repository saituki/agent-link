"""状態の監視ループ．2秒おきに herdr を見て，変わった所だけ通知する。

通知の対象は **sai-link が書き出した実行中の Run のペインだけ**
（herdr で手動起動したエージェントには鳴らさない・00 §5）。

通知の条件は2つだけ（増やすと無視されるようになるため増やさない）:
1. 判断が必要になった（working → blocked）
2. 長い作業が終わった（3分以上 working が続いた後に idle / done）

新しい仕組みは増やさない．herdr 呼び出しは 01 のものを再利用する。
"""

from __future__ import annotations

import asyncio
import logging
import time
from urllib.parse import quote

logger = logging.getLogger("sai_web")

# 「長い作業」の基準（秒）．変えたくなったらこの定数を書き換える．
_LONG_WORK_SEC = 3 * 60
# 監視の間隔（秒）
_POLL_SEC = 2.0

# pane ごとの前回状態と，working が続いている開始時刻
_prev_status: dict[str, str] = {}
_working_since: dict[str, float] = {}


def _title_of(run: dict) -> str:
    """通知に出す見出し（説明があればそれ，なければブロック名）．"""
    text = run.get("description") or run.get("block_name") or "実行"
    return f"{text} #{run.get('run_id', '')}"


def _run_url(run: dict) -> str:
    """通知タップで開く実行画面の住所（02 §3.1）．"""
    project = quote(str(run.get("project_name") or ""), safe="")
    return f"/#/run/{project}/{run.get('run_id', '')}"


def notify_heading(run: dict) -> str:
    """通知の見出し（＝プロジェクト名）．無いときだけ sai（計画書 A の U05・U17）．

    watch.py と approvals.py の両方から使うのでここに置く．
    """
    return str(run.get("project_name") or "").strip() or "sai"


def notify_status_change(run: dict, pane: dict, prev: str, now: float) -> None:
    """pane の状態が変わったときの通知を出す（見出しはプロジェクト名・計画書 A の U05）．

    監視ループから切り出したもの．テストから引数を確かめられる．
    """
    from sai_web import notify

    status = pane.get("agent_status") or ""
    title = _title_of(run)
    url = _run_url(run)
    heading = notify_heading(run)
    if prev == "working" and status == "blocked":
        notify.broadcast(heading, f"{title} が質問しています", url)
    elif prev == "working" and status in ("idle", "done"):
        # 「長い作業」の基準：3分以上 working が続いた後のみ
        if should_notify_done(pane.get("pane_id") or "", now):
            notify.broadcast(heading, f"{title} の作業が終わりました", url)


async def watch_loop() -> None:
    """サーバー起動中ずっと回る監視ループ．失敗しても止めない．"""
    from sai_web import herdr, runs

    while True:
        try:
            agents = herdr.list_agents()
            panes = herdr.list_panes()
            # sai-link の実行中の Run のみが対象（00 §5）
            running = [r for r in runs.load_runs() if r.get("status") == "running"]
            now = time.time()
            seen: set[str] = set()
            for run in running:
                pane = runs.pane_for_run(run, agents, panes)
                if pane is None or not pane.get("pane_id"):
                    # ペインが見つからない実行は対象外（00 §3.7）
                    continue
                pane_id = pane["pane_id"]
                seen.add(pane_id)
                status = pane.get("agent_status") or ""
                prev = _prev_status.get(pane_id)
                # Mac 側で操作中は通知しない（手元で見ているのに鳴るのは邪魔なため）
                focused = pane.get("focused") is True
                if prev is not None and prev != status and not focused:
                    notify_status_change(run, pane, prev, now)
                # working を入れた時刻を覚える／抜けたら消す
                if status == "working":
                    _working_since.setdefault(pane_id, now)
                else:
                    _working_since.pop(pane_id, None)
                _prev_status[pane_id] = status
            # 対象から外れた pane（実行終了・消えた）の記録を掃除する
            for pane_id in list(_prev_status):
                if pane_id not in seen:
                    _prev_status.pop(pane_id, None)
                    _working_since.pop(pane_id, None)
        except Exception as e:  # 監視の失敗でサーバーを落とさない
            logger.warning(f"監視ループでエラー: {e}")
        await asyncio.sleep(_POLL_SEC)


def should_notify_done(pane_id: str, now: float) -> bool:
    """3分以上 working が続いていたか．テスト用の小さな関数．"""
    started = _working_since.get(pane_id)
    if started is None:
        return False
    return now - started >= _LONG_WORK_SEC
