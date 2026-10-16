"""ホーム画面の集計（`GET /api/home`・U02 §1 / U3 §変更内容 3）。

1 本の住所で次をまとめて返す（共有契約 C-A3）:
- 答えを待っている質問（`store.list_open_questions`）
- 動いているプロジェクト（`control.list_projects` + `control.phone_tree` の review / todo の一覧）

`GET /api/home/overview`（`build_overview`）は全プロジェクトのダッシュボードを足し合わせたもの。
各プロジェクトの `phone dashboard` の JSON を足し合わせるだけで、それ以外の集計は作らない。

大原則（`docs/integration.md` §3）どおり、他アプリの DB は開かない。
登録簿と agent-link の CLI の JSON だけを使う。

1 つのプロジェクトが失敗しても全体は止めない（`errors` に「<名前>: <理由>」を足す）。
画面が開くたび・更新するたびに全プロジェクトの CLI を呼ばないよう、結果は 10 秒だけ覚える。
"""

from __future__ import annotations

import concurrent.futures
import logging
import time

from fastapi import HTTPException

from agent_link_web import control, runs, store

logger = logging.getLogger("agent_link_web")

# 一覧の題・要約の長さの上限（画面が溢れないように切る）
_TITLE_MAX = 200
# 結果を覚える秒数（ホームを開くたび・更新のたびに全プロジェクトの CLI を呼ばないため）
_HOME_TTL = 10.0
# プロジェクトごとの同時実行数（agent-link の CLI は重い）
_HOME_WORKERS = 4

# 合計のダッシュボードが取れる期間（CLI の dashboard と同じ）
_OVERVIEW_PERIODS = ("week", "month", "all")
# 合計の「最近のアクティビティ」が出す件数
_OVERVIEW_RECENT = 20
# summary で足し合わせる数（agent-link phone dashboard .summary と同じ名前）
_SUMMARY_KEYS = (
    "executions",
    "completed",
    "failed",
    "running",
    "duration_ms",
    "tokens_input",
    "tokens_output",
    "tokens_cache",
    "tokens_total",
    "cost_usd",
)

# 覚えておいた結果（キーは空文字 1 つ → (時刻, 結果)）。テストから消すため global で持つ。
_cache: dict[str, tuple[float, dict]] = {}
# 合計の覚え置き（期間ごとの (時刻, 結果)）。`forget()` で消す。
_overview_cache: dict[str, tuple[float, dict]] = {}


def _now() -> float:
    return time.monotonic()


def _text(value, limit: int = _TITLE_MAX) -> str:
    """文字列にして長さで切る（None や数字が混ざっても落ちない）。"""
    if value is None:
        return ""
    return str(value)[:limit]


def _run_of_session(session_map: list[tuple[str, set[str], int]], session_id) -> tuple[str | None, int | None]:
    """session_id を含む実行を探して `(プロジェクト名, run_id)` を返す。無ければ (None, None)。"""
    if not isinstance(session_id, str) or not session_id:
        return None, None
    for project_name, ids, run_id in session_map:
        if session_id in ids:
            return project_name, run_id
    return None, None


def _session_map() -> list[tuple[str, set[str], int]]:
    """`(プロジェクト名, セッション ID の一致, run_id)` の並び（質問・承認の対応づけ用）。"""
    out: list[tuple[str, set[str], int]] = []
    try:
        all_runs = runs.load_runs()
    except Exception as e:  # 実行の書き出しが読めなくても質問は出せる
        logger.warning(f"実行の一覧が読めません（質問の対応づけだけ無しにします）: {e}")
        return out
    for run in all_runs:
        run_id = run.get("run_id")
        if not isinstance(run_id, int) or isinstance(run_id, bool):
            continue
        out.append(
            (
                str(run.get("project_name") or ""),
                runs.run_session_ids(run),
                run_id,
            )
        )
    return out


def _home_questions() -> list[dict]:
    """答えを待っている質問（新しい順・`store.list_open_questions`）。"""
    session_map = _session_map()
    out: list[dict] = []
    for row in store.list_open_questions():
        if not isinstance(row, dict):
            continue
        questions = row.get("questions")
        first = questions[0] if isinstance(questions, list) and questions else {}
        text = first.get("question") if isinstance(first, dict) else None
        project_name, run_id = _run_of_session(session_map, row.get("session_id"))
        out.append(
            {
                "id": row.get("id") or "",
                "agent": row.get("agent") or "",
                "title": _text(text) if text else "質問",
                "created_at": row.get("created_at"),
                "project": project_name,
                "run_id": run_id,
            }
        )
    return out


def _folder_runs(items, label: str) -> list[dict]:
    """`phone tree` の中の top レベルの `label` フォルダにある run を数える前提で取り出す。"""
    out: list[dict] = []
    for node in items if isinstance(items, list) else []:
        if not isinstance(node, dict) or node.get("type") != "folder":
            continue
        if str(node.get("label") or node.get("id") or "") != label:
            continue
        children = node.get("children")
        for child in children if isinstance(children, list) else []:
            if not isinstance(child, dict) or child.get("type") != "run":
                continue
            if child.get("run_id") is None:
                continue
            row = {
                "run_id": child.get("run_id"),
                "label": _text(child.get("label") or f'ID{child.get("run_id")}'),
            }
            # 実行予約した todo だけ `scheduled_at`（`YYYY-MM-DD HH:MM`）が付く
            if isinstance(child.get("scheduled_at"), str) and child["scheduled_at"]:
                row["scheduled_at"] = _text(child["scheduled_at"])
            # todo だけ task.md の項目（作成日・block・model・説明）が付く
            card = child.get("card")
            if isinstance(card, dict):
                row["card"] = {k: _text(card.get(k) or "") for k in ("creation_date", "block", "model", "description")}
            out.append(row)
    return out


def _home_project(row: dict) -> dict:
    """1 プロジェクトのホームの行（`phone tree` の review / todo を使う）。"""
    name = str(row.get("name") or "")
    tree = control.phone_tree(name) or {}
    items = tree.get("items") if isinstance(tree, dict) else None
    review = _folder_runs(items, "review")
    todo = _folder_runs(items, "todo")
    return {
        "name": name,
        "daemon_alive": bool(row.get("daemon_alive")),
        "running": row.get("running") or 0,
        "last_executed_at": row.get("last_executed_at"),
        "review": review,
        "todo": len(todo),
        # 件数だけだった todo を、行ごと返す（画面の todo カードが押せるようにするため・U02 §2）
        "todo_runs": todo,
    }


def _error_text(e: Exception) -> str:
    """1 つの失敗の理由（HTTPException の detail が日本語なのでそれを優先する）。"""
    if isinstance(e, HTTPException):
        return _text(e.detail) or f"HTTP {e.status_code}"
    return _text(e) or "原因不明のエラー"


def _build() -> dict:
    """集める本体（10 秒の覚え置きは `build_home` 側）。"""
    errors: list[str] = []
    project_rows = [
        row
        for row in control.list_projects()
        if row.get("root_exists") and row.get("db_exists")
    ]

    results: dict[str, object] = {}
    # プロジェクトの集計は CLI を待つので並べる。
    if project_rows:
        with concurrent.futures.ThreadPoolExecutor(max_workers=_HOME_WORKERS) as pool:
            futures = {
                pool.submit(_home_project, row): str(row.get("name") or "")
                for row in project_rows
            }
            for future in concurrent.futures.as_completed(futures):
                name = futures[future]
                try:
                    results[name] = future.result()
                except Exception as e:  # 1 つが失敗しても全体は止めない
                    logger.warning(f"ホームの集計に失敗（他は続けます）: {name}: {e}")
                    errors.append(f"{name}: {_error_text(e)}")

    # 並びは登録簿のまま（最後に動いた順）。
    projects_out = [results[str(r.get("name") or "")] for r in project_rows
                    if str(r.get("name") or "") in results]

    return {
        "questions": _home_questions(),
        "projects": projects_out,
        "errors": errors,
    }


def build_home() -> dict:
    """ホームの集計（`GET /api/home`・U3 §変更内容 3）。

    10 秒だけ覚える（開くたび・更新するたびに全プロジェクトの CLI を呼ばないため）。
    """
    now = _now()
    hit = _cache.get("")
    if hit is not None and now - hit[0] < _HOME_TTL:
        return hit[1]
    value = _build()
    _cache[""] = (now, value)
    return value


def forget() -> None:
    """覚え置きを消す（テストと、強制的に読み直すときに使う）。"""
    _cache.clear()
    _overview_cache.clear()


# --- 全プロジェクトの合計（`GET /api/home/overview`・U02 §1） ---


def _overview_project(name: str, period: str) -> dict:
    """1 プロジェクトのダッシュボード（`agent-link phone dashboard`）。"""
    return control.dashboard(name, period)


def _add_summary(total: dict, summary: dict) -> None:
    """summary の同じ名前の数を足す（0 の項目は足さない）。"""
    for key in _SUMMARY_KEYS:
        value = summary.get(key)
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            continue
        total[key] += value


def _merge_daily(total: dict, daily) -> None:
    """日ごとの実行数と token を日付ごとに足す。"""
    for row in daily if isinstance(daily, list) else []:
        if not isinstance(row, dict):
            continue
        date = str(row.get("date") or "")
        if not date:
            continue
        hit = total.get(date)
        if hit is None:
            hit = {"date": date, "executions": 0, "tokens": 0}
            total[date] = hit
        executions = row.get("executions")
        tokens = row.get("tokens")
        if isinstance(executions, (int, float)) and not isinstance(executions, bool):
            hit["executions"] += executions
        if isinstance(tokens, (int, float)) and not isinstance(tokens, bool):
            hit["tokens"] += tokens


def _merge_recent(total: list[dict], recent, name: str) -> None:
    """全プロジェクトの「最近の実行」をまとめる（プロジェクト名が無い行には登録簿の名前にする）。"""
    for row in recent if isinstance(recent, list) else []:
        if not isinstance(row, dict):
            continue
        run_id = row.get("run_id")
        if not isinstance(run_id, int) or isinstance(run_id, bool):
            continue
        total.append(
            {
                "project": str(row.get("project") or name),
                "run_id": run_id,
                "parent_run_id": row.get("parent_run_id"),
                "block": _text(row.get("block")),
                "status": str(row.get("status") or ""),
                "started_at": row.get("started_at"),
                "description": _text(row.get("description")),
                "linkable": bool(row.get("linkable")),
            }
        )


def _recent_key(row: dict) -> tuple[str, int]:
    """最近の実行の並び替えの鍵（新しい順・同じ時刻なら run_id の大きい順）。"""
    stamp = str(row.get("started_at") or "")
    return (stamp, int(row.get("run_id") or 0))


def _build_overview(period: str) -> dict:
    """全プロジェクトを足し合わせる本体（10 秒の覚え置きは `build_overview` 側）。"""
    errors: list[str] = []
    project_rows = [
        row
        for row in control.list_projects()
        if row.get("root_exists") and row.get("db_exists")
    ]
    names = [str(row.get("name") or "") for row in project_rows]

    got: dict[str, dict] = {}
    if names:
        # プロジェクトごとに CLI を待つので並べる（1 つが失敗しても全体は止めない）
        with concurrent.futures.ThreadPoolExecutor(max_workers=_HOME_WORKERS) as pool:
            futures = {
                pool.submit(_overview_project, name, period): name
                for name in names
            }
            for future in concurrent.futures.as_completed(futures):
                name = futures[future]
                try:
                    data = future.result()
                except Exception as e:
                    logger.warning(f"合計の集計に失敗（他は続けます）: {name}: {e}")
                    errors.append(f"{name}: {_error_text(e)}")
                    continue
                if isinstance(data, dict):
                    got[name] = data

    # 並びは登録簿のまま（最後に動いた順）
    rows = [(name, got[name]) for name in names if name in got]

    summary = {key: 0 for key in _SUMMARY_KEYS}
    daily_total: dict[str, dict] = {}
    recent: list[dict] = []
    dates: list[str] = []
    for name, data in rows:
        raw_summary = data.get("summary")
        _add_summary(summary, raw_summary if isinstance(raw_summary, dict) else {})
        _merge_daily(daily_total, data.get("daily"))
        _merge_recent(recent, data.get("recent"), name)
        for key in ("since", "until"):
            value = str(data.get(key) or "")
            if len(value) == 10:
                dates.append(value)
    for row in daily_total.values():
        dates.append(row["date"])

    # 成功率は足したあとに求め直す（0 件なら 0.0）
    summary["success_rate"] = (
        round(summary["completed"] / summary["executions"], 4)
        if summary["executions"] > 0
        else 0.0
    )
    recent.sort(key=_recent_key, reverse=True)

    return {
        "period": period,
        "since": min(dates) if dates else None,
        "until": max(dates) if dates else None,
        "projects": len(rows),
        "summary": summary,
        "daily": [daily_total[date] for date in sorted(daily_total)],
        "recent": recent[:_OVERVIEW_RECENT],
        "errors": errors,
    }


def build_overview(period: str = "week") -> dict:
    """全プロジェクトの合計のダッシュボード（`GET /api/home/overview`・U02 §1）。

    期間は `week` / `month` / `all`（それ以外は CLI を呼ぶ前に 400）。
    期間ごとに 10 秒だけ覚える。
    """
    if period not in _OVERVIEW_PERIODS:
        raise HTTPException(
            status_code=400, detail="period は week / month / all のいずれかにしてください"
        )
    now = _now()
    hit = _overview_cache.get(period)
    if hit is not None and now - hit[0] < _HOME_TTL:
        return hit[1]
    value = _build_overview(period)
    _overview_cache[period] = (now, value)
    return value
