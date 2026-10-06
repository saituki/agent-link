"""Web の「コマンド」画面のサーバ（U10・`apps/web/src/sai_web/commands.py`）の試験。

確かめること:
- schema の取得と一覧（`web: false` の命令・引数を外す / `stdin: bytes` を外す /
  実行ファイルが無い CLI を外す / schema を JSON として読めない CLI を外す）
- 実行の検査（知らない CLI・命令・引数 / `web: false` / 型 / enum / 必須 /
  改行 / 長さ / 標準入力の種類 / `bytes`）で CLI を呼ばないこと
- argv の組み方（共有契約「コマンドの実行の口」）
- env（`SAI_PROJECT_ROOT`・`SAI_ORIGIN=phone`・`SAI_PAPER_ROOT` を渡さない）
- 終了コードの読み替え（2 → 400・3 → 404・4 → 409・1 とその他 → 502）
- 標準出力が 2MB を超えたら 502

偽の CLI（一時フォルダの小さな Python スクリプト）を使う。スクリプトは
`schema --json` には同じ場所の `<名前>.schema.json` を返し、ほかの呼び出しは
argv と標準入力と env を JSON にして返す。

実行:
    cd <作業用コピー>
    env -u SAI_LINK_BIN PYTHONPATH=apps/web/src \
      /Users/touta/workdir/Sai/sai-moon/apps/web/.venv/bin/python apps/web/tests/test_commands_api.py
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

from fastapi import HTTPException

from sai_web import commands, control

# 偽の CLI 本体。名前の隣の `<名前>.schema.json` を schema として返す。
SCRIPT = """#!/usr/bin/env python3
import json, os, sys
self_path = os.path.abspath(sys.argv[0])
if sys.argv[1:] == ["schema", "--json"]:
    with open(self_path + ".schema.json", encoding="utf-8") as f:
        sys.stdout.write(f.read())
    raise SystemExit(0)
if "--big" in sys.argv:
    sys.stdout.write("x" * (2 * 1024 * 1024 + 10))
    raise SystemExit(0)
if sys.argv[1:2] == ["fail"]:
    sys.stderr.write("失敗の理由\\n")
    code = 1
    if "--code" in sys.argv:
        code = int(sys.argv[sys.argv.index("--code") + 1])
    raise SystemExit(code)
payload = {
    "argv": list(sys.argv),
    "stdin": sys.stdin.read(),
    "env": {k: os.environ.get(k) for k in ("SAI_PROJECT_ROOT", "SAI_ORIGIN", "SAI_PAPER_ROOT")},
}
sys.stdout.write(json.dumps(payload, ensure_ascii=False))
"""


def arg(name, kind, type_, required=False, multiple=False, values=None, web=True):
    return {
        "name": name,
        "kind": kind,
        "type": type_,
        "required": required,
        "multiple": multiple,
        "values": values or [],
        "default": None,
        "help": "",
        "web": web,
    }


LINK_SCHEMA = {
    "bin": "fake-link",
    "commands": [
        {
            "path": ["show"], "about": "実行を見る", "stdin": "none",
            "json": True, "web": True,
            "args": [
                arg("run_id", "positional", "int", required=True),
                arg("file", "option", "path", web=False),
            ],
        },
        {"path": ["secret"], "about": "下回り", "stdin": "none", "json": True, "web": False, "args": []},
        {"path": ["upload"], "about": "ファイル", "stdin": "bytes", "json": True, "web": True, "args": []},
    ],
}

PAPER_SCHEMA = {
    "bin": "fake-paper",
    "commands": [
        {
            "path": ["set", "create"], "about": "組を作る", "stdin": "none",
            "json": True, "web": True,
            "args": [
                arg("name", "positional", "string", required=True),
                arg("query", "option", "string"),
                arg("file", "option", "path", web=False),
                arg("hidden", "option", "string", web=False),
            ],
        },
        {
            "path": ["find"], "about": "探す", "stdin": "none",
            "json": True, "web": True,
            "args": [
                arg("query", "option", "string", required=True),
                arg("paper", "option", "int", multiple=True),
                arg("fts_expr", "option", "enum", values=["a", "b"]),
            ],
        },
        {
            "path": ["rm"], "about": "消す", "stdin": "none",
            "json": False, "web": True,
            "args": [
                arg("run_id", "positional", "int", required=True),
                arg("yes", "flag", "enum", values=["true", "false"]),
            ],
        },
        {
            "path": ["add"], "about": "足す", "stdin": "text",
            "json": True, "web": True,
            "args": [arg("name", "positional", "string", required=True)],
        },
        {
            "path": ["doc", "create"], "about": "作る", "stdin": "json",
            "json": True, "web": True, "args": [],
        },
        {"path": ["import-pdf"], "about": "PDF", "stdin": "bytes", "json": True, "web": True, "args": []},
        {
            "path": ["fail"], "about": "失敗", "stdin": "none",
            "json": False, "web": True,
            "args": [arg("code", "option", "int", required=True)],
        },
        {
            "path": ["big"], "about": "大きな出力", "stdin": "none",
            "json": True, "web": True,
            "args": [arg("big", "flag", "enum", values=["true", "false"])],
        },
    ],
}


def write_cli(folder: str, name: str, schema: dict) -> str:
    path = Path(folder) / name
    path.write_text(SCRIPT, encoding="utf-8")
    path.chmod(0o755)
    Path(folder, name + ".schema.json").write_text(
        json.dumps(schema, ensure_ascii=False), encoding="utf-8"
    )
    return str(path)


def expect(code: int, fn) -> HTTPException:
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{code} のときは日本語の理由を書く"
        return e
    raise AssertionError(f"{code} になるはず")


class Setup:
    """偽の CLI とプロジェクトを用意して，呼び出しの前後を片づける。"""

    def __init__(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = str(Path(self.tmp.name) / "root")
        os.makedirs(self.root, exist_ok=True)
        self.link = write_cli(self.tmp.name, "link-cli", LINK_SCHEMA)
        self.paper = write_cli(self.tmp.name, "paper-cli", PAPER_SCHEMA)
        self.orig_table = commands.cli_table
        self.orig_find = control.find_project
        commands.cli_table = lambda: {"link": self.link, "paper": self.paper, "sci": "/no/such/sci-zzz"}
        commands._schema_cache.clear()

        def find(name: str) -> dict:
            if name != "demo":
                raise HTTPException(status_code=404, detail="プロジェクトが見つかりません")
            return {"name": "demo", "root": self.root, "root_exists": True, "db_exists": True}

        control.find_project = find

    def close(self) -> None:
        commands.cli_table = self.orig_table
        control.find_project = self.orig_find
        commands._schema_cache.clear()
        self.tmp.cleanup()


def payload_of(out: dict) -> dict:
    assert out["ok"] is True, out
    assert out["text"] is None, out
    return out["json"]


def test_list(setup: Setup) -> None:
    data = commands.list_commands("demo")
    keys = [c["key"] for c in data["clis"]]
    assert keys == ["link", "paper"], keys  # sci は実行ファイルが無いので外れる
    link = data["clis"][0]
    paths = [" ".join(c["path"]) for c in link["commands"]]
    assert "secret" not in paths, paths
    assert "upload" not in paths, paths  # stdin: bytes は出さない
    show = [c for c in link["commands"] if c["path"] == ["show"]][0]
    names = [a["name"] for a in show["args"]]
    assert "file" not in names, names  # web: false の引数は出さない


def test_argv(setup: Setup) -> None:
    out = commands.run_command(
        "demo",
        {"cli": "paper", "path": ["set", "create"], "args": {"name": "demo", "query": "attention"}},
    )
    body = payload_of(out)
    assert body["argv"] == [
        setup.paper, "set", "create", "--json", "--query", "attention", "--", "demo",
    ], body["argv"]

    # int の位置引数・flag・json でない命令
    out = commands.run_command(
        "demo", {"cli": "paper", "path": ["rm"], "args": {"run_id": 5, "yes": True}}
    )
    assert payload_of(out)["argv"] == [setup.paper, "rm", "--yes", "--", "5"]

    # multiple は値ごとに繰り返す
    out = commands.run_command(
        "demo", {"cli": "paper", "path": ["find"], "args": {"query": "x", "paper": [1, 2]}}
    )
    assert payload_of(out)["argv"] == [
        setup.paper, "find", "--json", "--query", "x", "--paper", "1", "--paper", "2",
    ]


def test_env_and_stdin(setup: Setup) -> None:
    out = commands.run_command("demo", {"cli": "link", "path": ["show"], "args": {"run_id": 7}})
    body = payload_of(out)
    assert body["env"]["SAI_PROJECT_ROOT"] == setup.root, body["env"]
    assert body["env"]["SAI_ORIGIN"] == "phone", body["env"]
    assert body["env"]["SAI_PAPER_ROOT"] is None, body["env"]
    assert body["stdin"] == "", body

    # text の標準入力
    out = commands.run_command(
        "demo", {"cli": "paper", "path": ["add"], "args": {"name": "x"}, "stdin": "本文\n2 行"}
    )
    assert payload_of(out)["stdin"] == "本文\n2 行"

    # json の標準入力
    out = commands.run_command(
        "demo", {"cli": "paper", "path": ["doc", "create"], "stdin": '{"a": 1}'}
    )
    assert payload_of(out)["stdin"] == '{"a": 1}'


def test_validation(setup: Setup) -> None:
    # schema は取れている前提にして，CLI が呼ばれたら分かるようにする
    commands._schema_cache["link"] = (time.monotonic(), LINK_SCHEMA)
    commands._schema_cache["paper"] = (time.monotonic(), PAPER_SCHEMA)
    calls: list = []

    def boom(*a, **k):
        calls.append(a)
        raise AssertionError("検査で外れた値なのに CLI が呼ばれた")

    orig = commands.cli.run_cli
    commands.cli.run_cli = boom
    try:
        bad = [
            {"cli": "bogus", "path": ["show"], "args": {"run_id": 1}},
            {"cli": "link", "path": ["nope"], "args": {}},
            {"cli": "link", "path": ["secret"], "args": {}},
            {"cli": "link", "path": ["upload"], "args": {}},
            {"cli": "paper", "path": ["import-pdf"], "args": {}},
            {"cli": "link", "path": ["show"], "args": {"run_id": 1, "file": "x"}},
            {"cli": "link", "path": ["show"], "args": {}},  # 必須が無い
            {"cli": "link", "path": ["show"], "args": {"run_id": "1"}},  # 型違い
            {"cli": "paper", "path": ["find"], "args": {"query": "x", "fts_expr": "c"}},  # enum
            {"cli": "paper", "path": ["find"], "args": {"query": "x", "paper": []}},  # 空の一覧
            {"cli": "paper", "path": ["find"], "args": {"query": "x\nx"}},  # 改行
            {"cli": "paper", "path": ["find"], "args": {"query": "x" * 2001}},  # 長さ
            {"cli": "link", "path": ["show"], "args": {"run_id": 1}, "stdin": "x"},  # stdin none
            {"cli": "paper", "path": ["doc", "create"], "stdin": "{壊れた"},  # json でない
            {"cli": "paper", "path": ["add"], "args": {"name": "x"}, "stdin": "y" * (1024 * 1024 + 1)},
        ]
        for req in bad:
            expect(400, lambda req=req: commands.run_command("demo", req))
        assert calls == [], calls
    finally:
        commands.cli.run_cli = orig


def test_exit_codes(setup: Setup) -> None:
    for code, http in ((2, 400), (3, 404), (4, 409), (1, 502), (7, 502)):
        e = expect(
            http,
            lambda code=code: commands.run_command(
                "demo", {"cli": "paper", "path": ["fail"], "args": {"code": code}}
            ),
        )
        assert e.detail == "失敗の理由", e.detail


def test_output_too_large(setup: Setup) -> None:
    e = expect(
        502,
        lambda: commands.run_command(
            "demo", {"cli": "paper", "path": ["big"], "args": {"big": True}}
        ),
    )
    assert e.detail == "出力が大きすぎます", e.detail


def test_routes() -> None:
    from sai_web.main import app

    paths = {(r.path, tuple(sorted(r.methods or []))) for r in app.routes if hasattr(r, "methods")}
    assert ("/api/projects/{name}/commands", ("GET",)) in paths, "一覧の住所が無い"
    assert ("/api/projects/{name}/commands/run", ("POST",)) in paths, "実行の住所が無い"


def main() -> None:
    setup = Setup()
    try:
        test_list(setup)
        test_argv(setup)
        test_env_and_stdin(setup)
        test_validation(setup)
        test_exit_codes(setup)
        test_output_too_large(setup)
    finally:
        setup.close()
    test_routes()
    print("test_commands_api: すべて通りました")


main()
