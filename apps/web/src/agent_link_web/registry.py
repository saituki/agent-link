"""登録簿 `~/.agent-link/registry.json` の読み込み（Project タブ）。

agent-link-web は他アプリの DB を直接開かないため，ここもファイルと
プロセスだけを見る。DB の中（実行中件数・最終実行日時）は `agent-link project
summary` に聞く（`control.list_projects`）。

読み込むもの:
- `~/.agent-link/registry.json`（配列）。`kind == "link"` の項目だけ返す
  （横断対象は `link` のみという決まり。2026-09-26 までは別のアプリも同じ絞り込みだった）
- 各項目に次を足す
  - `root_exists`: root のフォルダがあるか
  - `db_exists`  : `<root>/.agent-link/engine/agent-link.db` があるか
  - `daemon_alive`: `<root>/.agent-link/engine/daemon.pid` の PID が生きているか

`agent-link` との取り決め（変えない）:
- DB と PID ファイルの位置は agent-link `project::sai_path` と同じ場所
  （`.agent-link/engine/agent-link.db` / `.agent-link/engine/daemon.pid`）
- 登録簿の位置は `~/.agent-link/registry.json`（agent-link の登録簿と同じ）

`AGENT_LINK_HOME` を設定すれば登録簿の置き場（`$AGENT_LINK_HOME/registry.json`）を
一時フォルダに変えられる（テストと手元確認のため）。
"""

from __future__ import annotations

import json
import logging
import os
from pathlib import Path

logger = logging.getLogger("agent_link_web")

# 実行の DB と常駐の PID ファイル（agent-link の `project::sai_path` と同じ場所）
_DB_REL = Path(".agent-link") / "engine" / "agent-link.db"
_PID_REL = Path(".agent-link") / "engine" / "daemon.pid"
# 登録簿のファイル名（`$AGENT_LINK_HOME/registry.json`）
_REGISTRY_NAME = "registry.json"


def sai_home() -> Path:
    """`~/.agent-link` の置き場（`AGENT_LINK_HOME` → `~/.agent-link`）。Web 自身の DB・通知の鍵もここに置く。"""
    return Path(os.getenv("AGENT_LINK_HOME") or (Path.home() / ".agent-link"))


def registry_path() -> Path:
    """登録簿 `~/.agent-link/registry.json` のパス。"""
    return sai_home() / _REGISTRY_NAME


def engine_db_path(root: str) -> Path:
    """実行の DB のパス（agent-link `project::sai_path` と同じ）。"""
    return Path(root) / _DB_REL


def daemon_pid_path(root: str) -> Path:
    """常駐の PID ファイルのパス（agent-link `daemon::pid_file_path` と同じ）。"""
    return Path(root) / _PID_REL


def process_alive(pid: int) -> bool:
    """PID のプロセスが生きているか（Unix の kill(pid, 0)・`ps` の代用）。"""
    if pid <= 0:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        # 他人のプロセスだが存在はする
        return True
    except OSError:
        return False
    return True



def daemon_alive(root: str) -> bool:
    """常駐が生きているか（`<root>/.agent-link/engine/daemon.pid` の PID が生きているか）。

    agent-link の `daemon::is_running()` は PID に加えてプロセス名まで確かめるが，
    ここでは軽量の生死判定だけを行う。agent-link とずれる場合は報告に残すこと。
    """
    try:
        text = daemon_pid_path(root).read_text(encoding="utf-8").strip()
    except (OSError, ValueError):
        return False
    try:
        pid = int(text)
    except ValueError:
        return False
    return process_alive(pid)



def load_registry(path: Path | None = None) -> list[dict]:
    """登録簿の `kind == "link"` の項目を（最後に開いた順に）返す。

    - ファイルが無い / 読めない / JSON が壊れている場合は空のリスト
      （ログに 1 行だけ出す。Project タブは空の一覧を出す）。
    - 並びは `last_opened` の降順（登録簿の並びの決まり）。
      `last_opened` が無い項目は後ろ。
    """
    target = Path(path) if path is not None else registry_path()
    try:
        raw = target.read_text(encoding="utf-8")
    except FileNotFoundError:
        return []
    except OSError as e:
        logger.warning(f"登録簿を読めません: {target}: {e}")
        return []
    try:
        rows = json.loads(raw)
    except (json.JSONDecodeError, TypeError):
        logger.warning(f"登録簿の JSON が読めません: {target}")
        return []
    if not isinstance(rows, list):
        logger.warning(f"登録簿の形が違います（配列でない）: {target}")
        return []

    projects: list[dict] = []
    for row in rows:
        if not isinstance(row, dict):
            continue
        if (row.get("kind") or "") != "link":
            continue
        root = str(row.get("root") or "")
        projects.append(
            {
                "name": str(row.get("name") or ""),
                "root": root,
                "kind": str(row.get("kind") or ""),
                "last_opened": row.get("last_opened") or "",
                "root_exists": Path(root).is_dir() if root else False,
                "db_exists": engine_db_path(root).is_file() if root else False,
                "daemon_alive": daemon_alive(root) if root else False,
            }
        )
    return sorted(projects, key=lambda p: p["last_opened"], reverse=True)
