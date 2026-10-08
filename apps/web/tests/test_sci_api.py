"""research-tree（Q・A・F・E・S）の API のテスト。

確かめること:
- 中継が表どおりに CLI を呼ぶ（引数・標準入力・cwd・`SAI_PROJECT_ROOT`・待ち時間）
- 検査に通らない値では CLI が 1 回も呼ばれず、400 と日本語の理由が返る
- 知らない項目と `path` のようなファイルのパスが CLI に渡らない
- READ_CARD の終わりは、始まっていない札でも 1 回の操作で review まで進む
- 9 本の住所が `main.app.routes` にあり、合言葉（`require_token`）で守られている
- C3 の表にない op は 400（op の集合は test_op_ok で確かめる）。research-tree の住所は 9 本だけ

実行:
    cd <作業用コピー>
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_sci_api.py
"""

from __future__ import annotations

import inspect
import json
import tempfile
from pathlib import Path

from fastapi import HTTPException
from fastapi import params as fastapi_params

from sai_web import cli, control, papers, sci


class Recorder:
    """偽の `cli.run_json` と偽の `control._phone_json`。

    - `order` に「どちらを・どの引数で」呼んだか積む（sai-paper と sai-link を呼ぶ順を確かめるため）。
    - `replies` に渡した値を順に返す（空なら `{"ok": True}`）。
    """

    def __init__(self, order: list, replies: list | None = None, app: str = "paper"):
        self.calls: list[dict] = []
        self.order = order
        self.replies = list(replies or [])
        self.app = app
        # sai-sci / sai-paper 側は先頭が実行ファイル・link 側（`phone user …` の引数）はそのまま
        self.trim = 1 if app in ("sci", "paper") else 0

    def __call__(self, args, **kw) -> dict:
        self.calls.append({"app": self.app, "args": list(args), **kw})
        self.order.append((self.app, " ".join(str(a) for a in args[self.trim:])))
        if self.replies:
            value = self.replies.pop(0)
            if isinstance(value, Exception):
                raise value
            return value
        return {"ok": True}

    @property
    def last(self) -> dict:
        assert self.calls, "CLI が呼ばれていない"
        return self.calls[-1]


def make_registry(base: Path) -> list[dict]:
    """登録簿を想定した行（論文の置き場があるプロジェクトが 1 つ）。"""
    root = base / "sai-paper"
    db = root / ".sai" / "paper"
    db.mkdir(parents=True, exist_ok=True)
    (db / "papr.db").write_text("")
    return [
        {
            "name": "sai-paper",
            "root": str(root),
            "root_exists": True,
            "last_executed_at": None,
        }
    ]


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


def swap(order: list, replies: list | None = None):
    """偽の CLI を差し込む。(sci, link, 元に戻す関数) を返す。"""
    paper = Recorder(order, replies=replies, app="sci")
    link = Recorder(order, app="link")
    orig = cli.run_json, control._phone_json
    cli.run_json, control._phone_json = paper, link

    def restore() -> None:
        cli.run_json, control._phone_json = orig

    return paper, link, restore


# C-OP の 14 op（C3 の表）の通る例
C3_OPS = {
    "add_q", "insert_q", "move_a", "copy", "remove", "add_a", "add_f",
    "add_e", "add_sprout", "set_stance", "set_mlflow", "star", "unstar", "edit",
}
OP_EXAMPLES = {
    "add_q": {"parent": "Q1", "text": "なぜ？", "guide": "g-1"},
    "insert_q": {"q": "Q1", "text": "間に入れる問い", "guide": "g-1"},
    "move_a": {"from": "Q1", "to": "Q3"},
    "copy": {"node": "A2", "to": "Q3"},
    "remove": {"node": "F4"},
    "add_a": {"q": "Q1", "text": "候補", "body": "説明"},
    "add_f": {
        "parent": "A2",
        "text": "速さが効いた。",
        "stance": "support",
        "cites": ["#37.3.1 速さ", "#40", "#41.2-4"],
        "paper": 37,
        "report": "R-0012",
    },
    "add_e": {"parent": "E1", "text": "実験", "body": "b", "mlflow": "run/abc"},
    "add_sprout": {"title": "題", "body": "計画", "paths": ["A1", "A2"]},
    "set_stance": {"node": "F3", "stance": "deny"},
    "set_mlflow": {"node": "E1", "mlflow": "run/xyz"},
    "star": {"node": "A2"},
    "unstar": {"node": "Q1"},
    "edit": {"node": "A2", "text": "直した", "body": ""},
}


def test_ids() -> None:
    """ID と run_id の検査。O は無くなったので 400。"""
    assert sci.check_node("Q12") == "Q12" and sci.check_node(" A5 ") == "A5"
    assert sci.check_node("F31") == "F31" and sci.check_node("E2") == "E2"
    assert sci.check_node("S1") == "S1"
    for bad in ("12", "Q0", "O1", "X1", "N1", "L1", "P1", "q1", "Q", "SEED", "", "Q1000000000", None, 12):
        expect_400(lambda value=bad: sci.check_node(value), "ID の形が正しくありません")
    assert not hasattr(sci, "check_proposal") and not hasattr(sci, "check_ref")
    assert not hasattr(sci, "graph")

    assert sci.check_run_id(130) == 130 and sci.check_run_id("130") == 130
    for bad in (0, -1, 1_000_000_000, "0", "x", None, True, 1.5):
        expect_400(
            lambda value=bad: sci.check_run_id(value),
            "run_id は 1〜999999999 の整数にしてください",
        )
    assert sci.check_type("problem") == "problem"
    for bad in ("unknown", "", "課題", None):
        expect_400(lambda value=bad: sci.check_type(value), "type が正しくありません")


def test_op_ok() -> None:
    """14 op が通り、前後の空白は落ち、body は空のまま送れる。"""
    assert set(OP_EXAMPLES) == set(sci._OP_FIELDS) == C3_OPS, set(sci._OP_FIELDS) ^ C3_OPS
    assert len(sci._OP_FIELDS) == 14
    for name, fields in OP_EXAMPLES.items():
        got = sci.validate_op({"op": name, **fields})
        for key, value in fields.items():
            assert got[key] == value, (name, key, got)
    assert sci.validate_op({"op": "star", "node": " A2 "}) == {"op": "star", "node": "A2"}
    # 種類の組（star は F も A も通る）は CLI が見る。Web は ID の形だけ
    assert sci.validate_op({"op": "star", "node": "F31"})["node"] == "F31"
    assert sci.validate_op({"op": "add_q", "parent": "A1", "text": "x"})["parent"] == "A1"
    assert sci.validate_op({"op": "add_q", "parent": "SEED", "text": "x"})["parent"] == "SEED"
    assert sci.validate_op({"op": "add_e", "parent": "E1", "text": "x"})["parent"] == "E1"
    assert sci.validate_op({"op": "add_e", "parent": "S2", "text": "x"})["parent"] == "S2"
    assert sci.validate_op({"op": "remove", "node": "S1"})["node"] == "S1"
    # 任意の項目は無くてよい
    assert sci.validate_op({"op": "add_a", "q": "Q1", "text": "候補"}) == {
        "op": "add_a", "q": "Q1", "text": "候補",
    }
    # set_mlflow の空文字は「外す」の意味で、そのまま CLI へ渡す
    assert sci.validate_op({"op": "set_mlflow", "node": "E1", "mlflow": ""}) == {
        "op": "set_mlflow", "node": "E1", "mlflow": "",
    }
    # add_f は stance が要る。cites・paper・report は任意
    assert sci.validate_op({"op": "add_f", "parent": "A1", "text": "x", "stance": "issue"}) == {
        "op": "add_f", "parent": "A1", "text": "x", "stance": "issue",
    }


def test_op_ng() -> None:
    """通らない op は 400。"""

    def bad(payload, detail: str = "") -> None:
        expect_400(lambda: sci.validate_op(payload), detail)

    bad({}, "op がありません")
    bad({"op": "unknown_op"}, "知らない操作です")
    bad({"op": "add_finding", "node": "A1", "text": "x"}, "知らない操作です")
    bad({"op": "star"}, "node がありません")
    bad({"op": "star", "node": "O1"}, "ID の形が正しくありません")
    bad({"op": "star", "node": "12"}, "ID の形が正しくありません")
    bad({"op": "add_q", "parent": "O1", "text": "x"}, "ID の形が正しくありません")
    bad({"op": "add_q", "parent": "Q1", "text": "あ" * 301})
    bad({"op": "add_q", "parent": "Q1", "text": "x", "guide": "日本語"}, "guide の形が正しくありません")
    bad({"op": "insert_q", "q": "A1", "text": "x"}, "ID の形が正しくありません")
    bad({"op": "insert_q", "q": "Q1", "text": "あ" * 301})
    bad({"op": "move_a", "from": "Q1"}, "to がありません")
    bad({"op": "move_a", "from": "A1", "to": "Q2"}, "ID の形が正しくありません")
    bad({"op": "copy", "node": "Q1", "to": "Q2"}, "ID の形が正しくありません")
    bad({"op": "copy", "node": "A1", "to": "A2"}, "ID の形が正しくありません")
    bad({"op": "add_a", "q": "Q1", "text": "あ" * 121})
    bad({"op": "add_a", "q": "Q1", "text": "x", "body": "あ" * 2001})
    bad({"op": "add_f", "parent": "A1", "text": "x"}, "stance がありません")
    bad({"op": "add_f", "parent": "A1", "text": "x", "stance": "maybe"}, "stance が正しくありません")
    bad({"op": "add_f", "parent": "Q1", "text": "x", "stance": "support"}, "ID の形が正しくありません")
    bad({"op": "add_f", "parent": "A1", "text": "あ" * 1201, "stance": "support"})
    bad({"op": "add_f", "parent": "A1", "text": "x", "stance": "support",
         "cites": [f"#{i}" for i in range(1, 12)]})
    for cite in ("37", "#0", "#37.", "#37 " + "あ" * 81, "#37.0", "#a"):
        bad({"op": "add_f", "parent": "A1", "text": "x", "stance": "support", "cites": [cite]})
    bad({"op": "add_f", "parent": "A1", "text": "x", "stance": "support", "paper": 0})
    bad({"op": "add_e", "parent": "Q1", "text": "x"})
    bad({"op": "add_e", "parent": "A2", "text": "x"})
    bad({"op": "add_e", "parent": "E1", "text": "x", "mlflow": "あ" * 201})
    bad({"op": "set_stance", "node": "A1", "stance": "support"}, "ID の形が正しくありません")
    bad({"op": "set_stance", "node": "F1"}, "stance がありません")
    bad({"op": "set_mlflow", "node": "Q1", "mlflow": "x"}, "ID の形が正しくありません")
    bad({"op": "set_mlflow", "node": "E1"}, "mlflow がありません")
    bad({"op": "set_mlflow", "node": "E1", "mlflow": "あ" * 201})
    bad({"op": "add_sprout", "title": "題", "paths": []}, "paths の指定が正しくありません")
    bad({"op": "add_sprout", "title": "題", "paths": ["Q1"]}, "ID の形が正しくありません")
    bad({"op": "add_sprout", "title": "題", "paths": ["A1", "A1"]}, "paths に同じ A が 2 回入っています")
    bad({"op": "add_sprout", "title": "題", "paths": [f"A{i}" for i in range(1, 22)]})
    bad({"op": "add_sprout", "title": "題"}, "paths がありません")
    bad({"op": "add_sprout", "title": "あ" * 121, "paths": ["A1"]})
    bad({"op": "add_sprout", "title": "題", "body": "あ" * 2001, "paths": ["A1"]})
    bad({"op": "edit", "node": "A1"}, "text か body のどちらかが要ります")
    bad({"op": "edit", "node": "A1", "text": "あ" * 1201})


def test_unknown_fields_dropped() -> None:
    """知らない項目と `path` のようなファイルのパスは落ちている。"""
    got = sci.validate_op(
        {"op": "add_q", "parent": "Q1", "text": "なぜ？", "path": "/etc/passwd",
         "proposal": "P55", "actor": "run:130", "reason": "足す"}
    )
    assert got == {"op": "add_q", "parent": "Q1", "text": "なぜ？"}, got
    card = sci.validate_card(
        {"node": "Q1", "block": "sci-read", "purpose": "目的", "papers": [37],
         "path": "/etc/passwd", "doc": "waggle:AXANSMpo", "question": "Q7",
         "role": "exemplar", "executer": "professor"}
    )
    for key in ("path", "doc", "question", "role", "executer"):
        assert key not in card, (key, card)
    assert card["papers"] == [37] and card["runner"] == "pi", card


def test_card() -> None:
    """カードの検査（C-CARD）。"""
    full = {
        "node": "Q12", "block": "sci-ideate", "purpose": "  答えの候補を出す  ",
        "expect": "O を 2 つ以上", "scope": "path", "assume": ["Q9=SST-2 を使う（仮）"],
        "mode": "purpose_first", "papers": [37], "runner": "pi", "depth": "full",
        "origin_run": 120,
    }
    got = sci.validate_card(full)
    # U22: 範囲と仮置きは受けない（知らない項目として落とす）。
    expected = {**full, "purpose": "答えの候補を出す"}
    expected.pop("scope")
    expected.pop("assume")
    assert got == expected, got
    ok_user = {"node": "A1", "block": "sci-read", "purpose": "目的", "papers": [37], "runner": "user"}
    assert sci.validate_card(ok_user)["runner"] == "user"
    assert sci.validate_card({**ok_user, "mode": "blind_first"})["mode"] == "blind_first"
    assert "mode" not in sci.validate_card(ok_user)
    # SEED はカードの対象にできる（実行は pi）
    seed_card = {"node": "SEED", "block": "sci-review", "purpose": "木全体を見直す", "runner": "pi"}
    assert sci.validate_card(seed_card)["node"] == "SEED"
    # C9: LabAgent の block id（lab-*）も通る
    assert sci.validate_card({**ok_user, "block": "lab-read"})["block"] == "lab-read"
    # C7: 足す子の型（Q・A・F）。空と null は未指定として落とす
    for child in ("Q", "A", "F"):
        assert sci.validate_card({**ok_user, "child": child})["child"] == child
    assert "child" not in sci.validate_card({**ok_user, "child": ""})
    assert "child" not in sci.validate_card({**ok_user, "child": None})
    assert "child" not in sci.validate_card(ok_user)

    def bad(extra: dict, detail: str = "") -> None:
        expect_400(lambda: sci.validate_card({**ok_user, **extra}), detail)

    bad({"papers": [37, 40]}, "READ_CARD は論文を 1 つだけ指定してください")
    bad({"papers": []}, "READ_CARD は論文を 1 つだけ指定してください")
    bad({"purpose": ""}, "purpose がありません")
    bad({"purpose": "あ" * 301})
    bad({"purpose": "1 行目\n2 行目"}, "purpose は 1 行にしてください")
    bad({"block": "sci-Read"}, "block の形が正しくありません")
    bad({"expect": "あ" * 301})
    bad({"node": "N1"}, "ID の形が正しくありません")
    bad({"node": "12"}, "ID の形が正しくありません")
    bad({"node": "O1"}, "ID の形が正しくありません")
    bad({"node": "F1"}, "READ_CARD の対象は Q か A にしてください")
    bad({"node": "SEED"}, "READ_CARD の対象は Q か A にしてください")
    bad({"child": "E"}, "child が正しくありません")
    bad({"child": "S"}, "child が正しくありません")
    bad({"child": "O"}, "child が正しくありません")
    bad({"depth": "deep"}, "depth が正しくありません")
    bad({"mode": "quick"}, "mode が正しくありません")
    bad({"runner": "human"}, "runner が正しくありません")
    bad({"origin_run": 0})
    bad({"papers": list(range(1, 22)), "runner": "pi"}, "papers の指定が正しくありません")
    bad({"papers": [0]}, "run_id は 1〜999999999 の整数にしてください")


def test_calls(order: list) -> None:
    """中継が表どおりに CLI を呼ぶ（cwd・env・待ち時間・標準入力込みで）。"""
    project = papers.resolve_project("sai-paper")
    root = str(project["root"])
    paper, link, restore = swap(order)
    try:
        sci.status(project)
        assert paper.last["args"][1:] == ["status", "--json"], paper.last["args"]
        assert paper.last["cwd"] == root and paper.last["env"]["SAI_PROJECT_ROOT"] == root
        assert paper.last["env"]["SAI_ORIGIN"] == "phone", paper.last["env"]
        assert paper.last["timeout"] == 20.0 and paper.last["input_text"] is None

        sci.types(project)
        assert paper.last["args"][1:] == ["types", "--json"] and paper.last["timeout"] == 20.0

        paper.replies = [{"messages": ["ok"], "tree": {"type": "improve"}}]
        assert sci.init(project, {"type": "improve"}) == {"type": "improve"}
        assert paper.last["args"][1:] == ["init", "--type", "improve", "--json"]
        assert paper.last["timeout"] == 60.0
        # `tree` が無い出力は 502（画面に渡す木が作れない）
        paper.replies = [{"messages": [], "tree": None}]
        try:
            sci.init(project, {"type": "improve"})
            raise AssertionError("502 になるはず")
        except HTTPException as e:
            assert e.status_code == 502, e.status_code
        expect_400(lambda: sci.init(project, {"type": "unknown"}))

        sci.node(project, "A12")
        assert paper.last["args"][1:] == ["node", "A12", "--json"], paper.last["args"]
        expect_400(lambda: sci.node(project, "N12"))

        sci.apply(project, {"op": "add_q", "parent": "Q1", "text": "なぜ？"})
        assert paper.last["args"][1:] == ["apply", "--json"]
        assert json.loads(paper.last["input_text"]) == {
            "op": "add_q", "parent": "Q1", "text": "なぜ？",
        }
        assert paper.last["timeout"] == 30.0

        sci.create_card(
            project,
            {"node": "Q1", "block": "sci-read", "purpose": "目的", "papers": [37],
             "mode": "blind_first", "child": "A"},
        )
        assert paper.last["args"][1:] == ["card", "--from-json", "--json"]
        sent = json.loads(paper.last["input_text"])
        assert sent["mode"] == "blind_first" and sent["child"] == "A", sent

        sci.read(project, 130)
        assert paper.last["args"][1:] == ["read", "130", "--json"]

        sci.read_start(project, 130)
        assert link.last["args"] == ["card", "start", "130"] and link.last["root"] == root
    finally:
        restore()


def test_read_finish() -> None:
    """READ_CARD の終わりは、todo からでも 1 回で review まで進む。F が 0 でも止めない。"""
    project = papers.resolve_project("sai-paper")
    root = str(project["root"])
    card = {
        "run_id": 130, "owner": "todo",
        "findings": [{"id": "F1"}, {"id": "F2"}],
    }

    order: list = []
    _, link, restore = swap(order, [card])
    try:
        got = sci.read_finish(project, 130)
    finally:
        restore()
    assert got == {"run_id": 130, "status": "review", "findings": 2}, got
    assert [f"{k}: {a}" for k, a in order] == [
        "sci: read 130 --json", "link: card start 130", "link: card finish 130",
    ], order
    assert link.calls[0]["root"] == root and link.calls[1]["root"] == root

    # 始まっている札は start を呼ばない。F が 0 でも終わる
    order = []
    _, link, restore = swap(order, [{"owner": "running", "findings": []}])
    try:
        got = sci.read_finish(project, 130)
    finally:
        restore()
    assert got == {"run_id": 130, "status": "review", "findings": 0}, got
    assert [f"{k}: {a}" for k, a in order] == [
        "sci: read 130 --json", "link: card finish 130",
    ], order

    # run_id の検査
    order = []
    _, _, restore = swap(order)
    try:
        expect_400(lambda: sci.read_finish(project, 0))
        expect_400(lambda: sci.read_finish(project, "abc"))
        assert not order, "検査で落ちたときは CLI を呼ばない"
    finally:
        restore()


def test_failure_codes() -> None:
    """sai-sci の失敗は終了コードで 400 / 404 / 409 / 502 に読み替える（U14 で形 A を消した）。"""
    import subprocess

    project = papers.resolve_project("sai-paper")
    orig = cli.run_cli
    try:
        for code, status in ((2, 400), (3, 404), (4, 409), (1, 502)):
            cli.run_cli = (
                lambda *a, _c=code, **k: subprocess.CompletedProcess(a[0], _c, "", "理由")
            )
            try:
                sci.status(project)
                raise AssertionError(f"{status} になるはず")
            except HTTPException as e:
                assert e.status_code == status, (code, e.status_code)
                assert e.detail == "理由", e.detail
    finally:
        cli.run_cli = orig


def test_user_cli_args() -> None:
    """READ_CARD の 2 つの関数は `sai-link phone user …` を呼ぶ。

    `control._phone_json` の本物の作り（引数の先頭に `phone` を付ける・`missing` の
    扱い）は、偽の `run_cli` を通して確かめる。
    """
    import subprocess

    calls: list[dict] = []

    def fake_run_cli(args, **kw):
        calls.append({"args": list(args), **kw})
        return subprocess.CompletedProcess(list(args), 0, stdout='{"run_id": 130}', stderr="")

    orig = control.run_cli
    control.run_cli = fake_run_cli
    try:
        assert control.phone_user_start("/tmp/root", 130) == {"run_id": 130}
        assert control.phone_user_finish("/tmp/root", 130) == {"run_id": 130}
    finally:
        control.run_cli = orig
    assert [c["args"][1:] for c in calls] == [
        ["card", "start", "130"],
        ["card", "finish", "130"],
    ], calls
    assert all(c["cwd"] == "/tmp/root" for c in calls), calls
    assert all(c["env"]["SAI_PROJECT_ROOT"] == "/tmp/root" for c in calls), calls
    for bad in (0, -1, "130", None):
        expect_400(
            lambda value=bad: control.phone_user_start("/tmp/root", value),
            "run_id は 1 以上の整数にしてください",
        )
        expect_400(lambda value=bad: control.phone_user_finish("/tmp/root", value))


def test_routes() -> None:
    """9 本の住所があり、合言葉（require_token）で守られている。TRIAGE の住所は無い。"""
    from sai_web.main import app, require_token

    paths = [(route.path, sorted(getattr(route, "methods", None) or [])) for route in app.routes]
    base = "/api/projects/{name}/papers"
    wanted = [
        (f"{base}/sci/status", ["GET"]),
        (f"{base}/sci/types", ["GET"]),
        (f"{base}/sci/init", ["POST"]),
        (f"{base}/sci/nodes/{{node_id}}", ["GET"]),
        (f"{base}/sci/apply", ["POST"]),
        (f"{base}/sci/cards", ["POST"]),
        (f"{base}/sci/reads/{{run_id}}", ["GET"]),
        (f"{base}/sci/reads/{{run_id}}/start", ["POST"]),
        (f"{base}/sci/reads/{{run_id}}/finish", ["POST"]),
    ]
    assert len(wanted) == 9
    for path, method in wanted:
        assert (path, method) in paths, f"住所が無い: {method} {path}"
    assert not any(p.endswith("/sci/graph") for p, _ in paths), "/sci/graph は消した"
    sci_paths = {p for p, _ in paths if "/sci/" in p}
    assert len(sci_paths) == 9, f"research-tree の住所が 9 本でない: {sorted(sci_paths)}"

    wanted_paths = {p for p, _ in wanted}
    for route in app.routes:
        if route.path not in wanted_paths:
            continue
        guarded = any(
            isinstance(param.default, fastapi_params.Depends)
            and param.default.dependency is require_token
            for param in inspect.signature(route.endpoint).parameters.values()
        )
        assert guarded, f"合言葉が付いていない: {route.path}"


def main() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp)  # type: ignore[assignment]
        orig_list = control.list_projects
        control.list_projects = lambda: make_registry(base)
        papers._projects_cache.clear()
        try:
            test_ids()
            test_op_ok()
            test_op_ng()
            test_unknown_fields_dropped()
            test_card()
            test_calls([])
            test_read_finish()
            test_failure_codes()
            test_user_cli_args()
        finally:
            control.list_projects = orig_list
            papers._projects_cache.clear()
    test_routes()
    print("test_sci_api: すべて通りました")


if __name__ == "__main__":
    main()
