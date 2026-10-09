"""時系列（sai-ts）の起動 `containers.ts_ensure` / `ts_forget`（実装計画 U03・共有契約 C3・C4）のテスト。

確かめること（AC3〜AC5）:
- `ts_ensure` が inspect → inspect（口）→ pkill → init → serve → port の順に docker を呼び、
  `{"base", "token"}` を返す。2 回目は docker を呼ばず同じ値を返す
- `serve` の引数に合言葉が無く、偽の docker が受けた環境変数 `SAI_TS_TOKEN` が返した `token` と同じ
- `ts_forget` のあとは起こし直し、合言葉が変わる
- 止まったコンテナ・口の無いコンテナは 409、答えない上流は 502（待ち時間は 1 秒に縮める）
- 名前の誤りは docker を呼ばずに 400

docker は偽物（`SAI_DOCKER_BIN`）を差す。上流の sai-ts は試験の中で `http.server` を別スレッドで起こす。
上流は `SAI_TS_TOKEN` の値（偽の docker が記録した最後の `serve` の環境変数）と一致する Bearer だけ 200 を返す。

実行:
    cd ~/workdir/Sai/sai-tree
    env -u SAI_LINK_BIN -u SAI_PAPER_BIN -u SAI_SCI_BIN PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_containers_ts.py
"""

from __future__ import annotations

import http.server
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

# import より前に一時フォルダを決める（config.DB_PATH は import 時に決まるため）
_TMP = Path(tempfile.mkdtemp(prefix="sai-web-containers-ts-"))
os.environ["SAI_HOME"] = str(_TMP / "sai-home")
os.environ["SAI_WEB_USERS_ROOT"] = str(_TMP / "users")
os.environ.pop("SAI_WEB_TOKEN", None)
os.environ.pop("SAI_WEB_RUNS_DIR", None)
os.environ.pop("SAI_WEB_SKEL_DIR", None)

# 偽の docker（引数と、`SAI_TS_TOKEN` を渡されたときの環境変数を記録する）
_FAKE_DIR = _TMP / "fake-docker"
_FAKE_DIR.mkdir(parents=True)
_FAKE_BIN = _TMP / "fake-docker-bin"
_FAKE_BIN.write_text(
    '''#!/usr/bin/env python3
"""テスト用の偽 docker。引数を記録し、設定（config.json）に従って応答する。"""
import json
import os
import sys

d = os.environ["FAKE_DOCKER_DIR"]
args = sys.argv[1:]
with open(os.path.join(d, "log.jsonl"), "a", encoding="utf-8") as f:
    f.write(json.dumps(args, ensure_ascii=False) + "\\n")
if "SAI_TS_TOKEN" in os.environ:
    with open(os.path.join(d, "env.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps({"args": args, "SAI_TS_TOKEN": os.environ["SAI_TS_TOKEN"]}) + "\\n")

with open(os.path.join(d, "config.json"), encoding="utf-8") as f:
    cfg = json.load(f)
port = cfg.get("port", 0)
cmd = args[0] if args else ""
if cmd == "inspect":
    if any(".HostConfig" in a for a in args):
        ports = cfg.get("ports", {"8210/tcp": [{"HostIp": "127.0.0.1", "HostPort": ""}]})
        sys.stdout.write(json.dumps(ports) + "\\n")
    else:
        sys.stdout.write(str(cfg.get("inspect_status", "running")) + "\\n")
elif cmd == "port":
    # 0.0.0.0 と [::] で返る（呼ぶ側は 127.0.0.1 に直す）
    sys.stdout.write(f"0.0.0.0:{port}\\n[::]:{port}\\n")
elif cmd == "exec":
    if "pkill" in args:
        sys.exit(int(cfg.get("pkill_rc", 1)))
    if "init" in args:
        sys.stdout.write('{"block": "exists"}\\n')
sys.exit(0)
''',
    encoding="utf-8",
)
os.chmod(_FAKE_BIN, 0o755)
os.environ["SAI_DOCKER_BIN"] = str(_FAKE_BIN)
os.environ["FAKE_DOCKER_DIR"] = str(_FAKE_DIR)

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from fastapi import HTTPException  # noqa: E402

from sai_web import containers  # noqa: E402

NAME = "saituki-west"
CONTAINER = "sai-saituki-west"


# --- 偽の docker の設定と記録 ---


def set_docker(**cfg) -> None:
    (_FAKE_DIR / "config.json").write_text(json.dumps(cfg, ensure_ascii=False), encoding="utf-8")


def _read_jsonl(name: str) -> list:
    path = _FAKE_DIR / name
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def docker_calls() -> list[list[str]]:
    return _read_jsonl("log.jsonl")


def clear_docker_log() -> None:
    for name in ("log.jsonl", "env.jsonl"):
        path = _FAKE_DIR / name
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


# --- 上流の偽の sai-ts（別スレッドの http.server） ---


class _Upstream(http.server.BaseHTTPRequestHandler):
    """`/api/datasets` だけ答える。合言葉は `serve` の環境変数（偽の docker が記録した最後の値）。"""

    def do_GET(self) -> None:  # noqa: N802 - http.server の名前
        envs = _read_jsonl("env.jsonl")
        want = envs[-1]["SAI_TS_TOKEN"] if envs else ""
        auth = self.headers.get("Authorization", "")
        ok = bool(want) and auth == f"Bearer {want}" and self.path == "/api/datasets"
        STATE["seen"].append(auth)
        self.send_response(200 if ok else 401)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b"[]" if ok else b'{"detail": "no"}')

    def log_message(self, *args) -> None:  # 試験の出力を汚さない
        pass


STATE: dict[str, list[str]] = {"seen": []}
_SERVER = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Upstream)
_THREAD = threading.Thread(target=_SERVER.serve_forever, daemon=True)
_THREAD.start()
UPSTREAM_PORT = _SERVER.server_address[1]


def closed_port() -> int:
    """つながらない番号（一度開いて閉じた番号）。"""
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


# --- 試験 ---


def test_ensure_order_and_reuse() -> None:
    """ts_ensure が決まった順に docker を呼び、{"base", "token"} を返す。2 回目は docker を呼ばない。"""
    containers.ts_forget(NAME)
    set_docker(port=UPSTREAM_PORT)
    clear_docker_log()
    got = containers.ts_ensure(NAME)
    assert got["base"] == f"http://127.0.0.1:{UPSTREAM_PORT}", got
    assert len(got["token"]) >= 40, got

    root = f"{_TMP / 'users' / 'saituki' / 'projects' / NAME}"
    calls = docker_calls()
    assert calls[0] == ["inspect", "-f", "{{.State.Status}}", CONTAINER], calls
    assert calls[1] == ["inspect", "-f", "{{json .HostConfig.PortBindings}}", CONTAINER], calls
    assert calls[2] == ["exec", CONTAINER, "pkill", "-f", "sai_ts.cli serve"], calls
    assert calls[3][:1] == ["exec"] and calls[3][-3:] == ["sai-ts", "init", "--json"], calls
    assert calls[3][calls[3].index("-w") + 1] == root, calls[3]
    assert f"SAI_PROJECT_ROOT={root}" in calls[3], calls[3]
    assert calls[4][:2] == ["exec", "-d"] and calls[4][-6:] == [
        "sai-ts", "serve", "--host", "0.0.0.0", "--port", "8210",
    ], calls[4]
    assert calls[5] == ["port", CONTAINER, "8210/tcp"], calls
    assert len(calls) == 6, calls

    clear_docker_log()
    again = containers.ts_ensure(NAME)
    assert again == got, (again, got)
    assert docker_calls() == [], docker_calls()


def test_token_only_in_env_not_argv() -> None:
    """serve の引数に合言葉が無く、偽の docker が受けた SAI_TS_TOKEN が返した token と同じ。上流もその値で 200 を返す。"""
    containers.ts_forget(NAME)
    set_docker(port=UPSTREAM_PORT)
    clear_docker_log()
    got = containers.ts_ensure(NAME)
    token = got["token"]

    for args in docker_calls():
        assert token not in " ".join(args), args
    serve_envs = [e for e in _read_jsonl("env.jsonl") if "serve" in e["args"]]
    assert len(serve_envs) == 1, serve_envs
    assert serve_envs[0]["SAI_TS_TOKEN"] == token, (serve_envs, token)
    assert STATE["seen"] and STATE["seen"][-1] == f"Bearer {token}", STATE["seen"][-3:]


def test_forget_restarts_with_new_token() -> None:
    """ts_forget のあとは起こし直し、合言葉が変わる。"""
    containers.ts_forget(NAME)
    set_docker(port=UPSTREAM_PORT)
    first = containers.ts_ensure(NAME)
    containers.ts_forget(NAME)
    clear_docker_log()
    second = containers.ts_ensure(NAME)
    assert second["token"] != first["token"], (first, second)
    assert any(c[:2] == ["exec", "-d"] for c in docker_calls()), docker_calls()


def test_stopped_container_409() -> None:
    """止まったコンテナは 409（docker の起動は見ない）。"""
    containers.ts_forget(NAME)
    set_docker(port=UPSTREAM_PORT, inspect_status="exited")
    clear_docker_log()
    e = expect(409, lambda: containers.ts_ensure(NAME))
    assert e.detail == "コンテナが動いていません（Projects から起動してください）", e.detail
    assert not any(c[0] == "exec" for c in docker_calls()), docker_calls()


def test_no_ts_port_409() -> None:
    """口の無い動いているコンテナは 409（exec を呼ばない）。"""
    containers.ts_forget(NAME)
    set_docker(port=UPSTREAM_PORT, ports={})
    clear_docker_log()
    e = expect(409, lambda: containers.ts_ensure(NAME))
    assert e.detail.startswith("時系列の口がありません"), e.detail
    assert not any(c[0] == "exec" for c in docker_calls()), docker_calls()


def test_silent_upstream_502() -> None:
    """上流が答えないと 502。待ち時間は 1 秒に縮める。"""
    containers.ts_forget(NAME)
    set_docker(port=closed_port())
    saved = containers._TS_WAIT_SECONDS
    containers._TS_WAIT_SECONDS = 1.0
    try:
        e = expect(502, lambda: containers.ts_ensure(NAME))
        assert e.detail == "時系列のサーバが起動しません", e.detail
    finally:
        containers._TS_WAIT_SECONDS = saved
    # 失敗した値は覚えない（次は起こし直す）
    assert NAME not in containers._TS_UP, containers._TS_UP


def test_bad_name_400() -> None:
    """名前の誤りは docker を呼ばずに 400。"""
    set_docker(port=UPSTREAM_PORT)
    clear_docker_log()
    expect(400, lambda: containers.ts_ensure("saituki/west"))
    assert docker_calls() == [], docker_calls()


def main() -> None:
    test_ensure_order_and_reuse()
    test_token_only_in_env_not_argv()
    test_forget_restarts_with_new_token()
    test_stopped_container_409()
    test_no_ts_port_409()
    test_silent_upstream_502()
    test_bad_name_400()
    _SERVER.shutdown()
    _SERVER.server_close()
    print("test_containers_ts: すべて通りました")


if __name__ == "__main__":
    main()
