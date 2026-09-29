"""高火力 DOK CLI（sai-sci/exec/dok/dok.py）のテスト。sai-ssh から取り込んだ。

    cd sai-sci/tools && python3 -m unittest test_dok -v

実際の DOK API は呼ばない。_request を差し替えて、
ネットワークに出ずに全経路を通す。
ルートの .env と mlflow-server/.env は読まない（ダミーの一時ファイルを使う）。
"""

from __future__ import annotations

import argparse
import io
import itertools
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "exec" / "dok"))

import dok  # noqa: E402

# テスト中にこの文字列がどの出力にも現れないことを確かめる。
SECRET = "SECRETVALUE-XYZ"

ALL_SECRETS = {
    "sakura_api_access_token": f"token-{SECRET}",
    "sakura_api_secret_key": f"secret-{SECRET}",
    "sakura_s3_access_key": f"s3key-{SECRET}",
    "sakura_s3_secret_key": f"s3secret-{SECRET}",
    "mlflow_basic_user": f"mluser-{SECRET}",
    "mlflow_basic_password": f"mlpass-{SECRET}",
}


def make_args(**overrides) -> argparse.Namespace:
    base = dict(
        zone="is1a",
        image="test-saito.sakuracr.jp/dok-test:0.1",
        plan="v100-32gb",
        limit_sec=900,
        mlflow_url="https://mlflow.example.test",
        experiment="dok",
        poll_sec=0,
        name="dok-test-20260929-153000",
    )
    base.update(overrides)
    return argparse.Namespace(**base)


class FakeApi:
    """dok._request の差し替え。呼び出しを記憶し、応答をプログラムする。"""

    def __init__(
        self,
        poll_statuses=("running", "done"),
        running=(),
        exit_code=0,
        post_status="waiting",
        download_url="https://dok.example.test/output/1",
        unit_price=0.14,
        registries=(
            {"id": "reg-1", "hostname": "test-saito.sakuracr.jp", "username": "saito"},
        ),
    ) -> None:
        self.poll_statuses = list(poll_statuses)
        self.running = list(running)
        self.registries = [dict(x) for x in registries]
        self.exit_code = exit_code
        self.post_status = post_status
        self.download_url = download_url
        self.unit_price = unit_price
        self.calls: list[tuple[str, str]] = []
        self.posted_tasks: list[dict] = []
        self.canceled = False
        self.output_text = '{"summary": "ok"}\n'
        self.task = {
            "id": "task-1",
            "status": "waiting",
            "error_message": "",
            "containers": [{"exit_code": None, "execution_seconds": 3}],
        }

    def calls_matching(self, method: str, prefix: str) -> list[str]:
        return [p for m, p in self.calls if m == method and p.startswith(prefix)]

    def _current_task(self) -> dict:
        if self.canceled:
            self.task["status"] = "aborted"
            return self.task
        if self.poll_statuses:
            nxt = self.poll_statuses.pop(0)
            if isinstance(nxt, BaseException):
                raise nxt
            self.task["status"] = nxt
            if nxt in dok.TERMINAL_STATUSES and nxt != "aborted":
                self.task["containers"][0]["exit_code"] = self.exit_code
        return self.task

    def __call__(self, method, path, body=None, auth=None, zone=None, timeout=30):
        self.calls.append((method, path))
        if method == "GET" and path == "/auth/":
            return {"account": {"tos_agreed_at": "2026-09-01T00:00:00Z", "account_code": "123-456"}}
        if method == "GET" and path.startswith("/tasks/?status="):
            return {"results": [dict(x) for x in self.running]}
        if method == "POST" and path == "/registries/":
            self.registries.append(
                {
                    "id": "reg-1",
                    "hostname": (body or {}).get("hostname"),
                    "username": (body or {}).get("username"),
                }
            )
            return {"id": "reg-1"}
        if method == "GET" and path.startswith("/registries/"):
            return {"results": [dict(x) for x in self.registries]}
        if method == "POST" and path == "/tasks/":
            self.posted_tasks.append(body or {})
            return {"id": "task-1", "status": self.post_status}
        if method == "GET" and path.startswith("/tasks/") and path.endswith("/download/output/"):
            if self.download_url is None:
                raise RuntimeError("DOK API GET /tasks/task-1/download/output/ が 404 を返した: {}")
            return {"url": self.download_url}
        if method == "GET" and path.startswith("/unit_prices/"):
            return {
                "results": [
                    {"plan": "v100-32gb", "price": self.unit_price},
                    {"plan": "h100-80gb", "price": 0.85},
                ]
            }
        if method == "POST" and path.endswith("/cancel/"):
            self.canceled = True
            return {}
        if method == "GET" and path.startswith("/tasks/"):
            return self._current_task()
        raise AssertionError(f"想定外の _request: {method} {path}")


class SshApi(FakeApi):
    """`/ssh/keys/` と `containers[0].ssh` を扱う FakeApi の拡張。

    実際の DOK API・ssh・ssh-keygen は呼ばない。
    """

    PUB = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample+Key+Data+For+Tests test@example.test"

    def __init__(self, ssh_keys=(), ssh_ready_after=1, cancel_fails=False, **kwargs) -> None:
        kwargs.setdefault("poll_statuses", ["running"])
        super().__init__(**kwargs)
        self.ssh_keys = [dict(x) for x in ssh_keys]
        self.posted_keys: list[dict] = []
        self.ssh_ready_after = ssh_ready_after
        self.cancel_fails = cancel_fails
        self.running_seen = 0
        self.cancel_calls = 0

    def _current_task(self) -> dict:
        task = super()._current_task()
        if task.get("status") == "running":
            self.running_seen += 1
            if self.running_seen >= self.ssh_ready_after:
                task["containers"][0]["ssh"] = {
                    "shell": "/bin/bash",
                    "host_name": "ssh.dok.example.test",
                    "port": 2222,
                }
                task["containers"][0]["start_at"] = "2026-09-29T16:00:00+09:00"
        return task

    def __call__(self, method, path, body=None, auth=None, zone=None, timeout=30):
        if method == "GET" and path.startswith("/ssh/keys/"):
            self.calls.append((method, path))
            return {"results": [dict(x) for x in self.ssh_keys]}
        if method == "POST" and path == "/ssh/keys/":
            self.calls.append((method, path))
            self.posted_keys.append(dict(body or {}))
            row = {"id": "ssh-key-1", "is_active": True, **(body or {})}
            self.ssh_keys.append(row)
            return dict(row)
        if method == "POST" and path.endswith("/cancel/") and self.cancel_fails:
            self.calls.append((method, path))
            self.cancel_calls += 1
            raise RuntimeError("DOK API POST cancel が HTTP 500 を返した: boom")
        return super().__call__(method, path, body, auth, zone, timeout)


def make_ssh_args(**overrides) -> argparse.Namespace:
    base = dict(
        zone="is1a",
        image=dok.DEFAULT_SSH_IMAGE,
        plan="v100-32gb",
        limit_sec=900,
        hold_sec=780,
        mlflow_url="https://mlflow.example.test",
        experiment="dok",
        poll_sec=0,
        ssh_timeout_sec=60,
        name="dok-ssh-20260929-153000",
        script=None,
        key=None,
        known_hosts=None,
    )
    base.update(overrides)
    return argparse.Namespace(**base)


def fake_ssh_result(returncode=0, calls=None, stdin_seen=None):
    """dok._drive_shell の差し替え。実際の ssh は呼ばない。"""

    def run(cmd, script=None, timeout=None):
        if calls is not None:
            calls.append(list(cmd))
        if stdin_seen is not None and script is not None:
            stdin_seen.append(script)
        return returncode

    return run


class SshKeyTest(unittest.TestCase):
    """`ssh-key` のテスト。実際の ~/.ssh/ は読み書きしない。"""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)
        self.key = self.dir / "dok_key"
        self.pub = self.dir / "dok_key.pub"
        self.subprocess_calls: list[list[str]] = []

    def _write_key(self, private_body: str, pub_body: str) -> None:
        self.key.write_text(private_body, encoding="utf-8")
        self.pub.write_text(pub_body, encoding="utf-8")

    def _run_subprocess(self, cmd, stdin=None, timeout=None, **kwargs):
        self.subprocess_calls.append(list(cmd))
        # 鍵ファイルは「足りないときだけ」作られるので、実際に作る。
        if cmd[:2] == ["ssh-keygen", "-t"]:
            self._write_key(f"-----BEGIN OPENSSH PRIVATE KEY-----\n{SECRET}\n", SshApi.PUB)
        return argparse.Namespace(returncode=0, stdout="256 SHA256:FAKE test (ED25519)\n", stderr="")

    def _cmd(self, api, name="sai-ssh-dok", secrets=None):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            with mock.patch.object(dok, "_request", api):
                code = dok.cmd_ssh_key(
                    make_args(key=str(self.key), name=name), secrets or dict(ALL_SECRETS)
                )
        return code, out.getvalue(), err.getvalue()

    def test_creates_key_and_registers_public_key(self) -> None:
        api = SshApi(ssh_keys=[])
        with mock.patch.object(dok.subprocess, "run", self._run_subprocess):
            code, out, err = self._cmd(api)
        self.assertEqual(code, 0)
        self.assertTrue(self.key.is_file())
        self.assertEqual(len(api.posted_keys), 1)
        self.assertEqual(api.posted_keys[0]["name"], "sai-ssh-dok")
        self.assertEqual(api.posted_keys[0]["pub_key"], SshApi.PUB)
        self.assertIs(api.posted_keys[0]["is_active"], True)
        self.assertIn("ssh_key_id=ssh-key-1", out)
        self.assertIn("SHA256:FAKE", out)
        # 秘密鍵の内容は出さない。
        self.assertNotIn(SECRET, out + err)
        self.assertNotIn(SECRET, " ".join(" ".join(c) for c in self.subprocess_calls))
        # パスフレーズなしで ed25519 を作る。
        gen = [c for c in self.subprocess_calls if c[:2] == ["ssh-keygen", "-t"]]
        self.assertEqual(len(gen), 1)
        self.assertIn("-N", gen[0])
        self.assertEqual(gen[0][gen[0].index("-N") + 1], "")

    def test_existing_key_is_not_recreated(self) -> None:
        self._write_key(f"PRIVATE-{SECRET}\n", SshApi.PUB + "\n")
        api = SshApi(ssh_keys=[])
        with mock.patch.object(dok.subprocess, "run", self._run_subprocess):
            code, out, _ = self._cmd(api)
        self.assertEqual(code, 0)
        self.assertEqual([c for c in self.subprocess_calls if c[:2] == ["ssh-keygen", "-t"]], [])
        self.assertEqual(len(api.posted_keys), 1)

    def test_no_post_when_public_key_already_registered(self) -> None:
        self._write_key(f"PRIVATE-{SECRET}\n", SshApi.PUB + "\n")
        api = SshApi(ssh_keys=[{"id": "ssh-key-9", "name": "old", "pub_key": SshApi.PUB, "is_active": True}])
        code, out, err = self._cmd(api)
        self.assertEqual(code, 0)
        self.assertEqual(api.calls_matching("POST", "/ssh/keys/"), [])
        self.assertEqual(api.posted_keys, [])
        self.assertIn("ssh_key_id=ssh-key-9", out)
        self.assertNotIn(SECRET, out + err)

    def test_no_post_when_registered_but_inactive(self) -> None:
        # 無効な鍵は「登録済み」とみなす（名前を直すのは別の操作）。
        self._write_key("PRIVATE\n", SshApi.PUB + "\n")
        api = SshApi(ssh_keys=[{"id": "ssh-key-8", "pub_key": SshApi.PUB, "is_active": False}])
        code, out, _ = self._cmd(api)
        self.assertEqual(code, 0)
        self.assertEqual(api.posted_keys, [])
        self.assertIn("ssh_key_id=ssh-key-8", out)

    def test_different_public_key_is_registered(self) -> None:
        self._write_key("PRIVATE\n", SshApi.PUB + "\n")
        other = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOther+Key other@example.test"
        api = SshApi(ssh_keys=[{"id": "ssh-key-7", "pub_key": other, "is_active": True}])
        code, _, _ = self._cmd(api)
        self.assertEqual(code, 0)
        self.assertEqual(len(api.posted_keys), 1)
        self.assertEqual(api.posted_keys[0]["pub_key"], SshApi.PUB)

    def test_ssh_keygen_failure_returns_one(self) -> None:
        api = SshApi(ssh_keys=[])

        def boom(cmd, stdin=None, timeout=None, **kwargs):
            self.subprocess_calls.append(list(cmd))
            return argparse.Namespace(returncode=1, stdout="", stderr=f"secret={SECRET}")

        with mock.patch.object(dok.subprocess, "run", boom):
            code, out, err = self._cmd(api)
        self.assertEqual(code, 1)
        self.assertEqual(api.calls_matching("POST", "/ssh/keys/"), [])
        self.assertNotIn(SECRET, out + err)


class SshRunTest(unittest.TestCase):
    """`ssh-run` のテスト。実際の DOK API・ssh・docker は呼ばない。"""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)
        self.script = self.dir / "pytorch_ssh_test.sh"
        self.script.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
        self.key = self.dir / "dok_key"
        self.key.write_text(f"PRIVATE-{SECRET}\n", encoding="utf-8")
        (self.dir / "dok_key.pub").write_text(SshApi.PUB + "\n", encoding="utf-8")
        self.known_hosts = self.dir / "known_hosts_dok"
        self.ssh_calls: list[list[str]] = []
        self.stdin_seen: list[bytes] = []

    def _run(self, api, returncode=0, args=None, secrets=None, ssh_side_effect=None, subproc=None):
        out, err = io.StringIO(), io.StringIO()
        runner = subproc or fake_ssh_result(
            returncode=returncode, calls=self.ssh_calls, stdin_seen=self.stdin_seen
        )
        if ssh_side_effect is not None:
            runner = ssh_side_effect
        base = make_ssh_args(
            script=str(self.script),
            key=str(self.key),
            known_hosts=str(self.known_hosts),
        )
        if args is not None:
            base = args
        with redirect_stdout(out), redirect_stderr(err):
            with mock.patch.object(dok, "_request", api), mock.patch.object(
                dok, "_fetch_text", lambda url, timeout=30: api.output_text
            ), mock.patch.object(dok, "_drive_shell", runner), mock.patch.object(
                dok.time, "sleep", lambda seconds: None
            ):
                code = dok.ssh_run(base, secrets or dict(ALL_SECRETS))
        return code, out.getvalue(), err.getvalue()

    def _api(self, **kwargs) -> SshApi:
        kwargs.setdefault(
            "ssh_keys", [{"id": "ssh-key-1", "pub_key": SshApi.PUB, "is_active": True}]
        )
        return SshApi(**kwargs)

    def test_happy_path_streams_script_and_cancels(self) -> None:
        api = self._api()
        code, out, err = self._run(api)
        self.assertEqual(code, 0)
        self.assertIn("task_id=task-1", out)
        self.assertIn("ssh_exit_code=0", out)
        self.assertEqual(self.ssh_calls[0][0], "ssh")
        # リモートコマンドは付けない (DOK の ssh は対話シェルに入る)。
        self.assertEqual(self.ssh_calls[0][-1], "task-1@ssh.dok.example.test")
        self.assertIn("-tt", self.ssh_calls[0])
        self.assertIn(f"UserKnownHostsFile={self.known_hosts}", self.ssh_calls[0])
        self.assertNotIn(str(Path.home() / ".ssh" / "known_hosts"), " ".join(self.ssh_calls[0]))
        # スクリプトの本体が標準入力として流れる。
        self.assertEqual(self.stdin_seen[0], b"#!/usr/bin/env bash\necho hi\n")
        self.assertEqual(api.calls_matching("POST", "/tasks/task-1/cancel/"), ["/tasks/task-1/cancel/"])
        self.assertNotIn(SECRET, out + err)

    def test_task_body_has_ssh_and_no_registry(self) -> None:
        api = self._api()
        self._run(api)
        body = api.posted_tasks[0]
        container = body["containers"][0]
        self.assertEqual(body["name"], "dok-ssh-20260929-153000")
        self.assertEqual(body["execution_time_limit_sec"], 900)
        self.assertEqual(body["tags"], ["sai-ssh", "dok-ssh"])
        self.assertEqual(container["image"], "pytorch/pytorch:2.8.0-cuda12.6-cudnn9-runtime")
        self.assertEqual(container["plan"], "v100-32gb")
        self.assertEqual(container["entrypoint"], ["/bin/bash"])
        self.assertEqual(container["command"], ["-c", "sleep 780"])
        self.assertEqual(container["ssh"], {"shell": "/bin/bash"})
        self.assertNotIn("registry", container)
        self.assertEqual(container["environment"]["DOK_REQUIRE_GPU"], "1")

    def test_running_task_blocks_start(self) -> None:
        api = self._api(running=[{"id": "task-9", "status": "running", "name": "old"}])
        code, out, _ = self._run(api)
        self.assertEqual(code, 3)
        self.assertIn("task-9", out)
        self.assertEqual(api.calls_matching("POST", "/tasks/"), [])
        self.assertEqual(self.ssh_calls, [])

    def test_missing_key_on_dok_blocks_start(self) -> None:
        api = self._api(ssh_keys=[])
        code, _, err = self._run(api)
        self.assertEqual(code, 2)
        self.assertIn("ssh-key", err)
        self.assertEqual(api.calls_matching("POST", "/tasks/"), [])

    def test_inactive_key_on_dok_blocks_start(self) -> None:
        api = self._api(ssh_keys=[{"id": "k", "pub_key": SshApi.PUB, "is_active": False}])
        code, _, err = self._run(api)
        self.assertEqual(code, 2)
        self.assertEqual(api.calls_matching("POST", "/tasks/"), [])

    def test_keyboard_interrupt_during_ssh_cancels(self) -> None:
        api = self._api()

        def boom(cmd, script=None, timeout=None):
            self.ssh_calls.append(list(cmd))
            raise KeyboardInterrupt

        with self.assertRaises(KeyboardInterrupt):
            self._run(api, ssh_side_effect=boom)
        self.assertEqual(api.calls_matching("POST", "/tasks/task-1/cancel/"), ["/tasks/task-1/cancel/"])

    def test_ssh_connection_failure_cancels_and_retries(self) -> None:
        api = self._api()
        code, _, err = self._run(api, returncode=dok.SSH_CONNECT_FAILED)
        self.assertEqual(code, 1)
        self.assertEqual(len(self.ssh_calls), dok.SSH_CONNECT_RETRIES)
        self.assertEqual(api.calls_matching("POST", "/tasks/task-1/cancel/"), ["/tasks/task-1/cancel/"])
        self.assertIn("255", err)

    def test_ssh_nonzero_exit_cancels(self) -> None:
        api = self._api()
        code, _, _ = self._run(api, returncode=3)
        self.assertEqual(code, 1)
        self.assertEqual(len(self.ssh_calls), 1)
        self.assertEqual(api.calls_matching("POST", "/tasks/task-1/cancel/"), ["/tasks/task-1/cancel/"])

    def test_task_dying_before_ssh_is_one(self) -> None:
        api = SshApi(
            poll_statuses=["error"],
            ssh_keys=[{"id": "k", "pub_key": SshApi.PUB, "is_active": True}],
        )
        code, _, err = self._run(api)
        self.assertEqual(code, 1)
        self.assertEqual(self.ssh_calls, [])
        self.assertIn("status=error", err)

    def test_wait_timeout_cancels(self) -> None:
        # 永远是 running だが ssh の host_name / port が入らない。
        api = self._api(ssh_ready_after=10_000)
        clock = itertools.count(0, 300)
        with mock.patch.object(dok, "_monotonic", side_effect=lambda: next(clock)):
            with self.assertRaises(TimeoutError):
                self._run(api)
        self.assertEqual(api.calls_matching("POST", "/tasks/task-1/cancel/"), ["/tasks/task-1/cancel/"])

    def test_cancel_failure_returns_four(self) -> None:
        api = self._api(cancel_fails=True)
        clock = itertools.count(0, 300)
        with mock.patch.object(dok, "_monotonic", side_effect=lambda: next(clock)):
            code, _, err = self._run(api)
        self.assertEqual(code, 4)
        self.assertEqual(api.cancel_calls, 2)
        self.assertIn("止められなかった", err)

    def test_bad_arguments_do_not_create_task(self) -> None:
        api = self._api()
        cases = [
            (make_ssh_args(script=str(self.dir / "nothing.sh"), key=str(self.key), known_hosts=str(self.known_hosts)), "script"),
            (make_ssh_args(script=str(self.script), key=str(self.dir / "no_key"), known_hosts=str(self.known_hosts)), "秘密鍵"),
            (make_ssh_args(script=str(self.script), key=str(self.key), known_hosts=str(self.known_hosts), limit_sec=300), "600"),
            (make_ssh_args(script=str(self.script), key=str(self.key), known_hosts=str(self.known_hosts), limit_sec=600, hold_sec=600), "hold-sec"),
        ]
        for args, needle in cases:
            code, _, err = self._run(api, args=args)
            self.assertEqual(code, 2, args)
            self.assertIn(needle, err)
        self.assertEqual(api.calls_matching("POST", "/tasks/"), [])
        self.assertEqual(self.ssh_calls, [])


class MainSshTest(unittest.TestCase):
    """main() から Timeline 経由の入口."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.env_file = Path(self.tmp.name) / ".env"
        self.env_file.write_text(
            "\n".join(f"{k.upper()}={v}" for k, v in ALL_SECRETS.items()) + "\n",
            encoding="utf-8",
        )

    def _main(self, argv):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = dok.main(argv, environ={}, env_files=[str(self.env_file)])
        return code, out.getvalue(), err.getvalue()

    def test_ssh_run_delegates_with_default_name(self) -> None:
        seen = {}

        def fake(args, secrets):
            seen["name"] = args.name
            seen["hold_sec"] = args.hold_sec
            seen["image"] = args.image
            return 0

        with mock.patch.object(dok, "ssh_run", fake):
            code, _, _ = self._main(["ssh-run"])
        self.assertEqual(code, 0)
        self.assertTrue(seen["name"].startswith("dok-ssh-"), seen["name"])
        self.assertEqual(seen["hold_sec"], dok.DEFAULT_HOLD_SEC)
        self.assertEqual(seen["image"], "pytorch/pytorch:2.8.0-cuda12.6-cudnn9-runtime")

    def test_ssh_key_delegates(self) -> None:
        seen = {}

        def fake(args, secrets):
            seen["key"] = args.key
            seen["name"] = args.name
            return 0

        with mock.patch.object(dok, "cmd_ssh_key", fake):
            code, _, _ = self._main(["ssh-key"])
        self.assertEqual(code, 0)
        self.assertEqual(seen["key"], "~/.ssh/dok_key")
        self.assertEqual(seen["name"], "sai-ssh-dok")

    def test_ssh_run_keyboard_interrupt_exits_130(self) -> None:
        with mock.patch.object(dok, "ssh_run", side_effect=KeyboardInterrupt):
            code, _, err = self._main(["ssh-run"])
        self.assertEqual(code, 130)
        self.assertNotIn(SECRET, err)


class BuildSshCommandTest(unittest.TestCase):
    def test_shape_and_known_hosts_are_isolated(self) -> None:
        cmd = dok.build_ssh_command(
            Path("/tmp/k"), 2222, "h.example.test", "task-1", Path("/tmp/kh/known_hosts_dok")
        )
        self.assertEqual(cmd[0], "ssh")
        self.assertIn("-i", cmd)
        self.assertEqual(cmd[cmd.index("-i") + 1], "/tmp/k")
        self.assertEqual(cmd[cmd.index("-p") + 1], "2222")
        self.assertIn("BatchMode=yes", cmd)
        self.assertIn("UserKnownHostsFile=/tmp/kh/known_hosts_dok", cmd)
        self.assertEqual(cmd[-1], "task-1@h.example.test")

    def test_pub_key_id_ignores_comment_and_spacing(self) -> None:
        self.assertEqual(
            dok._pub_key_id("  ssh-ed25519   AAAADATA  test@example.test \n"),
            "ssh-ed25519 AAAADATA",
        )
        self.assertEqual(dok._pub_key_id(""), "")

    def test_find_ssh_key_respects_is_active(self) -> None:
        keys = [{"id": "a", "pub_key": SshApi.PUB, "is_active": False}]
        self.assertIsNone(dok._find_ssh_key(keys, SshApi.PUB))
        self.assertEqual(dok._find_ssh_key(keys, SshApi.PUB, active_only=False)["id"], "a")


class LoadSecretsTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.env_file = Path(self.tmp.name) / ".env"
        self.env_file.write_text(
            "\n".join(
                [
                    "# コメント行",
                    "",
                    'SAKURA_API_ACCESS_TOKEN="from-file-A"',
                    "SAKURA_API_SECRET_kEY='from-file-B'",
                    "SAKURA_S3_ACCESS_KEY=from-file-C",
                    "broken line without equals",
                ]
            )
            + "\n",
            encoding="utf-8",
        )

    def test_case_insensitive_and_quotes_stripped(self) -> None:
        secrets = dok.load_secrets([str(self.env_file)], {})
        self.assertEqual(secrets["sakura_api_access_token"], "from-file-A")
        self.assertEqual(secrets["sakura_api_secret_key"], "from-file-B")
        self.assertEqual(secrets["sakura_s3_access_key"], "from-file-C")

    def test_environ_wins_over_file(self) -> None:
        secrets = dok.load_secrets([str(self.env_file)], {"SAKURA_S3_ACCESS_KEY": "from-env"})
        self.assertEqual(secrets["sakura_s3_access_key"], "from-env")
        self.assertEqual(secrets["sakura_api_secret_key"], "from-file-B")

    def test_missing_file_is_ignored(self) -> None:
        secrets = dok.load_secrets([str(Path(self.tmp.name) / "nothing.env")], {"A_B": "c"})
        self.assertEqual(secrets, {"a_b": "c"})


class BuildEnvironmentTest(unittest.TestCase):
    def test_keys_and_values(self) -> None:
        env = dok.build_environment(ALL_SECRETS, "https://mlflow.example.test", "dok-exp")
        self.assertEqual(
            set(env),
            {
                "AWS_ACCESS_KEY_ID",
                "AWS_SECRET_ACCESS_KEY",
                "AWS_DEFAULT_REGION",
                "MLFLOW_S3_ENDPOINT_URL",
                "DOK_BUCKET",
                "DOK_PREFIX",
                "MLFLOW_TRACKING_URI",
                "MLFLOW_TRACKING_USERNAME",
                "MLFLOW_TRACKING_PASSWORD",
                "MLFLOW_EXPERIMENT_NAME",
                "DOK_REQUIRE_GPU",
            },
        )
        self.assertEqual(env["AWS_ACCESS_KEY_ID"], ALL_SECRETS["sakura_s3_access_key"])
        self.assertEqual(env["AWS_SECRET_ACCESS_KEY"], ALL_SECRETS["sakura_s3_secret_key"])
        self.assertEqual(env["AWS_DEFAULT_REGION"], "us-east-1")
        self.assertEqual(env["MLFLOW_S3_ENDPOINT_URL"], "https://s3.isk01.sakurastorage.jp")
        self.assertEqual(env["DOK_BUCKET"], "hps-data-2026")
        self.assertEqual(env["DOK_PREFIX"], "saito260927")
        self.assertEqual(env["MLFLOW_TRACKING_URI"], "https://mlflow.example.test")
        self.assertEqual(env["MLFLOW_TRACKING_USERNAME"], ALL_SECRETS["mlflow_basic_user"])
        self.assertEqual(env["MLFLOW_TRACKING_PASSWORD"], ALL_SECRETS["mlflow_basic_password"])
        self.assertEqual(env["MLFLOW_EXPERIMENT_NAME"], "dok-exp")
        self.assertEqual(env["DOK_REQUIRE_GPU"], "1")

    def test_no_sakura_prefixed_keys(self) -> None:
        env = dok.build_environment(ALL_SECRETS, "https://mlflow.example.test", "dok")
        self.assertFalse([k for k in env if k.startswith("SAKURA_")], sorted(env))

    def test_missing_names_only(self) -> None:
        secrets = dict(ALL_SECRETS)
        del secrets["mlflow_basic_password"]
        del secrets["sakura_s3_access_key"]
        with self.assertRaises(dok.MissingSecrets) as ctx:
            dok.build_environment(secrets, "https://mlflow.example.test", "dok")
        self.assertEqual(
            sorted(ctx.exception.names), ["MLFLOW_BASIC_PASSWORD", "SAKURA_S3_ACCESS_KEY"]
        )
        self.assertNotIn(SECRET, str(ctx.exception))
        self.assertNotIn(SECRET, repr(ctx.exception.args))


class RunTaskTest(unittest.TestCase):
    def _run(self, api, args=None, secrets=None):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            with mock.patch.object(dok, "_request", api), mock.patch.object(
                dok, "_fetch_text", lambda url, timeout=30: api.output_text
            ):
                code = dok.run_task(args or make_args(), secrets or dict(ALL_SECRETS))
        return code, out.getvalue(), err.getvalue()

    def test_happy_path_returns_zero(self) -> None:
        api = FakeApi(poll_statuses=["running", "done"], exit_code=0)
        code, out, _ = self._run(api)
        self.assertEqual(code, 0)
        self.assertIn("task_id=task-1", out)
        self.assertIn("status=running", out)
        self.assertIn("status=done", out)
        self.assertIn('{"summary": "ok"}', out)
        self.assertIn("execution_seconds=3", out)
        self.assertEqual(api.calls_matching("POST", "/tasks/"), ["/tasks/"])
        self.assertEqual(api.calls_matching("POST", "/tasks/task-1/cancel/"), [])

    def test_error_status_returns_one(self) -> None:
        api = FakeApi(poll_statuses=["error"], exit_code=1)
        code, _, _ = self._run(api)
        self.assertEqual(code, 1)
        self.assertTrue(api.calls_matching("POST", "/tasks/task-1/cancel/") == [])

    def test_done_with_nonzero_exit_code_returns_one(self) -> None:
        api = FakeApi(poll_statuses=["done"], exit_code=3)
        code, _, _ = self._run(api)
        self.assertEqual(code, 1)

    def test_running_task_blocks_start(self) -> None:
        api = FakeApi(running=[{"id": "task-9", "status": "running", "name": "old"}])
        code, out, _ = self._run(api)
        self.assertEqual(code, 3)
        self.assertIn("task-9", out)
        self.assertEqual(api.calls_matching("POST", "/tasks/"), [])

    def test_keyboard_interrupt_cancels(self) -> None:
        api = FakeApi(poll_statuses=[KeyboardInterrupt(), "running", "running"])
        with self.assertRaises(KeyboardInterrupt):
            self._run(api)
        self.assertEqual(api.calls_matching("POST", "/tasks/task-1/cancel/"), ["/tasks/task-1/cancel/"])

    def test_timeout_cancels(self) -> None:
        # 常に running。limit_sec + 120 秒で打ち切って、finally で止める。
        api = FakeApi(poll_statuses=["running"] * 50)
        clock = itertools.count(0, 300)
        with mock.patch.object(dok, "_monotonic", side_effect=lambda: next(clock)):
            code, _, _ = self._run(api, args=make_args(limit_sec=600))
        self.assertEqual(code, 1)
        self.assertEqual(api.calls_matching("POST", "/tasks/task-1/cancel/"), ["/tasks/task-1/cancel/"])

    def test_environment_names_are_sent_and_values_are_not_shown(self) -> None:
        api = FakeApi(poll_statuses=["done"], exit_code=0)
        _, out, _ = self._run(api)
        self.assertNotIn(SECRET, out)
        body = api.posted_tasks[0]
        self.assertEqual(body["name"], "dok-test-20260929-153000")
        self.assertEqual(body["containers"][0]["plan"], "v100-32gb")
        self.assertEqual(body["containers"][0]["image"], "test-saito.sakuracr.jp/dok-test:0.1")
        self.assertEqual(body["containers"][0]["registry"], "reg-1")
        self.assertEqual(body["execution_time_limit_sec"], 900)
        self.assertEqual(body["containers"][0]["environment"]["DOK_REQUIRE_GPU"], "1")

    def test_missing_registry_is_refused(self) -> None:
        api = FakeApi(registries=())
        code, _, err = self._run(api)
        self.assertEqual(code, 2)
        self.assertIn("registry", err.lower())
        self.assertEqual(api.calls_matching("POST", "/tasks/"), [])

    def test_limit_sec_below_minimum_refused(self) -> None:
        api = FakeApi()
        code, _, err = self._run(api, args=make_args(limit_sec=300))
        self.assertEqual(code, 2)
        self.assertIn("600", err)
        self.assertEqual(api.calls_matching("POST", "/tasks/"), [])


class MainTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.env_file = Path(self.tmp.name) / ".env"
        self.env_file.write_text(
            "\n".join(f"{k.upper()}={v}" for k, v in ALL_SECRETS.items()) + "\n",
            encoding="utf-8",
        )

    def _main(self, argv, environ=None, env_file=None):
        out, err = io.StringIO(), io.StringIO()
        env = dict(environ or {})
        with redirect_stdout(out), redirect_stderr(err):
            code = dok.main(argv, environ=env, env_files=[str(env_file or self.env_file)])
        return code, out.getvalue(), err.getvalue()

    def test_auth_subcommand(self) -> None:
        api = FakeApi()
        with mock.patch.object(dok, "_request", api):
            code, out, _ = self._main(["auth"])
        self.assertEqual(code, 0)
        self.assertIn("tos_agreed_at", out)

    def test_missing_secret_names_only(self) -> None:
        # 秘密が一つも無い状態で、名前だけが stderr に出る。通信はしない。
        empty = Path(self.tmp.name) / "empty.env"
        empty.write_text("# nothing\n", encoding="utf-8")
        # _request を潰して、要求が出ていないことを確かめる。
        with mock.patch.object(dok, "_request", side_effect=AssertionError("通信した")):
            code, out, err = self._main(["run"], environ={}, env_file=empty)
        self.assertEqual(code, 2)
        self.assertIn("SAKURA_API_ACCESS_TOKEN", err)
        self.assertNotIn(SECRET, err)
        self.assertNotIn(SECRET, out)

    def test_registry_subcommand_is_idempotent(self) -> None:
        api = FakeApi()
        with mock.patch.object(dok, "_request", api):
            code, out, _ = self._main(["registry"], environ={"DOK_REGISTRY_PASSWORD": "pw"})
        self.assertEqual(code, 0)
        self.assertIn("reg-1", out)
        self.assertEqual(api.calls_matching("POST", "/registries/"), [])

    def test_registry_subcommand_needs_env_only_password(self) -> None:
        api = FakeApi(registries=())
        with mock.patch.object(dok, "_request", api):
            code, _, err = self._main(["registry"])
        self.assertEqual(code, 2)
        self.assertIn("DOK_REGISTRY_PASSWORD", err)
        self.assertNotIn(SECRET, err)

    def test_registry_subcommand_creates_when_missing(self) -> None:
        api = FakeApi(registries=())
        with mock.patch.object(dok, "_request", api):
            code, out, _ = self._main(
                ["registry"], environ={"DOK_REGISTRY_PASSWORD": f"pw-{SECRET}"}
            )
        self.assertEqual(code, 0)
        self.assertIn("registry_id=reg-1", out)
        posted = api.calls_matching("POST", "/registries/")
        self.assertEqual(posted, ["/registries/"])

    def test_registry_post_error_does_not_show_password(self) -> None:
        api = FakeApi(registries=())

        def boom(method, path, body=None, auth=None, zone=None, timeout=30):
            if method == "POST" and path == "/registries/":
                raise RuntimeError("DOK API POST /registries/ が HTTP 401 を返した: rejected")
            return FakeApi.__call__(api, method, path, body, auth, zone, timeout)

        with mock.patch.object(dok, "_request", boom):
            code, out, err = self._main(
                ["registry"], environ={"DOK_REGISTRY_PASSWORD": f"pw-{SECRET}"}
            )
        self.assertEqual(code, 1)
        self.assertIn("401", err)
        self.assertNotIn(SECRET, err + out)

    def test_status_list_prices_cancel(self) -> None:
        api = FakeApi(poll_statuses=[])
        with mock.patch.object(dok, "_request", api):
            for argv, expect in (
                (["status", "task-1"], 0),
                (["list"], 0),
                (["prices"], 0),
                (["cancel", "task-1"], 0),
            ):
                api.poll_statuses = ["running"]
                code, out, _ = self._main(argv)
                self.assertEqual(code, expect, argv)
                self.assertNotIn(SECRET, out)

    def test_status_prints_exit_code_and_seconds(self) -> None:
        api = FakeApi(poll_statuses=["done"], exit_code=0)
        with mock.patch.object(dok, "_request", api):
            code, out, _ = self._main(["status", "task-1"])
        self.assertEqual(code, 0)
        self.assertIn("status=done", out)
        self.assertIn("exit_code=0", out)
        self.assertIn("execution_seconds=3", out)

    def test_run_delegates_to_run_task(self) -> None:
        seen = {}

        def fake_run(args, secrets):
            seen["name"] = args.name
            seen["zone"] = args.zone
            seen["plan"] = args.plan
            return 0

        with mock.patch.object(dok, "run_task", fake_run):
            code, _, _ = self._main(["run"])
        self.assertEqual(code, 0)
        self.assertEqual(seen["zone"], "is1a")
        self.assertEqual(seen["plan"], "v100-32gb")
        self.assertTrue(seen["name"].startswith("dok-test-"), seen["name"])

    def test_run_keyboard_interrupt_exits_130(self) -> None:
        with mock.patch.object(dok, "run_task", side_effect=KeyboardInterrupt):
            code, _, err = self._main(["run"])
        self.assertEqual(code, 130)
        self.assertNotIn(SECRET, err)

    def test_run_runtime_error_exits_one(self) -> None:
        with mock.patch.object(dok, "run_task", side_effect=RuntimeError("DOK API が 500 を返した")):
            code, _, err = self._main(["run"])
        self.assertEqual(code, 1)
        self.assertIn("500", err)

    def test_common_options_before_and_after_subcommand(self) -> None:
        # 共通オプションはサブコマンドの前にも後ろにも書ける。
        parser = dok.build_parser()
        front = parser.parse_args(["--zone", "is2a", "--env-file", "a.env", "auth"])
        back = parser.parse_args(["auth", "--zone", "is2a", "--env-file", "a.env"])
        for args in (front, back):
            self.assertEqual(args.command, "auth")
            self.assertEqual(args.zone, "is2a")
            self.assertEqual(args.env_file, ["a.env"])
        # 前にも後ろにも書かなかったときは既定のまま。
        self.assertEqual(parser.parse_args(["run"]).env_file, None)
        self.assertEqual(parser.parse_args(["run"]).zone, dok.DEFAULT_ZONE)

    def test_no_command_prints_help(self) -> None:
        code, out, _ = self._main([])
        self.assertEqual(code, 2)
        self.assertIn("usage", out.lower())


class ApiAuthTest(unittest.TestCase):
    UUID = "12345678-1234-1234-1234-123456789abc"
    LONG = "x" * 64

    def test_keeps_correct_order(self) -> None:
        secrets = {"sakura_api_access_token": self.UUID, "sakura_api_secret_key": self.LONG}
        self.assertEqual(dok._api_auth(secrets), (self.UUID, self.LONG))

    def test_swaps_when_reversed(self) -> None:
        secrets = {"sakura_api_access_token": self.LONG, "sakura_api_secret_key": self.UUID}
        with redirect_stderr(io.StringIO()) as err:
            self.assertEqual(dok._api_auth(secrets), (self.UUID, self.LONG))
        self.assertNotIn(self.LONG, err.getvalue())


class DriveShellTest(unittest.TestCase):
    """`_drive_shell` の目印処理。ssh の代わりに小さな python を「シェル」にする。"""

    FAKE_SHELL = (
        "import sys\n"
        "for line in sys.stdin:\n"
        "    if line.startswith('base64 -d'):\n"
        "        print('script output', flush=True)\n"
        "        print('__DOK_' + 'DONE_7__', flush=True)\n"
        "    if line.strip() == 'exit':\n"
        "        break\n"
    )

    def test_returns_script_exit_code_and_streams_output(self) -> None:
        out = io.StringIO()
        with redirect_stdout(out), mock.patch.object(dok.time, "sleep", lambda seconds: None):
            code = dok._drive_shell([sys.executable, "-c", self.FAKE_SHELL], b"echo hi\n", timeout=20)
        self.assertEqual(code, 7)
        self.assertIn("script output", out.getvalue())

    def test_times_out_when_no_marker(self) -> None:
        hang = "import time\ntime.sleep(30)\n"
        with redirect_stdout(io.StringIO()), mock.patch.object(dok.time, "sleep", lambda seconds: None):
            code = dok._drive_shell([sys.executable, "-c", hang], b"echo hi\n", timeout=1)
        self.assertEqual(code, dok.SSH_TIMEOUT_CODE)


class HelpTest(unittest.TestCase):
    def test_help_exits_zero(self) -> None:
        for argv in (
            ["--help"],
            ["run", "--help"],
            ["registry", "--help"],
            ["list", "--help"],
            ["ssh-key", "--help"],
            ["ssh-run", "--help"],
        ):
            out = io.StringIO()
            with redirect_stdout(out), self.assertRaises(SystemExit) as ctx:
                dok.main(argv)
            self.assertEqual(ctx.exception.code, 0, argv)
            self.assertIn("usage", out.getvalue().lower())


class RequestTest(unittest.TestCase):
    def test_http_error_message_has_no_authorization(self) -> None:
        import urllib.error

        secret = SECRET
        err = urllib.error.HTTPError(
            "https://secure.sakura.ad.jp/x",
            403,
            "Forbidden",
            {},
            io.BytesIO(b"nope"),
        )
        with mock.patch.object(dok.request, "urlopen", side_effect=err):
            with self.assertRaises(RuntimeError) as ctx:
                dok._request("GET", "/tasks/", None, ("user", secret), "is1a", timeout=5)
        message = str(ctx.exception)
        self.assertIn("403", message)
        self.assertNotIn(secret, message)
        self.assertNotIn("Basic", message)

    def test_url_shape(self) -> None:
        captured = {}

        class FakeResp:
            def read(self):
                return b'{"ok": true}'

            def __enter__(self):
                return self

            def __exit__(self, *a):
                return False

        def fake_urlopen(req, timeout=None):
            captured["url"] = req.full_url
            captured["timeout"] = timeout
            captured["method"] = req.get_method()
            return FakeResp()

        with mock.patch.object(dok.request, "urlopen", fake_urlopen):
            result = dok._request("GET", "/auth/", None, ("u", SECRET), "is1a")
        self.assertEqual(
            captured["url"],
            "https://secure.sakura.ad.jp/cloud/zone/is1a/api/managed-container/1.0/auth/",
        )
        self.assertEqual(captured["timeout"], 30)
        self.assertEqual(captured["method"], "GET")
        self.assertEqual(result, {"ok": True})


if __name__ == "__main__":
    unittest.main()
