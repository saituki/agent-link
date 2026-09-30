"""`phone_api.py`（sai-phone の HTTP を呼ぶ道具）のテスト。

本物の sai-phone（8100 番や 8200 番）は使わない。
`http.server` で空いている番号に偽のサーバを立てて、
返り値・送る本文・合言葉の取り扱い・つながなかったときの終了コードを確かめる。

    cd sai-sci/tools && python3 -m unittest test_phone_api -v      # Python 3.11 以上
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
SCRIPT = TOOLS / "phone_api.py"

# 偽のサーバが受け取ったもの（テスト从中から読む）
RECEIVED: list[dict] = []

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
    """`/api/home`・質問の API・MLflow の中継を、それらしい返り値で应由だけの偽物。"""

    protocol_version = "HTTP/1.1"

    def log_message(self, *_args) -> None:  # 静かにする
        return

    def _send(self, status: int, body: dict) -> None:
        raw = json.dumps(body, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self) -> None:
        self._record("GET")
        if self.path == "/api/home":
            self._send(200, {"questions": [
                {"id": "q1", "agent": "pi", "title": "資料はどれですか",
                 "created_at": 1.0, "project": "研究A", "run_id": 11},
                {"id": "q2", "agent": "pi", "title": "epoch はいくつですか",
                 "created_at": 2.0, "project": "研究B", "run_id": 22},
            ], "approvals": [
                {"id": "a1", "tool_name": "Bash", "summary": "ls",
                 "created_at": 3.0, "project": "研究A", "run_id": 11},
                {"id": "a2", "tool_name": "Bash", "summary": "pwd",
                 "created_at": 4.0, "project": "研究B", "run_id": 22},
            ]})
        elif self.path == "/api/questions/q1":
            self._send(200, {"id": "q1", "state": "open", "questions": QUESTIONS})
        elif self.path == "/api/questions/gone":
            self._send(404, {"detail": "質問が見つかりません"})
        elif self.path.startswith("/api/mlflow/experiments"):
            self._send(200, {"experiments": [{"experiment_id": "1"}]})
        elif self.path.startswith("/api/mlflow/runs/"):
            self._send(200, {"run": {"info": {"run_id": self.path.rsplit("/", 1)[-1]}}})
        elif self.path.startswith("/api/mlflow/metrics/history"):
            self._send(200, {"metrics": [{"metric_key": "loss"}]})
        else:
            self._send(404, {"detail": "見つかりません"})

    def do_POST(self) -> None:
        self._record("POST")
        if self.path == "/api/questions/q1/answer":
            self._send(200, {"ok": True})
        elif self.path == "/api/questions/q9/answer":
            self._send(400, {"detail": "質問が期限切れです"})
        elif self.path.startswith("/api/approvals/") and self.path.endswith("/answer"):
            self._send(200, {"ok": True})
        elif self.path.endswith("/send"):
            self._send(200, {"ok": True})
        elif self.path == "/api/mlflow/runs/search":
            self._send(200, {"runs": []})
        else:
            self._send(404, {"detail": "見つかりません"})

    def _record(self, method: str) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length).decode("utf-8") if length else ""
        try:
            body = json.loads(raw) if raw else None
        except json.JSONDecodeError:
            body = raw
        RECEIVED.append({
            "method": method,
            "path": self.path,
            "body": body,
            # 合言葉は実際にはヘッダで来る（URL には入らない）
            "authorization": self.headers.get("Authorization"),
        })


def run_tool(*args: str, env: dict | None = None) -> tuple[int, str, str]:
    """`phone_api.py` を 1 回呼び、終了コードと出力（標準出力・標準エラー）を返す。"""
    environ = dict(os.environ)
    environ["SAI_PHONE_URL"] = environ.get("TEST_PHONE_URL", "http://127.0.0.1:8200")
    environ.pop("SAI_PHONE_TOKEN", None)
    environ.update(env or {})
    proc = subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        timeout=60,
        env=environ,
    )
    return proc.returncode, proc.stdout, proc.stderr


class PhoneApiTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.url = f"http://127.0.0.1:{cls.server.server_address[1]}"
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.tmp = tempfile.mkdtemp(prefix="sai-phone-api-")
        # 合言葉のファイルを用意する（`apps/phone` が書くのと同じ置き場）
        os.makedirs(os.path.join(cls.tmp, "phone"), exist_ok=True)
        with open(os.path.join(cls.tmp, "phone", "token"), "w", encoding="utf-8") as fh:
            fh.write("s3cret-token")
        os.environ["TEST_PHONE_URL"] = cls.url

    @classmethod
    def tearDownClass(cls) -> None:
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join(timeout=5)

    def setUp(self) -> None:
        RECEIVED.clear()
        self.env = {"SAI_SAI_HOME": self.tmp}

    # --- 質問の一覧: --run で狭められる
    def test_questions_all(self) -> None:
        code, out, _err = run_tool("questions", env=self.env)
        self.assertEqual(code, 0)
        ids = [row["id"] for row in json.loads(out)]
        self.assertEqual(ids, ["q1", "q2"])

    def test_questions_filter_by_run(self) -> None:
        code, out, _err = run_tool("questions", "--run", "22", env=self.env)
        self.assertEqual(code, 0)
        rows = json.loads(out)
        self.assertEqual([row["id"] for row in rows], ["q2"])
        self.assertEqual(rows[0]["project"], "研究B")

    def test_questions_filter_by_project(self) -> None:
        code, out, _err = run_tool("questions", "--project", "研究A", env=self.env)
        self.assertEqual(code, 0)
        self.assertEqual([row["id"] for row in json.loads(out)], ["q1"])

    # --- 質問 1 件: 選択肢に `--choice` に使う番号が付く
    def test_question_has_numbered_options(self) -> None:
        code, out, _err = run_tool("question", "q1", env=self.env)
        self.assertEqual(code, 0)
        data = json.loads(out)
        self.assertEqual(len(data["questions"]), 2)
        first = data["questions"][0]
        self.assertEqual(first["index"], 0)
        self.assertEqual([opt["index"] for opt in first["options"]], [0, 1])
        self.assertEqual(first["options"][1]["label"], "B")
        self.assertEqual(first["options"][1]["description"], "B の説明")

    # --- 答え: 問いの順に並べた本文が送られる
    def test_answer_sends_answers_in_order(self) -> None:
        code, out, err = run_tool(
            "answer", "q1", "--choice", "1", "--choices", "0,2", env=self.env
        )
        self.assertEqual(code, 0, err)
        self.assertEqual(json.loads(out), {"ok": True})
        self.assertEqual(RECEIVED[-1]["path"], "/api/questions/q1/answer")
        self.assertEqual(
            RECEIVED[-1]["body"],
            {"answers": [{"choice_index": 1}, {"choice_indices": [0, 2]}]},
        )

    def test_answer_with_text(self) -> None:
        # 2 個の問いに、1 つずつ並べる（選択肢と自由文が混ざってよい）
        code, _out, err = run_tool(
            "answer", "q1", "--choice", "0", "--text", "30 epoch で止める", env=self.env
        )
        self.assertEqual(code, 0, err)
        self.assertEqual(
            RECEIVED[-1]["body"],
            {"answers": [{"choice_index": 0}, {"text": "30 epoch で止める"}]},
        )

    def test_answer_count_mismatch_does_not_send(self) -> None:
        # 問いは 2 個だが答えが 1 個なら送らない（送信が 0 件のままであること）
        code, out, _err = run_tool("answer", "q1", "--choice", "0", env=self.env)
        self.assertEqual(code, 1)
        self.assertIn("問いは 2 個", json.loads(out)["error"])
        self.assertEqual([r for r in RECEIVED if r["path"].endswith("/answer")], [])

    def test_answer_server_error(self) -> None:
        # 存在しない質問（問いの数も読めない）に答えると、サーバの 404 をそのまま出す
        code, out, _err = run_tool("answer", "q9", "--text", "x", env=self.env)
        self.assertEqual(code, 1)
        self.assertEqual(json.loads(out)["error"], "404 見つかりません")

    # --- 承認
    def test_approvals_filter_by_run(self) -> None:
        code, out, _err = run_tool("approvals", "--run", "11", env=self.env)
        self.assertEqual(code, 0)
        self.assertEqual([row["id"] for row in json.loads(out)], ["a1"])

    def test_approve(self) -> None:
        code, _out, err = run_tool("approve", "a1", "allow", "--reason", "故意的", env=self.env)
        self.assertEqual(code, 0, err)
        self.assertEqual(RECEIVED[-1]["path"], "/api/approvals/a1/answer")
        self.assertEqual(RECEIVED[-1]["body"], {"decision": "allow", "reason": "故意的"})

    def test_approve_bad_decision(self) -> None:
        code, _out, _err = run_tool("approve", "a1", "maybe", env=self.env)
        self.assertEqual(code, 2)

    # --- 送信
    def test_send(self) -> None:
        code, _out, err = run_tool("send", "研究A", "11", "続けて", env=self.env)
        self.assertEqual(code, 0, err)
        self.assertEqual(RECEIVED[-1]["path"], "/api/runs/%E7%A0%94%E7%A9%B6A/11/send")
        self.assertEqual(RECEIVED[-1]["body"], {"text": "続けて"})

    # --- MLflow（読み取り）
    def test_mlflow_experiments(self) -> None:
        code, out, _err = run_tool("mlflow", "experiments", env=self.env)
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(out)["experiments"][0]["experiment_id"], "1")

    def test_mlflow_runs_search(self) -> None:
        code, _out, err = run_tool(
            "mlflow", "runs", "1", "--filter", "metrics.loss < 1", "--max", "5", env=self.env
        )
        self.assertEqual(code, 0, err)
        self.assertEqual(RECEIVED[-1]["path"], "/api/mlflow/runs/search")
        self.assertEqual(
            RECEIVED[-1]["body"],
            {"experiment_id": "1", "filter": "metrics.loss < 1", "max_results": 5},
        )

    def test_mlflow_run(self) -> None:
        code, out, _err = run_tool("mlflow", "run", "abc", env=self.env)
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(out)["run"]["info"]["run_id"], "abc")

    def test_mlflow_metric(self) -> None:
        code, _out, err = run_tool("mlflow", "metric", "abc", "loss", "--max", "3", env=self.env)
        self.assertEqual(code, 0, err)
        self.assertIn("run_id=abc", RECEIVED[-1]["path"])
        self.assertIn("metric_key=loss", RECEIVED[-1]["path"])
        self.assertIn("max_results=3", RECEIVED[-1]["path"])

    # --- 合言葉: ヘッダに入り、URL にも出力にも出ない
    def test_token_from_file_goes_to_header(self) -> None:
        code, out, err = run_tool("questions", env=self.env)
        self.assertEqual(code, 0, err)
        self.assertEqual(RECEIVED[-1]["authorization"], "Bearer s3cret-token")
        self.assertNotIn("s3cret-token", RECEIVED[-1]["path"], "合言葉が URL に入っています")
        self.assertNotIn("s3cret-token", out, "合言葉が出力に漏れています")
        self.assertNotIn("s3cret-token", err, "合言葉が標準エラーに漏れています")

    def test_token_env_wins(self) -> None:
        code, _out, err = run_tool(
            "questions", env={**self.env, "SAI_PHONE_TOKEN": "env-token"}
        )
        self.assertEqual(code, 0, err)
        self.assertEqual(RECEIVED[-1]["authorization"], "Bearer env-token")

    def test_no_token_when_nothing_is_set(self) -> None:
        empty = tempfile.mkdtemp(prefix="sai-phone-api-empty-")
        code, out, err = run_tool("questions", env={"SAI_SAI_HOME": empty})
        self.assertEqual(code, 0, err)
        self.assertIsNone(RECEIVED[-1]["authorization"], "合言葉が無いのにヘッダが付いています")
        self.assertTrue(out.strip().startswith("["), out)

    def test_token_with_unusable_characters(self) -> None:
        # HTTP のヘッダは latin-1 でしか送れない。日本語の合言葉でも
        # 記録を残さず、理由だけを返す（値そのものは出さない）
        code, out, err = run_tool(
            "questions", env={**self.env, "SAI_PHONE_TOKEN": "日本語の合言葉"}
        )
        self.assertEqual(code, 1)
        self.assertIn("ヘッダで送れない", json.loads(out)["error"])
        self.assertNotIn("日本語の合言葉", out + err, "合言葉が出力に漏れています")

    def test_server_error_is_reported(self) -> None:
        # サーバのエラー（404）は、状態コードと detail をそのまま JSON にする
        code, out, _err = run_tool("question", "gone", env=self.env)
        self.assertEqual(code, 1)
        self.assertEqual(json.loads(out)["error"], "404 質問が見つかりません")

    # --- つながなかったとき: 終了コード 2 と、聞き方の分かる文言
    def test_unreachable(self) -> None:
        # 誰も聞かれていないポート（偽のサーバとは別の空き番号）
        code, out, err = run_tool(
            "questions", env={**self.env, "SAI_PHONE_URL": "http://127.0.0.1:9"}
        )
        self.assertEqual(code, 2, out + err)
        data = json.loads(out)
        self.assertIn("つながりません", data["error"])
        self.assertIn("http://127.0.0.1:9", data["error"])
        self.assertNotIn("s3cret-token", out + err, "合言葉が出力に漏れています")


if __name__ == "__main__":
    unittest.main()
