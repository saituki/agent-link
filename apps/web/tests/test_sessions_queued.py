"""実行中に送った文（cc の queued_command）を user の項目にする小さなテスト（assert だけ）。

実行:
    python3 tests/test_sessions_queued.py
    （または PYTHONPATH=src python3 -m pytest tests/test_sessions_queued.py）
"""

from __future__ import annotations

import sys
from pathlib import Path

# リポジトリ直下から実行しなくても import できるようにしておく
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_web import sessions  # noqa: E402


def _queued(prompt: str, mode: str = "prompt", uuid: str = "q-1") -> dict:
    """実物の cc 記録（待ち行列の文）と同じ形の行を作る。"""
    return {
        "type": "attachment",
        "isSidechain": False,
        "uuid": uuid,
        "timestamp": "2026-09-25T07:45:01.040Z",
        "attachment": {
            "type": "queued_command",
            "prompt": prompt,
            "commandMode": mode,
            "origin": {"kind": "human"},
        },
    }


def test_queued_prompt_becomes_user_item() -> None:
    """commandMode が prompt の queued_command は利用者の文の項目になる。"""
    items: list[dict] = []
    sessions._convert_claude_row(_queued("止めて"), items, {})
    assert len(items) == 1, items
    assert items[0]["kind"] == "user"
    assert items[0]["text"] == "止めて"
    assert items[0]["id"] == "q-1"
    assert items[0]["time"] == "2026-09-25T07:45:01.040Z"


def test_task_notification_is_dropped() -> None:
    """バックグラウンドの通知（task-notification）は利用者の文ではないので出さない。"""
    row = _queued("<task-notification>\n<status>completed</status>\n</task-notification>", mode="task-notification")
    items: list[dict] = []
    sessions._convert_claude_row(row, items, {})
    assert items == [], items


def test_other_attachments_and_sidechain_dropped() -> None:
    """ほかの attachment・subagent（isSidechain）の待ち行列の文は出さない。"""
    items: list[dict] = []
    other = _queued("x")
    other["attachment"]["type"] = "hook_success"
    sessions._convert_claude_row(other, items, {})
    side = _queued("y")
    side["isSidechain"] = True
    sessions._convert_claude_row(side, items, {})
    empty = _queued("   ")
    sessions._convert_claude_row(empty, items, {})
    assert items == [], items


def main() -> None:
    test_queued_prompt_becomes_user_item()
    test_task_notification_is_dropped()
    test_other_attachments_and_sidechain_dropped()
    print("test_sessions_queued: すべて通りました")


if __name__ == "__main__":
    main()
