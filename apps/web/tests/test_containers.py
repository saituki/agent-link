"""プロジェクトの作成・コンテナの操作・API キーと Claude のトークン（実装計画 U09）のテスト。

確かめること（AC1〜AC4・AC6。AC5 の実物の Docker は U08 とのマージ後に kantoku が行う）:
- `run_args` が共有契約 C8 の全文と一致し、bind が root・home・Claude の共有のフォルダの 3 つ
  （U10 で `<home>/.claude` に共有のフォルダを足した）
- `create_project` が docker run → init 3 本の順に呼ぶ。途中の失敗で `docker rm -f` と
  フォルダの削除が走る。名前の誤りは docker を呼ばずに 400。同じ名前は 409。
  ひな形が上書きせずに写る
- `PUT /api/me/keys` が `secrets.env` を 0600 で書き、`GET` が値を返さず `last4` だけを返す。
  改行入りの値・`KEY_NAMES` に無い名前は 400。実行中のプロジェクトは `pending` に入り、
  作り直されない
- 他人のプロジェクト名で `/container` を叩くと 404（docker を呼ばない）。一般の利用者の
  `/api/admin/containers` は 403
- 多人数モードの `daemon_alive` が `daemon.status` の更新時刻で決まる

docker は偽物（`SAI_DOCKER_BIN`）を差す。呼ばれた引数を記録する。

実行:
    cd ~/workdir/Sai/sai-tree
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_containers.py
"""

from __future__ import annotations

import asyncio
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path

# import より前に一時フォルダを決める（config.DB_PATH は import 時に決まるため）
_TMP = Path(tempfile.mkdtemp(prefix="sai-web-containers-"))
os.environ["SAI_HOME"] = str(_TMP / "sai-home")
os.environ["SAI_WEB_USERS_ROOT"] = str(_TMP / "users")
os.environ["SAI_WEB_COOKIE_SECURE"] = "0"
os.environ.pop("SAI_WEB_TOKEN", None)
os.environ.pop("SAI_WEB_RUNS_DIR", None)
os.environ.pop("SAI_WEB_SKEL_DIR", None)

# 偽の docker を差す（呼ばれた引数を log.jsonl に記録する）
_FAKE_DIR = _TMP / "fake-docker"
_FAKE_DIR.mkdir(parents=True)
_FAKE_BIN = _TMP / "fake-docker-bin"
_FAKE_BIN.write_text(
    '''#!/usr/bin/env python3
"""テスト用の偽 docker。引数を記録し、設定（config.json）に従って応答する。"""
import json
import os
import sys

d = os.environ.get("FAKE_DOCKER_DIR") or "."
args = sys.argv[1:]
with open(os.path.join(d, "log.jsonl"), "a", encoding="utf-8") as f:
    f.write(json.dumps(args, ensure_ascii=False) + "\\n")

cfg = {}
cfg_path = os.path.join(d, "config.json")
if os.path.exists(cfg_path):
    with open(cfg_path, encoding="utf-8") as f:
        cfg = json.load(f)

for rule in cfg.get("fail", []):
    match = rule.get("match", [])
    if match and all(any(m in a for a in args) for m in match):
        sys.stderr.write(str(rule.get("stderr", "docker が失敗しました")) + "\\n")
        sys.exit(int(rule.get("returncode", 1)))

cmd = args[0] if args else ""
if cmd == "inspect":
    name = args[-1]
    if name in cfg.get("missing", []):
        sys.stderr.write("Error: No such object: " + name + "\\n")
        sys.exit(1)
    if any(".Mounts" in a for a in args):
        sys.stdout.write(str(cfg.get("mounts", "/h/.claude /w ")) + "\\n")
    elif any(".HostConfig" in a for a in args):
        ports = cfg.get("ports", {"8210/tcp": [{"HostIp": "127.0.0.1", "HostPort": ""}]})
        sys.stdout.write(json.dumps(ports) + "\\n")
    else:
        sys.stdout.write(str(cfg.get("inspect_status", "running")) + "\\n")
elif cmd == "ps":
    for row in cfg.get("ps", []):
        sys.stdout.write(json.dumps(row, ensure_ascii=False) + "\\n")
sys.exit(0)
''',
    encoding="utf-8",
)
os.chmod(_FAKE_BIN, 0o755)
os.environ["SAI_DOCKER_BIN"] = str(_FAKE_BIN)
os.environ["FAKE_DOCKER_DIR"] = str(_FAKE_DIR)

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from fastapi import HTTPException  # noqa: E402

from sai_web import auth, containers, control, registry, store  # noqa: E402
from sai_web.main import app  # noqa: E402

HOST = "testserver"
USERS = Path(os.environ["SAI_WEB_USERS_ROOT"])


# --- 偽の docker の設定と記録 ---


def set_docker(**cfg) -> None:
    (_FAKE_DIR / "config.json").write_text(
        json.dumps(cfg, ensure_ascii=False), encoding="utf-8"
    )


def docker_calls() -> list[list[str]]:
    path = _FAKE_DIR / "log.jsonl"
    if not path.exists():
        return []
    return [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def clear_docker_log() -> None:
    path = _FAKE_DIR / "log.jsonl"
    if path.exists():
        path.unlink()


def expect(code: int, fn) -> HTTPException:
    """fn は指定した状態番号の HTTPException で失敗し、日本語の理由が付くこと。"""
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{code} のときは日本語で理由を書く"
        return e
    raise AssertionError(f"{code} になるはず")


class FakeCli:
    """`control.run_cli` の置き換え係（`project summary` の応答を返す）。"""

    def __init__(self, stdout: str = "{}") -> None:
        self.calls: list[list[str]] = []
        self.stdout = stdout
        self.stderr = ""
        self.returncode = 0

    def __call__(self, args, *, app, cwd=None, env=None, timeout=20.0, input_text=None):
        self.calls.append(list(args))
        return subprocess.CompletedProcess(list(args), self.returncode, self.stdout, self.stderr)


# --- ASGI を直接叩く呼び出し係（httpx を使わない） ---


async def _run_asgi(method: str, path: str, body: bytes, headers: list[tuple[bytes, bytes]]):
    scope = {
        "type": "http",
        "asgi": {"version": "3.0"},
        "http_version": "1.1",
        "method": method,
        "scheme": "http",
        "path": path,
        "raw_path": path.encode(),
        "query_string": b"",
        "root_path": "",
        "headers": headers,
        "client": ("127.0.0.1", 40000),
        "server": (HOST, 80),
    }
    messages: list[dict] = []
    state = {"sent": False}

    async def receive():
        if state["sent"]:
            return {"type": "http.disconnect"}
        state["sent"] = True
        return {"type": "http.request", "body": body, "more_body": False}

    async def send(message):
        messages.append(message)

    await app(scope, receive, send)
    return messages


def call(method: str, path: str, *, json_body: dict | None = None, cookie: str | None = None):
    hdrs: list[tuple[bytes, bytes]] = [(b"host", HOST.encode())]
    body = b""
    if json_body is not None:
        body = json.dumps(json_body, ensure_ascii=False).encode("utf-8")
        hdrs.append((b"content-type", b"application/json"))
    if cookie:
        hdrs.append((b"cookie", f"{auth.SESSION_COOKIE}={cookie}".encode()))
    messages = asyncio.run(_run_asgi(method, path, body, hdrs))
    start = next(m for m in messages if m["type"] == "http.response.start")
    headers: dict[str, list[str]] = {}
    for key, value in start["headers"]:
        headers.setdefault(key.decode(), []).append(value.decode())
    raw = b"".join(m.get("body", b"") for m in messages if m["type"] == "http.response.body")
    try:
        parsed = json.loads(raw.decode("utf-8")) if raw else {}
    except (json.JSONDecodeError, UnicodeDecodeError):
        parsed = raw
    return start["status"], parsed, headers


def login(email: str, password: str) -> str:
    _, _, headers = call(
        "POST", "/api/auth/login", json_body={"email": email, "password": password}
    )
    for value in headers.get("set-cookie", []):
        if value.startswith(f"{auth.SESSION_COOKIE}="):
            return value.split(";", 1)[0].split("=", 1)[1]
    raise AssertionError("Cookie が付かない")


def add_user(name: str, email: str, password: str, is_admin: bool = False) -> None:
    auth.create_user_dirs(name)
    store.create_user(
        name,
        email,
        auth.hash_password(password),
        is_admin=is_admin,
        must_change_password=False,
    )


# --- テスト本体 ---


def test_run_args() -> None:
    """`run_args` が C8 の全文と一致し、bind が root・home・Claude の共有のフォルダの 3 つ。"""
    root = USERS / "alice" / "projects" / "alice-x"
    home = USERS / "alice" / "homes" / "alice-x"
    args = containers.run_args("alice", root, home, "TOKEN123")
    expected = [
        "run",
        "-d",
        "--name",
        "sai-alice-x",
        "--init",
        "--restart",
        "unless-stopped",
        "--memory",
        "8g",
        "--memory-swap",
        "8g",
        "--cpus",
        "2",
        "--pids-limit",
        "1024",
        "--security-opt",
        "no-new-privileges",
        "--cap-drop",
        "ALL",
        "--add-host",
        "host.docker.internal:host-gateway",
        "-v",
        f"{root}:{root}",
        "-v",
        f"{home}:{home}",
        "-v",
        f"{USERS / 'alice' / 'claude'}:{home / '.claude'}",
        "-w",
        str(root),
        "-e",
        f"HOME={home}",
        "-e",
        f"SAI_PROJECT_ROOT={root}",
        "-e",
        "SAI_WEB_URL=http://host.docker.internal:8200",
        "-e",
        "SAI_WEB_INTERNAL_TOKEN=TOKEN123",
        "--env-file",
        str(USERS / "alice" / "secrets.env"),
        "--label",
        "sai.owner=alice",
        "--label",
        "sai.project=alice-x",
        "-p",
        "127.0.0.1::8210",
        "sai-tree:latest",
    ]
    assert args == expected, args
    binds = [args[i + 1] for i, a in enumerate(args) if a == "-v"]
    assert binds == [
        f"{root}:{root}",
        f"{home}:{home}",
        f"{USERS / 'alice' / 'claude'}:{home / '.claude'}",
    ], binds

    # 環境変数で変えられる（既定の上書き）
    os.environ["SAI_CONTAINER_MEMORY"] = "4g"
    os.environ["SAI_CONTAINER_CPUS"] = "1"
    os.environ["SAI_WEB_URL_FOR_CONTAINERS"] = "http://10.0.0.1:8200"
    os.environ["SAI_IMAGE"] = "sai-tree:check"
    try:
        got = containers.run_args("alice", root, home, "T")
        assert got[got.index("--memory") + 1] == "4g"
        assert got[got.index("--memory-swap") + 1] == "4g"
        assert got[got.index("--cpus") + 1] == "1"
        assert "SAI_WEB_URL=http://10.0.0.1:8200" in got, got
        assert got[-1] == "sai-tree:check"
    finally:
        for name in (
            "SAI_CONTAINER_MEMORY",
            "SAI_CONTAINER_CPUS",
            "SAI_WEB_URL_FOR_CONTAINERS",
            "SAI_IMAGE",
        ):
            os.environ.pop(name, None)


def test_daemon_alive_multi() -> None:
    """多人数モードの `daemon_alive` は `daemon.status` の更新時刻で決まる。"""
    root = _TMP / "daemon-root"
    status = root / ".sai" / "sai-engine" / "daemon.status"
    status.parent.mkdir(parents=True, exist_ok=True)
    # 新しいファイル → 生きている
    status.write_text("{}", encoding="utf-8")
    now = time.time()
    os.utime(status, (now, now))
    assert registry.daemon_alive(str(root)) is True
    # 古いファイル → 死んでいる
    old = now - 100
    os.utime(status, (old, old))
    assert registry.daemon_alive(str(root)) is False
    # 無い → 死んでいる
    status.unlink()
    assert registry.daemon_alive(str(root)) is False


def test_create_project_order() -> None:
    """`create_project` が docker run → init 3 本の順に呼ぶ。"""
    set_docker()
    clear_docker_log()
    result = containers.create_project("alice", "x")
    assert result == {"name": "alice-x", "container": "sai-alice-x"}, result

    root = USERS / "alice" / "projects" / "alice-x"
    home = USERS / "alice" / "homes" / "alice-x"
    assert root.is_dir() and home.is_dir(), (root, home)
    assert stat.S_IMODE(root.stat().st_mode) == 0o700, oct(root.stat().st_mode)
    assert stat.S_IMODE(home.stat().st_mode) == 0o700, oct(home.stat().st_mode)
    secrets = USERS / "alice" / "secrets.env"
    assert secrets.is_file(), secrets
    assert stat.S_IMODE(secrets.stat().st_mode) == 0o600, oct(secrets.stat().st_mode)

    calls = docker_calls()
    assert calls and calls[0][0] == "run", calls
    assert calls[0][calls[0].index("--name") + 1] == "sai-alice-x"
    execs = [c for c in calls if c and c[0] == "exec"]
    assert len(execs) == 3, execs
    assert execs[0][-4:] == ["sai-link", "init", "--name", "alice-x"], execs[0]
    assert execs[1][-2:] == ["sai-paper", "init"], execs[1]
    assert execs[2][-3:] == ["sai-link", "daemon", "start"], execs[2]
    for c in execs:
        assert c[c.index("-w") + 1] == str(root), c
        assert f"SAI_PROJECT_ROOT={root}" in c, c
        assert "sai-alice-x" in c, c

    # 内部の合言葉が DB に SHA-256 で入り、コンテナの引数の値と一致する
    token_arg = next(a for a in calls[0] if a.startswith("SAI_WEB_INTERNAL_TOKEN="))
    token = token_arg.split("=", 1)[1]
    assert token and not token.startswith("sai_"), token
    import hashlib

    row = store.find_project_token(hashlib.sha256(token.encode()).hexdigest())
    assert row is not None and row["owner"] == "alice" and row["project"] == "alice-x", row


def test_create_project_duplicate() -> None:
    """同じ名前は 409（docker を呼ばない）。"""
    clear_docker_log()
    expect(409, lambda: containers.create_project("alice", "x"))
    assert docker_calls() == [], docker_calls()


def test_create_project_invalid_name() -> None:
    """名前の誤りは docker を呼ばずに 400。"""
    clear_docker_log()
    for bad in ("X", "-x", "x_y", "x" * 31, ""):
        e = expect(400, lambda b=bad: containers.create_project("alice", b))
        assert "30 文字" in str(e.detail), e.detail
    assert docker_calls() == [], docker_calls()


def test_create_project_failure_cleanup() -> None:
    """途中の失敗で `docker rm -f` とフォルダの削除が走る。"""
    set_docker(fail=[{"match": ["sai-paper"], "stderr": "init に失敗しました", "returncode": 1}])
    clear_docker_log()
    try:
        e = expect(502, lambda: containers.create_project("alice", "fail"))
        assert "init に失敗しました" in str(e.detail), e.detail
    finally:
        set_docker()

    calls = docker_calls()
    assert any(c[:3] == ["rm", "-f", "sai-alice-fail"] for c in calls), calls
    assert not (USERS / "alice" / "projects" / "alice-fail").exists()
    assert not (USERS / "alice" / "homes" / "alice-fail").exists()


def test_skeleton_not_overwritten() -> None:
    """ひな形は home に無いファイルだけ写す（先に写した物を上書きしない）。
    U10: `.pi/agent/models.json` は写さない（pi を廃止したため）。"""
    skel = _TMP / "skel"
    (skel / ".pi" / "agent").mkdir(parents=True)
    (skel / "a.txt").write_text("skel-a", encoding="utf-8")
    (skel / ".pi" / "agent" / "models.json").write_text("skel-model", encoding="utf-8")
    user_home = USERS / "alice" / "home"
    (user_home / ".pi" / "agent").mkdir(parents=True, exist_ok=True)
    (user_home / ".pi" / "agent" / "models.json").write_text("user-model", encoding="utf-8")

    os.environ["SAI_WEB_SKEL_DIR"] = str(skel)
    set_docker()
    try:
        containers.create_project("alice", "skel")
    finally:
        os.environ.pop("SAI_WEB_SKEL_DIR", None)

    home = USERS / "alice" / "homes" / "alice-skel"
    assert (home / "a.txt").read_text(encoding="utf-8") == "skel-a"
    # skel の中身は写す（models.json も skel の物なら写る）
    assert (home / ".pi" / "agent" / "models.json").read_text(encoding="utf-8") == "skel-model"

    # skel が無ければ、ユーザーの home の models.json は写らない（U10 で写す処理を消した）
    set_docker()
    containers.create_project("alice", "skel2")
    home2 = USERS / "alice" / "homes" / "alice-skel2"
    assert not (home2 / ".pi").exists(), "models.json を写してはいけない"


def test_claude_dir_and_history_copied_once() -> None:
    """共有のログインの置き場（0700）へ、HOME の会話の記録を一度だけ写す。
    `.credentials.json` は写さず、元の `.claude` は消さない。2 回目は写さない。"""
    shared = USERS / "alice" / "claude"
    if shared.exists():
        shutil.rmtree(shared)
    project_home = USERS / "alice" / "homes" / "alice-hist"
    old = project_home / ".claude"
    (old / "projects" / "-srv-x").mkdir(parents=True)
    (old / "projects" / "-srv-x" / "s1.jsonl").write_text("one", encoding="utf-8")
    (old / ".credentials.json").write_text("OLD-SHORT-TOKEN", encoding="utf-8")
    (old / "settings.json").write_text("{}", encoding="utf-8")

    set_docker()
    clear_docker_log()
    containers.create_container("alice", "alice-hist")
    assert stat.S_IMODE(shared.stat().st_mode) == 0o700, oct(shared.stat().st_mode)
    assert (shared / "projects" / "-srv-x" / "s1.jsonl").read_text(encoding="utf-8") == "one"
    assert (shared / "settings.json").exists()
    assert not (shared / ".credentials.json").exists(), "短命の token を共有へ写してはいけない"
    assert (old / "projects" / "-srv-x" / "s1.jsonl").exists(), "元の会話の記録を消してはいけない"
    assert (old / ".copied-to-shared").exists(), "写し終えの印が無い"
    # docker run の引数に共有のフォルダの mount が入る
    run = next(c for c in docker_calls() if c and c[0] == "run")
    assert f"{shared}:{project_home / '.claude'}" in run, run

    # 2 回目: 印があるので写さない（後から増えた物は共有へ来ない）
    (old / "projects" / "-srv-x" / "s2.jsonl").write_text("two", encoding="utf-8")
    containers.create_container("alice", "alice-hist")
    assert not (shared / "projects" / "-srv-x" / "s2.jsonl").exists(), "2 回目に写してはいけない"


def test_api_create_project() -> None:
    """`POST /api/projects` が作った名前とコンテナを返す。"""
    set_docker()
    clear_docker_log()
    cookie = login("alice@uni.edu", "alicepass1")
    status, body, _ = call("POST", "/api/projects", json_body={"name": "z"}, cookie=cookie)
    assert status == 200, (status, body)
    assert body == {"name": "alice-z", "container": "sai-alice-z"}, body
    # 誤った名前は 400
    status, body, _ = call("POST", "/api/projects", json_body={"name": "BAD"}, cookie=cookie)
    assert status == 400, (status, body)


def test_keys_api() -> None:
    """鍵の保存・読み出し・検査と、実行中のプロジェクトの作り直し。"""
    cookie = login("alice@uni.edu", "alicepass1")

    # 最初は全部未設定
    status, body, _ = call("GET", "/api/me/keys", cookie=cookie)
    assert status == 200, (status, body)
    assert [k["name"] for k in body["keys"]] == list(containers.KEY_NAMES)
    assert all(k["set"] is False and k["last4"] == "" for k in body["keys"]), body

    # 保存（実行中のコンテナが無いので recreated も pending も空）
    set_docker(ps=[])
    status, body, _ = call(
        "PUT",
        "/api/me/keys",
        json_body={"OPENROUTER_API_KEY": "sk-or-1234567890"},
        cookie=cookie,
    )
    assert status == 200, (status, body)
    assert body == {"ok": True, "recreated": [], "pending": []}, body

    secrets = USERS / "alice" / "secrets.env"
    assert stat.S_IMODE(secrets.stat().st_mode) == 0o600, oct(secrets.stat().st_mode)
    assert "OPENROUTER_API_KEY=sk-or-1234567890" in secrets.read_text(encoding="utf-8")

    # GET は値ではなく last4 だけを返す
    status, body, _ = call("GET", "/api/me/keys", cookie=cookie)
    assert status == 200, (status, body)
    by_name = {k["name"]: k for k in body["keys"]}
    assert by_name["OPENROUTER_API_KEY"] == {
        "name": "OPENROUTER_API_KEY",
        "set": True,
        "last4": "7890",
    }, by_name
    assert "sk-or" not in json.dumps(body), body

    # 改行入りの値は 400
    status, body, _ = call(
        "PUT", "/api/me/keys", json_body={"OPENROUTER_API_KEY": "a\nb"}, cookie=cookie
    )
    assert status == 400, (status, body)
    # KEY_NAMES に無い名前は 400
    status, body, _ = call(
        "PUT", "/api/me/keys", json_body={"UNKNOWN_KEY": "x"}, cookie=cookie
    )
    assert status == 400, (status, body)
    # 空文字は消す
    status, body, _ = call(
        "PUT", "/api/me/keys", json_body={"OPENROUTER_API_KEY": ""}, cookie=cookie
    )
    assert status == 200, (status, body)
    assert "OPENROUTER_API_KEY" not in secrets.read_text(encoding="utf-8")

    # --- 実行中のプロジェクトは pending に入り、作り直されない ---
    root = USERS / "alice" / "projects" / "alice-x"
    (root / ".sai" / "sai-engine").mkdir(parents=True, exist_ok=True)
    (root / ".sai" / "sai-engine" / "sai-engine.db").write_text("", encoding="utf-8")
    set_docker(
        ps=[
            {
                "Names": "sai-alice-x",
                "Labels": "sai.owner=alice,sai.project=alice-x",
                "State": "running",
                "Status": "Up 5 minutes",
            }
        ]
    )
    fake = FakeCli(json.dumps({"running": 1, "last_executed_at": None}))
    original = control.run_cli
    control.run_cli = fake
    control._summary_cache.clear()
    clear_docker_log()
    try:
        status, body, _ = call(
            "PUT", "/api/me/keys", json_body={"NCBI_API_KEY": "ncbi-1234"}, cookie=cookie
        )
    finally:
        control.run_cli = original
        control._summary_cache.clear()
    assert status == 200, (status, body)
    assert body == {"ok": True, "recreated": [], "pending": ["alice-x"]}, body
    # docker は ps だけ（rm も run も呼ばない）
    calls = docker_calls()
    assert all(c and c[0] == "ps" for c in calls), calls

    # --- 実行中でなければ作り直す ---
    set_docker(
        ps=[
            {
                "Names": "sai-alice-x",
                "Labels": "sai.owner=alice,sai.project=alice-x",
                "State": "exited",
                "Status": "Exited (0) 2 minutes ago",
            }
        ]
    )
    fake = FakeCli(json.dumps({"running": 0, "last_executed_at": None}))
    control.run_cli = fake
    control._summary_cache.clear()
    clear_docker_log()
    try:
        status, body, _ = call(
            "PUT", "/api/me/keys", json_body={"NCBI_API_KEY": "ncbi-5678"}, cookie=cookie
        )
    finally:
        control.run_cli = original
        control._summary_cache.clear()
    assert status == 200, (status, body)
    assert body == {"ok": True, "recreated": ["alice-x"], "pending": []}, body
    calls = docker_calls()
    assert any(c[:3] == ["rm", "-f", "sai-alice-x"] for c in calls), calls
    assert any(c and c[0] == "run" for c in calls), calls


def test_project_container_api() -> None:
    """自分のプロジェクトのコンテナの状態と起動・停止。他人の物は 404（docker を呼ばない）。"""
    cookie = login("alice@uni.edu", "alicepass1")
    set_docker(ps=[], inspect_status="running")
    status, body, _ = call("GET", "/api/projects/alice-x/container", cookie=cookie)
    assert status == 200, (status, body)
    assert body == {"container": "sai-alice-x", "state": "running"}, body

    # 起動・停止（存在するので docker start / stop）
    clear_docker_log()
    status, body, _ = call("POST", "/api/projects/alice-x/container/start", cookie=cookie)
    assert status == 200, (status, body)
    assert any(c[:2] == ["start", "sai-alice-x"] for c in docker_calls()), docker_calls()
    status, body, _ = call("POST", "/api/projects/alice-x/container/stop", cookie=cookie)
    assert status == 200, (status, body)
    assert any(c[:3] == ["stop", "-t", "20"] for c in docker_calls()), docker_calls()
    # 未知の action は 400
    status, body, _ = call("POST", "/api/projects/alice-x/container/reload", cookie=cookie)
    assert status == 400, (status, body)

    # 他人のプロジェクトは 404。docker を呼ばない
    clear_docker_log()
    status, body, _ = call("GET", "/api/projects/bob-y/container", cookie=cookie)
    assert status == 404, (status, body)
    status, body, _ = call("POST", "/api/projects/bob-y/container/start", cookie=cookie)
    assert status == 404, (status, body)
    assert docker_calls() == [], docker_calls()


def test_start_recreates_container_without_shared_claude() -> None:
    """止まっていて共有ログインの mount が無い古いコンテナは、起動で作り直す。mount があれば docker start。"""
    cookie = login("alice@uni.edu", "alicepass1")
    set_docker(ps=[], inspect_status="exited", mounts="/h /w ")
    clear_docker_log()
    status, body, _ = call("POST", "/api/projects/alice-x/container/start", cookie=cookie)
    assert status == 200, (status, body)
    calls = docker_calls()
    assert any(c[:3] == ["rm", "-f", "sai-alice-x"] for c in calls), calls
    assert any(c[0] == "run" for c in calls), calls
    assert not any(c[:2] == ["start", "sai-alice-x"] for c in calls), calls

    set_docker(ps=[], inspect_status="exited", mounts="/h/.claude /w ")
    clear_docker_log()
    status, body, _ = call("POST", "/api/projects/alice-x/container/start", cookie=cookie)
    assert status == 200, (status, body)
    calls = docker_calls()
    assert any(c[:2] == ["start", "sai-alice-x"] for c in calls), calls
    assert not any(c[:2] == ["rm", "-f"] for c in calls), calls


def test_start_recreates_container_without_ts_port() -> None:
    """止まっていて時系列の口（8210/tcp）が無い物は作り直す。口があれば docker start。"""
    cookie = login("alice@uni.edu", "alicepass1")
    set_docker(ps=[], inspect_status="exited", mounts="/h/.claude /w ", ports={})
    clear_docker_log()
    status, body, _ = call("POST", "/api/projects/alice-x/container/start", cookie=cookie)
    assert status == 200, (status, body)
    calls = docker_calls()
    assert any(c[:3] == ["rm", "-f", "sai-alice-x"] for c in calls), calls
    assert any(c[0] == "run" for c in calls), calls
    assert not any(c[:2] == ["start", "sai-alice-x"] for c in calls), calls

    set_docker(ps=[], inspect_status="exited", mounts="/h/.claude /w ")
    clear_docker_log()
    status, body, _ = call("POST", "/api/projects/alice-x/container/start", cookie=cookie)
    assert status == 200, (status, body)
    calls = docker_calls()
    assert any(c[:2] == ["start", "sai-alice-x"] for c in calls), calls
    assert not any(c[:2] == ["rm", "-f"] for c in calls), calls


def test_projects_container_state() -> None:
    """`GET /api/projects` の各行に `container_state` が付く。"""
    cookie = login("alice@uni.edu", "alicepass1")
    set_docker(ps=[], inspect_status="exited")
    status, body, _ = call("GET", "/api/projects", cookie=cookie)
    assert status == 200, (status, body)
    names = {p["name"] for p in body["projects"]}
    assert "alice-x" in names, names
    for project in body["projects"]:
        assert project["container_state"] == "exited", project


def test_admin_containers() -> None:
    """管理者の一覧と起動・停止。一般の利用者は 403。名前の誤りは 400。"""
    alice = login("alice@uni.edu", "alicepass1")
    admin = login("admin@uni.edu", "adminpass1")

    # 一般の利用者は 403
    status, body, _ = call("GET", "/api/admin/containers", cookie=alice)
    assert status == 403, (status, body)
    status, body, _ = call(
        "POST", "/api/admin/containers/sai-alice-x/start", cookie=alice
    )
    assert status == 403, (status, body)

    # 管理者は一覧が見える
    set_docker(
        ps=[
            {
                "Names": "sai-alice-x",
                "Labels": "sai.owner=alice,sai.project=alice-x",
                "State": "running",
                "Status": "Up 3 minutes",
            }
        ],
        inspect_status="running",
    )
    status, body, _ = call("GET", "/api/admin/containers", cookie=admin)
    assert status == 200, (status, body)
    assert body["containers"] == [
        {
            "container": "sai-alice-x",
            "owner": "alice",
            "project": "alice-x",
            "state": "running",
            "status": "Up 3 minutes",
        }
    ], body

    # 名前の誤りは 400（docker を呼ばない）
    clear_docker_log()
    status, body, _ = call("POST", "/api/admin/containers/bad/start", cookie=admin)
    assert status == 400, (status, body)
    assert docker_calls() == [], docker_calls()

    # 正しい名前なら起動できる
    status, body, _ = call("POST", "/api/admin/containers/sai-alice-x/start", cookie=admin)
    assert status == 200, (status, body)
    assert any(c[:2] == ["start", "sai-alice-x"] for c in docker_calls()), docker_calls()


def test_admin_list_parsing() -> None:
    """`list_all` がラベルから owner / project を取り出す。"""
    set_docker(
        ps=[
            {
                "Names": "sai-alice-x",
                "Labels": "other=1,sai.owner=alice,sai.project=alice-x",
                "State": "exited",
                "Status": "Exited (0) 1 minute ago",
            },
            {"Names": "no-labels", "Labels": "", "State": "running", "Status": "Up"},
        ]
    )
    rows = containers.list_all()
    assert rows[0] == {
        "container": "sai-alice-x",
        "owner": "alice",
        "project": "alice-x",
        "state": "exited",
        "status": "Exited (0) 1 minute ago",
    }, rows[0]
    assert rows[1]["owner"] == "" and rows[1]["project"] == "", rows[1]


def main() -> None:
    store.init_db()
    add_user("alice", "alice@uni.edu", "alicepass1")
    add_user("bob", "bob@uni.edu", "bobpass123")
    add_user("admin", "admin@uni.edu", "adminpass1", is_admin=True)

    # bob のプロジェクト（alice からは見えない）
    (USERS / "bob" / "projects" / "bob-y" / ".sai").mkdir(parents=True, exist_ok=True)

    test_run_args()
    test_daemon_alive_multi()
    test_create_project_order()
    test_create_project_duplicate()
    test_create_project_invalid_name()
    test_create_project_failure_cleanup()
    test_skeleton_not_overwritten()
    test_claude_dir_and_history_copied_once()
    test_api_create_project()
    test_keys_api()
    test_project_container_api()
    test_start_recreates_container_without_shared_claude()
    test_start_recreates_container_without_ts_port()
    test_projects_container_state()
    test_admin_containers()
    test_admin_list_parsing()
    print("test_containers: すべて通りました")


if __name__ == "__main__":
    main()
