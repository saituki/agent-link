//! task.md 発行：論文をエージェントに引き渡す。
//!
//! sai-paper から選択した論文を sai-link のカンバンボードに task.md 作成して
//! 引き渡す。カードは `sai-link card create`（標準入力に JSON）で作り、
//! run_id はその出力から得る。sai-link の DB は直に開かない。

use sai_paper_core::database::Database;
use sai_paper_core::paths::PaperRoots;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::bins::BinError;
use crate::link;

/// task.md 発行操作のエラー。
#[derive(Debug, Error)]
pub enum HandoffError {
    /// ファイル I/O 失敗
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// DB 操作失敗
    #[error("database error: {0}")]
    Database(#[from] sai_paper_core::database::DatabaseError),
    /// SQLite 操作失敗
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// JSON エラー
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// ブロック未検出
    #[error("block not found: {0}")]
    BlockNotFound(String),
    /// sai-link の CLI の失敗（終了コード付き）
    #[error("sai-link に失敗しました（終了コード {}）: {}", .0.code, .0.message)]
    Link(BinError),
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
/// `<root>/.sai/blocks/*.md` と `~/.sai-moon/blocks/*.md` を読み、
/// frontmatter から id / name / description を抜き出す。
/// ローカル（`<root>/.sai/blocks/`）の方がグローバル（`~/.sai-moon/blocks/`）より優先。
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

    // グローバルブロック（`~/.sai-moon/blocks/`）を先に読む
    let global_blocks_dir = sai_paper_core::paths::sai_home_path("blocks").filter(|p: &PathBuf| p.exists());

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

/// `lane` が `todo` の run の `task.md` のカードを一覧する。
///
/// run の一覧は `sai-link run list --json` から取り、`dir` の下の
/// `task.md` を読む。`tag` を指定すると frontmatter の `tag` が一致するカードだけを返す。
/// 返り値は `run_id` の昇順。sai-link の一覧を読めないときは空を返す。
///
/// # Errors
///
/// カードの読み込みに失敗したとき。
pub fn list_todo_cards(roots: &PaperRoots, tag: Option<&str>) -> Result<Vec<TodoCard>> {
    let Ok(runs) = link::runs(&roots.root) else {
        return Ok(Vec::new());
    };

    let mut cards = Vec::new();
    for run in runs {
        if run.lane != "todo" {
            continue;
        }
        let Some(dir) = run.dir else {
            continue;
        };
        let task_path = dir.join("task.md");
        let Ok(content) = fs::read_to_string(&task_path) else {
            continue;
        };
        // frontmatter が無い / 壊れているカードは飛ばす。
        let Ok((frontmatter, _)) = split_frontmatter(&content) else {
            continue;
        };
        let card_tag = get_fm_value(&frontmatter, "tag").unwrap_or_default();
        if let Some(want) = tag
            && card_tag != want
        {
            continue;
        }
        cards.push(TodoCard {
            run_id: run.run_id,
            block: get_fm_value(&frontmatter, "block").unwrap_or_default(),
            tag: card_tag,
            description: get_fm_value(&frontmatter, "description").unwrap_or_default(),
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
    /// 推薦した実行の run_id（任意。sai-sci の結び付けの actor に使う。task.md には書かない）
    pub origin_run: Option<i64>,
    /// 推薦の理由（任意。あれば本文の先頭に書く）
    pub reason: Option<&'a str>,
}

/// task.md を発行する。
///
/// # Errors
///
/// DB エラー、ファイル I/O エラー、ブロック未検出等。
pub fn create_handoff_with(
    db: &Database,
    roots: &PaperRoots,
    request: &HandoffRequest<'_>,
) -> Result<HandoffResult> {
    create_handoff_with_extra(db, roots, request, &[])
}

/// task.md を発行する（frontmatter の最後に `extra` のキーを足す）。
///
/// 処理:
/// 1. ブロックの存在を確認
/// 2. 論文情報を集める
/// 3. `sai-link card create` でカードを作る（本文と `extra` を渡す）
/// 4. { run_id, path } を返す
///
/// `extra` は `(キー, 値)` の並び。値の改行は空白に置き換え、空の値は書かない。
/// `executer` は `card create` の起動者に、それ以外は `meta` に入れる。
/// 並びは呼び出し側が決める（sai-sci のカードは `sci_node` から `sci_depth` までをこの順に足す）。
///
/// # Errors
///
/// sai-link の CLI の失敗、ファイル I/O エラー、ブロック未検出等。
pub fn create_handoff_with_extra(
    db: &Database,
    roots: &PaperRoots,
    request: &HandoffRequest<'_>,
    extra: &[(&str, String)],
) -> Result<HandoffResult> {
    let block_id = request.block_id;
    // `origin_run` はカードには書かない（結び付けは呼び出し側が別に記録する）。
    let _ = request.origin_run;

    // 1. ブロック情報を検索
    let blocks = list_blocks(roots)?;
    blocks
        .iter()
        .find(|b| b.id == block_id)
        .ok_or_else(|| HandoffError::BlockNotFound(block_id.to_string()))?;

    // 2. 次の global_round を求める（`R<n>` の最大 + 1）。
    let runs = link::runs(&roots.root).map_err(HandoffError::Link)?;
    let global_round = link::next_global_round(&runs);

    // 3. 論文情報を集める
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

    // 4. 本文を作る（`sai-link card create` が `# task_text` の見出しを付けるので、
    //    先頭の見出しは外して task_text にする）。
    let body = build_task_body(request.description, &paper_infos, request.reason);
    let task_text = body
        .strip_prefix("# task_text\n\n")
        .unwrap_or(&body)
        .to_string();

    // 5. `extra` を起動者と meta に分ける。
    let mut executer = String::new();
    let mut meta: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in extra {
        let value = one_line(value);
        if value.is_empty() {
            continue;
        }
        if *key == "executer" {
            executer = value;
        } else {
            meta.insert((*key).to_string(), value);
        }
    }

    // 6. `sai-link card create` でカードを作る。
    let card = link::CardCreate {
        runner: request.runner.to_string(),
        block: block_id.to_string(),
        global_round: global_round.to_string(),
        round: "1".to_string(),
        tag: request.tag.unwrap_or("").to_string(),
        description: request.description.lines().next().unwrap_or("").to_string(),
        task_text,
        executer,
        meta,
    };
    let (run_id, path) = link::card_create(&roots.root, &card).map_err(HandoffError::Link)?;

    Ok(HandoffResult { run_id, path })
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
/// （[`sai_paper_core::paths::paper_folder`]）から推定する。
fn resolve_paper_folder(
    roots: &PaperRoots,
    paper: &sai_paper_core::models::LibraryPaper,
) -> Option<PathBuf> {
    if let Some(pdf) = paper.pdf_path.as_deref() {
        if let Some(parent) = Path::new(pdf).parent() {
            if parent.is_dir() {
                return Some(parent.to_path_buf());
            }
        }
    }
    let candidate = sai_paper_core::paths::paper_folder(&roots.data_dir, paper.id, &paper.title);
    candidate.is_dir().then_some(candidate)
}

/// 論文の抽出 Markdown パスを解決する（記録 → 論文フォルダ → 旧レイアウト）。
///
/// 引用抽出の記録（`paper_extracts`）を、まず `papers_id` で、無ければ抽出キーで
/// 探す（core 共通の [`sai_paper_core::citation::resolve_extract_md`]。段階 2c）。
fn get_paper_extracts_path(
    db: &Database,
    roots: &PaperRoots,
    paper: &sai_paper_core::models::LibraryPaper,
) -> Option<PathBuf> {
    if let Some(path) =
        sai_paper_core::citation::resolve_extract_md(db.connection(), Some(paper.id), None, &roots.data_dir)
    {
        return Some(path);
    }
    if let Some(folder) = resolve_paper_folder(roots, paper) {
        let candidate = folder.join(sai_paper_core::paths::paper_file_names::EXTRACT);
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
    paper: &sai_paper_core::models::LibraryPaper,
) -> Option<PathBuf> {
    if let Some(folder) = resolve_paper_folder(roots, paper) {
        let candidate = folder.join(sai_paper_core::paths::paper_file_names::PDF_JA);
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
fn get_paper_note_path(roots: &PaperRoots, paper: &sai_paper_core::models::LibraryPaper) -> Option<PathBuf> {
    if let Some(folder) = resolve_paper_folder(roots, paper) {
        let candidate = folder.join(sai_paper_core::paths::paper_file_names::NOTE);
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

/// 値の改行を空白に置き換えて 1 行にする。
fn one_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ").trim().to_string()
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

    /// 一時ルートにブロック定義を作る（sai-link の DB は作らない）。
    fn setup_temp_project(root: &Path, block_id: &str) {
        fs::create_dir_all(root.join(".sai").join("blocks")).unwrap();
        fs::create_dir_all(root.join(".sai").join("paper")).unwrap();
        fs::write(
            root.join(".sai")
                .join("blocks")
                .join(format!("{block_id}.md")),
            format!("---\nid: {block_id}\nname: Test\ndescription: test block\n---\n\n# task_text\n"),
        )
        .unwrap();
    }

    fn roots_for(root: &Path) -> PaperRoots {
        PaperRoots {
            root: root.to_path_buf(),
            data_dir: root.join(".sai").join("paper"),
        }
    }

    fn request<'a>() -> HandoffRequest<'a> {
        HandoffRequest {
            block_id: "paper-search",
            paper_ids: &[],
            description: "EEG の公開データを探す",
            tag: Some("推薦"),
            runner: "pi",
            origin_run: Some(5),
            reason: Some("引用が伸びている"),
        }
    }

    /// カードの項目を `sai-link card create` に渡し、task.md もできること。
    #[test]
    fn test_create_handoff_passes_card_fields() {
        let root = temp_root("no-papers");
        setup_temp_project(&root, "paper-search");
        let roots = roots_for(&root);
        let db = Database::in_memory().unwrap();

        let result = create_handoff_with_extra(
            &db,
            &roots,
            &request(),
            &[
                ("executer", "agent".to_string()),
                ("sci_node", "Q1".to_string()),
                ("sci_mode", String::new()),
                ("sci_expect", "比較表と\n推奨".to_string()),
            ],
        )
        .unwrap();

        assert_eq!(result.run_id, 1);
        let card = crate::fake_link::last_card(&root).expect("card create に JSON を渡した");
        assert_eq!(card["block"], "paper-search");
        assert_eq!(card["tag"], "推薦");
        assert_eq!(card["runner"], "pi");
        assert_eq!(card["round"], "1");
        assert_eq!(card["global_round"], "1");
        assert_eq!(card["executer"], "agent");
        assert_eq!(card["description"], "EEG の公開データを探す");
        assert_eq!(card["meta"]["sci_node"], "Q1");
        assert_eq!(card["meta"]["sci_expect"], "比較表と 推奨");
        assert!(card["meta"].get("sci_mode").is_none(), "card={card}");
        let text = card["task_text"].as_str().unwrap();
        assert!(text.contains("推薦の理由: 引用が伸びている"), "text={text}");
        assert!(text.contains("EEG の公開データを探す"), "text={text}");
        assert!(!text.contains("# task_text"), "text={text}");
        assert!(!text.contains("## 対象論文"), "text={text}");

        let content = fs::read_to_string(&result.path).unwrap();
        assert!(content.contains("# task_text"), "content={content}");
        assert!(content.contains("executer: agent"), "content={content}");

        fs::remove_dir_all(root).unwrap();
    }

    /// 子が終了コード 3 で失敗したとき、その番号がエラーに残ること。
    #[test]
    fn link_failure_keeps_exit_code() {
        let root = temp_root("fail");
        setup_temp_project(&root, "paper-search");
        let roots = roots_for(&root);
        let db = Database::in_memory().unwrap();
        crate::fake_link::fail_card_create(&root, 3);

        let error = create_handoff_with(&db, &roots, &request()).unwrap_err();
        match error {
            HandoffError::Link(bin) => assert_eq!(bin.code, 3),
            other => panic!("Link でない: {other:?}"),
        }

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
        assert_eq!(recommend[0].block, "paper-search");
        assert_eq!(recommend[0].description, "desc 1");

        fs::remove_dir_all(root).unwrap();
    }
}
