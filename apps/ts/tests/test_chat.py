"""チャット（AC1・AC2）: 偽の sai-link で start・send・messages と、終了コードの読み替え。"""

import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from fastapi.testclient import TestClient  # noqa: E402

from sai_ts import chat, demo, linkcli, server  # noqa: E402

FAKE = str(Path(__file__).resolve().parent / "fake_sai_link.sh")


class TempProject:
    """一時フォルダをプロジェクトにし、SAI_LINK_BIN を偽の sai-link に向ける。"""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        (self.tmp / ".sai").mkdir()
        self.fake = self.tmp / "fake"
        self._saved = {k: os.environ.pop(k, None) for k in
                       ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN", "SAI_LINK_BIN", "FAKE_LINK_DIR")}
        os.environ.update(SAI_PROJECT_ROOT=str(self.tmp), SAI_LINK_BIN=FAKE, FAKE_LINK_DIR=str(self.fake))

    def tearDown(self):
        for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN", "SAI_LINK_BIN", "FAKE_LINK_DIR"):
            os.environ.pop(k, None)
        for k, v in self._saved.items():
            if v is not None:
                os.environ[k] = v
        shutil.rmtree(self.tmp, ignore_errors=True)

    def set_out(self, key, obj=None, rc=None, err=None):
        out = self.fake / "out"
        out.mkdir(parents=True, exist_ok=True)
        if obj is not None:
            (out / f"{key}.json").write_text(json.dumps(obj, ensure_ascii=False), encoding="utf-8")
        if rc is not None:
            (out / f"{key}.rc").write_text(str(rc), encoding="utf-8")  # 1 回だけ使われる
        if err is not None:
            (out / f"{key}.err").write_text(err + "\n", encoding="utf-8")

    def calls(self):
        d = self.fake / "calls"
        if not d.is_dir():
            return []
        return [(f.read_text(encoding="utf-8").splitlines(),
                 (d / (f.name + ".stdin")).read_text(encoding="utf-8"))
                for f in sorted(d.iterdir()) if f.suffix != ".stdin"]

    def clear_calls(self):
        shutil.rmtree(self.fake / "calls", ignore_errors=True)


class ChatTest(TempProject, unittest.TestCase):
    def setUp(self):
        super().setUp()
        (self.tmp / "ts" / "datasets").mkdir(parents=True)
        demo.make(self.tmp / "ts" / "datasets" / "demo", records=2, nvars=2, seconds=2.0, srate=100.0)

    def test_start_creates_card_and_runs(self):
        self.set_out("card_create", {"run_id": 7})
        self.assertEqual(chat.start("demo"), {"run_id": 7, "started": True})
        calls = self.calls()
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0][0], ["card", "create", "--json"])
        card = json.loads(calls[0][1])
        self.assertEqual((card["runner"], card["block"]), ("cc", "ts-agent"))
        self.assertIn("demo", card["description"])
        self.assertEqual(calls[1][0], ["card", "run", "7", "--json"])
        saved = json.loads((self.tmp / "ts/datasets/demo/chat.json").read_text(encoding="utf-8"))
        self.assertEqual(saved["run_id"], 7)

    def test_start_while_running_calls_only_list(self):
        (self.tmp / "ts/datasets/demo/chat.json").write_text('{"run_id": 7}', encoding="utf-8")
        self.set_out("run_list", {"runs": [{"run_id": 7, "status": "running", "block": "ts-agent"}]})
        self.assertEqual(chat.start("demo"), {"run_id": 7, "started": False})
        calls = self.calls()
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0][0][:2], ["run", "list"])

    def test_start_after_finished_creates_new_card(self):
        (self.tmp / "ts/datasets/demo/chat.json").write_text('{"run_id": 7}', encoding="utf-8")
        self.set_out("run_list", {"runs": [{"run_id": 7, "status": "done"}]})
        self.set_out("card_create", {"run_id": 8})
        self.assertEqual(chat.start("demo"), {"run_id": 8, "started": True})

    def test_send_passes_text_on_stdin(self):
        (self.tmp / "ts/datasets/demo/chat.json").write_text('{"run_id": 8}', encoding="utf-8")
        chat.send("demo", "この区間を見て")
        calls = self.calls()
        self.assertEqual(calls[0][0], ["run", "send", "8", "--json"])
        self.assertEqual(calls[0][1], "この区間を見て")

    def test_send_limits_and_missing_chat(self):
        (self.tmp / "ts/datasets/demo/chat.json").write_text('{"run_id": 8}', encoding="utf-8")
        for bad in ("", "x" * 20001):
            with self.assertRaises(ValueError):
                chat.send("demo", bad)
        (self.tmp / "ts/datasets/demo/chat.json").unlink()
        with self.assertRaises(linkcli.LinkError) as cm:
            chat.send("demo", "こんにちは")
        self.assertEqual(cm.exception.status, 409)

    def test_exit_codes_map_to_http_status(self):
        client = TestClient(server.app)
        for rc, status in ((2, 400), (3, 404), (4, 409)):
            with self.subTest(rc=rc):
                self.set_out("card_create", rc=rc, err="理由 %d" % rc)
                r = client.post("/api/datasets/demo/chat/start")
                self.assertEqual(r.status_code, status)
                self.assertEqual(r.json()["detail"], "理由 %d" % rc)

    def test_api_send_errors(self):
        client = TestClient(server.app)
        r = client.post("/api/datasets/demo/chat/send", json={"text": "やあ"})
        self.assertEqual(r.status_code, 409)
        (self.tmp / "ts/datasets/demo/chat.json").write_text('{"run_id": 8}', encoding="utf-8")
        r = client.post("/api/datasets/demo/chat/send", json={"text": ""})
        self.assertEqual(r.status_code, 400)

    def test_messages_split_and_status(self):
        (self.tmp / "ts/datasets/demo/chat.json").write_text('{"run_id": 8}', encoding="utf-8")
        self.set_out("run_list", {"runs": [{"run_id": 8, "status": "running"}]})
        text = ("# task_text\nあなたは sai-ts のチャット相手である。\n\n"
                "# user (1)\n区間を見て\n\n"
                "# assistant (1)\n見ました。\n\n## 補足\n1 段下げた見出し\n\n"
                "# user (2)\nありがとう\n")
        self.set_out("run_md", {"kind": "message", "name": "message.md", "text": text})
        got = chat.messages("demo")
        self.assertEqual(got["run_id"], 8)
        self.assertEqual(got["status"], "running")
        self.assertEqual(got["messages"], [
            {"role": "user", "n": 1, "text": "区間を見て"},
            {"role": "assistant", "n": 1, "text": "見ました。\n\n## 補足\n1 段下げた見出し"},
            {"role": "user", "n": 2, "text": "ありがとう"},
        ])

    def test_messages_not_yet_is_empty(self):
        (self.tmp / "ts/datasets/demo/chat.json").write_text('{"run_id": 8}', encoding="utf-8")
        self.set_out("run_md", rc=3, err="まだありません")
        self.assertEqual(chat.messages("demo")["messages"], [])

    def test_messages_without_chat(self):
        r = TestClient(server.app).get("/api/datasets/demo/chat/messages")
        self.assertEqual(r.json(), {"run_id": None, "status": None, "messages": []})


class SplitMessagesTest(unittest.TestCase):
    def test_drops_text_before_first_delimiter(self):
        self.assertEqual(chat.split_messages("# task_text\n前置き\n"), [])

    def test_indented_heading_is_not_a_delimiter(self):
        got = chat.split_messages("# assistant (1)\n本文\n# user (2)\n続き\n")
        self.assertEqual([m["n"] for m in got], [1, 2])
        self.assertEqual(got[0]["text"], "本文")


if __name__ == "__main__":
    unittest.main()
