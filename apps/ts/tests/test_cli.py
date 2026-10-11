"""CLI sai-ts（AC1〜AC4）。bin/sai-ts を subprocess で呼ぶ。一時フォルダをプロジェクトにし、sai-link は偽の sh。"""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

APP = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(APP / "src"))

BIN = str(APP / "bin" / "sai-ts")
FAKE = str(Path(__file__).resolve().parent / "fake_sai_link.sh")
JA = re.compile(r"[぀-ヿ一-鿿]")

# sdk.py の docstring にある例と同じ（agent が読む物と動きを揃える）
BURST = '''import json, sys
import numpy as np
from sai_ts import sdk
req = json.load(sys.stdin)
ds = req["dataset"]
found = []
for rec in req["records"] or sdk.records(ds):
    meta, data = sdk.load(ds, rec)
    x = data["ch01"]; sr = meta["srate"]
    idx = np.flatnonzero(np.abs(x) > req["params"].get("thr", 3.0) * np.std(x))
    for i in idx[::int(sr)]:   # 1 秒に 1 件まで
        found.append({"type": "line", "record": rec, "vars": ["ch01"], "t": float(i / sr), "category": "burst", "text": "", "metrics": {}})
sdk.add_events(ds, found)
print(json.dumps({"added": len(found)}))
'''

SCHEMA = {"categories": [{"name": "burst", "color": "#d9534f", "visible": True, "desc": ""}],
          "metrics": [{"name": "amp", "type": "number", "min": 0, "max": 100, "unit": "", "desc": ""}]}

# 命令の一覧（C12）。README と --help の確認に使う
COMMANDS = [
    ["serve"], ["init"], ["demo"], ["datasets"], ["meta"],
    ["events", "list"], ["events", "add"], ["events", "update"], ["events", "rm"], ["events", "export"],
    ["schema", "get"], ["schema", "set"], ["channels", "get"], ["channels", "set"],
    ["view"], ["view", "set"], ["run"], ["algos", "list"], ["ext", "list"],
]


SPEC_ECHO = {"version": 1, "title": "帯域", "kind": "channel", "target": "vars", "desc": "",
             "params": [{"name": "low_hz", "type": "number", "default": 80, "min": 0},
                        {"name": "envelope", "type": "boolean", "default": True}]}
ECHO = '''import json, sys
req = json.load(sys.stdin)
print(json.dumps({"message": "ok", "params": req["params"], "target": req["target"]}))
'''


class CliTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        (self.tmp / ".sai").mkdir()
        # SAI_ で始まる変数は外す（親の環境から来た設定で本物を触らないため）
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("SAI_")}
        self.env["SAI_PROJECT_ROOT"] = str(self.tmp)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def sai(self, *args, stdin="", env=None, cwd=None):
        return subprocess.run([BIN, *args], input=stdin, capture_output=True, text=True,
                              env=self.env if env is None else env, cwd=cwd or self.tmp, timeout=120)

    def ok_json(self, *args, stdin="", env=None):
        p = self.sai(*args, "--json", stdin=stdin, env=env)
        self.assertEqual(p.returncode, 0, p.stderr)
        return json.loads(p.stdout)

    def make_demo(self):
        p = self.sai("demo", "demo", "--records", "2", "--vars", "2", "--seconds", "2", "--srate", "100")
        self.assertEqual(p.returncode, 0, p.stderr)

    def test_events_flow(self):  # AC1
        self.make_demo()
        self.assertEqual([d["name"] for d in self.ok_json("datasets")["datasets"]], ["demo"])
        self.sai("schema", "set", "demo", stdin=json.dumps(SCHEMA))

        one = {"type": "line", "record": "r01", "vars": ["ch01"], "t": 0.25, "category": "burst", "text": "", "metrics": {}}
        res = self.ok_json("events", "add", "demo", stdin=json.dumps(one))
        self.assertEqual((res["count"], res["events"][0]["source"]), (1, "agent"))
        first_id = res["events"][0]["id"]

        many = [{"type": "area", "record": "r01", "vars": [], "t0": 0.5, "t1": 0.9, "category": "burst", "text": "広い", "metrics": {"amp": 3}},
                {"type": "line", "record": "r02", "vars": ["ch02"], "t": 1.0, "category": "burst", "text": "", "metrics": {}}]
        self.assertEqual(self.ok_json("events", "add", "demo", stdin=json.dumps(many))["count"], 2)

        listed = self.ok_json("events", "list", "demo")
        self.assertEqual(listed["count"], 3)
        self.assertTrue(all(e["source"] == "agent" for e in listed["events"]))

        upd = self.ok_json("events", "update", "demo", first_id, stdin='{"text": "直した", "metrics": {}}')
        self.assertEqual((upd["rev"], upd["text"], upd["source"]), (2, "直した", "agent"))

        self.assertEqual(self.ok_json("events", "rm", "demo", first_id), {"id": first_id, "deleted": True})
        self.assertEqual(self.ok_json("events", "list", "demo")["count"], 2)

        p = self.sai("events", "export", "demo")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertTrue(p.stdout.startswith("onset\tduration\ttrial_type\tchannel\trecord"))
        self.assertEqual(self.ok_json("events", "export", "demo")["tsv"], p.stdout)

    def test_failure_contract(self):  # AC2
        self.make_demo()
        bare = Path(tempfile.mkdtemp()).resolve()  # .sai の無い場所（上へたどっても見つからない）
        no_project = {k: v for k, v in self.env.items() if k != "SAI_PROJECT_ROOT"}
        bad_event = '{"type": "line", "record": "r01", "vars": [], "t": 0.5, "category": "nope", "text": "", "metrics": {}}'
        cases = [
            ("無いデータセット", ["events", "list", "nosuch"], {}, "", 3),
            ("検査の誤り", ["events", "add", "demo"], {}, bad_event, 2),
            ("名前の形", ["events", "list", "../x"], {}, "", 2),
            ("demo の二重作成", ["demo", "demo"], {}, "", 4),
            ("プロジェクトが無い", ["datasets"], {"env": no_project, "cwd": bare}, "", 2),
            ("引数の誤り", ["events", "list", "demo", "--type", "foo"], {}, "", 2),
        ]
        try:
            for name, args, opts, stdin, code in cases:
                with self.subTest(name):
                    p = self.sai(*args, stdin=stdin, **opts)
                    self.assertEqual(p.returncode, code, p.stderr)
                    self.assertEqual(p.stdout, "")
                    last = p.stderr.strip().splitlines()[-1]
                    self.assertRegex(last, JA)
                    if name == "引数の誤り":
                        self.assertTrue(last.startswith("引数の誤り:"), last)
        finally:
            shutil.rmtree(bare, ignore_errors=True)

    def test_view_and_run(self):  # AC3
        self.make_demo()
        self.sai("schema", "set", "demo", stdin=json.dumps(SCHEMA))
        (self.tmp / "ts" / "scripts").mkdir(exist_ok=True)
        (self.tmp / "ts" / "scripts" / "burst.py").write_text(BURST, encoding="utf-8")

        self.ok_json("view", "set", stdin='{"dataset": "demo", "record": "r01", "t0": 0.0, "t1": 1.0, "vars": ["ch01"]}')
        self.assertEqual(self.ok_json("view")["updated_by"], "agent")

        res = self.ok_json("run", "burst", "--dataset", "demo", "--params", '{"thr": 1.0}')
        self.assertEqual(set(res), {"script", "source", "result", "elapsed_s", "run"})
        self.assertEqual(res["script"], "burst")
        self.assertTrue(res["source"].startswith("script:burst@"))
        self.assertIsInstance(res["result"]["added"], int)
        self.assertGreater(res["result"]["added"], 0)
        listed = self.ok_json("events", "list", "demo")
        self.assertTrue(all(e["source"] == res["source"] for e in listed["events"]))

        for bad in ("not json", "[1]"):
            p = self.sai("run", "burst", "--dataset", "demo", "--params", bad)
            self.assertEqual(p.returncode, 2, bad)
            self.assertEqual(p.stdout, "")

    def test_algos_list_run_and_spec_checks(self):  # AC4
        self.make_demo()
        scripts = self.tmp / "ts" / "scripts"
        scripts.mkdir(exist_ok=True)
        (scripts / "echo.py").write_text(ECHO, encoding="utf-8")
        (scripts / "echo.json").write_text(json.dumps(SPEC_ECHO, ensure_ascii=False), encoding="utf-8")
        (scripts / "bare.py").write_text(ECHO, encoding="utf-8")

        rows = {a["name"]: a for a in self.ok_json("algos", "list")["algos"]}
        self.assertEqual(rows["echo"]["title"], "帯域")
        self.assertEqual(rows["bare"]["error"], "仕様（bare.json）がありません")
        self.assertIn("echo  project  帯域", self.sai("algos", "list").stdout)

        # 仕様のある物は値を検査し、既定で埋め、対象を渡す
        res = self.ok_json("run", "echo", "--dataset", "demo", "--vars", "ch01,ch02", "--params", '{"low_hz": 90}')
        self.assertEqual(res["result"]["params"], {"low_hz": 90, "envelope": True})
        self.assertEqual(res["result"]["target"], {"vars": ["ch01", "ch02"], "events": []})
        self.assertIsInstance(res["run"], str)
        row = [json.loads(x) for x in (self.tmp / "ts/datasets/demo/runs.jsonl").read_text().splitlines()][0]
        self.assertEqual((row["op"], row["by"], row["id"]), ("start", "agent", res["run"]))

        for args, code in [
            (["run", "echo", "--dataset", "demo"], 2),  # vars が無い
            (["run", "echo", "--dataset", "demo", "--vars", "ch01", "--params", '{"zz": 1}'], 2),
            (["run", "echo", "--dataset", "demo", "--vars", "ch01", "--params", '{"low_hz": -5}'], 2),
            (["run", "bare", "--dataset", "demo", "--vars", "ch01"], 2),  # 仕様が無ければ、--params を検査しない代わりに値は渡る
            (["run", "nosuch", "--dataset", "demo"], 3),
        ]:
            with self.subTest(args=args):
                p = self.sai(*args, "--json")
                if args[1] == "bare":
                    self.assertEqual(p.returncode, 0, p.stderr)  # 仕様が無い物は今までどおり動く
                    continue
                self.assertEqual((p.returncode, p.stdout), (code, ""))
                self.assertRegex(p.stderr.strip().splitlines()[-1], JA)

        # 親の実行（SAI_PARENT_RUN_ID）があれば、by は agent:<値>
        env = {**self.env, "SAI_PARENT_RUN_ID": "205"}
        self.ok_json("run", "bare", "--dataset", "demo", env=env)
        last = [json.loads(x) for x in (self.tmp / "ts/datasets/demo/runs.jsonl").read_text().splitlines()][-2]
        self.assertEqual(last["by"], "agent:205")

    def test_channels_get_set(self):  # AC3
        self.make_demo()
        self.assertEqual(self.ok_json("channels", "get", "demo"), {"version": 1, "channels": []})
        body = json.dumps({"channels": [{"name": "ch02", "gain": 2}, {"name": "ch01", "label": "Fp1"}]})
        res = self.ok_json("channels", "set", "demo", stdin=body)
        self.assertEqual([c["name"] for c in res["channels"]], ["ch02", "ch01"])
        self.assertEqual(self.ok_json("channels", "get", "demo")["channels"][0]["gain"], 2)

        p = self.sai("channels", "get", "demo")  # --json 無しは JSON を整形して出す
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(json.loads(p.stdout), res)

        for stdin, code in [('{"channels": [{"name": "ch01", "gain": 0}]}', 2), ("not json", 2)]:
            p = self.sai("channels", "set", "demo", stdin=stdin)
            self.assertEqual(p.returncode, code, p.stderr)
            self.assertEqual(p.stdout, "")
            self.assertRegex(p.stderr.strip().splitlines()[-1], JA)
        p = self.sai("channels", "get", "nosuch", "--json")
        self.assertEqual((p.returncode, p.stdout), (3, ""))

    def test_help_is_japanese(self):  # AC4
        for args in [["--help"]] + [c + ["--help"] for c in COMMANDS]:
            with self.subTest(args=args):
                p = self.sai(*args)
                self.assertEqual(p.returncode, 0, p.stderr)
                self.assertIn("使い方:", p.stdout)
                self.assertNotIn("usage:", p.stdout)

    def test_readme_lists_every_command(self):  # AC4
        readme = (APP / "README.md").read_text(encoding="utf-8")
        for c in COMMANDS:
            with self.subTest(c=c):
                self.assertIn("sai-ts " + " ".join(c), readme)

    def test_init_calls_sai_link(self):
        fake = self.tmp / "fake"
        env = {**self.env, "SAI_LINK_BIN": FAKE, "FAKE_LINK_DIR": str(fake)}
        res = self.ok_json("init", env=env)
        self.assertEqual(res["block"], "created")
        self.assertTrue((self.tmp / "ts" / "datasets").is_dir())

    def test_serve_public_host_needs_token(self):  # AC1（CLI）
        p = self.sai("serve", "--host", "0.0.0.0")  # SAI_TS_TOKEN は無い
        self.assertEqual(p.returncode, 2, p.stderr)
        self.assertEqual(p.stdout, "")
        self.assertEqual(p.stderr.splitlines()[-1], "127.0.0.1 以外で待つときは SAI_TS_TOKEN が要ります")

    def test_serve_host_choice_without_starting(self):  # AC1（関数）
        from unittest import mock
        from sai_ts import server

        saved = {k: os.environ.pop(k, None) for k in ("SAI_TS_TOKEN", "SAI_TS_HOST", "SAI_PROJECT_ROOT")}
        os.environ["SAI_PROJECT_ROOT"] = str(self.tmp)
        try:
            with mock.patch.object(server.uvicorn, "run") as run:
                self.assertEqual(server.serve(host="0.0.0.0"), 2)
                run.assert_not_called()

                os.environ["SAI_TS_TOKEN"] = "x"
                self.assertEqual(server.serve(port=8299, host="0.0.0.0"), 0)
                self.assertEqual(run.call_args.kwargs["host"], "0.0.0.0")

                os.environ.pop("SAI_TS_TOKEN")
                self.assertEqual(server.serve(port=8299), 0)  # --host 無しは 127.0.0.1
                self.assertEqual(run.call_args.kwargs["host"], "127.0.0.1")
        finally:
            for k in ("SAI_TS_TOKEN", "SAI_TS_HOST", "SAI_PROJECT_ROOT"):
                os.environ.pop(k, None)
                if saved[k] is not None:
                    os.environ[k] = saved[k]


if __name__ == "__main__":
    unittest.main()
