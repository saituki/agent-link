"""変化の知らせ（AC5）。"""

import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_ts import demo, paths, watch  # noqa: E402


class TempProject:
    """setUp で一時フォルダを作って SAI_PROJECT_ROOT に入れ、tearDown で戻して消す。"""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        self._saved = {k: os.environ.pop(k, None) for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN")}
        os.environ["SAI_PROJECT_ROOT"] = str(self.tmp)

    def tearDown(self):
        for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN"):
            os.environ.pop(k, None)
            if self._saved[k] is not None:
                os.environ[k] = self._saved[k]
        shutil.rmtree(self.tmp, ignore_errors=True)


class WatchTest(TempProject, unittest.TestCase):
    def setUp(self):
        super().setUp()
        ds = self.tmp / "ts" / "datasets" / "demo"
        ds.mkdir(parents=True)
        (ds / "events.jsonl").write_text('{"op":"put"}\n', encoding="utf-8")

    def test_no_change_no_event(self):
        s = watch.snapshot()
        self.assertEqual(watch.diff(s, watch.snapshot()), [])

    def test_events_append(self):
        s0 = watch.snapshot()
        with open(self.tmp / "ts" / "datasets" / "demo" / "events.jsonl", "a", encoding="utf-8") as f:
            f.write('{"op":"del"}\n')
        self.assertEqual(watch.diff(s0, watch.snapshot()), [{"kind": "events", "dataset": "demo"}])

    def test_vars_meta_change(self):
        demo.make(self.tmp / "ts" / "datasets" / "demo2", records=1, nvars=1, seconds=1.0)
        s0 = watch.snapshot()
        meta = self.tmp / "ts" / "datasets" / "demo2" / "records" / "r01" / "meta.json"
        meta.write_text(meta.read_text(encoding="utf-8") + " ", encoding="utf-8")
        self.assertEqual(watch.diff(s0, watch.snapshot()),
                         [{"kind": "vars", "dataset": "demo2", "record": "r01"}])

    def test_ext_version_change_and_removal(self):
        ext = self.tmp / "ts" / "ext" / "review"
        ext.mkdir(parents=True)
        (ext / "manifest.json").write_text('{"entry":"main.js"}', encoding="utf-8")
        (ext / "main.js").write_text("export default function activate(ts) {}", encoding="utf-8")
        v0 = watch.ext_version(ext)
        s0 = watch.snapshot()

        (ext / "main.js").write_text("export default function activate(ts) { /*改*/ }", encoding="utf-8")
        ev = watch.diff(s0, watch.snapshot())
        self.assertEqual(len(ev), 1)
        self.assertEqual(ev[0]["kind"], "ext")
        self.assertEqual(ev[0]["name"], "review")
        self.assertNotEqual(ev[0]["version"], v0)
        self.assertEqual(ev[0]["version"], watch.ext_version(ext))

        s1 = watch.snapshot()
        shutil.rmtree(ext)
        self.assertEqual(watch.diff(s1, watch.snapshot()),
                         [{"kind": "ext", "name": "review", "version": None}])

    def test_view_only_when_agent(self):
        view = self.tmp / "ts" / "view.json"
        view.write_text(json.dumps({"updated_by": "agent"}), encoding="utf-8")
        s0 = watch.snapshot()

        view.write_text(json.dumps({"updated_by": "human", "t0": 1.0}), encoding="utf-8")
        self.assertEqual(watch.diff(s0, watch.snapshot()), [])  # 画面からの変更は知らせない

        s1 = watch.snapshot()
        view.write_text(json.dumps({"updated_by": "agent", "t0": 2.0}), encoding="utf-8")
        self.assertEqual(watch.diff(s1, watch.snapshot()), [{"kind": "view"}])

    def test_ext_version_is_deterministic(self):
        ext = self.tmp / "ts" / "ext" / "x"
        ext.mkdir(parents=True)
        (ext / "a.js").write_text("1", encoding="utf-8")
        self.assertEqual(watch.ext_version(ext), watch.ext_version(ext))
        self.assertEqual(len(watch.ext_version(ext)), 12)


if __name__ == "__main__":
    unittest.main()
