#!/usr/bin/env python3
"""1 つの研究の MLflow の記録から、レビューの判定材料を計算する台帳。

呼び方（共有契約 C2）:
    uv run --no-project --with mlflow python "$SAI_PROJECT_ROOT/.sai/tools/exp_summary.py" \
        --study <study> --metric <名前> --direction maximize|minimize \
        [--min-delta 0.0001] [--leak-max 0.999] [--leak-min 1e-6] \
        [--experiment <名前>] [--tracking-uri <URI>]

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
from dataclasses import dataclass, field

# mlflow の案内メッセージが標準出力に混ざらないようにする。
# mlflow を import する前に設定する。
os.environ.setdefault("MLFLOW_DISABLE_AGENT_HINT", "1")

WARNING_PREFIX = "警告: "
# 親 run（1 回のスクリプトの実行）に付くタグ `exp.role` の値。
SCRIPT_ROLE = "script"
# 入れ子の子 run（パラメータの組 1 つ）に付くタグ `exp.role` の値。
TRIAL_ROLE = "trial"
# 成功とみなす run の状態（C1）。
SUCCESS_STATUS = "FINISHED"


def _warning(message: str) -> None:
    print(WARNING_PREFIX + message, file=sys.stderr)


def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="exp_summary.py",
        description="MLflow に記録された 1 つの研究の台帳を計算する。",
    )
    parser.add_argument("--study", required=True, help="研究の ID（タグ exp.study）")
    parser.add_argument("--metric", required=True, help="指標の名前")
    parser.add_argument(
        "--direction",
        required=True,
        choices=["maximize", "minimize"],
        help="よい値の方向（maximize は大きいほどよい）",
    )
    parser.add_argument(
        "--min-delta", type=float, default=0.0001, help="改善とみなす最小の差（既定 0.0001）"
    )
    parser.add_argument(
        "--leak-max",
        type=float,
        default=0.999,
        help="maximize のとき、この値以上なら漏洩の疑い（既定 0.999）",
    )
    parser.add_argument(
        "--leak-min",
        type=float,
        default=1e-6,
        help="minimize のとき、この値以下なら漏洩の疑い（既定 1e-6）",
    )
    parser.add_argument(
        "--experiment", default=None, help="探す experiment（無ければ全 experiment）"
    )
    parser.add_argument("--tracking-uri", default=None, help="mlflow の接続先")
    return parser.parse_args(argv)


@dataclass
class Trial:
    """試行（`exp.role=trial` の run）1 つ。"""

    script: str
    round: int
    run_id: str
    start_time: int
    status: str
    value: float | None
    params: dict[str, str] = field(default_factory=dict)

    @property
    def ok(self) -> bool:
        """成功した試行か。"""
        return self.status == SUCCESS_STATUS and self.value is not None


@dataclass
class Parent:
    """親 run（`exp.role=script` の run）1 つ。"""

    script: str
    round: int
    action: str
    parent_script: str
    run_id: str
    start_time: int


def _status_text(status) -> str:
    """run の状態を文字列にする。"""
    return str(getattr(status, "value", status))


def _tag(run, key: str) -> str:
    return (run.data.tags or {}).get(key, "")


def _round_of(run) -> int | None:
    """run の世代を整数にする。読めなければ None。

    `exp.generation` があればそれを、無ければ `exp.round` を使う（C6）。
    """
    for key in ("exp.generation", "exp.round"):
        try:
            return int(_tag(run, key))
        except (TypeError, ValueError):
            continue
    return None


def _quote_filter_value(value: str) -> str:
    """mlflow の検索条件の中で使う文字列を引用する。"""
    escaped = value.replace("\\", "\\\\").replace("'", "\\'")
    return f"'{escaped}'"


def _list_runs(client, experiment_id: str, filter_string: str) -> list:
    """1 つの experiment の run を、ページ送りで全部取り出す。"""
    runs = []
    page_token = None
    while True:
        page = client.search_runs(
            [experiment_id],
            filter_string=filter_string,
            run_view_type=_view_type_all(),
            max_results=1000,
            page_token=page_token,
        )
        runs.extend(page)
        page_token = page.token
        if not page_token:
            return runs


def _view_type_all():
    from mlflow.entities import ViewType

    return ViewType.ALL


def _experiment_ids(client, experiment: str | None) -> list[str]:
    """探す experiment の ID を並べる。experiment 名が無いなら全部。"""
    if experiment is not None:
        found = client.get_experiment_by_name(experiment)
        return [found.experiment_id] if found is not None else []
    page_token = None
    ids: list[str] = []
    while True:
        page = client.search_experiments(
            view_type=_view_type_all(), max_results=1000, page_token=page_token
        )
        ids.extend(page.experiment_id for page in page)
        page_token = page.token
        if not page_token:
            return ids


def _collect(client, study: str, metric_name: str, experiment: str | None):
    """study の run を取り、親 run と試行に分けて返す。"""
    filter_string = f"tags.`exp.study` = {_quote_filter_value(study)}"
    parents: list[Parent] = []
    trials: list[Trial] = []
    for experiment_id in _experiment_ids(client, experiment):
        for run in _list_runs(client, experiment_id, filter_string):
            role = _tag(run, "exp.role")
            if role not in (SCRIPT_ROLE, TRIAL_ROLE):
                continue
            round_no = _round_of(run)
            script = _tag(run, "exp.script")
            if round_no is None or not script:
                _warning(
                    "exp.generation（または exp.round）か exp.script が無いので数えません: "
                    f"run_id={run.info.run_id}"
                )
                continue
            if role == SCRIPT_ROLE:
                parents.append(
                    Parent(
                        script=script,
                        round=round_no,
                        action=_tag(run, "exp.action"),
                        parent_script=_tag(run, "exp.parent_script"),
                        run_id=run.info.run_id,
                        start_time=run.info.start_time or 0,
                    )
                )
                continue
            recorded = run.data.metrics.get(metric_name)
            value = None
            if recorded is not None:
                # mlflow 3 の run.data.metrics は値（float）で、2.x は Metric オブジェクト。
                raw = getattr(recorded, "value", recorded)
                try:
                    value = float(raw)
                except (TypeError, ValueError):
                    _warning(
                        f"指標 {metric_name} が数値でないので無視します: "
                        f"run_id={run.info.run_id}"
                    )
            trials.append(
                Trial(
                    script=script,
                    round=round_no,
                    run_id=run.info.run_id,
                    start_time=run.info.start_time or 0,
                    status=_status_text(run.info.status),
                    value=value,
                    params=dict(run.data.params or {}),
                )
            )
    # 新しい round から順に、同じ値のものを安定した順に見られるようにする。
    trials.sort(key=lambda t: (t.round, t.start_time, t.run_id))
    parents.sort(key=lambda p: (p.round, p.start_time, p.run_id))
    return parents, trials


def _pick_parents(parents: list[Parent]) -> list[Parent]:
    """(round, script) ごとに親 run を 1 つだけ選ぶ。同じ組が 2 つ以上あれば新しいほう。

    同じ round でも script が違う親は全部残す（C6）。
    """
    chosen: list[Parent] = []
    for round_no in sorted({p.round for p in parents}):
        for script in _round_scripts(parents, round_no):
            same = [p for p in parents if p.round == round_no and p.script == script]
            if len(same) > 1:
                _warning(
                    f"round {round_no} の script {script} の親 run が {len(same)} つあるので、"
                    f"開始時刻が新しい {same[-1].run_id} を採ります"
                )
            chosen.append(same[-1])
    return chosen


def _round_scripts(parents: list[Parent], round_no: int) -> list[str]:
    """その round にある script を、親の記録が残っている順に並べる。"""
    seen: list[str] = []
    for parent in parents:
        if parent.round == round_no and parent.script not in seen:
            seen.append(parent.script)
    return seen


def _better(value: float | None, other: float | None, direction: str) -> bool:
    """value のほうが other より「よい」か。"""
    if value is None:
        return False
    if other is None:
        return True
    return value > other if direction == "maximize" else value < other


def _improved(value: float | None, other: float | None, direction: str, min_delta: float) -> bool:
    """value のほうが other より min_delta を超えてよいか。"""
    if not _better(value, other, direction):
        return False
    if other is None:
        return True
    delta = (value - other) if direction == "maximize" else (other - value)
    return delta > min_delta


def _best_of(trials: list[Trial], direction: str) -> Trial | None:
    """成功した試行の中の最良（同じ値なら先に現れたもの）。"""
    best: Trial | None = None
    for trial in trials:
        if not trial.ok:
            continue
        if _better(trial.value, best.value if best else None, direction):
            best = trial
    return best


def _trailing_rounds(rounds: list[int], improved: dict[int, bool]) -> int:
    """新しい方から、改善しなかった round が何回続いているか。"""
    count = 0
    for round_no in sorted(rounds, reverse=True):
        if improved.get(round_no, False):
            return count
        count += 1
    return count


def _action_of(parents: list[Parent], round_no: int, script: str) -> str:
    """その世代・その script の親 run の `exp.action`。親が無くければ空文字。"""
    for parent in parents:
        if parent.round == round_no and parent.script == script:
            return parent.action
    return ""


def _generation_script(
    parents: list[Parent], trials: list[Trial], direction: str, round_no: int
) -> str:
    """その世代で最良の試行を出した script を返す。

    成功した試行が無く、記録された script も無いときは、最初の親 run の script を返す。
    """
    best = _best_of([t for t in trials if t.round == round_no], direction)
    scripts = _round_scripts(parents, round_no)
    if best is not None and best.script in scripts:
        return best.script
    return scripts[0]


def _round_entries(parents: list[Parent], trials: list[Trial], direction: str, min_delta: float) -> list[dict]:
    """rounds の欄を作る。世代（round）ごとに 1 行。best_so_far は round 1 から見た累積の最良。"""
    entries = []
    best_so_far: float | None = None
    ok_trials = [t for t in trials if t.ok]
    for round_no in sorted({p.round for p in parents}):
        in_round = [t for t in trials if t.round == round_no]
        cumulative = _best_of([t for t in ok_trials if t.round <= round_no], direction)
        improved = _improved(
            cumulative.value if cumulative else None, best_so_far, direction, min_delta
        )
        if cumulative is not None:
            best_so_far = cumulative.value
        script = _generation_script(parents, trials, direction, round_no)
        entries.append(
            {
                "round": round_no,
                "script": script,
                "action": _action_of(parents, round_no, script),
                "scripts": _round_scripts(parents, round_no),
                "trials": len(in_round),
                "failed": sum(1 for t in in_round if not t.ok),
                "best_so_far": best_so_far,
                "improved": improved,
            }
        )
    return entries


def _debug_rounds(parents: list[Parent], script: str) -> int:
    """そのスクリプトの round を新しい順に見て、debug が何回続いているか。"""
    rounds = sorted((p.round for p in parents if p.script == script), reverse=True)
    actions = {p.round: p.action for p in parents if p.script == script}
    count = 0
    for round_no in rounds:
        if actions[round_no] == "debug":
            count += 1
        else:
            break
    return count


def _stale_rounds(trials: list[Trial], rounds: list[int], direction: str, min_delta: float) -> int:
    """そのスクリプトの round を新しい順に見て、
    スクリプト自身の最良を min_delta を超えて更新しなかった round が何回続いているか。"""
    improved: dict[int, bool] = {}
    previous: float | None = None
    for round_no in sorted(rounds):
        own = _best_of([t for t in trials if t.round <= round_no], direction)
        improved[round_no] = _improved(
            own.value if own else None, previous, direction, min_delta
        )
        if own is not None:
            previous = own.value
    return _trailing_rounds(rounds, improved)


def _param_edges(script: str, trials: list[Trial], direction: str) -> list[dict]:
    """最良の試行の値が、そのスクリプトの試行の中で最小か最大だったパラメータ。"""
    best = _best_of(trials, direction)
    if best is None:
        return []
    edges = []
    for name in sorted(best.params):
        try:
            value = float(best.params[name])
        except (TypeError, ValueError):
            continue  # 数値でないパラメータは出さない
        tried = set()
        for trial in trials:
            raw = trial.params.get(name)
            if raw is None:
                continue
            try:
                tried.add(float(raw))
            except (TypeError, ValueError):
                continue
        if len(tried) < 2:
            continue  # 2 種類以上の値が試されていないパラメータは出さない
        if value == max(tried):
            side = "max"
        elif value == min(tried):
            side = "min"
        else:
            continue
        edges.append({"script": script, "param": name, "side": side})
    return edges


def _script_entries(parents: list[Parent], trials: list[Trial], direction: str, min_delta: float) -> list[dict]:
    """scripts の欄を作る（round の小さい順に並べる）。"""
    entries = []
    for script in sorted({p.script for p in parents}):
        own_parents = sorted(
            (p for p in parents if p.script == script), key=lambda p: (p.round, p.start_time)
        )
        own_trials = [t for t in trials if t.script == script]
        rounds = [p.round for p in own_parents]
        best = _best_of(own_trials, direction)
        entries.append(
            {
                "script": script,
                "action_first": own_parents[0].action,
                "parent_script": own_parents[0].parent_script,
                "rounds": rounds,
                "trials": len(own_trials),
                "failed": sum(1 for t in own_trials if not t.ok),
                "best": best.value if best else None,
                "best_params": dict(best.params) if best else None,
                "stale_rounds": _stale_rounds(own_trials, rounds, direction, min_delta),
                "debug_rounds": _debug_rounds(parents, script),
            }
        )
    return entries


def _best_entry(trials: list[Trial], direction: str) -> dict | None:
    """best の欄。成功した試行が 0 件なら None。"""
    best = _best_of(trials, direction)
    if best is None:
        return None
    return {
        "script": best.script,
        "round": best.round,
        "run_id": best.run_id,
        "value": best.value,
        "params": dict(best.params),
    }


def _last_entry(parents: list[Parent], trials: list[Trial], direction: str) -> dict | None:
    """いちばん大きい世代（round）の内容。世代が無ければ None。

    trials と failed はその世代にある全部の合計なので、
    「最後の世代は全部失敗」を trials > 0 かつ failed == trials で判定できる。
    """
    if not parents:
        return None
    round_no = max(p.round for p in parents)
    in_round = [t for t in trials if t.round == round_no]
    script = _generation_script(parents, trials, direction, round_no)
    return {
        "round": round_no,
        "script": script,
        "action": _action_of(parents, round_no, script),
        "scripts": _round_scripts(parents, round_no),
        "trials": len(in_round),
        "failed": sum(1 for t in in_round if not t.ok),
    }


def build_summary(
    tracking_uri: str,
    study: str,
    metric: str,
    direction: str,
    min_delta: float = 0.0001,
    leak_max: float = 0.999,
    leak_min: float = 1e-6,
    experiment: str | None = None,
) -> dict:
    """mlflow の記録を呼んで、台帳の JSON 相当の dict を作る。"""
    from mlflow.tracking import MlflowClient

    client = MlflowClient(tracking_uri=tracking_uri)
    parents, trials = _collect(client, study, metric, experiment)
    parents = _pick_parents(parents)

    if direction == "maximize":
        leaks = [t for t in trials if t.ok and t.value >= leak_max]
    else:
        leaks = [t for t in trials if t.ok and t.value <= leak_min]

    return {
        "study": study,
        "metric": metric,
        "direction": direction,
        "min_delta": min_delta,
        "best": _best_entry(trials, direction),
        "last": _last_entry(parents, trials, direction),
        "rounds": _round_entries(parents, trials, direction, min_delta),
        "scripts": _script_entries(parents, trials, direction, min_delta),
        "param_edges": [
            edge
            for script in sorted({t.script for t in trials})
            for edge in _param_edges(script, [t for t in trials if t.script == script], direction)
        ],
        "ablation_done": any(p.action == "ablation" for p in parents),
        "leak_suspects": [
            {
                "script": t.script,
                "round": t.round,
                "run_id": t.run_id,
                "value": t.value,
            }
            for t in leaks
        ],
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
            study=args.study,
            metric=args.metric,
            direction=args.direction,
            min_delta=args.min_delta,
            leak_max=args.leak_max,
            leak_min=args.leak_min,
            experiment=args.experiment,
        )
    except SystemExit:
        raise
    except Exception as exc:  # 何が起きても、標準エラー 1 行と終了コード 1 で終わる。
        print(f"{WARNING_PREFIX}台帳の計算に失敗しました: {exc}", file=sys.stderr)
        return 1

    sys.stdout.write(json.dumps(summary, ensure_ascii=False, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
