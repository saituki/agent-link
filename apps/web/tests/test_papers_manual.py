"""引用一覧（citations）と引用の抜き出しのテスト（assert だけ・U13 で手動取り込みは消した）。

確かめること:
- `phone citations` の引数
- 引用の抜き出し（`phone citations-extract`）: 段 0→1→完了、docling_error の文、
  失敗は `failed`、同じ論文の 2 回目は 409
- 一覧に `kind` が付き、音声化と要旨の訳の仕事も出ること
- 新しい住所（keep・bundle・pending・jobs）が登録済みで、消した住所（manual/*）が無い

実行:
    cd <作業用コピー>
    PYTHONPATH=src .venv/bin/python tests/test_papers_manual.py
"""

from __future__ import annotations

import asyncio
import json
from types import SimpleNamespace

from fastapi import HTTPException

from sai_web import papers
from sai_web.main import app

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
        ("POST", "/api/projects/{name}/papers/keep"),
        ("POST", "/api/projects/{name}/papers/bundle"),
        ("GET", "/api/projects/{name}/papers/pending"),
        ("GET", "/api/projects/{name}/papers/jobs/{key}"),
        ("GET", "/api/prep/download"),
        ("GET", "/api/projects/{name}/papers/{paper_id:int}/citations"),
        ("POST", "/api/projects/{name}/papers/{paper_id:int}/citations/extract"),
    ]
    for item in want:
        assert item in got, f"未登録: {item}"
    paths = [p for _m, p in got]
    int_path = "/api/projects/{name}/papers/{paper_id:int}"
    for fixed in (
        "/api/projects/{name}/papers/keep",
        "/api/projects/{name}/papers/bundle",
        "/api/projects/{name}/papers/pending",
        "/api/projects/{name}/papers/jobs/{key}",
    ):
        assert paths.index(fixed) < paths.index(int_path), fixed
    # 消した住所は残っていない（U13 §2）
    for gone in (
        ("/api/projects/{name}/papers/manual/candidates", "GET"),
        ("/api/projects/{name}/papers/manual/import", "POST"),
        ("/api/projects/{name}/papers/manual/upload", "POST"),
    ):
        assert gone not in got, f"消した住所が残っている: {gone}"


def main() -> None:
    cli = FakeRunJson()
    papers.cli.run_json = cli
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    fake = FakeNotify()
    papers.notify.broadcast = fake.broadcast
    test_citations(cli)
    test_cite_extract_success(fake)
    test_cite_extract_docling_error(fake)
    test_cite_extract_failed(fake)
    test_cite_extract_conflict()
    test_list_includes_radio(cli)
    test_list_includes_translate(cli)
    test_routes()
    print("test_papers_manual: すべて通りました")


main()
