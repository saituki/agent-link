"""Usage（Claude Code の上限と費用）のデータ集め．

`agent-link cc-statusline` が `<sai_home>` に書いたファイルを読むだけで，外部 API は呼ばない．
"""

from __future__ import annotations

import calendar
import json
import time
from pathlib import Path

from agent_link_web import registry


def date_key(ts_ms: float) -> str:
    """時刻（ms）→ 端末の現地時刻の日付（``2026-09-24``）．"""
    return time.strftime("%Y-%m-%d", time.localtime(ts_ms / 1000))


# statusLine の入力の rate_limits を書いたファイル（`<sai_home>` の直下）
CC_RATE_FILE = "cc-rate-limits.json"
# 会話ごとの費用（`<sai_home>/cc-cost/<session_id>.json`）
CC_COST_DIR = "cc-cost"


def cc_homes() -> list[Path]:
    """cc の記録を置く `<sai_home>` の一覧。

    `registry.sai_home()` 1 つ。
    """
    return [registry.sai_home()]


def _is_num(value) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def _read_json_file(path: Path) -> dict | None:
    """JSON のオブジェクトを読む（無い・壊れている・オブジェクトでないなら None）。"""
    try:
        body = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    return body if isinstance(body, dict) else None


def _iso_epoch(text) -> float | None:
    """`2026-10-11T07:00:00Z`（UTC）→ 秒（読めなければ None）。"""
    if not isinstance(text, str):
        return None
    try:
        return float(calendar.timegm(time.strptime(text, "%Y-%m-%dT%H:%M:%SZ")))
    except ValueError:
        return None


def cc_limits(homes: list[Path], now: float) -> dict | None:
    """5 時間枠・7 日枠の値（各 home の `cc-rate-limits.json` のうち `updated_at` が一番新しいもの）。

    戻る時刻（`resets_at`）を過ぎた窓は今の値ではないので落とす。1 つも残らなければ None。
    """
    newest: tuple[float, dict] | None = None
    for home in homes:
        body = _read_json_file(home / CC_RATE_FILE)
        at = _iso_epoch(body.get("updated_at")) if body else None
        if at is not None and (newest is None or at > newest[0]):
            newest = (at, body)
    if newest is None:
        return None
    body = newest[1]
    out: dict = {}
    for key in ("five_hour", "seven_day"):
        win = body.get(key)
        if not isinstance(win, dict) or not _is_num(win.get("used_percentage")):
            continue
        resets = win.get("resets_at") if _is_num(win.get("resets_at")) else None
        if resets is not None and resets < now:
            continue
        out[key] = {"used_percentage": float(win["used_percentage"]), "resets_at": resets}
    if not out:
        return None
    out["updated_at"] = body["updated_at"]
    return out


def cc_cost(homes: list[Path], now: float) -> dict:
    """cc の費用の合計（今日・今月・今月の会話数）。読めないファイルは飛ばす。

    日付は `updated_at` をサーバの現地時刻で割る（``date_key`` と同じ）。
    # ponytail: total_cost_usd は会話ごとの累計なので、日をまたいだ会話は最後に更新した日に全額が入る。日ごとに正しく割るなら差分を記録する
    """
    today = date_key(now * 1000)
    month = today[:7]
    out = {"today": 0.0, "month": 0.0, "sessions": 0}
    for home in homes:
        for path in (home / CC_COST_DIR).glob("*.json"):
            body = _read_json_file(path)
            at = _iso_epoch(body.get("updated_at")) if body else None
            if at is None or not _is_num(body.get("total_cost_usd")):
                continue
            day = date_key(at * 1000)
            if day[:7] != month:
                continue
            out["sessions"] += 1
            out["month"] += float(body["total_cost_usd"])
            if day == today:
                out["today"] += float(body["total_cost_usd"])
    out["today"] = round(out["today"], 6)
    out["month"] = round(out["month"], 6)
    return out


def cc_card(homes: list[Path] | None = None, now: float | None = None) -> dict:
    """Claude（cc）のカード（枠の値と費用・計画 U02）。枠の値が無いときは ``limits`` が None。"""
    now = time.time() if now is None else now
    homes = cc_homes() if homes is None else homes
    return {"limits": cc_limits(homes, now), "cost": cc_cost(homes, now)}
