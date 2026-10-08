"""多人数モードのアカウント・ログイン・個人のトークン（実装計画 U06）のテスト。

確かめること（AC2・AC4）:
- 管理者を作り、大学のメールアドレスで login → Cookie で API が使える
- メールは大文字が混ざっても通る
- 初回は「パスワードを変えてください」で 403、変えたあとは 200
- 間違ったパスワードは 401
- 別の Origin からの Cookie の POST は 403
- 一般の人の /api/admin/users は 403
- 無効にした人の Cookie は 401
- 個人のトークンを作り、その Bearer で API が使える。消したあとは 401
- DB にパスワード・Cookie・トークンの値がそのまま入っていない
- 同期の住所の関数の中で `registry.current_user.get()` が取れる（AC4）

`fastapi.testclient` は httpx を要するが、この環境には入っていない（依存を増やさない）。
そこで ASGI を直接叩く小さな呼び出し係をここに置く。

実行:
    cd ~/workdir/Sai/sai-tree
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_auth.py
"""

from __future__ import annotations

import asyncio
import json
import os
import sqlite3
import sys
import tempfile
from pathlib import Path

# import より前に一時フォルダを決める（config.DB_PATH は import 時に決まるため）
_TMP = Path(tempfile.mkdtemp(prefix="sai-web-auth-"))
os.environ["SAI_HOME"] = str(_TMP / "sai-home")
os.environ["SAI_WEB_USERS_ROOT"] = str(_TMP / "users")
os.environ["SAI_WEB_COOKIE_SECURE"] = "0"
os.environ.pop("SAI_WEB_TOKEN", None)
os.environ.pop("SAI_WEB_RUNS_DIR", None)

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_web import auth, registry, store  # noqa: E402
from sai_web.config import DB_PATH  # noqa: E402
from sai_web.main import app  # noqa: E402

HOST = "testserver"


# --- ASGI を直接叩く呼び出し係（httpx を使わない） ---


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
        "client": ("127.0.0.1", 40000),
        "server": (HOST, 80),
    }
    messages: list[dict] = []
    state = {"sent": False}

    async def receive():
        if state["sent"]:
            return {"type": "http.disconnect"}
        state["sent"] = True
        return {"type": "http.request", "body": body, "more_body": False}

    async def send(message):
        messages.append(message)

    await app(scope, receive, send)
    return messages


def call(
    method: str,
    path: str,
    *,
    json_body: dict | None = None,
    headers: dict | None = None,
    cookie: str | None = None,
    origin: str | None = None,
) -> tuple[int, object, dict]:
    """1 つの住所を呼び、`(状態コード, 本文(JSON なら dict), 応答ヘッダ)` を返す。"""
    hdrs: list[tuple[bytes, bytes]] = [(b"host", HOST.encode())]
    body = b""
    if json_body is not None:
        body = json.dumps(json_body, ensure_ascii=False).encode("utf-8")
        hdrs.append((b"content-type", b"application/json"))
    if cookie:
        hdrs.append((b"cookie", f"{auth.SESSION_COOKIE}={cookie}".encode()))
    if origin:
        hdrs.append((b"origin", origin.encode()))
    for key, value in (headers or {}).items():
        hdrs.append((key.lower().encode(), value.encode()))
    messages = asyncio.run(_run_asgi(method, path, body, hdrs))
    start = next(m for m in messages if m["type"] == "http.response.start")
    out_headers: dict[str, list[str]] = {}
    for key, value in start["headers"]:
        out_headers.setdefault(key.decode(), []).append(value.decode())
    raw = b"".join(m.get("body", b"") for m in messages if m["type"] == "http.response.body")
    try:
        parsed = json.loads(raw.decode("utf-8")) if raw else {}
    except (json.JSONDecodeError, UnicodeDecodeError):
        parsed = raw
    return start["status"], parsed, out_headers


def _cookie_of(headers: dict) -> str | None:
    for value in headers.get("set-cookie", []):
        if value.startswith(f"{auth.SESSION_COOKIE}="):
            return value.split(";", 1)[0].split("=", 1)[1]
    return None


def login(email: str, password: str):
    status, body, headers = call(
        "POST", "/api/auth/login", json_body={"email": email, "password": password}
    )
    return status, body, _cookie_of(headers)


def add_user(
    name: str,
    email: str,
    password: str,
    *,
    is_admin: bool = False,
    must_change: bool = False,
    disabled: bool = False,
) -> None:
    auth.create_user_dirs(name)
    store.create_user(
        name,
        email,
        auth.hash_password(password),
        is_admin=is_admin,
        must_change_password=must_change,
        disabled=disabled,
    )


def main() -> None:
    store.init_db()

    # --- 管理者を作る（`sai-web user add --admin` 相当） ---
    add_user("alice", "Alice@Uni.EDU", "initialpass", is_admin=True, must_change=True)
    # 一般の人と、あとで無効にする人
    add_user("bob", "bob@uni.edu", "bobpass123", must_change=False)
    add_user("carol", "carol@uni.edu", "carolpass1", must_change=False)
    # 名前とメールが規則どおりか（検査の確認）
    assert auth.check_name("alice") == "alice"
    assert auth.check_email("Alice@Uni.EDU") == "alice@uni.edu"

    # --- login（大文字混じりのメールでも通る） ---
    status, body, cookie = login("ALICE@uni.edu", "initialpass")
    assert status == 200, (status, body)
    assert body["name"] == "alice" and body["is_admin"] is True, body
    assert body["must_change_password"] is True, body
    assert cookie, "Cookie が付かない"

    # --- Cookie 無しは 401 ---
    assert call("GET", "/api/projects")[0] == 401
    # --- 初回はパスワード変更を求める 403 ---
    status, body, _ = call("GET", "/api/projects", cookie=cookie)
    assert status == 403, (status, body)
    assert body["detail"] == "パスワードを変えてください", body

    # --- 間違ったパスワードは 401 ---
    assert login("alice@uni.edu", "wrong-password")[0] == 401
    # --- 居ないメールも 401 ---
    assert login("nobody@uni.edu", "initialpass")[0] == 401

    # --- 別の Origin からの Cookie の POST は 403（CSRF） ---
    status, body, _ = call(
        "POST", "/api/auth/logout", cookie=cookie, origin="http://evil.test"
    )
    assert status == 403, (status, body)
    assert body["detail"] == "別のサイトからの要求は受けません", body

    # --- パスワードを変える ---
    status, body, headers = call(
        "POST",
        "/api/auth/password",
        json_body={"current": "initialpass", "new": "newpassword1"},
        cookie=cookie,
    )
    assert status == 200 and body == {"ok": True}, (status, body)
    cookie2 = _cookie_of(headers)
    assert cookie2 and cookie2 != cookie, "新しい Cookie が配られていない"
    # 古い Cookie は消えている（他のセッションを消した）
    assert call("GET", "/api/projects", cookie=cookie)[0] == 401

    # --- 変えたあとは API が使える ---
    assert call("GET", "/api/projects", cookie=cookie2)[0] == 200
    # --- me は multi の形 ---
    status, body, _ = call("GET", "/api/auth/me", cookie=cookie2)
    assert status == 200 and body["mode"] == "multi", (status, body)
    assert body["name"] == "alice" and body["must_change_password"] is False, body

    # --- 単独モードの me は {"mode":"single"} ---
    saved_root = os.environ.pop("SAI_WEB_USERS_ROOT")
    try:
        status, body, _ = call("GET", "/api/auth/me")
        assert status == 200 and body == {"mode": "single"}, (status, body)
    finally:
        os.environ["SAI_WEB_USERS_ROOT"] = saved_root

    # --- 管理者の一覧 ---
    status, body, _ = call("GET", "/api/admin/users", cookie=cookie2)
    assert status == 200, (status, body)
    names = sorted(u["name"] for u in body["users"])
    assert names == ["alice", "bob", "carol"], body

    # --- 一般の人の /api/admin/users は 403 ---
    status, _, bob_cookie = login("bob@uni.edu", "bobpass123")
    assert status == 200, status
    status, body, _ = call("GET", "/api/admin/users", cookie=bob_cookie)
    assert status == 403 and body["detail"] == "管理者だけが使えます", (status, body)

    # --- 自分自身の管理者外し・無効化は 400 ---
    assert (
        call(
            "PATCH",
            "/api/admin/users/alice",
            json_body={"is_admin": False},
            cookie=cookie2,
        )[0]
        == 400
    )
    assert (
        call(
            "PATCH",
            "/api/admin/users/alice",
            json_body={"disabled": True},
            cookie=cookie2,
        )[0]
        == 400
    )

    # --- 一般の人を作る（管理者が作る） ---
    status, body, _ = call(
        "POST",
        "/api/admin/users",
        json_body={
            "name": "dave",
            "email": "dave@uni.edu",
            "password": "davepass99",
            "is_admin": False,
        },
        cookie=cookie2,
    )
    assert status == 200 and body["must_change_password"] is True, (status, body)
    # 同じ名前・同じメールは 409
    assert (
        call(
            "POST",
            "/api/admin/users",
            json_body={
                "name": "dave",
                "email": "other@uni.edu",
                "password": "davepass99",
            },
            cookie=cookie2,
        )[0]
        == 409
    )
    assert (
        call(
            "POST",
            "/api/admin/users",
            json_body={
                "name": "dave2",
                "email": "dave@uni.edu",
                "password": "davepass99",
            },
            cookie=cookie2,
        )[0]
        == 409
    )
    # C4 のフォルダができる
    users_root = registry.users_root()
    assert users_root is not None
    assert (users_root / "dave" / "home" / ".sai").is_dir()
    assert (users_root / "dave" / "projects").is_dir()
    assert (users_root / "dave" / "homes").is_dir()

    # --- 無効にした人の Cookie は 401 ---
    status, _, carol_cookie = login("carol@uni.edu", "carolpass1")
    assert status == 200, status
    assert call("GET", "/api/projects", cookie=carol_cookie)[0] == 200
    assert (
        call(
            "PATCH",
            "/api/admin/users/carol",
            json_body={"disabled": True},
            cookie=cookie2,
        )[0]
        == 200
    )
    assert call("GET", "/api/projects", cookie=carol_cookie)[0] == 401
    # 無効な人は login できない
    assert login("carol@uni.edu", "carolpass1")[0] == 401

    # --- AC4: 同期の住所の関数の中で current_user が取れる ---
    # alice のプロジェクトを 1 つ置くと、`GET /api/projects`（同期の `def`）が
    # registry.load_registry() 越しに今の利用者のプロジェクトを返す。
    project = users_root / "alice" / "projects" / "alice-demo"
    (project / ".sai").mkdir(parents=True, exist_ok=True)
    status, body, _ = call("GET", "/api/projects", cookie=cookie2)
    assert status == 200, (status, body)
    assert [p["name"] for p in body["projects"]] == ["alice-demo"], body

    # --- 個人のトークン ---
    status, body, _ = call("GET", "/api/me/tokens", cookie=cookie2)
    assert status == 200 and body == {"tokens": []}, (status, body)
    status, body, _ = call(
        "POST", "/api/me/tokens", json_body={"label": "sai-prep (MacBook)"}, cookie=cookie2
    )
    assert status == 200, (status, body)
    token = body["token"]
    token_id = body["id"]
    assert token.startswith("sai_"), body
    assert len(token_id) == 12, body
    # 一覧には値が出ない
    status, body, _ = call("GET", "/api/me/tokens", cookie=cookie2)
    assert status == 200 and len(body["tokens"]) == 1, body
    assert "token" not in body["tokens"][0], body["tokens"][0]
    assert body["tokens"][0]["label"] == "sai-prep (MacBook)", body
    # ラベルが空・長すぎは 400
    assert (
        call("POST", "/api/me/tokens", json_body={"label": ""}, cookie=cookie2)[0] == 400
    )
    assert (
        call("POST", "/api/me/tokens", json_body={"label": "x" * 61}, cookie=cookie2)[0]
        == 400
    )

    # トークンの Bearer で API が使える
    bearer = {"authorization": f"Bearer {token}"}
    assert call("GET", "/api/projects", headers=bearer)[0] == 200
    # 個人のトークンでは管理はできない（管理者でない bob のトークンで確かめる）
    status, _, bob_cookie2 = login("bob@uni.edu", "bobpass123")
    status, body, _ = call(
        "POST", "/api/me/tokens", json_body={"label": "bob-prep"}, cookie=bob_cookie2
    )
    bob_token = body["token"]
    assert (
        call(
            "GET",
            "/api/admin/users",
            headers={"authorization": f"Bearer {bob_token}"},
        )[0]
        == 403
    )

    # 消したあとは 401
    assert call("DELETE", f"/api/me/tokens/{token_id}", cookie=cookie2)[0] == 200
    assert call("GET", "/api/projects", headers=bearer)[0] == 401
    # 無い id は 404
    assert call("DELETE", "/api/me/tokens/deadbeef0000", cookie=cookie2)[0] == 404

    # --- DB に値そのものが入っていない ---
    with sqlite3.connect(DB_PATH) as conn:
        password_hash = conn.execute(
            "SELECT password_hash FROM users WHERE name = 'alice'"
        ).fetchone()[0]
    assert password_hash.startswith("scrypt$"), password_hash
    raw = Path(DB_PATH).read_bytes()
    for secret in ("initialpass", "newpassword1", cookie, cookie2, token, bob_token):
        assert secret.encode("utf-8") not in raw, f"DB に値がそのまま入っている: {secret!r}"

    print("test_auth: すべて通りました")


if __name__ == "__main__":
    main()
