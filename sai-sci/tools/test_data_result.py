#!/usr/bin/env python3
"""data_result.py のテスト。標準ライブラリだけで動く（mlflow は要らない）。"""

from __future__ import annotations

import copy
import hashlib
import io
import json
import os
import shutil
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import data_result  # noqa: E402


def make_plan() -> dict:
    """C4 の例と同じ形の、正しい計画。"""
    return {
        "study": "O6-ID130",
        "run_id": 131,
        "data_card": "AbCdEf12",
        "created_at": "2026-10-04T12:00:00+09:00",
        "alpha": 0.05,
        "seed": 20261004,
        "hypotheses": [
            {
                "id": "H1",
                "kind": "confirmatory",
                "question": "刺激群は対照群より RMSSD が高い",
                "outcome": "rmssd",
                "predictor": "group",
                "unit": "subject",
                "paradigm": "frequentist",
                "test": "welch_t",
                "alternative": "greater",
                "family": "F1",
                "effect": "hedges_g",
                "sesoi": 0.3,
                "assumptions": ["independence", "normality_of_residuals"],
                "fallback": "mann_whitney_u",
            },
            {
                "id": "H2",
                "kind": "exploratory",
                "question": "RMSSD は群で違うか",
                "outcome": "rmssd",
                "predictor": "group",
                "unit": "subject",
                "paradigm": "bayes",
                "test": "bayes_t",
                "alternative": "two-sided",
                "family": "F1",
                "effect": None,
                "sesoi": None,
                "assumptions": [],
                "fallback": None,
            },
            {
                "id": "H3",
                "kind": "exploratory",
                "question": "RMSSD を当てるか",
                "outcome": "rmssd",
                "predictor": "group",
                "unit": "subject",
                "paradigm": "ml",
                "test": "cv_classifier",
                "alternative": "two-sided",
                "family": None,
                "effect": None,
                "sesoi": None,
                "assumptions": [],
                "fallback": None,
            },
            {
                "id": "H4",
                "kind": "exploratory",
                "question": "RMSSD の平均はいくつか",
                "outcome": "rmssd",
                "predictor": None,
                "unit": "subject",
                "paradigm": "descriptive",
                "test": "mean",
                "alternative": "two-sided",
                "family": None,
                "effect": None,
                "sesoi": None,
                "assumptions": [],
                "fallback": None,
            },
        ],
        "families": [{"id": "F1", "correction": "holm", "members": ["H1", "H2"]}],
    }


def make_results(plan_hash: str) -> dict:
    """C5 の例と同じ形の、正しい結果。"""
    return {
        "study": "O6-ID130",
        "run_id": 131,
        "plan_sha256": plan_hash,
        "software": {"python": "3.12.6", "packages": {"scipy": "1.14.1", "pingouin": "0.5.5"}},
        "seed": 20261004,
        "results": [
            {
                "id": "H1",
                "paradigm": "frequentist",
                "test": "welch_t",
                "n": {"total": 98, "groups": {"stim": 50, "ctrl": 48}},
                "estimate": {"name": "mean_diff", "value": 4.2, "ci": [0.6, 7.8]},
                "statistic": {"name": "t", "value": 2.31, "df": 95.2},
                "p": 0.023,
                "p_adj": 0.046,
                "ci_level": 0.95,
                "effect": {"name": "hedges_g", "value": 0.47, "ci": [0.06, 0.87]},
                "assumptions": [{"name": "normality_of_residuals", "method": "qq", "ok": True, "note": ""}],
                "deviation": "",
                "figures": ["fig/H1.png"],
            },
            {
                "id": "H2",
                "paradigm": "bayes",
                "test": "bayes_t",
                "n": {"total": 98},
                "posterior": {"name": "mean_diff", "mean": 3.9, "hdi": [0.4, 7.1], "hdi_prob": 0.95},
                "diagnostics": {"rhat_max": 1.003, "ess_bulk_min": 1800, "divergences": 0},
                "assumptions": [],
                "deviation": "",
                "figures": [],
            },
            {
                "id": "H3",
                "paradigm": "ml",
                "test": "cv_classifier",
                "n": {"total": 98},
                "cv": {
                    "metric": "roc_auc",
                    "mean": 0.71,
                    "sd": 0.05,
                    "folds": 5,
                    "scheme": "GroupKFold(subject)",
                },
                "assumptions": [],
                "deviation": "",
                "figures": [],
            },
            {
                "id": "H4",
                "paradigm": "descriptive",
                "test": "mean",
                "n": {"total": 98},
                "estimate": {"name": "mean", "value": 52.3, "ci": [50.1, 54.5]},
                "assumptions": [],
                "deviation": "",
                "figures": [],
            },
        ],
    }


class FakeExperiment:
    def __init__(self, experiment_id):
        self.experiment_id = experiment_id


class FakeRunInfo:
    def __init__(self, run_id):
        self.run_id = run_id


class FakeRun:
    def __init__(self, run_id):
        self.info = FakeRunInfo(run_id)


class FakeClient:
    """呼び出しだけ記録する偽物の MlflowClient。"""

    def __init__(self):
        self.calls = []
        self.experiment_id = "17"

    def get_experiment_by_name(self, name):
        self.calls.append(("get_experiment_by_name", name))
        return None

    def create_experiment(self, name, artifact_location=None, tags=None):
        self.calls.append(("create_experiment", name, artifact_location, tags))
        return self.experiment_id

    def create_run(self, experiment_id, run_name=None, tags=None):
        self.calls.append(("create_run", experiment_id, run_name, tags))
        return FakeRun("run-abc")

    def log_param(self, run_id, key, value):
        self.calls.append(("log_param", run_id, key, value))

    def log_metric(self, run_id, key, value):
        self.calls.append(("log_metric", run_id, key, value))

    def log_artifact(self, run_id, path):
        self.calls.append(("log_artifact", run_id, path))

    def log_artifacts(self, run_id, path):
        self.calls.append(("log_artifacts", run_id, path))

    def set_terminated(self, run_id, status):
        self.calls.append(("set_terminated", run_id, status))


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="data-result-test-")
        self.plan = make_plan()
        self.plan_path = os.path.join(self.tmp, "plan.json")
        self.write_json(self.plan_path, self.plan)
        self.hash = data_result.plan_sha256(self.plan_path)
        self.results = make_results(self.hash)
        self.results_path = os.path.join(self.tmp, "results.json")
        self.write_json(self.results_path, self.results)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def write_json(self, path, data):
        with open(path, "w", encoding="utf-8") as fh:
            json.dump(data, fh, ensure_ascii=False)
        return path

    def violations(self, results=None, plan=None, plan_hash=None):
        return data_result.check(
            self.results if results is None else results,
            self.plan if plan is None else plan,
            self.hash if plan_hash is None else plan_hash,
        )

    def rules_hit(self, results=None, plan=None, plan_hash=None):
        return {v["rule"] for v in self.violations(results, plan, plan_hash)}

    def item(self, results, key):
        for entry in results["results"]:
            if entry.get("id") == key:
                return entry
        raise AssertionError(f"結果に {key} が無い")


class TestGoodInput(Base):
    def test_check_returns_no_violations(self):
        self.assertEqual(self.violations(), [])

    def test_check_returns_zero(self):
        self.assertEqual(self.main(["check", self.results_path, "--plan", self.plan_path]), 0)

    def test_table_returns_zero(self):
        self.assertEqual(self.main(["table", self.results_path, "--plan", self.plan_path]), 0)

    def main(self, argv):
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            return data_result.main(argv)


class TestRules(Base):
    """K1〜K9 のそれぞれが、1 つだけ壊した入力で出ることを確かめる。"""

    def test_k1_plan_sha256_mismatch(self):
        results = copy.deepcopy(self.results)
        results["plan_sha256"] = "0" * 64
        self.assertIn("K1", self.rules_hit(results))
        # id は "-"（どの仮説の違反でもないことを示す）
        viol = [v for v in self.violations(results) if v["rule"] == "K1"]
        self.assertEqual(viol[0]["id"], "-")

    def test_k2_confirmatory_missing(self):
        results = copy.deepcopy(self.results)
        results["results"] = [r for r in results["results"] if r["id"] != "H1"]
        hits = [v for v in self.violations(results) if v["rule"] == "K2"]
        self.assertTrue(hits)
        self.assertEqual(hits[0]["id"], "H1")

    def test_k3_unplanned_without_deviation(self):
        results = copy.deepcopy(self.results)
        extra = copy.deepcopy(self.item(results, "H4"))
        extra["id"] = "H9"
        extra["deviation"] = ""
        results["results"].append(extra)
        hits = [v for v in self.violations(results) if v["rule"] == "K3"]
        self.assertTrue(hits)
        self.assertEqual(hits[0]["id"], "H9")

    def test_k3_unplanned_with_deviation_is_ok(self):
        results = copy.deepcopy(self.results)
        extra = copy.deepcopy(self.item(results, "H4"))
        extra["id"] = "H9"
        extra["deviation"] = "探索ammaで追加した"
        results["results"].append(extra)
        self.assertNotIn("K3", self.rules_hit(results))

    def test_k4_test_changed_without_deviation(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H1")["test"] = "mann_whitney_u"
        hits = [v for v in self.violations(results) if v["rule"] == "K4"]
        self.assertTrue(hits)
        self.assertEqual(hits[0]["id"], "H1")

    def test_k4_fallback_without_failed_assumption(self):
        results = copy.deepcopy(self.results)
        item = self.item(results, "H1")
        item["test"] = "mann_whitney_u"  # 計画の fallback
        item["deviation"] = "正規性を仮定できないため"
        # assumptions の ok が全部 true のまま
        self.assertIn("K4", self.rules_hit(results))

    def test_k4_fallback_with_failed_assumption_is_ok(self):
        results = copy.deepcopy(self.results)
        item = self.item(results, "H1")
        item["test"] = "mann_whitney_u"
        item["deviation"] = "正規性を仮定できないため"
        item["assumptions"] = [{"name": "normality_of_residuals", "method": "qq", "ok": False, "note": ""}]
        self.assertNotIn("K4", self.rules_hit(results))

    def test_k5_p_out_of_range(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H1")["p"] = 1.4
        self.assertIn("K5", self.rules_hit(results))

    def test_k5_effect_ci_wrong_order(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H1")["effect"]["ci"] = [7.8, 0.6]
        self.assertIn("K5", self.rules_hit(results))

    def test_k5_effect_ci_not_two_numbers(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H1")["effect"]["ci"] = [0.6]
        self.assertIn("K5", self.rules_hit(results))

    def test_k5_ci_level_out_of_range(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H1")["ci_level"] = 1.5
        self.assertIn("K5", self.rules_hit(results))

    def test_k5_confirmatory_without_assumptions(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H1")["assumptions"] = []
        self.assertIn("K5", self.rules_hit(results))

    def test_k5_missing_effect(self):
        results = copy.deepcopy(self.results)
        del self.item(results, "H1")["effect"]
        self.assertIn("K5", self.rules_hit(results))

    def test_k6_missing_p_adj(self):
        results = copy.deepcopy(self.results)
        del self.item(results, "H1")["p_adj"]
        hits = [v for v in self.violations(results) if v["rule"] == "K6"]
        self.assertTrue(hits)
        self.assertEqual(hits[0]["id"], "H1")

    def test_k6_p_adj_smaller_than_p(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H1")["p_adj"] = 0.001
        self.assertIn("K6", self.rules_hit(results))

    def test_k6_not_required_for_singleton_family(self):
        plan = copy.deepcopy(self.plan)
        plan["families"] = [{"id": "F1", "correction": "holm", "members": ["H1"]}]
        results = copy.deepcopy(self.results)
        del self.item(results, "H1")["p_adj"]
        self.assertNotIn("K6", self.rules_hit(results, plan=plan))

    def test_k6_not_required_when_correction_is_none(self):
        plan = copy.deepcopy(self.plan)
        plan["families"] = [{"id": "F1", "correction": "none", "members": ["H1", "H2"]}]
        results = copy.deepcopy(self.results)
        del self.item(results, "H1")["p_adj"]
        self.assertNotIn("K6", self.rules_hit(results, plan=plan))

    def test_k7_rhat_too_large(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H2")["diagnostics"]["rhat_max"] = 1.2
        self.assertIn("K7", self.rules_hit(results))

    def test_k7_ess_too_small(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H2")["diagnostics"]["ess_bulk_min"] = 120
        self.assertIn("K7", self.rules_hit(results))

    def test_k7_divergences(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H2")["diagnostics"]["divergences"] = 3
        self.assertIn("K7", self.rules_hit(results))

    def test_k7_hdi_wrong_order(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H2")["posterior"]["hdi"] = [7.1, 0.4]
        self.assertIn("K7", self.rules_hit(results))

    def test_k7_missing_diagnostics(self):
        results = copy.deepcopy(self.results)
        del self.item(results, "H2")["diagnostics"]
        self.assertIn("K7", self.rules_hit(results))

    def test_k8_folds_too_few(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H3")["cv"]["folds"] = 1
        self.assertIn("K8", self.rules_hit(results))

    def test_k8_empty_scheme(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H3")["cv"]["scheme"] = ""
        self.assertIn("K8", self.rules_hit(results))

    def test_k8_missing_cv(self):
        results = copy.deepcopy(self.results)
        del self.item(results, "H3")["cv"]
        self.assertIn("K8", self.rules_hit(results))

    def test_k9_missing_estimate(self):
        results = copy.deepcopy(self.results)
        del self.item(results, "H4")["estimate"]
        self.assertIn("K9", self.rules_hit(results))

    def test_k9_estimate_not_finite(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H4")["estimate"]["value"] = "高い"
        self.assertIn("K9", self.rules_hit(results))

    def test_all_nine_rules_are_covered(self):
        """K1〜K9 のすべてを 1 つ以上確かめていること。"""
        covered = set()
        cases = []
        r = copy.deepcopy(self.results)
        r["plan_sha256"] = "0" * 64
        cases.append(r)

        r = copy.deepcopy(self.results)
        r["results"] = [x for x in r["results"] if x["id"] != "H1"]
        cases.append(r)

        r = copy.deepcopy(self.results)
        extra = copy.deepcopy(self.item(r, "H4"))
        extra["id"] = "H9"
        r["results"].append(extra)
        cases.append(r)

        r = copy.deepcopy(self.results)
        self.item(r, "H1")["test"] = "mann_whitney_u"
        cases.append(r)

        r = copy.deepcopy(self.results)
        self.item(r, "H1")["p"] = 1.4
        cases.append(r)

        r = copy.deepcopy(self.results)
        del self.item(r, "H1")["p_adj"]
        cases.append(r)

        r = copy.deepcopy(self.results)
        self.item(r, "H2")["diagnostics"]["rhat_max"] = 1.2
        cases.append(r)

        r = copy.deepcopy(self.results)
        self.item(r, "H3")["cv"]["folds"] = 1
        cases.append(r)

        r = copy.deepcopy(self.results)
        del self.item(r, "H4")["estimate"]
        cases.append(r)

        for one in cases:
            covered |= self.rules_hit(one)
        self.assertEqual(covered, {"K1", "K2", "K3", "K4", "K5", "K6", "K7", "K8", "K9"})


class TestUsageErrors(Base):
    def run_main(self, argv):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = data_result.main(argv)
        return code, out.getvalue(), err.getvalue()

    def test_missing_file(self):
        code, _, err = self.run_main(
            ["check", os.path.join(self.tmp, "無い.json"), "--plan", self.plan_path]
        )
        self.assertEqual(code, 2)
        self.assertTrue(err.strip())

    def test_broken_json(self):
        broken = os.path.join(self.tmp, "broken.json")
        with open(broken, "w", encoding="utf-8") as fh:
            fh.write("{これは JSON ではない")
        code, _, err = self.run_main(["check", broken, "--plan", self.plan_path])
        self.assertEqual(code, 2)
        self.assertTrue(err.strip())

    def test_missing_plan(self):
        code, _, _ = self.run_main(
            ["check", self.results_path, "--plan", os.path.join(self.tmp, "無い.json")]
        )
        self.assertEqual(code, 2)

    def test_json_not_an_object(self):
        path = os.path.join(self.tmp, "list.json")
        with open(path, "w", encoding="utf-8") as fh:
            fh.write("[1, 2, 3]")
        code, _, _ = self.run_main(["check", path, "--plan", self.plan_path])
        self.assertEqual(code, 2)

    def test_violations_exit_one(self):
        results = copy.deepcopy(self.results)
        results["plan_sha256"] = "0" * 64
        path = self.write_json(os.path.join(self.tmp, "bad.json"), results)
        code, out, _ = self.run_main(["check", path, "--plan", self.plan_path])
        self.assertEqual(code, 1)
        self.assertIn("K1", out)

    def test_check_json_output(self):
        results = copy.deepcopy(self.results)
        results["plan_sha256"] = "0" * 64
        path = self.write_json(os.path.join(self.tmp, "bad.json"), results)
        code, out, _ = self.run_main(["check", path, "--plan", self.plan_path, "--json"])
        self.assertEqual(code, 1)
        payload = json.loads(out)
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["violations"][0]["rule"], "K1")
        self.assertIn("id", payload["violations"][0])
        self.assertIn("message", payload["violations"][0])

    def test_no_command(self):
        code, _, _ = self.run_main([])
        self.assertEqual(code, 2)

    def test_check_without_plan(self):
        code, _, _ = self.run_main(["check", self.results_path])
        self.assertEqual(code, 2)


class TestTable(Base):
    def render(self):
        return data_result.table(self.results, self.plan)

    def test_header_and_divider(self):
        lines = self.render().split("\n")
        self.assertTrue(lines[0].startswith("| ID | 種類 | 検定 | n |"))
        self.assertIn("効果量", lines[0])
        self.assertIn("計画からの変更", lines[0])
        self.assertEqual(lines[1], "|---|---|---|---|---|---|---|---|---|")

    def test_one_row_per_hypothesis(self):
        rows = [ln for ln in self.render().split("\n") if ln.startswith("| H")]
        self.assertEqual(len(rows), len(self.results["results"]))

    def test_kind_labels(self):
        text = self.render()
        self.assertIn("確証", text)
        self.assertIn("探索", text)

    def test_unplanned_label_and_change(self):
        results = copy.deepcopy(self.results)
        extra = copy.deepcopy(self.item(results, "H4"))
        extra["id"] = "H9"
        extra["deviation"] = "計画の外で追加した"
        results["results"].append(extra)
        text = data_result.table(results, self.plan)
        row = [ln for ln in text.split("\n") if ln.startswith("| H9")][0]
        self.assertIn("計画外", row)
        self.assertIn("計画の外で追加した", row)

    def test_empty_change_is_none(self):
        row = [ln for ln in self.render().split("\n") if ln.startswith("| H1")][0]
        self.assertTrue(row.rstrip().endswith("なし |"))

    def test_bayes_estimate_cell(self):
        row = [ln for ln in self.render().split("\n") if ln.startswith("| H2")][0]
        self.assertIn("事後平均 3.9", row)
        self.assertIn("HDI95%", row)
        self.assertIn("[0.4, 7.1]", row)

    def test_ml_estimate_cell(self):
        row = [ln for ln in self.render().split("\n") if ln.startswith("| H3")][0]
        self.assertIn("roc_auc 0.71 ± 0.05", row)
        self.assertIn("GroupKFold(subject)", row)

    def test_small_p_form(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H1")["p"] = 0.00002
        row = [ln for ln in data_result.table(results, self.plan).split("\n") if ln.startswith("| H1")][0]
        self.assertIn("p < 0.001", row)

    def test_missing_value_mark(self):
        row = [ln for ln in self.render().split("\n") if ln.startswith("| H2")][0]
        self.assertIn("―", row)

    def test_three_significant_digits(self):
        results = copy.deepcopy(self.results)
        self.item(results, "H1")["estimate"]["value"] = 4.23456
        row = [ln for ln in data_result.table(results, self.plan).split("\n") if ln.startswith("| H1")][0]
        self.assertIn("4.23", row)


class TestMetricsAndParams(Base):
    def test_metrics_names(self):
        metrics = data_result.metrics_of(self.results)
        self.assertEqual(metrics["H1.p"], 0.023)
        self.assertEqual(metrics["H1.p_adj"], 0.046)
        self.assertEqual(metrics["H1.effect"], 0.47)
        self.assertEqual(metrics["H1.effect_ci_low"], 0.06)
        self.assertEqual(metrics["H1.effect_ci_high"], 0.87)
        self.assertEqual(metrics["H1.estimate"], 4.2)
        self.assertEqual(metrics["H2.post_mean"], 3.9)
        self.assertEqual(metrics["H2.hdi_low"], 0.4)
        self.assertEqual(metrics["H2.hdi_high"], 7.1)
        self.assertEqual(metrics["H2.rhat_max"], 1.003)
        self.assertEqual(metrics["H3.cv_mean"], 0.71)
        self.assertEqual(metrics["H3.cv_sd"], 0.05)
        self.assertEqual(metrics["H1.n"], 98)

    def test_metrics_skip_missing(self):
        self.assertNotIn("H2.p", data_result.metrics_of(self.results))

    def test_params(self):
        params = data_result.params_of(self.plan)
        self.assertEqual(params["alpha"], "0.05")
        self.assertEqual(params["seed"], "20261004")
        self.assertEqual(params["H1.test"], "welch_t")
        self.assertEqual(params["H1.kind"], "confirmatory")
        self.assertEqual(params["H1.correction"], "holm")
        self.assertEqual(params["H3.correction"], "none")  # family が無い


class FakeArgs:
    def __init__(self, tmp):
        self.experiment = "west/ID130/data"
        self.run_name = "ID131-test"
        self.project = "west"
        self.run_id = 131
        self.study = "O6-ID130"
        self.node = 6
        self.kind = "test"
        self.plan = os.path.join(tmp, "plan.json")
        self.results = os.path.join(tmp, "results.json")
        self.artifact = []
        self.tracking_uri = "http://example.invalid"


class TestLogRun(Base):
    """偽物の client で記録の流れを確かめる。mlflow は使わない。"""

    def setUp(self):
        super().setUp()
        self.client = FakeClient()
        self.args = FakeArgs(self.tmp)
        # 記録 #: 渡したときは mlflow を一切使わないことを確かめるため
        self.saved = {k: v for k, v in sys.modules.items() if k == "mlflow" or k.startswith("mlflow.")}
        for k in self.saved:
            sys.modules.pop(k, None)

    def tearDown(self):
        super().tearDown()
        sys.modules.update(self.saved)

    def calls(self, name):
        return [c for c in self.client.calls if c[0] == name]

    def test_returns_run_id(self):
        out = data_result.log_run(self.args, client=self.client)
        self.assertEqual(out["run_id"], "run-abc")
        self.assertEqual(out["run_name"], "ID131-test")
        self.assertEqual(out["experiment_id"], "17")

    def test_does_not_import_mlflow(self):
        data_result.log_run(self.args, client=self.client)
        self.assertNotIn("mlflow", sys.modules)

    def test_creates_experiment_with_tags(self):
        data_result.log_run(self.args, client=self.client)
        created = self.calls("create_experiment")
        self.assertEqual(len(created), 1)
        self.assertEqual(created[0][1], "west/ID130/data")
        tags = created[0][3]
        self.assertEqual(tags["sai.project"], "west")
        self.assertEqual(tags["exp.study"], "O6-ID130")
        self.assertEqual(tags["sci.node"], "6")
        self.assertEqual(tags["exp.kind"], "data")

    def test_run_tags(self):
        data_result.log_run(self.args, client=self.client)
        created = self.calls("create_run")
        self.assertEqual(len(created), 1)
        self.assertEqual(created[0][2], "ID131-test")
        tags = created[0][3]
        self.assertEqual(tags["sai.project"], "west")
        self.assertEqual(tags["sai.run_id"], "131")
        self.assertEqual(tags["exp.study"], "O6-ID130")
        self.assertEqual(tags["sci.node"], "6")
        self.assertEqual(tags["exp.kind"], "test")

    def test_params_logged(self):
        data_result.log_run(self.args, client=self.client)
        names = [c[2] for c in self.calls("log_param")]
        for wanted in ("alpha", "seed", "H1.test", "H1.kind", "H1.correction"):
            self.assertIn(wanted, names)

    def test_metrics_logged(self):
        data_result.log_run(self.args, client=self.client)
        names = [c[2] for c in self.calls("log_metric")]
        self.assertIn("H1.p", names)
        self.assertIn("H1.p_adj", names)
        self.assertIn("H1.effect", names)
        self.assertIn("H2.post_mean", names)
        self.assertIn("H3.cv_mean", names)

    def test_artifacts_logged(self):
        self.args.artifact = [self.tmp]
        out = data_result.log_run(self.args, client=self.client)
        # plan と results の 2 つ + folder 1 つ
        self.assertEqual(out["artifacts"], 3)
        self.assertEqual(len(self.calls("log_artifact")), 2)
        self.assertEqual(len(self.calls("log_artifacts")), 1)

    def test_terminated_as_finished(self):
        data_result.log_run(self.args, client=self.client)
        calls = self.calls("set_terminated")
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0][2], "FINISHED")

    def test_terminated_as_failed_on_error(self):
        class Boom(FakeClient):
            def log_param(self, run_id, key, value):
                raise RuntimeError(" purposely")

        client = Boom()
        with self.assertRaises(RuntimeError):
            data_result.log_run(self.args, client=client)
        terminated = [c for c in client.calls if c[0] == "set_terminated"]
        self.assertEqual(len(terminated), 1)
        self.assertEqual(terminated[0][2], "FAILED")

    def log_argv(self):
        return [
            "log",
            "--experiment", "west/ID130/data",
            "--run-name", "ID131-test",
            "--project", "west",
            "--run-id", "131",
            "--study", "O6-ID130",
            "--node", "6",
            "--kind", "test",
            "--plan", self.plan_path,
            "--results", self.results_path,
        ]

    def test_log_command_prints_run_id_as_first_line(self):
        recorded = {
            "experiment_id": "17",
            "run_id": "run-abc",
            "run_name": "ID131-test",
            "metrics": 10,
            "artifacts": 2,
        }
        original = data_result.log_run
        data_result.log_run = lambda args: recorded
        out = io.StringIO()
        try:
            with redirect_stdout(out):
                code = data_result.main(self.log_argv())
        finally:
            data_result.log_run = original
        self.assertEqual(code, 0)
        self.assertEqual(out.getvalue().split("\n")[0], "run-abc")

    def test_log_command_json_output(self):
        recorded = {
            "experiment_id": "17",
            "run_id": "run-abc",
            "run_name": "ID131-test",
            "metrics": 10,
            "artifacts": 2,
        }
        original = data_result.log_run
        data_result.log_run = lambda args: recorded
        out = io.StringIO()
        try:
            with redirect_stdout(out):
                code = data_result.main(self.log_argv() + ["--json"])
        finally:
            data_result.log_run = original
        self.assertEqual(code, 0)
        payload = json.loads(out.getvalue())
        self.assertEqual(payload["run_id"], "run-abc")
        self.assertEqual(payload["metrics"], 10)
        self.assertEqual(payload["artifacts"], 2)

    def test_log_without_mlflow_ends_with_two(self):
        """mlflow が無いときは理由を出して 2 で終える。"""
        original = data_result.log_run
        data_result.log_run = lambda args: (_ for _ in ()).throw(ImportError("No module named 'mlflow'"))
        err = io.StringIO()
        try:
            with redirect_stdout(io.StringIO()), redirect_stderr(err):
                code = data_result.main(self.log_argv())
        finally:
            data_result.log_run = original
        self.assertEqual(code, 2)
        self.assertIn("mlflow", err.getvalue())


class TestLogRunArgs(Base):
    """log_run の引数の試し（client を渡さない経路）。"""

    def test_no_tracking_uri_is_usage_error(self):
        args = FakeArgs(self.tmp)
        args.tracking_uri = None
        saved = os.environ.pop("MLFLOW_TRACKING_URI", None)
        try:
            with self.assertRaises(data_result.UsageError):
                data_result.log_run(args)
        finally:
            if saved is not None:
                os.environ["MLFLOW_TRACKING_URI"] = saved

    def test_main_without_tracking_uri_ends_with_two(self):
        args = ["log",
            "--experiment", "west/ID130/data",
            "--run-name", "ID131-test",
            "--project", "west",
            "--run-id", "131",
            "--study", "O6-ID130",
            "--node", "6",
            "--kind", "eda"]
        saved = os.environ.pop("MLFLOW_TRACKING_URI", None)
        err = io.StringIO()
        try:
            with redirect_stdout(io.StringIO()), redirect_stderr(err):
                code = data_result.main(args)
        finally:
            if saved is not None:
                os.environ["MLFLOW_TRACKING_URI"] = saved
        self.assertEqual(code, 2)
        self.assertIn("接続先", err.getvalue())


if __name__ == "__main__":
    unittest.main()