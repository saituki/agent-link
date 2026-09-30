"""task.md 作成フォームと新形式 todo の ▶ 実行の API（計画 15 §3.3）の小さなテスト。

確かめること:
- 外から来る値の検査（block / task_text / runner / thinking / トークン / round / tag）
- 検査は CLI を呼ぶ前に行う（呼び出し 0 回）
- `card create` は stdin に JSON を渡して呼ぶ（cwd = プロジェクトの root）
- ▶ 実行（`phone card run`）:
    - 成功（終了コード 0）は中の JSON をそのまま返す
    - 終了コード 3 と stderr「未実行のカードが見つかりません」だけ 404
    - 常駐停止などその他は 502 に stderr の先頭 200 文字
- ■ 終了（`phone finalize`・計画書 A の D2）:
    - 成功（終了コード 0）は中の JSON をそのまま返す
    - 終了コード 3 と stderr「実行が見つかりません」だけ 404
    - 実行中でない・子ブロック・常駐停止などは 502 に stderr の先頭 200 文字
- review → done → Workflow の移動（`phone advance`・計画書 A の U10）:
    - 成功（終了コード 0）は中の JSON をそのまま返す
    - 終了コード 3 と stderr「実行が見つかりません」だけ 404
    - review / done 以外（終了コード 1）は 502 に stderr の先頭 200 文字
- 旧 API `GET …/todo` と `POST …/run` は消え，新しい 4 つの住所が登録済み

実行:
    cd <作業用コピー>
    PYTHONPATH=src /Users/touta/workdir/Sai/sai-phone/.venv/bin/python tests/test_card_api.py
"""

from __future__ import annotations

import json
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
    """`control.run_cli` の置き換え係。呼ばれた引数と stdin を全部覚えておく。"""

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


def good_payload() -> dict:
    """通るはずの送り値（毎回新しく作る・中身を壊して試すため）。"""
    return {
        "done_ids": [26],
        "runner": "cc",
        "block": "sub",
        "inputs": {"plan_doc": "waggle:aaa, waggle:bbb"},
        "provider": "",
        "model": "opus",
        "thinking": "high",
        "global_round": "5",
        "round": "2",
        "tag": "fix",
        "task_text": "計画を読んで実装せよ",
    }


def test_validation(cli: FakeCli) -> None:
    """外から来る値は CLI を呼ぶ前に弾く（計画 15 §3.3）。"""
    cli.calls.clear()

    # block は英数字と -_.: のみ（手順 4 の inputs）
    for bad in ("a/b", "", "a" * 129, "ブロック", "a b"):
        expect(400, lambda bad=bad: control.phone_card_inputs("demo", bad))

    def with_payload(**change):
        p = good_payload()
        p.update(change)
        return lambda: control.phone_card_create("demo", p)

    # task_text は 100KB まで（UTF-8 のバイト数）
    expect(400, with_payload(task_text="あ" * (100 * 1024)))
    # runner / thinking は一覧の値（空文字はスキップなので通る）
    expect(400, with_payload(runner="claude"))
    expect(400, with_payload(runner=5))
    expect(400, with_payload(thinking="turbo"))
    # トークンは waggle: 形式
    expect(400, with_payload(inputs={"plan_doc": "aaa"}))
    expect(400, with_payload(inputs={"plan_doc": "waggle:"}))
    expect(400, with_payload(inputs={"plan_doc": "waggle:a, bbb"}))
    expect(400, with_payload(inputs={"a/b": "waggle:a"}))
    expect(400, with_payload(inputs=[["plan_doc", "waggle:a"]]))
    # done_ids は 1 以上の整数の配列
    expect(400, with_payload(done_ids=[True]))
    expect(400, with_payload(done_ids=["1"]))
    expect(400, with_payload(done_ids=26))
    # block も同じ文字（create の方）
    expect(400, with_payload(block="a/b"))
    # global_round / round は数字、tag は 1 行
    expect(400, with_payload(global_round="five"))
    expect(400, with_payload(round="2x"))
    expect(400, with_payload(tag="2 行\nにじむ"))    # provider / model は名前として読める文字だけ
    expect(400, with_payload(provider="open router"))
    expect(400, with_payload(model="opus; rm -rf"))
    # 形が違うもの
    expect(400, lambda: control.phone_card_create("demo", "这不是对象"))

    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"


def test_create_calls_cli(cli: FakeCli) -> None:
    """`card create` は stdin に JSON を載せて，root で呼ぶ。"""
    cli.returncode = 0
    cli.stdout = json.dumps({"run_id": 39, "path": "/tmp/task.md"})
    cli.stderr = ""
    cli.calls.clear()

    out = control.phone_card_create("demo", good_payload())
    assert out["run_id"] == 39
    call = cli.last
    assert call["args"] == ["sai-moon-link", "phone", "card", "create"], call["args"]
    assert call["cwd"] == "/tmp/demo-root", call["cwd"]
    assert call["env"]["SAI_PROJECT_ROOT"] == "/tmp/demo-root"
    sent = json.loads(call["input_text"])
    # 送った JSON は検査を通った形（waggle: 形式のまま・空文字はスキップのまま）
    assert sent["inputs"] == {"plan_doc": "waggle:aaa, waggle:bbb"}, sent["inputs"]
    assert sent["runner"] == "cc"
    assert sent["done_ids"] == [26]

    # スキップした項目は空文字のまま送る（sai-link が自動値にする）
    minimal = good_payload()
    minimal.update({"runner": "", "block": "", "model": "", "thinking": "", "done_ids": [], "inputs": {}})
    control.phone_card_create("demo", minimal)
    sent = json.loads(cli.last["input_text"])
    assert sent["runner"] == ""
    assert sent["inputs"] == {}, sent["inputs"]

    # options と inputs の呼ぶ形
    control.phone_card_options("demo")
    assert cli.last["args"] == ["sai-moon-link", "phone", "card", "options"]
    control.phone_card_inputs("demo", "sub")
    assert cli.last["args"] == [
        "sai-moon-link", "phone", "card", "inputs", "--block", "sub",
    ], cli.last["args"]


def test_start_success(cli: FakeCli) -> None:
    """▶ 実行は `phone card run <run_id>` を root で呼び，応答をそのまま返す。"""
    cli.returncode = 0
    cli.stdout = json.dumps({"run_id": 39, "message": "実行を要求しました"})
    cli.stderr = ""
    cli.calls.clear()

    out = control.phone_card_start("demo", 39)
    assert out == {"run_id": 39, "message": "実行を要求しました"}
    call = cli.last
    assert call["args"] == ["sai-moon-link", "phone", "card", "run", "39"], call["args"]
    assert call["cwd"] == "/tmp/demo-root", call["cwd"]
    assert call["env"]["SAI_PROJECT_ROOT"] == "/tmp/demo-root"

    # run_id の検査は呼ぶ前（正の整数だけ）
    cli.calls.clear()
    for bad in (0, -1, True, "3"):
        expect(400, lambda bad=bad: control.phone_card_start("demo", bad))
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"


def test_start_404(cli: FakeCli) -> None:
    """未実行カードが無いとき（終了コード 3・stderr「未実行のカードが見つかりません」）だけ 404。"""
    cli.returncode = 3
    cli.stdout = ""
    cli.stderr = "未実行のカードが見つかりません"
    cli.calls.clear()
    e = expect(404, lambda: control.phone_card_start("demo", 99999))
    assert str(e.detail) == "未実行のカードが見つかりません", e.detail


def test_start_502(cli: FakeCli) -> None:
    """常駐停止などその他の失敗は 502 に stderr の先頭 200 文字（計画 15 の▶ 実行）。"""
    # 常駐が止まっている（終了コード 1）
    cli.returncode = 1
    cli.stdout = ""
    cli.stderr = "常駐が止まっています。`sai-link daemon start` を先に実行してください"
    e = expect(502, lambda: control.phone_card_start("demo", 39))
    assert "常駐が止まっています" in str(e.detail), e.detail

    # 終了コード 3 でも文言が違えば 404 にしない（md の「まだありません」との違いも確認）
    cli.returncode = 3
    cli.stderr = "まだありません"
    e = expect(502, lambda: control.phone_card_start("demo", 39))
    assert str(e.detail) == "まだありません", e.detail

    # 長い stderr は先頭 200 文字
    cli.returncode = 1
    cli.stderr = "x" * 500
    e = expect(502, lambda: control.phone_card_start("demo", 39))
    assert len(str(e.detail)) == 200, len(str(e.detail))


def test_missing_messages_do_not_cross(cli: FakeCli) -> None:
    """404 の文言は呼び出し側で渡すもの（md と ▶ 実行で入れ替わらない）。"""
    cli.returncode = 3
    cli.stdout = ""
    cli.stderr = "未実行のカードが見つかりません"
    cli.calls.clear()
    # md の API では同じ stderr でも 404 にしない（「まだありません」専用の判定のまま）
    e = expect(502, lambda: control.phone_md("demo", 7, "report"))
    assert str(e.detail) == "未実行のカードが見つかりません", e.detail


def test_md_kind_check_accepts_report_and_rejects_others(cli: FakeCli) -> None:
    """md の種類は task / message / report。それ以外は 400 で CLI を呼ばない。"""
    cli.returncode = 0
    cli.stdout = json.dumps({"kind": "report", "name": "finalize-report.md", "text": "結果: DONE"})
    cli.stderr = ""
    cli.calls.clear()

    out = control.phone_md("demo", 7, "report")
    assert out["name"] == "finalize-report.md", out
    assert cli.last["args"] == ["sai-moon-link", "phone", "md", "7", "--kind", "report"], cli.last["args"]

    e = expect(400, lambda: control.phone_md("demo", 7, "review"))
    assert "task / message / report" in str(e.detail), e.detail
    # 保存できるのは task だけ（report は読み取り）
    e = expect(400, lambda: control.phone_md_save("demo", 7, "report", "x"))
    assert "task だけ" in str(e.detail), e.detail


def test_finalize_success(cli: FakeCli) -> None:
    """■ 終了は `phone finalize <run_id>` を root で呼び，応答をそのまま返す（計画書 A の D2）。"""
    cli.returncode = 0
    cli.stdout = json.dumps({"run_id": 84, "message": "終了を要求しました"})
    cli.stderr = ""
    cli.calls.clear()

    out = control.phone_finalize("demo", 84)
    assert out == {"run_id": 84, "message": "終了を要求しました"}
    call = cli.last
    assert call["args"] == ["sai-moon-link", "phone", "finalize", "84"], call["args"]
    assert call["cwd"] == "/tmp/demo-root", call["cwd"]
    assert call["env"]["SAI_PROJECT_ROOT"] == "/tmp/demo-root"

    # run_id の検査は呼ぶ前（正の整数だけ）
    cli.calls.clear()
    for bad in (0, -1, True, "84"):
        expect(400, lambda bad=bad: control.phone_finalize("demo", bad))
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"


def test_finalize_404(cli: FakeCli) -> None:
    """その run が無いとき（終了コード 3・stderr「実行が見つかりません」）だけ 404。"""
    cli.returncode = 3
    cli.stdout = ""
    cli.stderr = "実行が見つかりません"
    cli.calls.clear()
    e = expect(404, lambda: control.phone_finalize("demo", 99999))
    assert str(e.detail) == "実行が見つかりません", e.detail


def test_finalize_502(cli: FakeCli) -> None:
    """実行中でない・子ブロック・常駐停止などは 502 に stderr の先頭 200 文字。"""
    cli.returncode = 1
    cli.stdout = ""
    cli.stderr = "実行中ではありません（status=completed）"
    e = expect(502, lambda: control.phone_finalize("demo", 84))
    assert "実行中ではありません" in str(e.detail), e.detail

    # 終了コード 3 でも文言が違えば 404 にしない（「未実行のカード…」との入れ替わりを防ぐ）
    cli.returncode = 3
    cli.stderr = "子ブロックは task_report で完了します"
    e = expect(502, lambda: control.phone_finalize("demo", 84))
    assert str(e.detail) == "子ブロックは task_report で完了します", e.detail

    cli.returncode = 1
    cli.stderr = "x" * 500
    e = expect(502, lambda: control.phone_finalize("demo", 84))
    assert len(str(e.detail)) == 200, len(str(e.detail))


def test_advance_calls(cli: FakeCli) -> None:
    """review → done → Workflow の移動（`phone advance <run_id>`・計画書 A の U10）。"""
    cli.returncode = 0
    cli.stdout = '{"run_id": 42, "from": "review", "to": "done"}'
    cli.stderr = ""
    cli.calls.clear()

    out = control.phone_advance("demo", 42)
    assert out == {"run_id": 42, "from": "review", "to": "done"}
    call = cli.last
    assert call["args"] == ["sai-moon-link", "phone", "advance", "42"], call["args"]
    assert call["cwd"] == "/tmp/demo-root", call["cwd"]
    assert call["env"]["SAI_PROJECT_ROOT"] == "/tmp/demo-root"

    # run_id の検査は呼ぶ前（正の整数だけ）
    cli.calls.clear()
    for bad in (0, -1, True, "42"):
        expect(400, lambda bad=bad: control.phone_advance("demo", bad))
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"

    # その run が無いとき（終了コード 3・stderr「実行が見つかりません」）だけ 404
    cli.returncode = 3
    cli.stdout = ""
    cli.stderr = "実行が見つかりません"
    e = expect(404, lambda: control.phone_advance("demo", 99999))
    assert str(e.detail) == "実行が見つかりません", e.detail

    # review / done 以外（終了コード 1）は 502 に stderr
    cli.returncode = 1
    cli.stderr = "review / done の run だけ移動できます"
    e = expect(502, lambda: control.phone_advance("demo", 42))
    assert str(e.detail) == "review / done の run だけ移動できます", e.detail

    # 404 の文言は ▶ 実行（別の物）と入れ替わらない
    cli.returncode = 3
    cli.stderr = "未実行のカードが見つかりません"
    e = expect(502, lambda: control.phone_advance("demo", 42))
    assert str(e.detail) == "未実行のカードが見つかりません", e.detail


def test_project_checks(cli: FakeCli) -> None:
    """登録簿に無い・root / db が無いプロジェクトは CLI を呼ばず 404。"""
    cli.calls.clear()
    expect(404, lambda: control.phone_card_options("unknown"))
    expect(404, lambda: control.phone_card_options("no-db"))
    expect(404, lambda: control.phone_card_start("unknown", 1))
    expect(404, lambda: control.phone_finalize("no-root", 1))
    expect(404, lambda: control.phone_finalize("no-db", 1))
    expect(404, lambda: control.phone_advance("unknown", 1))
    assert cli.calls == [], f"呼ばれるべきでない CLI 呼び出し: {cli.calls}"


def test_routes() -> None:
    """新しい 4 つの住所が登録済みで，旧 API（…/todo と …/run）は消えている。"""
    from sai_phone.main import app

    paths = set()
    for route in app.routes:
        for method in getattr(route, "methods", None) or []:
            paths.add((route.path, method))

    for path in [
        "/api/projects/{name}/card/options",
        "/api/projects/{name}/card/inputs",
        "/api/projects/{name}/card",
        "/api/projects/{name}/runs/{run_id:int}/start",
    ]:
        method = "POST" if path.endswith("/card") or path.endswith("/start") else "GET"
        assert (path, method) in paths, f"住所が無い: {path} {method}"

    # 旧 API は使わなくなったので消している（計画 15 §2）
    assert ("/api/projects/{name}/todo", "GET") not in paths, "旧 …/todo が残っている"
    assert ("/api/projects/{name}/run", "POST") not in paths, "旧 …/run が残っている"
    # メッセージ画面の ■ 終了（Agent タブの住所・計画書 A の D2）
    assert ("/api/runs/{project_name}/{run_id}/finalize", "POST") in paths
    # review → done → Workflow の移動（List 画面・計画書 A の U10）
    assert ("/api/projects/{name}/runs/{run_id:int}/advance", "POST") in paths


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
    control.run_cli = cli  # sai-link の代わりに偽物（test_linkview_api と同じやり方）
    control.find_project = make_find_project(rows)
    try:
        test_validation(cli)
        test_create_calls_cli(cli)
        test_start_success(cli)
        test_start_404(cli)
        test_start_502(cli)
        test_missing_messages_do_not_cross(cli)
        test_md_kind_check_accepts_report_and_rejects_others(cli)
        test_finalize_success(cli)
        test_finalize_404(cli)
        test_finalize_502(cli)
        test_advance_calls(cli)
        test_project_checks(cli)
    finally:
        control.run_cli = original_run_cli
        control.find_project = original_find
    test_routes()
    print("test_card_api: すべて通りました")


main()
