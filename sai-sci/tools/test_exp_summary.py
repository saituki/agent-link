"""台帳 `exp_summary.py` のテスト。

sqlite の一時ストアに MLflow の記録（親 run と入れ子の子 run）を作り、台帳の
各欄を確かめる。中央の MLflow サーバには書き込まない。

    cd sai-sci/tools && uv run --with mlflow python -m unittest test_exp_summary -v
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from mlflow.tracking import MlflowClient

SCRIPT = Path(__file__).resolve().parent / "exp_summary.py"

# テストと確認で共有する値。
STUDY = "N12-r45"
OTHER_STUDY = "N99-r99"
METRIC = "val_auc"
EXPERIMENT = "local"


class Store:
    """sqlite の mlflow ストアに run を作っていくための道具。

    入れ子の run を作る代わりに、exp.role タグで親と子を区別する。
    実際のスクリプトは `mlflow.start_run(nested=True)` を使う（C1）。
    """

    def __init__(self, root: Path, experiment: str = EXPERIMENT) -> None:
        self.root = root
        self.experiment = experiment
        (root / "artifacts").mkdir(parents=True, exist_ok=True)
        self.uri = f"sqlite:///{root / 'mlflow.db'}"
        client = MlflowClient(tracking_uri=self.uri)
        client.create_experiment(
            experiment, artifact_location=(root / "artifacts" / experiment).as_uri()
        )
        self.client = client
        self.experiment_id = client.get_experiment_by_name(experiment).experiment_id
        self.clock = 1_700_000_000_000

    def _next_time(self) -> int:
        self.clock += 1000
        return self.clock

    def _tags(self, study: str, script: str, round_no: int, action: str,
              parent_script: str, role: str, generation: int | None = None,
              with_round: bool = False) -> dict:
        tags = {
            "exp.study": study,
            "exp.script": script,
            "exp.action": action,
            "exp.parent_script": parent_script,
            "exp.role": role,
            "sci.node": study.split("-r")[0],
            "sai.run_id": "47",
        }
        # generation を渡したら新しい記録の形（exp.generation だけ）にする。
        # with_round も真なら両方のタグを付けるので、優先順位を試せる。
        if generation is not None:
            tags["exp.generation"] = str(generation)
        if generation is None or with_round:
            tags["exp.round"] = str(round_no)
        return tags

    def script(self, script: str, round_no: int, action: str, parent_script: str = "",
               study: str = STUDY, generation: int | None = None,
               with_round: bool = False) -> str:
        """親 run（1 回のスクリプトの実行）を作る。"""
        run = self.client.create_run(
            self.experiment_id,
            start_time=self._next_time(),
            run_name=f"{script}-r{round_no}",
            tags=self._tags(study, script, round_no, action, parent_script, "script",
                            generation, with_round),
        )
        self.client.set_terminated(run.info.run_id, status="FINISHED")
        return run.info.run_id

    def trial(self, script: str, round_no: int, value: float | None, params: dict,
              failed: bool = False, action: str = "tune", study: str = STUDY,
              metric: str = METRIC, generation: int | None = None,
              with_round: bool = False) -> str:
        """子 run（パラメータの組 1 つ = 試行）を作る。"""
        tags = self._tags(study, script, round_no, action, "", "trial",
                          generation, with_round)
        if failed:
            tags["exp.error"] = "CUDA out of memory"
        run = self.client.create_run(
            self.experiment_id, start_time=self._next_time(),
            run_name=f"{script}-r{round_no}", tags=tags,
        )
        metrics = []
        if value is not None:
            metrics.append(mlflow_metric(metric, value))
        self.client.log_batch(
            run.info.run_id,
            metrics=metrics,
            params=[mlflow_param(name, str(v)) for name, v in sorted(params.items())],
        )
        self.client.set_terminated(
            run.info.run_id, status="FAILED" if failed else "FINISHED"
        )
        return run.info.run_id


def mlflow_metric(name: str, value: float):
    from mlflow.entities import Metric

    return Metric(name, value, 1_700_000_000_000, 0)


def mlflow_param(name: str, value: str):
    from mlflow.entities import Param

    return Param(name, value)


def two_scripts_three_rounds(store: Store) -> None:
    """2 スクリプト × 3 round の記録。

    - s01: round 2 で改善し、round 3 で改善しない。lr の最大値（0.3）で最良。
    - s02: 親は s01。1 試行が FAILED。round 6 に漏洩の疑いがある値（0.9995）。
    """
    store.script("s01", 1, "draft")
    store.trial("s01", 1, 0.70, {"lr": "0.01", "depth": "2"}, action="draft")
    store.trial("s01", 1, 0.72, {"lr": "0.03", "depth": "2"}, action="draft")
    store.trial("s01", 1, 0.75, {"lr": "0.1", "depth": "2"}, action="draft")
    store.script("s01", 2, "tune")
    store.trial("s01", 2, 0.80, {"lr": "0.2", "depth": "2"})
    store.trial("s01", 2, 0.82, {"lr": "0.3", "depth": "2"})
    store.script("s01", 3, "tune")
    store.trial("s01", 3, 0.815, {"lr": "0.2", "depth": "2"})
    store.trial("s01", 3, 0.81, {"lr": "0.25", "depth": "2"})

    store.script("s02", 4, "new", parent_script="s01")
    store.trial("s02", 4, 0.60, {"lr": "0.1", "depth": "3"}, action="new")
    store.trial("s02", 4, 0.55, {"lr": "0.5", "depth": "3"}, action="new")
    store.trial("s02", 4, None, {"lr": "0.9", "depth": "3"}, failed=True, action="new")
    store.script("s02", 5, "extend", parent_script="s01")
    store.trial("s02", 5, 0.65, {"lr": "0.1", "depth": "3"}, action="extend")
    store.trial("s02", 5, 0.66, {"lr": "0.5", "depth": "3"}, action="extend")
    store.script("s02", 6, "extend", parent_script="s01")
    store.trial("s02", 6, 0.9995, {"lr": "0.2", "depth": "3"}, action="extend")
    store.trial("s02", 6, 0.67, {"lr": "0.5", "depth": "3"}, action="extend")


def two_scripts_in_one_generation(store: Store) -> None:
    """1 世代に script 2 本がある記録（新しい実験ループの形）。`exp.generation` だけを使う。

    - 世代 1: s01 が 0.72、s02 が 0.55。最良は s01。
    - 世代 2: s01 だけで 0.80。
    """
    store.script("s01", 1, "draft", generation=1)
    store.trial("s01", 1, 0.70, {"lr": "0.1"}, action="draft", generation=1)
    store.trial("s01", 1, 0.72, {"lr": "0.2"}, action="draft", generation=1)
    store.script("s02", 1, "new", parent_script="s01", generation=1)
    store.trial("s02", 1, 0.55, {"lr": "0.3"}, action="new", generation=1)
    store.script("s01", 2, "tune", generation=2)
    store.trial("s01", 2, 0.80, {"lr": "0.4"}, generation=2)


class ExpSummaryTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)

    def store(self) -> Store:
        return Store(self.root)

    def run_cmd(self, *args: str, tracking_uri: str | None = None) -> subprocess.CompletedProcess:
        """exp_summary.py をこのテストの python で実行する。"""
        env = dict(os.environ)
        env.pop("MLFLOW_TRACKING_URI", None)
        env["_MLFLOW_SERVER_ARTIFACT_ROOT"] = (self.root / "artifacts").as_uri()
        cmd = [sys.executable, str(SCRIPT), *args]
        if tracking_uri is not None:
            cmd += ["--tracking-uri", tracking_uri]
        return subprocess.run(
            cmd, capture_output=True, text=True, env=env, cwd=self.root, timeout=300
        )

    def summary(self, store: Store, *args: str) -> dict:
        """台帳の JSON を標準出力から取り出す。"""
        done = self.run_cmd(
            "--study", STUDY, "--metric", METRIC, "--direction", "maximize",
            *args, tracking_uri=store.uri,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        return json.loads(done.stdout)

    def summary_quiet(self, store: Store, *args: str) -> dict:
        """台帳の JSON を取り出す。警告が 1 行も出ないことも確かめる。"""
        done = self.run_cmd(
            "--study", STUDY, "--metric", METRIC, "--direction", "maximize",
            *args, tracking_uri=store.uri,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertNotIn("警告", done.stderr, f"警告が出た:\n{done.stderr}")
        return json.loads(done.stdout)

    def main_summary(self) -> dict:
        store = self.store()
        two_scripts_three_rounds(store)
        return self.summary(store)

    def script_of(self, summary: dict, script: str) -> dict:
        return next(s for s in summary["scripts"] if s["script"] == script)

    def round_of(self, summary: dict, round_no: int) -> dict:
        return next(r for r in summary["rounds"] if r["round"] == round_no)

    # --- 計算の規則 -------------------------------------------------------

    def test_round2_up_round3_no_improvement(self) -> None:
        """round 2 で改善し、round 3 で改善しない → improved false、stale_rounds 1。"""
        summary = self.main_summary()
        self.assertTrue(self.round_of(summary, 1)["improved"])
        self.assertTrue(self.round_of(summary, 2)["improved"])
        self.assertFalse(self.round_of(summary, 3)["improved"])
        self.assertEqual(self.script_of(summary, "s01")["stale_rounds"], 1)
        self.assertEqual(self.script_of(summary, "s02")["stale_rounds"], 0)

    def test_min_delta_changes_improvement(self) -> None:
        """--min-delta を大きくすると、改善とみなさない round が増える。"""
        store = self.store()
        two_scripts_three_rounds(store)
        summary = self.summary(store, "--min-delta", "0.1")
        # 0.75 → 0.82 は 0.07 しか変わらないので改善ではない。
        self.assertFalse(self.round_of(summary, 2)["improved"])
        self.assertEqual(self.script_of(summary, "s01")["stale_rounds"], 2)

    def test_param_edges(self) -> None:
        """最良が lr の最大値 → {"script": "s01", "param": "lr", "side": "max"}。"""
        summary = self.main_summary()
        self.assertEqual(
            summary["param_edges"], [{"script": "s01", "param": "lr", "side": "max"}]
        )

    def test_all_failed_script(self) -> None:
        """全試行が FAILED のスクリプト → trials == failed、best は null。"""
        store = self.store()
        store.script("s09", 1, "debug")
        store.trial("s09", 1, None, {"lr": "0.1"}, failed=True, action="debug")
        store.trial("s09", 1, None, {"lr": "0.2"}, failed=True, action="debug")
        summary = self.summary(store)
        script = self.script_of(summary, "s09")
        self.assertEqual(script["trials"], 2)
        self.assertEqual(script["failed"], 2)
        self.assertIsNone(script["best"])
        self.assertIsNone(script["best_params"])
        self.assertEqual(script["debug_rounds"], 1)
        self.assertIsNone(summary["best"])
        self.assertFalse(self.round_of(summary, 1)["improved"])
        self.assertEqual(summary["last"], {
            "round": 1, "script": "s09", "action": "debug", "scripts": ["s09"],
            "trials": 2, "failed": 2,
        })
        # 規則 R4 の判定材料（最後の世代が全部失敗）。
        self.assertTrue(summary["last"]["trials"] > 0)
        self.assertEqual(summary["last"]["failed"], summary["last"]["trials"])

    def test_best_and_last(self) -> None:
        """best はいちばんよい成功試行、last はいちばん大きい round。"""
        summary = self.main_summary()
        self.assertEqual(summary["best"]["script"], "s02")
        self.assertEqual(summary["best"]["round"], 6)
        self.assertEqual(summary["best"]["value"], 0.9995)
        self.assertEqual(summary["best"]["params"], {"lr": "0.2", "depth": "3"})
        self.assertEqual(summary["last"], {
            "round": 6, "script": "s02", "action": "extend", "scripts": ["s02"],
            "trials": 2, "failed": 0,
        })
        # 最後の世代は全部失敗ではない（規則 R4）。
        self.assertNotEqual(summary["last"]["failed"], summary["last"]["trials"])
        self.assertEqual([r["round"] for r in summary["rounds"]], [1, 2, 3, 4, 5, 6])
        self.assertFalse(summary["ablation_done"])

    def test_scripts_entries(self) -> None:
        """scripts の欄（action_first / parent_script / trials / failed / best）。"""
        summary = self.main_summary()
        s01 = self.script_of(summary, "s01")
        self.assertEqual(s01["action_first"], "draft")
        self.assertEqual(s01["parent_script"], "")
        self.assertEqual(s01["rounds"], [1, 2, 3])
        self.assertEqual(s01["trials"], 7)
        self.assertEqual(s01["failed"], 0)
        self.assertEqual(s01["best"], 0.82)
        self.assertEqual(s01["best_params"], {"lr": "0.3", "depth": "2"})
        self.assertEqual(s01["debug_rounds"], 0)
        s02 = self.script_of(summary, "s02")
        self.assertEqual(s02["action_first"], "new")
        self.assertEqual(s02["parent_script"], "s01")
        self.assertEqual(s02["failed"], 1)

    def test_best_so_far_is_cumulative(self) -> None:
        """best_so_far は round 1 から見た累積の最良。"""
        summary = self.main_summary()
        self.assertEqual(self.round_of(summary, 1)["best_so_far"], 0.75)
        self.assertEqual(self.round_of(summary, 3)["best_so_far"], 0.82)
        self.assertEqual(self.round_of(summary, 5)["best_so_far"], 0.82)
        self.assertEqual(self.round_of(summary, 6)["best_so_far"], 0.9995)
        self.assertTrue(self.round_of(summary, 6)["improved"])

    def test_leak_suspects_maximize(self) -> None:
        """maximize で 0.9995 → leak_suspects に 1 件。"""
        summary = self.main_summary()
        self.assertEqual(len(summary["leak_suspects"]), 1)
        suspect = summary["leak_suspects"][0]
        self.assertEqual(suspect["script"], "s02")
        self.assertEqual(suspect["round"], 6)
        self.assertEqual(suspect["value"], 0.9995)

    def test_leak_suspects_minimize(self) -> None:
        """minimize でも 1 件。"""
        store = self.store()
        store.script("s01", 1, "draft")
        store.trial("s01", 1, 0.5, {"lr": "0.1"}, metric="val_loss", action="draft")
        store.trial("s01", 1, 1e-7, {"lr": "0.2"}, metric="val_loss", action="draft")
        done = self.run_cmd(
            "--study", STUDY, "--metric", "val_loss", "--direction", "minimize",
            tracking_uri=store.uri,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        summary = json.loads(done.stdout)
        self.assertEqual(summary["best"]["value"], 1e-7)
        self.assertEqual(len(summary["leak_suspects"]), 1)
        self.assertEqual(summary["leak_suspects"][0]["value"], 1e-7)

    def test_leak_threshold_option(self) -> None:
        """--leak-max を上げると、漏洩の疑いは無くなる。"""
        store = self.store()
        two_scripts_three_rounds(store)
        summary = self.summary(store, "--leak-max", "0.9999")
        self.assertEqual(summary["leak_suspects"], [])

    def test_ablation_done(self) -> None:
        """exp.action=ablation の親 run があれば ablation_done == true。"""
        store = self.store()
        store.script("s01", 1, "draft")
        store.trial("s01", 1, 0.75, {"lr": "0.1"}, action="draft")
        store.script("s01", 2, "ablation")
        store.trial("s01", 2, 0.74, {"lr": "0.1"}, action="ablation")
        summary = self.summary(store)
        self.assertTrue(summary["ablation_done"])

    def test_duplicate_script_in_round_warns(self) -> None:
        """同じ世代・同じ script の親 run が 2 つあれば警告を出し、開始時刻が新しいほうを採る。"""
        store = self.store()
        store.script("s01", 1, "draft")
        store.trial("s01", 1, 0.75, {"lr": "0.1"}, action="draft")
        store.script("s01", 1, "tune")  # あとから作られた、同じ世代・同じ script の親 run
        done = self.run_cmd(
            "--study", STUDY, "--metric", METRIC, "--direction", "maximize",
            tracking_uri=store.uri,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("警告: ", done.stderr)
        summary = json.loads(done.stdout)
        self.assertEqual(len(summary["rounds"]), 1)
        self.assertEqual(summary["rounds"][0]["scripts"], ["s01"])
        self.assertEqual(summary["rounds"][0]["action"], "tune")

    def test_two_scripts_in_one_round_have_no_warning(self) -> None:
        """同じ round でも script が違う親は 2 つとも数え、警告は出さない。"""
        store = self.store()
        store.script("s01", 1, "draft")
        store.trial("s01", 1, 0.75, {"lr": "0.1"}, action="draft")
        store.script("s02", 1, "new")  # 同じ round・違う script の親 run
        summary = self.summary_quiet(store)
        self.assertEqual(len(summary["rounds"]), 1)
        self.assertEqual(summary["rounds"][0]["scripts"], ["s01", "s02"])
        self.assertEqual(summary["last"]["scripts"], ["s01", "s02"])

    # --- 世代（exp.generation）で数える -------------------------------------

    def test_generation_counts_every_script(self) -> None:
        """世代 1 に script 2 本、世代 2 に 1 本 → rounds は 2 行、1 行目は 2 本分。"""
        store = self.store()
        two_scripts_in_one_generation(store)
        summary = self.summary_quiet(store)
        self.assertEqual([r["round"] for r in summary["rounds"]], [1, 2])
        first = summary["rounds"][0]
        self.assertEqual(first["scripts"], ["s01", "s02"])
        # script はその世代で最良の試行を出した s01。
        self.assertEqual(first["script"], "s01")
        self.assertEqual(first["action"], "draft")
        self.assertEqual(first["trials"], 3)
        self.assertEqual(first["failed"], 0)
        self.assertEqual(summary["last"]["round"], 2)
        self.assertEqual(summary["last"]["scripts"], ["s01"])
        # 2 世代ぶんとも数えられる。
        self.assertEqual(self.script_of(summary, "s01")["trials"], 3)
        self.assertEqual(self.script_of(summary, "s02")["trials"], 1)

    def test_generation_wins_over_round(self) -> None:
        """exp.generation と exp.round の両方があれば exp.generation を数える。"""
        store = self.store()
        store.script("s01", 9, "draft", generation=2, with_round=True)
        store.trial("s01", 9, 0.75, {"lr": "0.1"}, action="draft", generation=2, with_round=True)
        summary = self.summary_quiet(store)
        self.assertEqual([r["round"] for r in summary["rounds"]], [2])
        self.assertEqual(summary["last"]["round"], 2)

    def test_last_generation_all_failed(self) -> None:
        """最後の世代の全試行が失敗なら trials == failed > 0（規則 R4 の判定材料）。"""
        store = self.store()
        store.script("s01", 1, "draft", generation=1)
        store.trial("s01", 1, 0.70, {"lr": "0.1"}, action="draft", generation=1)
        store.script("s02", 2, "new", parent_script="s01", generation=2)
        store.trial("s02", 2, None, {"lr": "0.2"}, failed=True, action="new", generation=2)
        store.script("s03", 2, "new", parent_script="s01", generation=2)
        store.trial("s03", 2, None, {"lr": "0.3"}, failed=True, action="new", generation=2)
        summary = self.summary_quiet(store)
        self.assertEqual(summary["last"]["round"], 2)
        self.assertEqual(summary["last"]["scripts"], ["s02", "s03"])
        self.assertEqual(summary["last"]["trials"], 2)
        self.assertEqual(summary["last"]["failed"], 2)
        self.assertEqual(summary["rounds"][1]["trials"], 2)
        self.assertEqual(summary["rounds"][1]["failed"], 2)

    def test_other_study_is_ignored(self) -> None:
        """別の study の run は数えない。"""
        store = self.store()
        store.script("s01", 1, "draft", study=OTHER_STUDY)
        store.trial("s01", 1, 0.99, {"lr": "0.1"}, study=OTHER_STUDY, action="draft")
        summary = self.summary(store)
        self.assertIsNone(summary["best"])
        self.assertEqual(summary["rounds"], [])
        self.assertEqual(summary["scripts"], [])
        self.assertEqual(summary["param_edges"], [])
        self.assertEqual(summary["leak_suspects"], [])
        self.assertIsNone(summary["last"])
        self.assertFalse(summary["ablation_done"])

    def test_experiment_filter(self) -> None:
        """--experiment で探す experiment を絞れる。"""
        store = self.store()
        two_scripts_three_rounds(store)
        summary = self.summary(store, "--experiment", EXPERIMENT)
        self.assertEqual(len(summary["scripts"]), 2)
        # 存在しない experiment を指定しても 0 件として 0 で終わる。
        done = self.run_cmd(
            "--study", STUDY, "--metric", METRIC, "--direction", "maximize",
            "--experiment", "no-such-experiment", tracking_uri=store.uri,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(json.loads(done.stdout)["scripts"], [])

    def test_no_matching_runs(self) -> None:
        """該当 run が 0 件でも終了コード 0。"""
        store = self.store()
        summary = self.summary(store)
        self.assertIsNone(summary["best"])
        self.assertEqual(summary["rounds"], [])
        self.assertEqual(summary["scripts"], [])

    def test_tracking_uri_from_environment(self) -> None:
        """--tracking-uri が無ければ環境変数 MLFLOW_TRACKING_URI を使う。"""
        store = self.store()
        two_scripts_three_rounds(store)
        env = dict(os.environ)
        env["MLFLOW_TRACKING_URI"] = store.uri
        env["_MLFLOW_SERVER_ARTIFACT_ROOT"] = (self.root / "artifacts").as_uri()
        done = subprocess.run(
            [sys.executable, str(SCRIPT), "--study", STUDY, "--metric", METRIC,
             "--direction", "maximize"],
            capture_output=True, text=True, env=env, cwd=self.root, timeout=300,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(len(json.loads(done.stdout)["scripts"]), 2)

    def test_missing_tracking_uri(self) -> None:
        """接続先が無い場合は終了コード 2、標準エラー 1 行、標準出力は空。"""
        done = self.run_cmd(
            "--study", STUDY, "--metric", METRIC, "--direction", "maximize"
        )
        self.assertEqual(done.returncode, 2)
        self.assertEqual(done.stdout, "")
        self.assertEqual(len(done.stderr.strip().splitlines()), 1)

    def test_bad_arguments(self) -> None:
        """引数の誤りは終了コード 2。"""
        store = self.store()
        for args in (
            ("--metric", METRIC, "--direction", "maximize"),
            ("--study", STUDY, "--direction", "maximize"),
            ("--study", STUDY, "--metric", METRIC),
            ("--study", STUDY, "--metric", METRIC, "--direction", "sideways"),
        ):
            with self.subTest(args=args):
                done = self.run_cmd(*args, tracking_uri=store.uri)
                self.assertEqual(done.returncode, 2)
                self.assertEqual(done.stdout, "")

    def test_stdout_is_json_only(self) -> None:
        """標準出力は JSON 1 つだけで、python -m json.tool を通る。"""
        store = self.store()
        two_scripts_three_rounds(store)
        done = self.run_cmd(
            "--study", STUDY, "--metric", METRIC, "--direction", "maximize",
            tracking_uri=store.uri,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        checked = subprocess.run(
            [sys.executable, "-m", "json.tool"], input=done.stdout,
            capture_output=True, text=True, timeout=300,
        )
        self.assertEqual(checked.returncode, 0, checked.stderr)
        self.assertEqual(json.loads(checked.stdout), json.loads(done.stdout))
        self.assertEqual(
            sorted(json.loads(done.stdout)),
            ["ablation_done", "best", "direction", "last", "leak_suspects", "metric",
             "min_delta", "param_edges", "rounds", "scripts", "study"],
        )
        self.assertEqual(json.loads(done.stdout)["min_delta"], 0.0001)


if __name__ == "__main__":
    unittest.main()
