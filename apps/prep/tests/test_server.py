"""ローカルの画面と API: 合言葉、設定の検査、仕事の受け付け。"""

from __future__ import annotations

import helpers  # noqa: F401  （src を sys.path に入れる）
import base64
import io
import json
import os
import socket
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
import zipfile
from pathlib import Path
from unittest import mock

from fakes import FakeWebServer
from sai_prep import config as config_mod
from sai_prep.client import Client, ClientError
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

    def submit_paper(self, project, paper_id, pdf_bytes, filename, translations=None):
        self.submitted.append(("paper", project, paper_id, filename, pdf_bytes, translations))
        return FakeJob("paper", {"paper_id": paper_id})

    def submit_bibtex(self, project, bibtex, pdf_bytes, filename, translations=None):
        self.submitted.append(("bibtex", project, bibtex, filename, pdf_bytes, translations))
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

    def _zip(self, files):
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as archive:
            for name, data in files.items():
                archive.writestr(name, data)
        return buffer.getvalue()

    def _post_zip(self, files, query="project=alpha&paper_id=12"):
        return self.request(
            "POST",
            f"/local/jobs?{query}",
            body=self._zip(files),
            headers={"Content-Type": "application/zip"},
        )

    def test_post_zip_with_translation_passes_bytes_through(self):
        status, data = self._post_zip(
            {
                "paper.pdf": b"%PDF-1.4 en",
                "translated/paper-mono.pdf": b"%PDF-1.4 ja",
                "translated/paper-dual.pdf": b"%PDF-1.4 both",
            }
        )
        self.assertEqual(status, 202)
        self.assertEqual(json.loads(data)["target"], {"paper_id": 12})
        kind, project, paper_id, _, pdf_bytes, translations = self.pipeline.submitted[0]
        self.assertEqual((kind, project, paper_id), ("paper", "alpha", 12))
        self.assertEqual(pdf_bytes, b"%PDF-1.4 en")
        self.assertEqual(translations, {"mono": b"%PDF-1.4 ja", "dual": b"%PDF-1.4 both"})

    def test_post_zip_without_translation_sends_empty_translations(self):
        status, _ = self._post_zip({"paper.pdf": b"%PDF-1.4 en", "notes.txt": b"ignored"})
        self.assertEqual(status, 202)
        self.assertEqual(self.pipeline.submitted[0][5], {})

    def test_post_zip_bibtex_target(self):
        bibtex = "@article{x, title={T}}"
        encoded = base64.b64encode(bibtex.encode("utf-8")).decode("ascii")
        status, _ = self.request(
            "POST",
            "/local/jobs?project=alpha&filename=paper.pdf",
            body=self._zip({"paper.pdf": b"%PDF-1.4 en"}),
            headers={"Content-Type": "application/zip", "X-Sai-Bibtex": encoded},
        )
        self.assertEqual(status, 202)
        self.assertEqual(self.pipeline.submitted[0][0], "bibtex")
        self.assertEqual(self.pipeline.submitted[0][5], {})

    def test_post_zip_with_only_mono_is_rejected(self):
        status, data = self._post_zip(
            {"paper.pdf": b"%PDF-1.4 en", "translated/paper-mono.pdf": b"%PDF-1.4 ja"}
        )
        self.assertEqual(status, 400)
        self.assertIn("両方", json.loads(data)["error"])

    def test_post_zip_without_paper_is_rejected(self):
        status, _ = self._post_zip({"translated/paper-mono.pdf": b"%PDF-1.4 ja"})
        self.assertEqual(status, 400)

    def test_post_zip_with_non_pdf_entry_is_rejected(self):
        status, _ = self._post_zip({"paper.pdf": b"%PDF-1.4 en", "translated/paper-mono.pdf": b"x"})
        self.assertEqual(status, 400)
        status, _ = self._post_zip({"paper.pdf": b"hello"})
        self.assertEqual(status, 400)

    def test_post_zip_that_is_not_a_zip_is_rejected(self):
        status, data = self.request(
            "POST",
            "/local/jobs?project=alpha&paper_id=12",
            body=b"%PDF-1.4 fake",
            headers={"Content-Type": "application/zip"},
        )
        self.assertEqual(status, 400)
        self.assertIn("zip", json.loads(data)["error"])

    def test_post_zip_with_corrupt_deflate_data_is_rejected(self):
        # 中身の deflate を壊す（開くのは通り、展開の read で zlib.error が出る経路）
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.writestr("paper.pdf", b"%PDF-1.4 " + b"en " * 500)
        data = bytearray(buffer.getvalue())
        # ローカルのファイルヘッダ（30 バイト＋名前と extra の長さ）の後が圧縮された中身
        start = 30 + int.from_bytes(data[26:28], "little") + int.from_bytes(data[28:30], "little")
        data[start:start + 16] = b"\xff" * 16
        status, body = self.request(
            "POST",
            "/local/jobs?project=alpha&paper_id=12",
            body=bytes(data),
            headers={"Content-Type": "application/zip"},
        )
        self.assertEqual(status, 400)
        self.assertIn("zip", json.loads(body)["error"])

    def test_post_zip_still_needs_nonce(self):
        status, _ = self.request(
            "POST",
            "/local/jobs?project=alpha&paper_id=12",
            body=self._zip({"paper.pdf": b"%PDF-1.4 en"}),
            headers={"Content-Type": "application/zip"},
            nonce=False,
        )
        self.assertEqual(status, 403)


class RecordingClient:
    """`data_files` と `post_data` だけを持つ偽の Client。受けた中身を記録し、必要なら失敗させる。"""

    def __init__(self, fail: ClientError | None = None):
        self.fail = fail
        self.sent: list[tuple] = []

    def data_files(self, project):
        return [{"name": "a.bin", "size": 3, "mtime": "2026-10-09T10:00:00+00:00"}]

    def post_data(self, project, filename, fileobj, size):
        blob = fileobj.read()
        self.sent.append((project, filename, size, blob))
        if self.fail is not None:
            raise self.fail
        return {"name": filename, "size": len(blob), "path": f"data/raw/{filename}"}


class DataTest(unittest.TestCase):
    """Data upload（U01）: 合言葉・中身をそのまま送る・失敗しても一時ファイルを残さない。"""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.home = Path(self.tmp.name) / "home"
        env = mock.patch.dict(
            os.environ,
            {"SAI_PREP_CONFIG": str(Path(self.tmp.name) / "config.json"), "HOME": str(self.home)},
        )
        env.start()
        self.addCleanup(env.stop)

        self.web = FakeWebServer()
        self.addCleanup(self.web.close)
        self.config = config_mod.Config(server_url=self.web.url, token="sai_x")
        self.override: RecordingClient | None = None
        self.httpd = create_server(
            lambda: self.config,
            StubPipeline(),
            port=0,
            client_factory=lambda url, token: self.override or Client(url, token),
        )
        self.addCleanup(self.httpd.server_close)
        self.port = self.httpd.server_address[1]
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.httpd.shutdown)

    def upload_dir_files(self):
        folder = self.home / "Library" / "Caches" / "sai-prep" / "uploads"
        return list(folder.iterdir()) if folder.exists() else []

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

    def test_data_needs_the_nonce(self):
        status, _ = self.request("GET", "/local/data?project=alpha", nonce=False)
        self.assertEqual(status, 403)
        status, _ = self.request(
            "POST", "/local/data?project=alpha&filename=a.bin", body=b"x", nonce=False
        )
        self.assertEqual(status, 403)

    def test_data_passes_the_bytes_and_leaves_no_temp_file(self):
        body = bytes(range(256)) * 100
        status, raw = self.request(
            "POST",
            "/local/data?project=alpha&filename=a.bin",
            body=body,
            headers={"Content-Type": "application/octet-stream"},
        )
        self.assertEqual(status, 201, raw)
        self.assertEqual(json.loads(raw)["size"], len(body))
        sent = self.web.data_posts[0]
        self.assertEqual((sent["project"], sent["filename"]), ("alpha", "a.bin"))
        self.assertEqual(sent["authorization"], "Bearer sai_x")
        self.assertEqual(sent["content_length"], len(body))
        self.assertEqual(sent["blob"], body)
        self.assertEqual(self.upload_dir_files(), [])

    def test_data_list_uses_the_server(self):
        self.request(
            "POST",
            "/local/data?project=alpha&filename=b.csv",
            body=b"1,2\n",
            headers={"Content-Type": "application/octet-stream"},
        )
        status, raw = self.request("GET", "/local/data?project=alpha")
        self.assertEqual(status, 200)
        names = [f["name"] for f in json.loads(raw)["files"]]
        self.assertEqual(names, ["b.csv"])

    def test_same_name_is_409_and_the_temp_file_is_removed(self):
        self.request("POST", "/local/data?project=alpha&filename=c.bin", body=b"first")
        status, raw = self.request("POST", "/local/data?project=alpha&filename=c.bin", body=b"second")
        self.assertEqual(status, 409)
        self.assertIn("同じ名前", json.loads(raw)["error"])
        self.assertEqual(self.upload_dir_files(), [])

    def test_bad_name_or_missing_project_is_400_and_not_sent(self):
        for query in ("project=alpha&filename=../x", "project=alpha&filename=.hidden", "project=&filename=a.bin"):
            status, _ = self.request("POST", f"/local/data?{query}", body=b"x")
            self.assertEqual(status, 400, query)
        self.assertEqual(self.web.data_posts, [])

    def test_contents_go_to_the_client_and_failures_clean_up(self):
        self.override = RecordingClient()
        status, _ = self.request("POST", "/local/data?project=alpha&filename=b.bin", body=b"abc")
        self.assertEqual(status, 201)
        self.assertEqual(self.override.sent, [("alpha", "b.bin", 3, b"abc")])
        self.assertEqual(self.upload_dir_files(), [])

        self.override = RecordingClient(fail=ClientError("413 です", status=413))
        status, _ = self.request("POST", "/local/data?project=alpha&filename=big.bin", body=b"abc")
        self.assertEqual(status, 413)
        self.assertEqual(self.upload_dir_files(), [])

        self.override = RecordingClient(fail=ClientError("つながりません"))
        status, _ = self.request("POST", "/local/data?project=alpha&filename=x.bin", body=b"abc")
        self.assertEqual(status, 400)
        self.assertEqual(self.upload_dir_files(), [])

    def test_list_through_the_client(self):
        self.override = RecordingClient()
        status, raw = self.request("GET", "/local/data?project=alpha")
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(raw)["files"][0]["name"], "a.bin")

    def test_truncated_body_is_400_and_not_forwarded(self):
        # Content-Length 100 と言って 10 バイトだけ送り、書き込みを閉じる（EOF）
        self.override = RecordingClient()
        head = (
            "POST /local/data?project=alpha&filename=t.bin HTTP/1.1\r\n"
            f"Host: 127.0.0.1\r\n{NONCE_HEADER}: {self.httpd.nonce}\r\n"
            "Content-Type: application/octet-stream\r\nContent-Length: 100\r\n\r\n"
        )
        with socket.create_connection(("127.0.0.1", self.port), timeout=10) as sock:
            sock.sendall(head.encode("ascii") + b"x" * 10)
            sock.shutdown(socket.SHUT_WR)
            reply = b""
            while True:
                chunk = sock.recv(4096)
                if not chunk:
                    break
                reply += chunk
        self.assertTrue(reply.startswith(b"HTTP/1.1 400 "), reply[:40])
        self.assertEqual(self.override.sent, [])
        self.assertEqual(self.upload_dir_files(), [])


if __name__ == "__main__":
    unittest.main()
