"""多人数モードで利用者ごとに置き場が分かれること（実装計画 U06・AC3）のテスト。

確かめること:
- alice と bob の `projects/` と `homes/<name>/.sai/web/runs/` に別々のプロジェクトと
  実行を置くと、alice の `GET /api/projects`・`GET /api/days` に bob の物が出ない
- `GET /api/runs/<bob のプロジェクト>/<id>` は alice には 404
- alice のプロジェクトの書き出しに `transcript_path` が bob の HOME を指す物を置いても
  `/items` は 404

実行:
    cd ~/workdir/Sai/sai-tree
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_multi_user_paths.py
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
import tempfile
from pathlib import Path

# import より前に一時フォルダを決める（config.DB_PATH は import 時に決まるため）
_TMP = Path(tempfile.mkdtemp(prefix="sai-web-multi-"))
os.environ["SAI_HOME"] = str(_TMP / "sai-home")
os.environ["SAI_WEB_USERS_ROOT"] = str(_TMP / "users")
os.environ["SAI_WEB_COOKIE_SECURE"] = "0"
os.environ.pop("SAI_WEB_TOKEN", None)
os.environ.pop("SAI_WEB_RUNS_DIR", None)

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_web import auth, registry, runs, store  # noqa: E402
from sai_web.main import app  # noqa: E402

HOST = "testserver"
USERS = Path(os.environ["SAI_WEB_USERS_ROOT"])


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
    headers: dict[str, list[str]] = {}
    for key, value in start["headers"]:
        headers.setdefault(key.decode(), []).append(value.decode())
    raw = b"".join(m.get("body", b"") for m in messages if m["type"] == "http.response.body")
    try:
        parsed = json.loads(raw.decode("utf-8")) if raw else {}
    except (json.JSONDecodeError, UnicodeDecodeError):
        parsed = raw
    return start["status"], parsed, headers


def login(email: str, password: str) -> str:
    _, _, headers = call(
        "POST", "/api/auth/login", json_body={"email": email, "password": password}
    )
    for value in headers.get("set-cookie", []):
        if value.startswith(f"{auth.SESSION_COOKIE}="):
            return value.split(";", 1)[0].split("=", 1)[1]
    raise AssertionError("Cookie が付かない")


def add_user(name: str, email: str, password: str) -> None:
    auth.create_user_dirs(name)
    store.create_user(
        name, email, auth.hash_password(password), must_change_password=False
    )


def make_run(run_id: int, project_name: str, project_root: Path, transcript: Path) -> dict:
    """書き出し 1 件（schema_version 1・表示対象の形）。"""
    return {
        "schema_version": 1,
        "project_name": project_name,
        "project_root": str(project_root),
        "run_id": run_id,
        "parent_run_id": None,
        "block_name": "demo",
        "description": f"実行 {run_id}",
        "runner": "cc",
        "status": "completed",
        "model": None,
        "started_at": "2026-09-23T00:00:00Z",
        "finished_at": "2026-09-23T00:01:00Z",
        "duration_ms": 60000,
        "agent_session_id": None,
        "transcript_path": str(transcript),
        "task_md_path": None,
        "updated_at": "2026-09-23T00:01:00Z",
        "stats": None,
    }


def write_run(project_name: str, run: dict) -> Path:
    """そのプロジェクトの HOME の書き出しフォルダに 1 件書く。

    置き場は `<users>/<利用者>/homes/<利用者>-<プロジェクト>/.sai/web/runs/<名前>/`。
    利用者はプロジェクト名の最初のハイフンより前（名前にはハイフンを許さない・C4）。
    """
    user = project_name.split("-", 1)[0]
    home = USERS / user / "homes" / project_name
    out = home / ".sai" / "web" / "runs" / project_name
    out.mkdir(parents=True, exist_ok=True)
    path = out / f"{run['run_id']}.json"
    path.write_text(json.dumps(run, ensure_ascii=False), encoding="utf-8")
    return path


def main() -> None:
    store.init_db()

    add_user("alice", "alice@uni.edu", "alicepass1")
    add_user("bob", "bob@uni.edu", "bobpass123")

    alice_root = USERS / "alice" / "projects" / "alice-alpha"
    bob_root = USERS / "bob" / "projects" / "bob-beta"
    (alice_root / ".sai").mkdir(parents=True, exist_ok=True)
    (bob_root / ".sai").mkdir(parents=True, exist_ok=True)

    # 会話の記録（transcript）のダミー。alice の HOME と bob の HOME に 1 つずつ。
    alice_transcript = USERS / "alice" / "home" / "transcript.jsonl"
    bob_transcript = USERS / "bob" / "home" / "transcript.jsonl"
    for path in (alice_transcript, bob_transcript):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("", encoding="utf-8")

    # alice の実行 2 件。1 件は transcript が bob の HOME を指す（なりすまし）。
    write_run("alice-alpha", make_run(1, "alice-alpha", alice_root, alice_transcript))
    write_run("alice-alpha", make_run(3, "alice-alpha", alice_root, bob_transcript))
    # bob の実行
    write_run("bob-beta", make_run(2, "bob-beta", bob_root, bob_transcript))

    alice_cookie = login("alice@uni.edu", "alicepass1")
    bob_cookie = login("bob@uni.edu", "bobpass123")

    # --- alice の projects に bob の物が出ない ---
    status, body, _ = call("GET", "/api/projects", cookie=alice_cookie)
    assert status == 200, (status, body)
    assert [p["name"] for p in body["projects"]] == ["alice-alpha"], body

    # --- alice の days に bob の実行が出ない ---
    status, body, _ = call("GET", "/api/days", cookie=alice_cookie)
    assert status == 200, (status, body)
    assert body["days"], body
    status, day_body, _ = call("GET", "/api/days/2026-09-23", cookie=alice_cookie)
    assert status == 200, (status, day_body)
    project_names = {p["project_name"] for p in day_body["projects"]}
    assert project_names == {"alice-alpha"}, day_body

    # --- bob から見ると bob の物だけ ---
    status, day_body, _ = call("GET", "/api/days/2026-09-23", cookie=bob_cookie)
    assert status == 200, (status, day_body)
    project_names = {p["project_name"] for p in day_body["projects"]}
    assert project_names == {"bob-beta"}, day_body

    # --- bob のプロジェクトの実行は alice には 404 ---
    assert call("GET", "/api/runs/bob-beta/2", cookie=alice_cookie)[0] == 404
    # --- alice のプロジェクトの実行は bob には 404 ---
    assert call("GET", "/api/runs/alice-alpha/1", cookie=bob_cookie)[0] == 404
    # --- 自分の物は見える ---
    assert call("GET", "/api/runs/alice-alpha/1", cookie=alice_cookie)[0] == 200

    # --- transcript が他人の HOME を指す実行は /items が 404 ---
    status, body, _ = call("GET", "/api/runs/alice-alpha/3/items", cookie=alice_cookie)
    assert status == 404, (status, body)
    assert body["detail"] == "会話の記録が見つかりません", body

    # --- runs_dirs() が利用者のプロジェクトごとのフォルダを返す ---
    # （今の利用者はミドルウェアが入れる。ここでは直接確かめる）
    token = registry.current_user.set({"name": "alice", "email": "", "is_admin": False})
    try:
        dirs = runs.runs_dirs()
        assert any(str(d).endswith("homes/alice-alpha/.sai/web/runs") for d in dirs), dirs
        loaded = runs.load_runs()
        assert sorted(r["run_id"] for r in loaded) == [1, 3], loaded
    finally:
        registry.current_user.reset(token)

    print("test_multi_user_paths: すべて通りました")


if __name__ == "__main__":
    main()
