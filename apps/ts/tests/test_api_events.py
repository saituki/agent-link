"""イベント・スキーマ・画面の状態の API（AC5）。成功・400・404・TSV の返り方。"""

import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from fastapi.testclient import TestClient  # noqa: E402

from sai_ts import demo  # noqa: E402
from sai_ts.server import app  # noqa: E402


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


AREA = {"type": "area", "record": "r01", "vars": ["ch01"], "t0": 1.0, "t1": 2.0,
        "category": "burst", "text": "前頭部", "metrics": {"amp": 3.2}}
LINE = {"type": "line", "record": "r01", "vars": [], "t": 4.0, "category": "noise"}


class ApiEventsTest(TempProject, unittest.TestCase):
    def setUp(self):
        super().setUp()
        demo.make(self.tmp / "ts" / "datasets" / "demo", records=2, nvars=2, seconds=10.0, srate=500.0)
        self.c = TestClient(app)

    def test_events_crud_success(self):
        r = self.c.post("/api/datasets/demo/events", json=AREA)
        self.assertEqual(r.status_code, 200)
        row = r.json()
        self.assertEqual((row["rev"], row["source"], row["metrics"]), (1, "human", {"amp": 3.2}))
        eid = row["id"]

        r = self.c.post("/api/datasets/demo/events", json=LINE)
        self.assertEqual(r.status_code, 200)

        r = self.c.get("/api/datasets/demo/events")
        self.assertEqual(r.json()["count"], 2)
        self.assertEqual([e["type"] for e in r.json()["events"]], ["area", "line"])
        r = self.c.get("/api/datasets/demo/events", params={"category": "noise"})
        self.assertEqual([e["type"] for e in r.json()["events"]], ["line"])
        r = self.c.get("/api/datasets/demo/events", params={"record": "r02"})
        self.assertEqual(r.json(), {"events": [], "count": 0})

        r = self.c.patch(f"/api/datasets/demo/events/{eid}", json={"text": "直した", "source": "agent"})
        self.assertEqual(r.status_code, 200)
        self.assertEqual((r.json()["rev"], r.json()["text"], r.json()["source"]), (2, "直した", "human"))

        r = self.c.delete(f"/api/datasets/demo/events/{eid}")
        self.assertEqual(r.json(), {"id": eid, "deleted": True})
        self.assertEqual(self.c.get("/api/datasets/demo/events").json()["count"], 1)

    def test_400_and_404(self):
        self.assertEqual(self.c.post("/api/datasets/demo/events", json={**AREA, "category": "x"}).status_code, 400)
        self.assertEqual(self.c.post("/api/datasets/demo/events", json={**AREA, "t1": 99}).status_code, 400)
        self.assertEqual(self.c.post("/api/datasets/nope/events", json=AREA).status_code, 404)
        self.assertEqual(self.c.get("/api/datasets/demo/events", params={"type": "point"}).status_code, 400)
        self.assertEqual(self.c.patch("/api/datasets/demo/events/e-000000000000", json={"text": "x"}).status_code, 404)
        self.assertEqual(self.c.delete("/api/datasets/demo/events/e-000000000000").status_code, 404)
        r = self.c.post("/api/datasets/demo/events", json={**AREA, "category": "x"})
        self.assertIn("detail", r.json())

    def test_schema_get_put(self):
        s = self.c.get("/api/datasets/demo/schema").json()
        self.assertEqual([c["name"] for c in s["categories"]], ["burst", "noise"])

        ok = {"categories": [{"name": "burst", "color": "#d9534f"}],
              "metrics": [{"name": "amp", "type": "number", "min": 0, "max": 100}]}
        self.assertEqual(self.c.put("/api/datasets/demo/schema", json=ok).status_code, 200)
        self.assertEqual(self.c.get("/api/datasets/demo/schema").json()["metrics"][0]["max"], 100)

        self.assertEqual(self.c.post("/api/datasets/demo/events", json=AREA).status_code, 200)
        r = self.c.put("/api/datasets/demo/schema", json={"categories": [{"name": "noise", "color": "#000000"}]})
        self.assertEqual(r.status_code, 400)
        self.assertIn("burst", r.json()["detail"])
        self.assertEqual(self.c.put("/api/datasets/demo/schema", json={"categories": "x"}).status_code, 400)

    def test_tsv(self):
        self.c.post("/api/datasets/demo/events", json=AREA)
        r = self.c.get("/api/datasets/demo/events.tsv")
        self.assertEqual(r.status_code, 200)
        self.assertTrue(r.headers["content-type"].startswith("text/tab-separated-values"))
        self.assertEqual(r.headers["content-disposition"], 'attachment; filename="demo-events.tsv"')
        self.assertTrue(r.text.splitlines()[0].startswith("onset\tduration\ttrial_type"))
        self.assertIn("\tburst\tch01\tr01\t前頭部\thuman\t", r.text)
        self.assertEqual(self.c.get("/api/datasets/nope/events.tsv").status_code, 404)

    def test_view_get_put(self):
        self.assertEqual(self.c.get("/api/view").json(), {})
        r = self.c.put("/api/view", json={"dataset": "demo", "record": "r01", "t0": 1.0, "t1": 2.0})
        self.assertEqual(r.status_code, 200)
        self.assertEqual((r.json()["updated_by"], r.json()["record"]), ("human", "r01"))
        r = self.c.put("/api/view", json={"selected_events": ["e-3f9a1c2b7d10"]})
        self.assertEqual(r.json()["t1"], 2.0)
        self.assertEqual(self.c.get("/api/view").json()["selected_events"], ["e-3f9a1c2b7d10"])
        self.assertEqual(self.c.put("/api/view", json={"t0": 3.0}).status_code, 400)


if __name__ == "__main__":
    unittest.main()
