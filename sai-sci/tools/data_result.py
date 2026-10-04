#!/usr/bin/env python3
"""解析計画と結果を照らして、統計の報告の決まりを満たすかを機械的に確かめ、MLflow に記録する道具。

検査（`check`）と表（`table`）は**標準ライブラリだけ**で動く。`log` だけが mlflow を使う。

閉じ方:
    uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/data_result.py" \
        check <results.json> --plan <plan.json> [--json]
    uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/data_result.py" \
        table <results.json> --plan <plan.json>
    uv run --no-project --python ">=3.11" --with mlflow --with boto3 python "$SAI_PROJECT_ROOT/.sai/tools/data_result.py" \
        log --experiment <名> --run-name <名> --project <p> --run-id <n> --study <s> --node <N> --kind <eda|test> \
        [--plan <plan.json>] [--results <results.json>] [--artifact <ファイルかフォルダ>]... \
        [--tracking-uri <URI>] [--json]

検査の規則（違反は 1 行 1 件 `<ID>: <規則> <理由>` を標準出力へ）:
    K1  results.json の plan_sha256 が plan.json のバイト列の SHA-256 と一致する
    K2  計画の kind: confirmatory の仮説は、すべて結果がある
    K3  計画に無い id の結果は、deviation が空でない
    K4  test が計画の test と違うときは deviation が空でない。計画の fallback に替えたときは
        assumptions に ok: false が 1 つ以上ある
    K5  frequentist: p が 0〜1、effect.value が有限、effect.ci が有限の 2 数で小さい順、
        ci_level が 0〜1 の間。confirmatory なら assumptions が空でない
    K6  属する family の members が 2 つ以上で correction が none でなければ、
        p_adj があり p_adj >= p
    K7  bayes: rhat_max <= 1.01、ess_bulk_min >= 400、divergences == 0。hdi が小さい順
    K8  ml: cv.folds >= 2、cv.scheme が空でない
    K9  descriptive: estimate.value が有限

接続先は --tracking-uri か、環境変数 MLFLOW_TRACKING_URI の順で決める。
どちらも無ければ終了コード 2 で終える。

終了コード: 0 = 違反なし（`log` は成功）、1 = 違反あり・実行中の失敗、2 = 使い方の誤り。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import sys

# mlflow の案内メッセージが標準出力に混ざらないようにする。mlflow を import する前に設定する。
os.environ.setdefault("MLFLOW_DISABLE_AGENT_HINT", "1")

WARNING_PREFIX = "警告: "
RUN_STATUS_OK = "FINISHED"
RUN_STATUS_FAILED = "FAILED"

# 有効数字の桁数。表の数値と標準出力の p に使う。
SIG_DIGITS = 3
# K6 が「補正を持つ family」とみなす members の最少の長さ。
FAMILY_MIN_MEMBERS = 2
# K7 で求める R-hat と ESS の下限。
RHAT_MAX = 1.01
ESS_BULK_MIN = 400

# 表で無い値を表す記号。
MISSING_MARK = "―"
KIND_LABELS = {"confirmatory": "確証", "exploratory": "探索"}
KIND_LABEL_UNPLANNED = "計画外"
CHANGE_NONE = "なし"


class UsageError(Exception):
    """使い方の誤り。終了コード 2 で終わる。"""


# ---------------------------------------------------------------- 読む


def load_json(path: str) -> dict:
    """JSON を読む。読めない・壊れていれば UsageError。"""
    try:
        with open(path, "rb") as fh:
            raw = fh.read()
    except OSError as exc:
        raise UsageError(f"読めないファイルです: {path} ({exc.strerror or exc})") from exc
    try:
        data = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise UsageError(f"JSON が読めないか壊れています: {path} ({exc})") from exc
    if not isinstance(data, dict):
        raise UsageError(f"JSON の最上段がオブジェクトではありません: {path}")
    return data


def plan_sha256(path: str) -> str:
    """ファイルのバイト列の SHA-256（16 進）。"""
    try:
        with open(path, "rb") as fh:
            return hashlib.sha256(fh.read()).hexdigest()
    except OSError as exc:
        raise UsageError(f"読めないファイルです: {path} ({exc.strerror or exc})") from exc


# ---------------------------------------------------------------- 検査


def _finite(value) -> float | None:
    """有限の float を返す。それ以外（文字列・真偽値・NaN・±inf）は None。"""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    number = float(value)
    if math.isnan(number) or math.isinf(number):
        return None
    return number


def _mapping(value) -> dict:
    """オブジェクトならそれ、空なら {}。"""
    return value if isinstance(value, dict) else {}


def _sequence(value) -> list:
    """配列ならそれ、空なら []。"""
    return value if isinstance(value, list) else []


def _text(value) -> str:
    """文字列ならそれ、空なら ""。"""
    return value if isinstance(value, str) else ""


def _assumption_failed(assumptions) -> bool:
    """assumptions に ok: false が 1 つ以上あるか。"""
    for item in _sequence(assumptions):
        if isinstance(item, dict) and item.get("ok") is False:
            return True
    return False


def _results_of(results: dict) -> list[dict]:
    """results の results 配列。オブジェクトでない要素は落とす。"""
    return [r for r in _sequence(results.get("results")) if isinstance(r, dict)]


def _hypotheses_of(plan: dict) -> list[dict]:
    return [h for h in _sequence(plan.get("hypotheses")) if isinstance(h, dict)]


def _families_of(plan: dict) -> dict[str, dict]:
    """families を id → そのもの の表にする。"""
    out: dict[str, dict] = {}
    for family in _sequence(plan.get("families")):
        if isinstance(family, dict):
            key = _text(family.get("id"))
            if key:
                out[key] = family
    return out


def check(results: dict, plan: dict, plan_hash: str) -> list[dict]:
    """違反の一覧 [{"id", "rule", "message"}] を返す（空なら違反なし）。

    必須の欄が無い・型が違うときも、その規則の違反として出す（例外で落とさない）。
    """
    out: list[dict] = []

    def add(rule: str, ident: str, message: str) -> None:
        out.append({"id": ident, "rule": rule, "message": message})

    hypotheses = {h.get("id"): h for h in _hypotheses_of(plan) if _text(h.get("id"))}
    families = _families_of(plan)
    items = _results_of(results)
    by_id: dict[str, dict] = {}
    for item in items:
        key = _text(item.get("id"))
        if key and key not in by_id:
            by_id[key] = item

    # K1: plan_sha256 が plan.json のバイト列と一致する
    declared = results.get("plan_sha256")
    if _text(declared) != plan_hash:
        add("K1", "-", f"plan_sha256 が plan.json の SHA-256 ({plan_hash[:12]}…) と一致しない")

    # K2: 計画の confirmatory はすべて結果がある
    for key, hypothesis in hypotheses.items():
        if _text(hypothesis.get("kind")) == "confirmatory" and key not in by_id:
            add("K2", key, "計画の仮説が confirmatory なのに結果が無い")

    for item in items:
        key = _text(item.get("id")) or MISSING_MARK
        paradigm = _text(item.get("paradigm"))
        deviation = _text(item.get("deviation"))
        planned = hypotheses.get(key)

        # K3: 計画に無い id の結果は deviation が空でない
        if planned is None:
            if not deviation:
                add("K3", key, "計画に無い仮説なのに deviation が空")

        # K4: test が計画と違うときは deviation が空でない。fallback に替えたときは
        #     assumptions に ok: false が要る
        if planned is not None:
            planned_test = _text(planned.get("test"))
            done_test = _text(item.get("test"))
            if planned_test and done_test and planned_test != done_test:
                if not deviation:
                    add("K4", key, f"計画の {planned_test} でなく {done_test} なのに deviation が空")
                if done_test == _text(planned.get("fallback")) and not _assumption_failed(item.get("assumptions")):
                    add("K4", key, f"fallback の {done_test} に替えたのに assumptions に ok: false が無い")

        # K5: frequentist
        if paradigm == "frequentist":
            out.extend(_check_frequentist(key, item, planned is not None and _text(planned.get("kind")) == "confirmatory"))
        # K6: family の補正
        if planned is not None:
            # K6 は p があるもの（frequentist）にだけ掛ける。p の無い結果（bayes・ml など）に補正は掛からない。
            # frequentist な結果が p 無しでも K5 が出るので、p_adj の忘れは見つかる。
            out.extend(_check_family(key, item, planned, families, paradigm))
        # K7: bayes
        if paradigm == "bayes":
            out.extend(_check_bayes(key, item))
        # K8: ml
        if paradigm == "ml":
            out.extend(_check_ml(key, item))
        # K9: descriptive
        if paradigm == "descriptive":
            out.extend(_check_descriptive(key, item))

    return out


def _check_frequentist(key: str, item: dict, confirmatory: bool) -> list[dict]:
    out: list[dict] = []

    def add(message: str) -> None:
        out.append({"id": key, "rule": "K5", "message": message})

    if "p" not in item:
        add("p が無い")
    else:
        p = _finite(item.get("p"))
        if p is None or not 0.0 <= p <= 1.0:
            add(f"p が 0〜1 の数でない: {item.get('p')!r}")

    level = _finite(item.get("ci_level"))
    if "ci_level" not in item:
        add("ci_level が無い")
    elif level is None or not 0.0 < level <= 1.0:
        add(f"ci_level が 0〜1 の数でない: {item.get('ci_level')!r}")

    effect = item.get("effect")
    if not isinstance(effect, dict):
        add("effect が無い")
    else:
        if _finite(effect.get("value")) is None:
            add(f"effect.value が有限の数でない: {effect.get('value')!r}")
        ci = effect.get("ci")
        if not isinstance(ci, list) or len(ci) != 2:
            add(f"effect.ci が 2 数の配列でない: {ci!r}")
        else:
            low, high = _finite(ci[0]), _finite(ci[1])
            if low is None or high is None:
                add(f"effect.ci が有限の数でない: {ci!r}")
            elif low > high:
                add(f"effect.ci が小さい順でない: {ci!r}")

    if confirmatory and not _sequence(item.get("assumptions")):
        add("confirmatory なのに assumptions が空")
    return out


def _check_family(
    key: str, item: dict, planned: dict, families: dict[str, dict], paradigm: str
) -> list[dict]:
    if paradigm != "frequentist":
        return []
    family_id = _text(planned.get("family"))
    family = families.get(family_id) if family_id else None
    if family is None:
        return []
    members = [m for m in _sequence(family.get("members")) if isinstance(m, str)]
    correction = _text(family.get("correction"))
    if len(members) < FAMILY_MIN_MEMBERS or correction in ("", "none"):
        return []
    if key not in members:
        return []

    out: list[dict] = []
    if "p_adj" not in item:
        out.append({
            "id": key,
            "rule": "K6",
            "message": f"family {family_id} は {correction} で補正するが p_adj が無い",
        })
        return out
    p_adj = _finite(item.get("p_adj"))
    p = _finite(item.get("p"))
    if p_adj is None:
        out.append({
            "id": key,
            "rule": "K6",
            "message": f"p_adj が数でない: {item.get('p_adj')!r}",
        })
    elif p is not None and p_adj < p:
        out.append({
            "id": key,
            "rule": "K6",
            "message": f"family {family_id} は {correction} で補正するが p_adj ({p_adj:g}) が p ({p:g}) より小さい",
        })
    return out


def _check_bayes(key: str, item: dict) -> list[dict]:
    out: list[dict] = []

    def add(message: str) -> None:
        out.append({"id": key, "rule": "K7", "message": message})

    posterior = item.get("posterior")
    if not isinstance(posterior, dict):
        add("posterior が無い")
    else:
        if _finite(posterior.get("mean")) is None:
            add(f"posterior.mean が有限の数でない: {posterior.get('mean')!r}")
        hdi = posterior.get("hdi")
        if not isinstance(hdi, list) or len(hdi) != 2:
            add(f"posterior.hdi が 2 数の配列でない: {hdi!r}")
        else:
            low, high = _finite(hdi[0]), _finite(hdi[1])
            if low is None or high is None:
                add(f"posterior.hdi が有限の数でない: {hdi!r}")
            elif low > high:
                add(f"posterior.hdi が小さい順でない: {hdi!r}")

    diagnostics = item.get("diagnostics")
    if not isinstance(diagnostics, dict):
        add("diagnostics が無い")
        return out

    rhat = _finite(diagnostics.get("rhat_max"))
    if rhat is None:
        add(f"diagnostics.rhat_max が有限の数でない: {diagnostics.get('rhat_max')!r}")
    elif rhat > RHAT_MAX:
        add(f"R-hat が {RHAT_MAX} を超える: {rhat:g}")

    ess = _finite(diagnostics.get("ess_bulk_min"))
    if ess is None:
        add(f"diagnostics.ess_bulk_min が有限の数でない: {diagnostics.get('ess_bulk_min')!r}")
    elif ess < ESS_BULK_MIN:
        add(f"ESS が {ESS_BULK_MIN} 未満: {ess:g}")

    divergences = diagnostics.get("divergences")
    if isinstance(divergences, bool) or not isinstance(divergences, (int, float)):
        add(f"diagnostics.divergences が整数でない: {divergences!r}")
    elif float(divergences) != 0.0:
        add(f"発散がある: {divergences!r}")
    return out


def _check_ml(key: str, item: dict) -> list[dict]:
    out: list[dict] = []

    def add(message: str) -> None:
        out.append({"id": key, "rule": "K8", "message": message})

    cv = item.get("cv")
    if not isinstance(cv, dict):
        add("cv が無い")
        return out
    folds = _finite(cv.get("folds"))
    if folds is None or folds != int(folds):
        add(f"cv.folds が整数でない: {cv.get('folds')!r}")
    elif folds < 2:
        add(f"cv.folds が 2 未満: {folds:g}")
    if not _text(cv.get("scheme")):
        add("cv.scheme が空")
    if _text(cv.get("metric")) == "":
        add("cv.metric が空")
    return out


def _check_descriptive(key: str, item: dict) -> list[dict]:
    estimate = item.get("estimate")
    if not isinstance(estimate, dict):
        return [{"id": key, "rule": "K9", "message": "estimate が無い"}]
    if _finite(estimate.get("value")) is None:
        return [{"id": key, "rule": "K9", "message": f"estimate.value が有限の数でない: {estimate.get('value')!r}"}]
    return []


# ---------------------------------------------------------------- 表


def _sig(value) -> str:
    """有限の数なら有効数字 3 桁、無ければ ―。"""
    number = _finite(value)
    if number is None:
        return MISSING_MARK
    return f"{number:.{SIG_DIGITS}g}"


def _p_cell(value) -> str:
    """p の欄。0.001 未満なら `p < 0.001`、無ければ ―。"""
    number = _finite(value)
    if number is None:
        return MISSING_MARK
    if number < 0.001:
        return "p < 0.001"
    return _sig(number)


def _bracket(low, high) -> str:
    a, b = _finite(low), _finite(high)
    if a is None or b is None:
        return MISSING_MARK
    return f"[{_sig(a)}, {_sig(b)}]"


def table(results: dict, plan: dict) -> str:
    """Markdown の表を返す（C6 の列）。"""
    hypotheses = {h.get("id"): h for h in _hypotheses_of(plan) if _text(h.get("id"))}
    header = (
        "| ID | 種類 | 検定 | n | 推定値 [CI] | 効果量 [CI] | p | p（補正後） | 計画からの変更 |"
    )
    divider = "|---|---|---|---|---|---|---|---|---|"
    lines = [header, divider]

    for item in _results_of(results):
        key = _text(item.get("id")) or MISSING_MARK
        planned = hypotheses.get(key)
        if planned is None:
            kind = KIND_LABEL_UNPLANNED
        else:
            kind = KIND_LABELS.get(_text(planned.get("kind")), _text(item.get("paradigm")) or MISSING_MARK)

        paradigm = _text(item.get("paradigm"))
        n = _mapping(item.get("n")).get("total")
        n_cell = _sig(n) if n is not None else MISSING_MARK
        change = _text(item.get("deviation")) or CHANGE_NONE

        estimate_cell = MISSING_MARK
        effect_cell = MISSING_MARK
        if paradigm == "bayes":
            posterior = _mapping(item.get("posterior"))
            hdi = _bracket(
                (posterior.get("hdi") or [None, None])[0] if isinstance(posterior.get("hdi"), list) and len(posterior["hdi"]) == 2 else None,
                (posterior.get("hdi") or [None, None])[1] if isinstance(posterior.get("hdi"), list) and len(posterior["hdi"]) == 2 else None,
            )
            prob = _finite(posterior.get("hdi_prob"))
            prob_text = f"HDI{prob * 100:g}%" if prob is not None else "HDI"
            estimate_cell = f"事後平均 {_sig(posterior.get('mean'))} {prob_text} {hdi}"
        elif paradigm == "ml":
            cv = _mapping(item.get("cv"))
            folds = _finite(cv.get("folds"))
            folds_text = f"{folds:g} 分割" if folds is not None else MISSING_MARK
            estimate_cell = (
                f"{cv.get('metric') or MISSING_MARK} {_sig(cv.get('mean'))} ± {_sig(cv.get('sd'))}"
                f"（{cv.get('scheme') or MISSING_MARK}）、{folds_text}"
            )
        else:
            estimate = _mapping(item.get("estimate"))
            if estimate:
                ci = estimate.get("ci")
                low = ci[0] if isinstance(ci, list) and len(ci) == 2 else None
                high = ci[1] if isinstance(ci, list) and len(ci) == 2 else None
                estimate_cell = f"{estimate.get('name') or MISSING_MARK} {_sig(estimate.get('value'))} {_bracket(low, high)}"
            effect = _mapping(item.get("effect"))
            if effect:
                ci = effect.get("ci")
                low = ci[0] if isinstance(ci, list) and len(ci) == 2 else None
                high = ci[1] if isinstance(ci, list) and len(ci) == 2 else None
                effect_cell = f"{effect.get('name') or MISSING_MARK} {_sig(effect.get('value'))} {_bracket(low, high)}"

        lines.append(
            "| {id} | {kind} | {test} | {n} | {est} | {eff} | {p} | {padj} | {chg} |".format(
                id=key,
                kind=kind,
                test=_text(item.get("test")) or MISSING_MARK,
                n=n_cell,
                est=estimate_cell,
                eff=effect_cell,
                p=_p_cell(item.get("p")) if "p" in item else MISSING_MARK,
                padj=_p_cell(item.get("p_adj")) if "p_adj" in item else MISSING_MARK,
                chg=change,
            )
        )
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------- MLflow に記録するための中身


def metrics_of(results: dict) -> dict[str, float]:
    """MLflow に残す指標の名前 → 値（ある物だけ）。"""
    out: dict[str, float] = {}
    for item in _results_of(results):
        key = _text(item.get("id"))
        if not key:
            continue

        def put(suffix, value) -> None:
            number = _finite(value)
            if number is not None:
                out[f"{key}.{suffix}"] = number

        def put_from_ci(suffix_low, suffix_high, ci) -> None:
            if isinstance(ci, list) and len(ci) == 2:
                put(suffix_low, ci[0])
                put(suffix_high, ci[1])

        put("p", item.get("p"))
        put("p_adj", item.get("p_adj"))
        effect = _mapping(item.get("effect"))
        put("effect", effect.get("value"))
        put_from_ci("effect_ci_low", "effect_ci_high", effect.get("ci"))
        put("estimate", _mapping(item.get("estimate")).get("value"))
        posterior = _mapping(item.get("posterior"))
        put("post_mean", posterior.get("mean"))
        put_from_ci("hdi_low", "hdi_high", posterior.get("hdi"))
        put("rhat_max", _mapping(item.get("diagnostics")).get("rhat_max"))
        cv = _mapping(item.get("cv"))
        put("cv_mean", cv.get("mean"))
        put("cv_sd", cv.get("sd"))
        put("n", _mapping(item.get("n")).get("total"))
    return out


def params_of(plan: dict) -> dict[str, str]:
    """MLflow に残す引数の名前 → 値（文字列）。"""
    out: dict[str, str] = {}
    families = _families_of(plan)
    for name in ("alpha", "seed"):
        if plan.get(name) is not None:
            out[name] = str(plan[name])
    for hypothesis in _hypotheses_of(plan):
        key = _text(hypothesis.get("id"))
        if not key:
            continue
        out[f"{key}.test"] = _text(hypothesis.get("test"))
        out[f"{key}.kind"] = _text(hypothesis.get("kind"))
        family_id = _text(hypothesis.get("family"))
        family = families.get(family_id) if family_id else None
        out[f"{key}.correction"] = _text(family.get("correction")) if family else "none"
    return out


# ---------------------------------------------------------------- MLflow に記録する


def log_run(args, client=None) -> dict:
    """MLflow へ記録する。client を渡せばそれを使う（テストは偽物のものを渡す）。

    返り値は {"experiment_id", "run_id", "run_name", "metrics", "artifacts"}。
    metrics と artifacts は数。
    """
    if client is None:
        tracking_uri = args.tracking_uri or os.environ.get("MLFLOW_TRACKING_URI")
        if not tracking_uri:
            raise UsageError(
                "接続先が指定されていません: --tracking-uri か環境変数 MLFLOW_TRACKING_URI を指定してください"
            )
        import mlflow  # noqa: F401  （呼出し側にあることを確かめるだけ）
        from mlflow.tracking import MlflowClient

        client = MlflowClient(tracking_uri=tracking_uri)

    plan: dict = {}
    results: dict = {}
    if getattr(args, "plan", None):
        plan = load_json(args.plan)
    if getattr(args, "results", None):
        results = load_json(args.results)

    exp_tags = {
        "sai.project": args.project,
        "exp.study": args.study,
        "sci.node": str(args.node),
        "exp.kind": "data",
    }
    experiment = client.get_experiment_by_name(args.experiment)
    if experiment is None:
        experiment_id = client.create_experiment(
            args.experiment,
            artifact_location=os.environ.get("EXP_ARTIFACT_LOCATION") or None,
            tags=exp_tags,
        )
    else:
        experiment_id = experiment.experiment_id

    run_tags = {
        "sai.project": args.project,
        "sai.run_id": str(args.run_id),
        "exp.study": args.study,
        "sci.node": str(args.node),
        "exp.kind": args.kind,
    }
    run = client.create_run(experiment_id, run_name=args.run_name, tags=run_tags)
    run_id = run.info.run_id

    # 途中で失敗しても run は必ず閉じる。
    status = RUN_STATUS_OK
    artifacts = 0
    try:
        for name, value in params_of(plan).items():
            client.log_param(run_id, name, value)
        for name, value in metrics_of(results).items():
            client.log_metric(run_id, name, value)

        for path in (getattr(args, "plan", None), getattr(args, "results", None)):
            if path:
                client.log_artifact(run_id, path)
                artifacts += 1
        for path in getattr(args, "artifact", None) or []:
            if os.path.isdir(path):
                client.log_artifacts(run_id, path)
            else:
                client.log_artifact(run_id, path)
            artifacts += 1
    except Exception:
        status = RUN_STATUS_FAILED
        raise
    finally:
        client.set_terminated(run_id, status)

    return {
        "experiment_id": str(experiment_id),
        "run_id": str(run_id),
        "run_name": args.run_name,
        "metrics": len(metrics_of(results)),
        "artifacts": artifacts,
    }


# ---------------------------------------------------------------- 入口


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="data_result.py",
        description="解析計画と結果を照らして統計の報告の決まりを確かめ、MLflow に記録する。",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    check_cmd = sub.add_parser("check", help="計画と結果を照らして違反を列挙する")
    check_cmd.add_argument("results", help="results.json")
    check_cmd.add_argument("--plan", required=True, help="plan.json")
    check_cmd.add_argument("--json", action="store_true", help="JSON で出力する")

    table_cmd = sub.add_parser("table", help="結果を Markdown の表にする")
    table_cmd.add_argument("results", help="results.json")
    table_cmd.add_argument("--plan", required=True, help="plan.json")

    log_cmd = sub.add_parser("log", help="MLflow に記録する")
    log_cmd.add_argument("--experiment", required=True, help="experiment の名前")
    log_cmd.add_argument("--run-name", required=True, help="run の名前")
    log_cmd.add_argument("--project", required=True, help="sai.project タグの値")
    log_cmd.add_argument("--run-id", required=True, type=int, help="sai.run_id タグの値")
    log_cmd.add_argument("--study", required=True, help="exp.study タグの値")
    log_cmd.add_argument("--node", required=True, type=int, help="sci.node タグの値")
    log_cmd.add_argument("--kind", required=True, choices=["eda", "test"], help="exp.kind タグの値")
    log_cmd.add_argument("--plan", default=None, help="plan.json")
    log_cmd.add_argument("--results", default=None, help="results.json")
    log_cmd.add_argument("--artifact", action="append", default=None, help="記録するファイルかフォルダ")
    log_cmd.add_argument("--tracking-uri", default=None, help="mlflow の接続先")
    log_cmd.add_argument("--json", action="store_true", help="JSON で出力する")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = _build_parser()
    try:
        args = parser.parse_args(argv)
    except SystemExit as exc:
        return int(exc.code or 0)

    try:
        if args.command in ("check", "table"):
            results = load_json(args.results)
            plan = load_json(args.plan)

        if args.command == "check":
            violations = check(results, plan, plan_sha256(args.plan))
            if args.json:
                sys.stdout.write(
                    json.dumps(
                        {"ok": not violations, "violations": violations},
                        ensure_ascii=False,
                        indent=2,
                    )
                    + "\n"
                )
            else:
                for violation in violations:
                    sys.stdout.write(f"{violation['id']}: {violation['rule']} {violation['message']}\n")
            return 1 if violations else 0

        if args.command == "table":
            sys.stdout.write(table(results, plan))
            return 0

        # log
        try:
            recorded = log_run(args)
        except UsageError as exc:
            print(f"警告: {exc}", file=sys.stderr)
            return 2
        except ImportError as exc:
            print(f"警告: mlflow が無いようです: {exc}", file=sys.stderr)
            return 2
        if args.json:
            sys.stdout.write(json.dumps(recorded, ensure_ascii=False, indent=2) + "\n")
        else:
            sys.stdout.write(f"{recorded['run_id']}\n")
        return 0
    except UsageError as exc:
        print(f"警告: {exc}", file=sys.stderr)
        return 2
    except Exception as exc:  # 何が起きても、標準エラー 1 行と終了コード 1 で終わる。
        print(f"{WARNING_PREFIX}失敗しました: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())