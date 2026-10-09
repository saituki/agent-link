"""FastAPI の本体。既定は 127.0.0.1 だけで待つ。コンテナの中では 0.0.0.0 と合言葉で待つ。"""

from __future__ import annotations

import os
import secrets
import sys
from pathlib import Path

import uvicorn
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, StreamingResponse
from fastapi.staticfiles import StaticFiles

from . import api_chat, api_data, api_events, api_ext, paths, watch

app = FastAPI()
for _r in (api_data.router, api_events.router, api_chat.router, api_ext.router):
    app.include_router(_r)


def _msg(exc: Exception) -> str:
    return str(exc.args[0]) if isinstance(exc, KeyError) and exc.args else str(exc)


@app.exception_handler(ValueError)
async def _bad_request(request: Request, exc: ValueError):
    return JSONResponse({"detail": _msg(exc)}, status_code=400)


@app.exception_handler(KeyError)
async def _not_found(request: Request, exc: KeyError):
    return JSONResponse({"detail": _msg(exc)}, status_code=404)


@app.exception_handler(paths.ProjectNotFound)
async def _no_project(request: Request, exc: paths.ProjectNotFound):
    return JSONResponse({"detail": _msg(exc)}, status_code=500)


@app.middleware("http")
async def _token(request: Request, call_next):
    # 空なら認証なし。環境変数は要求のたびに読む
    want = os.environ.get("SAI_TS_TOKEN", "")
    p = request.url.path
    if want and (p.startswith("/api/") or p.startswith("/ext/")):
        auth = request.headers.get("authorization", "")
        got = auth[len("Bearer "):] if auth.startswith("Bearer ") else request.query_params.get("token", "")
        if not secrets.compare_digest(got.encode("utf-8"), want.encode("utf-8")):
            return JSONResponse({"detail": "合言葉が違います"}, status_code=401)
    return await call_next(request)


@app.get("/api/stream")
async def stream():
    return StreamingResponse(watch.stream(), media_type="text/event-stream")


_static = Path(__file__).parent / "static"
if _static.is_dir():
    app.mount("/", StaticFiles(directory=_static, html=True), name="static")


_LOOPBACK = ("127.0.0.1", "localhost", "::1")


def serve(port=None, host=None) -> int:
    try:
        paths.project_root()
    except paths.ProjectNotFound as e:
        print(e, file=sys.stderr)
        return 2
    host = host or os.environ.get("SAI_TS_HOST") or "127.0.0.1"
    # 外から届く住所では、合言葉が無ければ待たない（同じ docker ネットワークの他の利用者にも届くため）
    if host not in _LOOPBACK and not os.environ.get("SAI_TS_TOKEN"):
        print("127.0.0.1 以外で待つときは SAI_TS_TOKEN が要ります", file=sys.stderr)
        return 2
    uvicorn.run(app, host=host, port=port or int(os.environ.get("SAI_TS_PORT", "8210")))
    return 0
