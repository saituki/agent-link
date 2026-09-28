"""sai-paper の `sai-moon-paper phone sci …`（研究方針の木）を中継する（U18 C-13）。

中の処理は Rust のコア、窓口は apps/phone だけ（AGENTS.md の「設計」）。
ここでは中の処理をせず、DB（`papr.db` など）を開くこともしない。
外から来る値は **CLI を呼ぶ前に** すべてここで検査し、通らない値では CLI を呼ばない（400）。

## 住所（C-13）

| 住所 | 本文 | 呼ぶ CLI |
|---|---|---|
| `GET /api/projects/{name}/papers/sci` | — | `phone sci status` |
| `POST /api/projects/{name}/papers/sci/init` | `{"type": "problem"}` | `phone sci init --type <型>` |
| `GET /api/projects/{name}/papers/sci/nodes/{node}` | — | `phone sci node <N>` |
| `POST /api/projects/{name}/papers/sci/apply` | C-07 の op の JSON | `phone sci apply`（標準入力） |
| `GET /api/projects/{name}/papers/sci/proposals` | — | `phone sci proposals` |
| `POST /api/projects/{name}/papers/sci/proposals/{pid}` | `{"action","note","edit"}` | `phone sci triage <P> --action … [--note …]`（edit は標準入力） |
| `POST /api/projects/{name}/papers/sci/cards` | カードの JSON | `phone sci card`（標準入力） |
| `GET /api/projects/{name}/papers/sci/reads/{run_id}` | — | `phone sci read <run_id>` |
| `POST /api/projects/{name}/papers/sci/reads/{run_id}/start` | — | `sai-moon-link phone user start <run_id>` |
| `POST /api/projects/{name}/papers/sci/reads/{run_id}/finish` | `{"no_reflect": "…"}`（任意） | `phone sci finish-read <run_id>` の後に `sai-moon-link phone user finish <run_id>` |

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

# --- 待ち時間（C-13） ---
# 読み取り（status / node / proposals / read）
SCI_READ_TIMEOUT = 20.0
# 書き込む（apply / triage / card / finish-read）
SCI_WRITE_TIMEOUT = 30.0
# 始めの 1 回（骨組みの展開で時間がかかる）
SCI_INIT_TIMEOUT = 60.0

# 反映しない理由の字数の上限（読む札を終えるとき・C-13）
NO_REFLECT_MAX = 500

# --- C-01 の ID と値 ---
# 節点 N / 問い Q / 知見 F / 結び付き L / 提案 P / 種 S（数だけ「12」は受け付けない）
_REF_TEMPLATES = {
    "N": re.compile(r"^N[1-9][0-9]{0,8}$"),
    "Q": re.compile(r"^Q[1-9][0-9]{0,8}$"),
    "F": re.compile(r"^F[1-9][0-9]{0,8}$"),
    "L": re.compile(r"^L[1-9][0-9]{0,8}$"),
    "P": re.compile(r"^P[1-9][0-9]{0,8}$"),
}
# 結び付きの相手（C-13）
_TARGET_RE = re.compile(
    r"^(paper:[1-9][0-9]{0,8}"
    r"|report:R-[0-9]{4,8}"
    r"|note:[1-9][0-9]{0,8}"
    r"|set:[1-9][0-9]{0,8}"
    r"|run:[1-9][0-9]{0,8}"
    r"|candidate:[A-Za-z0-9:._/-]{1,200})$"
)
# 研究の型（C-01）
SCI_TYPES = ("problem", "phenomenon", "method", "data", "improve")
# 項目の型（C-01）
ITEM_TYPES = ("choose", "list", "write", "verify")
# 知見の向き（C-01）
POLARITIES = ("support", "refute", "neutral", "none")
# 論文の役割（C-01）
ROLES = (
    "exemplar",
    "compare",
    "issue_evidence",
    "method_source",
    "data_source",
    "background",
    "counter",
)
# 論文の読み具合（C-01）
READ_STATES = ("unread", "read", "reflected", "irrelevant")
# 候補の状態（C-01）
OPTION_STATES = ("rejected", "dormant", "candidate")
# 線（`parent` は指定できない・C-07）
EDGE_KINDS = ("depends", "derived", "analogous")
# 範囲（C-01）
SCOPES = ("node", "path", "subtree", "siblings", "summary", "blind")
# 読む深さ（C-01）
DEPTHS = ("abstract", "figures", "full")
# 読む札の実行者（C-09）
RUNNERS = ("pi", "cc", "user")
# ガイドのキー（add_child の key）
_GUIDE_KEY_RE = re.compile(r"^[A-Za-z0-9._:-]{1,64}$")
# 仮定の 1 つ（C-13）
_ASSUME_RE = re.compile(r"^N[1-9][0-9]{0,8}=.{1,120}$")
# カード定義のブロック（C-13）
_CARD_BLOCK_RE = re.compile(r"^sci-[a-z-]{2,30}$")

# run_id の範囲（1〜999999999・C-13）
RUN_ID_MAX = 999_999_999
# 一度に送れる数の上限（C-07 / C-13）
CITES_MAX = 10
CITE_MAX_CHARS = 200
EVIDENCE_MAX = 20
REJECTED_MAX = 10
ASSUME_MAX = 5
CARD_PAPERS_MAX = 20
# 文字数の上限（C-07 の表）
TEXT_MAX = 1200
QUESTION_TEXT_MAX = 300
TITLE_MAX = 120
BODY_MAX = 2000
REASON_MAX = 1000
NOTE_MAX = 200
# 仕分けのメモの上限（C-13）
TRIAGE_NOTE_MAX = 1000

# --- C-07 の op ごとの項目 ---
# 値は「検査のしかた」。知らない項目は落とすので、1 つ 1 つ書く。
# `reason` は op ではなく共通（C-07 §共通）なので、全部の op に付ける。
_OP_FIELDS: dict[str, dict[str, str]] = {
    "add_finding": {
        "node": "ref:N",
        "question": "opt_ref:Q",
        "text": "text",
        "cites": "opt_cites",
        "polarity": "opt_polarity",
        "level": "opt_level",
        "paper": "opt_paper",
        "run": "opt_run",
    },
    "add_question": {"node": "ref:N", "text": "short_text"},
    "answer_question": {"question": "ref:Q", "text": "text"},
    "link": {
        "node": "ref:N",
        "target": "target",
        "role": "opt_role",
        "read_state": "opt_read_state",
    },
    "add_option": {"node": "ref:N", "title": "title", "body": "opt_body"},
    "add_child": {"node": "ref:N", "title": "title", "types": "types", "key": "opt_key"},
    "decide": {
        "node": "ref:N",
        "option": "opt_ref:N",
        "tentative": "opt_flag",
        "evidence": "opt_evidence",
        "rejected": "opt_rejected",
        "counter_searched": "opt_flag",
    },
    "reopen": {"node": "ref:N"},
    "exclude_item": {"node": "ref:N"},
    "option_state": {"node": "ref:N", "state": "state:option"},
    "exclude": {"target": "target", "node": "opt_ref:N", "title": "opt_title_max"},
    "edit_text": {"node": "ref:N", "title": "opt_title_max", "body": "opt_body"},
    "add_edge": {
        "kind": "kind",
        "from": "ref:N",
        "to": "ref:N",
        "note": "opt_note",
    },
    "unlink": {"link": "ref:L"},
    "remove_finding": {"finding": "ref:F"},
    "set_read_state": {"link": "ref:L", "state": "state:read"},
    "withdraw_question": {"question": "ref:Q"},
    "remove_edge": {"kind": "kind", "from": "ref:N", "to": "ref:N"},
}
# 種を足すのは提案と CLI の仕事（Web からはファイルのパスを受け取らないため使わない）
_WEB_DENIED_OPS = ("add_seed",)


def _bad(detail: str) -> HTTPException:
    """400 の例外を作る（理由は日本語で書く）。"""
    return HTTPException(status_code=400, detail=detail)


def _trim(value) -> str:
    """文字列の前後の空白を落とす。文字列でなければ 400。"""
    if not isinstance(value, str):
        raise _bad("文字列で送ってください")
    return value.strip()


def check_ref(value, prefix: str) -> str:
    """ID を検査して返す（`check_ref("N12", "N")` → `"N12"`）。違反は 400。

    節点 `^N[1-9][0-9]{0,8}$`・問い `^Q…`・知見 `^F…`・結び付き `^L…`・提案 `^P…`（C-13）。
    数だけ（`"12"`）は受け付けない。
    """
    pattern = _REF_TEMPLATES.get(prefix)
    if pattern is None or not isinstance(value, str):
        raise _bad("ID の形が正しくありません")
    text = value.strip()
    if not pattern.match(text):
        raise _bad("ID の形が正しくありません")
    return text


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


# --- op の検査（C-07 と C-13） ---


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


def _check_one(key: str, kind: str, value):
    """項目 1 つを検査して、CLI に渡す形にして返す（`opt_` は外した状態で受け取る）。"""
    if kind.startswith("ref:"):
        return check_ref(value, kind.split(":", 1)[1])
    if kind == "text":
        return _req_text({"text": value}, key, TEXT_MAX)
    if kind == "short_text":
        return _req_text({"text": value}, key, QUESTION_TEXT_MAX)
    if kind == "title":
        return _req_text({"title": value}, key, TITLE_MAX)
    if kind == "body":
        return _opt_text({"body": value}, key, BODY_MAX)
    if kind == "title_max":
        return _opt_text({"title": value}, key, TITLE_MAX)
    if kind == "note":
        return _opt_text({"note": value}, key, NOTE_MAX)
    if kind == "polarity":
        return _enum(value, POLARITIES, "polarity")
    if kind == "role":
        return _enum(value, ROLES, "role")
    if kind == "read_state":
        return _enum(value, READ_STATES, "read_state")
    if kind == "kind":
        return _enum(value, EDGE_KINDS, "kind")
    if kind.startswith("state:"):
        which = kind.split(":", 1)[1]
        if which == "option":
            return _enum(value, OPTION_STATES, "state")
        return _enum(value, READ_STATES, "state")
    if kind == "target":
        text = _trim(value)
        if not _TARGET_RE.match(text):
            raise _bad("target の形が正しくありません")
        return text
    if kind == "level":
        if isinstance(value, bool) or not isinstance(value, int) or not (1 <= value <= 4):
            raise _bad("level は 1〜4 の整数にしてください")
        return value
    if kind == "flag":
        if not isinstance(value, bool):
            raise _bad(f"{key} は真偽値にしてください")
        return value
    if kind in ("run", "paper"):
        return check_run_id(value)
    if kind == "cites":
        return _check_cites(value)
    if kind == "types":
        return _check_types(value)
    if kind == "key":
        text = _trim(value)
        if not _GUIDE_KEY_RE.match(text):
            raise _bad("key の形が正しくありません")
        return text
    if kind == "evidence":
        return _check_refs(value, "F", EVIDENCE_MAX, "evidence")
    if kind == "rejected":
        return _check_rejected(value)
    raise _bad("指定が正しくありません")


def _check_cites(value) -> list[str]:
    """引用（10 個まで・各 200 字まで）。"""
    if not isinstance(value, list) or len(value) > CITES_MAX:
        raise _bad("cites の指定が正しくありません")
    out: list[str] = []
    for item in value:
        if not isinstance(item, str) or not item.strip() or len(item) > CITE_MAX_CHARS:
            raise _bad("cites の指定が正しくありません")
        out.append(item.strip())
    return out


def _check_types(value) -> list[str]:
    """項目の型（1 つ以上・C-01）。"""
    if not isinstance(value, list) or not value:
        raise _bad("types の指定が正しくありません")
    out: list[str] = []
    for item in value:
        name = _enum(item, ITEM_TYPES, "types")
        if name in out:
            raise _bad("types の指定が正しくありません")
        out.append(name)
    return out


def _check_refs(value, prefix: str, limit: int, label: str) -> list[str]:
    """ID の配列（`evidence` は 20 個まで）。"""
    if not isinstance(value, list) or len(value) > limit:
        raise _bad(f"{label} の指定が正しくありません")
    return [check_ref(item, prefix) for item in value]


def _check_rejected(value) -> list[dict]:
    """却下する候補の配列（10 個まで・`{"option": "N21", "reason": "…"}`）。"""
    if not isinstance(value, list) or len(value) > REJECTED_MAX:
        raise _bad("rejected の指定が正しくありません")
    out: list[dict] = []
    for item in value:
        if not isinstance(item, dict):
            raise _bad("rejected の指定が正しくありません")
        option = check_ref(item.get("option"), "N")
        reason = _trim(item.get("reason"))
        if not reason:
            raise _bad("reason がありません")
        if len(reason) > REASON_MAX:
            raise _bad(f"reason は {REASON_MAX} 字までです")
        out.append({"option": option, "reason": reason})
    return out


def validate_op(payload) -> dict:
    """C-07 の op の JSON を検査して、CLI に渡す形の dict にして返す（違反は 400）。

    - `op` は C-07 の一覧のうち `add_seed` 以外（18 種類）。
    - op ごとに表の項目だけを残す。知らない項目と `path` のようなファイルのパスは落ちるので、
      CLI には渡らない（AC3）。
    - 文字列は前後の空白を落とす。必須の項目が無ければ 400「<項目> がありません」。
    """
    if not isinstance(payload, dict):
        raise _bad("送る JSON が正しくありません")
    op = payload.get("op")
    if op is None:
        raise _bad("op がありません")
    if not isinstance(op, str):
        raise _bad("op がありません")
    name = op.strip()
    if name in _WEB_DENIED_OPS:
        raise _bad(f"{name} は Web から使えません")
    fields = _OP_FIELDS.get(name)
    if fields is None:
        raise _bad("知らない操作です")

    out: dict = {"op": name}
    for key, kind in fields.items():
        raw = payload.get(key)
        if kind.startswith("opt_") and raw is None:
            continue
        if raw is None:
            raise _bad(f"{key} がありません")
        out[key] = _check_one(key, kind[4:] if kind.startswith("opt_") else kind, raw)
    # `reason` は共通項目（提案では必須。人の操作ではコアが決める）
    reason = _opt_text(payload, "reason", REASON_MAX)
    if reason is not None:
        out["reason"] = reason
    # `decide` の既定（C-07）
    if name == "decide":
        for key in ("tentative", "counter_searched"):
            if not isinstance(out.get(key), bool):
                out[key] = False
    return out


# --- カードの検査（C-09 / C-13） ---


def validate_card(payload) -> dict:
    """読む札の JSON を検査して、CLI に渡す形の dict にして返す（違反は 400）。

    - `block` は `^sci-[a-z-]{2,30}$`
    - `purpose` 1〜300 字、`expect` 300 字まで、`scope` は C-01
    - `assume` は 5 個まで・各 `^N[1-9][0-9]{0,8}=.{1,120}$`
    - `papers` は 20 個まで・各 1〜999999999
    - `runner` は pi / cc / user（既定 pi）、`role` と `depth` は C-01
    - `runner` が `user` なら `papers` は 1 つ・`role` 必須
    - ファイルのパスは受け取らない（知らない項目は落とす）
    """
    if not isinstance(payload, dict):
        raise _bad("送る JSON が正しくありません")

    block = _trim(payload.get("block"))
    if not _CARD_BLOCK_RE.match(block):
        raise _bad("block の形が正しくありません")

    node = check_ref(payload.get("node"), "N")
    purpose = _req_text(payload, "purpose", 300)

    out: dict = {
        "node": node,
        "block": block,
        "purpose": purpose,
        "runner": _enum(payload.get("runner", "pi"), RUNNERS, "runner"),
    }

    question = payload.get("question")
    if question is not None:
        out["question"] = check_ref(question, "Q")
    expect = _opt_text(payload, "expect", 300)
    if expect is not None:
        out["expect"] = expect
    scope = payload.get("scope")
    if scope is not None:
        out["scope"] = _enum(scope, SCOPES, "scope")
    depth = payload.get("depth")
    if depth is not None:
        out["depth"] = _enum(depth, DEPTHS, "depth")
    role = payload.get("role")
    if role is not None:
        out["role"] = _enum(role, ROLES, "role")

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
        for item in papers_value:
            papers_list.append(check_run_id(item))
    out["papers"] = papers_list

    if out["runner"] == "user":
        if len(papers_list) != 1:
            raise _bad("読む札は論文を 1 つだけ指定してください")
        if not out.get("role"):
            raise _bad("読む札には role が要ります")
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


# --- 中継（C-13 の住所の「呼ぶ CLI」の列） ---


def _sci_cli(project: dict, args: list[str], timeout: float, input_text: str | None = None):
    """`sai-moon-paper phone sci …` を呼ぶ（cwd と SAI_PAPER_ROOT はそのプロジェクトの root）。"""
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


def init(project: dict, payload) -> dict:
    """始める（`phone sci init --type <型>`・骨組みの展開で待つ時間は長め）。"""
    if not isinstance(payload, dict):
        raise _bad("送る JSON が正しくありません")
    kind = check_type(payload.get("type"))
    return _sci_cli(project, ["phone", "sci", "init", "--type", kind], SCI_INIT_TIMEOUT)


def node(project: dict, node_id) -> dict:
    """節点の中身（`phone sci node <N>`）。"""
    return _sci_cli(
        project, ["phone", "sci", "node", check_ref(node_id, "N")], SCI_READ_TIMEOUT
    )


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
    proposal = check_ref(pid, "P")
    action, note, edit = validate_triage(payload)
    args = ["phone", "sci", "triage", proposal, "--action", action]
    if note:
        args += ["--note", note]
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


def read_finish(project: dict, run_id, payload) -> dict:
    """読む札を終えて review へ移す（C-13・R-A4 と R-A5）。

    1. `no_reflect` があれば 1〜500 字。
    2. `phone sci read <run_id>` を見て、`owner` が `todo` なら先に始める
       （利用者が「始める」を押し忘れても、答えて終えるだけで済むようにするため・U18 決定）。
    3. `phone sci finish-read <run_id> [--no-reflect …]`。失敗（知見が 0 で理由が無いなど）は
       400 のまま返し、sai-link は呼ばない（R-A5 を守るため）。
    4. `sai-moon-link phone user finish <run_id>`。
    5. `{"run_id": N, "findings": n, "reflected": true, "status": "review"}` を返す。
    """
    rid = check_run_id(run_id)
    no_reflect = None
    if payload is not None:
        if not isinstance(payload, dict):
            raise _bad("送る JSON が正しくありません")
        raw = payload.get("no_reflect")
        if raw is not None:
            no_reflect = _trim(raw)
            if not no_reflect:
                raise _bad("no_reflect がありません")
            if len(no_reflect) > NO_REFLECT_MAX:
                raise _bad(f"no_reflect は {NO_REFLECT_MAX} 字までです")

    info = read(project, rid)
    owner = info.get("owner") if isinstance(info, dict) else None
    if owner == "todo":
        read_start(project, rid)

    args = ["phone", "sci", "finish-read", str(rid)]
    if no_reflect:
        args += ["--no-reflect", no_reflect]
    result = _sci_cli(project, args, SCI_WRITE_TIMEOUT)
    control.phone_user_finish(str(project["root"]), rid)

    findings = result.get("findings", 0) if isinstance(result, dict) else 0
    reflected = result.get("reflected", True) if isinstance(result, dict) else True
    return {
        "run_id": rid,
        "findings": findings if isinstance(findings, int) else 0,
        "reflected": bool(reflected),
        "status": "review",
    }
