"""時系列の入口の守り・券の住所・`sai-web serve` の 2 つの待ち受け（実装計画 U05・共有契約 C6）のテスト。

確かめること（AC4・AC6 の一部）:
- 多人数モードの応答に CORS の許可が無い（OPTIONS と GET）
- 自分の sai-web を枠（iframe）に入れさせない（X-Frame-Options と frame-ancestors）
- Cookie の `sai_session` が 2 つ以上なら `/api/` は 400 と Cookie の消し方を返す
- POST …/ts/open: 持ち主だけ・券の URL を返す・券は ts 入口で 303 に替わる・
  失敗は伝わる・合言葉の無い URL の入口の設定が無いと 503・単独モードは 404
- `SAI_WEB_TS_PORT` は多人数モードだけ（単独モードは終了コード 2）
- 番号ごとに正しい app へ渡し、時系列の住所は access log に出さない

`fastapi.testclient` は httpx を要するので使わない（test_auth.py と同じく ASGI を直接叩く）。

実行:
    cd ~/workdir/Sai/sai-tree
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_ts_open.py
"""

from __future__ import annotations

import asyncio
import dataclasses
import json
import logging
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from urllib.parse import quote

# import より前に一時フォルダを決める（config.DB_PATH は import 時に決まるため）
_TMP = Path(tempfile.mkdtemp(prefix="sai-web-ts-open-"))
os.environ["SAI_HOME"] = str(_TMP / "sai-home")
os.environ["SAI_WEB_USERS_ROOT"] = str(_TMP / "users")
os.environ["SAI_WEB_COOKIE_SECURE"] = "0"
os.environ.pop("SAI_WEB_TOKEN", None)
os.environ.pop("SAI_WEB_TS_PORT", None)
os.environ.pop("SAI_WEB_RUNS_DIR", None)

APP_SRC = Path(__file__).resolve().parent.parent / "src"
sys.path.insert(0, str(APP_SRC))

from fastapi import HTTPException  # noqa: E402

from sai_web import auth, containers, registry, store, ts_gate, webcli  # noqa: E402
from sai_web import main as web_main  # noqa: E402

HOST = "testserver"
TS_URL = "https://ts.example:8443"
ORIGIN_SAME_HOST = "http://testserver:8443"


# --- ASGI を直接叩く呼び出し係（httpx を使わない） ---


async def _run_asgi(app, method: str, path: str, query: str, body: bytes, headers):
    scope = {
        "type": "http",
        "asgi": {"version": "3.0"},
        "http_version": "1.1",
        "method": method,
        "scheme": "http",
        "path": path,
        "raw_path": path.encode(),
        "query_string": query.encode(),
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


def _respond(messages: list[dict]) -> tuple[int, object, dict]:
    start = next(m for m in messages if m["type"] == "http.response.start")
    out_headers: dict[str, list[str]] = {}
    for key, value in start["headers"]:
        out_headers.setdefault(key.decode(), []).append(value.decode())
    raw = b"".join(m.get("body", b"") for m in messages if m["type"] == "http.response.body")
    try:
        parsed = json.loads(raw.decode("utf-8")) if raw else {}
    except (json.JSONDecodeError, UnicodeDecodeError):
        parsed = raw
    return start["status"], parsed, out_headers


def call(
    method: str,
    path: str,
    *,
    json_body: dict | None = None,
    cookie_header: str | None = None,
    cookie: str | None = None,
    origin: str | None = None,
    app=None,
    query: str = "",
) -> tuple[int, object, dict]:
    """1 つの住所を呼び、`(状態コード, 本文(JSON なら dict), 応答ヘッダ)` を返す。"""
    hdrs: list[tuple[bytes, bytes]] = [(b"host", HOST.encode())]
    body = b""
    if json_body is not None:
        body = json.dumps(json_body, ensure_ascii=False).encode("utf-8")
        hdrs.append((b"content-type", b"application/json"))
    if cookie_header is not None:
        hdrs.append((b"cookie", cookie_header.encode()))
    elif cookie:
        hdrs.append((b"cookie", f"{auth.SESSION_COOKIE}={cookie}".encode()))
    if origin:
        hdrs.append((b"origin", origin.encode()))
    messages = asyncio.run(_run_asgi(app or web_main.app, method, path, query, body, hdrs))
    return _respond(messages)


def _cookie_of(headers: dict) -> str | None:
    for value in headers.get("set-cookie", []):
        if value.startswith(f"{auth.SESSION_COOKIE}="):
            return value.split(";", 1)[0].split("=", 1)[1]
    return None


def add_user(name: str, email: str, password: str) -> None:
    auth.create_user_dirs(name)
    store.create_user(name, email, auth.hash_password(password), is_admin=False, must_change_password=False)


def make_project(owner: str, name: str) -> None:
    (registry.user_dir(owner) / "projects" / name / ".sai").mkdir(parents=True, exist_ok=True)


def _set_settings(**changes) -> None:
    web_main.settings = dataclasses.replace(web_main.settings, **changes)


def main() -> None:
    store.init_db()
    add_user("alice", "alice@uni.edu", "alicepass1")
    add_user("bob", "bob@uni.edu", "bobpass123")
    make_project("alice", "alice-west")
    make_project("bob", "bob-east")
    _set_settings(ts_url=TS_URL)

    status, _, headers = call("POST", "/api/auth/login", json_body={"email": "alice@uni.edu", "password": "alicepass1"})
    assert status == 200, status
    cookie = _cookie_of(headers)
    assert cookie, "Cookie が付かない"

    # --- 枠に入れさせない（sai-web の画面と API の両方） ---
    for path in ("/", "/api/auth/me"):
        status, _, headers = call("GET", path, cookie=cookie)
        assert status == 200, (path, status)
        assert headers.get("x-frame-options") == ["DENY"], (path, headers)
        assert headers.get("content-security-policy") == ["frame-ancestors 'none'"], (path, headers)

    # --- CORS の許可が無い（ts のオリジンから応答を読ませない） ---
    _, _, headers = call("OPTIONS", "/api/auth/me", origin=ORIGIN_SAME_HOST)
    assert "access-control-allow-origin" not in headers, headers
    status, _, headers = call("GET", "/api/auth/me", cookie=cookie, origin=ORIGIN_SAME_HOST)
    assert status == 200, status
    assert "access-control-allow-origin" not in headers, headers
    # Cookie で入った変更系は、別のポートの Origin から 403（同じホスト名でも Host と違えば拒む）
    status, body, _ = call("POST", "/api/projects/alice-west/container/start", cookie=cookie, origin=ORIGIN_SAME_HOST)
    assert status == 403, (status, body)

    # --- Cookie が重なったら 400（path を細かくした偽物を読ませない） ---
    status, body, headers = call(
        "GET", "/api/auth/me", cookie_header=f"{auth.SESSION_COOKIE}={cookie}; {auth.SESSION_COOKIE}=evil"
    )
    assert status == 400, (status, body)
    assert "ログインの Cookie が重なっています" in body["detail"], body
    cleared = " ".join(headers.get("set-cookie", []))
    assert f"{auth.SESSION_COOKIE}=" in cleared and "Max-Age=0" in cleared and "Path=/" in cleared, headers
    # 1 つだけなら今までどおり
    assert call("GET", "/api/auth/me", cookie=cookie)[0] == 200

    # --- 券を出す: 持ち主で 200・URL は ts 入口の住所 ---
    started: list[str] = []
    real_ensure = containers.ts_ensure
    containers.ts_ensure = lambda name: (started.append(name), {"base": "http://127.0.0.1:1", "token": "x"})[1]
    try:
        status, body, _ = call("POST", "/api/projects/alice-west/ts/open", cookie=cookie)
        assert status == 200, (status, body)
        assert started == ["alice-west"], started
        prefix = f"{TS_URL}/p/alice-west/__gate/enter?ticket="
        assert body["url"].startswith(prefix), body
        ticket = body["url"][len(prefix):]

        # 券で ts 入口に入ると、合言葉つきの 303 になる（U04 の住所を直接呼ぶ）
        status, _, headers = call("GET", "/p/alice-west/__gate/enter", app=ts_gate.app, query=f"ticket={ticket}")
        assert status == 303, (status, headers)
        location = headers["location"][0]
        assert location.startswith("/p/alice-west/#token="), location
        # 券は 1 回だけ
        assert call("GET", "/p/alice-west/__gate/enter", app=ts_gate.app, query=f"ticket={ticket}")[0] == 403

        # 別の利用者のプロジェクトは 404（持ち主でない人に券を出さない）
        assert call("POST", "/api/projects/bob-east/ts/open", cookie=cookie)[0] == 404

        # 起動の失敗はそのまま伝わる（券は出さない）
        def _fail(name: str) -> dict:
            raise HTTPException(status_code=409, detail="コンテナが動いていません（Projects から起動してください）")

        containers.ts_ensure = _fail
        status, body, _ = call("POST", "/api/projects/alice-west/ts/open", cookie=cookie)
        assert status == 409 and "コンテナが動いていません" in body["detail"], (status, body)
    finally:
        containers.ts_ensure = real_ensure

    # --- 本文の web は形を確かめてから券の URL に付ける（G3-C3・入口で断片に入る） ---
    web = "https://sai.example:8443"
    containers.ts_ensure = lambda name: {"base": "http://127.0.0.1:1", "token": "x"}
    try:
        status, body, _ = call("POST", "/api/projects/alice-west/ts/open", cookie=cookie, json_body={"web": web})
        assert status == 200, (status, body)
        assert body["url"].endswith(f"&web={quote(web, safe='')}"), body
        ticket = body["url"].split("ticket=", 1)[1].split("&", 1)[0]
        status, _, headers = call(
            "GET", "/p/alice-west/__gate/enter", app=ts_gate.app, query=f"ticket={ticket}&web={quote(web, safe='')}"
        )
        assert status == 303, (status, headers)
        assert headers["location"][0].endswith(f"&web={quote(web, safe='')}&project=alice-west"), headers
        # 形の違う web は 400（券は出さない）
        for bad in ("javascript:alert(1)", "https://a/b", "https://a?x", ""):
            status, body, _ = call("POST", "/api/projects/alice-west/ts/open", cookie=cookie, json_body={"web": bad})
            assert status == 400 and "web の形が正しくありません" in body["detail"], (bad, status, body)
        # 本文が無ければ今までどおり（web の無い URL）
        status, body, _ = call("POST", "/api/projects/alice-west/ts/open", cookie=cookie)
        assert status == 200 and "&web=" not in body["url"], body
    finally:
        containers.ts_ensure = real_ensure

    # --- 入口の住所が空なら 503（起動はしない） ---
    _set_settings(ts_url="")
    try:
        status, body, _ = call("POST", "/api/projects/alice-west/ts/open", cookie=cookie)
        assert status == 503 and body["detail"] == "時系列の入口が設定されていません", (status, body)
    finally:
        _set_settings(ts_url=TS_URL)

    # --- 単独モードでは 404（多人数モードの住所） ---
    users_root = os.environ.pop("SAI_WEB_USERS_ROOT")
    try:
        status, body, _ = call("POST", "/api/projects/alice-west/ts/open")
        assert status == 404, (status, body)
    finally:
        os.environ["SAI_WEB_USERS_ROOT"] = users_root

    # --- `SAI_WEB_TS_PORT` は多人数モードでだけ（単独モードは終了コード 2） ---
    env = {k: v for k, v in os.environ.items() if k != "SAI_WEB_USERS_ROOT"}
    env.update({"SAI_WEB_TS_PORT": "8210", "SAI_HOME": str(_TMP / "sai-home-solo")})
    proc = subprocess.run(
        [str(APP_SRC.parent / "bin" / "sai-web"), "serve", "--port", "8297"],
        env=env, capture_output=True, text=True, timeout=60,
    )
    assert proc.returncode == 2, (proc.returncode, proc.stderr)
    assert "SAI_WEB_TS_PORT は多人数モード（SAI_WEB_USERS_ROOT）でだけ使えます" in proc.stderr, proc.stderr

    # --- 番号ごとに正しい app へ渡す（lifespan は sai-web） ---
    seen: list[str] = []

    async def web_app(scope, receive, send):
        seen.append("web")

    async def ts_app(scope, receive, send):
        seen.append("ts")

    route = webcli._route_by_port(web_app, ts_app, 8210)

    async def drive(scope_type: str, server_port: int) -> None:
        await route({"type": scope_type, "server": ("127.0.0.1", server_port)}, None, None)

    asyncio.run(drive("http", 8210))
    asyncio.run(drive("http", 8200))
    asyncio.run(drive("lifespan", 8210))
    assert seen == ["ts", "web", "web"], seen

    # --- 時系列の住所は access log に出さない ---
    filt = webcli._DropTsAccess()
    rec = logging.LogRecord("uvicorn.access", logging.INFO, "", 0, "", (), None)
    rec.args = ("127.0.0.1:1", "GET", "/p/alice-west/api/x?token=abc", "1.1", 200)
    assert filt.filter(rec) is False
    rec.args = ("127.0.0.1:1", "GET", "/api/auth/me", "1.1", 200)
    assert filt.filter(rec) is True

    print("test_ts_open: すべて通りました")


if __name__ == "__main__":
    main()
