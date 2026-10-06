"""文書検索 API（`GET /api/projects/{name}/search`・`control.phone_search`）の小さなテスト。

確かめること:
- 外から来る値は CLI を呼ぶ前に弾く（400・呼び出し 0 回）
- CLI に渡す形（検索語・種類・日付・block・run_id の範囲・件数）
- 住所が登録されている

実行:
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_search_api.py
"""

from __future__ import annotations

import subprocess

from fastapi import HTTPException

from sai_web import control


def expect_400(fn) -> None:
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == 400, f"400 のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), "日本語の理由が付くこと"
        return
    raise AssertionError("400 になるはず")


class FakeCli:
    """`control.run_cli` の置き換え。呼ばれた引数を覚える。"""

    def __init__(self) -> None:
        self.calls: list[list[str]] = []

    def __call__(self, args, *, app, cwd=None, env=None, timeout=20.0, input_text=None):
        self.calls.append(list(args))
        return subprocess.CompletedProcess(args, 0, '{"items": [], "truncated": false}', "")


def test_validation(cli: FakeCli) -> None:
    cli.calls.clear()
    bad_kwargs = [
        {"word": ""},
        {"word": "   "},
        {"word": "あ" * 201},
        {"word": "x", "category": "task,bogus"},
        {"word": "x", "since": "2026-13-01"},
        {"word": "x", "since": "2026/10/01"},
        {"word": "x", "until": "明日"},
        {"word": "x", "since": "2026-10-02", "until": "2026-10-01"},
        {"word": "x", "block": "../etc"},
        {"word": "x", "block": "a b"},
        {"word": "x", "from_id": 0},
        {"word": "x", "to_id": -1},
        {"word": "x", "from_id": True},
        {"word": "x", "from_id": 9, "to_id": 3},
        {"word": "x", "limit": 0},
        {"word": "x", "limit": 101},
    ]
    for kw in bad_kwargs:
        expect_400(lambda kw=kw: control.phone_search("demo", **kw))
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"


def test_calls(cli: FakeCli) -> None:
    cli.calls.clear()
    assert control.phone_search("demo", " mlflow  run ") == {"items": [], "truncated": False}
    assert cli.calls[-1] == [
        "sai-link", "doc", "search",
        "--word", "mlflow  run", "--category", "task,message,waggle", "--limit", "50",
    ], cli.calls[-1]

    control.phone_search(
        "demo",
        "x",
        category="waggle, task",
        since="2026-09-01",
        until="2026-09-30",
        block="worker",
        from_id=10,
        to_id=99,
        limit=5,
    )
    assert cli.calls[-1] == [
        "sai-link", "doc", "search",
        "--word", "x", "--category", "task,waggle", "--limit", "5",
        "--since", "2026-09-01", "--until", "2026-09-30", "--block", "worker",
        "--from-id", "10", "--to-id", "99",
    ], cli.calls[-1]


def test_route() -> None:
    from sai_web.main import app

    paths = {(r.path, tuple(sorted(r.methods or []))) for r in app.routes if hasattr(r, "methods")}
    assert ("/api/projects/{name}/search", ("GET",)) in paths, "住所が無い"


def main() -> None:
    rows = {"demo": {"name": "demo", "root": "/tmp/demo-root", "root_exists": True, "db_exists": True}}

    def find(name: str) -> dict:
        if name not in rows:
            raise HTTPException(status_code=404, detail="プロジェクトが見つかりません")
        return rows[name]

    cli = FakeCli()
    original_run_cli, original_find = control.run_cli, control.find_project
    control.run_cli = cli
    control.find_project = find
    try:
        test_validation(cli)
        test_calls(cli)
    finally:
        control.run_cli, control.find_project = original_run_cli, original_find
    test_route()
    print("test_search_api: すべて通りました")


main()
