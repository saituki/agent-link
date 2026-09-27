"""プッシュ通知の送り方の小さなテスト。

確かめること:
- 鍵は Vapid として渡し，送り主の連絡先（sub）は @localhost ではない
- 無効な登録（404 / 410）だけ消し，一時的な失敗（403・500 など）では消さない
- 通知の見出しはプロジェクト名（無ければ sai-phone・計画書 A の U05）

実行:
    PYTHONPATH=src .venv/bin/python tests/test_notify.py
"""

from __future__ import annotations

import json
import tempfile
from pathlib import Path

import pywebpush
from py_vapid import Vapid

from sai_phone import approvals, notify, runs, store, watch


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
    print("test_notify: すべて通りました")


def test_heading_is_project_name() -> None:
    """通知の見出しはプロジェクト名（無ければ sai-phone・計画書 A の U05）。"""
    assert watch.notify_heading({"project_name": "sai-phone"}) == "sai-phone"
    assert watch.notify_heading({"project_name": "sai-base"}) == "sai-base"
    # project_name が無いときは sai-phone（従来と同じ見出し）
    assert watch.notify_heading({}) == "sai-phone"
    assert watch.notify_heading({"project_name": ""}) == "sai-phone"

    sent: list[tuple] = []
    original = notify.broadcast
    notify.broadcast = lambda title, body, url: sent.append((title, body, url)) or 0
    try:
        # 質問（working → blocked）
        watch._working_since.pop("p1", None)
        watch.notify_status_change(
            {"project_name": "sai-base", "run_id": 7, "description": "U01"},
            {"pane_id": "p1", "agent_status": "blocked"},
            "working",
            0.0,
        )
        assert sent == [("sai-base", "U01 #7 が質問しています", "/#/run/sai-base/7")], sent

        # 長い作業が終わった（working が続いた後の idle）
        watch._working_since["p1"] = 0.0
        watch.notify_status_change(
            {"project_name": "sai-base", "run_id": 7, "description": "U01"},
            {"pane_id": "p1", "agent_status": "idle"},
            "working",
            watch._LONG_WORK_SEC,
        )
        assert sent[-1] == ("sai-base", "U01 #7 の作業が終わりました", "/#/run/sai-base/7")
        watch._working_since.pop("p1", None)

        # 承認（approvals も同じ見出しを使う）
        original_load = runs.load_runs
        original_ids = runs.run_session_ids
        runs.load_runs = lambda: [
            {"project_name": "sai-base", "run_id": 7, "agent_session_id": "s1"}
        ]
        runs.run_session_ids = lambda run: {"s1"}
        try:
            approvals._notify_if_run("s1", "Bash")
        finally:
            runs.load_runs, runs.run_session_ids = original_load, original_ids
        assert sent[-1] == ("sai-base", "承認を求めています: Bash", "/#/run/sai-base/7"), sent
    finally:
        notify.broadcast = original


main()
