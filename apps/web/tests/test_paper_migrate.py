"""論文の移植の受け口のテスト（assert だけ・実装計画 M02・共有契約 C-M1〜C-M3）。

確かめること（AC1・AC3）:
- 正しい移植の束: 展開 → `library import-migration --dir <そこ> --json` → フォルダが消える →
  job が `done` と `result` を返す（`GET …/papers/jobs/{key}` の形も）
- 危ない移植の束（`../x`・シンボリックリンク・manifest 無し・format / version の違い・
  papers の空・文字列・bool・0・1001 個・papr.db 無し・先頭違い）は CLI を呼ばずに 400
- `migrate-from`: 移植元の root で `library export --ids … --out <outbox>` → 移植先の
  inbox へ移す → 移植先の root で `import-migration` → 両方の残りが無く `done`
- 書き出しが失敗（終了コード 3）なら `failed`、取り込みは呼ばれない
- `migrate-from` の 400（source 空・自分自身・ids が不正）と 404（引けない名前）
- 住所: 種類違いは 415、大きすぎは 413、個人のトークンで通る（多人数モード）

実行:
    cd <作業用コピー>
    PYTHONPATH=src .venv/bin/python tests/test_paper_migrate.py
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
_TMP = Path(tempfile.mkdtemp(prefix="sai-web-migrate-"))
os.environ["SAI_HOME"] = str(_TMP / "sai-home")
os.environ["SAI_WEB_USERS_ROOT"] = str(_TMP / "users")
os.environ["SAI_WEB_COOKIE_SECURE"] = "0"
os.environ.pop("SAI_WEB_TOKEN", None)
os.environ.pop("SAI_WEB_RUNS_DIR", None)

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from fastapi import HTTPException  # noqa: E402

from sai_web import auth, bundles, paper_migrate, papers, registry, store  # noqa: E402
from sai_web.main import app  # noqa: E402

HOST = "testserver"
SQLITE_OK = paper_migrate.SQLITE_MAGIC + b"\x00rest"


def expect_async(code: int, coro):
    """await した結果が指定した状態番号の HTTPException になること（理由は日本語）。"""

    async def run() -> None:
        try:
            await coro
        except HTTPException as e:
            assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
            assert str(e.detail).strip(), f"{code} のときは日本語で理由を書く"
            return
        raise AssertionError(f"{code} になるはず")

    return run()


# --- 移植の束（zip）の組み立て（C-M1） ---


def migration_manifest(ids=(5, 6), fmt=paper_migrate.MIGRATE_FORMAT, version=1) -> dict:
    return {"format": fmt, "version": version, "papers": list(ids)}


def build_zip(
    manifest_obj=None,
    db=SQLITE_OK,
    include_manifest=True,
    include_db=True,
    extra=None,
    symlink=False,
) -> bytes:
    """移植の束の zip を作る（壊れた形も作れる）。"""
    files: dict[str, bytes] = {"papers/5/paper.pdf": b"%PDF-1.7\nfive"}
    if include_db:
        files["papr.db"] = db
    if include_manifest:
        obj = manifest_obj if manifest_obj is not None else migration_manifest()
        files["manifest.json"] = json.dumps(obj, ensure_ascii=False).encode("utf-8")
    if extra:
        files.update(extra)
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        for name, data in files.items():
            zf.writestr(name, data)
        if symlink:
            info = zipfile.ZipInfo("link")
            info.external_attr = (stat.S_IFLNK | 0o777) << 16
            zf.writestr(info, "papr.db")
    return buf.getvalue()


# --- 偽の sai-paper（呼ばれた引数と cwd を覚え、命令ごとの応答を返す） ---


class FakeProc:
    def __init__(self, stdout: bytes, stderr: bytes, returncode: int) -> None:
        self._out = stdout
        self._err = stderr
        self.returncode = returncode

    async def communicate(self):
        await asyncio.sleep(0)
        return self._out, self._err

    def kill(self) -> None:
        pass


class FakeSai:
    """`asyncio.create_subprocess_exec` の偽物。`handler(args, cwd) -> (code, stdout, stderr)`。"""

    def __init__(self, handler) -> None:
        self.handler = handler
        self.calls: list[dict] = []

    async def create(self, *args, cwd=None, **kwargs):
        argv = [str(a) for a in args]
        self.calls.append({"args": argv[1:], "cwd": cwd})
        code, stdout, stderr = self.handler(argv[1:], cwd)
        return FakeProc(stdout, stderr, code)


def ok(data: dict) -> tuple[int, bytes, bytes]:
    return 0, json.dumps(data).encode(), b""


IMPORTED = {
    "imported": [{"source_id": 5, "paper_id": 21, "title": "A Paper", "mode": "new"}],
    "skipped": [{"source_id": 6, "paper_id": 3, "title": "B Paper", "reason": "exists"}],
    "failed": [],
}


class FakeNotify:
    def __init__(self) -> None:
        self.calls: list[tuple[str, str, str]] = []

    def broadcast(self, title: str, body: str, url: str) -> int:
        self.calls.append((title, body, url))
        return 1


class patched:
    """asyncio の subprocess・papers.settings・papers.resolve_project を一時的に置き換える。"""

    def __init__(self, fake: FakeSai, resolve=None) -> None:
        self.fake = fake
        self.resolve = resolve

    def __enter__(self):
        self.saved = (
            papers.asyncio.create_subprocess_exec,
            papers.settings,
            papers.resolve_project,
        )
        papers.asyncio.create_subprocess_exec = self.fake.create
        papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
        if self.resolve is not None:
            papers.resolve_project = self.resolve
        papers._download_jobs.clear()
        return self

    def __exit__(self, *exc) -> None:
        papers.asyncio.create_subprocess_exec, papers.settings, papers.resolve_project = self.saved
        papers._download_jobs.clear()


def only_inbox_empty(root: Path) -> bool:
    inbox = root / ".sai" / "paper" / "inbox"
    return not inbox.exists() or not any(inbox.iterdir())


# --- AC1: 移植の束を取り込む ---


def test_upload_ok(base: Path) -> None:
    """202 → 展開 → import-migration → フォルダ削除 → job done（result つき）。"""
    seen_contents: list[list[str]] = []

    def handler(args, cwd):
        folder = Path(args[args.index("--dir") + 1])
        seen_contents.append(sorted(p.name for p in folder.iterdir()))
        return ok(IMPORTED)

    fake = FakeSai(handler)
    notes = FakeNotify()
    original_notify = paper_migrate.notify
    paper_migrate.notify = notes
    project = {"name": "sai-paper", "root": str(base), "root_name": "sai-paper"}

    async def main() -> None:
        with patched(fake):
            res = await paper_migrate.request_upload(project, build_zip(migration_manifest()))
            assert res["status"] == "processing", res
            key = res["key"]
            assert key.startswith("migrate:"), key
            job = papers._download_jobs[("sai-paper", key)]
            assert job["kind"] == "migrate", job
            assert job["title"] == "論文の移植（2 件）", job
            assert job["steps"] == paper_migrate.MIGRATE_STEPS_UPLOAD, job
            await job["_task"]
            assert job["status"] == "done", job
            assert job["result"] == IMPORTED, job
            view = bundles.job_view(project, key)
            assert view["status"] == "done" and view["result"] == IMPORTED, view

    try:
        asyncio.run(main())
    finally:
        paper_migrate.notify = original_notify
    assert len(fake.calls) == 1, fake.calls
    assert fake.calls[0]["args"][:3] == ["library", "import-migration", "--dir"], fake.calls
    assert fake.calls[0]["args"][-1] == "--json", fake.calls
    assert fake.calls[0]["cwd"] == str(base), fake.calls
    # 展開したフォルダには移植の束の中身があった
    assert seen_contents == [["manifest.json", "papers", "papr.db"]], seen_contents
    # 終わったらフォルダは消えている
    assert only_inbox_empty(base), list((base / ".sai" / "paper" / "inbox").iterdir())
    assert notes.calls and notes.calls[0][0] == "📚 論文を移しました", notes.calls
    assert "移した 1 件・飛ばした 1 件・失敗 0 件" in notes.calls[0][1], notes.calls


def test_upload_import_failed(base: Path) -> None:
    """取り込みが失敗したら failed（理由は標準エラーの最後の行）・フォルダは消える。"""
    fake = FakeSai(lambda args, cwd: (1, b"", "sai-progress step=1\npapr.db の版が違います\n".encode()))
    notes = FakeNotify()
    original_notify = paper_migrate.notify
    paper_migrate.notify = notes
    project = {"name": "sai-paper", "root": str(base), "root_name": "sai-paper"}

    async def main() -> None:
        with patched(fake):
            res = await paper_migrate.request_upload(project, build_zip())
            job = papers._download_jobs[("sai-paper", res["key"])]
            await job["_task"]
            assert job["status"] == "failed", job
            assert job["error"] == "papr.db の版が違います", job

    try:
        asyncio.run(main())
    finally:
        paper_migrate.notify = original_notify
    assert notes.calls and notes.calls[0][0] == "論文を移せませんでした", notes.calls
    assert only_inbox_empty(base), "失敗でもフォルダを消す"


# --- AC1: 危ない移植の束は CLI を呼ばずに 400 ---


def test_upload_rejects(base: Path) -> None:
    """危ない移植の束は CLI を 1 度も呼ばずに 400。"""
    fake = FakeSai(lambda args, cwd: ok({}))
    project = {"name": "sai-paper", "root": str(base), "root_name": "sai-paper"}
    bad_bodies = {
        "../x の名前": build_zip(extra={"../x": b"x"}),
        "絶対パス": build_zip(extra={"/etc/passwd": b"x"}),
        "シンボリックリンク": build_zip(symlink=True),
        "manifest.json 無し": build_zip(include_manifest=False),
        "format の違い": build_zip(manifest_obj=migration_manifest(fmt="sai-prep-bundle")),
        "version の違い": build_zip(manifest_obj=migration_manifest(version=2)),
        "papers が空": build_zip(manifest_obj=migration_manifest(ids=())),
        "papers に文字列": build_zip(manifest_obj={**migration_manifest(), "papers": ["5"]}),
        "papers に bool": build_zip(manifest_obj={**migration_manifest(), "papers": [True]}),
        "papers に 0": build_zip(manifest_obj=migration_manifest(ids=(0,))),
        "papers が 1001 個": build_zip(manifest_obj=migration_manifest(ids=range(1, 1002))),
        "papr.db 無し": build_zip(include_db=False),
        "papr.db の先頭違い": build_zip(db=b"NOT SQLITE FILE"),
        "zip ではない": b"not a zip",
    }

    async def main() -> None:
        with patched(fake):
            for _label, body in bad_bodies.items():
                await expect_async(400, paper_migrate.request_upload(project, body))
            assert papers._download_jobs == {}, papers._download_jobs

    asyncio.run(main())
    assert fake.calls == [], f"検査前に CLI が呼ばれた: {fake.calls}"
    assert only_inbox_empty(base), "展開の途中で止まったフォルダも残っていない"


# --- AC1: migrate-from（サーバの中で移す） ---


def test_from_ok(base: Path) -> None:
    """移植元で書き出し → inbox へ移す → 移植先で取り込む → 両方の残りが無い。"""
    src_root = base / "src"
    dst_root = base / "dst"
    src = {"name": "sai-old", "root": str(src_root), "root_name": "sai-old"}
    dst = {"name": "sai-new", "root": str(dst_root), "root_name": "sai-new"}
    export_seen: list[str] = []

    def handler(args, cwd):
        if args[:2] == ["library", "export"]:
            assert cwd == str(src_root), cwd
            out = Path(args[args.index("--out") + 1])
            export_seen.append(str(out))
            (out / "papers" / "5").mkdir(parents=True)
            (out / "papers" / "5" / "paper.pdf").write_bytes(b"%PDF-1.7")
            (out / "papr.db").write_bytes(SQLITE_OK)
            return ok({"dir": str(out), "papers": [5, 6], "missing": []})
        assert args[:2] == ["library", "import-migration"], args
        assert cwd == str(dst_root), cwd
        return ok(IMPORTED)

    fake = FakeSai(handler)
    notes = FakeNotify()
    original_notify = paper_migrate.notify
    paper_migrate.notify = notes

    def resolve(name: str) -> dict:
        if name == "sai-old":
            return src
        raise HTTPException(status_code=404, detail="論文の置き場があるプロジェクトが見つかりません")

    async def main() -> None:
        with patched(fake, resolve=resolve):
            res = await paper_migrate.request_from(dst, {"source": "sai-old", "ids": [5, 6, 5]})
            assert res["status"] == "processing" and res["key"].startswith("migrate:"), res
            job = papers._download_jobs[("sai-new", res["key"])]
            assert job["steps"] == paper_migrate.MIGRATE_STEPS_FROM, job
            assert job["title"] == "論文の移植（2 件）", job
            await job["_task"]
            assert job["status"] == "done", job
            assert job["result"] == IMPORTED, job

    try:
        asyncio.run(main())
    finally:
        paper_migrate.notify = original_notify
    # 重複を 1 つにまとめ、移植元の root で書き出す
    assert fake.calls[0]["args"][:6] == ["library", "export", "--ids", "5,6", "--out", export_seen[0]], fake.calls[0]
    assert fake.calls[0]["args"][-1] == "--json", fake.calls[0]
    assert Path(export_seen[0]).parent == src_root / ".sai" / "paper" / "outbox", export_seen
    assert fake.calls[1]["args"][:2] == ["library", "import-migration"], fake.calls
    assert len(fake.calls) == 2, fake.calls
    # 移植元の outbox・移植先の inbox の両方に残りが無い
    outbox = src_root / ".sai" / "paper" / "outbox"
    assert outbox.is_dir() and not any(outbox.iterdir()), list(outbox.iterdir())
    assert only_inbox_empty(dst_root), "移植先の inbox に残りがある"
    assert notes.calls and notes.calls[0][0] == "📚 論文を移しました", notes.calls


def test_from_export_failed(base: Path) -> None:
    """書き出しが終了コード 3 なら failed。取り込みは呼ばれず、outbox にも残らない。"""
    src_root = base / "src2"
    dst_root = base / "dst2"
    src = {"name": "sai-old2", "root": str(src_root), "root_name": "sai-old2"}
    dst = {"name": "sai-new2", "root": str(dst_root), "root_name": "sai-new2"}

    def handler(args, cwd):
        return 3, b"", "sai-progress step=0\nids が 1 件も見つかりません\n".encode()

    fake = FakeSai(handler)
    original_notify = paper_migrate.notify
    paper_migrate.notify = FakeNotify()

    async def main() -> None:
        with patched(fake, resolve=lambda name: src):
            res = await paper_migrate.request_from(dst, {"source": "sai-old2", "ids": [12]})
            job = papers._download_jobs[("sai-new2", res["key"])]
            await job["_task"]
            assert job["status"] == "failed", job
            assert job["error"] == "ids が 1 件も見つかりません", job

    try:
        asyncio.run(main())
    finally:
        paper_migrate.notify = original_notify
    assert len(fake.calls) == 1 and fake.calls[0]["args"][:2] == ["library", "export"], fake.calls
    outbox = src_root / ".sai" / "paper" / "outbox"
    assert not outbox.exists() or not any(outbox.iterdir()), "失敗でも書き出し先を残さない"
    assert only_inbox_empty(dst_root)


# --- AC1: migrate-from の 400（CLI を呼ばない） ---


def test_from_rejects() -> None:
    """source・ids の検査は CLI を呼ぶ前に 400。"""
    fake = FakeSai(lambda args, cwd: ok({}))
    dst = {"name": "sai-new", "root": "/nowhere", "root_name": "sai-new"}
    bad_payloads = {
        "本文が dict でない": ["sai-old", [5]],
        "source 無し": {"ids": [5]},
        "source が空": {"source": "  ", "ids": [5]},
        "source が文字列でない": {"source": 3, "ids": [5]},
        "自分自身": {"source": "sai-new", "ids": [5]},
        "ids 無し": {"source": "sai-old"},
        "ids が空": {"source": "sai-old", "ids": []},
        "ids に 0": {"source": "sai-old", "ids": [0]},
        "ids に文字列": {"source": "sai-old", "ids": ["5"]},
        "ids に bool": {"source": "sai-old", "ids": [True]},
        "ids が 1001 個": {"source": "sai-old", "ids": list(range(1, 1002))},
    }

    def resolve(name: str) -> dict:
        raise AssertionError("検査の前に移植元を引いてはいけない")

    async def main() -> None:
        with patched(fake, resolve=resolve):
            for _label, payload in bad_payloads.items():
                await expect_async(400, paper_migrate.request_from(dst, payload))
            assert papers._download_jobs == {}, papers._download_jobs

    asyncio.run(main())
    assert fake.calls == [], fake.calls


# --- AC2: 住所（多人数モードの個人のトークン・ASGI を直接叩く） ---


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
    # 背景の仕事（移植の取り込み）を走らせる
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


def test_routes_and_multi_user() -> None:
    """住所が登録済み。個人のトークンで取り込みと移植の入口が通り、種類違いは 415。"""
    store.init_db()
    auth.create_user_dirs("carol")
    store.create_user(
        "carol",
        "carol@uni.edu",
        auth.hash_password("carolpass1"),
        is_admin=False,
        must_change_password=False,
    )
    token = auth.new_api_token()
    store.create_api_token(auth.sha256_hex(token), "carol", "sai-prep")
    users = registry.users_root() / "carol" / "projects"
    for name in ("carol-a", "carol-b"):
        root = users / name
        (root / ".sai" / "paper").mkdir(parents=True, exist_ok=True)
        (root / ".sai" / "paper" / "papr.db").write_bytes(SQLITE_OK)
    papers._projects_cache.clear()

    got = [
        (method, getattr(route, "path", ""))
        for route in app.routes
        for method in (getattr(route, "methods", None) or [])
    ]
    assert ("POST", "/api/projects/{name}/papers/migrate") in got, got
    assert ("POST", "/api/projects/{name}/papers/migrate-from") in got, got

    bearer = {"authorization": f"Bearer {token}"}
    fake = FakeSai(lambda args, cwd: ok(IMPORTED))
    original_cli = papers.asyncio.create_subprocess_exec
    original_settings = papers.settings
    original_notify = paper_migrate.notify
    papers.asyncio.create_subprocess_exec = fake.create
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    paper_migrate.notify = FakeNotify()
    try:
        # トークン無しは 401
        status, _ = call("POST", "/api/projects/carol-a/papers/migrate", body=build_zip(), content_type="application/zip")
        assert status == 401, status

        # 種類が違えば 415（CLI は呼ばない）
        status, _ = call(
            "POST",
            "/api/projects/carol-a/papers/migrate",
            body=build_zip(),
            headers=bearer,
            content_type="application/pdf",
        )
        assert status == 415, status

        # 本文が移植の束なら 202 で、背景の取り込みが終わって done になる
        status, body = call(
            "POST",
            "/api/projects/carol-a/papers/migrate",
            body=build_zip(),
            headers=bearer,
            content_type="application/zip",
        )
        assert status == 202, (status, body)
        assert body["status"] == "processing" and body["key"].startswith("migrate:"), body
        assert fake.calls and fake.calls[-1]["args"][:2] == ["library", "import-migration"], fake.calls
        status, view = call("GET", f"/api/projects/carol-a/papers/jobs/{body['key']}", headers=bearer)
        assert status == 200 and view["status"] == "done" and view["result"] == IMPORTED, (status, view)
        assert not any((users / "carol-a" / ".sai" / "paper" / "inbox").glob("*")), "展開したフォルダが残っている"

        # migrate-from: 不正な body は 400、引けない移植元は 404（取り込みは始まらない）
        before = len(fake.calls)
        bad = json.dumps({"source": "carol-a", "ids": [5]}).encode()
        status, _ = call("POST", "/api/projects/carol-a/papers/migrate-from", body=bad, headers=bearer, content_type="application/json")
        assert status == 400, status
        status, _ = call(
            "POST",
            "/api/projects/carol-b/papers/migrate-from",
            body=json.dumps({"source": "carol-z", "ids": [5]}).encode(),
            headers=bearer,
            content_type="application/json",
        )
        assert status == 404, status
        status, _ = call(
            "POST",
            "/api/projects/carol-b/papers/migrate-from",
            body=json.dumps({"source": "carol-a", "ids": []}).encode(),
            headers=bearer,
            content_type="application/json",
        )
        assert status == 400, status

        # 本文が大きすぎれば 413（上限を小さくして確かめる・取り込みは始まらない）
        original_limit = bundles.BUNDLE_MAX_BYTES
        bundles.BUNDLE_MAX_BYTES = 10
        try:
            status, _ = call(
                "POST",
                "/api/projects/carol-a/papers/migrate",
                body=build_zip(),
                headers=bearer,
                content_type="application/zip",
            )
            assert status == 413, status
        finally:
            bundles.BUNDLE_MAX_BYTES = original_limit
        assert len(fake.calls) == before, fake.calls
    finally:
        papers.asyncio.create_subprocess_exec = original_cli
        papers.settings = original_settings
        paper_migrate.notify = original_notify
        papers._download_jobs.clear()
        papers._projects_cache.clear()


def main() -> None:
    paper_migrate.notify = FakeNotify()
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp)
        test_upload_ok(base)
        test_upload_import_failed(base)
        test_upload_rejects(base)
        test_from_ok(base)
        test_from_export_failed(base)
    test_from_rejects()
    test_routes_and_multi_user()
    print("test_paper_migrate: すべて通りました")


main()
