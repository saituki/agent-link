#!/usr/bin/env python3
"""onto_build.py のテスト。標準ライブラリだけで動く。

小さな固定の OBO と XML を一時フォルダに書き、スクリプトを走らせて onto.db と manifest.json を確かめる。
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import onto_build  # noqa: E402

SCRIPT = Path(onto_build.__file__).resolve()
TIMEOUT = 120

MONDO_OBO = """format-version: 1.2
data-version: releases/2026-10-06
ontology: mondo

[Term]
id: MONDO:0000001
name: disease

[Term]
id: MONDO:0000002
name: brain disorder
is_a: MONDO:0000001 ! disease

[Term]
id: MONDO:0000003
name: syndromic disease
is_a: MONDO:0000001 ! disease

[Term]
id: MONDO:0000004
name: infantile epilepsy syndrome
is_a: MONDO:0000001 ! disease

[Term]
id: MONDO:0005027
name: epilepsy
synonym: "Epilepsy NOS" EXACT []
is_a: MONDO:0000002 ! brain disorder

[Term]
id: MONDO:0018097
name: infantile spasms
synonym: "West syndrome" EXACT []
synonym: "IESS" RELATED []
synonym: "infantile spasms" EXACT []
is_a: MONDO:0000003 ! syndromic disease
is_a: MONDO:0000004 ! infantile epilepsy syndrome
is_a: MONDO:0009999 ! old term
is_a: CHEBI:1 ! not a MONDO term
xref: MESH:D013036
xref: DOID:1234

[Term]
id: MONDO:0009999
name: old term
synonym: "old" EXACT []
xref: MESH:D000009
is_a: MONDO:0000001 ! disease
is_obsolete: true

[Typedef]
id: part_of
name: part of
"""

MESH_XML = """<?xml version="1.0" encoding="UTF-8"?>
<DescriptorRecordSet LanguageCode="eng">
  <DescriptorRecord>
    <DescriptorUI>D000001</DescriptorUI>
    <DescriptorName><String>Nervous System Diseases</String></DescriptorName>
    <TreeNumberList><TreeNumber>C10</TreeNumber></TreeNumberList>
  </DescriptorRecord>
  <DescriptorRecord>
    <DescriptorUI>D000002</DescriptorUI>
    <DescriptorName><String>Brain Diseases</String></DescriptorName>
    <TreeNumberList><TreeNumber>C10.228</TreeNumber></TreeNumberList>
  </DescriptorRecord>
  <DescriptorRecord>
    <DescriptorUI>D000003</DescriptorUI>
    <DescriptorName><String>Cerebral Diseases</String></DescriptorName>
    <TreeNumberList><TreeNumber>C10.228.140</TreeNumber></TreeNumberList>
  </DescriptorRecord>
  <DescriptorRecord>
    <DescriptorUI>D000004</DescriptorUI>
    <DescriptorName><String>Brain Stem Diseases</String></DescriptorName>
    <TreeNumberList><TreeNumber>C10.228.141</TreeNumber></TreeNumberList>
  </DescriptorRecord>
  <DescriptorRecord>
    <DescriptorUI>D013036</DescriptorUI>
    <DescriptorName><String>Spasms, Infantile</String></DescriptorName>
    <TreeNumberList>
      <TreeNumber>C10.228.140.490</TreeNumber>
      <TreeNumber>C10.228.141.300</TreeNumber>
    </TreeNumberList>
    <ConceptList>
      <Concept>
        <TermList>
          <Term><String>Spasms, Infantile</String></Term>
          <Term><String>West Syndrome</String></Term>
        </TermList>
      </Concept>
      <Concept>
        <TermList>
          <Term><String>Infantile Spasms</String></Term>
        </TermList>
      </Concept>
    </ConceptList>
  </DescriptorRecord>
</DescriptorRecordSet>
"""


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="onto-build-test-"))
        self.obo = self.tmp / "mondo.obo"
        self.obo.write_text(MONDO_OBO, encoding="utf-8")
        self.xml = self.tmp / "desc2026.xml"
        self.xml.write_text(MESH_XML, encoding="utf-8")
        self.out = self.tmp / "out"

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_script(self, *args) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(SCRIPT), *[str(a) for a in args]],
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
        )

    def build(self, *args) -> subprocess.CompletedProcess:
        proc = self.run_script(*args, "--out", self.out)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        return proc

    def db(self) -> sqlite3.Connection:
        return sqlite3.connect(self.out / "onto.db")

    def mondo_db(self) -> sqlite3.Connection:
        self.build("--mondo", self.obo)
        return self.db()


class NormTest(unittest.TestCase):
    def test_norm(self):
        self.assertEqual(onto_build.norm("West  Syndrome"), "west syndrome")
        self.assertEqual(onto_build.norm("ＩＥＳＳ"), "iess")
        self.assertEqual(onto_build.norm("  IESS\t\n"), "iess")


class MondoTest(Base):
    def test_terms_synonyms_edges_xrefs(self):
        db = self.mondo_db()
        ids = {r[0] for r in db.execute("SELECT id FROM term")}
        # 廃止の語は入らない。Typedef も入らない。
        self.assertEqual(
            ids,
            {"MONDO:0000001", "MONDO:0000002", "MONDO:0000003", "MONDO:0000004", "MONDO:0005027", "MONDO:0018097"},
        )
        self.assertEqual(db.execute("SELECT label FROM term WHERE id='MONDO:0018097'").fetchone()[0], "infantile spasms")
        self.assertEqual(db.execute("SELECT source FROM term WHERE id='MONDO:0005027'").fetchone()[0], "mondo")

        syn = set(db.execute("SELECT text, norm, is_label FROM synonym WHERE term_id='MONDO:0018097'"))
        # 項目名と同じ別名（infantile spasms）は label に 1 つにまとまる。
        self.assertEqual(syn, {("infantile spasms", "infantile spasms", 1), ("West syndrome", "west syndrome", 0), ("IESS", "iess", 0)})
        self.assertEqual(db.execute("SELECT text FROM synonym WHERE term_id='MONDO:0005027' AND is_label=1").fetchone()[0], "epilepsy")

        edges = set(db.execute("SELECT child, parent FROM edge"))
        # 廃止の語・MONDO 以外への is_a は edge に入らない。後ろの節を親にする場合も入る。
        self.assertEqual(
            edges,
            {
                ("MONDO:0000002", "MONDO:0000001"),
                ("MONDO:0000003", "MONDO:0000001"),
                ("MONDO:0000004", "MONDO:0000001"),
                ("MONDO:0005027", "MONDO:0000002"),
                ("MONDO:0018097", "MONDO:0000003"),
                ("MONDO:0018097", "MONDO:0000004"),
            },
        )
        xrefs = set(db.execute("SELECT term_id, other FROM xref"))
        self.assertEqual(xrefs, {("MONDO:0018097", "MESH:D013036"), ("MONDO:0018097", "DOID:1234")})
        db.close()

    def test_meta_and_manifest(self):
        self.build("--mondo", self.obo)
        db = self.db()
        meta = dict(db.execute("SELECT key, value FROM meta"))
        self.assertEqual(meta["schema_version"], "1")
        self.assertEqual(meta["mondo_version"], "releases/2026-10-06")
        self.assertEqual(meta["mondo_terms"], "6")
        self.assertNotIn("mesh_version", meta)
        db.close()

        manifest = json.loads((self.out / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(len(manifest["sources"]), 1)
        src = manifest["sources"][0]
        self.assertEqual(src["name"], "mondo")
        self.assertEqual(src["version"], "releases/2026-10-06")
        self.assertEqual(src["path"], "mondo.obo")
        self.assertEqual(src["sha256"], sha256(self.obo))
        self.assertEqual(src["terms"], 6)
        self.assertEqual(src["license"], "CC BY 4.0")
        self.assertIn("Mondo Disease Ontology", src["attribution"])
        self.assertEqual(manifest["tables"]["term"], 6)


class MeshTest(Base):
    def test_terms_synonyms_and_multi_parent_edges(self):
        self.build("--mesh", self.xml, "--mesh-version", "2026")
        db = self.db()
        self.assertEqual(db.execute("SELECT COUNT(*) FROM term").fetchone()[0], 5)

        syn = set(db.execute("SELECT text, is_label FROM synonym WHERE term_id='MESH:D013036'"))
        # 記述子名と同じ別名は 1 つにまとまり、label の行になる。
        self.assertEqual(syn, {("Spasms, Infantile", 1), ("West Syndrome", 0), ("Infantile Spasms", 0)})

        edges = set(db.execute("SELECT child, parent FROM edge"))
        # 木番号の親は「末尾の .nnn を除いた木番号」の持ち主。D013036 は 2 つの親を持つ。
        self.assertEqual(
            edges,
            {
                ("MESH:D000002", "MESH:D000001"),
                ("MESH:D000003", "MESH:D000002"),
                ("MESH:D000004", "MESH:D000002"),
                ("MESH:D013036", "MESH:D000003"),
                ("MESH:D013036", "MESH:D000004"),
            },
        )
        meta = dict(db.execute("SELECT key, value FROM meta"))
        db.close()
        self.assertEqual(meta["mesh_version"], "2026")
        self.assertEqual(meta["mesh_descriptors"], "5")

        manifest = json.loads((self.out / "manifest.json").read_text(encoding="utf-8"))
        src = manifest["sources"][0]
        self.assertEqual(src["name"], "mesh")
        self.assertEqual(src["sha256"], sha256(self.xml))
        self.assertEqual(src["attribution"], "Courtesy of the U.S. National Library of Medicine")

    def test_both_sources(self):
        self.build("--mondo", self.obo, "--mesh", self.xml, "--mesh-version", "2026")
        manifest = json.loads((self.out / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual([s["name"] for s in manifest["sources"]], ["mondo", "mesh"])
        self.assertEqual(manifest["tables"]["term"], 11)
        db = self.db()
        meta = dict(db.execute("SELECT key, value FROM meta"))
        self.assertEqual(meta["mondo_version"], "releases/2026-10-06")
        self.assertEqual(meta["mesh_version"], "2026")
        self.assertEqual(meta["mondo_terms"], "6")
        self.assertEqual(meta["mesh_descriptors"], "5")
        db.close()


class CliTest(Base):
    def test_argument_errors_exit_2(self):
        self.assertEqual(self.run_script("--out", self.out).returncode, 2)
        self.assertEqual(self.run_script("--mesh", self.xml, "--out", self.out).returncode, 2)
        self.assertFalse(self.out.exists())

    def test_existing_output_needs_force(self):
        self.build("--mondo", self.obo)
        proc = self.run_script("--mondo", self.obo, "--out", self.out)
        self.assertEqual(proc.returncode, 4)
        self.assertTrue(proc.stderr.strip().splitlines()[-1])
        proc = self.run_script("--mondo", self.obo, "--out", self.out, "--force")
        self.assertEqual(proc.returncode, 0, proc.stderr)

    def test_failure_keeps_finished_output(self):
        self.build("--mondo", self.obo)
        before = sha256(self.out / "onto.db")
        self.xml.write_text("<DescriptorRecordSet><DescriptorRecord>", encoding="utf-8")
        proc = self.run_script("--mondo", self.obo, "--mesh", self.xml, "--mesh-version", "2026", "--out", self.out, "--force")
        self.assertEqual(proc.returncode, 1)
        self.assertTrue(proc.stderr.strip())
        self.assertEqual(sha256(self.out / "onto.db"), before)
        self.assertFalse((self.out / "onto.db.tmp").exists())
        self.assertFalse((self.out / "manifest.json.tmp").exists())


if __name__ == "__main__":
    unittest.main()
