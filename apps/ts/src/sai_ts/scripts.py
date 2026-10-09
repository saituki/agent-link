"""スクリプト（C9）。ts/scripts/<name>.py を sai-ts run で動かす。"""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path

from . import paths

SRC_DIR = Path(__file__).resolve().parent.parent  # apps/ts/src（子の PYTHONPATH）
TIMEOUT = 1800


def _script(name: str) -> Path:
    paths.check_name(name)
    p = paths.inside(paths.scripts_dir() / f"{name}.py")
    if not p.is_file():
        raise KeyError(f"スクリプトがありません: {name}")
    return p


def _last_json(out: str):
    # 標準出力の最後の行から、読める JSON を探す（前の行は進み具合の表示として読み飛ばす）
    for line in reversed(out.splitlines()):
        try:
            return json.loads(line)
        except ValueError:
            continue
    return {}


def run_script(name: str, dataset: str, records: list[str], params: dict | None = None,
               timeout: float = TIMEOUT) -> dict:
    path = _script(name)
    params = {} if params is None else params
    if not isinstance(params, dict):
        raise ValueError("params は JSON の object にしてください")
    paths.dataset_dir(dataset)
    if not records:
        rdir = paths.dataset_dir(dataset) / "records"
        records = sorted(p.name for p in rdir.iterdir() if p.is_dir()) if rdir.is_dir() else []
    for r in records:
        paths.record_dir(dataset, r)  # 名前と存在を確かめる

    sha = hashlib.sha256(path.read_bytes()).hexdigest()[:8]
    source = f"script:{name}@{sha}"
    root = paths.project_root()
    env = {**os.environ, "SAI_PROJECT_ROOT": str(root), "PYTHONPATH": str(SRC_DIR), "SAI_TS_SOURCE": source}
    py = os.environ.get("SAI_TS_PYTHON") or sys.executable
    req = json.dumps({"dataset": dataset, "records": records, "params": params}, ensure_ascii=False)

    t0 = time.monotonic()
    try:
        p = subprocess.run([py, str(path)], input=req, capture_output=True, text=True,
                           timeout=timeout, env=env, cwd=root)
    except subprocess.TimeoutExpired:
        raise TimeoutError(f"{name} が時間切れです")
    if p.returncode != 0:
        lines = [ln for ln in p.stderr.splitlines() if ln.strip()]
        raise RuntimeError(f"{name} が失敗しました: {lines[-1] if lines else p.returncode}")
    return {"script": name, "source": source, "result": _last_json(p.stdout),
            "elapsed_s": round(time.monotonic() - t0, 3)}
