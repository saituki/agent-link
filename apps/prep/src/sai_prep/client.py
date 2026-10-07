"""サーバへの要求。標準ライブラリの `urllib.request` だけを使う。

使う住所は共有契約 C14（U13 が作る）。個人のトークンを
`Authorization: Bearer sai_…` で送る。
"""

from __future__ import annotations

import json
import urllib.error
import urllib.parse
import urllib.request

from .config import allowed_server_url, normalize_server_url

# 1 回の要求の上限（秒）。束の送信は大きいので長めにする。
DEFAULT_TIMEOUT = 60
BUNDLE_TIMEOUT = 600


class ClientError(Exception):
    """サーバへの要求が失敗した。"""

    def __init__(self, message: str, status: int | None = None):
        super().__init__(message)
        self.status = status


class Client:
    """sai-tree の Web サーバへの要求。"""

    def __init__(self, server_url: str, token: str, timeout: int = DEFAULT_TIMEOUT):
        self.server_url = normalize_server_url(server_url)
        self.token = token.strip()
        self.timeout = timeout

    # -- 内部 --------------------------------------------------------------

    def _url(self, path: str) -> str:
        return f"{self.server_url}{path}"

    def _request(
        self,
        method: str,
        path: str,
        *,
        data: bytes | None = None,
        headers: dict[str, str] | None = None,
        timeout: int | None = None,
    ) -> tuple[int, bytes]:
        if not allowed_server_url(self.server_url):
            raise ClientError(
                "サーバの住所は https:// か http://127.0.0.1 / http://localhost だけを使えます"
            )
        request_headers = {"Accept": "application/json"}
        if self.token:
            request_headers["Authorization"] = f"Bearer {self.token}"
        if headers:
            request_headers.update(headers)
        request = urllib.request.Request(
            self._url(path), data=data, headers=request_headers, method=method
        )
        try:
            with urllib.request.urlopen(request, timeout=timeout or self.timeout) as response:
                return response.status, response.read()
        except urllib.error.HTTPError as error:
            body = error.read()
            raise ClientError(_error_message(error.code, body), status=error.code) from error
        except urllib.error.URLError as error:
            raise ClientError(f"サーバにつながりません: {error.reason}") from error

    # -- 住所 --------------------------------------------------------------

    def me(self) -> dict:
        """`GET /api/auth/me`。トークンの確かめに使う。"""
        _, body = self._request("GET", "/api/auth/me")
        return _json(body)

    def projects(self) -> list[dict]:
        """`GET /api/projects`。名前の一覧に使う。"""
        _, body = self._request("GET", "/api/projects")
        data = _json(body)
        if isinstance(data, dict):
            # 形が `{"projects": [...]}` でも `[...]` でも受けられるようにする。
            return data.get("projects", [])
        return data

    def pending(self, project: str) -> dict:
        """`GET /api/projects/{name}/papers/pending`。PDF 待ちの一覧。"""
        name = urllib.parse.quote(project, safe="")
        _, body = self._request("GET", f"/api/projects/{name}/papers/pending")
        return _json(body)

    def post_bundle(self, project: str, blob: bytes) -> dict:
        """`POST /api/projects/{name}/papers/bundle`。束（zip）を送る。"""
        name = urllib.parse.quote(project, safe="")
        _, body = self._request(
            "POST",
            f"/api/projects/{name}/papers/bundle",
            data=blob,
            headers={"Content-Type": "application/zip"},
            timeout=BUNDLE_TIMEOUT,
        )
        return _json(body)

    def job(self, project: str, key: str) -> dict:
        """`GET /api/projects/{name}/papers/jobs/{key}`。取り込みの進み具合。"""
        name = urllib.parse.quote(project, safe="")
        job_key = urllib.parse.quote(key, safe="")
        _, body = self._request("GET", f"/api/projects/{name}/papers/jobs/{job_key}")
        return _json(body)


def _json(body: bytes) -> dict | list:
    if not body:
        return {}
    try:
        return json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ClientError("サーバの返事を読めません") from error


def _error_message(status: int, body: bytes) -> str:
    text = ""
    try:
        data = json.loads(body.decode("utf-8"))
        if isinstance(data, dict):
            text = str(data.get("detail") or data.get("error") or "")
    except (UnicodeDecodeError, json.JSONDecodeError):
        text = body.decode("utf-8", "replace").strip()
    if text:
        return f"サーバがエラーを返しました（{status}）: {text}"
    return f"サーバがエラーを返しました（{status}）"
