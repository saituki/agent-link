"""拡張（C8・C10）: ts/ext/<name>/ の一覧・版・配信・server.py の呼び出し。"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

from . import paths, watch

MANIFEST = "manifest.json"
CALL_TIMEOUT = 60
ASSET_TYPES = {
    ".js": "text/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".json": "application/json; charset=utf-8",
}
SRC_DIR = Path(__file__).resolve().parent.parent  # apps/ts/src（子の PYTHONPATH）


class ExtError(Exception):
    def __init__(self, status: int, detail: str):
        super().__init__(detail)
        self.status = status
        self.detail = detail


def _join(base: Path, rel: str) -> Path:
    # base の中に収まる場合だけ返す（.. で外へ出ない）
    p = (base / rel).resolve()
    if not p.is_relative_to(base.resolve()):
        raise ValueError(f"拡張の外です: {rel}")
    return paths.inside(p)


def _read(d: Path) -> dict:
    try:
        m = json.loads((d / MANIFEST).read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise ValueError("manifest.json がありません")
    except ValueError:
        raise ValueError("manifest.json が読めません")
    if not isinstance(m, dict):
        raise ValueError("manifest.json は対象にしてください")
    return m


def _describe(d: Path) -> dict:
    out = {"name": d.name, "title": "", "entry": "", "enabled": False,
           "version": watch.ext_version(d), "error": ""}
    try:
        if not paths.valid_name(d.name):
            raise ValueError("拡張の名前の形が違います")
        m = _read(d)
        if m.get("name") != d.name:
            raise ValueError("manifest の name がフォルダ名と違います")
        entry = m.get("entry")
        if not isinstance(entry, str) or not entry.endswith(".js"):
            raise ValueError("entry は .js のファイルにしてください")
        if not _join(d, entry).is_file():
            raise ValueError(f"entry がありません: {entry}")
        if m.get("server") is not None and not _join(d, str(m["server"])).is_file():
            raise ValueError(f"server がありません: {m['server']}")
        out.update(title=str(m.get("title", d.name)), entry=entry, enabled=bool(m.get("enabled", True)))
    except ValueError as e:
        out["error"] = str(e)
    return out


def list_exts() -> list[dict]:
    root = paths.ext_dir()
    if not root.is_dir():
        return []
    return [_describe(d) for d in sorted(root.iterdir(), key=lambda p: p.name) if d.is_dir()]


def asset(name: str, rel: str) -> tuple[Path, str]:
    """配信するファイルと Content-Type。.js・.css・.json だけ。"""
    paths.check_name(name)
    base = paths.ext_dir() / name
    p = _join(base, rel)
    if p.suffix not in ASSET_TYPES:
        raise ValueError(f"配信しない種類です: {p.name}")
    if not p.is_file():
        raise KeyError(f"ファイルがありません: {rel}")
    return p, ASSET_TYPES[p.suffix]


def call(name: str, fn: str, payload: dict, timeout: float = CALL_TIMEOUT) -> dict:
    paths.check_name(name)
    paths.check_name(fn)
    d = paths.ext_dir() / name
    if not (d / MANIFEST).is_file():
        raise KeyError(f"拡張がありません: {name}")
    try:
        server = _read(d).get("server")
    except ValueError as e:
        raise ExtError(502, str(e))
    if not server:
        raise KeyError(f"server がありません: {name}")
    script = _join(d, str(server))
    py = os.environ.get("SAI_TS_PYTHON") or sys.executable
    env = {**os.environ, "SAI_PROJECT_ROOT": str(paths.project_root()), "PYTHONPATH": str(SRC_DIR)}
    try:
        p = subprocess.run([py, str(script), fn], input=json.dumps(payload, ensure_ascii=False),
                           capture_output=True, text=True, timeout=timeout, env=env,
                           cwd=paths.project_root())
    except subprocess.TimeoutExpired:
        raise ExtError(504, f"{name}/{fn} が時間切れです")
    if p.returncode != 0:
        lines = [ln for ln in p.stderr.splitlines() if ln.strip()]
        raise ExtError(502, lines[-1] if lines else f"終了コード {p.returncode}")
    try:
        return json.loads(p.stdout)
    except ValueError:
        raise ExtError(502, "server.py の出力が JSON ではありません")
