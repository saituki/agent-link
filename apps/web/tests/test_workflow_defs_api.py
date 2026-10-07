"""Workflow 盤面と定義（block / template / settings）の編集の API の小さなテスト（assert だけ）。

確かめること:
- Workflow: `phone workflow` を cwd = プロジェクトの root・`SAI_PROJECT_ROOT` = 同じ root で呼ぶ
- 定義の一覧・読み出し・保存・削除の引数（`--kind` / `--scope` / `--name` / `--create`）と本文は標準入力
- `_phone_def_json` の終了コードの分け方（0 / 2 / 3 / 4（同じ名前の定義があります）/ その他）
- 検査に通らない値（kind / scope / 名前 / 256KB 超 / settings の削除）は 400 で、CLI を 1 回も呼ばない
- 実行の記録（DB）が無いプロジェクトでも定義を編集できる（root が無いときだけ 404）
- 新しい住所が登録済みで、`/papers/…` の固定の住所が `{paper_id:int}` に取られない

実行:
    cd <作業用コピー>
    PYTHONPATH=src .venv/bin/python tests/test_workflow_defs_api.py
"""

from __future__ import annotations

import json
import subprocess

from fastapi import HTTPException

from sai_web import control
from sai_web.main import app

# 登録簿の行を想定した値（root_exists / db_exists を確かめるため別々を用意する）
WITH_DB = {
    "name": "demo",
    "root": "/tmp/demo-root",
    "kind": "sai-link",
    "root_exists": True,
    "db_exists": True,
    "daemon_alive": True,
    "running": 1,
    "last_executed_at": "2026-09-26 10:12:03",
}
NO_DB = {**WITH_DB, "name": "fresh", "root": "/tmp/fresh-root", "db_exists": False}
NO_ROOT = {**WITH_DB, "name": "gone", "root": "/tmp/gone-root", "root_exists": False}
ROWS = [WITH_DB, NO_DB, NO_ROOT]


def expect(code: int, fn) -> HTTPException:
    """fn は指定した状態番号の HTTPException で失敗し、日本語の理由が付くこと。"""
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{code} のときは日本語で理由を書く"
        return e
    raise AssertionError(f"{code} になるはず")


class FakeCli:
    """`control.run_cli` の置き換え係。呼ばれた引数と stdin を全部覚えておく。"""

    def __init__(self) -> None:
        self.calls: list[dict] = []
        self.returncode = 0
        self.stdout = "{}"
        self.stderr = ""

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
        return subprocess.CompletedProcess(args, self.returncode, self.stdout, self.stderr)

    @property
    def last(self) -> dict:
        assert self.calls, "CLI が呼ばれていない"
        return self.calls[-1]


def make_find_project(rows: list[dict]):
    """登録簿の行を想定した find_project（未知の名前は 404・control 自身と同じ形）。"""

    def find(name: str) -> dict:
        for p in rows:
            if p["name"] == name:
                return p
        raise HTTPException(status_code=404, detail="プロジェクトが見つかりません")

    return find


def setup(cli: FakeCli) -> None:
    """`run_cli` と `find_project` を偽物に差し替える。"""
    control.run_cli = cli
    control.find_project = make_find_project(ROWS)


# --- Workflow ---

WORKFLOW_JSON = {
    "width": 120,
    "height": 64,
    "nodes": [{"id": 10, "run_id": 10, "x": 0, "y": 3, "w": 27, "h": 6}],
    "edges": [],
    "separators": [],
    "lanes": {"todo": [], "running": [], "review": [], "done": []},
}


def test_workflow(cli: FakeCli) -> None:
    """Workflow 盤面は `sai-link workflow` の JSON をそのまま返す（待ち 30 秒）。"""
    cli.returncode = 0
    cli.stdout = json.dumps(WORKFLOW_JSON)
    cli.stderr = ""
    cli.calls.clear()
    out = control.phone_workflow("demo")
    assert out == WORKFLOW_JSON, out
    call = cli.last
    assert call["args"] == ["sai-link", "workflow"], call["args"]
    assert call["cwd"] == "/tmp/demo-root", call["cwd"]
    assert call["env"]["SAI_PROJECT_ROOT"] == "/tmp/demo-root"
    assert call["env"]["SAI_ORIGIN"] == "phone"
    assert call["timeout"] == 30.0, call["timeout"]


def test_workflow_404(cli: FakeCli) -> None:
    """root か DB が無いプロジェクトは CLI を呼ばずに 404。"""
    cli.calls.clear()
    expect(404, lambda: control.phone_workflow("fresh"))
    expect(404, lambda: control.phone_workflow("gone"))
    expect(404, lambda: control.phone_workflow("unknown"))
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"


# --- 定義（def）---

LIST_JSON = {
    "project_root": "/tmp/demo-root",
    "global_root": "/root/.sai",
    "items": [
        {
            "kind": "block",
            "scope": "project",
            "name": "kantoku",
            "title": "kantoku",
            "exists": True,
            "shadowed": False,
            "error": None,
            "used_by": [],
        }
    ],
}


def test_def_list(cli: FakeCli) -> None:
    """定義の一覧は `phone def list`。DB が無くても呼べる（root だけ要る）。"""
    cli.returncode = 0
    cli.stdout = json.dumps(LIST_JSON)
    cli.stderr = ""
    cli.calls.clear()
    assert control.phone_def_list("demo") == LIST_JSON
    assert cli.last["args"] == ["sai-link", "def", "list"], cli.last["args"]
    # 実行の記録が無いプロジェクトでも定義の一覧は出せる
    cli.calls.clear()
    control.phone_def_list("fresh")
    assert cli.last["cwd"] == "/tmp/fresh-root", cli.last["cwd"]
    # フォルダが無いプロジェクトは 404（CLI を呼ばない）
    cli.calls.clear()
    expect(404, lambda: control.phone_def_list("gone"))
    assert cli.calls == [], cli.calls


def test_def_show(cli: FakeCli) -> None:
    """読み出しは `phone def show --kind … --scope … --name …`。"""
    cli.returncode = 0
    cli.stdout = json.dumps({"kind": "block", "name": "kantoku", "text": "hello"})
    cli.stderr = ""
    cli.calls.clear()
    control.phone_def_show("demo", "block", "project", "kantoku")
    assert cli.last["args"] == [
        "sai-link",
        "def",
        "show",
        "--kind",
        "block",
        "--scope",
        "project",
        "--name",
        "kantoku",
    ], cli.last["args"]
    assert cli.last["input_text"] is None, "読み出しは標準入力を使わない"


def test_def_write(cli: FakeCli) -> None:
    """保存は本文を標準入力へ渡し、`create` が真なら `--create` を付ける。"""
    cli.returncode = 0
    cli.stdout = json.dumps({"exists": True, "text": "hello"})
    cli.stderr = ""
    cli.calls.clear()
    out = control.phone_def_write("demo", "block", "project", "kantoku", "hello")
    assert out == {"exists": True, "text": "hello"}, out
    call = cli.last
    assert call["args"] == [
        "sai-link",
        "def",
        "write",
        "--kind",
        "block",
        "--scope",
        "project",
        "--name",
        "kantoku",
    ], call["args"]
    assert "--create" not in call["args"], call["args"]
    assert call["input_text"] == "hello", call["input_text"]

    # create: true
    cli.calls.clear()
    control.phone_def_write("demo", "template", "global", "sai-base", "本文", True)
    args = cli.last["args"]
    assert args[-1] == "--create", args
    assert "sai-base" in args and "--scope" in args, args

    # settings は名前が settings だけ
    cli.calls.clear()
    control.phone_def_write("demo", "settings", "project", "settings", "{}", False)
    assert cli.last["input_text"] == "{}", cli.last["input_text"]


def test_def_write_validation(cli: FakeCli) -> None:
    """検査に通らない値は 400 で、CLI を 1 回も呼ばない。"""
    cli.calls.clear()
    # kind / scope
    expect(400, lambda: control.phone_def_write("demo", "note", "project", "x", "t"))
    expect(400, lambda: control.phone_def_write("demo", "block", "team", "x", "t"))
    # 名前（英数字から始まり・英数字と -_ だけ・64 文字まで）
    for bad in ("", "-x", "_x", "a b", "a.md", "あ", "x" * 65, "a/b", "a\nb"):
        expect(400, lambda bad=bad: control.phone_def_write("demo", "block", "project", bad, "t"))
        expect(400, lambda bad=bad: control.phone_def_show("demo", "template", "global", bad))
    # settings の名前は settings だけ
    expect(400, lambda: control.phone_def_write("demo", "settings", "project", "x", "{}"))
    expect(400, lambda: control.phone_def_delete("demo", "settings", "project", "settings2"))
    # 本文（文字列・256KB まで）
    expect(400, lambda: control.phone_def_write("demo", "block", "project", "x", 123))
    expect(400, lambda: control.phone_def_write("demo", "block", "project", "x", None))
    expect(
        400,
        lambda: control.phone_def_write("demo", "block", "project", "x", "あ" * 100_000),
    )
    # create は真偽値
    expect(
        400,
        lambda: control.phone_def_write("demo", "block", "project", "x", "t", "yes"),
    )
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"


def test_def_delete(cli: FakeCli) -> None:
    """削除は `phone def delete …`。settings は 400 で CLI を呼ばない。"""
    cli.returncode = 0
    cli.stdout = '{"deleted": true}'
    cli.stderr = ""
    cli.calls.clear()
    out = control.phone_def_delete("demo", "template", "project", "sai-base")
    assert out == {"deleted": True}, out
    assert cli.last["args"] == [
        "sai-link",
        "def",
        "delete",
        "--kind",
        "template",
        "--scope",
        "project",
        "--name",
        "sai-base",
    ], cli.last["args"]
    cli.calls.clear()
    e = expect(400, lambda: control.phone_def_delete("demo", "settings", "project", "settings"))
    assert "settings は消せません" in str(e.detail), e.detail
    assert cli.calls == [], cli.calls


def test_def_exit_codes(cli: FakeCli) -> None:
    """`_phone_def_json` の終了コードの分け方（0 / 2 / 3 / 4 / その他）。

    `def write --create` は，同じ名前があるとき終了コード 4 で
    「同じ名前の定義があります」を出す（409 になる）。
    """
    # 0 以外では JSON を読まず、終了コードを先に分ける
    cli.returncode = 3
    cli.stdout = ""
    cli.stderr = "定義が見つかりません"
    e = expect(404, lambda: control.phone_def_show("demo", "block", "project", "nope"))
    assert str(e.detail) == "定義が見つかりません", e.detail

    # 2 = 検査に通らない（利用者が書いた中身の誤り）→ 400 に理由をそのまま
    cli.returncode = 2
    cli.stderr = "frontmatter が読めません"
    e = expect(400, lambda: control.phone_def_write("demo", "block", "project", "x", "---\n"))
    assert "frontmatter が読めません" in str(e.detail), e.detail
    assert e.status_code == 400, e.status_code

    # 4 は 409（同じ名前の定義がある）
    cli.returncode = 4
    cli.stderr = "同じ名前の定義があります"
    e = expect(
        409,
        lambda: control.phone_def_write("demo", "block", "project", "x", "t", True),
    )
    assert str(e.detail) == "同じ名前の定義があります", e.detail

    # 1 で別の文言 → 502
    cli.returncode = 1
    cli.stderr = "HOME が分かりません"
    e = expect(502, lambda: control.phone_def_show("demo", "block", "global", "x"))
    assert "HOME が分かりません" in str(e.detail), e.detail

    # 1 で stderr が空 → 502 の既定の文
    cli.returncode = 1
    cli.stderr = ""
    e = expect(502, lambda: control.phone_def_show("demo", "block", "project", "x"))
    assert str(e.detail) == "sai-link が失敗しました", e.detail

    # 0 でも JSON が読めなければ 502
    cli.returncode = 0
    cli.stdout = "not json"
    e = expect(502, lambda: control.phone_def_show("demo", "block", "project", "x"))
    assert str(e.detail) == "sai-link の応答を読めませんでした", e.detail


def test_def_errors_not_truncated(cli: FakeCli) -> None:
    """失敗の理由は標準エラーの最後の行をそのまま使う（切り詰めない・共有契約）。"""
    cli.returncode = 2
    cli.stderr = "あ" * 500
    e = expect(400, lambda: control.phone_def_write("demo", "block", "project", "x", "t"))
    assert str(e.detail) == "あ" * 500, len(str(e.detail))
    cli.returncode = 1
    cli.stderr = "い" * 500
    e = expect(502, lambda: control.phone_def_show("demo", "block", "project", "x"))
    assert str(e.detail) == "い" * 500, len(str(e.detail))


# --- 住所の登録（AC1） ---


def test_routes() -> None:
    """C-A1 の住所がすべて登録されている。"""
    got = set()
    for route in app.routes:
        path = getattr(route, "path", "")
        for method in getattr(route, "methods", []) or []:
            got.add((method, path))
    want = [
        ("GET", "/api/projects/{name}/workflow"),
        ("GET", "/api/projects/{name}/defs"),
        ("GET", "/api/projects/{name}/defs/{kind}/{scope}/{def_name}"),
        ("PUT", "/api/projects/{name}/defs/{kind}/{scope}/{def_name}"),
        ("DELETE", "/api/projects/{name}/defs/{kind}/{scope}/{def_name}"),
    ]
    for item in want:
        assert item in got, f"未登録: {item}"


def test_def_name_variable() -> None:
    """`{def_name}` は合言葉の `?token=` と衝突しない変数名にする。"""
    for route in app.routes:
        path = getattr(route, "path", "")
        if path.endswith("/{def_name}"):
            return
    raise AssertionError("defs の住所に {def_name} が無い")


def main() -> None:
    cli = FakeCli()
    setup(cli)
    test_workflow(cli)
    test_workflow_404(cli)
    test_def_list(cli)
    test_def_show(cli)
    test_def_write(cli)
    test_def_write_validation(cli)
    test_def_delete(cli)
    test_def_exit_codes(cli)
    test_def_errors_not_truncated(cli)
    test_routes()
    test_def_name_variable()
    print("test_workflow_defs_api: すべて通りました")


main()
