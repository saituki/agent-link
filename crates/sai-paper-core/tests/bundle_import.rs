//! U12: 前処理の束（sai-prep bundle）の取り込みの結合テスト。
//!
//! 一時フォルダの `papr.db` に論文を作り、C12 の形の束（manifest.json・PDF・
//! docling の md/json/図・翻訳 PDF）を置いて `bundle::import_bundle` を呼ぶ。
//! docling も pdf2zh も呼ばずに、図・節・引用の表ができることを確かめる。
//!
//! 注意: この crate は edition 2024 で `unsafe_code = "forbid"` のため、
//! `std::env::set_var` で偽の docling/pdf2zh を指すことができない。代わりに
//! 「docling が無くても取り込みが成功する」ことと、docling の一時フォルダ
//! （`.docling-figures-tmp`）が残らないことで、docling を呼んでいないことを確かめる。
//! CLI 側のテスト（`crates/sai-paper-cli/tests/bundle_cli.rs`）では子プロセスの
//! 環境変数に偽の bin を渡し、印のファイルが作られないことを確かめる。

use std::path::{Path, PathBuf};

use sai_paper_core::bundle::{BundleError, import_bundle};
use sai_paper_core::{Database, paths, pipeline};
use sha2::{Digest, Sha256};

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
            "sai-paper-bundle-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// docling の Markdown（見出しと References を含む）。
const MARKDOWN: &str = "# Introduction\n\nAs shown in Figure 1, the method works well. Figure 2 shows the results.\n\n# Method\n\nWe use a transformer [1].\n\n# References\n\n[1] Vaswani, A. et al. 2017. Attention is all you need.\n[2] Doe, J. 2020. A study.\n";

/// docling の JSON（図 2 つ。画像は `docling/paper_artifacts/` に置く）。
const DOCLING_JSON: &str = r##"{
  "texts": [
    {"text": "Figure 1: The workflow."},
    {"text": "Figure 2: Results."}
  ],
  "pictures": [
    {"image": {"uri": "paper_artifacts/fig1.png"}, "captions": [{"$ref": "#/texts/0"}], "prov": [{"page_no": 1}]},
    {"image": {"uri": "paper_artifacts/fig2.png"}, "captions": [{"$ref": "#/texts/1"}], "prov": [{"page_no": 2}]}
  ]
}"##;

/// 小さな PDF（`%PDF-` で始まれば取り込みの検査を通る）。
fn pdf_bytes(body: &str) -> Vec<u8> {
    format!("%PDF-1.4 {body}\n%%EOF\n").into_bytes()
}

/// バイト列の SHA-256（小文字 16 進）。
fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// C12 の形の束を 1 つ作る。`target` は manifest の `target` の JSON。
fn write_bundle(dir: &Path, target: serde_json::Value, body: &str, translated: bool) -> Vec<u8> {
    std::fs::create_dir_all(dir.join("docling/paper_artifacts")).unwrap();
    std::fs::create_dir_all(dir.join("translated")).unwrap();
    let pdf = pdf_bytes(body);
    std::fs::write(dir.join("paper.pdf"), &pdf).unwrap();
    std::fs::write(dir.join("docling/paper.md"), MARKDOWN).unwrap();
    std::fs::write(dir.join("docling/paper.json"), DOCLING_JSON).unwrap();
    std::fs::write(dir.join("docling/paper_artifacts/fig1.png"), b"\x89PNG-one").unwrap();
    std::fs::write(dir.join("docling/paper_artifacts/fig2.png"), b"\x89PNG-two").unwrap();
    let translation = if translated {
        std::fs::write(dir.join("translated/paper-mono.pdf"), pdf_bytes("mono")).unwrap();
        std::fs::write(dir.join("translated/paper-dual.pdf"), pdf_bytes("dual")).unwrap();
        serde_json::json!({
            "mono": "translated/paper-mono.pdf",
            "dual": "translated/paper-dual.pdf",
            "service": "mlx:test",
        })
    } else {
        serde_json::Value::Null
    };
    let manifest = serde_json::json!({
        "format": "sai-prep-bundle",
        "version": 1,
        "target": target,
        "pdf": "paper.pdf",
        "pdf_sha256": sha256_hex(&pdf),
        "docling": {"md": "docling/paper.md", "json": "docling/paper.json", "dir": "docling"},
        "translation": translation,
        "tools": {"sai_prep": "0.1.0"},
        "created_at": "2026-10-07T12:00:00+09:00",
    });
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    pdf
}

/// 論文 1 件（PDF 無し）を持つ DB を作る。
fn db_with_paper(data_dir: &Path, title: &str, doi: Option<&str>) -> (Database, i64) {
    let db = Database::open(&data_dir.join("papr.db")).unwrap();
    match doi {
        Some(doi) => db
            .connection()
            .execute(
                "INSERT INTO papers (title, doi) VALUES (?1, ?2)",
                rusqlite::params![title, doi],
            )
            .unwrap(),
        None => db
            .connection()
            .execute("INSERT INTO papers (title) VALUES (?1)", [title])
            .unwrap(),
    };
    let paper_id = db.connection().last_insert_rowid();
    (db, paper_id)
}

/// `paper_figure_extracts` の状態を読む。
fn figure_status(db: &Database, paper_id: i64) -> Option<String> {
    db.connection()
        .query_row(
            "SELECT status FROM paper_figure_extracts WHERE papers_id = ?1",
            [paper_id],
            |row| row.get(0),
        )
        .ok()
}

/// `paper_extracts` の状態を読む。
fn extract_status(db: &Database, paper_id: i64) -> Option<String> {
    db.connection()
        .query_row(
            "SELECT status FROM paper_extracts WHERE papers_id = ?1",
            [paper_id],
            |row| row.get(0),
        )
        .ok()
}

/// AC1: Local の論文に束を取り込むと、docling・pdf2zh を呼ばずに
/// 図・節・引用・翻訳の表ができ、DL 後の自動処理が済みになる。
#[test]
fn import_bundle_for_local_paper_builds_tables() {
    let temp = Temp::new("local");
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let (mut db, paper_id) = db_with_paper(&data_dir, "Local Paper", Some("10.1/local"));

    let bundle_dir = temp.path().join("bundle");
    write_bundle(
        &bundle_dir,
        serde_json::json!({"paper_id": paper_id}),
        "local-body",
        true,
    );

    let result = import_bundle(&mut db, &data_dir, &bundle_dir).expect("取り込める");
    assert_eq!(result.paper_id, paper_id);
    assert_eq!(result.figures, 2, "図が 2 件");
    assert!(result.sections >= 1, "節が 1 つ以上");
    assert!(result.refs >= 1, "引用が 1 件以上");
    assert!(result.translated, "翻訳を取り込んだ");

    // ファイルが論文フォルダにできている。
    let folder = paths::paper_folder(&data_dir, paper_id, "Local Paper");
    assert!(folder.join("paper.pdf").is_file(), "paper.pdf");
    assert!(folder.join("extract.md").is_file(), "extract.md");
    assert!(folder.join("figures/fig-001.png").is_file(), "fig-001.png");
    assert!(folder.join("figures/fig-002.png").is_file(), "fig-002.png");
    assert!(folder.join("paper-ja.pdf").is_file(), "paper-ja.pdf");
    assert!(folder.join("paper-dual.pdf").is_file(), "paper-dual.pdf");
    // docling の一時フォルダは残らない（docling を呼んでいない）。
    assert!(!folder.join(".docling-figures-tmp").exists());

    // 表ができている。
    let figures: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM paper_figures WHERE papers_id = ?1",
            [paper_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(figures, 2, "paper_figures が 2 行");
    assert_eq!(figure_status(&db, paper_id).as_deref(), Some("ready"));
    assert_eq!(extract_status(&db, paper_id).as_deref(), Some("ready"));
    let translation: Option<String> = db
        .connection()
        .query_row(
            "SELECT status FROM paper_translations WHERE papers_id = ?1",
            [paper_id],
            |row| row.get(0),
        )
        .ok();
    assert_eq!(translation.as_deref(), Some("ready"));
    let sections: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM paper_sections WHERE paper_id = ?1",
            [paper_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(sections >= 1, "paper_sections が 1 行以上");

    // DL 後の自動処理は図も翻訳も済みになる。
    assert_eq!(pipeline::pending_after_download(&db, paper_id), (false, false));
}

/// AC2: `target.bibtex` の束で新しい論文ができる。
#[test]
fn import_bundle_for_bibtex_creates_a_paper() {
    let temp = Temp::new("bibtex");
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let mut db = Database::open(&data_dir.join("papr.db")).unwrap();

    let bibtex = "@article{vaswani2017,\n  title = {Attention Is All You Need},\n  author = {Vaswani, Ashish and Shazeer, Noam},\n  year = {2017},\n  doi = {10.1000/xyz},\n}\n";
    let bundle_dir = temp.path().join("bundle");
    write_bundle(
        &bundle_dir,
        serde_json::json!({"bibtex": bibtex}),
        "bibtex-body",
        false,
    );

    let result = import_bundle(&mut db, &data_dir, &bundle_dir).expect("取り込める");
    assert_eq!(result.title, "Attention Is All You Need");
    assert!(!result.translated, "翻訳は無い");
    let count: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM papers", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "論文が 1 件できる");
    let doi: Option<String> = db
        .connection()
        .query_row(
            "SELECT doi FROM papers WHERE id = ?1",
            [result.paper_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(doi.as_deref(), Some("10.1000/xyz"));
    // 図の表もできている。
    assert_eq!(result.figures, 2);
    assert_eq!(extract_status(&db, result.paper_id).as_deref(), Some("ready"));
}

/// AC2: `pdf_sha256` の不一致と `../` のパスは誤り（何も書かない）。
#[test]
fn import_bundle_rejects_a_bad_manifest() {
    let temp = Temp::new("bad");
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let (mut db, paper_id) = db_with_paper(&data_dir, "Local Paper", Some("10.1/local"));

    // SHA-256 の不一致。
    let dir = temp.path().join("bad-sha");
    write_bundle(&dir, serde_json::json!({"paper_id": paper_id}), "body", false);
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    manifest["pdf_sha256"] = serde_json::json!("0".repeat(64));
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
    let error = import_bundle(&mut db, &data_dir, &dir).expect_err("SHA が違えば失敗する");
    assert!(matches!(error, BundleError::Manifest(_)), "Manifest: {error}");

    // `..` を含むパス。
    let dir = temp.path().join("bad-dotdot");
    write_bundle(&dir, serde_json::json!({"paper_id": paper_id}), "body2", false);
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    manifest["pdf"] = serde_json::json!("../paper.pdf");
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
    let error = import_bundle(&mut db, &data_dir, &dir).expect_err("`..` は失敗する");
    assert!(matches!(error, BundleError::Manifest(_)), "Manifest: {error}");

    // どちらも PDF を取り込んでいない。
    let folder = paths::paper_folder(&data_dir, paper_id, "Local Paper");
    assert!(!folder.join("paper.pdf").exists(), "PDF を書いていない");
}

/// AC2: PDF がある論文への 2 回目は「この論文にはもう PDF があります」。
#[test]
fn import_bundle_rejects_a_paper_that_already_has_a_pdf() {
    let temp = Temp::new("exists");
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let (mut db, paper_id) = db_with_paper(&data_dir, "Local Paper", Some("10.1/local"));

    let first = temp.path().join("first");
    write_bundle(&first, serde_json::json!({"paper_id": paper_id}), "one", false);
    import_bundle(&mut db, &data_dir, &first).expect("1 回目は取り込める");

    let second = temp.path().join("second");
    write_bundle(&second, serde_json::json!({"paper_id": paper_id}), "two", false);
    let error = import_bundle(&mut db, &data_dir, &second).expect_err("2 回目は失敗する");
    assert!(matches!(error, BundleError::PdfExists(_)), "PdfExists: {error}");
    assert!(error.to_string().contains("もう PDF があります"));
}
