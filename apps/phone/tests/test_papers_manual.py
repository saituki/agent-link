"""引用一覧（citations）と手動取り込み（manual import / アップロード）のテスト（assert だけ）。

確かめること:
- `phone citations` / `manual-candidates` の引数
- 引用の抜き出し（`phone citations-extract`）: 段 0→1→完了、docling_error の文、
  失敗は `failed`、同じ論文の 2 回目は 409
- 手動取り込みの対象の検査（4 通りのどちらかちょうど 1 つ・型・範囲・クエリの文字列）
- ファイル名の検査（`../a.pdf` / `a/b.pdf` / `a.txt` / 空 / NUL / 256 字）
- `--file` と `--stdin` の引数の組み立て、取り込んだあとの after-download 、失敗の扱い
- アップロードの 415 / 413 / 400 の分かれ
- 一覧に `kind` が付き、音声化と要旨の訳の仕事も出ること

実行:
    cd <作業用コピー>
    PYTHONPATH=src ~/workdir/Sai/sai-moon/apps/phone/.venv/bin/python tests/test_papers_manual.py
"""

from __future__ import annotations

import asyncio
import json
from types import SimpleNamespace

from fastapi import HTTPException

from sai_phone import papers
from sai_phone.main import app

PROJECT = {"name": "sai-paper", "root": "/root/sai-paper", "root_name": "sai-paper"}
PROJECT2 = {"name": "other", "root": "/root/other", "root_name": "other"}


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
    """await する版。"""
    try:
        await coro
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{code} のときは日本語で理由を書く"
        return e
    raise AssertionError(f"{code} になるはず")


class FakeRunJson:
    """`papers.cli.run_json` の置き換え係。引数を全部覚えておく。"""

    def __init__(self) -> None:
        self.calls: list[dict] = []
        self.value: object = {}
        self.error: Exception | None = None

    def __call__(self, args, *, app, cwd=None, env=None, timeout=20.0, input_text=None):
        self.calls.append(
            {
                "args": list(args),
                "app": app,
                "cwd": cwd,
                "env": env,
                "timeout": timeout,
                "input_text": input_text,
            }
        )
        if self.error is not None:
            raise self.error
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


class FakeStdin:
    """`proc.stdin` の偽物（書いた内容を覚えておく）。"""

    def __init__(self) -> None:
        self.written = b""
        self.closed = False

    def write(self, data) -> None:
        self.written += bytes(data)

    async def drain(self) -> None:
        await asyncio.sleep(0)

    def close(self) -> None:
        self.closed = True

    async def wait_closed(self) -> None:
        await asyncio.sleep(0)


class FakeStream:
    """`proc.stderr` / `proc.stdout` の偽物。"""

    def __init__(self, data: bytes) -> None:
        self._rest = data
        self._lines = data.splitlines(keepends=True) if data else []

    async def readline(self) -> bytes:
        await asyncio.sleep(0)
        return self._lines.pop(0) if self._lines else b""

    async def read(self) -> bytes:
        await asyncio.sleep(0)
        return self._rest


class FakeProc:
    """`asyncio.create_subprocess_exec` の偽物（test_papers_import_api.py と同じ形）。"""

    def __init__(
        self,
        stdout: bytes,
        returncode: int = 0,
        hang: bool = False,
        stderr: bytes = b"",
        has_stdin: bool = False,
    ) -> None:
        self._stdout = stdout
        self.returncode = returncode
        self.killed = False
        self._hang = hang
        self.stdout = FakeStream(stdout)
        self.stderr = FakeStream(stderr)
        self.stdin = FakeStdin() if has_stdin else None

    async def communicate(self):
        if self._hang:
            await asyncio.sleep(3600)
        return self._stdout, b""

    async def wait(self) -> int:
        if self._hang and not self.killed:
            await asyncio.sleep(3600)
        return self.returncode

    def kill(self) -> None:
        self.killed = True
        self._hang = False


def fake_exec(results: dict):
    """引数（実行ファイル名を除いたもの）ごとに返す偽物を作る係。"""
    seen: list[list[str]] = []
    procs: list[FakeProc] = []

    async def create(*args, **kwargs):
        argv = [str(a) for a in args]
        seen.append(argv)
        hit = results.get(" ".join(argv[1:]))
        if hit is None:
            proc = FakeProc(b"{}", 0)
        elif isinstance(hit, Exception):
            raise hit
        else:
            proc = hit
        procs.append(proc)
        return proc

    return create, seen, procs


class FakeRequest:
    """アップロードの `request` の偽物（ヘッダー・本文・ストリーム）。"""

    def __init__(self, body: bytes, content_type="application/pdf", length=None) -> None:
        self.headers = {"content-type": content_type}
        if length != "omit":
            self.headers["content-length"] = str(
                len(body) if length is None else length
            )
        self._body = body

    async def stream(self):
        # 64KB ずつ（区切り发挥作用ように）
        step = 65536
        for i in range(0, len(self._body), step):
            await asyncio.sleep(0)
            yield self._body[i : i + step]


# --- 引用一覧（読み取り） ---


def test_citations(cli: FakeRunJson) -> None:
    """引用一覧は `paper citations <id>`。"""
    cli.calls.clear()
    cli.value = {
        "paper": {"id": 12, "title": "題名"},
        "key": "ArXiv:1706.03762",
        "has_pdf": True,
        "extracted": True,
        "refs": [],
    }
    out = papers.citations(PROJECT, 12)
    assert out["extracted"] is True, out
    assert cli.last["args"] == ["sai-paper", "paper", "citations", "12"], cli.last["args"]
    assert cli.last["cwd"] == "/root/sai-paper", cli.last["cwd"]
    assert cli.last["env"]["SAI_PROJECT_ROOT"] == "/root/sai-paper", cli.last["env"]
    assert cli.last["env"]["SAI_ORIGIN"] == "phone", cli.last["env"]


def test_manual_candidates(cli: FakeRunJson) -> None:
    """候補は `paper candidates --minutes 30`（利用者が決めた 30 分）。"""
    cli.calls.clear()
    cli.value = {"dir": "~/Downloads", "minutes": 30, "items": []}
    assert papers.manual_candidates(PROJECT)["minutes"] == 30
    assert cli.last["args"] == [
        "sai-paper",
        "paper",
        "candidates",
        "--minutes",
        "30",
    ], cli.last["args"]


# --- 引用の抜き出し ---


def test_cite_extract_success(fake: FakeNotify) -> None:
    """段 0 → 1 → 完了（100%）。`docling_error` があれば messages に足す。"""
    fake.calls.clear()
    create, seen, _procs = fake_exec(
        {
            "paper citations-extract 12": FakeProc(
                json.dumps(
                    {"paper_id": 12, "refs": 42, "sentences": 87, "docling_error": None}
                ).encode(),
                stderr="sai-progress step=1\nsai-progress msg=本文を読んでいます\n".encode(),
            )
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        res = await papers.request_cite_extract(PROJECT, 12)
        assert res == {"key": "cite:12", "status": "processing"}, res
        job = papers._download_jobs[("sai-paper", "cite:12")]
        assert job["kind"] == "cite-extract", job
        assert job["status"] == "processing", job
        assert job["step"] == 0, job
        for _ in range(20):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "cite:12")]
        assert job["status"] == "done", job
        assert job["step"] == 2, job
        assert job["percent"] == 100, job
        assert job["paper_id"] == 12, job
        assert job["messages"] == ["本文を読んでいます"], job["messages"]
        listed = papers.list_downloads(PROJECT)["jobs"]
        assert listed[0]["kind"] == "cite-extract", listed[0]
        assert listed[0]["steps"] == ["PDF確認", "引用抽出"], listed[0]

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert seen[0][1:] == ["paper", "citations-extract", "12"], seen
    assert len(fake.calls) == 1 and fake.calls[0][2].endswith("/cites"), fake.calls


def test_cite_extract_docling_error(fake: FakeNotify) -> None:
    """docling で読めなかったときは S2 の引用だけになった旨を messages に入れる。"""
    fake.calls.clear()
    create, _seen, _procs = fake_exec(
        {
            "paper citations-extract 13": FakeProc(
                json.dumps(
                    {
                        "paper_id": 13,
                        "refs": 5,
                        "sentences": 0,
                        "docling_error": "docling がありません",
                    }
                ).encode(),
                stderr=b"sai-progress step=1\n",
            )
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        await papers.request_cite_extract(PROJECT, 13)
        for _ in range(20):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "cite:13")]
        assert job["status"] == "done", job
        assert job["messages"] == [
            "docling で本文を読めなかったので、S2 の引用だけにしました: docling がありません"
        ], job["messages"]

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert len(fake.calls) == 1, fake.calls


def test_cite_extract_failed(fake: FakeNotify) -> None:
    """失敗（PDF 無しなど）→ `failed` と通知。"""
    fake.calls.clear()
    create, _seen, _procs = fake_exec(
        {
            "paper citations-extract 14": FakeProc(
                json.dumps({"error": "PDF がありません"}).encode(), 1
            )
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        await papers.request_cite_extract(PROJECT, 14)
        for _ in range(20):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "cite:14")]
        assert job["status"] == "failed", job
        assert job["error"] == "PDF がありません", job

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert fake.calls and "引用を抜けませんでした" in fake.calls[0][0], fake.calls


def test_cite_extract_conflict() -> None:
    """同じ論文の 2 回目は 409。"""
    create, _seen, _procs = fake_exec(
        {"paper citations-extract 15": FakeProc(b"{}", 0, hang=True)}
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    original_timeout = papers.CITE_EXTRACT_TIMEOUT
    papers.CITE_EXTRACT_TIMEOUT = 30.0

    async def main() -> None:
        await papers.request_cite_extract(PROJECT, 15)
        await asyncio.sleep(0)
        e = await expect_async(409, papers.request_cite_extract(PROJECT, 15))
        assert "抜き出し中" in str(e.detail), e.detail
        # 別プロジェクトの同じ番号は別扱い
        await papers.request_cite_extract(PROJECT2, 15)

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers.CITE_EXTRACT_TIMEOUT = original_timeout
        papers._download_jobs.clear()


# --- 手動取り込みの検査 ---


def test_manual_target_validation() -> None:
    """対象は 4 通りのどちらかちょうど 1 つ（型と範囲も見る）。"""
    key, args, title = papers._manual_target({"log_id": 3, "position": 2})
    assert key == "log:3:2", key
    assert args == ["--log", "3", "--position", "2"], args
    assert title == "論文", title
    key, args, _ = papers._manual_target({"import_id": "DOI:1", "title": "題名"})
    assert key == "import:DOI:1", key
    assert args == ["--import-id", "DOI:1"], args
    key, args, _ = papers._manual_target({"cite_paper_id": 12, "ref_no": 3})
    assert key == "cite:12:3", key
    assert args == ["--cite", "12", "--ref", "3"], args
    key, args, _ = papers._manual_target({"paper_id": 12})
    assert key == "paper:12", key
    assert args == ["--paper-id", "12"], args

    # アップロードはクエリから引くので数字の文字列も受ける
    key, args, _ = papers._manual_target({"log_id": "3", "position": "2"})
    assert (key, args) == ("log:3:2", ["--log", "3", "--position", "2"]), (key, args)
    key, _args, _ = papers._manual_target({"paper_id": "12"})
    assert key == "paper:12", key

    for bad in (
        {},
        {"title": "x"},
        {"log_id": 3, "position": 2, "paper_id": 12},
        {"log_id": 3, "position": 2, "import_id": "DOI:1"},
        {"log_id": 3, "position": 2, "cite_paper_id": 12, "ref_no": 3},
        {"log_id": 3},
        {"position": 2},
        {"log_id": 0, "position": 2},
        {"log_id": 3, "position": -1},
        {"log_id": True, "position": 2},
        {"log_id": "three", "position": 2},
        {"import_id": ""},
        {"import_id": 12},
        {"cite_paper_id": 12},
        {"ref_no": 3},
        {"cite_paper_id": 12, "ref_no": 0},
        {"paper_id": 0},
        {"paper_id": "abc"},
        "not a dict",
    ):
        expect(400, lambda bad=bad: papers._manual_target(bad))


def test_pdf_name_validation() -> None:
    """ファイル名は 1〜255 字・.pdf で終わる物だけ（パスは受けない）。"""
    assert papers._validate_pdf_name("paper.pdf") == "paper.pdf"
    assert papers._validate_pdf_name("Paper.PDF") == "Paper.PDF"
    assert papers._validate_pdf_name("a b（1）.pdf") == "a b（1）.pdf"
    assert papers._validate_pdf_name("a" * 251 + ".pdf") == "a" * 251 + ".pdf"
    for bad in (
        "",
        "a" * 252 + ".pdf",  # 256 字
        "../a.pdf",
        "a/b.pdf",
        "a\\b.pdf",
        "a..pdf",
        "..",
        "a.txt",
        "a.pdf ",
        "a.pdf\nb",
        "a\0.pdf",
        None,
        12,
    ):
        e = expect(400, lambda bad=bad: papers._validate_pdf_name(bad))
        assert "ファイル名" in str(e.detail), e.detail


# --- 手動取り込み（--file / --stdin） ---


def test_manual_import_file(fake: FakeNotify) -> None:
    """ダウンロードフォルダのファイル名は `--file … --minutes 30` で渡す。"""
    fake.calls.clear()
    create, seen, _procs = fake_exec(
        {
            "paper import-pdf --log 3 --position 2 --file paper.pdf --minutes 30": FakeProc(
                json.dumps(
                    {
                        "paper_id": 71,
                        "title": "題名",
                        "status": "imported",
                        "pdf_path": "/root/sai-paper/.sai/paper/papers/71/paper.pdf",
                    }
                ).encode()
            ),
            "paper after-download 71": FakeProc(
                json.dumps({"paper_id": 71, "messages": []}).encode(),
                stderr="sai-progress step=1\nsai-progress msg=図を 2 件抽出しました\n".encode(),
            ),
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        res = await papers.request_manual_import(
            PROJECT, {"log_id": 3, "position": 2, "title": "題名"}, "paper.pdf"
        )
        assert res == {"key": "manual:log:3:2", "status": "downloading"}, res
        job = papers._download_jobs[("sai-paper", "manual:log:3:2")]
        assert job["kind"] == "manual", job
        assert job["status"] == "downloading", job
        for _ in range(30):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "manual:log:3:2")]
        assert job["status"] == "done", job
        assert job["paper_id"] == 71, job
        assert job["percent"] == 100, job
        assert job["messages"] == ["図を 2 件抽出しました"], job["messages"]
        listed = papers.list_downloads(PROJECT)["jobs"]
        assert listed[0]["kind"] == "manual", listed[0]
        assert listed[0]["steps"] == ["PDF取得", "図抽出", "翻訳"], listed[0]

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert seen[0][1:] == [
        "paper",
        "import-pdf",
        "--log",
        "3",
        "--position",
        "2",
        "--file",
        "paper.pdf",
        "--minutes",
        "30",
    ], seen
    assert seen[1][1:] == ["paper", "after-download", "71"], seen
    # 「保存しました」と「取り込みました」の 2 回
    assert [c[0] for c in fake.calls] == ["📄 論文を保存しました"], fake.calls


def test_manual_import_stdin(fake: FakeNotify) -> None:
    """アップロードは `--stdin` で本文を標準入力に書く（ファイル名は付けない）。"""
    fake.calls.clear()
    body = b"%PDF-1.7\nhello"
    create, seen, procs = fake_exec(
        {
            "paper import-pdf --paper-id 12 --stdin": FakeProc(
                json.dumps(
                    {"paper_id": 12, "title": "題名", "status": "imported", "pdf_path": "/x.pdf"}
                ).encode(),
                has_stdin=True,
            ),
            "paper after-download 12": FakeProc(json.dumps({"paper_id": 12}).encode()),
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        await papers.request_manual_import(
            PROJECT, {"paper_id": 12, "title": "題名"}, body=body
        )
        for _ in range(30):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "manual:paper:12")]
        assert job["status"] == "done", job
        assert job["paper_id"] == 12, job

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert seen[0][1:] == ["paper", "import-pdf", "--paper-id", "12", "--stdin"], seen
    assert procs[0].stdin.written == body, procs[0].stdin.written
    assert procs[0].stdin.closed, "標準入力は閉じる"


def test_manual_import_bad_source(fake: FakeNotify) -> None:
    """中身は 1 つだけ。ファイル名も本文も無ければ 400（仕事を作らない）。"""
    fake.calls.clear()
    original = papers.asyncio.create_subprocess_exec
    called: list = []

    async def create(*args, **kwargs):  # 呼ばれてはいけない
        called.append(args)
        return FakeProc(b"{}", 0)

    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        await expect_async(
            400, papers.request_manual_import(PROJECT, {"paper_id": 12})
        )
        await expect_async(
            400,
            papers.request_manual_import(PROJECT, {"paper_id": 12}, "a.pdf", b"%PDF-"),
        )
        await expect_async(
            400, papers.request_manual_import(PROJECT, {"paper_id": 12}, "../a.pdf")
        )
        await expect_async(
            400, papers.request_manual_import(PROJECT, {"paper_id": 12}, "a.txt")
        )
        await expect_async(
            400, papers.request_manual_import(PROJECT, {"paper_id": 12}, body=b"")
        )
        assert papers._download_jobs == {}, papers._download_jobs

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert called == [], f"検査前に CLI が呼ばれた: {called}"


def test_manual_import_failed(fake: FakeNotify) -> None:
    """取り込みの失敗 → `failed` と通知（after-download は走らない）。"""
    fake.calls.clear()
    create, seen, _procs = fake_exec(
        {
            "paper import-pdf --cite 12 --ref 3 --file a.pdf --minutes 30": FakeProc(
                json.dumps({"error": "この論文は既に Library にあります（ID 34）"}).encode(), 1
            )
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        await papers.request_manual_import(
            PROJECT, {"cite_paper_id": 12, "ref_no": 3, "title": "題名"}, "a.pdf"
        )
        for _ in range(20):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "manual:cite:12:3")]
        assert job["status"] == "failed", job
        assert job["error"] == "この論文は既に Library にあります（ID 34）", job

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert len(seen) == 1, seen
    assert fake.calls == [
        (
            "論文を取り込めませんでした",
            "題名: この論文は既に Library にあります（ID 34）",
            "/#/papers/library",
        )
    ], fake.calls


def test_manual_import_conflict() -> None:
    """同じキーの 2 回目は 409。"""
    create, _seen, _procs = fake_exec(
        {
            "paper import-pdf --paper-id 20 --file a.pdf --minutes 30": FakeProc(
                b"{}", 0, hang=True
            )
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    original_timeout = papers.MANUAL_TIMEOUT
    papers.MANUAL_TIMEOUT = 30.0

    async def main() -> None:
        await papers.request_manual_import(PROJECT, {"paper_id": 20}, "a.pdf")
        await asyncio.sleep(0)
        e = await expect_async(
            409, papers.request_manual_import(PROJECT, {"paper_id": 20}, "a.pdf")
        )
        assert "取り込み中" in str(e.detail), e.detail

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers.MANUAL_TIMEOUT = original_timeout
        papers._download_jobs.clear()


# --- アップロードの検査（415 / 413 / 400） ---


async def read_upload(body: bytes, **kwargs) -> bytes:
    return await papers.read_pdf_upload(FakeRequest(body, **kwargs), {"paper_id": 12})


def test_upload_limits() -> None:
    """Content-Type が違えば 415 / 大きすぎれば 413 / 先頭が違えば 400。"""
    good = b"%PDF-1.7\nhello"

    async def ok() -> None:
        assert await read_upload(good) == good
        # charset が付けても application/pdf なら受ける
        assert await read_upload(good, content_type="application/pdf; charset=binary") == good

    asyncio.run(ok())

    # 415（種類が違う）
    for ctype in ("application/octet-stream", "text/plain", "application/json", ""):
        try:
            asyncio.run(read_upload(good, content_type=ctype))
        except HTTPException as e:
            assert e.status_code == 415, f"{ctype}: {e.status_code}"
        else:
            raise AssertionError(f"{ctype} は 415 のはず")

    # 413（Content-Length が無い / 超えている / 読みながら超える）
    try:
        asyncio.run(read_upload(good, length="omit"))
    except HTTPException as e:
        assert e.status_code == 413, e.status_code
    else:
        raise AssertionError("Content-Length が無ければ 413 のはず")
    try:
        asyncio.run(read_upload(good, length=papers.UPLOAD_MAX_BYTES + 1))
    except HTTPException as e:
        assert e.status_code == 413, e.status_code
    else:
        raise AssertionError("上限を越えたら 413 のはず")
    # 宣言は小さくても、実際に大きいなら 413
    big = b"%PDF-" + b"x" * (papers.UPLOAD_MAX_BYTES + 10)
    try:
        asyncio.run(read_upload(big, length=10))
    except HTTPException as e:
        assert e.status_code == 413, e.status_code
    else:
        raise AssertionError("読みながら上限を超えたら 413 のはず")

    # 400（PDF ではない）
    for body in (b"hello", b"%PD", b"", b"\x00\x01\x02\x03\x04\x05"):
        try:
            asyncio.run(read_upload(body))
        except HTTPException as e:
            assert e.status_code == 400, f"{body!r}: {e.status_code}"
        else:
            raise AssertionError(f"{body!r} は 400 のはず")


def test_upload_max_is_100mb() -> None:
    """上限は利用者が決めた 100MB（104857600 バイト）。"""
    assert papers.UPLOAD_MAX_BYTES == 104857600, papers.UPLOAD_MAX_BYTES
    assert papers.MANUAL_MINUTES == 30, papers.MANUAL_MINUTES


# --- 一覧に載る仕事（音声化・要旨の訳） ---


def test_list_includes_radio(cli: FakeRunJson) -> None:
    """走っている音声化は `kind="radio"` として一覧に出る（% は None）。"""

    async def main() -> None:
        async def noop() -> None:
            return None

        task = asyncio.create_task(noop())
        papers._radio_jobs[("sai-paper", 9)] = task
        papers._radio_meta[("sai-paper", 9)] = {
            "title": "音声の題名",
            "started_at": "2026-09-27T12:00:00+09:00",
        }
        try:
            listed = papers.list_downloads(PROJECT)["jobs"]
            hit = [j for j in listed if j["kind"] == "radio"]
            assert len(hit) == 1, listed
            assert hit[0]["key"] == "radio:9", hit[0]
            assert hit[0]["title"] == "音声の題名", hit[0]
            assert hit[0]["paper_id"] == 9, hit[0]
            assert hit[0]["percent"] is None, hit[0]
            assert hit[0]["status"] == "processing", hit[0]
            # 別プロジェクトからは出ない
            assert [
                j for j in papers.list_downloads(PROJECT2)["jobs"] if j["kind"] == "radio"
            ] == []
        finally:
            papers._radio_jobs.clear()
            papers._radio_meta.clear()

    asyncio.run(main())


def test_list_includes_translate(cli: FakeRunJson) -> None:
    """要旨の訳は動いているあいだ `kind="translate"` として一覧に出る。"""
    cli.calls.clear()
    cli.value = {"id": 30, "abstract_ja": "訳"}
    seen_status: list[str] = []
    original = papers.translate

    def slow(project, paper_id):
        seen_status.append(papers.list_downloads(project)["jobs"][0]["status"])
        seen_status.append(str(papers.list_downloads(project)["jobs"][0]["percent"]))
        return original(project, paper_id)

    papers.translate = slow
    try:
        out = papers.translate_with_job(PROJECT, 30)
        assert out == {"id": 30, "abstract_ja": "訳"}, out
    finally:
        papers.translate = original
    assert seen_status == ["processing", "None"], seen_status
    listed = papers.list_downloads(PROJECT)["jobs"]
    assert listed[0]["kind"] == "translate", listed[0]
    assert listed[0]["status"] == "done", listed[0]
    assert listed[0]["paper_id"] == 30, listed[0]
    # 失敗しても一覧には残り、すぐに消える
    papers._download_jobs.clear()
    papers.cli.run_json = FakeRunJson()  # type: ignore[assignment]
    failing = FakeRunJson()
    failing.error = RuntimeError("途中 で失敗した")
    papers.cli.run_json = failing  # type: ignore[assignment]
    try:
        papers.translate_with_job(PROJECT, 31)
    except Exception:
        pass
    else:
        raise AssertionError("失敗は投げられる")
    listed = papers.list_downloads(PROJECT)["jobs"]
    assert [j["status"] for j in listed] == ["failed"], listed
    papers._download_jobs.clear()


# --- 住所の登録 ---


def test_routes() -> None:
    """新しい住所がすべて登録され、固定の住所が {paper_id:int} より前にある。"""
    got = []
    for route in app.routes:
        path = getattr(route, "path", "")
        for method in getattr(route, "methods", []) or []:
            got.append((method, path))
    want = [
        ("GET", "/api/projects/{name}/papers/manual/candidates"),
        ("POST", "/api/projects/{name}/papers/manual/import"),
        ("POST", "/api/projects/{name}/papers/manual/upload"),
        ("GET", "/api/projects/{name}/papers/{paper_id:int}/citations"),
        ("POST", "/api/projects/{name}/papers/{paper_id:int}/citations/extract"),
    ]
    for item in want:
        assert item in got, f"未登録: {item}"
    paths = [p for _m, p in got]
    int_path = "/api/projects/{name}/papers/{paper_id:int}"
    for fixed in (
        "/api/projects/{name}/papers/manual/candidates",
        "/api/projects/{name}/papers/manual/import",
        "/api/projects/{name}/papers/manual/upload",
    ):
        assert paths.index(fixed) < paths.index(int_path), fixed


def main() -> None:
    cli = FakeRunJson()
    papers.cli.run_json = cli
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    fake = FakeNotify()
    papers.notify.broadcast = fake.broadcast
    test_citations(cli)
    test_manual_candidates(cli)
    test_cite_extract_success(fake)
    test_cite_extract_docling_error(fake)
    test_cite_extract_failed(fake)
    test_cite_extract_conflict()
    test_manual_target_validation()
    test_pdf_name_validation()
    test_manual_import_file(fake)
    test_manual_import_stdin(fake)
    test_manual_import_bad_source(fake)
    test_manual_import_failed(fake)
    test_manual_import_conflict()
    test_upload_limits()
    test_upload_max_is_100mb()
    test_list_includes_radio(cli)
    test_list_includes_translate(cli)
    test_routes()
    print("test_papers_manual: すべて通りました")


main()
