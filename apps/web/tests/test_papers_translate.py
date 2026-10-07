"""PDF 翻訳 API（`POST …/papers/{id}/translate-pdf`）のテスト（assert だけ・偽の CLI を使う）。

もとは遠隔ブラウザの API のテストと同じファイルにあったものを、遠隔ブラウザの削除（U05）で
ここへ移した（遠隔ブラウザとは無関係のテストを消さないため・調査 E3 §1.3）。

実行:
    cd <作業用コピー>
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_papers_translate.py
"""

from __future__ import annotations

import asyncio
import os
import tempfile
from pathlib import Path

from fastapi import HTTPException

TMP = Path(tempfile.mkdtemp(prefix="papers-translate-test-"))
os.environ["SAI_HOME"] = str(TMP / "home")

from sai_web import papers  # noqa: E402
from sai_web.main import app  # noqa: E402

PROJECT = {"name": "p", "root": "/root/p", "root_name": "p"}


async def expect_async(code: int, coro) -> None:
    try:
        await coro
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        return
    raise AssertionError(f"{code} になるはず")


async def test_translate_pdf() -> None:
    calls = []
    result = {"v": (0, b"{}", "")}

    async def fake_run(root, args, timeout, input_bytes=None):
        calls.append(args)
        await asyncio.sleep(0.05)
        return result["v"]

    notes = []
    orig_run, orig_b = papers._run_paper_command, papers.notify.broadcast
    papers._run_paper_command = fake_run
    papers.notify.broadcast = lambda *a, **k: notes.append(a)
    try:
        out = await papers.request_translate_pdf(PROJECT, 7)
        assert out == {"key": "pdf:7", "status": "processing"}
        await expect_async(409, papers.request_translate_pdf(PROJECT, 7))
        await papers._download_jobs[("p", "pdf:7")]["_task"]
        job = papers._download_jobs[("p", "pdf:7")]
        assert job["status"] == "done" and job["kind"] == "translate" and job["percent"] is None
        assert calls == [["paper", "translate-pdf", "7", "--json"]]
        assert notes[-1][0].startswith("📄")
        result["v"] = (0, b"", "時間内に終わりませんでした")
        await papers.request_translate_pdf(PROJECT, 7)  # 終わった後は再依頼できる
        await papers._download_jobs[("p", "pdf:7")]["_task"]
        job = papers._download_jobs[("p", "pdf:7")]
        assert job["status"] == "failed" and "時間内" in job["error"]
        assert notes[-1][0] == "PDF を翻訳できませんでした"
    finally:
        papers._run_paper_command, papers.notify.broadcast = orig_run, orig_b


def test_route() -> None:
    paths = [getattr(r, "path", "") for r in app.routes]
    base = "/api/projects/{name}/papers/"
    assert base + "{paper_id:int}/translate-pdf" in paths


def main() -> None:
    test_route()
    asyncio.run(test_translate_pdf())
    print("test_papers_translate: すべて通りました")


if __name__ == "__main__":
    main()
