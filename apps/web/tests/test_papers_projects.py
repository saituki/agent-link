"""論文の置き場があるプロジェクトの一覧・root 引き・PDF・音声化の管理の小さなテスト（計画 17 §5）。

確かめること:
- 置き場（`<root>/.sai/paper/papr.db`）があるプロジェクトだけ一覧に出る／無いと出ない
- 名前の検査（`^[A-Za-z0-9_.-]{1,64}$`）で外れる
- 一覧は 30 秒だけ覚える（プロジェクト一覧を呼びすぎない・17 §3.2）
- 知らない名前は 404（住所から root を引けない・17 §3.2）
- PDF が root の外なら 404（17 §3.4）
- 音声化の作業中の管理が (プロジェクト名, 論文番号)（17 §3.4）
- 古い `/api/papers/...` は消え，新しい住所が登録済み（17 §3.4・00 §6）

実行:
    cd <作業用コピー>
    PYTHONPATH=src .venv/bin/python tests/test_papers_projects.py
"""

from __future__ import annotations

import asyncio
import tempfile
import time
from pathlib import Path

from fastapi import HTTPException

from sai_web import control, papers


class Calls:
    """偽の list_projects が呼ばれた回数を数えるだけの記録係。"""

    def __init__(self, rows: list[dict]) -> None:
        self.rows = rows
        self.count = 0

    def list_projects(self) -> list[dict]:
        self.count += 1
        return self.rows


def expect_404(fn) -> None:
    """fn は 404 の HTTPException で失敗し，理由の文も付くこと。"""
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == 404, f"404 のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), "404 のときは日本語で理由を書く"
        return
    raise AssertionError("404 になるはず")


async def expect_404_async(coro) -> None:
    """coro（await 可）は 404 の HTTPException で失敗すること。"""
    try:
        await coro
    except HTTPException as e:
        assert e.status_code == 404, f"404 のはずが {e.status_code}: {e.detail}"
        return
    raise AssertionError("404 になるはず")


def make_registry(base: Path) -> list[dict]:
    """登録簿を想定した行を作る（プロジェクト一覧の形・06 §3）。"""
    rows: list[dict] = []

    def project(name: str, root: Path, root_exists: bool, last: str) -> dict:
        if root_exists:
            # 論文の置き場（sai-paper init でできる）
            db = root / ".sai" / "paper"
            db.mkdir(parents=True, exist_ok=True)
            (db / "papr.db").write_text("")
        return {"name": name, "root": str(root), "root_exists": root_exists, "last_executed_at": last}

    # 置き場あり（一覧に載る・順不同で最後に動いた順）
    rows.append(project("sai-paper", base / "sai-paper", True, "2026-09-23T10:00:00Z"))
    rows.append(project("second", base / "second", True, "2026-09-22T10:00:00Z"))
    # 置き場が無い（出ない）
    rows.append(project("no-db", base / "no-db", False, "2026-09-21T10:00:00Z"))
    rows[-1] = {"name": "no-db", "root": str(base / "no-db"), "root_exists": True, "last_executed_at": "2026-09-21T10:00:00Z"}
    (base / "no-db").mkdir(parents=True, exist_ok=True)  # root はあるが papr.db が無い
    # root が無い（出ない）
    rows.append({"name": "gone", "root": str(base / "gone"), "root_exists": False, "last_executed_at": None})
    # 置き場はあるが名前が規則に合わない（出ない・17 §3.1）
    rows.append({"name": "bad name!", "root": str(base / "sai-paper"), "root_exists": True, "last_executed_at": None})
    return rows


def test_list(calls: Calls) -> None:
    """置き場があり，名前の規則にも合うプロジェクトだけ，入力（最後に動いた順）のまま出る。"""
    papers._projects_cache.clear()
    calls.count = 0
    projects = papers.list_paper_projects()
    assert [p["name"] for p in projects] == ["sai-paper", "second"], projects
    # root_name は root の末尾のフォルダ名（音声の名前に使う・17 §3.3）
    assert projects[0]["root_name"] == "sai-paper", projects[0]
    assert projects[1]["root_name"] == "second", projects[1]
    assert projects[0]["root"].endswith("/sai-paper"), projects[0]
    # 画面に渡す形には root の絶対パスを出さない（17 §3.4）
    view = papers.paper_projects_view()
    assert [set(p) for p in view] == [
        {"name", "root_name", "last_executed_at"},
        {"name", "root_name", "last_executed_at"},
    ], view
    assert view[0] == {
        "name": "sai-paper",
        "root_name": "sai-paper",
        "last_executed_at": "2026-09-23T10:00:00Z",
    }, view[0]


def test_cache(calls: Calls) -> None:
    """30 秒のあいだは 1 度だけ引き，切れると引き直す（17 §3.2）。"""
    papers._projects_cache.clear()
    calls.count = 0
    papers.resolve_project("sai-paper")
    papers.resolve_project("second")
    papers.paper_projects_view()
    assert calls.count == 1, f"プロジェクト一覧を {calls.count} 回呼んだ（1 回のはず）"

    # 30 秒を過ぎた想定で …
    assert papers._projects_cache, "一覧を覚えていない"
    for k in list(papers._projects_cache):
        papers._projects_cache[k] = (
            time.monotonic() - 31.0,
            papers._projects_cache[k][1],
        )
    papers.resolve_project("sai-paper")
    assert calls.count == 2, calls.count

    # 明示的に引き直すこともできる（テストや手動の確認用）
    papers.list_paper_projects(refresh=True)
    assert calls.count == 3, calls.count


def test_resolve(calls: Calls) -> None:
    """名前で引く。一覧に無い物（規則外・置き場なし・未知）はどれも 404（17 §3.2）。"""
    papers._projects_cache.clear()
    for bad in ("unknown", "bad name!", "no-db", "gone", ""):
        expect_404(lambda name=bad: papers.resolve_project(name))
    project = papers.resolve_project("sai-paper")
    assert project["root"].endswith("/sai-paper"), project


def test_pdf(base: Path) -> None:
    """PDF はそのプロジェクトの root の中だけ返す。外なら 404（17 §3.4）。"""
    project = papers.resolve_project("sai-paper")
    inside = Path(project["root"]) / "paper.pdf"
    inside.write_bytes(b"%PDF-1.4")
    outside = base / "outside.pdf"
    outside.write_bytes(b"%PDF-1.4")
    missing = Path(project["root"]) / "none.pdf"

    original_detail = papers.detail
    try:
        # root の中の PDF → 返す
        papers.detail = lambda proj, pid: {"files": {"pdf": str(inside), "pdf_ja": str(outside)}}
        assert papers.pdf_path(project, 1, None) == inside
        # lang=ja の PDF が root の外 → 404
        expect_404(lambda: papers.pdf_path(project, 1, "ja"))
        # root の中だがファイルが無い → 404
        papers.detail = lambda proj, pid: {"files": {"pdf": str(missing)}}
        expect_404(lambda: papers.pdf_path(project, 1, None))
        # sai-paper が何も返さない → 404
        papers.detail = lambda proj, pid: {"files": {}}
        expect_404(lambda: papers.pdf_path(project, 1, None))
    finally:
        papers.detail = original_detail


async def radio_checks() -> None:
    """音声化の作業中の管理が (プロジェクト名, 論文番号)（17 §3.4）。"""
    original_job = papers._radio_job
    original_detail = papers.detail
    started: dict[tuple[str, int], str] = {}

    async def fake_job(project: dict, paper_id: int, title: str) -> None:
        started[(project["name"], paper_id)] = title

    try:
        papers._radio_job = fake_job
        papers._radio_jobs.clear()
        proj_a = papers.resolve_project("sai-paper")
        proj_b = papers.resolve_project("second")
        papers.detail = lambda proj, pid: {
            "paper": {"id": 1, "title": "題名"},
            "files": {"extract": str(Path(proj["root"]) / "extract.md")},
            "podcast": {"status": "ready", "error": None},
        }

        # sai-paper の 1 番が作成中であるようにする
        running = asyncio.create_task(asyncio.sleep(30))
        papers._radio_jobs[("sai-paper", 1)] = running
        try:
            # 同じプロジェクトの同じ番号 → 409
            try:
                await papers.request_radio(proj_a, 1)
                raise AssertionError("409（作成中）になるはず")
            except HTTPException as e:
                assert e.status_code == 409, e
            # 別プロジェクトの同じ番号 → 通る（誤判定しない・17 §3.4）
            res = await papers.request_radio(proj_b, 1)
            assert res == {"status": "generating", "radio_id": "second-1"}, res
            # まだ消えていない（受け付けた直後）
            assert ("sai-paper", 1) in papers._radio_jobs, papers._radio_jobs.keys()
            assert ("second", 1) in papers._radio_jobs, papers._radio_jobs.keys()
            await asyncio.sleep(0)  # fake_job を走らせる
            assert started == {("second", 1): "題名"}, started
        finally:
            running.cancel()
    finally:
        papers._radio_job = original_job
        papers.detail = original_detail
        papers._radio_jobs.clear()


def test_radio_id() -> None:
    """音声の名前は `<root のフォルダ名>-<論文番号>`（17 §3.3）。"""
    project = papers.resolve_project("sai-paper")
    assert papers.radio_id_for(project, 30) == "sai-paper-30"
    other = papers.resolve_project("second")
    assert papers.radio_id_for(other, 30) == "second-30"


def test_figure_and_note(base: Path) -> None:
    """図の画像は root の中だけ返す（外・無い番号は 404）。ノートは 100KB を超えると 400。"""
    project = papers.resolve_project("sai-paper")
    inside = Path(project["root"]) / "fig-001.png"
    inside.write_bytes(b"\x89PNG")
    outside = base / "fig-002.png"
    outside.write_bytes(b"\x89PNG")
    original_detail = papers.detail
    try:
        papers.detail = lambda proj, pid: {
            "figures": {
                "state": "ready",
                "items": [
                    {"fig_no": 1, "image": str(inside)},
                    {"fig_no": 2, "image": str(outside)},
                ],
            }
        }
        assert papers.figure_path(project, 1, 1) == inside
        expect_404(lambda: papers.figure_path(project, 1, 2))  # root の外
        expect_404(lambda: papers.figure_path(project, 1, 3))  # 無い番号
        papers.detail = lambda proj, pid: {}
        expect_404(lambda: papers.figure_path(project, 1, 1))  # 古い sai-paper（figures 無し）
    finally:
        papers.detail = original_detail
    for bad in ("x" * (papers.NOTE_MAX + 1), None, 3):
        try:
            papers.save_note(project, 1, bad)
        except HTTPException as e:
            assert e.status_code == 400, e.status_code
        else:
            raise AssertionError("大きすぎる・文字でないノートが通った")


def test_routes() -> None:
    """住所は 3.4 の新しい形だけ。古い /api/papers/... は消え，固定パスが先に登録（00 §6）。"""
    from sai_web.main import app

    paths = [(route.path, sorted(getattr(route, "methods", None) or [])) for route in app.routes]

    # 新しい住所（17 §3.4）
    assert ("/api/paper-projects", ["GET"]) in paths, paths
    for path, method in [
        ("/api/projects/{name}/papers/library", ["GET"]),
        ("/api/projects/{name}/papers/tree", ["GET"]),
        ("/api/projects/{name}/papers/providers", ["GET"]),
        ("/api/projects/{name}/papers/search", ["POST"]),
        ("/api/projects/{name}/papers/logs/{log_id:int}", ["GET"]),
        ("/api/projects/{name}/papers/{paper_id:int}", ["GET"]),
        ("/api/projects/{name}/papers/{paper_id:int}/translate", ["POST"]),
        ("/api/projects/{name}/papers/{paper_id:int}/radio", ["POST"]),
        ("/api/projects/{name}/papers/{paper_id:int}/text", ["GET"]),
        ("/api/projects/{name}/papers/{paper_id:int}/pdf", ["GET"]),
        ("/api/projects/{name}/papers/dashboard", ["GET"]),
        ("/api/projects/{name}/papers/{paper_id:int}/note", ["PUT"]),
        ("/api/projects/{name}/papers/{paper_id:int}/figures/{fig_no:int}", ["GET"]),
    ]:
        assert (path, method) in paths, f"住所が無い: {path}"

    # 古い住所は消す（17 §3.4）
    for path, method in [
        ("/api/papers/library", ["GET"]),
        ("/api/papers/tree", ["GET"]),
        ("/api/papers/providers", ["GET"]),
        ("/api/papers/search", ["POST"]),
        ("/api/papers/logs/{log_id:int}", ["GET"]),
        ("/api/papers/{paper_id:int}", ["GET"]),
        ("/api/papers/{paper_id:int}/radio", ["POST"]),
    ]:
        assert (path, method) not in paths, f"古い住所が残っている: {path}"

    # 固定の住所を {paper_id} より先に登録する決まり（00 §6）
    order = [p for p, _ in paths]
    fixed = "/api/projects/{name}/papers/library"
    by_id = "/api/projects/{name}/papers/{paper_id:int}"
    assert order.index(fixed) < order.index(by_id), "library が {paper_id} より先に登録されていない"


def main() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp)  # type: ignore[assignment]
        rows = make_registry(base)
        calls = Calls(rows)
        original_list = control.list_projects
        control.list_projects = calls.list_projects  # 本物の代わりに偽物（10 §5 と同じやり方）
        try:
            test_list(calls)
            test_cache(calls)
            test_resolve(calls)
            test_pdf(base)
            test_figure_and_note(base)
            test_radio_id()
            asyncio.run(radio_checks())
        finally:
            control.list_projects = original_list
            papers._projects_cache.clear()
    test_routes()
    print("test_papers_projects: すべて通りました")


main()
