//! 論文の移植（書き出し `export` と取り込み `import`）の結合テスト。
//!
//! 一時フォルダに移植元と移植先の 2 つの `data_dir` を作り、`Database::open` で DB を作る。
//! 移植元には PDF・図・翻訳・chunk と埋め込み・節・引用を持つ論文 1 本と、書誌だけの論文 1 本を入れる。

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::params;
use sai_paper_core::migration::{self, MigrationError};
use sai_paper_core::sections::{self, BuildState};
use sai_paper_core::{Database, figures, paths};

/// テストごとの一時フォルダ（`Drop` で消す）。
struct Temp {
    dir: PathBuf,
}

impl Temp {
    fn new(tag: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-migration-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// 移植元の extract.md（見出しを含む。節の表ができる）。
const MARKDOWN: &str = "# Introduction\n\nThe zebra quartz method works well [1].\n\n# Method\n\nWe use a transformer.\n\n# References\n\n[1] Vaswani, A. 2017. Attention is all you need.\n";

/// 移植元の chunk の埋め込み（バイト列をそのまま写すことを確かめる）。
const VEC: [u8; 8] = [0, 0, 128, 63, 0, 0, 0, 64];

/// 論文 1 件を入れて id を返す。
fn insert_paper(db: &Database, title: &str, doi: Option<&str>) -> i64 {
    db.connection()
        .execute(
            "INSERT INTO papers (title, doi) VALUES (?1, ?2)",
            params![title, doi],
        )
        .unwrap();
    db.connection().last_insert_rowid()
}

/// 移植元を作る。PDF などを持つ `Alpha Study` と、書誌だけの `Beta Notes` の id を返す。
fn seed_source(data_dir: &Path) -> (Database, i64, i64) {
    let db = Database::open(&data_dir.join("papr.db")).unwrap();
    let id1 = insert_paper(&db, "Alpha Study", Some("10.1000/alpha"));
    let folder = paths::paper_folder(data_dir, id1, "Alpha Study");
    fs::create_dir_all(folder.join("figures")).unwrap();
    fs::write(folder.join("paper.pdf"), b"%PDF-1.4 alpha").unwrap();
    fs::write(folder.join("extract.md"), MARKDOWN).unwrap();
    fs::write(folder.join("note.md"), "source note").unwrap();
    fs::write(folder.join("paper-ja.pdf"), b"%PDF-1.4 ja").unwrap();
    fs::write(folder.join("figures/fig-001.png"), b"\x89PNG-alpha").unwrap();
    let stored = |name: &str| paths::to_stored_path(data_dir, folder.join(name));
    let pdf = stored("paper.pdf");
    let md = stored("extract.md");
    let ja = stored("paper-ja.pdf");
    let fig = stored("figures/fig-001.png");

    let conn = db.connection();
    conn.execute(
        "UPDATE papers SET pdf_path = ?1, content_hash = 'hash-alpha', file_size = 13 WHERE id = ?2",
        params![pdf, id1],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO authors (name, openalex_id, orcid) VALUES ('Doe, Jane', 'A1', NULL)",
        [],
    )
    .unwrap();
    let author = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO paper_authors (paper_id, author_id, position) VALUES (?1, ?2, 1)",
        params![id1, author],
    )
    .unwrap();
    conn.execute("INSERT INTO tags (name) VALUES ('ml')", [])
        .unwrap();
    let tag = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO paper_tags (paper_id, tag_id) VALUES (?1, ?2)",
        params![id1, tag],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO embedding_models (name, endpoint, dim, created_at) VALUES ('m1', 'http://x', 2, '2026-10-01')",
        [],
    )
    .unwrap();
    let model = conn.last_insert_rowid();
    for (ord, text) in [(0, "zebra quartz method"), (1, "second chunk words")] {
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at)
             VALUES ('body', 'p1', ?1, ?2, ?3, 0, 10, 'sentence', '2026-10-01')",
            params![id1, ord, text],
        )
        .unwrap();
        let chunk = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunk_embeddings (chunk_id, model_id, vec) VALUES (?1, ?2, ?3)",
            params![chunk, model, VEC.to_vec()],
        )
        .unwrap();
    }

    let key = figures::figure_key_for_paper(conn, id1).unwrap();
    conn.execute(
        "INSERT INTO paper_extracts (paper_id, status, pdf_path, pdf_mtime, md_path, papers_id)
         VALUES (?1, 'ready', ?2, '1', ?3, ?4)",
        params![key, pdf, md, id1],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO paper_figures (paper_id, papers_id, fig_no, label, caption, image_path, created_at)
         VALUES (?1, ?2, 1, 'Figure 1', 'The workflow.', ?3, '2026-10-01')",
        params![key, id1, fig],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO paper_translations (paper_id, status, pdf_path, pdf_mtime, pdf_ja_path, service, papers_id)
         VALUES (?1, 'ready', ?2, '1', ?3, 'mlx:test', ?4)",
        params![key, pdf, ja, id1],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO paper_refs (paper_id, ref_no, raw_text, papers_id) VALUES (?1, 1, 'Vaswani 2017', ?2)",
        params![key, id1],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO paper_sentences (paper_id, sent_no, text, cite_refs, papers_id)
         VALUES (?1, 1, 'We use a method [1].', '[1]', ?2)",
        params![key, id1],
    )
    .unwrap();
    sections::ensure(conn, data_dir, id1).unwrap();

    let id2 = insert_paper(&db, "Beta Notes", Some("10.1000/beta"));
    (db, id1, id2)
}

/// 論文フォルダの中身を含めて確かめるための、移植先の保存パスの解決。
fn resolved(data_dir: &Path, stored: &str) -> PathBuf {
    paths::resolve_stored_path(data_dir, stored)
}

/// 移植先の論文の、保存パス 1 つを読む。
fn stored_path(db: &Database, sql: &str, id: i64) -> String {
    db.connection()
        .query_row(sql, [id], |row| row.get(0))
        .unwrap()
}

/// 移植で新しく作った論文が、ファイル・派生の表・埋め込み・検索まで届くこと。
#[test]
fn export_then_import_moves_paper_with_files_and_rows() {
    let temp = Temp::new("move");
    let src = temp.path().join("src");
    let dst = temp.path().join("dst");
    let pkg = temp.path().join("pkg");
    let (src_db, id1, id2) = seed_source(&src);
    drop(src_db);

    let export = migration::export(&src.join("papr.db"), &src, &[id1, id2], &pkg).unwrap();
    assert_eq!(export.papers, vec![id1, id2]);
    assert!(export.missing.is_empty());
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(pkg.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["format"], "sai-paper-migration");
    assert_eq!(manifest["version"], 1);
    assert!(pkg.join("papr.db").is_file());

    // 移植先では id をずらして、id が振り直されることを確かめる。
    let mut dst_db = Database::open(&dst.join("papr.db")).unwrap();
    insert_paper(&dst_db, "Existing Paper", Some("10.1000/other"));
    let report = migration::import(&mut dst_db, &dst, &pkg).unwrap();
    assert!(report.failed.is_empty(), "failed: {:?}", report.failed);
    assert!(report.skipped.is_empty());
    assert_eq!(report.imported.len(), 2);
    let alpha = report.imported.iter().find(|i| i.source_id == id1).unwrap();
    let beta = report.imported.iter().find(|i| i.source_id == id2).unwrap();
    assert_eq!(alpha.mode, "new");
    assert_eq!(beta.mode, "new");
    assert_ne!(alpha.paper_id, id1);
    assert_eq!(alpha.title, "Alpha Study");

    let new_id = alpha.paper_id;
    let pdf = stored_path(&dst_db, "SELECT pdf_path FROM papers WHERE id = ?1", new_id);
    assert!(pdf.contains(&format!("{new_id:04}_")), "pdf_path: {pdf}");
    assert!(resolved(&dst, &pdf).is_file());
    assert_eq!(
        fs::read_to_string(resolved(&dst, &pdf).with_file_name("note.md")).unwrap(),
        "source note"
    );

    let md = stored_path(
        &dst_db,
        "SELECT md_path FROM paper_extracts WHERE papers_id = ?1",
        new_id,
    );
    assert!(resolved(&dst, &md).is_file());
    assert!(md.contains(&format!("{new_id:04}_")), "md_path: {md}");

    let image = stored_path(
        &dst_db,
        "SELECT image_path FROM paper_figures WHERE papers_id = ?1",
        new_id,
    );
    assert!(resolved(&dst, &image).is_file());
    assert!(
        image.contains(&format!("{new_id:04}_")),
        "image_path: {image}"
    );

    let ja = stored_path(
        &dst_db,
        "SELECT pdf_ja_path FROM paper_translations WHERE papers_id = ?1",
        new_id,
    );
    assert!(resolved(&dst, &ja).is_file());

    let vec: Vec<u8> = dst_db
        .connection()
        .query_row(
            "SELECT e.vec FROM chunk_embeddings e JOIN chunks c ON c.id = e.chunk_id
             WHERE c.paper_id = ?1 AND c.ord = 0",
            [new_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(vec, VEC.to_vec());

    let conn = dst_db.connection();
    let chunk_hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM chunk_fts WHERE chunk_fts MATCH 'quartz'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(chunk_hits, 1);
    let title_hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM paper_search WHERE paper_search MATCH 'Alpha'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(title_hits, 1);

    let authors: i64 = conn
        .query_row(
            "SELECT count(*) FROM paper_authors pa JOIN authors a ON a.id = pa.author_id
             WHERE pa.paper_id = ?1 AND a.name = 'Doe, Jane'",
            [new_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(authors, 1);
    let tags: i64 = conn
        .query_row(
            "SELECT count(*) FROM paper_tags pt JOIN tags t ON t.id = pt.tag_id
             WHERE pt.paper_id = ?1 AND t.name = 'ml'",
            [new_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tags, 1);

    // 節は作り直されず、extract.md の sha が写っているので Fresh。
    assert!(matches!(
        sections::ensure(conn, &dst, new_id).unwrap(),
        BuildState::Fresh
    ));
    let sections_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM paper_sections WHERE paper_id = ?1",
            [new_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(sections_count > 0);
}

/// 同じ束をもう一度取り込むと、PDF のある論文は飛ばし、書誌だけの論文は補う（id は増えない）。
#[test]
fn import_again_skips_pdf_paper_and_fills_metadata_only_paper() {
    let temp = Temp::new("again");
    let src = temp.path().join("src");
    let dst = temp.path().join("dst");
    let pkg = temp.path().join("pkg");
    let (src_db, id1, id2) = seed_source(&src);
    drop(src_db);
    migration::export(&src.join("papr.db"), &src, &[id1, id2], &pkg).unwrap();

    let mut dst_db = Database::open(&dst.join("papr.db")).unwrap();
    let first = migration::import(&mut dst_db, &dst, &pkg).unwrap();
    let alpha_id = first
        .imported
        .iter()
        .find(|i| i.source_id == id1)
        .unwrap()
        .paper_id;
    let beta_id = first
        .imported
        .iter()
        .find(|i| i.source_id == id2)
        .unwrap()
        .paper_id;

    let again = migration::import(&mut dst_db, &dst, &pkg).unwrap();
    assert!(again.failed.is_empty(), "failed: {:?}", again.failed);
    assert_eq!(again.skipped.len(), 1);
    assert_eq!(again.skipped[0].source_id, id1);
    assert_eq!(again.skipped[0].paper_id, alpha_id);
    assert_eq!(again.skipped[0].reason, "既に PDF があります");
    assert_eq!(again.imported.len(), 1);
    assert_eq!(again.imported[0].source_id, id2);
    assert_eq!(again.imported[0].paper_id, beta_id);
    assert_eq!(again.imported[0].mode, "filled");

    let count: i64 = dst_db
        .connection()
        .query_row("SELECT count(*) FROM papers", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2, "重複で id が増えないこと");
}

/// 移植先に同じ DOI で PDF 待ちの論文があれば、その id のまま補い、移植先の note.md を守る。
#[test]
fn pdf_waiting_paper_is_filled_in_place_and_keeps_its_note() {
    let temp = Temp::new("fill");
    let src = temp.path().join("src");
    let dst = temp.path().join("dst");
    let pkg = temp.path().join("pkg");
    let (src_db, id1, _id2) = seed_source(&src);
    drop(src_db);
    migration::export(&src.join("papr.db"), &src, &[id1], &pkg).unwrap();

    let mut dst_db = Database::open(&dst.join("papr.db")).unwrap();
    // 大文字の DOI でも同じ論文とみなす（DOI は NOCASE）。
    let waiting = insert_paper(&dst_db, "Alpha Study", Some("10.1000/ALPHA"));
    let folder = paths::paper_folder(&dst, waiting, "Alpha Study");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("note.md"), "destination note").unwrap();

    let report = migration::import(&mut dst_db, &dst, &pkg).unwrap();
    assert!(report.failed.is_empty(), "failed: {:?}", report.failed);
    assert_eq!(report.imported.len(), 1);
    assert_eq!(report.imported[0].paper_id, waiting);
    assert_eq!(report.imported[0].mode, "filled");

    assert_eq!(
        fs::read_to_string(folder.join("note.md")).unwrap(),
        "destination note"
    );
    let pdf = stored_path(
        &dst_db,
        "SELECT pdf_path FROM papers WHERE id = ?1",
        waiting,
    );
    assert!(resolved(&dst, &pdf).is_file());
    let figure_image = stored_path(
        &dst_db,
        "SELECT image_path FROM paper_figures WHERE papers_id = ?1",
        waiting,
    );
    assert!(resolved(&dst, &figure_image).is_file());
}

/// 書き出しの失敗は終了コードで分かれる（2 = 入力の誤り・3 = 見つからない）。
/// 失敗した書き出し先は作らない・残さない。
#[test]
fn export_errors_have_exit_codes_and_leave_no_folder() {
    let temp = Temp::new("errors");
    let src = temp.path().join("src");
    let (src_db, id1, _id2) = seed_source(&src);
    drop(src_db);
    let db_file = src.join("papr.db");

    let existing = temp.path().join("existing");
    fs::create_dir_all(&existing).unwrap();
    let error = migration::export(&db_file, &src, &[id1], &existing).unwrap_err();
    assert_eq!(error.code(), 2);
    assert!(matches!(error, MigrationError::Input(_)));
    assert!(existing.is_dir(), "既存のフォルダは消さない");

    let empty = migration::export(&db_file, &src, &[], &temp.path().join("none")).unwrap_err();
    assert_eq!(empty.code(), 2);

    let out = temp.path().join("missing-ids");
    let error = migration::export(&db_file, &src, &[9999], &out).unwrap_err();
    assert_eq!(error.code(), 3);
    assert!(!out.exists(), "1 つも見つからないときは作らない");
}

/// 束の形が違うと入力の誤り（2）。
#[test]
fn import_rejects_a_folder_that_is_not_a_migration_bundle() {
    let temp = Temp::new("bad");
    let dst = temp.path().join("dst");
    let not_bundle = temp.path().join("not-bundle");
    fs::create_dir_all(&not_bundle).unwrap();
    fs::write(
        not_bundle.join("manifest.json"),
        r#"{"format":"sai-prep-bundle","version":1,"papers":[1]}"#,
    )
    .unwrap();
    let mut dst_db = Database::open(&dst.join("papr.db")).unwrap();

    let error = migration::import(&mut dst_db, &dst, &not_bundle).unwrap_err();
    assert_eq!(error.code(), 2);
    let no_manifest = migration::import(&mut dst_db, &dst, &temp.path().join("none")).unwrap_err();
    assert_eq!(no_manifest.code(), 2);
}
