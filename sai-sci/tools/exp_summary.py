#!/usr/bin/env python3
"""1 つの sprout の MLflow の記録から、レビューの判定材料を計算する台帳。

記録の形: branch 1 つ = experiment（`<project>/ID<sprout>/<model>`）の中の平らな run の列。
run のタグ `sai.run_id` が branch の run_id。親 run も `exp.generation` も無い。

呼び方:
    uv run --no-project --python ">=3.10" --with mlflow python "$SAI_PROJECT_ROOT/.sai/tools/exp_summary.py" \
        --sprout <sprout の run_id> --metric <名前> --direction maximize|minimize \
        [--branch <branch の run_id>] [--min-delta 0.0001] [--leak-max 0.999] [--leak-min 1e-6] \
        [--tracking-uri <URI>]

接続先は --tracking-uri か、環境変数 MLFLOW_TRACKING_URI の順で決める。
どちらも無ければ終了コード 2 で終える。

標準出力には、台帳の JSON 1 つだけを出す。
それ以外の出力（mlflow のログや警告）は標準エラーに出す。
終了コード: 0 = 成功（該当 run が 0 件でも 0）、1 = 実行中の失敗、2 = 引数の誤り。
"""

from __future__ import annotations

import argparse
import json
import os
import sys

# mlflow の案内メッセージが標準出力に混ざらないようにする。
# mlflow を import する前に設定する。
os.environ.setdefault("MLFLOW_DISABLE_AGENT_HINT", "1")

WARNING_PREFIX = "警告: "
# 成功とみなす run の状態。
SUCCESS_STATUS = "FINISHED"
FAILED_STATUS = "FAILED"


def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="exp_summary.py",
        description="MLflow に記録された 1 つの sprout の台帳を計算する。",
    )
    parser.add_argument("--sprout", required=True, help="sprout の run_id（experiment のタグ sai.sprout_run_id）")
    parser.add_argument("--metric", required=True, help="指標の名前")
    parser.add_argument(
        "--direction",
        required=True,
        choices=["maximize", "minimize"],
        help="よい値の方向（maximize は大きいほどよい）",
    )
    parser.add_argument("--branch", type=int, default=None, help="要約を target に出す branch の run_id")
    parser.add_argument(
        "--min-delta", type=float, default=0.0001, help="改善とみなす最小の差（既定 0.0001）"
    )
    parser.add_argument(
        "--leak-max", type=float, default=0.999, help="maximize のとき、この値以上なら漏洩の疑い（既定 0.999）"
    )
    parser.add_argument(
        "--leak-min", type=float, default=1e-6, help="minimize のとき、この値以下なら漏洩の疑い（既定 1e-6）"
    )
    parser.add_argument("--tracking-uri", default=None, help="mlflow の接続先")
    return parser.parse_args(argv)


class Run:
    """run 1 つ（branch の試行）。"""

    def __init__(self, run, experiment, metric_name: str, branch: int, warn) -> None:
        tags = run.data.tags or {}
        self.run_id = run.info.run_id
        self.name = run.info.run_name or ""
        self.start_time = run.info.start_time or 0
        self.status = str(getattr(run.info.status, "value", run.info.status))
        self.params = dict(run.data.params or {})
        self.branch = branch
        self.experiment = experiment.name
        self.action = tags.get("exp.action", "")
        self.parent_branch = tags.get("exp.parent_branch", "")
        self.value: float | None = None
        recorded = run.data.metrics.get(metric_name)
        if recorded is not None:
            # mlflow 3 の run.data.metrics は値（float）で、2.x は Metric オブジェクト。
            try:
                self.value = float(getattr(recorded, "value", recorded))
            except (TypeError, ValueError):
                warn(f"指標 {metric_name} が数値でないので無視します: run_id={self.run_id}")

    @property
    def ok(self) -> bool:
        return self.status == SUCCESS_STATUS and self.value is not None


def _quote(value: str) -> str:
    escaped = value.replace("\\", "\\\\").replace("'", "\\'")
    return f"'{escaped}'"


def _pages(fetch):
    """mlflow の検索をページ送りで全部取り出す。fetch(page_token) -> PagedList。"""
    token = None
    while True:
        page = fetch(token)
        yield from page
        token = page.token
        if not token:
            return


def _better(value, other, direction: str) -> bool:
    if value is None:
        return False
    if other is None:
        return True
    return value > other if direction == "maximize" else value < other


def _improved(value, other, direction: str, min_delta: float) -> bool:
    """value のほうが other より min_delta を超えてよいか（other が無ければ value があれば真）。"""
    if not _better(value, other, direction):
        return False
    if other is None:
        return True
    return abs(value - other) > min_delta


def _best_of(runs: list[Run], direction: str) -> Run | None:
    """成功した run の中の最良（同じ値なら先に現れたもの）。"""
    best = None
    for run in runs:
        if run.ok and _better(run.value, best.value if best else None, direction):
            best = run
    return best


def _param_edges(runs: list[Run], direction: str) -> list[dict]:
    """最良の run の値が、その branch で試した値の最小か最大だったパラメータ（2 種類以上試したものだけ）。"""
    best = _best_of(runs, direction)
    if best is None:
        return []
    edges = []
    for name in sorted(best.params):
        tried = set()
        for run in runs:
            try:
                tried.add(float(run.params[name]))
            except (KeyError, TypeError, ValueError):
                continue  # 無い・数値でない値は数えない
        try:
            value = float(best.params[name])
        except (TypeError, ValueError):
            continue
        if len(tried) < 2:
            continue
        if value == max(tried):
            edges.append({"param": name, "side": "max"})
        elif value == min(tried):
            edges.append({"param": name, "side": "min"})
    return edges


def build_summary(
    tracking_uri: str,
    sprout: str,
    metric: str,
    direction: str,
    branch: int | None = None,
    min_delta: float = 0.0001,
    leak_max: float = 0.999,
    leak_min: float = 1e-6,
) -> dict:
    """mlflow の記録を呼んで、台帳の JSON 相当の dict を作る。"""
    from mlflow.entities import ViewType
    from mlflow.tracking import MlflowClient

    warnings: list[str] = []

    def warn(message: str) -> None:
        warnings.append(message)
        print(WARNING_PREFIX + message, file=sys.stderr)

    client = MlflowClient(tracking_uri=tracking_uri)
    flt = f"tags.`sai.sprout_run_id` = {_quote(sprout)}"
    experiments = sorted(
        _pages(lambda t: client.search_experiments(
            view_type=ViewType.ACTIVE_ONLY, filter_string=flt, max_results=1000, page_token=t)),
        key=lambda e: int(e.experiment_id) if str(e.experiment_id).isdigit() else 0,
    )

    runs: list[Run] = []
    for exp in experiments:
        for raw in _pages(lambda t: client.search_runs(
                [exp.experiment_id], run_view_type=ViewType.ACTIVE_ONLY, max_results=1000, page_token=t)):
            try:
                branch_id = int((raw.data.tags or {}).get("sai.run_id", ""))
            except ValueError:
                warn(f"sai.run_id が無い（または整数でない）ので数えません: run_id={raw.info.run_id}")
                continue
            runs.append(Run(raw, exp, metric, branch_id, warn))
    runs.sort(key=lambda r: (r.branch, r.start_time, r.run_id))

    curves: dict[str, bool] = {}

    def has_curve(run: Run) -> bool:
        if run.run_id not in curves:
            curves[run.run_id] = len(client.get_metric_history(run.run_id, "train_loss")) >= 2
        return curves[run.run_id]

    # branch ごと（同じ experiment の中）の親・action
    parent_of: dict[int, str] = {}
    action_of: dict[int, str] = {}
    for run in runs:
        if run.parent_branch and not parent_of.get(run.branch):
            parent_of[run.branch] = run.parent_branch
        if run.action and not action_of.get(run.branch):
            action_of[run.branch] = run.action

    def depth_and_debug(b: int) -> tuple[int, int]:
        depth, seen, cur = 1, {b}, b
        while True:
            parent = parent_of.get(cur, "")
            if not parent.isdigit() or int(parent) in seen:
                break
            cur = int(parent)
            seen.add(cur)
            depth += 1
        debug, cur, seen = 0, b, set()
        while action_of.get(cur) == "debug" and cur not in seen:
            seen.add(cur)
            debug += 1
            parent = parent_of.get(cur, "")
            if not parent.isdigit():
                break
            cur = int(parent)
        return depth, debug

    # (experiment, branch) ごとの entry。experiment の中で branch の小さい順に best を積み上げる
    exp_entries, branch_entries = [], []
    for exp in experiments:
        in_exp = [r for r in runs if r.experiment == exp.name]
        previous = None  # この experiment のそれまでの最良の値
        flags = []
        for b in sorted({r.branch for r in in_exp}):
            mine = [r for r in in_exp if r.branch == b]
            best = _best_of(mine, direction)
            improved = _improved(best.value if best else None, previous, direction, min_delta)
            if best is not None and _better(best.value, previous, direction):
                previous = best.value
            flags.append(improved)
            depth, debug = depth_and_debug(b)
            branch_entries.append({
                "branch": b,
                "experiment": exp.name,
                "model": (exp.tags or {}).get("exp.model") or exp.name.rsplit("/", 1)[-1],
                "action": action_of.get(b, ""),
                "parent_branch": int(parent_of[b]) if parent_of.get(b, "").isdigit() else None,
                "depth": depth,
                "debug_depth": debug,
                "runs": len(mine),
                "failed": sum(1 for r in mine if r.status == FAILED_STATUS),
                "best": best.value if best else None,
                "best_params": dict(best.params) if best else None,
                "improved": improved,
                "has_curves": any(has_curve(r) for r in mine),
                "param_edges": _param_edges(mine, direction),
            })
        stale = 0
        for flag in reversed(flags):
            if flag:
                break
            stale += 1
        best = _best_of(in_exp, direction)
        exp_entries.append({
            "name": exp.name,
            "experiment_id": str(exp.experiment_id),
            "model": (exp.tags or {}).get("exp.model") or exp.name.rsplit("/", 1)[-1],
            "branches": sorted({r.branch for r in in_exp}),
            "runs": len(in_exp),
            "failed": sum(1 for r in in_exp if r.status == FAILED_STATUS),
            "best": best.value if best else None,
            "best_branch": best.branch if best else None,
            "stale": stale,
            "has_curves": any(has_curve(r) for r in in_exp),
        })

    best = _best_of(runs, direction)
    if direction == "maximize":
        leaks = [r for r in runs if r.ok and r.value >= leak_max]
    else:
        leaks = [r for r in runs if r.ok and r.value <= leak_min]

    target = None
    if branch is not None:
        found = next((e for e in branch_entries if e["branch"] == branch), None)
        keys = ("runs", "failed", "best", "improved")
        target = {"branch": branch, **({k: found[k] for k in keys} if found else
                  {"runs": 0, "failed": 0, "best": None, "improved": False})}

    return {
        "sprout": int(sprout) if sprout.isdigit() else sprout,
        "metric": metric,
        "direction": direction,
        "min_delta": min_delta,
        "best": None if best is None else {
            "experiment": best.experiment, "branch": best.branch, "run_name": best.name,
            "run_id": best.run_id, "value": best.value, "params": dict(best.params),
        },
        "experiments": exp_entries,
        "branches": branch_entries,
        "target": target,
        "total_branches": len({r.branch for r in runs}),
        "ablation_done": any(r.action == "ablation" for r in runs),
        "leak_suspects": [{"branch": r.branch, "run_name": r.name, "value": r.value} for r in leaks],
        "warnings": warnings,
    }


def main(argv: list[str] | None = None) -> int:
    try:
        args = _parse_args(argv)
    except SystemExit as exc:
        return int(exc.code or 0)

    tracking_uri = args.tracking_uri or os.environ.get("MLFLOW_TRACKING_URI")
    if not tracking_uri:
        print(
            "接続先が指定されていません: --tracking-uri か環境変数 MLFLOW_TRACKING_URI を指定してください",
            file=sys.stderr,
        )
        return 2

    try:
        summary = build_summary(
            tracking_uri=tracking_uri,
            sprout=args.sprout,
            metric=args.metric,
            direction=args.direction,
            branch=args.branch,
            min_delta=args.min_delta,
            leak_max=args.leak_max,
            leak_min=args.leak_min,
        )
    except Exception as exc:  # 何が起きても、標準エラー 1 行と終了コード 1 で終わる。
        print(f"{WARNING_PREFIX}台帳の計算に失敗しました: {exc}", file=sys.stderr)
        return 1

    sys.stdout.write(json.dumps(summary, ensure_ascii=False, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
