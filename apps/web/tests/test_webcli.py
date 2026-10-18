"""`agent-link-web`（Web の窓口の CLI）のテスト。

本物の Web のサーバ（`agent_link_web.main:app`・8400 番）は起こさない。
`http.server.ThreadingHTTPServer` を空いている番号（ポート 0）に立てて
決まった JSON を返す偽のサーバにし、`webcli.main([...])` を直接呼んで
終了コード・標準出力・標準エラーを確かめる。

確かめるもの:
    - 終了コードの読み替え（404 → 3、409 → 4、422 → 2、500 → 1）
    - 合言葉は無い（Authorization を付けない）
    - `api` の検査（METHOD・path・--data・扱えない応答）
    - `answer` の検査（問いの数と答えの数の照合）
    - `serve` が正しい引数で uvicorn に替わること

実行:
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_webcli.py
"""

from __future__ import annotations

import contextlib
import io
import json
import os
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

# リポジトリ直下から実行しなくても import できるようにしておく
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from agent_link_web import webcli  # noqa: E402

# 偽のサーバが受け取ったもの（テストの中から読む）
RECEIVED: list[dict] = []

# `question` の応答に入れる問い（`cli_api.py` のテストと同じ形）
QUESTIONS = [
    {
        "question": "資料はどれですか",
        "header": "資料",
        "multiSelect": False,
        "options": [{"label": "A"}, {"label": "B", "description": "B の説明"}],
    },
    {
        "question": "epoch はいくつですか",
        "header": "epoch",
        "multiSelect": True,
        "options": [{"label": "10"}, {"label": "30"}, {"label": "60"}],
    },
]


class Handler(BaseHTTPRequestHandler):
    """決まった JSON を返すだけの偽の Web の窓口。"""

    protocol_version = "HTTP/1.1"

    def log_message(self, *_args) -> None:  # 静かにする
        return

    def _send(self, status: int, body: object) -> None:
        raw = json.dumps(body, ensure_ascii=False).encode("utf-8")
        self._send_raw(status, raw, "application/json; charset=utf-8")

    def _send_raw(self, status: int, raw: bytes, content_type: str) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def _record(self, method: str) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length).decode("utf-8") if length else ""
        try:
            body = json.loads(raw) if raw else None
        except json.JSONDecodeError:
            body = raw
        RECEIVED.append(
            {
                "method": method,
                "path": self.path,
                "body": body,
                "content_type": self.headers.get("Content-Type"),
                "authorization": self.headers.get("Authorization"),
            }
        )

    def do_GET(self) -> None:
        self._record("GET")
        if self.path == "/api/home":
            self._send(
                200,
                {
                    "questions": [
                        {"id": "q1", "agent": "pi", "title": "資料はどれですか",
                         "created_at": 1.0, "project": "研究A", "run_id": 11},
                        {"id": "q2", "agent": "pi", "title": "epoch はいくつですか",
                         "created_at": 2.0, "project": "研究B", "run_id": 22},
                        {"id": "q5", "agent": "pi", "title": "あと少しですか",
                         "created_at": 5.0, "project": "研究C", "run_id": 5},
                    ],
                },
            )
        elif self.path == "/api/questions/q1":
            self._send(200, {"id": "q1", "state": "open", "questions": QUESTIONS})
        elif self.path == "/api/questions/q3":
            # 答えたあと（答えと答えた人が返る）
            self._send(200, {"id": "q3", "state": "answered", "questions": QUESTIONS,
                             "answer": {"answers": [{"text": "緑"}]},
                             "answered_by": "saituki"})
        elif self.path == "/api/projects":
            self._send(200, [{"name": "研究A", "root": "/tmp/a", "kind": "link"}])
        elif self.path == "/api/usage":
            self._send(200, {"cc": {"limits": None, "cost": {"today": 1.5, "month": 3.0, "sessions": 2}}, "saved_at": 1760000000})
        elif self.path == "/api/status/notfound":
            self._send(404, {"detail": "質問が見つかりません"})
        elif self.path == "/api/status/conflict":
            self._send(409, {"detail": "すでに答えています"})
        elif self.path == "/api/status/invalid":
            self._send(422, {"detail": [{"msg": "必須です"}, {"msg": "番号が違います"}]})
        elif self.path == "/api/status/boom":
            self._send(500, {"detail": "内部で失敗しました"})
        elif self.path == "/api/file/report":
            self._send_raw(200, b"%PDF-1.4 fake", "application/pdf")
        elif self.path == "/api/plain/hello":
            self._send_raw(200, "こんにちは\n".encode("utf-8"), "text/plain; charset=utf-8")
        else:
            self._send(404, {"detail": "見つかりません"})

    def do_POST(self) -> None:
        self._record("POST")
        if self.path == "/api/questions/q1/answer":
            self._send(200, {"ok": True})
        elif self.path.endswith("/send"):
            self._send(200, {"ok": True})
        else:
            self._send(404, {"detail": "見つかりません"})


def run_cli(*args: str, env: dict | None = None) -> tuple[int, str, str]:
    """`webcli.main` を 1 回呼び、終了コードと出力を返す。

    環境変数は呼ぶ前に差し替え、終わったら元に戻す（`webcli` は呼ぶたびに
    環境変数を読む）。`env` の値が `None` の項目は外す。
    """
    keys = ("AGENT_LINK_WEB_URL", "AGENT_LINK_WEB_TOKEN", "AGENT_LINK_HOME", "AGENT_LINK_WEB_PORT",
            "AGENT_LINK_PARENT_RUN_ID", "AGENT_LINK_PROJECT_ROOT")
    saved = {key: os.environ.get(key) for key in keys}
    os.environ["AGENT_LINK_WEB_URL"] = BASE["url"]
    os.environ.pop("AGENT_LINK_WEB_TOKEN", None)
    os.environ.pop("AGENT_LINK_PARENT_RUN_ID", None)
    os.environ.pop("AGENT_LINK_PROJECT_ROOT", None)
    os.environ["AGENT_LINK_HOME"] = BASE["home"]
    for key, value in (env or {}).items():
        if value is None:
            os.environ.pop(key, None)
        else:
            os.environ[key] = value
    out = io.StringIO()
    err = io.StringIO()
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = webcli.main(list(args))
    finally:
        for key, value in saved.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value
    return code, out.getvalue(), err.getvalue()


# 偽のサーバと一時の置き場（`setUpClass` で埋める）
BASE: dict[str, str] = {}


class WebCliTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.url = f"http://127.0.0.1:{cls.server.server_address[1]}"
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.tmp = tempfile.mkdtemp(prefix="agent-link-webcli-")
        # 古い版が書いた合言葉のファイルがあっても読まない（`test_no_authorization_header`）
        os.makedirs(os.path.join(cls.tmp, "web"), exist_ok=True)
        with open(os.path.join(cls.tmp, "web", "token"), "w", encoding="utf-8") as fh:
            fh.write("s3cret-token")
        BASE["url"] = cls.url
        BASE["home"] = cls.tmp

    @classmethod
    def tearDownClass(cls) -> None:
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join(timeout=5)

    def setUp(self) -> None:
        RECEIVED.clear()

    # --- --help に命令が並ぶ
    def test_help_lists_commands(self) -> None:
        text = webcli.build_parser().format_help()
        for name in ("serve", "projects", "home", "questions", "question",
                     "answer", "send", "usage", "api"):
            self.assertIn(name, text, f"--help に {name} が出ていません")
        for name in ("feedback", "mlflow"):
            self.assertNotIn(name, text, f"--help に消した命令 {name} が出ています")

    # --- questions: --run で狭められる
    def test_questions_filter_by_run(self) -> None:
        code, out, err = run_cli("questions", "--run", "5")
        self.assertEqual(code, 0, err)
        self.assertEqual([row["id"] for row in json.loads(out)], ["q5"])

    # --- question: 答えと答えた人が出る（自由記述も）
    def test_question_shows_answer_and_answered_by(self) -> None:
        code, out, err = run_cli("question", "q3")
        self.assertEqual(code, 0, err)
        row = json.loads(out)
        self.assertEqual(row["answer"], {"answers": [{"text": "緑"}]})
        self.assertEqual(row["answered_by"], "saituki")
        self.assertEqual(row["state"], "answered")

    # --- 名前の付いた読み取り（projects / usage / mlflow）
    def test_projects_and_usage(self) -> None:
        code, out, err = run_cli("projects")
        self.assertEqual(code, 0, err)
        self.assertEqual(json.loads(out)[0]["name"], "研究A")
        code, out, err = run_cli("usage")
        self.assertEqual(code, 0, err)
        self.assertEqual(json.loads(out)["cc"]["cost"]["today"], 1.5)

    def test_questions_filter_by_project(self) -> None:
        code, out, err = run_cli("questions", "--project", "研究A")
        self.assertEqual(code, 0, err)
        self.assertEqual([row["id"] for row in json.loads(out)], ["q1"])

    # --- 合言葉は無い: Authorization を付けない（古い `web/token` のファイルや環境変数があっても）
    def test_no_authorization_header(self) -> None:
        code, out, err = run_cli("questions", env={"AGENT_LINK_WEB_TOKEN": "old-token"})
        self.assertEqual(code, 0, err)
        self.assertEqual(RECEIVED[-1]["path"], "/api/home", "質問は /api/home の questions から読む")
        self.assertIsNone(RECEIVED[-1]["authorization"], "Authorization を付けてはいけません")
        self.assertTrue(out.strip().startswith("["))

    # --- answer: 問いの数と答えの数が合わなければ送らない
    def test_answer_count_mismatch_does_not_send(self) -> None:
        code, out, err = run_cli("answer", "q1", "--choice", "0")
        self.assertEqual(code, 2, out + err)
        self.assertEqual(out, "")
        self.assertIn("The question has 2 items", err)
        self.assertEqual(
            [r for r in RECEIVED if r["path"].endswith("/answer")], [],
            "数を間違えているのに送ってしまいました",
        )

    def test_answer_sends_answers_in_order(self) -> None:
        code, out, err = run_cli(
            "answer", "q1", "--choice", "1", "--choices", "0,2"
        )
        self.assertEqual(code, 0, err)
        self.assertEqual(json.loads(out), {"ok": True})
        self.assertEqual(RECEIVED[-1]["path"], "/api/questions/q1/answer")
        self.assertEqual(
            RECEIVED[-1]["body"],
            {"answers": [{"choice_index": 1}, {"choice_indices": [0, 2]}]},
        )

    # --- 終了コードの読み替え（404 → 3、409 → 4、422 → 2、500 → 1）
    def _status_case(self, path: str, expected_code: int, expected_text: str) -> None:
        code, out, err = run_cli("api", "GET", path)
        self.assertEqual(code, expected_code, f"{path}: {out}{err}")
        self.assertEqual(out, "", f"{path}: 失敗のとき標準出力に書いてはいけません")
        last = [line for line in err.splitlines() if line.strip()][-1]
        self.assertIn(expected_text, last, f"{path}: 標準エラーの最後の行が違います")

    def test_http_status_mapping(self) -> None:
        self._status_case("/api/status/notfound", 3, "質問が見つかりません")
        self._status_case("/api/status/conflict", 4, "すでに答えています")
        # 422 の detail は一覧。各項目の msg を「・」でつなぐ
        self._status_case("/api/status/invalid", 2, "必須です; 番号が違います")
        self._status_case("/api/status/boom", 1, "内部で失敗しました")

    # --- つながらない
    def test_unreachable(self) -> None:
        code, out, err = run_cli(
            "questions", env={"AGENT_LINK_WEB_URL": "http://127.0.0.1:9"}
        )
        self.assertEqual(code, 1, out + err)
        self.assertEqual(out, "")
        last = [line for line in err.splitlines() if line.strip()][-1]
        self.assertIn("Cannot reach the Web server (http://127.0.0.1:9)", last)

    # --- api
    def test_api_get_returns_json(self) -> None:
        code, out, err = run_cli("api", "GET", "/api/home")
        self.assertEqual(code, 0, err)
        self.assertIn("questions", json.loads(out))

    def test_api_post_sends_body(self) -> None:
        code, out, err = run_cli(
            "api", "POST", "/api/runs/p/1/send", "--data", '{"text": "x"}'
        )
        self.assertEqual(code, 0, err)
        self.assertEqual(RECEIVED[-1]["path"], "/api/runs/p/1/send")
        self.assertEqual(RECEIVED[-1]["body"], {"text": "x"})
        self.assertEqual(json.loads(out), {"ok": True})

    def test_api_rejects_foreign_url(self) -> None:
        code, out, _err = run_cli("api", "GET", "http://evil/api")
        self.assertEqual(code, 2)
        self.assertEqual(out, "")

    def test_api_rejects_space_in_path(self) -> None:
        code, out, err = run_cli("api", "GET", "/api/a b")
        self.assertEqual(code, 2)
        self.assertEqual(out, "")
        self.assertNotIn("Traceback", err)

    def test_api_rejects_bad_method(self) -> None:
        code, out, _err = run_cli("api", "FOO", "/api/home")
        self.assertEqual(code, 2)
        self.assertEqual(out, "")

    def test_api_rejects_bad_data(self) -> None:
        code, out, _err = run_cli("api", "POST", "/api/home", "--data", "{")
        self.assertEqual(code, 2)
        self.assertEqual(out, "")

    def test_api_unhandled_response_is_error(self) -> None:
        code, out, err = run_cli("api", "GET", "/api/file/report")
        self.assertEqual(code, 1)
        self.assertEqual(out, "")
        last = [line for line in err.splitlines() if line.strip()][-1]
        self.assertIn("The CLI cannot handle this API response (application/pdf).", last)

    def test_api_text_response_is_printed(self) -> None:
        code, out, err = run_cli("api", "GET", "/api/plain/hello")
        self.assertEqual(code, 0, err)
        self.assertEqual(out, "こんにちは\n")

    # --- serve: `os.execv` を差し替えて引数を確かめる
    def test_serve_passes_uvicorn_arguments(self) -> None:
        calls: list[tuple] = []
        original = os.execv
        os.execv = lambda *a, **k: calls.append(a)  # type: ignore[assignment]
        try:
            code, out, err = run_cli("serve", "--port", "8299")
        finally:
            os.execv = original
        self.assertEqual(code, 0, out + err)
        self.assertEqual(
            calls,
            [(
                sys.executable,
                [sys.executable, "-m", "uvicorn", "agent_link_web.main:app",
                 "--host", "127.0.0.1", "--port", "8299"],
            )],
        )

    def test_serve_reads_port_from_env(self) -> None:
        calls: list[tuple] = []
        original = os.execv
        os.execv = lambda *a, **k: calls.append(a)  # type: ignore[assignment]
        try:
            code, out, err = run_cli("serve", env={"AGENT_LINK_WEB_PORT": "8288"})
        finally:
            os.execv = original
        self.assertEqual(code, 0, out + err)
        argv = calls[0][1]
        self.assertEqual(argv[argv.index("--port") + 1], "8288")
        self.assertEqual(argv[argv.index("--host") + 1], "127.0.0.1")

    # --- 消した命令（feedback・mlflow・user）は parser に無い
    def test_removed_commands_are_unknown(self) -> None:
        for name in ("feedback", "mlflow", "user"):
            err = io.StringIO()
            with contextlib.redirect_stderr(err), self.assertRaises(SystemExit) as cm:
                webcli.main([name])
            self.assertEqual(cm.exception.code, 2, name)


class DefaultPortTest(unittest.TestCase):
    """Web の既定ポートは 8400。sai-moon の Web（8200）と分けるため、全部の置き場でそろえる。"""

    def test_default_port_is_same_everywhere(self) -> None:
        repo = Path(__file__).resolve().parents[3]
        want = {
            "apps/web/src/agent_link_web/webcli.py": "registry.config_var('AGENT_LINK_WEB_PORT') or '8400'",
            "apps/web/hooks/cc_question.py": 'os.environ.get("AGENT_LINK_WEB_PORT") or "8400"',
            "apps/web/hooks/cc_register.py": 'os.environ.get("AGENT_LINK_WEB_PORT") or "8400"',
            "crates/agent-link-engine/src/cc_command.rs": 'pub const DEFAULT_WEB_PORT: &str = "8400";',
            "README.md": "| `AGENT_LINK_WEB_PORT` | `8400` |",
        }
        for rel, text in want.items():
            body = (repo / rel).read_text(encoding="utf-8")
            self.assertIn(text, body, rel)
            self.assertNotIn("8200", body.replace("8200 番（Tailscale 4444）", "").replace("8200〜8202", ""), rel)
        # 利用者の settings.json（config の AGENT_LINK_WEB_PORT）に左右されないよう、置き場を一時フォルダにする
        with tempfile.TemporaryDirectory() as home, patch_env(AGENT_LINK_HOME=home):
            with patch_env(AGENT_LINK_WEB_URL=None, AGENT_LINK_WEB_PORT=None):
                self.assertEqual(webcli.base_url(), "http://127.0.0.1:8400")
            with patch_env(AGENT_LINK_WEB_URL=None, AGENT_LINK_WEB_PORT="8297"):
                self.assertEqual(webcli.base_url(), "http://127.0.0.1:8297")


@contextlib.contextmanager
def patch_env(**values):
    saved = {k: os.environ.get(k) for k in values}
    try:
        for k, v in values.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        yield
    finally:
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


if __name__ == "__main__":
    unittest.main()
