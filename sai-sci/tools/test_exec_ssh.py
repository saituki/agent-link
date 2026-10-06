"""ssh の実行先（sai-sci/exec/ssh）のテスト。

本物のホストは使わない。`ssh` を「同じ Mac の別フォルダ（ホストのホーム）でコマンドを動かす」
偽物に差し替え、Linux にしか無い `setsid` と `flock` も小さな偽物で補う。
runner.sh・rsync・環境変数の送り方・状態の読み取りまでを、そのままの部品で確かめる。

    cd sai-sci/tools && python3 -m unittest test_exec_ssh -v      # Python 3.11 以上
"""

from __future__ import annotations

import http.server
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
SCRIPT = TOOLS / "exp_exec.py"
EXEC = TOOLS.parent / "exec"

FAKE_SSH = """#!/usr/bin/env python3
import os, subprocess, sys
args = sys.argv[1:]
i = 0
while args[i] == "-o":  # ssh のオプション（-o 名=値）を読み飛ばす
    i += 2
command = " ".join(args[i + 1:])  # args[i] はホスト名
env = dict(os.environ, HOME=os.environ["FAKE_REMOTE_HOME"])
sys.exit(subprocess.run(["sh", "-c", command], cwd=env["HOME"], env=env).returncode)
"""
FAKE_SETSID = """#!/usr/bin/env python3
import os, sys
try:
    os.setsid()
except OSError:
    pass
os.execvp(sys.argv[1], sys.argv[1:])
"""
FAKE_FLOCK = """#!/usr/bin/env python3
import fcntl, sys
try:
    fcntl.flock(int(sys.argv[-1]), fcntl.LOCK_EX | fcntl.LOCK_NB)
except OSError:
    sys.exit(1)
"""


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # 認証つきの MLflow のふり（401 が返れば「届いている」）
        self.send_response(401)
        self.end_headers()

    def log_message(self, *_) -> None:
        pass


class Env:
    def __init__(self, max_jobs: int = 1) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="exec-ssh-"))
        self.home = self.tmp / "home"
        self.remote = self.tmp / "remote"
        self.project = self.tmp / "project"
        self.work = self.tmp / "work"
        self.bin = self.tmp / "bin"
        for d in (self.home, self.remote, self.work, self.bin, self.project / ".sai" / "exec"):
            d.mkdir(parents=True)
        for name, text in (("ssh", FAKE_SSH), ("setsid", FAKE_SETSID), ("flock", FAKE_FLOCK)):
            path = self.bin / name
            path.write_text(text)
            path.chmod(0o755)
        target = self.project / ".sai/exec/lab"
        shutil.copytree(EXEC / "ssh", target)
        cfg = (target / "target.toml").read_text()
        cfg = cfg.replace("enable = false", "enable = true").replace('host = ""', 'host = "fakehost"')
        cfg = cfg.replace("max_jobs = 1", f"max_jobs = {max_jobs}")
        cfg = cfg.replace('remote_root = ".sai-exec"', 'remote_root = ".exp-test"')
        (target / "target.toml").write_text(cfg)
        (self.work / "hello.txt").write_text("こんにちは\n")
        (self.work / ".venv").mkdir()
        (self.work / ".venv" / "big.bin").write_text("送らない")
        self.env = dict(
            os.environ,
            SAI_HOME=str(self.home),
            SAI_PROJECT_ROOT=str(self.project),
            FAKE_REMOTE_HOME=str(self.remote),
            PATH=f"{self.bin}:{os.environ['PATH']}",
            MLFLOW_TRACKING_PASSWORD="ひみつ",
        )

    def run(self, *args: str, extra_env: dict | None = None) -> subprocess.CompletedProcess:
        env = dict(self.env, **(extra_env or {}))
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args],
            capture_output=True, text=True, cwd=self.work, env=env, timeout=120,
        )

    def submit(self, *command: str) -> str:
        r = self.run("run", "lab", "--", *command)
        assert r.returncode == 0, r.stderr
        return r.stdout.splitlines()[0]

    def wait(self, job_id: str, want: tuple[str, ...], seconds: float = 30) -> str:
        end, state = time.time() + seconds, ""
        while time.time() < end:
            state = self.run("status", job_id).stdout.strip()
            if state in want:
                return state
            time.sleep(0.3)
        return state

    def job_dir(self, job_id: str) -> Path:
        return self.remote / ".exp-test" / "jobs" / job_id

    def close(self) -> None:
        # 動いたままの子を残さない
        for pid_file in self.remote.glob(".exp-test/jobs/*/pid"):
            try:
                os.killpg(int(pid_file.read_text()), 9)
            except (OSError, ValueError):
                pass
        shutil.rmtree(self.tmp, ignore_errors=True)


class SshTest(unittest.TestCase):
    def make(self, **kw) -> Env:
        env = Env(**kw)
        self.addCleanup(env.close)
        return env

    def test_run_syncs_code_forwards_env_and_finishes(self) -> None:
        e = self.make()
        job = e.submit("sh", "-c", "cat hello.txt; echo $MLFLOW_TRACKING_URI; echo $MLFLOW_TRACKING_PASSWORD; ls -a | grep -c venv || true")
        self.assertEqual(e.wait(job, ("done", "failed")), "done")
        logs = e.run("logs", job).stdout.split("\n")
        self.assertEqual(logs[0], "こんにちは", "コードが送られている")
        self.assertIn("<Mac の IP>:8443", logs[1], "target.toml の [env] が届く")
        self.assertEqual(logs[2], "ひみつ", "MLFLOW_* は自動で送られる")
        self.assertEqual(logs[3], "0", ".venv は送られない")
        mode = stat.S_IMODE((e.job_dir(job) / "env").stat().st_mode)
        self.assertEqual(mode, 0o600, "鍵の入る env は自分だけが読める")

    def test_failed_and_logs(self) -> None:
        e = self.make()
        job = e.submit("sh", "-c", "echo 落ちた; exit 3")
        self.assertEqual(e.wait(job, ("done", "failed")), "failed")
        self.assertIn("落ちた", e.run("logs", job, "-n", "5").stdout)
        self.assertEqual((e.job_dir(job) / "exit_code").read_text().strip(), "3")

    def test_cancel(self) -> None:
        e = self.make()
        job = e.submit("sleep", "60")
        self.assertEqual(e.wait(job, ("running",)), "running")
        self.assertEqual(e.run("cancel", job).returncode, 0)
        self.assertEqual(e.wait(job, ("cancelled",)), "cancelled")

    def test_killed_runner_is_lost(self) -> None:
        e = self.make()
        job = e.submit("sleep", "60")
        self.assertEqual(e.wait(job, ("running",)), "running")
        pgid = int((e.job_dir(job) / "pid").read_text())
        os.killpg(pgid, 9)  # runner ごと落とす（ホストの再起動のつもり）
        time.sleep(0.5)
        # 子（sleep）は別のグループなので、それも止めてから読む
        subprocess.run(["pkill", "-9", "-f", "sleep 60"], check=False)
        self.assertEqual(e.wait(job, ("lost",)), "lost")

    def test_max_jobs_queues_second(self) -> None:
        e = self.make(max_jobs=1)
        first = e.submit("sleep", "60")
        self.assertEqual(e.wait(first, ("running",)), "running")
        second = e.submit("sh", "-c", "echo 二番目")
        time.sleep(1)
        self.assertEqual(e.run("status", second).stdout.strip(), "queued", "上限のあいだは順番待ち")
        e.run("cancel", first)
        self.assertEqual(e.wait(second, ("done",), seconds=40), "done", "空いたら動く")

    def test_check_reports_reachability(self) -> None:
        e = self.make()
        server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        cfg = e.project / ".sai/exec/lab/target.toml"
        text = cfg.read_text()
        # 届き先は target.toml の [env] で決まる（実行先ごとに違う）
        cfg.write_text(text.replace("http://<Mac の IP>:8443", f"http://127.0.0.1:{server.server_port}"))
        ok = e.run("check", "lab")
        self.assertEqual(ok.returncode, 0, ok.stderr)
        cfg.write_text(text.replace("http://<Mac の IP>:8443", "http://127.0.0.1:9"))
        bad = e.run("check", "lab")
        self.assertEqual(bad.returncode, 1)
        self.assertIn("MLflow", bad.stderr)

    def test_missing_host_and_bad_root_are_refused(self) -> None:
        e = self.make()
        cfg = e.project / ".sai/exec/lab/target.toml"
        text = cfg.read_text()
        cfg.write_text(text.replace('host = "fakehost"', 'host = ""'))
        r = e.run("check", "lab")
        self.assertEqual(r.returncode, 1)
        self.assertIn("host が未設定", r.stderr)
        cfg.write_text(text.replace('remote_root = ".exp-test"', 'remote_root = "../x"'))
        r = e.run("check", "lab")
        self.assertEqual(r.returncode, 1)
        self.assertIn("remote_root", r.stderr)

    def test_shipped_ssh_target_is_disabled_by_default(self) -> None:
        text = (EXEC / "ssh" / "target.toml").read_text()
        self.assertIn("enable = false", text)
        self.assertIn('host = ""', text)


if __name__ == "__main__":
    unittest.main()
