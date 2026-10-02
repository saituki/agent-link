"""研究方針の木（Q・O・F・E）と検索セットの API のテスト。

確かめること:
- 中継が表どおりに CLI を呼ぶ（引数・標準入力・cwd・`SAI_PAPER_ROOT`・待ち時間）
- 検査に通らない値では CLI が 1 回も呼ばれず、400 と日本語の理由が返る
- 知らない項目と `path` のようなファイルのパスが CLI に渡らない
- 読む札の終わりは、始まっていない札でも 1 回の操作で review まで進む
- 15 本の住所が `main.app.routes` にあり、合言葉（`require_token`）で守られている

実行:
    cd <作業用コピー>
    PYTHONPATH=apps/phone/src apps/phone/.venv/bin/python apps/phone/tests/test_sci_api.py
"""

from __future__ import annotations

import inspect
import json
import tempfile
from pathlib import Path

from fastapi import HTTPException
from fastapi import params as fastapi_params

from sai_phone import cli, control, papers, sci


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
        # paper 側は先頭が実行ファイル・link 側（`phone user …` の引数）はそのまま
        self.trim = 1 if app == "paper" else 0

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
    """偽の CLI を差し込む。(paper, link, 元に戻す関数) を返す。"""
    paper = Recorder(order, replies=replies, app="paper")
    link = Recorder(order, app="link")
    orig = cli.run_json, control._phone_json
    cli.run_json, control._phone_json = paper, link

    def restore() -> None:
        cli.run_json, control._phone_json = orig

    return paper, link, restore


# C-OP の 13 op の通る例
OP_EXAMPLES = {
    "add_q": {"parent": "Q1", "text": "なぜ？", "guide": "g-1", "reason": "足す"},
    "add_o": {"q": "Q1", "text": "候補", "body": "説明", "kind": "issue"},
    "add_f": {
        "parent": "O2",
        "text": "速さが効いた。",
        "cites": ["#37.3.1 速さ", "#40", "#41.2-4"],
        "paper": 37,
        "report": "R-0012",
    },
    "add_e": {"parent": "O2", "text": "実験", "body": "b", "mlflow": "run/abc"},
    "record_logic": {"q": "Q1", "value": "contradiction", "note": "n", "findings": ["F1", "F2"]},
    "add_seed": {"title": "題", "sentence": "文", "path": ["O1", "O2"]},
    "copy": {"node": "O2", "to": "Q3"},
    "remove": {"node": "F4", "reason": "誤り"},
    "star": {"o": "O2"},
    "unstar": {"o": "O2"},
    "set_kind": {"o": "O2", "kind": "answer"},
    "edit": {"node": "O2", "text": "直した", "body": ""},
    "set_target": {"q": "Q1", "target": 3},
}


def test_ids() -> None:
    """ID と run_id の検査。"""
    assert sci.check_node("Q12") == "Q12" and sci.check_node(" O5 ") == "O5"
    assert sci.check_node("F31") == "F31" and sci.check_node("E2") == "E2"
    for bad in ("12", "Q0", "X1", "N1", "L1", "P1", "q1", "Q", "", "Q1000000000", None, 12):
        expect_400(lambda value=bad: sci.check_node(value), "ID の形が正しくありません")
    assert sci.check_proposal("P55") == "P55"
    for bad in ("Q1", "P0", "55", None):
        expect_400(lambda value=bad: sci.check_proposal(value), "ID の形が正しくありません")
    assert not hasattr(sci, "check_ref") and not hasattr(sci, "graph")

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
    """13 op が通り、前後の空白は落ち、body は空のまま送れる。"""
    assert set(OP_EXAMPLES) == set(sci._OP_FIELDS) and len(sci._OP_FIELDS) == 13
    for name, fields in OP_EXAMPLES.items():
        got = sci.validate_op({"op": name, **fields})
        for key, value in fields.items():
            assert got[key] == value, (name, key, got)
    assert sci.validate_op({"op": "star", "o": " O2 "}) == {"op": "star", "o": "O2"}
    assert sci.validate_op({"op": "add_q", "parent": "O1", "text": "x"})["parent"] == "O1"
    # 任意の項目は無くてよい
    assert sci.validate_op({"op": "add_o", "q": "Q1", "text": "候補"}) == {
        "op": "add_o", "q": "Q1", "text": "候補",
    }
    # target は null で外せる
    assert sci.validate_op({"op": "set_target", "q": "Q1", "target": None})["target"] is None
    assert sci.validate_op({"op": "set_target", "q": "Q1", "target": 0})["target"] == 0


def test_op_ng() -> None:
    """通らない op は 400。"""

    def bad(payload, detail: str = "") -> None:
        expect_400(lambda: sci.validate_op(payload), detail)

    bad({}, "op がありません")
    bad({"op": "unknown_op"}, "知らない操作です")
    bad({"op": "add_finding", "node": "O1", "text": "x"}, "知らない操作です")
    bad({"op": "star"}, "o がありません")
    bad({"op": "star", "o": "Q1"}, "ID の形が正しくありません")
    bad({"op": "star", "o": "12"}, "ID の形が正しくありません")
    bad({"op": "add_q", "parent": "F1", "text": "x"})
    bad({"op": "add_f", "parent": "Q1", "text": "x"})
    bad({"op": "add_e", "parent": "Q1", "text": "x"})
    bad({"op": "copy", "node": "Q1", "to": "Q2"})
    bad({"op": "copy", "node": "O1", "to": "O2"})
    bad({"op": "add_q", "parent": "Q1", "text": "あ" * 301})
    bad({"op": "add_o", "q": "Q1", "text": "あ" * 121})
    bad({"op": "add_o", "q": "Q1", "text": "x", "body": "あ" * 2001})
    bad({"op": "add_o", "q": "Q1", "text": "x", "kind": "other"})
    bad({"op": "add_f", "parent": "O1", "text": "あ" * 1201})
    bad({"op": "add_f", "parent": "O1", "text": "x", "cites": [f"#{i}" for i in range(1, 12)]})
    for cite in ("37", "#0", "#37.", "#37 " + "あ" * 81, "#37.0", "#a"):
        bad({"op": "add_f", "parent": "O1", "text": "x", "cites": [cite]})
    bad({"op": "add_f", "parent": "O1", "text": "x", "paper": 0})
    bad({"op": "add_e", "parent": "O1", "text": "x", "mlflow": "あ" * 201})
    bad({"op": "record_logic", "q": "Q1", "value": "maybe"})
    bad({"op": "record_logic", "q": "Q1", "value": "ok", "findings": ["O1"]})
    bad({"op": "record_logic", "q": "Q1", "value": "ok", "note": "あ" * 1001})
    bad({"op": "add_seed", "title": "題", "sentence": "文", "path": []})
    bad({"op": "add_seed", "title": "題", "sentence": "文", "path": ["Q1"]})
    bad({"op": "add_seed", "title": "題", "sentence": "文"}, "path がありません")
    bad({"op": "add_seed", "title": "題", "sentence": "あ" * 601, "path": ["O1"]})
    bad({"op": "remove", "node": "O1"}, "reason がありません")
    bad({"op": "remove", "node": "O1", "reason": "あ" * 501})
    bad({"op": "star", "o": "O1", "reason": "あ" * 501})
    bad({"op": "set_kind", "o": "O1", "kind": "x"})
    bad({"op": "edit", "node": "O1"}, "text か body のどちらかが要ります")
    bad({"op": "edit", "node": "O1", "text": "あ" * 1201})
    bad({"op": "set_target", "q": "Q1"}, "target がありません")
    for target in (100, -1, True, "3", 1.5):
        bad({"op": "set_target", "q": "Q1", "target": target})


def test_unknown_fields_dropped() -> None:
    """知らない項目と `path` のようなファイルのパスは落ちている。"""
    got = sci.validate_op(
        {"op": "add_q", "parent": "Q1", "text": "なぜ？", "path": "/etc/passwd",
         "proposal": "P55", "actor": "run:130"}
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
    assert got == {**full, "purpose": "答えの候補を出す"}, got
    ok_user = {"node": "O1", "block": "sci-read", "purpose": "目的", "papers": [37], "runner": "user"}
    assert sci.validate_card(ok_user)["runner"] == "user"
    assert sci.validate_card({**ok_user, "mode": "blind_first"})["mode"] == "blind_first"
    assert "mode" not in sci.validate_card(ok_user)

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
    bad({"scope": "everything"}, "scope が正しくありません")
    bad({"depth": "deep"}, "depth が正しくありません")
    bad({"mode": "quick"}, "mode が正しくありません")
    bad({"runner": "human"}, "runner が正しくありません")
    bad({"origin_run": 0})
    bad({"assume": [f"Q{i}=あ" for i in range(1, 7)]}, "assume の指定が正しくありません")
    bad({"assume": ["N1=あ"]}, "assume の指定が正しくありません")
    bad({"assume": ["Q1="]}, "assume の指定が正しくありません")
    bad({"papers": list(range(1, 22)), "runner": "pi"}, "papers の指定が正しくありません")
    bad({"papers": [0]}, "run_id は 1〜999999999 の整数にしてください")


def test_triage() -> None:
    """仕分けの検査。"""
    assert sci.validate_triage({"action": "accept"}) == ("accept", "", None)
    assert sci.validate_triage({"action": "reject", "note": " 違う "}) == ("reject", "違う", None)
    action, note, edit = sci.validate_triage(
        {"action": "accept", "note": "直して採用",
         "edit": {"op": "add_q", "parent": "Q1", "text": "なぜ？"}}
    )
    assert (action, note) == ("accept", "直して採用")
    assert edit == {"op": "add_q", "parent": "Q1", "text": "なぜ？"}
    for bad in ({}, {"action": "keep"}, {"action": "accept", "note": "あ" * 1001},
                {"action": "accept", "edit": {"op": "unknown_op"}}):
        expect_400(lambda value=bad: sci.validate_triage(value))


def test_calls(order: list) -> None:
    """中継が表どおりに CLI を呼ぶ（cwd・env・待ち時間・標準入力込みで）。"""
    project = papers.resolve_project("sai-paper")
    root = str(project["root"])
    paper, link, restore = swap(order)
    try:
        sci.status(project)
        assert paper.last["args"][1:] == ["phone", "sci", "status"], paper.last["args"]
        assert paper.last["cwd"] == root and paper.last["env"]["SAI_PAPER_ROOT"] == root
        assert paper.last["timeout"] == 20.0 and paper.last["input_text"] is None

        sci.types(project)
        assert paper.last["args"][1:] == ["phone", "sci", "types"] and paper.last["timeout"] == 20.0

        sci.init(project, {"type": "improve"})
        assert paper.last["args"][1:] == ["phone", "sci", "init", "--type", "improve"]
        assert paper.last["timeout"] == 60.0
        expect_400(lambda: sci.init(project, {"type": "unknown"}))

        sci.node(project, "O12")
        assert paper.last["args"][1:] == ["phone", "sci", "node", "O12"], paper.last["args"]
        expect_400(lambda: sci.node(project, "N12"))

        sci.apply(project, {"op": "add_q", "parent": "Q1", "text": "なぜ？"})
        assert paper.last["args"][1:] == ["phone", "sci", "apply"]
        assert json.loads(paper.last["input_text"]) == {
            "op": "add_q", "parent": "Q1", "text": "なぜ？",
        }
        assert paper.last["timeout"] == 30.0

        sci.proposals(project)
        assert paper.last["args"][1:] == ["phone", "sci", "proposals"]

        sci.triage(project, "P55", {"action": "reject", "note": "--help"})
        assert paper.last["args"][1:] == [
            "phone", "sci", "triage", "P55", "--action", "reject", "--note=--help",
        ], paper.last["args"]
        assert paper.last["input_text"] == ""
        sci.triage(project, "P56", {"action": "accept", "edit": {"op": "star", "o": "O1"}})
        assert paper.last["args"][1:] == ["phone", "sci", "triage", "P56", "--action", "accept"]
        assert json.loads(paper.last["input_text"]) == {"op": "star", "o": "O1"}
        expect_400(lambda: sci.triage(project, "N55", {"action": "reject"}))
        expect_400(lambda: sci.triage(project, "P55", {"action": "keep"}))

        sci.create_card(
            project,
            {"node": "Q1", "block": "sci-read", "purpose": "目的", "papers": [37],
             "mode": "blind_first"},
        )
        assert paper.last["args"][1:] == ["phone", "sci", "card"]
        assert json.loads(paper.last["input_text"])["mode"] == "blind_first"

        sci.read(project, 130)
        assert paper.last["args"][1:] == ["phone", "sci", "read", "130"]

        sci.read_start(project, 130)
        assert link.last["args"] == ["user", "start", "130"] and link.last["root"] == root

        # 検索セット
        sci.set_list(project)
        assert paper.last["args"][1:] == ["phone", "set", "list"] and paper.last["timeout"] == 20.0
        sci.set_show(project, "7")
        assert paper.last["args"][1:] == ["phone", "set", "show", "7"], paper.last["args"]
        sci.set_mark(project, 7, {"positions": [0, 2, 5], "decision": "keep", "reason": " 要る "})
        assert paper.last["args"][1:] == [
            "phone", "set", "mark", "7", "--position=0,2,5", "--decision", "keep", "--reason=要る",
        ], paper.last["args"]
        assert paper.last["timeout"] == 30.0 and paper.last["cwd"] == root
        sci.set_mark(project, 7, {"positions": [1], "decision": "drop"})
        assert paper.last["args"][1:] == [
            "phone", "set", "mark", "7", "--position=1", "--decision", "drop",
        ], paper.last["args"]
        sci.set_drop_pending(project, 7, {"reason": "古い"})
        assert paper.last["args"][1:] == ["phone", "set", "drop-pending", "7", "--reason=古い"]
        sci.set_drop_pending(project, 7, {})
        assert paper.last["args"][1:] == ["phone", "set", "drop-pending", "7"]

        count = len(paper.calls)
        for bad in (
            lambda: sci.set_show(project, 0),
            lambda: sci.set_show(project, "x"),
            lambda: sci.set_mark(project, 7, {"positions": [], "decision": "keep"}),
            lambda: sci.set_mark(project, 7, {"positions": list(range(501)), "decision": "keep"}),
            lambda: sci.set_mark(project, 7, {"positions": [-1], "decision": "keep"}),
            lambda: sci.set_mark(project, 7, {"positions": [True], "decision": "keep"}),
            lambda: sci.set_mark(project, 7, {"positions": [1], "decision": "pending"}),
            lambda: sci.set_mark(
                project, 7, {"positions": [1], "decision": "keep", "reason": "あ" * 201}
            ),
            lambda: sci.set_mark(project, 7, [1]),
            lambda: sci.set_drop_pending(project, 7, {"reason": "あ" * 201}),
            lambda: sci.set_drop_pending(project, 1_000_000_000, {}),
        ):
            expect_400(bad)
        assert len(paper.calls) == count, "検査で落ちたときは CLI を呼ばない"
    finally:
        restore()


def test_read_finish() -> None:
    """読む札の終わりは、todo からでも 1 回で review まで進む。F が 0 でも止めない。"""
    project = papers.resolve_project("sai-paper")
    root = str(project["root"])
    card = {
        "run_id": 130, "owner": "todo",
        "findings": [{"id": "F1"}, {"id": "F2"}], "debts": ["メモが無い"],
    }

    order: list = []
    _, link, restore = swap(order, [card])
    try:
        got = sci.read_finish(project, 130)
    finally:
        restore()
    assert got == {"run_id": 130, "status": "review", "findings": 2, "debts": ["メモが無い"]}, got
    assert [f"{k}: {a}" for k, a in order] == [
        "paper: phone sci read 130", "link: user start 130", "link: user finish 130",
    ], order
    assert link.calls[0]["root"] == root and link.calls[1]["root"] == root

    # 始まっている札は start を呼ばない。F が 0 でも終わる
    order = []
    _, link, restore = swap(order, [{"owner": "running", "findings": [], "debts": []}])
    try:
        got = sci.read_finish(project, 130)
    finally:
        restore()
    assert got == {"run_id": 130, "status": "review", "findings": 0, "debts": []}, got
    assert [f"{k}: {a}" for k, a in order] == [
        "paper: phone sci read 130", "link: user finish 130",
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


def test_user_cli_args() -> None:
    """読む札の 2 つの関数は `sai-moon-link phone user …` を呼ぶ。

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
        ["phone", "user", "start", "130"],
        ["phone", "user", "finish", "130"],
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
    """15 本の住所があり、合言葉（require_token）で守られている。"""
    from sai_phone.main import app, require_token

    paths = [(route.path, sorted(getattr(route, "methods", None) or [])) for route in app.routes]
    base = "/api/projects/{name}/papers"
    wanted = [
        (f"{base}/sci/status", ["GET"]),
        (f"{base}/sci/types", ["GET"]),
        (f"{base}/sci/init", ["POST"]),
        (f"{base}/sci/nodes/{{node_id}}", ["GET"]),
        (f"{base}/sci/apply", ["POST"]),
        (f"{base}/sci/proposals", ["GET"]),
        (f"{base}/sci/proposals/{{pid}}", ["POST"]),
        (f"{base}/sci/cards", ["POST"]),
        (f"{base}/sci/reads/{{run_id}}", ["GET"]),
        (f"{base}/sci/reads/{{run_id}}/start", ["POST"]),
        (f"{base}/sci/reads/{{run_id}}/finish", ["POST"]),
        (f"{base}/sets", ["GET"]),
        (f"{base}/sets/{{set_id}}", ["GET"]),
        (f"{base}/sets/{{set_id}}/mark", ["POST"]),
        (f"{base}/sets/{{set_id}}/drop-pending", ["POST"]),
    ]
    assert len(wanted) == 15
    for path, method in wanted:
        assert (path, method) in paths, f"住所が無い: {method} {path}"
    assert not any(p.endswith("/sci/graph") for p, _ in paths), "/sci/graph は消した"

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
        papers._projects_cache = None
        try:
            test_ids()
            test_op_ok()
            test_op_ng()
            test_unknown_fields_dropped()
            test_card()
            test_triage()
            test_calls([])
            test_read_finish()
            test_user_cli_args()
        finally:
            control.list_projects = orig_list
            papers._projects_cache = None
    test_routes()
    print("test_sci_api: すべて通りました")


if __name__ == "__main__":
    main()
