"""置き場と名前の検査（共有契約 C1）。読み書きは <project>/ts/ の中だけ。"""

from __future__ import annotations

import os
import re
from pathlib import Path

NAME_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,63}")


class ProjectNotFound(Exception):
    """プロジェクトが見つからない。サーバも CLI も動かさない。"""


def project_root() -> Path:
    # 呼ぶたびに環境変数を読む（テストで差し替えられるようキャッシュしない）
    env = os.environ.get("SAI_PROJECT_ROOT", "")
    if env:
        return Path(env).resolve()
    home = Path.home().resolve()
    cur = Path.cwd().resolve()
    for d in [cur, *cur.parents]:
        # ホームとそれより上は数えない
        if d == home or home.is_relative_to(d):
            break
        if (d / ".sai").is_dir():
            return d
    raise ProjectNotFound("プロジェクトが見つかりません（SAI_PROJECT_ROOT を設定してください）")


def ts_root() -> Path:
    return project_root() / "ts"


def datasets_dir() -> Path:
    return ts_root() / "datasets"


def ext_dir() -> Path:
    return ts_root() / "ext"


def scripts_dir() -> Path:
    return ts_root() / "scripts"


def view_path() -> Path:
    return ts_root() / "view.json"


def valid_name(name: str) -> bool:
    # fullmatch を使う（$ は末尾の改行を許すため）
    return isinstance(name, str) and ".." not in name and NAME_RE.fullmatch(name) is not None


def check_name(name: str) -> str:
    if not valid_name(name):
        raise ValueError(f"名前の形が違います: {name}")
    return name


def inside(path: Path) -> Path:
    p = path.resolve()
    if not p.is_relative_to(ts_root().resolve()):
        raise ValueError(f"sai-ts の置き場の外です: {path}")
    return p


def dataset_dir(ds: str) -> Path:
    check_name(ds)
    p = inside(datasets_dir() / ds)
    if not p.is_dir():
        raise KeyError(f"データセットがありません: {ds}")
    return p


def record_dir(ds: str, rec: str) -> Path:
    check_name(rec)
    p = inside(dataset_dir(ds) / "records" / rec)
    if not p.is_dir():
        raise KeyError(f"記録がありません: {rec}")
    return p
