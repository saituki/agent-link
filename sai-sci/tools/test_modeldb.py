"""ModelDB のカタログ `modeldb.py` のテスト。

ネットは使わない。`http_get` を URL から応答を返す辞書の偽物に差し替え、
`SAI_SAI_HOME` は一時フォルダにする。実際の ModelDB も `~/.sai-moon` も触らない。

    cd sai-sci/tools && python3 -m unittest test_modeldb -v      # Python 3.11 以上
"""

from __future__ import annotations

import contextlib
import io
import json
import os
import shutil
import sys
import tempfile
import unittest
import urllib.parse
import xml.etree.ElementTree as ET
import time
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOLS))

import modeldb  # noqa: E402  （TOOLS を sys.path に入れてから読む）

# ---- 偽の ModelDB に入れる 4 モデル ------------------------------------------------------
A, B, C, D = 101, 102, 103, 104
PAPER_A, PAPER_B, PAPER_C = 201, 202, 203
DOI_B = "10.1038/s41598-021-00283-w"
PMID_A = "30001234"

# A: PubMed ID があり、PubMed でアブストラクトが取れる
MODEL_A = {
    "id": A, "name": "Alpha thalamic oscillation model", "ver_number": 1, "ver_date": "2017-05-01",
    "notes": {"value": "A unified model of thalamic oscillations."},
    "modeling_application": {"value": [{"object_id": 1, "object_name": "C or C++ program"}]},
    "region": {"value": [{"object_id": 11, "object_name": "Thalamus"}]},
    "neurons": {"value": [{"object_id": 12, "object_name": "Thalamus GLU cell"}]},
    "currents": {"value": [{"object_id": 13, "object_name": "I_h"}]},
    "receptors": {"value": []},
    "model_type": {"value": [{"object_id": 14, "object_name": "Single compartment"}]},
    "model_concept": {"value": [{"object_id": 15, "object_name": "Oscillation"}]},
    "implemented_by": {"value": [{"object_id": 16, "object_name": "Li G"}]},
    "model_paper": {"value": [{"object_id": PAPER_A, "object_name": "Li G (2017)"}]},
}
PAPER_A_JSON = {
    "title": {"value": "A unified thalamic model of multiple oscillations"},
    "authors": {"value": [{"object_id": 21, "object_name": "Li G"}, {"object_id": 22, "object_name": "Frohlich F"}]},
    "journal": {"value": "J Neurophysiol"},
    "year": {"value": "2017"},
    "doi": {"value": "10.1000/alpha"},
    "pubmed_id": {"value": PMID_A},
}
# B: PubMed ID が無く、DOI があり、Crossref でアブストラクトが取れる
MODEL_B = {
    "id": B, "name": "Beta cortical network model", "ver_number": 3, "ver_date": "2021-01-05",
    "notes": {"value": "A network model of cortical oscillations."},
    "modeling_application": {"value": [{"object_id": 1, "object_name": "C or C++ program"}]},
    "region": {"value": [{"object_id": 17, "object_name": "Neocortex"}]},
    "neurons": {"value": []},
    "model_paper": {"value": [{"object_id": PAPER_B, "object_name": "Beta authors (2021)"}]},
}
PAPER_B_JSON = {
    "title": {"value": "Beta network dynamics in cortex"},
    "authors": {"value": [{"object_id": 23, "object_name": "Beta H"}]},
    "journal": {"value": "Sci Rep"},
    "year": {"value": "2021"},
    "doi": {"value": DOI_B},
    "pubmed_id": None,
}
# C: 論文の DOI も PubMed ID も無い
MODEL_C = {
    "id": C, "name": "Gamma cascade model", "ver_number": 2,
    "notes": {"value": "A cascade model."},
    "modeling_application": {"value": [{"object_id": 1, "object_name": "NEURON"}]},
    "region": {"value": [{"object_id": 18, "object_name": "Hippocampus"}]},
    "model_paper": {"value": [{"object_id": PAPER_C, "object_name": "Gamma (2019)"}]},
}
PAPER_C_JSON = {
    "title": {"value": "Gamma cascade in hippocampus"},
    "authors": {"value": [{"object_id": 24, "object_name": "Gamma K"}]},
    "journal": {"value": "J Comput Neurosci"},
    "year": {"value": "2019"},
}
# D は `/models/104` が 404 になる（models の一覧には残す）

PUBMED_XML = """<?xml version="1.0"?>
<PubmedArticleSet>
  <PubmedArticle>
    <MedlineCitation>
      <PMID>30001234</PMID>
      <Article>
        <Abstract>
          <AbstractText Label="BACKGROUND">Thalamic oscillations shape cortex.</AbstractText>
          <AbstractText>We built a <i>unified</i> thalamic model.</AbstractText>
        </Abstract>
      </Article>
    </MedlineCitation>
  </PubmedArticle>
</PubmedArticleSet>
"""

CROSSREF_BODY = {
    "message": {
        "abstract": (
            "<jats:p><jats:italic>Abstract</jats:italic> Cortical beta dynamics "
            "were simulated with &amp; compared.</jats:p>"
        )
    }
}


def file_html(model_id: int, paths: list[str]) -> bytes:
    """ファイル一覧のページの HTML（同じファイルを 2 回出し、重なり除去を確かめる）。"""
    links = [f'<a href="/{model_id}?tab=2&file={p}">{p}</a>' for p in paths]
    return ("<html><body>" + "".join(links) + "</body></html>").encode("utf-8")


class FakeNet:
    """ModelDB と PubMed と Crossref の偽物。"""

    def __init__(self) -> None:
        # 同じ定数を別のテストで書き換えないよう、コピーして持つ
        self.models = {A: dict(MODEL_A), B: dict(MODEL_B), C: dict(MODEL_C)}  # D は一覧には出る
        self.vers = {D: 1}                                   # D は `/models/<id>` だけが 404
        self.papers = {PAPER_A: PAPER_A_JSON, PAPER_B: PAPER_B_JSON, PAPER_C: PAPER_C_JSON}
        self.files = {A: ["model/gate.c", "readme.txt", "model/gate.c"], B: ["net.py"], C: []}
        self.calls: list[str] = []
        self.model_calls = 0
        self.interrupt_at: int | None = None   # 3 番目など（`/models/<id>` の回数）

    def index_ids(self) -> list[int]:
        """サーバの一覧に出る ID（昇順）。"""
        return sorted(self.models) + ([D] if D not in self.models else [])

    def ver_of(self, model_id: int):
        """一覧に出る版。"""
        if model_id in self.models:
            return self.models[model_id]["ver_number"]
        return self.vers[model_id]

    def get(self, url: str, timeout: float = 30) -> tuple[int, bytes]:
        self.calls.append(url)
        parts = urllib.parse.urlsplit(url)
        path, query = parts.path, urllib.parse.parse_qs(parts.query)
        if parts.netloc.endswith("eutils.ncbi.nlm.nih.gov"):
            return 200, PUBMED_XML.encode("utf-8")
        if parts.netloc == "api.crossref.org":
            return 200, json.dumps(CROSSREF_BODY).encode("utf-8")
        if path == "/api/v1/models":
            return 200, json.dumps(self.index_ids()).encode()
        if path == "/api/v1/models/ver_number":
            return 200, json.dumps([self.ver_of(m) for m in self.index_ids()]).encode()
        if path.startswith("/api/v1/models/"):
            self.model_calls += 1
            if self.interrupt_at is not None and self.model_calls == self.interrupt_at:
                raise KeyboardInterrupt
            model_id = int(path.rsplit("/", 1)[1])
            if model_id not in self.models:
                return 404, b"not found"
            return 200, json.dumps(self.models[model_id]).encode("utf-8")
        if path.startswith("/api/v1/papers/"):
            paper_id = int(path.rsplit("/", 1)[1])
            if paper_id not in self.papers:
                return 404, b"not found"
            return 200, json.dumps(self.papers[paper_id]).encode("utf-8")
        if path == "/showmodel":
            model_id = int(query.get("model", ["0"])[0])
            return 200, file_html(model_id, self.files.get(model_id, []))
        return 404, b"not found"


class ExplodingNet:
    """呼ばれたら失敗する偽物（`search`・`show`・`status` はネットを使わない）。"""

    def __init__(self) -> None:
        self.calls: list[str] = []

    def get(self, url: str, timeout: float = 30) -> tuple[int, bytes]:
        self.calls.append(url)
        raise AssertionError(f"ネットを使ってはいけないところでした: {url}")


class Base(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="modeldb-"))
        self.home = self.tmp / "home"
        self._env = os.environ.get("SAI_SAI_HOME")
        os.environ["SAI_SAI_HOME"] = str(self.home)
        self._saved = (modeldb.MIN_INTERVAL_S, modeldb.RETRY_SLEEPS, modeldb.http_get)
        modeldb.MIN_INTERVAL_S = 0.0
        modeldb.RETRY_SLEEPS = ()
        self.real_http_get = modeldb.http_get
        self.net = FakeNet()
        modeldb.http_get = self.net.get

    def tearDown(self) -> None:
        modeldb.MIN_INTERVAL_S, modeldb.RETRY_SLEEPS, modeldb.http_get = self._saved
        if self._env is None:
            os.environ.pop("SAI_SAI_HOME", None)
        else:
            os.environ["SAI_SAI_HOME"] = self._env
        shutil.rmtree(self.tmp, ignore_errors=True)

    # --- 呼ぶヘルパ ---------------------------------------------------------------------
    def call(self, *argv: str) -> tuple[int, str, str]:
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = modeldb.main(list(argv))
        return code, out.getvalue(), err.getvalue()

    def sync(self, *argv: str) -> dict:
        code, out, err = self.call("sync", "--json", *argv)
        self.assertEqual(code, 0, f"sync が失敗しました: {err}")
        return json.loads(out)

    def records(self) -> dict[int, dict]:
        path = modeldb.catalog_path()
        rows = {}
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.strip():
                row = json.loads(line)  # 全行が JSON であること
                rows[row["model_id"]] = row
        return rows


class SyncTest(Base):
    def test_sync_writes_four_records(self) -> None:
        result = self.sync()
        self.assertEqual(result["total"], 4)
        self.assertEqual(result["fetched"], 3)   # D は 404 で失敗
        self.assertEqual(result["failed"], 1)
        self.assertEqual(result["skipped"], 0)
        self.assertEqual(result["removed"], 0)
        self.assertEqual(result["path"], str(self.home / "refcode" / "modeldb" / "catalog.jsonl"))

        rows = self.records()
        self.assertEqual(sorted(rows), [A, B, C, D])

    def test_record_fields(self) -> None:
        self.sync()
        rows = self.records()

        alpha = rows[A]
        self.assertEqual(alpha["model_name"], "Alpha thalamic oscillation model")
        self.assertEqual(alpha["ver_number"], 1)
        self.assertEqual(alpha["notes"], "A unified model of thalamic oscillations.")
        self.assertEqual(alpha["simulators"], ["C or C++ program"])
        self.assertEqual(alpha["regions"], [{"id": 11, "name": "Thalamus"}])
        self.assertEqual(alpha["celltypes"], [{"id": 12, "name": "Thalamus GLU cell"}])
        self.assertEqual(alpha["currents"], [{"id": 13, "name": "I_h"}])
        self.assertEqual(alpha["receptors"], [])
        self.assertEqual(alpha["model_concepts"], [{"id": 15, "name": "Oscillation"}])
        self.assertEqual(alpha["model_types"], [{"id": 14, "name": "Single compartment"}])
        self.assertEqual(alpha["implementers"], ["Li G"])
        self.assertIsNone(alpha["error"])

        # 3 つの URL と、アブストラクト
        paper = alpha["papers"][0]
        self.assertEqual(paper["paper_id"], PAPER_A)
        self.assertEqual(paper["title"], "A unified thalamic model of multiple oscillations")
        self.assertEqual(paper["authors"], ["Li G", "Frohlich F"])
        self.assertEqual(paper["year"], "2017")
        self.assertEqual(paper["pubmed_id"], PMID_A)
        self.assertEqual(alpha["source_zip_url"], f"https://modeldb.science/download/{A}")
        self.assertEqual(alpha["source_github_url"], f"https://github.com/ModelDBRepository/{A}")
        self.assertEqual(alpha["source_files_url"], f"https://modeldb.science/showmodel?model={A}&tab=2")
        self.assertEqual(alpha["model_url"], f"https://modeldb.science/showmodel?model={A}")

        # PubMed の 2 つの AbstractText（Label 付き）をつなぐ
        self.assertEqual(paper["abstract_source"], "pubmed")
        self.assertEqual(paper["abstract"],
                         "BACKGROUND: Thalamic oscillations shape cortex.\n"
                         "We built a unified thalamic model.")

        # ファイルは重なりを除いて 2 個
        self.assertEqual(alpha["file_count"], 2)
        self.assertEqual(rows[B]["file_count"], 1)

    def test_abstract_sources(self) -> None:
        self.sync()
        rows = self.records()
        # B: PubMed ID が無いので Crossref（JATS の XML を普通の文章にする）
        beta = rows[B]["papers"][0]
        self.assertEqual(beta["abstract_source"], "crossref")
        self.assertEqual(beta["abstract"], "Cortical beta dynamics were simulated with & compared.")
        # C: どちらの ID も無いので null / none（空文字は入れない）
        gamma = rows[C]["papers"][0]
        self.assertIsNone(gamma["abstract"])
        self.assertEqual(gamma["abstract_source"], "none")
        self.assertEqual(gamma["title"], "Gamma cascade in hippocampus")
        # D: 404 でもレコードはある
        self.assertIn("404", rows[D]["error"])
        self.assertEqual(rows[D]["papers"], [])
        self.assertIsNone(rows[D]["notes"])

    def test_no_empty_abstract(self) -> None:
        self.sync()
        path = modeldb.catalog_path()
        self.assertNotIn('"abstract": ""', path.read_text(encoding="utf-8"))

    def test_incremental_sync(self) -> None:
        first = self.sync()
        self.assertEqual((first["fetched"], first["skipped"]), (3, 0))

        # 2 回目: A〜C は使い回し、D だけ取り直す
        again = self.sync()
        self.assertEqual(again["skipped"], 3)
        self.assertEqual(again["fetched"], 0)
        self.assertEqual(again["failed"], 1)   # D はまた 404
        self.assertEqual(sorted(self.records()), [A, B, C, D])

        # A の版を変えると A だけ取り直す
        self.net.models[A]["ver_number"] = 9
        third = self.sync()
        # D は前回失敗しているので、いつも取り直す（error が null でない ID）
        self.assertEqual((third["fetched"], third["failed"], third["skipped"]), (1, 1, 2))
        self.assertEqual(self.records()[A]["ver_number"], 9)
        self.assertEqual(self.records()[B]["ver_number"], 3)

        # C をサーバから消すと外れる
        del self.net.models[C]
        fourth = self.sync()
        self.assertEqual(fourth["removed"], 1)
        self.assertEqual(sorted(self.records()), [A, B, D])

    def test_ids_and_limit(self) -> None:
        # --limit は取り直す ID の先頭 N 件に絞る
        self.sync("--limit", "2")
        self.assertEqual(sorted(self.records()), [A, B])
        # --ids を渡せば、その ID だけ同じ版でも取り直す
        only_b = self.sync("--ids", str(B))
        self.assertEqual(only_b["fetched"], 1)
        self.assertEqual(only_b["skipped"], 1)
        self.assertEqual(only_b["total"], 4)
        # 取り直した B の論文の要約は、前回のカタログから写して使う（PubMed には聞かない）
        before = len(self.net.calls)
        self.sync("--ids", str(B))
        self.assertFalse(any("eutils" in u for u in self.net.calls[before:]))

    def test_bad_ids(self) -> None:
        code, _, err = self.call("sync", "--ids", "abc")
        self.assertEqual(code, 1)
        self.assertIn("abc", err)

    def test_interrupt(self) -> None:
        self.net.interrupt_at = 3   # 3 番目のモデル（ID の昇順で C）で割り込む
        code, out, err = self.call("sync", "--workers", "1")
        self.assertEqual(code, 130)
        self.assertIn("中断した", err)

        # 壊れた行を残さない：全行が JSON として読める
        path = modeldb.catalog_path()
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.strip():
                json.loads(line)
        partial = sorted(self.records())
        self.assertEqual(partial, [A, B])   # D は 404 なので取れない

        # もう一度 sync すると残りが取れる
        self.net.interrupt_at = None
        again = self.sync()
        self.assertEqual(again["fetched"], 1)   # C（D はまた 404）
        self.assertEqual(again["failed"], 1)
        self.assertEqual(again["skipped"], 2)
        rows = self.records()
        self.assertEqual(sorted(rows), [A, B, C, D])
        # 中断で要約を埋められなかった分も、次の sync でまとめて埋まる
        self.assertEqual(rows[A]["papers"][0]["abstract_source"], "pubmed")
        self.assertIsNone(rows[C]["papers"][0]["abstract"])
        self.assertEqual(rows[C]["papers"][0]["abstract_source"], "none")

    def test_never_calls_blocked_paths(self) -> None:
        self.sync()
        for url in self.net.calls:
            self.assertNotIn("/download", url)
            self.assertNotIn("/getModelFile", url)
            self.assertNotIn("/search", url)

    def test_http_get_blocks_urls(self) -> None:
        for url in ("https://modeldb.science/download/1",
                    "https://modeldb.science/getModelFile?model=1",
                    "https://modeldb.science/search?q=thalamus",
                    "https://modeldb.science/login"):
            with self.assertRaises(modeldb.ModelDBError):
                self.real_http_get(url)   # 実際に要求は出ない（入口で弾く）


class ReadTest(Base):
    """`search`・`show`・`status` はネットを使わない。"""

    def setUp(self) -> None:
        super().setUp()
        self.sync()
        modeldb.http_get = ExplodingNet()

    def test_search_words(self) -> None:
        code, out, _ = self.call("search", "thalamic", "--json")
        self.assertEqual(code, 0)
        rows = json.loads(out)
        self.assertEqual([r["model_id"] for r in rows], [A])
        self.assertEqual(rows[0]["score"], 3)          # model_name に当たると 3
        self.assertEqual(rows[0]["year"], "2017")
        self.assertEqual(rows[0]["simulators"], ["C or C++ program"])
        self.assertEqual(rows[0]["regions"], ["Thalamus"])
        self.assertEqual(rows[0]["source_github_url"], f"https://github.com/ModelDBRepository/{A}")

        # 論文のタイトルに当たると 2
        code, out, _ = self.call("search", "multiple", "--json")
        self.assertEqual(json.loads(out)[0]["score"], 2)

        # 日本語の要約に当たると 1
        code, out, _ = self.call("search", "unified", "--json")
        self.assertEqual([r["model_id"] for r in json.loads(out)], [A])

        # 語は AND。両方当てないと出ない
        code, out, _ = self.call("search", "thalamus", "neocortex", "--json")
        self.assertEqual(json.loads(out), [])
        code, out, _ = self.call("search", "THALAMIC", "--json")   # 大文字小文字は区別しない
        self.assertEqual([r["model_id"] for r in json.loads(out)], [A])

    def test_search_filters(self) -> None:
        def ids(*argv: str) -> list[int]:
            code, out, _ = self.call("search", "--json", *argv)
            self.assertEqual(code, 0)
            return [row["model_id"] for row in json.loads(out)]

        self.assertEqual(ids("--doi", f"https://doi.org/{DOI_B}"), [B])   # 前後は外して比べる
        self.assertEqual(ids("--doi", DOI_B), [B])
        self.assertEqual(ids("--region", "hippo"), [C])
        self.assertEqual(ids("--simulator", "neuron"), [C])             # 大文字小文字は区別しない
        self.assertEqual(ids("--simulator", "c or c++"), [A, B])
        self.assertEqual(ids("--celltype", "GLU"), [A])
        self.assertEqual(ids("--concept", "oscillation"), [A])
        self.assertEqual(ids("--region", "Thalamus", "--concept", "Oscillation"), [A])

    def test_search_order_and_limit(self) -> None:
        code, out, _ = self.call("search", "--json")
        self.assertEqual([r["model_id"] for r in json.loads(out)], [A, B, C, D])  # 同点は ID 順
        code, out, _ = self.call("search", "model", "--json")
        scores = [r["score"] for r in json.loads(out)]
        self.assertEqual(scores, sorted(scores, reverse=True))
        code, out, _ = self.call("search", "--limit", "2", "--json")
        self.assertEqual(len(json.loads(out)), 2)
        code, out, _ = self.call("search", "--limit", "1")
        self.assertEqual(len(out.strip().splitlines()), 1)   # 表は 1 行

    def test_show(self) -> None:
        code, out, _ = self.call("show", str(A), "--json")
        self.assertEqual(code, 0)
        record = json.loads(out)
        self.assertEqual(record["model_id"], A)
        self.assertEqual(record["source_github_url"], f"https://github.com/ModelDBRepository/{A}")

        code, out, _ = self.call("show", str(A))
        self.assertEqual(code, 0)
        self.assertIn("Alpha thalamic oscillation model", out)
        self.assertIn("アブストラクト", out)
        self.assertIn("https://modeldb.science/showmodel", out)

        code, _, err = self.call("show", "999", "--json")
        self.assertEqual(code, 1)
        self.assertIn("999", err)

    def test_status(self) -> None:
        code, out, _ = self.call("status", "--json")
        self.assertEqual(code, 0)
        result = json.loads(out)
        self.assertEqual(result["count"], 4)
        self.assertEqual(result["failed"], 1)     # D
        self.assertEqual(result["path"], str(self.home / "refcode" / "modeldb" / "catalog.jsonl"))
        self.assertTrue(result["oldest_fetched_at"])
        self.assertTrue(result["newest_fetched_at"])

    def test_no_catalog(self) -> None:
        shutil.rmtree(self.home / "refcode", ignore_errors=True)
        for argv in (("search", "thalamus"), ("show", str(A))):
            code, _, err = self.call(*argv)
            self.assertEqual(code, 1)
            self.assertIn("sync", err)
        # `status` はカタログが無くても 0 件として答える
        code, out, _ = self.call("status", "--json")
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(out)["count"], 0)


class HelperTest(unittest.TestCase):
    """XML を直す関数だけの単体テスト。"""

    def test_join_abstract_text(self) -> None:
        root = ET.fromstring(
            "<PubmedArticle><MedlineCitation><PMID>1</PMID><Article><Abstract>"
            '<AbstractText Label="BACKGROUND">One two.</AbstractText>'
            "<AbstractText>Three <i>four</i> five.</AbstractText>"
            '<AbstractText Label="METHODS">  </AbstractText>'
            "</Abstract></Article></MedlineCitation></PubmedArticle>"
        )
        self.assertEqual(
            modeldb.join_abstract_text(root.findall(".//Abstract/AbstractText")),
            "BACKGROUND: One two.\nThree four five.",
        )
        self.assertEqual(modeldb.join_abstract_text([]), "")

    def test_clean_jats(self) -> None:
        self.assertEqual(modeldb.clean_jats("<jats:p>Abstract\n  One &amp; two.</jats:p>"),
                         "One & two.")
        self.assertEqual(modeldb.clean_jats("<p>No label here.</p>"), "No label here.")
        self.assertEqual(modeldb.clean_jats(""), "")

    def test_blocked_paths(self) -> None:
        # 入口で弾くのは 4 つのパス。`/download`・`/getModelFile` は本体の取得（refcode.py）の担当
        self.assertEqual(modeldb.BLOCKED_PATHS,
                         ("/download", "/getModelFile", "/search", "/login"))

    def test_host_gate_interval(self) -> None:
        # 同じホストへの要求の間隔が `MIN_INTERVAL_S` 以上あくこと（テストでは 0 にする）
        saved = modeldb.MIN_INTERVAL_S
        modeldb.MIN_INTERVAL_S = 0.15
        try:
            modeldb._host_gates.clear()
            start = time.monotonic()
            for _ in range(3):
                modeldb._host_gate("https://example.com/api/v1/models")
            self.assertGreaterEqual(time.monotonic() - start, 0.15 * 2)
        finally:
            modeldb.MIN_INTERVAL_S = saved
            modeldb._host_gates.clear()


if __name__ == "__main__":
    unittest.main()
