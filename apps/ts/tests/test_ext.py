"""拡張（AC3）: 版・配信の検査・server.py の呼び出し（成功・時間切れ 504・失敗 502）。"""

import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from fastapi.testclient import TestClient  # noqa: E402

from sai_ts import ext, server  # noqa: E402

SERVER_PY = '''import json, sys, time
fn = sys.argv[1]
req = json.load(sys.stdin)
if fn == "echo":
    print(json.dumps({"echo": req}))
elif fn == "slow":
    time.sleep(5)
    print("{}")
elif fn == "fail":
    print("途中の出力", file=sys.stderr)
    print("壊れた", file=sys.stderr)
    sys.exit(3)
'''


class ExtTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        (self.tmp / ".sai").mkdir()
        self._saved = {k: os.environ.pop(k, None) for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN")}
        os.environ["SAI_PROJECT_ROOT"] = str(self.tmp)
        d = self.tmp / "ts" / "ext" / "hello"
        d.mkdir(parents=True)
        (d / "manifest.json").write_text(json.dumps(
            {"name": "hello", "title": "挨拶", "entry": "main.js", "server": "server.py", "enabled": True}),
            encoding="utf-8")
        (d / "main.js").write_text("export default function activate(ts) {}\n", encoding="utf-8")
        (d / "server.py").write_text(SERVER_PY, encoding="utf-8")
        p = self.tmp / "ts" / "ext" / "plain"
        p.mkdir()
        (p / "manifest.json").write_text(json.dumps({"name": "plain", "entry": "main.js"}), encoding="utf-8")
        (p / "main.js").write_text("", encoding="utf-8")

    def tearDown(self):
        for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN"):
            os.environ.pop(k, None)
        for k, v in self._saved.items():
            if v is not None:
                os.environ[k] = v
        shutil.rmtree(self.tmp, ignore_errors=True)

    def by_name(self, name):
        return next(e for e in ext.list_exts() if e["name"] == name)

    def test_version_changes_with_content(self):
        before = self.by_name("hello")
        self.assertEqual((before["title"], before["entry"], before["enabled"], before["error"]),
                         ("挨拶", "main.js", True, ""))
        (self.tmp / "ts/ext/hello/main.js").write_text("// 変えた\n", encoding="utf-8")
        self.assertNotEqual(self.by_name("hello")["version"], before["version"])

    def test_bad_manifest_reports_error(self):
        d = self.tmp / "ts/ext/wrong"
        d.mkdir()
        (d / "manifest.json").write_text(json.dumps({"name": "other", "entry": "main.js"}), encoding="utf-8")
        (d / "main.js").write_text("", encoding="utf-8")
        self.assertIn("name", self.by_name("wrong")["error"])
        self.assertEqual(self.by_name("hello")["error"], "")

    def test_asset_refuses_outside_and_wrong_types(self):
        with self.assertRaises(ValueError):
            ext.asset("hello", "../plain/main.js")
        with self.assertRaises(ValueError):
            ext.asset("hello", "server.py")
        with self.assertRaises(KeyError):
            ext.asset("hello", "nope.js")
        p, media = ext.asset("hello", "main.js")
        self.assertEqual(p.name, "main.js")
        self.assertEqual(media, "text/javascript; charset=utf-8")

    def test_http_serves_js_with_no_store(self):
        client = TestClient(server.app)
        r = client.get("/ext/hello/main.js")
        self.assertEqual(r.status_code, 200)
        self.assertTrue(r.headers["content-type"].startswith("text/javascript"))
        self.assertEqual(r.headers["cache-control"], "no-store")
        self.assertEqual(client.get("/ext/hello/server.py").status_code, 400)
        exts = client.get("/api/ext").json()["exts"]
        self.assertEqual([e["name"] for e in exts], ["hello", "plain"])

    def test_call_returns_json(self):
        self.assertEqual(ext.call("hello", "echo", {"a": 1}), {"echo": {"a": 1}})
        r = TestClient(server.app).post("/api/ext/hello/call/echo", json={"a": 2})
        self.assertEqual(r.json(), {"echo": {"a": 2}})

    def test_call_timeout_is_504(self):
        with self.assertRaises(ext.ExtError) as cm:
            ext.call("hello", "slow", {}, timeout=1)
        self.assertEqual(cm.exception.status, 504)

    def test_call_failure_is_502_with_last_stderr_line(self):
        with self.assertRaises(ext.ExtError) as cm:
            ext.call("hello", "fail", {})
        self.assertEqual((cm.exception.status, cm.exception.detail), (502, "壊れた"))
        r = TestClient(server.app).post("/api/ext/hello/call/fail", json={})
        self.assertEqual(r.status_code, 502)

    def test_call_without_server_is_404(self):
        with self.assertRaises(KeyError):
            ext.call("plain", "echo", {})


if __name__ == "__main__":
    unittest.main()
