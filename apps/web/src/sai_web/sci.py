"""研究の `sai-sci …` を中継する。

中の処理は Rust のコア、窓口は apps/web だけ（AGENTS.md の「設計」）。
ここでは中の処理をせず、DB（`papr.db` など）を開くこともしない。
外から来る値は **CLI を呼ぶ前に** すべてここで検査し、通らない値では CLI を呼ばない（400）。

## 住所（`main.py` に並べる）

| 住所 | 本文 | 呼ぶ CLI |
|---|---|---|
| `GET …/papers/sci/status` | — | `sai-sci status --json` |
| `GET …/papers/sci/types` | — | `sai-sci types --json` |
| `POST …/papers/sci/init` | `{"type": "problem"}` | `sai-sci init --type <型> --json` |
| `GET …/papers/sci/nodes/{node_id}` | — | `sai-sci node <ID> --json` |
| `POST …/papers/sci/apply` | C-OP の op の JSON | `sai-sci apply --json`（標準入力） |
| `POST …/papers/sci/cards` | C-CARD の JSON | `sai-sci card --from-json --json`（標準入力） |
| `GET …/papers/sci/reads/{run_id}` | — | `sai-sci read <run_id> --json` |
| `POST …/papers/sci/reads/{run_id}/start` | — | `sai-link phone user start <run_id>` |
| `POST …/papers/sci/reads/{run_id}/finish` | — | `sai-link phone user finish <run_id>` |

- どの住所も合言葉（`main.require_token`）で守る。
- `{name}` は論文の置き場があるプロジェクトの名前（`papers.resolve_project`）。
  CLI は cwd と `SAI_PROJECT_ROOT` をそのプロジェクトの root にして呼ぶ（`papers._paper_cli` と同じ）。
- **ファイルのパスは iPhone から受け取らない。** 知らない項目は落としてから渡す。
"""

from __future__ import annotations

import json
import os
import re

from fastapi import HTTPException

from sai_web import cli, control
from sai_web.config import settings

# --- 待ち時間 ---
# 読み取り（status / types / node / read / set）
SCI_READ_TIMEOUT = 20.0
# 書き込む（apply / card）
SCI_WRITE_TIMEOUT = 30.0
# 始めの 1 回（骨組みの展開で時間がかかる）
SCI_INIT_TIMEOUT = 60.0

# --- ID と値 ---
# Node は Q・A・F・E・S・D（数だけ「12」は受け付けない）。SEED は ID を持たないので別に扱う
_NODE_RE = re.compile(r"^[QAFESD][1-9][0-9]{0,8}$")
# RESEARCH_TYPE
SCI_TYPES = ("problem", "phenomenon", "method", "data", "improve")
# 読む深さ・READ_CARD の実行者と読み方
DEPTHS = ("abstract", "figures", "full")
RUNNERS = ("cc", "user")
# CARD の model・thinking（省略可。空は block の既定に任せる）
CARD_MODELS = ("opus", "sonnet", "haiku")
CARD_THINKING = ("none", "low", "medium", "high", "xhigh", "max")
CARD_MODES = ("purpose_first", "blind_first")
# F の立場（C1・C3）・CARD の足す子の型（C7）
STANCES = ("support", "deny", "issue")
CHILD_KINDS = ("Q", "A", "F")
# 引用 C-CITE
_CITE_RE = re.compile(
    r"^#([1-9][0-9]{0,8})((?:\.[1-9][0-9]*){1,2}(?:-[1-9][0-9]*)?)?( .{1,80})?$"
)
# ガイドのキー・報告書の ID
_GUIDE_KEY_RE = re.compile(r"^[A-Za-z0-9._:-]{1,64}$")
_REPORT_RE = re.compile(r"^R-[0-9]{4,8}$")
# カード定義のブロック
_CARD_BLOCK_RE = re.compile(r"^(sci|lab)-[a-z-]{2,30}$")

# 数の上限
RUN_ID_MAX = 999_999_999
CITES_MAX = 10
PATH_MAX = 20
CARD_PAPERS_MAX = 20
# 文字数の上限
BODY_MAX = 2000

# --- C-OP の op ごとの項目（C3 の表と同じ 15 種類） ---
# 値は「検査のしかた」。知らない項目は落とすので、1 つ 1 つ書く。
#   ref:<種類>   必須の ID（種類は Q・A・F・E・S・D の文字の並び）
#   text:<n>     必須の文字（1〜n 字）
#   ztext:<n>    必須の文字（0〜n 字。空も通す）
#   otext:<n>    任意の文字（空なら無いものとして落とす）
#   body         任意の本文（0〜2000 字。空のまま送れる）
#   enum:<名前>  必須の値
#   他は _check_one を見る
# 親子の規則・許可・承認は CLI（ops.rs）が見る。ここは形だけ。
_OP_FIELDS: dict[str, dict[str, str]] = {
    "add_q": {"parent": "qparent", "text": "text:300", "guide": "oguide"},
    "insert_q": {"q": "ref:Q", "text": "text:300", "guide": "oguide"},
    "move_a": {"from": "ref:Q", "to": "ref:Q"},
    "copy": {"node": "ref:A", "to": "ref:Q"},
    "remove": {"node": "ref:QAFESD"},
    "add_a": {"q": "ref:Q", "text": "text:120", "body": "body"},
    "add_f": {
        "parent": "ref:A",
        "text": "text:1200",
        "stance": "enum:stance",
        "cites": "ocites",
        "paper": "opaper",
        "report": "oreport",
        "run": "orun",
    },
    "add_e": {"parent": "ref:SE", "text": "text:120", "body": "body", "mlflow": "otext:200"},
    "add_seed": {"title": "text:120", "body": "body"},
    "add_sprout": {"seed": "ref:D", "title": "text:120", "body": "body", "paths": "sproutpaths"},
    "set_stance": {"node": "ref:F", "stance": "enum:stance"},
    "set_mlflow": {"node": "ref:E", "mlflow": "ztext:200"},
    "star": {"node": "ref:QAFESD"},
    "unstar": {"node": "ref:QAFESD"},
    "edit": {"node": "ref:QAFESD", "text": "otext:1200", "body": "body"},
}
_ENUMS = {"stance": STANCES}


def _bad(detail: str) -> HTTPException:
    """400 の例外を作る（理由は日本語で書く）。"""
    return HTTPException(status_code=400, detail=detail)


def _trim(value) -> str:
    """文字列の前後の空白を落とす。文字列でなければ 400。"""
    if not isinstance(value, str):
        raise _bad("文字列で送ってください")
    return value.strip()


def check_node(value) -> str:
    """Node の ID（`Q12`・`A5`・`F31`・`E2`・`S1`）を検査して返す。違反は 400。数だけは受けない。"""
    if not isinstance(value, str) or not _NODE_RE.match(value.strip()):
        raise _bad("ID の形が正しくありません")
    return value.strip()


def _check_ref(value, kinds: str) -> str:
    """Node の ID で、種類が `kinds` の文字のどれかであるもの。"""
    node = check_node(value)
    if node[0] not in kinds:
        raise _bad("ID の形が正しくありません")
    return node


def _check_node_or_seed(value) -> str:
    """`SEED`（ID を持たない特別な Node）か、Node の ID（Q・A・F・E・S・D）。

    `check_node` は `SEED` を受けないので、カードの `node` と add_q の親はここを通す。
    """
    if isinstance(value, str) and value.strip() == "SEED":
        return "SEED"
    return check_node(value)


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
    """RESEARCH_TYPE を検査して返す（C-01 の 5 つ）。違反は 400。"""
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


def _check_one(key: str, kind: str, value):
    """項目 1 つを検査して、CLI に渡す形にして返す。"""
    name, _, arg = kind.partition(":")
    if name == "ref":
        return _check_ref(value, arg)
    if name == "text":
        return _req_text({key: value}, key, int(arg))
    if name == "otext":
        return _opt_text({key: value}, key, int(arg))
    if name == "ztext":
        # 空も通す（set_mlflow の空文字は CLI で外す）
        text = _trim(value)
        if len(text) > int(arg):
            raise _bad(f"{key} は {arg} 字までです")
        return text
    if name == "body":
        text = _trim(value)
        if len(text) > BODY_MAX:
            raise _bad(f"{key} は {BODY_MAX} 字までです")
        return text
    if name == "enum":
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
    if name == "qparent":
        # add_q の親は SEED（ID を持たない特別な Node）か Node の ID（種類の組は CLI が見る）
        return _check_node_or_seed(value)
    if name == "sproutpaths":
        out = _check_id_list(value, "A", PATH_MAX, key, minimum=1)
        if len(set(out)) != len(out):
            raise _bad("paths に同じ A が 2 回入っています")
        return out
    raise _bad("指定が正しくありません")


def validate_op(payload) -> dict:
    """C-OP の op の JSON を検査して、CLI に渡す形の dict にして返す（違反は 400）。

    - `op` は C-OP の 15 種類（Web からもすべて送れる）。
    - op ごとに表の項目だけを残す。知らない項目と `path` のようなファイルのパスは落ちるので、
      CLI には渡らない（`add_sprout` の `paths` だけは A の ID の配列で、検査して渡す）。
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
        # `ref:`・`enum:`・`text:`・`ztext:`・`qparent`・`sproutpaths` は必須、ほかは任意
        optional = kind.split(":")[0] in ("otext", "body", "oguide", "ocites", "opaper", "orun", "oreport")
        if raw is None:
            if optional:
                continue
            raise _bad(f"{key} がありません")
        value = _check_one(key, kind, raw)
        if value is not None:
            out[key] = value
    if name == "edit" and "text" not in out and "body" not in out:
        raise _bad("text か body のどちらかが要ります")
    return out


# --- カードの検査（C-CARD） ---


def validate_card(payload) -> dict:
    """READ_CARD の JSON を検査して、CLI に渡す形の dict にして返す（違反は 400）。

    - `node`（Q・A・F・E・S・D と `SEED`）・`block`（`^sci-[a-z-]{2,30}$`）・`purpose`（1〜300 字、1 行）は必須
    - `expect` 300 字、`depth`・`mode`・`runner` は表の値（`runner` は空なら `cc`・`pi` は 400）、`origin_run` は run_id
    - `model`・`thinking` は省略可（`CARD_MODELS`・`CARD_THINKING` の値だけ）
    - `child`（任意。`Q`・`A`・`F` のどれか。空は未指定）は C7 の `sci_child` として渡す。対象との組は CLI が見る
    - `papers` は 20 個まで・各 1〜999999999
    - `runner` が `user` なら `papers` はちょうど 1 つで、`node` は Q か A（`SEED` は選べない）
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

    runner = payload.get("runner") or "cc"
    if runner == "pi":
        raise _bad("runner の pi は廃止しました")
    out: dict = {
        "node": _check_node_or_seed(payload.get("node")),
        "block": block,
        "purpose": purpose,
        "runner": _enum(runner, RUNNERS, "runner"),
    }
    expect = _opt_text(payload, "expect", 300)
    if expect is not None:
        out["expect"] = expect
    # model・thinking は省略可（空・無しは block の既定に任せる）
    for key, allowed in (("model", CARD_MODELS), ("thinking", CARD_THINKING)):
        if payload.get(key) is not None and _trim(payload[key]):
            out[key] = _enum(payload[key], allowed, key)
    for key, allowed in (("depth", DEPTHS), ("mode", CARD_MODES)):
        if payload.get(key) is not None:
            out[key] = _enum(payload[key], allowed, key)
    if payload.get("origin_run") is not None:
        out["origin_run"] = check_run_id(payload["origin_run"])
    child = payload.get("child")
    if child is not None and _trim(child):
        out["child"] = _enum(child, CHILD_KINDS, "child")

    papers_value = payload.get("papers")
    papers_list: list[int] = []
    if papers_value is not None:
        if not isinstance(papers_value, list) or len(papers_value) > CARD_PAPERS_MAX:
            raise _bad("papers の指定が正しくありません")
        papers_list = [check_run_id(item) for item in papers_value]
    out["papers"] = papers_list

    if out["runner"] == "user":
        if len(papers_list) != 1:
            raise _bad("READ_CARD は論文を 1 つだけ指定してください")
        if out["node"] == "SEED" or out["node"][0] not in ("Q", "A"):
            raise _bad("READ_CARD の対象は Q か A にしてください")
    return out


# --- 中継 ---


def _sci_cli(project: dict, args: list[str], timeout: float, input_text: str | None = None):
    """sai-sci の CLI を呼ぶ（cwd = root，`SAI_PROJECT_ROOT` = root，`SAI_ORIGIN` = phone）。"""
    root = str(project["root"])
    return cli.run_json(
        [settings.sai_sci_bin, *args],
        app="sai-sci",
        cwd=root,
        env={**os.environ, "SAI_PROJECT_ROOT": root, "SAI_ORIGIN": "phone"},
        timeout=timeout,
        input_text=input_text,
    )


def status(project: dict) -> dict:
    """research-tree 全体（`sai-sci status --json`）。"""
    return _sci_cli(project, ["status", "--json"], SCI_READ_TIMEOUT)


def types(project: dict) -> dict:
    """使える RESEARCH_TYPE の一覧（`sai-sci types --json`）。"""
    return _sci_cli(project, ["types", "--json"], SCI_READ_TIMEOUT)


def init(project: dict, payload) -> dict:
    """始める（`sai-sci init --type <型> --json`・骨組みの展開で待つ時間は長め）。

    出力の `tree` だけを返す（画面が読む形は今の `phone sci init` と同じ）。
    木を作れず `tree` が無いときは 502。
    """
    if not isinstance(payload, dict):
        raise _bad("送る JSON が正しくありません")
    kind = check_type(payload.get("type"))
    data = _sci_cli(
        project, ["init", "--type", kind, "--json"], SCI_INIT_TIMEOUT
    )
    tree = data.get("tree") if isinstance(data, dict) else None
    if not tree:
        raise HTTPException(
            status_code=502, detail="research-tree を作れませんでした"
        )
    return tree


def node(project: dict, node_id) -> dict:
    """Node の中身（`sai-sci node <ID> --json`）。"""
    return _sci_cli(project, ["node", check_node(node_id), "--json"], SCI_READ_TIMEOUT)


def apply(project: dict, payload) -> dict:
    """人の操作を反映する（`sai-sci apply --json`・本文は標準入力）。"""
    data = validate_op(payload)
    return _sci_cli(
        project,
        ["apply", "--json"],
        SCI_WRITE_TIMEOUT,
        input_text=json.dumps(data, ensure_ascii=False),
    )


def create_card(project: dict, payload) -> dict:
    """READ_CARD を作る（`sai-sci card --from-json --json`・本文は標準入力）。"""
    data = validate_card(payload)
    return _sci_cli(
        project,
        ["card", "--from-json", "--json"],
        SCI_WRITE_TIMEOUT,
        input_text=json.dumps(data, ensure_ascii=False),
    )


def read(project: dict, run_id) -> dict:
    """READ_CARD の中身（`sai-sci read <run_id> --json`）。"""
    return _sci_cli(
        project, ["read", str(check_run_id(run_id)), "--json"], SCI_READ_TIMEOUT
    )


def read_start(project: dict, run_id) -> dict:
    """READ_CARD を始める（`sai-link phone user start <run_id>`・C-12）。"""
    return control.phone_user_start(str(project["root"]), check_run_id(run_id))


def read_finish(project: dict, run_id) -> dict:
    """READ_CARD を終えて review へ移す。

    `read` で owner を見て、`todo` なら先に始める（利用者が「始める」を押し忘れても 1 回で済ませる）。
    F が 0 でも止めない。
    """
    rid = check_run_id(run_id)
    info = read(project, rid)
    if not isinstance(info, dict):
        info = {}
    if info.get("owner") == "todo":
        read_start(project, rid)
    control.phone_user_finish(str(project["root"]), rid)
    findings = info.get("findings")
    return {
        "run_id": rid,
        "status": "review",
        "findings": len(findings) if isinstance(findings, list) else 0,
    }
