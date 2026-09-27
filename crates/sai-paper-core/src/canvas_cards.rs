//! Canvas に配置するカード（note_card / report_card）と right_sidebar の候補を集める。
//!
//! - `note_card`: `note.md` と 1 対 1 で紐づく（`note.md` は論文と 1 対 1）
//! - `report_card`: agent が保存した report（`reports` テーブル）と 1 対 1
//!
//! `summary` は md の先頭数行をそのまま使う（要約生成は行わない）。
//! `tag` は既存のタグ情報を表示用にカンマ区切りで持つ。

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use thiserror::Error;

/// Canvas カードの種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasCardKind {
    /// `note.md` のカード
    Note,
    /// report のカード
    Report,
    /// `note.md` を持たない論文のカード（V02 で追加）
    Paper,
}

impl CanvasCardKind {
    /// JSON Canvas ノードの `extra` に保存する種別名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Report => "report",
            Self::Paper => "paper",
        }
    }

    /// 種別名から復元する。
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "note" => Some(Self::Note),
            "report" => Some(Self::Report),
            "paper" => Some(Self::Paper),
            _ => None,
        }
    }
}

/// 論文の札（`saiKind: "paper"`）に表示する情報（V02）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperCardInfo {
    /// `papers.id`
    pub papers_id: i64,
    /// 論文タイトル
    pub title: String,
    /// 出版年（`published_at` の先頭 4 桁。不明なら空文字）
    pub year: String,
    /// 表示用タグ（カンマ区切り）
    pub tags: String,
}

/// カードの候補 1 件。
#[derive(Debug, Clone)]
pub struct CanvasCardSource {
    /// カード種別
    pub kind: CanvasCardKind,
    /// 対応する論文（`papers.id`）
    pub papers_id: Option<i64>,
    /// 対応する report（report_card のみ）
    pub report_id: Option<String>,
    /// 論文タイトル
    pub paper_title: String,
    /// 表示用タグ（カンマ区切り）
    pub tags: String,
    /// md 先頭数行（カードの summary）
    pub summary: String,
    /// md の実パス
    pub md_path: PathBuf,
}

/// right_sidebar の論文行。
#[derive(Debug, Clone)]
pub struct SidebarPaper {
    /// `papers.id`
    pub papers_id: i64,
    /// 論文タイトル
    pub title: String,
    /// `note.md` のパス（無ければ None）
    pub note_md: Option<PathBuf>,
    /// `note.md` の先頭数行
    pub note_summary: String,
    /// この論文を参照している report
    pub reports: Vec<SidebarReport>,
}

/// right_sidebar の report 行。
#[derive(Debug, Clone)]
pub struct SidebarReport {
    /// report_id
    pub report_id: String,
    /// report タイトル
    pub title: String,
    /// md の実パス
    pub md_path: PathBuf,
    /// md 先頭数行
    pub summary: String,
}

/// Canvas カード候補の取得エラー。
#[derive(Debug, Error)]
pub enum CanvasCardError {
    /// SQLite 操作の失敗
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// `published_at` から出版年（先頭 4 桁）を取り出す。
fn year_of(published_at: Option<&str>) -> String {
    let raw = published_at.map(str::trim).unwrap_or("");
    let year: String = raw.chars().take_while(char::is_ascii_digit).collect();
    if year.len() == 4 {
        year
    } else {
        String::new()
    }
}

/// 論文の札に表示する情報を 1 件引く（存在しなければ `None`）。
///
/// # Errors
///
/// SQLite エラー。
pub fn paper_card_info(
    conn: &Connection,
    papers_id: i64,
) -> Result<Option<PaperCardInfo>, CanvasCardError> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(p.title, ''), p.published_at,
                COALESCE((SELECT GROUP_CONCAT(t.name, ', ')
                          FROM paper_tags pt JOIN tags t ON t.id = pt.tag_id
                          WHERE pt.paper_id = p.id), '')
         FROM papers p WHERE p.id = ?1",
    )?;
    let mut rows = stmt.query_map([papers_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let Some(row) = rows.next() else {
        return Ok(None);
    };
    let (title, published_at, tags) = row?;
    Ok(Some(PaperCardInfo {
        papers_id,
        title,
        year: year_of(published_at.as_deref()),
        tags,
    }))
}

/// 論文フォルダの `note.md`（あるときだけ）。
///
/// `canvas_sync::canvas_lines`（note の言及の線）も使うため `pub(crate)`。
pub(crate) fn note_path(data_dir: &Path, papers_id: i64, title: &str) -> Option<PathBuf> {
    let candidate = crate::paths::paper_folder(data_dir, papers_id, title)
        .join(crate::paths::paper_file_names::NOTE);
    candidate.is_file().then_some(candidate)
}

/// md ファイルの先頭数行を summary にする（読めない場合は空文字）。
fn summary_of(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .map(|md| crate::reports::summary_from_markdown(&md, 3))
        .unwrap_or_default()
}

/// note_card の候補（`note.md` を持つ全論文）を返す。
///
/// # Errors
///
/// SQLite エラー。
pub fn note_card_sources(
    conn: &Connection,
    data_dir: &Path,
) -> Result<Vec<CanvasCardSource>, CanvasCardError> {
    let mut stmt = conn.prepare(
        "SELECT p.id, COALESCE(p.title, ''), p.pdf_path,
                COALESCE((SELECT GROUP_CONCAT(t.name, ', ')
                          FROM paper_tags pt JOIN tags t ON t.id = pt.tag_id
                          WHERE pt.paper_id = p.id), '')
         FROM papers p
         WHERE p.pdf_path IS NOT NULL
         ORDER BY p.id",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::new();
    for (id, title, _pdf, tags) in rows {
        let Some(path) = note_path(data_dir, id, &title) else {
            continue;
        };
        out.push(CanvasCardSource {
            kind: CanvasCardKind::Note,
            papers_id: Some(id),
            report_id: None,
            paper_title: title,
            tags,
            summary: summary_of(&path),
            md_path: path,
        });
    }
    Ok(out)
}

/// report_card の候補（`paper` を 1 つ以上参照している report）を返す。
///
/// 論文が特定できない report は Canvas の矢印を張れないため対象外とする。
///
/// # Errors
///
/// SQLite エラー。
pub fn report_card_sources(conn: &Connection) -> Result<Vec<CanvasCardSource>, CanvasCardError> {
    let links = crate::reports::paper_report_links(conn)
        .map_err(|_| CanvasCardError::Sqlite(rusqlite::Error::InvalidQuery))?;
    let mut out = Vec::new();
    for (papers_id, paper_title, report) in links {
        let Some(path) = crate::reports::resolve_md_path(&report) else {
            continue;
        };
        out.push(CanvasCardSource {
            kind: CanvasCardKind::Report,
            papers_id: Some(papers_id),
            report_id: Some(report.id.clone()),
            paper_title,
            tags: String::new(),
            summary: report.summary.clone().unwrap_or_else(|| summary_of(&path)),
            md_path: path,
        });
    }
    Ok(out)
}

/// 指定タグが付いた論文の note_card / report_card 候補を返す。
///
/// Canvas はタグ 1 つにつき 1 枚なので、その Canvas に載るべきカードがこれになる。
/// タグの付いていない論文はどの Canvas にも現れない。
///
/// # Errors
///
/// SQLite エラー。
pub fn tag_card_sources(
    conn: &Connection,
    data_dir: &Path,
    tag: &str,
) -> Result<Vec<CanvasCardSource>, CanvasCardError> {
    let mut stmt = conn.prepare(
        "SELECT pt.paper_id FROM paper_tags pt
         JOIN tags t ON t.id = pt.tag_id
         WHERE t.name = ?1 COLLATE NOCASE",
    )?;
    let paper_ids: std::collections::HashSet<i64> = stmt
        .query_map([tag], |row| row.get::<_, i64>(0))?
        .collect::<Result<_, _>>()?;
    let mut out = note_card_sources(conn, data_dir)?;
    out.extend(report_card_sources(conn)?);
    out.retain(|source| source.papers_id.is_some_and(|id| paper_ids.contains(&id)));
    Ok(out)
}

/// right_sidebar 用に、note / report を持つ論文を入れ子で返す。
///
/// 同じ report が複数の論文の下に現れてよい（要件 R5.5）。
///
/// # Errors
///
/// SQLite エラー。
pub fn sidebar_papers(
    conn: &Connection,
    data_dir: &Path,
) -> Result<Vec<SidebarPaper>, CanvasCardError> {
    // 1) note（論文フォルダの note.md）または report を持つ論文を集める（note 優先で並べる）
    let mut stmt = conn.prepare(
        "SELECT p.id, COALESCE(p.title, ''), p.pdf_path,
                EXISTS(SELECT 1 FROM report_sources s
                       WHERE s.kind = 'paper' AND s.ref_id = CAST(p.id AS TEXT)) AS has_report
         FROM papers p
         ORDER BY p.id",
    )?;
    let papers = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::new();
    for (papers_id, title, _pdf, has_report) in papers {
        let note_md = note_path(data_dir, papers_id, &title);
        if note_md.is_none() && has_report == 0 {
            continue;
        }
        let note_summary = note_md.as_deref().map(summary_of).unwrap_or_default();
        let reports = crate::reports::reports_for_paper(conn, papers_id)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|report| {
                let md_path = crate::reports::resolve_md_path(&report)?;
                Some(SidebarReport {
                    report_id: report.id.clone(),
                    title: report.title.clone().unwrap_or_default(),
                    md_path,
                    summary: report.summary.clone().unwrap_or_default(),
                })
            })
            .collect();
        out.push(SidebarPaper {
            papers_id,
            title,
            note_md,
            note_summary,
            reports,
        });
    }
    Ok(out)
}

/// 引用・被引用の関係（`paper_refs` ベース）にある論文ペアを返す。
///
/// `s` キーで解析済みの論文のみが対象になる（未解析の論文には矢印が出ない）。
/// 参照元（引用を書いた論文）はまず `papers_id` で、無ければ抽出キーの照合で
/// 結び付ける（段階 2c）。参照先は S2 ID / DOI / タイトルで照合する。
///
/// # Errors
///
/// SQLite エラー。
pub fn citation_links(conn: &Connection) -> Result<Vec<(i64, i64)>, CanvasCardError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT pa.id, pb.id
         FROM paper_refs r
         JOIN papers pa ON (r.papers_id IS NOT NULL AND pa.id = r.papers_id)
                        OR (r.papers_id IS NULL AND (
                              pa.s2_paper_id = r.paper_id COLLATE NOCASE
                           OR ('ArXiv:' || pa.arxiv_id) = r.paper_id COLLATE NOCASE
                           OR ('DOI:' || pa.doi) = r.paper_id COLLATE NOCASE
                           OR pa.arxiv_id = r.paper_id COLLATE NOCASE
                           OR pa.doi = r.paper_id COLLATE NOCASE))
         JOIN papers pb ON (r.s2_paper_id IS NOT NULL AND pb.s2_paper_id = r.s2_paper_id COLLATE NOCASE)
                        OR (r.doi IS NOT NULL AND pb.doi = r.doi COLLATE NOCASE)
                        OR (r.title IS NOT NULL AND pb.title = r.title COLLATE NOCASE)
         WHERE pa.id <> pb.id",
    )?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 指定した report_id を入力元に持つ report（report 間の関係）を返す。
///
/// # Errors
///
/// SQLite エラー。
pub fn child_reports(
    conn: &Connection,
    source_report_id: &str,
) -> Result<Vec<String>, CanvasCardError> {
    let mut stmt =
        conn.prepare("SELECT report_id FROM report_sources WHERE kind = 'report' AND ref_id = ?1")?;
    let rows = stmt
        .query_map([source_report_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 選んだ札のつながり（V04）。
///
/// - `reports`: その論文を使った report / その report が使った・元にした report
/// - `papers`: 引用・被引用の関係にある論文 / report が使った論文
/// - `notes_mentioning`: `#<番号>` を `note.md` に書いた論文
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Connections {
    /// つながっている report の id
    pub reports: Vec<String>,
    /// つながっている論文の `papers.id`
    pub papers: Vec<i64>,
    /// `#<番号>` を note.md に書いた論文の `papers.id`
    pub notes_mentioning: Vec<i64>,
}

/// report id の並びから、`exclude` を除いて整列・重複除去する。
fn sorted_unique_reports(values: Vec<String>, exclude: Option<&str>) -> Vec<String> {
    let mut values: Vec<String> = values
        .into_iter()
        .filter(|value| Some(value.as_str()) != exclude)
        .collect();
    values.sort();
    values.dedup();
    values
}

/// 論文番号の並びを整列・重複除去する。
fn sorted_unique_papers(mut values: Vec<i64>) -> Vec<i64> {
    values.sort_unstable();
    values.dedup();
    values
}

/// md の中に `#<paper_id>` があるか。
///
/// `#370` を `#37` と数えないよう、`#` の直後の数字の並びが番号と完全に一致する
/// ものだけを数える（`#37` が正、`#370` や `#137` は別の番号）。
///
/// `canvas_sync::canvas_lines`（note の言及の線）も使うため `pub(crate)`。
pub(crate) fn mentions_paper(markdown: &str, paper_id: i64) -> bool {
    let needle = paper_id.to_string();
    let mut rest = markdown;
    while let Some(hash) = rest.find('#') {
        let after = &rest[hash + 1..];
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        if digits == needle {
            return true;
        }
        rest = after;
    }
    false
}

/// note.md を読んで `#<paper_id>` があるか（読めないファイルは無視）。
fn note_mentions_paper(path: &Path, paper_id: i64) -> bool {
    std::fs::read_to_string(path).is_ok_and(|md| mentions_paper(&md, paper_id))
}

/// 論文につながるものを求める（V04）。
///
/// - その論文を使った report（`report_sources` の `kind='paper'`）
/// - 引用・被引用の論文（[`citation_links`] の両向き）
/// - その番号（`#<番号>`）を note.md に書いた論文（自分自身は除く）
///
/// # Errors
///
/// SQLite エラー。
pub fn connections_of_paper(
    conn: &Connection,
    data_dir: &Path,
    paper_id: i64,
) -> Result<Connections, CanvasCardError> {
    // その論文を使った report
    let mut stmt = conn.prepare(
        "SELECT report_id FROM report_sources WHERE kind = 'paper' AND ref_id = ?1",
    )?;
    let reports = stmt
        .query_map([paper_id.to_string()], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;

    // 引用・被引用の論文（両向き）
    let mut papers = Vec::new();
    for (from, to) in citation_links(conn)? {
        if from == paper_id {
            papers.push(to);
        } else if to == paper_id {
            papers.push(from);
        }
    }

    // その番号を書いた note（自分自身は除く）
    let mut notes_mentioning = Vec::new();
    for source in note_card_sources(conn, data_dir)? {
        let Some(other) = source.papers_id else {
            continue;
        };
        if other == paper_id {
            continue;
        }
        if note_mentions_paper(&source.md_path, paper_id) {
            notes_mentioning.push(other);
        }
    }

    Ok(Connections {
        reports: sorted_unique_reports(reports, None),
        papers: sorted_unique_papers(papers),
        notes_mentioning: sorted_unique_papers(notes_mentioning),
    })
}

/// report につながるものを求める（V04）。
///
/// - その report が使った論文（`report_sources` の `kind='paper'`）
/// - その report が使った・元にした report（`report_sources` の `kind='report'` と
///   [`child_reports`] の両向き、自分自身は除く）
///
/// # Errors
///
/// SQLite エラー。
pub fn connections_of_report(
    conn: &Connection,
    report_id: &str,
) -> Result<Connections, CanvasCardError> {
    let mut stmt = conn.prepare("SELECT kind, ref_id FROM report_sources WHERE report_id = ?1")?;
    let rows = stmt
        .query_map([report_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut papers = Vec::new();
    let mut reports = Vec::new();
    for (kind, ref_id) in rows {
        match kind.as_str() {
            "paper" => {
                if let Ok(id) = ref_id.parse::<i64>() {
                    papers.push(id);
                }
            }
            "report" => reports.push(ref_id),
            _ => {}
        }
    }
    // この report を元にした report（逆向き）
    reports.extend(child_reports(conn, report_id)?);

    Ok(Connections {
        reports: sorted_unique_reports(reports, Some(report_id)),
        papers: sorted_unique_papers(papers),
        notes_mentioning: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE papers (id INTEGER PRIMARY KEY, title TEXT, pdf_path TEXT, abstract TEXT, published_at TEXT);
             CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT);
             CREATE TABLE paper_tags (paper_id INTEGER, tag_id INTEGER);",
        )
        .unwrap();
        conn.execute_batch(include_str!("../migrations/0028_reports.sql"))
            .unwrap();
        conn
    }

    #[test]
    fn note_and_report_sources_are_listed() {
        let conn = setup();
        let dir = std::env::temp_dir().join(format!(
            "sai_canvas_cards_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("papers/0001_Paper-A")).unwrap();
        let note = dir.join("papers/0001_Paper-A/note.md");
        std::fs::write(&note, "# Note title\n\nFirst line.\n").unwrap();
        let pdf = dir.join("paper.pdf");
        std::fs::write(&pdf, b"%PDF").unwrap();
        conn.execute(
            "INSERT INTO papers (id, title, pdf_path) VALUES (1, 'Paper A', ?1)",
            [pdf.to_string_lossy().to_string()],
        )
        .unwrap();
        let notes = note_card_sources(&conn, &dir).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].papers_id, Some(1));
        assert_eq!(notes[0].summary, "Note title / First line.");

        // report カード
        let report_md = dir.join("report.md");
        std::fs::write(&report_md, "# Report title\n\nSummary line.\n").unwrap();
        crate::reports::create_report(
            &conn,
            &crate::reports::NewReport {
                md_path: report_md.to_string_lossy().to_string(),
                sources: vec![crate::reports::ReportSource {
                    kind: "paper".into(),
                    ref_id: "1".into(),
                    role: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let reports = report_card_sources(&conn).unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].report_id.as_deref(), Some("R-0001"));

        let sidebar = sidebar_papers(&conn, &dir).unwrap();
        assert_eq!(sidebar.len(), 1);
        assert_eq!(sidebar[0].reports.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn paper_card_info_reads_title_year_and_tags() {
        let conn = setup();
        conn.execute(
            "INSERT INTO papers (id, title, pdf_path, published_at) VALUES (7, 'Paper G', '/tmp/g.pdf', '2024-05-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO tags (id, name) VALUES (1, 'nlp'), (2, 'llm')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO paper_tags (paper_id, tag_id) VALUES (7, 2), (7, 1)",
            [],
        )
        .unwrap();

        let info = paper_card_info(&conn, 7).unwrap().expect("paper 7");
        assert_eq!(info.papers_id, 7);
        assert_eq!(info.title, "Paper G");
        assert_eq!(info.year, "2024");
        assert!(info.tags.contains("nlp"), "tags: {}", info.tags);
        assert!(info.tags.contains("llm"), "tags: {}", info.tags);

        // 未知の id は None。
        assert!(paper_card_info(&conn, 999).unwrap().is_none());
    }

    #[test]
    fn card_kind_round_trips_paper() {
        assert_eq!(CanvasCardKind::Paper.as_str(), "paper");
        assert_eq!(CanvasCardKind::parse("paper"), Some(CanvasCardKind::Paper));
    }

    /// V04 のつながりのテスト用 DB（引用の `paper_refs` を足す）。
    fn setup_connections() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE papers (id INTEGER PRIMARY KEY, title TEXT, pdf_path TEXT,
                 abstract TEXT, published_at TEXT, arxiv_id TEXT, doi TEXT, s2_paper_id TEXT);
             CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT);
             CREATE TABLE paper_tags (paper_id INTEGER, tag_id INTEGER);",
        )
        .unwrap();
        conn.execute_batch(include_str!("../migrations/0028_reports.sql"))
            .unwrap();
        conn.execute_batch(
            "CREATE TABLE paper_refs (paper_id TEXT NOT NULL, ref_no INTEGER NOT NULL,
                 title TEXT, doi TEXT, s2_paper_id TEXT, papers_id INTEGER,
                 PRIMARY KEY (paper_id, ref_no));",
        )
        .unwrap();
        conn
    }

    /// テストごとに使い捨てのフォルダを作る。
    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai_canvas_{label}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 論文の note.md を書く。
    fn write_note(dir: &Path, paper_id: i64, title: &str, body: &str) -> PathBuf {
        let folder = crate::paths::paper_folder(dir, paper_id, title);
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join(crate::paths::paper_file_names::NOTE);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn mentions_paper_matches_the_exact_number() {
        assert!(mentions_paper("see #37 for details", 37));
        assert!(mentions_paper("#37", 37));
        assert!(mentions_paper("#37#38", 37));
        // `#370` や `#137` を `#37` と数えない。
        assert!(!mentions_paper("see #370", 37));
        assert!(!mentions_paper("see #137", 37));
        assert!(!mentions_paper("see #3", 37));
        assert!(!mentions_paper("no number", 37));
    }

    #[test]
    fn connections_of_paper_collect_reports_citations_and_notes() {
        let conn = setup_connections();
        let dir = temp_dir("conn_paper");
        conn.execute_batch(
            "INSERT INTO papers (id, title, pdf_path, s2_paper_id) VALUES
                 (1, 'Paper A', '/tmp/a.pdf', 's2a'),
                 (2, 'Paper B', '/tmp/b.pdf', 's2b'),
                 (3, 'Paper C', '/tmp/c.pdf', 's2c');",
        )
        .unwrap();
        // 論文 1 が論文 2 を引用する。
        conn.execute(
            "INSERT INTO paper_refs (paper_id, ref_no, s2_paper_id, papers_id)
             VALUES ('key-1', 1, 's2b', 1)",
            [],
        )
        .unwrap();
        // 論文 2 の note は #1 と #370 を書いている（#370 は 37 ではない）。
        write_note(&dir, 2, "Paper B", "# B\n\nsee #1 and #370\n");
        // 論文 3 の note は #370 だけ（論文 1 とはつながらない）。
        write_note(&dir, 3, "Paper C", "# C\n\nsee #370\n");
        // 論文 1 を使った report。
        let report_md = dir.join("R-0001.md");
        std::fs::write(&report_md, "# Report\n\nbody\n").unwrap();
        crate::reports::create_report(
            &conn,
            &crate::reports::NewReport {
                md_path: report_md.to_string_lossy().to_string(),
                sources: vec![crate::reports::ReportSource {
                    kind: "paper".into(),
                    ref_id: "1".into(),
                    role: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();

        let connections = connections_of_paper(&conn, &dir, 1).unwrap();
        assert_eq!(connections.reports, vec!["R-0001".to_string()]);
        assert_eq!(connections.papers, vec![2]);
        assert_eq!(connections.notes_mentioning, vec![2], "#370 は #37 ではない");

        // 被引用の向きも見る（論文 2 から見ると論文 1 が引用元）。
        let reverse = connections_of_paper(&conn, &dir, 2).unwrap();
        assert_eq!(reverse.papers, vec![1]);
        assert!(reverse.reports.is_empty());
        assert!(reverse.notes_mentioning.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn connections_of_report_collect_sources_and_children() {
        let conn = setup_connections();
        let dir = temp_dir("conn_report");
        conn.execute_batch(
            "INSERT INTO papers (id, title) VALUES (1, 'Paper A'), (2, 'Paper B');",
        )
        .unwrap();
        let report_md = dir.join("R-0001.md");
        std::fs::write(&report_md, "# Report 1\n").unwrap();
        crate::reports::create_report(
            &conn,
            &crate::reports::NewReport {
                md_path: report_md.to_string_lossy().to_string(),
                sources: vec![crate::reports::ReportSource {
                    kind: "paper".into(),
                    ref_id: "1".into(),
                    role: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let report2_md = dir.join("R-0002.md");
        std::fs::write(&report2_md, "# Report 2\n").unwrap();
        crate::reports::create_report(
            &conn,
            &crate::reports::NewReport {
                md_path: report2_md.to_string_lossy().to_string(),
                sources: vec![
                    crate::reports::ReportSource {
                        kind: "paper".into(),
                        ref_id: "2".into(),
                        role: None,
                    },
                    crate::reports::ReportSource {
                        kind: "report".into(),
                        ref_id: "R-0001".into(),
                        role: None,
                    },
                ],
                ..Default::default()
            },
        )
        .unwrap();

        // R-0002 は論文 2 と R-0001 を使った。
        let report2 = connections_of_report(&conn, "R-0002").unwrap();
        assert_eq!(report2.reports, vec!["R-0001".to_string()]);
        assert_eq!(report2.papers, vec![2]);
        assert!(report2.notes_mentioning.is_empty());

        // R-0001 は論文 1 を使い、R-0002 の元になった。
        let report1 = connections_of_report(&conn, "R-0001").unwrap();
        assert_eq!(report1.reports, vec!["R-0002".to_string()]);
        assert_eq!(report1.papers, vec![1]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
