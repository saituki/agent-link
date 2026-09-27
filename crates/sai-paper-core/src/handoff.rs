//! task.md 発行：論文をエージェントに引き渡す。
//!
//! sai-paper から選択した論文を sai-link のカンバンボードに task.md 作成して
//! 引き渡す。run_id は sai-link の execution_history テーブルに reserve して得る。

use crate::database::Database;
use crate::paths::PaperRoots;
use rusqlite::OptionalExtension;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// task.md 発行操作のエラー。
#[derive(Debug, Error)]
pub enum HandoffError {
    /// ファイル I/O 失敗
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// DB 操作失敗
    #[error("database error: {0}")]
    Database(#[from] crate::database::DatabaseError),
    /// SQLite 操作失敗
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// JSON エラー
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// ブロック未検出
    #[error("block not found: {0}")]
    BlockNotFound(String),
    /// その他エラー
    #[error("{0}")]
    Other(String),
}

/// task.md 発行操作の結果型。
pub type Result<T> = std::result::Result<T, HandoffError>;

/// ブロック情報（list_blocks の返り値）。
#[derive(Debug, Clone)]
pub struct BlockInfo {
    /// frontmatter の id フィールド
    pub id: String,
    /// frontmatter の name フィールド
    pub name: String,
    /// frontmatter の description フィールド
    pub description: String,
    /// ファイルの格納パス
    pub path: PathBuf,
}

/// todo に残っているカード 1 枚（[`list_todo_cards`] の返り値）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TodoCard {
    /// frontmatter の run_id
    pub run_id: i64,
    /// frontmatter の block
    pub block: String,
    /// frontmatter の tag
    pub tag: String,
    /// frontmatter の description
    pub description: String,
    /// frontmatter の origin_run_id（`-` のときは None）
    pub origin_run_id: Option<i64>,
    /// task.md の絶対パス
    pub path: PathBuf,
}

/// 発行結果。
#[derive(Debug, Clone)]
pub struct HandoffResult {
    /// 割り当てられた run_id
    pub run_id: i64,
    /// 作成された task.md の絶対パス
    pub path: PathBuf,
}

/// ブロック情報一覧を取得する。
///
/// `<root>/.sai/blocks/*.md` と `~/.sai/blocks/*.md` を読み、
/// frontmatter から id / name / description を抜き出す。
/// ローカル（`<root>/.sai/blocks/`）の方がグローバル（`~/.sai/blocks/`）より優先。
///
/// キーは frontmatter の `id` フィールドであり、ファイル名ではない。
/// 同じ id を持つファイルが複数あれば、ローカルが勝つ。
///
/// # Errors
///
/// ファイル読み込み失敗やフロントマター解析失敗の場合。
pub fn list_blocks(roots: &PaperRoots) -> Result<Vec<BlockInfo>> {
    use std::collections::HashMap;

    let mut blocks: HashMap<String, BlockInfo> = HashMap::new();

    // グローバルブロック（`~/.sai/blocks/`）を先に読む
    let global_blocks_dir = std::env::var("HOME")
        .ok()
        .map(|h: String| PathBuf::from(h).join(".sai").join("blocks"))
        .filter(|p: &PathBuf| p.exists());

    if let Some(dir) = global_blocks_dir {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                if let Ok(info) = parse_block_file(&path) {
                    blocks.insert(info.id.clone(), info);
                }
            }
        }
    }

    // ローカルブロック（`<root>/.sai/blocks/`）を後に読む（上書きする）
    let local_blocks_dir = roots.root.join(".sai").join("blocks");
    if local_blocks_dir.exists() {
        for entry in fs::read_dir(&local_blocks_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                if let Ok(info) = parse_block_file(&path) {
                    blocks.insert(info.id.clone(), info);
                }
            }
        }
    }

    let mut result: Vec<_> = blocks.into_values().collect();
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
}

/// `<root>/.sai/outputs/todo/*/task.md` のカードを一覧する。
///
/// `todo` 以外のフォルダ（`running` / `done` 等）は見ない。
/// `tag` を指定すると、frontmatter の `tag` が一致するカードだけを返す。
/// 返り値は `run_id` の昇順。
///
/// # Errors
///
/// `todo` ディレクトリの読み込み失敗、`task.md` の読み込み失敗の場合。
pub fn list_todo_cards(roots: &PaperRoots, tag: Option<&str>) -> Result<Vec<TodoCard>> {
    let todo_dir = roots.root.join(".sai").join("outputs").join("todo");
    let mut cards = Vec::new();
    if !todo_dir.exists() {
        return Ok(cards);
    }

    for entry in fs::read_dir(&todo_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let task_path = entry.path().join("task.md");
        if !task_path.exists() {
            continue;
        }
        let content = fs::read_to_string(&task_path)?;
        // frontmatter が無い / 壊れているカードは飛ばす。
        let Ok((frontmatter, _)) = split_frontmatter(&content) else {
            continue;
        };
        let card_tag = get_fm_value(&frontmatter, "tag").unwrap_or_default();
        if let Some(want) = tag {
            if card_tag != want {
                continue;
            }
        }
        let run_id = get_fm_value(&frontmatter, "run_id")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let origin_run_id = get_fm_value(&frontmatter, "origin_run_id")
            .and_then(|v| v.parse().ok());
        cards.push(TodoCard {
            run_id,
            block: get_fm_value(&frontmatter, "block").unwrap_or_default(),
            tag: card_tag,
            description: get_fm_value(&frontmatter, "description").unwrap_or_default(),
            origin_run_id,
            path: task_path,
        });
    }

    cards.sort_by_key(|c| c.run_id);
    Ok(cards)
}

/// ブロック md ファイルをパースして BlockInfo を抽出する。
fn parse_block_file(path: &Path) -> Result<BlockInfo> {
    let content = fs::read_to_string(path)?;
    let (frontmatter, _) = split_frontmatter(&content)?;

    let id = get_fm_value(&frontmatter, "id")
        .ok_or_else(|| HandoffError::Other("ブロック md に id フィールドがない".to_string()))?;
    let name = get_fm_value(&frontmatter, "name").unwrap_or_else(|| id.clone());
    let description = get_fm_value(&frontmatter, "description").unwrap_or_default();

    Ok(BlockInfo {
        id,
        name,
        description,
        path: path.to_path_buf(),
    })
}

/// frontmatter（`---` ... `---`）と本文に分割する。
fn split_frontmatter(content: &str) -> Result<(String, String)> {
    let mut lines = content.lines().peekable();

    // 先頭の `---`
    if lines.peek().map(|l| l.trim()) != Some("---") {
        return Err(HandoffError::Other(
            "frontmatter が `---` で始まっていない".to_string(),
        ));
    }
    lines.next();

    let mut fm_lines = Vec::new();
    let mut found_end = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            found_end = true;
            break;
        }
        fm_lines.push(line);
    }

    if !found_end {
        return Err(HandoffError::Other(
            "frontmatter の終端 `---` が見つからない".to_string(),
        ));
    }

    let fm = fm_lines.join("\n");
    let body = lines.collect::<Vec<_>>().join("\n");

    Ok((fm, body))
}

/// frontmatter 行群から指定キーの値を取得する。
fn get_fm_value(frontmatter: &str, key: &str) -> Option<String> {
    for line in frontmatter.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim() == key {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// カード発行の依頼（[`create_handoff_with`] の引数）。
pub struct HandoffRequest<'a> {
    /// 実行するブロックの id
    pub block_id: &'a str,
    /// 対象論文の `papers.id`（無くてもよい）
    pub paper_ids: &'a [i64],
    /// 依頼の本文
    pub description: &'a str,
    /// カードのタグ（任意）
    pub tag: Option<&'a str>,
    /// 実行エージェント（`pi` / `cc`）
    pub runner: &'a str,
    /// 推薦した実行の run_id（任意。あれば `origin_run_id` に書く）
    pub origin_run: Option<i64>,
    /// 推薦の理由（任意。あれば本文の先頭に書く）
    pub reason: Option<&'a str>,
}

/// task.md を発行する（従来の入口。TUI と CLI の既存呼び出しが使う）。
///
/// `origin_run` と `reason` は無しで [`create_handoff_with`] に委譲する。
///
/// # Errors
///
/// DB エラー、ファイル I/O エラー、ブロック未検出等。
pub fn create_handoff(
    db: &Database,
    roots: &PaperRoots,
    block_id: &str,
    paper_ids: &[i64],
    description: &str,
    tag: Option<&str>,
    runner: &str,
) -> Result<HandoffResult> {
    create_handoff_with(
        db,
        roots,
        &HandoffRequest {
            block_id,
            paper_ids,
            description,
            tag,
            runner,
            origin_run: None,
            reason: None,
        },
    )
}

/// task.md を発行する。
///
/// 処理:
/// 1. sai-link DB に run_id を予約
/// 2. ブロックの存在を確認
/// 3. 論文情報を集める
/// 4. `<root>/.sai/outputs/todo/<run_id>/task.md` に書く
/// 5. { run_id, path } を返す
///
/// # Errors
///
/// DB エラー、ファイル I/O エラー、ブロック未検出等。
pub fn create_handoff_with(
    db: &Database,
    roots: &PaperRoots,
    request: &HandoffRequest<'_>,
) -> Result<HandoffResult> {
    let block_id = request.block_id;

    // 1. ブロック情報を検索
    let blocks = list_blocks(roots)?;
    blocks
        .iter()
        .find(|b| b.id == block_id)
        .ok_or_else(|| HandoffError::BlockNotFound(block_id.to_string()))?;

    // 2. sai-link DB に run_id を予約
    let run_id = reserve_run_id(roots)?;

    // 3. global_round を決定
    let global_round = calculate_global_round(roots)?;

    // 4. 論文情報を集める
    let mut paper_infos = Vec::new();
    for paper_id in request.paper_ids {
        if let Ok(Some(paper)) = db.library_paper_by_id(*paper_id) {
            let extracts_path = get_paper_extracts_path(db, roots, &paper);
            let translations_path = get_paper_translations_path(roots, &paper);
            let note_path = get_paper_note_path(roots, &paper);

            paper_infos.push(PaperInfo {
                id: paper.id,
                title: paper.title,
                authors: paper.authors,
                year: parse_year(&paper.published_at),
                journal: paper.journal,
                doi: paper.doi,
                arxiv_id: paper.arxiv_id,
                pdf_path: paper.pdf_path,
                extracts_path,
                translations_path,
                note_path,
            });
        }
    }

    // 5. task.md の本文を構築
    let body = build_task_body(request.description, &paper_infos, request.reason);

    // 6. frontmatter を構築
    let frontmatter = build_frontmatter(
        run_id,
        block_id,
        global_round,
        request.tag.unwrap_or(""),
        request.runner,
        request.description,
        request.origin_run,
    )?;

    // 7. task.md を出力ディレクトリに書く
    let output_dir = roots
        .root
        .join(".sai")
        .join("outputs")
        .join("todo")
        .join(run_id.to_string());
    fs::create_dir_all(&output_dir)?;
    let task_path = output_dir.join("task.md");

    let task_md = format!("{}\n---\n\n{}\n", frontmatter, body);
    fs::write(&task_path, &task_md)?;

    Ok(HandoffResult {
        run_id,
        path: task_path,
    })
}

/// task.md 用の情報（1 論文）。
struct PaperInfo {
    id: i64,
    title: String,
    authors: String,
    year: Option<String>,
    journal: Option<String>,
    doi: Option<String>,
    arxiv_id: Option<String>,
    pdf_path: Option<String>,
    extracts_path: Option<PathBuf>,
    translations_path: Option<PathBuf>,
    note_path: Option<PathBuf>,
}

/// published_at から年を抽出する（ISO8601 形式想定）。
fn parse_year(published_at: &Option<String>) -> Option<String> {
    published_at
        .as_ref()
        .and_then(|s| s.split('-').next())
        .map(|y| y.to_string())
}

/// 論文フォルダを解決する。
///
/// 現行レイアウトは `papers/{id:04}_{短いタイトル}/` で、`papers.pdf_path` の親が
/// そのフォルダになる。pdf_path が無い場合は置き場の規則
/// （[`crate::paths::paper_folder`]）から推定する。
fn resolve_paper_folder(
    roots: &PaperRoots,
    paper: &crate::models::LibraryPaper,
) -> Option<PathBuf> {
    if let Some(pdf) = paper.pdf_path.as_deref() {
        if let Some(parent) = Path::new(pdf).parent() {
            if parent.is_dir() {
                return Some(parent.to_path_buf());
            }
        }
    }
    let candidate = crate::paths::paper_folder(&roots.data_dir, paper.id, &paper.title);
    candidate.is_dir().then_some(candidate)
}

/// 論文の抽出 Markdown パスを解決する（記録 → 論文フォルダ → 旧レイアウト）。
///
/// 引用抽出の記録（`paper_extracts`）を、まず `papers_id` で、無ければ抽出キーで
/// 探す（core 共通の [`crate::citation::resolve_extract_md`]。段階 2c）。
fn get_paper_extracts_path(
    db: &Database,
    roots: &PaperRoots,
    paper: &crate::models::LibraryPaper,
) -> Option<PathBuf> {
    if let Some(path) =
        crate::citation::resolve_extract_md(db.connection(), Some(paper.id), None, &roots.data_dir)
    {
        return Some(path);
    }
    if let Some(folder) = resolve_paper_folder(roots, paper) {
        let candidate = folder.join(crate::paths::paper_file_names::EXTRACT);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    // 旧レイアウト（フラット・番号ファイル名）フォールバック
    let legacy = roots
        .data_dir
        .join("extracts")
        .join(format!("{}.md", paper.id));
    legacy.exists().then_some(legacy)
}

/// 論文の日本語翻訳 PDF パスを解決する。
fn get_paper_translations_path(
    roots: &PaperRoots,
    paper: &crate::models::LibraryPaper,
) -> Option<PathBuf> {
    if let Some(folder) = resolve_paper_folder(roots, paper) {
        let candidate = folder.join(crate::paths::paper_file_names::PDF_JA);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let legacy = roots
        .data_dir
        .join("translations")
        .join(format!("{}.pdf", paper.id));
    legacy.exists().then_some(legacy)
}

/// 論文のノートファイルパスを解決する。
fn get_paper_note_path(roots: &PaperRoots, paper: &crate::models::LibraryPaper) -> Option<PathBuf> {
    if let Some(folder) = resolve_paper_folder(roots, paper) {
        let candidate = folder.join(crate::paths::paper_file_names::NOTE);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let legacy = roots
        .data_dir
        .join("notes")
        .join(format!("{}.md", paper.id));
    legacy.exists().then_some(legacy)
}

/// task.md の本文（frontmatter の下）を構築する。
///
/// `reason` があれば `# task_text` の先頭に「推薦の理由: …」を足す。
/// 論文が無ければ `## 対象論文` を書かない。
fn build_task_body(description: &str, papers: &[PaperInfo], reason: Option<&str>) -> String {
    let mut body = String::new();
    body.push_str("# task_text\n\n");
    if let Some(reason) = reason {
        body.push_str(&format!("推薦の理由: {reason}\n\n"));
    }
    body.push_str(description);
    body.push('\n');

    if !papers.is_empty() {
        body.push_str("\n## 対象論文\n\n");
        for paper in papers {
            body.push_str(&format!("### {}\n\n", paper.title));
            if !paper.authors.is_empty() {
                body.push_str(&format!("**著者**: {}\n\n", paper.authors));
            }
            if let Some(year) = &paper.year {
                body.push_str(&format!("**年**: {}\n\n", year));
            }
            if let Some(journal) = &paper.journal {
                body.push_str(&format!("**ジャーナル**: {}\n\n", journal));
            }
            if let Some(doi) = &paper.doi {
                body.push_str(&format!("**DOI**: {}\n\n", doi));
            }
            if let Some(arxiv_id) = &paper.arxiv_id {
                body.push_str(&format!("**arXiv ID**: {}\n\n", arxiv_id));
            }
            body.push_str(&format!("**papers.id**: {}\n\n", paper.id));
            if let Some(pdf) = &paper.pdf_path {
                body.push_str(&format!("**pdf_path**: {}\n\n", pdf));
            }
            if let Some(extracts) = &paper.extracts_path {
                if let Some(path_str) = extracts.to_str() {
                    body.push_str(&format!("**md_path**: {}\n\n", path_str));
                }
            }
            if let Some(translations) = &paper.translations_path {
                if let Some(path_str) = translations.to_str() {
                    body.push_str(&format!("**pdf_ja_path**: {}\n\n", path_str));
                }
            }
            if let Some(note) = &paper.note_path {
                if let Some(path_str) = note.to_str() {
                    body.push_str(&format!("**note_path**: {}\n\n", path_str));
                }
            }
        }
    }

    body
}

/// task.md の frontmatter を構築する。
///
/// sai-link の `render_card`（`sai-link-engine/src/kanban.rs`）と同じキーと順で書く。
/// `origin_run_id` は `origin_run` があればその番号、無ければ `-`。
/// 説明文は最初の行のみを frontmatter に書く。
fn build_frontmatter(
    run_id: i64,
    block_id: &str,
    global_round: i64,
    tag: &str,
    runner: &str,
    description: &str,
    origin_run: Option<i64>,
) -> Result<String> {
    let creation_date = chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string();
    // 説明文は最初の行のみを frontmatter に書く
    let desc_first_line = description.lines().next().unwrap_or("").to_string();
    let origin_run_id = origin_run
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-".to_string());

    let fm = format!(
        "---\nrunner: {}\ncreation_date: {}\nexecution_date: -\norigin_run_id: {}\nrun_id: {}\nblock: {}\nthinking: \nmodel: \nprovider: \nround: 1\nglobal_round: {}\ntag: {}\ndescription: {}",
        runner, creation_date, origin_run_id, run_id, block_id, global_round, tag, desc_first_line
    );

    Ok(fm)
}

/// 最大の global_round を計算する。
///
/// `.sai/outputs/R<n>/` ディレクトリを探して最大の `n` を取得し、`n + 1` を返す。
/// 無ければ 1 を返す。
fn calculate_global_round(roots: &PaperRoots) -> Result<i64> {
    let outputs_dir = roots.root.join(".sai").join("outputs");
    if !outputs_dir.exists() {
        return Ok(1);
    }

    let mut max_round = 0i64;
    for entry in fs::read_dir(&outputs_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Some(rest) = name_str.strip_prefix('R') {
            if let Ok(n) = rest.parse::<i64>() {
                max_round = max_round.max(n);
            }
        }
    }

    Ok(max_round + 1)
}

/// sai-link DB に run_id を予約する。
///
/// sai-link の `<project_root>/.sai/sai-engine/sai-engine.db` に
/// `execution_history` テーブルの行を INSERT して run_id を得る。
fn reserve_run_id(roots: &PaperRoots) -> Result<i64> {
    use rusqlite::Connection;

    let db_path = roots
        .root
        .join(".sai")
        .join("sai-engine")
        .join("sai-engine.db");
    if !db_path.exists() {
        return Err(HandoffError::Other(
            "sai-link DB が見つからない".to_string(),
        ));
    }

    let conn = Connection::open(&db_path)?;

    // `__todo__` ブロック ID を確保または取得
    let todo_block_id = get_or_create_todo_block(&conn)?;

    // execution_history に insert
    let user_inputs_json: Option<&str> = None;
    conn.execute(
        "INSERT INTO execution_history (block_id, status, user_inputs_json, execution_mode)
         VALUES (?1, 'todo', ?2, 'kanban')",
        rusqlite::params![todo_block_id, user_inputs_json],
    )?;

    let id = conn.last_insert_rowid();
    Ok(id)
}

/// `__todo__` プレースホルダーブロックを確保または取得する。
fn get_or_create_todo_block(conn: &rusqlite::Connection) -> Result<i64> {
    // 既存の `__todo__` を探す
    let mut stmt = conn.prepare("SELECT id FROM block_metadata WHERE name = '__todo__' LIMIT 1")?;
    if let Ok(Some(id)) = stmt.query_row([], |row| row.get::<_, i64>(0)).optional() {
        return Ok(id);
    }

    // 存在しなければ作成する
    // `file_path` は NOT NULL のため、sai-link 本体（`store.rs` の
    // `get_or_create_todo_block`）と同じく空文字を入れる。
    conn.execute(
        "INSERT INTO block_metadata (name, display_name, description, model, thinking, tools, extensions, max_turns, timeout, worktree, enable, file_path)
         VALUES ('__todo__', '未割当', 'カード予約用プレースホルダー', 'inherit', 'off', 'false', 'false', NULL, 300, 'none', 0, '')",
        [],
    )?;

    Ok(conn.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_frontmatter_valid() {
        let content = "---\nid: test\nname: Test\n---\n\nBody content";
        let (fm, body) = split_frontmatter(content).unwrap();
        assert!(fm.contains("id: test"));
        assert!(fm.contains("name: Test"));
        assert!(body.contains("Body content"));
    }

    #[test]
    fn test_get_fm_value() {
        let fm = "id: test\nname: Test Block\ndescription: A test block";
        assert_eq!(get_fm_value(fm, "id"), Some("test".to_string()));
        assert_eq!(get_fm_value(fm, "name"), Some("Test Block".to_string()));
        assert_eq!(get_fm_value(fm, "missing"), None);
    }

    #[test]
    fn test_build_frontmatter_matches_render_card_keys() {
        let fm = build_frontmatter(42, "test_block", 5, "tag1", "pi", "Test description", None)
            .unwrap();
        // sai-link の render_card と同じキーと順
        let keys: Vec<&str> = fm
            .lines()
            .filter_map(|l| l.split_once(':').map(|(k, _)| k))
            .collect();
        assert_eq!(
            keys,
            vec![
                "runner",
                "creation_date",
                "execution_date",
                "origin_run_id",
                "run_id",
                "block",
                "thinking",
                "model",
                "provider",
                "round",
                "global_round",
                "tag",
                "description",
            ]
        );
        // 廃止されたキーは書かない
        assert!(!fm.contains("inputs:"), "fm={fm}");
        assert!(!fm.contains("timeout:"), "fm={fm}");
        assert!(!fm.contains("max_turns:"), "fm={fm}");
    }

    #[test]
    fn test_build_frontmatter_writes_origin_run() {
        let fm = build_frontmatter(7, "b", 1, "推薦", "pi", "desc", Some(5)).unwrap();
        assert!(fm.contains("origin_run_id: 5"), "fm={fm}");
        let fm_default = build_frontmatter(7, "b", 1, "", "pi", "desc", None).unwrap();
        assert!(fm_default.contains("origin_run_id: -"), "fm={fm_default}");
    }

    #[test]
    fn test_build_task_body_prepends_reason_and_omits_papers() {
        let body = build_task_body("EEG の公開データを探す", &[], Some("引用が伸びている"));
        assert!(
            body.starts_with("# task_text\n\n推薦の理由: 引用が伸びている\n\n"),
            "body={body}"
        );
        assert!(body.contains("EEG の公開データを探す"));
        assert!(!body.contains("## 対象論文"));
    }

    #[test]
    fn test_parse_year_from_iso8601() {
        let published = Some("2024-09-20T12:00:00Z".to_string());
        let year = parse_year(&published);
        assert_eq!(year, Some("2024".to_string()));
    }

    #[test]
    fn test_build_frontmatter() {
        let fm =
            build_frontmatter(42, "test_block", 5, "tag1", "pi", "Test description", None)
                .unwrap();
        assert!(fm.contains("run_id: 42"));
        assert!(fm.contains("runner: pi"));
        assert!(fm.contains("creation_date:"));
        assert!(fm.contains("block: test_block"));
        assert!(fm.contains("global_round: 5"));
        assert!(fm.contains("round: 1"));
        assert!(fm.contains("tag: tag1"));
        assert!(fm.contains("description: Test description"));
    }

    #[test]
    fn test_task_md_structure() {
        let fm =
            build_frontmatter(42, "test_block", 5, "tag1", "pi", "My description", None)
                .unwrap();
        let body = String::from("# task_text\n\nTest body");
        let task_md = format!("{}\n---\n\n{}\n", fm, body);

        // frontmatter は `---` で始まり、`---` で終わる
        assert!(task_md.starts_with("---\n"));

        // ちょうど2つの `---` 行を含む
        let dashes_count = task_md.lines().filter(|l| *l == "---").count();
        assert_eq!(
            dashes_count, 2,
            "Expected exactly 2 '---' lines, found {}",
            dashes_count
        );

        // description フィールドに説明文が含まれている
        assert!(task_md.contains("description: My description"));
    }

    /// テスト用に一意な一時ディレクトリを作る。
    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-handoff-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 一時ルートに最小の sai-link DB とブロック定義を作る。
    fn setup_temp_project(root: &Path, block_id: &str) {
        fs::create_dir_all(root.join(".sai").join("blocks")).unwrap();
        fs::create_dir_all(root.join(".sai").join("paper")).unwrap();
        fs::create_dir_all(root.join(".sai").join("sai-engine")).unwrap();
        fs::write(
            root.join(".sai")
                .join("blocks")
                .join(format!("{block_id}.md")),
            format!("---\nid: {block_id}\nname: Test\ndescription: test block\n---\n\n# task_text\n"),
        )
        .unwrap();
        let conn = rusqlite::Connection::open(
            root.join(".sai").join("sai-engine").join("sai-engine.db"),
        )
        .unwrap();
        conn.execute_batch(
            "CREATE TABLE block_metadata (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, display_name TEXT, description TEXT, model TEXT, thinking TEXT, tools TEXT, extensions TEXT, max_turns INTEGER, timeout INTEGER, worktree TEXT, enable INTEGER, file_path TEXT NOT NULL);\n             CREATE TABLE execution_history (id INTEGER PRIMARY KEY AUTOINCREMENT, block_id INTEGER, status TEXT, user_inputs_json TEXT, execution_mode TEXT);",
        )
        .unwrap();
    }

    fn roots_for(root: &Path) -> PaperRoots {
        PaperRoots {
            root: root.to_path_buf(),
            data_dir: root.join(".sai").join("paper"),
        }
    }

    /// 論文が無くてもカードが作れ、`origin_run_id` と推薦の理由が書かれること。
    #[test]
    fn test_create_handoff_without_papers() {
        let root = temp_root("no-papers");
        setup_temp_project(&root, "paper-search");
        let roots = roots_for(&root);
        let db = Database::in_memory().unwrap();

        let result = create_handoff_with(
            &db,
            &roots,
            &HandoffRequest {
                block_id: "paper-search",
                paper_ids: &[],
                description: "EEG の公開データを探す",
                tag: Some("推薦"),
                runner: "pi",
                origin_run: Some(5),
                reason: Some("引用が伸びている"),
            },
        )
        .unwrap();

        let content = fs::read_to_string(&result.path).unwrap();
        assert!(content.contains("origin_run_id: 5"), "content={content}");
        assert!(content.contains("tag: 推薦"), "content={content}");
        assert!(content.contains("推薦の理由: 引用が伸びている"), "content={content}");
        assert!(content.contains("# task_text"));
        assert!(content.contains("EEG の公開データを探す"));
        assert!(!content.contains("## 対象論文"), "content={content}");
        assert!(!content.contains("inputs:"), "content={content}");
        assert!(!content.contains("timeout:"), "content={content}");
        assert!(!content.contains("max_turns:"), "content={content}");

        fs::remove_dir_all(root).unwrap();
    }

    /// `list_todo_cards` がタグで絞ること、`todo` 以外のフォルダを見ないこと。
    #[test]
    fn test_list_todo_cards_filters_tag_and_folder() {
        let root = temp_root("list-cards");
        let roots = roots_for(&root);
        let card = |run_id: i64, tag: &str| {
            format!(
                "---\nrunner: pi\ncreation_date: 2026/09/25 10:00:00\nexecution_date: -\norigin_run_id: 3\nrun_id: {run_id}\nblock: paper-search\nthinking: \nmodel: \nprovider: \nround: 1\nglobal_round: 1\ntag: {tag}\ndescription: desc {run_id}\n---\n\n# task_text\n"
            )
        };
        for (run_id, tag) in [(1i64, "推薦"), (2, "その他")] {
            let dir = root.join(".sai/outputs/todo").join(run_id.to_string());
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("task.md"), card(run_id, tag)).unwrap();
        }
        // todo 以外のフォルダは見ない
        let running = root.join(".sai/outputs/running/3");
        fs::create_dir_all(&running).unwrap();
        fs::write(running.join("task.md"), card(3, "推薦")).unwrap();

        let all = list_todo_cards(&roots, None).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].run_id, 1);
        assert_eq!(all[1].run_id, 2);

        let recommend = list_todo_cards(&roots, Some("推薦")).unwrap();
        assert_eq!(recommend.len(), 1);
        assert_eq!(recommend[0].run_id, 1);
        assert_eq!(recommend[0].origin_run_id, Some(3));
        assert_eq!(recommend[0].block, "paper-search");
        assert_eq!(recommend[0].description, "desc 1");

        fs::remove_dir_all(root).unwrap();
    }
}
