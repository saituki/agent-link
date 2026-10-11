"""スナップショット（AC1〜AC3）。一時のプロジェクトで書き出し・同名・作り直し・失敗の掃除・結ぶ（偽の sai-sci）・CLI。

実行:
    cd apps/ts && PYTHONPATH=src .venv/bin/python tests/test_snapshots.py
"""

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_ts import demo, events, runs, sdk, snapshots, watch  # noqa: E402

APP = Path(__file__).resolve().parent.parent
ENV_KEYS = ("SAI_PROJECT_ROOT", "SAI_PARENT_RUN_ID", "SAI_TS_SOURCE", "SAI_TS_SHA256", "SAI_TS_RUN",
            "SAI_SCI_BIN", "SAI_LINK_BIN")


def sha_file(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


class Project(unittest.TestCase):
    """一時のプロジェクト。派生変数 ch01_env（run a-test01）と 2 件の実行の記録とイベントを用意する。"""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        self._saved = {k: os.environ.pop(k, None) for k in ENV_KEYS}
        os.environ["SAI_PROJECT_ROOT"] = str(self.tmp)
        (self.tmp / ".sai").mkdir()
        self.ds = self.tmp / "ts" / "datasets" / "demo"
        demo.make(self.ds, records=2, nvars=3, seconds=2.0)
        self.root = self.tmp / "ts" / "snapshots"
        self._derive("a-test01", "bandpass", "1a2b3c4d", np.full(1000, 1.0, dtype=np.float32))
        # 実行の記録。a-test01 は派生変数の run、bandpass@1a2b3c4d はイベントの source と合う
        runs.append("demo", {"op": "start", "id": "a-test01", "algo": "bandpass", "origin": "project",
                             "sha256": "1a2b3c4d" + "0" * 56, "params": {}, "target": {}, "records": [],
                             "by": "human", "started_at": "2026-10-11T10:00:00Z"})
        runs.append("demo", {"op": "end", "id": "a-test01", "status": "ok", "result": {}, "error": "",
                             "ended_at": "2026-10-11T10:00:01Z", "elapsed_s": 1.0})
        runs.append("demo", {"op": "start", "id": "a-other", "algo": "other", "origin": "project",
                             "sha256": "f" * 64, "params": {}, "target": {}, "records": [], "by": "human",
                             "started_at": "2026-10-11T10:00:00Z"})
        events.add_events("demo", [
            {"type": "line", "record": "r01", "vars": ["ch01"], "t": 0.5, "category": "burst", "text": "", "metrics": {}},
            {"type": "line", "record": "r01", "vars": ["ch03"], "t": 1.5, "category": "noise", "text": "", "metrics": {}},
            {"type": "line", "record": "r02", "vars": ["ch02"], "t": 0.2, "category": "burst", "text": "", "metrics": {}},
        ], source="script:bandpass@1a2b3c4d")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)
        for k in ENV_KEYS:
            os.environ.pop(k, None)
            if self._saved[k] is not None:
                os.environ[k] = self._saved[k]

    def _derive(self, run, script, sha8, values):
        os.environ["SAI_TS_SOURCE"] = f"script:{script}@{sha8}"
        os.environ["SAI_TS_SHA256"] = sha8 + "0" * 56
        os.environ["SAI_TS_RUN"] = run
        sdk.add_var("demo", "r01", "ch01_env", values, "uV", params={"low_hz": 80}, inputs=["ch01"])
        for k in ("SAI_TS_SOURCE", "SAI_TS_SHA256", "SAI_TS_RUN"):
            os.environ.pop(k, None)


class SnapTest(Project):
    def test_create_manifest_matches_files(self):
        m = snapshots.create("demo", "snap1", ["r01"], [], [], "human")
        d = self.root / "snap1"
        # files の sha256 が実際のファイルと合う
        for rel, want in m["files"].items():
            self.assertEqual(sha_file(d / rel), want, rel)
        self.assertNotIn("manifest.json", m["files"])
        self.assertNotIn("links.json", m["files"])
        body = json.dumps(m["files"], sort_keys=True, ensure_ascii=False).encode("utf-8")
        self.assertEqual(hashlib.sha256(body).hexdigest(), m["sha256"])
        self.assertEqual(m["ref"], f"ts:snap1@{m['sha256'][:12]}")
        self.assertEqual(json.loads((d / "manifest.json").read_text(encoding="utf-8"))["ref"], m["ref"])
        self.assertEqual(m["created_by"], "human")
        self.assertEqual(m["records"], ["r01"])
        # 派生変数の来歴と、その実行の記録
        derived = [v for v in m["vars"] if v["source"] == "derived"]
        self.assertEqual([v["name"] for v in derived], ["ch01_env"])
        self.assertEqual(derived[0]["derived"]["run"], "a-test01")
        self.assertEqual(derived[0]["derived"]["script"], "bandpass")
        self.assertEqual(derived[0]["derived"]["params"], {"low_hz": 80})
        self.assertEqual([r["id"] for r in m["runs"]], ["a-test01"])
        self.assertEqual(m["events"]["sources"], ["script:bandpass@1a2b3c4d"])
        self.assertEqual(m["events"]["count"], 2)  # r01 の 2 件（r02 は別の記録）
        self.assertEqual(m["skipped_events"], 0)
        # 中身の形（meta・schema・events）
        meta = json.loads((d / "records" / "r01" / "meta.json").read_text(encoding="utf-8"))
        self.assertEqual({v["name"] for v in meta["vars"]}, {"ch01", "ch02", "ch03", "ch01_env"})
        self.assertFalse(list(self.root.glob(".tmp-*")))

    def test_vars_filter_skips_events_with_other_channels(self):
        m = snapshots.create("demo", "snap2", [], ["ch01", "ch02"], [], "agent:9")
        self.assertEqual(m["records"], ["r01", "r02"])
        self.assertEqual(m["created_by"], "agent:9")
        self.assertEqual(m["skipped_events"], 1)  # r01 の ch03 を含むイベント
        self.assertEqual({v["name"] for v in m["vars"]}, {"ch01", "ch02"})
        meta = json.loads((self.root / "snap2" / "records" / "r01" / "meta.json").read_text(encoding="utf-8"))
        self.assertEqual([v["name"] for v in meta["vars"]], ["ch01", "ch02"])
        self.assertEqual(m["events"]["count"], 2)  # r01 の ch01 と r02 の ch02

    def test_same_name_is_conflict(self):
        snapshots.create("demo", "snap1", ["r01"], [], [], "human")
        with self.assertRaises(FileExistsError):
            snapshots.create("demo", "snap1", ["r01"], [], [], "human")

    def test_rewriting_derived_keeps_snapshot_bytes(self):
        m = snapshots.create("demo", "snap1", ["r01"], [], [], "human")
        f = "records/r01/ch01_env.f32"
        before = sha_file(self.root / "snap1" / f)
        self.assertEqual(before, m["files"][f])
        # 元の派生変数を作り直す（os.replace で置き換わる）
        self._derive("a-test02", "bandpass", "1a2b3c4d", np.full(1000, 9.0, dtype=np.float32))
        self.assertNotEqual(sha_file(self.ds / "records" / "r01" / "ch01_env.f32"), before)
        self.assertEqual(sha_file(self.root / "snap1" / f), before)
        self.assertEqual(snapshots.show("snap1")["manifest"]["files"][f], before)

    def test_failure_leaves_no_temp_dir(self):
        with self.assertRaises(ValueError):
            snapshots.create("demo", "snap3", ["r01"], ["ch01", "nope"], [], "human")
        self.assertFalse((self.root / "snap3").exists())
        self.assertEqual(list(self.root.glob(".tmp-*")), [])

    def test_bad_name_and_missing_record(self):
        with self.assertRaises(ValueError):
            snapshots.create("demo", "../x", [], [], [], "human")
        with self.assertRaises(KeyError):
            snapshots.create("demo", "snap4", ["r99"], [], [], "human")

    def test_list_and_show(self):
        snapshots.create("demo", "snap1", ["r01"], [], [], "human")
        rows = snapshots.list_snapshots()
        self.assertEqual([r["name"] for r in rows], ["snap1"])
        self.assertEqual(rows[0]["vars"], ["ch01", "ch02", "ch03", "ch01_env"])
        self.assertEqual(rows[0]["events_count"], 2)
        self.assertEqual(rows[0]["links"], [])
        self.assertEqual(snapshots.show("snap1")["manifest"]["name"], "snap1")
        with self.assertRaises(KeyError):
            snapshots.show("nope")


class LinkTest(Project):
    """結ぶ処理は偽の sai-sci（台本）で確かめる。本物の sai-sci は呼ばない。"""

    def setUp(self):
        super().setUp()
        self.fake_dir = self.tmp / "fake"
        self.fake_dir.mkdir()
        self.script = self.fake_dir / "sai-sci"
        self.script.write_text(
            "#!/bin/sh\n"
            f"printf '%s\\n' \"$@\" > '{self.fake_dir}/args'\n"
            f"cat > '{self.fake_dir}/last.json'\n"
            f"if [ -f '{self.fake_dir}/rc' ]; then cat '{self.fake_dir}/err' >&2; exit \"$(cat '{self.fake_dir}/rc')\"; fi\n"
            "echo '{\"result\": \"E12\"}'\n", encoding="utf-8")
        self.script.chmod(0o755)
        os.environ["SAI_SCI_BIN"] = str(self.script)
        snapshots.create("demo", "snap1", ["r01"], [], [], "human")

    def _last(self):
        return json.loads((self.fake_dir / "last.json").read_text(encoding="utf-8"))

    def test_s_adds_e_and_e_sets_dataset(self):
        r = snapshots.link("snap1", "S3", None, "human")
        self.assertEqual(r["node"], "E12")
        self.assertEqual(r["op"], "add_e")
        ref = snapshots.show("snap1")["manifest"]["ref"]
        self.assertEqual(self._last(), {"op": "add_e", "parent": "S3", "text": "データセット snap1", "dataset": ref})
        self.assertEqual((self.fake_dir / "args").read_text().split(), ["apply", "--json"])
        r = snapshots.link("snap1", "E12", None, "agent:7")
        self.assertEqual(r["op"], "set_dataset")
        self.assertEqual(self._last(), {"op": "set_dataset", "node": "E12", "dataset": ref})
        links = snapshots.show("snap1")["links"]
        self.assertEqual([(x["node"], x["op"], x["by"]) for x in links],
                         [("E12", "add_e", "human"), ("E12", "set_dataset", "agent:7")])
        self.assertEqual(snapshots.list_snapshots()[0]["links"], links)

    def test_sci_error_maps_status(self):
        (self.fake_dir / "rc").write_text("3")
        (self.fake_dir / "err").write_text("見つかりません: E99\n")
        with self.assertRaises(snapshots.SciError) as cm:
            snapshots.link("snap1", "E99", None, "human")
        self.assertEqual(cm.exception.status, 404)
        self.assertEqual(cm.exception.detail, "見つかりません: E99")
        self.assertEqual(snapshots.show("snap1")["links"], [], "失敗したら links に残さない")

    def test_bad_node(self):
        with self.assertRaises(ValueError):
            snapshots.link("snap1", "X3", None, "human")
        with self.assertRaises(ValueError):
            snapshots.link("snap1", "S0", None, "human")


class CliTest(Project):
    def _sai(self, *args, extra=None):
        env = {**os.environ, "PYTHONPATH": str(APP / "src")}
        env.pop("SAI_PARENT_RUN_ID", None)
        env.update(extra or {})
        return subprocess.run([sys.executable, "-m", "sai_ts.cli", *args], capture_output=True, text=True,
                              env=env, cwd=self.tmp, timeout=60)

    def test_cli_create_list_show_and_codes(self):
        p = self._sai("snapshot", "create", "demo", "--name", "cli1", "--record", "r01", "--vars", "ch01,ch02")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("書き出しました: cli1", p.stdout)
        p = self._sai("snapshot", "create", "demo", "--name", "cli1", "--json")
        self.assertEqual(p.returncode, 4)
        self.assertEqual(p.stdout, "")
        self.assertTrue(p.stderr.strip().endswith("同じ名前のスナップショットがあります: cli1"))
        p = self._sai("snapshot", "list", "--json")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual([s["name"] for s in json.loads(p.stdout)["snapshots"]], ["cli1"])
        p = self._sai("snapshot", "show", "cli1", "--json")
        self.assertEqual(json.loads(p.stdout)["manifest"]["name"], "cli1")
        p = self._sai("snapshot", "show", "nope", "--json")
        self.assertEqual(p.returncode, 3)
        p = self._sai("snapshot", "create", "demo", "--name", "bad/name")
        self.assertEqual(p.returncode, 2)

    def test_cli_link_uses_fake_sci(self):
        fake = self.tmp / "fake-sci"
        fake.write_text("#!/bin/sh\ncat >/dev/null\necho '{\"result\": \"E5\"}'\n", encoding="utf-8")
        fake.chmod(0o755)
        self._sai("snapshot", "create", "demo", "--name", "cli2")
        p = self._sai("snapshot", "link", "cli2", "--node", "S1", "--json", extra={"SAI_SCI_BIN": str(fake)})
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(json.loads(p.stdout)["node"], "E5")


class WatchApiTest(Project):
    """変化の知らせ（1 回の差分に 1 つ）と、HTTP の住所（201・409・404・結ぶ）。"""

    def test_change_is_announced_once(self):
        old = watch.snapshot()
        snapshots.create("demo", "snap1", ["r01"], [], [], "human")
        self.assertEqual(watch.diff(old, watch.snapshot()), [{"kind": "snapshots"}])

    def test_http(self):
        from fastapi.testclient import TestClient
        from sai_ts.server import app

        os.environ.pop("SAI_TS_TOKEN", None)
        c = TestClient(app)
        r = c.post("/api/datasets/demo/snapshots", json={"name": "h1", "records": ["r01"]})
        self.assertEqual(r.status_code, 201, r.text)
        self.assertEqual(r.json()["created_by"], "human")
        self.assertEqual(c.post("/api/datasets/demo/snapshots", json={"name": "h1"}).status_code, 409)
        self.assertEqual(c.post("/api/datasets/nope/snapshots", json={"name": "h2"}).status_code, 404)
        self.assertEqual(c.post("/api/datasets/demo/snapshots", json={"name": "bad/x"}).status_code, 400)
        self.assertEqual([s["name"] for s in c.get("/api/snapshots").json()["snapshots"]], ["h1"])
        self.assertEqual(c.get("/api/snapshots/h1").json()["manifest"]["name"], "h1")
        self.assertEqual(c.get("/api/snapshots/nope").status_code, 404)
        fake = self.tmp / "sci"
        fake.write_text("#!/bin/sh\ncat >/dev/null\necho '{\"result\": \"E7\"}'\n", encoding="utf-8")
        fake.chmod(0o755)
        os.environ["SAI_SCI_BIN"] = str(fake)
        r = c.post("/api/snapshots/h1/link", json={"node": "S2"})
        self.assertEqual(r.status_code, 200, r.text)
        self.assertEqual(r.json()["node"], "E7")
        self.assertEqual(c.post("/api/snapshots/h1/link", json={"node": "Q2"}).status_code, 400)
        fake.write_text("#!/bin/sh\ncat >/dev/null\necho '見つかりません: E9' >&2\nexit 3\n", encoding="utf-8")
        r = c.post("/api/snapshots/h1/link", json={"node": "E9"})
        self.assertEqual((r.status_code, r.json()["detail"]), (404, "見つかりません: E9"))


if __name__ == "__main__":
    unittest.main()
