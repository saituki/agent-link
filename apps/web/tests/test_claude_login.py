"""Claude のログインの中継（実装計画 U10・AC1）のテスト。

偽の docker（`SAI_DOCKER_BIN`）が `claude auth login` の端末の振る舞いを真似る。
- `auth login`: ANSI 付きの URL と `Paste code here if prompted > ` を出し、1 行読んで
  `GOOD` なら `Login successful.`、`nourl` なら URL を出さずに終わる、それ以外は
  `error: rejected <行>`（貼った行を端末が echo する）。
- `auth status`: 設定（config.json）の `logged_in` に従って JSON と終了コードを返す。

確かめること:
- `start` が URL を返す（ANSI と OSC 8 が除かれている）。URL が出ないと 502。
- `submit("GOOD")` が成功し、API では `CLAUDE_CODE_OAUTH_TOKEN` が消えて `recreate_for_user` が呼ばれる。
- `submit("BAD")` が 502。理由にコードの値が出ない。
- ログインを始めていないと 409。コードの検査（空白・長さ・非 ASCII）は 400。
- `GET /api/me/claude-login` は値を返さず、`token_set` で token の有無だけを返す。
- 10 分（ここでは短くした値）で補助のコンテナを止める。

実行:
    cd ~/workdir/Sai/sai-tree
    env -u SAI_LINK_BIN -u SAI_PAPER_BIN -u SAI_SCI_BIN PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_claude_login.py
"""

from __future__ import annotations

import asyncio
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

# import より前に一時フォルダを決める（config.DB_PATH は import 時に決まるため）
_TMP = Path(tempfile.mkdtemp(prefix="sai-web-claude-login-"))
os.environ["SAI_HOME"] = str(_TMP / "sai-home")
os.environ["SAI_WEB_USERS_ROOT"] = str(_TMP / "users")
os.environ["SAI_WEB_COOKIE_SECURE"] = "0"
os.environ.pop("SAI_WEB_TOKEN", None)
os.environ.pop("SAI_WEB_RUNS_DIR", None)
os.environ.pop("SAI_WEB_SKEL_DIR", None)
os.environ.pop("SAI_IMAGE", None)

# 偽の docker（呼ばれた引数を log.jsonl に記録し、config.json に従って応答する）
_FAKE_DIR = _TMP / "fake-docker"
_FAKE_DIR.mkdir(parents=True)
_FAKE_BIN = _TMP / "fake-docker-bin"
_FAKE_BIN.write_text(
    f'''#!{sys.executable}
"""テスト用の偽 docker（claude auth login・auth status・rm だけを真似る）。"""
import json
import os
import sys

d = os.environ["FAKE_DOCKER_DIR"]
args = sys.argv[1:]
with open(os.path.join(d, "log.jsonl"), "a", encoding="utf-8") as f:
    f.write(json.dumps(args, ensure_ascii=False) + "\\n")

cfg = {{}}
cfg_path = os.path.join(d, "config.json")
if os.path.exists(cfg_path):
    with open(cfg_path, encoding="utf-8") as f:
        cfg = json.load(f)


def out(text):
    sys.stdout.write(text)
    sys.stdout.flush()


if "login" in args and "auth" in args:
    mode = cfg.get("login_mode", "ok")
    if mode == "nourl":
        out("Choose the text style to use\\r\\n")
        sys.exit(1)
    url = "https://claude.ai/oauth/authorize?code=true&state=S1"
    # OSC 8 の hyperlink（URL が 2 回出る）と色の制御文字を含める
    out("Browser didn't open? Use the url below to sign in:\\r\\n")
    out(f"\\x1b]8;;{{url}}\\x1b\\\\\\x1b[36m{{url}}\\x1b[39m\\x1b]8;;\\x1b\\\\\\r\\n")
    out("\\x1b[2mPaste code here if prompted >\\x1b[0m ")
    line = sys.stdin.readline().rstrip("\\r\\n")
    if line == "GOOD":
        out("\\r\\nLogin successful.\\r\\n")
        sys.exit(0)
    out(f"{{line}}\\r\\nerror: rejected {{line}}\\r\\n")
    sys.exit(1)

if "status" in args and "auth" in args:
    if cfg.get("logged_in", False):
        out(json.dumps({{"loggedIn": True, "authMethod": "claude.ai"}}) + "\\n")
        sys.exit(0)
    out(json.dumps({{"loggedIn": False}}) + "\\n")
    sys.exit(1)

sys.exit(0)
''',
    encoding="utf-8",
)
os.chmod(_FAKE_BIN, 0o755)
os.environ["SAI_DOCKER_BIN"] = str(_FAKE_BIN)
os.environ["FAKE_DOCKER_DIR"] = str(_FAKE_DIR)

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from fastapi import HTTPException  # noqa: E402

from sai_web import auth, claude_login, containers, store  # noqa: E402
from sai_web.main import app  # noqa: E402

HOST = "testserver"
USERS = Path(os.environ["SAI_WEB_USERS_ROOT"])
URL = "https://claude.ai/oauth/authorize?code=true&state=S1"


def set_fake(**cfg) -> None:
    (_FAKE_DIR / "config.json").write_text(json.dumps(cfg), encoding="utf-8")


def docker_calls() -> list[list[str]]:
    path = _FAKE_DIR / "log.jsonl"
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def clear_docker_log() -> None:
    path = _FAKE_DIR / "log.jsonl"
    if path.exists():
        path.unlink()


def expect(code: int, fn) -> HTTPException:
    """fn は指定した状態番号の HTTPException で失敗し、日本語の理由が付くこと。"""
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{code} のときは日本語で理由を書く"
        return e
    raise AssertionError(f"{code} になるはず")


def call(method: str, path: str, *, json_body: dict | None = None, cookie: str | None = None):
    hdrs: list[tuple[bytes, bytes]] = [(b"host", HOST.encode())]
    body = b""
    if json_body is not None:
        body = json.dumps(json_body, ensure_ascii=False).encode("utf-8")
        hdrs.append((b"content-type", b"application/json"))
    if cookie:
        hdrs.append((b"cookie", f"{auth.SESSION_COOKIE}={cookie}".encode()))
    messages = asyncio.run(_run_asgi(method, path, body, hdrs))
    start = next(m for m in messages if m["type"] == "http.response.start")
    raw = b"".join(m.get("body", b"") for m in messages if m["type"] == "http.response.body")
    try:
        parsed = json.loads(raw.decode("utf-8")) if raw else {}
    except (json.JSONDecodeError, UnicodeDecodeError):
        parsed = raw
    headers: dict[str, list[str]] = {}
    for key, value in start["headers"]:
        headers.setdefault(key.decode(), []).append(value.decode())
    return start["status"], parsed, headers


async def _run_asgi(method: str, path: str, body: bytes, headers: list[tuple[bytes, bytes]]):
    scope = {
        "type": "http",
        "asgi": {"version": "3.0"},
        "http_version": "1.1",
        "method": method,
        "scheme": "http",
        "path": path,
        "raw_path": path.encode(),
        "query_string": b"",
        "root_path": "",
        "headers": headers,
        "client": ("127.0.0.1", 12345),
        "server": ("testserver", 80),
    }
    sent = False
    messages: list[dict] = []

    async def receive():
        nonlocal sent
        if sent:
            return {"type": "http.disconnect"}
        sent = True
        return {"type": "http.request", "body": body, "more_body": False}

    async def send(message):
        messages.append(message)

    await app(scope, receive, send)
    return messages


def login(email: str, password: str) -> str:
    _, _, headers = call("POST", "/api/auth/login", json_body={"email": email, "password": password})
    for value in headers.get("set-cookie", []):
        if value.startswith(f"{auth.SESSION_COOKIE}="):
            return value.split(";", 1)[0].split("=", 1)[1]
    raise AssertionError("Cookie が付かない")


def add_user(name: str, email: str, password: str) -> None:
    auth.create_user_dirs(name)
    store.create_user(
        name,
        email,
        auth.hash_password(password),
        is_admin=False,
        must_change_password=False,
    )


def reset_login() -> None:
    """進行中のログインを止める（テストの間で持ち越さない）。"""
    claude_login.cancel("alice")


# --- テスト本体 ---


def test_start_returns_url() -> None:
    """URL が出るまで待ち、ANSI と OSC 8 を除いた URL を返す（同じ URL が 2 回出ても 1 つ）。"""
    set_fake(logged_in=False)
    clear_docker_log()
    try:
        got = claude_login.start("alice")
        assert got == {"url": URL}, got
        calls = docker_calls()
        run = next(c for c in calls if c and c[0] == "run")
        assert "--rm" in run and "-i" in run and "-t" in run, run
        assert "sai-login-alice" in run, run
        assert f"{USERS / 'alice' / 'claude'}:/home/login/.claude" in run, run
        assert "HOME=/home/login" in run and "COLUMNS=1000" in run, run
        assert run[-3:] == ["claude", "auth", "login"], run
    finally:
        reset_login()


def test_start_without_url_is_502() -> None:
    """URL の前に別の画面で終わったら 502 で、その最後の行を理由に出す。"""
    set_fake(login_mode="nourl")
    try:
        e = expect(502, lambda: claude_login.start("alice"))
        assert "URL を取れませんでした" in str(e.detail), e.detail
        assert "Choose the text style" in str(e.detail), e.detail
    finally:
        reset_login()


def test_submit_good_and_bad() -> None:
    """GOOD は成功。BAD は 502 で、理由に貼ったコードの値が出ない。どちらも登録は外れる。"""
    set_fake(login_mode="ok", logged_in=True)
    claude_login.start("alice")
    e = expect(502, lambda: claude_login.submit("alice", "BAD"))
    assert "BAD" not in str(e.detail), e.detail
    assert "error: rejected" in str(e.detail), e.detail
    assert "alice" not in claude_login._sessions

    claude_login.start("alice")
    assert claude_login.submit("alice", "GOOD") == {"ok": True}
    assert "alice" not in claude_login._sessions


def test_submit_without_session_is_409() -> None:
    reset_login()
    expect(409, lambda: claude_login.submit("alice", "GOOD"))


def test_submit_code_validation_is_400() -> None:
    """検査は セッションの有無より先（空白・長さ・非 ASCII は 400）。"""
    for bad in ["", "a b", "x" * 501, "コード", "a\nb", None, 123]:
        expect(400, lambda bad=bad: claude_login.submit("alice", bad))


def test_status_parses_json_and_exit_code() -> None:
    """ログイン中なら logged_in が真で authMethod を返す。中身（メールなど）は返さない。"""
    set_fake(logged_in=True)
    got = claude_login.status("alice")
    assert got == {"logged_in": True, "auth_method": "claude.ai"}, got
    set_fake(logged_in=False)
    got = claude_login.status("alice")
    assert got == {"logged_in": False, "auth_method": ""}, got


def test_session_expires_and_stops_container() -> None:
    """期限（ここでは 1 秒）で登録を外し、補助のコンテナを消す。"""
    set_fake(login_mode="ok")
    old_ttl = claude_login._SESSION_TTL
    claude_login._SESSION_TTL = 1.0
    clear_docker_log()
    try:
        claude_login.start("alice")
        assert "alice" in claude_login._sessions
        deadline = time.monotonic() + 10.0
        while "alice" in claude_login._sessions and time.monotonic() < deadline:
            time.sleep(0.1)
        assert "alice" not in claude_login._sessions, "期限で登録が外れない"
        assert ["rm", "-f", "sai-login-alice"] in docker_calls(), docker_calls()
    finally:
        claude_login._SESSION_TTL = old_ttl
        reset_login()


def test_api_login_flow() -> None:
    """API: 状態 → URL → 誤ったコード → 正しいコード。token は消えて作り直しが呼ばれる。"""
    cookie = login("alice@uni.edu", "alicepass1")
    set_fake(logged_in=False)

    # token を入れておく（成功したら消えること）
    status, body, _ = call(
        "PUT",
        "/api/me/keys",
        json_body={"CLAUDE_CODE_OAUTH_TOKEN": "sk-ant-oat01-SECRETVALUE"},
        cookie=cookie,
    )
    assert status == 200, (status, body)

    status, body, _ = call("GET", "/api/me/claude-login", cookie=cookie)
    assert status == 200, (status, body)
    assert body == {"logged_in": False, "auth_method": "", "token_set": True}, body

    status, body, _ = call("POST", "/api/me/claude-login/start", cookie=cookie)
    assert status == 200 and body == {"url": URL}, (status, body)

    status, body, _ = call("POST", "/api/me/claude-login/code", json_body={"code": "BAD"}, cookie=cookie)
    assert status == 502, (status, body)
    assert "BAD" not in json.dumps(body, ensure_ascii=False), body

    # 失敗で登録は外れる。もう一度始めてから成功させる
    status, body, _ = call("POST", "/api/me/claude-login/start", cookie=cookie)
    assert status == 200 and body == {"url": URL}, (status, body)

    recorded: list[str] = []
    original = containers.recreate_for_user

    def fake_recreate(user: str) -> dict:
        recorded.append(user)
        return {"recreated": ["alice-x"], "pending": ["alice-y"]}

    containers.recreate_for_user = fake_recreate
    try:
        status, body, _ = call("POST", "/api/me/claude-login/code", json_body={"code": "GOOD"}, cookie=cookie)
    finally:
        containers.recreate_for_user = original
    assert status == 200, (status, body)
    assert body == {"ok": True, "recreated": ["alice-x"], "pending": ["alice-y"]}, body
    assert recorded == ["alice"], recorded

    # secrets.env から token が消えている（環境変数の token がファイルのログインより優先されるため）
    secrets = (USERS / "alice" / "secrets.env").read_text(encoding="utf-8")
    assert "CLAUDE_CODE_OAUTH_TOKEN" not in secrets, "token が消えていない"

    # 状態: ログイン済み・token は無し。値はどの返り値にも出ない
    set_fake(logged_in=True)
    status, body, _ = call("GET", "/api/me/claude-login", cookie=cookie)
    assert body == {"logged_in": True, "auth_method": "claude.ai", "token_set": False}, body
    assert "SECRETVALUE" not in json.dumps(body), body


def test_api_code_without_session_is_409() -> None:
    cookie = login("alice@uni.edu", "alicepass1")
    reset_login()
    status, body, _ = call("POST", "/api/me/claude-login/code", json_body={"code": "GOOD"}, cookie=cookie)
    assert status == 409, (status, body)


def test_api_cancel() -> None:
    cookie = login("alice@uni.edu", "alicepass1")
    status, body, _ = call("POST", "/api/me/claude-login/start", cookie=cookie)
    assert status == 200, (status, body)
    assert "alice" in claude_login._sessions
    status, body, _ = call("DELETE", "/api/me/claude-login", cookie=cookie)
    assert status == 200 and body == {"ok": True}, (status, body)
    assert "alice" not in claude_login._sessions


def test_api_requires_multi_user() -> None:
    """多人数モードでなければ 404（単独モードでは使わない）。"""
    cookie = login("alice@uni.edu", "alicepass1")
    old = os.environ.pop("SAI_WEB_USERS_ROOT")
    try:
        status, body, _ = call("GET", "/api/me/claude-login", cookie=cookie)
    finally:
        os.environ["SAI_WEB_USERS_ROOT"] = old
    assert status == 404, (status, body)


def main() -> None:
    store.init_db()
    add_user("alice", "alice@uni.edu", "alicepass1")
    test_start_returns_url()
    test_start_without_url_is_502()
    test_submit_good_and_bad()
    test_submit_without_session_is_409()
    test_submit_code_validation_is_400()
    test_status_parses_json_and_exit_code()
    test_session_expires_and_stops_container()
    test_api_login_flow()
    test_api_code_without_session_is_409()
    test_api_cancel()
    test_api_requires_multi_user()
    print("test_claude_login: すべて通りました")


if __name__ == "__main__":
    main()
