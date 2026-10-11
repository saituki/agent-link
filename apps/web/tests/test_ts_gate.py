"""ts 入口（実装計画 U04・共有契約 C5）のテスト。

確かめること（AC1〜AC6）:
- 入場券は 1 回だけ・60 秒・プロジェクトが合うときだけ使え、合言葉を含む 303 になる
- 合言葉が無い・違う・別のプロジェクト・無効にした利用者は 401
- 上流へは Cookie・元の Authorization・token を送らず、内部の合言葉だけを送る
- 応答に Set-Cookie を写さず、利用者向けの安全な見出しを足す
- SSE（text/event-stream）は届いたぶんから流れる
- 上流が 401 を返したら覚えを捨てて 1 回だけ起こし直す
- 画面ファイルは静的に返し、<static> の外と存在しないファイルは 404

上流（sai-ts）は別スレッドの HTTP/1.0 の偽サーバで代える。ts_ensure / ts_forget は試験で差し替える。
`fastapi.testclient` は httpx を要するので使わない（test_auth.py と同じく ASGI を直接叩く）。

実行:
    cd ~/workdir/Sai/sai-tree
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_ts_gate.py
"""

from __future__ import annotations

import asyncio
import http.server
import json
import os
import re
import socket
import sys
import tempfile
import threading
from pathlib import Path
from urllib.parse import quote

# import より前に一時フォルダを決める（config.DB_PATH は import 時に決まるため）
_TMP = Path(tempfile.mkdtemp(prefix="sai-web-ts-gate-"))
os.environ["SAI_HOME"] = str(_TMP / "sai-home")
os.environ["SAI_WEB_USERS_ROOT"] = str(_TMP / "users")
os.environ.pop("SAI_WEB_TOKEN", None)

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_web import auth, containers, store, ts_gate  # noqa: E402

PROJECT = "saituki-west"
OTHER = "saituki-east"
HOST = "testserver"
TOKEN_RE = re.compile(r"/p/saituki-west/#token=([A-Za-z0-9_-]{43})(&.*)?")


# --- 偽の sai-ts（HTTP/1.0・別スレッド） ---

SEEN: list[dict] = []  # 偽の上流が受けた要求
UNAUTH_LEFT = [0]  # 残りの 401 の数
ENSURE = {"calls": 0, "forgets": 0}
LAST_UP: dict[str, str] = {}  # 最後に ts_ensure が返した内部の合言葉
BAD_FIRST: list[str] = []  # 最初の ts_ensure だけ返す、つながらない住所


class _Upstream(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"

    def log_message(self, *args):
        pass

    def _reply(self, status: int, ctype: str, body: bytes, extra=()) -> None:
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        for key, value in extra:
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(body)

    def _handle(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        SEEN.append(
            {
                "method": self.command,
                "path": self.path,
                "headers": {k.lower(): v for k, v in self.headers.items()},
            }
        )
        if self.path.startswith("/api/stream"):
            # SSE は 2 件に分けて送る（間に少し置く）
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            for event in (b"data: one\n\n", b"data: two\n\n"):
                self.wfile.write(event)
                self.wfile.flush()
            return
        if UNAUTH_LEFT[0] > 0:
            UNAUTH_LEFT[0] -= 1
            self._reply(401, "application/json", b'{"detail":"no"}')
            return
        # Set-Cookie を返す。入口はこれを写さないこと
        out = json.dumps({"path": self.path, "body": body.decode("utf-8")}).encode("utf-8")
        self._reply(200, "application/json", out, [("Set-Cookie", "upstream=1")])

    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = _handle


_SERVER = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Upstream)
threading.Thread(target=_SERVER.serve_forever, daemon=True).start()
UP_PORT = _SERVER.server_address[1]
UP_BASE = f"http://127.0.0.1:{UP_PORT}"


def _fake_ensure(name: str) -> dict:
    ENSURE["calls"] += 1
    base = BAD_FIRST.pop() if BAD_FIRST else UP_BASE
    LAST_UP["token"] = f"internal-{ENSURE['calls']}"
    return {"base": base, "token": LAST_UP["token"]}


def _fake_forget(name: str) -> None:
    ENSURE["forgets"] += 1


containers.ts_ensure = _fake_ensure
containers.ts_forget = _fake_forget


# --- ASGI を直接叩く呼び出し係（httpx を使わない） ---


async def _run_asgi(method: str, path: str, query: str, body: bytes, headers: list[tuple[bytes, bytes]]):
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
            # 本物のサーバと同じく、切れるまで待つ（SSE を途中で切らない）
            await asyncio.get_running_loop().create_future()
        state["sent"] = True
        return {"type": "http.request", "body": body, "more_body": False}

    async def send(message):
        messages.append(message)

    await ts_gate.app(scope, receive, send)
    return messages


def call(
    method: str,
    path: str,
    *,
    query: str = "",
    headers: dict | None = None,
    cookie: str | None = None,
    body: bytes = b"",
) -> tuple[int, bytes, dict[str, str]]:
    """1 つの住所を呼び、`(状態コード, 本文の生バイト, 応答ヘッダ（小文字の名前）)` を返す。"""
    hdrs: list[tuple[bytes, bytes]] = [(b"host", HOST.encode())]
    if cookie:
        hdrs.append((b"cookie", f"{auth.SESSION_COOKIE}={cookie}".encode()))
    for key, value in (headers or {}).items():
        hdrs.append((key.lower().encode(), value.encode()))
    messages = asyncio.run(_run_asgi(method, path, query, body, hdrs))
    start = next(m for m in messages if m["type"] == "http.response.start")
    out = {k.decode(): v.decode() for k, v in start["headers"]}
    raw = b"".join(m.get("body", b"") for m in messages if m["type"] == "http.response.body")
    return start["status"], raw, out


def detail_of(raw: bytes) -> str:
    return json.loads(raw.decode("utf-8"))["detail"]


def enter(project: str, ticket: str):
    return call("GET", f"/p/{project}/__gate/enter", query=f"ticket={ticket}")


def enter_web(project: str, ticket: str, web: str):
    return call("GET", f"/p/{project}/__gate/enter", query=f"ticket={ticket}&web={quote(web, safe='')}")


def token_for(user: str) -> str:
    """入場券を出して enter し、合言葉を受け取る。"""
    status, _, headers = enter(PROJECT, ts_gate.issue_ticket(user, PROJECT))
    assert status == 303, status
    match = TOKEN_RE.fullmatch(headers["location"])
    assert match, headers["location"]
    return match.group(1)


def add_user(name: str, email: str) -> None:
    store.create_user(name, email, auth.hash_password("pw-" + name), is_admin=False, must_change_password=False)


def main() -> None:
    store.init_db()
    add_user("saituki", "saituki@uni.edu")
    add_user("bob", "bob@uni.edu")

    # --- AC1: 入場券 → enter ---
    clock = [1000.0]
    ts_gate._now = lambda: clock[0]

    ticket = ts_gate.issue_ticket("saituki", PROJECT)
    status, raw, headers = enter(PROJECT, ticket)
    assert status == 303, (status, raw)
    assert TOKEN_RE.fullmatch(headers["location"]), headers["location"]
    assert "no-store" in headers["cache-control"], headers
    assert enter(PROJECT, ticket)[0] == 403, "同じ券の 2 回目が通った"

    ticket = ts_gate.issue_ticket("saituki", PROJECT)
    clock[0] += 61
    status, raw, _ = enter(PROJECT, ticket)
    assert status == 403, "61 秒後の券が通った"
    assert raw.decode("utf-8") == "入口の券が無効です。sai-web から開き直してください", raw

    ticket = ts_gate.issue_ticket("saituki", PROJECT)
    assert enter(OTHER, ticket)[0] == 403, "別のプロジェクトの enter が通った"

    bob_token = token_for("bob")  # 無効にする前に一度入っておく
    ticket = ts_gate.issue_ticket("bob", PROJECT)
    store.update_user("bob", disabled=True)
    assert enter(PROJECT, ticket)[0] == 403, "無効な利用者の券が通った"

    # --- AC2: 合言葉の確認 ---
    token = token_for("saituki")
    SEEN.clear()
    datasets = f"/p/{PROJECT}/api/datasets"

    status, raw, _ = call("GET", datasets)
    assert status == 401 and detail_of(raw) == ts_gate._EXPIRED, (status, raw)
    assert call("GET", datasets, headers={"Authorization": "Bearer nope"})[0] == 401
    assert call("GET", datasets, query="token=nope")[0] == 401
    assert call("GET", f"/p/{OTHER}/api/datasets", headers={"Authorization": f"Bearer {token}"})[0] == 401
    assert call("GET", datasets, headers={"Authorization": f"Bearer {bob_token}"})[0] == 401, "無効な利用者の合言葉が通った"
    assert SEEN == [], "401 のあとに上流へ要求が行った"

    status, raw, _ = call("GET", datasets, headers={"Authorization": f"Bearer {token}"})
    assert status == 200, (status, raw)
    assert json.loads(raw)["path"] == "/api/datasets", raw
    status, _, _ = call("GET", datasets, query=f"token={token}")
    assert status == 200, "?token= で通らない"

    # --- AC3: 上流へ送る要求と、利用者へ返す応答 ---
    SEEN.clear()
    status, raw, headers = call(
        "GET",
        datasets,
        query=f"token={token}&limit=2",
        cookie="XYZ",
        headers={"Authorization": f"Bearer {token}", "X-Extra": "1"},
    )
    assert status == 200, (status, raw)
    sent = SEEN[-1]
    assert "cookie" not in sent["headers"], sent["headers"]
    assert "x-extra" not in sent["headers"], sent["headers"]
    assert sent["headers"]["authorization"] == f"Bearer {LAST_UP['token']}", sent["headers"]
    assert sent["headers"]["host"] == f"127.0.0.1:{UP_PORT}", sent["headers"]
    assert sent["path"] == "/api/datasets?limit=2", sent["path"]
    assert token not in sent["path"], sent["path"]
    assert "set-cookie" not in headers, headers
    assert headers["cache-control"] == "no-store", headers
    assert headers["x-frame-options"] == "DENY", headers

    SEEN.clear()
    status, _, _ = call("GET", f"/p/{PROJECT}/ext/import", query=f"token={token}")
    assert status == 200 and SEEN[-1]["path"] == "/ext/import", SEEN

    # --- AC4: SSE は届いたぶんから流れる ---
    status, raw, headers = call("GET", f"/p/{PROJECT}/api/stream", query=f"token={token}")
    assert status == 200 and headers["content-type"].startswith("text/event-stream"), headers
    assert raw.index(b"data: one") < raw.index(b"data: two"), raw

    # --- AC5: 上流の 401 で覚えを捨てて起こし直す ---
    before = dict(ENSURE)
    UNAUTH_LEFT[0] = 1
    status, raw, _ = call("GET", datasets, query=f"token={token}")
    assert status == 200, (status, raw)
    assert ENSURE["forgets"] == before["forgets"] + 1, ENSURE
    assert ENSURE["calls"] == before["calls"] + 2, ENSURE

    # 2 回続けて 401 なら 502（利用者の合言葉は正しい）
    UNAUTH_LEFT[0] = 2
    status, raw, _ = call("GET", datasets, query=f"token={token}")
    assert status == 502 and detail_of(raw) == "時系列のサーバにつながりません", (status, raw)

    # つながらない住所から始まっても、起こし直して 200
    BAD_FIRST.append(f"http://127.0.0.1:{_closed_port()}")
    status, raw, _ = call("GET", datasets, query=f"token={token}")
    assert status == 200, (status, raw)

    # --- AC6: 画面ファイル ---
    status, raw, _ = call("GET", f"/p/{PROJECT}/")
    assert status == 200 and b"<html" in raw.lower(), (status, raw[:80])
    status, _, headers = call("GET", f"/p/{PROJECT}")
    assert status == 307 and headers["location"] == f"/p/{PROJECT}/", headers
    status, _, headers = call("GET", f"/p/{PROJECT}/app.js")
    assert status == 200 and headers["content-type"].startswith("text/javascript"), headers
    assert call("GET", f"/p/{PROJECT}/../../etc/passwd")[0] == 404
    assert call("GET", f"/p/{PROJECT}/nope.js")[0] == 404
    assert call("GET", "/p/Bad_Name/")[0] == 404
    status, raw, _ = call("GET", "/")
    assert status == 404 and raw.decode("utf-8") == "見つかりません", raw

    # 入場券の無い enter は 403
    assert call("GET", f"/p/{PROJECT}/__gate/enter")[0] == 403

    # --- AC7: 断片に sai-web の origin（web）と project を入れる（G3-C3） ---
    web_ok = "https://sai.example:8443"
    status, _, headers = enter_web(PROJECT, ts_gate.issue_ticket("saituki", PROJECT), web_ok)
    assert status == 303, (status, headers)
    assert headers["location"].endswith(f"&web={quote(web_ok, safe='')}&project={PROJECT}"), headers["location"]
    assert TOKEN_RE.fullmatch(headers["location"]), headers["location"]
    # 形の違う web は捨てる（合言葉と project だけの断片になる）
    for web in ("javascript:alert(1)", "https://a/b", "https://a?x", "https://a#x", "", "https://a b", "https://"):
        status, _, headers = enter_web(PROJECT, ts_gate.issue_ticket("saituki", PROJECT), web)
        assert status == 303, (web, status)
        assert "&web=" not in headers["location"], (web, headers["location"])
        assert headers["location"].endswith(f"&project={PROJECT}"), (web, headers["location"])
    # 形の検査の関数そのもの
    assert ts_gate.valid_web_origin("http://127.0.0.1:8200")
    assert ts_gate.valid_web_origin("https://sai.example/")
    assert not ts_gate.valid_web_origin(None)

    print("test_ts_gate: すべて通りました")


def _closed_port() -> int:
    """つながらない番号を作る（閉じた待ち受けの番号）。"""
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


if __name__ == "__main__":
    main()
