//! 段階 1 の手動確認: 本物の DB の**コピー**にマイグレーションを当て、
//! 索引済みの論文を `delete_paper` で消せることを確かめるテスト。
//!
//! 本物の `.sai/paper/papr.db` には一切書き込まない（コピーの側だけを開く）。
//!
//! 使い方:
//! ```text
//! sqlite3 "$SAI_PROJECT_ROOT/.sai/paper/papr.db" ".backup /tmp/stage1-check.db"
//! cargo test -p sai-paper-core --test real_db_delete -- --ignored --nocapture
//! ```
//!
//! コピーの場所は環境変数 `SAI_PAPER_DB_COPY` で差し替えられる。

use sai_paper_core::Database;

/// 確認対象の DB コピーのパス。
fn db_copy_path() -> std::path::PathBuf {
    std::env::var_os("SAI_PAPER_DB_COPY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/stage1-check.db"))
}

/// コピー上の論文 id（既定は計画書の例と同じ id 3）。
fn target_paper_id() -> i64 {
    std::env::var("SAI_PAPER_DB_COPY_PAPER_ID")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3)
}

#[test]
#[ignore = "本物の DB のコピー（/tmp/stage1-check.db）を事前用意して手動で実行する"]
fn delete_indexed_paper_on_real_db_copy() -> Result<(), Box<dyn std::error::Error>> {
    let path = db_copy_path();
    assert!(
        path.is_file(),
        "DB コピーが見つからない: {}（sqlite3 \".backup\" で作ること）",
        path.display()
    );

    // コピーに対してのみ開く。ここでマイグレーション 0033 が適用される。
    let db = Database::open(&path)?;
    let conn = db.connection();

    // マイグレーション 0033 まで適用されたこと。
    let applied: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    println!("適用済みマイグレーション: 最大 version = {applied}");
    assert_eq!(applied, 33, "マイグレーション 0033 が適用されていない");

    // 索引（chunks）と、papers_id で結び付く 6 表の外部キーが CASCADE になったこと。
    let fk_sql: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT sql FROM sqlite_master
             WHERE name IN ('chunks', 'paper_extracts', 'paper_figures',
                            'paper_refs', 'paper_sentences', 'paper_translations',
                            'paper_figure_extracts')",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<_, _>>()?
    };
    assert_eq!(
        fk_sql.len(),
        7,
        "表定義の取得が揃っていない（{} 件）",
        fk_sql.len()
    );
    for sql in &fk_sql {
        assert!(
            sql.contains("ON DELETE CASCADE"),
            "CASCADE になっていない表定義: {sql}"
        );
    }
    println!("chunks + 6 表: ON DELETE CASCADE を確認");

    // 既存行への papers_id の埋め込み（段階 2c）が済んでいること。
    for table in [
        "paper_refs",
        "paper_sentences",
        "paper_extracts",
        "paper_figures",
        "paper_figure_extracts",
        "paper_translations",
    ] {
        let (total, filled): (i64, i64) = conn.query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(CASE WHEN papers_id IS NOT NULL THEN 1 ELSE 0 END), 0)
                 FROM {table}"
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        println!("{table}: 全 {total} 行 / papers_id 埋まり {filled} 行");
    }

    let count = |sql: &str| -> Result<i64, rusqlite::Error> {
        conn.query_row(sql, [], |row| row.get(0))
    };

    let paper_id = target_paper_id();
    let chunks_total_before = count("SELECT COUNT(*) FROM chunks")?;
    println!(
        "削除前: papers={}, chunks={}, chunk_embeddings={}",
        count("SELECT COUNT(*) FROM papers")?,
        chunks_total_before,
        count("SELECT COUNT(*) FROM chunk_embeddings")?,
    );

    let paper_exists = count(&format!(
        "SELECT COUNT(*) FROM papers WHERE id = {paper_id}"
    ))?;
    let indexed_before = count(&format!(
        "SELECT COUNT(*) FROM chunks WHERE paper_id = {paper_id}"
    ))?;
    if paper_exists == 1 {
        let key = db.citation_key_for_paper(paper_id);
        println!("対象論文: id = {paper_id}, 抽出キー = {key:?}");
        println!("対象論文の chunks: {indexed_before} 行");

        db.delete_paper(paper_id)?;
        println!("delete_paper({paper_id}) を実行した");

        // 対象論文の索引・抽出・翻訳・音声の行が残っていないこと。
        let leftovers: Vec<(&str, i64)> = vec![
            (
                "papers",
                count(&format!("SELECT COUNT(*) FROM papers WHERE id = {paper_id}"))?,
            ),
            (
                "chunks",
                count(&format!("SELECT COUNT(*) FROM chunks WHERE paper_id = {paper_id}"))?,
            ),
            (
                "paper_extracts",
                count(&format!(
                    "SELECT COUNT(*) FROM paper_extracts WHERE papers_id = {paper_id}"
                ))?,
            ),
            (
                "paper_figures",
                count(&format!(
                    "SELECT COUNT(*) FROM paper_figures WHERE papers_id = {paper_id}"
                ))?,
            ),
            (
                "paper_figure_extracts",
                count(&format!(
                    "SELECT COUNT(*) FROM paper_figure_extracts WHERE papers_id = {paper_id}"
                ))?,
            ),
            // 抽出キーが変わった後の記録（papers_id のみの行）も消えること。
            (
                "paper_refs (changed key)",
                count(&format!(
                    "SELECT COUNT(*) FROM paper_refs WHERE papers_id = {paper_id}"
                ))?,
            ),
            (
                "paper_sentences (changed key)",
                count(&format!(
                    "SELECT COUNT(*) FROM paper_sentences WHERE papers_id = {paper_id}"
                ))?,
            ),
            (
                "paper_translations (changed key)",
                count(&format!(
                    "SELECT COUNT(*) FROM paper_translations WHERE papers_id = {paper_id}"
                ))?,
            ),
            (
                "paper_podcasts",
                count(&format!(
                    "SELECT COUNT(*) FROM paper_podcasts WHERE paper_id = {paper_id}"
                ))?,
            ),
        ];
        for (label, n) in leftovers {
            assert_eq!(n, 0, "削除後も {label} に残りの行がある");
        }
        // 埋め込みも、対象論文の分が消えていること。
        let embeddings_left = count(&format!(
            "SELECT COUNT(*) FROM chunk_embeddings WHERE chunk_id IN \
             (SELECT id FROM chunks WHERE paper_id = {paper_id})"
        ))?;
        assert_eq!(embeddings_left, 0, "削除後も対象論文の埋め込みが残っている");

        // 他の論文の索引は残っていること（取り壊しが起きていないこと）。
        let chunks_total_after = count("SELECT COUNT(*) FROM chunks")?;
        println!(
            "削除後: chunks={}（削除前 {} 行から {} 行減）",
            chunks_total_after,
            chunks_total_before,
            chunks_total_before - chunks_total_after
        );
        assert_eq!(
            chunks_total_after,
            chunks_total_before - indexed_before,
            "対象論文以外の chunks まで消えた（または対象の chunks が残っている）"
        );
    } else {
        println!("論文 id = {paper_id} は既に無い（再実行）。残り行が 0 であることを確かめる");
        assert_eq!(indexed_before, 0, "削除済みのはずの論文の chunks が残っている");
    }

    // 他の論文のデータ（行数）がマイグレーションで保たれていること。
    let embeddings_total = count("SELECT COUNT(*) FROM chunk_embeddings")?;
    println!(
        "最終: papers={}, chunks={}, chunk_embeddings={}, paper_extracts={}, paper_figures={}",
        count("SELECT COUNT(*) FROM papers")?,
        count("SELECT COUNT(*) FROM chunks")?,
        embeddings_total,
        count("SELECT COUNT(*) FROM paper_extracts")?,
        count("SELECT COUNT(*) FROM paper_figures")?,
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM chunks")?,
        count("SELECT COUNT(*) FROM chunk_embeddings")?,
        "chunks と chunk_embeddings の件数が合わない（埋め込みが欠けた）"
    );

    // FTS の索引が壊れていないこと（外部コンテンツの整合性チェック）。
    conn.execute(
        "INSERT INTO chunk_fts(chunk_fts, rank) VALUES('integrity-check', 1)",
        [],
    )?;
    println!("chunk_fts の integrity-check: OK");

    Ok(())
}
