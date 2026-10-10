"""DB Migration: 移植元（sai-moon）の一覧、移植の束の中身、読み取り専用、ローカルの API と仕事。"""

from __future__ import annotations

import helpers  # noqa: F401  （src を sys.path に入れる）
import hashlib
import io
import json
import os
import sqlite3
import tempfile
import threading
import unittest
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path
from unittest import mock

from fakes import FakeWebServer
from sai_prep import config as config_mod
from sai_prep import migrate
from sai_prep.client import Client
from sai_prep.pipeline import Pipeline
from sai_prep.server import NONCE_HEADER, create_server


def make_moon(root: Path) -> Path:
    """偽の sai-moon のプロジェクトを作る。論文 3 本（1 は フォルダあり・2 はフォルダ無し・3 は別の論文）。"""
    db_dir = root / ".sai" / "paper"
    db_dir.mkdir(parents=True)
    db = db_dir / "papr.db"
    con = sqlite3.connect(db)
    con.executescript(
        """
        CREATE TABLE papers (id INTEGER PRIMARY KEY, title TEXT NOT NULL, published_at TEXT, created_at TEXT NOT NULL);
        CREATE TABLE authors (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
        CREATE TABLE paper_authors (paper_id INTEGER NOT NULL, author_id INTEGER NOT NULL, position INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (paper_id, author_id));
        CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
        CREATE TABLE paper_tags (paper_id INTEGER NOT NULL, tag_id INTEGER NOT NULL, PRIMARY KEY (paper_id, tag_id));
        CREATE TABLE chunks (id INTEGER PRIMARY KEY, paper_id INTEGER, ord INTEGER NOT NULL, text TEXT NOT NULL);
        CREATE TABLE embedding_models (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
        CREATE TABLE chunk_embeddings (chunk_id INTEGER NOT NULL, model_id INTEGER NOT NULL, vec BLOB NOT NULL, PRIMARY KEY (chunk_id, model_id));
        CREATE TABLE paper_extracts (paper_id TEXT PRIMARY KEY, papers_id INTEGER, status TEXT NOT NULL);
        CREATE TABLE paper_sections (paper_id INTEGER NOT NULL, sec_no INTEGER NOT NULL, heading TEXT NOT NULL, PRIMARY KEY (paper_id, sec_no));
        """
    )
    con.executemany(
        "INSERT INTO papers (id, title, published_at, created_at) VALUES (?, ?, ?, ?)",
        [
            (1, "Alpha paper", "1993-05-01", "2026-01-01"),
            (2, "Beta paper", None, "2026-03-01"),
            (3, "Gamma paper", "n.d.", "2026-02-01"),
        ],
    )
    con.executemany("INSERT INTO authors (id, name) VALUES (?, ?)", [(1, "Ada"), (2, "Bob"), (3, "Cy"), (4, "Dee")])
    con.executemany(
        "INSERT INTO paper_authors (paper_id, author_id, position) VALUES (?, ?, ?)",
        [(1, 1, 0), (1, 2, 1), (2, 3, 0), (3, 4, 0)],
    )
    con.executemany("INSERT INTO tags (id, name) VALUES (?, ?)", [(1, "sleep"), (2, "other")])
    con.executemany("INSERT INTO paper_tags (paper_id, tag_id) VALUES (?, ?)", [(1, 1), (3, 2)])
    con.executemany(
        "INSERT INTO chunks (id, paper_id, ord, text) VALUES (?, ?, ?, ?)",
        [(1, 1, 0, "a"), (2, 1, 1, "b"), (3, 2, 0, "c"), (4, 3, 0, "d")],
    )
    con.executemany("INSERT INTO embedding_models (id, name) VALUES (?, ?)", [(1, "m1"), (2, "m2")])
    con.executemany(
        "INSERT INTO chunk_embeddings (chunk_id, model_id, vec) VALUES (?, ?, ?)",
        [(1, 1, b"\x00"), (2, 1, b"\x00"), (3, 1, b"\x00"), (4, 1, b"\x00")],
    )
    con.executemany(
        "INSERT INTO paper_extracts (paper_id, papers_id, status) VALUES (?, ?, ?)",
        [("S2A", 1, "ready"), ("S2C", 3, "ready")],
    )
    con.executemany(
        "INSERT INTO paper_sections (paper_id, sec_no, heading) VALUES (?, ?, ?)",
        [(1, 1, "Intro"), (3, 1, "Other")],
    )
    con.commit()
    con.close()

    papers = root / ".sai" / "paper" / "papers"
    alpha = papers / "0001_Alpha"
    alpha.mkdir(parents=True)
    (alpha / "paper.pdf").write_bytes(b"%PDF-1.4 alpha")
    (alpha / "extract.md").write_text("# Alpha\n", encoding="utf-8")
    (alpha / "paper-ja.pdf").write_bytes(b"%PDF-1.4 alpha ja")
    (alpha / "link.pdf").symlink_to(alpha / "paper.pdf")
    gamma = papers / "0003_Gamma"
    gamma.mkdir(parents=True)
    (gamma / "paper.pdf").write_bytes(b"%PDF-1.4 gamma")
    return root


def read_db_bytes(path: Path) -> bytes:
    return path.read_bytes()


class MigratePackageTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = make_moon(Path(self.tmp.name) / "west")
        self.db = self.root / ".sai" / "paper" / "papr.db"
        self.out = Path(self.tmp.name) / "out"
        self.out.mkdir()

    def test_list_papers_newest_first_with_flags(self):
        papers = migrate.list_papers(self.root)
        self.assertEqual([p["id"] for p in papers], [2, 3, 1])
        alpha = papers[2]
        self.assertEqual(
            alpha,
            {
                "id": 1,
                "title": "Alpha paper",
                "year": 1993,
                "authors": "Ada, Bob",
                "has_pdf": True,
                "has_extract": True,
                "translated": True,
                "chunks": 2,
            },
        )
        beta = papers[0]
        self.assertEqual(
            (beta["year"], beta["authors"], beta["has_pdf"], beta["has_extract"], beta["translated"], beta["chunks"]),
            (None, "Cy", False, False, False, 1),
        )
        self.assertIsNone(papers[1]["year"])
        self.assertTrue(papers[1]["has_pdf"])

    def test_find_paper_dir_matches_sai_tree_rule(self):
        papers_dir = self.root / ".sai" / "paper" / "papers"
        self.assertEqual(migrate.find_paper_dir(papers_dir, 1).name, "0001_Alpha")
        self.assertEqual(migrate.find_paper_dir(papers_dir, 3).name, "0003_Gamma")
        self.assertIsNone(migrate.find_paper_dir(papers_dir, 2))

    def test_build_package_contents(self):
        out = self.out / "m.zip"
        migrate.build_package(self.root, [1, 2], out)
        with zipfile.ZipFile(out) as archive:
            names = set(archive.namelist())
            self.assertIn("manifest.json", names)
            self.assertIn("papr.db", names)
            self.assertIn("papers/0001_Alpha/paper.pdf", names)
            self.assertIn("papers/0001_Alpha/extract.md", names)
            self.assertIn("papers/0001_Alpha/paper-ja.pdf", names)
            self.assertFalse(any(n.startswith("papers/0003_Gamma/") for n in names))
            self.assertNotIn("papers/0001_Alpha/link.pdf", names)
            manifest = json.loads(archive.read("manifest.json"))
            self.assertEqual(
                manifest,
                {"format": "sai-paper-migration", "version": 1, "papers": [1, 2]},
            )
            packed = self.out / "packed.db"
            packed.write_bytes(archive.read("papr.db"))

        con = sqlite3.connect(packed)
        try:
            self.assertEqual(con.execute("SELECT id FROM papers ORDER BY id").fetchall(), [(1,), (2,)])
            self.assertEqual(con.execute("SELECT id FROM chunks ORDER BY id").fetchall(), [(1,), (2,), (3,)])
            self.assertEqual(con.execute("SELECT COUNT(*) FROM chunk_embeddings").fetchone(), (3,))
            self.assertEqual(con.execute("SELECT COUNT(*) FROM embedding_models").fetchone(), (2,))
            self.assertEqual(
                con.execute("SELECT name FROM authors ORDER BY id").fetchall(), [("Ada",), ("Bob",), ("Cy",)]
            )
            self.assertEqual(con.execute("SELECT id FROM tags").fetchall(), [(1,)])
            self.assertEqual(con.execute("SELECT paper_id FROM paper_sections").fetchall(), [(1,)])
            self.assertEqual(con.execute("SELECT paper_id FROM paper_extracts").fetchall(), [("S2A",)])
        finally:
            con.close()

    def test_build_package_skips_ids_not_in_papers(self):
        out = self.out / "m.zip"
        migrate.build_package(self.root, [1, 99], out)
        with zipfile.ZipFile(out) as archive:
            manifest = json.loads(archive.read("manifest.json"))
        self.assertEqual(manifest["papers"], [1])

    def test_build_package_without_known_ids_is_error(self):
        with self.assertRaises(migrate.MigrateError):
            migrate.build_package(self.root, [99], self.out / "m.zip")

    def test_missing_db_is_error(self):
        empty = Path(self.tmp.name) / "not-moon"
        empty.mkdir()
        with self.assertRaises(migrate.MigrateError):
            migrate.list_papers(empty)
        with self.assertRaises(migrate.MigrateError):
            migrate.build_package(empty, [1], self.out / "m.zip")

    def test_source_db_is_not_changed(self):
        before_bytes = read_db_bytes(self.db)
        before_mtime = os.stat(self.db).st_mtime_ns
        migrate.list_papers(self.root)
        migrate.build_package(self.root, [1, 2], self.out / "m.zip")
        self.assertEqual(hashlib.sha256(read_db_bytes(self.db)).hexdigest(), hashlib.sha256(before_bytes).hexdigest())
        self.assertEqual(os.stat(self.db).st_mtime_ns, before_mtime)


class MigrateApiTest(unittest.TestCase):
    """ローカルの API と、仕事（sai-prep の列）が移植を終えるところまで。偽のサーバを相手にする。"""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = make_moon(Path(self.tmp.name) / "west")
        self.config_path = Path(self.tmp.name) / "config.json"
        env = mock.patch.dict(os.environ, {"SAI_PREP_CONFIG": str(self.config_path)})
        env.start()
        self.addCleanup(env.stop)

        self.web = FakeWebServer()
        self.addCleanup(self.web.close)
        self.config = config_mod.Config(server_url=self.web.url, token="sai_x", project="beta")
        self.pipeline = Pipeline(
            lambda: self.config,
            work_root=Path(self.tmp.name) / "jobs",
            poll_interval=0.05,
        )
        self.addCleanup(self.pipeline.close)
        self.httpd = create_server(lambda: self.config, self.pipeline, port=0, client_factory=Client)
        self.addCleanup(self.httpd.server_close)
        self.port = self.httpd.server_address[1]
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.httpd.shutdown)

    def call(self, method, path, body=None, nonce=True):
        headers = {}
        if nonce:
            headers[NONCE_HEADER] = self.httpd.nonce
        data = None
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.port}{path}", data=data, headers=headers, method=method
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return response.status, json.loads(response.read() or b"{}")
        except urllib.error.HTTPError as error:
            with error:
                return error.code, json.loads(error.read() or b"{}")

    def moon_query(self, path=None):
        return f"/local/migrate/papers?source=moon&path={urllib.parse.quote(str(path or self.root))}"

    # -- 一覧 --------------------------------------------------------------

    def test_list_moon(self):
        status, data = self.call("GET", self.moon_query())
        self.assertEqual(status, 200)
        self.assertEqual([p["id"] for p in data["papers"]], [2, 3, 1])
        self.assertEqual(data["papers"][2]["chunks"], 2)

    def test_list_moon_errors(self):
        self.assertEqual(self.call("GET", "/local/migrate/papers?source=moon")[0], 400)
        not_moon = Path(self.tmp.name) / "plain"
        not_moon.mkdir()
        status, data = self.call("GET", self.moon_query(not_moon))
        self.assertEqual(status, 400)
        self.assertIn(".sai/paper/papr.db", data["error"])

    def test_list_tree_shapes_library_rows(self):
        status, data = self.call("GET", "/local/migrate/papers?source=tree&project=alpha")
        self.assertEqual(status, 200)
        self.assertEqual(
            data["papers"],
            [
                {
                    "id": 5,
                    "title": "Lib Paper",
                    "year": 2023,
                    "authors": "Ada, Bob",
                    "has_pdf": True,
                    "has_extract": False,
                    "translated": None,
                    "chunks": None,
                }
            ],
        )

    def test_list_rejects_bad_source_and_missing_nonce(self):
        self.assertEqual(self.call("GET", "/local/migrate/papers?source=xx&project=a")[0], 400)
        self.assertEqual(self.call("GET", "/local/migrate/papers?source=tree")[0], 400)
        self.assertEqual(self.call("GET", "/local/migrate/papers?source=tree&project=alpha", nonce=False)[0], 403)

    # -- 仕事 --------------------------------------------------------------

    def wait_done(self, jobs):
        done = []
        for job in jobs:
            finished = self.pipeline.wait(job["id"], timeout=60)
            self.assertEqual(finished.status, "done", finished.error)
            done.append(finished.to_public())
        return done

    def test_post_moon_sends_one_bundle_per_paper(self):
        status, data = self.call(
            "POST",
            "/local/migrate?project=beta",
            {"source": "moon", "path": str(self.root), "ids": [1, 3]},
        )
        self.assertEqual(status, 202)
        self.assertEqual([j["kind"] for j in data["jobs"]], ["migrate", "migrate"])
        self.assertEqual(data["jobs"][0]["filename"], "Alpha paper")
        done = self.wait_done(data["jobs"])
        self.assertEqual(done[0]["result"], self.web.migrate_result)
        self.assertEqual(len(self.web.migrations), 2)
        self.assertTrue(all(m["kind"] == "zip" and m["project"] == "beta" for m in self.web.migrations))
        with zipfile.ZipFile(io.BytesIO(self.web.migrations[0]["blob"])) as archive:
            self.assertEqual(json.loads(archive.read("manifest.json"))["papers"], [1])
        with zipfile.ZipFile(io.BytesIO(self.web.migrations[1]["blob"])) as archive:
            self.assertEqual(json.loads(archive.read("manifest.json"))["papers"], [3])
            self.assertIn("papers/0003_Gamma/paper.pdf", archive.namelist())

    def test_post_tree_is_one_job(self):
        status, data = self.call(
            "POST",
            "/local/migrate?project=beta",
            {"source": "tree", "project": "alpha", "ids": [5, 6]},
        )
        self.assertEqual(status, 202)
        self.assertEqual(len(data["jobs"]), 1)
        self.assertEqual(data["jobs"][0]["filename"], "alpha から 2 件")
        self.wait_done(data["jobs"])
        self.assertEqual(
            self.web.migrations,
            [{"kind": "from", "project": "beta", "body": {"source": "alpha", "ids": [5, 6]}}],
        )

    def test_post_rejects_bad_requests(self):
        cases = [
            ("/local/migrate", {"source": "moon", "path": str(self.root), "ids": [1]}),  # project が無い
            ("/local/migrate?project=beta", {"source": "moon", "path": str(self.root), "ids": []}),
            ("/local/migrate?project=beta", {"source": "moon", "path": str(self.root), "ids": [True]}),
            ("/local/migrate?project=beta", {"source": "moon", "path": str(self.root), "ids": ["1"]}),
            ("/local/migrate?project=beta", {"source": "moon", "path": str(self.root), "ids": list(range(1, 1002))}),
            ("/local/migrate?project=beta", {"source": "moon", "path": str(self.root), "ids": [99]}),
            ("/local/migrate?project=beta", {"source": "moon", "path": "", "ids": [1]}),
            ("/local/migrate?project=beta", {"source": "moon", "path": str(Path(self.tmp.name) / "plain"), "ids": [1]}),
            ("/local/migrate?project=beta", {"source": "tree", "project": "beta", "ids": [5]}),
            ("/local/migrate?project=beta", {"source": "tree", "project": "", "ids": [5]}),
            ("/local/migrate?project=beta", {"source": "other", "ids": [5]}),
        ]
        for path, body in cases:
            with self.subTest(body=body, path=path):
                self.assertEqual(self.call("POST", path, body)[0], 400)
        self.assertEqual(self.web.migrations, [])


if __name__ == "__main__":
    unittest.main()
