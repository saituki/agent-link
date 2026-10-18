"""拡張（extension）の住所の小さなテスト（一覧・差し込み・画面の配信・CLI と service の中継）。

確かめること:
- `GET /` の HTML: 画面に出す拡張のタブ・style・script が出る / 出さない拡張（0 個・有効でない・
  宛先が ready でない・画面が無い）/ 目印が残らない / 値はエスケープされる / 拡張が 0 個なら
  目印の 3 行を消しただけの HTML になる
- `GET /ext/<name>/<path>`: `ui/` の中のファイルだけ返す（`..`・symlink の外・ui を持たない・知らない拡張は 404）
- `POST /api/projects/<p>/ext/<e>/cli`: 検査を通った物だけ `ext run <e> -- <args…>` を呼ぶ
  （値の誤りは 400・有効でない / web に無い は 403・CLI は呼ばない・終了コード 3 は 404）
- `svc` の中継: GET は中継・POST は service.post の道だけ・宛先の未設定は 404・リダイレクトは 502・
  つながらない宛先は 502・Basic 認証は両方の値があるときだけ付く
- 一覧は 10 秒覚える。CLI の失敗・壊れた JSON では空の一覧（base の画面は出す）

実行:
    cd <作業用コピー>
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_ext_api.py
"""

from __future__ import annotations

import base64
import json
import os
import shutil
import socket
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

# config.py は import のときに $AGENT_LINK_HOME/web を作るので、先に一時フォルダにする
os.environ["AGENT_LINK_HOME"] = tempfile.mkdtemp(prefix="agent-link-ext-api-")
os.environ["AGENT_LINK_WEB_NO_WATCH"] = "1"

from fastapi import HTTPException  # noqa: E402

from agent_link_web import control, ext  # noqa: E402
from agent_link_web import main as server  # noqa: E402

STATIC = Path(server.__file__).resolve().parent / "static"
MARKERS = {"<!-- ext:styles -->", "<!-- ext:tabs -->", "<!-- ext:scripts -->"}
SVC_ENV = "AGENT_LINK_EXT_HELLO_URL"


def expect(code: int, fn) -> HTTPException:
    """fn は指定した状態番号の HTTPException で失敗し，英語の理由が付くこと。"""
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{code} のときは英語で理由を書く"
        return e
    raise AssertionError(f"{code} になるはず")


class FakeCli:
    """`control.run_cli` の置き換え係。`ext list` と `ext run` の応答を差し替えられる。"""

    def __init__(self) -> None:
        self.calls: list[dict] = []
        self.list_json = json.dumps({"extensions": []})
        self.list_rc = 0
        self.run_rc = 0
        self.run_out = "{}"
        self.run_err = ""

    def __call__(self, args, *, app, cwd=None, env=None, timeout=20.0, input_text=None):
        self.calls.append(
            {"args": list(args), "cwd": cwd, "env": env, "timeout": timeout, "input": input_text}
        )
        if args[1:3] == ["ext", "list"]:
            return subprocess.CompletedProcess(list(args), self.list_rc, self.list_json, "")
        return subprocess.CompletedProcess(list(args), self.run_rc, self.run_out, self.run_err)

    def calls_of(self, sub: str) -> list[dict]:
        # argv は [agent-link, ext, <sub>, ...]
        return [c for c in self.calls if c["args"][2:3] == [sub]]


def use(cli: FakeCli, *entries: dict) -> None:
    """CLI の `ext list --json` の応答を入れ替え、覚えた一覧を消す。"""
    cli.list_json = json.dumps({"extensions": list(entries)})
    cli.list_rc = 0
    ext.clear_cache()


def entry(name: str, path: Path, *, ui=True, enabled=("p",), service=None, web=("greet",),
          version="0.1.0", title="Hello") -> dict:
    """`agent-link ext list --json` の 1 項目の形（Rust の listed_json と同じ項目）。"""
    return {
        "name": name,
        "title": title,
        "version": version,
        "description": "",
        "path": str(path),
        "linked": False,
        "requires": [],
        "init": [],
        "update": [],
        "blocks": [],
        "templates": [],
        "cli": {"bin": "bin/" + name, "web": list(web)},
        "ui": {"title": title, "icon": "icon.svg", "scripts": ["hello.js"], "styles": ["hello.css"]}
        if ui
        else None,
        "service": service,
        "enabled_in": list(enabled),
    }


def fake_find(rows: dict[str, dict]):
    """登録簿の行を想定した find_project（未知の名前は 404・control 自身と同じ形）。"""

    def find(name: str) -> dict:
        if name in rows:
            return rows[name]
        raise HTTPException(status_code=404, detail="Project not found.")

    return find


def make_tree(base: Path) -> Path:
    """拡張の置き場の見本。hello は ui を持ち、bare は持たない。戻り値は hello のフォルダ。"""
    hello = base / "hello"
    (hello / "ui").mkdir(parents=True)
    (hello / "ui" / "hello.js").write_text("// hello", encoding="utf-8")
    (hello / "ui" / "hello.css").write_text("/* hello */", encoding="utf-8")
    (hello / "ui" / "icon.svg").write_text("<svg></svg>", encoding="utf-8")
    (hello / "secret.txt").write_text("secret", encoding="utf-8")
    # ui の中から ui の外（secret.txt）を指す symlink
    os.symlink("../secret.txt", hello / "ui" / "link.js")
    (base / "bare").mkdir()
    return hello


def test_index_shows_tab_style_script(cli: FakeCli, hello: Path) -> None:
    """画面に出す拡張は、タブ・style・script が目印の場所に入る（script は app.js より前）。"""
    use(cli, entry("hello", hello))
    html = server.serve_index().body.decode("utf-8")
    assert '<link rel="stylesheet" href="/ext/hello/hello.css?v=0.1.0" />' in html, html
    assert '<script src="/ext/hello/hello.js?v=0.1.0"></script>' in html, html
    tab = (
        '<button data-tab="ext-hello" data-first="#/ext/hello" aria-label="Hello">'
        '<img src="/ext/hello/icon.svg" alt="" aria-hidden="true"><span>Hello</span></button>'
    )
    assert tab in html, html
    # タブは nav#tabbar の中、script は app.js より前
    assert html.index('<nav id="tabbar"') < html.index(tab) < html.index("</nav>", html.index(tab))
    assert html.index("/ext/hello/hello.js") < html.index("/app.js?v="), "拡張の script が app.js より後"
    assert not any(m in html for m in MARKERS), "目印が残っている"


def test_index_hides_unready_extensions(cli: FakeCli, hello: Path) -> None:
    """有効でない・宛先が ready でない・画面が無い拡張は出さない。"""
    not_ready = {"url_env": SVC_ENV, "user_env": None, "password_env": None, "post": [], "ready": False}
    use(
        cli,
        entry("off-a", hello, enabled=()),
        entry("off-b", hello, service=not_ready),
        entry("off-c", hello, ui=False),
    )
    html = server.serve_index().body.decode("utf-8")
    assert "data-tab=\"ext-" not in html, html
    assert "/ext/off-" not in html, html

    # 宛先が ready なら出る
    ready = dict(not_ready, ready=True)
    use(cli, entry("off-b", hello, service=ready))
    assert 'data-tab="ext-off-b"' in server.serve_index().body.decode("utf-8")


def test_index_escapes_values(cli: FakeCli, hello: Path) -> None:
    """title と version の `<` や `"` は HTML にそのまま出ない。"""
    use(cli, entry("hello", hello, title='A<"&>', version='1"<'))
    html = server.serve_index().body.decode("utf-8")
    assert "<b>" not in html
    assert 'aria-label="A&lt;&quot;&amp;&gt;"' in html, html
    assert 'href="/ext/hello/hello.css?v=1&quot;&lt;"' in html, html


def test_index_without_extensions_is_stripped_original(cli: FakeCli) -> None:
    """拡張が 0 個のときは、目印の 3 行を消しただけの HTML になる（index.html と比べる）。"""
    use(cli)
    original = (STATIC / "index.html").read_text(encoding="utf-8")
    assert sum(line.strip() in MARKERS for line in original.splitlines()) == 3, "目印が 3 行あること"
    expected = "".join(
        line for line in original.splitlines(keepends=True) if line.strip() not in MARKERS
    )
    assert server.serve_index().body.decode("utf-8") == expected


def test_ui_file_inside_only(cli: FakeCli, hello: Path) -> None:
    """ui/ の中のファイルだけ返す。外・symlink の先・ui を持たない拡張・知らない拡張は 404。"""
    use(cli, entry("hello", hello), entry("bare", hello.parent / "bare", ui=False))
    got = server.serve_ext_file("hello", "hello.js")
    assert Path(got.path).resolve() == (hello / "ui" / "hello.js").resolve(), got.path

    for name, path in [
        ("hello", "../secret.txt"),  # 親へ出る（%2e%2e も Starlette が .. に直してから来る）
        ("hello", "/etc/passwd"),  # 先頭の /
        ("hello", "a//b.js"),  # 空の区切り
        ("hello", "sub\\x.js"),  # バックスラッシュ
        ("hello", "link.js"),  # ui の外を指す symlink
        ("hello", "nope.js"),  # 無いファイル
        ("hello", ""),
        ("bare", "hello.js"),  # ui を持たない
        ("unknown", "hello.js"),  # 知らない拡張
        ("Bad", "hello.js"),  # 名前の規則に合わない
    ]:
        expect(404, lambda name=name, path=path: ext.ui_file(name, path))


def test_cli_success_and_checks(cli: FakeCli, root: Path) -> None:
    """正しい値は `ext run <e> -- <args>` を root で呼ぶ。誤った値・有効でない・web に無いは CLI を呼ばない。"""
    rows = {"p": {"name": "p", "root": str(root), "root_exists": True, "db_exists": True}}
    control.find_project = fake_find(rows)
    use(cli, entry("hello", root, enabled=("p",), web=("greet",)))
    cli.run_out = json.dumps({"reply": "hi x"})
    cli.calls.clear()

    out = server.api_project_ext_cli("p", "hello", {"args": ["greet", "x"]})
    assert out == {"reply": "hi x"}, out
    call = cli.calls[-1]
    assert call["args"] == [control.settings.agent_link_bin, "ext", "run", "hello", "--", "greet", "x"], call
    assert call["cwd"] == str(root), call["cwd"]
    assert call["env"]["AGENT_LINK_PROJECT_ROOT"] == str(root), call["env"]
    assert call["input"] is None, call

    # input 付きは --stdin を足し、本文を標準入力に渡す
    body = "# 見出し\n本文"
    server.api_project_ext_cli("p", "hello", {"args": ["greet", "A12"], "input": body})
    call = cli.calls[-1]
    assert call["args"] == [
        control.settings.agent_link_bin, "ext", "run", "hello", "--stdin", "--", "greet", "A12",
    ], call
    assert call["input"] == body, call
    server.api_project_ext_cli("p", "hello", {"args": ["greet"], "input": "x" * 1_000_000})

    # input の誤り（型・長さ）は 400 で CLI を呼ばない
    cli.calls.clear()
    for bad in (1, ["a"], "x" * 1_000_001):
        expect(400, lambda bad=bad: server.api_project_ext_cli("p", "hello", {"args": ["greet"], "input": bad}))
    assert cli.calls_of("run") == [], cli.calls

    # 値の誤り（型・個数・長さ・改行・NUL・空）は 400 で CLI を呼ばない
    cli.calls.clear()
    bad_args = [
        [],
        "greet",
        [1],
        ["greet", "a\nb"],
        ["greet", "a\rb"],
        ["greet", "a\x00"],
        [""],
        ["x" * 1001],
        ["greet"] * 33,
    ]
    for args in bad_args:
        expect(400, lambda args=args: server.api_project_ext_cli("p", "hello", {"args": args}))
    expect(400, lambda: server.api_project_ext_cli("p", "hello", {}))
    expect(400, lambda: server.api_project_ext_cli("p", "Hello", {"args": ["greet"]}))
    # 「CLI を呼ばない」は ext run が無いこと（一覧の ext list は呼んでよい）
    assert cli.calls_of("run") == [], f"検査の前に CLI が呼ばれた: {cli.calls}"

    # web に無いサブ命令は 403（CLI は呼ばない）
    expect(403, lambda: server.api_project_ext_cli("p", "hello", {"args": ["init"]}))
    assert cli.calls_of("run") == [], cli.calls

    # 有効でないプロジェクト・知らない拡張は 403。知らないプロジェクトは 404
    expect(403, lambda: server.api_project_ext_cli("p", "nope", {"args": ["greet"]}))
    use(cli, entry("hello", root, enabled=()))
    expect(403, lambda: server.api_project_ext_cli("p", "hello", {"args": ["greet"]}))
    expect(404, lambda: server.api_project_ext_cli("zzz", "hello", {"args": ["greet"]}))
    assert cli.calls_of("run") == [], cli.calls

    # CLI の終了コード 3 は 404
    use(cli, entry("hello", root, enabled=("p",)))
    cli.run_rc = 3
    cli.run_err = "item not found"
    expect(404, lambda: server.api_project_ext_cli("p", "hello", {"args": ["greet"]}))
    cli.run_rc = 0


def test_list_cache_and_failures(cli: FakeCli, hello: Path) -> None:
    """一覧は 10 秒覚える。CLI の失敗・壊れた JSON は空の一覧（例外にしない）。"""
    use(cli, entry("hello", hello))
    cli.calls.clear()
    first = server.api_ext()
    assert first["extensions"][0]["name"] == "hello", first
    assert first["extensions"][0]["ready"] is True, first
    server.api_ext()
    assert len(cli.calls_of("list")) == 1, cli.calls

    # 形が違う項目（service の宛先が無い等）は ready false で出す
    not_ready = {"url_env": SVC_ENV, "user_env": None, "password_env": None, "post": [], "ready": False}
    use(cli, entry("hello", hello, service=not_ready))
    listed = server.api_ext()["extensions"][0]
    assert listed["service"] is True and listed["ready"] is False, listed

    # CLI が失敗したら空の一覧
    ext.clear_cache()
    cli.list_rc = 1
    assert server.api_ext() == {"extensions": []}
    cli.list_rc = 0
    ext.clear_cache()
    cli.list_json = "{ 壊れている"
    assert server.api_ext() == {"extensions": []}
    ext.clear_cache()


class _Echo(BaseHTTPRequestHandler):
    """宛先の見本。GET は道・問い合わせ・認証を返し、POST は本文と Content-Type を返す。"""

    def log_message(self, *args) -> None:  # テストの出力を静かにする
        pass

    def _send(self, code: int, obj: dict) -> None:
        data = json.dumps(obj).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self) -> None:
        path, _, query = self.path.partition("?")
        if path == "/redir":
            self.send_response(302)
            self.send_header("Location", "http://127.0.0.1:9/elsewhere")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if path == "/missing":
            self._send(404, {"error": "no such item"})
            return
        self._send(200, {"method": "GET", "path": path, "query": query, "auth": self.headers.get("Authorization")})

    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length).decode("utf-8")
        self._send(200, {"method": "POST", "path": self.path, "body": body, "ctype": self.headers.get("Content-Type")})


def test_svc_relay(cli: FakeCli, hello: Path) -> None:
    """service は宛先へ中継する。POST は service.post の道だけ。宛先の失敗は 502。"""
    server_obj = ThreadingHTTPServer(("127.0.0.1", 0), _Echo)
    thread = threading.Thread(target=server_obj.serve_forever, daemon=True)
    thread.start()
    port = server_obj.server_address[1]
    os.environ[SVC_ENV] = f"http://127.0.0.1:{port}"
    os.environ["AGENT_LINK_EXT_HELLO_USER"] = "u"
    os.environ["AGENT_LINK_EXT_HELLO_PASS"] = "p"
    svc = {
        "url_env": SVC_ENV,
        "user_env": "AGENT_LINK_EXT_HELLO_USER",
        "password_env": "AGENT_LINK_EXT_HELLO_PASS",
        "post": ["api/items"],
        "ready": True,
    }
    try:
        use(cli, entry("hello", hello, service=svc))

        # GET は問い合わせ文字列と認証を付けて中継する
        got = json.loads(ext.relay("hello", "api/items", "GET", "a=1", None).body)
        assert got["method"] == "GET" and got["path"] == "/api/items" and got["query"] == "a=1", got
        token = base64.b64encode(b"u:p").decode()
        assert got["auth"] == f"Basic {token}", got

        # POST は service.post の道だけ。本文と Content-Type を送る
        got = json.loads(ext.relay("hello", "api/items", "POST", "", b'{"x": 1}').body)
        assert got["method"] == "POST" and got["body"] == '{"x": 1}', got
        assert got["ctype"] == "application/json", got
        expect(403, lambda: ext.relay("hello", "api/other", "POST", "", b"{}"))

        # 不正な道（..・空の区切り）は 400
        expect(400, lambda: ext.relay("hello", "api/../x", "GET", "", None))
        expect(400, lambda: ext.relay("hello", "/api", "GET", "", None))

        # 宛先の 404 は 502（理由は宛先の番号）
        e = expect(502, lambda: ext.relay("hello", "missing", "GET", "", None))
        assert str(e.detail) == "Extension service returned 404.", e.detail

        # リダイレクトは追わない（3xx は 502）
        e = expect(502, lambda: ext.relay("hello", "redir", "GET", "", None))
        assert str(e.detail) == "Extension service returned 302.", e.detail

        # 認証は両方の値があるときだけ付く
        os.environ.pop("AGENT_LINK_EXT_HELLO_PASS")
        got = json.loads(ext.relay("hello", "api/items", "GET", "", None).body)
        assert got["auth"] is None, got
        os.environ["AGENT_LINK_EXT_HELLO_PASS"] = "p"
    finally:
        server_obj.shutdown()
        server_obj.server_close()

    # 宛先が未設定（環境変数が無い）・service が無い拡張は 404
    os.environ.pop(SVC_ENV)
    expect(404, lambda: ext.relay("hello", "api/items", "GET", "", None))
    use(cli, entry("hello", hello, service=None))
    expect(404, lambda: ext.relay("hello", "api/items", "GET", "", None))

    # つながらない宛先は 502
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        closed_port = s.getsockname()[1]
    os.environ[SVC_ENV] = f"http://127.0.0.1:{closed_port}"
    use(cli, entry("hello", hello, service=svc))
    e = expect(502, lambda: ext.relay("hello", "api/items", "GET", "", None))
    assert str(e.detail) == "Could not reach the extension service.", e.detail
    os.environ.pop(SVC_ENV)
    os.environ.pop("AGENT_LINK_EXT_HELLO_USER")
    os.environ.pop("AGENT_LINK_EXT_HELLO_PASS")
    ext.clear_cache()


def main() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp)
        hello = make_tree(base)
        root = base / "p-root"
        root.mkdir()
        cli = FakeCli()
        original_run_cli = control.run_cli
        original_find = control.find_project
        control.run_cli = cli
        control.find_project = fake_find(
            {"p": {"name": "p", "root": str(root), "root_exists": True, "db_exists": True}}
        )
        try:
            test_index_shows_tab_style_script(cli, hello)
            test_index_hides_unready_extensions(cli, hello)
            test_index_escapes_values(cli, hello)
            test_index_without_extensions_is_stripped_original(cli)
            test_ui_file_inside_only(cli, hello)
            test_cli_success_and_checks(cli, root)
            test_list_cache_and_failures(cli, hello)
            test_svc_relay(cli, hello)
        finally:
            control.run_cli = original_run_cli
            control.find_project = original_find
            ext.clear_cache()
            shutil.rmtree(os.environ["AGENT_LINK_HOME"], ignore_errors=True)
    print("test_ext_api: すべて通りました")


main()
