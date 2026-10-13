"""Papers の Import・Library に入れる・Project の API の小さなテスト（assert だけ）。

確かめること:
- Import: `phone import list / add / remove` の引数と本文は標準入力、名前の規則と 1MB の検査
  （検査に通らない値は 400 で、CLI を 1 回も呼ばない）
- Library に入れる（U13 §2）: keep の依頼の検査（`_download_payload`）と、`sai-progress` の読み取り
- Project: `sai-sci project show` / `project mind`（U04 で `project memo` の住所は消えた・確かめは test_papers_mind_index.py）/ `attention resolve` / `attention list`
- 新しい住所（keep・bundle・pending・jobs）が登録済みで、固定の住所が
  `/papers/{paper_id:int}` に取られず、消した住所（download・manual/*）が無い

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
        self._stderr = stderr
        self.returncode = returncode
        self.killed = False
        self._hang = hang
        self.stdout = FakeStream(stdout)
        self.stderr = FakeStream(stderr)

    async def communicate(self):
        if self._hang:
            await asyncio.sleep(3600)
        return self._stdout, self._stderr

    async def wait(self) -> int:
        if self._hang and not self.killed:
            await asyncio.sleep(3600)
        return self.returncode

    def kill(self) -> None:
        self.killed = True
        self._hang = False


def fake_exec(results: dict[str, FakeProc | Exception]):
    """`paper download` / `paper after-download` ごとに返す偽物を作る係。"""
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
    """一覧は `import list`（cwd と SAI_PROJECT_ROOT はそのプロジェクトの root）。"""
    cli.calls.clear()
    cli.value = {"files": [], "entries": []}
    out = papers.import_list(PROJECT)
    assert out == {"files": [], "entries": []}, out
    call = cli.last
    assert call["args"] == ["sai-paper", "import", "list"], call["args"]
    assert call["cwd"] == "/root/sai-paper", call["cwd"]
    assert call["env"]["SAI_PROJECT_ROOT"] == "/root/sai-paper", call["env"]
    assert call["env"]["SAI_ORIGIN"] == "phone", call["env"]


def test_import_add(cli: FakeRunJson) -> None:
    """保存は `--name` を付けて本文を標準入力へ渡す（`import add`）。"""
    cli.calls.clear()
    cli.value = {"name": "refs.bib", "count": 12, "path": "/x/refs.bib"}
    out = papers.import_add(PROJECT, "refs.bib", "@article{a, title={x}}")
    assert out["count"] == 12, out
    call = cli.last
    assert call["args"] == [
        "sai-paper",
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
    """削除は `import remove --name …`（名前は同じ規則で検査）。"""
    cli.calls.clear()
    cli.value = {"name": "refs.bib", "removed": True}
    assert papers.import_remove(PROJECT, "refs.bib") == {"name": "refs.bib", "removed": True}
    assert cli.last["args"] == [
        "sai-paper",
        "import",
        "remove",
        "--name",
        "refs.bib",
    ], cli.last["args"]
    cli.calls.clear()
    expect(400, lambda: papers.import_remove(PROJECT, "refs.txt"))
    expect(400, lambda: papers.import_remove(PROJECT, "../../etc/passwd"))
    assert cli.calls == [], cli.calls


# --- Library に入れる（U13 §2。かつての Download と同じ検査） ---


def test_download_payload_validation() -> None:
    """keep の依頼の形（log / import / cite のどれか 1 つ）と題名を確かめる。"""
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
    # position は 0 始まり（検索履歴の先頭の論文が 0）
    key, args, _title = papers._download_payload({"log_id": 3, "position": 0})
    assert key == "log:3:0", key
    assert args == ["--log", "3", "--position", "0"], args

    for bad in (
        {},
        {"title": "x"},
        {"log_id": 3, "position": 2, "import_id": "DOI:1"},
        {"log_id": 3, "position": 2, "cite_paper_id": 12, "ref_no": 3},
        {"log_id": 3},
        {"position": 2},
        {"log_id": 0, "position": 2},
        {"log_id": 3, "position": -1},
        {"log_id": 3, "position": "0"},
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
    """現況は `sai-sci project show --json`（待ち 30 秒・U14）。"""
    cli.calls.clear()
    cli.value = {"state": "# 現況", "mind": "", "brief": {}, "attention": []}
    out = papers.project_view(PROJECT)
    assert out["state"] == "# 現況", out
    assert cli.last["args"] == ["sai-sci", "project", "show", "--json"], cli.last["args"]
    assert cli.last["timeout"] == 30.0, cli.last["timeout"]


def test_attention_resolve(cli: FakeRunJson) -> None:
    """要対応の 1 件を「対応済み」にする（key は 1〜300 文字の 1 行・U14）。"""
    cli.calls.clear()
    cli.value = {"ok": True, "key": "question:41:対象は?"}
    out = papers.attention_resolve(PROJECT, "question:41:対象は?")
    assert out == {"ok": True, "key": "question:41:対象は?"}, out
    assert cli.last["args"] == [
        "sai-sci",
        "attention",
        "resolve",
        "question:41:対象は?",
        "--json",
    ], cli.last["args"]
    cli.calls.clear()
    for bad in ("", "x" * 301, "a\nb", None, 12):
        expect(400, lambda bad=bad: papers.attention_resolve(PROJECT, bad))
    assert cli.calls == [], cli.calls


def test_attention_list(cli: FakeRunJson) -> None:
    """ホーム用の要対応一覧は `sai-sci attention list --json`（待ち 15 秒・U14）。"""
    cli.calls.clear()
    cli.value = [{"key": "k", "kind": "question", "title": "題"}]
    assert papers.attention_list(PROJECT) == [{"key": "k", "kind": "question", "title": "題"}]
    assert cli.last["args"] == ["sai-sci", "attention", "list", "--json"], cli.last["args"]
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
        ("POST", "/api/projects/{name}/papers/keep"),
        ("POST", "/api/projects/{name}/papers/bundle"),
        ("GET", "/api/projects/{name}/papers/pending"),
        ("GET", "/api/projects/{name}/papers/jobs/{key}"),
        ("GET", "/api/projects/{name}/papers/downloads"),
        ("GET", "/api/prep/download"),
        ("GET", "/api/projects/{name}/papers/{paper_id:int}/citations"),
        ("POST", "/api/projects/{name}/papers/{paper_id:int}/citations/extract"),
        ("GET", "/api/projects/{name}/papers/project"),
        ("PUT", "/api/projects/{name}/papers/project/mind"),
        ("POST", "/api/projects/{name}/papers/attention/resolve"),
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
    # 消した住所は残っていない（U13 §2）
    for gone in (
        ("POST", "/api/projects/{name}/papers/download"),
        ("GET", "/api/projects/{name}/papers/manual/candidates"),
        ("POST", "/api/projects/{name}/papers/manual/import"),
        ("POST", "/api/projects/{name}/papers/manual/upload"),
        ("POST", "/api/projects/{name}/papers/project/memo"),
    ):
        assert gone not in got, f"消した住所が残っている: {gone}"


def main() -> None:
    cli = FakeRunJson()
    papers.cli.run_json = cli
    # 引数を文字列として確かめるため、CLI の場所を "sai-paper" / "sai-sci" に固定する
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper", sai_sci_bin="sai-sci")
    fake = FakeNotify()
    papers.notify = fake  # 通知は記録するだけの偽物（実機へは送らない）
    test_import_list(cli)
    test_import_add(cli)
    test_import_remove(cli)
    test_download_payload_validation()
    test_apply_progress_line()
    test_apply_progress_line_keeps_50()
    test_download_expired()
    test_project_view(cli)
    test_attention_resolve(cli)
    test_attention_list(cli)
    test_routes()
    print("test_papers_import_api: すべて通りました")


main()
