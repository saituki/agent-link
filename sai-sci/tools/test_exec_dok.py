"""dok の実行先（sai-sci/exec/dok）の adapter のテスト。

DOK には触らない。同じフォルダの `dok.py` を、引数を記録して指定の動きをする偽物に差し替える。
確かめるのは、送るスクリプトの中身・状態の読み取り・中止が dok.py まで届くこと（課金の安全）。

    cd sai-sci/tools && python3 -m unittest test_exec_dok -v      # Python 3.11 以上
"""

from __future__ import annotations

import base64
import io
import os
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
SCRIPT = TOOLS / "exp_exec.py"
EXEC = TOOLS.parent / "exec"

FAKE_DOK = '''#!/usr/bin/env python3
import os, signal, sys, time
args = sys.argv[1:]
if "auth" in args:
    print("tos_agreed_at=2026-01-01"); sys.exit(0)
script = args[args.index("--script") + 1]
folder = os.path.dirname(script)
open(os.path.join(folder, "fake_args"), "w").write(" ".join(args))
def on_term(*_):
    open(os.path.join(folder, "task_cancelled"), "w").write("1")
    print("タスクを止めた", flush=True)
    sys.exit(130)
signal.signal(signal.SIGTERM, on_term)
print("task_id=abc status=running", flush=True)
mode = os.environ.get("FAKE_MODE", "ok")
if mode == "hang":
    time.sleep(60)
if mode == "fail":
    sys.exit(1)
if mode == "leak":
    print("警告: タスクを止められなかった", flush=True)
    sys.exit(4)
print("ssh_exit_code=0")
'''


class Env:
    def __init__(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="exec-dok-"))
        self.home = self.tmp / "home"
        self.project = self.tmp / "project"
        self.work = self.tmp / "work"
        for d in (self.home, self.work, self.project / ".sai" / "exec"):
            d.mkdir(parents=True)
        self.target = self.project / ".sai/exec/cloud"
        shutil.copytree(EXEC / "dok", self.target)
        (self.target / "dok.py").write_text(FAKE_DOK)
        cfg = (self.target / "target.toml").read_text().replace("enable = false", "enable = true")
        (self.target / "target.toml").write_text(cfg)
        (self.work / "hello.txt").write_text("こんにちは\n")
        for d in (".venv", ".git"):
            (self.work / d).mkdir()
            (self.work / d / "x").write_text("送らない")
        self.key = self.tmp / "key"
        self.env = dict(
            os.environ, SAI_HOME=str(self.home), SAI_PROJECT_ROOT=str(self.project),
            MLFLOW_TRACKING_PASSWORD="ひみつ",
        )

    def run(self, *args: str, extra_env: dict | None = None) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args], capture_output=True, text=True,
            cwd=self.work, env=dict(self.env, **(extra_env or {})), timeout=60,
        )

    def submit(self, *command: str, mode: str = "ok") -> str:
        r = self.run("run", "cloud", "--", *command or ("true",), extra_env={"FAKE_MODE": mode})
        assert r.returncode == 0, r.stderr
        return r.stdout.splitlines()[0]

    def wait(self, job_id: str, want: tuple[str, ...], seconds: float = 15) -> str:
        end, state = time.time() + seconds, ""
        while time.time() < end:
            state = self.run("status", job_id).stdout.strip()
            if state in want:
                return state
            time.sleep(0.2)
        return state

    def job_dir(self, job_id: str) -> Path:
        return self.home / "exec" / "dok" / job_id

    def close(self) -> None:
        for pid_file in self.home.glob("exec/dok/*/pid"):
            try:
                os.kill(int(pid_file.read_text()), 9)
            except (OSError, ValueError):
                pass
        shutil.rmtree(self.tmp, ignore_errors=True)


def unpack(script_text: str) -> list[str]:
    body = script_text.split("<<'__CODE__' | tar xz\n", 1)[1].split("\n__CODE__", 1)[0]
    with tarfile.open(fileobj=io.BytesIO(base64.b64decode(body)), mode="r:gz") as tar:
        return sorted(tar.getnames())


class DokTest(unittest.TestCase):
    def make(self) -> Env:
        env = Env()
        self.addCleanup(env.close)
        return env

    def test_script_has_code_env_setup_and_command(self) -> None:
        e = self.make()
        job = e.submit("uv", "run", "python", "s01/g1.py")
        self.assertEqual(e.wait(job, ("done", "failed")), "done")
        script = (e.job_dir(job) / "script.sh").read_text()
        names = unpack(script)
        self.assertIn("./hello.txt", names)
        self.assertFalse([n for n in names if ".venv" in n or ".git" in n], "除外の物は送らない")
        self.assertIn("export MLFLOW_TRACKING_PASSWORD='ひみつ'", script)
        self.assertIn("pip install -q uv", script)
        self.assertEqual(script.strip().splitlines()[-1], "uv run python s01/g1.py")
        self.assertEqual(stat.S_IMODE((e.job_dir(job) / "script.sh").stat().st_mode), 0o600, "鍵が入るので自分だけ")
        args = (e.job_dir(job) / "fake_args").read_text()
        for expected in ("ssh-run", "--plan v100-32gb", "--limit-sec 900", "--mlflow-url https://", f"--name exp-{job}"):
            self.assertIn(expected, args)

    def test_failed_and_leaked_task_are_failed_with_log(self) -> None:
        e = self.make()
        job = e.submit(mode="fail")
        self.assertEqual(e.wait(job, ("done", "failed")), "failed")
        job = e.submit(mode="leak")
        self.assertEqual(e.wait(job, ("done", "failed")), "failed")
        self.assertIn("止められなかった", e.run("logs", job).stdout, "課金の警告がログに残る")

    def test_cancel_reaches_dok_and_waits_for_it(self) -> None:
        e = self.make()
        job = e.submit(mode="hang")
        self.assertEqual(e.wait(job, ("running",)), "running")
        time.sleep(0.5)  # dok.py が SIGTERM の受け口を作るのを待つ
        self.assertEqual(e.run("cancel", job).returncode, 0)
        self.assertEqual(e.wait(job, ("cancelled",)), "cancelled")
        self.assertTrue((e.job_dir(job) / "task_cancelled").exists(), "SIGTERM が dok.py に届いてタスクを止めた")
        self.assertIn("タスクを止めた", e.run("logs", job).stdout)

    def test_only_one_task_at_a_time(self) -> None:
        e = self.make()
        first = e.submit(mode="hang")
        self.assertEqual(e.wait(first, ("running",)), "running")
        second = e.run("run", "cloud", "--", "true", extra_env={"FAKE_MODE": "ok"})
        self.assertEqual(second.returncode, 1)
        self.assertIn("1 タスク", second.stderr)
        e.run("cancel", first)

    def test_big_code_is_refused(self) -> None:
        e = self.make()
        cfg = e.target / "target.toml"
        cfg.write_text(cfg.read_text().replace("max_code_mb = 5", "max_code_mb = 0.0001"))
        (e.work / "big.bin").write_bytes(os.urandom(50_000))
        r = e.run("run", "cloud", "--", "true")
        self.assertEqual(r.returncode, 1)
        self.assertIn("MB を超えています", r.stderr)

    def test_check_needs_key_and_auth(self) -> None:
        e = self.make()
        cfg = e.target / "target.toml"
        cfg.write_text(cfg.read_text().replace('key = "~/.ssh/dok_key"', f'key = "{e.key}"'))
        r = e.run("check", "cloud")
        self.assertEqual(r.returncode, 1)
        self.assertIn("秘密鍵がありません", r.stderr)
        e.key.write_text("dummy")
        self.assertEqual(e.run("check", "cloud").returncode, 0)

    def test_shipped_dok_target_is_disabled_and_docs_have_no_old_paths(self) -> None:
        text = (EXEC / "dok" / "target.toml").read_text()
        self.assertIn("enable = false", text)
        for name in ("dok.py", "adapter"):
            self.assertNotIn("sai_ssh", (EXEC / "dok" / name).read_text(), name)
        self.assertNotIn("python3 -m sai_ssh", (EXEC / "dok" / "README.md").read_text())


if __name__ == "__main__":
    unittest.main()
