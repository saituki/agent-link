"""ts 入口（実装計画 U04・共有契約 C5）。sai-web と別のオリジンで sai-ts を出す。

- 入場券（60 秒・1 回だけ）を合言葉に取り替える（`/p/<名前>/__gate/enter`）。
- 合言葉を確かめ、コンテナの中の sai-ts へ `/api/…`・`/ext/…` を中継する（asyncio で話す）。
- 画面ファイルはホストの `apps/ts/src/sai_ts/static` から返す（認証なし）。

入場券と合言葉はメモリに SHA-256 で置く。sai-web の Cookie・ミドルウェアは使わない。
この app は uvicorn の access log を出さない前提（U05 が `access_log=False` で待ち受ける）。
合言葉・券・Cookie は logger にも書かない。
"""

from __future__ import annotations

import asyncio
import hashlib
import logging
import mimetypes
import re
import secrets
import threading
import time
from pathlib import Path
from urllib.parse import quote, urlencode, urlsplit

from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import FileResponse, PlainTextResponse, RedirectResponse, StreamingResponse

from sai_web import containers, store

logger = logging.getLogger(__name__)

app = FastAPI(docs_url=None, redoc_url=None, openapi_url=None)

_PROJECT_RE = re.compile(r"[a-z][a-z0-9]{1,15}-[a-z0-9][a-z0-9-]{0,29}")
_STATIC = Path(__file__).resolve().parents[3] / "ts" / "src" / "sai_ts" / "static"
_METHODS = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
# 上流へ送る利用者の見出し（これ以外は送らない）
_FORWARD = {"content-type": "Content-Type", "accept": "Accept", "last-event-id": "Last-Event-ID"}
_TICKET_SECONDS = 60.0
_BODY_LIMIT = 10 * 1024 * 1024
_CHUNK = 64 * 1024
_EXPIRED = "時系列の合言葉が無効です。sai-web から開き直してください"
_SECURE_HEADERS = {
    "Cache-Control": "no-store",
    "Referrer-Policy": "no-referrer",
    "X-Frame-Options": "DENY",
    "X-Content-Type-Options": "nosniff",
}

# 入場券と合言葉。どちらも `threading.Lock` で守る（同期の住所からも呼ばれるため）
_LOCK = threading.Lock()
_TICKETS: dict[str, dict] = {}  # sha256(券) -> {"user", "project", "exp"}
_TOKENS: dict[str, dict] = {}  # sha256(合言葉) -> {"user", "project"}


def _now() -> float:
    return time.monotonic()


def _digest(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _sweep(now: float) -> None:
    """期限の切れた券を消す（呼ぶ側が `_LOCK` を持つこと）。"""
    for key in [k for k, v in _TICKETS.items() if v["exp"] <= now]:
        del _TICKETS[key]


def issue_ticket(user: str, project: str) -> str:
    """入場券を作る。60 秒で切れ、1 回だけ使える。sai-web の POST から呼ぶ。"""
    ticket = secrets.token_urlsafe(32)
    with _LOCK:
        now = _now()
        _sweep(now)
        _TICKETS[_digest(ticket)] = {"user": user, "project": project, "exp": now + _TICKET_SECONDS}
    return ticket


def _take_ticket(ticket: str, project: str) -> str | None:
    """券を使う。正しければ利用者の名前を返す。使ったら、失敗でも消す。"""
    with _LOCK:
        now = _now()
        _sweep(now)
        entry = _TICKETS.pop(_digest(ticket), None)
    if entry is None or entry["project"] != project:
        return None
    return entry["user"]


def _token_user(token: str, project: str) -> str | None:
    with _LOCK:
        entry = _TOKENS.get(_digest(token))
    if entry is None or entry["project"] != project:
        return None
    return entry["user"]


def _active(user: str) -> bool:
    u = store.get_user(user)
    return u is not None and not u.get("disabled")


def _valid(project: str) -> bool:
    return _PROJECT_RE.fullmatch(project) is not None


def valid_web_origin(web) -> bool:
    """sai-web の origin の形か（`http(s)://host[:port]` だけ・path・query・fragment は無い）。

    `main.py`（`ts/open` の本文）と `enter`（入場券の住所）の両方で使う。通らなければ使わない。
    """
    if not isinstance(web, str) or not web.startswith(("http://", "https://")):
        return False
    if not web.isprintable() or any(c.isspace() for c in web):
        return False
    try:
        parts = urlsplit(web)
    except ValueError:
        return False
    return bool(parts.netloc) and parts.path in ("", "/") and not parts.query and not parts.fragment


def _not_found() -> PlainTextResponse:
    return PlainTextResponse("見つかりません", status_code=404)


@app.get("/p/{project}")
async def project_root(project: str):
    if not _valid(project):
        return _not_found()
    return RedirectResponse(f"/p/{project}/", status_code=307)


@app.get("/p/{project}/__gate/enter")
async def enter(project: str, ticket: str = "", web: str = ""):
    if not _valid(project):
        return _not_found()
    user = _take_ticket(ticket, project)
    if user is None or not _active(user):
        return PlainTextResponse(
            "入口の券が無効です。sai-web から開き直してください",
            status_code=403,
            headers=_SECURE_HEADERS,
        )
    token = secrets.token_urlsafe(32)
    with _LOCK:
        _TOKENS[_digest(token)] = {"user": user, "project": project}
    # 合言葉は断片（#）に置く。断片はサーバに届かず、ログにも残らない。
    # sai-web の origin（「Chats で開く」の行き先）は形が正しいときだけ足す
    frag = f"token={token}"
    if valid_web_origin(web):
        frag += f"&web={quote(web, safe='')}"
    frag += f"&project={project}"
    return RedirectResponse(f"/p/{project}/#{frag}", status_code=303, headers=_SECURE_HEADERS)


def _user_of(request: Request, project: str) -> str:
    """`Authorization: Bearer` を先に見て、無ければ `?token=` の合言葉で利用者を決める。"""
    auth = request.headers.get("authorization", "")
    if auth.startswith("Bearer "):
        got = auth[len("Bearer "):]
    else:
        got = request.query_params.get("token", "")
    user = _token_user(got, project)
    if user is None or not _active(user):
        raise HTTPException(status_code=401, detail=_EXPIRED)
    return user


async def _read_body(request: Request) -> bytes:
    body = bytearray()
    async for chunk in request.stream():
        body += chunk
        if len(body) > _BODY_LIMIT:
            raise HTTPException(status_code=413, detail="本文が大きすぎます（10 MB まで）")
    return bytes(body)


async def _open(up: dict, method: str, target: str, request: Request, body: bytes):
    """上流へ要求を送り、状態行と見出しを読む。本文は読まずに返す。

    送る見出しは決めたものだけ。Cookie・元の Authorization は送らない。
    """
    parts = urlsplit(up["base"])
    reader, writer = await asyncio.open_connection(parts.hostname, parts.port)
    try:
        lines = [
            f"{method} {target} HTTP/1.0",
            f"Host: {parts.hostname}:{parts.port}",
            f"Authorization: Bearer {up['token']}",
            "Connection: close",
            f"Content-Length: {len(body)}",
        ]
        for name, label in _FORWARD.items():
            if name in request.headers:
                lines.append(f"{label}: {request.headers[name]}")
        writer.write(("\r\n".join(lines) + "\r\n\r\n").encode("latin-1") + body)
        await writer.drain()

        status = int((await reader.readline()).split(b" ", 2)[1])
        headers: dict[str, str] = {}
        while True:
            line = await reader.readline()
            if line in (b"\r\n", b"\n", b""):
                break
            name, _, value = line.decode("latin-1").partition(":")
            headers[name.strip().lower()] = value.strip()
    except BaseException:
        writer.close()
        raise
    return status, headers, reader, writer


async def _pump(reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
    """上流の本文を届いたぶんから流す（SSE も同じ道）。終わったら閉じる。"""
    try:
        while True:
            chunk = await reader.read(_CHUNK)
            if not chunk:
                break
            yield chunk
    finally:
        writer.close()


async def _relay(kind: str, project: str, rest: str, request: Request):
    if not _valid(project):
        return _not_found()
    _user_of(request, project)
    body = await _read_body(request)
    query = urlencode([(k, v) for k, v in request.query_params.multi_items() if k != "token"])
    target = f"/{kind}/{quote(rest, safe='/')}" + (f"?{query}" if query else "")

    # 上流につながらない・401 のときは、覚えを捨てて 1 回だけ起こし直す
    for attempt in (1, 2):
        up = await asyncio.to_thread(containers.ts_ensure, project)
        try:
            status, headers, reader, writer = await _open(up, request.method, target, request, body)
        except (OSError, ValueError, IndexError):
            status = None
        if status is not None and status != 401:
            out = dict(_SECURE_HEADERS)
            for name in ("content-type", "content-disposition"):
                if name in headers:
                    out[name] = headers[name]
            return StreamingResponse(_pump(reader, writer), status_code=status, headers=out)
        if status is not None:
            writer.close()
        logger.warning("ts の上流が応えない・合言葉を拒んだため覚えを捨てます: %s (試行 %d)", project, attempt)
        await asyncio.to_thread(containers.ts_forget, project)
    raise HTTPException(status_code=502, detail="時系列のサーバにつながりません")


@app.api_route("/p/{project}/api/{rest:path}", methods=_METHODS)
async def api(project: str, rest: str, request: Request):
    return await _relay("api", project, rest, request)


@app.api_route("/p/{project}/ext/{rest:path}", methods=_METHODS)
async def ext(project: str, rest: str, request: Request):
    return await _relay("ext", project, rest, request)


@app.get("/p/{project}/{path:path}")
async def static_file(project: str, path: str):
    if not _valid(project):
        return _not_found()
    base = _STATIC.resolve()
    try:
        target = (base / (path or "index.html")).resolve()
        if not target.is_relative_to(base) or not target.is_file():
            return _not_found()
    except (OSError, ValueError):
        return _not_found()
    media = mimetypes.guess_type(target.name)[0] or "application/octet-stream"
    if target.suffix == ".js":
        media = "text/javascript"
    if media.startswith("text/"):
        media += "; charset=utf-8"
    return FileResponse(target, media_type=media, headers=_SECURE_HEADERS)


@app.api_route("/{rest:path}", methods=_METHODS, include_in_schema=False)
async def _other(rest: str):
    return _not_found()
