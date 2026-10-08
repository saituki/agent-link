"""Mind の全文保存と論文 1 本の Index の小さなテスト（assert だけ・U04）。

確かめること:
- Mind: `papers.mind_save` の検査（空・空白だけ・NUL・200000 字超は 400 で、CLI を 1 回も呼ばない）と、
  正しい本文は `sai-sci project mind --json` の標準入力に渡る
- Index: `papers.request_index` は `{"key": "index:<id>", "status": "processing"}` を返し、
  同じ論文の 2 回目は 409。終わりは通知（chunk と埋め込みの数。本文が無い・失敗は失敗の通知）
- 住所: `PUT …/project/mind` と `POST …/{paper_id:int}/index`（202）が登録済みで、memo の住所は無い

実行:
    cd <作業用コピー>/apps/web
    PYTHONPATH=src .venv/bin/python tests/test_papers_mind_index.py
"""

from __future__ import annotations

import asyncio
import json
from types import SimpleNamespace

from fastapi import HTTPException

from sai_web import papers
from sai_web.main import app

PROJECT = {"name": "sai-paper", "root": "/root/sai-paper", "root_name": "sai-paper"}


def expect(code: int, fn) -> HTTPException:
    """fn は指定した状態番号の HTTPException で失敗し、日本語の理由が付くこと。"""
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{code} のときは日本語で理由を書く"
        return e
    raise AssertionError(f"{code} になるはず")


async def expect_async(code: int, coro) -> HTTPException:
    """await する版（Index の依頼は `async def`）。"""
    try:
        await coro
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{code} のときは日本語で理由を書く"
        return e
    raise AssertionError(f"{code} になるはず")


class FakeRunJson:
    """`papers.cli.run_json` の置き換え係。引数と stdin を全部覚えておく。"""

    def __init__(self) -> None:
        self.calls: list[dict] = []
        self.value: object = {}

    def __call__(self, args, *, app, cwd=None, env=None, timeout=20.0, input_text=None):
        self.calls.append(
            {"args": list(args), "cwd": cwd, "timeout": timeout, "input_text": input_text}
        )
        return self.value

    @property
    def last(self) -> dict:
        assert self.calls, "CLI が呼ばれていない"
        return self.calls[-1]


class FakeNotify:
    """`notify.broadcast` を記録するだけの偽物。"""

    def __init__(self) -> None:
        self.calls: list[tuple[str, str, str]] = []

    def broadcast(self, title: str, body: str, url: str) -> int:
        self.calls.append((title, body, url))
        return 1


def test_mind_save(cli: FakeRunJson) -> None:
    """全文は標準入力で渡し、空・空白だけ・NUL・200000 字超は CLI の前に 400。"""
    cli.calls.clear()
    cli.value = {"ok": True, "bytes": 9}
    assert papers.mind_save(PROJECT, "# mind\n新しい\n") == {"ok": True, "bytes": 9}
    call = cli.last
    assert call["args"] == ["sai-sci", "project", "mind", "--json"], call["args"]
    assert call["cwd"] == "/root/sai-paper", call["cwd"]
    assert call["input_text"] == "# mind\n新しい\n", call["input_text"]

    cli.calls.clear()
    for bad in ("", "  \n\t", "x" * 200001, "a\0b", None, 12):
        expect(400, lambda bad=bad: papers.mind_save(PROJECT, bad))
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"

    # 上限ちょうどは通る
    papers.mind_save(PROJECT, "x" * 200000)
    assert len(cli.last["input_text"]) == 200000


def fake_paper_command(stdout: bytes, code: int = 0, reason: str = ""):
    """`papers._run_paper_command` の置き換え係。呼ばれた引数を覚えておく。"""
    calls: list[dict] = []

    async def run(root, args, timeout, input_bytes=None):
        calls.append({"root": root, "args": list(args), "timeout": timeout})
        return code, stdout, reason

    return run, calls


async def run_index(stdout: bytes, code: int = 0, reason: str = ""):
    """Index を 1 本作り終わるまで走らせ、(仕事, CLI の呼び出し) を返す。"""
    papers._download_jobs.clear()
    fake, calls = fake_paper_command(stdout, code, reason)
    original = papers._run_paper_command
    papers._run_paper_command = fake
    try:
        out = await papers.request_index(PROJECT, 12)
        assert out == {"key": "index:12", "status": "processing"}, out
        # まだ終わっていないあいだは同じ論文を受け付けない
        await expect_async(409, papers.request_index(PROJECT, 12))
        job = papers._download_jobs[("sai-paper", "index:12")]
        await job["_task"]
    finally:
        papers._run_paper_command = original
    return job, calls


def test_index_job(notify: FakeNotify) -> None:
    """成功は done と「chunk・埋め込み」の通知。本文が無い・失敗は failed の通知。"""
    ok = json.dumps(
        {
            "paper_id": 12,
            "index": "default",
            "chunks_created": 40,
            "embedded_count": 40,
            "chunks": 40,
            "chunks_with_embeddings": 40,
        }
    ).encode()
    notify.calls.clear()
    job, calls = asyncio.run(run_index(ok))
    assert job["status"] == "done", job
    assert calls[0]["args"] == ["index", "paper", "12", "--json"], calls[0]
    assert calls[0]["timeout"] == 1800.0, calls[0]
    assert notify.calls[-1][0] == "🔎 Index ができました", notify.calls
    assert "chunk 40・埋め込み 40" in notify.calls[-1][1], notify.calls
    assert notify.calls[-1][2] == "/#/paper/sai-paper/12", notify.calls

    # 本文が無い（chunk 0）は失敗として通知する
    notify.calls.clear()
    empty = json.dumps({"paper_id": 12, "chunks": 0, "chunks_with_embeddings": 0}).encode()
    job, _ = asyncio.run(run_index(empty))
    assert job["status"] == "failed", job
    assert "本文が無いので Index を作れませんでした" in notify.calls[-1][1], notify.calls

    # CLI の失敗（理由あり）も失敗として通知する
    notify.calls.clear()
    job, _ = asyncio.run(run_index(b"", code=1, reason="埋め込みの API に届きません"))
    assert job["status"] == "failed", job
    assert notify.calls[-1][0] == "Index を作れませんでした", notify.calls
    assert "埋め込みの API に届きません" in notify.calls[-1][1], notify.calls


def test_routes() -> None:
    """Mind の PUT と Index の POST（202）が登録済み。memo の住所は無い。"""
    got = {}
    for route in app.routes:
        path = getattr(route, "path", "")
        for method in getattr(route, "methods", []) or []:
            got[(method, path)] = route
    mind = got.get(("PUT", "/api/projects/{name}/papers/project/mind"))
    assert mind is not None, "PUT …/project/mind が無い"
    index = got.get(("POST", "/api/projects/{name}/papers/{paper_id:int}/index"))
    assert index is not None, "POST …/{paper_id}/index が無い"
    assert index.status_code == 202, index.status_code
    assert ("POST", "/api/projects/{name}/papers/project/memo") not in got, "memo が残っている"


def main() -> None:
    cli = FakeRunJson()
    papers.cli.run_json = cli
    # 引数を文字列として確かめるため、CLI の場所を "sai-paper" / "sai-sci" に固定する
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper", sai_sci_bin="sai-sci")
    notify = FakeNotify()
    papers.notify = notify  # 通知は記録するだけの偽物（実機へは送らない）
    test_mind_save(cli)
    test_index_job(notify)
    test_routes()
    print("test_papers_mind_index: すべて通りました")


main()
