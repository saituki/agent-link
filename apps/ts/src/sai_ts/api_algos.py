"""アルゴリズムの HTTP（G2-C4）。画面からの実行は裏（スレッド）で回し、すぐ 202 を返す。結果は runs.jsonl で読む。"""

from __future__ import annotations

import threading

from fastapi import APIRouter, Body

from . import algos, paths, runs, scripts

router = APIRouter()


@router.get("/api/algos")
def list_algos():
    return {"algos": algos.list_algos()}


@router.post("/api/datasets/{ds}/algos/{name}/runs", status_code=202)
def start_run(ds: str, name: str, body: dict = Body(default={})):
    _, spec = algos.find(name)
    paths.dataset_dir(ds)
    req = algos.check_request(spec, body)
    run_id = runs.new_id()
    # ponytail: 同時に動かす数は止めない。並べすぎて遅くなったら待ち行列を足す
    threading.Thread(target=scripts.run_script,
                     args=(name, ds, req["records"], req["params"], scripts.TIMEOUT, req["target"]),
                     kwargs={"by": "human", "run_id": run_id}, daemon=True).start()
    return {"run": run_id}


@router.get("/api/datasets/{ds}/runs")
def list_runs(ds: str, limit: int = 50):
    if not 1 <= limit <= 500:
        raise ValueError("limit は 1〜500 にしてください")
    return {"runs": runs.list_runs(ds, limit)}
