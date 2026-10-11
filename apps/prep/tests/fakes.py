"""テストで使う偽の道具と偽のサーバ（C14 の住所に答える）。"""

from __future__ import annotations

import json
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

DOCLING_SCRIPT = """#!/usr/bin/env python3
import pathlib
import sys

args = sys.argv[1:]
if "--version" in args:
    print("docling 2.134.0")
    sys.exit(0)

out = "."
for i, a in enumerate(args):
    if a == "--output":
        out = args[i + 1]
pdf = args[1] if len(args) > 1 else "paper.pdf"
stem = pathlib.Path(pdf).stem
outdir = pathlib.Path(out)
outdir.mkdir(parents=True, exist_ok=True)
(outdir / (stem + ".md")).write_text("# fake docling\\n", encoding="utf-8")
(outdir / (stem + ".json")).write_text("{}\\n", encoding="utf-8")
art = outdir / (stem + "_artifacts")
art.mkdir(exist_ok=True)
(art / "fig1.png").write_bytes(b"\\x89PNG\\r\\n")
print("converted")
"""

PDF2ZH_SCRIPT = """#!/usr/bin/env python3
import json
import os
import pathlib
import sys

args = sys.argv[1:]
if "--version" in args:
    print("pdf2zh 1.9.11")
    sys.exit(0)

out = "."
for i, a in enumerate(args):
    if a == "-o":
        out = args[i + 1]
stem = pathlib.Path(args[0]).stem
outdir = pathlib.Path(out)
if not outdir.is_dir():
    # 本物の pdf2zh は出力フォルダを自分で作らない。
    print("output dir missing: " + str(outdir), file=sys.stderr)
    sys.exit(2)
(outdir / (stem + "-mono.pdf")).write_bytes(b"%PDF-1.4 mono")
(outdir / (stem + "-dual.pdf")).write_bytes(b"%PDF-1.4 dual")
log = os.environ.get("FAKE_PDF2ZH_LOG")
if log:
    with open(log, "a", encoding="utf-8") as f:
        f.write(json.dumps({
            "base_url": os.environ.get("OPENAILIKED_BASE_URL"),
            "model": os.environ.get("OPENAILIKED_MODEL"),
            "args": args,
        }) + "\\n")
print("translated")
"""

MLX_SCRIPT = """#!/usr/bin/env python3
import json
import os
import signal
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

args = sys.argv[1:]
if "--help" in args or "-h" in args:
    print("usage: mlx_lm.server --model <repo> --host <host> --port <port>")
    sys.exit(0)
port = 8080
for i, a in enumerate(args):
    if a == "--port":
        port = int(args[i + 1])

log = os.environ.get("FAKE_MLX_LOG")


def note(event):
    if log:
        with open(log, "a", encoding="utf-8") as f:
            f.write(event + "\\n")


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/v1/models":
            body = json.dumps({"object": "list", "data": []}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, *a):
        pass


def stop(signum, frame):
    note("stop")
    sys.exit(0)


signal.signal(signal.SIGTERM, stop)
note("start")
HTTPServer(("127.0.0.1", port), Handler).serve_forever()
"""


def write_fake_tools(bindir: Path) -> Path:
    """偽の docling・pdf2zh・mlx_lm.server を置き、そのフォルダを返す。"""
    bindir.mkdir(parents=True, exist_ok=True)
    for name, script in (
        ("docling", DOCLING_SCRIPT),
        ("pdf2zh", PDF2ZH_SCRIPT),
        ("mlx_lm.server", MLX_SCRIPT),
    ):
        path = bindir / name
        path.write_text(script, encoding="utf-8")
        path.chmod(0o755)
    return bindir


class FakeWebServer:
    """C14 の住所に答える偽のサーバ。送られた束と Authorization を記録する。"""

    def __init__(self, job_polls_before_done: int = 0):
        self.bundles: list[dict] = []
        self.jobs: dict[str, dict] = {}
        self.pending = {
            "papers": [
                {
                    "id": 12,
                    "title": "A Fake Paper",
                    "year": 2024,
                    "authors": ["Ada"],
                    "doi": "10.1/fake",
                    "arxiv_id": "",
                    "s2_paper_id": "",
                    "created_at": "2026-01-01T00:00:00+09:00",
                }
            ]
        }
        self.me = {"mode": "multi", "name": "tester", "email": "t@example.com"}
        self.projects = [{"name": "alpha"}, {"name": "beta"}]
        # Data upload（U01）: 置いたファイル（名前 → 中身）と、受けた要求の記録
        self.data_files: dict[str, bytes] = {}
        self.data_posts: list[dict] = []
        # DB Migration: sai-tree の Library の一覧、受けた移植の要求、取り込みの答え
        self.library = [
            {"id": 5, "title": "Lib Paper", "year": 2023, "authors": "Ada, Bob",
             "has_pdf": True, "has_extract": False},
        ]
        self.migrations: list[dict] = []
        self.migrate_result = {
            "imported": [{"source_id": 1, "paper_id": 12, "title": "A Fake Paper", "mode": "new"}],
            "skipped": [{"source_id": 2, "paper_id": 13, "title": "Skipped Paper", "reason": "PDF があるので飛ばしました"}],
            "failed": [{"source_id": 3, "title": "Broken Paper", "error": "zip が壊れています"}],
        }
        self._polls: dict[str, int] = {}
        self._job_polls_before_done = job_polls_before_done
        state = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *a):
                pass

            def _send(self, status, payload):
                body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                path = self.path.split("?")[0]
                if path == "/api/auth/me":
                    self._send(200, state.me)
                elif path == "/api/projects":
                    self._send(200, {"projects": state.projects})
                elif path.endswith("/papers/pending"):
                    self._send(200, state.pending)
                elif path.endswith("/papers/library"):
                    self._send(200, {"papers": state.library})
                elif path.endswith("/data/raw"):
                    files = [
                        {"name": name, "size": len(blob), "mtime": "2026-10-09T10:00:00+00:00"}
                        for name, blob in sorted(state.data_files.items())
                    ]
                    self._send(200, {"files": files})
                elif "/papers/jobs/" in path:
                    key = urllib.parse.unquote(path.rsplit("/", 1)[-1])
                    count = state._polls.get(key, 0)
                    state._polls[key] = count + 1
                    if count >= state._job_polls_before_done:
                        job = state.jobs.get(key, {"status": "done", "title": "A Fake Paper", "paper_id": 12})
                        job = dict(job)
                        job["status"] = "done"
                    else:
                        job = {"key": key, "status": "processing"}
                    self._send(200, job)
                else:
                    self._send(404, {"error": "not found"})

            def do_POST(self):
                path = self.path.split("?")[0]
                if path.endswith("/papers/bundle"):
                    length = int(self.headers.get("Content-Length", "0"))
                    blob = self.rfile.read(length) if length else b""
                    project = path.split("/api/projects/")[1].split("/")[0]
                    state.bundles.append(
                        {
                            "project": project,
                            "authorization": self.headers.get("Authorization", ""),
                            "content_type": self.headers.get("Content-Type", ""),
                            "blob": blob,
                        }
                    )
                    key = f"bundle:fake-{len(state.bundles)}"
                    state.jobs[key] = {"key": key, "status": "done", "title": "A Fake Paper", "paper_id": 12}
                    self._send(202, {"key": key, "status": "processing"})
                elif path.endswith("/papers/migrate") or path.endswith("/papers/migrate-from"):
                    length = int(self.headers.get("Content-Length", "0"))
                    blob = self.rfile.read(length) if length else b""
                    project = path.split("/api/projects/")[1].split("/")[0]
                    if path.endswith("/papers/migrate"):
                        record = {"kind": "zip", "project": project, "blob": blob}
                    else:
                        record = {"kind": "from", "project": project, "body": json.loads(blob or b"{}")}
                    state.migrations.append(record)
                    key = f"migrate:fake-{len(state.migrations)}"
                    state.jobs[key] = {
                        "key": key, "status": "done", "title": "DB Migration",
                        "result": state.migrate_result,
                    }
                    self._send(202, {"key": key, "status": "processing"})
                elif path.endswith("/data/raw"):
                    query = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query)
                    filename = (query.get("filename") or [""])[0]
                    length = int(self.headers.get("Content-Length", "-1"))
                    blob = self.rfile.read(length) if length > 0 else b""
                    state.data_posts.append(
                        {
                            "project": path.split("/api/projects/")[1].split("/")[0],
                            "filename": filename,
                            "authorization": self.headers.get("Authorization", ""),
                            "content_type": self.headers.get("Content-Type", ""),
                            "content_length": length,
                            "blob": blob,
                        }
                    )
                    if filename in state.data_files:
                        self._send(409, {"detail": "同じ名前のファイルがあります（上書きしません）"})
                        return
                    state.data_files[filename] = blob
                    self._send(201, {"name": filename, "size": len(blob), "path": f"data/raw/{filename}"})
                else:
                    self._send(404, {"error": "not found"})

        self._httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self._httpd.daemon_threads = True
        self.port = self._httpd.server_address[1]
        self._thread = threading.Thread(target=self._httpd.serve_forever, daemon=True)
        self._thread.start()

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def close(self) -> None:
        self._httpd.shutdown()
        self._httpd.server_close()
