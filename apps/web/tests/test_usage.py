"""usage.py（Claude Code の枠と費用）の小さなテスト（assert だけ）．

外部 API は呼ばない．cc の記録は一時フォルダに作った偽物を使う．
`GET /api/usage` の形は test_internal_auth.py で確かめる．

実行:
    PYTHONPATH=src .venv/bin/python tests/test_usage.py
"""

from __future__ import annotations

import calendar
import json
import os
import sys
import tempfile
import time
from pathlib import Path

# リポジトリ直下から実行しなくても import できるようにしておく
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from agent_link_web import usage  # noqa: E402


def iso(epoch: float) -> str:
    """秒 → cc の `updated_at` と同じ UTC の形。"""
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(epoch))


def write_json(path: Path, body) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(body), encoding="utf-8")


# 2026-10-15 03:00 UTC（どの時差でも 10/15 の 12 時前後・9 月と 10 月を分けられる）
CC_NOW = float(calendar.timegm((2026, 10, 15, 3, 0, 0)))


def write_cc_home(
    home: Path, *, five_pct: float, cost_files: dict[str, dict], age: float = 3600
) -> None:
    """cc の記録（枠の値と費用）を `<home>`（= `<sai_home>`）に置く。枠の値は `age` 秒前に更新したことにする。"""
    write_json(
        home / "cc-rate-limits.json",
        {
            "five_hour": {"used_percentage": five_pct, "resets_at": CC_NOW + 3600},
            # 戻る時刻を過ぎた 7 日枠は落ちる
            "seven_day": {"used_percentage": 41.2, "resets_at": CC_NOW - 10},
            "session_id": "s",
            "updated_at": iso(CC_NOW - age),
        },
    )
    for name, body in cost_files.items():
        write_json(home / "cc-cost" / name, body)


def test_cc_card_single() -> None:
    """単独モード: `AGENT_LINK_HOME` の枠の値と費用から cc_card() が形どおりに返る。"""
    saved = os.environ.get("AGENT_LINK_HOME")
    with tempfile.TemporaryDirectory() as tmp:
        home = Path(tmp)
        try:
            os.environ["AGENT_LINK_HOME"] = str(home)
            write_cc_home(
                home,
                five_pct=23.5,
                cost_files={
                    "a.json": {"session_id": "a", "total_cost_usd": 1.23, "updated_at": iso(CC_NOW - 600)},
                    "b.json": {"session_id": "b", "total_cost_usd": 44.37, "updated_at": iso(CC_NOW - 5 * 86400)},
                    # 先月の会話は今月に入らない
                    "c.json": {"session_id": "c", "total_cost_usd": 9.0, "updated_at": iso(CC_NOW - 16 * 86400)},
                    # 読めないファイルは飛ばす
                    "d.json": None,
                },
            )
            (home / "cc-cost" / "e.json").write_text("{壊れている", encoding="utf-8")
            card = usage.cc_card(now=CC_NOW)
        finally:
            if saved is None:
                os.environ.pop("AGENT_LINK_HOME", None)
            else:
                os.environ["AGENT_LINK_HOME"] = saved
    assert card["limits"] == {
        "five_hour": {"used_percentage": 23.5, "resets_at": CC_NOW + 3600},
        "updated_at": iso(CC_NOW - 3600),
    }, card
    assert abs(card["cost"]["today"] - 1.23) < 1e-9, card
    assert abs(card["cost"]["month"] - 45.6) < 1e-9, card
    assert card["cost"]["sessions"] == 2, card


def test_cc_card_homes() -> None:
    """`homes` を渡すと、その `<sai_home>` を全部合計し、枠の値は updated_at が一番新しい物を使う。"""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        home_a = root / "a"
        home_b = root / "b"
        write_cc_home(
            home_a,
            five_pct=23.5,
            age=3600,
            cost_files={
                "a.json": {"session_id": "a", "total_cost_usd": 1.0, "updated_at": iso(CC_NOW - 600)},
            },
        )
        write_cc_home(
            home_b,
            five_pct=99.0,
            age=60,
            cost_files={
                "b.json": {"session_id": "b", "total_cost_usd": 0.5, "updated_at": iso(CC_NOW - 60)},
            },
        )
        card = usage.cc_card(homes=[home_a, home_b], now=CC_NOW)
    assert card["limits"]["updated_at"] == iso(CC_NOW - 60), card
    assert card["limits"]["five_hour"]["used_percentage"] == 99.0, card
    assert abs(card["cost"]["today"] - 1.5) < 1e-9, card
    assert card["cost"]["sessions"] == 2, card


def test_cc_card_no_limits() -> None:
    """枠の値の無い home（Pro・Max 以外・応答の前）は limits が None になる。"""
    with tempfile.TemporaryDirectory() as tmp:
        home = Path(tmp)
        card = usage.cc_card(homes=[home], now=CC_NOW)
    assert card["limits"] is None, card
    assert card["cost"] == {"today": 0.0, "month": 0.0, "sessions": 0}, card


def main() -> None:
    test_cc_card_single()
    test_cc_card_homes()
    test_cc_card_no_limits()
    print("test_usage: すべて通りました")


if __name__ == "__main__":
    main()
