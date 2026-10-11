"""アルゴリズム（AC1・AC3）: 仕様の検査・要求の検査・一覧（組み込みとプロジェクト）・HTTP での裏の実行。"""

import copy
import hashlib
import json
import os
import shutil
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from fastapi.testclient import TestClient  # noqa: E402

from sai_ts import algos, demo  # noqa: E402
from sai_ts.server import app  # noqa: E402

# G2-C1 の例そのもの
SPEC = {
    "version": 1, "title": "バンドパスと包絡", "kind": "channel", "target": "vars",
    "desc": "選んだ ch を帯域通過し、包絡の ch も足す",
    "params": [
        {"name": "low_hz", "type": "number", "default": 80, "min": 0, "unit": "Hz", "desc": "通過域の下端"},
        {"name": "high_hz", "type": "number", "default": 250, "min": 0, "unit": "Hz", "desc": "通過域の上端"},
        {"name": "envelope", "type": "boolean", "default": True, "desc": "包絡の ch も足す"},
        {"name": "mode", "type": "enum", "values": ["fft", "box"], "default": "fft"},
    ],
}
NONE = {"version": 1, "title": "対象なし", "kind": "measure", "target": "none"}
EVENTS = {"version": 1, "title": "イベント", "kind": "measure", "target": "events"}

# 標準入力の params と target を、標準出力に JSON で返すだけの物
ECHO = '''import json, sys
req = json.load(sys.stdin)
print(json.dumps({"message": "ok", "params": req["params"], "target": req["target"]}))
'''
FAIL = "import sys\nsys.stderr.write('理由あり\\n')\nsys.exit(3)\n"


class TempProject:
    """setUp で一時フォルダを作って SAI_PROJECT_ROOT に入れ、tearDown で戻して消す。組み込みの置き場も一時にする。"""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        self._saved = {k: os.environ.pop(k, None) for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN")}
        os.environ["SAI_PROJECT_ROOT"] = str(self.tmp)
        (self.tmp / ".sai").mkdir()
        self.builtin = Path(tempfile.mkdtemp()).resolve()
        p = mock.patch.object(algos, "BUILTIN_DIR", self.builtin)
        p.start()
        self.addCleanup(p.stop)

    def tearDown(self):
        for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN"):
            os.environ.pop(k, None)
            if self._saved[k] is not None:
                os.environ[k] = self._saved[k]
        shutil.rmtree(self.tmp, ignore_errors=True)
        shutil.rmtree(self.builtin, ignore_errors=True)

    def put(self, folder: Path, name: str, body: str, spec=None, raw_spec: str | None = None):
        folder.mkdir(parents=True, exist_ok=True)
        (folder / f"{name}.py").write_text(body, encoding="utf-8")
        if raw_spec is not None:
            (folder / f"{name}.json").write_text(raw_spec, encoding="utf-8")
        elif spec is not None:
            (folder / f"{name}.json").write_text(json.dumps(spec, ensure_ascii=False), encoding="utf-8")


class SpecTest(unittest.TestCase):
    def test_valid_example_is_normalized(self):
        s = algos.check_spec(SPEC)
        self.assertEqual(s["params"][3]["default"], "fft")
        self.assertEqual((s["kind"], s["target"], s["desc"]), ("channel", "vars", SPEC["desc"]))
        self.assertEqual(algos.check_spec(NONE)["desc"], "")

    def test_bad_specs_are_refused(self):  # AC1（仕様の検査）
        cases = {
            "version 2": lambda s: s.update(version=2),
            "version True": lambda s: s.update(version=True),
            "title empty": lambda s: s.update(title=""),
            "title 81 chars": lambda s: s.update(title="あ" * 81),
            "kind": lambda s: s.update(kind="nope"),
            "target": lambda s: s.update(target="ch"),
            "desc 2001 chars": lambda s: s.update(desc="x" * 2001),
            "unknown top key": lambda s: s.update(extra=1),
            "unknown param key": lambda s: s["params"][0].update(defualt=1),
            "duplicate param": lambda s: s["params"].append(dict(s["params"][0])),
            "bad param name": lambda s: s["params"][0].update(name="../x"),
            "bad param type": lambda s: s["params"][0].update(type="float"),
            "enum without values": lambda s: s["params"][3].pop("values"),
            "number default as string": lambda s: s["params"][0].update(default="80"),
            "boolean default as number": lambda s: s["params"][2].update(default=1),
            "default below min": lambda s: s["params"][0].update(default=-1),
            "enum default not in values": lambda s: s["params"][3].update(default="zzz"),
            "min above max": lambda s: s["params"][0].update(max=-5),
            "params not a list": lambda s: s.update(params={}),
        }
        for label, mutate in cases.items():
            with self.subTest(label):
                s = copy.deepcopy(SPEC)
                mutate(s)
                with self.assertRaises(ValueError):
                    algos.check_spec(s)


class RequestTest(unittest.TestCase):
    def test_defaults_fill_and_target_is_kept(self):  # AC1（要求の検査）
        r = algos.check_request(algos.check_spec(SPEC), {"target": {"vars": ["ch01"], "events": []}})
        self.assertEqual(r, {"records": [], "params": {"low_hz": 80, "high_hz": 250, "envelope": True, "mode": "fft"},
                             "target": {"vars": ["ch01"], "events": []}})

    def test_boolean_without_default_is_false(self):
        spec = algos.check_spec({"version": 1, "title": "t", "kind": "measure", "target": "none",
                                 "params": [{"name": "flag", "type": "boolean"}]})
        self.assertEqual(algos.check_request(spec, {})["params"], {"flag": False})

    def test_refusals(self):  # AC1（要求の検査・G2-C5）
        spec = algos.check_spec(SPEC)
        one = {"vars": ["ch01"], "events": []}
        cases = {
            "unknown param": {"params": {"zz": 1}, "target": one},
            "number as string": {"params": {"low_hz": "80"}, "target": one},
            "below min": {"params": {"low_hz": -1}, "target": one},
            "enum not in values": {"params": {"mode": "x"}, "target": one},
            "boolean as number": {"params": {"envelope": 1}, "target": one},
            "no vars": {"target": {"vars": [], "events": []}},
            "vars with events": {"target": {"vars": ["ch01"], "events": ["e-3f9a1c2b7d10"]}},
            "bad var name": {"target": {"vars": ["../x"], "events": []}},
            "unknown target key": {"target": {"vars": ["ch01"], "record": "r01"}},
            "bad record name": {"records": ["../r"], "target": one},
            "params not object": {"params": [1], "target": one},
            "unknown body key": {"target": one, "x": 1},
        }
        for label, body in cases.items():
            with self.subTest(label):
                with self.assertRaises(ValueError):
                    algos.check_request(spec, body)

        ev = algos.check_spec(EVENTS)
        self.assertEqual(algos.check_request(ev, {"target": {"vars": [], "events": []}})["target"],
                         {"vars": [], "events": []})
        with self.assertRaises(ValueError):
            algos.check_request(ev, {"target": {"vars": [], "events": ["e-zzz"]}})
        with self.assertRaises(ValueError):
            algos.check_request(algos.check_spec(NONE), {"target": {"vars": ["ch01"], "events": []}})


class ListTest(TempProject, unittest.TestCase):
    def setUp(self):
        super().setUp()
        self.proj = self.tmp / "ts" / "scripts"
        # 同じ名前（echo）は組み込みとプロジェクトの両方に置く。プロジェクトが勝つ
        self.put(self.builtin, "echo", ECHO, {**SPEC, "title": "組み込み"})
        self.put(self.proj, "echo", ECHO, {**SPEC, "title": "プロジェクト"})
        self.put(self.builtin, "only", ECHO, NONE)
        self.put(self.proj, "nojson", ECHO)
        self.put(self.proj, "broken", ECHO, raw_spec="{")
        self.put(self.proj, "_dsp", ECHO, NONE)  # 共通の部品。一覧に出ない

    def test_list_project_wins_and_errors_show(self):  # AC1（一覧）
        rows = algos.list_algos()
        self.assertEqual([r["name"] for r in rows], ["broken", "echo", "nojson", "only"])
        by = {r["name"]: r for r in rows}
        self.assertEqual((by["echo"]["origin"], by["echo"]["title"]), ("project", "プロジェクト"))
        self.assertEqual((by["only"]["origin"], by["only"]["title"]), ("builtin", "対象なし"))
        self.assertEqual(by["nojson"]["error"], "仕様（nojson.json）がありません")
        self.assertTrue(by["broken"]["error"].startswith("仕様（broken.json）"))
        self.assertEqual(by["echo"]["sha256"], hashlib.sha256(ECHO.encode("utf-8")).hexdigest())
        self.assertEqual(by["echo"]["kind"], "channel")

    def test_find_and_locate(self):
        path, spec = algos.find("echo")
        self.assertEqual(path, (self.proj / "echo.py").resolve())
        self.assertEqual(spec["target"], "vars")
        self.assertEqual(algos.locate("only")[1], "builtin")
        with self.assertRaises(algos.NoSpec):
            algos.find("nojson")
        with self.assertRaises(KeyError):
            algos.find("nope")
        with self.assertRaises(ValueError):
            algos.find("../x")

    def test_link_out_of_the_folder_is_not_used(self):
        outside = self.tmp / "outside.py"
        outside.write_text(ECHO, encoding="utf-8")
        os.symlink(outside, self.builtin / "link.py")
        os.symlink(outside, self.builtin / "link.json")
        self.assertNotIn("link", [r["name"] for r in algos.list_algos()])
        with self.assertRaises(KeyError):
            algos.locate("link")


class ApiTest(TempProject, unittest.TestCase):
    def setUp(self):
        super().setUp()
        demo.make(self.tmp / "ts" / "datasets" / "demo", records=2, nvars=2, seconds=2.0, srate=100.0)
        self.put(self.tmp / "ts" / "scripts", "echo", ECHO, SPEC)
        self.put(self.tmp / "ts" / "scripts", "bad", FAIL, {**NONE, "title": "失敗"})
        self.put(self.tmp / "ts" / "scripts", "nospec", ECHO)
        self.put(self.builtin, "hello", ECHO, NONE)  # 組み込み（試験の一時の BUILTIN_DIR）
        self.c = TestClient(app)

    def wait(self, run_id: str) -> dict:
        # 裏の実行は数秒以内に終わる。終わった記録が読めるまで待つ（試験の後始末のため）
        end = time.monotonic() + 60
        while time.monotonic() < end:
            rows = self.c.get("/api/datasets/demo/runs?limit=50").json()["runs"]
            row = next((r for r in rows if r["id"] == run_id), None)
            if row is not None and row["status"] != "running":
                return row
            time.sleep(0.2)
        self.fail("実行が終わりません")

    def test_list_algos_over_http(self):
        body = self.c.get("/api/algos").json()["algos"]
        self.assertEqual([a["name"] for a in body], ["bad", "echo", "hello", "nospec"])
        echo = next(a for a in body if a["name"] == "echo")
        self.assertEqual((echo["origin"], echo["target"], echo["error"]), ("project", "vars", ""))
        self.assertEqual(next(a for a in body if a["name"] == "nospec")["error"], "仕様（nospec.json）がありません")

    def test_run_is_202_and_result_is_recorded(self):  # AC3
        req = {"records": ["r01"], "params": {"low_hz": 90}, "target": {"vars": ["ch01"], "events": []}}
        r = self.c.post("/api/datasets/demo/algos/echo/runs", json=req)
        self.assertEqual(r.status_code, 202, r.text)
        row = self.wait(r.json()["run"])
        self.assertEqual((row["status"], row["by"], row["algo"]), ("ok", "human", "echo"))
        self.assertEqual(row["result"]["params"], {"low_hz": 90, "high_hz": 250, "envelope": True, "mode": "fft"})
        self.assertEqual(row["result"]["target"], {"vars": ["ch01"], "events": []})
        self.assertEqual(row["records"], ["r01"])

    def test_failure_keeps_the_reason(self):  # AC3
        r = self.c.post("/api/datasets/demo/algos/bad/runs", json={})
        self.assertEqual(r.status_code, 202, r.text)
        row = self.wait(r.json()["run"])
        self.assertEqual((row["status"], row["error"]), ("error", "bad が失敗しました: 理由あり"))

    def test_builtin_runs_too(self):  # AC3（組み込み）
        r = self.c.post("/api/datasets/demo/algos/hello/runs", json={})
        row = self.wait(r.json()["run"])
        self.assertEqual((row["status"], row["origin"], row["result"]["message"]), ("ok", "builtin", "ok"))

    def test_refusals(self):
        bad_params = {"params": {"low_hz": "x"}, "target": {"vars": ["ch01"], "events": []}}
        self.assertEqual(self.c.post("/api/datasets/demo/algos/echo/runs", json=bad_params).status_code, 400)
        self.assertEqual(self.c.post("/api/datasets/demo/algos/nope/runs", json={}).status_code, 404)
        no = self.c.post("/api/datasets/demo/algos/nospec/runs", json={})
        self.assertEqual((no.status_code, no.json()["detail"]), (400, "仕様（nospec.json）がありません"))
        self.assertEqual(self.c.post("/api/datasets/nope/algos/hello/runs", json={}).status_code, 404)
        self.assertEqual(self.c.get("/api/datasets/demo/runs?limit=0").status_code, 400)
        self.assertEqual(self.c.get("/api/datasets/demo/runs?limit=501").status_code, 400)
        self.assertEqual(self.c.get("/api/datasets/nope/runs").status_code, 404)


if __name__ == "__main__":
    unittest.main()
