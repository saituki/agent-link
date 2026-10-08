"""ローカルの画面と API: 合言葉、設定の検査、仕事の受け付け。"""

from __future__ import annotations

import helpers  # noqa: F401  （src を sys.path に入れる）
import base64
import json
import os
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from pathlib import Path
from unittest import mock

from fakes import FakeWebServer
from sai_prep import config as config_mod
from sai_prep.client import Client
from sai_prep.server import NONCE_HEADER, create_server


class FakeJob:
    def __init__(self, kind, target):
        self.kind = kind
        self.target = target

    def to_public(self):
        return {"id": "j1", "kind": self.kind, "target": self.target, "status": "queued"}


class StubPipeline:
    def __init__(self):
        self.submitted = []

    def submit_paper(self, project, paper_id, pdf_bytes, filename):
        self.submitted.append(("paper", project, paper_id, filename, pdf_bytes))
        return FakeJob("paper", {"paper_id": paper_id})

    def submit_bibtex(self, project, bibtex, pdf_bytes, filename):
        self.submitted.append(("bibtex", project, bibtex, filename, pdf_bytes))
        return FakeJob("bibtex", {"bibtex": bibtex})

    def list_jobs(self):
        return []

    def retry(self, job_id):
        return FakeJob("paper", {"paper_id": 12}) if job_id == "failed1" else None


class ServerTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.config_path = self.root / "config.json"
        env = mock.patch.dict(os.environ, {"SAI_PREP_CONFIG": str(self.config_path)})
        env.start()
        self.addCleanup(env.stop)

        self.web = FakeWebServer()
        self.addCleanup(self.web.close)
        self.config = config_mod.Config(server_url=self.web.url, token="sai_x")
        self.pipeline = StubPipeline()
        self.httpd = create_server(
            lambda: self.config, self.pipeline, port=0, client_factory=Client
        )
        self.addCleanup(self.httpd.server_close)
        self.port = self.httpd.server_address[1]
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.httpd.shutdown)

    def request(self, method, path, body=None, headers=None, nonce=True):
        all_headers = dict(headers or {})
        if nonce:
            all_headers[NONCE_HEADER] = self.httpd.nonce
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.port}{path}", data=body, headers=all_headers, method=method
        )
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                return response.status, response.read()
        except urllib.error.HTTPError as error:
            body = error.read()
            error.close()
            return error.code, body

    def json_request(self, method, path, payload=None, headers=None, nonce=True):
        body = json.dumps(payload).encode("utf-8") if payload is not None else None
        all_headers = {"Content-Type": "application/json"}
        all_headers.update(headers or {})
        status, raw = self.request(method, path, body=body, headers=all_headers, nonce=nonce)
        return status, json.loads(raw) if raw else {}

    def test_index_serves_the_nonce(self):
        status, raw = self.request("GET", "/", nonce=False)
        self.assertEqual(status, 200)
        self.assertIn(self.httpd.nonce, raw.decode("utf-8"))
        self.assertNotIn("__SAI_PREP_NONCE__", raw.decode("utf-8"))

    def test_api_without_nonce_is_forbidden(self):
        status, _ = self.request("GET", "/local/config", nonce=False)
        self.assertEqual(status, 403)

    def test_config_roundtrip(self):
        status, data = self.json_request(
            "PUT",
            "/local/config",
            {"server_url": "https://sai.example.ts.net/", "token": "sai_new", "translate": False},
        )
        self.assertEqual(status, 200)
        self.assertEqual(data["server_url"], "https://sai.example.ts.net")
        self.assertFalse(data["translate"])
        saved = json.loads(self.config_path.read_text(encoding="utf-8"))
        self.assertEqual(saved["token"], "sai_new")

    def test_config_rejects_plain_http_other_host(self):
        status, data = self.json_request("PUT", "/local/config", {"server_url": "http://example.com"})
        self.assertEqual(status, 400)
        self.assertIn("https", data["error"])

    def test_projects_and_pending_use_the_server(self):
        status, data = self.json_request("GET", "/local/projects")
        self.assertEqual(status, 200)
        self.assertEqual([p["name"] for p in data["projects"]], ["alpha", "beta"])

        status, data = self.json_request("GET", "/local/pending?project=alpha")
        self.assertEqual(status, 200)
        self.assertEqual(data["papers"][0]["id"], 12)

    def test_check_reports_the_name(self):
        status, data = self.json_request("POST", "/local/check", {})
        self.assertEqual(status, 200)
        self.assertEqual(data["name"], "tester")

    def test_post_paper_job(self):
        pdf = b"%PDF-1.4 fake"
        status, data = self.request(
            "POST",
            "/local/jobs?project=alpha&paper_id=12&filename=paper.pdf",
            body=pdf,
            headers={"Content-Type": "application/pdf"},
        )
        self.assertEqual(status, 202)
        data = json.loads(data)
        self.assertEqual(data["target"], {"paper_id": 12})
        self.assertEqual(self.pipeline.submitted[0][0], "paper")
        self.assertEqual(self.pipeline.submitted[0][2], 12)

    def test_retry_failed_job(self):
        status, data = self.request("POST", "/local/jobs/failed1/retry")
        self.assertEqual(status, 202)
        self.assertEqual(json.loads(data)["target"], {"paper_id": 12})
        status, _ = self.request("POST", "/local/jobs/nothing/retry")
        self.assertEqual(status, 404)

    def test_post_bibtex_job(self):
        bibtex = "@article{x, title={T}}"
        encoded = base64.b64encode(bibtex.encode("utf-8")).decode("ascii")
        status, data = self.request(
            "POST",
            "/local/jobs?project=alpha&filename=paper.pdf",
            body=b"%PDF-1.4 fake",
            headers={"Content-Type": "application/pdf", "X-Sai-Bibtex": encoded},
        )
        self.assertEqual(status, 202)
        data = json.loads(data)
        self.assertEqual(data["target"], {"bibtex": bibtex})
        self.assertEqual(self.pipeline.submitted[0][0], "bibtex")

    def test_post_job_rejects_wrong_content_type(self):
        status, _ = self.request(
            "POST",
            "/local/jobs?project=alpha&paper_id=12",
            body=b"%PDF-1.4 fake",
            headers={"Content-Type": "application/json"},
        )
        self.assertEqual(status, 415)

    def test_post_job_requires_a_target(self):
        status, data = self.request(
            "POST",
            "/local/jobs?project=alpha",
            body=b"%PDF-1.4 fake",
            headers={"Content-Type": "application/pdf"},
        )
        self.assertEqual(status, 400)
        self.assertIn("paper_id", json.loads(data)["error"])

    def test_post_job_rejects_non_pdf(self):
        status, _ = self.request(
            "POST",
            "/local/jobs?project=alpha&paper_id=12",
            body=b"not a pdf",
            headers={"Content-Type": "application/pdf"},
        )
        self.assertEqual(status, 400)


if __name__ == "__main__":
    unittest.main()
