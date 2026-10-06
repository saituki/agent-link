"""Papers の Import・Download・Project の API の小さなテスト（assert だけ）。

確かめること:
- Import: `phone import list / add / remove` の引数と本文は標準入力、名前の規則と 1MB の検査
  （検査に通らない値は 400 で、CLI を 1 回も呼ばない）
- Download の背景の仕事（`asyncio.create_subprocess_exec` を偽物にする・test_radio_job.py が手本）:
    - 成功 → `processing` → `after-download` → `done` と通知 1 回
    - `exists` → `done` で通知なし
    - 失敗 → `failed` と通知
    - 時間切れ → `failed`
    - 同じキーの 2 回目は 409
- Project: `phone project` / `phone mind-append`（改行は 400）/ `phone attention-resolve`
- 新しい住所が登録済みで、固定の住所が `/papers/{paper_id:int}` に取られない

実行:
    cd <作業用コピー>
    PYTHONPATH=src .venv/bin/python tests/test_papers_import_api.py
"""

from __future__ import annotations

import asyncio
import json
import re
import tempfile
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
    """await する版（Download の依頼は `async def`）。"""
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


class FakeStream:
    """`proc.stderr` / `proc.stdout` の偽物（`readline` で 1 行ずつ、`read` で全部）。"""

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
    """`asyncio.create_subprocess_exec` の偽物（communicate / readline / kill / returncode）。"""

    def __init__(
        self,
        stdout: bytes,
        returncode: int = 0,
        hang: bool = False,
        stderr: bytes = b"",
    ) -> None:
        self._stdout = stdout
        self.returncode = returncode
        self.killed = False
        self._hang = hang
        self.stdout = FakeStream(stdout)
        self.stderr = FakeStream(stderr)

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


def fake_exec(results: dict[str, FakeProc | Exception]):
    """`phone download` / `phone after-download` ごとに返す偽物を作る係。"""
    seen: list[list[str]] = []

    async def create(*args, **kwargs):
        argv = [str(a) for a in args]
        seen.append(argv)
        hit = results.get(" ".join(argv[1:]))
        if hit is None:
            return FakeProc(b"{}", 0)
        if isinstance(hit, Exception):
            raise hit
        return hit

    return create, seen


# --- Import ---


def test_import_list(cli: FakeRunJson) -> None:
    """一覧は `phone import list`（cwd と SAI_PAPER_ROOT はそのプロジェクトの root）。"""
    cli.calls.clear()
    cli.value = {"files": [], "entries": []}
    out = papers.import_list(PROJECT)
    assert out == {"files": [], "entries": []}, out
    call = cli.last
    assert call["args"] == ["sai-paper", "phone", "import", "list"], call["args"]
    assert call["cwd"] == "/root/sai-paper", call["cwd"]
    assert call["env"]["SAI_PAPER_ROOT"] == "/root/sai-paper", call["env"]


def test_import_add(cli: FakeRunJson) -> None:
    """保存は `--name` を付けて本文を標準入力へ渡す。"""
    cli.calls.clear()
    cli.value = {"name": "refs.bib", "count": 12, "path": "/x/refs.bib"}
    out = papers.import_add(PROJECT, "refs.bib", "@article{a, title={x}}")
    assert out["count"] == 12, out
    call = cli.last
    assert call["args"] == [
        "sai-paper",
        "phone",
        "import",
        "add",
        "--name",
        "refs.bib",
    ], call["args"]
    assert call["input_text"] == "@article{a, title={x}}", call["input_text"]

    # 名前の規則
    cli.calls.clear()
    expect(400, lambda: papers.import_add(PROJECT, "refs.txt", "@article{a}"))
    expect(400, lambda: papers.import_add(PROJECT, "../x.bib", "@article{a}"))
    expect(400, lambda: papers.import_add(PROJECT, "a/b.bib", "@article{a}"))
    expect(400, lambda: papers.import_add(PROJECT, "x" * 100 + ".bib", "@article{a}"))
    expect(400, lambda: papers.import_add(PROJECT, 123, "@article{a}"))
    # 中身（文字列・空白以外あり・1MB まで）
    expect(400, lambda: papers.import_add(PROJECT, "a.bib", ""))
    expect(400, lambda: papers.import_add(PROJECT, "a.bib", "   \n"))
    expect(400, lambda: papers.import_add(PROJECT, "a.bib", None))
    expect(400, lambda: papers.import_add(PROJECT, "a.bib", "x" * (1024 * 1024 + 1)))
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"

    # 名前の空文字は `phone-<YYYYMMDD-HHMMSS>.bib` にする
    cli.calls.clear()
    papers.import_add(PROJECT, "", "@article{a}")
    name = cli.last["args"][-1]
    assert re.match(r"^phone-\d{8}-\d{6}\.bib$", name), name
    cli.calls.clear()
    papers.import_add(PROJECT, None, "@article{a}")
    assert cli.last["args"][-1].startswith("phone-"), cli.last["args"]


def test_import_remove(cli: FakeRunJson) -> None:
    """削除は `phone import remove --name …`（名前は同じ規則で検査）。"""
    cli.calls.clear()
    cli.value = {"name": "refs.bib", "removed": True}
    assert papers.import_remove(PROJECT, "refs.bib") == {"name": "refs.bib", "removed": True}
    assert cli.last["args"] == [
        "sai-paper",
        "phone",
        "import",
        "remove",
        "--name",
        "refs.bib",
    ], cli.last["args"]
    cli.calls.clear()
    expect(400, lambda: papers.import_remove(PROJECT, "refs.txt"))
    expect(400, lambda: papers.import_remove(PROJECT, "../../etc/passwd"))
    assert cli.calls == [], cli.calls


# --- Download ---


def test_download_payload_validation() -> None:
    """依頼の形（log / import のどちらか一方）と題名を確かめる。"""
    key, args, title = papers._download_payload({"log_id": 3, "position": 2})
    assert key == "log:3:2", key
    assert args == ["--log", "3", "--position", "2"], args
    assert title == "論文", title
    key, args, title = papers._download_payload(
        {"import_id": "DOI:10.1/x", "title": "題名"}
    )
    assert key == "import:DOI:10.1/x", key
    assert args == ["--import-id", "DOI:10.1/x"], args
    assert title == "題名", title
    # 引用の指定（U05 §2）
    key, args, title = papers._download_payload(
        {"cite_paper_id": 12, "ref_no": 3, "title": "引用の題名"}
    )
    assert key == "cite:12:3", key
    assert args == ["--cite", "12", "--ref", "3"], args
    assert title == "引用の題名", title

    for bad in (
        {},
        {"title": "x"},
        {"log_id": 3, "position": 2, "import_id": "DOI:1"},
        {"log_id": 3, "position": 2, "cite_paper_id": 12, "ref_no": 3},
        {"log_id": 3},
        {"position": 2},
        {"log_id": 0, "position": 2},
        {"log_id": 3, "position": -1},
        {"log_id": "3", "position": 2},
        {"log_id": 3, "position": True},
        {"import_id": ""},
        {"import_id": "x" * 301},
        {"import_id": "DOI:1\nDOI:2"},
        {"import_id": 12},
        {"cite_paper_id": 12},
        {"ref_no": 3},
        {"cite_paper_id": 0, "ref_no": 3},
        {"cite_paper_id": 12, "ref_no": -1},
        {"cite_paper_id": "12", "ref_no": 3},
        {"title": 12},
        "not a dict",
    ):
        expect(400, lambda bad=bad: papers._download_payload(bad))


def test_download_success(fake: FakeNotify) -> None:
    """成功: `processing` → `after-download` → `done` と通知 1 回。"""
    fake.calls.clear()
    create, seen = fake_exec(
        {
            "phone download --log 3 --position 2": FakeProc(
                json.dumps({"status": "downloaded", "paper_id": 41}).encode()
            ),
            "phone after-download 41": FakeProc(
                json.dumps({"paper_id": 41, "messages": []}).encode()
            ),
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        res = await papers.request_download(
            PROJECT, {"log_id": 3, "position": 2, "title": "題名"}
        )
        assert res == {"key": "log:3:2", "status": "downloading"}, res
        # 202 の直後は downloading にして `processing` を通る
        job = papers._download_jobs[("sai-paper", "log:3:2")]
        assert job["status"] == "downloading", job
        for _ in range(10):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "log:3:2")]
        assert job["status"] == "done", job
        assert job["paper_id"] == 41, job
        assert job["error"] is None, job
        assert job["finished_at"], job
        # 一覧には内部のフィールドを出さない
        listed = papers.list_downloads(PROJECT)["jobs"]
        assert len(listed) == 1, listed
        assert set(listed[0]) == {
            "key",
            "kind",
            "title",
            "status",
            "paper_id",
            "error",
            "started_at",
            "finished_at",
            "steps",
            "step",
            "percent",
            "messages",
        }, listed[0]
        assert listed[0]["key"] == "log:3:2", listed[0]
        assert listed[0]["kind"] == "download", listed[0]
        # 別プロジェクトからは出ない
        assert papers.list_downloads(PROJECT2)["jobs"] == []

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert fake.calls == [("📄 論文を保存しました", "題名", "/#/paper/sai-paper/41")], fake.calls
    assert seen[0][1:] == ["phone", "download", "--log", "3", "--position", "2"], seen
    assert seen[1][1:] == ["phone", "after-download", "41"], seen


def test_download_exists(fake: FakeNotify) -> None:
    """`exists`（既に Library に有る）→ `done` で通知なし。"""
    fake.calls.clear()
    create, seen = fake_exec(
        {
            "phone download --import-id DOI:1": FakeProc(
                json.dumps({"status": "exists", "paper_id": 37}).encode()
            )
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        await papers.request_download(PROJECT, {"import_id": "DOI:1", "title": "題名"})
        for _ in range(10):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "import:DOI:1")]
        assert job["status"] == "done", job
        assert job["paper_id"] == 37, job

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert fake.calls == [], f"exists で通知してはいけない: {fake.calls}"
    assert len(seen) == 1, seen  # after-download は走らせない


def test_download_failed(fake: FakeNotify) -> None:
    """失敗 → `failed` と通知（理由は CLI の `error`）。"""
    fake.calls.clear()
    create, _seen = fake_exec(
        {
            "phone download --log 1 --position 1": FakeProc(
                json.dumps({"error": "PDF を取れませんでした"}).encode(), 1
            )
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        await papers.request_download(PROJECT, {"log_id": 1, "position": 1, "title": "題名"})
        for _ in range(10):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "log:1:1")]
        assert job["status"] == "failed", job
        assert job["error"] == "PDF を取れませんでした", job

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert fake.calls == [
        ("論文を保存できませんでした", "題名: PDF を取れませんでした", "/#/papers/library")
    ], fake.calls


def test_download_unreadable(fake: FakeNotify) -> None:
    """読めない失敗 → 「原因不明のエラー」で `failed`。"""
    fake.calls.clear()
    create, _seen = fake_exec(
        {"phone download --log 2 --position 1": FakeProc(b"not json", 1)}
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        await papers.request_download(PROJECT, {"log_id": 2, "position": 1, "title": "題名"})
        for _ in range(10):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "log:2:1")]
        assert job["status"] == "failed", job
        assert job["error"] == "原因不明のエラー", job

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert len(fake.calls) == 1 and fake.calls[0][1].endswith("原因不明のエラー"), fake.calls


def test_download_not_found_bin(fake: FakeNotify) -> None:
    """sai-paper が無い（起動できない）→ `failed` と通知。"""
    fake.calls.clear()
    create, _seen = fake_exec(
        {"phone download --log 4 --position 1": FileNotFoundError("sai-paper")}
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        await papers.request_download(PROJECT, {"log_id": 4, "position": 1, "title": "題名"})
        for _ in range(10):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "log:4:1")]
        assert job["status"] == "failed", job
        assert "sai-paper が見つかりません" in job["error"], job

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert len(fake.calls) == 1, fake.calls


def test_download_timeout(fake: FakeNotify) -> None:
    """時間切れ → kill して `failed`。"""
    fake.calls.clear()
    proc = FakeProc(b"", 0, hang=True)
    create, _seen = fake_exec({"phone download --log 5 --position 1": proc})
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    # 待ちは一瞬に縮める（テストが待たないように）
    original_timeout = papers.DOWNLOAD_TIMEOUT
    papers.DOWNLOAD_TIMEOUT = 0.05

    async def main() -> None:
        await papers.request_download(PROJECT, {"log_id": 5, "position": 1, "title": "題名"})
        for _ in range(10):
            await asyncio.sleep(0.02)
        job = papers._download_jobs[("sai-paper", "log:5:1")]
        assert job["status"] == "failed", job
        assert "時間内" in job["error"], job

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers.DOWNLOAD_TIMEOUT = original_timeout
        papers._download_jobs.clear()
    assert proc.killed, "時間切れなら kill する"
    assert len(fake.calls) == 1, fake.calls


def test_download_after_failed(fake: FakeNotify) -> None:
    """後処理の失敗でも論文は入っているので `done`（`error` に理由だけ入れる）。"""
    fake.calls.clear()
    create, seen = fake_exec(
        {
            "phone download --log 6 --position 1": FakeProc(
                json.dumps({"status": "downloaded", "paper_id": 42}).encode()
            ),
            "phone after-download 42": FakeProc(json.dumps({"error": "PDF が紐付いていません"}).encode(), 1),
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        await papers.request_download(PROJECT, {"log_id": 6, "position": 1, "title": "題名"})
        for _ in range(10):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "log:6:1")]
        assert job["status"] == "done", job
        assert job["error"] == "PDF が紐付いていません", job

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert len(fake.calls) == 1, fake.calls  # 「保存しました」の通知だけ
    assert seen[1][1:] == ["phone", "after-download", "42"], seen


def test_apply_progress_line() -> None:
    """`sai-progress` の行は段と % とログに、それ以外の行は False（AC4）。"""
    job = {
        "steps": list(papers.DOWNLOAD_STEPS),
        "step": 0,
        "percent": 0,
        "messages": [],
    }
    # step は 0 → 33 → 66 → 100
    for n, percent in ((1, 33), (2, 66), (3, 100)):
        assert papers.apply_progress_line(job, f"sai-progress step={n}") is True
        assert job["step"] == n, job
        assert job["percent"] == percent, job
    # 0 に戻しても 0%
    papers.apply_progress_line(job, "sai-progress step=0")
    assert (job["step"], job["percent"]) == (0, 0), job

    # ログの行（前後の空白は落とす）
    assert papers.apply_progress_line(job, "sai-progress msg=図を 3 件抽出しました") is True
    assert job["messages"] == ["図を 3 件抽出しました"], job
    # 別の行は進捗ではない（呼び出し側がログへ出す）
    assert papers.apply_progress_line(job, "警告: 何かがおかしい") is False
    # 読めない step は進捗として扱わない
    assert papers.apply_progress_line(job, "sai-progress step=abc") is False
    assert job["step"] == 0, job
    # 段の範囲は収める（越えたら端）
    assert papers.apply_progress_line(job, "sai-progress step=99") is True
    assert (job["step"], job["percent"]) == (3, 100), job


def test_apply_progress_line_keeps_50() -> None:
    """ログは 50 行まで。古いものから捨てる。"""
    job = {"steps": list(papers.DOWNLOAD_STEPS), "step": 0, "percent": 0, "messages": []}
    for i in range(60):
        papers.apply_progress_line(job, f"sai-progress msg={i}")
    assert len(job["messages"]) == 50, len(job["messages"])
    assert job["messages"][0] == "10", job["messages"][0]
    assert job["messages"][-1] == "59", job["messages"][-1]


def test_download_progress_lines(fake: FakeNotify) -> None:
    """`after-download` の標準エラーから段・%・ログが入る（AC3・AC4）。"""
    fake.calls.clear()
    create, _seen = fake_exec(
        {
            "phone download --log 8 --position 1": FakeProc(
                json.dumps({"status": "downloaded", "paper_id": 55}).encode()
            ),
            "phone after-download 55": FakeProc(
                json.dumps({"paper_id": 55, "messages": ["done"]}).encode(),
                stderr=(
                    "sai-progress step=1\n"
                    "sai-progress msg=図を 3 件抽出しました\n"
                    "sai-progress step=2\n"
                    "sai-progress msg=PDF翻訳完了: /x/y.md\n"
                ).encode(),
            ),
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        await papers.request_download(PROJECT, {"log_id": 8, "position": 1, "title": "題名"})
        for _ in range(30):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "log:8:1")]
        assert job["status"] == "done", job
        assert job["step"] == 3, job  # 終わったら 3
        assert job["percent"] == 100, job
        assert job["messages"] == [
            "図を 3 件抽出しました",
            "PDF翻訳完了: /x/y.md",
        ], job["messages"]
        listed = papers.list_downloads(PROJECT)["jobs"]
        assert listed[0]["steps"] == ["PDF取得", "図抽出", "翻訳"], listed[0]
        assert listed[0]["percent"] == 100, listed[0]
        assert listed[0]["messages"] == job["messages"], listed[0]

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()


def test_download_after_keeps_step_on_failure(fake: FakeNotify) -> None:
    """後処理の失敗は `done` だが 100% にする。途中までは % を残す。"""
    fake.calls.clear()
    create, _seen = fake_exec(
        {
            "phone download --log 10 --position 1": FakeProc(
                json.dumps({"status": "downloaded", "paper_id": 56}).encode()
            ),
            "phone after-download 56": FakeProc(
                json.dumps({"error": "翻訳に失敗しました"}).encode(),
                1,
                stderr=b"sai-progress step=2\n",
            ),
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        await papers.request_download(PROJECT, {"log_id": 10, "position": 1, "title": "題名"})
        for _ in range(30):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "log:10:1")]
        assert job["status"] == "done", job
        assert job["error"] == "翻訳に失敗しました", job
        assert job["percent"] == 100, job  # 終わったので 100%

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()


def test_download_failure_keeps_percent(fake: FakeNotify) -> None:
    """`failed`（PDF が取れない）なら 0% のまま残す。"""
    fake.calls.clear()
    create, _seen = fake_exec(
        {
            "phone download --log 11 --position 1": FakeProc(
                json.dumps({"error": "PDF を取れませんでした"}).encode(), 1
            )
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        await papers.request_download(PROJECT, {"log_id": 11, "position": 1, "title": "題名"})
        for _ in range(10):
            await asyncio.sleep(0)
        job = papers._download_jobs[("sai-paper", "log:11:1")]
        assert job["status"] == "failed", job
        assert job["percent"] == 0, job
        assert job["step"] == 0, job

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()


def test_download_after_timeout(fake: FakeNotify) -> None:
    """`after-download` が時間切れ → kill して `done`（論文は入っているため）。"""
    fake.calls.clear()
    proc = FakeProc(b"", 0, hang=True)
    create, _seen = fake_exec(
        {
            "phone download --log 12 --position 1": FakeProc(
                json.dumps({"status": "downloaded", "paper_id": 57}).encode()
            ),
            "phone after-download 57": proc,
        }
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    original_timeout = papers.AFTER_DOWNLOAD_TIMEOUT
    papers.AFTER_DOWNLOAD_TIMEOUT = 0.05

    async def main() -> None:
        await papers.request_download(PROJECT, {"log_id": 12, "position": 1, "title": "題名"})
        for _ in range(20):
            await asyncio.sleep(0.02)
        job = papers._download_jobs[("sai-paper", "log:12:1")]
        assert job["status"] == "done", job
        assert "時間内" in job["error"], job

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers.AFTER_DOWNLOAD_TIMEOUT = original_timeout
        papers._download_jobs.clear()
    assert proc.killed, "時間切れなら kill する"


def test_download_conflict() -> None:
    """同じキーの 2 回目は 409（進行中の間は受け付けない）。"""
    create, _seen = fake_exec({"phone download --log 7 --position 1": FakeProc(b"{}", 0, hang=True)})
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    original_timeout = papers.DOWNLOAD_TIMEOUT
    papers.DOWNLOAD_TIMEOUT = 30.0

    async def main() -> None:
        await papers.request_download(PROJECT, {"log_id": 7, "position": 1, "title": "題名"})
        await asyncio.sleep(0)
        e = await expect_async(
            409, papers.request_download(PROJECT, {"log_id": 7, "position": 1})
        )
        assert "ダウンロード中" in str(e.detail), e.detail
        # 別プロジェクトの同じ番号は別扱い
        await papers.request_download(PROJECT2, {"log_id": 7, "position": 1, "title": "他"})
        # 検査に通らない依頼は 409 ではなく 400
        await expect_async(400, papers.request_download(PROJECT, {}))

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers.DOWNLOAD_TIMEOUT = original_timeout
        papers._download_jobs.clear()


def test_download_expired() -> None:
    """30 分を過ぎた仕事は一覧から出て、消える。"""
    papers._download_jobs.clear()
    job = {
        "key": "log:9:1",
        "title": "題名",
        "status": "done",
        "paper_id": 1,
        "error": None,
        "started_at": "2026-09-26T12:00:00+09:00",
        "finished_at": "2026-09-26T12:05:00+09:00",
        "_done_at": 0.0,
    }
    papers._download_jobs[("sai-paper", "log:9:1")] = job
    assert papers.list_downloads(PROJECT)["jobs"] == [], papers.list_downloads(PROJECT)
    assert ("sai-paper", "log:9:1") not in papers._download_jobs
    # まだ新しいものは残る
    job2 = {**job, "_done_at": papers.time.monotonic()}
    papers._download_jobs[("sai-paper", "log:9:2")] = {**job2, "key": "log:9:2"}
    listed = papers.list_downloads(PROJECT)["jobs"]
    assert [j["key"] for j in listed] == ["log:9:2"], listed
    papers._download_jobs.clear()


# --- Project ---


def test_project_view(cli: FakeRunJson) -> None:
    """現況は `phone project`（待ち 30 秒）。"""
    cli.calls.clear()
    cli.value = {"state": "# 現況", "mind": "", "brief": {}, "attention": []}
    out = papers.project_view(PROJECT)
    assert out["state"] == "# 現況", out
    assert cli.last["args"] == ["sai-paper", "phone", "project"], cli.last["args"]
    assert cli.last["timeout"] == 30.0, cli.last["timeout"]


def test_mind_append(cli: FakeRunJson) -> None:
    """メモは 1 行・2000 文字まで（標準入力）。"""
    cli.calls.clear()
    cli.value = {"ok": True, "line": "- 09/26 12:00 メモ"}
    out = papers.mind_append(PROJECT, "メモ")
    assert out == {"ok": True, "line": "- 09/26 12:00 メモ"}, out
    assert cli.last["args"] == ["sai-paper", "phone", "mind-append"], cli.last["args"]
    assert cli.last["input_text"] == "メモ", cli.last["input_text"]
    cli.calls.clear()
    for bad in ("", "x" * 2001, "a\nb", "a\rb", None, 12):
        expect(400, lambda bad=bad: papers.mind_append(PROJECT, bad))
    assert cli.calls == [], cli.calls


def test_attention_resolve(cli: FakeRunJson) -> None:
    """要対応の 1 件を「対応済み」にする（key は 1〜300 文字の 1 行）。"""
    cli.calls.clear()
    cli.value = {"ok": True, "key": "question:41:対象は?"}
    out = papers.attention_resolve(PROJECT, "question:41:対象は?")
    assert out == {"ok": True, "key": "question:41:対象は?"}, out
    assert cli.last["args"] == [
        "sai-paper",
        "phone",
        "attention-resolve",
        "question:41:対象は?",
    ], cli.last["args"]
    cli.calls.clear()
    for bad in ("", "x" * 301, "a\nb", None, 12):
        expect(400, lambda bad=bad: papers.attention_resolve(PROJECT, bad))
    assert cli.calls == [], cli.calls


def test_attention_list(cli: FakeRunJson) -> None:
    """ホーム用の要対応一覧は `attention --json`（待ち 15 秒）。"""
    cli.calls.clear()
    cli.value = [{"key": "k", "kind": "question", "title": "題"}]
    assert papers.attention_list(PROJECT) == [{"key": "k", "kind": "question", "title": "題"}]
    assert cli.last["args"] == ["sai-paper", "attention", "--json"], cli.last["args"]
    assert cli.last["timeout"] == 15.0, cli.last["timeout"]
    # 配列でなければ空の並びにする
    cli.value = {"error": "x"}
    assert papers.attention_list(PROJECT) == []


# --- 住所の登録（AC1） ---


def test_routes() -> None:
    """C-A2 の住所がすべて登録され、固定の住所が {paper_id:int} より前にある。"""
    got = []
    for route in app.routes:
        path = getattr(route, "path", "")
        for method in getattr(route, "methods", []) or []:
            got.append((method, path))
    want = [
        ("GET", "/api/projects/{name}/papers/import"),
        ("POST", "/api/projects/{name}/papers/import"),
        ("DELETE", "/api/projects/{name}/papers/import/{file_name}"),
        ("POST", "/api/projects/{name}/papers/download"),
        ("GET", "/api/projects/{name}/papers/downloads"),
        ("GET", "/api/projects/{name}/papers/manual/candidates"),
        ("POST", "/api/projects/{name}/papers/manual/import"),
        ("POST", "/api/projects/{name}/papers/manual/upload"),
        ("GET", "/api/projects/{name}/papers/{paper_id:int}/citations"),
        ("POST", "/api/projects/{name}/papers/{paper_id:int}/citations/extract"),
        ("GET", "/api/projects/{name}/papers/project"),
        ("POST", "/api/projects/{name}/papers/project/memo"),
        ("POST", "/api/projects/{name}/papers/attention/resolve"),
    ]
    for item in want:
        assert item in got, f"未登録: {item}"
    paths = [p for _m, p in got]
    int_path = "/api/projects/{name}/papers/{paper_id:int}"
    for fixed in (
        "/api/projects/{name}/papers/manual/candidates",
        "/api/projects/{name}/papers/manual/import",
        "/api/projects/{name}/papers/manual/upload",
        "/api/projects/{name}/papers/downloads",
    ):
        assert paths.index(fixed) < paths.index(int_path), (
            f"固定の住所は {{paper_id:int}} より先に登録する: {fixed}"
        )
    for fixed in (
        "/api/projects/{name}/papers/import",
        "/api/projects/{name}/papers/downloads",
        "/api/projects/{name}/papers/project",
        "/api/projects/{name}/papers/attention/resolve",
    ):
        assert paths.index(fixed) < paths.index(int_path), (
            f"{fixed} は {int_path} より先に登録する"
        )


def main() -> None:
    cli = FakeRunJson()
    papers.cli.run_json = cli
    # 引数を文字列として確かめるため、CLI の場所を "sai-paper" に固定する
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    fake = FakeNotify()
    papers.notify = fake  # 通知は記録するだけの偽物（実機へは送らない）
    test_import_list(cli)
    test_import_add(cli)
    test_import_remove(cli)
    test_download_payload_validation()
    test_download_success(fake)
    test_download_exists(fake)
    test_download_failed(fake)
    test_download_unreadable(fake)
    test_download_not_found_bin(fake)
    test_download_timeout(fake)
    test_download_after_failed(fake)
    test_apply_progress_line()
    test_apply_progress_line_keeps_50()
    test_download_progress_lines(fake)
    test_download_after_keeps_step_on_failure(fake)
    test_download_failure_keeps_percent(fake)
    test_download_after_timeout(fake)
    test_download_conflict()
    test_download_expired()
    test_project_view(cli)
    test_mind_append(cli)
    test_attention_resolve(cli)
    test_attention_list(cli)
    test_routes()
    print("test_papers_import_api: すべて通りました")


main()
