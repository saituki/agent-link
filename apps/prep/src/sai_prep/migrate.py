"""DB Migration: sai-moon のプロジェクトの論文を、移植の束（zip）にまとめる（共有契約 C-M1）。

移植元の `<root>/.sai/paper/papr.db` と `papers/` は読むだけで、書き込まない（読み取り専用で開く）。
束の中の `papr.db` は、論文の表だけを `CREATE TABLE … AS SELECT` で写した新しい DB である。
"""

from __future__ import annotations

import json
import os
import sqlite3
import tempfile
import zipfile
from pathlib import Path

FORMAT = "sai-paper-migration"
VERSION = 1
# (表, 条件)。{ids} は "?, ?, …" に置き換える。条件が None なら全行。
TABLES = [
    ("papers", "id IN ({ids})"),
    ("authors", "id IN (SELECT author_id FROM src.paper_authors WHERE paper_id IN ({ids}))"),
    ("paper_authors", "paper_id IN ({ids})"),
    ("tags", "id IN (SELECT tag_id FROM src.paper_tags WHERE paper_id IN ({ids}))"),
    ("paper_tags", "paper_id IN ({ids})"),
    ("chunks", "paper_id IN ({ids})"),
    ("chunk_embeddings", "chunk_id IN (SELECT id FROM src.chunks WHERE paper_id IN ({ids}))"),
    ("embedding_models", None),
    ("paper_sections", "paper_id IN ({ids})"),
    ("paper_passages", "paper_id IN ({ids})"),
    ("paper_section_builds", "paper_id IN ({ids})"),
    ("paper_extracts", "papers_id IN ({ids})"),
    ("paper_figure_extracts", "papers_id IN ({ids})"),
    ("paper_figures", "papers_id IN ({ids})"),
    ("paper_refs", "papers_id IN ({ids})"),
    ("paper_sentences", "papers_id IN ({ids})"),
    ("paper_translations", "papers_id IN ({ids})"),
]


class MigrateError(Exception):
    """移植の前に分かった失敗。画面にそのまま出す理由。"""


def paper_db(root: Path) -> Path:
    """sai-moon のプロジェクトの論文 DB のパス。無ければ MigrateError。"""
    base = Path(root).expanduser().resolve()
    db = base / ".sai" / "paper" / "papr.db"
    if not db.is_file():
        raise MigrateError("sai-moon のプロジェクトではありません（.sai/paper/papr.db がありません）")
    return db


def find_paper_dir(papers_dir: Path, paper_id: int) -> Path | None:
    """論文のフォルダ。名前が `0001_…` か `0001` のもののうち、名前の順で最初のもの（sai-tree の規則）。"""
    if not papers_dir.is_dir():
        return None
    prefix = f"{paper_id:04d}_"
    exact = f"{paper_id:04d}"
    for child in sorted(papers_dir.iterdir()):
        if child.is_dir() and (child.name == exact or child.name.startswith(prefix)):
            return child
    return None


def list_papers(root: Path) -> list[dict]:
    """移植元の論文の一覧（新しい順）。読み取り専用。"""
    db = paper_db(root)
    papers_dir = db.parent / "papers"
    try:
        con = _connect_ro(db)
        try:
            papers = con.execute(
                "SELECT id, title, published_at FROM papers ORDER BY created_at DESC, id DESC"
            ).fetchall()
            authors: dict[int, list[str]] = {}
            for paper_id, name in con.execute(
                "SELECT pa.paper_id, a.name FROM paper_authors AS pa "
                "JOIN authors AS a ON a.id = pa.author_id ORDER BY pa.paper_id, pa.position, a.id"
            ):
                authors.setdefault(paper_id, []).append(name)
            chunks: dict[int, int] = {}
            if _has_table(con, "chunks"):
                chunks = dict(con.execute("SELECT paper_id, COUNT(*) FROM chunks GROUP BY paper_id"))
        finally:
            con.close()
    except sqlite3.Error as error:
        raise MigrateError(f"移植元の DB を読めません: {error}") from error

    result = []
    for paper_id, title, published_at in papers:
        folder = find_paper_dir(papers_dir, paper_id)
        result.append(
            {
                "id": paper_id,
                "title": title or "",
                "year": _year(published_at),
                "authors": ", ".join(authors.get(paper_id, [])),
                "has_pdf": folder is not None and (folder / "paper.pdf").is_file(),
                "has_extract": folder is not None and (folder / "extract.md").is_file(),
                "translated": folder is not None and (folder / "paper-ja.pdf").is_file(),
                "chunks": chunks.get(paper_id, 0),
            }
        )
    return result


def build_package(root: Path, ids: list[int], out_zip: Path) -> None:
    """ids の論文の移植の束（zip）を out_zip に作る。移植元は読むだけ。

    manifest の `papers` は、移植元の papers に実際にあった id だけ。1 つも無ければ MigrateError。
    """
    db = paper_db(root)
    papers_dir = db.parent / "papers"
    with tempfile.TemporaryDirectory() as tmp:
        packed = Path(tmp) / "papr.db"
        con = sqlite3.connect(packed.as_uri(), uri=True)
        try:
            con.execute("ATTACH DATABASE ? AS src", (_ro_uri(db),))
            if not _has_table(con, "papers", schema="src"):
                raise MigrateError("移植元の DB に papers の表がありません")
            asked = [int(i) for i in ids]
            found = {
                int(row[0])
                for row in con.execute(
                    f"SELECT id FROM src.papers WHERE id IN ({_marks(asked)})", asked
                )
            }
            keep = [i for i in dict.fromkeys(asked) if i in found]
            if not keep:
                raise MigrateError("移す論文が移植元にありません")
            for table, cond in TABLES:
                if not _has_table(con, table, schema="src"):
                    continue
                sql = f"CREATE TABLE {table} AS SELECT * FROM src.{table}"
                params: list[int] = []
                if cond is not None:
                    sql += " WHERE " + cond.replace("{ids}", _marks(keep))
                    params = keep * cond.count("{ids}")
                con.execute(sql, params)
            con.commit()
            con.execute("DETACH DATABASE src")
        except sqlite3.Error as error:
            raise MigrateError(f"移植元の DB を読めません: {error}") from error
        finally:
            con.close()

        manifest = {"format": FORMAT, "version": VERSION, "papers": keep}
        with zipfile.ZipFile(out_zip, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.writestr(
                "manifest.json", json.dumps(manifest, ensure_ascii=False, indent=2) + "\n"
            )
            archive.write(packed, "papr.db")
            for paper_id in keep:
                folder = find_paper_dir(papers_dir, paper_id)
                if folder is None:
                    continue
                _add_folder(archive, folder)


def _add_folder(archive: zipfile.ZipFile, folder: Path) -> None:
    """論文のフォルダの中身を入れる。シンボリックリンクは入れない。"""
    for dirpath, _dirnames, filenames in os.walk(folder):
        for name in sorted(filenames):
            path = Path(dirpath) / name
            if path.is_symlink() or not path.is_file():
                continue
            relative = path.relative_to(folder).as_posix()
            archive.write(path, f"papers/{folder.name}/{relative}")


def _ro_uri(db: Path) -> str:
    return f"{db.as_uri()}?mode=ro"


def _connect_ro(db: Path) -> sqlite3.Connection:
    return sqlite3.connect(_ro_uri(db), uri=True)


def _has_table(con: sqlite3.Connection, name: str, schema: str = "main") -> bool:
    row = con.execute(
        f"SELECT 1 FROM {schema}.sqlite_master WHERE type = 'table' AND name = ?", (name,)
    ).fetchone()
    return row is not None


def _marks(values: list) -> str:
    return ", ".join("?" for _ in values)


def _year(published_at: str | None) -> int | None:
    text = str(published_at or "")[:4]
    return int(text) if text.isdigit() else None
