"""スクリプト（C9・G2-C2・G2-C3）。アルゴリズムを子プロセスで動かし、開始と終わりを runs.jsonl に残す。"""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path

from . import algos, events, paths, runs

SRC_DIR = Path(__file__).resolve().parent.parent  # apps/ts/src（子の PYTHONPATH）
TIMEOUT = 1800
TARGET_ENV_MAX = 65536  # ponytail: 対象は環境変数 SAI_TS_TARGET で渡す。大きいと子を起こせないので載せない（sdk.target() は {} を返す）


def _last_json(out: str):
    # 標準出力の最後の行から、読める JSON を探す（前の行は進み具合の表示として読み飛ばす）
    for line in reversed(out.splitlines()):
        try:
            return json.loads(line)
        except ValueError:
            continue
    return {}


def _exec(path: Path, name: str, dataset: str, records: list[str], params: dict, target: dict,
          run_id: str, sha: str, source: str, timeout: float):
    paths.dataset_dir(dataset)
    if not records:
        rdir = paths.dataset_dir(dataset) / "records"
        records = sorted(p.name for p in rdir.iterdir() if p.is_dir()) if rdir.is_dir() else []
    for r in records:
        paths.record_dir(dataset, r)  # 名前と存在を確かめる

    root = paths.project_root()
    env = {**os.environ, "SAI_PROJECT_ROOT": str(root), "PYTHONPATH": str(SRC_DIR), "SAI_TS_SOURCE": source,
           "SAI_TS_RUN": run_id, "SAI_TS_SHA256": sha}
    tgt = json.dumps(target, ensure_ascii=False)
    if len(tgt.encode("utf-8")) <= TARGET_ENV_MAX:
        env["SAI_TS_TARGET"] = tgt
    else:
        env.pop("SAI_TS_TARGET", None)
    py = os.environ.get("SAI_TS_PYTHON") or sys.executable
    req = json.dumps({"dataset": dataset, "records": records, "params": params, "target": target,
                      "run": run_id}, ensure_ascii=False)
    try:
        p = subprocess.run([py, str(path)], input=req, capture_output=True, text=True,
                           timeout=timeout, env=env, cwd=root)
    except subprocess.TimeoutExpired:
        raise TimeoutError(f"{name} が時間切れです")
    if p.returncode != 0:
        lines = [ln for ln in p.stderr.splitlines() if ln.strip()]
        raise RuntimeError(f"{name} が失敗しました: {lines[-1] if lines else p.returncode}")
    return _last_json(p.stdout)


def run_script(name: str, dataset: str, records: list[str], params: dict | None = None,
               timeout: float = TIMEOUT, target: dict | None = None, by: str = "agent",
               run_id: str | None = None) -> dict:
    run_id = run_id or runs.new_id()
    path, origin = algos.locate(name)  # 無ければ記録を残さず KeyError
    sha = hashlib.sha256(path.read_bytes()).hexdigest()
    source = f"script:{name}@{sha[:8]}"
    params = {} if params is None else params
    if not isinstance(params, dict):
        raise ValueError("params は JSON の object にしてください")
    target = {"vars": [], "events": []} if target is None else target
    records = list(records or [])

    runs.append(dataset, {"op": "start", "id": run_id, "algo": name, "origin": origin, "sha256": sha,
                          "params": params, "target": target, "records": records, "by": by,
                          "started_at": events.now()})
    status, result, error = "error", {}, ""
    t0 = time.monotonic()
    try:
        result = _exec(path, name, dataset, records, params, target, run_id, sha, source, timeout)
        status = "ok"
    except TimeoutError as e:
        status, error = "timeout", str(e)
        raise
    except BaseException as e:  # 途中で止められた場合も、終わりを残す
        error = str(e) or type(e).__name__
        raise
    finally:
        elapsed = round(time.monotonic() - t0, 3)
        runs.append(dataset, {"op": "end", "id": run_id, "status": status, "result": result, "error": error,
                              "ended_at": events.now(), "elapsed_s": elapsed})
    return {"script": name, "source": source, "result": result, "elapsed_s": elapsed, "run": run_id}
