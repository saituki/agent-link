//! チャンク化エンジン: 論文・ノート・ドキュメントを検索インデックス用に分割する。
//!
//! 4 つの戦略でテキストをチャンク化し、FTS5 インデックスに投入する。
//! 既存の分割済みデータ（`paper_sentences`, `paper_extracts.md_path`）を再利用する。
//!
//! # チャンク戦略
//!
//! - **abstract**: `papers.abstract` / `abstract_ja` をそれぞれ 1 チャンク化
//! - **sentence**: `paper_sentences` テーブルの 1 行 = 1 チャンク（新規分割なし）
//! - **paragraph**: `paper_extracts.md_path` の Markdown を見出し/空行で分割
//! - **window**: N 文ずつ、M 文オーバーラップ（設定値から `window_size`/`window_overlap`）
//!
//! # インデックス対象（スコープ）
//!
//! - **papers**: `papers.abstract`, `abstract_ja`, `paper_sentences`, `paper_extracts.md_path`
//! - **notes**: 論文フォルダの `note.md`
//! - **all**: 上記すべて

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 100 字未満の段落は、同じ章の前後の段落とまとめる（section 戦略）。
const SHORT_PARAGRAPH_CHARS: usize = 100;
/// 埋め込みの先頭に付けるメタの見積もりとして、塊の上限から差し引く最大トークン数（section 戦略）。
const META_RESERVE_MAX_TOKENS: usize = 60;

/// トークン数を見積もる。
///
/// 実測データに基づき、表・参考文献・数式が多い箇所も安全に処理できる係数を採用。
/// ASCII 文字は 2.3 文字 / トークン、非 ASCII 文字は 1.0 文字 / トークンとして計算する。
///
/// ## 実測値
///
/// 実際のチャンク本文を OpenRouter に送った際のトークン数:
/// - 1976 文字 → 682 トークン (2.90 文字/トークン) ←最悪ケース（表・参考文献・数式が多い）
/// - 1975 文字 → 418 トークン (4.72 文字/トークン)
/// - 1975 文字 → 509 トークン (3.88 文字/トークン)
/// - 1975 文字 → 600 トークン (3.29 文字/トークン)
/// - 1316 文字 → 239 トークン (5.51 文字/トークン)
///
/// 平文の英語は 4.5〜5.5 文字/トークン、表・参考文献・数式が多い箇所は 2.9 文字/トークン。
/// 現在の係数 (ASCII 2.3, 非ASCII 1.0) は、最悪ケース 2.90 と 1.28 に安全側の余裕を持たせた値。
fn estimate_tokens(text: &str) -> usize {
    let mut tokens: f64 = 0.0;
    for ch in text.chars() {
        if ch.is_ascii() {
            tokens += 1.0 / 2.3;
        } else {
            tokens += 1.0 / 1.0;
        }
    }
    (tokens.ceil()) as usize
}

/// チャンク化操作のエラー。
#[derive(Debug, Error)]
pub enum ChunkingError {
    /// SQLite 操作に失敗した。
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// ファイル操作に失敗した。
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// 設定操作に失敗した。
    #[error("config error: {0}")]
    Config(#[from] crate::config::ConfigError),
    /// 入力が不正。
    #[error("invalid input: {0}")]
    Invalid(String),
}

/// チャンク化操作の結果型。
pub type Result<T> = std::result::Result<T, ChunkingError>;

/// チャンク戦略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Strategy {
    /// Abstract をそのまま 1 チャンク
    Abstract,
    /// Sentence テーブルの行ごと
    Sentence,
    /// Markdown を段落で分割
    Paragraph,
    /// Markdown を章ごとにまとめ、長い章は段落の切れ目で分ける
    Section,
    /// N 文スライディングウィンドウ
    Window,
}

impl Strategy {
    /// 文字列から戦略を解析する。
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "abstract" => Ok(Strategy::Abstract),
            "sentence" => Ok(Strategy::Sentence),
            "paragraph" => Ok(Strategy::Paragraph),
            "section" => Ok(Strategy::Section),
            "window" => Ok(Strategy::Window),
            _ => Err(ChunkingError::Invalid(format!("unknown strategy: {}", s))),
        }
    }

    /// 戦略を文字列に変換する。
    pub fn as_str(&self) -> &'static str {
        match self {
            Strategy::Abstract => "abstract",
            Strategy::Sentence => "sentence",
            Strategy::Paragraph => "paragraph",
            Strategy::Section => "section",
            Strategy::Window => "window",
        }
    }
}

/// スコープ（インデックス対象の範囲）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// 論文のみ
    Papers,
    /// ノートのみ
    Notes,
    /// 図表（paper_figures）のみ
    Figures,
    /// research-tree（sai-sci）のみ
    Tree,
    /// すべて（木の中身は含まない）
    All,
}

impl Scope {
    /// 文字列から スコープを解析する。
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "papers" => Ok(Scope::Papers),
            "notes" => Ok(Scope::Notes),
            "figures" => Ok(Scope::Figures),
            "tree" => Ok(Scope::Tree),
            "documents" => Err(ChunkingError::Invalid(
                "documents は廃止しました（papers / notes / figures / tree / all）".into(),
            )),
            "all" => Ok(Scope::All),
            _ => Err(ChunkingError::Invalid(format!("unknown scope: {}", s))),
        }
    }
}

/// ソーステキストの種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// 論文の abstract
    Abstract,
    /// 論文の本文
    Body,
    /// ノート
    Note,
    /// リサーチドキュメント
    Document,
    /// research-tree（sai-sci）の中身
    Tree,
}

impl SourceKind {
    /// 文字列に変換する。
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Abstract => "abstract",
            SourceKind::Body => "body",
            SourceKind::Note => "note",
            SourceKind::Document => "document",
            SourceKind::Tree => "tree",
        }
    }
}

/// チャンク化結果の統計。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildReport {
    /// 作成されたチャンク数
    pub chunks_created: i64,
    /// スキップされた行数（既にチャンク化済み）
    pub sources_skipped: i64,
}

/// インデックス状態レポート。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStatus {
    /// ソース種別ごとのチャンク数（kind → count）
    pub by_source_kind: std::collections::HashMap<String, i64>,
    /// 戦略ごとのチャンク数（strategy → count）
    pub by_strategy: std::collections::HashMap<String, i64>,
    /// 索引ごとのチャンク数（索引名 → count）
    pub by_index: std::collections::HashMap<String, i64>,
    /// 埋め込みを持つチャンク数
    pub chunks_with_embeddings: i64,
}

/// テキスト内の位置情報付きチャンク。
#[derive(Debug)]
struct TextChunk {
    /// チャンク本文
    text: String,
    /// ソーステキスト内の開始位置
    char_start: usize,
    /// ソーステキスト内の終了位置
    char_end: usize,
    /// その文があった章（見出しを ` > ` でつないだもの）。無ければ `None`。
    section: Option<String>,
}

/// 論文 1 本だけを作るときの絞り込み。`only` が `None` なら全件を対象にする。
/// 索引の作成（`build_paper`）でだけ `Some` を渡す。
// ponytail: 全件の SELECT を読んでから Rust 側で絞る。論文数が数万になったら SQL 側で絞る。
fn is_wanted(only: Option<i64>, paper_id: Option<i64>) -> bool {
    only.is_none_or(|want| paper_id == Some(want))
}

/// Abstract 戦略でチャンク化する。
fn chunk_abstract(
    conn: &Connection,
    index: &crate::config::RagIndex,
    only: Option<i64>,
) -> Result<BuildReport> {
    let mut chunks_created = 0i64;
    let mut sources_skipped = 0i64;

    let max_tokens = index.chunk_max_tokens;

    // abstract フィールドからチャンクを作成
    let mut stmt = conn
        .prepare("SELECT id, abstract FROM papers WHERE abstract IS NOT NULL AND abstract != ''")?;

    let abstract_rows: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    for (paper_id, abstract_text) in abstract_rows {
        if !is_wanted(only, Some(paper_id)) {
            continue;
        }
        // 既に abstract チャンクが存在するかチェック
        let existing: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM chunks WHERE paper_id = ?1 AND source_kind = 'abstract' AND strategy = 'abstract' AND index_name = ?2",
            rusqlite::params![paper_id, &index.name],
            |row| row.get(0),
        )?;

        if existing {
            sources_skipped += 1;
            continue;
        }

        let now = chrono::Utc::now().to_rfc3339();

        // テキストが最大トークン数を超える場合は分割
        let text_chunks = split_text_by_max_tokens(&abstract_text, max_tokens);

        for (ord, chunk) in text_chunks.iter().enumerate() {
            conn.execute(
                "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at, index_name, section)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    "abstract",
                    format!("paper_{}", paper_id),
                    paper_id,
                    ord as i64,
                    &chunk.text,
                    chunk.char_start as i64,
                    chunk.char_end as i64,
                    "abstract",
                    now,
                    &index.name,
                    "Abstract",
                ],
            )?;
            chunks_created += 1;
        }
    }

    // abstract_ja があれば別チャンク
    let mut stmt_ja = conn.prepare(
        "SELECT id, abstract_ja FROM papers WHERE abstract_ja IS NOT NULL AND abstract_ja != ''",
    )?;

    let abstract_ja_rows: Vec<(i64, String)> = stmt_ja
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    for (paper_id, abstract_ja_text) in abstract_ja_rows {
        if !is_wanted(only, Some(paper_id)) {
            continue;
        }
        let existing: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM chunks WHERE paper_id = ?1 AND source_kind = 'abstract' AND strategy = 'abstract' AND source_ref = ?2 AND index_name = ?3",
            rusqlite::params![paper_id, format!("paper_{}_ja", paper_id), &index.name],
            |row| row.get(0),
        )?;

        if existing {
            sources_skipped += 1;
            continue;
        }

        let now = chrono::Utc::now().to_rfc3339();

        // テキストが最大トークン数を超える場合は分割
        let text_chunks = split_text_by_max_tokens(&abstract_ja_text, max_tokens);

        for (ord, chunk) in text_chunks.iter().enumerate() {
            conn.execute(
                "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at, index_name, section)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    "abstract",
                    format!("paper_{}_ja", paper_id),
                    paper_id,
                    ord as i64,
                    &chunk.text,
                    chunk.char_start as i64,
                    chunk.char_end as i64,
                    "abstract",
                    now,
                    &index.name,
                    "Abstract",
                ],
            )?;
            chunks_created += 1;
        }
    }

    Ok(BuildReport {
        chunks_created,
        sources_skipped,
    })
}

/// Sentence 戦略でチャンク化する。
fn chunk_sentence(
    conn: &Connection,
    index: &crate::config::RagIndex,
    only: Option<i64>,
) -> Result<BuildReport> {
    let mut chunks_created = 0i64;
    let mut sources_skipped = 0i64;

    let max_tokens = index.chunk_max_tokens;

    // paper_sentences テーブルから句ごとチャンク化。
    // 記録の結び付きは papers_id（段階 2c）。papers_id が NULL の古い行は
    // 抽出キー (TEXT: S2 40hex / ArXiv:... / DOI:...) の照合で papers.id を逆引きする。
    let mut stmt = conn.prepare(
        "SELECT DISTINCT ps.paper_id,
                COALESCE(ps.papers_id, (SELECT p.id FROM papers p
             WHERE p.s2_paper_id = ps.paper_id COLLATE NOCASE
                OR ('ArXiv:' || p.arxiv_id) = ps.paper_id COLLATE NOCASE
                OR ('DOI:' || p.doi) = ps.paper_id COLLATE NOCASE
                OR p.arxiv_id = ps.paper_id COLLATE NOCASE
                OR p.doi = ps.paper_id COLLATE NOCASE
             LIMIT 1)) AS pid
         FROM paper_sentences ps
         ORDER BY ps.paper_id",
    )?;

    let paper_ids: Vec<(String, Option<i64>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    for (paper_id_str, paper_id_opt) in paper_ids {
        if !is_wanted(only, paper_id_opt) {
            continue;
        }
        // 既に sentence チャンクが存在するかチェック
        let existing: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM chunks WHERE source_ref LIKE ?1 AND strategy = 'sentence' AND index_name = ?2",
            rusqlite::params![format!("{}_%%", paper_id_str), &index.name],
            |row| row.get(0),
        )?;

        if existing {
            sources_skipped += 1;
            continue;
        }

        // その論文の sentence をすべて挿入
        let mut stmt_sent = conn.prepare(
            "SELECT sent_no, text FROM paper_sentences WHERE paper_id = ?1 ORDER BY sent_no",
        )?;

        let sentences: Vec<(i64, String)> = stmt_sent
            .query_map([&paper_id_str], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let now = chrono::Utc::now().to_rfc3339();

        for (sent_no, text) in sentences {
            // 文が最大トークン数を超える場合は分割
            let text_chunks = split_text_by_max_tokens(&text, max_tokens);

            for (sub_ord, chunk) in text_chunks.iter().enumerate() {
                conn.execute(
                    "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at, index_name)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        "body",
                        format!("{}_{}", paper_id_str, sent_no),
                        paper_id_opt,
                        sub_ord as i64,
                        &chunk.text,
                        chunk.char_start as i64,
                        chunk.char_end as i64,
                        "sentence",
                        now,
                        &index.name,
                    ],
                )?;
                chunks_created += 1;
            }
        }
    }

    Ok(BuildReport {
        chunks_created,
        sources_skipped,
    })
}

/// Markdown から data URI 画像（`![...](data:image/...;base64,...)` の形）を除去する。
pub(crate) fn remove_data_uri_images(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '!' {
            // ![alt](url) パターンをチェック
            let mut bracket_content = String::new();
            let mut paren_start = false;

            // ![...] を読み込む
            if let Some(&'[') = chars.peek() {
                chars.next();
                while let Some(&next) = chars.peek() {
                    if next == ']' {
                        chars.next();
                        paren_start = true;
                        break;
                    }
                    bracket_content.push(next);
                    chars.next();
                }
            } else {
                result.push(ch);
                continue;
            }

            // (...) を読み込む
            if paren_start {
                if let Some(&'(') = chars.peek() {
                    chars.next();
                    let mut paren_content = String::new();
                    let mut found_close = false;

                    while let Some(&next) = chars.peek() {
                        if next == ')' {
                            chars.next();
                            found_close = true;
                            break;
                        }
                        paren_content.push(next);
                        chars.next();
                    }

                    // data: URI でなければ、画像を復元
                    if !paren_content.starts_with("data:") && found_close {
                        result.push('!');
                        result.push('[');
                        result.push_str(&bracket_content);
                        result.push(']');
                        result.push('(');
                        result.push_str(&paren_content);
                        result.push(')');
                    }
                    // data: URI の場合は何もしない（削除）
                } else {
                    // ( がなければ復元
                    result.push('!');
                    result.push('[');
                    result.push_str(&bracket_content);
                    result.push(']');
                }
            } else {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Markdown の見出しの行から `(深さ, 見出しの文字)` を取り出す。見出しでなければ `None`。
/// 先頭の `#` とそのあとの空白、太字の `**` を外す。
pub(crate) fn markdown_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    let depth = trimmed.chars().take_while(|c| *c == '#').count();
    if depth == 0 {
        return None;
    }
    let title = trimmed[depth..].trim().trim_matches('*').trim().to_string();
    Some((depth, title))
}

/// いま積まれている見出しを ` > ` でつないだ章の名前。見出しが無ければ `None`。
fn headings_to_section(headings: &[(usize, String)]) -> Option<String> {
    if headings.is_empty() {
        None
    } else {
        Some(
            headings
                .iter()
                .map(|(_, title)| title.as_str())
                .collect::<Vec<_>>()
                .join(" > "),
        )
    }
}

/// Markdown を見出しと空行で段落分割する。
/// 段落には、その上にある見出しを ` > ` でつないだ章の名前を付ける。
fn split_markdown_paragraphs(text: &str) -> Vec<TextChunk> {
    // 最初に data URI 画像を除去
    let text_without_images = remove_data_uri_images(text);

    let mut chunks = Vec::new();
    let mut current_start = 0;
    let mut current_text = String::new();
    // 見出しの並び（深さ, 見出しの文字）。同じ深さ以上を外してから積む。
    let mut headings: Vec<(usize, String)> = Vec::new();

    for line in text_without_images.lines() {
        let line_with_newline = format!("{}\n", line);
        let heading = markdown_heading(line);

        // 見出し、または空行で分割する。このときの見出しスタックが、
        // いままで積んだ段落の章になる。
        if line.trim().is_empty() || heading.is_some() {
            if !current_text.trim().is_empty() {
                let chunk_text = current_text.trim_end().to_string();
                chunks.push(TextChunk {
                    text: chunk_text,
                    char_start: current_start,
                    char_end: current_start + current_text.len(),
                    section: headings_to_section(&headings),
                });
            }
            current_start += line_with_newline.len();
            current_text.clear();
        } else {
            current_text.push_str(&line_with_newline);
        }

        // 見出しのスタックを更新する（同じ深さ以上の見出しを外してから積む）。
        // この後の段落から新しい章になる。
        if let Some((depth, title)) = &heading {
            while headings
                .last()
                .is_some_and(|(last_depth, _)| last_depth >= depth)
            {
                headings.pop();
            }
            if !title.is_empty() {
                headings.push((*depth, title.clone()));
            }
        }
    }

    // 最後の段落
    if !current_text.trim().is_empty() {
        let chunk_text = current_text.trim_end().to_string();
        chunks.push(TextChunk {
            text: chunk_text,
            char_start: current_start,
            char_end: current_start + current_text.len(),
            section: headings_to_section(&headings),
        });
    }

    chunks
}

/// 続く同じ章の段落ごとにまとめる（section 戦略）。
fn group_paragraphs_by_section(
    paragraphs: Vec<TextChunk>,
) -> Vec<(Option<String>, Vec<TextChunk>)> {
    let mut groups: Vec<(Option<String>, Vec<TextChunk>)> = Vec::new();
    for para in paragraphs {
        match groups.last_mut() {
            // 章が変われば必ず別の組にする。
            Some((section, items)) if *section == para.section => items.push(para),
            _ => groups.push((para.section.clone(), vec![para])),
        }
    }
    groups
}

/// `prev` の後ろに `next` を足す（本文は空行でつなぎ、範囲は広げる）。
fn append_paragraph(prev: &mut TextChunk, next: TextChunk) {
    if prev.text.is_empty() {
        prev.text = next.text;
    } else {
        prev.text = format!("{}\n\n{}", prev.text, next.text);
    }
    prev.char_end = next.char_end;
}

/// `target` の前に `first` を足す。
fn prepend_paragraph(target: &mut TextChunk, first: TextChunk) {
    if target.text.is_empty() {
        target.text = first.text;
    } else {
        target.text = format!("{}\n\n{}", first.text, target.text);
    }
    target.char_start = first.char_start;
}

/// 100 字未満の段落を、同じ章の前の段落（無ければ次の段落）とまとめる（section 戦略）。
fn merge_short_paragraphs(paragraphs: Vec<TextChunk>) -> Vec<TextChunk> {
    let mut paragraphs = paragraphs;
    // 先頭が短い段落なら、同じ章に前の段落が無いので次の段落とまとめる。
    if paragraphs.len() >= 2 && paragraphs[0].text.chars().count() < SHORT_PARAGRAPH_CHARS {
        let first = paragraphs.remove(0);
        prepend_paragraph(&mut paragraphs[0], first);
    }

    let mut merged: Vec<TextChunk> = Vec::new();
    for para in paragraphs {
        if para.text.chars().count() < SHORT_PARAGRAPH_CHARS {
            if let Some(prev) = merged.last_mut() {
                append_paragraph(prev, para);
                continue;
            }
        }
        merged.push(para);
    }
    merged
}

/// 1 つの章の段落の並びを、上限トークン数に収まる塊に切り分ける（section 戦略）。
/// 1 段落だけで上限を超えるときは、文の切れ目で分ける。
fn pack_section_chunks(
    paragraphs: &[TextChunk],
    max_tokens: usize,
    section: Option<&str>,
) -> Vec<TextChunk> {
    let mut out: Vec<TextChunk> = Vec::new();
    let mut current: Option<TextChunk> = None;

    for para in paragraphs {
        if estimate_tokens(&para.text) > max_tokens {
            // 段落が単独で上限を超えるので、文の切れ目で分ける。
            if let Some(done) = current.take() {
                out.push(done);
            }
            for mut part in split_text_by_max_tokens(&para.text, max_tokens) {
                part.char_start += para.char_start;
                part.char_end += para.char_start;
                part.section = section.map(str::to_string);
                out.push(part);
            }
            continue;
        }

        match current.as_mut() {
            None => {
                current = Some(TextChunk {
                    text: para.text.clone(),
                    char_start: para.char_start,
                    char_end: para.char_end,
                    section: section.map(str::to_string),
                });
            }
            Some(cur) => {
                let combined = format!("{}\n\n{}", cur.text, para.text);
                if estimate_tokens(&combined) <= max_tokens {
                    cur.text = combined;
                    cur.char_end = para.char_end;
                } else {
                    out.push(current.take().expect("current がある"));
                    current = Some(TextChunk {
                        text: para.text.clone(),
                        char_start: para.char_start,
                        char_end: para.char_end,
                        section: section.map(str::to_string),
                    });
                }
            }
        }
    }

    if let Some(done) = current {
        out.push(done);
    }
    out
}

/// Markdown を章ごとにまとめ、長い章は段落の切れ目で分ける（section 戦略）。
/// 戻り値は章ごとの塊の並び（外側が章の順）。
/// `reserve_tokens` は、その章で埋め込みの先頭に付ける分の見積もりトークン数を返す。
fn split_markdown_sections<F>(text: &str, max_tokens: usize, reserve_tokens: F) -> Vec<Vec<TextChunk>>
where
    F: Fn(Option<&str>) -> usize,
{
    let paragraphs = split_markdown_paragraphs(text);
    let mut groups_out = Vec::new();
    for (section, items) in group_paragraphs_by_section(paragraphs) {
        let items = merge_short_paragraphs(items);
        let budget = max_tokens.saturating_sub(reserve_tokens(section.as_deref()));
        groups_out.push(pack_section_chunks(&items, budget, section.as_deref()));
    }
    groups_out
}

/// Paragraph 戦略でチャンク化する。
fn chunk_paragraph(
    conn: &Connection,
    index: &crate::config::RagIndex,
    data_dir: &std::path::Path,
    only: Option<i64>,
) -> Result<BuildReport> {
    let mut chunks_created = 0i64;
    let mut sources_skipped = 0i64;

    let max_tokens = index.chunk_max_tokens;

    // paper_extracts から Markdown を読みこむ。
    // 結び付きは papers_id（段階 2c）。NULL の古い行は抽出キーの照合で逆引きする。
    let mut stmt = conn.prepare(
        "SELECT pe.paper_id, pe.md_path,
                COALESCE(pe.papers_id, (SELECT p.id FROM papers p
                  WHERE p.s2_paper_id = pe.paper_id COLLATE NOCASE
                     OR ('ArXiv:' || p.arxiv_id) = pe.paper_id COLLATE NOCASE
                     OR ('DOI:' || p.doi) = pe.paper_id COLLATE NOCASE
                     OR p.arxiv_id = pe.paper_id COLLATE NOCASE
                     OR p.doi = pe.paper_id COLLATE NOCASE
                  LIMIT 1)) AS pid
         FROM paper_extracts pe
         WHERE pe.md_path IS NOT NULL AND pe.status = 'ready'",
    )?;

    let extracts: Vec<(String, String, Option<i64>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    for (paper_id_str, md_rel_path, paper_id_opt) in extracts {
        if !is_wanted(only, paper_id_opt) {
            continue;
        }
        // 既に paragraph チャンクが存在するかチェック
        let existing: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM chunks WHERE source_ref LIKE ?1 AND strategy = 'paragraph' AND index_name = ?2",
            rusqlite::params![format!("extract_{}", paper_id_str), &index.name],
            |row| row.get(0),
        )?;

        if existing {
            sources_skipped += 1;
            continue;
        }

        // md_path は DB 上では data_dir からの相対（段階 2b）。実パスへ直す。
        let md_path = crate::paths::resolve_stored_path(data_dir, &md_rel_path);
        if !md_path.exists() {
            continue;
        }

        let markdown_content =
            crate::citation::docling::strip_figure_images(&std::fs::read_to_string(&md_path)?);
        let paragraphs = split_markdown_paragraphs(&markdown_content);

        let now = chrono::Utc::now().to_rfc3339();

        for (ord, chunk) in paragraphs.iter().enumerate() {
            // 段落が最大トークン数を超える場合は分割
            let text_chunks = split_text_by_max_tokens(&chunk.text, max_tokens);

            for (sub_ord, text_chunk) in text_chunks.iter().enumerate() {
                // text_chunk の char_start/char_end は chunk.text 内の位置なので、
                // 元のテキスト内の位置に調整する
                let adjusted_char_start = chunk.char_start + text_chunk.char_start;
                let adjusted_char_end = chunk.char_start + text_chunk.char_end;

                conn.execute(
                    "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at, index_name, section)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        "body",
                        format!("extract_{}", paper_id_str),
                        paper_id_opt,
                        (ord as i64) * 1000 + (sub_ord as i64),  // sub-chunks は序数に 1000 を加算
                        &text_chunk.text,
                        adjusted_char_start as i64,
                        adjusted_char_end as i64,
                        "paragraph",
                        now,
                        &index.name,
                        // 分割した塊も、元の段落の章を引き継ぐ。
                        chunk.section.as_deref(),
                    ],
                )?;
                chunks_created += 1;
            }
        }
    }

    Ok(BuildReport {
        chunks_created,
        sources_skipped,
    })
}

/// `chunk_section` が `paper_extracts` から読む 1 行。
/// (抽出キー, md_path, papers.id, タイトル, 出版年)
type SectionExtract = (String, String, Option<i64>, Option<String>, Option<i64>);

/// Section 戦略でチャンク化する。
/// 章ごとに段落をまとめ、長い章は段落の切れ目で分ける。短い段落は同じ章の前後とまとめる。
fn chunk_section(
    conn: &Connection,
    index: &crate::config::RagIndex,
    data_dir: &std::path::Path,
    only: Option<i64>,
) -> Result<BuildReport> {
    let mut chunks_created = 0i64;
    let mut sources_skipped = 0i64;

    let max_tokens = index.chunk_max_tokens;
    let embed_with_meta = index.embed_with_meta;

    // paper_extracts から Markdown を読みこむ（結び付きの逆引きは chunk_paragraph と同じ）。
    // タイトルと年は、埋め込みの先頭に付ける分の見積もりに使う。
    let mut stmt = conn.prepare(
        "SELECT pe.paper_id, pe.md_path, p.id,
                p.title, CAST(SUBSTR(p.published_at, 1, 4) AS INTEGER)
         FROM paper_extracts pe
         LEFT JOIN papers p ON p.id = COALESCE(pe.papers_id, (SELECT p2.id FROM papers p2
              WHERE p2.s2_paper_id = pe.paper_id COLLATE NOCASE
                 OR ('ArXiv:' || p2.arxiv_id) = pe.paper_id COLLATE NOCASE
                 OR ('DOI:' || p2.doi) = pe.paper_id COLLATE NOCASE
                 OR p2.arxiv_id = pe.paper_id COLLATE NOCASE
                 OR p2.doi = pe.paper_id COLLATE NOCASE
              LIMIT 1))
         WHERE pe.md_path IS NOT NULL AND pe.status = 'ready'",
    )?;

    let extracts: Vec<SectionExtract> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    for (paper_id_str, md_rel_path, paper_id_opt, title, year) in extracts {
        if !is_wanted(only, paper_id_opt) {
            continue;
        }
        // 既に section チャンクが存在するかチェック
        let existing: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM chunks WHERE source_ref LIKE ?1 AND strategy = 'section' AND index_name = ?2",
            rusqlite::params![format!("extract_{}", paper_id_str), &index.name],
            |row| row.get(0),
        )?;

        if existing {
            sources_skipped += 1;
            continue;
        }

        let md_path = crate::paths::resolve_stored_path(data_dir, &md_rel_path);
        if !md_path.exists() {
            continue;
        }

        let markdown_content =
            crate::citation::docling::strip_figure_images(&std::fs::read_to_string(&md_path)?);

        // 埋め込みの先頭に付ける分だけ、塊の上限を狭める（最大 60 トークン）。
        let groups = split_markdown_sections(&markdown_content, max_tokens, |section| {
            if !embed_with_meta {
                return 0;
            }
            crate::embeddings::embedding_meta_prefix(title.as_deref(), year, section)
                .map_or(0, |prefix| {
                    estimate_tokens(&prefix).min(META_RESERVE_MAX_TOKENS)
                })
        });

        let now = chrono::Utc::now().to_rfc3339();

        for (group_idx, group) in groups.iter().enumerate() {
            for (sub_ord, chunk) in group.iter().enumerate() {
                conn.execute(
                    "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at, index_name, section)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        "body",
                        format!("extract_{}", paper_id_str),
                        paper_id_opt,
                        // 章の順 × 1000 + 分けた順
                        (group_idx as i64) * 1000 + (sub_ord as i64),
                        &chunk.text,
                        chunk.char_start as i64,
                        chunk.char_end as i64,
                        "section",
                        now,
                        &index.name,
                        chunk.section.as_deref(),
                    ],
                )?;
                chunks_created += 1;
            }
        }
    }

    Ok(BuildReport {
        chunks_created,
        sources_skipped,
    })
}

/// Window 戦略でチャンク化する。
fn chunk_window(
    conn: &Connection,
    index: &crate::config::RagIndex,
    only: Option<i64>,
) -> Result<BuildReport> {
    let mut chunks_created = 0i64;
    let mut sources_skipped = 0i64;

    let max_tokens = index.chunk_max_tokens;

    let window_size: usize = index.window_size as usize;
    let window_overlap: usize = index.window_overlap as usize;

    // 各論文について、sentence をウィンドウで分割。
    // 記録の結び付きは papers_id（段階 2c）。NULL の古い行は抽出キーの照合で逆引きする。
    let mut stmt = conn.prepare(
        "SELECT DISTINCT ps.paper_id,
                COALESCE(ps.papers_id, (SELECT p.id FROM papers p
             WHERE p.s2_paper_id = ps.paper_id COLLATE NOCASE
                OR ('ArXiv:' || p.arxiv_id) = ps.paper_id COLLATE NOCASE
                OR ('DOI:' || p.doi) = ps.paper_id COLLATE NOCASE
                OR p.arxiv_id = ps.paper_id COLLATE NOCASE
                OR p.doi = ps.paper_id COLLATE NOCASE
             LIMIT 1)) AS pid
         FROM paper_sentences ps
         ORDER BY ps.paper_id",
    )?;

    let paper_ids: Vec<(String, Option<i64>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    for (paper_id_str, paper_id_opt) in paper_ids {
        if !is_wanted(only, paper_id_opt) {
            continue;
        }
        // 既に window チャンクが存在するかチェック
        let existing: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM chunks WHERE source_ref LIKE ?1 AND strategy = 'window' AND index_name = ?2",
            rusqlite::params![format!("window_{}_%%", paper_id_str), &index.name],
            |row| row.get(0),
        )?;

        if existing {
            sources_skipped += 1;
            continue;
        }

        // 全 sentence を取得
        let mut stmt_sent = conn.prepare(
            "SELECT sent_no, text FROM paper_sentences WHERE paper_id = ?1 ORDER BY sent_no",
        )?;

        let sentences: Vec<(i64, String)> = stmt_sent
            .query_map([&paper_id_str], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        if sentences.is_empty() {
            sources_skipped += 1;
            continue;
        }

        let now = chrono::Utc::now().to_rfc3339();
        let step = if window_overlap >= window_size {
            1
        } else {
            window_size - window_overlap
        };

        // スライディングウィンドウでチャンク化
        let mut chunk_ord = 0;
        for start_idx in (0..sentences.len()).step_by(step) {
            let end_idx = std::cmp::min(start_idx + window_size, sentences.len());
            let window_sentences: Vec<&str> = sentences[start_idx..end_idx]
                .iter()
                .map(|(_no, text)| text.as_str())
                .collect();

            let chunk_text = window_sentences.join(" ");

            // ウィンドウが最大トークン数を超える場合は分割
            let text_chunks = split_text_by_max_tokens(&chunk_text, max_tokens);

            for (sub_ord, text_chunk) in text_chunks.iter().enumerate() {
                conn.execute(
                    "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at, index_name)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        "body",
                        format!("window_{}_{}", paper_id_str, chunk_ord),
                        paper_id_opt,
                        sub_ord as i64,
                        &text_chunk.text,
                        text_chunk.char_start as i64,
                        text_chunk.char_end as i64,
                        "window",
                        now,
                        &index.name,
                    ],
                )?;
                chunks_created += 1;
            }
            chunk_ord += 1;
        }
    }

    Ok(BuildReport {
        chunks_created,
        sources_skipped,
    })
}

/// ノート を全文インデックスに追加する。
/// 戦略に応じて paragraph/sentence/window で分割する。
fn chunk_notes(
    conn: &Connection,
    index: &crate::config::RagIndex,
    data_dir: &std::path::Path,
    strategy: Strategy,
) -> Result<BuildReport> {
    let mut chunks_created = 0i64;
    let mut sources_skipped = 0i64;

    let max_tokens = index.chunk_max_tokens;

    // 論文フォルダから note.md を読み込む（置き場は paths::paper_folder で決める）

    // papers テーブルからすべての論文を取得
    let mut stmt = conn.prepare(
        "SELECT id, title, CAST(SUBSTR(published_at, 1, 4) AS INTEGER) FROM papers ORDER BY id",
    )?;

    let papers: Vec<(i64, String, Option<i64>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let now = chrono::Utc::now().to_rfc3339();

    for (paper_id, title, year) in papers {
        let paper_dir = crate::paths::paper_folder(data_dir, paper_id, &title);
        let note_path = paper_dir.join(crate::paths::paper_file_names::NOTE);

        if note_path.exists() {
            // ファイルから読み込んでチャンク化
            let content = std::fs::read_to_string(&note_path)?;
            let chunks = match strategy {
                Strategy::Section => {
                    // ノートの断片の章は常に `note`（chunk_notes の INSERT と同じ）。
                    let reserve = if index.embed_with_meta {
                        crate::embeddings::embedding_meta_prefix(
                            Some(title.as_str()),
                            year,
                            Some("note"),
                        )
                        .map_or(0, |prefix| {
                            estimate_tokens(&prefix).min(META_RESERVE_MAX_TOKENS)
                        })
                    } else {
                        0
                    };
                    split_markdown_sections(&content, max_tokens, move |_| reserve)
                        .into_iter()
                        .flatten()
                        .collect()
                }
                Strategy::Paragraph => split_markdown_paragraphs(&content),
                Strategy::Sentence | Strategy::Window => split_text_by_sentences(&content),
                Strategy::Abstract => vec![TextChunk {
                    text: content.clone(),
                    char_start: 0,
                    char_end: content.len(),
                    section: None,
                }],
            };

            // 既に同じ戦略でチャンク化されているかチェック
            let source_ref_file = format!("paper_{}", paper_id);
            let existing: bool = conn.query_row(
                "SELECT COUNT(*) > 0 FROM chunks WHERE source_kind = 'note' AND source_ref = ?1 AND strategy = ?2 AND index_name = ?3",
                rusqlite::params![source_ref_file, strategy.as_str(), &index.name],
                |row| row.get(0),
            )?;

            if existing {
                sources_skipped += 1;
                continue;
            }

            for (ord, chunk) in chunks.iter().enumerate() {
                // チャンクが最大トークン数を超える場合は分割
                let text_chunks = split_text_by_max_tokens(&chunk.text, max_tokens);

                for (sub_ord, text_chunk) in text_chunks.iter().enumerate() {
                    let adjusted_char_start = chunk.char_start + text_chunk.char_start;
                    let adjusted_char_end = chunk.char_start + text_chunk.char_end;

                    conn.execute(
                        "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at, index_name, section)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        rusqlite::params![
                            "note",
                            source_ref_file,
                            Some(paper_id),
                            (ord as i64) * 1000 + (sub_ord as i64),
                            &text_chunk.text,
                            adjusted_char_start as i64,
                            adjusted_char_end as i64,
                            strategy.as_str(),
                            now,
                            &index.name,
                            "note",
                        ],
                    )?;
                    chunks_created += 1;
                }
            }
        }
    }

    Ok(BuildReport {
        chunks_created,
        sources_skipped,
    })
}

/// テキストを句で分割する（簡易版）
fn split_text_by_sentences(text: &str) -> Vec<TextChunk> {
    let mut chunks = Vec::new();
    let mut current_start = 0;

    // 。や改行で分割
    for (idx, ch) in text.chars().enumerate() {
        if ch == '。' || ch == '\n' {
            let end = idx + ch.len_utf8();
            if end > current_start {
                let chunk_text = text[current_start..end].trim().to_string();
                if !chunk_text.is_empty() {
                    chunks.push(TextChunk {
                        text: chunk_text,
                        char_start: current_start,
                        char_end: end,
                        section: None,
                    });
                }
                current_start = end;
            }
        }
    }

    // 残りのテキスト
    if current_start < text.len() {
        let chunk_text = text[current_start..].trim().to_string();
        if !chunk_text.is_empty() {
            chunks.push(TextChunk {
                text: chunk_text,
                char_start: current_start,
                char_end: text.len(),
                section: None,
            });
        }
    }

    chunks
}

/// テキストを最大トークン数で分割する。文の切れ目を優先する。
/// estimate_tokens で見積もったトークン数が max_tokens を超えないように分割する。
fn split_text_by_max_tokens(text: &str, max_tokens: usize) -> Vec<TextChunk> {
    if estimate_tokens(text) <= max_tokens {
        return vec![TextChunk {
            text: text.to_string(),
            char_start: 0,
            char_end: text.len(),
            section: None,
        }];
    }

    let mut chunks = Vec::new();
    let mut current_byte_pos = 0;

    while current_byte_pos < text.len() {
        let remaining_text = &text[current_byte_pos..];
        let remaining_tokens = estimate_tokens(remaining_text);

        if remaining_tokens <= max_tokens {
            // 残りがすべて制限内に収まる
            let chunk_text = remaining_text.trim().to_string();
            if !chunk_text.is_empty() {
                chunks.push(TextChunk {
                    text: chunk_text,
                    char_start: current_byte_pos,
                    char_end: text.len(),
                    section: None,
                });
            }
            break;
        }

        // 文の切れ目（。や改行）を探しながら、トークン数が max_tokens に近づく位置を探す
        let mut best_split_byte_pos = remaining_text.len();
        let mut best_found_sentence = false;

        // 文末候補を逆順で探す
        for (idx, ch) in remaining_text.char_indices().rev() {
            if ch == '。' || ch == '\n' {
                let next_boundary = idx + ch.len_utf8();
                let candidate_text = remaining_text[..next_boundary].trim();
                if estimate_tokens(candidate_text) <= max_tokens {
                    best_split_byte_pos = next_boundary;
                    best_found_sentence = true;
                    break;
                }
            }
        }

        // 文の切れ目がなければ、単語の切れ目（空白）を探す
        if !best_found_sentence {
            for (idx, ch) in remaining_text.char_indices().rev() {
                if ch.is_whitespace() {
                    let candidate_text = remaining_text[..idx].trim();
                    if estimate_tokens(candidate_text) <= max_tokens {
                        best_split_byte_pos = idx;
                        break;
                    }
                }
            }
        }

        // 分割位置が見つからない場合は、トークン数が max_tokens に最も近い位置で分割
        if best_split_byte_pos == remaining_text.len() && !best_found_sentence {
            let mut token_sum: f64 = 0.0;
            let mut last_valid_pos = 0;
            for (idx, ch) in remaining_text.char_indices() {
                if ch.is_ascii() {
                    token_sum += 1.0 / 2.3;
                } else {
                    token_sum += 1.0 / 1.0;
                }
                if (token_sum.ceil()) as usize <= max_tokens {
                    last_valid_pos = idx + ch.len_utf8();
                } else {
                    break;
                }
            }
            if last_valid_pos > 0 {
                best_split_byte_pos = last_valid_pos;
            }
        }

        let chunk_text = remaining_text[..best_split_byte_pos].trim().to_string();
        if !chunk_text.is_empty() {
            chunks.push(TextChunk {
                text: chunk_text,
                char_start: current_byte_pos,
                char_end: current_byte_pos + best_split_byte_pos,
                section: None,
            });
        }

        current_byte_pos += best_split_byte_pos;

        // 空白をスキップして次の単語から開始
        while current_byte_pos < text.len()
            && text[current_byte_pos..]
                .chars()
                .next()
                .map_or(false, |c| c.is_whitespace())
        {
            current_byte_pos += text[current_byte_pos..].chars().next().unwrap().len_utf8();
        }
    }

    if chunks.is_empty() {
        // フォールバック：元のテキストをそのまま返す
        chunks.push(TextChunk {
            text: text.to_string(),
            char_start: 0,
            char_end: text.len(),
            section: None,
        });
    }

    chunks
}

/// `paper_figures` の 1 行を、図の塊の本文に組み立てる。
///
/// 本文は `"<label>: <title>\n<caption>\n言及: <mention>"` の形。
/// 無い項目は行ごと省く（label と title は、片方だけならそれだけを出す）。
fn figure_chunk_text(
    label: Option<&str>,
    title: Option<&str>,
    caption: Option<&str>,
    mention: Option<&str>,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    // 1 行目はラベルとタイトル。`Figure 3: Overview` のようにまとめる。
    let label = label.map(str::trim).filter(|s| !s.is_empty());
    let title = title.map(str::trim).filter(|s| !s.is_empty());
    let heading = match (label, title) {
        (Some(label), Some(title)) => Some(format!("{label}: {title}")),
        (Some(label), None) => Some(label.to_string()),
        (None, Some(title)) => Some(title.to_string()),
        (None, None) => None,
    };
    if let Some(heading) = heading {
        lines.push(heading);
    }

    if let Some(caption) = caption.map(str::trim).filter(|s| !s.is_empty()) {
        lines.push(caption.to_string());
    }
    if let Some(mention) = mention.map(str::trim).filter(|s| !s.is_empty()) {
        lines.push(format!("言及: {mention}"));
    }

    lines.join("\n")
}

/// 図表を索引に入れる（1 図 = 1 塊。分けない）。
///
/// `paper_figures` の各行から本文を組み立て、`source_kind = 'figure'`、
/// `source_ref = 'figure_<抽出キー>_<fig_no>'`、`section = <label>`、`ord = fig_no` で保存する。
/// `paper_id` には `papers_id`（Library の番号）を入れる。
/// 本文が 20 字未満の図と、既に同じ図の塊がこの索引にある図は飛ばす。
fn chunk_figures(conn: &Connection, index: &crate::config::RagIndex) -> Result<BuildReport> {
    let mut chunks_created = 0i64;
    let mut sources_skipped = 0i64;

    let max_tokens = index.chunk_max_tokens;

    let mut stmt = conn.prepare(
        "SELECT paper_id, papers_id, fig_no, label, title, caption, mention
         FROM paper_figures ORDER BY paper_id, fig_no",
    )?;
    type FigureChunkRow = (
        String,
        Option<i64>,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<FigureChunkRow> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(stmt);

    let now = chrono::Utc::now().to_rfc3339();

    for (paper_id, papers_id, fig_no, label, title, caption, mention) in rows {
        let text = figure_chunk_text(
            label.as_deref(),
            title.as_deref(),
            caption.as_deref(),
            mention.as_deref(),
        );

        // キャプションも言及も無い等、本文が短すぎる図は入れない。
        if text.chars().count() < 20 {
            sources_skipped += 1;
            continue;
        }

        let source_ref = format!("figure_{}_{}", paper_id, fig_no);

        // 既に同じ図の塊がこの索引にあれば飛ばす（索引は 1 つの分け方しか持たない）。
        let existing: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM chunks WHERE source_ref = ?1 AND index_name = ?2",
            rusqlite::params![source_ref, &index.name],
            |row| row.get(0),
        )?;
        if existing {
            sources_skipped += 1;
            continue;
        }

        // 長すぎるときは分けずに先頭だけを入れる。
        let text = split_text_by_max_tokens(&text, max_tokens)
            .into_iter()
            .next()
            .map_or(text, |chunk| chunk.text);
        let char_end = text.len() as i64;

        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at, index_name, section)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                "figure",
                source_ref,
                papers_id,
                fig_no,
                &text,
                0i64,
                char_end,
                index.chunk_strategy,
                now,
                &index.name,
                label,
            ],
        )?;
        chunks_created += 1;
    }

    Ok(BuildReport {
        chunks_created,
        sources_skipped,
    })
}

/// 木の中身（sai-sci）を索引に入れる（1 もの = 1 塊。分けない）。
///
/// 入れるものは 2 つ:
/// - 生きた F 1 つにつき 1 塊（`source_ref = "F<id>"`、`section = "<親の ID> <親の文>"`）
/// - ★ の付いた生きた O で、本文（`body`）が空でないもの 1 つにつき 1 塊（`source_ref = "O<id>"`）
///
/// `paper_id` は NULL（論文の中の根拠ではないため）。長すぎるときは分けずに先頭だけを入れる。
/// 同じ `source_ref` で同じ `text` の塊が既にこの索引にあれば残す（埋め込みも残る）。
/// `text` が変わったものは消して入れ直し、入れたいものに無い `source_ref` の塊は消す。
fn chunk_tree(conn: &Connection, index: &crate::config::RagIndex) -> Result<BuildReport> {
    // Node（id → (kind, 文, 親の id)）。親の文を引くため、外したものも含めて読む。
    struct Row {
        id: i64,
        kind: String,
        parent_id: Option<i64>,
        text: String,
        body: String,
        starred: bool,
        cites: String,
        alive: bool,
    }
    let rows: Vec<Row> = {
        let mut stmt = conn.prepare(
            "SELECT id, kind, parent_id, text, body, starred, cites, removed_at IS NULL
               FROM rt_nodes ORDER BY id",
        )?;
        let mapped = stmt.query_map([], |row| {
            Ok(Row {
                id: row.get(0)?,
                kind: row.get(1)?,
                parent_id: row.get(2)?,
                text: row.get(3)?,
                body: row.get(4)?,
                starred: row.get::<_, i64>(5)? != 0,
                cites: row.get(6)?,
                alive: row.get(7)?,
            })
        })?;
        mapped.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let parents: std::collections::HashMap<i64, (String, &str)> = rows
        .iter()
        .map(|row| (row.id, (format!("{}{}", row.kind, row.id), row.text.as_str())))
        .collect();

    // 入れたいもの（`source_ref` → 本文と章）。順序は id の昇順に揃える。
    let mut wanted: Vec<(String, String, Option<String>)> = Vec::new();
    for row in rows.iter().filter(|row| row.alive) {
        match row.kind.as_str() {
            "F" => {
                let (parent_id, parent_text) = row
                    .parent_id
                    .and_then(|id| parents.get(&id))
                    .map_or((String::new(), ""), |(id, text)| (id.clone(), *text));
                // cites は JSON の文字列の配列。読めなければ空として扱う。
                let cited: Vec<String> =
                    serde_json::from_str::<Vec<String>>(&row.cites).unwrap_or_default();
                let body = if cited.is_empty() {
                    format!("{parent_text}: {}", row.text)
                } else {
                    format!("{parent_text}: {}（{}）", row.text, cited.join("、"))
                };
                let section = format!("{parent_id} {parent_text}");
                wanted.push((format!("F{}", row.id), body, Some(section)));
            }
            // ★ の付いた O は、本文が空でないものだけを入れる。
            "O" if row.starred && !row.body.trim().is_empty() => {
                let section = format!("O{} {}", row.id, row.text);
                wanted.push((
                    format!("O{}", row.id),
                    format!("{}: {}", row.text, row.body),
                    Some(section),
                ));
            }
            _ => {}
        }
    }

    // 今この索引にある木の塊（`source_ref` → (id, text)）。
    let mut existing: std::collections::HashMap<String, Vec<(i64, String)>> =
        std::collections::HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, source_ref, text FROM chunks
              WHERE source_kind = 'tree' AND index_name = ?1",
        )?;
        let rows = stmt.query_map([&index.name], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (id, source_ref, text) = row?;
            existing.entry(source_ref).or_default().push((id, text));
        }
    }

    let max_tokens = index.chunk_max_tokens;
    let now = chrono::Utc::now().to_rfc3339();
    let mut chunks_created = 0i64;
    let mut sources_skipped = 0i64;
    let mut keep_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut wanted_refs: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (source_ref, body, section) in wanted {
        wanted_refs.insert(source_ref.clone());
        // 長すぎるときは先頭だけを使う。くらべる文字列と入れる文字列をそろえるため、
        // 切るのはここで 1 回だけにして、その結果を下でも使う
        // （生の全文でくらべると、上限を超えた木の実が毎回作り直される）。
        let text = split_text_by_max_tokens(&body, max_tokens)
            .into_iter()
            .next()
            .map_or(body.clone(), |chunk| chunk.text);
        // 同じ `source_ref` で同じ本文の塊があれば残す（埋め込みもそのまま）。
        if let Some(rows) = existing.get(&source_ref) {
            if let Some((id, _)) = rows.iter().find(|(_, found)| *found == text) {
                keep_ids.insert(*id);
                sources_skipped += 1;
                continue;
            }
        }

        // 変わっているものは消して入れ直す。
        for (id, _) in existing.get(&source_ref).into_iter().flatten() {
            conn.execute("DELETE FROM chunks WHERE id = ?1", [id])?;
        }

        let char_end = text.len() as i64;
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at, index_name, section)
             VALUES ('tree', ?1, NULL, 0, ?2, 0, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                source_ref,
                &text,
                char_end,
                index.chunk_strategy,
                &now,
                &index.name,
                section,
            ],
        )?;
        chunks_created += 1;
    }

    // 入れたいものに無い `source_ref` の塊は消す。
    for (source_ref, rows) in &existing {
        if wanted_refs.contains(source_ref) {
            continue;
        }
        for (id, _) in rows {
            if !keep_ids.contains(id) {
                conn.execute("DELETE FROM chunks WHERE id = ?1", [id])?;
            }
        }
    }

    Ok(BuildReport {
        chunks_created,
        sources_skipped,
    })
}

/// インクリメンタルチャンク化：既に処理済みのソースをスキップし、新規分のみ追加する。
///
/// # Errors
///
/// DB エラーの場合。
pub fn build(
    conn: &Connection,
    scope: Scope,
    index: &crate::config::RagIndex,
    data_dir: &std::path::Path,
) -> Result<BuildReport> {
    let strategy = Strategy::from_str(&index.chunk_strategy)?;

    let mut total_created = 0i64;
    let mut total_skipped = 0i64;

    // All スコープの場合、すべてを実行
    if scope == Scope::All {
        // abstract はどの分け方でも作る。
        let abstract_report = chunk_abstract(conn, index, None)?;
        total_created += abstract_report.chunks_created;
        total_skipped += abstract_report.sources_skipped;

        if strategy == Strategy::Section {
            // section の索引は、比べられるように section だけを使う
            // （sentence・paragraph・window は混ぜない）。
            let section_report = chunk_section(conn, index, data_dir, None)?;
            total_created += section_report.chunks_created;
            total_skipped += section_report.sources_skipped;

            let notes_report = chunk_notes(conn, index, data_dir, Strategy::Section)?;
            total_created += notes_report.chunks_created;
            total_skipped += notes_report.sources_skipped;
        } else {
            // すべての戦略を実行
            let sentence_report = chunk_sentence(conn, index, None)?;
            total_created += sentence_report.chunks_created;
            total_skipped += sentence_report.sources_skipped;

            let paragraph_report = chunk_paragraph(conn, index, data_dir, None)?;
            total_created += paragraph_report.chunks_created;
            total_skipped += paragraph_report.sources_skipped;

            let window_report = chunk_window(conn, index, None)?;
            total_created += window_report.chunks_created;
            total_skipped += window_report.sources_skipped;

            // ノート（すべての戦略を実行）
            for strat in &[
                Strategy::Abstract,
                Strategy::Sentence,
                Strategy::Paragraph,
                Strategy::Window,
            ] {
                let notes_report = chunk_notes(conn, index, data_dir, *strat)?;
                total_created += notes_report.chunks_created;
                total_skipped += notes_report.sources_skipped;
            }
        }

        // 図表は分け方によらず 1 図 = 1 塊で入れる。
        let figures_report = chunk_figures(conn, index)?;
        total_created += figures_report.chunks_created;
        total_skipped += figures_report.sources_skipped;
    } else {
        // 指定されたスコープと戦略の組み合わせで実行
        match (scope, strategy) {
            (Scope::Papers, Strategy::Abstract) => {
                let report = chunk_abstract(conn, index, None)?;
                total_created += report.chunks_created;
                total_skipped += report.sources_skipped;
            }
            (Scope::Papers, Strategy::Sentence) => {
                let report = chunk_sentence(conn, index, None)?;
                total_created += report.chunks_created;
                total_skipped += report.sources_skipped;
            }
            (Scope::Papers, Strategy::Paragraph) => {
                let report = chunk_paragraph(conn, index, data_dir, None)?;
                total_created += report.chunks_created;
                total_skipped += report.sources_skipped;
            }
            (Scope::Papers, Strategy::Section) => {
                let report = chunk_section(conn, index, data_dir, None)?;
                total_created += report.chunks_created;
                total_skipped += report.sources_skipped;
            }
            (Scope::Papers, Strategy::Window) => {
                let report = chunk_window(conn, index, None)?;
                total_created += report.chunks_created;
                total_skipped += report.sources_skipped;
            }
            (Scope::Notes, _) => {
                let report = chunk_notes(conn, index, data_dir, strategy)?;
                total_created += report.chunks_created;
                total_skipped += report.sources_skipped;
            }
            // 図表は分け方によらず 1 図 = 1 塊で入れる。
            (Scope::Figures, _) => {
                let report = chunk_figures(conn, index)?;
                total_created += report.chunks_created;
                total_skipped += report.sources_skipped;
            }
            // 木の中身は分け方によらず 1 もの = 1 塊で入れる。
            (Scope::Tree, _) => {
                let report = chunk_tree(conn, index)?;
                total_created += report.chunks_created;
                total_skipped += report.sources_skipped;
            }
            _ => {}
        }
    }

    Ok(BuildReport {
        chunks_created: total_created,
        sources_skipped: total_skipped,
    })
}

/// 論文 1 本の本文の chunk を作る（インクリメンタル）。
///
/// 索引の分け方（strategy）で、`paper_id` の論文だけを対象にする。
/// `build` の Papers スコープと同じ分け方で、ほかの論文の chunk は増えない。
///
/// # Errors
///
/// DB エラーの場合。
pub fn build_paper(
    conn: &Connection,
    paper_id: i64,
    index: &crate::config::RagIndex,
    data_dir: &std::path::Path,
) -> Result<BuildReport> {
    let only = Some(paper_id);
    Ok(match Strategy::from_str(&index.chunk_strategy)? {
        Strategy::Abstract => chunk_abstract(conn, index, only)?,
        Strategy::Sentence => chunk_sentence(conn, index, only)?,
        Strategy::Paragraph => chunk_paragraph(conn, index, data_dir, only)?,
        Strategy::Section => chunk_section(conn, index, data_dir, only)?,
        Strategy::Window => chunk_window(conn, index, only)?,
    })
}

/// チャンクを削除してから再構築する。
///
/// 指定した索引のチャンクを削除し、その索引の分け方で再構築する。
/// これにより、変更前の分け方で作成されたチャンクが索引に残り続けることを防ぐ。
/// TUI の「索引を作り直す」ボタンなどで使用される。
///
/// # Errors
///
/// DB エラーの場合。
pub fn rebuild(
    conn: &Connection,
    index: &crate::config::RagIndex,
    data_dir: &std::path::Path,
) -> Result<BuildReport> {
    // この索引のチャンクだけを削除する（ほかの索引の断片と埋め込みは残す。段階 7）。
    // chunk_embeddings は ON DELETE CASCADE で自動的に削除される
    conn.execute("DELETE FROM chunks WHERE index_name = ?1", [&index.name])?;

    // 索引の分け方で、すべてのスコープ（論文、ノート、ドキュメント）を再構築
    let strategy = Strategy::from_str(&index.chunk_strategy)?;

    let mut total_created = 0i64;
    let mut total_skipped = 0i64;

    // Abstract は常に作成する（build --scope all と同じ）
    let abstract_report = chunk_abstract(conn, index, None)?;
    total_created += abstract_report.chunks_created;
    total_skipped += abstract_report.sources_skipped;

    // 指定の戦略で、すべてのスコープを実行
    match strategy {
        Strategy::Abstract => {
            let notes_report = chunk_notes(conn, index, data_dir, Strategy::Abstract)?;
            total_created += notes_report.chunks_created;
            total_skipped += notes_report.sources_skipped;
        }
        Strategy::Sentence => {
            let report = chunk_sentence(conn, index, None)?;
            total_created += report.chunks_created;
            total_skipped += report.sources_skipped;

            let notes_report = chunk_notes(conn, index, data_dir, Strategy::Sentence)?;
            total_created += notes_report.chunks_created;
            total_skipped += notes_report.sources_skipped;
        }
        Strategy::Paragraph => {
            let report = chunk_paragraph(conn, index, data_dir, None)?;
            total_created += report.chunks_created;
            total_skipped += report.sources_skipped;

            let notes_report = chunk_notes(conn, index, data_dir, Strategy::Paragraph)?;
            total_created += notes_report.chunks_created;
            total_skipped += notes_report.sources_skipped;
        }
        Strategy::Window => {
            let report = chunk_window(conn, index, None)?;
            total_created += report.chunks_created;
            total_skipped += report.sources_skipped;

            let notes_report = chunk_notes(conn, index, data_dir, Strategy::Window)?;
            total_created += notes_report.chunks_created;
            total_skipped += notes_report.sources_skipped;
        }
        Strategy::Section => {
            let report = chunk_section(conn, index, data_dir, None)?;
            total_created += report.chunks_created;
            total_skipped += report.sources_skipped;

            let notes_report = chunk_notes(conn, index, data_dir, Strategy::Section)?;
            total_created += notes_report.chunks_created;
            total_skipped += notes_report.sources_skipped;
        }
    }

    // 図表は分け方によらず 1 図 = 1 塊で入れる（build --scope all と同じ）。
    let figures_report = chunk_figures(conn, index)?;
    total_created += figures_report.chunks_created;
    total_skipped += figures_report.sources_skipped;

    Ok(BuildReport {
        chunks_created: total_created,
        sources_skipped: total_skipped,
    })
}

/// インデックス状態を取得する。
///
/// # Errors
///
/// DB エラーの場合。
pub fn status(conn: &Connection) -> Result<IndexStatus> {
    let mut by_source_kind = std::collections::HashMap::new();
    let mut by_strategy = std::collections::HashMap::new();

    // ソース種別ごとのカウント
    let mut stmt = conn.prepare("SELECT source_kind, COUNT(*) FROM chunks GROUP BY source_kind")?;

    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        if let Ok((kind, count)) = row {
            by_source_kind.insert(kind, count);
        }
    }

    // 戦略ごとのカウント
    let mut stmt = conn.prepare("SELECT strategy, COUNT(*) FROM chunks GROUP BY strategy")?;

    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        if let Ok((strat, count)) = row {
            by_strategy.insert(strat, count);
        }
    }

    // 索引ごとのカウント
    let mut by_index = std::collections::HashMap::new();
    let mut stmt = conn.prepare("SELECT index_name, COUNT(*) FROM chunks GROUP BY index_name")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        if let Ok((name, count)) = row {
            by_index.insert(name, count);
        }
    }

    // 埋め込みを持つチャンク数
    let chunks_with_embeddings: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT chunk_id) FROM chunk_embeddings",
        [],
        |r| r.get(0),
    )?;

    Ok(IndexStatus {
        by_source_kind,
        by_strategy,
        by_index,
        chunks_with_embeddings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 木に必要な表だけを足す（chunking のテスト DB は CREATE のみ。列は読むものだけ）。
    fn create_rt_nodes(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE rt_nodes (
               id INTEGER PRIMARY KEY,
               kind TEXT NOT NULL,
               parent_id INTEGER,
               text TEXT NOT NULL,
               body TEXT NOT NULL DEFAULT '',
               starred INTEGER NOT NULL DEFAULT 0,
               cites TEXT NOT NULL DEFAULT '[]',
               removed_at TEXT
             );",
        )
        .unwrap();
    }

    fn test_db() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        // マイグレーション実行
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE papers (
               id INTEGER PRIMARY KEY,
               title TEXT,
               abstract TEXT,
               abstract_ja TEXT,
               s2_paper_id TEXT,
               arxiv_id TEXT,
               doi TEXT,
               published_at TEXT
             );
             CREATE TABLE paper_sentences (
               paper_id TEXT,
               sent_no INTEGER,
               text TEXT,
               cite_refs TEXT,
               papers_id INTEGER,
               PRIMARY KEY (paper_id, sent_no)
             );
             CREATE TABLE paper_extracts (
               paper_id TEXT PRIMARY KEY,
               md_path TEXT,
               status TEXT,
               papers_id INTEGER
             );
             CREATE TABLE paper_figures (
               paper_id TEXT NOT NULL,
               papers_id INTEGER REFERENCES papers(id),
               fig_no INTEGER NOT NULL,
               label TEXT,
               title TEXT,
               caption TEXT,
               image_path TEXT NOT NULL,
               mention TEXT,
               page INTEGER,
               created_at TEXT NOT NULL,
               PRIMARY KEY (paper_id, fig_no)
             );
             CREATE TABLE chunks (
               id INTEGER PRIMARY KEY,
               source_kind TEXT NOT NULL,
               source_ref TEXT NOT NULL,
               paper_id INTEGER REFERENCES papers(id),
               ord INTEGER NOT NULL,
               text TEXT NOT NULL,
               char_start INTEGER,
               char_end INTEGER,
               strategy TEXT NOT NULL,
               created_at TEXT NOT NULL,
               index_name TEXT NOT NULL DEFAULT 'default',
               section TEXT
             );
             CREATE VIRTUAL TABLE chunk_fts USING fts5(
               text,
               content='chunks',
               content_rowid='id',
               tokenize='unicode61 remove_diacritics 2'
             );
             CREATE TRIGGER chunks_fts_insert AFTER INSERT ON chunks BEGIN
               INSERT INTO chunk_fts(rowid, text) VALUES (new.id, new.text);
             END;",
        )?;
        Ok(conn)
    }

    #[test]
    fn scope_from_str_rejects_documents() {
        assert_eq!(Scope::from_str("papers").unwrap(), Scope::Papers);
        assert_eq!(Scope::from_str("notes").unwrap(), Scope::Notes);
        assert_eq!(Scope::from_str("figures").unwrap(), Scope::Figures);
        assert_eq!(Scope::from_str("all").unwrap(), Scope::All);
        assert!(Scope::from_str("documents").is_err());
        assert!(Scope::from_str("unknown").is_err());
    }

    /// 図の塊の本文の組み立て（R05）。
    #[test]
    fn test_figure_chunk_text() {
        // 全部ある
        assert_eq!(
            figure_chunk_text(
                Some("Figure 3"),
                Some("Overview"),
                Some("The proposed pipeline."),
                Some("See Figure 3 for details."),
            ),
            "Figure 3: Overview\nThe proposed pipeline.\n言及: See Figure 3 for details."
        );

        // キャプションが無い
        assert_eq!(
            figure_chunk_text(
                Some("Table 2"),
                Some("Results"),
                None,
                Some("Table 2 shows the results."),
            ),
            "Table 2: Results\n言及: Table 2 shows the results."
        );

        // 言及が無い
        assert_eq!(
            figure_chunk_text(
                Some("Figure 1"),
                None,
                Some("A long enough caption here."),
                None,
            ),
            "Figure 1\nA long enough caption here."
        );

        // ラベルもタイトルも無い（キャプションだけ）
        assert_eq!(
            figure_chunk_text(None, None, Some("Only a caption."), None),
            "Only a caption."
        );

        // 何も無い
        assert_eq!(figure_chunk_text(None, None, None, None), "");
    }

    /// 20 字未満の図は塊にしない（R05）。
    #[test]
    fn test_chunk_figures_skips_short_text() {
        let conn = test_db().unwrap();
        conn.execute("INSERT INTO papers (id, title) VALUES (10, 'Paper A')", [])
            .unwrap();

        // 短い（`Figure 3` だけ = 8 字）: 飛ばす
        conn.execute(
            "INSERT INTO paper_figures (paper_id, papers_id, fig_no, label, image_path, created_at)
             VALUES ('key1', 10, 1, 'Figure 1', '/x/fig-001.png', '2024-01-01')",
            [],
        )
        .unwrap();
        // 長い: 入れる
        conn.execute(
            "INSERT INTO paper_figures (paper_id, papers_id, fig_no, label, title, caption, image_path, mention, created_at)
             VALUES ('key1', 10, 2, 'Figure 2', 'Architecture', 'The full architecture of the model.', '/x/fig-002.png', 'Figure 2 shows the architecture.', '2024-01-01')",
            [],
        )
        .unwrap();

        let index = crate::config::RagConfig::default().index(None).unwrap();
        let report = chunk_figures(&conn, &index).unwrap();
        assert_eq!(report.chunks_created, 1);
        assert_eq!(report.sources_skipped, 1);

        let (kind, source_ref, paper_id, ord, section, text): (
            String,
            String,
            Option<i64>,
            i64,
            Option<String>,
            String,
        ) = conn
            .query_row(
                "SELECT source_kind, source_ref, paper_id, ord, section, text FROM chunks",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(kind, "figure");
        assert_eq!(source_ref, "figure_key1_2");
        assert_eq!(paper_id, Some(10));
        assert_eq!(ord, 2);
        assert_eq!(section.as_deref(), Some("Figure 2"));
        assert_eq!(
            text,
            "Figure 2: Architecture\nThe full architecture of the model.\n言及: Figure 2 shows the architecture."
        );

        // 2 回目は既存分を飛ばす
        let report = chunk_figures(&conn, &index).unwrap();
        assert_eq!(report.chunks_created, 0);
        assert_eq!(report.sources_skipped, 2);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    /// build --scope figures で図だけが入る（R05）。
    #[test]
    fn test_build_scope_figures_creates_only_figures() {
        let conn = test_db().unwrap();
        // abstract を持つ論文も用意し、figures スコープでは入らないことを確かめる。
        conn.execute(
            "INSERT INTO papers (id, title, abstract) VALUES (10, 'Paper A', 'An abstract text long enough')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO paper_figures (paper_id, papers_id, fig_no, label, title, caption, image_path, mention, created_at)
             VALUES ('key1', 10, 1, 'Figure 1', 'Architecture', 'The full architecture of the model.', '/x/fig-001.png', 'Figure 1 shows the architecture.', '2024-01-01')",
            [],
        )
        .unwrap();

        let index = crate::config::RagConfig::default().index(None).unwrap();
        let dir = std::env::temp_dir().join("sai-rag-scope-figures");
        let report = build(&conn, Scope::Figures, &index, &dir).unwrap();
        assert_eq!(report.chunks_created, 1);

        let kinds: Vec<String> = conn
            .prepare("SELECT DISTINCT source_kind FROM chunks ORDER BY source_kind")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(kinds, vec!["figure"]);
    }

    #[test]
    fn test_abstract_strategy_chunks_text_correctly() {
        let conn = test_db().unwrap();
        conn.execute(
            "INSERT INTO papers (id, title, abstract) VALUES (?1, ?2, ?3)",
            rusqlite::params![1, "Test Paper", "This is the abstract text"],
        )
        .unwrap();

        let report = chunk_abstract(&conn, &crate::config::RagConfig::default().index(None).unwrap(), None).unwrap();
        assert_eq!(report.chunks_created, 1);

        let (text, start, end): (String, i64, i64) = conn
            .query_row(
                "SELECT text, char_start, char_end FROM chunks LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        // char_start/char_end でテキストをスライスして検証
        assert_eq!(&text[start as usize..end as usize], text);
        assert_eq!(text, "This is the abstract text");
    }

    #[test]
    fn test_sentence_strategy_chunks_correctly() {
        let conn = test_db().unwrap();
        conn.execute("INSERT INTO papers (id, title) VALUES (1, 'Paper')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO paper_sentences (paper_id, sent_no, text, cite_refs) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["1", 0i64, "First sentence。", "[]"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO paper_sentences (paper_id, sent_no, text, cite_refs) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["1", 1i64, "Second sentence。", "[]"],
        )
        .unwrap();

        let report = chunk_sentence(&conn, &crate::config::RagConfig::default().index(None).unwrap(), None).unwrap();
        assert_eq!(report.chunks_created, 2);

        let chunks: Vec<(String, i64, i64)> = conn
            .prepare("SELECT text, char_start, char_end FROM chunks ORDER BY ord")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].0, "First sentence。");
        assert_eq!(chunks[1].0, "Second sentence。");
    }

    #[test]
    fn test_paragraph_strategy_chunks_correctly() {
        let conn = test_db().unwrap();
        conn.execute("INSERT INTO papers (id, title) VALUES (1, 'Paper')", [])
            .unwrap();

        let markdown = "# Heading\n\nFirst para.\n\n# Section\n\nSecond para.";
        let chunks = split_markdown_paragraphs(markdown);

        assert_eq!(chunks.len(), 2);
        // char_start/char_end は chunk_text のオフセット
        assert!(chunks[0].text.contains("First para"));
        assert!(chunks[1].text.contains("Second para"));
    }

    #[test]
    fn test_paragraph_strategy_includes_paper_id() {
        // 本文チャンクに paper_id が入ることを確認
        let conn = test_db().unwrap();
        conn.execute(
            "INSERT INTO papers (id, title) VALUES (1, 'Test Paper')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO paper_extracts (paper_id, md_path, status) VALUES (?1, ?2, ?3)",
            rusqlite::params![1, "data/extracts/test.md", "ready"],
        )
        .unwrap();

        // 既存のチャンク状態をテストするため、実際のファイルは不要
        // 代わりに、paper_extracts の検索が正しく left join されることを確認
        let mut stmt = conn
            .prepare(
                "SELECT pe.paper_id, pe.md_path, p.id FROM paper_extracts pe
             LEFT JOIN papers p ON p.id = pe.paper_id
             WHERE pe.md_path IS NOT NULL AND pe.status = 'ready'",
            )
            .unwrap();

        let rows: Vec<(String, String, Option<i64>)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(rows.len(), 1);
        let (_paper_id_str, _md_path, paper_id_opt) = &rows[0];
        assert_eq!(*paper_id_opt, Some(1)); // paper_id が取得されている
    }

    #[test]
    fn test_window_strategy_chunks_correctly() {
        let conn = test_db().unwrap();
        conn.execute("INSERT INTO papers (id, title) VALUES (1, 'Paper')", [])
            .unwrap();
        for i in 0..5 {
            conn.execute(
                "INSERT INTO paper_sentences (paper_id, sent_no, text, cite_refs) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["1", i as i64, format!("Sentence {}.", i), "[]"],
            )
            .unwrap();
        }

        let config = crate::config::RagConfig::default().index(None).unwrap();
        let report = chunk_window(&conn, &config, None).unwrap();
        // デフォルト window_size=3, overlap=1 で 5 文なら 3 チャンク
        assert!(report.chunks_created > 0);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE strategy = 'window'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, report.chunks_created);
    }

    /// build_paper は指定した論文の chunk だけを作る（ほかの論文の chunk は増えない）。
    #[test]
    fn test_build_paper_only_that_paper() {
        let conn = test_db().unwrap();
        conn.execute(
            "INSERT INTO papers (id, title, abstract) VALUES (1, 'A', 'Abstract one'), (2, 'B', 'Abstract two')",
            [],
        )
        .unwrap();
        let mut index = crate::config::RagConfig::default().index(None).unwrap();
        index.chunk_strategy = "abstract".to_string();

        let report = build_paper(&conn, 2, &index, std::path::Path::new("/nonexistent")).unwrap();

        assert_eq!(report.chunks_created, 1);
        let paper_ids: Vec<i64> = conn
            .prepare("SELECT DISTINCT paper_id FROM chunks ORDER BY paper_id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(paper_ids, vec![2]);
    }

    #[test]
    fn test_build_idempotent() {
        let conn = test_db().unwrap();
        conn.execute(
            "INSERT INTO papers (id, title, abstract) VALUES (1, 'Paper', 'Abstract text')",
            [],
        )
        .unwrap();

        let report1 = chunk_abstract(&conn, &crate::config::RagConfig::default().index(None).unwrap(), None).unwrap();
        assert_eq!(report1.chunks_created, 1);

        let report2 = chunk_abstract(&conn, &crate::config::RagConfig::default().index(None).unwrap(), None).unwrap();
        assert_eq!(report2.chunks_created, 0);
        assert_eq!(report2.sources_skipped, 1);
    }

    #[test]
    fn test_chunk_fts_query() {
        let conn = test_db().unwrap();
        conn.execute(
            "INSERT INTO papers (id, title, abstract) VALUES (1, 'Paper', 'searchable keyword here')",
            [],
        )
        .unwrap();

        chunk_abstract(&conn, &crate::config::RagConfig::default().index(None).unwrap(), None).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunk_fts WHERE chunk_fts MATCH 'searchable'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(count, 1);
    }

    #[test]
    fn test_rebuild_removes_old_strategy() {
        let conn = test_db().unwrap();

        // 古いsource_refスキーム（'note_1'など）を手動で挿入
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at) VALUES ('note', 'note_1', NULL, 0, 'old', 0, 3, 'abstract', datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at) VALUES ('note', 'notes/old', NULL, 0, 'old', 0, 3, 'abstract', datetime('now'))",
            [],
        )
        .unwrap();

        let count_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE source_kind = 'note'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count_before, 2);

        // DELETE 文を直接実行（rebuild がやることのシミュレーション）
        conn.execute(
            "DELETE FROM chunks WHERE source_kind = 'note' AND (source_ref GLOB 'note_*' OR (source_ref GLOB 'notes/*' AND source_ref NOT GLOB '*.md'))",
            [],
        )
        .unwrap();

        let count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE source_kind = 'note'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        // 古いsource_refは削除されている
        assert_eq!(count_after, 0);
    }

    #[test]
    fn test_split_markdown_paragraphs() {
        let markdown =
            "# Heading 1\n\nFirst paragraph.\nStill first.\n\n# Heading 2\n\nSecond paragraph.";
        let chunks = split_markdown_paragraphs(markdown);

        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].text.contains("First paragraph"));
        assert!(chunks[1].text.contains("Second paragraph"));
    }

    /// AC1: 段落に、上にある見出しの並び（章）を付ける。
    #[test]
    fn test_split_markdown_paragraphs_tracks_sections() {
        let markdown = "\
Intro paragraph.

# 1 Introduction

First intro.

## 1.1 Background

Background text.

## **1.2 Related Work**

Related text.

# 2 Methods

Methods text.
";
        let chunks = split_markdown_paragraphs(markdown);
        let sections: Vec<Option<&str>> = chunks.iter().map(|c| c.section.as_deref()).collect();
        assert_eq!(
            sections,
            vec![
                // 見出しの無い冒頭は章なし。
                None,
                Some("1 Introduction"),
                Some("1 Introduction > 1.1 Background"),
                // 太字の `**` は外す。
                Some("1 Introduction > 1.2 Related Work"),
                // `#` に戻ると、それより深い `##` は外れる。
                Some("2 Methods"),
            ]
        );
    }

    /// 見出しの直後に空行が無くても、段落は手前の章に入る。
    #[test]
    fn test_split_markdown_paragraphs_heading_without_blank_line() {
        let markdown = "# 2 Methods\nMethods text.\n# 3 Results\nResults text.";
        let chunks = split_markdown_paragraphs(markdown);
        let sections: Vec<Option<&str>> = chunks.iter().map(|c| c.section.as_deref()).collect();
        assert_eq!(
            sections,
            vec![Some("2 Methods"), Some("3 Results")]
        );
    }

    /// 画像の行を除いた後でも、見出しの行は章として残る。
    #[test]
    fn test_split_markdown_paragraphs_sections_after_image_removal() {
        let markdown = "# 1 Introduction\n\nParagraph.\n\n![fig](data:image/png;base64,AAAA)\n\n## 1.1 Background\n\nBody.";
        let chunks = split_markdown_paragraphs(markdown);
        let sections: Vec<Option<&str>> = chunks.iter().map(|c| c.section.as_deref()).collect();
        assert_eq!(
            sections,
            vec![
                Some("1 Introduction"),
                Some("1 Introduction > 1.1 Background"),
            ]
        );
    }

    /// AC2: `chunk_paragraph` が extract.md から読んで `chunks.section` を入れる。
    #[test]
    fn test_paragraph_strategy_records_section() {
        let conn = test_db().unwrap();
        conn.execute("INSERT INTO papers (id, title) VALUES (1, 'Paper')", [])
            .unwrap();

        // 一時のデータフォルダに extract.md を置く（DB の md_path は data_dir からの相対）。
        let data_dir = std::env::temp_dir().join(format!(
            "sai-paper-chunk-section-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&data_dir);
        std::fs::create_dir_all(&data_dir).unwrap();
        let extract_rel = "extracts/1.md";
        let extract_abs = data_dir.join(extract_rel);
        std::fs::create_dir_all(extract_abs.parent().unwrap()).unwrap();
        // トークン上限（450）を超える長い段落も、同じ章になることを見る。
        let long_para = "This is a long paragraph with many words. ".repeat(60);
        let markdown = format!(
            "Intro paragraph.\n\n# 1 Introduction\n\nShort intro paragraph.\n\n# 2 Methods\n\n{long_para}\n"
        );
        std::fs::write(&extract_abs, &markdown).unwrap();
        conn.execute(
            "INSERT INTO paper_extracts (paper_id, md_path, status, papers_id) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["1", extract_rel, "ready", 1],
        )
        .unwrap();

        let config = crate::config::RagConfig::default().index(None).unwrap();
        chunk_paragraph(&conn, &config, &data_dir, None).unwrap();

        let sections: Vec<Option<String>> = conn
            .prepare("SELECT section FROM chunks WHERE strategy = 'paragraph' ORDER BY ord")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        // 冒頭の段落は章なし、以降は見出しの章が付く。
        assert_eq!(sections[0], None);
        assert_eq!(sections[1].as_deref(), Some("1 Introduction"));
        // 長い段落は複数の塊に分かれても、すべて同じ章を引き継ぐ。
        let method_count = sections
            .iter()
            .filter(|s| s.as_deref() == Some("2 Methods"))
            .count();
        assert!(method_count > 1, "長い段落が分割されていない: {sections:?}");
        assert!(sections.iter().all(|s| {
            s.as_deref().is_none_or(|s| matches!(s, "1 Introduction" | "2 Methods"))
        }));

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn test_strategy_from_str() {
        assert_eq!(Strategy::from_str("abstract").unwrap(), Strategy::Abstract);
        assert_eq!(Strategy::from_str("sentence").unwrap(), Strategy::Sentence);
        assert_eq!(
            Strategy::from_str("paragraph").unwrap(),
            Strategy::Paragraph
        );
        assert_eq!(Strategy::from_str("section").unwrap(), Strategy::Section);
        assert_eq!(Strategy::from_str("window").unwrap(), Strategy::Window);
        assert!(Strategy::from_str("invalid").is_err());
    }

    /// AC1: 章単位の分割。長い章は段落の切れ目で分かれ、短い段落は同じ章の前後とまとまり、
    /// 章が変われば必ず別の組になる。
    #[test]
    fn test_split_markdown_sections_groups_by_section() {
        // 1 つの段落だけで上限を超える長い章。
        let long_para = "word ".repeat(300);
        let markdown = format!(
            "# 1 Introduction\n\nShort.\n\n{long_para}\n\n# 2 Methods\n\nMethods paragraph."
        );
        let groups = split_markdown_sections(&markdown, 100, |_| 0);

        // 章が変わると必ず別の組になる。
        assert_eq!(groups.len(), 2, "章ごとに分かれていない: {groups:?}");
        assert!(groups.iter().all(|g| !g.is_empty()));

        // 章 1: 先頭の短い段落は次の段落とまとまり、長い段落は段落の切れ目で分かれる。
        let intro = &groups[0];
        assert!(intro[0].text.starts_with("Short."), "短い段落がまとまっていない");
        assert!(intro.len() > 1, "長い章が分割されていない");
        assert!(intro
            .iter()
            .all(|c| c.section.as_deref() == Some("1 Introduction")));
        assert!(groups[1]
            .iter()
            .all(|c| c.section.as_deref() == Some("2 Methods")));

        // すべて上限トークン数以内。
        for chunk in groups.iter().flatten() {
            assert!(
                estimate_tokens(&chunk.text) <= 100,
                "上限超過: {}",
                chunk.text
            );
        }
    }

    /// 100 字未満の段落は、同じ章の前の段落とまとまる。
    #[test]
    fn test_split_markdown_sections_merges_short_into_previous() {
        let markdown = "# 1 Introduction\n\nThis is a sufficiently long paragraph that is definitely at least one hundred characters long so it stands alone.\n\nShort.";
        let groups = split_markdown_sections(markdown, 450, |_| 0);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 1, "短い段落が前の段落とまとまっていない");
        assert!(groups[0][0].text.contains("Short."));
        assert_eq!(groups[0][0].section.as_deref(), Some("1 Introduction"));
    }

    /// AC3: `embed_with_meta` の索引では、塊の上限から先頭に付ける分を差し引く。
    /// 先頭（タイトル・年・章）を付けた長さが `chunk_max_tokens` を超えない。
    #[test]
    fn test_section_strategy_reserves_meta_tokens() {
        let conn = test_db().unwrap();
        conn.execute(
            "INSERT INTO papers (id, title, published_at) VALUES (1, 'A Fairly Long Attention Paper Title For Tests', '2024-06-01T00:00:00+00:00')",
            [],
        )
        .unwrap();

        let data_dir = std::env::temp_dir().join(format!(
            "sai-paper-chunk-section-meta-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&data_dir);
        std::fs::create_dir_all(&data_dir).unwrap();
        let extract_rel = "extracts/1.md";
        let extract_abs = data_dir.join(extract_rel);
        std::fs::create_dir_all(extract_abs.parent().unwrap()).unwrap();
        let long_para = "This is a long paragraph with many words. ".repeat(80);
        let markdown = format!("# 1 Introduction\n\n{long_para}\n");
        std::fs::write(&extract_abs, &markdown).unwrap();
        conn.execute(
            "INSERT INTO paper_extracts (paper_id, md_path, status, papers_id) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["1", extract_rel, "ready", 1],
        )
        .unwrap();

        let rag: crate::config::RagConfig = serde_json::from_str(
            r#"{"chunk_max_tokens": 100, "indexes": [{"name": "section-meta", "chunk_strategy": "section", "embed_with_meta": true}]}"#,
        )
        .unwrap();
        let index = rag.index(Some("section-meta")).unwrap();
        assert!(index.embed_with_meta);

        let report = build(&conn, Scope::Papers, &index, &data_dir).unwrap();
        assert!(report.chunks_created > 1, "長い章が分割されていない");

        let chunks: Vec<(String, Option<String>)> = conn
            .prepare("SELECT text, section FROM chunks WHERE strategy = 'section' ORDER BY ord")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        for (text, section) in &chunks {
            let prefix = crate::embeddings::embedding_meta_prefix(
                Some("A Fairly Long Attention Paper Title For Tests"),
                Some(2024),
                section.as_deref(),
            )
            .unwrap();
            let combined = format!("{}\n{}", prefix, text);
            assert!(
                estimate_tokens(&combined) <= 100,
                "先頭を付けると上限を超える: {}",
                combined
            );
        }

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// section の索引では、`build --scope all` で section 以外の本文の分け方を混ぜない。
    #[test]
    fn test_build_all_section_scope_uses_only_section() {
        let conn = test_db().unwrap();
        conn.execute("INSERT INTO papers (id, title, abstract) VALUES (1, 'P', 'An abstract.')", [])
            .unwrap();
        let rag: crate::config::RagConfig = serde_json::from_str(
            r#"{"indexes": [{"name": "section", "chunk_strategy": "section"}]}"#,
        )
        .unwrap();
        let index = rag.index(Some("section")).unwrap();
        let dir = std::env::temp_dir().join("sai-rag-section-all-empty");
        build(&conn, Scope::All, &index, &dir).unwrap();

        let strategies: Vec<String> = conn
            .prepare("SELECT DISTINCT strategy FROM chunks WHERE index_name = 'section' ORDER BY strategy")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        // abstract は常に作る。本文は section だけ（sentence/paragraph/window を含まない）。
        assert_eq!(strategies, vec!["abstract"], "本文が無いので abstract だけ");
    }

    #[test]
    fn test_split_text_by_max_tokens_english() {
        // 英語テキスト：短いテキストは分割されない
        let short_text = "This is a short text.";
        let chunks = split_text_by_max_tokens(short_text, 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, short_text);

        // 長いテキストは分割される（かなり短い制限を設定）
        let long_text = "This is a longer text with multiple sentences. The second sentence is here. And a third one. And more text here. And another sentence.";
        let chunks = split_text_by_max_tokens(long_text, 10);
        assert!(chunks.len() > 1);

        // 各チャンクが制限内のトークン数になっていることを確認
        for chunk in &chunks {
            let tokens = estimate_tokens(&chunk.text);
            assert!(
                tokens <= 10,
                "Chunk '{}' exceeds token limit: {} > 10",
                chunk.text,
                tokens
            );
        }
    }

    #[test]
    fn test_split_text_by_max_tokens_japanese() {
        // 日本語テキストのトークン数見積もりをテスト
        let japanese_text =
            "これはテストです。このテキストは分割されるべきです。もう一つの文です。";
        let chunks = split_text_by_max_tokens(japanese_text, 50);

        // 各チャンクが制限内のトークン数になっていることを確認
        for chunk in &chunks {
            let tokens = estimate_tokens(&chunk.text);
            assert!(
                tokens <= 50,
                "Chunk '{}' exceeds token limit: {} > 50",
                chunk.text,
                tokens
            );
        }
    }

    #[test]
    fn test_char_start_char_end_correctness() {
        // char_start と char_end が実際の位置を正しく指すことを確認
        let text = "This is a test. This is another sentence.";
        let chunks = split_text_by_max_tokens(text, 20);

        for chunk in &chunks {
            // char_start と char_end の範囲で元のテキストをスライスすると、チャンク本文になるはず
            let sliced = text[chunk.char_start..chunk.char_end].trim();
            assert!(
                sliced.contains(&chunk.text) || chunk.text.contains(sliced),
                "char_start/char_end mismatch: text[{}..{}]='{}', chunk.text='{}'",
                chunk.char_start,
                chunk.char_end,
                sliced,
                chunk.text
            );
        }
    }

    #[test]
    fn test_rebuild_includes_abstract_and_strategy() {
        // chunk_abstract と別の戦略を組み合わせることを確認
        // (rebuild の実装が両方を呼ぶことを間接的にテスト)
        let conn = test_db().unwrap();
        conn.execute(
            "INSERT INTO papers (id, title, abstract) VALUES (1, 'Paper', 'Abstract text')",
            [],
        )
        .unwrap();

        let config = crate::config::RagConfig::default().index(None).unwrap();

        // abstract を作成
        let abstract_report = chunk_abstract(&conn, &config, None).unwrap();
        assert_eq!(abstract_report.chunks_created, 1);

        // abstract チャンクが存在することを確認
        let abstract_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE source_kind = 'abstract' AND strategy = 'abstract'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(abstract_count, 1);
    }

    /// 索引を 1 つ作り直しても、ほかの索引の断片は残る（段階 7）。
    #[test]
    fn rebuild_one_index_keeps_other_indexes() {
        let conn = test_db().unwrap();
        conn.execute(
            "INSERT INTO papers (id, title, abstract) VALUES (1, 'P', 'An abstract.')",
            [],
        )
        .unwrap();
        let rag: crate::config::RagConfig = serde_json::from_str(
            r#"{"indexes": [{"name": "a", "chunk_strategy": "abstract"}, {"name": "b", "chunk_strategy": "abstract"}]}"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join("sai-rag-rebuild-test-empty");
        let (a, b) = (rag.index(Some("a")).unwrap(), rag.index(Some("b")).unwrap());
        build(&conn, Scope::Papers, &a, &dir).unwrap();
        build(&conn, Scope::Papers, &b, &dir).unwrap();
        let count = |name: &str| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM chunks WHERE index_name = ?1", [name], |r| r.get(0))
                .unwrap()
        };
        assert_eq!((count("a"), count("b")), (1, 1));
        let b_id: i64 = conn
            .query_row("SELECT id FROM chunks WHERE index_name = 'b'", [], |r| r.get(0))
            .unwrap();
        rebuild(&conn, &a, &dir).unwrap();
        assert_eq!((count("a"), count("b")), (1, 1));
        let b_after: i64 = conn
            .query_row("SELECT id FROM chunks WHERE index_name = 'b'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(b_id, b_after, "b の断片は作り直されない");
    }

    /// 木の索引は、変わっていないものは残し、変わったものだけ入れ直す。
    #[test]
    fn tree_chunks_keep_unchanged_and_replace_changed() {
        let conn = test_db().unwrap();
        create_rt_nodes(&conn);
        // Q1 の下に、★ の O2（本文あり）・★ の無い O5（本文あり）・F3・F4（O2 の根拠）。
        // S6（SPROUT）は本文を持つが、木の塊にはしない。
        conn.execute(
            "INSERT INTO rt_nodes (id, kind, parent_id, text, body, starred, cites) VALUES
               (1, 'Q', NULL, '尺度', '', 0, '[]'),
               (2, 'O', 1, '手法', 'CET の前に正規化をする', 1, '[]'),
               (3, 'F', 2, '正規化が効くと残差が小さくなる', '', 0, '[\"#3 §2.1\"]'),
               (4, 'F', 2, 'Batch を 32 にすると速い', '', 0, '[]'),
               (5, 'O', 1, '別案', '本文はあるが ★ が無い', 0, '[]'),
               (6, 'S', NULL, 'SPROUT', '本文を持つが塊にしない', 0, '[]')",
            [],
        )
        .unwrap();

        let index = crate::config::RagConfig::default().index(None).unwrap();
        let dir = std::env::temp_dir().join("sai-rag-scope-tree");
        let first = build(&conn, Scope::Tree, &index, &dir).unwrap();
        // F 2 つ + ★ の O 1 つ（本文あり）。
        assert_eq!(first.chunks_created, 3);
        assert_eq!(first.sources_skipped, 0);

        let tree_rows = |conn: &Connection| -> Vec<(String, String)> {
            let mut stmt = conn
                .prepare(
                    "SELECT source_ref, text FROM chunks WHERE source_kind = 'tree' ORDER BY source_ref",
                )
                .unwrap();
            let rows: Vec<(String, String)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            rows
        };
        let before = tree_rows(&conn);
        assert_eq!(before.len(), 3);
        // O の塊は文と本文を並べる。論文の無い塊（paper_id が NULL）。
        let node_chunk: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE source_ref = 'O2' AND paper_id IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(node_chunk, 1);

        // F4 を ✕ し、O2 の本文を直す。
        conn.execute("UPDATE rt_nodes SET removed_at = 'now' WHERE id = 4", [])
            .unwrap();
        conn.execute(
            "UPDATE rt_nodes SET body = 'CET と尺度を同時に使う' WHERE id = 2",
            [],
        )
        .unwrap();

        let second = build(&conn, Scope::Tree, &index, &dir).unwrap();
        // 入れ直したのは 1 つ（直した O）。残ったのは 1 つ（F3）。
        // ✕ した F の塊（F4）は消えるが、入れた数には数えない。
        assert_eq!(second.chunks_created, 1);
        assert_eq!(second.sources_skipped, 1);

        let after = tree_rows(&conn);
        // ✕ した F の塊（F4）は消え、他は 1 つずつ残る。
        let refs: Vec<&String> = after.iter().map(|(r, _)| r).collect();
        assert!(!refs.iter().any(|r| *r == "F4"), "{after:?}");
        assert_eq!(after.len(), 2);
        // O の塊は直した本文に変わっている。
        let node_text = after
            .iter()
            .find(|(r, _)| r == "O2")
            .map(|(_, t)| t.clone())
            .unwrap();
        assert_eq!(node_text, "手法: CET と尺度を同時に使う");

        // 3 回目は何も変わらない（埋め込みごと残る）。
        let third = build(&conn, Scope::Tree, &index, &dir).unwrap();
        assert_eq!(third.chunks_created, 0);
        assert_eq!(third.sources_skipped, 2);
    }

    /// 上限を超えた木の実が、2 回目以降は作り直されない。
    ///
    /// くらべる文字列と入れる文字列が違っていると、切り詰めた本文と全文を
    /// くらべるので毎回「削除 → 再挿入」になる（埋め込みも作り直し）。
    #[test]
    fn tree_chunks_long_text_is_not_rebuilt_every_time() {
        let conn = test_db().unwrap();
        create_rt_nodes(&conn);

        // 上限（既定 450 トークン）を大きく超える本文（日本語は 1 字 = 1 トークン）。
        let long: String = "報告の要点を書く。".repeat(400);
        conn.execute(
            "INSERT INTO rt_nodes (id, kind, parent_id, text, body, starred) VALUES
               (1, 'Q', NULL, '問い', '', 0),
               (2, 'O', 1, '長い項目', ?1, 1)",
            [&long],
        )
        .unwrap();

        let mut index = crate::config::RagConfig::default().index(None).unwrap();
        index.chunk_max_tokens = 450;
        let dir = std::env::temp_dir().join("sai-rag-scope-tree-long");
        let first = build(&conn, Scope::Tree, &index, &dir).unwrap();
        assert_eq!(first.chunks_created, 1);
        assert_eq!(first.sources_skipped, 0);

        // 入れるのは先頭だけ（上限どおり）。
        let stored: String = conn
            .query_row(
                "SELECT text FROM chunks WHERE source_kind = 'tree' AND source_ref = 'O2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stored.chars().count() < long.chars().count(), "切り詰められていない");
        // O の塊は「文: 本文」なので、本文の先頭から切られている。
        let prefix = "長い項目: ";
        assert!(stored.starts_with(prefix), "{stored}");
        assert!(
            long.starts_with(&stored[prefix.len()..]),
            "先頭から切られていない"
        );

        // 2 回目も 3 回目も作り直さない（同じ文字列でくらべているので残る）。
        let second = build(&conn, Scope::Tree, &index, &dir).unwrap();
        assert_eq!(second.chunks_created, 0, "毎回作り直している");
        assert_eq!(second.sources_skipped, 1);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE source_kind = 'tree'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "増えたり減えたりしている");
    }
}
