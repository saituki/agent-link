"""usage.py の集計と provider→アカウント対応の小さなテスト（assert だけ）・計画 42 §3.6．

外部 API は呼ばない．pi の記録は一時フォルダに作った偽物を使う．

実行:
    python3 tests/test_usage.py
    （または PYTHONPATH=src python3 -m pytest tests/test_usage.py）
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
import time
from pathlib import Path

# リポジトリ直下から実行しなくても import できるようにしておく
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_phone import usage  # noqa: E402


def noon_ms() -> float:
    """今日の正午（現地時刻・ms）．日付の境目を気にせず日ごとを試すため．"""
    lt = time.localtime()
    return time.mktime((lt.tm_year, lt.tm_mon, lt.tm_mday, 12, 0, 0, 0, 0, -1)) * 1000


def assistant_line(
    provider: str,
    model: str,
    ts_ms: float,
    *,
    cost: float = 0.0,
    inp: int = 0,
    out: int = 0,
    cache_read: int = 0,
) -> str:
    """pi の記録 1 行（assistant の使用）．"""
    return json.dumps(
        {
            "type": "message",
            "timestamp": ts_ms,
            "message": {
                "role": "assistant",
                "provider": provider,
                "model": model,
                "usage": {
                    "input": inp,
                    "output": out,
                    "cacheRead": cache_read,
                    "cacheWrite": 0,
                    "totalTokens": inp + out + cache_read,
                    "cost": {"total": cost},
                },
            },
        },
        ensure_ascii=False,
    )


def write_session(base: Path, folder: str, name: str, lines: list[str]) -> Path:
    path = base / folder / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return path


def test_read_pi_calls() -> None:
    """assistant の行だけを読み，30 日より古い記録は落とす（42 §3.3）．"""
    now = noon_ms()
    recent = now - 3600 * 1000
    old = now - 40 * 86400 * 1000
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp)
        write_session(
            base,
            "proj-a",
            "s.jsonl",
            [
                # 壊れた行・assistant でない行は読み飛ばす
                "{壊れている",
                json.dumps({"type": "message", "message": {"role": "user"}}),
                assistant_line("openrouter-meta", "meta/muse", recent, cost=0.5, inp=10, out=2, cache_read=3),
                # 30 日より古い記録は残さない
                assistant_line("openrouter-meta", "meta/muse", old, cost=9.9, inp=100),
            ],
        )
        rows = usage.read_pi_calls(base, now_ms=now)
    assert len(rows) == 1, rows
    row = rows[0]
    assert row["provider"] == "openrouter-meta"
    assert row["model"] == "meta/muse"
    assert row["input"] == 10 and row["output"] == 2 and row["cache_read"] == 3
    assert row["total"] == 15
    assert abs(row["cost"] - 0.5) < 1e-9


def test_days_series() -> None:
    """日ごとの費用・token・回数（無い日は 0・42 §3.3）．"""
    now = noon_ms()
    rows = [
        {"timestamp": int(now), "cost": 0.25, "total": 100},
        {"timestamp": int(now - 3600 * 1000), "cost": 0.75, "total": 300},
        {"timestamp": int(now - 2 * 86400 * 1000), "cost": 1.5, "total": 500},
    ]
    series = usage.days_series(rows, now_ms=now, days=30)
    assert len(series) == 30
    today = series[-1]
    assert today["date"] == usage.date_key(now)
    assert abs(today["cost"] - 1.0) < 1e-9
    assert today["tokens"] == 400
    assert today["calls"] == 2
    # 2 日前は別の日の枠に入る
    two_days = series[-3]
    assert two_days["date"] == usage.date_key(now - 2 * 86400 * 1000)
    assert abs(two_days["cost"] - 1.5) < 1e-9
    assert two_days["calls"] == 1
    # 合計は元の合計と一致する
    assert abs(sum(s["cost"] for s in series) - 2.5) < 1e-9


def test_by_model() -> None:
    """モデル別の集計は費用の多い順（42 §3.1 の Activity）．"""
    rows = [
        {"model": "a", "cost": 0.1, "total": 10},
        {"model": "b", "cost": 0.5, "total": 20},
        {"model": "a", "cost": 0.1, "total": 30},
    ]
    out = usage.by_model(rows)
    assert [x["model"] for x in out] == ["b", "a"]
    assert out[0]["calls"] == 1 and out[0]["tokens"] == 20
    assert out[1]["calls"] == 2 and out[1]["tokens"] == 40
    assert abs(out[1]["cost"] - 0.2) < 1e-9
    # モデル名が空でも「(不明)」でまとまる
    assert usage.by_model([{"model": "", "cost": 0, "total": 0}])[0]["model"] == "(不明)"


def test_usage_totals() -> None:
    """今日と今月の使用（pi の記録から・42 §3.1 の DeepSeek カード）．"""
    now = noon_ms()
    lt = time.localtime(now / 1000)
    first_day = time.mktime((lt.tm_year, lt.tm_mon, 1, 12, 0, 0, 0, 0, -1)) * 1000
    rows = [
        {"timestamp": int(now), "cost": 0.2, "total": 20},
        # 同じ月の別の日
        {"timestamp": int(first_day), "cost": 0.3, "total": 30},
    ]
    out = usage.usage_totals(rows, now_ms=now)
    assert out["calls"] == 2
    assert abs(out["today"] - 0.2) < 1e-9
    assert out["today_tokens"] == 20
    # 今月は 1 日の記録も入る（今日が 1 日なら today と同じ値になるだけ）
    assert out["month"] >= 0.2 - 1e-9
    if usage.date_key(first_day) != usage.date_key(now):
        assert abs(out["month"] - 0.5) < 1e-9


def test_map_provider() -> None:
    """provider → アカウントの対応（42 §3.2）．"""
    accounts = [
        {"name": "main", "env": "OPENROUTER_API_KEY"},
        {"name": "sub", "env": "OPENROUTER_API_KEY_2"},
    ]
    env_of = {
        "openrouter-meta": "OPENROUTER_API_KEY",
        "openrouter-deepinfra": "OPENROUTER_API_KEY",
        "openrouter-modal": "OPENROUTER_API_KEY_2",
        "openrouter": "OPENROUTER_API_KEY",
        "deepseek": "DEEPSEEK_API_KEY",
        "google": "GEMINI_API_KEY",
    }
    # 同じ環境変数を参照する provider は同じアカウントに入る
    assert usage.map_provider("openrouter-meta", env_of, accounts) == ("openrouter", "main")
    assert usage.map_provider("openrouter-deepinfra", env_of, accounts) == ("openrouter", "main")
    assert usage.map_provider("openrouter-modal", env_of, accounts) == ("openrouter", "sub")
    assert usage.map_provider("openrouter", env_of, accounts) == ("openrouter", "main")
    # DeepSeek は専用
    assert usage.map_provider("deepseek", env_of, accounts) == ("deepseek", "deepseek")
    # 設定に無い provider（google / 未知の名前）は対応が分からない
    assert usage.map_provider("google", env_of, accounts) is None
    assert usage.map_provider("open-router2", env_of, accounts) is None


def test_filter_rows() -> None:
    """アカウントごとに記録を分ける（42 §3.3 の activity / logs）．"""
    accounts = [{"name": "main", "env": "OPENROUTER_API_KEY"}]
    env_of = {"openrouter-meta": "OPENROUTER_API_KEY", "deepseek": "DEEPSEEK_API_KEY"}
    rows = [
        {"provider": "openrouter-meta", "cost": 1.0},
        {"provider": "deepseek", "cost": 2.0},
        {"provider": "open-router2", "cost": 3.0},
    ]
    main = usage.filter_rows(rows, "openrouter", "main", env_of, accounts)
    assert [r["cost"] for r in main] == [1.0]
    deep = usage.filter_rows(rows, "deepseek", "deepseek", env_of, accounts)
    assert [r["cost"] for r in deep] == [2.0]
    # 設定に無い provider はどのアカウントにも入らない
    assert usage.filter_rows(rows, "openrouter", "sub", env_of, accounts) == []


def test_unknown_providers() -> None:
    """設定に無い provider は別枠で費用の多い順にまとめる（42 §3.2）．"""
    env_of = {"openrouter-meta": "OPENROUTER_API_KEY", "deepseek": "DEEPSEEK_API_KEY"}
    accounts = [{"name": "main", "env": "OPENROUTER_API_KEY"}]
    saved = usage.provider_env_map, usage.openrouter_accounts
    usage.provider_env_map = lambda: env_of
    usage.openrouter_accounts = lambda: accounts
    try:
        rows = [
            {"provider": "open-router2", "cost": 1.0, "total": 10},
            {"provider": "open-router2", "cost": 2.0, "total": 20},
            {"provider": "open-router", "cost": 0.5, "total": 5},
            {"provider": "openrouter-meta", "cost": 9.0, "total": 90},
            {"provider": "deepseek", "cost": 8.0, "total": 80},
        ]
        out = usage.unknown_providers(rows)
    finally:
        usage.provider_env_map, usage.openrouter_accounts = saved
    assert [x["provider"] for x in out] == ["open-router2", "open-router"]
    assert abs(out[0]["cost"] - 3.0) < 1e-9
    assert out[0]["calls"] == 2 and out[0]["tokens"] == 30


def test_openrouter_accounts() -> None:
    """`.env` の書き方の読み取りと既定値（42 §3.2）．"""
    saved = os.environ.get("SAI_PHONE_OPENROUTER_ACCOUNTS")
    try:
        os.environ.pop("SAI_PHONE_OPENROUTER_ACCOUNTS", None)
        assert usage.openrouter_accounts() == [
            {"name": "main", "env": "OPENROUTER_API_KEY"}
        ]
        os.environ["SAI_PHONE_OPENROUTER_ACCOUNTS"] = (
            "main=OPENROUTER_API_KEY, sub=OPENROUTER_API_KEY_2 ,,壊れた"
        )
        assert usage.openrouter_accounts() == [
            {"name": "main", "env": "OPENROUTER_API_KEY"},
            {"name": "sub", "env": "OPENROUTER_API_KEY_2"},
        ]
    finally:
        if saved is None:
            os.environ.pop("SAI_PHONE_OPENROUTER_ACCOUNTS", None)
        else:
            os.environ["SAI_PHONE_OPENROUTER_ACCOUNTS"] = saved


def test_known_target() -> None:
    """外から来た provider / account は設定にある名前だけ受け付ける（42 §3.3）．"""
    saved = os.environ.get("SAI_PHONE_OPENROUTER_ACCOUNTS")
    try:
        os.environ["SAI_PHONE_OPENROUTER_ACCOUNTS"] = (
            "main=OPENROUTER_API_KEY,sub=OPENROUTER_API_KEY_2"
        )
        assert usage.known_target("openrouter", "main")
        assert usage.known_target("openrouter", "sub")
        assert not usage.known_target("openrouter", "other")
        assert usage.known_target("deepseek", "deepseek")
        assert not usage.known_target("deepseek", "main")
        assert not usage.known_target("gemini", "x")
    finally:
        if saved is None:
            os.environ.pop("SAI_PHONE_OPENROUTER_ACCOUNTS", None)
        else:
            os.environ["SAI_PHONE_OPENROUTER_ACCOUNTS"] = saved


def main() -> None:
    test_read_pi_calls()
    test_days_series()
    test_by_model()
    test_usage_totals()
    test_map_provider()
    test_filter_rows()
    test_unknown_providers()
    test_openrouter_accounts()
    test_known_target()
    print("test_usage: すべて通りました")


if __name__ == "__main__":
    main()
