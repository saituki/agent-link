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
import time
from contextvars import ContextVar
from datetime import datetime, timezone
from pathlib import Path

logger = logging.getLogger("sai_web")

# 実行の DB と常駐の PID ファイル（sai-link の `project::sai_path` と同じ場所）
_DB_REL = Path(".sai") / "sai-engine" / "sai-engine.db"
_PID_REL = Path(".sai") / "sai-engine" / "daemon.pid"
# 常駐が約 5 秒ごとに書き換える状態ファイル（多人数モードの生死判定に使う）
_STATUS_REL = Path(".sai") / "sai-engine" / "daemon.status"
# 多人数モードで「生きている」と見なす更新からの秒数（daemon の書き込み間隔に余裕を持たせる）
_STATUS_FRESH_SEC = 30.0

# 登録簿のファイル名（`$SAI_HOME/registry.json`）
_REGISTRY_NAME = "registry.json"

# 多人数モードの合図（`SAI_WEB_USERS_ROOT`・共有契約 C4）。
# 空でない絶対パスなら多人数モード。無ければ今までの単独モード。
_USERS_ROOT_ENV = "SAI_WEB_USERS_ROOT"

# 今の利用者（多人数モードでミドルウェアが入れる・共有契約 C5）。
# 値は `{"name": "saito", "email": "saito@…", "is_admin": False}`。
# 単独モードと、多人数モードでもまだログインしていないときは None。
current_user: ContextVar[dict | None] = ContextVar("sai_current_user", default=None)


def users_root() -> Path | None:
    """`SAI_WEB_USERS_ROOT`（多人数モードの置き場）。無ければ None。

    空文字と、絶対パスでない値は「無い」として扱う（サーバを単独モードで動かす）。
    """
    value = (os.getenv(_USERS_ROOT_ENV) or "").strip()
    if not value:
        return None
    path = Path(value)
    if not path.is_absolute():
        logger.warning(f"{_USERS_ROOT_ENV} が絶対パスではありません（単独モードにします）")
        return None
    return path


def multi_user() -> bool:
    """多人数モードか（`SAI_WEB_USERS_ROOT` があるか）。"""
    return users_root() is not None


def _user_name(name: str | None = None) -> str | None:
    """今の利用者の名前（引数があればそれ）。無ければ None。"""
    if name:
        return name
    user = current_user.get()
    if isinstance(user, dict):
        got = user.get("name")
        if isinstance(got, str) and got:
            return got
    return None


def user_dir(name: str | None = None) -> Path:
    """`<SAI_WEB_USERS_ROOT>/<名前>`（その人の置き場の根・共有契約 C4）。"""
    root = users_root()
    if root is None:
        raise RuntimeError("多人数モードではありません")
    got = _user_name(name)
    if got is None:
        raise RuntimeError("今の利用者がいません")
    return root / got


def user_home(name: str | None = None) -> Path:
    """その人のひな形の HOME。多人数: `<user_dir>/home`。単独: `Path.home()`。"""
    if not multi_user():
        return Path.home()
    return user_dir(name) / "home"


def project_home(project: str) -> Path:
    """プロジェクトのコンテナの HOME。多人数: `<user_dir>/homes/<project>`。単独: `Path.home()`。"""
    if not multi_user():
        return Path.home()
    return user_dir() / "homes" / project


def host_transcript(path: str) -> Path:
    """会話の記録の場所を、Web から読める場所に読み替える。

    多人数モードのコンテナは `<user_dir>/claude/` を `<home>/.claude` に mount するので、
    `<U>/<user>/homes/<project>/.claude/…` の記録の実物は `<U>/<user>/claude/…` にある。
    元の場所にファイルがあれば（mount の前の記録）そのまま使う。
    """
    original = Path(path)
    root = users_root()
    if root is None or original.exists():
        return original
    try:
        parts = original.relative_to(root).parts
    except ValueError:
        return original
    if len(parts) > 4 and parts[1] == "homes" and parts[3] == ".claude":
        return root / parts[0] / "claude" / Path(*parts[4:])
    return original


def sai_home() -> Path:
    """`~/.sai` の置き場（`SAI_HOME` で上書きできる）。

    多人数モードで今の利用者がいる場合は、その人の `user_home()/.sai` を返す。
    それ以外（単独モード・多人数モードでもログインしていないとき）は今までどおり
    `SAI_HOME` → `~/.sai`。Web 自身の DB・通知の鍵はこちらに置く。
    """
    if multi_user() and current_user.get() is not None:
        return user_home() / ".sai"
    return Path(os.getenv("SAI_HOME") or (Path.home() / ".sai"))


def within_user(path) -> bool:
    """そのパスが今の利用者の置き場の中か。単独モードは常に真。"""
    if not multi_user():
        return True
    got = _user_name()
    if got is None:
        return False
    try:
        target = Path(path).resolve()
        base = user_dir(got).resolve()
    except (OSError, RuntimeError):
        return False
    return target == base or base in target.parents


def user_key() -> str:
    """覚え（キャッシュ）のキーの頭に使う今の利用者の名前（単独モードは空文字）。"""
    if not multi_user():
        return ""
    return _user_name() or ""


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


def daemon_status_path(root: str) -> Path:
    """常駐の状態ファイルのパス（多人数モードの生死判定に使う）。"""
    return Path(root) / _STATUS_REL


def daemon_alive(root: str) -> bool:
    """常駐が生きているか。

    - 単独モード: `<root>/.sai/sai-engine/daemon.pid` の PID が生きているか。
    - 多人数モード: コンテナの中の PID はホストから見えないので、常駐が約 5 秒ごとに
      書く `<root>/.sai/sai-engine/daemon.status` の更新時刻が 30 秒以内かで見る。

    sai-link の `daemon::is_running()` は PID に加えてプロセス名まで確かめるが，
    ここでは軽量の生死判定だけを行う。sai-link とずれる場合は報告に残すこと
    （実装計画 C の追加指示）。
    """
    if multi_user():
        try:
            mtime = daemon_status_path(root).stat().st_mtime
        except OSError:
            return False
        return (time.time() - mtime) <= _STATUS_FRESH_SEC
    try:
        text = daemon_pid_path(root).read_text(encoding="utf-8").strip()
    except (OSError, ValueError):
        return False
    try:
        pid = int(text)
    except ValueError:
        return False
    return process_alive(pid)


def _load_user_projects() -> list[dict]:
    """多人数モードのプロジェクト一覧（フォルダを数える・共有契約 C4）。

    `<user_dir>/projects/` の直下のフォルダのうち、名前が `<今の利用者>-` で
    始まり `.sai/` を持つ物を返す。`last_opened` は `.sai/` の更新時刻。
    登録簿（`registry.json`）は読まない（プロジェクトは Web だけが作るため）。
    """
    name = _user_name()
    if name is None:
        return []
    projects_dir = user_dir(name) / "projects"
    try:
        entries = [p for p in projects_dir.iterdir() if not p.name.startswith(".")]
    except OSError:
        return []
    prefix = f"{name}-"
    projects: list[dict] = []
    for entry in entries:
        if not entry.is_dir():
            continue
        if not entry.name.startswith(prefix):
            continue
        if not (entry / ".sai").is_dir():
            continue
        try:
            mtime = (entry / ".sai").stat().st_mtime
        except OSError:
            mtime = 0.0
        root = str(entry)
        projects.append(
            {
                "name": entry.name,
                "root": root,
                "kind": "link",
                "last_opened": datetime.fromtimestamp(mtime, timezone.utc).isoformat(),
                "root_exists": True,
                "db_exists": engine_db_path(root).is_file(),
                "daemon_alive": daemon_alive(root),
            }
        )
    return sorted(projects, key=lambda p: p["last_opened"], reverse=True)


def load_registry(path: Path | None = None) -> list[dict]:
    """登録簿の `kind == "link"` の項目を（最後に開いた順に）返す。

    - 多人数モード（`SAI_WEB_USERS_ROOT` があり、今の利用者がいる）では
      登録簿を読まず、`<user_dir>/projects/` のフォルダを数えて返す。
    - ファイルが無い / 読めない / JSON が壊れている場合は空のリスト
      （ログに 1 行だけ出す。Project タブは空の一覧を出す）。
    - 並びは `last_opened` の降順（登録簿の並びの決まり）。
      `last_opened` が無い項目は後ろ。
    """
    if path is None and multi_user():
        return _load_user_projects()
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
