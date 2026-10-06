//! Library 階層表示の共通処理（TUI と CLI の共有）。
//!
//! TUI にあった「PDF 実在判定」「検索ヒット ↔ Library 論文の対応付け」
//! 「Library 対象フォルダの決定」「階層（query / Import）のまとめ方」を
//! ここに集約する。TUI はこのモジュールを呼び出し、表示（折りたたみ・ラベル）は
//! 従来どおり TUI 側で行う。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::config::{Config, Paths};
use crate::models::{LibraryPaper, RemotePaper};

/// PDF が実在するか（TUI の `paper_pdf_exists` と同じ）。
#[must_use]
pub fn paper_pdf_exists(paper: &LibraryPaper) -> bool {
    paper
        .pdf_path
        .as_deref()
        .is_some_and(|path| std::path::Path::new(path).is_file())
}

/// 検索ヒットに対応するダウンロード済み論文の添字（TUI の `match_downloaded_paper` と同じ判定）。
///
/// arXiv ID → DOI（大文字小文字無視）→ 正規化タイトル の順に照合し、
/// **ローカル PDF が実在する**論文だけを対象にする。
#[must_use]
pub fn match_downloaded_paper(papers: &[LibraryPaper], remote: &RemotePaper) -> Option<usize> {
    if let Some(index) = papers
        .iter()
        .position(|p| paper_pdf_exists(p) && p.arxiv_id.as_deref() == Some(remote.id.as_str()))
    {
        return Some(index);
    }
    if let Some(doi) = remote
        .doi
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && let Some(index) = papers.iter().position(|p| {
            paper_pdf_exists(p)
                && p.doi
                    .as_deref()
                    .is_some_and(|d| d.eq_ignore_ascii_case(doi))
        })
    {
        return Some(index);
    }
    let target = crate::citation::matcher::normalize_title(&remote.title);
    if target.is_empty() {
        return None;
    }
    papers.iter().position(|p| {
        paper_pdf_exists(p) && crate::citation::matcher::normalize_title(&p.title) == target
    })
}

/// Library の対象フォルダ（TUI の `LibraryLocations::resolve` の `library_roots` と同じ）。
///
/// 次の順に並べる（段階 2a）:
///
/// 1. `config.library.import_folders`（正規化済み）… 「外の PDF を読み込む元」。走査の意味は今までと同じ。
/// 2. **論文の置き場 `<data_dir>/papers/`（`paths.downloads_dir`）** … 設定に関係なく常に含める。
/// 3. `download_path`（未設定なら既定のダウンロード先）… 既に 1 / 2 でカバーされていなければ加える。
///
/// DL 先そのものは `paths::paper_folder` が決めるため、`download_path` や
/// `library_folders` では置き場所を変えられない。
/// **フォルダの作成はしない**（作成は TUI 側に残す）。
#[must_use]
pub fn library_roots(config: &Config, paths: &Paths) -> Vec<PathBuf> {
    library_roots_for(config, &paths.downloads_dir)
}

/// [`library_roots`] の本体（論文の置き場のパスだけを明示的に渡す形）。
///
/// TUI の設定リロード時など、`Paths` を作り直さずに再計算したいときに使う。
#[must_use]
pub fn library_roots_for(config: &Config, papers_dir: &Path) -> Vec<PathBuf> {
    let download_dir = config
        .library.download_path
        .clone()
        .unwrap_or_else(|| papers_dir.to_path_buf());
    let download_dir = std::fs::canonicalize(&download_dir).unwrap_or(download_dir);
    let papers_dir = std::fs::canonicalize(papers_dir).unwrap_or_else(|_| papers_dir.to_path_buf());
    let collection_roots = config
        .library.import_folders
        .iter()
        .map(|root| std::fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
        .collect::<Vec<_>>();
    let mut library_roots = collection_roots.clone();
    // 論文の置き場は設定に関係なく常に走査対象に含める。
    if !library_roots
        .iter()
        .any(|root| papers_dir.starts_with(root))
    {
        library_roots.push(papers_dir);
    }
    if !library_roots
        .iter()
        .any(|root| download_dir.starts_with(root))
    {
        library_roots.push(download_dir);
    }
    library_roots
}

/// 階層グループの種別（JSON では小文字 `query` / `import`）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupKind {
    /// 検索ログ 1 件（その検索で見つかったダウンロード済み論文を従える）。
    Query,
    /// 検索に紐付かない論文を DB 登録日ごとにまとめたもの。
    Import,
}

/// 階層表示の 1 まとまり（折りたたみなどの表示状態は含めない）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct LibraryGroup {
    /// グループの種別。
    pub kind: GroupKind,
    /// 検索ログ ID（`query` のみ。`import` は `None`）。
    pub log_id: Option<i64>,
    /// 日付グループ（`YYYY/MM/DD`。`import` で登録日が不明なときは `"unknown"`）。
    pub date: String,
    /// 検索ワード（`query` のみ）。
    pub query: Option<String>,
    /// 検索対象 DB（カンマ区切り、`query` のみ）。
    pub source_dbs: Option<String>,
    /// 検索ヒット件数（ダウンロード済み与否を問わない。`import` は `paper_ids.len()`）。
    pub hit_count: usize,
    /// このグループに属する Library 論文の ID（query の子、Import の論文）。
    pub paper_ids: Vec<i64>,
}

/// TUI の `rebuild_library_tree` と同じまとめ方で階層を作る。
///
/// - query: 検索ログの新しい順。子はその検索でヒットし、PDF が実在する論文。
/// - import: query に紐付かない論文を `papers.created_at` の日付ごとに、
///   新しい日付が先で query の後ろに並べる。
///
/// 折りたたみ状態や表示ラベルは含まない（表示は呼び出し側の仕事）。
#[must_use]
pub fn build_groups(conn: &Connection, papers: &[LibraryPaper]) -> Vec<LibraryGroup> {
    // 論文 ID → DB 登録日（YYYY/MM/DD）。Import グループの日付に使う。
    let mut created_dates: HashMap<i64, String> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id, created_at FROM papers") {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        }) {
            for row in rows.flatten() {
                if let (id, Some(created)) = row {
                    let date = created
                        .split(['T', ' '])
                        .next()
                        .unwrap_or("")
                        .replace('-', "/");
                    if !date.is_empty() {
                        created_dates.insert(id, date);
                    }
                }
            }
        }
    }

    let logs = crate::search_log::list(conn).unwrap_or_default();
    let mut groups: Vec<LibraryGroup> = Vec::new();
    let mut used_paper_ids: HashSet<i64> = HashSet::new();

    for log in &logs {
        let items = crate::search_log::items(conn, log.id).unwrap_or_default();
        let hit_count = items.len();
        let mut paper_ids: Vec<i64> = Vec::new();
        for item in &items {
            // 候補が指す論文が既に取り込まれているなら、arXiv ID / DOI / 題名の
            // 照合より先にその論文を使う（Discover で `d` や `i` を押した論文は
            // メタデータが別物でも紐付けられているため）。
            let index = item
                .paper_id
                .and_then(|id| {
                    papers
                        .iter()
                        .position(|p| p.id == id && paper_pdf_exists(p))
                })
                .or_else(|| match_downloaded_paper(papers, &item.remote));
            if let Some(index) = index {
                let id = papers[index].id;
                if !paper_ids.contains(&id) {
                    paper_ids.push(id);
                    used_paper_ids.insert(id);
                }
            }
        }
        groups.push(LibraryGroup {
            kind: GroupKind::Query,
            log_id: Some(log.id),
            date: log.log_date.clone(),
            query: Some(log.query.clone()),
            source_dbs: Some(log.source_dbs.clone()),
            hit_count,
            paper_ids,
        });
    }

    // Import: 検索に紐付かない論文を登録日ごとにまとめる（新しい日付が先）。
    let mut imports: HashMap<String, Vec<i64>> = HashMap::new();
    for paper in papers {
        if used_paper_ids.contains(&paper.id) {
            continue;
        }
        let date = created_dates
            .get(&paper.id)
            .cloned()
            .unwrap_or_else(|| "unknown".to_owned());
        imports.entry(date).or_default().push(paper.id);
    }
    let mut import_dates: Vec<String> = imports.keys().cloned().collect();
    import_dates.sort_by(|a, b| b.cmp(a));
    for date in import_dates {
        let paper_ids = imports.remove(&date).unwrap_or_default();
        groups.push(LibraryGroup {
            kind: GroupKind::Import,
            log_id: None,
            date,
            query: None,
            source_dbs: None,
            hit_count: paper_ids.len(),
            paper_ids,
        });
    }

    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use rusqlite::params;

    /// 検索ログ 1 件（ヒット 2 件のうち Library にあるのは 1 件）+ 検索に紐付かない
    /// 論文 1 件 → `[query(paper_ids=[ヒットした1件], hit_count=2), import(残り)]` になる。
    #[test]
    fn build_groups_separates_query_children_from_imports(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = Database::in_memory()?;
        let conn = db.connection();

        // PDF の実在確認があるため、テスト用の PDF（空ファイル）を一時フォルダに作る。
        let dir =
            std::env::temp_dir().join(format!("sai-paper-library-tree-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let pdf_a = dir.join("a.pdf");
        let pdf_b = dir.join("b.pdf");
        std::fs::write(&pdf_a, "")?;
        std::fs::write(&pdf_b, "")?;
        let pdf_a_str = pdf_a.to_string_lossy().to_string();
        let pdf_b_str = pdf_b.to_string_lossy().to_string();

        // Library 論文 2 件（A = 検索ヒットと一致、B = 検索に紐付かない）。
        conn.execute(
            "INSERT INTO papers (title, arxiv_id, pdf_path) VALUES ('Downloaded A', '2401.00001', ?1)",
            params![pdf_a_str],
        )?;
        let id_a = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO papers (title, pdf_path) VALUES ('Imported B', ?1)",
            params![pdf_b_str],
        )?;
        let id_b = conn.last_insert_rowid();

        let papers = db.library_papers_in_roots(std::slice::from_ref(&dir))?;
        assert_eq!(papers.len(), 2);

        // 検索ログ 1 件（ヒット 2 件のうち 1 件だけ Library にある）。
        let remote = |id: &str, title: &str| RemotePaper {
            id: id.to_string(),
            title: title.to_string(),
            authors: vec!["A. Author".to_string()],
            abstract_text: String::new(),
            published: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            categories: Vec::new(),
            pdf_url: None,
            doi: None,
            journal_ref: None,
            citation_count: None,
        };
        let hits = vec![
            remote("2401.00001", "Something Else"),
            remote("2401.99999", "Not downloaded"),
        ];
        let log_id = crate::search_log::record_search(
            conn,
            "discover",
            "test query",
            "2020-2026",
            "10",
            "arxiv",
            &hits,
        )?;

        let groups = build_groups(conn, &papers);
        assert_eq!(groups.len(), 2, "query 1 まとまり + import 1 まとまり");

        // query: ヒットは 2 件だが Library にあるのは 1 件。
        let query = &groups[0];
        assert_eq!(query.kind, GroupKind::Query);
        assert_eq!(query.log_id, Some(log_id));
        assert_eq!(query.date.len(), 10, "YYYY/MM/DD 形式");
        assert_eq!(query.query.as_deref(), Some("test query"));
        assert_eq!(query.source_dbs.as_deref(), Some("arxiv"));
        assert_eq!(query.hit_count, 2, "hit_count はダウンロードの有無を問わない");
        assert_eq!(query.paper_ids, vec![id_a], "arXiv ID が一致した論文だけ子になる");

        // import: 検索に紐付かない残りの論文。
        let import = &groups[1];
        assert_eq!(import.kind, GroupKind::Import);
        assert_eq!(import.log_id, None);
        assert_eq!(import.query, None);
        assert_eq!(import.paper_ids, vec![id_b]);
        assert_eq!(import.hit_count, 1);
        // 日付は papers.created_at の日付（YYYY/MM/DD）。
        let created: String = conn.query_row(
            "SELECT created_at FROM papers WHERE id = ?1",
            [id_b],
            |row| row.get(0),
        )?;
        let expected = created
            .split(['T', ' '])
            .next()
            .unwrap_or("")
            .replace('-', "/");
        assert_eq!(import.date, expected);

        // JSON では kind は小文字。
        let value = serde_json::to_value(&groups[0])?;
        assert_eq!(value.get("kind").and_then(serde_json::Value::as_str), Some("query"));

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// 候補の `paper_id` が付いた論文は、題名も arXiv ID も DOI も
    /// `remote` と違していても、その検索の子になる。
    #[test]
    fn build_groups_prefers_linked_paper_id_over_metadata_match()
    -> Result<(), Box<dyn std::error::Error>> {
        let db = Database::in_memory()?;
        let conn = db.connection();

        // テストごとに別のフォルダを使う（既存のテストと同じ folder を共有して消すと
        // 並列実行で对方的 PDF を消してしまう）。
        let dir = std::env::temp_dir()
            .join(format!("sai-paper-library-tree-linked-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let pdf = dir.join("linked.pdf");
        std::fs::write(&pdf, "")?;

        conn.execute(
            "INSERT INTO papers (title, pdf_path) VALUES ('Totally Different Title', ?1)",
            params![pdf.to_string_lossy()],
        )?;
        let id = conn.last_insert_rowid();
        let papers = db.library_papers_in_roots(std::slice::from_ref(&dir))?;
        assert_eq!(papers.len(), 1);

        let remote = RemotePaper {
            id: "2401.00001".to_string(),
            title: "Remote Title".to_string(),
            authors: vec!["A. Author".to_string()],
            abstract_text: String::new(),
            published: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            categories: Vec::new(),
            pdf_url: None,
            doi: None,
            journal_ref: None,
            citation_count: None,
        };
        let log_id = crate::search_log::record_search(
            conn,
            "discover",
            "linked",
            "",
            "",
            "arxiv",
            std::slice::from_ref(&remote),
        )?;
        // メタデータは一切一致しないので、`link_paper` だけが紐付けの根拠になる。
        crate::search_sets::link_paper(conn, log_id, 0, id)?;

        let groups = build_groups(conn, &papers);
        assert_eq!(groups.len(), 1, "Import グループが残らない");
        assert_eq!(groups[0].kind, GroupKind::Query);
        assert_eq!(groups[0].paper_ids, vec![id]);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// 検索語が空で名前だけある記録（CLI・エージェントが作った検索セット）は、
    /// 階層表示の `query` に名前が出る（sai-phone の `(検索)` を出さない）。
    #[test]
    fn build_groups_falls_back_to_name_when_query_is_empty()
    -> Result<(), Box<dyn std::error::Error>> {
        let db = Database::in_memory()?;
        let conn = db.connection();
        conn.execute(
            "INSERT INTO search_sets (name, query, source_dbs, origin, created_at)
             VALUES ('seeg-eeg-llm-2026-09', '', 'arxiv', 'cli', '2026-09-26T00:00:00+09:00')",
            [],
        )?;
        let log_id = conn.last_insert_rowid();

        let groups = build_groups(conn, &[]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].log_id, Some(log_id));
        assert_eq!(groups[0].query.as_deref(), Some("seeg-eeg-llm-2026-09"));
        Ok(())
    }

    /// 論文の置き場 `<data_dir>/papers/` は、`library_folders` / `download_path` を
    /// 外部に向けても常に走査対象に含まれる（段階 2a）。逆に、設定した読み込み元も
    /// 今までどおり残る。
    #[test]
    fn library_roots_always_include_papers_dir() -> Result<(), Box<dyn std::error::Error>> {
        let temp = std::env::temp_dir().join(format!(
            "sai-paper-library-roots-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        let papers_dir = temp.join("papers");
        let external = temp.join("external");
        std::fs::create_dir_all(&papers_dir)?;
        std::fs::create_dir_all(&external)?;

        // 論文の置き場を指さない設定（DL 先・読み込み元をすべて外部に向ける）。
        let mut config = Config::default();
        config.library.import_folders = vec![external.clone()];
        config.library.download_path = Some(external.clone());
        let paths = Paths {
            config_file: temp.join("settings.json"),
            database_file: temp.join("papr.db"),
            downloads_dir: papers_dir.clone(),
            plugins_dir: temp.join("plugins"),
            plugins_config_file: temp.join("plugins.toml"),
        };

        let roots = library_roots(&config, &paths);
        let expected_papers = std::fs::canonicalize(&papers_dir)?;
        assert!(
            roots.contains(&expected_papers),
            "論文の置き場が走査対象に含まれること: {roots:?}"
        );
        let expected_external = std::fs::canonicalize(&external)?;
        assert!(
            roots.contains(&expected_external),
            "設定の読み込み元（外部 PDF）が残ること: {roots:?}"
        );
        // 既定設定（download_path 未設定）でも論文の置き場が入る。
        let default_roots = library_roots_for(&Config::default(), &papers_dir);
        assert!(default_roots.contains(&expected_papers));

        let _ = std::fs::remove_dir_all(&temp);
        Ok(())
    }
}
