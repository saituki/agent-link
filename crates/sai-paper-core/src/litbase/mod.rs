//! pi-litbase の道具が使う、表を持たない API。
//!
//! litbase（生物医学研究者向けの論文精読・研究構想システム）のうち、
//! 論文の特定と最近の論文の取得だけを提供する。研究ドキュメントや
//! エージェントの 4 節ノートを保存する表は migration 0038 で
//! 消した（現況は `.sai/paper/state.md`、ノートは論文フォルダの `note.md` が正）。
//! LLM 推論は pi agent が担当し、このモジュールは決定論的な取得のみを提供する。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// litbase 操作のエラー。
#[derive(Debug, Error)]
pub enum LitbaseError {
    /// `SQLite` 操作に失敗した。
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// JSON の変換に失敗した。
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// 対象が見つからない。
    #[error("not found: {0}")]
    NotFound(String),
    /// 入力が不正。
    #[error("invalid input: {0}")]
    Invalid(String),
}

/// litbase 操作の結果型。
pub type Result<T> = std::result::Result<T, LitbaseError>;

/// 論文解決の結果（Library からの選択用）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedPaper {
    /// 論文 ID。
    pub id: i64,
    /// タイトル。
    pub title: String,
    /// DOI。
    pub doi: Option<String>,
    /// arXiv ID。
    pub arxiv_id: Option<String>,
    /// S2 paperId。
    pub s2_paper_id: Option<String>,
    /// ローカル PDF パス。
    pub pdf_path: Option<String>,
    /// 読書状態。
    pub reading_status: String,
}

/// 論文を ID / DOI / arXiv ID / S2 ID / タイトルから解決する。
///
/// # Errors
///
/// クエリに失敗した場合にエラーを返す。
pub fn resolve_paper(connection: &Connection, query: &str) -> Result<Option<ResolvedPaper>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(None);
    }
    let row_mapper = |row: &rusqlite::Row<'_>| {
        Ok(ResolvedPaper {
            id: row.get(0)?,
            title: row.get(1)?,
            doi: row.get(2)?,
            arxiv_id: row.get(3)?,
            s2_paper_id: row.get(4)?,
            pdf_path: row.get(5)?,
            reading_status: row.get(6)?,
        })
    };
    const SELECT: &str = "SELECT id, title, doi, arxiv_id, s2_paper_id, pdf_path, reading_status
                          FROM papers";

    if let Ok(id) = query.parse::<i64>()
        && let Some(found) = connection
            .query_row(&format!("{SELECT} WHERE id = ?1"), [id], row_mapper)
            .optional()?
    {
        return Ok(Some(found));
    }
    for column in ["doi", "arxiv_id", "s2_paper_id"] {
        if let Some(found) = connection
            .query_row(
                &format!("{SELECT} WHERE {column} = ?1 COLLATE NOCASE LIMIT 1"),
                [query],
                row_mapper,
            )
            .optional()?
        {
            return Ok(Some(found));
        }
    }
    let like = format!("%{query}%");
    Ok(connection
        .query_row(
            &format!(
                "{SELECT} WHERE title LIKE ?1 ORDER BY citation_count DESC NULLS LAST LIMIT 1"
            ),
            [like],
            row_mapper,
        )
        .optional()?)
}

/// Library の論文を最近更新順に取得する（Library からの選択用）。
///
/// # Errors
///
/// クエリに失敗した場合にエラーを返す。
pub fn recent_papers(connection: &Connection, limit: usize) -> Result<Vec<ResolvedPaper>> {
    let mut statement = connection.prepare(
        "SELECT id, title, doi, arxiv_id, s2_paper_id, pdf_path, reading_status
         FROM papers ORDER BY COALESCE(last_opened_at, created_at) DESC LIMIT ?1",
    )?;
    let rows = statement.query_map([limit as i64], |row| {
        Ok(ResolvedPaper {
            id: row.get(0)?,
            title: row.get(1)?,
            doi: row.get(2)?,
            arxiv_id: row.get(3)?,
            s2_paper_id: row.get(4)?,
            pdf_path: row.get(5)?,
            reading_status: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn seed_paper(connection: &Connection, title: &str) -> i64 {
        connection
            .execute("INSERT INTO papers (title) VALUES (?1)", [title])
            .unwrap();
        connection.last_insert_rowid()
    }

    #[test]
    fn resolve_paper_by_id_and_title() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let paper_id = seed_paper(conn, "Ferroptosis in glioma");

        let by_id = resolve_paper(conn, &paper_id.to_string()).unwrap().unwrap();
        assert_eq!(by_id.id, paper_id);
        assert_eq!(by_id.title, "Ferroptosis in glioma");

        let by_title = resolve_paper(conn, "Ferroptosis").unwrap().unwrap();
        assert_eq!(by_title.id, paper_id);

        assert!(resolve_paper(conn, "  ").unwrap().is_none());
    }

    #[test]
    fn recent_papers_returns_library_rows() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        seed_paper(conn, "One");
        seed_paper(conn, "Two");

        let papers = recent_papers(conn, 10).unwrap();
        assert_eq!(papers.len(), 2);
    }
}
