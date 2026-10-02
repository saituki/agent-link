"""sai-paper の `sai-moon-paper phone sci …` と `phone set …`（研究方針の木・検索セット）を中継する。

中の処理は Rust のコア、窓口は apps/phone だけ（AGENTS.md の「設計」）。
ここでは中の処理をせず、DB（`papr.db` など）を開くこともしない。
外から来る値は **CLI を呼ぶ前に** すべてここで検査し、通らない値では CLI を呼ばない（400）。

## 住所（`main.py` に並べる）

| 住所 | 本文 | 呼ぶ CLI |
|---|---|---|
| `GET …/papers/sci/status` | — | `phone sci status` |
| `GET …/papers/sci/types` | — | `phone sci types` |
| `POST …/papers/sci/init` | `{"type": "problem"}` | `phone sci init --type <型>` |
| `GET …/papers/sci/nodes/{node_id}` | — | `phone sci node <ID>` |
| `POST …/papers/sci/apply` | C-OP の op の JSON | `phone sci apply`（標準入力） |
| `GET …/papers/sci/proposals` | — | `phone sci proposals` |
| `POST …/papers/sci/proposals/{pid}` | `{"action","note","edit"}` | `phone sci triage <P> --action …`（edit は標準入力） |
| `POST …/papers/sci/cards` | C-CARD の JSON | `phone sci card`（標準入力） |
| `GET …/papers/sci/reads/{run_id}` | — | `phone sci read <run_id>` |
| `POST …/papers/sci/reads/{run_id}/start` | — | `sai-moon-link phone user start <run_id>` |
| `POST …/papers/sci/reads/{run_id}/finish` | — | `sai-moon-link phone user finish <run_id>` |
| `GET …/papers/sets` | — | `phone set list` |
| `GET …/papers/sets/{set_id}` | — | `phone set show <id>` |
| `POST …/papers/sets/{set_id}/mark` | `{"positions","decision","reason"?}` | `phone set mark <id> --position … --decision …` |
| `POST …/papers/sets/{set_id}/drop-pending` | `{"reason"?}` | `phone set drop-pending <id>` |

- どの住所も合言葉（`main.require_token`）で守る。
- `{name}` は論文の置き場があるプロジェクトの名前（`papers.resolve_project`）。
  CLI は cwd と `SAI_PAPER_ROOT` をそのプロジェクトの root にして呼ぶ（`papers._paper_cli` と同じ）。
- **ファイルのパスは iPhone から受け取らない。** 知らない項目は落としてから渡す。
"""

from __future__ import annotations

import json
import os
import re

from fastapi import HTTPException

from sai_phone import cli, control
from sai_phone.config import settings

# --- 待ち時間 ---
# 読み取り（status / types / node / proposals / read / set）
SCI_READ_TIMEOUT = 20.0
# 書き込む（apply / triage / card / set mark）
SCI_WRITE_TIMEOUT = 30.0
# 始めの 1 回（骨組みの展開で時間がかかる）
SCI_INIT_TIMEOUT = 60.0

# --- ID と値 ---
# Node は Q・O・F・E、提案は P（数だけ「12」は受け付けない）
_NODE_RE = re.compile(r"^[QOFE][1-9][0-9]{0,8}$")
_PROPOSAL_RE = re.compile(r"^P[1-9][0-9]{0,8}$")
# 研究の型
SCI_TYPES = ("problem", "phenomenon", "method", "data", "improve")
# 範囲・読む深さ・読む札の実行者と読み方
SCOPES = ("node", "path", "subtree", "siblings", "summary", "blind")
DEPTHS = ("abstract", "figures", "full")
RUNNERS = ("pi", "cc", "user")
CARD_MODES = ("purpose_first", "blind_first")
# O の種類・論理の判定
ANSWER_KINDS = ("answer", "issue")
LOGIC_VALUES = ("ok", "contradiction")
# 引用 C-CITE
_CITE_RE = re.compile(
    r"^#([1-9][0-9]{0,8})((?:\.[1-9][0-9]*){1,2}(?:-[1-9][0-9]*)?)?( .{1,80})?$"
)
# ガイドのキー・報告書の ID
_GUIDE_KEY_RE = re.compile(r"^[A-Za-z0-9._:-]{1,64}$")
_REPORT_RE = re.compile(r"^R-[0-9]{4,8}$")
# 仮定の 1 つ
_ASSUME_RE = re.compile(r"^[QOFE][1-9][0-9]{0,8}=.{1,120}$")
# カード定義のブロック
_CARD_BLOCK_RE = re.compile(r"^sci-[a-z-]{2,30}$")

# 数の上限
RUN_ID_MAX = 999_999_999
CITES_MAX = 10
FINDINGS_MAX = 20
PATH_MAX = 20
ASSUME_MAX = 5
CARD_PAPERS_MAX = 20
SET_POSITIONS_MAX = 500
SET_POSITION_MAX = 999_999
# 文字数の上限
REASON_MAX = 500
BODY_MAX = 2000
TRIAGE_NOTE_MAX = 1000
SET_REASON_MAX = 200

# --- C-OP の op ごとの項目 ---
# 値は「検査のしかた」。知らない項目は落とすので、1 つ 1 つ書く。
#   ref:<種類>   必須の ID（種類は Q・O・F・E の文字の並び）
#   oref:<種類>  任意の ID
#   text:<n>     必須の文字（1〜n 字）
#   otext:<n>    任意の文字（空なら無いものとして落とす）
#   body         任意の本文（0〜2000 字。空のまま送れる）
#   enum:<名前>  必須の値   oenum:<名前>  任意の値
#   他は _check_one を見る
# `reason` は全部の op に付く共通の項目（`remove` だけ必須）。
_OP_FIELDS: dict[str, dict[str, str]] = {
    "add_q": {"parent": "ref:QO", "text": "text:300", "guide": "oguide"},
    "add_o": {"q": "ref:Q", "text": "text:120", "body": "body", "kind": "oenum:answer"},
    "add_f": {
        "parent": "ref:OE",
        "text": "text:1200",
        "cites": "ocites",
        "paper": "opaper",
        "report": "oreport",
        "run": "orun",
    },
    "add_e": {"parent": "ref:OE", "text": "text:120", "body": "body", "mlflow": "otext:200"},
    "record_logic": {
        "q": "ref:Q",
        "value": "enum:logic",
        "note": "otext:1000",
        "findings": "ofindings",
    },
    "add_seed": {"title": "text:120", "sentence": "text:600", "path": "seedpath"},
    "copy": {"node": "ref:O", "to": "ref:Q"},
    "remove": {"node": "ref:QOFE"},
    "star": {"o": "ref:O"},
    "unstar": {"o": "ref:O"},
    "set_kind": {"o": "ref:O", "kind": "enum:answer"},
    "edit": {"node": "ref:QOFE", "text": "otext:1200", "body": "body"},
    "set_target": {"q": "ref:Q", "target": "target"},
}
_ENUMS = {"answer": ANSWER_KINDS, "logic": LOGIC_VALUES}


def _bad(detail: str) -> HTTPException:
    """400 の例外を作る（理由は日本語で書く）。"""
    return HTTPException(status_code=400, detail=detail)


def _trim(value) -> str:
    """文字列の前後の空白を落とす。文字列でなければ 400。"""
    if not isinstance(value, str):
        raise _bad("文字列で送ってください")
    return value.strip()


def check_node(value) -> str:
    """Node の ID（`Q12`・`O5`・`F31`・`E2`）を検査して返す。違反は 400。数だけは受けない。"""
    if not isinstance(value, str) or not _NODE_RE.match(value.strip()):
        raise _bad("ID の形が正しくありません")
    return value.strip()


def check_proposal(value) -> str:
    """提案の ID（`P55`）を検査して返す。違反は 400。"""
    if not isinstance(value, str) or not _PROPOSAL_RE.match(value.strip()):
        raise _bad("ID の形が正しくありません")
    return value.strip()


def _check_ref(value, kinds: str) -> str:
    """Node の ID で、種類が `kinds` の文字のどれかであるもの。"""
    node = check_node(value)
    if node[0] not in kinds:
        raise _bad("ID の形が正しくありません")
    return node


def check_run_id(value) -> int:
    """run_id を 1〜999999999 の整数にして返す（数字の文字列も受ける）。違反は 400。"""
    if isinstance(value, bool):
        raise _bad("run_id は 1〜999999999 の整数にしてください")
    if isinstance(value, int):
        number = value
    elif isinstance(value, str) and re.fullmatch(r"[0-9]{1,9}", value.strip()):
        number = int(value.strip())
    else:
        raise _bad("run_id は 1〜999999999 の整数にしてください")
    if not (1 <= number <= RUN_ID_MAX):
        raise _bad("run_id は 1〜999999999 の整数にしてください")
    return number


def check_type(value) -> str:
    """研究の型を検査して返す（C-01 の 5 つ）。違反は 400。"""
    if not isinstance(value, str) or value.strip() not in SCI_TYPES:
        raise _bad("type が正しくありません")
    return value.strip()


# --- op の検査（C-OP） ---


def _enum(value, allowed: tuple[str, ...], label: str) -> str:
    text = _trim(value)
    if text not in allowed:
        raise _bad(f"{label} が正しくありません")
    return text


def _opt_text(payload: dict, key: str, max_chars: int) -> str | None:
    """任意の文字（前後の空白を落とす。空なら無いものとして落とす）。"""
    raw = payload.get(key)
    if raw is None:
        return None
    text = _trim(raw)
    if not text:
        return None
    if len(text) > max_chars:
        raise _bad(f"{key} は {max_chars} 字までです")
    return text


def _req_text(payload: dict, key: str, max_chars: int) -> str:
    """必須の文字（空なら 400「<key> がありません」）。"""
    raw = payload.get(key)
    if raw is None:
        raise _bad(f"{key} がありません")
    text = _trim(raw)
    if not text:
        raise _bad(f"{key} がありません")
    if len(text) > max_chars:
        raise _bad(f"{key} は {max_chars} 字までです")
    return text


def _check_cites(value) -> list[str]:
    """引用（10 個まで・C-CITE の形）。"""
    if not isinstance(value, list) or len(value) > CITES_MAX:
        raise _bad("cites の指定が正しくありません")
    out: list[str] = []
    for item in value:
        if not isinstance(item, str) or not _CITE_RE.match(item.strip()):
            raise _bad("cites の指定が正しくありません")
        out.append(item.strip())
    return out


def _check_id_list(value, kinds: str, limit: int, label: str, minimum: int = 0) -> list[str]:
    """ID の配列。"""
    if not isinstance(value, list) or not (minimum <= len(value) <= limit):
        raise _bad(f"{label} の指定が正しくありません")
    return [_check_ref(item, kinds) for item in value]


def _check_target(value):
    """目標の本数（0〜99 の整数か null）。"""
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int) or not (0 <= value <= 99):
        raise _bad("target は 0〜99 の整数か null にしてください")
    return value


def _check_one(key: str, kind: str, value):
    """項目 1 つを検査して、CLI に渡す形にして返す。"""
    name, _, arg = kind.partition(":")
    if name == "ref":
        return _check_ref(value, arg)
    if name == "text":
        return _req_text({key: value}, key, int(arg))
    if name == "otext":
        return _opt_text({key: value}, key, int(arg))
    if name == "body":
        text = _trim(value)
        if len(text) > BODY_MAX:
            raise _bad(f"{key} は {BODY_MAX} 字までです")
        return text
    if name in ("enum", "oenum"):
        return _enum(value, _ENUMS[arg], key)
    if name == "oguide":
        text = _trim(value)
        if not _GUIDE_KEY_RE.match(text):
            raise _bad("guide の形が正しくありません")
        return text
    if name == "ocites":
        return _check_cites(value)
    if name in ("opaper", "orun"):
        return check_run_id(value)
    if name == "oreport":
        text = _trim(value)
        if not _REPORT_RE.match(text):
            raise _bad("report の形が正しくありません")
        return text
    if name == "ofindings":
        return _check_id_list(value, "F", FINDINGS_MAX, key)
    if name == "seedpath":
        return _check_id_list(value, "O", PATH_MAX, key, minimum=1)
    if name == "target":
        return _check_target(value)
    raise _bad("指定が正しくありません")


def validate_op(payload) -> dict:
    """C-OP の op の JSON を検査して、CLI に渡す形の dict にして返す（違反は 400）。

    - `op` は C-OP の 13 種類（Web からもすべて送れる）。
    - op ごとに表の項目だけを残す。知らない項目と `path` のようなファイルのパスは落ちるので、
      CLI には渡らない（`add_seed` の `path` だけは O の ID の配列で、検査して渡す）。
    - 文字列は前後の空白を落とす。必須の項目が無ければ 400「<項目> がありません」。
    """
    if not isinstance(payload, dict):
        raise _bad("送る JSON が正しくありません")
    op = payload.get("op")
    if not isinstance(op, str):
        raise _bad("op がありません")
    name = op.strip()
    fields = _OP_FIELDS.get(name)
    if fields is None:
        raise _bad("知らない操作です")

    out: dict = {"op": name}
    for key, kind in fields.items():
        raw = payload.get(key)
        # `ref:`・`enum:`・`text:`・`seedpath`・`target` は必須、ほかは任意
        optional = kind.split(":")[0] in (
            "oenum", "otext", "body", "oguide", "ocites", "opaper", "orun", "oreport", "ofindings"
        )
        if raw is None and kind == "target" and key in payload:
            out[key] = None  # 目標を外す（null）
            continue
        if raw is None:
            if optional:
                continue
            raise _bad(f"{key} がありません")
        value = _check_one(key, kind, raw)
        if value is not None:
            out[key] = value
    # `reason`: `remove` は必須 1〜500、ほかは 0〜500
    if name == "remove":
        out["reason"] = _req_text(payload, "reason", REASON_MAX)
    else:
        reason = _opt_text(payload, "reason", REASON_MAX)
        if reason is not None:
            out["reason"] = reason
    if name == "edit" and "text" not in out and "body" not in out:
        raise _bad("text か body のどちらかが要ります")
    return out


# --- カードの検査（C-CARD） ---


def validate_card(payload) -> dict:
    """読む札の JSON を検査して、CLI に渡す形の dict にして返す（違反は 400）。

    - `node`（Q・O・F・E）・`block`（`^sci-[a-z-]{2,30}$`）・`purpose`（1〜300 字、1 行）は必須
    - `expect` 300 字、`scope`・`depth`・`mode`・`runner` は表の値、`origin_run` は run_id
    - `assume` は 5 個まで・各 `^[QOFE][1-9][0-9]{0,8}=.{1,120}$`
    - `papers` は 20 個まで・各 1〜999999999
    - `runner` が `user` なら `papers` はちょうど 1 つ
    - `question`・`role`・`executer` は受けない。ファイルのパスも受けない（知らない項目は落とす）
    """
    if not isinstance(payload, dict):
        raise _bad("送る JSON が正しくありません")

    block = _trim(payload.get("block"))
    if not _CARD_BLOCK_RE.match(block):
        raise _bad("block の形が正しくありません")
    purpose = _req_text(payload, "purpose", 300)
    if "\n" in purpose or "\r" in purpose:
        raise _bad("purpose は 1 行にしてください")

    out: dict = {
        "node": check_node(payload.get("node")),
        "block": block,
        "purpose": purpose,
        "runner": _enum(payload.get("runner", "pi"), RUNNERS, "runner"),
    }
    expect = _opt_text(payload, "expect", 300)
    if expect is not None:
        out["expect"] = expect
    for key, allowed in (("scope", SCOPES), ("depth", DEPTHS), ("mode", CARD_MODES)):
        if payload.get(key) is not None:
            out[key] = _enum(payload[key], allowed, key)
    if payload.get("origin_run") is not None:
        out["origin_run"] = check_run_id(payload["origin_run"])

    raw_assume = payload.get("assume")
    if raw_assume is not None:
        if not isinstance(raw_assume, list) or len(raw_assume) > ASSUME_MAX:
            raise _bad("assume の指定が正しくありません")
        assume: list[str] = []
        for item in raw_assume:
            text = _trim(item)
            if not _ASSUME_RE.match(text):
                raise _bad("assume の指定が正しくありません")
            assume.append(text)
        out["assume"] = assume

    papers_value = payload.get("papers")
    papers_list: list[int] = []
    if papers_value is not None:
        if not isinstance(papers_value, list) or len(papers_value) > CARD_PAPERS_MAX:
            raise _bad("papers の指定が正しくありません")
        papers_list = [check_run_id(item) for item in papers_value]
    out["papers"] = papers_list

    if out["runner"] == "user" and len(papers_list) != 1:
        raise _bad("READ_CARD は論文を 1 つだけ指定してください")
    return out


# --- 仕分けの検査（C-13） ---


def validate_triage(payload) -> tuple[str, str, dict | None]:
    """仕分けの JSON を検査して `(action, note, edit)` を返す（違反は 400）。

    - `action` は accept / reject / revert の 3 つ
    - `note` は 1000 字まで
    - `edit` があれば C-07 の op として検査する（無いなら None）
    """
    if not isinstance(payload, dict):
        raise _bad("送る JSON が正しくありません")
    action = _enum(payload.get("action"), ("accept", "reject", "revert"), "action")
    note = _opt_text(payload, "note", TRIAGE_NOTE_MAX) or ""
    raw_edit = payload.get("edit")
    edit = None
    if raw_edit is not None:
        edit = validate_op(raw_edit)
    return action, note, edit


# --- 中継 ---


def _sci_cli(project: dict, args: list[str], timeout: float, input_text: str | None = None):
    """`sai-moon-paper phone …` を呼ぶ（cwd と SAI_PAPER_ROOT はそのプロジェクトの root）。"""
    root = str(project["root"])
    return cli.run_json(
        [settings.sai_paper_bin, *args],
        app="sai-paper",
        cwd=root,
        env={**os.environ, "SAI_PAPER_ROOT": root},
        timeout=timeout,
        input_text=input_text,
    )


def status(project: dict) -> dict:
    """研究方針の木全体（`phone sci status`）。"""
    return _sci_cli(project, ["phone", "sci", "status"], SCI_READ_TIMEOUT)


def types(project: dict) -> dict:
    """使える研究の型の一覧（`phone sci types`）。"""
    return _sci_cli(project, ["phone", "sci", "types"], SCI_READ_TIMEOUT)


def init(project: dict, payload) -> dict:
    """始める（`phone sci init --type <型>`・骨組みの展開で待つ時間は長め）。"""
    if not isinstance(payload, dict):
        raise _bad("送る JSON が正しくありません")
    kind = check_type(payload.get("type"))
    return _sci_cli(project, ["phone", "sci", "init", "--type", kind], SCI_INIT_TIMEOUT)


def node(project: dict, node_id) -> dict:
    """Node の中身（`phone sci node <ID>`）。"""
    return _sci_cli(project, ["phone", "sci", "node", check_node(node_id)], SCI_READ_TIMEOUT)


def apply(project: dict, payload) -> dict:
    """人の操作を反映する（`phone sci apply`・本文は標準入力）。"""
    data = validate_op(payload)
    return _sci_cli(
        project,
        ["phone", "sci", "apply"],
        SCI_WRITE_TIMEOUT,
        input_text=json.dumps(data, ensure_ascii=False),
    )


def proposals(project: dict) -> dict:
    """仕分け待ちの一覧（`phone sci proposals`）。"""
    return _sci_cli(project, ["phone", "sci", "proposals"], SCI_READ_TIMEOUT)


def triage(project: dict, pid, payload) -> dict:
    """仕分ける（`phone sci triage <P> --action … [--note …]`・edit は標準入力）。

    `edit` が無ければ標準入力には空文字を渡す（コアが「空なら無し」と見る）。
    """
    proposal = check_proposal(pid)
    action, note, edit = validate_triage(payload)
    args = ["phone", "sci", "triage", proposal, "--action", action]
    if note:
        # 値が `--` で始まっていてもオプションと誤解されないように 1 つの引数にする
        args += [f"--note={note}"]
    return _sci_cli(
        project,
        args,
        SCI_WRITE_TIMEOUT,
        input_text=json.dumps(edit, ensure_ascii=False) if edit is not None else "",
    )


def create_card(project: dict, payload) -> dict:
    """読む札を作る（`phone sci card`・本文は標準入力）。"""
    data = validate_card(payload)
    return _sci_cli(
        project,
        ["phone", "sci", "card"],
        SCI_WRITE_TIMEOUT,
        input_text=json.dumps(data, ensure_ascii=False),
    )


def read(project: dict, run_id) -> dict:
    """読む札の中身（`phone sci read <run_id>`）。"""
    return _sci_cli(
        project, ["phone", "sci", "read", str(check_run_id(run_id))], SCI_READ_TIMEOUT
    )


def read_start(project: dict, run_id) -> dict:
    """読む札を始める（`sai-moon-link phone user start <run_id>`・C-12）。"""
    return control.phone_user_start(str(project["root"]), check_run_id(run_id))


def read_finish(project: dict, run_id) -> dict:
    """読む札を終えて review へ移す。

    `read` で owner を見て、`todo` なら先に始める（利用者が「始める」を押し忘れても 1 回で済ませる）。
    F が 0 でも止めない（足りないものは DEBT として残る）。
    """
    rid = check_run_id(run_id)
    info = read(project, rid)
    if not isinstance(info, dict):
        info = {}
    if info.get("owner") == "todo":
        read_start(project, rid)
    control.phone_user_finish(str(project["root"]), rid)
    findings = info.get("findings")
    debts = info.get("debts")
    return {
        "run_id": rid,
        "status": "review",
        "findings": len(findings) if isinstance(findings, list) else 0,
        "debts": debts if isinstance(debts, list) else [],
    }


# --- 検索セット ---


def check_set_id(value) -> int:
    """セット ID を 1〜999999999 の整数にして返す（数字の文字列も受ける）。違反は 400。"""
    try:
        return check_run_id(value)
    except HTTPException:
        raise _bad("set_id は 1〜999999999 の整数にしてください") from None


def _check_set_reason(payload) -> str | None:
    if payload is None:
        return None
    if not isinstance(payload, dict):
        raise _bad("送る JSON が正しくありません")
    return _opt_text(payload, "reason", SET_REASON_MAX)


def set_list(project: dict) -> dict:
    """検索セットの一覧（`phone set list`）。"""
    return _sci_cli(project, ["phone", "set", "list"], SCI_READ_TIMEOUT)


def set_show(project: dict, set_id) -> dict:
    """検索セット 1 つと候補（`phone set show <id>`）。"""
    return _sci_cli(project, ["phone", "set", "show", str(check_set_id(set_id))], SCI_READ_TIMEOUT)


def set_mark(project: dict, set_id, payload) -> dict:
    """候補に keep / drop を付ける（`phone set mark <id> --position … --decision …`）。"""
    sid = check_set_id(set_id)
    if not isinstance(payload, dict):
        raise _bad("送る JSON が正しくありません")
    positions = payload.get("positions")
    if not isinstance(positions, list) or not (1 <= len(positions) <= SET_POSITIONS_MAX):
        raise _bad("positions の指定が正しくありません")
    for item in positions:
        if isinstance(item, bool) or not isinstance(item, int) or not (0 <= item <= SET_POSITION_MAX):
            raise _bad("positions の指定が正しくありません")
    decision = _enum(payload.get("decision"), ("keep", "drop"), "decision")
    reason = _check_set_reason(payload)
    args = [
        "phone", "set", "mark", str(sid),
        f"--position={','.join(str(n) for n in positions)}",
        "--decision", decision,
    ]
    if reason:
        args.append(f"--reason={reason}")
    return _sci_cli(project, args, SCI_WRITE_TIMEOUT)


def set_drop_pending(project: dict, set_id, payload=None) -> dict:
    """未判定の候補をまとめて drop にする（`phone set drop-pending <id>`）。"""
    sid = check_set_id(set_id)
    reason = _check_set_reason(payload)
    args = ["phone", "set", "drop-pending", str(sid)]
    if reason:
        args.append(f"--reason={reason}")
    return _sci_cli(project, args, SCI_WRITE_TIMEOUT)
