"""C3 の API・400/404・合言葉（AC4）。"""

import base64
import os
import shutil
import tempfile
import sys
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


class ServerDataTest(TempProject, unittest.TestCase):
    def setUp(self):
        super().setUp()
        demo.make(self.tmp / "ts" / "datasets" / "demo", records=2, nvars=2, seconds=10.0, srate=500.0)
        self.client = TestClient(app)

    def test_datasets(self):
        r = self.client.get("/api/datasets")
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.json(), {"datasets": [{"name": "demo", "records": ["r01", "r02"]}]})

    def test_meta(self):
        r = self.client.get("/api/datasets/demo/records/r01/meta")
        self.assertEqual(r.status_code, 200)
        self.assertEqual((r.json()["version"], r.json()["n_samples"]), (1, 5000))

    def test_m4(self):
        r = self.client.get("/api/datasets/demo/records/r01/m4",
                            params={"vars": "ch01,ch02", "t0": 0, "t1": 10, "px": 100})
        self.assertEqual(r.status_code, 200)
        body = r.json()
        self.assertEqual((body["t0"], body["t1"], body["px"], body["srate"]), (0.0, 10.0, 100, 500.0))
        for v in ("ch01", "ch02"):
            self.assertEqual(body["vars"][v]["mode"], "m4")
            self.assertEqual(len(body["vars"][v]["first"]), 100)

    def test_m4_raw(self):
        r = self.client.get("/api/datasets/demo/records/r01/m4",
                            params={"vars": "ch01", "t0": 1, "t1": 1.5, "px": 200})
        self.assertEqual(r.json()["vars"]["ch01"]["mode"], "raw")

    def test_spec(self):
        url = "/api/datasets/demo/records/r01/spec"
        q = {"var": "ch01", "t0": 0, "t1": 10, "cols": 50, "fmin": 0, "fmax": 250, "win_ms": 250}
        r = self.client.get(url, params=q)
        self.assertEqual(r.status_code, 200)
        body = r.json()
        self.assertEqual((body["cols"], body["fmin"], body["fmax"]), (50, 0, 250))
        self.assertEqual(len(base64.b64decode(body["data"])), body["rows"] * 50)
        for bad in [{"var": "zz"}, {"cols": 0}, {"fmin": 100, "fmax": 100}, {"win_ms": 0}, {"t0": 10, "t1": 10}]:
            self.assertEqual(self.client.get(url, params={**q, **bad}).status_code, 400, bad)

    def test_bad_request_400(self):
        base = "/api/datasets/demo/records/r01/m4"
        for params in [{"vars": "ch01", "t0": 10, "t1": 10, "px": 100},
                       {"vars": "ch01", "t0": 0, "t1": 10, "px": 0},
                       {"vars": ",".join(["ch01"] * 65), "t0": 0, "t1": 10, "px": 100}]:
            self.assertEqual(self.client.get(base, params=params).status_code, 400, params)
        self.assertEqual(self.client.get("/api/datasets/demo/records/a..b/meta").status_code, 400)

    def test_not_found_404(self):
        base = "/api/datasets/demo/records/r01/m4"
        self.assertEqual(self.client.get(base, params={"vars": "zz", "t0": 0, "t1": 10, "px": 10}).status_code, 404)
        self.assertEqual(self.client.get("/api/datasets/demo/records/r09/meta").status_code, 404)
        self.assertEqual(self.client.get("/api/datasets/nope/records/r01/meta").status_code, 404)

    def test_token(self):
        os.environ["SAI_TS_TOKEN"] = "s3cret"
        url = "/api/datasets"
        r = self.client.get(url)
        self.assertEqual(r.status_code, 401)
        self.assertEqual(r.json(), {"detail": "合言葉が違います"})
        self.assertEqual(self.client.get(url, headers={"Authorization": "Bearer nope"}).status_code, 401)
        self.assertEqual(self.client.get(url, headers={"Authorization": "Bearer s3cret"}).status_code, 200)
        self.assertEqual(self.client.get(url, params={"token": "s3cret"}).status_code, 200)
        # 画面の静的ファイルは合言葉が要らない（/api/ でも /ext/ でもない）
        self.assertNotEqual(self.client.get("/index.html").status_code, 401)


if __name__ == "__main__":
    unittest.main()
