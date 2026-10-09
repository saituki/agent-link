"""データの受け口（`data/raw/`）のテスト（assert だけ・実装計画 U01 §1・AC1・AC2）。

確かめること:
- `POST …/data/raw?filename=…` が本文をそのまま `<root>/data/raw/<filename>` に置く（201）
- 同じ名前は 409（上書きしない）・名前の規則に合わなければ 400（パスの区切り・`..`・隠し名）
- 大きさの無い・上限（差し替えて 10 バイト）を超える本文は 413、ファイルも `.part` も残さない
- 無いプロジェクト・root が無いプロジェクトは 404
- 届いた量が Content-Length と合わない・途中で上限を超えた本文は 400 / 413 で、何も残さない
- `GET …/data/raw` は名前の昇順。`.part` と隠しファイルを出さない。フォルダが無ければ空

実行:
    cd apps/web
    PYTHONPATH=src .venv/bin/python tests/test_datafiles.py
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
import tempfile
import urllib.parse
from pathlib import Path

# import より前に一時フォルダを決める（config.DB_PATH は import 時に決まるため）
_TMP = Path(tempfile.mkdtemp(prefix="sai-web-datafiles-"))
os.environ["SAI_HOME"] = str(_TMP / "sai-home")
# SAI_WEB_USERS_ROOT は置かない（置くと多人数モードになり、ログインの確認が要る）
os.environ["SAI_WEB_COOKIE_SECURE"] = "0"
os.environ.pop("SAI_WEB_TOKEN", None)
os.environ.pop("SAI_WEB_RUNS_DIR", None)

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from fastapi import HTTPException  # noqa: E402

from sai_web import control, datafiles  # noqa: E402
from sai_web.main import app  # noqa: E402

HOST = "testserver"
ROOTS = {"alpha": _TMP / "alpha", "gone": _TMP / "gone"}
ORIGINAL_MAX = datafiles.DATA_MAX_BYTES


def fake_find_project(name: str) -> dict:
    """`control.find_project` の代わり（登録簿を読まない）。gone は root が無い。"""
    if name == "alpha":
        return {"name": "alpha", "root": str(ROOTS["alpha"]), "root_exists": True}
    if name == "gone":
        return {"name": "gone", "root": str(ROOTS["gone"]), "root_exists": False}
    raise HTTPException(status_code=404, detail="プロジェクトが見つかりません")


control.find_project = fake_find_project


def raw_dir(project: str = "alpha") -> Path:
    return ROOTS[project] / "data" / "raw"


# --- ASGI を直接叩く ---


async def _run_asgi(method, path, query, body, headers):
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


def call(method, path, *, query="", body=b"", headers=None):
    hdrs = [(b"host", HOST.encode())]
    if body:
        hdrs.append((b"content-length", str(len(body)).encode()))
    for key, value in (headers or {}).items():
        hdrs.append((key.lower().encode(), value.encode()))
    messages = asyncio.run(_run_asgi(method, path, query, body, hdrs))
    start = next(m for m in messages if m["type"] == "http.response.start")
    raw = b"".join(m.get("body", b"") for m in messages if m["type"] == "http.response.body")
    try:
        parsed = json.loads(raw.decode("utf-8")) if raw else {}
    except (json.JSONDecodeError, UnicodeDecodeError):
        parsed = raw
    return start["status"], parsed


def put(project: str, filename: str, body: bytes):
    query = "filename=" + urllib.parse.quote(filename, safe="")
    return call(
        "POST",
        f"/api/projects/{project}/data/raw",
        query=query,
        body=body,
        headers={"Content-Type": "application/octet-stream"},
    )


def get(project: str):
    return call("GET", f"/api/projects/{project}/data/raw")


class FakeRequest:
    """`store_raw` が使う分だけ持つ偽の要求（headers と stream）。"""

    def __init__(self, headers: dict, chunks: list[bytes]) -> None:
        self.headers = headers
        self._chunks = chunks

    async def stream(self):
        for chunk in self._chunks:
            yield chunk


def expect(code: int, coro) -> None:
    """coro は指定した状態番号の HTTPException で失敗すること。"""
    try:
        asyncio.run(coro)
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), "理由を日本語で書く"
        return
    raise AssertionError(f"{code} になるはず")


def test_routes() -> None:
    """2 つの住所が登録済み（AC1・AC2）。"""
    got = []
    for route in app.routes:
        path = getattr(route, "path", "")
        for method in getattr(route, "methods", []) or []:
            got.append((method, path))
    assert ("POST", "/api/projects/{name}/data/raw") in got
    assert ("GET", "/api/projects/{name}/data/raw") in got


def test_list_empty_before_any_put() -> None:
    """置き場がまだ無ければ空の一覧（AC2）。"""
    status, data = get("alpha")
    assert status == 200, data
    assert data == {"files": []}, data


def test_put_and_list() -> None:
    """本文をそのまま置き、一覧に大きさと日時が出る（AC1・AC2）。"""
    body = bytes(range(256)) * 40  # 10240 バイト・バイナリ
    status, data = put("alpha", "a.bin", body)
    assert status == 201, data
    assert data == {"name": "a.bin", "size": len(body), "path": "data/raw/a.bin"}, data
    assert (raw_dir() / "a.bin").read_bytes() == body
    assert not list(raw_dir().glob(".a.bin.*.part"))

    status, data = get("alpha")
    assert status == 200, data
    entry = next(f for f in data["files"] if f["name"] == "a.bin")
    assert entry["size"] == len(body), entry
    assert entry["mtime"].endswith("+00:00"), entry["mtime"]


def test_hidden_and_part_not_listed() -> None:
    """`.part` と隠しファイルは一覧に出ない・名前の昇順（AC2）。"""
    folder = raw_dir()
    (folder / ".secret").write_bytes(b"x")
    (folder / ".half.csv.part").write_bytes(b"x")
    (folder / "b.csv").write_bytes(b"x")
    (folder / "c.csv").write_bytes(b"x")
    status, data = get("alpha")
    assert status == 200, data
    names = [f["name"] for f in data["files"]]
    assert ".secret" not in names and ".half.csv.part" not in names, names
    assert names == sorted(names), names
    assert "b.csv" in names and "c.csv" in names, names


def test_conflict_is_409_and_keeps_the_file() -> None:
    """同じ名前は 409。元のファイルは変わらない（上書きしない）。"""
    status, _ = put("alpha", "same.txt", b"first")
    assert status == 201
    status, data = put("alpha", "same.txt", b"second")
    assert status == 409, data
    assert "上書き" in data["detail"], data
    assert (raw_dir() / "same.txt").read_bytes() == b"first"


def test_bad_names_are_400() -> None:
    """名前の規則に合わなければ 400。何も作らない（AC1）。"""
    for bad in ["../x", "a/b", ".hidden", "bad name.csv", "", "a\n", "-x", "a" * 129]:
        status, data = put("alpha", bad, b"x")
        assert status == 400, (bad, status, data)
    status, data = put("alpha", "a" * 128, b"x")
    assert status == 201, data
    assert (raw_dir() / ("a" * 128)).exists()


def test_size_limits() -> None:
    """上限（差し替えて 10 バイト）を超えれば 413。Content-Length が無くても 413（AC1）。"""
    datafiles.DATA_MAX_BYTES = 10
    try:
        status, data = put("alpha", "over.bin", b"x" * 11)
        assert status == 413, data
        assert not (raw_dir() / "over.bin").exists()
        assert not list(raw_dir().glob(".over.bin.*.part"))

        status, data = put("alpha", "edge.bin", b"x" * 10)
        assert status == 201, data

        status, data = call(
            "POST",
            "/api/projects/alpha/data/raw",
            query="filename=nolength.bin",
            body=b"",
            headers={"Content-Type": "application/octet-stream"},
        )
        assert status == 413, data
    finally:
        datafiles.DATA_MAX_BYTES = ORIGINAL_MAX


def test_missing_project_is_404() -> None:
    """無いプロジェクト・root が無いプロジェクトは 404（AC1）。"""
    for project in ("nope", "gone"):
        status, data = put(project, "a.csv", b"x")
        assert status == 404, (project, data)
        status, data = get(project)
        assert status == 404, (project, data)


def test_no_leftovers_on_truncated_or_overflow() -> None:
    """届いた量が合わない・途中で上限を超えた本文は、何も残さない（AC1）。"""
    folder = raw_dir()
    # Content-Length 5 だが 3 バイトしか届かない → 400
    expect(400, datafiles.store_raw("alpha", "short.bin", FakeRequest({"content-length": "5"}, [b"abc"])))
    assert not (folder / "short.bin").exists()
    assert not list(folder.glob(".short.bin.*.part"))

    # Content-Length は 3 と偽るが、上限（10）を超えて届く → 413・.part も消す
    datafiles.DATA_MAX_BYTES = 10
    try:
        expect(
            413,
            datafiles.store_raw("alpha", "long.bin", FakeRequest({"content-length": "3"}, [b"a" * 6, b"b" * 6])),
        )
    finally:
        datafiles.DATA_MAX_BYTES = ORIGINAL_MAX
    assert not (folder / "long.bin").exists()
    assert not list(folder.glob(".long.bin.*.part"))


def main() -> None:
    ROOTS["alpha"].mkdir(parents=True, exist_ok=True)
    test_routes()
    test_list_empty_before_any_put()
    test_put_and_list()
    test_hidden_and_part_not_listed()
    test_conflict_is_409_and_keeps_the_file()
    test_bad_names_are_400()
    test_size_limits()
    test_missing_project_is_404()
    test_no_leftovers_on_truncated_or_overflow()
    print("test_datafiles: すべて通りました")


if __name__ == "__main__":
    main()
