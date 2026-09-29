"""sai-link の呼び出し（Project タブ・計画 06、List は計画 14、
task.md 作成と新形式 todo の実行は計画 15、
一覧・Dashboard・常駐の操作の移設は実装計画 C）。

 sai-phone は DB を直接開かない（00 §2）ため，ここはすべて CLI に頼る。
 - 一覧       : 登録簿（`registry.load_registry`）+ `sai-link phone summary`
 - Dashboard  : `sai-link phone dashboard --period …`（cwd = プロジェクトの root）
 - 常駐の操作 : `sai-link daemon <start|stop>`（cwd = プロジェクトの root・文字出力）
 - 書き出し   : `sai-link phone export`（cwd = プロジェクトの root）
 - List 画面  : `sai-link phone tree / run <id> --view … / md <id> --kind … / waggle <id> [--token …]`
   （cwd = root かつ `SAI_PROJECT_ROOT` = root。値の検査と 400 / 404 / 502 もここで）
 - 作成フォーム: `sai-link phone card options / inputs --block <id> / create`（計画 15 §1.5）
 - ▶ 実行     : `sai-link phone card run <run_id>`（新形式 todo・計画 15）
 - Workflow と定義の編集（U3）:
     - Workflow 盤面: `sai-link phone workflow`
     - 定義（block / template / settings）: `sai-link phone def list / show / write / delete`
       （検査と 400 / 404 / 409 / 502 の分け方もここで）
"""

from __future__ import annotations

import json
import logging
import os
import re
import time

from fastapi import HTTPException

from sai_phone import registry, runs
from sai_phone.cli import run_cli
from sai_phone.config import settings

logger = logging.getLogger("sai_phone")

# 同じプロジェクトの書き出しは 60 秒に 1 回まで（06 §3.3）
_EXPORT_INTERVAL = 60.0
_last_export: dict[str, float] = {}

_PERIODS = ("week", "month", "all")

# 同じプロジェクトの最小集計は 5 秒に 1 回まで（一覧のたびに呼ぶと重い・実装計画 C C1）
_SUMMARY_TTL = 5.0
_summary_cache: dict[str, tuple[float, dict]] = {}

# --- List 画面（計画 14 §3.3）で受け取る値の検査 ---
# `sai-link phone run --view` が返す 3 種（計画 12 §3.2）
_PHONE_VIEWS = ("log", "stats", "history")
# `sai-link phone md --kind` が返す 3 種（計画 12 §3.2）
_PHONE_MD_KINDS = ("task", "message")
# 保存できる md（task は未実行の todo カードだけ。確かめるのは sai-link）
_PHONE_MD_WRITABLE = ("task",)
# waggle トークンとブロック ID に許す文字（英数字と -_.:・長さ上限付き・計画 12 §3.3）
_PHONE_TOKEN_RE = re.compile(r"^[A-Za-z0-9._:-]{1,128}$")
# md が無いときに sai-link が返す stderr（終了コード 3・計画 12 §3.2）
_PHONE_MISSING = "まだありません"
# 未実行カードが無いときに sai-link が返す stderr（終了コード 3・計画 15 の ▶ 実行）
_PHONE_MISSING_START = "未実行のカードが見つかりません"
# 実行が見つからないときに sai-link が返す stderr（終了コード 3・メッセージ画面の ■ 終了）
_PHONE_MISSING_RUN = "実行が見つかりません"

# --- task.md 作成フォーム（計画 15 §1.5・§3.3）で受け取る値の検査 ---
# runner / thinking は一覧の値だけ（TUI の RUNNER_OPTIONS / THINKING_OPTIONS と同じ）
_CARD_RUNNERS = ("user", "pi", "cc", "gl")
_CARD_THINKING = ("none", "low", "medium", "high", "xhigh", "max")
# provider / model に許す文字（`openrouter-xiaomi` / `deepseek/deepseek-v4-flash` など）
_CARD_NAME_RE = re.compile(r"^[A-Za-z0-9._:/@+-]{1,200}$")
# file_token 入力の値は「waggle:<token>」をカンマ区切りで（TUI の validate_card_spec と同じ形）
_CARD_TOKENS_RE = re.compile(
    r"^waggle:[A-Za-z0-9._:-]+(\s*,\s*waggle:[A-Za-z0-9._:-]+)*$"
)
# task_text の上限（100KB・UTF-8 のバイト数で数える・計画 15 §3.3）
_TASK_TEXT_MAX = 100 * 1024
# global_round / round は数字（空 = スキップ）
_CARD_ROUND_RE = re.compile(r"^\d{1,12}$")
# tag は frontmatter に 1 行で書くので改行を許さない
_CARD_TAG_RE = re.compile(r"^[^\r\n]{0,64}$")


def _project_summary(name: str, root: str) -> dict:
    """1 プロジェクトの最小集計（`sai-link phone summary`・実装計画 C C1）。

    - DB が無い・root が無い・失敗したときは `running=0` / `last_executed_at=None`
      を返す（一覧は止めない）。
    - 呼ぶ回数を抑えるため、同じプロジェクトは 5 秒だけ覚える。
    """
    empty = {"running": 0, "last_executed_at": None}
    now = time.monotonic()
    hit = _summary_cache.get(name)
    if hit is not None and now - hit[0] < _SUMMARY_TTL:
        return hit[1]
    # 失敗しても呼びすぎないよう，先に時刻を記録する
    _summary_cache[name] = (now, empty)
    if not root:
        return empty
    try:
        data = _phone_json(["summary"], root=root, timeout=10.0)
    except HTTPException as e:
        logger.warning(f"最小集計に失敗（一覧は続けます）: {name}: {e.detail}")
        return empty
    running = data.get("running")
    if not isinstance(running, int) or isinstance(running, bool) or running < 0:
        running = 0
    value = {"running": running, "last_executed_at": data.get("last_executed_at") or None}
    _summary_cache[name] = (now, value)
    return value


def list_projects() -> list[dict]:
    """登録簿の全プロジェクト（最後に動いた順・`last_executed_at` が無い物は最後）．

    - 一覧そのものは登録簿（`registry.load_registry`）から作る。
    - `running` と `last_executed_at` は DB が要るので `sai-link phone summary`
      で聞く（root と DB が両方あるプロジェクトだけ・実装計画 C C1）。
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
    raise HTTPException(status_code=404, detail="プロジェクトが見つかりません")


def export_runs(project: dict) -> None:
    """`sai-link phone export` で実行を書き出させる（メッセージ画面が開けるよう・06 §3.3）。

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
    last = _last_export.get(name)
    if last is not None and now - last < _EXPORT_INTERVAL:
        return
    # 失敗しても呼びすぎないよう，先に時刻を記録する
    _last_export[name] = now
    try:
        run_cli(
            [settings.sai_link_bin, "phone", "export"],
            app="sai-link",
            cwd=root,
            timeout=10,
        )
    except HTTPException as e:
        logger.warning(f"実行の書き出しに失敗（画面は続けます）: {name}: {e.detail}")


def dashboard(name: str, period: str) -> dict:
    """カード型 Dashboard（06 §3.3）．`recent` に description / linkable を足す。"""
    if period not in _PERIODS:
        raise HTTPException(
            status_code=400, detail="period は week / month / all のいずれかにしてください"
        )
    project = find_project(name)
    if not project.get("db_exists"):
        raise HTTPException(status_code=404, detail="このプロジェクトにはまだ実行の記録がありません")
    export_runs(project)
    data = _phone_json(["dashboard", "--period", period], root=project["root"])
    # recent の各行に description と linkable を足す（06 §3.3）。
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
    """常駐の起動・停止（06 §3.3・実装計画 C C1）．CLI は文字出力なので run_cli で呼ぶ。

    - プロジェクト名から登録簿を引く（無ければ 404）。root が無い場合も 404。
    - 実行の記録が無い（DB が無い）プロジェクトでも起動できる。
    """
    if action not in ("start", "stop"):
        raise HTTPException(status_code=400, detail="action は start / stop を指定してください")
    project = find_project(name)
    if not project.get("root_exists"):
        raise HTTPException(status_code=404, detail="プロジェクトのフォルダが見つかりません")
    root = project["root"]
    proc = run_cli(
        [settings.sai_link_bin, "daemon", action],
        app="sai-link",
        cwd=root,
        env=_phone_env(root),
        timeout=30.0,
    )
    if proc.returncode == 0:
        lines = [ln for ln in (proc.stdout or "").splitlines() if ln.strip()]
        fallback = "起動を要求しました" if action == "start" else "停止を要求しました"
        return {"ok": True, "message": lines[-1].strip() if lines else fallback}
    msg = (proc.stderr or proc.stdout or "").strip()
    logger.warning(f"常駐の操作に失敗: {name} {action}: {msg[:300]}")
    raise HTTPException(status_code=502, detail=(msg[:200] or "失敗しました"))


def _phone_env(root: str) -> dict:
    """`sai-link phone …` 用の環境変数。

    `SAI_PROJECT_ROOT` はデーモン起動時に設定された値が優先されるため，
    起動元の環境に引きずられず選んだプロジェクトの DB を開けるよう，
    ここで必ず登録簿の root に置き換える（sai-link `project::project_root`）。
    """
    env = dict(os.environ)
    env["SAI_PROJECT_ROOT"] = root
    return env


def _phone_json(
    args: list[str],
    *,
    root: str,
    timeout: float = 20.0,
    missing: str = _PHONE_MISSING,
    stdin_text: str | None = None,
):
    """`sai-link phone …`（cwd = プロジェクトの root）を呼んで JSON を返す（計画 12 §3.1）。

    - 終了コード 0: 標準出力の JSON をそのまま返す。
    - 終了コード 3 かつ stderr が `missing` と同じ: 404（`missing` は呼び出し側で
      文言を渡せる・md は「まだありません」、▶ 実行は「未実行のカードが見つかりません」）。
    - それ以外の失敗: 502 に stderr の先頭 200 文字（計画 12 §3.1）。
    - JSON として読めない: 502。
    - `stdin_text` は標準入力へ渡す（`card create` の JSON・計画 15 §1.5）。
    """
    proc = run_cli(
        [settings.sai_link_bin, "phone", *args],
        app="sai-link",
        cwd=root,
        env=_phone_env(root),
        timeout=timeout,
        input_text=stdin_text,
    )
    err = (proc.stderr or "").strip()
    if proc.returncode == 3 and err == missing:
        raise HTTPException(status_code=404, detail=missing)
    if proc.returncode != 0:
        logger.warning(f"sai-link phone が失敗: {' '.join(args)}: {err[:300]}")
        raise HTTPException(
            status_code=502, detail=(err or "sai-link が失敗しました")[:200]
        )
    try:
        return json.loads(proc.stdout or "")
    except (json.JSONDecodeError, TypeError):
        logger.warning(f"sai-link phone の応答を読めません: {' '.join(args)}")
        raise HTTPException(status_code=502, detail="sai-link の応答を読めませんでした")


def _phone_project(name: str) -> dict:
    """List 画面が呼ぶ CLI の前に登録簿を確かめる（計画 12 §3.1）。

    - 名前で登録簿から引き（無ければ 404）。
    - root / db が無い物は呼ばない（404）。
    """
    project = find_project(name)
    if not project.get("root_exists"):
        raise HTTPException(status_code=404, detail="プロジェクトのフォルダが見つかりません")
    if not project.get("db_exists"):
        raise HTTPException(status_code=404, detail="このプロジェクトにはまだ実行の記録がありません")
    return project


def _check_run_id(run_id: int) -> None:
    """run_id は正の整数だけ（外から来る値なので必ず確かめる・計画 12 §3.3）。"""
    if not isinstance(run_id, int) or isinstance(run_id, bool) or run_id <= 0:
        raise HTTPException(status_code=400, detail="run_id は 1 以上の整数にしてください")


def phone_tree(name: str) -> dict:
    """`sai-link phone tree`（左サイドバーと同じツリー・計画 12 §3.2）。"""
    project = _phone_project(name)
    return _phone_json(["tree"], root=project["root"])


def phone_rtree(name: str) -> dict:
    """`sai-link phone rtree`（実験の beside な木・U03 §変更内容 3）。

    実行の記録が要るため、root と DB が両方無いプロジェクトは 404（`_phone_project`）。
    返すのは C-02 の JSON（`{"nodes": [...], "edges": [...]}`）で、そのまま中継する。
    """
    project = _phone_project(name)
    return _phone_json(["rtree"], root=project["root"], timeout=30.0)


def phone_run_view(name: str, run_id: int, view: str) -> dict:
    """`sai-link phone run <id> --view <view>`（Log / Stats / History・計画 12 §3.2）。"""
    _check_run_id(run_id)
    if view not in _PHONE_VIEWS:
        raise HTTPException(
            status_code=400,
            detail="view は log / stats / history のいずれかにしてください",
        )
    project = _phone_project(name)
    return _phone_json(["run", str(run_id), "--view", view], root=project["root"])


def phone_md(name: str, run_id: int, kind: str) -> dict:
    """`sai-link phone md <id> --kind <kind>`（無いときは 404・計画 12 §3.2）。"""
    _check_run_id(run_id)
    if kind not in _PHONE_MD_KINDS:
        raise HTTPException(
            status_code=400,
            detail="kind は task / message のいずれかにしてください",
        )
    project = _phone_project(name)
    return _phone_json(["md", str(run_id), "--kind", kind], root=project["root"])


def phone_md_save(name: str, run_id: int, kind: str, text) -> dict:
    """`sai-link phone md <id> --kind <kind> --write`（本文は標準入力・保存後の本文を返す）。

    - 未実行のカードでない task（実行済み・送信済み）は 404「未実行のカードが見つかりません」。
    - frontmatter を壊す内容などその他の失敗は 502 に stderr の先頭 200 文字。
    """
    _check_run_id(run_id)
    if kind not in _PHONE_MD_WRITABLE:
        raise HTTPException(status_code=400, detail="保存できるのは task だけです")
    if not isinstance(text, str) or len(text.encode("utf-8")) > _TASK_TEXT_MAX:
        raise HTTPException(status_code=400, detail="本文は 100KB までの文字にしてください")
    project = _phone_project(name)
    return _phone_json(
        ["md", str(run_id), "--kind", kind, "--write"],
        root=project["root"],
        missing=_PHONE_MISSING_START,
        stdin_text=text,
    )


def phone_waggle(name: str, run_id: int, token: str | None = None) -> dict:
    """`sai-link phone waggle <id> [--token <t>]`（成果物の一覧と本文・計画 12 §3.2）。"""
    _check_run_id(run_id)
    args = ["waggle", str(run_id)]
    if token is not None:
        if not _PHONE_TOKEN_RE.match(token or ""):
            raise HTTPException(status_code=400, detail="token の指定が正しくありません")
        args += ["--token", token]
    project = _phone_project(name)
    return _phone_json(args, root=project["root"])


def phone_card_options(name: str) -> dict:
    """`sai-link phone card options`（ウィザードの選択肢・計画 15 §1.5）。"""
    project = _phone_project(name)
    return _phone_json(["card", "options"], root=project["root"])


def phone_card_inputs(name: str, block: str) -> dict:
    """`sai-link phone card inputs --block <id>`（手順 4 の入力欄と候補・計画 15 §1.5）。"""
    if not _PHONE_TOKEN_RE.match(block or ""):
        raise HTTPException(status_code=400, detail="block の指定が正しくありません")
    project = _phone_project(name)
    return _phone_json(
        ["card", "inputs", "--block", block], root=project["root"]
    )


def _validate_card_payload(payload: dict) -> dict:
    """作成フォームが送る値を CLI を呼ぶ前に確かめる（計画 15 §3.3）。

    - `block` / 入力のキーは英数字と `-_.:` のみ
    - `task_text` は 100KB（UTF-8 のバイト数）まで
    - `runner` / `thinking` は一覧の値
    - file_token 入力の値は `waggle:` 形式（カンマ区切り）
    返すのは形をそろえた dict（送る JSON そのもの）。
    """
    if not isinstance(payload, dict):
        raise HTTPException(status_code=400, detail="送る値が正しくありません")

    def text(key: str) -> str:
        v = payload.get(key, "")
        if v is None:
            return ""
        if not isinstance(v, str):
            raise HTTPException(
                status_code=400, detail=f"{key} は文字列で送ってください"
            )
        return v

    # done_ids: 1 以上の整数の配列
    done = payload.get("done_ids", [])
    if done is None:
        done = []
    if not isinstance(done, list) or len(done) > 64:
        raise HTTPException(
            status_code=400, detail="done_ids は整数の配列で送ってください"
        )
    done_ids: list[int] = []
    for v in done:
        if not isinstance(v, int) or isinstance(v, bool) or v <= 0:
            raise HTTPException(
                status_code=400, detail="done_ids は 1 以上の整数で送ってください"
            )
        done_ids.append(v)

    runner = text("runner")
    if runner and runner not in _CARD_RUNNERS:
        raise HTTPException(
            status_code=400,
            detail="runner は user / pi / cc / gl のいずれか（空でスキップ）",
        )

    block = text("block")
    if block and not _PHONE_TOKEN_RE.match(block):
        raise HTTPException(status_code=400, detail="block の指定が正しくありません")

    thinking = text("thinking")
    if thinking and thinking not in _CARD_THINKING:
        raise HTTPException(
            status_code=400,
            detail="thinking は none / low / medium / high / xhigh / max のいずれか（空でスキップ）",
        )

    for key in ("provider", "model"):
        v = text(key)
        if v and not _CARD_NAME_RE.match(v):
            raise HTTPException(
                status_code=400, detail=f"{key} の指定が正しくありません"
            )

    for key in ("global_round", "round"):
        v = text(key)
        if v and not _CARD_ROUND_RE.match(v):
            raise HTTPException(
                status_code=400, detail=f"{key} は数字にしてください（空でスキップ）"
            )

    tag = text("tag")
    if tag and not _CARD_TAG_RE.match(tag):
        raise HTTPException(
            status_code=400, detail="tag は 64 文字までの 1 行にしてください"
        )

    # file_token 入力（`{"plan_doc": "waggle:abc, waggle:def"}`）
    raw_inputs = payload.get("inputs", {})
    if raw_inputs is None:
        raw_inputs = {}
    if not isinstance(raw_inputs, dict) or len(raw_inputs) > 64:
        raise HTTPException(
            status_code=400, detail="inputs は対の形で送ってください"
        )
    inputs: dict[str, str] = {}
    for key, value in raw_inputs.items():
        if not isinstance(key, str) or not _PHONE_TOKEN_RE.match(key):
            raise HTTPException(
                status_code=400, detail="inputs の名前が正しくありません"
            )
        if not isinstance(value, str) or not _CARD_TOKENS_RE.match(value):
            raise HTTPException(
                status_code=400, detail="トークンは waggle: 形式で指定してください"
            )
        inputs[key] = value

    task_text = text("task_text")
    if len(task_text.encode("utf-8")) > _TASK_TEXT_MAX:
        raise HTTPException(status_code=400, detail="task_text は 100KB までです")

    return {
        "done_ids": done_ids,
        "runner": runner,
        "block": block,
        "inputs": inputs,
        "provider": text("provider"),
        "model": text("model"),
        "thinking": thinking,
        "global_round": text("global_round"),
        "round": text("round"),
        "tag": tag,
        "task_text": task_text,
    }


def phone_card_create(name: str, payload: dict) -> dict:
    """`sai-link phone card create`（stdin に JSON・作成後は todo に配置・計画 15 §1.5）。"""
    data = _validate_card_payload(payload)
    project = _phone_project(name)
    return _phone_json(
        ["card", "create"],
        root=project["root"],
        stdin_text=json.dumps(data, ensure_ascii=False),
    )


def phone_card_start(name: str, run_id: int) -> dict:
    """`sai-link phone card run <run_id>`（新形式 todo の ▶ 実行・計画 15）。

    - 成功: `{'run_id': N, 'message': '実行を要求しました'}` をそのまま返す。
    - 未実行カードが無い（終了コード 3・stderr「未実行のカードが見つかりません」）: 404。
    - 常駐停止などその他: 502 に stderr の先頭 200 文字。
    """
    _check_run_id(run_id)
    project = _phone_project(name)
    return _phone_json(
        ["card", "run", str(run_id)],
        root=project["root"],
        missing=_PHONE_MISSING_START,
    )


# --- 読む札（利用者の札）を Web から始める・終える（C-12・U18） ---
# 読む札は研究方針の木が作る `runner: user` のカード（sai-paper `phone sci card`）。
# この 2 つの関数は root を直接取るので、別のプロジェクト（論文の置き場とは別の root）を
# 指せる。検査は `_phone_json` の決まりに従う（終了コード 3 は 404、他は 502）。


def phone_user_start(root: str, run_id: int) -> dict:
    """読む札を始める（`sai-moon-link phone user start <run_id>`・C-12）。"""
    _check_run_id(run_id)
    return _phone_json(
        ["user", "start", str(run_id)],
        root=root,
        missing=_PHONE_MISSING_START,
    )


def phone_user_finish(root: str, run_id: int) -> dict:
    """読む札を終える（`sai-moon-link phone user finish <run_id>`・C-12）。"""
    _check_run_id(run_id)
    return _phone_json(
        ["user", "finish", str(run_id)],
        root=root,
        missing=_PHONE_MISSING_RUN,
    )


def phone_finalize(name: str, run_id: int) -> dict:
    """`sai-link phone finalize <run_id>`（メッセージ画面の ■ 終了・計画書 A の D2）。

    - 成功: `{'run_id': N, 'message': '終了を要求しました'}` をそのまま返す。
    - その run が無い（終了コード 3・stderr「実行が見つかりません」）: 404。
    - 実行中でない・子ブロック・常駐停止など: 502 に stderr の先頭 200 文字。
    """
    _check_run_id(run_id)
    project = _phone_project(name)
    return _phone_json(
        ["finalize", str(run_id)],
        root=project["root"],
        missing=_PHONE_MISSING_RUN,
    )


def phone_advance(name: str, run_id: int) -> dict:
    """`sai-link phone advance <run_id>`（review → done → Workflow の移動・計画書 A の U10）。

    - 成功: `{'run_id': N, 'from': '…', 'to': '…'}` をそのまま返す。
    - その run が無い（終了コード 3・stderr「実行が見つかりません」）: 404。
    - review / done 以外・run フォルダが無いとき: 502 に stderr の先頭 200 文字。
    """
    _check_run_id(run_id)
    project = _phone_project(name)
    return _phone_json(
        ["advance", str(run_id)],
        root=project["root"],
        missing=_PHONE_MISSING_RUN,
    )


def phone_rm(name: str, run_id: int) -> dict:
    """`sai-moon-link phone rm <run_id>`（List の項目を消す。子の実行も一緒に消える）。

    - 成功: `{'run_id': N, 'executions': 消した実行の数}`。
    - その run が無い（終了コード 3）: 404。実行中など: 502 に stderr。
    """
    _check_run_id(run_id)
    project = _phone_project(name)
    return _phone_json(
        ["rm", str(run_id)],
        root=project["root"],
        missing=_PHONE_MISSING_RUN,
    )


def phone_card_docs(name: str) -> dict:
    """`sai-moon-link phone card docs`（最近の文書・task_text に貼る候補）。"""
    project = _phone_project(name)
    return _phone_json(["card", "docs"], root=project["root"])


# 実行予定時刻（`YYYY-MM-DD HH:MM`。ブラウザの datetime-local の `T` 区切りも受ける）
_SCHEDULE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}$")


def phone_card_schedule(name: str, run_id: int, at: str) -> dict:
    """`sai-moon-link phone card schedule <run_id> [--at …]`（todo の実行予定時刻）。

    `at` が空なら予定を消す。形が違えば CLI を呼ばずに 400。
    """
    _check_run_id(run_id)
    at = (at or "").strip()
    args = ["card", "schedule", str(run_id)]
    if at:
        if not _SCHEDULE_RE.match(at):
            raise HTTPException(status_code=400, detail="時刻の形が正しくありません")
        args += ["--at", at.replace("T", " ")]
    project = _phone_project(name)
    return _phone_json(args, root=project["root"], missing=_PHONE_MISSING_START)


# --- Workflow と定義の編集（U3・共有契約 C-A1） ---
# sai-link-tui の中にしか無い Workflow の盤面と block / template / settings の編集を、
# Web からも同じ CLI 経由でできるようにする（sai-phone は DB を開かない・00 §2）。
# 外から来る値（kind / scope / 名前 / 本文）は、CLI を呼ぶ前に必ずここで確かめる。

# 編集できる定義の種類（`phone def --kind`）
_DEF_KINDS = ("block", "template", "settings")
# 定義の置き場（そのプロジェクト / 全体で共通の `~/.sai/`）
_DEF_SCOPES = ("project", "global")
# block / template の名前（英数字から始まり、英数字と - _ ・64 文字まで・拡張子 .md は付けない）
_DEF_NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$")
# settings の名前は `settings` だけ
_DEF_SETTINGS_NAME = "settings"
# 定義の本文の上限（256KB・UTF-8 のバイト数で数える）
_DEF_TEXT_MAX = 256 * 1024
# `write --create` で既に同じ名前があるときの stderr（終了コード 1・409 にする）
_DEF_DUPLICATE = "同じ名前の定義があります"


def _phone_def_project(name: str) -> dict:
    """定義の編集の前に登録簿を確かめる（U3 §変更内容 1）。

    実行の記録（DB）が無くても block は書けるので、root が無いときだけ 404 にする。
    """
    project = find_project(name)
    if not project.get("root_exists"):
        raise HTTPException(status_code=404, detail="プロジェクトのフォルダが見つかりません")
    return project


def _phone_def_json(args: list[str], *, root: str, stdin_text: str | None = None):
    """`sai-link phone def …`（cwd = プロジェクトの root）を呼んで JSON を返す（U3 §変更内容 1）。

    - 終了コード 0: 標準出力の JSON をそのまま返す（読めなければ 502）。
    - 終了コード 2: 400（検査に通らない中身の誤りなので、stderr の理由をそのまま見せる）。
    - 終了コード 3: 404（stderr の文言）。
    - 終了コード 1 かつ stderr が「同じ名前の定義があります」: 409（その文）。
    - そのほか: 502 に stderr の先頭 200 文字（無ければ「sai-link が失敗しました」）。
    - `stdin_text` は本文を標準入力へ渡す（`write`）。
    """
    proc = run_cli(
        [settings.sai_link_bin, "phone", "def", *args],
        app="sai-link",
        cwd=root,
        env=_phone_env(root),
        timeout=20.0,
        input_text=stdin_text,
    )
    err = (proc.stderr or "").strip()
    if proc.returncode == 3:
        raise HTTPException(status_code=404, detail=err or "定義が見つかりません")
    if proc.returncode == 2:
        # 利用者が書いた中身の誤り（壊れた frontmatter・settings が JSON オブジェクトでない等）
        logger.warning(f"sai-link phone def の検査エラー: {' '.join(args)}: {err[:300]}")
        raise HTTPException(status_code=400, detail=(err or "中身が正しくありません")[:300])
    if proc.returncode == 1 and err == _DEF_DUPLICATE:
        raise HTTPException(status_code=409, detail=_DEF_DUPLICATE)
    if proc.returncode != 0:
        logger.warning(f"sai-link phone def が失敗: {' '.join(args)}: {err[:300]}")
        raise HTTPException(
            status_code=502, detail=(err[:200] or "sai-link が失敗しました")
        )
    try:
        return json.loads(proc.stdout or "")
    except (json.JSONDecodeError, TypeError):
        logger.warning(f"sai-link phone def の応答を読めません: {' '.join(args)}")
        raise HTTPException(status_code=502, detail="sai-link の応答を読めませんでした")


def _validate_def(kind, scope, def_name) -> None:
    """定義の指定（kind / scope / 名前）を確かめる（違反は 400・U3 §変更内容 1）。"""
    if kind not in _DEF_KINDS:
        raise HTTPException(
            status_code=400, detail="kind は block / template / settings のいずれかです"
        )
    if scope not in _DEF_SCOPES:
        raise HTTPException(
            status_code=400, detail="scope は project / global のいずれかです"
        )
    if kind == "settings":
        if def_name != _DEF_SETTINGS_NAME:
            raise HTTPException(
                status_code=400, detail="settings の名前は settings だけです"
            )
        return
    if not isinstance(def_name, str) or not _DEF_NAME_RE.match(def_name):
        raise HTTPException(
            status_code=400, detail="名前は英数字と - _ の 64 文字までにしてください"
        )


def _validate_def_text(text) -> str:
    """定義の本文を確かめる（文字列で 256KB まで・違反は 400）。"""
    if not isinstance(text, str):
        raise HTTPException(status_code=400, detail="本文は文字列で送ってください")
    if len(text.encode("utf-8")) > _DEF_TEXT_MAX:
        raise HTTPException(status_code=400, detail="本文は 256KB までの文字にしてください")
    return text


def phone_workflow(name: str) -> dict:
    """Workflow 盤面（`sai-link phone workflow`・U3 §変更内容 1）。

    実行の記録が要るため、root と DB が両方無いプロジェクトは 404（`_phone_project`）。
    """
    project = _phone_project(name)
    return _phone_json(["workflow"], root=project["root"], timeout=30.0)


def phone_def_list(name: str) -> dict:
    """定義の一覧（`phone def list`・U3 §変更内容 1）。"""
    project = _phone_def_project(name)
    return _phone_def_json(["list"], root=project["root"])


def phone_def_show(name: str, kind: str, scope: str, def_name: str) -> dict:
    """定義 1 件の中身（`phone def show`・U3 §変更内容 1）。"""
    _validate_def(kind, scope, def_name)
    project = _phone_def_project(name)
    return _phone_def_json(
        ["show", "--kind", kind, "--scope", scope, "--name", def_name],
        root=project["root"],
    )


def phone_def_write(
    name: str, kind: str, scope: str, def_name: str, text, create: bool = False
) -> dict:
    """定義の保存（`phone def write`・本文は標準入力・U3 §変更内容 1）。

    - 検査に通らない本文（壊れた frontmatter 等）は 400（sai-link が終了コード 2 で返す）。
    - 無い定義は 404、`--create` で同じ名前があるときは 409。
    - 検査は必ず CLI を呼ぶ前に行う（呼び出し 0 回）。
    """
    _validate_def(kind, scope, def_name)
    body = _validate_def_text(text)
    if create is not None and not isinstance(create, bool):
        raise HTTPException(status_code=400, detail="create は真偽値で送ってください")
    args = ["write", "--kind", kind, "--scope", scope, "--name", def_name]
    if create:
        args.append("--create")
    project = _phone_def_project(name)
    return _phone_def_json(args, root=project["root"], stdin_text=body)


def phone_def_delete(name: str, kind: str, scope: str, def_name: str) -> dict:
    """定義の削除（`phone def delete`・U3 §変更内容 1）。settings は消せない（400）。"""
    _validate_def(kind, scope, def_name)
    if kind == "settings":
        raise HTTPException(status_code=400, detail="settings は消せません")
    project = _phone_def_project(name)
    return _phone_def_json(
        ["delete", "--kind", kind, "--scope", scope, "--name", def_name],
        root=project["root"],
    )

