"""agent-link の呼び出し（Project タブ・List・task.md の作成と新形式 todo の実行・
一覧・Dashboard・常駐の操作）。

 agent-link-web は DB を直接開かないため，ここはすべて CLI に頼る。
 - 一覧       : 登録簿（`registry.load_registry`）+ `agent-link project summary`
 - Dashboard  : `agent-link project dashboard --period …`（cwd = プロジェクトの root）
 - 常駐の操作 : `agent-link daemon <start|stop>`（cwd = プロジェクトの root・文字出力）
 - 書き出し   : `agent-link run export`（cwd = プロジェクトの root）
 - List 画面  : `agent-link run tree / run log|stats|history <id> / run md <id> --kind … /
   doc list --run <id> / doc read <token>`
   （cwd = root かつ `AGENT_LINK_PROJECT_ROOT` = root。値の検査と
   400 / 404 / 409 / 502 もここで・Web が CLI を呼ぶ約束）
 - 作成フォーム: `agent-link card options / create`（入力は task_text に統一）
 - ▶ 実行     : `agent-link card run <run_id>`（新形式 todo）
 - Workflow と定義の編集:
     - Workflow 盤面: `agent-link workflow`
     - 定義（block / template / settings）: `agent-link def list / show / write / delete`
       （検査と 400 / 404 / 409 / 502 の分け方もここで）
"""

from __future__ import annotations

import json
import logging
import os
import re
import time
from datetime import date

from fastapi import HTTPException

from agent_link_web import registry, runs
from agent_link_web.cli import raise_for_failure, run_cli, run_json
from agent_link_web.config import settings

logger = logging.getLogger("agent_link_web")

# 同じプロジェクトの書き出しは 60 秒に 1 回まで
_EXPORT_INTERVAL = 60.0
_last_export: dict[str, float] = {}

_PERIODS = ("week", "month", "all")

# 同じプロジェクトの最小集計は 5 秒に 1 回まで（一覧のたびに呼ぶと重い）
_SUMMARY_TTL = 5.0
# キーはプロジェクト名
_summary_cache: dict[str, tuple[float, dict]] = {}

# --- List 画面で受け取る値の検査 ---
# `agent-link run <view>` が返す 3 種
_PHONE_VIEWS = ("log", "stats", "history")
# `agent-link run md --kind` が返す 4 種（system は固定部分）
_PHONE_MD_KINDS = ("task", "message", "report", "system")
# 保存できる md（task は未実行の todo カードだけ。確かめるのは agent-link）
_PHONE_MD_WRITABLE = ("task",)
# waggle トークンとブロック ID に許す文字（英数字と -_.:・長さ上限付き）
_PHONE_TOKEN_RE = re.compile(r"^[A-Za-z0-9._:-]{1,128}$")
# History の呼び出しの番号（cc の tool_use.id・pi の toolCall.id の形。`/` は入れない）
_PHONE_CALL_ID_RE = re.compile(r"[A-Za-z0-9_-]{1,80}")

# --- task.md 作成フォームで受け取る値の検査 ---
# thinking は一覧の値だけ
_CARD_THINKING = ("none", "low", "medium", "high", "xhigh", "max")
# model に許す文字
_CARD_MODELS = ("opus", "sonnet", "haiku")
# task_text の上限（100KB・UTF-8 のバイト数で数える）
_TASK_TEXT_MAX = 100 * 1024


def _project_summary(name: str, root: str) -> dict:
    """1 プロジェクトの最小集計（`agent-link project summary`）。

    - DB が無い・root が無い・失敗したときは `running=0` / `last_executed_at=None`
      を返す（一覧は止めない）。
    - 呼ぶ回数を抑えるため、同じプロジェクトは 5 秒だけ覚える。
    """
    empty = {"running": 0, "last_executed_at": None}
    now = time.monotonic()
    key = name
    hit = _summary_cache.get(key)
    if hit is not None and now - hit[0] < _SUMMARY_TTL:
        return hit[1]
    # 失敗しても呼びすぎないよう，先に時刻を記録する
    _summary_cache[key] = (now, empty)
    if not root:
        return empty
    try:
        data = _cli_json(["project", "summary"], root=root, timeout=10.0)
    except HTTPException as e:
        logger.warning(f"最小集計に失敗（一覧は続けます）: {name}: {e.detail}")
        return empty
    running = data.get("running")
    if not isinstance(running, int) or isinstance(running, bool) or running < 0:
        running = 0
    value = {"running": running, "last_executed_at": data.get("last_executed_at") or None}
    _summary_cache[key] = (now, value)
    return value


def list_projects() -> list[dict]:
    """登録簿の全プロジェクト（最後に動いた順・`last_executed_at` が無い物は最後）．

    - 一覧そのものは登録簿（`registry.load_registry`）から作る。
    - `running` と `last_executed_at` は DB が要るので `agent-link project summary`
      で聞く（root と DB が両方あるプロジェクトだけ）。
    """
    projects = []
    for row in registry.load_registry():
        root_exists = bool(row.get("root_exists"))
        db_exists = bool(row.get("db_exists"))
        summary = (
            _project_summary(str(row.get("name") or ""), str(row.get("root") or ""))
            if root_exists and db_exists
            else {"running": 0, "last_executed_at": None}
        )
        projects.append(
            {
                "name": row.get("name") or "",
                "root": row.get("root") or "",
                "kind": row.get("kind") or "",
                "root_exists": root_exists,
                "db_exists": db_exists,
                "daemon_alive": bool(row.get("daemon_alive")),
                "running": summary["running"],
                "last_executed_at": summary["last_executed_at"],
            }
        )
    with_time = sorted(
        [p for p in projects if p["last_executed_at"]],
        key=lambda p: p["last_executed_at"],
        reverse=True,
    )
    without = [p for p in projects if not p["last_executed_at"]]
    return with_time + without


def find_project(name: str) -> dict:
    """名前で探す。無ければ 404．"""
    for p in list_projects():
        if p["name"] == name:
            return p
    raise HTTPException(status_code=404, detail="Project not found.")


def export_runs(project: dict) -> None:
    """`agent-link run export` で実行を書き出させる（メッセージ画面が開けるよう）。

    - root / db が無い物は呼ばない。
    - 失敗はログだけ（画面は止めない）。
    - 同じプロジェクトは 60 秒に 1 回まで。
    """
    if not project.get("root_exists") or not project.get("db_exists"):
        return
    root = project.get("root") or ""
    name = project.get("name") or ""
    if not root:
        return
    now = time.monotonic()
    export_key = name
    last = _last_export.get(export_key)
    if last is not None and now - last < _EXPORT_INTERVAL:
        return
    # 失敗しても呼びすぎないよう，先に時刻を記録する
    _last_export[export_key] = now
    try:
        run_cli(
            [settings.sai_link_bin, "run", "export"],
            app="agent-link",
            cwd=root,
            env=_cli_env(root),
            timeout=10,
        )
    except HTTPException as e:
        logger.warning(f"実行の書き出しに失敗（画面は続けます）: {name}: {e.detail}")


def dashboard(name: str, period: str) -> dict:
    """カード型 Dashboard．`recent` に description / linkable を足す。"""
    if period not in _PERIODS:
        raise HTTPException(
            status_code=400, detail="period must be one of week, month, all."
        )
    project = find_project(name)
    if not project.get("db_exists"):
        raise HTTPException(status_code=404, detail="This project has no run records yet.")
    export_runs(project)
    data = _cli_json(["project", "dashboard", "--period", period], root=project["root"])
    # recent の各行に description と linkable を足す。
    # メッセージ画面は書き出しファイルが無いと開けないため。
    root = project.get("root") or ""
    exported = {
        (r.get("project_root"), r.get("run_id")): r for r in runs.load_runs()
    }
    for row in data.get("recent") or []:
        if not isinstance(row, dict):
            continue
        hit = exported.get((root, row.get("run_id")))
        row["description"] = (hit or {}).get("description") or ""
        row["linkable"] = hit is not None
        # 書き出し側の名前（登録簿の名前と違ってもメッセージ画面の URL を壊さないため）
        if hit is not None:
            row["project"] = hit.get("project_name") or name
    return data


def daemon(name: str, action: str) -> dict:
    """常駐の起動・停止．CLI は文字出力なので run_cli で呼ぶ。

    - プロジェクト名から登録簿を引く（無ければ 404）。root が無い場合も 404。
    - 実行の記録が無い（DB が無い）プロジェクトでも起動できる。
    """
    if action not in ("start", "stop"):
        raise HTTPException(status_code=400, detail="action must be start or stop.")
    project = find_project(name)
    if not project.get("root_exists"):
        raise HTTPException(status_code=404, detail="Project folder not found.")
    root = project["root"]
    proc = run_cli(
        [settings.sai_link_bin, "daemon", action],
        app="agent-link",
        cwd=root,
        env=_cli_env(root),
        timeout=30.0,
    )
    if proc.returncode == 0:
        lines = [ln for ln in (proc.stdout or "").splitlines() if ln.strip()]
        fallback = "Start requested." if action == "start" else "Stop requested."
        return {"ok": True, "message": lines[-1].strip() if lines else fallback}
    # 失敗は JSON を返す命令と同じ読み替え（Web が CLI を呼ぶ約束）
    logger.warning(f"常駐の操作に失敗: {name} {action}")
    raise_for_failure("agent-link", proc)
    raise AssertionError("raise_for_failure は必ず失敗させる")


def _cli_env(root: str) -> dict:
    """`agent-link <新しい命令>` 用の環境変数。

    `AGENT_LINK_PROJECT_ROOT` はデーモン起動時に設定された値が優先されるため，
    起動元の環境に引きずられず選んだプロジェクトの DB を開けるよう，
    ここで必ず登録簿の root に置き換える（agent-link `project::project_root`）。
    """
    env = dict(os.environ)
    env["AGENT_LINK_PROJECT_ROOT"] = root
    return env


def _cli_json(
    args: list[str],
    *,
    root: str,
    timeout: float = 20.0,
    stdin_text: str | None = None,
):
    """`agent-link <新しい命令>`（cwd = プロジェクトの root）を呼んで JSON を返す。

    読み替えは `cli.run_json` の 1 か所に集めた（Web が CLI を呼ぶ約束）。
    終了コード 3 は文に関係なく 404，2 は 400，4 は 409，1 とそのほかは 502。
    `stdin_text` は標準入力へ渡す（`card create` の JSON）。

    テストが `control.run_cli` を差し替えても効くよう，`runner` にこのモジュールの
    `run_cli`（差し替え可能な名前）を渡す。
    """
    return run_json(
        [settings.sai_link_bin, *args],
        app="agent-link",
        cwd=root,
        env=_cli_env(root),
        timeout=timeout,
        input_text=stdin_text,
        runner=run_cli,
    )


def _cli_project(name: str) -> dict:
    """List 画面が呼ぶ CLI の前に登録簿を確かめる。

    - 名前で登録簿から引き（無ければ 404）。
    - root / db が無い物は呼ばない（404）。
    """
    project = find_project(name)
    if not project.get("root_exists"):
        raise HTTPException(status_code=404, detail="Project folder not found.")
    if not project.get("db_exists"):
        raise HTTPException(status_code=404, detail="This project has no run records yet.")
    return project


def _check_run_id(run_id: int) -> None:
    """run_id は正の整数だけ（外から来る値なので必ず確かめる）。"""
    if not isinstance(run_id, int) or isinstance(run_id, bool) or run_id <= 0:
        raise HTTPException(status_code=400, detail="run_id must be an integer of 1 or more.")


def cli_tree(name: str) -> dict:
    """`agent-link run tree`（左サイドバーと同じツリー）。"""
    project = _cli_project(name)
    return _cli_json(["run", "tree"], root=project["root"])


def cli_run_view(name: str, run_id: int, view: str) -> dict:
    """`agent-link run log|stats|history <id>`（Log / Stats / History）。"""
    _check_run_id(run_id)
    if view not in _PHONE_VIEWS:
        raise HTTPException(
            status_code=400,
            detail="view must be one of log, stats, history.",
        )
    project = _cli_project(name)
    return _cli_json(["run", view, str(run_id)], root=project["root"])


def cli_run_call(name: str, run_id: int, call_id: str) -> dict:
    """`agent-link run history <id> --call <call_id> --json`（History の 1 呼び出しの全文）。

    `call_id` の形が通らなければ CLI を呼ばず 400。見つからなければ CLI の 3 から 404。
    """
    _check_run_id(run_id)
    if not isinstance(call_id, str) or not _PHONE_CALL_ID_RE.fullmatch(call_id):
        raise HTTPException(status_code=400, detail="call_id must be at most 80 characters of letters, digits, - and _.")
    project = _cli_project(name)
    return _cli_json(
        ["run", "history", str(run_id), "--call", call_id, "--json"], root=project["root"]
    )


def cli_md(name: str, run_id: int, kind: str) -> dict:
    """`agent-link run md <id> --kind <kind>`（無いときは 404）。"""
    _check_run_id(run_id)
    if kind not in _PHONE_MD_KINDS:
        raise HTTPException(
            status_code=400,
            detail="kind must be one of task, message, report, system.",
        )
    project = _cli_project(name)
    return _cli_json(["run", "md", str(run_id), "--kind", kind], root=project["root"])


def cli_md_save(name: str, run_id: int, kind: str, text) -> dict:
    """`agent-link run md <id> --kind <kind> --write`（本文は標準入力・保存後の本文を返す）。

    - 未実行のカードでない task（実行済み・送信済み）は 404（CLI の文をそのまま使う）。
    - frontmatter を壊す内容などその他の失敗は CLI の約束どおりの読み替え。
    """
    _check_run_id(run_id)
    if kind not in _PHONE_MD_WRITABLE:
        raise HTTPException(status_code=400, detail="Only task can be saved.")
    if not isinstance(text, str) or len(text.encode("utf-8")) > _TASK_TEXT_MAX:
        raise HTTPException(status_code=400, detail="The text must be a string of at most 100KB.")
    project = _cli_project(name)
    return _cli_json(
        ["run", "md", str(run_id), "--kind", kind, "--write"],
        root=project["root"],
        stdin_text=text,
    )


def cli_waggle(name: str, run_id: int, token: str | None = None) -> dict:
    """`agent-link doc list --run <id> --json` / `doc read <token> --json`（成果物の一覧と本文）。

    画面が読む項目（`items[].token|label|description|status|ref`・`token|label|text`）は
    今までと同じ形で返す。
    """
    _check_run_id(run_id)
    if token is not None:
        if not _PHONE_TOKEN_RE.match(token or ""):
            raise HTTPException(status_code=400, detail="token is invalid.")
        args = ["doc", "read", token, "--json"]
    else:
        args = ["doc", "list", "--run", str(run_id), "--json"]
    project = _cli_project(name)
    return _cli_json(args, root=project["root"])


# --- 文書の検索（`doc search`）で受け取る値の検査 ---

_SEARCH_CATEGORIES = ("task", "message", "waggle")
_SEARCH_WORD_MAX = 200
_SEARCH_LIMIT_MAX = 100
_SEARCH_DAY_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
# block 名（`.agent-link/blocks/<名前>.md`）の形
_SEARCH_BLOCK_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$")


def _search_day(label: str, value: str | None) -> str | None:
    """`YYYY-MM-DD` の実在する日付だけ（空は None）。"""
    v = (value or "").strip()
    if not v:
        return None
    try:
        if not _SEARCH_DAY_RE.match(v):
            raise ValueError(v)
        date.fromisoformat(v)
    except ValueError:
        raise HTTPException(status_code=400, detail=f"{label} must be in YYYY-MM-DD format.") from None
    return v


def cli_search(
    name: str,
    word: str,
    category: str = "",
    since: str | None = None,
    until: str | None = None,
    block: str | None = None,
    from_id: int | None = None,
    to_id: int | None = None,
    limit: int = 50,
) -> dict:
    """`agent-link doc search`（task / message / waggle の本文検索・会話検索の画面の文書の検索）。

    外から来る値は CLI を呼ぶ前に全部確かめる（通らなければ 400）。
    """
    word = (word or "").strip()
    if not word:
        raise HTTPException(status_code=400, detail="Enter a search word.")
    if len(word) > _SEARCH_WORD_MAX:
        raise HTTPException(status_code=400, detail=f"The search word must be at most {_SEARCH_WORD_MAX} characters.")
    cats = [c.strip() for c in (category or "").split(",") if c.strip()]
    if any(c not in _SEARCH_CATEGORIES for c in cats):
        raise HTTPException(status_code=400, detail="type must be one of task, message, waggle.")
    cats = [c for c in _SEARCH_CATEGORIES if c in cats] or list(_SEARCH_CATEGORIES)
    since = _search_day("Start date", since)
    until = _search_day("End date", until)
    if since and until and since > until:
        raise HTTPException(status_code=400, detail="The start date must be before the end date.")
    block = (block or "").strip() or None
    if block is not None and not _SEARCH_BLOCK_RE.match(block):
        raise HTTPException(status_code=400, detail="block is invalid.")
    for label, n in (("The run_id lower bound", from_id), ("The run_id upper bound", to_id)):
        if n is not None and (isinstance(n, bool) or not isinstance(n, int) or n <= 0):
            raise HTTPException(status_code=400, detail=f"{label} must be an integer of 1 or more.")
    if from_id is not None and to_id is not None and from_id > to_id:
        raise HTTPException(status_code=400, detail="The run_id lower bound must not exceed the upper bound.")
    if isinstance(limit, bool) or not isinstance(limit, int) or not 1 <= limit <= _SEARCH_LIMIT_MAX:
        raise HTTPException(status_code=400, detail=f"The limit must be between 1 and {_SEARCH_LIMIT_MAX}.")

    args = ["doc", "search", "--word", word, "--category", ",".join(cats), "--limit", str(limit)]
    for flag, value in (
        ("--since", since),
        ("--until", until),
        ("--block", block),
        ("--from-id", from_id),
        ("--to-id", to_id),
    ):
        if value is not None:
            args += [flag, str(value)]
    project = _cli_project(name)
    return _cli_json(args, root=project["root"], timeout=60.0)


# --- 会話の検索（`agent-link session search`・会話の検索の口） ---

# run の番号は N または N-M
_SESSION_RANGE_RE = re.compile(r"^\d+(-\d+)?$")
# ブロック名の部分一致（`.agent-link/blocks/<名前>.md` の形）
_SESSION_BLOCK_RE = re.compile(r"^[A-Za-z0-9._:-]{1,64}$")
# 実行日 YYYY/MM/DD または YYYY/MM/DD-YYYY/MM/DD
_SESSION_DATE_RE = re.compile(r"^\d{4}/\d{2}/\d{2}(-\d{4}/\d{2}/\d{2})?$")
_SESSION_STATUSES = ("todo", "pending", "running", "completed", "failed", "cancelled")
_SESSION_TYPES = ("user", "agent", "thinking", "tool_call", "tool_name", "tool_result", "tool")
# 検索語の全体の長さ（カンマ区切りを含む）
_SESSION_WORD_MAX = 200
# 画面に返す件数の上限（これを超えたら truncated）
_SESSION_LIMIT = 200


def _session_subset(label: str, value: str | None, allowed: tuple) -> str | None:
    """候補の並び（カンマ区切り）を確かめて，正規の順にそろえて返す。"""
    text = (value or "").strip()
    if not text:
        return None
    picked = [x.strip() for x in text.split(",") if x.strip()]
    if not picked or any(x not in allowed for x in picked):
        raise HTTPException(status_code=400, detail=f"{label} is invalid.")
    return ",".join(x for x in allowed if x in picked)


def cli_session_search(
    name: str,
    word: str | None = None,
    run_id: str | None = None,
    block: str | None = None,
    on_date: str | None = None,
    status: str | None = None,
    sess_type: str | None = None,
) -> dict:
    """`agent-link session search`（エージェントの会話の履歴の検索）。

    外から来る値は CLI を呼ぶ前に全部確かめる（通らなければ 400）。
    返り値は `{"items": 先頭 200 件, "truncated": 201 件目があるか}`。
    """

    def norm(value: str | None) -> str | None:
        text = (value or "").strip()
        return text or None

    word = norm(word)
    run_id = norm(run_id)
    block = norm(block)
    on_date = norm(on_date)
    status = norm(status)
    sess_type = norm(sess_type)
    if not any((word, run_id, block, on_date, status, sess_type)):
        raise HTTPException(status_code=400, detail="Enter at least one search condition.")

    if word is not None:
        if "\n" in word or "\r" in word:
            raise HTTPException(status_code=400, detail="The search word must not contain a line break.")
        if len(word) > _SESSION_WORD_MAX:
            raise HTTPException(status_code=400, detail=f"The search word must be at most {_SESSION_WORD_MAX} characters.")
    for label, value in (("The run number", run_id),):
        if value is not None and not _SESSION_RANGE_RE.match(value):
            raise HTTPException(status_code=400, detail=f"{label} must be in N or N-M format.")
    if block is not None and not _SESSION_BLOCK_RE.match(block):
        raise HTTPException(status_code=400, detail="block is invalid.")
    if on_date is not None and not _SESSION_DATE_RE.match(on_date):
        raise HTTPException(status_code=400, detail="The date must be in YYYY/MM/DD or YYYY/MM/DD-YYYY/MM/DD format.")
    status = _session_subset("Status", status, _SESSION_STATUSES)
    sess_type = _session_subset("Type", sess_type, _SESSION_TYPES)

    args = ["session", "search"]
    for flag, value in (
        ("--id", run_id),
        ("--block", block),
        ("--date", on_date),
        ("--status", status),
        ("--type", sess_type),
        ("-w", word),
    ):
        if value is not None:
            args += [flag, value]
    args.append("--json")
    project = _cli_project(name)
    data = _cli_json(args, root=project["root"], timeout=60.0)
    if isinstance(data, dict):
        items = data.get("items") or []
    else:
        items = data if isinstance(data, list) else []
    truncated = len(items) > _SESSION_LIMIT
    return {"items": items[:_SESSION_LIMIT], "truncated": truncated}


def cli_card_options(name: str) -> dict:
    """`agent-link card options`（ウィザードの選択肢）。block は絞らない。"""
    project = _cli_project(name)
    return _cli_json(["card", "options"], root=project["root"])


def _validate_card_payload(payload: dict) -> dict:
    """作成フォームが送る値を CLI を呼ぶ前に確かめる。

    - `block` は必須で、英数字と `-_.:` のみ
    - `task_text` は 100KB（UTF-8 のバイト数）まで
    - `thinking` は一覧の値
    返すのは形をそろえた dict（送る JSON そのもの）。

    `inputs` と、廃止した runner・provider・tag・round 系の項目は
    受け取っても検査も送りもしない。古い画面がしばらく送ってくるかもしれないが、断らない。
    """
    if not isinstance(payload, dict):
        raise HTTPException(status_code=400, detail="The submitted value is invalid.")

    def text(key: str) -> str:
        v = payload.get(key, "")
        if v is None:
            return ""
        if not isinstance(v, str):
            raise HTTPException(
                status_code=400, detail=f"{key} must be a string."
            )
        return v

    # done_ids: 1 以上の整数の配列
    done = payload.get("done_ids", [])
    if done is None:
        done = []
    if not isinstance(done, list) or len(done) > 64:
        raise HTTPException(
            status_code=400, detail="done_ids must be an array of integers."
        )
    done_ids: list[int] = []
    for v in done:
        if not isinstance(v, int) or isinstance(v, bool) or v <= 0:
            raise HTTPException(
                status_code=400, detail="done_ids must be integers of 1 or more."
            )
        done_ids.append(v)

    block = text("block")
    if not block:
        raise HTTPException(status_code=400, detail="block is required.")
    if not _PHONE_TOKEN_RE.match(block):
        raise HTTPException(status_code=400, detail="block is invalid.")

    thinking = text("thinking")
    if thinking and thinking not in _CARD_THINKING:
        raise HTTPException(
            status_code=400,
            detail="thinking must be one of none, low, medium, high, xhigh, max (empty to skip).",
        )

    model = text("model")
    if model and model not in _CARD_MODELS:
        raise HTTPException(
            status_code=400,
            detail="model must be one of opus, sonnet, haiku (empty to skip).",
        )

    # 一覧に出す説明（200 文字までの 1 行）。空なら agent-link が決める
    description = text("description")
    if len(description) > 200 or "\n" in description or "\r" in description:
        raise HTTPException(
            status_code=400, detail="description must be a single line of at most 200 characters."
        )

    # file_token 入力（古い画面が送る `inputs`）は受け取っても使わない（入力は task_text に統一）

    task_text = text("task_text")
    if len(task_text.encode("utf-8")) > _TASK_TEXT_MAX:
        raise HTTPException(status_code=400, detail="task_text must be at most 100KB.")

    return {
        "done_ids": done_ids,
        "block": block,
        "model": model,
        "thinking": thinking,
        "description": description,
        "task_text": task_text,
    }


def cli_card_create(name: str, payload: dict) -> dict:
    """`agent-link card create`（stdin に JSON・作成後は todo に配置）。"""
    data = _validate_card_payload(payload)
    project = _cli_project(name)
    return _cli_json(
        ["card", "create"],
        root=project["root"],
        stdin_text=json.dumps(data, ensure_ascii=False),
    )


def cli_card_start(name: str, run_id: int) -> dict:
    """`agent-link card run <run_id>`（新形式 todo の ▶ 実行）。

    - 成功: `{'run_id': N, 'message': '実行を要求しました'}` をそのまま返す。
    - 未実行カードが無い（終了コード 3）: 404。
    - 常駐停止などその他: CLI の約束どおりの読み替え（1 とそのほかは 502）。
    """
    _check_run_id(run_id)
    project = _cli_project(name)
    return _cli_json(
        ["card", "run", str(run_id)],
        root=project["root"],
    )


def cli_finalize(name: str, run_id: int) -> dict:
    """`agent-link run finalize <run_id>`（メッセージ画面の ■ 終了）。

    - 成功: `{'run_id': N, 'message': '終了を要求しました'}` をそのまま返す。
    - その run が無い（終了コード 3）: 404。
    - 実行中でない・子ブロック・常駐停止など: CLI の約束どおりの読み替え（1 とそのほかは 502）。
    """
    _check_run_id(run_id)
    project = _cli_project(name)
    return _cli_json(
        ["run", "finalize", str(run_id)],
        root=project["root"],
    )


def run_send(name: str, run_id: int, text: str) -> dict:
    """`agent-link run send <run_id> --json`（本文は標準入力）。

    - 成功: `{'run_id', 'pane_id', 'status': 'sent'}` をそのまま返す。
    - 空の文（終了コード 2）: 400。無い実行（3）: 404。
    - 終了している・質問待ち（4）: 409。
    """
    _check_run_id(run_id)
    project = _cli_project(name)
    return _cli_json(
        ["run", "send", str(run_id)],
        root=project["root"],
        stdin_text=text,
    )


def run_interrupt(name: str, run_id: int) -> dict:
    """`agent-link run interrupt <run_id> --json`。

    - 成功: `{'run_id', 'status', 'agent_status'}` をそのまま返す。
    - 無い実行（3）: 404。終了している（4）: 409。
    """
    _check_run_id(run_id)
    project = _cli_project(name)
    return _cli_json(
        ["run", "interrupt", str(run_id)],
        root=project["root"],
    )


def cli_advance(name: str, run_id: int) -> dict:
    """`agent-link run advance <run_id>`（review → done → Workflow の移動）。

    - 成功: `{'run_id': N, 'from': '…', 'to': '…'}` をそのまま返す。
    - その run が無い（終了コード 3）: 404。
    - review / done 以外・run フォルダが無いとき: CLI の約束どおりの読み替え。
    """
    _check_run_id(run_id)
    project = _cli_project(name)
    return _cli_json(
        ["run", "advance", str(run_id)],
        root=project["root"],
    )


def cli_rm(name: str, run_id: int) -> dict:
    """`agent-link run rm <run_id> --yes`（List の項目を消す。子の実行も一緒に消える）。

    - 成功: `{'run_id': N, 'executions': 消した実行の数}`。
    - その run が無い（終了コード 3）: 404。実行中など: CLI の約束どおりの読み替え。
    """
    _check_run_id(run_id)
    project = _cli_project(name)
    return _cli_json(
        ["run", "rm", str(run_id), "--yes"],
        root=project["root"],
    )


def cli_card_docs(name: str) -> dict:
    """`agent-link card docs`（最近の文書・task_text に貼る候補）。"""
    project = _cli_project(name)
    return _cli_json(["card", "docs"], root=project["root"])


# 実行予定時刻（`YYYY-MM-DD HH:MM`。ブラウザの datetime-local の `T` 区切りも受ける）
_SCHEDULE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}$")


def cli_card_schedule(name: str, run_id: int, at: str) -> dict:
    """`agent-link card schedule <run_id> [--at …]`（todo の実行予定時刻）。

    `at` が空なら予定を消す。形が違えば CLI を呼ばずに 400。
    """
    _check_run_id(run_id)
    at = (at or "").strip()
    args = ["card", "schedule", str(run_id)]
    if at:
        if not _SCHEDULE_RE.match(at):
            raise HTTPException(status_code=400, detail="The time format is invalid.")
        args += ["--at", at.replace("T", " ")]
    project = _cli_project(name)
    return _cli_json(args, root=project["root"])


# --- Workflow と定義の編集 ---
# agent-link-tui の中にしか無い Workflow の盤面と block / template / settings の編集を、
# Web からも同じ CLI 経由でできるようにする（agent-link-web は DB を開かない）。
# 外から来る値（kind / scope / 名前 / 本文）は、CLI を呼ぶ前に必ずここで確かめる。

# 編集できる定義の種類（`def --kind`）
_DEF_KINDS = ("block", "template", "settings")
# 定義の置き場（そのプロジェクト / 全体で共通の `~/.agent-link/`）
_DEF_SCOPES = ("project", "global")
# block / template の名前（英数字から始まり、英数字と - _ ・64 文字まで・拡張子 .md は付けない）
_DEF_NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$")
# settings の名前は `settings` だけ
_DEF_SETTINGS_NAME = "settings"
# 定義の本文の上限（256KB・UTF-8 のバイト数で数える）
_DEF_TEXT_MAX = 256 * 1024


def _cli_def_project(name: str) -> dict:
    """定義の編集の前に登録簿を確かめる。

    実行の記録（DB）が無くても block は書けるので、root が無いときだけ 404 にする。
    """
    project = find_project(name)
    if not project.get("root_exists"):
        raise HTTPException(status_code=404, detail="Project folder not found.")
    return project


def _cli_def_json(args: list[str], *, root: str, stdin_text: str | None = None):
    """`agent-link def …`（cwd = プロジェクトの root）を呼んで JSON を返す。

    読み替えは Web が CLI を呼ぶ約束と同じ（`cli.run_json` の 1 か所）。
    - 終了コード 0: 標準出力の JSON をそのまま返す。
    - 2: 400（検査に通らない中身の誤り）・3: 404・4: 409・1 とそのほか: 502。
    - `stdin_text` は本文を標準入力へ渡す（`write`）。
    """
    return run_json(
        [settings.sai_link_bin, "def", *args],
        app="agent-link",
        cwd=root,
        env=_cli_env(root),
        timeout=20.0,
        input_text=stdin_text,
        runner=run_cli,
    )


def _validate_def(kind, scope, def_name) -> None:
    """定義の指定（kind / scope / 名前）を確かめる（違反は 400）。"""
    if kind not in _DEF_KINDS:
        raise HTTPException(
            status_code=400, detail="kind must be one of block, template, settings."
        )
    if scope not in _DEF_SCOPES:
        raise HTTPException(
            status_code=400, detail="scope must be project or global."
        )
    if kind == "settings":
        if def_name != _DEF_SETTINGS_NAME:
            raise HTTPException(
                status_code=400, detail="The name for settings must be settings."
            )
        return
    if not isinstance(def_name, str) or not _DEF_NAME_RE.match(def_name):
        raise HTTPException(
            status_code=400, detail="The name must be at most 64 characters of letters, digits, - and _."
        )


def _validate_def_text(text) -> str:
    """定義の本文を確かめる（文字列で 256KB まで・違反は 400）。"""
    if not isinstance(text, str):
        raise HTTPException(status_code=400, detail="The text must be a string.")
    if len(text.encode("utf-8")) > _DEF_TEXT_MAX:
        raise HTTPException(status_code=400, detail="The text must be a string of at most 256KB.")
    return text


def cli_workflow(name: str, subtasks: bool = False) -> dict:
    """Workflow 盤面（`agent-link workflow`・`subtasks` が真なら `--subtasks` で子の実行も含める）。

    実行の記録が要るため、root と DB が両方無いプロジェクトは 404（`_cli_project`）。
    """
    project = _cli_project(name)
    argv = ["workflow", "--subtasks"] if subtasks else ["workflow"]
    return _cli_json(argv, root=project["root"], timeout=30.0)


def cli_def_list(name: str) -> dict:
    """定義の一覧（`agent-link def list`）。"""
    project = _cli_def_project(name)
    return _cli_def_json(["list"], root=project["root"])


def cli_def_show(name: str, kind: str, scope: str, def_name: str) -> dict:
    """定義 1 件の中身（`agent-link def show`）。"""
    _validate_def(kind, scope, def_name)
    project = _cli_def_project(name)
    return _cli_def_json(
        ["show", "--kind", kind, "--scope", scope, "--name", def_name],
        root=project["root"],
    )


def cli_def_write(
    name: str, kind: str, scope: str, def_name: str, text, create: bool = False
) -> dict:
    """定義の保存（`agent-link def write`・本文は標準入力）。

    - 検査に通らない本文（壊れた frontmatter 等）は 400（agent-link が終了コード 2 で返す）。
    - 無い定義は 404、`--create` で同じ名前があるときは 409。
    - 検査は必ず CLI を呼ぶ前に行う（呼び出し 0 回）。
    """
    _validate_def(kind, scope, def_name)
    body = _validate_def_text(text)
    if create is not None and not isinstance(create, bool):
        raise HTTPException(status_code=400, detail="create must be a boolean.")
    args = ["write", "--kind", kind, "--scope", scope, "--name", def_name]
    if create:
        args.append("--create")
    project = _cli_def_project(name)
    return _cli_def_json(args, root=project["root"], stdin_text=body)


def cli_def_delete(name: str, kind: str, scope: str, def_name: str) -> dict:
    """定義の削除（`agent-link def delete`）。settings は消せない（400）。"""
    _validate_def(kind, scope, def_name)
    if kind == "settings":
        raise HTTPException(status_code=400, detail="settings cannot be deleted.")
    project = _cli_def_project(name)
    return _cli_def_json(
        ["delete", "--kind", kind, "--scope", scope, "--name", def_name],
        root=project["root"],
    )

