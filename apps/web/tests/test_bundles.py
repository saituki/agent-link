"""前処理の束（zip）の受け口のテスト（assert だけ・実装計画 U13 §1〜§4）。

確かめること（AC1・AC2・AC3）:
- 正しい zip: 202 が返り、`inbox/<uuid>/` に展開され、
  `sai-paper paper import-bundle --dir <そこ> --json` が呼ばれ、終わるとフォルダが消え、
  job が `done`（`paper_id`・`figures`・`translated`）になる
- 危ない zip（`../x` の名前・シンボリックリンク・manifest.json 無し・`%PDF-` で始まらない PDF・
  format の違い）は CLI を呼ばずに 400
- `Content-Type: application/pdf` は 415、300MB 超・Content-Length 無しは 413
- 個人のトークン（Bearer）で `GET …/papers/pending` と `POST …/papers/bundle` が通る（多人数モード）
- `POST …/papers/keep` が `sai-paper paper keep --import-id … --json` を呼ぶ
- 消した住所（download・manual/*）は無い

実行:
    cd <作業用コピー>
    PYTHONPATH=src .venv/bin/python tests/test_bundles.py
"""

from __future__ import annotations

import asyncio
import io
import json
import os
import stat
import sys
import tempfile
import zipfile
from pathlib import Path
from types import SimpleNamespace

# import より前に一時フォルダを決める（config.DB_PATH は import 時に決まるため）
_TMP = Path(tempfile.mkdtemp(prefix="sai-web-bundles-"))
os.environ["SAI_HOME"] = str(_TMP / "sai-home")
os.environ["SAI_WEB_USERS_ROOT"] = str(_TMP / "users")
os.environ["SAI_WEB_COOKIE_SECURE"] = "0"
os.environ.pop("SAI_WEB_TOKEN", None)
os.environ.pop("SAI_WEB_RUNS_DIR", None)

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from fastapi import HTTPException  # noqa: E402

from sai_web import auth, bundles, papers, registry, store  # noqa: E402
from sai_web.main import app  # noqa: E402

PROJECT = {"name": "sai-paper", "root": "/root/sai-paper", "root_name": "sai-paper"}
HOST = "testserver"


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


# --- zip の組み立て ---


def manifest(target=None, pdf="paper.pdf", fmt="sai-prep-bundle", translation=None):
    """C12 の形の manifest.json を作る。"""
    return {
        "format": fmt,
        "version": 1,
        "target": target if target is not None else {"bibtex": "@article{a, title={A Paper}, year={2020}}"},
        "pdf": pdf,
        "pdf_sha256": "0" * 64,
        "docling": {"md": "docling/paper.md", "json": "docling/paper.json", "dir": "docling"},
        "translation": translation,
        "tools": {"sai_prep": "0.1.0", "docling": "2.0", "pdf2zh": "1.9"},
        "created_at": "2026-10-08T00:00:00Z",
    }


def build_zip(
    manifest_obj=None,
    pdf=b"%PDF-1.7\nhello",
    include_manifest=True,
    extra=None,
    symlink=False,
    translation=None,
) -> bytes:
    """前処理の束の zip を作る（壊れた形も作れる）。"""
    files: dict[str, bytes] = {
        "paper.pdf": pdf,
        "docling/paper.md": b"# title\n",
        "docling/paper.json": b"{}",
    }
    if include_manifest:
        obj = manifest_obj if manifest_obj is not None else manifest(translation=translation)
        files["manifest.json"] = json.dumps(obj, ensure_ascii=False).encode("utf-8")
    if translation is not None:
        files["translated/paper-mono.pdf"] = b"%PDF-1.7\nmono"
        files["translated/paper-dual.pdf"] = b"%PDF-1.7\ndual"
    if extra:
        files.update(extra)
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        for name, data in files.items():
            zf.writestr(name, data)
        if symlink:
            info = zipfile.ZipInfo("link")
            info.external_attr = (stat.S_IFLNK | 0o777) << 16
            zf.writestr(info, "paper.pdf")
    return buf.getvalue()


# --- 偽の CLI（背景の subprocess） ---


class FakeStream:
    def __init__(self, data: bytes) -> None:
        self._rest = data

    async def read(self) -> bytes:
        await asyncio.sleep(0)
        return self._rest


class FakeProc:
    """`asyncio.create_subprocess_exec` の偽物。"""

    def __init__(self, stdout: bytes, returncode: int = 0) -> None:
        self._stdout = stdout
        self.returncode = returncode
        self.stdout = FakeStream(stdout)

    async def communicate(self):
        await asyncio.sleep(0)
        return self._stdout, b""

    def kill(self) -> None:
        pass


def fake_exec(stdout: bytes, returncode: int = 0):
    """呼ばれた引数を覚える偽の create_subprocess_exec を作る。"""
    seen: list[list[str]] = []

    async def create(*args, **kwargs):
        seen.append([str(a) for a in args])
        return FakeProc(stdout, returncode)

    return create, seen


class FakeRunJson:
    """`papers.cli.run_json` の置き換え係。"""

    def __init__(self) -> None:
        self.calls: list[dict] = []
        self.value: object = {}

    def __call__(self, args, *, app, cwd=None, env=None, timeout=20.0, input_text=None):
        self.calls.append({"args": list(args), "cwd": cwd, "env": env, "timeout": timeout})
        return self.value

    @property
    def last(self) -> dict:
        assert self.calls, "CLI が呼ばれていない"
        return self.calls[-1]


class FakeNotify:
    """`notify.broadcast` を記録するだけの偽物（store の DB を要しない）。"""

    def __init__(self) -> None:
        self.calls: list[tuple[str, str, str]] = []

    def broadcast(self, title: str, body: str, url: str) -> int:
        self.calls.append((title, body, url))
        return 1


# --- 本文の読み取り（415 / 413） ---


class FakeRequest:
    def __init__(self, body: bytes, content_type="application/zip", length=None) -> None:
        self.headers = {"content-type": content_type}
        if length != "omit":
            self.headers["content-length"] = str(len(body) if length is None else length)
        self._body = body

    async def stream(self):
        step = 1024 * 1024
        for i in range(0, len(self._body), step):
            await asyncio.sleep(0)
            yield self._body[i : i + step]


async def read_upload(body: bytes, **kwargs) -> bytes:
    return await bundles.read_bundle_upload(FakeRequest(body, **kwargs))


def test_read_upload() -> None:
    """Content-Type が違えば 415 / 大きすぎ・Content-Length 無しは 413。"""
    good = build_zip()

    async def ok() -> None:
        assert await read_upload(good) == good
        # charset が付いても application/zip なら受ける
        assert await read_upload(good, content_type="application/zip; charset=binary") == good

    asyncio.run(ok())

    for ctype in ("application/pdf", "application/octet-stream", "text/plain", ""):
        try:
            asyncio.run(read_upload(good, content_type=ctype))
        except HTTPException as e:
            assert e.status_code == 415, f"{ctype}: {e.status_code}"
        else:
            raise AssertionError(f"{ctype} は 415 のはず")

    try:
        asyncio.run(read_upload(good, length="omit"))
    except HTTPException as e:
        assert e.status_code == 413, e.status_code
    else:
        raise AssertionError("Content-Length が無ければ 413 のはず")
    try:
        asyncio.run(read_upload(good, length=bundles.BUNDLE_MAX_BYTES + 1))
    except HTTPException as e:
        assert e.status_code == 413, e.status_code
    else:
        raise AssertionError("上限を超えたら 413 のはず")
    # 上限は 300MB（C14）
    assert bundles.BUNDLE_MAX_BYTES == 300 * 1024 * 1024, bundles.BUNDLE_MAX_BYTES


# --- AC1: 正しい zip は取り込む ---


def test_bundle_ok(base: Path) -> None:
    """202 → 展開 → CLI 呼び出し → フォルダ削除 → job done。"""
    papers._download_jobs.clear()
    project = {"name": "sai-paper", "root": str(base), "root_name": "sai-paper"}
    create, seen = fake_exec(
        json.dumps(
            {"paper_id": 12, "title": "A Paper", "figures": 3, "sections": 5, "refs": 10, "translated": True}
        ).encode()
    )
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    seen_folder: list[Path] = []

    async def main() -> None:
        res = await bundles.request_bundle(project, build_zip())
        assert res["status"] == "processing", res
        key = res["key"]
        assert key.startswith("bundle:"), key
        # 展開先は inbox/<uuid>/（1 つだけ）
        inbox = base / ".sai" / "paper" / "inbox"
        dirs = [p for p in inbox.iterdir() if p.is_dir()]
        assert len(dirs) == 1, dirs
        folder = dirs[0]
        seen_folder.append(folder)
        assert (folder / "manifest.json").is_file()
        assert (folder / "paper.pdf").is_file()
        job = papers._download_jobs[("sai-paper", key)]
        assert job["kind"] == "bundle", job
        assert job["status"] == "processing", job
        assert job["title"] == "A Paper", job  # bibtex の題名
        # 背景の仕事を走らせる
        await job["_task"]
        assert job["status"] == "done", job
        assert job["paper_id"] == 12, job
        assert job["figures"] == 3, job
        assert job["translated"] is True, job
        # 終わったらフォルダは消えている
        assert not folder.exists(), "展開したフォルダが残っている"
        # 一覧にも出る
        listed = papers.list_downloads(project)["jobs"]
        assert listed and listed[0]["key"] == key, listed

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert seen_folder, "展開先を覚えていない"
    assert seen[0][1:] == [
        "paper",
        "import-bundle",
        "--dir",
        str(seen_folder[0]),
        "--json",
    ], seen
    assert len(seen) == 1, seen


def test_bundle_failed(base: Path) -> None:
    """CLI が失敗したら job は failed（フォルダは消える）。"""
    papers._download_jobs.clear()
    project = {"name": "sai-paper", "root": str(base), "root_name": "sai-paper"}
    create, _seen = fake_exec(json.dumps({"error": "PDF を読めません"}).encode(), 1)
    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        res = await bundles.request_bundle(project, build_zip())
        job = papers._download_jobs[("sai-paper", res["key"])]
        folder = next(p for p in (base / ".sai" / "paper" / "inbox").iterdir() if p.is_dir())
        await job["_task"]
        assert job["status"] == "failed", job
        assert job["error"] == "PDF を読めません", job
        assert not folder.exists(), "失敗でもフォルダを消す"

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()


def test_bundle_paper_id_title(base: Path) -> None:
    """paper_id の対象は、論文の題名を manifest からでなく CLI から取る。"""
    papers._download_jobs.clear()
    project = {"name": "sai-paper", "root": str(base), "root_name": "sai-paper"}
    create, _seen = fake_exec(json.dumps({"paper_id": 7, "figures": 0, "translated": False}).encode())
    original_sub = papers.asyncio.create_subprocess_exec
    original_detail = papers.detail
    papers.asyncio.create_subprocess_exec = create
    papers.detail = lambda proj, pid: {"paper": {"id": pid, "title": "題名 from CLI"}}
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    async def main() -> None:
        body = build_zip(manifest_obj=manifest(target={"paper_id": 7}))
        res = await bundles.request_bundle(project, body)
        job = papers._download_jobs[("sai-paper", res["key"])]
        assert job["title"] == "題名 from CLI", job
        await job["_task"]

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original_sub
        papers.detail = original_detail
        papers._download_jobs.clear()


def test_bundle_rejects(base: Path) -> None:
    """危ない zip は CLI を 1 度も呼ばずに 400（検査は Web が行う）。"""
    papers._download_jobs.clear()
    project = {"name": "sai-paper", "root": str(base), "root_name": "sai-paper"}
    called: list = []

    async def create(*args, **kwargs):  # 呼ばれてはいけない
        called.append(args)
        return FakeProc(b"{}", 0)

    original = papers.asyncio.create_subprocess_exec
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")

    bad_bodies = {
        "../x の名前": build_zip(extra={"../x": b"x"}),
        "絶対パス": build_zip(extra={"/etc/passwd": b"x"}),
        "シンボリックリンク": build_zip(symlink=True),
        "manifest.json 無し": build_zip(include_manifest=False),
        "%PDF- で始まらない": build_zip(pdf=b"NOT A PDF"),
        "format の違い": build_zip(manifest_obj=manifest(fmt="other")),
        "pdf が zip に無い": build_zip(manifest_obj=manifest(pdf="missing.pdf")),
        "target が空": build_zip(manifest_obj=manifest(target={})),
        "target が両方": build_zip(manifest_obj=manifest(target={"paper_id": 1, "bibtex": "@a{x}"})),
        "paper_id が 0": build_zip(manifest_obj=manifest(target={"paper_id": 0})),
        "zip ではない": b"not a zip",
    }

    async def main() -> None:
        for label, body in bad_bodies.items():
            await expect_async(400, bundles.request_bundle(project, body))
        assert papers._download_jobs == {}, papers._download_jobs

    try:
        asyncio.run(main())
    finally:
        papers.asyncio.create_subprocess_exec = original
        papers._download_jobs.clear()
    assert called == [], f"検査前に CLI が呼ばれた: {called}"
    # 展開の途中で止まったフォルダも残っていない
    inbox = base / ".sai" / "paper" / "inbox"
    assert not inbox.exists() or not any(p.is_dir() for p in inbox.iterdir()), list(inbox.iterdir())


# --- AC3: keep ---


def test_keep() -> None:
    """`POST …/papers/keep` は `sai-paper paper keep --import-id … --json` を呼ぶ。"""
    cli = FakeRunJson()
    cli.value = {"paper_id": 21, "title": "題名", "status": "added"}
    original = papers.cli.run_json
    papers.cli.run_json = cli
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    try:
        out = papers.keep(PROJECT, {"import_id": "DOI:1"})
        assert out == {"paper_id": 21, "title": "題名", "status": "added"}, out
        assert cli.last["args"] == [
            "sai-paper",
            "paper",
            "keep",
            "--import-id",
            "DOI:1",
            "--json",
        ], cli.last["args"]
        # log / cite の形も同じ
        cli.calls.clear()
        papers.keep(PROJECT, {"log_id": 3, "position": 2})
        assert cli.last["args"] == [
            "sai-paper",
            "paper",
            "keep",
            "--log",
            "3",
            "--position",
            "2",
            "--json",
        ], cli.last["args"]
        cli.calls.clear()
        papers.keep(PROJECT, {"cite_paper_id": 12, "ref_no": 3})
        assert cli.last["args"] == [
            "sai-paper",
            "paper",
            "keep",
            "--cite",
            "12",
            "--ref",
            "3",
            "--json",
        ], cli.last["args"]
        # 検査に通らない依頼は 400（CLI を呼ばない）
        cli.calls.clear()
        expect(400, lambda: papers.keep(PROJECT, {}))
        expect(400, lambda: papers.keep(PROJECT, {"import_id": ""}))
        assert cli.calls == [], cli.calls
    finally:
        papers.cli.run_json = original


def test_pending() -> None:
    """`GET …/papers/pending` は `sai-paper library pending --json` を呼ぶ。"""
    cli = FakeRunJson()
    cli.value = {"papers": [{"id": 1, "title": "待ち"}]}
    original = papers.cli.run_json
    papers.cli.run_json = cli
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    try:
        out = bundles.pending(PROJECT)
        assert out == {"papers": [{"id": 1, "title": "待ち"}]}, out
        assert cli.last["args"] == [
            "sai-paper",
            "library",
            "pending",
            "--json",
        ], cli.last["args"]
    finally:
        papers.cli.run_json = original


def test_job_view() -> None:
    """`GET …/papers/jobs/{key}` の形。無ければ 404。"""
    papers._download_jobs.clear()
    papers._download_jobs[("sai-paper", "bundle:abc")] = {
        "key": "bundle:abc",
        "status": "done",
        "title": "題名",
        "paper_id": 12,
        "figures": 3,
        "translated": True,
        "error": None,
    }
    try:
        out = bundles.job_view(PROJECT, "bundle:abc")
        assert out == {
            "key": "bundle:abc",
            "status": "done",
            "title": "題名",
            "paper_id": 12,
            "figures": 3,
            "translated": True,
        }, out
        expect(404, lambda: bundles.job_view(PROJECT, "bundle:none"))
    finally:
        papers._download_jobs.clear()


# --- AC2: 多人数モードの個人のトークン（ASGI を直接叩く） ---


async def _run_asgi(method, path, body, headers, query_string=b""):
    scope = {
        "type": "http",
        "asgi": {"version": "3.0"},
        "http_version": "1.1",
        "method": method,
        "scheme": "http",
        "path": path,
        "raw_path": path.encode(),
        "query_string": query_string,
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
    # 背景の仕事（束の取り込み）を走らせる
    for _ in range(100):
        await asyncio.sleep(0)
    return messages


def call(method, path, *, body=b"", headers=None, content_type=None):
    hdrs = [(b"host", HOST.encode())]
    if content_type:
        hdrs.append((b"content-type", content_type.encode()))
    if body:
        hdrs.append((b"content-length", str(len(body)).encode()))
    for key, value in (headers or {}).items():
        hdrs.append((key.lower().encode(), value.encode()))
    messages = asyncio.run(_run_asgi(method, path, body, hdrs))
    start = next(m for m in messages if m["type"] == "http.response.start")
    raw = b"".join(m.get("body", b"") for m in messages if m["type"] == "http.response.body")
    try:
        parsed = json.loads(raw.decode("utf-8")) if raw else {}
    except (json.JSONDecodeError, UnicodeDecodeError):
        parsed = raw
    return start["status"], parsed


def test_multi_user_token() -> None:
    """個人のトークンで pending と bundle が通る（多人数モード・AC2）。"""
    store.init_db()
    auth.create_user_dirs("alice")
    store.create_user(
        "alice",
        "alice@uni.edu",
        auth.hash_password("alicepass1"),
        is_admin=False,
        must_change_password=False,
    )
    token = auth.new_api_token()
    store.create_api_token(auth.sha256_hex(token), "alice", "sai-prep")
    # プロジェクトの置き場（<users_root>/alice/projects/alice-demo/.sai/paper/papr.db）
    root = registry.users_root() / "alice" / "projects" / "alice-demo"
    (root / ".sai" / "paper").mkdir(parents=True, exist_ok=True)
    (root / ".sai" / "paper" / "papr.db").write_text("")
    papers._projects_cache.clear()

    bearer = {"authorization": f"Bearer {token}"}
    cli = FakeRunJson()
    cli.value = {"papers": [{"id": 1, "title": "待ち"}]}
    create, seen = fake_exec(json.dumps({"paper_id": 5, "figures": 1, "translated": False}).encode())
    original_cli = papers.cli.run_json
    original_sub = papers.asyncio.create_subprocess_exec
    papers.cli.run_json = cli
    papers.asyncio.create_subprocess_exec = create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    try:
        # トークン無しは 401
        status, _ = call("GET", "/api/projects/alice-demo/papers/pending")
        assert status == 401, status

        status, body = call("GET", "/api/projects/alice-demo/papers/pending", headers=bearer)
        assert status == 200 and body == {"papers": [{"id": 1, "title": "待ち"}]}, (status, body)
        assert cli.last["args"] == ["sai-paper", "library", "pending", "--json"], cli.last["args"]

        # bundle（本文は zip）
        status, body = call(
            "POST",
            "/api/projects/alice-demo/papers/bundle",
            body=build_zip(),
            headers=bearer,
            content_type="application/zip",
        )
        assert status == 202, (status, body)
        assert body["status"] == "processing" and body["key"].startswith("bundle:"), body
        # 背景で CLI が呼ばれ、job が done になっている
        assert seen and seen[0][1:3] == ["paper", "import-bundle"], seen
        job = papers._download_jobs[("alice-demo", body["key"])]
        assert job["status"] == "done", job
        assert job["paper_id"] == 5, job
        # 展開したフォルダは消えている
        inbox = root / ".sai" / "paper" / "inbox"
        assert not inbox.exists() or not any(p.is_dir() for p in inbox.iterdir())

        # 種類が違えば 415
        status, _ = call(
            "POST",
            "/api/projects/alice-demo/papers/bundle",
            body=build_zip(),
            headers=bearer,
            content_type="application/pdf",
        )
        assert status == 415, status
        # 他人のプロジェクト（無い名前）は 404
        status, _ = call("GET", "/api/projects/other/papers/pending", headers=bearer)
        assert status == 404, status
    finally:
        papers.cli.run_json = original_cli
        papers.asyncio.create_subprocess_exec = original_sub
        papers._download_jobs.clear()
        papers._projects_cache.clear()


def test_routes() -> None:
    """新しい住所が登録済みで、消した住所（download・manual/*）が無い（AC3）。"""
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
    ]
    for item in want:
        assert item in got, f"未登録: {item}"
    for gone in (
        ("POST", "/api/projects/{name}/papers/download"),
        ("GET", "/api/projects/{name}/papers/manual/candidates"),
        ("POST", "/api/projects/{name}/papers/manual/import"),
        ("POST", "/api/projects/{name}/papers/manual/upload"),
    ):
        assert gone not in got, f"消した住所が残っている: {gone}"
    paths = [p for _m, p in got]
    int_path = "/api/projects/{name}/papers/{paper_id:int}"
    for fixed in (
        "/api/projects/{name}/papers/keep",
        "/api/projects/{name}/papers/bundle",
        "/api/projects/{name}/papers/pending",
        "/api/projects/{name}/papers/jobs/{key}",
        "/api/projects/{name}/papers/downloads",
    ):
        assert paths.index(fixed) < paths.index(int_path), fixed


def main() -> None:
    bundles.notify = FakeNotify()  # 通知は記録するだけ（store の DB を要しない）
    test_read_upload()
    test_routes()
    test_keep()
    test_pending()
    test_job_view()
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp)
        test_bundle_ok(base)
        test_bundle_failed(base)
        test_bundle_paper_id_title(base)
        test_bundle_rejects(base)
    test_multi_user_token()
    print("test_bundles: すべて通りました")


main()
