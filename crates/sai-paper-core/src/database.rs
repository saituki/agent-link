//! `SQLite` persistence and schema migrations.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use chrono::{NaiveDateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use thiserror::Error;

use crate::{
    library::ImportedPdf,
    models::{ActivityItem, LibraryPaper, RemotePaper},
};

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/0001_initial.sql")),
    (2, include_str!("../migrations/0002_library.sql")),
    (
        3,
        include_str!("../migrations/0003_research_organization.sql"),
    ),
    (4, include_str!("../migrations/0004_activity.sql")),
    (
        5,
        include_str!("../migrations/0005_merge_tags_into_collections.sql"),
    ),
    (
        6,
        include_str!("../migrations/0006_filesystem_collections.sql"),
    ),
    (
        7,
        include_str!("../migrations/0007_dashboard_feed_cache.sql"),
    ),
    (8, include_str!("../migrations/0008_reading_queue.sql")),
    (9, include_str!("../migrations/0009_previous_status.sql")),
    (
        10,
        include_str!("../migrations/0010_metadata_enrichment.sql"),
    ),
    (
        11,
        include_str!("../migrations/0011_activity_kind_paper_browsed.sql"),
    ),
    (
        12,
        include_str!("../migrations/0012_activity_kind_project_events.sql"),
    ),
    (
        13,
        include_str!("../migrations/0013_activity_kind_paper_creation_modification.sql"),
    ),
    (
        14,
        include_str!("../migrations/0014_dashboard_feed_history.sql"),
    ),
    (15, include_str!("../migrations/0015_s2_graph.sql")),
    (16, include_str!("../migrations/0016_citation_tables.sql")),
    (17, include_str!("../migrations/0017_translation.sql")),
    (
        18,
        include_str!("../migrations/0018_tags_library_refactor.sql"),
    ),
    (19, include_str!("../migrations/0019_citation_count.sql")),
    (20, include_str!("../migrations/0020_abstract_cleanup.sql")),
    (21, include_str!("../migrations/0021_litbase.sql")),
    (22, include_str!("../migrations/0022_search_sets.sql")),
    (23, include_str!("../migrations/0023_rag_index.sql")),
    (
        24,
        include_str!("../migrations/0024_chunks_paper_id_backfill.sql"),
    ),
    (
        25,
        include_str!("../migrations/0025_paper_extracts_papers_id.sql"),
    ),
    (26, include_str!("../migrations/0026_search_log.sql")),
    (27, include_str!("../migrations/0027_paper_figures.sql")),
    (28, include_str!("../migrations/0028_reports.sql")),
    (
        29,
        include_str!("../migrations/0029_chunks_paper_id_backfill.sql"),
    ),
    (30, include_str!("../migrations/0030_podcasts.sql")),
    (
        31,
        include_str!("../migrations/0031_activity_kind_import_translate_radio.sql"),
    ),
    (
        32,
        include_str!("../migrations/0032_cascade_paper_deletion.sql"),
    ),
    (33, include_str!("../migrations/0033_papers_id_unify.sql")),
    (34, include_str!("../migrations/0034_jobs.sql")),
    (35, include_str!("../migrations/0035_unify_search_logs.sql")),
    (36, include_str!("../migrations/0036_chunk_index_name.sql")),
    (37, include_str!("../migrations/0037_drop_notes_table.sql")),
    (38, include_str!("../migrations/0038_drop_litbase_tables.sql")),
    (39, include_str!("../migrations/0039_report_scan.sql")),
    (40, include_str!("../migrations/0040_citation_checks.sql")),
    (41, include_str!("../migrations/0041_papers_retracted.sql")),
    (42, include_str!("../migrations/0042_chunk_section.sql")),
    (43, include_str!("../migrations/0043_activity_open.sql")),
    (44, include_str!("../migrations/0044_understanding.sql")),
    (
        45,
        include_str!("../migrations/0045_activity_resolved.sql"),
    ),
    (
        46,
        include_str!("../migrations/0046_sci_tree.sql"),
    ),
    (47, include_str!("../migrations/0047_paper_sections.sql")),
    (48, include_str!("../migrations/0048_research_tree.sql")),
    (49, include_str!("../migrations/0049_search_actor.sql")),
    (50, include_str!("../migrations/0050_rt_search_log.sql")),
    (51, include_str!("../migrations/0051_find_log.sql")),
    (
        52,
        include_str!("../migrations/0052_drop_reasons.sql"),
    ),
    (
        53,
        include_str!("../migrations/0053_sprout_branch.sql"),
    ),
    (
        54,
        include_str!("../migrations/0054_top_q.sql"),
    ),
    (
        55,
        include_str!("../migrations/0055_search_history.sql"),
    ),
    (
        56,
        include_str!("../migrations/0056_rt_redesign.sql"),
    ),
    (
        57,
        include_str!("../migrations/0057_seed_node.sql"),
    ),
    (
        58,
        include_str!("../migrations/0058_rt_log_regroup.sql"),
    ),
];

/// 抽出キーで結び付いていた 6 表（段階 2c で `papers_id` を持つ）。
const PAPER_KEY_TABLES: &[&str] = &[
    "paper_refs",
    "paper_sentences",
    "paper_extracts",
    "paper_figures",
    "paper_figure_extracts",
    "paper_translations",
];

/// 既存行への `papers_id` 埋め込みが済んだ印（`app_meta` のキー。段階 2c）。
const PAPERS_ID_BACKFILL_MARKER: &str = "papers_id_backfill_v1";

/// Paper identifier, optional arXiv identifier, and optional local PDF path.
pub type EnrichmentCandidate = (i64, Option<String>, Option<String>);

/// Paper identifier, optional arXiv identifier, DOI, and local PDF path.
pub type EnrichmentCandidateWithDoi = (i64, Option<String>, Option<String>, Option<String>);

/// Write a source-provided author sequence only when the paper has no canonical
/// author metadata yet. Existing rows and their `position` values are authoritative.
fn persist_authors_if_missing(
    transaction: &Transaction<'_>,
    paper_id: i64,
    authors: &[String],
) -> Result<(), DatabaseError> {
    let existing_author_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM paper_authors WHERE paper_id = ?1",
        [paper_id],
        |row| row.get(0),
    )?;
    if existing_author_count > 0 {
        return Ok(());
    }

    for (position, author) in authors.iter().enumerate() {
        let author_id = if let Some(id) = transaction
            .query_row(
                "SELECT id FROM authors WHERE name = ?1 COLLATE NOCASE ORDER BY id LIMIT 1",
                [author],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        {
            id
        } else {
            transaction.execute("INSERT INTO authors (name) VALUES (?1)", [author])?;
            transaction.last_insert_rowid()
        };
        transaction.execute(
            "INSERT OR IGNORE INTO paper_authors (paper_id, author_id, position) VALUES (?1, ?2, ?3)",
            params![
                paper_id,
                author_id,
                i64::try_from(position).unwrap_or(i64::MAX)
            ],
        )?;
    }
    Ok(())
}

/// Database initialization and query errors.
#[derive(Debug, Error)]
pub enum DatabaseError {
    /// Filesystem setup failed.
    #[error("database filesystem setup failed: {0}")]
    Io(#[from] std::io::Error),
    /// `SQLite` operation failed.
    #[error("database operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Cached JSON payload could not be encoded or decoded.
    #[error("database JSON payload failed: {0}")]
    Json(#[from] serde_json::Error),
}

/// Local `SQLite` database with explicit migrations.
#[derive(Debug)]
pub struct Database {
    connection: Connection,
    /// `papr.db` の親ディレクトリ（`data_dir`、`data_dir/.sai/paper/`）。
    ///
    /// DB に書くパスはこの中なら `data_dir` からの相対、外なら絶対（段階 2b）。
    /// `None` のとき（`in_memory`・テスト用）は絶対パスのまま扱う。
    data_dir: Option<PathBuf>,
}

impl Database {
    /// Open the database, configure safe defaults, and apply pending migrations.
    ///
    /// `papr.db` の親を `data_dir` として保持し、既存の絶対パス行を
    /// 相対パスへ 1 回だけ書き換える（[`Self::convert_stored_paths_if_needed`]）。
    ///
    /// # Errors
    ///
    /// Returns an error when the parent directory, database, or migration fails.
    pub fn open(path: &Path) -> Result<Self, DatabaseError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data_dir = path.parent().map(Path::to_path_buf);
        let connection = Connection::open(path)?;
        let mut database = Self::from_connection(connection, data_dir)?;
        database.convert_stored_paths_if_needed();
        database.backfill_papers_id_if_needed();
        Ok(database)
    }

    /// テスト用: `last` の版までの移行を済ませた DB を開く（後の版は入れない）。
    #[cfg(test)]
    fn open_upto(path: &Path, last: i64) -> Result<Self, DatabaseError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data_dir = path.parent().map(Path::to_path_buf);
        let connection = Connection::open(path)?;
        Self::from_connection_upto(connection, data_dir, last)
    }

    /// Create a migrated in-memory database, useful for tests and previews.
    ///
    /// `data_dir` を持たないため、パスは絶対パスのまま扱う（`paths::to_stored_path`
    /// を通らない）。
    ///
    /// # Errors
    ///
    /// Returns an error when database initialization or migration fails.
    pub fn in_memory() -> Result<Self, DatabaseError> {
        Self::from_connection(Connection::open_in_memory()?, None)
    }

    /// テスト用: `data_dir` を指定したメモリ DB（相対保存の規則をそのまま試せる）。
    ///
    /// # Errors
    ///
    /// Returns an error when database initialization or migration fails.
    pub fn in_memory_with_data_dir(data_dir: &Path) -> Result<Self, DatabaseError> {
        Self::from_connection(Connection::open_in_memory()?, Some(data_dir.to_path_buf()))
    }

    /// 生の SQL を解決する側に渡す基準ディレクトリ。
    ///
    /// `in_memory`（テスト用・`data_dir` 無し）では空パスを返す。空パスの連結は
    /// 元のパスをそのまま意味するため、絶対パスのまま扱われる。
    #[must_use]
    pub fn path_base(&self) -> &Path {
        self.data_dir.as_deref().unwrap_or(Path::new(""))
    }

    /// `papr.db` の親（`data_dir`）。`in_memory` では `None`。
    #[must_use]
    pub fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    /// 書く側: DB に保存するパスへ変換する（段階 2b）。
    ///
    /// 生の SQL でこれらの列に書く処理は必ずこれを通すこと。
    /// `data_dir` 無し（`in_memory`）では絶対パスのまま。
    #[must_use]
    pub fn store_path(&self, path: &Path) -> String {
        Self::store_path_with(self.data_dir.as_deref(), path)
    }

    /// 読む側: DB から読んだパスを実ファイルパスへ直す（段階 2b）。
    ///
    /// 生の SQL でこれらの列を読み、ファイルに触れる処理は必ずこれを通すこと。
    /// 絶対パス（外のファイル・古い行）はそのまま返す。
    #[must_use]
    pub fn resolve_path(&self, stored: &str) -> PathBuf {
        Self::resolve_path_with(self.data_dir.as_deref(), stored)
    }

    fn store_path_with(data_dir: Option<&Path>, path: &Path) -> String {
        match data_dir {
            Some(dir) => crate::paths::to_stored_path(dir, path),
            None => path.to_string_lossy().into_owned(),
        }
    }

    fn resolve_path_with(data_dir: Option<&Path>, stored: &str) -> PathBuf {
        match data_dir {
            Some(dir) => crate::paths::resolve_stored_path(dir, stored),
            None => PathBuf::from(stored),
        }
    }

    /// `Option<String>` のパス列をまとめて解決する（読む側）。
    fn resolve_opt(&self, stored: &Option<String>) -> Option<String> {
        stored
            .as_ref()
            .map(|value| self.resolve_path(value).to_string_lossy().into_owned())
    }

    /// 既存の絶対パス行を `data_dir` からの相対パスへ、起動時に 1 回だけ書き換える。
    ///
    /// 済んだことは `app_meta`（`stored_paths_relative_v1`）に印を残して二度行わない。
    /// 変換は 1 つのトランザクションで行い、UNIQUE 違反などで失敗したら全体を
    /// 戻して警告だけ出して続行する（起動は止めない）。`data_dir` の外にあるパス
    /// （`<root>/papers/` や `<root>/data/extracts/` 等）は絶対パスのまま残す。
    /// `in_memory`（`data_dir` 無し）では何もしない。
    fn convert_stored_paths_if_needed(&mut self) {
        const MARKER: &str = "stored_paths_relative_v1";
        let Some(data_dir) = self.data_dir.clone() else {
            return;
        };
        let done: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM app_meta WHERE key = ?1",
                [MARKER],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        if done.as_deref() == Some("1") {
            return;
        }
        match Self::write_converted_paths(&self.connection, &data_dir, MARKER) {
            Ok(0) => {}
            Ok(changed) => {
                eprintln!(
                    "sai-paper: DB 内のパスを data_dir からの相対パスへ変換しました（{changed} 件）"
                );
            }
            Err(error) => {
                eprintln!(
                    "警告: DB 内のパスの相対化に失敗しました（変更は元に戻しました。起動は続行します）: {error}"
                );
            }
        }
    }

    /// 相対化と `app_meta` への印を 1 つのトランザクションで行う。
    ///
    /// 失敗時は呼び出し側で自動的にロールバックされる（`?` で抜けるため）。
    fn write_converted_paths(
        connection: &Connection,
        data_dir: &Path,
        marker_key: &str,
    ) -> Result<usize, rusqlite::Error> {
        /// 変換対象の列（段階 2b の対象）。
        const COLUMNS: &[(&str, &str)] = &[
            ("papers", "pdf_path"),
            ("paper_extracts", "pdf_path"),
            ("paper_extracts", "md_path"),
            ("paper_translations", "pdf_path"),
            ("paper_translations", "pdf_ja_path"),
            ("paper_figures", "image_path"),
            ("paper_figure_extracts", "pdf_path"),
        ];
        let transaction = connection.unchecked_transaction()?;
        let mut changed = 0_usize;
        for (table, column) in COLUMNS {
            let select = format!("SELECT rowid, {column} FROM {table} WHERE {column} IS NOT NULL");
            let rows: Vec<(i64, String)> = {
                let mut statement = transaction.prepare(&select)?;
                statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?
            };
            for (rowid, stored) in rows {
                // 相対パス・空文字は対象外（二度触らない）。
                if stored.trim().is_empty() || Path::new(&stored).is_relative() {
                    continue;
                }
                let converted = crate::paths::to_stored_path(data_dir, &stored);
                if converted == stored {
                    // data_dir の外（絶対パスのまま残す）。
                    continue;
                }
                let update = format!("UPDATE {table} SET {column} = ?1 WHERE rowid = ?2");
                transaction.execute(&update, params![converted, rowid])?;
                changed += 1;
            }
        }
        transaction.execute(
            "INSERT OR REPLACE INTO app_meta (key, value) VALUES (?1, '1')",
            [marker_key],
        )?;
        transaction.commit()?;
        Ok(changed)
    }

    /// Borrow the underlying connection.
    ///
    /// 引用文献パイプライン（`citation`）等、追加機能から既存接続を再利用するためのアクセサ。
    #[must_use]
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// `papers` 行の ID 列（`s2_paper_id` / `arxiv_id` / `doi`）を正規化した
    /// 対応表（正規化キー → `papers.id`）を作る（段階 2c）。
    ///
    /// 正規化は SQL では書けないため Rust 側で [`crate::citation::normalize_paper_id`] を使う。
    /// 同じキーに当たる論文が複数ある場合は番号の小さい方を採用する。
    fn normalized_paper_key_map(
        conn: &Connection,
    ) -> Result<HashMap<String, i64>, rusqlite::Error> {
        let mut statement =
            conn.prepare("SELECT id, s2_paper_id, arxiv_id, doi FROM papers ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (id, s2_paper_id, arxiv_id, doi) = row?;
            for raw in [s2_paper_id, arxiv_id, doi] {
                let Some(value) = raw else { continue };
                if value.trim().is_empty() {
                    continue;
                }
                // 最初に当たった論文（番号が小さい方）を採用する。
                map.entry(crate::citation::normalize_paper_id(&value))
                    .or_insert(id);
            }
        }
        Ok(map)
    }

    /// 抽出キー（S2 40hex / `ArXiv:…` / `DOI:…`）に対応する Library 論文の
    /// `papers.id` を返す（段階 2c）。見つからなければ `None`。
    ///
    /// 既存行への `papers_id` の埋め込みと、書き込む側が `papers_id` を決める
    /// ときに使う。生の SQL で照合する場合は必ずこれを通すこと（正規化は SQL では書けない）。
    #[must_use]
    pub fn papers_id_for_extract_key(conn: &Connection, key: &str) -> Option<i64> {
        let map = Self::normalized_paper_key_map(conn).ok()?;
        map.get(&crate::citation::normalize_paper_id(key)).copied()
    }

    /// 既存行に `papers_id` を入れる（`Database::open` のときに 1 回だけ）。
    ///
    /// 済んだことは `app_meta`（[`PAPERS_ID_BACKFILL_MARKER`]）に印を残して二度行わない。
    /// 変換は 1 つのトランザクションで行い、失敗したら全体を戻して警告だけ
    /// 出力して続行する（起動は止めない）。Library 外の論文（CLI の
    /// `extract <S2 ID>` で抽出したもの等）は見つからないので NULL のまま残る。
    fn backfill_papers_id_if_needed(&self) {
        let done: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM app_meta WHERE key = ?1",
                [PAPERS_ID_BACKFILL_MARKER],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        if done.as_deref() == Some("1") {
            return;
        }
        match self.backfill_papers_id() {
            Ok(0) => {}
            Ok(changed) => {
                eprintln!("sai-paper: 既存の記録に Library の番号を結び付けました（{changed} 件）");
            }
            Err(error) => {
                eprintln!(
                    "警告: 記録への Library の番号の結び付けに失敗しました（変更は元に戻しました。起動は続行します）: {error}"
                );
            }
        }
    }

    /// 6 表で `papers_id` が NULL の行を、抽出キーから探して埋める（段階 2c）。
    ///
    /// 印の有無を見ない明示的な実行（テスト・手動で使える）。1 つのトランザクションで
    /// 行い、途中で失敗すると自動的にロールバックされる。戻り値は埋めた行数。
    ///
    /// # Errors
    ///
    /// SQL の実行に失敗した場合。
    pub fn backfill_papers_id(&self) -> Result<usize, DatabaseError> {
        let transaction = self.connection.unchecked_transaction()?;
        let map = Self::normalized_paper_key_map(&transaction)?;
        let mut changed = 0_usize;
        for table in PAPER_KEY_TABLES {
            let select = format!("SELECT rowid, paper_id FROM {table} WHERE papers_id IS NULL");
            let rows: Vec<(i64, String)> = {
                let mut statement = transaction.prepare(&select)?;
                statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?
            };
            for (rowid, key) in rows {
                // Library 外の論文の行（対応する papers 行が無い）は NULL のまま残す。
                let Some(papers_id) = map.get(&crate::citation::normalize_paper_id(&key)) else {
                    continue;
                };
                let update = format!("UPDATE {table} SET papers_id = ?1 WHERE rowid = ?2");
                transaction.execute(&update, params![papers_id, rowid])?;
                changed += 1;
            }
        }
        transaction.execute(
            "INSERT OR REPLACE INTO app_meta (key, value) VALUES (?1, '1')",
            [PAPERS_ID_BACKFILL_MARKER],
        )?;
        transaction.commit()?;
        Ok(changed)
    }

    fn from_connection(
        connection: Connection,
        data_dir: Option<PathBuf>,
    ) -> Result<Self, DatabaseError> {
        Self::from_connection_upto(connection, data_dir, i64::MAX)
    }

    /// `last` の版まで移行して開く（テストで、後の版の効き目を入れずに確かめるため）。
    fn from_connection_upto(
        connection: Connection,
        data_dir: Option<PathBuf>,
        last: i64,
    ) -> Result<Self, DatabaseError> {
        connection.execute_batch(
            "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 5000;",
        )?;
        let mut database = Self {
            connection,
            data_dir,
        };
        database.migrate_upto(last)?;
        Ok(database)
    }

    fn migrate(&mut self) -> Result<(), DatabaseError> {
        self.migrate_upto(i64::MAX)
    }

    fn migrate_upto(&mut self, last: i64) -> Result<(), DatabaseError> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
        )?;
        for &(version, sql) in MIGRATIONS.iter().filter(|(version, _)| *version <= last) {
            let applied = self
                .connection
                .query_row(
                    "SELECT version FROM schema_migrations WHERE version = ?1",
                    [version],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            if applied.is_none() {
                if version == 37 {
                    // notes 表を消す前に、中身を note.md へ移す（失わない）
                    self.move_notes_table_to_files()?;
                }
                if version == 53 || version == 56 || version == 57 {
                    // rt_nodes を外部キー付きで作り直すため、外部キーの検査を一時的に外す。
                    // PRAGMA はトランザクションの中では効かないので、ここ（外）で切り替える。
                    self.connection.execute_batch("PRAGMA foreign_keys = OFF;")?;
                }
                let transaction = self.connection.transaction()?;
                transaction.execute_batch(sql)?;
                transaction.execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [version],
                )?;
                transaction.commit()?;
                if version == 53 || version == 56 || version == 57 {
                    self.connection.execute_batch("PRAGMA foreign_keys = ON;")?;
                }
            }
        }
        Ok(())
    }

    /// `notes` 表（古いノート置き場）の中身を論文フォルダの `note.md` へ移す
    /// （migration 0037 の直前に 1 回だけ）。
    ///
    /// `note.md` が無ければそのまま書く。あって中身が違えば末尾に足す（どちらも消さない）。
    /// `data_dir` が無い（メモリ DB）ときは何もしない。
    fn move_notes_table_to_files(&self) -> Result<(), DatabaseError> {
        let Some(data_dir) = self.data_dir.clone() else {
            return Ok(());
        };
        let rows = self
            .connection
            .prepare(
                "SELECT n.paper_id, COALESCE(p.title, ''), n.body
                 FROM notes n JOIN papers p ON p.id = n.paper_id
                 WHERE trim(COALESCE(n.body, '')) != ''",
            )?
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (paper_id, title, body) in rows {
            let path = crate::paths::paper_folder(&data_dir, paper_id, &title)
                .join(crate::paths::paper_file_names::NOTE);
            let body = body.trim();
            match fs::read_to_string(&path) {
                Ok(existing) if existing.contains(body) => {}
                Ok(existing) => fs::write(&path, format!("{}\n\n---\n\n{body}\n", existing.trim_end()))?,
                Err(_) => {
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&path, format!("{body}\n"))?;
                }
            }
        }
        Ok(())
    }

    /// Add a minimal paper record while enforcing DOI uniqueness.
    ///
    /// # Errors
    ///
    /// Returns an error when validation or a database constraint fails.
    pub fn insert_paper(&self, title: &str, doi: Option<&str>) -> Result<i64, DatabaseError> {
        self.connection.execute(
            "INSERT INTO papers (title, doi) VALUES (?1, ?2)",
            params![title, doi],
        )?;
        // 出来事には記録しない（ダウンロードは TUI のダウンロード完了時に記録する）
        Ok(self.connection.last_insert_rowid())
    }

    /// Import a locally discovered PDF unless its content hash already exists.
    ///
    /// # Errors
    ///
    /// Returns an error when the record cannot be inserted or queried.
    pub fn import_pdf(&self, pdf: &ImportedPdf) -> Result<bool, DatabaseError> {
        // 書く側は必ず相対保存の規則を通す（段階 2b）。
        let pdf_stored = self.store_path(&pdf.path);
        let existing: Option<(i64, String)> = self
            .connection
            .query_row(
                "SELECT id, pdf_path FROM papers WHERE content_hash = ?1 LIMIT 1",
                params![pdf.content_hash],
                |row| Ok((row.get(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;

        if let Some((id, existing_path)) = existing {
            // 読む側は実ファイルパスへ直してから存在判定する。
            let existing_exists = self.resolve_path(&existing_path).is_file();
            if !existing_exists {
                // Moved file on disk! Update DB path.
                self.connection.execute(
                    "UPDATE papers SET pdf_path = ?1, file_size = ?2, indexed_at = CURRENT_TIMESTAMP WHERE id = ?3",
                    params![
                        pdf_stored,
                        i64::try_from(pdf.file_size).unwrap_or(i64::MAX),
                        id
                    ],
                )?;
                return Ok(true);
            } else if pdf_stored != existing_path {
                // Check if the new path is in a collection and the existing one is not.
                let old_in_collection = self
                    .connection
                    .query_row(
                        "SELECT COUNT(*) FROM collection_papers WHERE paper_id = ?1",
                        [id],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap_or(0)
                    > 0;
                let new_in_collection = pdf.relative_directory.is_some();
                if new_in_collection && !old_in_collection {
                    self.connection.execute(
                        "UPDATE papers SET pdf_path = ?1, file_size = ?2, indexed_at = CURRENT_TIMESTAMP WHERE id = ?3",
                        params![
                            pdf_stored,
                            i64::try_from(pdf.file_size).unwrap_or(i64::MAX),
                            id
                        ],
                    )?;
                    return Ok(true);
                }
                return Ok(false);
            }
            return Ok(false);
        }

        let changed = self.connection.execute(
            "INSERT INTO papers (title, pdf_path, file_size, content_hash, indexed_at)
             VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP)
             ON CONFLICT(pdf_path) DO UPDATE SET
                 content_hash = excluded.content_hash,
                 title = CASE WHEN arxiv_id IS NULL AND doi IS NULL THEN excluded.title ELSE title END,
                 file_size = excluded.file_size,
                 indexed_at = CURRENT_TIMESTAMP",
            params![
                pdf.title,
                pdf_stored,
                i64::try_from(pdf.file_size).unwrap_or(i64::MAX),
                pdf.content_hash,
            ],
        )?;
        if changed > 0 && existing.is_none() {
            let paper_id = self.connection.last_insert_rowid();
            self.record_paper_activity("import", Some(paper_id), Some(&pdf.title))?;
        }
        Ok(changed > 0)
    }

    /// Resolve an imported PDF by content hash or current path.
    ///
    /// # Errors
    /// Returns an error when the lookup fails.
    pub fn paper_id_for_pdf(&self, pdf: &ImportedPdf) -> Result<Option<i64>, DatabaseError> {
        self.paper_id_for_path(&pdf.path.to_string_lossy())
    }

    /// Resolve a paper ID from a PDF path.
    ///
    /// # Errors
    /// Returns an error when the lookup fails.
    pub fn paper_id_for_path(&self, path: &str) -> Result<Option<i64>, DatabaseError> {
        // 比較前に DB 保存形へ変換する（絶対パスで照合すると相対保存では当たらない）。
        let stored = self.store_path(Path::new(path));
        self.connection
            .query_row(
                "SELECT id FROM papers WHERE pdf_path = ?1 LIMIT 1",
                params![stored],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// List catalog entries that have a local PDF with author display metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the library query fails.
    pub fn library_papers(&self) -> Result<Vec<LibraryPaper>, DatabaseError> {
        let mut statement = self.connection.prepare(
            "SELECT p.id, p.title,
                    COALESCE((SELECT GROUP_CONCAT(name, ', ')
                              FROM (SELECT a.name
                                    FROM paper_authors pa JOIN authors a ON a.id = pa.author_id
                                    WHERE pa.paper_id = p.id
                                    ORDER BY pa.position)), ''),
                    p.doi, p.arxiv_id, p.pdf_path, p.file_size, p.reading_status, p.is_favorite,
                    p.abstract, p.abstract_ja, p.abstract_ja_translated_at, p.journal, p.published_at, p.updated_at, p.citation_count, p.is_retracted, p.retraction_checked_at
             FROM papers p
             WHERE p.pdf_path IS NOT NULL
               AND lower(p.pdf_path) NOT LIKE '%-dual.pdf'
               AND lower(p.pdf_path) NOT LIKE '%-ja.pdf'
             ORDER BY p.created_at DESC, p.id DESC",
        )?;
        let rows = statement.query_map([], |row| {
            let file_size: Option<i64> = row.get(6)?;
            Ok(LibraryPaper {
                id: row.get(0)?,
                title: row.get(1)?,
                authors: row.get(2)?,
                doi: row.get(3)?,
                arxiv_id: row.get(4)?,
                // 読む側は実ファイルパスへ解決する（段階 2b）。
                pdf_path: self.resolve_opt(&row.get(5)?),
                file_size: file_size.and_then(|size| u64::try_from(size).ok()),
                reading_status: row.get(7)?,
                is_favorite: row.get(8)?,
                abstract_text: row.get(9)?,
                journal: row.get(12)?,
                published_at: row.get(13)?,
                updated_at: row.get(14)?,
                abstract_ja: row.get(10)?,
                abstract_ja_translated_at: row.get(11)?,
                citation_count: row.get(15)?,
                is_retracted: row.get(16)?,
                retraction_checked_at: row.get(17)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Retrieve a single paper by its database ID formatted as a `LibraryPaper`.
    ///
    /// # Errors
    ///
    /// Returns an error when the database query fails.
    pub fn library_paper_by_id(
        &self,
        paper_id: i64,
    ) -> Result<Option<LibraryPaper>, DatabaseError> {
        let mut statement = self.connection.prepare(
            "SELECT p.id, p.title,
                    COALESCE((SELECT GROUP_CONCAT(name, ', ')
                              FROM (SELECT a.name
                                    FROM paper_authors pa JOIN authors a ON a.id = pa.author_id
                                    WHERE pa.paper_id = p.id
                                    ORDER BY pa.position)), ''),
                    p.doi, p.arxiv_id, p.pdf_path, p.file_size, p.reading_status, p.is_favorite,
                    p.abstract, p.abstract_ja, p.abstract_ja_translated_at, p.journal, p.published_at, p.updated_at, p.citation_count, p.is_retracted, p.retraction_checked_at
             FROM papers p
             WHERE p.id = ?1",
        )?;
        let mut rows = statement.query_map([paper_id], |row| {
            let file_size: Option<i64> = row.get(6)?;
            Ok(LibraryPaper {
                id: row.get(0)?,
                title: row.get(1)?,
                authors: row.get(2)?,
                doi: row.get(3)?,
                arxiv_id: row.get(4)?,
                // 読む側は実ファイルパスへ解決する（段階 2b）。
                pdf_path: self.resolve_opt(&row.get(5)?),
                file_size: file_size.and_then(|size| u64::try_from(size).ok()),
                reading_status: row.get(7)?,
                is_favorite: row.get(8)?,
                abstract_text: row.get(9)?,
                journal: row.get(12)?,
                published_at: row.get(13)?,
                updated_at: row.get(14)?,
                abstract_ja: row.get(10)?,
                abstract_ja_translated_at: row.get(11)?,
                citation_count: row.get(15)?,
                is_retracted: row.get(16)?,
                retraction_checked_at: row.get(17)?,
            })
        })?;
        if let Some(row) = rows.next() {
            Ok(Some(row?))
        } else {
            Ok(None)
        }
    }

    /// List local PDF-backed papers whose files are inside the configured roots.
    ///
    /// # Errors
    ///
    /// Returns an error when the library query fails.
    pub fn library_papers_in_roots(
        &self,
        roots: &[PathBuf],
    ) -> Result<Vec<LibraryPaper>, DatabaseError> {
        Ok(self
            .library_papers()?
            .into_iter()
            .filter(|paper| {
                paper
                    .pdf_path
                    .as_deref()
                    .is_some_and(|path| crate::paths::is_file_in_roots(Path::new(path), roots))
            })
            .collect())
    }

    /// Upsert downloaded arXiv metadata and attach its local PDF path.
    ///
    /// PDF の置き場（論文フォルダ）は `data_dir`（`.sai/paper/`）と
    /// `paths::paper_folder` の規則だけで決まる。設定の `download_path` や
    /// `library_root` には依存しない。
    ///
    /// # Errors
    ///
    /// Returns an error when paper or author persistence fails.
    pub fn attach_download(
        &mut self,
        paper: &RemotePaper,
        pdf: &ImportedPdf,
        data_dir: &Path,
    ) -> Result<i64, DatabaseError> {
        use crate::paths::{move_pdf_file, paper_file_path, paper_folder};

        // 書く側は必ず相対保存の規則を通す（段階 2b）。トランザクション
        // （`self.connection` の可変借用）より先に計算しておく。
        let pdf_stored = self.store_path(&pdf.path);
        let data_dir_owned = self.data_dir.clone();
        let transaction = self.connection.transaction()?;
        let matching_ids: Vec<i64> = {
            let mut stmt = transaction.prepare(
                "SELECT id FROM papers
                 WHERE arxiv_id = ?1 OR (?2 IS NOT NULL AND doi = ?2)
                    OR (?3 IS NOT NULL AND s2_paper_id = ?3)
                    OR pdf_path = ?4 OR (?5 IS NOT NULL AND content_hash = ?5)
                 ORDER BY CASE WHEN s2_paper_id = ?3 THEN 0 WHEN arxiv_id = ?1 THEN 1 ELSE 2 END",
            )?;
            let rows = stmt.query_map(
                params![
                    paper.id,
                    paper.doi,
                    paper.id,
                    pdf_stored.as_str(),
                    pdf.content_hash
                ],
                |row| row.get(0),
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        let file_size = i64::try_from(pdf.file_size).unwrap_or(i64::MAX);
        let paper_id = if let Some(paper_id) = matching_ids.first().copied() {
            for id in matching_ids.into_iter().skip(1) {
                transaction
                    .execute("DELETE FROM paper_authors WHERE paper_id = ?1", [id])
                    .ok();
                transaction
                    .execute("DELETE FROM collection_papers WHERE paper_id = ?1", [id])
                    .ok();
                transaction.execute("DELETE FROM papers WHERE id = ?1", [id])?;
            }

            transaction.execute(
                "UPDATE papers SET title = ?1,
                    abstract = CASE WHEN trim(?2) = '' THEN abstract ELSE ?2 END,
                    doi = ?3, arxiv_id = ?4, s2_paper_id = ?5,
                    published_at = ?6, updated_at = ?7, pdf_path = ?8,
                    file_size = ?9, content_hash = ?10, journal = ?11,
                    citation_count = COALESCE(?12, citation_count), indexed_at = CURRENT_TIMESTAMP WHERE id = ?13",
                params![
                    paper.title,
                    paper.abstract_text,
                    paper.doi,
                    paper.id,
                    paper.id,
                    paper.published.to_rfc3339(),
                    paper.updated.to_rfc3339(),
                    pdf_stored.as_str(),
                    file_size,
                    pdf.content_hash,
                    paper.journal_ref,
                    paper.citation_count.map(i64::from),
                    paper_id
                ],
            )?;
            paper_id
        } else {
            transaction.execute(
                "INSERT INTO papers
                    (title, abstract, doi, arxiv_id, s2_paper_id, published_at, updated_at, pdf_path,
                     file_size, content_hash, journal, citation_count, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, CURRENT_TIMESTAMP)",
                params![
                    paper.title,
                    paper.abstract_text,
                    paper.doi,
                    paper.id,
                    paper.id,
                    paper.published.to_rfc3339(),
                    paper.updated.to_rfc3339(),
                    pdf_stored.as_str(),
                    file_size,
                    pdf.content_hash,
                    paper.journal_ref,
                    paper.citation_count.map(i64::from)
                ],
            )?;
            transaction.last_insert_rowid()
        };

        // PDF を置く論文フォルダを決める（置き場所はこの関数だけが決める）。
        let folder = paper_folder(data_dir, paper_id, &paper.title);
        let dest_path = paper_file_path(&folder, "pdf");
        // DB には data_dir からの相対パスで書く（段階 2b）。
        let new_path_str = Self::store_path_with(data_dir_owned.as_deref(), &dest_path);

        // DB の pdf_path を新しいパスに更新
        transaction.execute(
            "UPDATE papers SET pdf_path = ?1 WHERE id = ?2",
            params![new_path_str, paper_id],
        )?;

        persist_authors_if_missing(&transaction, paper_id, &paper.authors)?;
        transaction.commit()?;

        // トランザクション完了後、PDF を論文フォルダへ移動する。
        // 書き込む前にフォルダを作る。移動元が無い場合は空フォルダを作らない。
        if pdf.path.is_file() && std::fs::create_dir_all(&folder).is_ok() {
            let _ = move_pdf_file(&pdf.path, &dest_path);
        }

        Ok(paper_id)
    }

    /// 既存論文にユーザーが手動取得した PDF を紐付ける（手動取り込み用）。
    ///
    /// `pdf_path` / `file_size` / `content_hash` を更新し、ライブラリ認識を
    /// リフレッシュするため `indexed_at` も現在時刻に揃える。
    ///
    /// # Errors
    ///
    /// Returns an error when the update fails.
    pub fn attach_manual_pdf(&self, paper_id: i64, pdf: &ImportedPdf) -> Result<(), DatabaseError> {
        self.connection.execute(
            "UPDATE papers SET pdf_path = ?1, file_size = ?2, content_hash = ?3,
                indexed_at = CURRENT_TIMESTAMP WHERE id = ?4",
            params![
                self.store_path(&pdf.path),
                i64::try_from(pdf.file_size).unwrap_or(i64::MAX),
                pdf.content_hash,
                paper_id
            ],
        )?;
        Ok(())
    }

    /// Resolve the citation extraction key for a library paper.
    ///
    /// 引用文献抽出のキー（`s2_paper_id` → `arxiv_id` → `doi`）を優先順に返す。
    /// 呼び出し側は `citation::normalize_paper_id` で正規化して使用する。
    #[must_use]
    pub fn citation_key_for_paper(&self, paper_id: i64) -> Option<String> {
        let key: Option<Option<String>> = self
            .connection
            .query_row(
                "SELECT s2_paper_id, arxiv_id, doi FROM papers WHERE id = ?1",
                [paper_id],
                |row| {
                    let s2: Option<String> = row.get(0)?;
                    let arxiv: Option<String> = row.get(1)?;
                    let doi: Option<String> = row.get(2)?;
                    Ok(s2.or(arxiv).or(doi))
                },
            )
            .optional()
            .ok()
            .flatten();
        key.flatten().filter(|k| !k.trim().is_empty())
    }

    /// Ensure a remote paper has a local metadata record and return its identifier.
    ///
    /// # Errors
    ///
    /// Returns an error when metadata cannot be queried or persisted.
    pub fn ensure_remote_paper(&mut self, paper: &RemotePaper) -> Result<i64, DatabaseError> {
        let transaction = self.connection.transaction()?;
        let existing = transaction
            .query_row(
                "SELECT id FROM papers WHERE arxiv_id = ?1 OR (?2 IS NOT NULL AND doi = ?2)
                 ORDER BY CASE WHEN arxiv_id = ?1 THEN 0 ELSE 1 END LIMIT 1",
                params![paper.id, paper.doi],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let paper_id = if let Some(id) = existing {
            transaction.execute(
                "UPDATE papers SET title = ?1,
                    abstract = CASE WHEN trim(?2) = '' THEN abstract ELSE ?2 END,
                    arxiv_id = ?3,
                 doi = COALESCE(?4, doi), published_at = ?5, updated_at = ?6, journal = ?7,
                 citation_count = COALESCE(?8, citation_count) WHERE id = ?9",
                params![
                    paper.title,
                    paper.abstract_text,
                    paper.id,
                    paper.doi,
                    paper.published.to_rfc3339(),
                    paper.updated.to_rfc3339(),
                    paper.journal_ref,
                    paper.citation_count.map(i64::from),
                    id
                ],
            )?;
            id
        } else {
            transaction.execute(
                "INSERT INTO papers (title, abstract, arxiv_id, doi, published_at, updated_at, journal, citation_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    paper.title,
                    paper.abstract_text,
                    paper.id,
                    paper.doi,
                    paper.published.to_rfc3339(),
                    paper.updated.to_rfc3339(),
                    paper.journal_ref,
                    paper.citation_count.map(i64::from)
                ],
            )?;
            transaction.last_insert_rowid()
        };
        persist_authors_if_missing(&transaction, paper_id, &paper.authors)?;
        transaction.commit()?;
        Ok(paper_id)
    }

    /// ノート（論文フォルダの `note.md`、空でないもの）を持つ論文を、ノートを
    /// 新しく書いた順に返す（PDF が `roots` の中にある論文だけ）。
    ///
    /// `data_dir` が無い（メモリ DB）ときは空。
    ///
    /// # Errors
    ///
    /// Returns an error when the paper query fails.
    pub fn papers_with_notes(&self, roots: &[PathBuf]) -> Result<Vec<LibraryPaper>, DatabaseError> {
        let Some(data_dir) = self.data_dir.as_deref() else {
            return Ok(Vec::new());
        };
        let mut noted: Vec<(std::time::SystemTime, LibraryPaper)> = self
            .library_papers()?
            .into_iter()
            .filter(|paper| {
                paper
                    .pdf_path
                    .as_deref()
                    .is_some_and(|path| crate::paths::is_file_in_roots(Path::new(path), roots))
            })
            .filter_map(|paper| {
                let path = crate::paths::paper_folder(data_dir, paper.id, &paper.title)
                    .join(crate::paths::paper_file_names::NOTE);
                let text = fs::read_to_string(&path).ok()?;
                if text.trim().is_empty() {
                    return None;
                }
                let modified = fs::metadata(&path).and_then(|m| m.modified()).ok()?;
                Some((modified, paper))
            })
            .collect();
        noted.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(noted.into_iter().map(|(_, paper)| paper).collect())
    }

    /// Delete a paper by its database ID.
    ///
    /// 索引（`chunks` / `chunk_embeddings`）と `papers.id` に結び付く表は
    /// マイグレーション 0032 / 0033 の `ON DELETE CASCADE` で消える。
    /// 抽出キーで結び付く表（`paper_refs` 等）は外部キーの対象外だが、
    /// `papers_id` が NULL のまま残った古い行が在るため、削除前に
    /// [`Self::citation_key_for_paper`] でキーを求めてこちらでも消す。
    ///
    /// # Errors
    /// Returns an error when the deletion fails.
    pub fn delete_paper(&self, paper_id: i64) -> Result<(), DatabaseError> {
        // 抽出キーは削除の「前」に求める（papers 行が消えると取得できない）。
        let key = self
            .citation_key_for_paper(paper_id)
            .map(|raw| crate::citation::normalize_paper_id(&raw));

        let transaction = self.connection.unchecked_transaction()?;
        if let Some(key) = key.as_deref() {
            // 抽出キー（正規化済み: 40hex / ArXiv:… / DOI:…）で結び付く表。
            transaction.execute("DELETE FROM paper_refs WHERE paper_id = ?1", [key])?;
            transaction.execute(
                "DELETE FROM paper_sentences WHERE paper_id = ?1",
                [key],
            )?;
            transaction.execute(
                "DELETE FROM paper_translations WHERE paper_id = ?1",
                [key],
            )?;
            transaction.execute(
                "DELETE FROM paper_extracts WHERE paper_id = ?1",
                [key],
            )?;
            transaction.execute(
                "DELETE FROM paper_figures WHERE paper_id = ?1",
                [key],
            )?;
            transaction.execute(
                "DELETE FROM paper_figure_extracts WHERE paper_id = ?1",
                [key],
            )?;
        }
        // papers.id で結び付く行（6 表の papers_id 列を含む）は 0032 / 0033 の
        // ON DELETE CASCADE で、下の `DELETE FROM papers` のときに消える。
        // 音声化の状態は papers.id（整数）で結び付くが、外部キーが未宣言なので手で消す。
        transaction.execute(
            "DELETE FROM paper_podcasts WHERE paper_id = ?1",
            [paper_id],
        )?;
        // 検索履歴から取り込んだ論文の行は、papers を NO ACTION で参照している
        // （マイグレーション 0022）。このまま papers を消すと
        // 「FOREIGN KEY constraint failed」になるため、履歴の候補の行自体は残した
        // まま paper_id を外す。
        transaction.execute(
            "UPDATE search_history_items
                SET paper_id = NULL
              WHERE paper_id = ?1",
            [paper_id],
        )?;
        // 残り（chunks / chunk_embeddings / notes / collection_papers 等）は CASCADE に任せる。
        transaction.execute("DELETE FROM papers WHERE id = ?1", [paper_id])?;
        transaction.commit()?;
        Ok(())
    }

    /// Update a paper's path after renaming it on the filesystem.
    ///
    /// # Errors
    /// Returns an error when the database cannot be updated.
    pub fn rename_pdf(&self, paper_id: i64, new_path: &Path) -> Result<(), DatabaseError> {
        // 名前の変更は出来事に記録しない（download と紛らわしいため）
        self.connection.execute(
            "UPDATE papers SET pdf_path = ?1 WHERE id = ?2",
            params![self.store_path(new_path), paper_id],
        )?;
        Ok(())
    }

    /// Record a paper or PDF open in reading history and recent activity.
    ///
    /// # Errors
    ///
    /// Returns an error when either history record cannot be inserted.
    pub fn record_open(&self, paper_id: i64, pdf: bool) -> Result<i64, DatabaseError> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO reading_history (paper_id) VALUES (?1)",
            [paper_id],
        )?;
        let session_id = transaction.last_insert_rowid();
        // PDF を開くことは「download 済み論文の利用」だが種別は download / discover の
        // 3 種に収まらないため、activity_log へは記録しない（reading_history のみ）。

        if pdf {
            transaction.execute(
                "UPDATE papers SET last_opened_at = CURRENT_TIMESTAMP, reading_status = 'read' WHERE id = ?1",
                [paper_id],
            )?;
        } else {
            transaction.execute(
                "UPDATE papers SET last_opened_at = CURRENT_TIMESTAMP WHERE id = ?1",
                [paper_id],
            )?;
        }
        transaction.commit()?;
        Ok(session_id)
    }

    /// Record the duration of a reading session.
    ///
    /// # Errors
    ///
    /// Returns an error when the history record cannot be updated.
    pub fn record_reading_duration(
        &self,
        session_id: i64,
        duration_s: u64,
    ) -> Result<(), DatabaseError> {
        self.connection.execute(
            "UPDATE reading_history SET duration_s = ?1 WHERE id = ?2",
            params![duration_s, session_id],
        )?;
        Ok(())
    }

    /// Reset a paper's current reading state to unread.
    ///
    /// # Errors
    ///
    /// Returns an error when the paper cannot be updated.
    pub fn mark_unread(&self, paper_id: i64) -> Result<(), DatabaseError> {
        self.connection.execute(
            "UPDATE papers SET reading_status = 'unread' WHERE id = ?1",
            [paper_id],
        )?;
        Ok(())
    }

    /// Record a non-reading research activity.
    ///
    /// # Errors
    ///
    /// Returns an error when the event cannot be inserted.
    pub fn record_activity(
        &self,
        kind: &str,
        paper_id: Option<i64>,
        detail: Option<&str>,
    ) -> Result<(), DatabaseError> {
        // activity_log の種別は normalize_activity_kind の 6 種（0031）。旧種別は対応付け、
        // 対応しない種別は記録せず成功扱いにする。
        let Some(kind) = Self::normalize_activity_kind(kind) else {
            return Ok(());
        };
        self.connection.execute(
            "INSERT INTO activity_log (kind, paper_id, detail) VALUES (?1, ?2, ?3)",
            params![kind, paper_id, detail],
        )?;
        Ok(())
    }

    /// 同じ対象・同じ種類の記録が `window_minutes` 分以内にあれば足さず `false`、
    /// 無ければ足して `true` を返す（N01）。
    ///
    /// `target_key` は [`crate::activity::target_key`] で作った重複判定のキー
    /// （`detail` の `path`、無ければ `target`＋`paper_id`）。既存の行のキーも
    /// 同じ規則で取り出す。時刻の比較は `occurred_at`（UTC）で行う。
    ///
    /// # Errors
    ///
    /// Returns an error when database operations fail.
    pub fn record_activity_dedup(
        &self,
        kind: &str,
        paper_id: Option<i64>,
        detail_json: &str,
        target_key: &str,
        window_minutes: i64,
    ) -> Result<bool, DatabaseError> {
        // 対応しない旧種別は記録しない（成功扱い）。
        let Some(kind) = Self::normalize_activity_kind(kind) else {
            return Ok(false);
        };
        // `datetime('now')` は `CURRENT_TIMESTAMP` と同じ UTC の書式を返す。
        let since = format!("-{window_minutes} minutes");
        let duplicates: Vec<(Option<String>, Option<i64>)> = {
            let mut statement = self.connection.prepare(
                "SELECT detail, paper_id FROM activity_log
                 WHERE kind = ?1 AND occurred_at >= datetime('now', ?2)",
            )?;
            statement
                .query_map(params![kind, since], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        for (detail, existing_paper_id) in duplicates {
            let detail = detail.unwrap_or_default();
            if crate::activity::target_key(&detail, existing_paper_id) == target_key {
                return Ok(false);
            }
        }
        self.connection.execute(
            "INSERT INTO activity_log (kind, paper_id, detail) VALUES (?1, ?2, ?3)",
            params![kind, paper_id, detail_json],
        )?;
        Ok(true)
    }

    /// activity_log の新種別への正規化。対応しない旧種別は None。
    #[must_use]
    fn normalize_activity_kind(kind: &str) -> Option<&'static str> {
        match kind {
            // 新しい名前はそのまま（download はこれまで記録されず捨てられていた）
            "download" | "downloaded" => Some("download"),
            "discover" => Some("discover"),
            "notes" => Some("notes"),
            "import" => Some("import"),
            "translate" => Some("translate"),
            "radio" => Some("radio"),
            "open" => Some("open"),
            "agent_read" => Some("agent_read"),
            "resolved" => Some("resolved"),
            "search" => Some("discover"),
            "note_opened" => Some("notes"),
            _ => None,
        }
    }

    /// Record a paper-specific activity, avoiding consecutive duplicates for the same kind and paper within 5 minutes.
    ///
    /// # Errors
    ///
    /// Returns an error when database operations fail.
    pub fn record_paper_activity(
        &self,
        kind: &str,
        paper_id: Option<i64>,
        detail: Option<&str>,
    ) -> Result<(), DatabaseError> {
        // 対応しない旧種別は記録しない。
        let Some(kind) = Self::normalize_activity_kind(kind) else {
            return Ok(());
        };
        let recent: Option<(i64, String, Option<i64>, String, NaiveDateTime)> = self
            .connection
            .query_row(
                "SELECT id, kind, paper_id, COALESCE(detail, ''), occurred_at
             FROM activity_log
             ORDER BY occurred_at DESC, id DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;

        if let Some((id, last_kind, last_paper_id, last_detail, occurred_at)) = recent {
            let same_paper = match (paper_id, last_paper_id) {
                (Some(pid), Some(lpid)) => pid == lpid,
                (None, None) => detail.unwrap_or("") == last_detail,
                _ => false,
            };

            if last_kind == kind && same_paper {
                let now = chrono::Utc::now().naive_utc();
                let diff = now.signed_duration_since(occurred_at);
                if diff.num_seconds() >= 0 && diff.num_seconds() < 300 {
                    if let Some(det) = detail {
                        self.connection.execute(
                            "UPDATE activity_log SET occurred_at = CURRENT_TIMESTAMP, detail = ?1 WHERE id = ?2",
                            params![det, id],
                        )?;
                    } else {
                        self.connection.execute(
                            "UPDATE activity_log SET occurred_at = CURRENT_TIMESTAMP WHERE id = ?1",
                            [id],
                        )?;
                    }
                    return Ok(());
                }
            }
        }

        self.connection.execute(
            "INSERT INTO activity_log (kind, paper_id, detail) VALUES (?1, ?2, ?3)",
            params![kind, paper_id, detail],
        )?;
        Ok(())
    }

    /// Return recent activity for dashboard and history views.
    ///
    /// # Errors
    ///
    /// Returns an error when the activity query fails.
    pub fn recent_activity(&self, limit: u16) -> Result<Vec<ActivityItem>, DatabaseError> {
        let mut statement = self.connection.prepare(
            "SELECT a.kind, COALESCE(p.title, a.detail, a.kind), a.occurred_at, a.paper_id
             FROM activity_log a LEFT JOIN papers p ON p.id = a.paper_id
             ORDER BY a.occurred_at DESC, a.id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit], |row| {
            let occurred_at: NaiveDateTime = row.get(2)?;
            Ok(ActivityItem {
                kind: row.get(0)?,
                label: row.get(1)?,
                occurred_at: occurred_at.and_utc(),
                paper_id: row.get(3)?,
                radio_id: None,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// List all recent activity items (full history for the RA list).
    ///
    /// # Errors
    ///
    /// Returns an error when the database query fails.
    pub fn all_activity(&self) -> Result<Vec<ActivityItem>, DatabaseError> {
        self.recent_activity(u16::MAX)
    }

    /// Create a tag if missing and return its ID.
    ///
    /// # Errors
    ///
    /// Returns an error when the database query fails.
    pub fn ensure_tag(&self, name: &str) -> Result<i64, DatabaseError> {
        self.connection
            .execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [name])?;
        let id: i64 = self
            .connection
            .query_row(
                "SELECT id FROM tags WHERE name = ?1 COLLATE NOCASE",
                [name],
                |row| row.get(0),
            )
            .map_err(DatabaseError::from)?;
        Ok(id)
    }

    /// Assign a tag to a paper (creating the tag if missing). Returns true when newly added.
    ///
    /// # Errors
    ///
    /// Returns an error when the database query fails.
    pub fn add_paper_tag(&self, paper_id: i64, name: &str) -> Result<bool, DatabaseError> {
        let tag_id = self.ensure_tag(name)?;
        let inserted = self.connection.execute(
            "INSERT OR IGNORE INTO paper_tags (paper_id, tag_id) VALUES (?1, ?2)",
            params![paper_id, tag_id],
        )?;
        Ok(inserted > 0)
    }

    /// タグの差分同期（R1）。
    ///
    /// 確定時に入力欄のタグ集合に合わせて DB を更新する:
    /// 追加されたタグを `paper_tags` に登録し、入力欄から消えたタグを削除する。
    /// 空入力で確定した場合は全タグが外れる（差分同期の自然な帰結）。
    ///
    /// # Errors
    ///
    /// Returns an error when the database query fails.
    pub fn sync_paper_tags(&self, paper_id: i64, names: &[String]) -> Result<(), DatabaseError> {
        // 既存付与タグ集合を取得
        let current = self.tags_for_paper(paper_id)?;
        // 入力欄の集合（重複排除・大文字小文字を同一視するため tags_for_paper の照合順に合わせる）
        let desired: Vec<String> = names
            .iter()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect();
        // 追加: 入力欄にあり DB に無いタグ
        for name in &desired {
            if !current.iter().any(|c| c.eq_ignore_ascii_case(name)) {
                self.add_paper_tag(paper_id, name)?;
            }
        }
        // 削除: 現在付き、入力欄から消えたタグ
        for name in &current {
            if !desired.iter().any(|d| d.eq_ignore_ascii_case(name)) {
                self.remove_paper_tag(paper_id, name)?;
            }
        }
        Ok(())
    }

    /// Web（`phone_cite`・`phone_import`）の Library 照合用に
    /// papers テーブルの最小限情報（arxiv_id / doi / s2_paper_id / title / pdf_path）を
    /// 全件取得する（R5）。
    ///
    /// # Errors
    ///
    /// Returns an error when the database query fails.
    pub fn library_match_entries(
        &self,
    ) -> Result<Vec<crate::models::LibraryMatchEntry>, DatabaseError> {
        let mut statement = self
            .connection
            .prepare("SELECT id, arxiv_id, doi, s2_paper_id, title, pdf_path FROM papers")?;
        let rows = statement
            .query_map([], |row| {
                Ok(crate::models::LibraryMatchEntry {
                    id: row.get(0)?,
                    arxiv_id: row.get(1)?,
                    doi: row.get(2)?,
                    s2_paper_id: row.get(3)?,
                    title: row.get(4)?,
                    // 読む側は実ファイルパスへ解決する（段階 2b）。
                    pdf_path: self.resolve_opt(&row.get(5)?),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Remove a tag from a paper. Returns true when it was attached.
    ///
    /// # Errors
    ///
    /// Returns an error when the database query fails.
    pub fn remove_paper_tag(&self, paper_id: i64, name: &str) -> Result<bool, DatabaseError> {
        let changed = self.connection.execute(
            "DELETE FROM paper_tags WHERE paper_id = ?1
             AND tag_id = (SELECT id FROM tags WHERE name = ?2 COLLATE NOCASE)",
            params![paper_id, name],
        )?;
        Ok(changed > 0)
    }

    /// Tag names attached to a paper, alphabetical.
    ///
    /// # Errors
    ///
    /// Returns an error when the database query fails.
    pub fn tags_for_paper(&self, paper_id: i64) -> Result<Vec<String>, DatabaseError> {
        let mut statement = self.connection.prepare(
            "SELECT t.name FROM tags t
             JOIN paper_tags pt ON pt.tag_id = t.id
             WHERE pt.paper_id = ?1
             ORDER BY t.name COLLATE NOCASE ASC",
        )?;
        let rows = statement.query_map([paper_id], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Papers carrying the given tag, newest first.
    ///
    /// # Errors
    ///
    /// Returns an error when the database query fails.
    pub fn papers_for_tag_name(&self, name: &str) -> Result<Vec<LibraryPaper>, DatabaseError> {
        let mut statement = self.connection.prepare(
            "SELECT p.id FROM papers p
             JOIN paper_tags pt ON pt.paper_id = p.id
             JOIN tags t ON t.id = pt.tag_id
             WHERE t.name = ?1 COLLATE NOCASE
             ORDER BY p.created_at DESC, p.id DESC",
        )?;
        let ids: Vec<i64> = statement
            .query_map([name], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut papers = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(paper) = self.library_paper_by_id(id)? {
                papers.push(paper);
            }
        }
        Ok(papers)
    }

    /// Return local papers that can be enriched by a scholarly provider.
    ///
    /// Each entry is `(paper_id, candidate_arxiv_id, pdf_path)` ready for API lookup.
    ///
    /// # Errors
    /// Returns an error when the query fails.
    pub fn papers_needing_enrichment(&self) -> Result<Vec<EnrichmentCandidate>, DatabaseError> {
        Ok(self
            .papers_needing_enrichment_with_doi()?
            .into_iter()
            .map(|(id, arxiv_id, _, pdf_path)| (id, arxiv_id, pdf_path))
            .collect())
    }

    /// Return enrichable papers including their stored DOI for provider selection.
    /// Each entry is `(paper_id, candidate_arxiv_id, doi, pdf_path)`.
    ///
    /// # Errors
    ///
    /// Returns an error when the enrichment candidate query fails.
    pub fn papers_needing_enrichment_with_doi(
        &self,
    ) -> Result<Vec<EnrichmentCandidateWithDoi>, DatabaseError> {
        let mut statement = self.connection.prepare(
            "SELECT p.id, p.arxiv_id, p.doi, p.pdf_path
             FROM papers p
             WHERE p.pdf_path IS NOT NULL
               AND (
                   (p.enrichment_status != 'success' AND (
                       p.enrichment_status = 'pending'
                       OR (p.enrichment_status = 'failed' AND (
                           p.last_enrichment_attempt IS NULL
                           OR datetime(p.last_enrichment_attempt) <= datetime('now', '-5 minutes')
                       ))
                       OR (p.enrichment_status = 'unavailable' AND (
                           p.last_enrichment_attempt IS NULL
                           OR datetime(p.last_enrichment_attempt) <= datetime('now', '-24 hours')
                       ))
                   ))
                   OR ((p.journal IS NULL OR trim(p.journal) = '') AND (
                       p.last_enrichment_attempt IS NULL
                       OR datetime(p.last_enrichment_attempt) <= datetime('now', '-24 hours')
                   ))
               )",
        )?;
        let rows = statement.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let arxiv_id: Option<String> = row.get(1)?;
            let doi: Option<String> = row.get(2)?;
            let pdf_path: Option<String> = row.get(3)?;
            Ok((id, arxiv_id, doi, pdf_path))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (id, arxiv_id, doi, pdf_path) = row?;
            // 読む側は実ファイルパスへ解決する（段階 2b）。
            let pdf_path = self.resolve_opt(&pdf_path);
            if let Some(aid) = arxiv_id.filter(|s| !s.is_empty()) {
                let bare = aid.rsplit('/').next().unwrap_or(&aid);
                let bare = strip_arxiv_version(bare);
                result.push((id, Some(bare.to_owned()), doi, pdf_path));
            } else if let Some(path) = &pdf_path {
                if let Some(extracted) = extract_arxiv_id(path) {
                    result.push((id, Some(extracted), doi, pdf_path));
                } else {
                    result.push((id, None, doi, pdf_path));
                }
            } else {
                result.push((id, None, doi, pdf_path));
            }
        }
        Ok(result)
    }

    /// Update an existing paper row with metadata fetched from arXiv.
    ///
    /// # Errors
    /// Returns an error when the update or author insertion fails.
    pub fn apply_arxiv_metadata(
        &mut self,
        paper_id: i64,
        paper: &RemotePaper,
    ) -> Result<(), DatabaseError> {
        let transaction = self.connection.transaction()?;
        let existing_id: Option<i64> = transaction
            .query_row(
                "SELECT id FROM papers WHERE arxiv_id = ?1 LIMIT 1",
                params![paper.id],
                |row| row.get(0),
            )
            .optional()?;

        let target_id = if let Some(existing) = existing_id {
            if existing == paper_id {
                transaction.execute(
                    "UPDATE papers SET title = ?1,
                     abstract = CASE WHEN ?2 IS NULL OR trim(?2) = '' THEN abstract ELSE ?2 END,
                     arxiv_id = ?3,
                     doi = COALESCE(?4, doi), published_at = ?5, updated_at = ?6, journal = COALESCE(?7, journal)
                     WHERE id = ?8",
                    params![
                        paper.title, paper.abstract_text, paper.id, paper.doi,
                        paper.published.to_rfc3339(), paper.updated.to_rfc3339(), paper.journal_ref, paper_id
                    ],
                )?;
                paper_id
            } else {
                let (pdf_path, file_size): (Option<String>, Option<i64>) = transaction
                    .query_row(
                        "SELECT pdf_path, file_size FROM papers WHERE id = ?1",
                        params![paper_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?
                    .unwrap_or((None, None));

                transaction
                    .execute(
                        "DELETE FROM paper_authors WHERE paper_id = ?1",
                        params![paper_id],
                    )
                    .ok();
                transaction
                    .execute(
                        "DELETE FROM collection_papers WHERE paper_id = ?1",
                        params![paper_id],
                    )
                    .ok();
                transaction.execute("DELETE FROM papers WHERE id = ?1", params![paper_id])?;

                if let Some(path) = pdf_path {
                    transaction.execute(
                        "UPDATE papers SET pdf_path = ?1, file_size = COALESCE(?2, file_size),
                         title = ?3,
                         abstract = CASE WHEN ?4 IS NULL OR trim(?4) = '' THEN abstract ELSE ?4 END,
                         doi = COALESCE(?5, doi),
                         published_at = ?6, updated_at = ?7, journal = COALESCE(?8, journal)
                         WHERE id = ?9",
                        params![
                            path,
                            file_size,
                            paper.title,
                            paper.abstract_text,
                            paper.doi,
                            paper.published.to_rfc3339(),
                            paper.updated.to_rfc3339(),
                            paper.journal_ref,
                            existing
                        ],
                    )?;
                } else {
                    transaction.execute(
                        "UPDATE papers SET title = ?1,
                         abstract = CASE WHEN ?2 IS NULL OR trim(?2) = '' THEN abstract ELSE ?2 END,
                         doi = COALESCE(?3, doi), published_at = ?4, updated_at = ?5, journal = COALESCE(?6, journal)
                         WHERE id = ?7",
                        params![
                            paper.title, paper.abstract_text, paper.doi,
                            paper.published.to_rfc3339(), paper.updated.to_rfc3339(), paper.journal_ref, existing
                        ]
                    )?;
                }
                existing
            }
        } else {
            transaction.execute(
            "UPDATE papers SET title = ?1,
             abstract = CASE WHEN ?2 IS NULL OR trim(?2) = '' THEN abstract ELSE ?2 END,
             arxiv_id = ?3,
                 doi = COALESCE(?4, doi), published_at = ?5, updated_at = ?6, journal = COALESCE(?7, journal)
                 WHERE id = ?8",
                params![
                    paper.title, paper.abstract_text, paper.id, paper.doi,
                    paper.published.to_rfc3339(), paper.updated.to_rfc3339(), paper.journal_ref, paper_id
                ],
            )?;
            paper_id
        };
        transaction.execute(
            "UPDATE papers SET enrichment_status = 'success',
             last_enrichment_attempt = CURRENT_TIMESTAMP WHERE id = ?1",
            [target_id],
        )?;
        persist_authors_if_missing(&transaction, target_id, &paper.authors)?;
        transaction.commit()?;
        Ok(())
    }

    /// Store a journal discovered by a fallback provider without replacing a valid value.
    ///
    /// # Errors
    ///
    /// Returns an error when the journal metadata cannot be persisted.
    pub fn apply_journal_metadata(
        &self,
        paper_id: i64,
        journal: &str,
    ) -> Result<(), DatabaseError> {
        self.connection.execute(
            "UPDATE papers SET journal = CASE WHEN journal IS NULL OR trim(journal) = '' THEN ?1 ELSE journal END,
             enrichment_status = 'success', last_enrichment_attempt = CURRENT_TIMESTAMP WHERE id = ?2",
            params![journal.trim(), paper_id],
        )?;
        Ok(())
    }

    /// Record failed or unavailable metadata enrichment outcomes.
    ///
    /// # Errors
    /// Returns an error when database writes fail.
    pub fn update_enrichment_status(
        &self,
        paper_id: i64,
        status: &str,
    ) -> Result<(), DatabaseError> {
        if status == "failed" {
            self.connection.execute(
                "UPDATE papers SET
                    enrichment_status = 'failed',
                    enrichment_attempts = enrichment_attempts + 1,
                    last_enrichment_attempt = CURRENT_TIMESTAMP
                 WHERE id = ?1",
                [paper_id],
            )?;
        } else if status == "unavailable" {
            self.connection.execute(
                "UPDATE papers SET
                    enrichment_status = 'unavailable',
                    last_enrichment_attempt = CURRENT_TIMESTAMP
                 WHERE id = ?1",
                [paper_id],
            )?;
        }
        Ok(())
    }

    /// DOI が一致する論文の撤回の状態を記録する（メタデータの補完・`enrich --retraction` 用）。
    ///
    /// `is_retracted` は `None` = 未確認、`Some(false)` = 撤回されていない、
    /// `Some(true)` = 撤回。確かめた時刻も一緒に更新する。
    /// 更新した行数を返す（同じ DOI の論文が複数ある場合がある）。
    ///
    /// # Errors
    ///
    /// Returns an error when database writes fail.
    pub fn set_retraction_by_doi(
        &self,
        doi: &str,
        is_retracted: Option<bool>,
    ) -> Result<usize, DatabaseError> {
        let updated = self.connection.execute(
            "UPDATE papers SET is_retracted = ?1, retraction_checked_at = ?2 WHERE doi = ?3",
            params![is_retracted, Utc::now().to_rfc3339(), doi],
        )?;
        Ok(updated)
    }

    /// 論文 ID を指定して撤回の状態を記録する（`enrich --retraction` 用）。
    ///
    /// # Errors
    ///
    /// Returns an error when database writes fail.
    pub fn set_retraction(
        &self,
        paper_id: i64,
        is_retracted: Option<bool>,
    ) -> Result<(), DatabaseError> {
        self.connection.execute(
            "UPDATE papers SET is_retracted = ?1, retraction_checked_at = ?2 WHERE id = ?3",
            params![is_retracted, Utc::now().to_rfc3339(), paper_id],
        )?;
        Ok(())
    }

    /// 撤回を確かめる対象（DOI のある論文）の `(papers.id, doi)` を返す。
    ///
    /// `force` が false なら `retraction_checked_at` が NULL の論文だけを返す。
    /// 並び順は `id`（問い合わせの順を再現できるようにする）。
    ///
    /// # Errors
    ///
    /// Returns an error when the query fails.
    pub fn papers_for_retraction_check(
        &self,
        force: bool,
    ) -> Result<Vec<(i64, String)>, DatabaseError> {
        let mut statement = self.connection.prepare(
            "SELECT id, doi FROM papers
             WHERE doi IS NOT NULL AND trim(doi) <> ''
               AND (?1 OR retraction_checked_at IS NULL)
             ORDER BY id",
        )?;
        let rows = statement.query_map(params![force], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

/// Strip a trailing version suffix like `v1`, `v2`, etc. from an arXiv ID.
fn strip_arxiv_version(id: &str) -> &str {
    if let Some(pos) = id.rfind('v')
        && id[pos + 1..].chars().all(|c| c.is_ascii_digit())
        && pos + 1 < id.len()
    {
        return &id[..pos];
    }
    id
}

/// Try to extract an arXiv ID from a file path.
fn extract_arxiv_id(path: &str) -> Option<String> {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let stem = filename.strip_suffix(".pdf").unwrap_or(filename);
    let bytes = stem.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i + 9 <= len {
        if bytes[i].is_ascii_digit()
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].is_ascii_digit()
            && bytes[i + 4] == b'.'
        {
            let mut j = i + 5;
            while j < len && bytes[j].is_ascii_digit() {
                j += 1;
            }
            let digit_count = j - (i + 5);
            if (4..=5).contains(&digit_count) {
                let candidate = &stem[i..j];
                return Some(candidate.to_owned());
            }
        }
        i += 1;
    }
    None
}

/// Return the venue portion of a journal reference without changing stored metadata.
///
/// arXiv commonly provides references such as `Phys. Rev. D 104, 043008 (2021)`.
/// The first volume-like numeric token marks the end of the journal name; entries
/// that are already venue names are returned unchanged.
#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use chrono::{TimeZone, Utc};
    use rusqlite::params;

    use super::{Database, PAPER_KEY_TABLES};
    use crate::{
        library::{ImportedPdf, LibraryIndexer},
        models::RemotePaper,
        paths::paper_folder,
    };

    #[test]
    fn migrations_create_queryable_schema() -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::in_memory()?;
        database.insert_paper("A useful paper", Some("10.1000/test"))?;
        let count: i64 = database
            .connection()
            .query_row("SELECT COUNT(*) FROM papers", [], |row| row.get(0))?;
        assert_eq!(count, 1);
        Ok(())
    }

    /// 索引（chunks + chunk_embeddings）と引用・図・翻訳の行を持つ論文を
    /// `delete_paper` で削除でき、関連行が残らないことを確かめる（段階 1）。
    /// 検索履歴から取り込んだ論文（`search_history_items`）でも削除が通ること、
    /// `paper_id` が NULL になることも確かめる。
    #[test]
    fn delete_paper_removes_index_and_key_linked_rows(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::in_memory()?;
        let paper_id = database.insert_paper("Indexed paper", Some("10.1000/indexed"))?;
        // 抽出キー（S2 40hex）を papers 行に持たせる。
        let key = "ff312b6027d012403760c288fd6531145006cb9f";
        let conn = database.connection();
        conn.execute(
            "UPDATE papers SET s2_paper_id = ?1 WHERE id = ?2",
            params![key, paper_id],
        )?;

        // 索引: chunks + FTS + chunk_embeddings。
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, strategy, created_at)
             VALUES ('abstract', 'abstract', ?1, 0, 'deletable chunk text', 'paragraph', '2026-09-24')",
            [paper_id],
        )?;
        let chunk_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO embedding_models (name, endpoint, dim, created_at)
             VALUES ('test-model', 'https://example.invalid', 4, '2026-09-24')",
            [],
        )?;
        conn.execute(
            "INSERT INTO chunk_embeddings (chunk_id, model_id, vec) VALUES (?1, 1, x'0001')",
            [chunk_id],
        )?;
        // 削除前に FTS から検索できることを確かめる（削除後の検証の前提）。
        let fts_hits: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunk_fts WHERE chunk_fts MATCH 'deletable'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(fts_hits, 1);

        // 抽出キーで結び付く表（引用・翻訳）。
        conn.execute(
            "INSERT INTO paper_refs (paper_id, ref_no, raw_text) VALUES (?1, 1, '引用文献')",
            [key],
        )?;
        conn.execute(
            "INSERT INTO paper_sentences (paper_id, sent_no, text, cite_refs)
             VALUES (?1, 1, '引用文', '[1]')",
            [key],
        )?;
        conn.execute(
            "INSERT INTO paper_translations (paper_id, status) VALUES (?1, 'ready')",
            [key],
        )?;
        // 抽出・図: キーと papers_id の両方を持つ。
        conn.execute(
            "INSERT INTO paper_extracts (paper_id, status, papers_id) VALUES (?1, 'ready', ?2)",
            params![key, paper_id],
        )?;
        conn.execute(
            "INSERT INTO paper_figures (paper_id, papers_id, fig_no, image_path, created_at)
             VALUES (?1, ?2, 1, '/tmp/fig-001.png', '2026-09-24')",
            params![key, paper_id],
        )?;
        conn.execute(
            "INSERT INTO paper_figure_extracts (paper_id, papers_id, status)
             VALUES (?1, ?2, 'ready')",
            params![key, paper_id],
        )?;
        // 抽出キーが変わった後の記録（papers_id のみで結び付く行）。
        // 抽出キーでの削除は届かないため、0033 の CASCADE で消えることを確かめる。
        let changed_key = "0000000000000000000000000000000000000001";
        conn.execute(
            "INSERT INTO paper_refs (paper_id, ref_no, raw_text, papers_id)
             VALUES (?1, 1, '旧キーの引用', ?2)",
            params![changed_key, paper_id],
        )?;
        conn.execute(
            "INSERT INTO paper_sentences (paper_id, sent_no, text, cite_refs, papers_id)
             VALUES (?1, 1, '旧キーの引用文', '[1]', ?2)",
            params![changed_key, paper_id],
        )?;
        conn.execute(
            "INSERT INTO paper_translations (paper_id, status, papers_id)
             VALUES (?1, 'ready', ?2)",
            params![changed_key, paper_id],
        )?;
        conn.execute(
            "INSERT INTO paper_extracts (paper_id, status, papers_id)
             VALUES (?1, 'ready', ?2)",
            params![changed_key, paper_id],
        )?;
        conn.execute(
            "INSERT INTO paper_figures (paper_id, papers_id, fig_no, image_path, created_at)
             VALUES (?1, ?2, 2, '/tmp/fig-002.png', '2026-09-24')",
            params![changed_key, paper_id],
        )?;
        conn.execute(
            "INSERT INTO paper_figure_extracts (paper_id, papers_id, status)
             VALUES (?1, ?2, 'ready')",
            params![changed_key, paper_id],
        )?;
        // 音声化: papers.id（整数）で結び付く。
        conn.execute(
            "INSERT INTO paper_podcasts (paper_id, status) VALUES (?1, 'ready')",
            [paper_id],
        )?;
        // 検索履歴: 候補の行は残したまま取り込んだ論文を消すので、
        // papers を NO ACTION で参照する search_history_items が削除の足枷になる。
        conn.execute(
            "INSERT INTO search_history (name, source_dbs, query) VALUES ('履歴', 's2', 'test query')",
            [],
        )?;
        let set_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO search_history_items
                 (history_id, position, remote_json, dedupe_key, paper_id)
             VALUES (?1, 0, '{}', 'dedupe-1', ?2)",
            params![set_id, paper_id],
        )?;
        // 同じセット内に、まだ DL していない候補も混ぜておく。
        conn.execute(
            "INSERT INTO search_history_items
                 (history_id, position, remote_json, dedupe_key, paper_id)
             VALUES (?1, 1, '{}', 'dedupe-2', NULL)",
            [set_id],
        )?;

        database.delete_paper(paper_id)?;

        // 関連行が残っていないこと。
        let count = |sql: &str| -> Result<i64, rusqlite::Error> {
            conn.query_row(sql, [], |row| row.get(0))
        };
        let leftovers: Vec<(&str, i64)> = vec![
            ("papers", count(&format!("SELECT COUNT(*) FROM papers WHERE id = {paper_id}"))?),
            ("chunks", count("SELECT COUNT(*) FROM chunks")?),
            ("chunk_embeddings", count("SELECT COUNT(*) FROM chunk_embeddings")?),
            ("paper_refs", count("SELECT COUNT(*) FROM paper_refs")?),
            ("paper_sentences", count("SELECT COUNT(*) FROM paper_sentences")?),
            ("paper_translations", count("SELECT COUNT(*) FROM paper_translations")?),
            ("paper_extracts", count("SELECT COUNT(*) FROM paper_extracts")?),
            ("paper_figures", count("SELECT COUNT(*) FROM paper_figures")?),
            ("paper_figure_extracts", count("SELECT COUNT(*) FROM paper_figure_extracts")?),
            ("paper_podcasts", count("SELECT COUNT(*) FROM paper_podcasts")?),
        ];
        for (table, n) in leftovers {
            assert_eq!(n, 0, "{table} に残りの行がある");
        }
        // FTS の索引からも消えていること。
        let fts_hits: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunk_fts WHERE chunk_fts MATCH 'deletable'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(fts_hits, 0, "chunk_fts に削除済みチャンクが残っている");

        // 検索履歴と候補の行は残るが、削除した論文への結び付きは外されている。
        let sets_left: i64 = count("SELECT COUNT(*) FROM search_history")?;
        assert_eq!(sets_left, 1, "search_history の親行が消えている");
        let items_left: i64 = count("SELECT COUNT(*) FROM search_history_items")?;
        assert_eq!(items_left, 2, "search_history_items の候補行が消えている");
        let linked_paper: Option<i64> = conn.query_row(
            "SELECT paper_id FROM search_history_items WHERE dedupe_key = 'dedupe-1'",
            [],
            |row| row.get(0),
        )?;
        assert!(
            linked_paper.is_none(),
            "削除済み論文を search_history_items がまだ参照している"
        );
        Ok(())
    }

    /// 既存行へ Library の番号を埋める（段階 2c）。S2 / arXiv / DOI の 3 通りが
    /// 埋まり、対応する論文が無い行（Library 外の抽出）は NULL のまま残ること。
    #[test]
    fn backfill_papers_id_fills_s2_arxiv_doi_and_keeps_unknown_null()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::in_memory()?;
        let conn = database.connection();
        // Library 論文 3 本。ID 列の持ち方は 3 通り（S2 は大文字で保存されている例）。
        conn.execute(
            "INSERT INTO papers (id, title, s2_paper_id)
             VALUES (1, 'S2 paper', 'FF312B6027D012403760C288FD6531145006CB9F')",
            [],
        )?;
        conn.execute(
            "INSERT INTO papers (id, title, arxiv_id) VALUES (2, 'arXiv paper', '1706.03762')",
            [],
        )?;
        conn.execute(
            "INSERT INTO papers (id, title, doi) VALUES (3, 'DOI paper', '10.1038/nature12373')",
            [],
        )?;

        // 6 表すべてに対応する行と、対応が無い Library 外の行を書き込む。
        const KEYS: &[&str] = &[
            "ff312b6027d012403760c288fd6531145006cb9f",
            "ArXiv:1706.03762",
            "DOI:10.1038/nature12373",
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        ];
        for table in PAPER_KEY_TABLES {
            for key in KEYS {
                let sql = match *table {
                    "paper_refs" => format!(
                        "INSERT INTO paper_refs (paper_id, ref_no, raw_text) VALUES ('{key}', 1, '引用')"
                    ),
                    "paper_sentences" => format!(
                        "INSERT INTO paper_sentences (paper_id, sent_no, text, cite_refs)
                         VALUES ('{key}', 1, '引用文', '[1]')"
                    ),
                    "paper_extracts" => format!(
                        "INSERT INTO paper_extracts (paper_id, status) VALUES ('{key}', 'ready')"
                    ),
                    "paper_figures" => format!(
                        "INSERT INTO paper_figures (paper_id, fig_no, image_path, created_at)
                         VALUES ('{key}', 1, '/tmp/fig.png', '2026-09-24')"
                    ),
                    "paper_figure_extracts" => format!(
                        "INSERT INTO paper_figure_extracts (paper_id, status) VALUES ('{key}', 'ready')"
                    ),
                    "paper_translations" => format!(
                        "INSERT INTO paper_translations (paper_id, status) VALUES ('{key}', 'ready')"
                    ),
                    other => panic!("未対応の表: {other}"),
                };
                conn.execute(&sql, [])?;
            }
        }

        let changed = database.backfill_papers_id()?;
        assert_eq!(
            changed,
            PAPER_KEY_TABLES.len() * KEYS.len().saturating_sub(1),
            "3 通り × 6 表が埋まる（Library 外の 1 行は対象外）"
        );

        let count = |table: &str, cond: &str| -> Result<i64, rusqlite::Error> {
            conn.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {cond}"),
                [],
                |row| row.get(0),
            )
        };
        for table in PAPER_KEY_TABLES {
            assert_eq!(count(table, "papers_id = 1")?, 1, "{table}: S2 の行");
            assert_eq!(count(table, "papers_id = 2")?, 1, "{table}: arXiv の行");
            assert_eq!(count(table, "papers_id = 3")?, 1, "{table}: DOI の行");
            assert_eq!(
                count(table, "papers_id IS NULL")?,
                1,
                "{table}: Library 外の行は NULL のまま"
            );
        }
        // 印が app_meta に付くこと。
        let marker: String = conn.query_row(
            "SELECT value FROM app_meta WHERE key = 'papers_id_backfill_v1'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(marker, "1");
        Ok(())
    }

    /// `Database::open` のときに 1 回だけ埋め込むこと（印は `app_meta`）。
    #[test]
    fn open_backfills_papers_id_only_once() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!(
            "sai_paper_backfill_once_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir)?;
        let db_path = dir.join("papr.db");

        // 1 回目: 表は空だが、印は付く。
        let database = Database::open(&db_path)?;
        let marker: String = database.connection().query_row(
            "SELECT value FROM app_meta WHERE key = 'papers_id_backfill_v1'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(marker, "1");
        database.connection().execute(
            "INSERT INTO papers (id, title, arxiv_id) VALUES (1, 'paper', '2401.00001')",
            [],
        )?;
        database.connection().execute(
            "INSERT INTO paper_refs (paper_id, ref_no, raw_text)
             VALUES ('ArXiv:2401.00001', 1, '引用')",
            [],
        )?;
        drop(database);

        // 印があるうちは、開き直しても 2 回目は走らない。
        let database = Database::open(&db_path)?;
        let papers_id: Option<i64> =
            database
                .connection()
                .query_row("SELECT papers_id FROM paper_refs", [], |row| row.get(0))?;
        assert_eq!(papers_id, None, "印があるのに 2 回目の埋め込みが走った");
        drop(database);

        // 印を消すと、次に開いたときに埋め込まれる。
        {
            let raw = rusqlite::Connection::open(&db_path)?;
            raw.execute(
                "DELETE FROM app_meta WHERE key = 'papers_id_backfill_v1'",
                [],
            )?;
        }
        let database = Database::open(&db_path)?;
        let papers_id: i64 =
            database
                .connection()
                .query_row("SELECT papers_id FROM paper_refs", [], |row| row.get(0))?;
        assert_eq!(papers_id, 1, "印を消して開き直しても埋まらない");
        drop(database);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn doi_is_unique_when_present() -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::in_memory()?;
        database.insert_paper("First", Some("10.1000/same"))?;
        assert!(
            database
                .insert_paper("Second", Some("10.1000/same"))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn remote_only_papers_do_not_appear_in_library() -> Result<(), Box<dyn std::error::Error>> {
        let mut database = Database::in_memory()?;
        let timestamp = Utc
            .with_ymd_and_hms(2026, 2, 1, 0, 0, 0)
            .single()
            .ok_or("invalid test timestamp")?;
        let paper = RemotePaper {
            id: "https://arxiv.org/abs/2602.00002".into(),
            title: "Search result only".into(),
            authors: vec!["Remote Author".into()],
            abstract_text: "Metadata without a downloaded PDF.".into(),
            published: timestamp,
            updated: timestamp,
            categories: vec!["cs.DL".into()],
            pdf_url: Some("https://arxiv.org/pdf/2602.00002".into()),
            doi: Some("10.1000/remote-only".into()),
            journal_ref: None,
            citation_count: None,
        };

        database.ensure_remote_paper(&paper)?;

        assert!(database.library_papers()?.is_empty());
        Ok(())
    }

    #[test]
    fn enrichment_preserves_an_existing_abstract_when_the_provider_has_none()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut database = Database::in_memory()?;
        let timestamp = Utc
            .with_ymd_and_hms(2026, 2, 1, 0, 0, 0)
            .single()
            .ok_or("invalid test timestamp")?;
        let mut paper = RemotePaper {
            id: "https://arxiv.org/abs/2602.00003".into(),
            title: "Original title".into(),
            authors: vec!["Researcher".into()],
            abstract_text: "Abstract from arXiv.".into(),
            published: timestamp,
            updated: timestamp,
            categories: vec![],
            pdf_url: None,
            doi: Some("10.1000/abstract-merge".into()),
            journal_ref: None,
            citation_count: None,
        };
        let paper_id = database.ensure_remote_paper(&paper)?;

        paper.abstract_text = "   ".into();
        database.apply_arxiv_metadata(paper_id, &paper)?;
        let abstract_text: String = database.connection.query_row(
            "SELECT abstract FROM papers WHERE id = ?1",
            [paper_id],
            |row| row.get(0),
        )?;
        assert_eq!(abstract_text, "Abstract from arXiv.");

        paper.abstract_text = "A richer provider abstract.".into();
        database.apply_arxiv_metadata(paper_id, &paper)?;
        let abstract_text: String = database.connection.query_row(
            "SELECT abstract FROM papers WHERE id = ?1",
            [paper_id],
            |row| row.get(0),
        )?;
        assert_eq!(abstract_text, "A richer provider abstract.");
        Ok(())
    }

    #[test]
    fn valid_abstract_survives_download_enrichment_and_subsequent_sync_updates()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut database = Database::in_memory()?;
        let timestamp = Utc
            .with_ymd_and_hms(2026, 2, 1, 0, 0, 0)
            .single()
            .ok_or("invalid test timestamp")?;
        let pdf = imported_pdf("abstract-lifecycle.pdf");
        let mut paper = RemotePaper {
            id: "https://arxiv.org/abs/2602.00004".into(),
            title: "Original title".into(),
            authors: vec!["Researcher".into()],
            abstract_text: "Abstract fetched from arXiv.".into(),
            published: timestamp,
            updated: timestamp,
            categories: vec![],
            pdf_url: None,
            doi: Some("10.1000/abstract-lifecycle".into()),
            journal_ref: None,
            citation_count: None,
        };

        let paper_id = database.attach_download(&paper, &pdf, std::path::Path::new("."))?;
        paper.abstract_text = "   ".into();

        // A provider without an abstract must not erase the downloaded arXiv value.
        database.apply_arxiv_metadata(paper_id, &paper)?;
        // A later download attachment and opening a cached remote record are both
        // metadata writes that can follow synchronization.
        database.attach_download(&paper, &pdf, std::path::Path::new("."))?;
        database.ensure_remote_paper(&paper)?;
        database.import_pdf(&pdf)?;

        let abstract_text: String = database.connection.query_row(
            "SELECT abstract FROM papers WHERE id = ?1",
            [paper_id],
            |row| row.get(0),
        )?;
        assert_eq!(abstract_text, "Abstract fetched from arXiv.");
        Ok(())
    }

    #[test]
    fn root_filtered_library_only_includes_existing_pdfs_inside_roots()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::in_memory()?;
        let root = std::env::temp_dir().join(format!("papr-root-filter-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let inside_path = root.join("inside.pdf");
        fs::write(&inside_path, b"%PDF-1.7\ninside")?;
        let outside_path = root.with_extension("outside.pdf");
        fs::write(&outside_path, b"%PDF-1.7\noutside")?;
        let inside_path_string = inside_path.to_string_lossy().into_owned();
        let outside_path_string = outside_path.to_string_lossy().into_owned();

        database.import_pdf(&imported_pdf(&inside_path_string))?;
        database.import_pdf(&imported_pdf(&outside_path_string))?;

        let rows = database.library_papers_in_roots(std::slice::from_ref(&root))?;

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].pdf_path.as_deref(),
            Some(inside_path_string.as_str())
        );
        let _ = fs::remove_file(outside_path);
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    /// 回帰テスト: `attach_download` は論文フォルダを `data_dir`（`.sai/paper/`）
    /// から `paths::paper_folder` の規則で決め、**絶対パス**として DB に記録し、
    /// PDF を実際にそこへ移動する（段階 2a）。
    ///
    /// 置き場は `library_root` にも設定の `download_path` にも依存しない。
    #[test]
    fn attach_download_stores_absolute_path_under_data_dir_papers()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut database = Database::in_memory()?;
        let root = std::env::temp_dir().join(format!(
            "papr-attach-root-{}-{}",
            std::process::id(),
            line!()
        ));
        fs::create_dir_all(&root)?;
        let downloads = root.join("incoming");
        fs::create_dir_all(&downloads)?;
        let source = downloads.join("downloaded.pdf");
        fs::write(&source, b"%PDF-1.7\ndownloaded")?;

        let pdf = imported_pdf(&source.to_string_lossy());

        let timestamp = Utc
            .with_ymd_and_hms(2026, 2, 1, 0, 0, 0)
            .single()
            .ok_or("invalid test timestamp")?;
        let paper = RemotePaper {
            id: "ArXiv:2602.99999".into(),
            title: "Rooted download".into(),
            authors: vec!["Researcher".into()],
            abstract_text: String::new(),
            published: timestamp,
            updated: timestamp,
            categories: vec![],
            pdf_url: None,
            doi: None,
            journal_ref: None,
            citation_count: None,
        };

        let paper_id = database.attach_download(&paper, &pdf, &root)?;
        let stored = database
            .library_paper_by_id(paper_id)?
            .and_then(|p| p.pdf_path)
            .expect("pdf_path must be stored");
        let stored_path = PathBuf::from(&stored);
        assert!(stored_path.is_absolute(), "絶対パスであること: {stored}");
        assert!(
            stored_path.starts_with(root.join("papers")),
            "<data_dir>/papers/ 配下であること: {stored}"
        );
        assert_eq!(
            stored_path.file_name().unwrap().to_string_lossy(),
            "paper.pdf"
        );
        assert!(stored_path.is_file(), "PDF が移動されていること: {stored}");

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn download_enriches_matching_local_pdf() -> Result<(), Box<dyn std::error::Error>> {
        let mut database = Database::in_memory()?;
        let pdf = imported_pdf("paper.pdf");
        database.import_pdf(&pdf)?;
        let timestamp = Utc
            .with_ymd_and_hms(2026, 2, 1, 0, 0, 0)
            .single()
            .ok_or("invalid test timestamp")?;
        let paper = RemotePaper {
            id: "https://arxiv.org/abs/2602.00001".into(),
            title: "Canonical title".into(),
            authors: vec!["Researcher One".into()],
            abstract_text: "Indexed abstract".into(),
            published: timestamp,
            updated: timestamp,
            categories: vec!["cs.DL".into()],
            pdf_url: Some("https://arxiv.org/pdf/2602.00001".into()),
            doi: Some("10.1000/download".into()),
            journal_ref: None,
            citation_count: None,
        };
        database.attach_download(&paper, &pdf, std::path::Path::new("."))?;
        let rows = database.library_papers()?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Canonical title");
        assert_eq!(rows[0].authors, "Researcher One");
        Ok(())
    }

    #[test]
    fn author_order_survives_download_and_metadata_updates()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut database = Database::in_memory()?;
        let timestamp = Utc
            .with_ymd_and_hms(2026, 2, 1, 0, 0, 0)
            .single()
            .ok_or("invalid test timestamp")?;

        // Seed the author table in a different order from the target paper so
        // this verifies `position`, rather than incidental author row order.
        let seed = RemotePaper {
            id: "https://arxiv.org/abs/2602.01000".into(),
            title: "Seed paper".into(),
            authors: vec!["Alpha Author".into(), "Zulu Author".into()],
            abstract_text: String::new(),
            published: timestamp,
            updated: timestamp,
            categories: vec![],
            pdf_url: None,
            doi: None,
            journal_ref: None,
            citation_count: None,
        };
        database.ensure_remote_paper(&seed)?;

        let mut paper = RemotePaper {
            id: "https://arxiv.org/abs/2602.01001".into(),
            title: "Ordered paper".into(),
            authors: vec!["Zulu Author".into(), "Alpha Author".into()],
            abstract_text: "Canonical ordering test.".into(),
            published: timestamp,
            updated: timestamp,
            categories: vec![],
            pdf_url: None,
            doi: None,
            journal_ref: None,
            citation_count: None,
        };
        let paper_id = database.attach_download(
            &paper,
            &imported_pdf("author-order.pdf"),
            std::path::Path::new("."),
        )?;

        let library_paper = database
            .library_paper_by_id(paper_id)?
            .ok_or("missing library paper")?;
        assert_eq!(library_paper.authors, "Zulu Author, Alpha Author");

        // Once persisted, metadata refreshes must retain the stored sequence.
        paper.authors.reverse();
        database.apply_arxiv_metadata(paper_id, &paper)?;
        let library_paper = database
            .library_paper_by_id(paper_id)?
            .ok_or("missing refreshed library paper")?;
        assert_eq!(library_paper.authors, "Zulu Author, Alpha Author");
        Ok(())
    }

    #[test]
    fn notes_stay_synchronized_with_filesystem_and_autodelete()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "papr-notes-sync-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&root)?;
        let mut database = Database::in_memory_with_data_dir(&root)?;
        let pdf_path = root.join("test_noted_paper.pdf");
        fs::write(&pdf_path, "%PDF-1.4 test")?;

        let timestamp = Utc::now();
        let paper = RemotePaper {
            id: "https://arxiv.org/abs/2602.00005".into(),
            title: "Test Noted Paper".into(),
            authors: vec!["Bob Specialist".into()],
            abstract_text: "Testing notes list synchronization.".into(),
            published: timestamp,
            updated: timestamp,
            categories: vec!["cs.SE".into()],
            pdf_url: None,
            doi: None,
            journal_ref: None,
            citation_count: None,
        };

        let pdf = ImportedPdf {
            path: pdf_path.clone(),
            title: "Test Noted Paper".into(),
            content_hash: "hash".into(),
            file_size: 14,
            library_root: Some(root.clone()),
            relative_directory: None,
        };

        let paper_id = database.attach_download(&paper, &pdf, &root)?;

        // attach_download で PDF が論文フォルダに移動されるため、DB から新しいパスを取得
        let lib_paper = database
            .library_paper_by_id(paper_id)?
            .ok_or("paper not found")?;
        let moved_pdf_path = lib_paper.pdf_path.as_deref().ok_or("pdf_path is null")?;
        let moved_pdf_full_path = std::path::PathBuf::from(moved_pdf_path);

        // ノートは論文フォルダの note.md
        let note_path = crate::paths::paper_folder(&root, paper_id, "Test Noted Paper")
            .join(crate::paths::paper_file_names::NOTE);
        fs::create_dir_all(note_path.parent().ok_or("no parent")?)?;
        fs::write(&note_path, "This is a great paper.")?;

        // Case 1: Valid roots, file exists -> Paper should be returned in papers_with_notes
        let papers = database.papers_with_notes(std::slice::from_ref(&root))?;
        assert_eq!(papers.len(), 1);
        assert_eq!(papers[0].title, "Test Noted Paper");

        // Case 2: 空のノートは一覧に出ない
        fs::write(&note_path, "   \n  ")?;
        let papers = database.papers_with_notes(std::slice::from_ref(&root))?;
        assert!(papers.is_empty());
        fs::write(&note_path, "This is a great paper.")?;

        // Case 3: File is missing -> Paper should NOT be returned
        fs::remove_file(&moved_pdf_full_path)?;
        let papers = database.papers_with_notes(std::slice::from_ref(&root))?;
        assert!(papers.is_empty());

        // Case 4: File re-created but wrong/different roots -> Paper should NOT be returned
        fs::write(&moved_pdf_full_path, "%PDF-1.4 test")?;
        let different_root = root.join("other_dir");
        let papers = database.papers_with_notes(std::slice::from_ref(&different_root))?;
        assert!(papers.is_empty());

        let _ = fs::remove_dir_all(&root);
        Ok(())
    }

    fn imported_pdf(path: &str) -> ImportedPdf {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(path.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        ImportedPdf {
            path: PathBuf::from(path),
            title: "Imported title".into(),
            content_hash: hash,
            file_size: 42,
            library_root: None,
            relative_directory: None,
        }
    }

    #[test]
    fn metadata_enrichment_tracking() -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::in_memory()?;
        let id1 = database.insert_paper("Paper 1", None)?;
        database.connection.execute(
            "UPDATE papers SET pdf_path = 'paper1.pdf' WHERE id = ?1",
            [id1],
        )?;
        let needing = database.papers_needing_enrichment()?;
        assert_eq!(needing.len(), 1);
        assert_eq!(needing[0].0, id1);

        database.update_enrichment_status(id1, "failed")?;
        let needing = database.papers_needing_enrichment()?;
        assert_eq!(needing.len(), 0);

        database.connection.execute(
            "UPDATE papers SET last_enrichment_attempt = datetime('now', '-10 minutes') WHERE id = ?1",
            [id1],
        )?;
        let needing = database.papers_needing_enrichment()?;
        assert_eq!(needing.len(), 1);

        database.update_enrichment_status(id1, "unavailable")?;
        let needing = database.papers_needing_enrichment()?;
        assert_eq!(needing.len(), 0);
        database.connection.execute(
            "UPDATE papers SET enrichment_status = 'success', journal = NULL,
             last_enrichment_attempt = CURRENT_TIMESTAMP WHERE id = ?1",
            [id1],
        )?;
        assert_eq!(database.papers_needing_enrichment()?.len(), 0);
        database.connection.execute(
            "UPDATE papers SET enrichment_status = 'success', journal = NULL,
             last_enrichment_attempt = datetime('now', '-25 hours') WHERE id = ?1",
            [id1],
        )?;
        assert_eq!(database.papers_needing_enrichment()?.len(), 1);
        database.apply_journal_metadata(id1, "Astronomy and Computing")?;
        assert_eq!(database.papers_needing_enrichment()?.len(), 0);
        database.apply_journal_metadata(id1, "Lower Quality Name")?;
        let journal: String = database.connection.query_row(
            "SELECT journal FROM papers WHERE id = ?1",
            [id1],
            |row| row.get(0),
        )?;
        assert_eq!(journal, "Astronomy and Computing");
        Ok(())
    }

    #[test]
    fn activity_logging_distinction() -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::in_memory()?;
        let id = database.insert_paper("Test Paper", None)?;
        database.record_activity("paper_browsed", Some(id), None)?;
        let sessions: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM reading_history", [], |row| row.get(0))?;
        assert_eq!(sessions, 0);

        let session_id = database.record_open(id, true)?;
        database.record_reading_duration(session_id, 30)?;
        let (sessions, average): (i64, i64) = database.connection.query_row(
            "SELECT COUNT(*), CAST(COALESCE(AVG(duration_s), 0) AS INTEGER)
               FROM reading_history WHERE duration_s > 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(sessions, 1);
        assert_eq!(average, 30);
        Ok(())
    }

    #[test]
    fn rename_pdf_updates_path_and_preserves_metadata() -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::in_memory()?;
        let root = std::env::temp_dir().join(format!(
            "papr-rename-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&root)?;
        let old_path = root.join("old_paper_name.pdf");
        fs::write(&old_path, "%PDF-1.4 old")?;

        let mut pdf = imported_pdf(&old_path.to_string_lossy());
        pdf.title = "Old Paper Name".into();
        database.import_pdf(&pdf)?;

        let papers = database.library_papers()?;
        let paper_id = papers[0].id;
        assert_eq!(papers[0].title, "Old Paper Name");

        let new_path = root.join("new_paper_name.pdf");
        fs::rename(&old_path, &new_path)?;
        database.rename_pdf(paper_id, &new_path)?;

        let papers = database.library_papers()?;
        assert_eq!(papers.len(), 1);
        assert_eq!(papers[0].title, "Old Paper Name");
        let new_path = new_path
            .to_str()
            .ok_or("temporary path is not valid UTF-8")?;
        assert_eq!(papers[0].pdf_path.as_deref(), Some(new_path));

        fs::remove_file(new_path)?;
        fs::remove_dir(&root)?;
        Ok(())
    }

    #[test]
    fn scan_after_external_rename_preserves_a_local_paper_title()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::in_memory()?;
        let root = std::env::temp_dir().join(format!(
            "papr-external-rename-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&root)?;
        let old_path = root.join("old_name.pdf");
        fs::write(&old_path, "%PDF-1.4 old")?;
        let old_pdf = LibraryIndexer::inspect(&old_path)?;
        database.import_pdf(&old_pdf)?;
        let paper_id = database.library_papers()?[0].id;

        let new_path = root.join("new_name.pdf");
        fs::rename(&old_path, &new_path)?;
        let renamed_pdf = LibraryIndexer::inspect(&new_path)?;
        assert!(database.import_pdf(&renamed_pdf)?);

        let papers = database.library_papers()?;
        assert_eq!(papers.len(), 1);
        assert_eq!(papers[0].id, paper_id);
        assert_eq!(papers[0].title, "old name");
        // macOS では canonicalize が /var -> /private/var を解決するため比較も正規化する
        let stored = papers[0].pdf_path.as_deref().map(std::path::PathBuf::from);
        let stored = stored.as_ref().and_then(|p| std::fs::canonicalize(p).ok());
        let expected = std::fs::canonicalize(&new_path).ok();
        assert_eq!(stored, expected);

        fs::remove_file(&new_path)?;
        fs::remove_dir(&root)?;
        Ok(())
    }

    /// DL した PDF は `<data_dir>/papers/NNNN_短いタイトル/paper.pdf` に保存され、
    /// DB にもその絶対パスが記録される（段階 2a）。
    #[test]
    fn test_pdf_download_saves_to_paper_folder() -> Result<(), Box<dyn std::error::Error>> {
        let mut database = Database::in_memory()?;
        let data_dir = std::env::temp_dir().join(format!(
            "papr-download-folder-{}-{}",
            std::process::id(),
            line!()
        ));
        fs::create_dir_all(&data_dir)?;
        let source = data_dir.join("incoming.pdf");
        fs::write(&source, b"%PDF-1.7 test")?;
        let pdf = imported_pdf(&source.to_string_lossy());
        database.import_pdf(&pdf)?;
        let timestamp = Utc::now();
        let paper = RemotePaper {
            id: "https://arxiv.org/abs/2602.00010".into(),
            title: "Test PDF Download".into(),
            authors: vec!["Author".into()],
            abstract_text: "Abstract".into(),
            published: timestamp,
            updated: timestamp,
            categories: vec![],
            pdf_url: None,
            doi: None,
            journal_ref: None,
            citation_count: None,
        };
        let paper_id = database.attach_download(&paper, &pdf, &data_dir)?;
        let lib_paper = database
            .library_paper_by_id(paper_id)?
            .ok_or("paper not found")?;
        let expected_pdf = paper_folder(&data_dir, paper_id, "Test PDF Download").join("paper.pdf");
        let stored = PathBuf::from(lib_paper.pdf_path.unwrap_or_default());
        assert_eq!(stored, expected_pdf, "論文フォルダに記録すること");
        assert!(
            expected_pdf.is_file(),
            "PDF が論文フォルダに移動していること: {}",
            expected_pdf.display()
        );
        assert!(stored.is_absolute());
        let _ = fs::remove_dir_all(&data_dir);
        Ok(())
    }

    /// notes 表の中身は migration 0037 の前に note.md へ移り、失われない。
    #[test]
    fn notes_table_moves_to_note_files() -> Result<(), Box<dyn std::error::Error>> {
        let data_dir = std::env::temp_dir().join(format!(
            "papr-notes-move-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&data_dir)?;
        let db_path = data_dir.join("papr.db");
        {
            // 0036 までの DB（notes 表がある）
            let conn = rusqlite::Connection::open(&db_path)?;
            conn.execute_batch("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT);")?;
            for (version, sql) in super::MIGRATIONS.iter().filter(|(version, _)| *version < 37) {
                conn.execute_batch(sql)?;
                conn.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [version])?;
            }
            conn.execute_batch(
                "INSERT INTO papers (id, title) VALUES (1, 'One'), (2, 'Two'), (3, 'Three');
                 INSERT INTO notes (paper_id, body) VALUES (1, 'db only'), (2, 'db extra'), (3, 'same');",
            )?;
        }
        let folder = |id: i64, title: &str| crate::paths::paper_folder(&data_dir, id, title);
        for (id, title, text) in [(2, "Two", "file text"), (3, "Three", "same and more")] {
            fs::create_dir_all(folder(id, title))?;
            fs::write(folder(id, title).join("note.md"), text)?;
        }

        let database = Database::open(&db_path)?;

        let read = |id: i64, title: &str| fs::read_to_string(folder(id, title).join("note.md"));
        assert_eq!(read(1, "One")?, "db only\n", "note.md が無ければ書く");
        let two = read(2, "Two")?;
        assert!(two.starts_with("file text") && two.contains("db extra"), "{two}");
        assert_eq!(read(3, "Three")?, "same and more", "同じ中身なら触らない");
        let has_table: bool = database.connection().query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE name = 'notes'",
            [],
            |row| row.get(0),
        )?;
        assert!(!has_table, "notes 表は消える");
        fs::remove_dir_all(&data_dir).ok();
        Ok(())
    }

    /// migration 0041 で `papers` に撤回の列が付き、行数は変わらない。
    #[test]
    fn migration_41_adds_retraction_columns_and_keeps_papers()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!(
            "papr-retraction-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&dir)?;
        let db_path = dir.join("papr.db");
        {
            // 0040 までの DB（撤回の列はまだ無い）
            let connection = rusqlite::Connection::open(&db_path)?;
            connection.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT);",
            )?;
            for (version, sql) in super::MIGRATIONS.iter().filter(|(version, _)| *version < 41) {
                connection.execute_batch(sql)?;
                connection.execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [version],
                )?;
            }
            connection.execute_batch("INSERT INTO papers (id, title) VALUES (1, 'One'), (2, 'Two');")?;
        }

        let database = Database::open(&db_path)?;
        let conn = database.connection();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM papers", [], |row| row.get(0))?;
        assert_eq!(count, 2, "papers の行数は変わらない");

        let columns: Vec<String> = {
            let mut statement = conn.prepare("PRAGMA table_info(papers)")?;
            let rows = statement.query_map([], |row| row.get(1))?;
            rows.collect::<Result<_, _>>()?
        };
        assert!(columns.iter().any(|name| name == "is_retracted"));
        assert!(columns.iter().any(|name| name == "retraction_checked_at"));

        // 既存行の既定は未確認（NULL）。撤回されていない（0）と区別する。
        let (is_retracted, checked): (Option<i64>, Option<String>) = conn.query_row(
            "SELECT is_retracted, retraction_checked_at FROM papers WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(is_retracted, None);
        assert_eq!(checked, None);
        fs::remove_dir_all(&dir).ok();
        Ok(())
    }

    /// migration 0038 で `litbase_*` の表が消え、他の表の行は変わらない。
    #[test]
    fn migration_38_drops_litbase_tables_and_keeps_other_rows()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!(
            "papr-drop-litbase-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&dir)?;
        let db_path = dir.join("papr.db");
        {
            // 0037 までの DB（litbase_* 表がある）
            let connection = rusqlite::Connection::open(&db_path)?;
            connection.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT);",
            )?;
            for (version, sql) in super::MIGRATIONS.iter().filter(|(version, _)| *version < 38) {
                connection.execute_batch(sql)?;
                connection.execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [version],
                )?;
            }
            connection.execute_batch(
                "INSERT INTO papers (id, title) VALUES (1, 'One'), (2, 'Two');
                 INSERT INTO chunks (source_kind, source_ref, ord, text, strategy, created_at)
                    VALUES ('abstract', 'abstract_1', 0, 'text', 'abstract', '2026-09-25T00:00:00Z');
                 INSERT INTO activity_log (kind, paper_id, occurred_at)
                    VALUES ('notes', 1, '2026-09-25T00:00:00Z');",
            )?;
        }

        let database = Database::open(&db_path)?;
        let conn = database.connection();

        let litbase_tables: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'litbase_%'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(litbase_tables, 0, "litbase_* の表は残らない");

        for (table, expected) in [("papers", 2i64), ("chunks", 1), ("activity_log", 1)] {
            let count: i64 =
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))?;
            assert_eq!(count, expected, "{table} の行数は変わらない");
        }
        fs::remove_dir_all(&dir).ok();
        Ok(())
    }

    /// migration 0043 の前後で既存の記録が残り、`open` / `agent_read` が入る。
    /// 知らない種類は CHECK で断られる。
    #[test]
    fn migration_43_adds_open_and_agent_read_kinds() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!(
            "papr-activity-open-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&dir)?;
        let db_path = dir.join("papr.db");
        {
            // 0042 までの DB（`open` / `agent_read` がまだ CHECK に無い）
            let connection = rusqlite::Connection::open(&db_path)?;
            connection.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT);",
            )?;
            for (version, sql) in super::MIGRATIONS.iter().filter(|(version, _)| *version < 43) {
                connection.execute_batch(sql)?;
                connection.execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [version],
                )?;
            }
            connection.execute_batch(
                "INSERT INTO papers (id, title) VALUES (1, 'One');
                 INSERT INTO activity_log (kind, paper_id, detail, occurred_at) VALUES
                    ('discover', NULL, 'gravity', '2026-09-25T00:00:00Z'),
                    ('download', 1, NULL, '2026-09-25T00:01:00Z'),
                    ('notes', 1, NULL, '2026-09-25T00:02:00Z');",
            )?;
        }

        let database = Database::open(&db_path)?;
        let conn = database.connection();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM activity_log", [], |row| row.get(0))?;
        assert_eq!(count, 3, "migration の前の記録は残る");
        for kind in ["discover", "download", "notes"] {
            let rows: i64 = conn.query_row(
                "SELECT COUNT(*) FROM activity_log WHERE kind = ?1",
                [kind],
                |row| row.get(0),
            )?;
            assert_eq!(rows, 1, "{kind} の記録が残る");
        }
        let discover_detail: String = conn.query_row(
            "SELECT detail FROM activity_log WHERE kind = 'discover'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(discover_detail, "gravity", "中身も変わらない");

        // 新しい 2 種類が入る。
        database.record_activity("open", Some(1), Some("{\"target\":\"note\"}"))?;
        database.record_activity("agent_read", Some(1), Some("{\"target\":\"paper\"}"))?;
        let added: i64 = conn.query_row(
            "SELECT COUNT(*) FROM activity_log WHERE kind IN ('open', 'agent_read')",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(added, 2);

        // 知らない種類は CHECK で断られる。
        let rejected = conn.execute(
            "INSERT INTO activity_log (kind, paper_id, detail) VALUES ('bogus', NULL, NULL)",
            [],
        );
        assert!(rejected.is_err(), "知らない種類は CHECK で断られる");

        fs::remove_dir_all(&dir).ok();
        Ok(())
    }

    /// migration 0045 の前後で既存の記録が残り、`resolved` が入る。
    /// 0043 で足した `open` / `agent_read` も残る。知らない種類は CHECK で断られる。
    #[test]
    fn migration_45_adds_resolved_kind() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!(
            "papr-activity-resolved-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&dir)?;
        let db_path = dir.join("papr.db");
        {
            // 0044 までの DB（`resolved` がまだ CHECK に無い）
            let connection = rusqlite::Connection::open(&db_path)?;
            connection.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT);",
            )?;
            for (version, sql) in super::MIGRATIONS.iter().filter(|(version, _)| *version < 45) {
                connection.execute_batch(sql)?;
                connection.execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [version],
                )?;
            }
            connection.execute_batch(
                "INSERT INTO activity_log (kind, paper_id, detail, occurred_at) VALUES
                    ('discover', NULL, 'gravity', '2026-09-26T00:00:00Z'),
                    ('download', NULL, NULL, '2026-09-26T00:01:00Z'),
                    ('notes', NULL, NULL, '2026-09-26T00:02:00Z'),
                    ('import', NULL, NULL, '2026-09-26T00:03:00Z'),
                    ('translate', NULL, NULL, '2026-09-26T00:04:00Z'),
                    ('radio', NULL, NULL, '2026-09-26T00:05:00Z'),
                    ('open', NULL, '{\"target\":\"note\"}', '2026-09-26T00:06:00Z'),
                    ('agent_read', NULL, '{\"target\":\"paper\"}', '2026-09-26T00:07:00Z');",
            )?;
        }

        let database = Database::open(&db_path)?;
        let conn = database.connection();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM activity_log", [], |row| row.get(0))?;
        assert_eq!(count, 8, "migration の前の記録は残る");
        for kind in [
            "discover",
            "download",
            "notes",
            "import",
            "translate",
            "radio",
            "open",
            "agent_read",
        ] {
            let rows: i64 = conn.query_row(
                "SELECT COUNT(*) FROM activity_log WHERE kind = ?1",
                [kind],
                |row| row.get(0),
            )?;
            assert_eq!(rows, 1, "{kind} の記録が残る");
        }
        let discover_detail: String = conn.query_row(
            "SELECT detail FROM activity_log WHERE kind = 'discover'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(discover_detail, "gravity", "中身も変わらない");

        // 新しい種類 `resolved` が入る。
        database.record_activity(
            "resolved",
            None,
            Some("{\"key\":\"question:41:x\",\"text\":\"y\"}"),
        )?;
        let added: i64 = conn.query_row(
            "SELECT COUNT(*) FROM activity_log WHERE kind = 'resolved'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(added, 1);

        // 知らない種類は CHECK で断られる。
        let rejected = conn.execute(
            "INSERT INTO activity_log (kind, paper_id, detail) VALUES ('bogus', NULL, NULL)",
            [],
        );
        assert!(rejected.is_err(), "知らない種類は CHECK で断られる");

        fs::remove_dir_all(&dir).ok();
        Ok(())
    }

    /// migration 0052 で research-tree の「理由」の 6 列が消え、行は残る。
    #[test]
    fn migration_52_drops_reason_columns_and_keeps_rows() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!(
            "papr-drop-reasons-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&dir)?;
        let db_path = dir.join("papr.db");
        {
            // 0051 までの DB（「理由」の列がある）
            let connection = rusqlite::Connection::open(&db_path)?;
            connection.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT);",
            )?;
            for (version, sql) in super::MIGRATIONS.iter().filter(|(version, _)| *version < 52) {
                connection.execute_batch(sql)?;
                connection.execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [version],
                )?;
            }
            connection.execute_batch(
                "INSERT INTO rt_nodes (kind, text) VALUES ('Q', '問い');
                 INSERT INTO rt_log (node_id, action, detail, reason) VALUES (1, 'edit', '', '理由');
                 INSERT INTO rt_proposals (op, payload, reason, risk, status)
                    VALUES ('edit', '{}', '提案の理由', 'auto', 'pending');
                 INSERT INTO rt_searches (tool, query, widen_reason) VALUES ('find', 'q', '広げた理由');
                 INSERT INTO find_logs (searched_at, log_date, query, widen_reason, origin)
                    VALUES ('2026-10-07T00:00:00Z', '2026/10/07', 'q', '広げた理由', 'cli');",
            )?;
        }

        let database = Database::open_upto(&db_path, 55)?;
        let conn = database.connection();

        let columns = |table: &str| -> Result<Vec<String>, Box<dyn std::error::Error>> {
            let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
            let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
            Ok(rows.collect::<Result<_, _>>()?)
        };
        for (table, gone) in [
            ("rt_log", "reason"),
            ("rt_proposals", "reason"),
            ("rt_proposals", "decide_note"),
            ("rt_nodes", "removed_reason"),
            ("rt_searches", "widen_reason"),
            ("find_history", "widen_reason"),
        ] {
            assert!(!columns(table)?.iter().any(|name| name == gone), "{table}.{gone} は消える");
        }

        // 行は残る。
        for table in ["rt_nodes", "rt_log", "rt_proposals", "rt_searches", "find_history"] {
            let count: i64 =
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))?;
            assert_eq!(count, 1, "{table} の行は残る");
        }
        fs::remove_dir_all(&dir).ok();
        Ok(())
    }

    /// 0053 で SPROUT（kind S）ができ、旧 rt_seeds が S に移り、親が O の E が S の下へ移る。
    #[test]
    fn migration_53_makes_sprouts_and_moves_branches() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!(
            "papr-sprout-branch-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&dir)?;
        let db_path = dir.join("papr.db");
        {
            // 0052 までの DB。
            let connection = rusqlite::Connection::open(&db_path)?;
            connection.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT);",
            )?;
            for (version, sql) in super::MIGRATIONS.iter().filter(|(version, _)| *version < 53) {
                connection.execute_batch(sql)?;
                connection.execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [version],
                )?;
            }
            // Q1 → O2(Issue)・O3(Issue)。E4 は O2 の下、E5 は E4 の下、E6 は O3 の下。
            // seed は O2 を paths に持つ。O3 はどの seed にも入っていない。
            connection.execute_batch(
                "INSERT INTO rt_nodes (id, kind, parent_id, text, answer_kind) VALUES
                    (1, 'Q', NULL, '大目的', NULL),
                    (2, 'O', 1, '課題 A', 'issue'),
                    (3, 'O', 1, '課題 B', 'issue'),
                    (4, 'E', 2, '実験 A', NULL),
                    (5, 'E', 4, '実行 A1', NULL),
                    (6, 'E', 3, '実験 B', NULL);
                 INSERT INTO rt_seeds (id, title, sentence, path, created_by)
                    VALUES (1, '種', '文', '[2]', 'user');
                 INSERT INTO rt_proposals (id, op, payload, risk, status, result_ref)
                    VALUES (1, 'add_seed', '{}', 'auto', 'applied', 'S1');",
            )?;
        }

        let database = Database::open_upto(&db_path, 55)?;
        let conn = database.connection();

        // rt_seeds は無い。
        let old: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'rt_seeds'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(old, 0, "rt_seeds は消える");

        // seed の S（id 7 = max 6 + seed 1）と、O3 のための S（id 8）。
        let sprouts: Vec<(i64, String)> = {
            let mut stmt =
                conn.prepare("SELECT id, paths FROM rt_nodes WHERE kind = 'S' ORDER BY id")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        assert_eq!(sprouts, vec![(7, "[2]".to_string()), (8, "[3]".to_string())]);

        // E4 は S7 の下、E5 は E4 の下のまま、E6 は S8 の下。
        let parent = |id: i64| -> Option<i64> {
            conn.query_row("SELECT parent_id FROM rt_nodes WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap()
        };
        assert_eq!(parent(4), Some(7));
        assert_eq!(parent(5), Some(4));
        assert_eq!(parent(6), Some(8));

        // 提案は add_sprout になり、S の参照も新しい id になる。
        let (op, result): (String, String) = conn.query_row(
            "SELECT op, result_ref FROM rt_proposals WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(op, "add_sprout");
        assert_eq!(result, "S7");

        // 外部キーの壊れは無い。
        let broken: i64 =
            conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| row.get(0))?;
        assert_eq!(broken, 0, "外部キーが壊れている");

        fs::remove_dir_all(&dir).ok();
        Ok(())
    }

    /// 0055 で、検索セット・選別が履歴へ移る。名前付きセットと Discover の行は残り、
    /// run_id のある行・origin が agent の行の executer が agent、他が human になる。
    #[test]
    fn migration_55_renames_history_and_sets_executer() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!(
            "papr-search-history-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&dir)?;
        let db_path = dir.join("papr.db");
        {
            // 0054 までの DB。
            let connection = rusqlite::Connection::open(&db_path)?;
            connection.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT);",
            )?;
            for (version, sql) in super::MIGRATIONS.iter().filter(|(version, _)| *version < 55) {
                connection.execute_batch(sql)?;
                connection.execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [version],
                )?;
            }
            connection.execute_batch(
                "INSERT INTO search_sets (id, name, source_dbs, query, origin, run_id)
                    VALUES (1, 'my-set', 's2', 'q1', 'cli', NULL),
                           (2, 'discover-log-1', 's2', 'q2', 'discover', NULL),
                           (3, 'agent-1', 's2', 'q3', 'agent', NULL),
                           (4, 'run-1', 's2', 'q4', 'cli', 7);
                 INSERT INTO search_set_items (set_id, position, remote_json, dedupe_key, decision, reason)
                    VALUES (1, 0, '{}', 'k', 'keep', '理由');
                 INSERT INTO find_logs (searched_at, log_date, query, origin, run_id)
                    VALUES ('2026-10-07T00:00:00Z', '2026/10/07', 'f1', 'cli', NULL),
                           ('2026-10-07T00:00:01Z', '2026/10/07', 'f2', 'mcp', NULL),
                           ('2026-10-07T00:00:02Z', '2026/10/07', 'f3', 'cli', 9);",
            )?;
        }

        let database = Database::open(&db_path)?;
        let conn = database.connection();
        let executers = |sql: &str| -> Result<Vec<String>, Box<dyn std::error::Error>> {
            let mut statement = conn.prepare(sql)?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<Result<_, _>>()?)
        };
        assert_eq!(
            executers("SELECT executer FROM search_history ORDER BY id")?,
            ["human", "human", "agent", "agent"]
        );
        assert_eq!(
            executers("SELECT executer FROM find_history ORDER BY id")?,
            ["human", "agent", "agent"]
        );
        let items: i64 =
            conn.query_row("SELECT COUNT(*) FROM search_history_items WHERE history_id = 1", [], |row| row.get(0))?;
        assert_eq!(items, 1, "候補の行は残る");
        let columns = executers("SELECT name FROM pragma_table_info('search_history_items')")?;
        assert!(!columns.iter().any(|name| name == "decision" || name == "reason"));
        fs::remove_dir_all(&dir).ok();
        Ok(())
    }

    /// 0054 で、生きた O を持たず Q の子を持つ一番上の Q が消え、その子の Q が top_Q になる。
    /// 生きた O を持つ一番上の Q はそのまま残る。
    #[test]
    fn migration_54_promotes_top_q_children() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!(
            "papr-top-q-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&dir)?;
        let db_path = dir.join("papr.db");
        {
            // 0053 までの DB。
            let connection = rusqlite::Connection::open(&db_path)?;
            connection.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT);",
            )?;
            for (version, sql) in super::MIGRATIONS.iter().filter(|(version, _)| *version < 54) {
                connection.execute_batch(sql)?;
                connection.execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [version],
                )?;
            }
            // Q1（子の Q が 3 つ・O なし）と Q5（O6 を持つ）。
            connection.execute_batch(
                "INSERT INTO rt_nodes (id, kind, parent_id, text, answer_kind, ord) VALUES
                    (1, 'Q', NULL, '大目的', NULL, 1),
                    (2, 'Q', 1, '背景', NULL, 1),
                    (3, 'Q', 1, '目的', NULL, 2),
                    (4, 'Q', 1, '手段', NULL, 3),
                    (5, 'Q', NULL, '別の問い', NULL, 2),
                    (6, 'O', 5, '答え', 'answer', 1);",
            )?;
        }

        let database = Database::open_upto(&db_path, 55)?;
        let conn = database.connection();

        let parent = |id: i64| -> Option<i64> {
            conn.query_row("SELECT parent_id FROM rt_nodes WHERE id = ?1", [id], |row| row.get(0))
                .unwrap()
        };
        let removed = |id: i64| -> bool {
            conn.query_row("SELECT removed_at IS NOT NULL FROM rt_nodes WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap()
        };
        // Q1 の子の Q が top_Q になり、Q1 は消えた Node になる。
        for id in [2, 3, 4] {
            assert_eq!(parent(id), None, "Q{id} は top_Q になる");
            assert!(!removed(id), "Q{id} は生きたまま");
        }
        assert!(removed(1), "Q1 は消えた Node になる");
        // O を持つ Q5 は子の O6 ごとそのまま。
        assert_eq!(parent(5), None);
        assert!(!removed(5), "O を持つ top_Q は残る");
        assert_eq!(parent(6), Some(5));

        let broken: i64 =
            conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| row.get(0))?;
        assert_eq!(broken, 0, "外部キーが壊れている");

        fs::remove_dir_all(&dir).ok();
        Ok(())
    }

    /// 10 分以内の同じ対象・同じ種類の記録は 1 件にまとまり、
    /// 10 分を過ぎたものや対象の違うものは別に残る（N01）。
    #[test]
    fn record_activity_dedup_merges_within_window() -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::in_memory()?;
        let detail = r#"{"target":"note","path":"papers/0001_Foo/note.md","surface":"tui"}"#;
        let key = crate::activity::target_key(detail, None);

        assert!(database.record_activity_dedup("open", None, detail, &key, 10)?);
        assert!(!database.record_activity_dedup("open", None, detail, &key, 10)?);
        let count = |database: &Database| -> Result<i64, Box<dyn std::error::Error>> {
            Ok(database.connection().query_row(
                "SELECT COUNT(*) FROM activity_log WHERE kind = 'open'",
                [],
                |row| row.get(0),
            )?)
        };
        assert_eq!(count(&database)?, 1, "10 分以内は 1 件のまま");

        // 10 分を過ぎれば別の記録になる。
        database.connection().execute(
            "UPDATE activity_log SET occurred_at = datetime('now', '-11 minutes') WHERE kind = 'open'",
            [],
        )?;
        assert!(database.record_activity_dedup("open", None, detail, &key, 10)?);
        assert_eq!(count(&database)?, 2, "10 分を過ぎれば 2 件");

        // 対象（path）が違えば別の記録になる。
        let other = r#"{"target":"note","path":"papers/0001_Foo/other.md","surface":"tui"}"#;
        let other_key = crate::activity::target_key(other, None);
        assert!(database.record_activity_dedup("open", None, other, &other_key, 10)?);
        assert_eq!(count(&database)?, 3, "パスが違えば別に残る");

        // 対応しない種類は記録しない（成功扱いの false）。
        assert!(!database.record_activity_dedup("bogus", None, detail, &key, 10)?);
        Ok(())
    }

    /// 旧形式（絶対パス）の DB ファイルを作る。
    ///
    /// マイグレーションだけを当て、`Database::open` はまだ通さない
    /// （`open` は相対化の印を `app_meta` に付けてしまうため）。
    fn seed_legacy_db(db_path: &Path) -> Result<rusqlite::Connection, Box<dyn std::error::Error>> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let connection = rusqlite::Connection::open(db_path)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
        )?;
        for (version, sql) in super::MIGRATIONS {
            connection.execute_batch(sql)?;
            connection.execute(
                "INSERT INTO schema_migrations (version) VALUES (?1)",
                [version],
            )?;
        }
        Ok(connection)
    }

    /// `Database::open` が data_dir の中の絶対パス行だけを相対化し、
    /// 外のパスはバイト単位でそのまま残すこと。変換は `app_meta` の印で
    /// 1 回だけ走ること（段階 2b）。
    #[test]
    fn open_converts_stored_paths_once_and_keeps_outside_paths()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "papr-relativize-{}-{}",
            std::process::id(),
            line!()
        ));
        let data_dir = root.join(".sai").join("paper");
        let folder = data_dir.join("papers").join("0001_Inside");
        fs::create_dir_all(&folder)?;
        let inside_pdf = folder.join("paper.pdf");
        fs::write(&inside_pdf, b"%PDF-1.4 inside")?;
        let inside_md = folder.join("extract.md");
        fs::write(&inside_md, "# body")?;
        let inside_image = folder.join("figures").join("fig-001.png");
        fs::create_dir_all(inside_image.parent().unwrap())?;
        fs::write(&inside_image, b"\x89PNG")?;
        // data_dir の外（例: <root>/data/extracts/）は絶対パスのまま残す。
        let outside_md = root.join("data").join("extracts").join("old.md");
        fs::create_dir_all(outside_md.parent().unwrap())?;
        fs::write(&outside_md, "# old")?;

        let db_path = data_dir.join("papr.db");
        {
            let connection = seed_legacy_db(&db_path)?;
            connection.execute(
                "INSERT INTO papers (id, title, arxiv_id, pdf_path) VALUES (1, 'Inside', '2401.00001', ?1)",
                params![inside_pdf.to_string_lossy().to_string()],
            )?;
            connection.execute(
                "INSERT INTO paper_extracts (paper_id, status, pdf_path, md_path)
                 VALUES ('ArXiv:2401.00001', 'ready', ?1, ?2)",
                params![
                    inside_pdf.to_string_lossy().to_string(),
                    outside_md.to_string_lossy().to_string()
                ],
            )?;
            connection.execute(
                "INSERT INTO paper_figures (paper_id, fig_no, image_path, created_at)
                 VALUES ('ArXiv:2401.00001', 1, ?1, '2026-09-24')",
                params![inside_image.to_string_lossy().to_string()],
            )?;
        }

        {
            let database = Database::open(&db_path)?;
            let connection = database.connection();

            // data_dir の中 → 相対パス（区切りは /）
            let pdf_path: String =
                connection.query_row("SELECT pdf_path FROM papers WHERE id = 1", [], |row| {
                    row.get(0)
                })?;
            assert_eq!(pdf_path, "papers/0001_Inside/paper.pdf");
            let image_path: String = connection.query_row(
                "SELECT image_path FROM paper_figures LIMIT 1",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(image_path, "papers/0001_Inside/figures/fig-001.png");

            // 外のパスは絶対パスのまま（バイト単位で変わらない）
            let md_path: String =
                connection.query_row("SELECT md_path FROM paper_extracts LIMIT 1", [], |row| {
                    row.get(0)
                })?;
            assert_eq!(md_path, outside_md.to_string_lossy());
            let extract_pdf: String =
                connection.query_row("SELECT pdf_path FROM paper_extracts LIMIT 1", [], |row| {
                    row.get(0)
                })?;
            assert_eq!(extract_pdf, "papers/0001_Inside/paper.pdf");

            // 印が残っている
            let marker: String = connection.query_row(
                "SELECT value FROM app_meta WHERE key = 'stored_paths_relative_v1'",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(marker, "1");

            // 読む側は実ファイルパスへ解決される
            let resolved = database.resolve_path(&pdf_path);
            assert_eq!(resolved, inside_pdf);
            assert!(resolved.is_file());

            // 印があるうえに後から絶対パスで書いた行は二度と変換されない。
            connection.execute(
                "INSERT INTO papers (id, title, pdf_path) VALUES (2, 'Late', ?1)",
                params![inside_pdf.to_string_lossy().to_string()],
            )?;
        }
        {
            let database = Database::open(&db_path)?;
            let late: String = database.connection().query_row(
                "SELECT pdf_path FROM papers WHERE id = 2",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(
                late,
                inside_pdf.to_string_lossy(),
                "2 回目の open では変換しないこと（印で 1 回だけ）"
            );
        }

        let _ = fs::remove_dir_all(&root);
        Ok(())
    }

    /// 相対パスに変換済みの DB で Library の走査を 2 回回しても、
    /// 同じ PDF が「新しい論文」として二重登録されないこと（段階 2b）。
    #[test]
    fn scan_twice_on_relative_db_does_not_duplicate_papers()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "papr-relative-scan-{}-{}",
            std::process::id(),
            line!()
        ));
        let data_dir = root.join(".sai").join("paper");
        let pdf = data_dir.join("papers").join("0001_Scan").join("paper.pdf");
        fs::create_dir_all(pdf.parent().unwrap())?;
        fs::write(&pdf, b"%PDF-1.4 scan target")?;

        // 旧形式（絶対パス + コンテンツハッシュ）で登録した状態を作る。
        let inspected = LibraryIndexer::inspect(&pdf)?;
        let db_path = data_dir.join("papr.db");
        {
            let connection = seed_legacy_db(&db_path)?;
            connection.execute(
                "INSERT INTO papers (id, title, pdf_path, content_hash, file_size)
                 VALUES (1, 'scan target', ?1, ?2, ?3)",
                params![
                    pdf.to_string_lossy().to_string(),
                    inspected.content_hash,
                    i64::try_from(inspected.file_size).unwrap_or(i64::MAX)
                ],
            )?;
        }

        let database = Database::open(&db_path)?;
        // 変換済み（相対パス）であること。
        let stored: String = database.connection().query_row(
            "SELECT pdf_path FROM papers WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(stored, "papers/0001_Scan/paper.pdf");

        // 走査を 2 回回す。
        let roots = vec![data_dir.clone()];
        for round in 0..2 {
            let scanned = LibraryIndexer::scan(&roots);
            assert_eq!(scanned.len(), 1, "{round} 回目の走査で PDF を 1 件見つける");
            let imported = database.import_pdf(&scanned[0])?;
            assert!(
                !imported,
                "{round} 回目: 既存論文として扱われ、新規登録されないこと"
            );
            // pdf_path での検索（重複判定）も相対パスで当たること。
            assert!(
                database.paper_id_for_pdf(&scanned[0])?.is_some(),
                "{round} 回目: pdf_path の照合が一致すること"
            );
        }

        let count: i64 =
            database
                .connection()
                .query_row("SELECT COUNT(*) FROM papers", [], |row| row.get(0))?;
        assert_eq!(count, 1, "論文が 1 件のまま変わらないこと");

        let _ = fs::remove_dir_all(&root);
        Ok(())
    }

    /// プロジェクトのフォルダを丸ごと別の名前に移しても、
    /// DB の相対パスから PDF と extract.md が見つかる（段階 2b）。
    #[test]
    fn renaming_project_folder_keeps_pdf_and_extract_findable()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = std::env::temp_dir().join(format!(
            "papr-rename-project-{}-{}",
            std::process::id(),
            line!()
        ));
        fs::create_dir_all(&temp)?;
        let project_a = temp.join("projA");
        let data_dir_a = project_a.join(".sai").join("paper");
        let folder = data_dir_a.join("papers").join("0001_Renamed-Project");
        fs::create_dir_all(&folder)?;
        let pdf = folder.join("paper.pdf");
        fs::write(&pdf, b"%PDF-1.4 rename me")?;
        let md = folder.join("extract.md");
        fs::write(&md, "# extract body")?;

        let key = "ArXiv:2401.00001";
        let db_path = data_dir_a.join("papr.db");
        {
            let connection = seed_legacy_db(&db_path)?;
            connection.execute(
                "INSERT INTO papers (id, title, arxiv_id, pdf_path)
                 VALUES (1, 'Renamed Project', '2401.00001', ?1)",
                params![pdf.to_string_lossy().to_string()],
            )?;
            connection.execute(
                "INSERT INTO paper_extracts (paper_id, status, md_path)
                 VALUES (?1, 'ready', ?2)",
                params![key, md.to_string_lossy().to_string()],
            )?;
        }
        // ここで相対化される（移動の前でも後でも有効な形にする）。
        {
            let database = Database::open(&db_path)?;
            let stored: String = database.connection().query_row(
                "SELECT pdf_path FROM papers WHERE id = 1",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(stored, "papers/0001_Renamed-Project/paper.pdf");
        }

        // プロジェクトのフォルダごと rename する。
        let project_b = temp.join("projB-renamed");
        fs::rename(&project_a, &project_b)?;
        let data_dir_b = project_b.join(".sai").join("paper");

        let database = Database::open(&data_dir_b.join("papr.db"))?;
        // PDF: DB から読んだパスが移動後の場所を指し、実在する。
        let paper = database
            .library_paper_by_id(1)?
            .ok_or("論文が見つからない")?;
        let found_pdf = PathBuf::from(paper.pdf_path.clone().ok_or("pdf_path が無い")?);
        assert!(
            found_pdf.is_file(),
            "移動後も PDF が見つかる: {found_pdf:?}"
        );
        assert!(
            found_pdf.starts_with(&project_b),
            "移動後のプロジェクトを指している: {found_pdf:?}"
        );
        // extract.md: DB（paper_extracts.md_path）から読める。
        let stored_md: String = database.connection().query_row(
            "SELECT md_path FROM paper_extracts WHERE paper_id = ?1",
            [key],
            |row| row.get(0),
        )?;
        assert_eq!(stored_md, "papers/0001_Renamed-Project/extract.md");
        let resolved_md = database.resolve_path(&stored_md);
        assert!(resolved_md.is_file(), "移動後も extract.md が見つかる");
        let found_md =
            crate::citation::resolve_extract_md(database.connection(), Some(1), None, &data_dir_b);
        let expected_md = data_dir_b
            .join("papers")
            .join("0001_Renamed-Project")
            .join("extract.md");
        assert_eq!(found_md.as_ref(), Some(&expected_md));

        let _ = fs::remove_dir_all(&temp);
        Ok(())
    }

    /// 0056 で research-tree を作り直す。古い O の行は消え、`rt_logic`・`rt_proposals` は無くなり、
    /// `rt_approvals` が入る（既存の research-tree のデータは無い前提）。
    #[test]
    fn migration_56_recreates_rt_tables() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!(
            "papr-rt-redesign-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&dir)?;
        let db_path = dir.join("papr.db");
        {
            // 0055 までの DB に、旧い形の research-tree（O の答え）を入れる。
            let connection = rusqlite::Connection::open(&db_path)?;
            connection.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT);",
            )?;
            for (version, sql) in super::MIGRATIONS.iter().filter(|(version, _)| *version < 56) {
                connection.execute_batch(sql)?;
                connection.execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [version],
                )?;
            }
            connection.execute_batch(
                "INSERT INTO rt_meta (key, value) VALUES ('initialized_at', 'now');
                 INSERT INTO rt_nodes (kind, text) VALUES ('Q', '問い');
                 INSERT INTO rt_nodes (kind, parent_id, text, answer_kind) VALUES ('O', 1, '答え', 'answer');
                 INSERT INTO rt_logic (q_id, value, note, findings, f_count, created_by)
                    VALUES (1, 'ok', '', '[]', 0, 'user');",
            )?;
        }

        let database = Database::open(&db_path)?;
        let conn = database.connection();
        let count = |sql: &str| -> Result<i64, Box<dyn std::error::Error>> {
            Ok(conn.query_row(sql, [], |row| row.get(0))?)
        };
        assert_eq!(count("SELECT COUNT(*) FROM rt_nodes")?, 0, "rt_nodes は作り直して空");
        assert_eq!(count("SELECT COUNT(*) FROM rt_meta")?, 0, "rt_meta は空（init で作り直す）");
        for table in ["rt_logic", "rt_proposals"] {
            assert_eq!(
                count(&format!(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '{table}'"
                ))?,
                0,
                "{table} は無くなる"
            );
        }
        assert_eq!(
            count("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'rt_approvals'")?,
            1,
            "rt_approvals が入る"
        );
        let stance_column = count("SELECT COUNT(*) FROM pragma_table_info('rt_nodes') WHERE name = 'stance'")?;
        assert_eq!(stance_column, 1, "stance の列が入る");

        fs::remove_dir_all(&dir).ok();
        Ok(())
    }

    #[test]
    fn migration_57_allows_seed_node() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!(
            "papr-seed-node-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&dir)?;
        let db_path = dir.join("papr.db");
        {
            // 0056 までの DB に、親 NULL の S（古い SPROUT）と、それを指す rt_log の行を入れる。
            let connection = rusqlite::Connection::open(&db_path)?;
            connection.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT);",
            )?;
            for (version, sql) in super::MIGRATIONS.iter().filter(|(version, _)| *version < 57) {
                connection.execute_batch(sql)?;
                connection.execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [version],
                )?;
            }
            connection.execute_batch(
                "INSERT INTO rt_nodes (kind, text) VALUES ('Q', '問い');
                 INSERT INTO rt_nodes (kind, text) VALUES ('S', '古い SPROUT');
                 INSERT INTO rt_log (node_id, action) VALUES (2, 'add_q');",
            )?;
        }

        let database = Database::open(&db_path)?;
        let conn = database.connection();
        let count = |sql: &str| -> Result<i64, Box<dyn std::error::Error>> {
            Ok(conn.query_row(sql, [], |row| row.get(0))?)
        };
        assert_eq!(
            count("SELECT COUNT(*) FROM rt_nodes WHERE kind = 'S' AND parent_id IS NULL")?,
            1,
            "親 NULL の既存の S は残る"
        );
        assert_eq!(count("SELECT COUNT(*) FROM rt_log")?, 1, "rt_log の行は残る");
        conn.execute("INSERT INTO rt_nodes (kind, text) VALUES ('D', 'seed')", [])?;
        let seed_id: i64 = conn.query_row("SELECT id FROM rt_nodes WHERE kind = 'D'", [], |row| row.get(0))?;
        conn.execute(
            "INSERT INTO rt_nodes (kind, parent_id, text) VALUES ('S', ?1, 'SPROUT')",
            [seed_id],
        )?;
        assert!(
            conn.execute("INSERT INTO rt_nodes (kind, parent_id, text) VALUES ('D', 1, 'x')", []).is_err(),
            "親を持つ D は弾く"
        );
        let broken = conn.prepare("PRAGMA foreign_key_check")?.query([])?.next()?.is_some();
        assert!(!broken, "外部キーが壊れていない");

        fs::remove_dir_all(&dir).ok();
        Ok(())
    }

    /// 0058 で rt_log に move_node・regroup が入る。既存の行は id ごと残り、追記だけの決まりも残る。
    #[test]
    fn migration_58_keeps_rt_log_rows_and_append_only() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!(
            "papr-rt-log-regroup-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
        fs::create_dir_all(&dir)?;
        let db_path = dir.join("papr.db");
        {
            // 0057 までの DB に、記録を 2 行入れる（id を 1・2 にして、後で同じ id が残るか見る）。
            let connection = rusqlite::Connection::open(&db_path)?;
            connection.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT);",
            )?;
            for (version, sql) in super::MIGRATIONS.iter().filter(|(version, _)| *version < 58) {
                connection.execute_batch(sql)?;
                connection.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [version])?;
            }
            connection.execute_batch(
                "INSERT INTO rt_nodes (kind, text) VALUES ('Q', '問い');
                 INSERT INTO rt_nodes (kind, parent_id, text) VALUES ('A', 1, '答え');
                 INSERT INTO rt_log (node_id, action, detail) VALUES (1, 'add_q', '');
                 INSERT INTO rt_log (node_id, action, detail) VALUES (2, 'move_a', '1 から A 1 件');",
            )?;
        }

        let database = Database::open(&db_path)?;
        let conn = database.connection();
        let rows: Vec<(i64, String)> = conn
            .prepare("SELECT id, action FROM rt_log ORDER BY id")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        assert_eq!(rows, vec![(1, "add_q".to_string()), (2, "move_a".to_string())], "既存の行は id ごと残る");
        conn.execute("INSERT INTO rt_log (node_id, action) VALUES (2, 'move_node')", [])?;
        conn.execute("INSERT INTO rt_log (node_id, action) VALUES (1, 'regroup')", [])?;
        assert!(conn.execute("INSERT INTO rt_log (node_id, action) VALUES (1, 'nope')", []).is_err(), "知らない action は弾く");
        assert!(conn.execute("UPDATE rt_log SET detail = 'x' WHERE id = 1", []).is_err(), "UPDATE は拒否");
        assert!(conn.execute("DELETE FROM rt_log WHERE id = 1", []).is_err(), "DELETE は拒否");

        fs::remove_dir_all(&dir).ok();
        Ok(())
    }
}
