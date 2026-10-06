"""分配器 `exp_exec.py` と同梱の実行先（local・_example）のテスト。

一時フォルダの中だけで動かす（`SAI_HOME` と `SAI_PROJECT_ROOT` を差し替える）。
実際の ~/.sai やプロジェクトは触らない。

    cd sai-sci/tools && python3 -m unittest test_exp_exec -v      # Python 3.11 以上
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
SCRIPT = TOOLS / "exp_exec.py"
EXEC = TOOLS.parent / "exec"


class Env:
    """一時の sai_home とプロジェクト。実行先は sai-sci/exec/ から写す。"""

    def __init__(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="exp-exec-"))
        self.home = self.tmp / "home"
        self.project = self.tmp / "project"
        self.work = self.tmp / "work"
        for d in (self.home, self.work, self.project / ".sai" / "exec"):
            d.mkdir(parents=True)
        shutil.copytree(EXEC, self.project / ".sai" / "exec", dirs_exist_ok=True)

    def run(self, *args: str, cwd: Path | None = None) -> subprocess.CompletedProcess:
        env = dict(os.environ, SAI_HOME=str(self.home), SAI_PROJECT_ROOT=str(self.project))
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args],
            capture_output=True, text=True, cwd=cwd or self.work, env=env, timeout=60,
        )

    def wait(self, job_id: str, want: tuple[str, ...], seconds: float = 15) -> str:
        end = time.time() + seconds
        state = ""
        while time.time() < end:
            state = self.run("status", job_id).stdout.strip()
            if state in want:
                return state
            time.sleep(0.2)
        return state

    def close(self) -> None:
        shutil.rmtree(self.tmp, ignore_errors=True)


class ExecTest(unittest.TestCase):
    def setUp(self) -> None:
        self.env = Env()
        self.addCleanup(self.env.close)

    def test_list_shows_local_and_hides_example(self) -> None:
        out = json.loads(self.env.run("list", "--json").stdout)
        names = {t["name"]: t for t in out["targets"]}
        self.assertIn("local", names)
        self.assertTrue(names["local"]["enable"])
        self.assertNotIn("_example", names, "下線始まりは実行先として扱わない")

    def test_run_success_logs_and_status(self) -> None:
        r = self.env.run("run", "local", "--name", "t1", "--", "sh", "-c", "echo こんにちは; echo $EXP_X")
        self.assertEqual(r.returncode, 0, r.stderr)
        job_id = r.stdout.splitlines()[0]
        self.assertTrue(job_id.startswith("j"))
        self.assertEqual(self.env.wait(job_id, ("done", "failed")), "done")
        logs = self.env.run("logs", job_id).stdout
        self.assertIn("こんにちは", logs)
        detail = json.loads(self.env.run("status", job_id, "--json").stdout)
        self.assertEqual((detail["state"], detail["target"]), ("done", "local"))

    def test_run_passes_cwd_and_env_prefix(self) -> None:
        (self.env.work / "marker.txt").write_text("ok")
        r = self.env.run("run", "local", "--", "env", "EXP_BRANCH_RUN_ID=121", "sh", "-c", "cat marker.txt; echo $EXP_BRANCH_RUN_ID")
        job_id = r.stdout.splitlines()[0]
        self.assertEqual(self.env.wait(job_id, ("done", "failed")), "done")
        self.assertEqual(self.env.run("logs", job_id).stdout.split(), ["ok121"])

    def test_target_env_reaches_job(self) -> None:
        r = self.env.run("run", "local", "--", "sh", "-c", "echo $MLFLOW_TRACKING_URI")
        job_id = r.stdout.splitlines()[0]
        self.env.wait(job_id, ("done",))
        self.assertIn("127.0.0.1:5001", self.env.run("logs", job_id).stdout)

    def test_failure_is_failed(self) -> None:
        job_id = self.env.run("run", "local", "--", "sh", "-c", "exit 3").stdout.splitlines()[0]
        self.assertEqual(self.env.wait(job_id, ("done", "failed")), "failed")

    def started(self, script: str, timeout: str = "20") -> dict:
        job_id = self.env.run("run", "local", "--", "sh", "-c", script).stdout.splitlines()[0]
        r = self.env.run("started", job_id, "--timeout", timeout, "--poll", "0.2", "--json")
        self.assertEqual(r.returncode, 0, r.stderr)
        return json.loads(r.stdout)

    def test_started_when_epoch_line_appears(self) -> None:
        out = self.started("echo device=cpu; echo '[epoch 1/2] train_loss=0.7 val_loss=0.6 (1.0s)'; sleep 2")
        self.assertTrue(out["started"])
        self.assertEqual(out["device"], "cpu")
        self.assertTrue(out["line"].startswith("[epoch 1/2]"))
        self.assertEqual(out["log_tail"], "")

    def test_started_false_when_job_fails_early(self) -> None:
        out = self.started("echo import error; exit 1")
        self.assertEqual((out["state"], out["started"]), ("failed", False))
        self.assertIn("import error", out["log_tail"])

    def test_started_false_on_timeout(self) -> None:
        out = self.started("sleep 30", timeout="2")
        self.assertEqual((out["state"], out["started"]), ("running", False))
        self.assertIsNone(out["line"])

    def test_cancel(self) -> None:
        job_id = self.env.run("run", "local", "--", "sleep", "60").stdout.splitlines()[0]
        self.assertEqual(self.env.wait(job_id, ("running",)), "running")
        self.assertEqual(self.env.run("cancel", job_id).returncode, 0)
        self.assertEqual(self.env.wait(job_id, ("cancelled",)), "cancelled")

    def test_max_jobs_refuses_extra(self) -> None:
        cfg = self.env.project / ".sai/exec/local/target.toml"
        cfg.write_text(cfg.read_text().replace("max_jobs = 4", "max_jobs = 1"))
        first = self.env.run("run", "local", "--", "sleep", "30")
        self.assertEqual(first.returncode, 0, first.stderr)
        second = self.env.run("run", "local", "--", "sleep", "30")
        self.assertEqual(second.returncode, 1)
        self.assertIn("上限", second.stderr)
        self.env.run("cancel", first.stdout.splitlines()[0])

    def test_bad_inputs_are_refused(self) -> None:
        for bad in ("../x", "a/b", "_example", ""):
            r = self.env.run("run", bad, "--", "true")
            self.assertNotEqual(r.returncode, 0, bad)
        self.assertEqual(self.env.run("run", "nothing", "--", "true").returncode, 1)
        self.assertEqual(self.env.run("run", "local").returncode, 2, "コマンドが無い")
        self.assertEqual(self.env.run("status", "../../etc").returncode, 1)
        self.assertEqual(self.env.run("status", "j000000-000000-0000").returncode, 1)

    def test_disabled_target_is_refused(self) -> None:
        cfg = self.env.project / ".sai/exec/local/target.toml"
        cfg.write_text(cfg.read_text().replace("enable = true", "enable = false"))
        r = self.env.run("run", "local", "--", "true")
        self.assertEqual(r.returncode, 1)
        self.assertIn("無効", r.stderr)

    def test_add_target_is_just_a_folder(self) -> None:
        """フォルダを置くだけで実行先が増える。adapter が壊れた state を返したら止める。"""
        folder = self.env.project / ".sai/exec/fake"
        folder.mkdir()
        (folder / "target.toml").write_text('description = "偽"\nkind = "fake"\n')
        adapter = folder / "adapter"
        adapter.write_text(
            '#!/bin/sh\ncase "$1" in run) exit 0;; status) echo \'{"state": "weird"}\';; check) exit 0;; esac\n'
        )
        adapter.chmod(0o755)
        self.assertIn("fake", {t["name"] for t in json.loads(self.env.run("list", "--json").stdout)["targets"]})
        self.assertEqual(self.env.run("check", "fake").returncode, 0)
        job_id = self.env.run("run", "fake", "--", "true").stdout.splitlines()[0]
        r = self.env.run("status", job_id)
        self.assertEqual(r.returncode, 1)
        self.assertIn("weird", r.stderr)

    def test_example_target_can_be_copied_but_refuses_to_run(self) -> None:
        shutil.copytree(EXEC / "_example", self.env.project / ".sai/exec/copy")
        cfg = self.env.project / ".sai/exec/copy/target.toml"
        cfg.write_text(cfg.read_text().replace("enable = false", "enable = true"))
        r = self.env.run("run", "copy", "--", "true")
        self.assertEqual(r.returncode, 1)
        self.assertIn("まだ作られていません", r.stderr)

    def test_project_target_wins_over_home(self) -> None:
        other = self.env.home / "exec" / "local"
        other.mkdir(parents=True)
        (other / "adapter").write_text("#!/bin/sh\nexit 1\n")
        (other / "adapter").chmod(0o755)
        job_id = self.env.run("run", "local", "--", "true").stdout.splitlines()[0]
        self.assertTrue(job_id, "プロジェクトの local が先に使われる")


if __name__ == "__main__":
    unittest.main()
