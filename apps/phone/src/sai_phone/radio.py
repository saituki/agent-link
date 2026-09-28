"""sai-radio への中継（計画 09 §3）。

sai-phone は他アプリの DB を直接開かない（00 §2）ため，Radio の画面は
この中継経由だけで sai-radio（`SAI_RADIO_URL`・既定 127.0.0.1:8000）と通信する。

- 音声の登録・再生成（`papers/upload`・`papers/from-extract`）は中継しない。
  音声化は計画 10 の `sai-moon-paper phone radio <id>` 経由だけにする（09 §3.2）。
- 通信手段は標準ライブラリの `urllib.request` のみ（00 §2 の依存制約）。
"""

from __future__ import annotations

import re
import urllib.error
import urllib.parse
import urllib.request

from fastapi import HTTPException
from fastapi.responses import Response, StreamingResponse

from sai_phone.config import settings

# 音声作りは sai-phone から使わせない（09 §3.2）
BLOCKED = ("papers/upload", "papers/from-extract")
# sai-radio から受け取ってそのまま返すヘッダ（09 §3.2）
PASS_HEADERS = ("content-type", "content-length", "content-range", "accept-ranges")
# 送る先 path の形（英数字と記号だけ・09 §3.2）
PATH_RE = re.compile(r"^[A-Za-z0-9_./-]+$")
# 一度に読み出す大きさ（64KB・09 §3.2）
CHUNK_SIZE = 64 * 1024
# 時間切れ: 質問（…/ask）は LLM が答えるので長く待つ（09 §3.2）
ASK_TIMEOUT = 180.0
DEFAULT_TIMEOUT = 30.0


def build_target(path: str, query: list[tuple[str, str]]) -> str:
    """送り先の URL を組み立てる（path の検査もここで行う・09 §5）。

    - path は英数字と `_.-/` だけで，`..` を含まない（違えば 400）
    - 音声作り（`BLOCKED`）は 403
    - 合言葉 `token` は sai-phone 自分の物なので sai-radio へは送らない
      （sai-radio 側の合言葉は `Authorization: Bearer` で付ける・09 §3.2）
    """
    if not PATH_RE.match(path) or ".." in path:
        raise HTTPException(status_code=400, detail="送り先のパスが正しくありません")
    for blocked in BLOCKED:
        if path == blocked or path.startswith(f"{blocked}/"):
            raise HTTPException(
                status_code=403, detail="この操作は sai-phone からは使えません"
            )
    pairs = [(k, v) for k, v in query if k != "token"]
    url = f"{settings.radio_url}/api/{path}"
    qs = urllib.parse.urlencode(pairs)
    return f"{url}?{qs}" if qs else url


def _pass_headers(headers) -> dict[str, str]:
    """渡す決まりのヘッダだけ抜き出す（名前は小文字で照合・09 §3.2）。"""
    picked: dict[str, str] = {}
    for name in PASS_HEADERS:
        value = headers.get(name) if headers is not None else None
        if value is not None:
            picked[name] = value
    return picked


def _chunks(resp):
    """64KB ずつ読み出して渡すジェネレータ（最後に必ず close・09 §3.2）。"""
    try:
        while True:
            data = resp.read(CHUNK_SIZE)
            if not data:
                break
            yield data
    finally:
        resp.close()


def forward(
    method: str,
    path: str,
    query: list[tuple[str, str]],
    body: bytes,
    content_type: str | None,
    range_header: str | None,
) -> Response:
    """sai-radio への中継（本体・09 §3.2）。

    成功は 64KB ずつ流して返す。sai-radio からのエラー（HTTPError）は
    同じ状態コードでそのまま返し，繋がらないときだけ 503 にする。
    """
    url = build_target(path, query)
    headers: dict[str, str] = {}
    if content_type:
        headers["Content-Type"] = content_type
    if range_header:
        headers["Range"] = range_header  # 音声の部分取り（Safari の再生に必要）
    if settings.radio_token:
        headers["Authorization"] = f"Bearer {settings.radio_token}"
    timeout = ASK_TIMEOUT if path.endswith("/ask") else DEFAULT_TIMEOUT
    req = urllib.request.Request(
        url,
        # GET に body は付けない（method は必ずそのまま伝える）
        data=body if method == "POST" and body else None,
        headers=headers,
        method=method,
    )
    try:
        resp = urllib.request.urlopen(req, timeout=timeout)
    except urllib.error.HTTPError as e:
        # sai-radio の {"detail": …} が iPhone に届くよう同じ状態コードで返す（09 §3.2）
        return Response(content=e.read(), status_code=e.code, headers=_pass_headers(e.headers))
    except (urllib.error.URLError, OSError) as e:
        # 接続できない・応答しない（常駐が止まっている）（09 §3.2）
        raise HTTPException(
            status_code=503, detail="Mac の sai-radio が応答しません"
        ) from e
    return StreamingResponse(
        _chunks(resp),
        status_code=getattr(resp, "status", 200) or 200,
        headers=_pass_headers(resp.headers),
    )
