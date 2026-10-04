"""参照コードの道具 `refcode.py` のテスト。

一時フォルダの中だけで動かす（`SAI_SAI_HOME` と `SAI_PROJECT_ROOT` を差し替える）。
ネットは使わない（`gh-search` は `http_get` を差し替える。`fetch` は `file://` の手元のリポジトリを使う）。
実際の ~/.sai-moon やプロジェクトは触らない。

    cd sai-sci/tools && python3.13 -W ignore -m unittest test_refcode -v      # Python 3.11 以上
"""

from __future__ import annotations

import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout, redirect_stderr
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOLS))

import refcode  # noqa: E402  （テストは同じフォルダの道具を直接読む）

SCRIPT = TOOLS / "refcode.py"


class Env:
    """一時の sai_home とプロジェクト。元の環境変数も元に戻す。"""

    def __init__(self, case: unittest.TestCase | None = None) -> None:
        self.case = case
        self.tmp = Path(tempfile.mkdtemp(prefix="refcode-"))
        self.home = self.tmp / "home"
        self.project = self.tmp / "project"
        self.work = self.tmp / "work"
        for d in (self.home, self.project, self.work):
            d.mkdir(parents=True)
        self.saved = {k: os.environ.get(k) for k in ("SAI_SAI_HOME", "SAI_PROJECT_ROOT", "SAI_PAPER_BIN", "GITHUB_TOKEN")}
        os.environ["SAI_SAI_HOME"] = str(self.home)
        os.environ["SAI_PROJECT_ROOT"] = str(self.project)
        os.environ.pop("SAI_PAPER_BIN", None)
        os.environ.pop("GITHUB_TOKEN", None)

    def run(self, *args: str) -> tuple[int, str, str]:
        """`refcode.py` をこの環境で走らせる。終了コード・標準出力・標準エラーを返す。"""
        out, err = io.StringIO(), io.StringIO()
        code = 0
        try:
            with redirect_stdout(out), redirect_stderr(err):
                code = refcode.main(list(args))
        except SystemExit as e:  # argparse が引数の誤りで呼ぶ
            code = int(e.code or 0)
        return code, out.getvalue(), err.getvalue()

    def run_json(self, *args: str):
        code, out, err = self.run(*args)
        if self.case is not None:
            self.case.assertEqual(code, 0, err)
        return json.loads(out)

    def close(self) -> None:
        for key, value in self.saved.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value
        shutil.rmtree(self.tmp, ignore_errors=True)


def git(*args: str, cwd: Path | None = None) -> str:
    """手元のリポジトリを作るための `git`。"""
    proc = subprocess.run(
        ["git", *args], cwd=str(cwd) if cwd else None, capture_output=True, text=True, timeout=60
    )
    if proc.returncode != 0:
        raise AssertionError(f"git {' '.join(args)} が失敗しました: {proc.stderr.strip()}")
    return proc.stdout


# --------------------------------------------------------------------------
# 1. 直し方の表（10 例）
# --------------------------------------------------------------------------

FIX_CASES = [
    # 元の文字列, kind, url, ほかの項目
    (r"https://github.com/miscco/NM\_Thalamus.", "github",
     "https://github.com/miscco/NM_Thalamus", {}),
    (r"github.com/ jajcayn/thalamocortical\_model\_study.", "github",
     "https://github.com/jajcayn/thalamocortical_model_study", {}),
    ("ttps:// github.com/BlueBrain/eFEL).", "github",
     "https://github.com/BlueBrain/eFEL", {}),
    (r"https://github.com/TomBugnon/ deNEST/tree/MC\_sleep/example\_networks/", "github",
     "https://github.com/TomBugnon/deNEST", {"ref": "MC_sleep", "subpath": "example_networks"}),
    ("https://github.com/miscco/NM Cortex SR.", "github",
     "https://github.com/miscco/NM",
     {"candidates": ["https://github.com/miscco/NM_Cortex_SR",
                     "https://github.com/miscco/NM-Cortex-SR"]}),
    ("https://github.com/jescab01/ThalamusInRSFC_2023.git)", "github",
     "https://github.com/jescab01/ThalamusInRSFC_2023", {}),
    ("(http:// modeldb.yale.edu/233509; Accession number 233509)", "modeldb",
     "https://modeldb.science/233509", {"model_id": 233509}),
    ("available on SenseLab ModelDB (accession no. 279).", "modeldb",
     "https://modeldb.science/279", {"model_id": 279}),
    ("[modelDB](https://modeldb.science/2016601)", "modeldb",
     "https://modeldb.science/2016601", {"model_id": 2016601}),
    ("[github](https://doi.org/10.5281/zenodo.7724411)", "zenodo",
     "https://doi.org/10.5281/zenodo.7724411", {}),
]


class FixTextTest(unittest.TestCase):
    def test_table_of_ten_cases(self) -> None:
        for text, kind, url, extra in FIX_CASES:
            with self.subTest(text=text):
                rows = refcode.extract_links(text)
                self.assertEqual(len(rows), 1, rows)
                row = rows[0]
                self.assertEqual(row["kind"], kind, row)
                self.assertEqual(row["url"], url, row)
                for key, want in extra.items():
                    self.assertEqual(row[key], want, row)

    def test_modeldb_yale_and_accession_agree(self) -> None:
        """同じ ModelDB の ID が 2 つの形で出ても 1 件にまとめる。"""
        rows = refcode.extract_links(FIX_CASES[6][0])
        self.assertEqual([r["model_id"] for r in rows], [233509])

    def test_other_repository_is_kind_other(self) -> None:
        rows = refcode.extract_links("see gitlab.com/group/proj and bitbucket.org/a/b")
        self.assertEqual([r["kind"] for r in rows], ["other", "other"])


# --------------------------------------------------------------------------
# 2. links
# --------------------------------------------------------------------------

FAKE_PAPER = """#!/usr/bin/env python3
import json, sys
chunks = [
  {"paper_id": 9, "title": "A", "cite": "[#9.13.1]", "source_kind": "body",
   "text": "https://github.com/miscco/NM Cortex SR."},
  {"paper_id": 9, "title": "A", "cite": "[#9.13.2]", "source_kind": "note",
   "text": "See https://github.com/miscco/NM and again github.com/miscco/NM."},
  {"paper_id": 8, "title": "B", "cite": "[#8.7.4]", "source_kind": "body",
   "text": "The code is available at github (Schellenberger Costa 2006a)."},
  {"paper_id": 8, "title": "B", "cite": "[#8.7.5]", "source_kind": "body",
   "text": "(http:// modeldb.yale.edu/233509; Accession number 233509)"},
]
if "--json" not in sys.argv:
    sys.exit("json だけ")
print(json.dumps(chunks))
"""


class Base(unittest.TestCase):
    """一時の置き場を用意する土台。"""

    def setUp(self) -> None:
        self.env = Env(self)
        self.addCleanup(self.env.close)


class LinksTest(Base):
    def setUp(self) -> None:
        super().setUp()
        self.fake = self.env.work / "fake-paper"
        self.fake.write_text(FAKE_PAPER, encoding="utf-8")
        self.fake.chmod(0o755)
        os.environ["SAI_PAPER_BIN"] = str(self.fake)

    def test_links_dedupes_and_lists_mentions(self) -> None:
        data = self.env.run_json("links", "--json")
        urls = [row["url"] for row in data["links"]]
        self.assertIn("https://github.com/miscco/NM", urls)
        self.assertEqual(len(urls), len(set(urls)), "同じ論文の同じ URL は 1 件だけ")
        model = [row for row in data["links"] if row["kind"] == "modeldb"]
        self.assertEqual([row["model_id"] for row in model], [233509])
        cites = sorted(row["cite"] for row in data["mentions"])
        self.assertEqual(cites, ["[#8.7.4]"], data["mentions"])
        # mentions には塊の本文も入る
        self.assertIn("Schellenberger", data["mentions"][0]["text"])

    def test_links_candidates_are_kept(self) -> None:
        """リポジトリ名が崩れているときは、並んだ語から 2 案が候補に残る。"""
        data = self.env.run_json("links", "--json")
        row = next(r for r in data["links"] if r["url"] == "https://github.com/miscco/NM")
        self.assertEqual(
            row["candidates"],
            ["https://github.com/miscco/NM_Cortex_SR", "https://github.com/miscco/NM-Cortex-SR"],
        )
        self.assertEqual(row["cite"], "[#9.13.1]")

    def test_links_filters_by_paper(self) -> None:
        data = self.env.run_json("links", "--paper", "8", "--json")
        self.assertEqual({row["paper_id"] for row in data["links"]}, {8})
        self.assertEqual({row["paper_id"] for row in data["mentions"]}, {8})

    def test_links_table_without_json(self) -> None:
        code, out, _ = self.env.run("links")
        self.assertEqual(code, 0)
        self.assertIn("https://github.com/miscco/NM", out)
        self.assertIn("[#8.7.4]", out)


# --------------------------------------------------------------------------
# 3. gh-search
# --------------------------------------------------------------------------

SEARCH_BODY = json.dumps(
    {
        "items": [
            {
                "full_name": "BlueBrain/eFEL",
                "html_url": "https://github.com/BlueBrain/eFEL",
                "description": "Electrophys feature extraction",
                "stargazers_count": 132,
                "language": "Python",
                "pushed_at": "2024-05-01T00:00:00Z",
                "license": {"spdx_id": "BSD-3-Clause"},
            },
            {
                "full_name": "other/repo",
                "html_url": "https://github.com/other/repo",
                "description": None,
                "stargazers_count": 3,
                "language": None,
                "pushed_at": None,
                "license": None,
            },
        ]
    }
).encode()


class GhSearchTest(Base):
    def setUp(self) -> None:
        super().setUp()
        self.seen: list[tuple] = []
        self.body = SEARCH_BODY
        self.raise_403 = False
        self.original = refcode.http_get

        def fake_get(url: str, headers: dict, timeout: int = 30) -> bytes:
            self.seen.append((url, dict(headers), timeout))
            if self.raise_403:
                raise refcode.RateLimited(1735689600)
            return self.body

        refcode.http_get = fake_get
        self.addCleanup(lambda: setattr(refcode, "http_get", self.original))

    def test_items_are_copied(self) -> None:
        rows = self.env.run_json("gh-search", "eFEL", "--json")
        self.assertEqual(rows[0]["full_name"], "BlueBrain/eFEL")
        self.assertEqual(rows[0]["stars"], 132)
        self.assertEqual(rows[0]["language"], "Python")
        self.assertEqual(rows[0]["license"], "BSD-3-Clause")
        self.assertIsNone(rows[1]["license"])
        self.assertIsNone(rows[1]["language"])

    def test_query_is_url_encoded_and_language_added(self) -> None:
        self.env.run_json("gh-search", "thalamic model", "--language", "python", "--limit", "5", "--json")
        url = self.seen[-1][0]
        self.assertIn("thalamic%20model", url)
        self.assertIn("language%3Apython", url)
        self.assertIn("per_page=5", url)
        self.assertIn("sort=stars", url)

    def test_rate_limit_exits_one(self) -> None:
        self.raise_403 = True
        code, _, err = self.env.run("gh-search", "eFEL", "--json")
        self.assertEqual(code, 1)
        self.assertIn("上限", err)
        self.assertIn("GITHUB_TOKEN", err)

    def test_token_is_only_sent_when_present(self) -> None:
        self.env.run_json("gh-search", "eFEL", "--json")
        headers = self.seen[-1][1]
        self.assertNotIn("Authorization", headers)
        self.assertEqual(headers["Accept"], "application/vnd.github+json")
        self.assertEqual(headers["User-Agent"], "sai-moon-refcode/1")

        os.environ["GITHUB_TOKEN"] = "sekret"
        try:
            self.env.run_json("gh-search", "eFEL", "--json")
            headers = self.seen[-1][1]
        finally:
            os.environ.pop("GITHUB_TOKEN", None)
        self.assertEqual(headers["Authorization"], "Bearer sekret")

    def test_table_without_json(self) -> None:
        code, out, _ = self.env.run("gh-search", "eFEL")
        self.assertEqual(code, 0)
        self.assertIn("BlueBrain/eFEL", out)
        self.assertIn("132", out)


# --------------------------------------------------------------------------
# 4. fetch
# --------------------------------------------------------------------------


class FetchTest(Base):
    def setUp(self) -> None:
        super().setUp()
        self.repo = self.env.work / "origin"
        self.repo.mkdir()
        git("init", "-q", "-b", "main", cwd=self.repo)
        (self.repo / "model.py").write_text("A = 1e-3\n", encoding="utf-8")
        (self.repo / "LICENSE").write_text("BSD 3-Clause License\n\nsecond line\n", encoding="utf-8")
        git("add", "-A", cwd=self.repo)
        git("-c", "user.email=t@example.com", "-c", "user.name=t", "commit", "-q", "-m", "first", cwd=self.repo)
        self.url = f"file://{self.repo}"
        self.sha = git("rev-parse", "HEAD", cwd=self.repo).strip()

    def test_fetch_git_gives_sha_and_leaves_no_git_dir(self) -> None:
        parent = self.env.home / "refcode" / "github" / "t"
        sha = refcode.fetch_git(self.url, parent, "repo", "")
        self.assertEqual(sha, self.sha)
        dest = parent / "repo"
        self.assertTrue((dest / "model.py").is_file())
        self.assertFalse((dest / ".git").exists(), ".git は残さない")

    def test_fetch_repo_writes_meta_and_caches(self) -> None:
        parent = self.env.home / "refcode" / "github" / "t"
        record = refcode.fetch_repo(self.url, parent, "repo", "github", "t/repo", "")
        dest = Path(record["path"])
        self.assertEqual(dest.name, f"repo@{self.sha[:12]}")
        self.assertEqual(record["sha"], self.sha)
        self.assertEqual(record["id"], "t/repo")
        self.assertEqual(record["kind"], "github")
        self.assertFalse(record["cached"])
        self.assertEqual(record["license"], "BSD 3-Clause License")
        meta = json.loads((dest / ".refcode.json").read_text(encoding="utf-8"))
        self.assertEqual(meta["id"], "t/repo")
        self.assertEqual(meta["sha"], self.sha)
        self.assertEqual(meta["kind"], "github")
        for key in ("url", "ref", "fetched_at", "license"):
            self.assertIn(key, meta)
        self.assertFalse((dest / ".git").exists())
        # files は .refcode.json を数えない
        self.assertEqual(record["files"], 2, record["files"])

        again = refcode.fetch_repo(self.url, parent, "repo", "github", "t/repo", "")
        self.assertTrue(again["cached"])
        self.assertEqual(again["path"], record["path"])

    def test_fetch_by_ref(self) -> None:
        git("checkout", "-q", "-b", "next", cwd=self.repo)
        (self.repo / "b.py").write_text("B = 2.0\n", encoding="utf-8")
        git("add", "-A", cwd=self.repo)
        git("-c", "user.email=t@example.com", "-c", "user.name=t", "commit", "-q", "-m", "second", cwd=self.repo)
        git("checkout", "-q", "main", cwd=self.repo)
        parent = self.env.home / "refcode" / "github" / "t"
        sha = refcode.fetch_git(self.url, parent, "branched", "next")
        self.assertNotEqual(sha, self.sha)
        self.assertTrue((parent / "branched" / "b.py").is_file())

    def test_unknown_ref_fails(self) -> None:
        with self.assertRaises(refcode.RefcodeError):
            refcode.fetch_git(self.url, self.env.home / "x", "y", "no-such-branch")

    def test_entry_rejects_zenodo(self) -> None:
        code, _, err = self.env.run("fetch", "https://doi.org/10.5281/zenodo.7724411")
        self.assertEqual(code, 2, err)
        self.assertIn("取得しません", err)

    def test_entry_rejects_garbage(self) -> None:
        code, _, err = self.env.run("fetch", " simply words ")
        self.assertEqual(code, 2, err)

    def test_entry_parses_github_url_with_tree(self) -> None:
        target = refcode.resolve_target("https://github.com/miscco/NM_TC/tree/MC_sleep")
        self.assertEqual(target["kind"], "github")
        self.assertEqual(target["owner"], "miscco")
        self.assertEqual(target["repo"], "NM_TC")
        self.assertEqual(target["ref"], "MC_sleep")

    def test_entry_parses_modeldb(self) -> None:
        for text in ("modeldb:233509", "https://modeldb.science/233509",
                     "https://modeldb.science/showmodel?model=233509"):
            with self.subTest(text=text):
                self.assertEqual(refcode.resolve_target(text), {"kind": "modeldb", "model_id": "233509"})

    def test_fetch_command_json(self) -> None:
        original = refcode.fetch_git

        def fake_git(url, dest_parent, name, ref=""):
            return original(self.url, dest_parent, name, ref)

        refcode.fetch_git = fake_git
        self.addCleanup(lambda: setattr(refcode, "fetch_git", original))
        # remote_sha も手元のリポジトリを見るようにする
        orig_sha = refcode.remote_sha

        def fake_sha(url, ref):
            return orig_sha(self.url, ref)

        refcode.remote_sha = fake_sha
        self.addCleanup(lambda: setattr(refcode, "remote_sha", orig_sha))
        code, out, err = self.env.run("fetch", "https://github.com/t/repo", "--json")
        self.assertEqual(code, 0, err)
        data = json.loads(out)
        self.assertEqual(data["id"], "t/repo")
        self.assertEqual(data["kind"], "github")
        self.assertTrue(Path(data["path"]).is_dir())


# --------------------------------------------------------------------------
# 5. params
# --------------------------------------------------------------------------


class ParamsTest(Base):
    def setUp(self) -> None:
        super().setUp()
        self.dir = self.env.work / "code"
        self.dir.mkdir()
        (self.dir / "Thalamic_Column.h").write_text(
            "\tconst double Qt_max= 400.E-3;  // 视床\n"
            "  double g_leak = 0.1;\n"
            "  int layer = 4, count = 7;\n",
            encoding="utf-8",
        )
        (self.dir / "net.mod").write_text(
            "NEURON {\n}\nPARAMETER {\n  gbar = 0.002 (mho/cm2)\n  e = -65 : 反転電位\n}\n",
            encoding="utf-8",
        )
        (self.dir / "net.ode").write_text("net_test() {\n  par gl=0.1, el=-65\n}\n", encoding="utf-8")
        (self.dir / "train.py").write_text("A = 1e-3  # 秒\nGRID = {'lr': 0.01, 'dropout': 0.3}\n", encoding="utf-8")

    def find(self, rows, name):
        hits = [r for r in rows if r["name"] == name]
        self.assertEqual(len(hits), 1, f"{name}: {rows}")
        return hits[0]

    def test_cpp_values(self) -> None:
        rows = self.env.run_json("params", str(self.dir / "Thalamic_Column.h"), "--json")
        hit = self.find(rows, "Qt_max")
        self.assertEqual(hit["value"], "400.E-3")
        self.assertEqual(hit["number"], 0.4)
        self.assertEqual(hit["line"], 1)
        self.assertEqual(hit["file"], "Thalamic_Column.h")
        self.assertIsNone(hit["unit"])
        self.assertEqual(hit["src"], "Thalamic_Column.h:1")
        self.assertTrue(hit["comment"])
        self.assertEqual(self.find(rows, "layer")["value"], "4")

    def test_mod_values_and_units(self) -> None:
        rows = self.env.run_json("params", str(self.dir / "net.mod"), "--json")
        gbar = self.find(rows, "gbar")
        self.assertEqual(gbar["unit"], "mho/cm2")
        self.assertEqual(gbar["line"], 4)
        self.assertEqual(gbar["number"], 0.002)
        self.assertEqual(self.find(rows, "e")["number"], -65.0)

    def test_ode_pairs(self) -> None:
        rows = self.env.run_json("params", str(self.dir / "net.ode"), "--json")
        self.assertEqual(sorted(r["name"] for r in rows), ["el", "gl"])
        self.assertEqual(self.find(rows, "el")["number"], -65.0)
        self.assertEqual(self.find(rows, "gl")["number"], 0.1)
        self.assertEqual(self.find(rows, "gl")["line"], 2)

    def test_python_values_and_dicts(self) -> None:
        rows = self.env.run_json("params", str(self.dir / "train.py"), "--json")
        a = self.find(rows, "A")
        self.assertEqual(a["value"], "1e-3")
        self.assertEqual(a["comment"], "秒")
        self.assertEqual(self.find(rows, "lr")["value"], "0.01")
        self.assertEqual(self.find(rows, "dropout")["value"], "0.3")

    def test_folder_and_ext_filter(self) -> None:
        rows = self.env.run_json("params", str(self.dir), "--ext", ".py,.ode", "--json")
        self.assertEqual({r["file"] for r in rows}, {"net.ode", "train.py"})

    def test_src_has_id_and_sha_when_meta_exists(self) -> None:
        """`.refcode.json` があれば `src` に `id@sha12` が付く。"""
        sha = "1c8f5f09d3a9" + "0" * 28
        (self.dir / ".refcode.json").write_text(
            json.dumps({"id": "miscco/NM_TC", "sha": sha, "kind": "github"}), encoding="utf-8"
        )
        rows = self.env.run_json("params", str(self.dir / "Thalamic_Column.h"), "--json")
        hit = self.find(rows, "Qt_max")
        self.assertEqual(hit["src"], f"miscco/NM_TC@{sha[:12]}:Thalamic_Column.h:{hit['line']}")
        self.assertEqual(hit["file"], "Thalamic_Column.h")

    def test_skips_git_and_big_files(self) -> None:
        (self.dir / ".git").mkdir()
        (self.dir / ".git" / "hidden.py").write_text("HIDDEN = 9.9\n", encoding="utf-8")
        big = self.dir / "big.py"
        big.write_text("BIG = 1.0\n" + "# padding\n" * 200000, encoding="utf-8")
        rows = self.env.run_json("params", str(self.dir), "--ext", ".py", "--json")
        names = {r["name"] for r in rows}
        self.assertNotIn("HIDDEN", names)
        self.assertNotIn("BIG", names)

    def test_missing_path_fails(self) -> None:
        code, _, err = self.env.run("params", str(self.dir / "no-such.py"))
        self.assertEqual(code, 1)
        self.assertIn("見つかりません", err)

    def test_table_without_json(self) -> None:
        code, out, _ = self.env.run("params", str(self.dir / "net.ode"))
        self.assertEqual(code, 0)
        self.assertIn("gl = 0.1", out)


# --------------------------------------------------------------------------
# 6. check
# --------------------------------------------------------------------------

GOOD_TRAIN = '''PROJECT, SCI_NODE, STUDY = "west", "O6", "O6-ID120"
EPOCHS, DEVICE = 25, "auto"

FIXED = {"batch_size": 50}  # src: 仕様書

QMAX_T = 0.4  # src: miscco/NM_TC@1c8f5f09d3a9:Thalamic_Column.h:122
SIGMA_P = 6.7  # src: #9.13.1
G_KNA = 1.93  # src: F12
TAU_H = 30.0  # src: 仕様書
G_LEAK = 0.01  # src: 目安
# src: #9.13.2
QMAX_U = 0.03  # src: modeldb/233509@v3:model.mod:12

TAU = {"p": 1.0, "t": 2.0}  # 出典が無い


def pick_device() -> str:
    return "cuda"


def features(x):
    """自明でない数を使う。"""
    return x * 0.3  # src: 仕様書


def main():
    run(features(1))
'''

BARE_TRAIN = '''PROJECT = "west"

QMAX_T = 0.4
SIGMA_P = 6.7


def features(x):
    return x * 0.3
'''

MEMORY_TRAIN = '''PROJECT = "west"
NOISE = 0.01  # src: 公開コードの記憶
'''

BAD_FORM_TRAIN = '''PROJECT = "west"
NOISE = 0.01  # src: たぶんだから
'''


class CheckTest(Base):
    def setUp(self) -> None:
        super().setUp()

    def write(self, text: str) -> str:
        path = self.env.work / "train.py"
        path.write_text(text, encoding="utf-8")
        return str(path)

    def test_without_sources_exits_one(self) -> None:
        code, _, err = self.env.run("check", self.write(BARE_TRAIN))
        self.assertEqual(code, 1, err)

    def test_five_source_forms_exit_zero(self) -> None:
        code, out, err = self.env.run("check", self.write(GOOD_TRAIN), "--json")
        self.assertEqual(code, 0, err)
        data = json.loads(out)
        self.assertTrue(data["ok"])
        self.assertEqual(data["errors"], [])
        counts = data["counts"]
        self.assertGreaterEqual(counts["ref"], 1)
        self.assertGreaterEqual(counts["chunk"], 1)
        self.assertGreaterEqual(counts["f"], 1)
        self.assertGreaterEqual(counts["spec"], 1)
        self.assertGreaterEqual(counts["guess"], 1)
        guess_names = [row["name"] for row in data["guesses"]]
        self.assertIn("G_LEAK", guess_names)

    def test_memory_is_rejected(self) -> None:
        code, out, err = self.env.run("check", self.write(MEMORY_TRAIN), "--json")
        self.assertEqual(code, 1, err)
        data = json.loads(out)
        self.assertFalse(data["ok"])
        self.assertTrue(any("記憶" in row["reason"] for row in data["errors"]))

    def test_unknown_source_form_is_rejected(self) -> None:
        code, out, err = self.env.run("check", self.write(BAD_FORM_TRAIN), "--json")
        self.assertEqual(code, 1, err)
        data = json.loads(out)
        self.assertTrue(any("形" in row["reason"] for row in data["errors"]))

    def test_template_names_are_skipped(self) -> None:
        path = self.write('PROJECT = "west"\nEPOCHS, DEVICE = 25, "auto"\nRUN_TAGS = {"a": 1}\n')
        code, _, err = self.env.run("check", path)
        self.assertEqual(code, 0, err)

    def test_trivial_numbers_need_no_source(self) -> None:
        path = self.write('PROJECT = "west"\nA = 0\nB = 1\nC = 2\nD = 0.5\nE = -1\n')
        code, _, err = self.env.run("check", path)
        self.assertEqual(code, 0, err)

    def test_function_without_source_is_rejected(self) -> None:
        path = self.write('PROJECT = "west"\ndef f(x):\n    return x * 0.3\n')
        code, out, err = self.env.run("check", path, "--json")
        self.assertEqual(code, 1, err)
        data = json.loads(out)
        self.assertEqual(data["errors"][0]["name"], "f")

    def test_source_above_first_line_is_allowed(self) -> None:
        path = self.write('PROJECT = "west"\n# src: 仕様書\nQMAX = 0.4\n')
        code, _, err = self.env.run("check", path)
        self.assertEqual(code, 0, err)

    def test_trailing_comment_does_not_cover_the_next_line(self) -> None:
        """コードの後ろの行末コメントは、次の文の注記には数えない。"""
        path = self.write('PROJECT = "west"\nA = 3.3  # src: 目安\nB = 4.4\n')
        code, out, err = self.env.run("check", path, "--json")
        self.assertEqual(code, 1, err)
        data = json.loads(out)
        self.assertFalse(data["ok"])
        names = [row["name"] for row in data["errors"]]
        self.assertEqual(names, ["B"], data["errors"])
        self.assertEqual(data["errors"][0]["line"], 3)
        # A の注記は A の分として数えられている
        self.assertEqual(data["counts"]["guess"], 1)
        self.assertEqual([row["name"] for row in data["guesses"]], ["A"])

    def test_comment_only_line_above_still_covers(self) -> None:
        """コメントだけの行の注記は、次の文に効く（従来の挙動）。"""
        path = self.write('PROJECT = "west"\nA = 3.3  # src: 目安\n# src: 仕様書\nB = 4.4\n')
        code, _, err = self.env.run("check", path)
        self.assertEqual(code, 0, err)

    def test_table_without_json(self) -> None:
        code, out, _ = self.env.run("check", self.write(BARE_TRAIN))
        self.assertEqual(code, 1)
        self.assertIn("QMAX_T", out)
        self.assertIn("出典", out)

    def test_missing_file_fails(self) -> None:
        code, _, err = self.env.run("check", str(self.env.work / "no.py"))
        self.assertEqual(code, 1)
        self.assertIn("見つかりません", err)


# --------------------------------------------------------------------------
# 7. 入口の全体
# --------------------------------------------------------------------------


class MainTest(Base):
    def setUp(self) -> None:
        super().setUp()

    def test_help_is_japanese(self) -> None:
        proc = subprocess.run(
            [sys.executable, str(SCRIPT), "--help"], capture_output=True, text=True, timeout=60
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        for verb in ("links", "gh-search", "fetch", "params", "check"):
            self.assertIn(verb, proc.stdout)
        self.assertIn("参照コード", proc.stdout)
        for verb in ("links", "gh-search", "fetch", "params", "check"):
            with self.subTest(verb=verb):
                out = subprocess.run(
                    [sys.executable, str(SCRIPT), verb, "--help"],
                    capture_output=True, text=True, timeout=60,
                )
                self.assertEqual(out.returncode, 0, out.stderr)
                self.assertIn("usage", out.stdout)
                self.assertNotIn("Traceback", out.stderr)


if __name__ == "__main__":
    unittest.main()