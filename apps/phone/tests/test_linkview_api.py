"""List 画面の API（計画 14 §3.3）の小さなテスト。

確かめること:
- 外から来る値の検査（run_id は正の整数 / view・kind は一覧の値 / token は英数字と -_.:）
- 検査は CLI を呼ぶ前に行う（呼び出し 0 回）
- `sai-link <新しい命令>` は cwd = プロジェクトの root，`SAI_PROJECT_ROOT` = 同じ root，
  `SAI_ORIGIN` = phone で呼ばれる
- CLI の失敗の読み替えは共有契約のとおり（2 → 400・3 → 404・4 → 409・1 とそのほか → 502。
  理由は標準エラーの最後の行）
- 終了コード 0 で JSON でない応答は 502
- root / db が無いプロジェクトは CLI を呼ばず 404
- 5 つの住所が登録済み（計画 14 §3.3）
- task の保存（PUT …/md/task）と report の読み出し（GET …/md/report）の検査と呼ぶ形

実行:
    cd <作業用コピー>
    PYTHONPATH=src .venv/bin/python tests/test_linkview_api.py
"""

from __future__ import annotations

import subprocess

from fastapi import HTTPException

from sai_phone import control


def expect(code: int, fn) -> HTTPException:
    """fn は指定した状態番号の HTTPException で失敗し，日本語の理由が付くこと。"""
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{code} のときは日本語で理由を書く"
        return e
    raise AssertionError(f"{code} になるはず")


class FakeCli:
    """`control.run_cli` の置き換え係。呼ばれた引数を全部覚えておく。"""

    def __init__(self) -> None:
        self.calls: list[dict] = []
        self.returncode = 0
        self.stdout = "{}"
        self.stderr = ""

    def __call__(self, args, *, app, cwd=None, env=None, timeout=20.0, input_text=None):
        self.calls.append(
            {
                "args": args,
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


def test_validation(cli: FakeCli) -> None:
    """外から来る値は CLI を呼ぶ前に弾く（計画 14 §3.3）。"""
    cli.calls.clear()
    # run_id: 1 以上の整数だけ
    for bad in (0, -1, True, "3"):
        expect(400, lambda bad=bad: control.phone_run_view("demo", bad, "log"))
    # view: log / stats / history
    for bad in ("bogus", "", "LOG", "task"):
        expect(400, lambda bad=bad: control.phone_run_view("demo", 1, bad))
    # kind: task / message / report
    for bad in ("bogus", "", "TASK", "log", "review"):
        expect(400, lambda bad=bad: control.phone_md("demo", 1, bad))
    # token: 英数字と -_.: だけ（長さ上限 128・: は入れられる形なので使わない）
    for bad in ("bad token", "a" * 129, "../x", "", "絵文字", "a/b"):
        expect(400, lambda bad=bad: control.phone_waggle("demo", 1, bad))
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"


def test_calls(cli: FakeCli) -> None:
    """呼ぶ形（cwd と SAI_PROJECT_ROOT は選んだプロジェクトの root・計画 12 §3.1）。"""
    cli.returncode = 0
    cli.stdout = '{"items": []}'
    cli.stderr = ""
    cli.calls.clear()

    assert control.phone_tree("demo") == {"items": []}
    call = cli.last
    assert call["args"] == ["sai-moon-link", "run", "tree"], call["args"]
    assert call["cwd"] == "/tmp/demo-root", call["cwd"]
    # 起動元の環境変数に引きずられないよう root を明示で渡す。出どころも phone にする
    assert call["env"]["SAI_PROJECT_ROOT"] == "/tmp/demo-root", call["env"].get(
        "SAI_PROJECT_ROOT"
    )
    assert call["env"]["SAI_ORIGIN"] == "phone", call["env"].get("SAI_ORIGIN")
    assert "SAI_PAPER_ROOT" not in call["env"], call["env"]

    cli.stdout = "{}"
    control.phone_run_view("demo", 7, "stats")
    assert cli.last["args"] == ["sai-moon-link", "run", "stats", "7"]

    control.phone_run_view("demo", 7, "history")
    assert cli.last["args"] == ["sai-moon-link", "run", "history", "7"], cli.last["args"]

    control.phone_md("demo", 7, "task")
    assert cli.last["args"] == ["sai-moon-link", "run", "md", "7", "--kind", "task"]

    control.phone_waggle("demo", 7)
    assert cli.last["args"] == [
        "sai-moon-link", "doc", "list", "--run", "7", "--json",
    ], cli.last["args"]

    control.phone_waggle("demo", 7, "abc-123_x.y")
    assert cli.last["args"] == [
        "sai-moon-link", "doc", "read", "abc-123_x.y", "--json",
    ], cli.last["args"]


def test_missing_404(cli: FakeCli) -> None:
    """終了コード 3 は文に関係なく 404（共有契約「Web が CLI を呼ぶ約束」）。"""
    cli.returncode = 3
    cli.stdout = ""
    cli.stderr = "まだありません"
    cli.calls.clear()
    e = expect(404, lambda: control.phone_md("demo", 7, "report"))
    assert str(e.detail) == "まだありません", e.detail
    assert cli.last["args"][1] == "run"
    # 文が「実行が見つかりません」でも 404（文の一致は見ない）
    cli.stderr = "実行が見つかりません: ID9999"
    e = expect(404, lambda: control.phone_run_view("demo", 9999, "log"))
    assert str(e.detail) == "実行が見つかりません: ID9999", e.detail


def test_cli_failure_502(cli: FakeCli) -> None:
    """1 とそのほかは 502（理由は stderr の最後の行），4 は 409。"""
    cli.returncode = 1
    cli.stdout = ""
    cli.stderr = "x" * 500
    cli.calls.clear()
    e = expect(502, lambda: control.phone_tree("demo"))
    assert str(e.detail) == "x" * 500, str(e.detail)

    # 最後の空でない行だけを使う（`sai-progress` の行は飛ばす）
    cli.returncode = 1
    cli.stderr = "sai-progress step=1\n本当の理由です\n"
    e = expect(502, lambda: control.phone_tree("demo"))
    assert str(e.detail) == "本当の理由です", e.detail

    # 終了コード 4 は 409
    cli.returncode = 4
    cli.stderr = "実行中です"
    e = expect(409, lambda: control.phone_tree("demo"))
    assert str(e.detail) == "実行中です", e.detail

    # stderr が空でも 502（文言は日本語）
    cli.returncode = 1
    cli.stderr = ""
    expect(502, lambda: control.phone_tree("demo"))


def test_bad_json_502(cli: FakeCli) -> None:
    """終了コード 0 なのに JSON でない応答は 502。"""
    cli.returncode = 0
    cli.stdout = "これは JSON ではありません"
    cli.stderr = ""
    expect(502, lambda: control.phone_tree("demo"))


def test_md_save(cli: FakeCli) -> None:
    """task の保存（本文は標準入力・検査は CLI を呼ぶ前・未実行カードが無ければ 404）。

    保存できるのは task だけ（message と report は読み取り）。
    """
    cli.calls.clear()
    for bad_kind in ("message", "report", "log", ""):
        expect(400, lambda k=bad_kind: control.phone_md_save("demo", 7, k, "x"))
    for bad_text in (None, 3, "x" * (100 * 1024 + 1)):
        expect(400, lambda t=bad_text: control.phone_md_save("demo", 7, "task", t))
    expect(400, lambda: control.phone_md_save("demo", 0, "task", "x"))
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"

    cli.returncode = 0
    cli.stdout = '{"kind": "task", "name": "task.md", "text": "本文"}'
    cli.stderr = ""
    assert control.phone_md_save("demo", 7, "task", "本文")["text"] == "本文"
    assert cli.last["args"] == [
        "sai-moon-link", "run", "md", "7", "--kind", "task", "--write",
    ], cli.last["args"]
    assert cli.last["input_text"] == "本文"
    # 実行済み・送信済みの task は 404（未実行のカードが見つかりません）
    cli.returncode = 3
    cli.stdout = ""
    cli.stderr = "未実行のカードが見つかりません"
    e = expect(404, lambda: control.phone_md_save("demo", 7, "task", "x"))
    assert str(e.detail) == "未実行のカードが見つかりません", e.detail


def test_project_checks(cli: FakeCli) -> None:
    """登録簿に無い・root / db が無いプロジェクトは CLI を呼ばず 404。"""
    cli.calls.clear()
    expect(404, lambda: control.phone_tree("unknown"))
    expect(404, lambda: control.phone_tree("no-root"))
    expect(404, lambda: control.phone_tree("no-db"))
    assert cli.calls == [], f"呼ばれるべきでない CLI 呼び出し: {cli.calls}"


def test_routes() -> None:
    """5 つの住所が登録済みで，他の /api/projects/… と衝突しない（計画 14 §3.3）。"""
    from sai_phone.main import app

    paths = [
        (route.path, sorted(getattr(route, "methods", None) or []))
        for route in app.routes
    ]
    for path in [
        "/api/projects/{name}/tree",
        "/api/projects/{name}/runs/{run_id:int}/view/{view}",
        "/api/projects/{name}/runs/{run_id:int}/md/{kind}",
        "/api/projects/{name}/runs/{run_id:int}/waggle",
        # 住所の名前は計画の `{token}` と同じ形だが，合言葉の Query(`token`) と
        # 衝突しないよう FastAPI 上の変数名だけ waggle_token にしている（main.py の docstring）
        "/api/projects/{name}/runs/{run_id:int}/waggle/{waggle_token}",
    ]:
        assert (path, ["GET"]) in paths, f"住所が無い: {path}"
    assert ("/api/projects/{name}/runs/{run_id:int}/md/{kind}", ["PUT"]) in paths
    # 旧 `POST …/run`（カード実行）は新形式 todo の
    # `POST …/runs/{run_id}/start` に置き換わった（計画 15・形は test_card_api で確認）
    assert ("/api/projects/{name}/run", ["POST"]) not in paths, paths


def main() -> None:
    # 失敗系のテストが意図的にエラーを出します（ログは最後の 1 行だけ見やすくする）
    import logging

    logging.disable(logging.WARNING)
    rows = [
        {
            "name": "demo",
            "root": "/tmp/demo-root",
            "root_exists": True,
            "db_exists": True,
        },
        {"name": "no-root", "root": "/tmp/gone", "root_exists": False, "db_exists": True},
        {"name": "no-db", "root": "/tmp/demo-root", "root_exists": True, "db_exists": False},
    ]
    cli = FakeCli()
    original_run_cli = control.run_cli
    original_find = control.find_project
    control.run_cli = cli  # sai-link の代わりに偽物（test_papers_projects と同じやり方）
    control.find_project = make_find_project(rows)
    try:
        test_validation(cli)
        test_calls(cli)
        test_missing_404(cli)
        test_cli_failure_502(cli)
        test_bad_json_502(cli)
        test_md_save(cli)
        test_project_checks(cli)
    finally:
        control.run_cli = original_run_cli
        control.find_project = original_find
    test_routes()
    print("test_linkview_api: すべて通りました")


main()
