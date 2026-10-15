"""headless の実行の状態と，送信・中断・質問の API の小さなテスト（U03）。

確かめること（共有契約 C1・C2・C6）:
- `headless_state` が `state.json` を読み，`stopped` は `live` が None（AC2）
- `status` が `completed` でも `idle` の headless 実行は `live == "idle"`（AC2）
- `GET …/items` が `partial` を返し，`transcript_path` が無くても `live` と `partial` を返す（AC2）
- `POST …/send` が `agent-link run send <id> --json` を本文を標準入力にして呼ぶ（AC3）
- `POST …/interrupt` は無い実行で CLI を呼ばずに 404（AC3）
- cc の質問に答えると，キーを送らずに `answered` になり `GET /api/questions/{qid}` が答えを返す（AC4）
- 承認の住所（`/api/approvals*`）が無い

実行:
    PYTHONPATH=src .venv/bin/python tests/test_headless_runs.py
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

# リポジトリ直下から実行しなくても import できるようにしておく
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from fastapi import HTTPException  # noqa: E402

from agent_link_web import control, main, questions, runs, store  # noqa: E402

PANE_ID = "hl-0123456789ab"


def expect(code: int, fn) -> HTTPException:
    """fn は指定した状態番号の HTTPException で失敗し，日本語の理由が付くこと。"""
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{code} のときは日本語で理由を書く"
        return e
    raise AssertionError(f"{code} になるはず")


class FakeCli:
    """`control.run_cli` の置き換え係。呼ばれた引数と stdin を全部覚えておく。"""

    def __init__(self) -> None:
        self.calls: list[dict] = []
        self.returncode = 0
        self.stdout = "{}"
        self.stderr = ""

    def __call__(self, args, *, app, cwd=None, env=None, timeout=20.0, input_text=None):
        self.calls.append(
            {
                "args": args,
                "app": app,
                "cwd": cwd,
                "env": env,
                "timeout": timeout,
                "input_text": input_text,
            }
        )
        return subprocess.CompletedProcess(args, self.returncode, self.stdout, self.stderr)

    @property
    def last(self) -> dict:
        assert self.calls, "CLI が呼ばれていない"
        return self.calls[-1]


def make_find_project(rows: list[dict]):
    """登録簿の行を想定した find_project（未知の名前は 404・control 自身と同じ形）。"""

    def find(name: str) -> dict:
        for p in rows:
            if p["name"] == name:
                return p
        raise HTTPException(status_code=404, detail="プロジェクトが見つかりません")

    return find


def fresh_db(tmp: str) -> None:
    """テスト用の一時 DB に差し替える（開発用の DB を汚さない）。"""
    path = Path(tmp) / "test.db"
    if path.exists():
        path.unlink()
    store.DB_PATH = path
    store.init_db()


def make_run(
    run_id: int,
    *,
    pane_id: str | None = None,
    project_root: str = "/tmp/demo-root",
    status: str = "running",
    runner: str = "cc",
    transcript_path: str | None = None,
    project_name: str = "demo",
) -> dict:
    """00 §3.2 の形のダミー実行を作る。"""
    return {
        "schema_version": 1,
        "project_name": project_name,
        "project_root": project_root,
        "run_id": run_id,
        "parent_run_id": None,
        "block_name": "sai2",
        "description": f"タスク{run_id}",
        "runner": runner,
        "status": status,
        "model": None,
        "started_at": "2026-09-23T00:00:00Z",
        "finished_at": None,
        "duration_ms": None,
        "herdr_pane_id": pane_id,
        "agent_session_id": "sess-1",
        "transcript_path": transcript_path,
        "task_md_path": None,
        "updated_at": "2026-09-23T00:00:00Z",
        "stats": None,
    }


def write_state(
    root: str,
    state: str,
    *,
    partial: str = "",
    error: str = "",
    pane_id: str = PANE_ID,
) -> Path:
    """headless の `state.json`（C1）を一時フォルダに書く。"""
    headless = Path(root) / ".agent-link" / "engine" / "headless" / pane_id
    headless.mkdir(parents=True, exist_ok=True)
    path = headless / "state.json"
    path.write_text(
        json.dumps(
            {
                "pane_id": pane_id,
                "runner": "cc",
                "state": state,
                "turn": 1,
                "partial": partial,
                "error": error,
                "updated_at": "2026-09-23T00:00:00+09:00",
            }
        ),
        encoding="utf-8",
    )
    return path


def test_headless_state_reads_state_json() -> None:
    """`headless_state` が `state.json` を読み，live の値になる（AC2）。"""
    with tempfile.TemporaryDirectory() as tmp:
        write_state(tmp, "working", partial="生成中の文")
        run = make_run(1, pane_id=PANE_ID, project_root=tmp)
        st = runs.headless_state(run)
        assert st is not None, "state.json を読めていない"
        assert st["state"] == "working", st
        assert st["partial"] == "生成中の文", st
        assert runs.live_status(run) == "working"

        # stopped は live が None
        write_state(tmp, "stopped")
        assert runs.headless_state(run)["state"] == "stopped"
        assert runs.live_status(run) is None

        # status が completed でも idle の headless 実行は live == "idle"
        write_state(tmp, "idle")
        completed = make_run(1, pane_id=PANE_ID, project_root=tmp, status="completed")
        assert runs.live_status(completed) == "idle"

        # 形の違う id・読めない JSON は None
        assert runs.headless_state(make_run(1, pane_id="w1:p1", project_root=tmp)) is None
        assert runs.headless_state(make_run(1, pane_id="hl-zzzzzzzzzzzz", project_root=tmp)) is None
        assert runs.headless_state(make_run(1, pane_id=PANE_ID, project_root="/tmp/gone")) is None
        write_state(tmp, "working")
        path = Path(tmp) / ".agent-link" / "engine" / "headless" / PANE_ID / "state.json"
        path.write_text("{ broken", encoding="utf-8")
        assert runs.headless_state(run) is None

        # `headless` の印は `hl-` で始まるか
        assert runs.is_headless(run) is True
        assert runs.is_headless(make_run(1, pane_id="w1:p1", project_root=tmp)) is False


def test_items_returns_partial_without_transcript() -> None:
    """`/items` が `partial` を返し，transcript が無くても live と partial を返す（AC2）。"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        write_state(tmp, "working", partial="途中の文", error="")
        run = make_run(7, pane_id=PANE_ID, project_root=tmp, transcript_path=None)
        original = runs.load_runs
        runs.load_runs = lambda: [run]  # type: ignore[assignment]
        try:
            body = main.api_run_items("demo", 7)
        finally:
            runs.load_runs = original  # type: ignore[assignment]
        assert body["items"] == [] and body["next_cursor"] == "", body
        assert body["live"] == "working", body
        assert body["partial"] == "途中の文", body
        assert body["error"] == "", body

        # 置き場所は分かっているがファイルが無いときも同じ形で返す
        missing = str(Path(tmp) / "no-such.jsonl")
        run2 = make_run(8, pane_id=PANE_ID, project_root=tmp, transcript_path=missing)
        runs.load_runs = lambda: [run2]  # type: ignore[assignment]
        try:
            body = main.api_run_items("demo", 8)
        finally:
            runs.load_runs = original  # type: ignore[assignment]
        assert body["partial"] == "途中の文", body
        assert body["live"] == "working", body


def test_items_live_blocked_when_question_open() -> None:
    """open の質問がある実行は `live` が `blocked`（共有契約 C6）。"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        store.save_question("claude", "sess-1", "", "tu-9", [{"question": "どれ？"}])
        run = make_run(9, pane_id=PANE_ID, project_root=tmp, transcript_path=None)
        original = runs.load_runs
        runs.load_runs = lambda: [run]  # type: ignore[assignment]
        try:
            body = main.api_run_items("demo", 9)
        finally:
            runs.load_runs = original  # type: ignore[assignment]
        assert body["live"] == "blocked", body


def test_send_calls_run_send_with_stdin() -> None:
    """`/send` が `agent-link run send <id> --json` を本文を標準入力にして呼ぶ（AC3）。"""
    fresh_db(tempfile.mkdtemp(prefix="agent-link-web-headless-"))  # 質問の取り下げを見るため DB が要る
    cli = FakeCli()
    cli.returncode = 0
    cli.stdout = json.dumps({"run_id": 42, "pane_id": PANE_ID, "status": "sent"})
    original_run_cli = control.run_cli
    original_find = control.find_project
    control.run_cli = cli
    control.find_project = make_find_project(
        [{"name": "demo", "root": "/tmp/demo-root", "root_exists": True, "db_exists": True}]
    )
    try:
        out = main.api_send_text("demo", 42, main.SendRequest(text="続きの文"))
        assert out == {"run_id": 42, "pane_id": PANE_ID, "status": "sent"}, out
        call = cli.last
        assert call["args"] == ["agent-link", "run", "send", "42"], call["args"]
        assert call["input_text"] == "続きの文", call["input_text"]
        assert call["cwd"] == "/tmp/demo-root", call["cwd"]
        assert call["env"]["AGENT_LINK_PROJECT_ROOT"] == "/tmp/demo-root"

        # 空の文は CLI を呼ぶ前に 400
        cli.calls.clear()
        expect(400, lambda: main.api_send_text("demo", 42, main.SendRequest(text="  ")))
        assert cli.calls == [], cli.calls
    finally:
        control.run_cli = original_run_cli
        control.find_project = original_find


def test_interrupt_missing_run_is_404_without_cli() -> None:
    """`/interrupt` は無い実行で CLI を呼ばずに 404（AC3）。"""
    fresh_db(tempfile.mkdtemp(prefix="agent-link-web-headless-"))  # 質問の取り下げを見るため DB が要る
    cli = FakeCli()
    original_run_cli = control.run_cli
    original_load = runs.load_runs
    control.run_cli = cli
    runs.load_runs = lambda: []  # type: ignore[assignment]
    try:
        expect(404, lambda: main.api_interrupt("demo", 999))
        assert cli.calls == [], cli.calls
    finally:
        control.run_cli = original_run_cli
        runs.load_runs = original_load  # type: ignore[assignment]

    # ある実行なら `agent-link run interrupt <id> --json` を呼び，応答をそのまま返す
    cli.returncode = 0
    cli.stdout = json.dumps(
        {"run_id": 42, "status": "interrupted", "agent_status": "idle"}
    )
    run = make_run(42, pane_id=PANE_ID, project_root="/tmp/demo-root")
    original_find = control.find_project
    control.run_cli = cli
    runs.load_runs = lambda: [run]  # type: ignore[assignment]
    control.find_project = make_find_project(
        [{"name": "demo", "root": "/tmp/demo-root", "root_exists": True, "db_exists": True}]
    )
    try:
        out = main.api_interrupt("demo", 42)
        assert out["status"] == "interrupted", out
        assert cli.last["args"] == ["agent-link", "run", "interrupt", "42"], cli.last["args"]
    finally:
        runs.load_runs = original_load  # type: ignore[assignment]
        control.find_project = original_find
        control.run_cli = original_run_cli


def test_cc_question_answer_and_get() -> None:
    """cc の質問に答えると answered になり，GET /api/questions/{qid} が答えを返す（AC4）。"""
    with tempfile.TemporaryDirectory() as tmp:
        fresh_db(tmp)
        q = store.save_question(
            "claude",
            "sess-1",
            "",
            "tu-1",
            [{"question": "どれにしますか？", "options": [{"label": "あ"}, {"label": "い"}]}],
        )
        qid = q["id"]
        ok, reason = questions.answer_question(qid, [{"choice_index": 0}])
        assert ok is True, reason
        assert store.get_question(qid)["state"] == "answered"

        body = main.api_get_question(qid)
        assert body["state"] == "answered", body
        assert body["answer"]["answers"] == [{"choice_index": 0}], body


def test_routes() -> None:
    """新しい住所があり，承認の住所は無い（共有契約 C6）。"""
    paths = set()
    for route in main.app.routes:
        for method in getattr(route, "methods", None) or []:
            paths.add((route.path, method))

    assert ("/api/runs/{project_name}/{run_id}/items", "GET") in paths
    assert ("/api/runs/{project_name}/{run_id}/send", "POST") in paths
    assert ("/api/runs/{project_name}/{run_id}/interrupt", "POST") in paths
    assert ("/api/home", "GET") in paths
    assert not [p for p, _ in paths if p.startswith("/api/approvals")], (
        "承認の住所が残っている"
    )


def main_test() -> None:
    test_headless_state_reads_state_json()
    test_items_returns_partial_without_transcript()
    test_items_live_blocked_when_question_open()
    test_send_calls_run_send_with_stdin()
    test_interrupt_missing_run_is_404_without_cli()
    test_cc_question_answer_and_get()
    test_routes()
    print("test_headless_runs: すべて通りました")


if __name__ == "__main__":
    main_test()
