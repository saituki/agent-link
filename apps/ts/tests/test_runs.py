"""実行の記録 runs.jsonl（AC2）。start と end のまとめ方・running と lost・同時の追記・途中で切れた行。"""

import json
import os
import shutil
import sys
import tempfile
import threading
import unittest
from datetime import datetime, timezone
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_ts import demo, runs  # noqa: E402


T0 = datetime(2026, 10, 11, 10, 0, 0, tzinfo=timezone.utc).timestamp()  # start の時刻（"2026-10-11T10:00:00Z"）


def start(id, when, algo="burst"):
    return {"op": "start", "id": id, "algo": algo, "origin": "project", "sha256": "0" * 64, "params": {},
            "target": {"vars": [], "events": []}, "records": [], "by": "human", "started_at": when}


def end(id, status="ok", result=None):
    return {"op": "end", "id": id, "status": status, "result": result or {}, "error": "",
            "ended_at": "2026-10-11T10:00:05Z", "elapsed_s": 5.0}


class RunsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        self._saved = os.environ.pop("SAI_PROJECT_ROOT", None)
        os.environ["SAI_PROJECT_ROOT"] = str(self.tmp)
        (self.tmp / ".sai").mkdir()
        demo.make(self.tmp / "ts" / "datasets" / "demo", records=1, nvars=1, seconds=1.0)
        self.file = self.tmp / "ts" / "datasets" / "demo" / "runs.jsonl"

    def tearDown(self):
        os.environ.pop("SAI_PROJECT_ROOT", None)
        if self._saved is not None:
            os.environ["SAI_PROJECT_ROOT"] = self._saved
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_start_and_end_are_merged_by_id(self):
        runs.append("demo", start("a-000000000001", "2026-10-11T10:00:00Z"))
        runs.append("demo", start("a-000000000002", "2026-10-11T10:01:00Z", algo="other"))
        runs.append("demo", end("a-000000000001", result={"added": 3}))
        with mock.patch.object(runs, "STARTED_AT", T0 + 3600):
            rows = runs.list_runs("demo")
        self.assertEqual([r["id"] for r in rows], ["a-000000000002", "a-000000000001"])  # 新しい順
        done = rows[1]
        self.assertEqual((done["status"], done["result"], done["elapsed_s"]), ("ok", {"added": 3}, 5.0))
        self.assertNotIn("op", done)
        self.assertEqual(rows[0]["status"], "lost")  # end が無く、このプロセスより前の start

    def test_running_and_lost_split_by_process_start(self):
        runs.append("demo", start("a-000000000001", "2026-10-11T10:00:00Z"))
        with mock.patch.object(runs, "STARTED_AT", T0 - 60):  # このプロセスは start より前に始まった（この中の分）
            self.assertEqual(runs.list_runs("demo")[0]["status"], "running")
        with mock.patch.object(runs, "STARTED_AT", T0 + 3600):  # このプロセスは start より後に始まった（前のプロセスの分）
            self.assertEqual(runs.list_runs("demo")[0]["status"], "lost")
        runs.append("demo", start("a-000000000002", "not a time"))
        with mock.patch.object(runs, "STARTED_AT", T0 - 60):
            self.assertEqual({r["id"]: r["status"] for r in runs.list_runs("demo")},
                             {"a-000000000001": "running", "a-000000000002": "lost"})

    def test_limit_and_order(self):
        for i in range(5):
            runs.append("demo", start(f"a-00000000000{i}", f"2026-10-11T10:00:0{i}Z"))
        got = runs.list_runs("demo", limit=2)
        self.assertEqual([r["id"] for r in got], ["a-000000000004", "a-000000000003"])

    def test_partial_line_is_not_glued_and_bad_line_skipped(self):
        self.file.write_text('{"op": "start", "id": "a-0000000000', encoding="utf-8")  # 途中で切れた行
        runs.append("demo", start("a-000000000009", "2026-10-11T10:00:00Z"))
        lines = self.file.read_text(encoding="utf-8").splitlines()
        self.assertEqual(json.loads(lines[-1])["id"], "a-000000000009")
        self.assertEqual([r["id"] for r in runs.list_runs("demo")], ["a-000000000009"])

    def test_concurrent_appends_keep_lines_whole(self):  # AC2: スレッド 4 本で各 50 行
        def work(k):
            for i in range(50):
                runs.append("demo", start(f"a-{k:02d}{i:010d}", "2026-10-11T10:00:00Z"))

        ts = [threading.Thread(target=work, args=(k,)) for k in range(4)]
        for t in ts:
            t.start()
        for t in ts:
            t.join()
        lines = self.file.read_text(encoding="utf-8").splitlines()
        rows = [json.loads(x) for x in lines]  # 1 行でも壊れていれば落ちる
        self.assertEqual(len(rows), 200)
        self.assertEqual(len({r["id"] for r in rows}), 200)

    def test_missing_dataset_is_key_error(self):
        with self.assertRaises(KeyError):
            runs.append("nope", start("a-000000000001", "2026-10-11T10:00:00Z"))
        with self.assertRaises(KeyError):
            runs.list_runs("nope")


if __name__ == "__main__":
    unittest.main()
