"""task.md 作成フォームと新形式 todo の ▶ 実行の APIの小さなテスト。

確かめること:
- 外から来る値の検査（block / task_text / thinking / model）。廃止した項目
  （runner / provider / round / tag）は送っても CLI に渡らない
- 検査は CLI を呼ぶ前に行う（呼び出し 0 回）
- `card create` は stdin に JSON を渡して呼ぶ（cwd = プロジェクトの root）。
  入力は task_text に統一したので `inputs` は送らない（受け取っても断らない）
- ▶ 実行（`card run`）:
    - 成功（終了コード 0）は中の JSON をそのまま返す
    - 終了コード 3 は 404（文に関係なく）
    - 常駐停止などその他は 502（理由は標準エラーの最後の行）
- ■ 終了（`run finalize`）
- review → Workflow（日付フォルダ）の移動（`run advance`）
- 旧 API `GET …/todo`・`POST …/run`・`GET …/card/inputs` は消え，新しい住所が登録済み

実行:
    cd <作業用コピー>
    PYTHONPATH=src .venv/bin/python tests/test_card_api.py
"""

from __future__ import annotations

import json
import subprocess

from fastapi import HTTPException

from agent_link_web import control


def expect(code: int, fn) -> HTTPException:
    """fn は指定した状態番号の HTTPException で失敗し，日本語の理由が付くこと。"""
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{code} のときは英語で理由を書く"
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
    """通るはずの送り値（毎回新しく作る・中身を壊して試すため）。

    古い画面が送る `inputs` も混ぜておき，受け取っても断らないことも確かめる。
    """
    return {
        "done_ids": [26],
        "block": "sub",
        "inputs": {"plan_doc": "waggle:aaa, waggle:bbb"},
        "model": "opus",
        "thinking": "high",
        "task_text": "計画を読んで実装せよ",
    }


def test_validation(cli: FakeCli) -> None:
    """外から来る値は CLI を呼ぶ前に弾く。"""
    cli.calls.clear()

    def with_payload(**change):
        p = good_payload()
        p.update(change)
        return lambda: control.cli_card_create("demo", p)

    # task_text は 100KB まで（UTF-8 のバイト数）
    expect(400, with_payload(task_text="あ" * (100 * 1024)))
    # thinking は一覧の値（空文字はスキップなので通る）
    expect(400, with_payload(thinking="turbo"))
    # done_ids は 1 以上の整数の配列
    expect(400, with_payload(done_ids=[True]))
    expect(400, with_payload(done_ids=["1"]))
    expect(400, with_payload(done_ids=26))
    # block は英数字と -_.: のみ
    expect(400, with_payload(block="a/b"))
    # description は 200 文字までの 1 行（空は通る）
    expect(400, with_payload(description="2 行\nにじむ"))
    expect(400, with_payload(description="あ" * 201))
    # block は必須（task_text は空でも通る。カード作成フォームが空のまま送る）
    expect(400, with_payload(block=""))
    expect(400, with_payload(block=None))
    # model は opus / sonnet / haiku（空は通る）
    expect(400, with_payload(model="gpt"))
    expect(400, with_payload(model="opus; rm -rf"))
    # 形が違うもの
    expect(400, lambda: control.cli_card_create("demo", "这不是对象"))
    # 古い `inputs` は中身が変でも断らない（入力は task_text に統一）
    control.cli_card_create("demo", good_payload() | {"inputs": {"a/b": "これは形式違反"}})

    assert len(cli.calls) == 1, f"inputs を断った: {cli.calls}"
    # task_text が空・model が空でも通る
    control.cli_card_create("demo", good_payload() | {"task_text": "", "model": ""})
    assert len(cli.calls) == 2, cli.calls
    cli.calls.pop()

    # 廃止した項目は，変な値でも 400 にせず無視する（古い画面のキャッシュ用）
    cli.calls.clear()
    control.cli_card_create(
        "demo",
        good_payload()
        | {"runner": "claude", "provider": "open router", "global_round": "five", "round": "2x", "tag": "2 行\nにじむ"},
    )
    assert len(cli.calls) == 1, cli.calls


def test_create_calls_cli(cli: FakeCli) -> None:
    """`card create` は stdin に JSON を載せて，root で呼ぶ。`inputs` は送らない。"""
    cli.returncode = 0
    cli.stdout = json.dumps({"run_id": 39, "path": "/tmp/task.md"})
    cli.stderr = ""
    cli.calls.clear()

    out = control.cli_card_create("demo", good_payload())
    assert out["run_id"] == 39
    call = cli.last
    assert call["args"] == ["agent-link", "card", "create"], call["args"]
    assert call["cwd"] == "/tmp/demo-root", call["cwd"]
    assert call["env"]["AGENT_LINK_PROJECT_ROOT"] == "/tmp/demo-root"
    sent = json.loads(call["input_text"])
    # 送った JSON に入力ファイルの指定は入らない
    assert "inputs" not in sent, sent
    assert sent["done_ids"] == [26]
    assert sent["model"] == "opus" and sent["thinking"] == "high" and sent["block"] == "sub", sent
    # 廃止した項目は，送ってきても CLI の stdin に入れない
    old = good_payload() | {"runner": "cc", "provider": "p", "tag": "fix", "global_round": "5", "round": "2"}
    control.cli_card_create("demo", old)
    sent = json.loads(cli.last["input_text"])
    for key in ("runner", "provider", "tag", "global_round", "round"):
        assert key not in sent, (key, sent)

    # スキップした項目は空文字のまま送る（agent-link が自動値にする）
    minimal = good_payload()
    minimal.update({"model": "", "thinking": "", "done_ids": [], "inputs": {}})
    control.cli_card_create("demo", minimal)
    sent = json.loads(cli.last["input_text"])
    assert sent["model"] == "" and sent["block"] == "sub", sent
    assert "inputs" not in sent, sent

    # options の呼ぶ形
    control.cli_card_options("demo")
    assert cli.last["args"] == ["agent-link", "card", "options"], cli.last["args"]

    # block は絞らず、CLI が返した順のまま全部出す
    cli.stdout = json.dumps(
        {
            "blocks": [
                {"id": "main", "label": "main"},
                {"id": "sub", "label": "sub"},
                {"id": "plan", "label": "plan"},
                {"id": "kantoku", "label": "kantoku"},
                {"id": "web-research", "label": "web-research"},
            ],
        }
    )
    ids = [b["id"] for b in control.cli_card_options("demo")["blocks"]]
    assert ids == ["main", "sub", "plan", "kantoku", "web-research"], ids


def test_start_success(cli: FakeCli) -> None:
    """▶ 実行は `card run <run_id>` を root で呼び，応答をそのまま返す。"""
    cli.returncode = 0
    cli.stdout = json.dumps({"run_id": 39, "message": "実行を要求しました"})
    cli.stderr = ""
    cli.calls.clear()

    out = control.cli_card_start("demo", 39)
    assert out == {"run_id": 39, "message": "実行を要求しました"}
    call = cli.last
    assert call["args"] == ["agent-link", "card", "run", "39"], call["args"]
    assert call["cwd"] == "/tmp/demo-root", call["cwd"]
    assert call["env"]["AGENT_LINK_PROJECT_ROOT"] == "/tmp/demo-root"

    # run_id の検査は呼ぶ前（正の整数だけ）
    cli.calls.clear()
    for bad in (0, -1, True, "3"):
        expect(400, lambda bad=bad: control.cli_card_start("demo", bad))
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"


def test_start_404(cli: FakeCli) -> None:
    """未実行カードが無い（終了コード 3）は 404（文に関係なく）。"""
    cli.returncode = 3
    cli.stdout = ""
    cli.stderr = "未実行のカードが見つかりません"
    cli.calls.clear()
    e = expect(404, lambda: control.cli_card_start("demo", 99999))
    assert str(e.detail) == "未実行のカードが見つかりません", e.detail
    # 別の文でも終了コード 3 なら 404（文の一致は見ない）
    cli.stderr = "まだありません"
    e = expect(404, lambda: control.cli_card_start("demo", 39))
    assert str(e.detail) == "まだありません", e.detail


def test_start_502(cli: FakeCli) -> None:
    """常駐停止などその他の失敗は 502（理由は標準エラーの最後の行）。"""
    # 常駐が止まっている（終了コード 1）
    cli.returncode = 1
    cli.stdout = ""
    cli.stderr = "常駐が止まっています。`agent-link daemon start` を先に実行してください"
    e = expect(502, lambda: control.cli_card_start("demo", 39))
    assert "常駐が止まっています" in str(e.detail), e.detail

    # 最後の空でない行だけを使う
    cli.returncode = 1
    cli.stderr = "first line\n本当の理由"
    e = expect(502, lambda: control.cli_card_start("demo", 39))
    assert str(e.detail) == "本当の理由", e.detail


def test_md_kind_check_accepts_report_and_rejects_others(cli: FakeCli) -> None:
    """md の種類は task / message / report / system。それ以外は 400 で CLI を呼ばない。"""
    cli.returncode = 0
    cli.stdout = json.dumps({"kind": "report", "name": "finalize-report.md", "text": "結果: DONE"})
    cli.stderr = ""
    cli.calls.clear()

    out = control.cli_md("demo", 7, "report")
    assert out["name"] == "finalize-report.md", out
    assert cli.last["args"] == ["agent-link", "run", "md", "7", "--kind=report"], cli.last["args"]

    e = expect(400, lambda: control.cli_md("demo", 7, "review"))
    assert "task, message, report" in str(e.detail), e.detail
    # 保存できるのは task だけ（report は読み取り）
    e = expect(400, lambda: control.cli_md_save("demo", 7, "report", "x"))
    assert "Only task" in str(e.detail), e.detail


def test_finalize_success(cli: FakeCli) -> None:
    """■ 終了は `run finalize <run_id>` を root で呼び，応答をそのまま返す。"""
    cli.returncode = 0
    cli.stdout = json.dumps({"run_id": 84, "message": "終了を要求しました"})
    cli.stderr = ""
    cli.calls.clear()

    out = control.cli_finalize("demo", 84)
    assert out == {"run_id": 84, "message": "終了を要求しました"}
    call = cli.last
    assert call["args"] == ["agent-link", "run", "finalize", "84"], call["args"]
    assert call["cwd"] == "/tmp/demo-root", call["cwd"]
    assert call["env"]["AGENT_LINK_PROJECT_ROOT"] == "/tmp/demo-root"

    # run_id の検査は呼ぶ前（正の整数だけ）
    cli.calls.clear()
    for bad in (0, -1, True, "84"):
        expect(400, lambda bad=bad: control.cli_finalize("demo", bad))
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"


def test_finalize_404(cli: FakeCli) -> None:
    """その run が無い（終了コード 3）は 404（文に関係なく）。"""
    cli.returncode = 3
    cli.stdout = ""
    cli.stderr = "実行が見つかりません"
    cli.calls.clear()
    e = expect(404, lambda: control.cli_finalize("demo", 99999))
    assert str(e.detail) == "実行が見つかりません", e.detail
    # 別の文でも終了コード 3 なら 404
    cli.stderr = "子ブロックは task_report で完了します"
    e = expect(404, lambda: control.cli_finalize("demo", 84))
    assert str(e.detail) == "子ブロックは task_report で完了します", e.detail


def test_finalize_502(cli: FakeCli) -> None:
    """実行中でない・常駐停止など（終了コード 1）は 502。"""
    cli.returncode = 1
    cli.stdout = ""
    cli.stderr = "実行中ではありません（status=completed）"
    e = expect(502, lambda: control.cli_finalize("demo", 84))
    assert "実行中ではありません" in str(e.detail), e.detail


def test_advance_calls(cli: FakeCli) -> None:
    """review → done → Workflow の移動（`run advance <run_id>`）。"""
    cli.returncode = 0
    cli.stdout = '{"run_id": 42, "from": "review", "to": "done"}'
    cli.stderr = ""
    cli.calls.clear()

    out = control.cli_advance("demo", 42)
    assert out == {"run_id": 42, "from": "review", "to": "done"}
    call = cli.last
    assert call["args"] == ["agent-link", "run", "advance", "42"], call["args"]
    assert call["cwd"] == "/tmp/demo-root", call["cwd"]
    assert call["env"]["AGENT_LINK_PROJECT_ROOT"] == "/tmp/demo-root"

    # run_id の検査は呼ぶ前（正の整数だけ）
    cli.calls.clear()
    for bad in (0, -1, True, "42"):
        expect(400, lambda bad=bad: control.cli_advance("demo", bad))
    assert cli.calls == [], f"検査前に CLI が呼ばれた: {cli.calls}"

    # その run が無いとき（終了コード 3）は 404
    cli.returncode = 3
    cli.stdout = ""
    cli.stderr = "実行が見つかりません"
    e = expect(404, lambda: control.cli_advance("demo", 99999))
    assert str(e.detail) == "実行が見つかりません", e.detail

    # review / done 以外（終了コード 1）は 502 に標準エラーの最後の行
    cli.returncode = 1
    cli.stderr = "review / done の run だけ移動できます"
    e = expect(502, lambda: control.cli_advance("demo", 42))
    assert str(e.detail) == "review / done の run だけ移動できます", e.detail


def test_rm_calls(cli: FakeCli) -> None:
    """List の項目を消す（`run rm <run_id> --yes`）。"""
    cli.returncode = 0
    cli.stdout = '{"run_id": 42, "executions": 3}'
    cli.stderr = ""
    cli.calls.clear()
    out = control.cli_rm("demo", 42)
    assert out == {"run_id": 42, "executions": 3}
    assert cli.last["args"] == ["agent-link", "run", "rm", "42", "--yes"], cli.last["args"]


def test_project_checks(cli: FakeCli) -> None:
    """登録簿に無い・root / db が無いプロジェクトは CLI を呼ばず 404。"""
    cli.calls.clear()
    expect(404, lambda: control.cli_card_options("unknown"))
    expect(404, lambda: control.cli_card_options("no-db"))
    expect(404, lambda: control.cli_card_start("unknown", 1))
    expect(404, lambda: control.cli_finalize("no-root", 1))
    expect(404, lambda: control.cli_finalize("no-db", 1))
    expect(404, lambda: control.cli_advance("unknown", 1))
    assert cli.calls == [], f"呼ばれるべきでない CLI 呼び出し: {cli.calls}"


def test_routes() -> None:
    """新しい住所が登録済みで，旧 API（…/todo・…/run・…/card/inputs）は消えている。"""
    from agent_link_web.main import app

    paths = set()
    for route in app.routes:
        for method in getattr(route, "methods", None) or []:
            paths.add((route.path, method))

    for path in [
        "/api/projects/{name}/card/options",
        "/api/projects/{name}/card",
        "/api/projects/{name}/runs/{run_id:int}/start",
    ]:
        method = "POST" if path.endswith("/card") or path.endswith("/start") else "GET"
        assert (path, method) in paths, f"住所が無い: {path} {method}"

    # 入力ファイルの手順は無くなったので住所も消している
    assert ("/api/projects/{name}/card/inputs", "GET") not in paths, "旧 …/card/inputs が残っている"
    # 旧 API は使わなくなったので消している
    assert ("/api/projects/{name}/todo", "GET") not in paths, "旧 …/todo が残っている"
    assert ("/api/projects/{name}/run", "POST") not in paths, "旧 …/run が残っている"
    # メッセージ画面の ■ 終了（Agent タブの住所）
    assert ("/api/runs/{project_name}/{run_id}/finalize", "POST") in paths
    # review → done → Workflow の移動（List 画面）
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
    control.run_cli = cli  # agent-link の代わりに偽物（test_linkview_api と同じやり方）
    control.find_project = make_find_project(rows)
    try:
        test_validation(cli)
        test_create_calls_cli(cli)
        test_start_success(cli)
        test_start_404(cli)
        test_start_502(cli)
        test_md_kind_check_accepts_report_and_rejects_others(cli)
        test_finalize_success(cli)
        test_finalize_404(cli)
        test_finalize_502(cli)
        test_advance_calls(cli)
        test_rm_calls(cli)
        test_project_checks(cli)
    finally:
        control.run_cli = original_run_cli
        control.find_project = original_find
    test_routes()
    # 質問の取り下げの住所は残る。Customer・feedback の住所は消えた
    from agent_link_web.main import app

    paths = {(route.path, m) for route in app.routes for m in (getattr(route, "methods", None) or [])}
    assert ("/api/questions/{qid}/withdraw", "POST") in paths, "住所が無い: withdraw"
    for path, method in (
        ("/api/projects/{name}/customer", "POST"),
        ("/api/customer", "GET"),
        ("/api/feedback", "POST"),
    ):
        assert (path, method) not in paths, f"消したはずの住所が残っている: {path} {method}"
    print("test_card_api: すべて通りました")


main()
