"""会話の検索 API（`GET /api/projects/{name}/sessions/search`・`control.phone_session_search`）の試験。

確かめること:
- 条件が 1 つも無い・範囲・日付・状態・種類・block・検索語の長さの検査
  （外れたら 400・CLI は呼ばない）
- CLI に渡す argv（共有契約「会話の検索の口」）
- 先頭 200 件で切り，201 件目があれば `truncated` を真にすること
- 住所が登録されていること

実行:
    cd <作業用コピー>
    env -u SAI_LINK_BIN PYTHONPATH=apps/web/src \
      /Users/touta/workdir/Sai/sai-moon/apps/web/.venv/bin/python apps/web/tests/test_session_search_api.py
"""

from __future__ import annotations

import json
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
        self.items: list = []

    def __call__(self, args, *, app, cwd=None, env=None, timeout=20.0, input_text=None):
        self.calls.append(list(args))
        return subprocess.CompletedProcess(args, 0, json.dumps(self.items), "")


def test_validation(cli: FakeCli) -> None:
    cli.calls.clear()
    bad = [
        {},
        {"word": "   "},
        {"word": "あ" * 201},
        {"word": "a\nb"},
        {"global_round": "x"},
        {"round": "1-"},
        {"run_id": "-1"},
        {"block": "a b"},
        {"block": "x" * 65},
        {"on_date": "2026-10-01"},
        {"on_date": "2026/1/01"},
        {"status": "bogus"},
        {"sess_type": "bogus"},
    ]
    for kw in bad:
        expect_400(lambda kw=kw: control.phone_session_search("demo", **kw))
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"


def test_calls(cli: FakeCli) -> None:
    cli.calls.clear()
    cli.items = []
    assert control.phone_session_search("demo", word=" mlflow ") == {"items": [], "truncated": False}
    assert cli.calls[-1] == ["sai-link", "session", "search", "-w", "mlflow", "--json"], cli.calls[-1]

    control.phone_session_search(
        "demo",
        word="a,b",
        global_round="2-4",
        round="1",
        run_id="10-20",
        block="worker",
        on_date="2026/09/01-2026/09/30",
        status="running,todo",
        sess_type="agent,user",
    )
    assert cli.calls[-1] == [
        "sai-link", "session", "search",
        "-R", "2-4", "-r", "1", "--id", "10-20", "--block", "worker",
        "--date", "2026/09/01-2026/09/30", "--status", "todo,running",
        "--type", "user,agent", "-w", "a,b", "--json",
    ], cli.calls[-1]


def test_truncate(cli: FakeCli) -> None:
    cli.calls.clear()
    cli.items = [{"block_id": i} for i in range(201)]
    out = control.phone_session_search("demo", word="x")
    assert len(out["items"]) == 200, len(out["items"])
    assert out["truncated"] is True
    cli.items = [{"block_id": 1}]
    out = control.phone_session_search("demo", word="x")
    assert out["truncated"] is False


def test_route() -> None:
    from sai_web.main import app

    paths = {(r.path, tuple(sorted(r.methods or []))) for r in app.routes if hasattr(r, "methods")}
    assert ("/api/projects/{name}/sessions/search", ("GET",)) in paths, "住所が無い"


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
        test_truncate(cli)
    finally:
        control.run_cli, control.find_project = original_run_cli, original_find
    test_route()
    print("test_session_search_api: すべて通りました")


main()
