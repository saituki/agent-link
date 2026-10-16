"""プッシュ通知の送り方の小さなテスト（U03）。

確かめること:
- 鍵は Vapid として渡し，送り主の連絡先（sub）は @localhost ではない
- 無効な登録（404 / 410）だけ消し，一時的な失敗（403・500 など）では消さない
- 通知の見出しはプロジェクト名（無ければ sai・計画書 A の U05）
- headless の `state.json`（共有契約 C1）が 3 分以上 working のあと idle になると，
  通知が 1 回だけ出る（AC5）

実行:
    PYTHONPATH=src .venv/bin/python tests/test_notify.py
"""

from __future__ import annotations

import json
import tempfile
from pathlib import Path

import pywebpush
from py_vapid import Vapid

from agent_link_web import notify, runs, store, watch


class FakeResponse:
    def __init__(self, status: int) -> None:
        self.status_code = status
        self.text = "{}"


def main() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        notify._VAPID_FILE = Path(tmp) / "vapid.json"
        notify.ensure_keys()

        sent: list[dict] = []
        status = {"code": 201}

        def fake_webpush(**kwargs):
            sent.append(kwargs)
            if status["code"] >= 400:
                raise pywebpush.WebPushException("失敗", response=FakeResponse(status["code"]))
            return FakeResponse(status["code"])

        subs = {"e1": {"endpoint": "e1", "keys": {}}}
        deleted: list[str] = []
        original = (pywebpush.webpush, store.list_subscriptions, store.delete_subscription)
        pywebpush.webpush = fake_webpush
        store.list_subscriptions = lambda: [
            {"endpoint": k, "subscription_json": json.dumps(v)} for k, v in subs.items()
        ]
        store.delete_subscription = lambda endpoint: deleted.append(endpoint)
        try:
            # 届く: 鍵は Vapid・sub は形の正しい宛先
            assert notify.broadcast("t", "b", "/") == 1
            assert isinstance(sent[-1]["vapid_private_key"], Vapid)
            assert "@localhost" not in sent[-1]["vapid_claims"]["sub"]
            # 一時的な失敗では消さない
            for code in (403, 429, 500):
                status["code"] = code
                assert notify.broadcast("t", "b", "/") == 0
            assert deleted == [], deleted
            # 無効な登録だけ消す
            for code in (404, 410):
                status["code"] = code
                notify.broadcast("t", "b", "/")
            assert deleted == ["e1", "e1"], deleted
        finally:
            pywebpush.webpush, store.list_subscriptions, store.delete_subscription = original
    test_heading_is_project_name()
    test_long_work_notifies_once()
    print("test_notify: すべて通りました")


def test_heading_is_project_name() -> None:
    """通知の見出しはプロジェクト名（無ければ sai・計画書 A の U05・U17）。"""
    assert notify.notify_heading({"project_name": "agent-link-web"}) == "agent-link-web"
    assert notify.notify_heading({"project_name": "sai-base"}) == "sai-base"
    # project_name が無いときは agent-link（アプリの名前）
    assert notify.notify_heading({}) == "agent-link"
    assert notify.notify_heading({"project_name": ""}) == "agent-link"

    sent: list[tuple] = []
    original = notify.broadcast
    notify.broadcast = lambda title, body, url: sent.append((title, body, url)) or 0
    pane_id = "hl-0123456789ab"
    run = {
        "project_name": "sai-base",
        "run_id": 7,
        "description": "U01",
        "herdr_pane_id": pane_id,
    }
    try:
        # 長い作業が終わった（working が続いた後の idle）
        watch._working_since[pane_id] = 0.0
        watch.notify_status_change(run, "idle", "working", watch._LONG_WORK_SEC)
        assert sent[-1] == ("sai-base", "U01 #7 has finished.", "/#/run/sai-base/7")
        # 3 分未満では通知しない
        watch._working_since[pane_id] = 0.0
        watch.notify_status_change(run, "idle", "working", 10.0)
        assert len(sent) == 1, sent
        watch._working_since.pop(pane_id, None)
    finally:
        notify.broadcast = original


def test_long_work_notifies_once() -> None:
    """偽の `state.json` が 3 分以上 working のあと idle になると通知が 1 回出る（AC5）。"""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        pane_id = "hl-0123456789ab"
        headless = root / ".agent-link" / "engine" / "headless" / pane_id
        headless.mkdir(parents=True)
        state_path = headless / "state.json"
        run = {
            "project_name": "sai-base",
            "run_id": 7,
            "description": "U01",
            "herdr_pane_id": pane_id,
            "project_root": str(root),
        }

        def write_state(state: str) -> None:
            state_path.write_text(
                json.dumps(
                    {
                        "pane_id": pane_id,
                        "runner": "cc",
                        "state": state,
                        "turn": 1,
                        "partial": "",
                        "error": "",
                        "updated_at": "2026-10-07T00:00:00+09:00",
                    }
                ),
                encoding="utf-8",
            )

        sent: list[tuple] = []
        original_load = runs.load_runs
        original_broadcast = notify.broadcast
        runs.load_runs = lambda: [run]  # type: ignore[assignment]
        notify.broadcast = lambda title, body, url: sent.append((title, body, url)) or 0
        watch._prev_state.clear()
        watch._working_since.clear()
        try:
            # working → blocked（質問待ち）でも時間は続けて数える
            write_state("working")
            watch.watch_once(now=0.0)
            assert sent == [], sent
            write_state("blocked")
            watch.watch_once(now=watch._LONG_WORK_SEC / 2)
            assert watch._working_since[pane_id] == 0.0, watch._working_since
            assert sent == [], sent
            # 3 分後に idle になる → 通知が 1 回だけ
            write_state("idle")
            watch.watch_once(now=watch._LONG_WORK_SEC)
            assert sent == [("sai-base", "U01 #7 has finished.", "/#/run/sai-base/7")], sent
            # もう一度 idle のまま見回っても増えない
            watch.watch_once(now=watch._LONG_WORK_SEC + watch._POLL_SEC)
            assert len(sent) == 1, sent
        finally:
            runs.load_runs = original_load  # type: ignore[assignment]
            notify.broadcast = original_broadcast
            watch._prev_state.clear()
            watch._working_since.clear()


main()
