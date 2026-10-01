"""リモートブラウザ API と PDF 翻訳 API のテスト（assert だけ・偽の Chrome だけを使う）。

実行:
    cd <作業用コピー>
    PYTHONPATH=apps/phone/src ~/workdir/Sai/sai-moon/apps/phone/.venv/bin/python apps/phone/tests/test_rbrowser.py
"""

from __future__ import annotations

import asyncio
import json
import os
import stat
import sys
import tempfile
import time
from pathlib import Path

from fastapi import HTTPException

TMP = Path(tempfile.mkdtemp(prefix="rbrowser-test-"))
os.environ["SAI_SAI_HOME"] = str(TMP / "home")

from sai_phone import papers, rbrowser  # noqa: E402
from sai_phone.main import app  # noqa: E402

PROJECT = {"name": "p", "root": "/root/p", "root_name": "p"}


def expect(code: int, fn) -> None:
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip()
        return
    raise AssertionError(f"{code} になるはず")


async def expect_async(code: int, coro) -> None:
    try:
        await coro
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        return
    raise AssertionError(f"{code} になるはず")


def test_doi() -> None:
    assert rbrowser.check_doi("10.1038/abc") == "10.1038/abc"
    for bad in ["abc", "", "10.1038/abc\n", "10.1038/a b", "10.1038/" + "a" * 201, None, 5]:
        expect(400, lambda b=bad: rbrowser.check_doi(b))


def test_input_check() -> None:
    good = [
        {"type": "tap", "x": 0.5, "y": 1},
        {"type": "scroll", "dy": -3000},
        {"type": "text", "text": "abc"},
        {"type": "key", "key": "Enter"},
        {"type": "back"},
        {"type": "reload"},
    ]
    for g in good:
        rbrowser.check_input(g)
    bad = [
        {"type": "tap", "x": 1.1, "y": 0},
        {"type": "tap", "x": True, "y": 0},
        {"type": "tap", "x": 0.5},
        {"type": "scroll", "dy": 3001},
        {"type": "scroll", "dy": True},
        {"type": "scroll", "dy": 1.5},
        {"type": "text", "text": "a\nb"},
        {"type": "text", "text": ""},
        {"type": "text", "text": "a" * 201},
        {"type": "key", "key": "F1"},
        {"type": "back", "x": 1},
        {"type": "nope"},
        "tap",
    ]
    for b in bad:
        expect(400, lambda b=b: rbrowser.check_input(b))


def test_downloads() -> None:
    d = rbrowser.download_dir()
    d.mkdir(parents=True, exist_ok=True)
    (d / "a.pdf").write_bytes(b"%PDF-1")
    (d / "b.pdf.crdownload").write_bytes(b"x")
    (d / "c.pdf").write_bytes(b"%PDF-1")
    (d / "c.pdf.crdownload").write_bytes(b"xx")
    (d / ".hidden.pdf").write_bytes(b"x")
    (d / "sub").mkdir(exist_ok=True)
    got = {x["name"]: x for x in rbrowser.list_downloads()}
    assert set(got) == {"a.pdf", "b.pdf", "c.pdf"}, got
    assert got["a.pdf"]["done"] is True
    assert got["b.pdf"]["done"] is False
    assert got["c.pdf"]["done"] is False and got["c.pdf"]["size"] == 2
    assert rbrowser.download_path("a.pdf") == d / "a.pdf"
    for bad in ["../a.pdf", "a/b.pdf", "x.txt"]:
        expect(400, lambda b=bad: rbrowser.download_path(b))
    expect(404, lambda: rbrowser.download_path("none.pdf"))
    expect(409, lambda: rbrowser.download_path("c.pdf"))
    expect(409, lambda: rbrowser.download_path("b.pdf"))


async def test_import() -> None:
    d = rbrowser.download_dir()
    (d / "ok.pdf").write_bytes(b"%PDF-1.7 body")
    (d / "bad.pdf").write_bytes(b"<html>")
    calls = []

    async def fake(project, target, file_name=None, body=None):
        calls.append((target, file_name, body))
        return {"key": "manual:paper:3", "status": "downloading"}

    orig = papers.request_manual_import
    papers.request_manual_import = fake
    try:
        await expect_async(400, papers.request_browser_import(PROJECT, {"file_name": "ok.pdf"}))
        await expect_async(
            400,
            papers.request_browser_import(
                PROJECT, {"file_name": "ok.pdf", "paper_id": 3, "import_id": "x"}
            ),
        )
        await expect_async(400, papers.request_browser_import(PROJECT, {"paper_id": 3}))
        await expect_async(
            400, papers.request_browser_import(PROJECT, {"paper_id": 3, "file_name": "bad.pdf"})
        )
        await expect_async(
            404, papers.request_browser_import(PROJECT, {"paper_id": 3, "file_name": "no.pdf"})
        )
        assert not calls
        out = await papers.request_browser_import(PROJECT, {"paper_id": 3, "file_name": "ok.pdf"})
        assert out["status"] == "downloading"
        assert calls == [({"paper_id": 3}, None, b"%PDF-1.7 body")], calls
        assert (d / "ok.pdf").exists()  # 取り込み後も消さない
    finally:
        papers.request_manual_import = orig


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
        assert calls == [["phone", "translate-pdf", "7"]]
        assert notes[-1][0].startswith("📄")
        result["v"] = (0, b"", "時間内に終わりませんでした")
        await papers.request_translate_pdf(PROJECT, 7)  # 終わった後は再依頼できる
        await papers._download_jobs[("p", "pdf:7")]["_task"]
        job = papers._download_jobs[("p", "pdf:7")]
        assert job["status"] == "failed" and "時間内" in job["error"]
        assert notes[-1][0] == "PDF を翻訳できませんでした"
    finally:
        papers._run_paper_command, papers.notify.broadcast = orig_run, orig_b


FAKE = r'''#!/usr/bin/env python3
import json, os, sys
log = os.environ["FAKE_LOG"]
buf = b""
while True:
    chunk = os.read(3, 65536)
    if not chunk:
        break
    buf += chunk
    while b"\0" in buf:
        raw, buf = buf.split(b"\0", 1)
        m = json.loads(raw)
        meth = m["method"]
        with open(log, "a") as f:
            f.write(json.dumps({"method": meth, "params": m.get("params"), "sid": m.get("sessionId")}) + "\n")
        res = {}
        if meth == "Target.getTargets":
            res = {"targetInfos": [{"type": "page", "targetId": "T1"}]}
        elif meth == "Target.attachToTarget":
            res = {"sessionId": "S1"}
        elif meth == "Target.getTargetInfo":
            res = {"targetInfo": {"url": "https://doi.org/x", "title": "T"}}
        elif meth == "Page.captureScreenshot":
            res = {"data": "/9j/4AAQ"}
        elif meth == "Page.getNavigationHistory":
            res = {"currentIndex": 1, "entries": [{"id": 10}, {"id": 11}]}
        os.write(4, json.dumps({"id": m["id"], "result": res}).encode() + b"\0")
        if meth == "Browser.close":
            sys.exit(0)
'''


async def test_fake_chrome() -> None:
    fake = TMP / "fake-chrome"
    fake.write_text(FAKE)
    fake.chmod(fake.stat().st_mode | stat.S_IXUSR)
    log = TMP / "cdp.log"
    os.environ["SAI_PHONE_CHROME"] = str(fake)
    os.environ["FAKE_LOG"] = str(log)
    await expect_async(409, rbrowser.screenshot())
    await expect_async(400, rbrowser.send_input({"type": "nope"}))
    st = await rbrowser.state()
    assert st["open"] is False and st["url"] == "" and st["width"] == 600
    await expect_async(400, rbrowser.open_page("abc"))
    st = await rbrowser.open_page("10.1038/abc")
    assert st["open"] is True and st["url"] == "https://doi.org/x", st
    assert (await rbrowser.screenshot()).startswith(b"\xff\xd8")
    for p in [
        {"type": "tap", "x": 0.5, "y": 0.25},
        {"type": "scroll", "dy": 100},
        {"type": "text", "text": "hi"},
        {"type": "key", "key": "Enter"},
        {"type": "back"},
        {"type": "reload"},
    ]:
        await rbrowser.send_input(p)
    proc = rbrowser._proc
    await rbrowser.close()
    assert proc.poll() is not None
    assert (await rbrowser.state())["open"] is False
    rows = [json.loads(line) for line in log.read_text().splitlines()]
    methods = [r["method"] for r in rows]
    for m in ["Input.dispatchMouseEvent", "Input.insertText", "Input.dispatchKeyEvent",
              "Page.navigateToHistoryEntry", "Page.reload", "Browser.setDownloadBehavior"]:
        assert m in methods, m
    nav = next(r for r in rows if r["method"] == "Page.navigate")
    assert nav["params"]["url"] == "https://doi.org/10.1038/abc" and nav["sid"] == "S1"
    tap = [r["params"] for r in rows if r["method"] == "Input.dispatchMouseEvent"][:3]
    assert [t["type"] for t in tap] == ["mouseMoved", "mousePressed", "mouseReleased"]
    assert tap[0]["x"] == 300 and tap[0]["y"] == 250
    hist = next(r for r in rows if r["method"] == "Page.navigateToHistoryEntry")
    assert hist["params"] == {"entryId": 10}
    prefs = json.loads((rbrowser.profile_dir() / "Default" / "Preferences").read_text())
    assert prefs["plugins"]["always_open_pdf_externally"] is True


def test_routes() -> None:
    paths = [getattr(r, "path", "") for r in app.routes]
    pid = next(i for i, p in enumerate(paths) if p.endswith("/papers/{paper_id:int}"))
    base = "/api/projects/{name}/papers/"
    for tail in ["browser/open", "browser/state", "browser/screen", "browser/input", "browser/import"]:
        assert paths.index(base + tail) < pid, tail
    assert [i for i, p in enumerate(paths) if p == base + "browser"][0] < pid
    assert base + "{paper_id:int}/translate-pdf" in paths


def main() -> None:
    test_doi()
    test_input_check()
    test_downloads()
    test_routes()

    async def run() -> None:
        await test_import()
        await test_translate_pdf()
        await test_fake_chrome()

    asyncio.run(run())
    print("OK")


if __name__ == "__main__":
    main()
