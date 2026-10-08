"""前処理の列: 束の形、翻訳の有無、mlx の起止、道具が無いときの失敗。"""

from __future__ import annotations

import helpers  # noqa: F401  （src を sys.path に入れる）
import hashlib
import io
import json
import os
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

from fakes import FakeWebServer, write_fake_tools
from sai_prep import config as config_mod
from sai_prep.pipeline import Pipeline

PDF_BYTES = b"%PDF-1.4\n% fake pdf\n"


class PipelineTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bindir = write_fake_tools(self.root / "bin")
        self.jobs = self.root / "jobs"
        self.server = FakeWebServer()
        self.addCleanup(self.server.close)

        self.mlx_log = self.root / "mlx.log"
        self.pdf2zh_log = self.root / "pdf2zh.log"
        env = mock.patch.dict(
            os.environ,
            {"FAKE_MLX_LOG": str(self.mlx_log), "FAKE_PDF2ZH_LOG": str(self.pdf2zh_log)},
        )
        env.start()
        self.addCleanup(env.stop)

        self.config = config_mod.Config(
            server_url=self.server.url,
            token="sai_test",
            project="alpha",
            translate=True,
            mlx_port=helpers.free_port(),
        )
        self.pipeline = Pipeline(
            lambda: self.config,
            work_root=self.jobs,
            extra_tool_dirs=[str(self.bindir)],
            poll_interval=0.05,
        )
        self.addCleanup(self.pipeline.close)

    def _run_paper(self):
        job = self.pipeline.submit_paper("alpha", 12, PDF_BYTES, "paper.pdf")
        done = self.pipeline.wait(job.id, timeout=90)
        self.assertEqual(done.status, "done", done.error)
        self.assertEqual(len(self.server.bundles), 1)
        return done

    def _manifest(self):
        bundle = self.server.bundles[0]
        with zipfile.ZipFile(io.BytesIO(bundle["blob"])) as archive:
            names = archive.namelist()
            manifest = json.loads(archive.read("manifest.json"))
        return bundle, names, manifest

    def test_paper_drop_builds_c12_bundle(self):
        done = self._run_paper()
        bundle, names, manifest = self._manifest()

        self.assertEqual(bundle["authorization"], "Bearer sai_test")
        self.assertEqual(bundle["content_type"], "application/zip")
        self.assertEqual(bundle["project"], "alpha")

        self.assertEqual(manifest["format"], "sai-prep-bundle")
        self.assertEqual(manifest["version"], 1)
        self.assertEqual(manifest["target"], {"paper_id": 12})
        self.assertEqual(manifest["pdf"], "paper.pdf")
        self.assertEqual(manifest["pdf_sha256"], hashlib.sha256(PDF_BYTES).hexdigest())
        self.assertEqual(manifest["docling"]["md"], "docling/paper.md")
        self.assertEqual(manifest["docling"]["json"], "docling/paper.json")
        self.assertEqual(manifest["docling"]["dir"], "docling")
        self.assertEqual(manifest["translation"]["mono"], "translated/paper-mono.pdf")
        self.assertEqual(manifest["translation"]["dual"], "translated/paper-dual.pdf")
        self.assertTrue(manifest["translation"]["service"].startswith("mlx:"))
        self.assertEqual(manifest["tools"]["sai_prep"], "0.1.0")
        self.assertIn("2.134.0", manifest["tools"]["docling"])
        self.assertIn("1.9.11", manifest["tools"]["pdf2zh"])

        self.assertIn("paper.pdf", names)
        self.assertIn("docling/paper.md", names)
        self.assertIn("docling/paper.json", names)
        self.assertTrue(any(n.startswith("docling/paper_artifacts/") for n in names))
        self.assertIn("translated/paper-mono.pdf", names)
        self.assertIn("translated/paper-dual.pdf", names)

        self.assertEqual(done.result.get("paper_id"), 12)
        # 成功したら作業の置き場を消す。
        self.assertFalse((self.jobs / done.id).exists())

    def test_bibtex_job_targets_bibtex(self):
        bibtex = "@article{fake, title={A Fake Paper}}"
        job = self.pipeline.submit_bibtex("alpha", bibtex, PDF_BYTES, "fake.pdf")
        done = self.pipeline.wait(job.id, timeout=90)
        self.assertEqual(done.status, "done", done.error)
        _, _, manifest = self._manifest()
        self.assertEqual(manifest["target"], {"bibtex": bibtex})

        # pdf2zh には /v1 で終わる住所とモデルが渡っている。
        line = json.loads(self.pdf2zh_log.read_text(encoding="utf-8").splitlines()[0])
        self.assertTrue(line["base_url"].endswith("/v1"))
        self.assertIn("translategemma", line["model"])
        self.assertIn("--prompt", line["args"])

    def test_translate_off_has_null_translation(self):
        self.config.translate = False
        self._run_paper()
        _, names, manifest = self._manifest()
        self.assertIsNone(manifest["translation"])
        self.assertFalse(any(n.startswith("translated/") for n in names))

    def test_mlx_server_starts_and_stops(self):
        self._run_paper()
        events = self.mlx_log.read_text(encoding="utf-8").split()
        self.assertIn("start", events)
        self.assertIn("stop", events)
        self.assertFalse(self.pipeline.mlx_running)

    def test_missing_tool_tells_to_run_setup(self):
        with mock.patch("sai_prep.pipeline.find_tool", return_value=None):
            job = self.pipeline.submit_paper("alpha", 12, PDF_BYTES, "paper.pdf")
            done = self.pipeline.wait(job.id, timeout=30)
        self.assertEqual(done.status, "failed")
        self.assertIn("sai-prep setup", done.error or "")

    def test_non_pdf_is_rejected(self):
        job = self.pipeline.submit_paper("alpha", 12, b"hello", "paper.pdf")
        done = self.pipeline.wait(job.id, timeout=30)
        self.assertEqual(done.status, "failed")
        self.assertIn("PDF", done.error or "")

    def test_retry_runs_a_failed_job_again_with_the_same_pdf(self):
        with mock.patch("sai_prep.pipeline.find_tool", return_value=None):
            job = self.pipeline.submit_paper("alpha", 12, PDF_BYTES, "paper.pdf")
            failed = self.pipeline.wait(job.id, timeout=30)
        self.assertEqual(failed.status, "failed")
        again = self.pipeline.retry(job.id)
        self.assertIsNotNone(again)
        self.assertNotEqual(again.id, job.id)
        self.assertIsNone(self.pipeline.get(job.id))  # 古い失敗の行は捨てる
        done = self.pipeline.wait(again.id, timeout=90)
        self.assertEqual(done.status, "done", done.error)
        self.assertIsNone(self.pipeline.retry(again.id))  # 失敗していない仕事はやり直せない

    def test_polls_until_job_is_done(self):
        self.server._job_polls_before_done = 2
        done = self._run_paper()
        self.assertEqual(done.status, "done")


class BundleOnlyTest(unittest.TestCase):
    def test_normalize_docling_output_flattens_nested_artifacts(self):
        from sai_prep.pipeline import normalize_docling_output

        with tempfile.TemporaryDirectory() as tmp:
            workdir = Path(tmp)
            nested = workdir / "docling" / "docling" / "paper_artifacts"
            nested.mkdir(parents=True)
            (nested / "fig1.png").write_bytes(b"x")
            (workdir / "docling" / "paper.md").write_text("# x", encoding="utf-8")
            normalize_docling_output(workdir)
            self.assertTrue((workdir / "docling" / "paper_artifacts" / "fig1.png").is_file())
            self.assertFalse((workdir / "docling" / "docling").exists())

    def test_build_bundle_shape_without_translation(self):
        from sai_prep.pipeline import build_bundle

        with tempfile.TemporaryDirectory() as tmp:
            workdir = Path(tmp)
            (workdir / "paper.pdf").write_bytes(PDF_BYTES)
            (workdir / "docling").mkdir()
            (workdir / "docling" / "paper.md").write_text("# x", encoding="utf-8")
            (workdir / "docling" / "paper.json").write_text("{}", encoding="utf-8")
            path = build_bundle(
                workdir,
                pdf_bytes=PDF_BYTES,
                target={"paper_id": 1},
                pdf_sha256="0" * 64,
                docling_version="docling 2.134.0",
                pdf2zh_version="",
                translated=False,
                mlx_model="mlx-community/translategemma-4b-it-4bit_immersive-translate",
            )
            with zipfile.ZipFile(path) as archive:
                manifest = json.loads(archive.read("manifest.json"))
            self.assertIsNone(manifest["translation"])
            self.assertEqual(manifest["target"], {"paper_id": 1})


if __name__ == "__main__":
    unittest.main()
