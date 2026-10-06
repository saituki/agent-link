//! 引用文献テーブル（`paper_refs` / `paper_sentences` / `paper_extracts`）の CRUD。
//!
//! 全関数は呼び出し側が所有する `&Connection` を受け取る（グローバル接続は持たない）。
//! 一括置換は `unchecked_transaction()` で原子性を担保する。

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::CitationDbError;

/// `paper_refs` の1行（引用文献）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RefRow {
    /// 抽出キー（S2 40hex または ArXiv:/DOI: プレフィックス付き）
    pub paper_id: String,
    /// 引用番号（1 起算）
    pub ref_no: i64,
    /// 第一著者
    pub first_author: Option<String>,
    /// 出版年
    pub year: Option<i64>,
    /// 年号サフィックス（例: "a", "b"）
    pub year_suffix: Option<String>,
    /// ジャーナル名
    pub journal: Option<String>,
    /// 引用文献タイトル
    pub title: Option<String>,
    /// DOI
    pub doi: Option<String>,
    /// docling 抽出元テキスト
    pub raw_text: String,
    /// S2 40hex（マッチした引用文献の S2 ID）
    pub s2_paper_id: Option<String>,
    /// S2 上のタイトル
    pub s2_title: Option<String>,
    /// S2 上の著者（JSON 配列）
    pub s2_authors: Option<String>,
    /// S2 上の出版年
    pub s2_year: Option<i64>,
    /// S2 上のジャーナル
    pub s2_venue: Option<String>,
    /// S2 上のジャーナル詳細（JSON）
    pub s2_journal: Option<String>,
    /// S2 外部 ID（JSON）
    pub s2_external_ids: Option<String>,
    /// S2 被引用数
    pub s2_citation_count: Option<i64>,
    /// S2 URL
    pub s2_url: Option<String>,
}

/// `paper_sentences` の1行（本文中の引用文）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentenceRow {
    /// 抽出キー
    pub paper_id: String,
    /// 文番号（1 起算）
    pub sent_no: i64,
    /// 文のテキスト
    pub text: String,
    /// 言及されている ref_no の JSON 配列（例: `"[1, 5, 12]"`）
    pub cite_refs: String,
}

/// `paper_extracts` の1行（抽出状態）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractRow {
    /// 抽出キー
    pub paper_id: String,
    /// none / parsing / ready / failed
    pub status: String,
    /// ジャーナルスタイル
    pub style: Option<String>,
    /// PDF ファイルパス
    pub pdf_path: Option<String>,
    /// PDF 最終更新時刻（エポック秒）
    pub pdf_mtime: Option<String>,
    /// 抽出した引用数
    pub n_refs: Option<i64>,
    /// 抽出した文数
    pub n_sentences: Option<i64>,
    /// docling 出力 Markdown パス
    pub md_path: Option<String>,
    /// 失敗時のエラーメッセージ
    pub error: Option<String>,
    /// 最終抽出日時（ISO8601）
    pub extracted_at: Option<String>,
    /// `papers(id)`（Library の番号。NULL 許容。段階 2c）
    pub papers_id: Option<i64>,
}

const REF_COLUMNS: &str = "paper_id, ref_no, first_author, year, year_suffix, journal, title, doi, raw_text, \
     s2_paper_id, s2_title, s2_authors, s2_year, s2_venue, s2_journal, s2_external_ids, s2_citation_count, s2_url";

/// 指定論文の引用文献を全件取得する（ref_no 昇順）。
pub fn get_refs(conn: &Connection, paper_id: &str) -> Result<Vec<RefRow>, CitationDbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {REF_COLUMNS} FROM paper_refs WHERE paper_id = ?1 ORDER BY ref_no"
    ))?;
    let rows = stmt
        .query_map(params![paper_id], map_ref_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Library 論文の引用文献を、まず `papers_id` で、無ければ抽出キーで取得する（段階 2c）。
///
/// 探す順序:
/// 1. `papers_id` かつ現在の抽出キー（同じ論文に複数のキーで残っている二重行を避ける）
/// 2. `papers_id`（抽出キーが変わっても過去の記録が見えるように）
/// 3. 抽出キー（`papers_id` が NULL の古い行・Library 外の抽出）
pub fn get_refs_for(
    conn: &Connection,
    papers_id: Option<i64>,
    key: Option<&str>,
) -> Result<Vec<RefRow>, CitationDbError> {
    if let Some(papers_id) = papers_id {
        if let Some(key) = key {
            let mut stmt = conn.prepare(&format!(
                "SELECT {REF_COLUMNS} FROM paper_refs
                 WHERE papers_id = ?1 AND paper_id = ?2 ORDER BY ref_no"
            ))?;
            let rows = stmt
                .query_map(params![papers_id, key], map_ref_row)?
                .collect::<Result<Vec<_>, _>>()?;
            if !rows.is_empty() {
                return Ok(rows);
            }
        }
        let mut stmt = conn.prepare(&format!(
            "SELECT {REF_COLUMNS} FROM paper_refs WHERE papers_id = ?1 ORDER BY ref_no"
        ))?;
        let rows = stmt
            .query_map(params![papers_id], map_ref_row)?
            .collect::<Result<Vec<_>, _>>()?;
        if !rows.is_empty() {
            return Ok(rows);
        }
    }
    match key {
        Some(key) => get_refs(conn, key),
        None => Ok(Vec::new()),
    }
}

fn map_ref_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RefRow> {
    Ok(RefRow {
        paper_id: row.get(0)?,
        ref_no: row.get(1)?,
        first_author: row.get(2)?,
        year: row.get(3)?,
        year_suffix: row.get(4)?,
        journal: row.get(5)?,
        title: row.get(6)?,
        doi: row.get(7)?,
        raw_text: row.get(8)?,
        s2_paper_id: row.get(9)?,
        s2_title: row.get(10)?,
        s2_authors: row.get(11)?,
        s2_year: row.get(12)?,
        s2_venue: row.get(13)?,
        s2_journal: row.get(14)?,
        s2_external_ids: row.get(15)?,
        s2_citation_count: row.get(16)?,
        s2_url: row.get(17)?,
    })
}

/// 引用文献を一括置換する（既存行を削除してから挿入）。
///
/// `papers_id` が分かっているときは、同じ論文（同じ `papers_id`）で別の抽出キーに
/// 残っている古い行も消す（キーが変わった論文で行が二重にならないように。段階 2c）。
pub fn replace_refs(
    conn: &Connection,
    paper_id: &str,
    papers_id: Option<i64>,
    refs: &[RefRow],
) -> Result<(), CitationDbError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM paper_refs WHERE paper_id = ?1",
        params![paper_id],
    )?;
    if let Some(papers_id) = papers_id {
        tx.execute(
            "DELETE FROM paper_refs WHERE papers_id = ?1 AND paper_id <> ?2",
            params![papers_id, paper_id],
        )?;
    }
    {
        let mut stmt = tx.prepare(&format!(
            "INSERT INTO paper_refs ({REF_COLUMNS}, papers_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)"
        ))?;
        for r in refs {
            stmt.execute(params![
                r.paper_id,
                r.ref_no,
                r.first_author,
                r.year,
                r.year_suffix,
                r.journal,
                r.title,
                r.doi,
                r.raw_text,
                r.s2_paper_id,
                r.s2_title,
                r.s2_authors,
                r.s2_year,
                r.s2_venue,
                r.s2_journal,
                r.s2_external_ids,
                r.s2_citation_count,
                r.s2_url,
                papers_id,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// 指定論文の引用文を全件取得する（sent_no 昇順）。
pub fn get_sentences(
    conn: &Connection,
    paper_id: &str,
) -> Result<Vec<SentenceRow>, CitationDbError> {
    query_sentences(
        conn,
        "SELECT paper_id, sent_no, text, cite_refs FROM paper_sentences WHERE paper_id = ?1 ORDER BY sent_no",
        params![paper_id],
    )
}

/// Library 論文の引用文を、まず `papers_id` で、無ければ抽出キーで取得する（段階 2c）。
///
/// 引用文献と同じ 3 段階（`papers_id` + 現在のキー → `papers_id` → キー）で探す。
pub fn get_sentences_for(
    conn: &Connection,
    papers_id: Option<i64>,
    key: Option<&str>,
) -> Result<Vec<SentenceRow>, CitationDbError> {
    if let Some(papers_id) = papers_id {
        if let Some(key) = key {
            let rows = query_sentences(
                conn,
                "SELECT paper_id, sent_no, text, cite_refs FROM paper_sentences
                 WHERE papers_id = ?1 AND paper_id = ?2 ORDER BY sent_no",
                params![papers_id, key],
            )?;
            if !rows.is_empty() {
                return Ok(rows);
            }
        }
        let rows = query_sentences(
            conn,
            "SELECT paper_id, sent_no, text, cite_refs FROM paper_sentences
             WHERE papers_id = ?1 ORDER BY sent_no",
            params![papers_id],
        )?;
        if !rows.is_empty() {
            return Ok(rows);
        }
    }
    match key {
        Some(key) => get_sentences(conn, key),
        None => Ok(Vec::new()),
    }
}

/// `paper_sentences` の行をパラメータ付きで読みこむ（`get_sentences_for` の共通部）。
fn query_sentences(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<SentenceRow>, CitationDbError> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params, |row| {
            Ok(SentenceRow {
                paper_id: row.get(0)?,
                sent_no: row.get(1)?,
                text: row.get(2)?,
                cite_refs: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 引用文を一括置換する（既存行を削除してから挿入）。
///
/// `sentences` は `(本文, 引用 ref_no リスト)` の列。sent_no は 1 起算で採番する。
/// `papers_id` が分かっているときは、同じ論文で別の抽出キーに残っている古い行も
/// 消す（段階 2c）。
pub fn replace_sentences(
    conn: &Connection,
    paper_id: &str,
    papers_id: Option<i64>,
    sentences: &[(String, Vec<i64>)],
) -> Result<(), CitationDbError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM paper_sentences WHERE paper_id = ?1",
        params![paper_id],
    )?;
    if let Some(papers_id) = papers_id {
        tx.execute(
            "DELETE FROM paper_sentences WHERE papers_id = ?1 AND paper_id <> ?2",
            params![papers_id, paper_id],
        )?;
    }
    {
        let mut stmt = tx.prepare(
            "INSERT INTO paper_sentences (paper_id, sent_no, text, cite_refs, papers_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (i, (text, cites)) in sentences.iter().enumerate() {
            let cite_json = serde_json::to_string(cites)?;
            stmt.execute(params![paper_id, i as i64 + 1, text, cite_json, papers_id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// 抽出状態を取得する。
pub fn get_extract(
    conn: &Connection,
    paper_id: &str,
) -> Result<Option<ExtractRow>, CitationDbError> {
    let mut stmt = conn.prepare(
        "SELECT paper_id, status, style, pdf_path, pdf_mtime, n_refs, n_sentences, md_path, error, extracted_at, papers_id
         FROM paper_extracts WHERE paper_id = ?1",
    )?;
    let row = stmt
        .query_row(params![paper_id], map_extract_row)
        .optional()?;
    Ok(row)
}

fn map_extract_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExtractRow> {
    Ok(ExtractRow {
        paper_id: row.get(0)?,
        status: row.get(1)?,
        style: row.get(2)?,
        pdf_path: row.get(3)?,
        pdf_mtime: row.get(4)?,
        n_refs: row.get(5)?,
        n_sentences: row.get(6)?,
        md_path: row.get(7)?,
        error: row.get(8)?,
        extracted_at: row.get(9)?,
        papers_id: row.get(10)?,
    })
}

/// Library 論文の抽出状態を、まず `papers_id` で、無ければ抽出キーで取得する（段階 2c）。
///
/// 引用文献と同じ 3 段階（`papers_id` + 現在のキー → `papers_id` → キー）で探す。
pub fn get_extract_for(
    conn: &Connection,
    papers_id: Option<i64>,
    key: Option<&str>,
) -> Result<Option<ExtractRow>, CitationDbError> {
    const SELECT: &str = "SELECT paper_id, status, style, pdf_path, pdf_mtime, n_refs, n_sentences,
                    md_path, error, extracted_at, papers_id
             FROM paper_extracts";
    if let Some(papers_id) = papers_id {
        if let Some(key) = key {
            let mut stmt =
                conn.prepare(&format!("{SELECT} WHERE papers_id = ?1 AND paper_id = ?2"))?;
            let row = stmt
                .query_row(params![papers_id, key], map_extract_row)
                .optional()?;
            if row.is_some() {
                return Ok(row);
            }
        }
        let mut stmt = conn.prepare(&format!("{SELECT} WHERE papers_id = ?1"))?;
        let row = stmt
            .query_row(params![papers_id], map_extract_row)
            .optional()?;
        if row.is_some() {
            return Ok(row);
        }
    }
    match key {
        Some(key) => get_extract(conn, key),
        None => Ok(None),
    }
}

/// 抽出状態を upsert する。
///
/// `papers_id` が分かっているときは、同じ論文で別の抽出キーに残っている古い行を
/// 消してから書く（段階 2c）。`papers_id` が NULL の upsert は既存値を消さない。
pub fn upsert_extract(conn: &Connection, row: &ExtractRow) -> Result<(), CitationDbError> {
    let tx = conn.unchecked_transaction()?;
    if let Some(papers_id) = row.papers_id {
        tx.execute(
            "DELETE FROM paper_extracts WHERE papers_id = ?1 AND paper_id <> ?2",
            params![papers_id, row.paper_id],
        )?;
    }
    tx.execute(
        "INSERT INTO paper_extracts (paper_id, status, style, pdf_path, pdf_mtime, n_refs, n_sentences, md_path, error, extracted_at, papers_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(paper_id) DO UPDATE SET
           status = excluded.status, style = excluded.style, pdf_path = excluded.pdf_path,
           pdf_mtime = excluded.pdf_mtime, n_refs = excluded.n_refs, n_sentences = excluded.n_sentences,
           md_path = excluded.md_path, error = excluded.error, extracted_at = excluded.extracted_at,
           papers_id = COALESCE(excluded.papers_id, paper_extracts.papers_id)",
        params![
            row.paper_id,
            row.status,
            row.style,
            row.pdf_path,
            row.pdf_mtime,
            row.n_refs,
            row.n_sentences,
            row.md_path,
            row.error,
            row.extracted_at,
            row.papers_id,
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// 論文メタデータ（TUI 表示用）。
#[derive(Debug, Clone)]
pub struct PaperMeta {
    /// データベース行 ID
    pub db_id: i64,
    /// 抽出キー
    pub paper_id: String,
    /// タイトル
    pub title: String,
    /// 出版年
    pub year: Option<i64>,
    /// ジャーナル / 場
    pub venue: Option<String>,
    /// 著者リスト
    pub authors: Vec<String>,
    /// アブストラクト
    pub abstract_text: Option<String>,
    /// DOI
    pub doi: Option<String>,
    /// arXiv ID
    pub arxiv_id: Option<String>,
    /// ローカル PDF パス
    pub pdf_path: Option<String>,
}

/// 抽出キー（s2_paper_id / arxiv_id / doi 正規化値）から論文メタデータを取得する。
///
/// papr.db の `papers` テーブルは INTEGER 主キーのため、引用パイプラインのキーとの
/// 対応は列値の照合で行う。TUI の表示に必要な最低限の列のみを返す。
pub fn get_paper_meta_by_extract_key(
    conn: &Connection,
    key: &str,
) -> Result<Option<PaperMeta>, CitationDbError> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.title, p.published_at, p.journal, p.abstract, p.doi, p.arxiv_id, p.pdf_path,
                (SELECT GROUP_CONCAT(a.name, ', ') FROM paper_authors pa
                 JOIN authors a ON a.id = pa.author_id
                 WHERE pa.paper_id = p.id) AS authors_csv
         FROM papers p
         WHERE p.s2_paper_id = ?1 COLLATE NOCASE
            OR ('ArXiv:' || p.arxiv_id) = ?1 COLLATE NOCASE
            OR ('DOI:' || p.doi) = ?1 COLLATE NOCASE
            OR p.arxiv_id = ?1 COLLATE NOCASE
            OR p.doi = ?1 COLLATE NOCASE
         ORDER BY CASE WHEN p.s2_paper_id = ?1 COLLATE NOCASE THEN 0 ELSE 1 END
         LIMIT 1",
    )?;
    let meta = stmt
        .query_row(params![key], |row| {
            let published_at: Option<String> = row.get(2)?;
            Ok(PaperMeta {
                db_id: row.get(0)?,
                paper_id: key.to_string(),
                title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                year: published_at.as_deref().and_then(extract_year),
                venue: row.get(3)?,
                authors: row
                    .get::<_, Option<String>>(8)?
                    .map(|csv| {
                        csv.split(", ")
                            .map(str::to_string)
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default(),
                abstract_text: row.get(4)?,
                doi: row.get(5)?,
                arxiv_id: row.get(6)?,
                pdf_path: row.get(7)?,
            })
        })
        .optional()?;
    Ok(meta)
}

/// ISO8601 文字列から年を抽出する。
fn extract_year(s: &str) -> Option<i64> {
    // RFC3339 の先頭4文字が西暦（例: "2024-01-01T..." / "2024-01-01 00:00:00"）
    s.get(0..4)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // 0016 は papers テーブルへの ALTER を含むため、最小限の papers を先に作成
        conn.execute_batch(
            "CREATE TABLE papers (id INTEGER PRIMARY KEY, title TEXT, doi TEXT, arxiv_id TEXT);",
        )
        .unwrap();
        // papers_id の列（段階 2c）をそろえるため、引用・翻訳・図のマイグレーションを
        // 0016 の順に適用する（0033 は 6 表すべての列を要求する）。
        for sql in [
            include_str!("../../migrations/0016_citation_tables.sql"),
            include_str!("../../migrations/0017_translation.sql"),
            include_str!("../../migrations/0025_paper_extracts_papers_id.sql"),
            include_str!("../../migrations/0027_paper_figures.sql"),
            include_str!("../../migrations/0033_papers_id_unify.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        conn
    }

    fn sample_ref(paper_id: &str, ref_no: i64) -> RefRow {
        RefRow {
            paper_id: paper_id.to_string(),
            ref_no,
            first_author: Some("Smith".into()),
            year: Some(2020),
            year_suffix: None,
            journal: Some("Nature".into()),
            title: Some("A study".into()),
            doi: Some("10.1/x".into()),
            raw_text: "[1] Smith, A study. Nature (2020).".into(),
            s2_paper_id: None,
            s2_title: None,
            s2_authors: None,
            s2_year: None,
            s2_venue: None,
            s2_journal: None,
            s2_external_ids: None,
            s2_citation_count: None,
            s2_url: None,
        }
    }

    #[test]
    fn refs_roundtrip_is_idempotent() {
        let conn = setup();
        let refs = vec![sample_ref("p1", 1), sample_ref("p1", 2)];
        replace_refs(&conn, "p1", None, &refs).unwrap();
        replace_refs(&conn, "p1", None, &refs).unwrap();
        let loaded = get_refs(&conn, "p1").unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].ref_no, 1);
        assert_eq!(loaded[1].title.as_deref(), Some("A study"));
        assert!(get_refs(&conn, "p2").unwrap().is_empty());
    }

    #[test]
    fn sentences_roundtrip() {
        let conn = setup();
        let sents = vec![
            ("This cites [1].".to_string(), vec![1]),
            ("This cites [2, 3].".to_string(), vec![2, 3]),
        ];
        replace_sentences(&conn, "p1", None, &sents).unwrap();
        let loaded = get_sentences(&conn, "p1").unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[1].cite_refs, "[2,3]");
        // 再実行で重複しない
        replace_sentences(&conn, "p1", None, &sents).unwrap();
        assert_eq!(get_sentences(&conn, "p1").unwrap().len(), 2);
    }

    #[test]
    fn extract_upsert() {
        let conn = setup();
        let mut row = ExtractRow::default();
        row.paper_id = "p1".into();
        row.status = "parsing".into();
        upsert_extract(&conn, &row).unwrap();
        row.status = "ready".into();
        row.n_refs = Some(3);
        upsert_extract(&conn, &row).unwrap();
        let loaded = get_extract(&conn, "p1").unwrap().unwrap();
        assert_eq!(loaded.status, "ready");
        assert_eq!(loaded.n_refs, Some(3));
        assert!(get_extract(&conn, "p2").unwrap().is_none());
    }
}
