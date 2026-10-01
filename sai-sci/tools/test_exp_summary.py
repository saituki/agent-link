"""台帳 `exp_summary.py` のテスト。

sqlite の一時ストアに MLflow の記録（experiment = モデル、run は平ら）を作り、台帳の
各欄を確かめる。中央の MLflow サーバには書き込まない。

    cd sai-sci/tools && uv run --no-project --python ">=3.11" --with mlflow python -m unittest test_exp_summary -v
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from mlflow.entities import Metric, Param
from mlflow.tracking import MlflowClient

SCRIPT = Path(__file__).resolve().parent / "exp_summary.py"

SPROUT = "120"
METRIC = "test_acc"
STATIC = "test-moon-260930/ID120/cnn-static"
MULTI = "test-moon-260930/ID120/cnn-multi"
OTHER = "test-moon-260930/ID999/cnn-static"


class Store:
    """sqlite の mlflow ストアに experiment と run を作っていくための道具。"""

    def __init__(self, root: Path) -> None:
        (root / "artifacts").mkdir(parents=True, exist_ok=True)
        self.root = root
        self.uri = f"sqlite:///{root / 'mlflow.db'}"
        self.client = MlflowClient(tracking_uri=self.uri)
        self.ids: dict[str, str] = {}
        self.clock = 1_700_000_000_000

    def experiment(self, name: str, sprout: str) -> None:
        self.ids[name] = self.client.create_experiment(
            name,
            artifact_location=(self.root / "artifacts" / name.replace("/", "_")).as_uri(),
            tags={"sai.sprout_run_id": sprout, "exp.model": name.rsplit("/", 1)[-1]},
        )

    def run(self, experiment: str, branch: str, n: int, value: float | None, params: dict,
            action: str = "tune", parent: str = "", failed: bool = False,
            curve: int = 2) -> str:
        self.clock += 1000
        run = self.client.create_run(
            self.ids[experiment],
            start_time=self.clock,
            run_name=f"ID{branch}-p{n:02d}",
            tags={
                "sai.run_id": branch,
                "exp.action": action,
                "exp.parent_branch": parent,
                "exp.trial": f"{n:02d}",
            },
        )
        metrics = [Metric("train_loss", 1.0 / (e + 1), self.clock, e + 1) for e in range(curve)]
        if value is not None:
            metrics.append(Metric(METRIC, value, self.clock, 0))
        self.client.log_batch(
            run.info.run_id, metrics=metrics,
            params=[Param(k, str(v)) for k, v in sorted(params.items())],
        )
        self.client.set_terminated(run.info.run_id, status="FAILED" if failed else "FINISHED")
        return run.info.run_id


def build(store: Store) -> None:
    """sprout 120: cnn-static（121 draft・125 tune・126 debug）と cnn-multi（130 model）。"""
    store.experiment(STATIC, SPROUT)
    store.experiment(MULTI, SPROUT)
    store.experiment(OTHER, "999")
    store.run(STATIC, "121", 1, 0.70, {"lr": 0.001}, action="draft")
    store.run(STATIC, "121", 2, 0.75, {"lr": 0.01}, action="draft")
    store.run(STATIC, "121", 3, None, {"lr": 0.005}, action="draft", failed=True)
    store.run(STATIC, "125", 1, 0.80, {"lr": 0.1}, parent="121")
    store.run(STATIC, "125", 2, 0.871, {"lr": 0.01}, parent="121")
    store.run(STATIC, "126", 1, 0.80, {"lr": 0.01}, action="debug", parent="125")
    store.run(MULTI, "130", 1, 0.60, {"lr": 0.1}, action="model", parent="121", curve=1)
    store.run(MULTI, "130", 2, 0.65, {"lr": 0.01}, action="model", parent="121", curve=1)
    # 別の sprout の run は混ざらない
    store.run(OTHER, "500", 1, 0.99, {"lr": 0.1}, action="ablation")


class ExpSummaryTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)
        self.store = Store(self.root)

    def run_cmd(self, *args: str, uri: str | None = "") -> subprocess.CompletedProcess:
        env = dict(os.environ)
        env.pop("MLFLOW_TRACKING_URI", None)
        cmd = [sys.executable, str(SCRIPT), *args]
        if uri is not None:
            cmd += ["--tracking-uri", uri or self.store.uri]
        return subprocess.run(cmd, capture_output=True, text=True, env=env, cwd=self.root, timeout=300)

    def summary(self, *args: str, direction: str = "maximize") -> dict:
        done = self.run_cmd("--sprout", SPROUT, "--metric", METRIC, "--direction", direction, *args)
        self.assertEqual(done.returncode, 0, done.stderr)
        return json.loads(done.stdout)

    def test_full_summary(self) -> None:
        build(self.store)
        s = self.summary("--branch", "125", "--leak-max", "0.86")
        self.assertEqual((s["sprout"], s["metric"], s["direction"]), (120, METRIC, "maximize"))
        self.assertEqual(s["best"]["experiment"], STATIC)
        self.assertEqual((s["best"]["branch"], s["best"]["run_name"], s["best"]["value"]), (125, "ID125-p02", 0.871))
        self.assertEqual(s["best"]["params"], {"lr": "0.01"})
        exps = {e["name"]: e for e in s["experiments"]}
        self.assertEqual(set(exps), {STATIC, MULTI}, "別の sprout の experiment は入らない")
        st = exps[STATIC]
        self.assertEqual((st["model"], st["branches"], st["runs"], st["failed"]), ("cnn-static", [121, 125, 126], 6, 1))
        self.assertEqual((st["best"], st["best_branch"], st["has_curves"]), (0.871, 125, True))
        self.assertEqual(st["stale"], 1, "126 だけ最良を更新していない")
        mu = exps[MULTI]
        self.assertEqual((mu["stale"], mu["has_curves"], mu["best_branch"]), (0, False, 130))
        br = {b["branch"]: b for b in s["branches"]}
        self.assertEqual(sorted(br), [121, 125, 126, 130])
        self.assertEqual((br[121]["depth"], br[125]["depth"], br[126]["depth"], br[130]["depth"]), (1, 2, 3, 2))
        self.assertEqual((br[125]["debug_depth"], br[126]["debug_depth"]), (0, 1))
        self.assertEqual((br[121]["improved"], br[125]["improved"], br[126]["improved"]), (True, True, False))
        self.assertTrue(br[130]["improved"], "別の experiment の最初の branch")
        self.assertEqual(br[125]["param_edges"], [{"param": "lr", "side": "min"}])
        self.assertEqual(br[121]["param_edges"], [{"param": "lr", "side": "max"}])
        self.assertEqual((br[125]["action"], br[125]["parent_branch"], br[125]["best_params"]), ("tune", 121, {"lr": "0.01"}))
        self.assertEqual((br[121]["runs"], br[121]["failed"], br[130]["has_curves"]), (3, 1, False))
        self.assertEqual(s["target"], {"branch": 125, "runs": 2, "failed": 0, "best": 0.871, "improved": True})
        self.assertEqual(s["total_branches"], 4)
        self.assertFalse(s["ablation_done"], "別の sprout の ablation は数えない")
        self.assertEqual(s["leak_suspects"], [{"branch": 125, "run_name": "ID125-p02", "value": 0.871}])
        self.assertEqual(s["warnings"], [])

    def test_target_without_runs(self) -> None:
        build(self.store)
        s = self.summary("--branch", "777")
        self.assertEqual(s["target"], {"branch": 777, "runs": 0, "failed": 0, "best": None, "improved": False})

    def test_minimize_and_ablation(self) -> None:
        build(self.store)
        self.store.run(STATIC, "140", 1, 0.0000001, {"lr": 0.1}, action="ablation", parent="125")
        s = self.summary(direction="minimize")
        self.assertEqual((s["best"]["branch"], s["best"]["value"]), (140, 0.0000001))
        self.assertTrue(s["ablation_done"])
        self.assertEqual([x["branch"] for x in s["leak_suspects"]], [140])

    def test_missing_run_id_is_warned(self) -> None:
        build(self.store)
        run = self.store.client.create_run(self.store.ids[STATIC], tags={})
        self.store.client.set_terminated(run.info.run_id, status="FINISHED")
        s = self.summary()
        self.assertEqual(len(s["warnings"]), 1)
        self.assertEqual(next(e for e in s["experiments"] if e["name"] == STATIC)["runs"], 6)

    def test_empty_sprout_is_success(self) -> None:
        done = self.run_cmd("--sprout", "5", "--metric", METRIC, "--direction", "maximize")
        self.assertEqual(done.returncode, 0, done.stderr)
        s = json.loads(done.stdout)
        self.assertEqual((s["best"], s["experiments"], s["total_branches"]), (None, [], 0))

    def test_no_tracking_uri_is_exit_2(self) -> None:
        done = self.run_cmd("--sprout", SPROUT, "--metric", METRIC, "--direction", "maximize", uri=None)
        self.assertEqual(done.returncode, 2)


if __name__ == "__main__":
    unittest.main()
