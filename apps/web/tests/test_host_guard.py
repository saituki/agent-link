"""入口の守り（Host と Origin）と push 購読の検査のテスト。

確かめること:
- `Host` が許可外（`evil.example`）なら 403
- `Host` が `127.0.0.1:8400` と `*.ts.net` の名前なら通る
- 書き込み（POST）に `Origin` が許可外なら 403、`Origin` が無ければ通る（403 以外）
- push 購読の `endpoint` が `https://` の各社でないもの（`http://127.0.0.1/x`）は 400

実行:
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_host_guard.py
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
import tempfile
from pathlib import Path

# import より前に一時フォルダを決める（config.DB_PATH は import 時に決まるため）
_TMP = Path(tempfile.mkdtemp(prefix="agent-link-web-host-guard-"))
os.environ["AGENT_LINK_HOME"] = str(_TMP / "sai-home")
os.environ["AGENT_LINK_WEB_NO_WATCH"] = "1"

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from agent_link_web import store  # noqa: E402
from agent_link_web.main import app  # noqa: E402

store.init_db()

LOCAL = ("127.0.0.1", 40000)


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
        "client": LOCAL,
        "server": ("127.0.0.1", 8400),
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


def call(
    method: str,
    path: str,
    *,
    host: str = "127.0.0.1:8400",
    origin: str | None = None,
    json_body: dict | None = None,
) -> tuple[int, object]:
    """1 つの住所を呼び、`(状態コード, 本文)` を返す。"""
    hdrs: list[tuple[bytes, bytes]] = [(b"host", host.encode())]
    body = b""
    if json_body is not None:
        body = json.dumps(json_body, ensure_ascii=False).encode("utf-8")
        hdrs.append((b"content-type", b"application/json"))
    if origin is not None:
        hdrs.append((b"origin", origin.encode()))
    messages = asyncio.run(_run_asgi(method, path, body, hdrs))
    start = next(m for m in messages if m["type"] == "http.response.start")
    raw = b"".join(m.get("body", b"") for m in messages if m["type"] == "http.response.body")
    try:
        parsed = json.loads(raw.decode("utf-8")) if raw else {}
    except (json.JSONDecodeError, UnicodeDecodeError):
        parsed = raw
    return start["status"], parsed


def test_foreign_host_refused() -> None:
    status, body = call("GET", "/api/health", host="evil.example")
    assert status == 403, (status, body)
    assert body == {"detail": "Host not allowed."}, body


def test_local_and_tailnet_hosts_pass() -> None:
    for host in ("127.0.0.1:8400", "localhost:8400", "[::1]:8400", "mac.tail1234.ts.net"):
        status, body = call("GET", "/api/health", host=host)
        assert status == 200, (host, status, body)


def test_cross_origin_post_refused() -> None:
    status, body = call(
        "POST", "/api/questions/x/answer", origin="http://evil.example", json_body={}
    )
    assert status == 403, (status, body)
    assert body == {"detail": "Cross-origin request refused."}, body


def test_null_origin_post_refused() -> None:
    status, _ = call("POST", "/api/questions/x/answer", origin="null", json_body={})
    assert status == 403, status


def test_post_without_origin_passes() -> None:
    # フック・agent-link-web api・curl は Origin を付けない
    status, _ = call("POST", "/api/questions/x/answer", json_body={})
    assert status != 403, status


def test_push_endpoint_refused() -> None:
    status, body = call(
        "POST",
        "/api/push/subscribe",
        json_body={"endpoint": "http://127.0.0.1/x", "keys": {"p256dh": "a", "auth": "b"}},
    )
    assert status == 400, (status, body)
    assert body == {"detail": "Invalid push subscription."}, body


if __name__ == "__main__":
    test_foreign_host_refused()
    test_local_and_tailnet_hosts_pass()
    test_cross_origin_post_refused()
    test_null_origin_post_refused()
    test_post_without_origin_passes()
    test_push_endpoint_refused()
    print("test_host_guard: OK")
