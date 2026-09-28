"""研究方針の木（sai-sci）の API のテスト（U18 C-13）。

確かめること:
- 10 の中継が C-13 の表どおりに CLI を呼ぶ（引数・標準入力・cwd・`SAI_PAPER_ROOT`・待ち時間）
- 検査に通らない値では CLI が 1 回も呼ばれず、400 と日本語の理由が返る
- 知らない項目と `path` のようなファイルのパスが CLI に渡らない
- 読む札の終わりは、始まっていない札でも 1 回の操作で review まで進む
- 10 本の住所が `main.app.routes` にあり、合言葉（`require_token`）で守られている

実行:
    cd <作業用コピー>
    PYTHONPATH=apps/phone/src /Users/touta/workdir/Sai/sai-moon/apps/phone/.venv/bin/python \
        apps/phone/tests/test_sci_api.py
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

    - `order` に「どちらを・どの引数で」呼んだか積む（2 つの時計の順を確かめるため）。
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


def test_ids() -> None:
    """ID と run_id の検査（C-13）。"""
    assert sci.check_ref("N12", "N") == "N12"
    assert sci.check_ref("Q7", "Q") == "Q7"
    assert sci.check_ref("F31", "F") == "F31"
    assert sci.check_ref("L3", "L") == "L3"
    assert sci.check_ref("P55", "P") == "P55"
    for bad in ("12", "N0", "X1", "n1", "N", "", "N1000000000", None, 12):
        expect_400(lambda value=bad: sci.check_ref(value, "N"), "ID の形が正しくありません")

    assert sci.check_run_id(130) == 130
    assert sci.check_run_id("130") == 130
    for bad in (0, -1, 1_000_000_000, "0", "x", None, True, 1.5):
        expect_400(
            lambda value=bad: sci.check_run_id(value),
            "run_id は 1〜999999999 の整数にしてください",
        )

    assert sci.check_type("problem") == "problem"
    for bad in ("unknown", "", "課題", None):
        expect_400(lambda value=bad: sci.check_type(value), "type が正しくありません")


def test_op_ok() -> None:
    """C-07 の op の検査（通るもの）。"""
    assert sci.validate_op({"op": "add_question", "node": "N1", "text": "なぜ？"}) == {
        "op": "add_question",
        "node": "N1",
        "text": "なぜ？",
    }
    # 前後の空白を落とす
    assert sci.validate_op({"op": "reopen", "node": " N1 "}) == {"op": "reopen", "node": "N1"}
    # decide の既定
    assert sci.validate_op({"op": "decide", "node": "N1", "option": "N2"}) == {
        "op": "decide",
        "node": "N1",
        "option": "N2",
        "tentative": False,
        "counter_searched": False,
    }
    # cites / level / run / paper
    got = sci.validate_op(
        {
            "op": "add_finding",
            "node": "N1",
            "text": "HDR を測る。速さが効いた。",
            "cites": ["#37 p.3", "#40"],
            "polarity": "support",
            "level": 2,
            "paper": 37,
            "run": 130,
            "reason": "論文 #37 を読んだ",
        }
    )
    assert got["cites"] == ["#37 p.3", "#40"] and got["level"] == 2
    assert got["paper"] == 37 and got["run"] == 130 and got["reason"] == "論文 #37 を読んだ"
    # link / 線 / 却下候補
    assert sci.validate_op(
        {"op": "link", "node": "N1", "target": "paper:37", "role": "exemplar", "read_state": "read"}
    )["target"] == "paper:37"
    assert sci.validate_op(
        {"op": "add_edge", "kind": "depends", "from": "N1", "to": "N2", "note": "前提"}
    )["note"] == "前提"
    assert sci.validate_op(
        {
            "op": "decide",
            "node": "N1",
            "tentative": True,
            "evidence": ["F3", "F4"],
            "rejected": [{"option": "N21", "reason": " weak "}],
            "counter_searched": True,
        }
    )["rejected"] == [{"option": "N21", "reason": "weak"}]
    # 種以外の op は全部通る（18 種類）
    assert len(sci._OP_FIELDS) == 18, len(sci._OP_FIELDS)
    for name, fields in sci._OP_FIELDS.items():
        payload = {"op": name}
        for key, kind in fields.items():
            if kind == "ref:N":
                payload[key] = "N1"
            elif kind == "ref:Q":
                payload[key] = "Q1"
            elif kind == "ref:F":
                payload[key] = "F1"
            elif kind == "ref:L":
                payload[key] = "L1"
            elif kind == "ref:P":
                payload[key] = "P1"
            elif kind == "target":
                payload[key] = "paper:37"
            elif kind == "text" or kind == "short_text":
                payload[key] = "本文"
            elif kind == "title":
                payload[key] = "題"
            elif kind == "opt_ref:N":
                payload[key] = "N2"
            elif kind == "polarity":
                payload[key] = "support"
            elif kind == "role":
                payload[key] = "exemplar"
            elif kind == "read_state" or kind == "state:read":
                payload[key] = "read"
            elif kind == "state:option":
                payload[key] = "rejected"
            elif kind == "kind":
                payload[key] = "depends"
            elif kind == "level":
                payload[key] = 1
            elif kind == "flag":
                payload[key] = False
            elif kind == "run":
                payload[key] = 1
            elif kind == "opt_paper":
                payload[key] = 37
            elif kind == "cites":
                payload[key] = ["#37"]
            elif kind == "types":
                payload[key] = ["choose"]
            elif kind == "key":
                payload[key] = "guide"
            elif kind == "evidence":
                payload[key] = ["F1"]
            elif kind == "rejected":
                payload[key] = [{"option": "N21", "reason": "弱い"}]
            elif kind.startswith("opt_"):
                continue
            else:  # pragma: no cover - 検査の増やし忘れを中专時に見つける
                raise AssertionError(f"未知の検査: {kind}")
        sci.validate_op(payload)


def test_op_ng() -> None:
    """検査に通らない op は 400（理由も日本語）。"""
    def bad(payload, detail: str = "") -> None:
        expect_400(lambda: sci.validate_op(payload), detail)

    bad({}, "op がありません")
    bad({"node": "N1"}, "op がありません")
    bad({"op": "unknown_op"}, "知らない操作です")
    bad(
        {"op": "add_seed", "title": "題", "sentence": "文", "path": ["N1"]},
        "add_seed は Web から使えません",
    )
    # 形の違う ID
    bad({"op": "reopen", "node": "12"}, "ID の形が正しくありません")
    bad({"op": "reopen", "node": "N0"}, "ID の形が正しくありません")
    bad({"op": "reopen", "node": "X1"}, "ID の形が正しくありません")
    bad({"op": "reopen"}, "node がありません")
    bad({"op": "answer_question", "question": "N1", "text": "答"})
    # 字数の超過
    bad({"op": "add_question", "node": "N1", "text": "あ" * 301})
    bad({"op": "add_finding", "node": "N1", "text": "あ" * 1201})
    bad({"op": "edit_text", "node": "N1", "body": "あ" * 2001})
    bad({"op": "add_option", "node": "N1", "title": "あ" * 121, "body": "b"})
    bad({"op": "add_edge", "kind": "depends", "from": "N1", "to": "N2", "note": "あ" * 201})
    bad({"op": "add_finding", "node": "N1", "text": "本文", "reason": "あ" * 1001})
    # cites は 10 個まで
    bad({"op": "add_finding", "node": "N1", "text": "本文", "cites": [f"#{i}" for i in range(11)]})
    bad({"op": "add_finding", "node": "N1", "text": "本文", "cites": ["あ" * 201]})
    # target の形
    for bad_target in ("paper:0", "note:x", "run:0", "unknown:1", "paper:37 extra", "/etc/passwd"):
        bad({"op": "link", "node": "N1", "target": bad_target}, "target の形が正しくありません")
    # 値の一覧
    bad({"op": "link", "node": "N1", "target": "paper:37", "role": "unknown"})
    bad({"op": "add_finding", "node": "N1", "text": "本文", "polarity": "unknown"})
    bad({"op": "add_finding", "node": "N1", "text": "本文", "level": 5})
    bad({"op": "add_finding", "node": "N1", "text": "本文", "level": True})
    bad({"op": "option_state", "node": "N1", "state": "selected"})
    bad({"op": "add_edge", "kind": "parent", "from": "N1", "to": "N2"})
    bad({"op": "add_child", "node": "N1", "title": "題", "types": ["unknown"]})
    bad({"op": "add_child", "node": "N1", "title": "題", "types": []})
    bad({"op": "decide", "node": "N1", "tentative": "yes"})
    bad({"op": "add_finding", "node": "N1", "text": "本文", "run": 0})
    bad({"op": "decide", "node": "N1", "evidence": [f"F{i}" for i in range(21)]})
    bad(
        {
            "op": "decide",
            "node": "N1",
            "rejected": [{"option": f"N{i}", "reason": "弱い"} for i in range(11)],
        }
    )
    bad({"op": "decide", "node": "N1", "rejected": [{"option": "N21"}]})
    bad({"op": "decide", "node": "N1", "rejected": [{"option": "21", "reason": "弱い"}]})
    bad({"op": "unlink", "link": "Q1"})


def test_unknown_fields_dropped() -> None:
    """知らない項目と `path` のようなファイルのパスは落ちている（AC3）。"""
    got = sci.validate_op(
        {
            "op": "add_question",
            "node": "N1",
            "text": "なぜ？",
            "path": "/etc/passwd",
            "proposal": "P55",
            "actor": "run:130",
        }
    )
    assert got == {"op": "add_question", "node": "N1", "text": "なぜ？"}, got
    # add_seed の `doc`（waggle 参照）も持たない
    card = sci.validate_card(
        {
            "node": "N1",
            "block": "sci-read",
            "purpose": "目的",
            "papers": [37],
            "path": "/etc/passwd",
            "doc": "waggle:AXANSMpo",
        }
    )
    assert "path" not in card and "doc" not in card, card
    assert card["papers"] == [37] and card["runner"] == "pi", card


def test_card() -> None:
    """カードの検査（C-09 / C-13）。"""
    got = sci.validate_card(
        {
            "node": "N12",
            "block": "sci-read",
            "purpose": "  この論文で HDR の話我校証できる  ",
            "question": "Q7",
            "expect": "速さが効いた",
            "scope": "subtree",
            "assume": ["N1=速さ", "N2=圧縮"],
            "papers": [37, 40],
            "role": "exemplar",
            "depth": "full",
        }
    )
    assert got == {
        "node": "N12",
        "block": "sci-read",
        "purpose": "この論文で HDR の話我校証できる",
        "question": "Q7",
        "expect": "速さが効いた",
        "scope": "subtree",
        "assume": ["N1=速さ", "N2=圧縮"],
        "papers": [37, 40],
        "runner": "pi",
        "role": "exemplar",
        "depth": "full",
    }
    # runner が user なら論文は 1 つ・role が要る
    ok_user = {
        "node": "N1",
        "block": "sci-read",
        "purpose": "目的",
        "papers": [37],
        "runner": "user",
        "role": "compare",
    }
    assert sci.validate_card(ok_user)["runner"] == "user"
    expect_400(lambda: sci.validate_card({**ok_user, "papers": [37, 40]}),
               "読む札は論文を 1 つだけ指定してください")
    expect_400(lambda: sci.validate_card({k: v for k, v in ok_user.items() if k != "role"}),
               "読む札には role が要ります")
    expect_400(lambda: sci.validate_card({**ok_user, "purpose": ""}),
               "purpose がありません")
    # 形の違う項目
    expect_400(lambda: sci.validate_card({**ok_user, "block": "sci-Read"}),
               "block の形が正しくありません")
    expect_400(lambda: sci.validate_card({**ok_user, "purpose": "あ" * 301}))
    expect_400(lambda: sci.validate_card({**ok_user, "expect": "あ" * 301}))
    expect_400(lambda: sci.validate_card({**ok_user, "node": "12"}),
               "ID の形が正しくありません")
    expect_400(lambda: sci.validate_card({**ok_user, "question": "Q1x"}),
               "ID の形が正しくありません")
    expect_400(lambda: sci.validate_card({**ok_user, "scope": "everything"}),
               "scope が正しくありません")
    expect_400(lambda: sci.validate_card({**ok_user, "depth": "deep"}),
               "depth が正しくありません")
    expect_400(lambda: sci.validate_card({**ok_user, "assume": [f"N{i}=あ" for i in range(6)]}),
               "assume の指定が正しくありません")
    expect_400(lambda: sci.validate_card({**ok_user, "assume": ["1=あ"]}),
               "assume の指定が正しくありません")
    expect_400(lambda: sci.validate_card({**ok_user, "papers": list(range(1, 22))}),
               "papers の指定が正しくありません")
    expect_400(lambda: sci.validate_card({**ok_user, "papers": [0]}),
               "run_id は 1〜999999999 の整数にしてください")
    expect_400(lambda: sci.validate_card({**ok_user, "runner": "human"}),
               "runner が正しくありません")


def test_triage() -> None:
    """仕分けの検査（C-13）。"""
    assert sci.validate_triage({"action": "accept"}) == ("accept", "", None)
    assert sci.validate_triage({"action": "reject", "note": " 違う "}) == ("reject", "違う", None)
    action, note, edit = sci.validate_triage(
        {
            "action": "accept",
            "note": "直して採用",
            "edit": {"op": "add_question", "node": "N1", "text": "なぜ？"},
        }
    )
    assert action == "accept" and note == "直して採用"
    assert edit == {"op": "add_question", "node": "N1", "text": "なぜ？"}
    for bad in ({}, {"action": "keep"}, {"action": "accept", "note": "あ" * 1001},
                {"action": "accept", "edit": {"op": "unknown_op"}}):
        expect_400(lambda value=bad: sci.validate_triage(value))


def test_calls(order: list) -> None:
    """10 の中継が C-13 の表どおりに CLI を呼ぶ（cwd・env・待ち時間込みで）。"""
    project = papers.resolve_project("sai-paper")
    root = str(project["root"])
    paper = Recorder(order, app="paper")
    link = Recorder(order, app="link")
    orig_json, orig_phone_json = cli.run_json, control._phone_json
    cli.run_json, control._phone_json = paper, link
    try:
        # 1. 木全体（読み取りは 20 秒）
        sci.status(project)
        assert paper.last["args"][1:] == ["phone", "sci", "status"], paper.last["args"]
        assert paper.last["cwd"] == root, paper.last["cwd"]
        assert paper.last["env"]["SAI_PAPER_ROOT"] == root, paper.last["env"]
        assert paper.last["timeout"] == 20.0, paper.last["timeout"]
        assert paper.last["input_text"] is None, paper.last["input_text"]

        # 2. 始める（60 秒）
        sci.init(project, {"type": "improve"})
        assert paper.last["args"][1:] == ["phone", "sci", "init", "--type", "improve"]
        assert paper.last["timeout"] == 60.0, paper.last
        expect_400(lambda: sci.init(project, {"type": "unknown"}))

        # 3. 節点
        sci.node(project, "N12")
        assert paper.last["args"][1:] == ["phone", "sci", "node", "N12"], paper.last["args"]
        expect_400(lambda: sci.node(project, "12"))

        # 4. apply（本文は標準入力）
        sci.apply(project, {"op": "add_question", "node": "N1", "text": "なぜ？"})
        assert paper.last["args"][1:] == ["phone", "sci", "apply"], paper.last["args"]
        assert json.loads(paper.last["input_text"]) == {
            "op": "add_question",
            "node": "N1",
            "text": "なぜ？",
        }, paper.last["input_text"]
        assert paper.last["timeout"] == 30.0, paper.last

        # 5. proposals
        sci.proposals(project)
        assert paper.last["args"][1:] == ["phone", "sci", "proposals"], paper.last["args"]

        # 6. triage（edit は標準入力・無ければ空文字）
        sci.triage(project, "P55", {"action": "reject", "note": "違う"})
        assert paper.last["args"][1:] == [
            "phone", "sci", "triage", "P55", "--action", "reject", "--note", "違う",
        ], paper.last["args"]
        assert paper.last["input_text"] == "", paper.last["input_text"]
        sci.triage(project, "P56", {"action": "accept", "edit": {"op": "reopen", "node": "N1"}})
        assert paper.last["args"][1:] == ["phone", "sci", "triage", "P56", "--action", "accept"]
        assert json.loads(paper.last["input_text"]) == {"op": "reopen", "node": "N1"}
        expect_400(lambda: sci.triage(project, "P55", {"action": "keep"}))

        # 7. カード（本文は標準入力）
        sci.create_card(
            project, {"node": "N1", "block": "sci-read", "purpose": "目的", "papers": [37]}
        )
        assert paper.last["args"][1:] == ["phone", "sci", "card"], paper.last["args"]
        assert json.loads(paper.last["input_text"])["block"] == "sci-read"

        # 8. 読む札の中身
        sci.read(project, 130)
        assert paper.last["args"][1:] == ["phone", "sci", "read", "130"], paper.last["args"]

        # 9. 読む札を始める（sai-link）
        sci.read_start(project, 130)
        assert link.last["args"] == ["user", "start", "130"], link.last["args"]
        assert link.last["root"] == root, link.last["root"]
        assert link.last["missing"] == "未実行のカードが見つかりません", link.last["missing"]
    finally:
        cli.run_json, control._phone_json = orig_json, orig_phone_json


def test_read_finish() -> None:
    """読む札を終える（始まっていなくても 1 回の操作で review まで進む・C-13）。"""
    project = papers.resolve_project("sai-paper")
    root = str(project["root"])

    def run(replies: list) -> tuple[list, Recorder, Recorder]:
        order: list = []
        paper = Recorder(order, replies=replies, app="paper")
        link = Recorder(order, app="link")
        orig_json, orig_phone_json = cli.run_json, control._phone_json
        cli.run_json, control._phone_json = paper, link
        try:
            got = sci.read_finish(project, 130, {"no_reflect": "  関係なし  "})
        finally:
            cli.run_json, control._phone_json = orig_json, orig_phone_json
        return got, order, paper, link

    # owner が todo なら start → finish-read → finish の順
    got, order, paper, link = run(
        [
            {"run_id": 130, "owner": "todo"},
            {"run_id": 130, "findings": 2, "reflected": True},
        ]
    )
    assert got == {"run_id": 130, "findings": 2, "reflected": True, "status": "review"}, got
    assert [f"{kind}: {args}" for kind, args in order] == [
        "paper: phone sci read 130",
        "link: user start 130",
        "paper: phone sci finish-read 130 --no-reflect 関係なし",
        "link: user finish 130",
    ], order
    assert link.last["args"] == ["user", "finish", "130"], link.last["args"]
    assert link.last["missing"] == "実行が見つかりません", link.last["missing"]
    assert link.calls[0]["root"] == root and link.calls[1]["root"] == root

    # 始まっている札（owner が running）は start を呼ばない
    _, order, _, link = run([{"owner": "running"}, {"findings": 1, "reflected": True}])
    assert [f"{kind}: {args}" for kind, args in order] == [
        "paper: phone sci read 130",
        "paper: phone sci finish-read 130 --no-reflect 関係なし",
        "link: user finish 130",
    ], order
    assert len(link.calls) == 1, link.calls

    # 理由が無いときは `--no-reflect` を付けない
    order2: list = []
    orig_json, orig_phone_json = cli.run_json, control._phone_json
    paper2 = Recorder(
        order2, replies=[{"owner": "running"}, {"findings": 1, "reflected": True}], app="paper"
    )
    link2 = Recorder(order2, app="link")
    cli.run_json, control._phone_json = paper2, link2
    try:
        got = sci.read_finish(project, 130, {})
    finally:
        cli.run_json, control._phone_json = orig_json, orig_phone_json
    assert [f"{kind}: {args}" for kind, args in order2] == [
        "paper: phone sci read 130",
        "paper: phone sci finish-read 130",
        "link: user finish 130",
    ], order2
    assert got == {"run_id": 130, "findings": 1, "reflected": True, "status": "review"}, got

    # finish-read が 400 なら sai-link を呼ばない（R-A5）
    order = []
    paper = Recorder(
        order,
        replies=[
            {"owner": "running"},
            HTTPException(
                status_code=400, detail="知見が 1 つもありません。反映しない理由を書いてください"
            ),
            HTTPException(status_code=400, detail="別の失敗"),
        ],
        app="paper",
    )
    link = Recorder(order, app="link")
    orig_json, orig_phone_json = cli.run_json, control._phone_json
    cli.run_json, control._phone_json = paper, link
    try:
        expect_400(
            lambda: sci.read_finish(project, 130, {}),
            "知見が 1 つもありません。反映しない理由を書いてください",
        )
        expect_400(lambda: sci.read_finish(project, 130, {}), "別の失敗")
    finally:
        cli.run_json, control._phone_json = orig_json, orig_phone_json
    # read のあとに落ちるので、link は 1 回も呼ばない
    assert not link.calls, link.calls

    # 理由の検査（長さ・型）
    orig_json, orig_phone_json = cli.run_json, control._phone_json
    order3: list = []
    guard = Recorder(order3, app="paper")
    guard_link = Recorder(order3, app="link")
    cli.run_json = guard
    control._phone_json = guard_link
    try:
        expect_400(lambda: sci.read_finish(project, 130, {"no_reflect": "あ" * 501}))
        expect_400(lambda: sci.read_finish(project, 130, {"no_reflect": " "}))
        expect_400(lambda: sci.read_finish(project, 130, {"no_reflect": 3}))
        expect_400(lambda: sci.read_finish(project, 0, {}))
        expect_400(lambda: sci.read_finish(project, "abc", {}))
        assert not order3, "検査で落ちたときは CLI を呼ばない"
    finally:
        cli.run_json, control._phone_json = orig_json, orig_phone_json


def test_user_cli_args() -> None:
    """読む札の 2 つの関数は `sai-moon-link phone user …` を呼ぶ（C-12）。

    `control._phone_json` の本物の作り（引数の先頭に `phone` を付ける・`missing` の
    扱い）は、偽の `run_cli` を通して確かめる。
    """
    import subprocess

    calls: list[dict] = []

    def fake_run_cli(args, **kw):
        calls.append({"args": list(args), **kw})
        return subprocess.CompletedProcess(
            list(args), 0, stdout='{"run_id": 130}', stderr=""
        )

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
    # run_id の検査（`cli` は呼ばない）
    for bad in (0, -1, "130", None):
        expect_400(
            lambda value=bad: control.phone_user_start("/tmp/root", value),
            "run_id は 1 以上の整数にしてください",
        )
        expect_400(lambda value=bad: control.phone_user_finish("/tmp/root", value))


def test_routes() -> None:
    """10 本の住所があり、合言葉（require_token）で守られている（C-13・AC1）。"""
    from sai_phone.main import app, require_token

    paths = [(route.path, sorted(getattr(route, "methods", None) or [])) for route in app.routes]
    wanted = [
        ("/api/projects/{name}/papers/sci", ["GET"]),
        ("/api/projects/{name}/papers/sci/init", ["POST"]),
        ("/api/projects/{name}/papers/sci/nodes/{node}", ["GET"]),
        ("/api/projects/{name}/papers/sci/apply", ["POST"]),
        ("/api/projects/{name}/papers/sci/proposals", ["GET"]),
        ("/api/projects/{name}/papers/sci/proposals/{pid}", ["POST"]),
        ("/api/projects/{name}/papers/sci/cards", ["POST"]),
        ("/api/projects/{name}/papers/sci/reads/{run_id}", ["GET"]),
        ("/api/projects/{name}/papers/sci/reads/{run_id}/start", ["POST"]),
        ("/api/projects/{name}/papers/sci/reads/{run_id}/finish", ["POST"]),
    ]
    for path, method in wanted:
        assert (path, method) in paths, f"住所が無い: {method} {path}"

    # どの住所も合言葉を通す（401・AC1）
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
