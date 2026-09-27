"""Canvas の API のテスト（U04 §C-D）。

確かめること:
- 7 つの中継が C-D / C-C の表どおりに CLI を呼ぶ（`--` の位置も）
- `toggle` の `--near` は `--` の前、無ければ付けない
- `layout` は位置の指定を JSON の標準入力で渡す
- cwd と `SAI_PAPER_ROOT` はそのプロジェクトの root
- 入力の検査（Canvas 名・ノードの id・位置の指定）
- 7 つの住所が `main.app.routes` に登録されている
- 壊れた応答（error JSON）は 400、起動できない時は 503（U04 §C-D の失敗の返し方）

実行:
    cd <作業用コピー>
    PYTHONPATH=src /Users/touta/workdir/Sai/sai-phone/.venv/bin/python tests/test_papers_canvas.py
"""

from __future__ import annotations

import dataclasses
import json
import subprocess
import tempfile
from pathlib import Path

from fastapi import HTTPException

from sai_phone import cli, control, papers

PY = "/Users/touta/workdir/Sai/sai-phone/.venv/bin/python"


class Recorder:
    """偽の `cli.run_json`。渡された引数などを覚えて，JSON を返す。"""

    def __init__(self) -> None:
        self.calls: list[dict] = []

    def __call__(self, args: list[str], **kw) -> dict:
        self.calls.append({"args": list(args), **kw})
        return {"ok": True, "echo": args[-1] if args else None}

    @property
    def last(self) -> dict:
        assert self.calls, "CLI が呼ばれていない"
        return self.calls[-1]


def make_registry(base: Path) -> list[dict]:
    """登録簿を想定した行（論文の置き場があるプロジェクトが 1 つ・17 §3.1）。"""
    root = base / "sai-paper"
    db = root / ".sai" / "paper"
    db.mkdir(parents=True, exist_ok=True)
    (db / "papr.db").write_text("")
    return [{"name": "sai-paper", "root": str(root), "root_exists": True, "last_executed_at": None}]


def expect_400(fn, detail: str = "") -> None:
    """fn は 400 の HTTPException で失敗する。detail を渡せば文言も確かめる。"""
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == 400, f"400 のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), "400 のときは日本語で理由を書く"
        if detail:
            assert e.detail == detail, f"文言が違う: {e.detail!r}（{detail!r} のはず）"
        return
    raise AssertionError("400 になるはず")


def test_calls(rec: Recorder) -> None:
    """7 つの中継が表どおりに CLI を呼ぶ（`--` の位置・cwd・env 込みで）。"""
    project = papers.resolve_project("sai-paper")
    root = str(project["root"])

    # 1. 一覧
    rec.calls.clear()
    assert papers.canvas_list(project)["ok"] is True
    assert rec.last["args"][1:] == ["phone", "canvas", "list"], rec.last["args"]
    assert rec.last["cwd"] == root, rec.last["cwd"]
    assert rec.last["env"]["SAI_PAPER_ROOT"] == root, rec.last["env"]
    assert "input_text" not in rec.last, rec.last

    # 2. 新建（名前も検査される）
    rec.calls.clear()
    papers.canvas_new(project, "map-preproc-20260926")
    assert rec.last["args"][1:] == [
        "phone", "canvas", "new", "--", "map-preproc-20260926"
    ], rec.last["args"]
    assert rec.last["timeout"] == 30.0, rec.last

    # 3. 中身（note.md を読むので待ち時間は 60 秒）
    rec.calls.clear()
    papers.canvas_show(project, "map")
    assert rec.last["args"][1:] == ["phone", "canvas", "show", "--", "map"], rec.last["args"]
    assert rec.last["timeout"] == 60.0, rec.last

    # 4. toggle（`--near` は `--` の前）
    rec.calls.clear()
    papers.canvas_toggle(project, "map", "note-1", "note-2")
    assert rec.last["args"][1:] == [
        "phone", "canvas", "toggle", "--near", "note-2", "--", "map", "note-1"
    ], rec.last["args"]
    # near が無ければ `--near` を付けない
    rec.calls.clear()
    papers.canvas_toggle(project, "map", "note-1", None)
    assert rec.last["args"][1:] == [
        "phone", "canvas", "toggle", "--", "map", "note-1"
    ], rec.last["args"]

    # 5. missing（60 秒）
    rec.calls.clear()
    papers.canvas_missing(project, "map", "note-1")
    assert rec.last["args"][1:] == [
        "phone", "canvas", "missing", "--", "map", "note-1"
    ], rec.last["args"]
    assert rec.last["timeout"] == 60.0, rec.last

    # 6. body
    rec.calls.clear()
    papers.canvas_body(project, "map", "note-1")
    assert rec.last["args"][1:] == [
        "phone", "canvas", "body", "--", "map", "note-1"
    ], rec.last["args"]


def test_layout_input(rec: Recorder) -> None:
    """位置の指定を JSON の標準入力で渡す（cwd と SAI_PAPER_ROOT も同じ）。"""
    project = papers.resolve_project("sai-paper")
    root = str(project["root"])
    rec.calls.clear()
    papers.canvas_layout(
        project,
        "map",
        {
            "nodes": [
                {"id": "note-1", "x": 10, "y": -20, "width": 300},
                {"id": "report-R-0007", "y": 5, "x": 0, "height": 100},
            ]
        },
    )
    assert rec.last["args"][1:] == ["phone", "canvas", "layout", "--", "map"], rec.last["args"]
    assert rec.last["cwd"] == root, rec.last["cwd"]
    assert rec.last["env"]["SAI_PAPER_ROOT"] == root, rec.last["env"]
    body = json.loads(rec.last["input_text"])
    assert body == {
        "nodes": [
            {"id": "note-1", "x": 10, "y": -20, "width": 300},
            {"id": "report-R-0007", "x": 0, "y": 5, "height": 100},
        ]
    }, body


def test_name_and_id() -> None:
    """Canvas 名とノードの id の検査（C-D）。"""
    for bad in ("../x", "a/b", ".", "", "..", "x" * 129, "a\\b", "a\nb", "a\x7fb", None, 3):
        expect_400(lambda name=bad: papers.validate_canvas_name(name), "Canvas の名前が正しくありません")
    for good in ("前処理", "map-preproc-20260926", "x" * 128, "a.b:c-d"):
        assert papers.validate_canvas_name(good) == good, good

    for bad in ("note 1", "a/b", "", "x" * 129, "../x", None, 3):
        expect_400(lambda nid=bad: papers.validate_node_id(bad), "ノードの id が正しくありません")
    assert papers.validate_node_id("report-R-0007") == "report-R-0007"

    # 中継側でも検査される（CLI を呼ばない）
    project = papers.resolve_project("sai-paper")
    expect_400(lambda: papers.canvas_show(project, "../x"))
    expect_400(lambda: papers.canvas_new(project, "a/b"))
    expect_400(lambda: papers.canvas_toggle(project, "map", "note 1", None))
    expect_400(lambda: papers.canvas_toggle(project, "map", "note-1", "note 1"))
    expect_400(lambda: papers.canvas_missing(project, "map", "a/b"))
    expect_400(lambda: papers.canvas_body(project, "map", "a/b"))


def test_layout_validation() -> None:
    """位置の指定の検査（C-D）。"""
    node = {"id": "note-1", "x": 1, "y": 2}
    assert papers.validate_layout({"nodes": [node]}) == {"nodes": [node]}
    # 大きさは省略してよい／上下限内の整数
    assert papers.validate_layout({"nodes": [{"id": "a", "x": 0, "y": 0, "width": 1}]}) == {
        "nodes": [{"id": "a", "x": 0, "y": 0, "width": 1}]
    }
    assert papers.validate_layout({"nodes": [{"id": "a", "x": 0, "y": 0, "height": 100000}]})
    # 位置の上限
    assert papers.validate_layout({"nodes": [{"id": "a", "x": 10_000_000, "y": -10_000_000}]})

    def bad(payload) -> None:
        expect_400(lambda: papers.validate_layout(payload), "位置の指定が正しくありません")

    bad({"nodes": [{"id": "a", "x": True, "y": 0}]})  # bool は整数でない
    bad({"nodes": [{"id": "a", "x": 1.5, "y": 0}]})  # 実数
    bad({"nodes": [{"id": "a", "x": 10_000_001, "y": 0}]})  # 大きすぎる
    bad({"nodes": [{"id": "a", "x": 0, "y": -10_000_001}]})
    bad({"nodes": [{"id": "a", "x": "1", "y": 0}]})
    bad({"nodes": [{"id": "a", "y": 0}]})  # x が無い
    bad({"nodes": [{"id": "a", "x": 0, "y": 0, "width": 0}]})  # 大きさが 0
    bad({"nodes": [{"id": "a", "x": 0, "y": 0, "height": 100001}]})
    bad({"nodes": [{"id": "a", "x": 0, "y": 0, "width": 1.5}]})
    bad({"nodes": []})  # 0 件
    bad({"nodes": [{"id": f"n{i}", "x": 0, "y": 0} for i in range(5001)]})  # 5001 件
    bad({"nodes": {"id": "a"}})  # 配列でない
    bad({})  # nodes が無い
    bad({"nodes": [1]})  # 要素が辞書でない
    # id の不正はノードの文言
    expect_400(
        lambda: papers.validate_layout({"nodes": [{"id": "a/b", "x": 0, "y": 0}]}),
        "ノードの id が正しくありません",
    )
    # 中継側でも検査される（CLI を呼ばない）
    project = papers.resolve_project("sai-paper")
    expect_400(lambda: papers.canvas_layout(project, "map", {"nodes": []}))
    expect_400(lambda: papers.canvas_layout(project, "map", {}), "位置の指定が正しくありません")
    expect_400(lambda: papers.canvas_layout(project, "map", {"nodes": [{"id": "a", "x": True, "y": 0}]}))


def test_failures(project: dict) -> None:
    """失敗の返し方は `cli.run_json` の決まりに従う（U04 §C-D）。

    - CLI が `{"error": …}` を返して終了コード 1 → 400（その文言）
    - sai-paper が無い → 503
    - JSON でない → 502
    """
    root = str(project["root"])

    # 1) CLI が失敗（error JSON）
    proc = subprocess.CompletedProcess(
        ["/x/sai-paper", "phone", "canvas", "list"],
        1,
        stdout='{"error": "Canvas がありません"}',
        stderr="",
    )

    def fake_run_cli(args, **kw):
        return proc

    orig = cli.run_cli
    cli.run_cli = fake_run_cli
    try:
        try:
            papers.canvas_list(project)
        except HTTPException as e:
            assert e.status_code == 400, e.status_code
            assert e.detail == "Canvas がありません", e.detail
        else:
            raise AssertionError("400 になるはず")
    finally:
        cli.run_cli = orig

    # 2) 起動できない → 503（本当に無いコマンドを呼ぶ・U03 の CLI が無い環境と同じ）
    #    settings は frozen なので，写り換えた物を使う
    orig_settings = papers.settings
    papers.settings = dataclasses.replace(
        orig_settings, sai_paper_bin="/definitely/not/exists/sai-paper"
    )
    try:
        papers.canvas_list(project)
    except HTTPException as e:
        assert e.status_code == 503, f"503 のはずが {e.status_code}: {e.detail}"
    else:
        raise AssertionError("503 になるはず")
    finally:
        papers.settings = orig_settings

    # 3) 読めない応答 → 502
    proc2 = subprocess.CompletedProcess(["x"], 0, stdout="not json", stderr="")
    cli.run_cli = lambda args, **kw: proc2
    try:
        try:
            papers.canvas_list(project)
        except HTTPException as e:
            assert e.status_code == 502, e.status_code
        else:
            raise AssertionError("502 になるはず")
    finally:
        cli.run_cli = orig


def test_routes() -> None:
    """7 つの住所が登録されている（U04 §C-D）。"""
    from sai_phone.main import app

    paths = [(route.path, sorted(getattr(route, "methods", None) or [])) for route in app.routes]
    for path, method in [
        ("/api/projects/{name}/papers/canvas", ["GET"]),
        ("/api/projects/{name}/papers/canvas", ["POST"]),
        ("/api/projects/{name}/papers/canvas/{canvas}", ["GET"]),
        ("/api/projects/{name}/papers/canvas/{canvas}/layout", ["PUT"]),
        ("/api/projects/{name}/papers/canvas/{canvas}/toggle", ["POST"]),
        ("/api/projects/{name}/papers/canvas/{canvas}/nodes/{node_id}/missing", ["GET"]),
        ("/api/projects/{name}/papers/canvas/{canvas}/nodes/{node_id}/body", ["GET"]),
    ]:
        assert (path, method) in paths, f"住所が無い: {method} {path}"

    # 論文番号の住所（`{paper_id:int}`）とぶつからない（`canvas` は int ではない）
    order = [p for p, _ in paths]
    assert order.index("/api/projects/{name}/papers/{paper_id:int}") < order.index(
        "/api/projects/{name}/papers/canvas/{canvas}"
    ), "論文番号の住所が Canvas の住所より先に来ている"


def main() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp)  # type: ignore[assignment]
        rec = Recorder()
        orig_list = control.list_projects
        orig_run_json = cli.run_json
        control.list_projects = lambda: make_registry(base)  # 偽物（10 §5 と同じやり方）
        cli.run_json = rec  # 偽物（U03 の CLI はまだ無い・U04 §前提）
        papers._projects_cache = None
        try:
            test_calls(rec)
            test_layout_input(rec)
            test_name_and_id()
            test_layout_validation()
            # 偽物無しで 503 / 502 / CLI の error 400 を見る（U04 §C-D の失敗の返し方）
            cli.run_json = orig_run_json
            test_failures(papers.resolve_project("sai-paper"))
        finally:
            control.list_projects = orig_list
            cli.run_json = orig_run_json
            papers._projects_cache = None
    test_routes()
    print("test_papers_canvas: すべて通りました")


if __name__ == "__main__":
    main()
