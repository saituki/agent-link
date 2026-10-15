"""登録簿 `~/.sai/registry.json` の読み込み（Project タブ・実装計画 C C1）。

sai-web は他アプリの DB を直接開かない（00 §2）ため，ここもファイルと
プロセスだけを見る。DB の中（実行中件数・最終実行日時）は `sai-link phone
summary` に聞く（`control.list_projects`）。

読み込むもの:
- `~/.sai/registry.json`（配列）。`kind == "link"` の項目だけ返す
  （横断対象は `link` のみという決まり。2026-09-26 までは別のアプリも同じ絞り込みだった）
- 各項目に次を足す
  - `root_exists`: root のフォルダがあるか
  - `db_exists`  : `<root>/.sai/sai-engine/sai-engine.db` があるか
  - `daemon_alive`: `<root>/.sai/sai-engine/daemon.pid` の PID が生きているか

`sai-link` との共有契約（変えない）:
- DB と PID ファイルの位置は sai-link `project::sai_path` と同じ場所
  （`.sai/sai-engine/sai-engine.db` / `.sai/sai-engine/daemon.pid`）
- 登録簿の位置は `~/.sai/registry.json`（sai-link `phone_export.rs:237` と同じ）

`SAI_HOME` を設定すれば登録簿の置き場（`$SAI_HOME/registry.json`）を
一時フォルダに変えられる（テストと手元確認のため）。
"""

from __future__ import annotations

import json
import logging
import os
from pathlib import Path

logger = logging.getLogger("sai_web")

# 実行の DB と常駐の PID ファイル（sai-link の `project::sai_path` と同じ場所）
_DB_REL = Path(".sai") / "sai-engine" / "sai-engine.db"
_PID_REL = Path(".sai") / "sai-engine" / "daemon.pid"
# 登録簿のファイル名（`$SAI_HOME/registry.json`）
_REGISTRY_NAME = "registry.json"


def sai_home() -> Path:
    """`~/.sai` の置き場（`SAI_HOME` → `~/.sai`）。Web 自身の DB・通知の鍵もここに置く。"""
    return Path(os.getenv("SAI_HOME") or (Path.home() / ".sai"))


def registry_path() -> Path:
    """登録簿 `~/.sai/registry.json` のパス。"""
    return sai_home() / _REGISTRY_NAME


def engine_db_path(root: str) -> Path:
    """実行の DB のパス（sai-link `project::sai_path` と同じ）。"""
    return Path(root) / _DB_REL


def daemon_pid_path(root: str) -> Path:
    """常駐の PID ファイルのパス（sai-link `daemon::pid_file_path` と同じ）。"""
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
    """常駐が生きているか（`<root>/.sai/sai-engine/daemon.pid` の PID が生きているか）。

    sai-link の `daemon::is_running()` は PID に加えてプロセス名まで確かめるが，
    ここでは軽量の生死判定だけを行う。sai-link とずれる場合は報告に残すこと
    （実装計画 C の追加指示）。
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
