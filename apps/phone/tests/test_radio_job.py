"""音声化の依頼（計画 10 §3・§5・プロジェクト付きは 17 §3.3・§3.4）の小さなテスト（assert だけ）。

- `_radio_job`: `settings.sai_paper_bin` を一時フォルダの小さな偽スクリプトに差し替え，
  `notify.broadcast` を記録するだけの偽物にして，通知の題名と URL が計画どおりかを確かめる。
  - 成功の URL は `/#/radio/<radio_id>`（応答に `radio_id` が無ければ
    `<フォルダ名>-<番号>` を組み立てる・17 §3.3・§3.4）。
  - 失敗の URL は `/#/paper/<プロジェクト名>/<id>`（17 §3.4）。
- `request_radio`: 400（本文がまだ無い）・409（作成中）・受け付け
  （{"status":"generating", "radio_id": …}）を確かめる。

実行:
    cd <作業用コピー>
    PYTHONPATH=src /Users/touta/workdir/Sai/sai-phone/.venv/bin/python tests/test_radio_job.py
"""

from __future__ import annotations

import asyncio
import tempfile
from pathlib import Path
from types import SimpleNamespace

from fastapi import HTTPException

from sai_phone import papers

# 論文の置き場があるプロジェクトとして渡す値（17 §3.1 で引いたものを想定・root は呼び時に置き換える）
PROJECT = {"name": "sai-paper", "root": "/root/sai-paper", "root_name": "sai-paper"}
PROJECT2 = {"name": "other", "root": "/root/other", "root_name": "other"}


def at(tmp: str, project: dict) -> dict:
    """_radio_job に渡す project（root だけ一時フォルダに差し替える・10 §5）。"""
    return {**project, "root": tmp}



class FakeNotify:
    """broadcast を記録するだけの偽物（10 §5）。"""

    def __init__(self) -> None:
        self.calls: list[tuple[str, str, str]] = []

    def broadcast(self, title: str, body: str, url: str) -> int:
        self.calls.append((title, body, url))
        return 1


def make_bin(tmp: str, line: str, code: int) -> str:
    """標準出力に line を1行出して終了コード code で終わる偽の sai-paper を作る（10 §5）。"""
    path = Path(tmp) / f"fake-sai-paper-{code}.sh"
    path.write_text(f"#!/bin/sh\necho '{line}'\nexit {code}\n")
    path.chmod(0o755)
    return str(path)


async def expect_error(coro, status: int, contains: str) -> None:
    """coro は status（400 / 409）の HTTPException で失敗し，文に contains を含むこと。"""
    try:
        await coro
    except HTTPException as e:
        assert e.status_code == status, f"{status} のはずが {e.status_code}: {e.detail}"
        assert contains in str(e.detail), f"{e.detail} に {contains!r} が無い"
        return
    raise AssertionError(f"{status} になるはず")


def test_job_success(fake: FakeNotify, tmp: str) -> None:
    """成功（終了コード 0・応答に radio_id なし）→ 通知の題名は「🎧 音声ができました」（10 §3）。"""
    fake.calls.clear()
    papers.settings = SimpleNamespace(sai_paper_bin=make_bin(tmp, '{"id":1,"status":"ready"}', 0))
    papers._radio_jobs.clear()
    asyncio.run(papers._radio_job(at(tmp, PROJECT), 1, "題名"))
    # radio_id は応答に無いため `<フォルダ名>-<番号>` を組み立てる（17 §3.3）
    assert fake.calls == [("🎧 音声ができました", "題名", "/#/radio/sai-paper-1")], fake.calls
    assert ("sai-paper", 1) not in papers._radio_jobs, "最後に _radio_jobs から消えること（10 §3）"


def test_job_success_with_radio_id(fake: FakeNotify, tmp: str) -> None:
    """応答に radio_id があればそれが正（sai-paper と名前が同じ決め方でも優先・17 §3.3）。"""
    fake.calls.clear()
    papers.settings = SimpleNamespace(sai_paper_bin=make_bin(tmp, '{"id":3,"radio_id":"other-3","status":"ready"}', 0))
    papers._radio_jobs.clear()
    asyncio.run(papers._radio_job(at(tmp, PROJECT2), 3, "題名"))
    assert fake.calls == [("🎧 音声ができました", "題名", "/#/radio/other-3")], fake.calls
    assert ("other", 3) not in papers._radio_jobs


def test_job_failed(fake: FakeNotify, tmp: str) -> None:
    """失敗（終了コード 1）→ 題名は「音声を作れませんでした」，飛先はそのプロジェクトの論文情報（17 §3.4）。"""
    fake.calls.clear()
    papers.settings = SimpleNamespace(sai_paper_bin=make_bin(tmp, '{"error":"x"}', 1))
    papers._radio_jobs.clear()
    asyncio.run(papers._radio_job(at(tmp, PROJECT), 2, "題名"))
    assert fake.calls == [("音声を作れませんでした", "題名: x", "/#/paper/sai-paper/2")], fake.calls
    assert ("sai-paper", 2) not in papers._radio_jobs


def test_job_unreadable(fake: FakeNotify, tmp: str) -> None:
    """error を読めない失敗 → 「原因不明のエラー」で知らせる（10 §3）。"""
    fake.calls.clear()
    papers.settings = SimpleNamespace(sai_paper_bin=make_bin(tmp, "not json", 1))
    papers._radio_jobs.clear()
    asyncio.run(papers._radio_job(at(tmp, PROJECT), 3, "題名"))
    assert len(fake.calls) == 1, fake.calls
    title, body, url = fake.calls[0]
    assert title == "音声を作れませんでした", title
    assert body.endswith("原因不明のエラー"), body
    assert url == "/#/paper/sai-paper/3", url


# --- request_radio（受け付けの検査・10 §3・17 §3.4） ---

READY = {
    "paper": {"id": 5, "title": "題名"},
    "files": {"extract": "/tmp/extract.md"},
    "podcast": {"status": "ready", "error": None},
}


def test_request_radio() -> None:
    original_job = papers._radio_job
    original_detail = papers.detail
    started: dict[tuple[str, int], str] = {}

    async def fake_job(project: dict, paper_id: int, title: str) -> None:
        started[(project["name"], paper_id)] = title

    async def checks() -> None:
        papers._radio_job = fake_job
        papers._radio_jobs.clear()

        # 1. 受け付け: 202 の {"status": "generating", "radio_id": …} を返し，背景処理を始める
        papers.detail = lambda proj, pid: {**READY, "podcast": None}
        res = await papers.request_radio(PROJECT, 5)
        assert res == {"status": "generating", "radio_id": "sai-paper-5"}, res
        await asyncio.sleep(0)  # create_task を走らせて受け渡しを確かめる
        assert started.get(("sai-paper", 5)) == "題名", started

        # 2. 本文がまだ取り出されていない → 400
        papers.detail = lambda proj, pid: {**READY, "files": {}, "podcast": None}
        await expect_error(papers.request_radio(PROJECT, 6), 400, "本文がまだ取り出されていない")

        # 3. paper_podcasts が generating → 409
        papers.detail = lambda proj, pid: {**READY, "podcast": {"status": "generating"}}
        await expect_error(papers.request_radio(PROJECT, 7), 409, "作成中")

        # 4. 今動いているジョブがある → 409（同じプロジェクトの同じ番号だけ）
        papers.detail = lambda proj, pid: READY
        task = asyncio.create_task(asyncio.sleep(30))
        papers._radio_jobs[("sai-paper", 8)] = task
        try:
            await expect_error(papers.request_radio(PROJECT, 8), 409, "作成中")
            # 別プロジェクトの同じ番号は「作成中」とはしない（17 §3.4）
            res2 = await papers.request_radio(PROJECT2, 8)
            assert res2 == {"status": "generating", "radio_id": "other-8"}, res2
            await asyncio.sleep(0)
            assert ("sai-paper", 8) in papers._radio_jobs, "管理は (プロジェクト名, 番号) の組"
        finally:
            task.cancel()
            papers._radio_jobs.pop(("sai-paper", 8), None)

    try:
        asyncio.run(checks())
    finally:
        papers._radio_job = original_job
        papers.detail = original_detail
        papers._radio_jobs.clear()


def main() -> None:
    fake = FakeNotify()
    papers.notify = fake  # 通知は記録するだけの偽物（実機へは送らない・10 §5）
    with tempfile.TemporaryDirectory() as tmp:
        test_job_success(fake, tmp)
        test_job_success_with_radio_id(fake, tmp)
        test_job_failed(fake, tmp)
        test_job_unreadable(fake, tmp)
    test_request_radio()
    print("test_radio_job: すべて通りました")


main()
