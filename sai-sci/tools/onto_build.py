#!/usr/bin/env python3
"""MONDO（OBO）と MeSH（記述子の XML）を onto.db と manifest.json にする前計算の道具。

`sai-sci onto`（U10）が読む onto.db を作る。実行時の外部通信をなくすため、1 回だけ手で走らせる。
標準ライブラリだけで動く（Python 3.11 以上）。

使い方:
  python3 onto_build.py [--mondo mondo.obo] [--mesh desc2026.xml --mesh-version 2026] --out DIR [--force]

終了コード: 0 成功 / 2 引数の誤り / 4 出力が既にある（--force で置き換え）/ 1 その他（標準エラーの最後の行に理由）。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sqlite3
import sys
import unicodedata
import xml.etree.ElementTree as ET
from datetime import datetime, timezone
from pathlib import Path
from typing import NoReturn

SCHEMA_VERSION = 1
CHUNK = 1024 * 1024
TABLES = ("term", "synonym", "edge", "xref")

# 共有契約（U10 が読む）。表名・列名・値を変えるときは U10 と一緒に直す。
SCHEMA = """
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE term(
  id     TEXT PRIMARY KEY,
  source TEXT NOT NULL CHECK (source IN ('mondo','mesh')),
  label  TEXT NOT NULL
);
CREATE TABLE synonym(
  term_id TEXT NOT NULL REFERENCES term(id),
  text    TEXT NOT NULL,
  norm    TEXT NOT NULL,
  is_label INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX synonym_norm ON synonym(norm);
CREATE INDEX synonym_term ON synonym(term_id);
CREATE TABLE edge(
  child  TEXT NOT NULL REFERENCES term(id),
  parent TEXT NOT NULL REFERENCES term(id),
  PRIMARY KEY (child, parent)
);
CREATE INDEX edge_parent ON edge(parent);
CREATE TABLE xref(
  term_id TEXT NOT NULL REFERENCES term(id),
  other   TEXT NOT NULL,
  PRIMARY KEY (term_id, other)
);
CREATE INDEX xref_other ON xref(other);
"""

MONDO_LICENSE = "CC BY 4.0"
MONDO_ATTRIBUTION = "Mondo Disease Ontology (https://mondo.monarchinitiative.org/), CC BY 4.0"
MESH_LICENSE = "NLM Terms and Conditions"
MESH_ATTRIBUTION = "Courtesy of the U.S. National Library of Medicine"

SYNONYM_RE = re.compile(r'^synonym: "((?:[^"\\]|\\.)*)"')


def fail(code: int, msg: str) -> NoReturn:
    print(msg, file=sys.stderr)
    sys.exit(code)


def norm(text: str) -> str:
    """検索用の正規化。NFKC → casefold → 連続する空白を 1 つにして前後を除く。"""
    return " ".join(unicodedata.normalize("NFKC", text).casefold().split())


def first_token(value: str) -> str:
    parts = value.split()
    return parts[0] if parts else ""


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(CHUNK):
            digest.update(chunk)
    return digest.hexdigest()


def insert_term(db: sqlite3.Connection, term_id: str, source: str, label: str, synonyms: list[str]) -> None:
    """term と synonym を入れる。label は is_label=1。正規化が同じ別名は 1 つにまとめる。"""
    db.execute("INSERT INTO term(id, source, label) VALUES (?,?,?)", (term_id, source, label))
    seen = set()
    for text, is_label in [(label, 1), *((s, 0) for s in synonyms)]:
        key = norm(text)
        if not key or key in seen:
            continue
        seen.add(key)
        db.execute(
            "INSERT INTO synonym(term_id, text, norm, is_label) VALUES (?,?,?,?)",
            (term_id, text, key, is_label),
        )


def obo_data_version(path: Path) -> str:
    """OBO のヘッダの data-version（例 releases/2026-10-06）。[Term] 節より前だけを読む。"""
    with path.open(encoding="utf-8") as f:
        for line in f:
            if line.startswith("["):
                break
            if line.startswith("data-version: "):
                return line[len("data-version: "):].strip()
    raise ValueError(f"{path.name} に data-version がありません")


def iter_obo_terms(path: Path):
    """[Term] 節を 1 つずつ dict にして返す。全体を 1 つの文字列にしない。"""
    cur = None
    with path.open(encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if line.startswith("["):
                if cur is not None:
                    yield cur
                cur = {"id": None, "name": None, "syn": [], "is_a": [], "xref": [], "obsolete": False} if line == "[Term]" else None
            elif cur is None:
                continue
            elif line.startswith("id: "):
                cur["id"] = first_token(line[4:])
            elif line.startswith("name: "):
                cur["name"] = line[6:].strip()
            elif line.startswith("synonym: "):
                m = SYNONYM_RE.match(line)
                if m:
                    cur["syn"].append(re.sub(r"\\(.)", r"\1", m.group(1)))
            elif line.startswith("is_a: "):
                cur["is_a"].append(first_token(line[6:]))
            elif line.startswith("xref: "):
                cur["xref"].append(first_token(line[6:]))
            elif line.startswith("is_obsolete: true"):
                cur["obsolete"] = True
    if cur is not None:
        yield cur


def build_mondo(db: sqlite3.Connection, path: Path) -> int:
    """MONDO の語を入れ、件数を返す。is_a は相手が MONDO の語で、廃止でないものだけ edge にする。"""
    ids = set()
    links = []
    for t in iter_obo_terms(path):
        if t["obsolete"] or not (t["id"] or "").startswith("MONDO:"):
            continue
        ids.add(t["id"])
        insert_term(db, t["id"], "mondo", t["name"] or "", t["syn"])
        db.executemany(
            "INSERT OR IGNORE INTO xref(term_id, other) VALUES (?,?)",
            [(t["id"], x) for x in t["xref"] if x],
        )
        links += [(t["id"], p) for p in t["is_a"] if p.startswith("MONDO:")]
    # 親は全部読んでから確かめる（後ろの節を親にすることがあるため）。
    db.executemany(
        "INSERT OR IGNORE INTO edge(child, parent) VALUES (?,?)",
        [(c, p) for c, p in links if p in ids],
    )
    return len(ids)


def build_mesh(db: sqlite3.Connection, path: Path) -> int:
    """MeSH の記述子を 1 件ずつ入れ、件数を返す。木番号の親子は全部読んだあとに入れる。"""
    count = 0
    links = []  # (記述子 ID, 木番号)
    for _, elem in ET.iterparse(path, events=("end",)):
        if elem.tag != "DescriptorRecord":
            continue
        ui = elem.findtext("DescriptorUI")
        if ui:
            term_id = f"MESH:{ui}"
            synonyms = [s.text or "" for s in elem.findall("ConceptList/Concept/TermList/Term/String")]
            insert_term(db, term_id, "mesh", elem.findtext("DescriptorName/String") or "", synonyms)
            links += [(term_id, (tn.text or "").strip()) for tn in elem.findall("TreeNumberList/TreeNumber")]
            count += 1
        elem.clear()
    owner = {tn: term_id for term_id, tn in links}
    edges = []
    for term_id, tn in links:
        if "." not in tn:
            continue
        parent = owner.get(tn.rsplit(".", 1)[0])
        if parent and parent != term_id:
            edges.append((term_id, parent))
    db.executemany("INSERT OR IGNORE INTO edge(child, parent) VALUES (?,?)", edges)
    return count


def write_db(args: argparse.Namespace, path: Path, built_at: str) -> dict:
    """onto.db を path に書き、manifest の中身を返す。"""
    path.unlink(missing_ok=True)
    db = sqlite3.connect(path)
    try:
        db.executescript(SCHEMA)
        meta = {"schema_version": str(SCHEMA_VERSION), "built_at": built_at}
        sources = []
        if args.mondo:
            version = obo_data_version(args.mondo)
            n = build_mondo(db, args.mondo)
            meta["mondo_version"] = version
            meta["mondo_terms"] = str(n)
            sources.append({
                "name": "mondo",
                "version": version,
                "path": args.mondo.name,
                "sha256": sha256_of(args.mondo),
                "terms": n,
                "license": MONDO_LICENSE,
                "attribution": MONDO_ATTRIBUTION,
            })
        if args.mesh:
            n = build_mesh(db, args.mesh)
            meta["mesh_version"] = args.mesh_version
            meta["mesh_descriptors"] = str(n)
            sources.append({
                "name": "mesh",
                "version": args.mesh_version,
                "path": args.mesh.name,
                "sha256": sha256_of(args.mesh),
                "terms": n,
                "license": MESH_LICENSE,
                "attribution": MESH_ATTRIBUTION,
            })
        db.executemany("INSERT INTO meta(key, value) VALUES (?,?)", meta.items())
        db.commit()
        tables = {t: db.execute(f"SELECT COUNT(*) FROM {t}").fetchone()[0] for t in TABLES}
    finally:
        db.close()
    return {"schema_version": SCHEMA_VERSION, "built_at": built_at, "sources": sources, "tables": tables}


def build(args: argparse.Namespace) -> dict:
    """一時ファイルに作ってから置き換える。途中で失敗しても完成品は壊れない。"""
    built_at = datetime.now(timezone.utc).isoformat(timespec="seconds")
    args.out.mkdir(parents=True, exist_ok=True)
    db_tmp = args.out / "onto.db.tmp"
    manifest_tmp = args.out / "manifest.json.tmp"
    try:
        manifest = write_db(args, db_tmp, built_at)
        manifest_tmp.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    except BaseException:
        db_tmp.unlink(missing_ok=True)
        manifest_tmp.unlink(missing_ok=True)
        raise
    os.replace(db_tmp, args.out / "onto.db")
    os.replace(manifest_tmp, args.out / "manifest.json")
    return manifest


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="MONDO と MeSH から onto.db と manifest.json を作る（前計算）。")
    parser.add_argument("--mondo", type=Path, help="MONDO の OBO ファイル（mondo.obo）")
    parser.add_argument("--mesh", type=Path, help="MeSH の記述子の XML（desc2026.xml）")
    parser.add_argument("--mesh-version", help="MeSH の版（例 2026）。--mesh と一緒に要る")
    parser.add_argument("--out", type=Path, required=True, help="出力フォルダ")
    parser.add_argument("--force", action="store_true", help="既にある onto.db・manifest.json を置き換える")
    args = parser.parse_args(argv)

    if not args.mondo and not args.mesh:
        parser.error("--mondo と --mesh の少なくとも 1 つが要る")
    if args.mesh and not args.mesh_version:
        parser.error("--mesh を使うときは --mesh-version が要る")
    if (args.out / "onto.db").exists() or (args.out / "manifest.json").exists():
        if not args.force:
            fail(4, f"{args.out} に onto.db か manifest.json が既にある（置き換えるなら --force）")

    try:
        manifest = build(args)
    except Exception as exc:  # 何が起きても、標準エラー 1 行と終了コード 1 で終わる。
        fail(1, f"作れなかった: {exc}")
    sys.stdout.write(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
