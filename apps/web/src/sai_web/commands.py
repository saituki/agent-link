"""Web の「コマンド」画面（U10）。

3 つの CLI（sai-link・sai-paper・sai-sci）の `schema --json` から命令の一覧を作り，
入力欄から命令を実行する。中の処理は CLI に任せ，ここは検査と読み替えだけを行う。

決め事（共有契約「schema の形」「コマンドの実行の口」）:
- 一覧は `web: true` の命令と引数だけ。`stdin` が `bytes` の命令は出さない
  （iPhone からファイルを送れないため）。
- 実行の値は CLI を呼ぶ前に全部確かめる（通らなければ 400・CLI は呼ばない）。
- argv は `[bin, *path]` ＋ `<--json>` ＋ オプション ＋ フラグ ＋ `--` ＋ 位置引数
  （schema の順）。位置引数を `--` のあとに置くのは `-` で始まる値を守るため。
- 失敗の読み替えは `cli.raise_for_failure`（U09 と同じ 1 か所）。
"""

from __future__ import annotations

import json
import logging
import time

from fastapi import HTTPException

from sai_web import cli
from sai_web.config import settings

logger = logging.getLogger("sai_web")

# schema を覚えておく時間（実行ファイルを作り直したときに古いままにしない）
_SCHEMA_TTL = 60.0
# schema 取得の時間の上限（CLI の起動が遅いときも一覧を止めない）
_SCHEMA_TIMEOUT = 20.0
# 命令の実行の時間の上限（計画 U10）。長い研究の命令は U14 で sai-sci へ移る。
_RUN_TIMEOUT = 120.0
# 標準出力の上限（これを超えたら 502）
_OUTPUT_MAX = 2 * 1024 * 1024
# 1 行の文字列の上限
_STRING_MAX = 2000
# 標準入力（text）の上限
_TEXT_MAX = 1024 * 1024
# 複数の値の個数の上限
_MULTIPLE_MAX = 100

# CLI の見分け（画面の選択に出す key と実行ファイルの設定・失敗の文に使う名前）
_CLI_LABELS = {"link": "sai-link", "paper": "sai-paper", "sci": "sai-sci"}

# key → (取得した時刻, schema または None)。None は「取れなかった」も覚える。
_schema_cache: dict[str, tuple[float, dict | None]] = {}


def cli_table() -> dict[str, str]:
    """画面の選択に出す CLI の表（key → 実行ファイル）。"""
    return {
        "link": settings.sai_link_bin,
        "paper": settings.sai_paper_bin,
        "sci": settings.sai_sci_bin,
    }


def _label(cli_key: str) -> str:
    return _CLI_LABELS.get(cli_key, cli_key)


def _fetch_schema(binpath: str, label: str) -> dict | None:
    """`<bin> schema --json` を呼んで schema を読む。取れなければ None（一覧から外す）。"""
    try:
        proc = cli.run_cli(
            [binpath, "schema", "--json"],
            app=label,
            timeout=_SCHEMA_TIMEOUT,
        )
    except HTTPException as e:
        # 実行ファイルが無い（503）・時間切れ（504）などは一覧から外すだけ
        logger.warning(f"schema を取得できません（一覧から外します）: {label}: {e.detail}")
        return None
    if proc.returncode != 0:
        logger.warning(
            f"schema が失敗しました（一覧から外します）: {label}: {cli.last_reason(proc)}"
        )
        return None
    try:
        data = json.loads(proc.stdout or "")
    except (json.JSONDecodeError, TypeError):
        logger.warning(f"schema を JSON として読めません（一覧から外します）: {label}")
        return None
    if not isinstance(data, dict) or not isinstance(data.get("commands"), list):
        logger.warning(f"schema の形が違います（一覧から外します）: {label}")
        return None
    return data


def get_schema(cli_key: str) -> dict | None:
    """key の schema（60 秒だけ覚える）。取れなければ None。"""
    table = cli_table()
    if cli_key not in table:
        return None
    now = time.monotonic()
    hit = _schema_cache.get(cli_key)
    if hit is not None and now - hit[0] < _SCHEMA_TTL:
        return hit[1]
    schema = _fetch_schema(table[cli_key], _label(cli_key))
    _schema_cache[cli_key] = (now, schema)
    return schema


def _web_commands(schema: dict) -> list[dict]:
    """schema から Web に出す命令だけを抜く（引数も `web: true` だけ）。"""
    out = []
    for c in schema.get("commands", []):
        if not isinstance(c, dict) or not c.get("web"):
            continue
        # iPhone からファイル（bytes）は送れない（計画 U10）
        if c.get("stdin") == "bytes":
            continue
        command = dict(c)
        command["args"] = [a for a in c.get("args", []) if isinstance(a, dict) and a.get("web")]
        out.append(command)
    return out


def list_commands(name: str) -> dict:
    """`GET /api/projects/{name}/commands` の本体。

    取れた CLI だけを返す（sci が無い環境では link・paper の 2 つ）。
    """
    from sai_web.control import _phone_project  # 循環 import を避けるためここで読む

    _phone_project(name)
    clis = []
    for key, binpath in cli_table().items():
        schema = get_schema(key)
        if schema is None:
            continue
        clis.append(
            {
                "key": key,
                "bin": schema.get("bin") or binpath,
                "commands": _web_commands(schema),
            }
        )
    return {"clis": clis}


def _find_command(schema: dict, path: list) -> dict:
    for c in schema.get("commands", []):
        if isinstance(c, dict) and c.get("path") == path:
            return c
    raise HTTPException(status_code=400, detail="その命令はありません")


def _bad(detail: str) -> HTTPException:
    return HTTPException(status_code=400, detail=detail)


def _validate_scalar(arg: dict, value):
    """引数 1 つを型ごとに確かめ，CLI へ渡す値にして返す。"""
    t = arg.get("type")
    if t in ("string", "path"):
        if not isinstance(value, str):
            raise _bad(f"{arg.get('name')} は文字列で送ってください")
        if "\n" in value or "\r" in value:
            raise _bad(f"{arg.get('name')} に改行は入れられません")
        if "\x00" in value:
            raise _bad(f"{arg.get('name')} に使えない文字が入っています")
        if len(value) > _STRING_MAX:
            raise _bad(f"{arg.get('name')} は {_STRING_MAX} 字までです")
        return value
    if t == "int":
        if isinstance(value, bool) or not isinstance(value, int):
            raise _bad(f"{arg.get('name')} は整数で送ってください")
        return value
    if t == "float":
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise _bad(f"{arg.get('name')} は数で送ってください")
        return value
    if t == "enum":
        values = arg.get("values") or []
        if value not in values:
            raise _bad(f"{arg.get('name')} の値が正しくありません")
        return value
    raise _bad(f"{arg.get('name')} の型が分かりません")


def _validate_arg(arg: dict, value):
    """引数 1 つ（複数ありも含む）を確かめる。"""
    if arg.get("kind") == "flag":
        if not isinstance(value, bool):
            raise _bad(f"{arg.get('name')} は真偽で送ってください")
        return value
    if arg.get("multiple"):
        if not isinstance(value, list):
            raise _bad(f"{arg.get('name')} は値の一覧で送ってください")
        if not 1 <= len(value) <= _MULTIPLE_MAX:
            raise _bad(f"{arg.get('name')} は 1〜{_MULTIPLE_MAX} 個にしてください")
        return [_validate_scalar(arg, v) for v in value]
    return _validate_scalar(arg, value)


def _format(value) -> str:
    """argv に書く文字にする（float の 1.0 → "1.0" は CLI が読む）。"""
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def _build_argv(binpath: str, command: dict, values: dict, stdin_text: str | None) -> list:
    """共有契約の argv を組む。"""
    path = command.get("path") or []
    argv = [binpath, *[str(p) for p in path]]
    if command.get("json"):
        argv.append("--json")
    positionals = []
    for arg in command.get("args", []):
        if not arg.get("web"):
            continue
        name = arg.get("name")
        if arg.get("kind") == "positional":
            positionals.append((arg, values.get(name)))
            continue
        if name not in values:
            continue
        value = values[name]
        if arg.get("kind") == "flag":
            if value:
                argv.append(f"--{name}")
            continue
        if arg.get("multiple"):
            for v in value:
                argv += [f"--{name}", _format(v)]
        else:
            argv += [f"--{name}", _format(value)]
    if positionals:
        argv.append("--")
        for arg, value in positionals:
            if value is None:
                continue
            if arg.get("multiple"):
                for v in value:
                    argv.append(_format(v))
            else:
                argv.append(_format(value))
    return argv


def _stdin_text(command: dict, stdin):
    """標準入力に渡す文字を確かめて返す（渡さないときは None）。"""
    kind = command.get("stdin") or "none"
    if kind == "none":
        if stdin not in (None, ""):
            raise _bad("この命令は標準入力を受け取りません")
        return None
    if kind == "bytes":
        raise _bad("この命令は Web から実行できません")
    if stdin in (None, ""):
        return None
    if not isinstance(stdin, str):
        raise _bad("標準入力は文字で送ってください")
    if kind == "json":
        try:
            json.loads(stdin)
        except (json.JSONDecodeError, TypeError):
            raise _bad("標準入力は JSON として読める形にしてください") from None
        return stdin
    # text
    if len(stdin.encode("utf-8")) > _TEXT_MAX:
        raise _bad("標準入力は 1MB までです")
    return stdin


def run_command(name: str, payload: dict) -> dict:
    """`POST /api/projects/{name}/commands/run` の本体。"""
    from sai_web.control import _phone_env, _phone_project  # 循環 import を避ける

    project = _phone_project(name)
    root = project.get("root") or ""
    if not isinstance(payload, dict):
        raise _bad("送る値が正しくありません")
    cli_key = payload.get("cli")
    path = payload.get("path")
    values = payload.get("args") or {}
    stdin = payload.get("stdin")

    if cli_key not in cli_table():
        raise _bad("知らない CLI です")
    schema = get_schema(cli_key)
    if schema is None:
        raise _bad("この CLI の命令一覧を取得できません")
    if not isinstance(path, list) or not path or not all(isinstance(p, str) for p in path):
        raise _bad("path は命令の語の一覧で送ってください")
    command = _find_command(schema, path)
    if not command.get("web"):
        raise _bad("この命令は Web から実行できません")
    if command.get("stdin") == "bytes":
        raise _bad("この命令は Web から実行できません")
    if not isinstance(values, dict):
        raise _bad("args は名前と値の組で送ってください")

    web_args = [a for a in command.get("args", []) if isinstance(a, dict) and a.get("web")]
    by_name = {a.get("name"): a for a in web_args}
    for key in values:
        if key not in by_name:
            raise _bad(f"知らない引数です: {key}")
    checked: dict = {}
    for arg in web_args:
        key = arg.get("name")
        if key in values:
            checked[key] = _validate_arg(arg, values[key])
        elif arg.get("required"):
            raise _bad(f"{key} は必須です")

    stdin_text = _stdin_text(command, stdin)
    argv = _build_argv(cli_table()[cli_key], command, checked, stdin_text)
    label = _label(cli_key)
    proc = cli.run_cli(
        argv,
        app=label,
        cwd=root,
        env=_phone_env(root),
        timeout=_RUN_TIMEOUT,
        input_text=stdin_text,
    )
    if proc.returncode != 0:
        cli.raise_for_failure(label, proc)
        raise AssertionError("raise_for_failure は必ず失敗させる")
    stdout = proc.stdout or ""
    if len(stdout.encode("utf-8")) > _OUTPUT_MAX:
        raise HTTPException(status_code=502, detail="出力が大きすぎます")
    try:
        return {"ok": True, "json": json.loads(stdout), "text": None}
    except (json.JSONDecodeError, TypeError):
        return {"ok": True, "json": None, "text": stdout}
