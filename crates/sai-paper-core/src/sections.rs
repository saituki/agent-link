//! 論文の節（section）と塊（passage）の表。
//!
//! `extract.md` を見出しで節に、空行で塊に分け、安定した ID（`#37.5` / `#37.5.2`）を付けて
//! papr.db に保存する。ID は `extract.md` の中身の順番だけで決まるので、同じ `extract.md` なら
//! 作り直しても変わらない。RAG の `chunks` 表とは別物（あちらは索引ごとで、作り直すと ID が変わる）。

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use futures_util::stream::{self, StreamExt};
use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::chunking::{markdown_heading, remove_data_uri_images};
use crate::evidence::ChatClient;
use crate::retrieval::Hit;

/// 1 つの塊の最大の長さ（文字数）。
const PASSAGE_MAX_CHARS: usize = 1500;
/// これより短い段落は前の塊につなぐ（文字数）。
const PASSAGE_MIN_CHARS: usize = 100;
/// `lead` の最大の長さ（文字数）。
const LEAD_MAX_CHARS: usize = 300;
/// 要約を作る節の最小の長さ（文字数）。
const SUMMARY_MIN_CHARS: i64 = 200;
/// 要約に渡す節の全文の最大の長さ（文字数）。
const SUMMARY_INPUT_CHARS: usize = 6000;

const SUMMARY_SYSTEM: &str =
    "あなたは論文の節を要約します。日本語で 2〜3 文、200 字以内。数値と手法名は原文のまま書く。";

/// このモジュールのエラー。
#[derive(Debug, thiserror::Error)]
pub enum SectionsError {
    /// `extract.md` が無い。
    #[error(
        "論文 #{0} の extract.md がありません。`sai-paper paper figures {0}` か `sai-paper paper citations-extract {0}` で作ってください"
    )]
    NoExtract(i64),
    /// ID の形が違う。
    #[error("ID は 37・37.5・37.5.2・37.5.2-4 の形で書いてください")]
    BadRef,
    /// 指定した ID の節・塊が無い。
    #[error("{0} が見つかりません")]
    NotFound(String),
    /// DB の失敗。
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    /// ファイルの読み込みの失敗。
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// チャット API の失敗。
    #[error("{0}")]
    Chat(String),
}

/// このモジュールの結果。
pub type Result<T> = std::result::Result<T, SectionsError>;

/// `ensure` / `rebuild` の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildState {
    /// `extract.md` が前回と同じなので何もしなかった。
    Fresh,
    /// 作り直した。
    Rebuilt,
}

/// 分けた塊（保存前）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassageDraft {
    /// 節の中の番号（1 始まり）。
    pub pas_no: i64,
    /// 本文。
    pub text: String,
    /// 文の数。
    pub n_sentences: i64,
    /// 画像を除いた本文の上の開始位置（バイト）。
    pub char_start: usize,
    /// 画像を除いた本文の上の終了位置（バイト）。
    pub char_end: usize,
}

/// 分けた節（保存前）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionDraft {
    /// 節の番号（見出しの順。最初の見出しより前は 0）。
    pub sec_no: i64,
    /// 見出しの文字。
    pub heading: String,
    /// 見出しの先頭の番号（`4.1`）。
    pub label: Option<String>,
    /// 番号の深さ。
    pub level: i64,
    /// `front` / `abstract` / `body` / `references`。
    pub kind: String,
    /// 塊。
    pub passages: Vec<PassageDraft>,
    /// 文の数。
    pub n_sentences: i64,
    /// 文字数。
    pub n_chars: i64,
    /// 最初の 2 文。
    pub lead: String,
}

// ---------------------------------------------------------------- 文の数え方

const ABBREVIATIONS: &[&str] = &["al", "e.g", "i.e", "fig", "eq", "no", "vs"];

/// 文の範囲（バイト）を返す。
fn sentence_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0usize;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (i, &(pos, ch)) in chars.iter().enumerate() {
        let end = pos + ch.len_utf8();
        let boundary = match ch {
            '。' | '！' | '？' => true,
            '.' | '?' | '!' => {
                let next_is_space = chars.get(i + 1).is_some_and(|(_, c)| c.is_whitespace());
                let next_head = chars[i + 1..]
                    .iter()
                    .map(|(_, c)| *c)
                    .find(|c| !c.is_whitespace());
                let starts_sentence = next_head.is_some_and(|c| {
                    c.is_uppercase()
                        || c.is_ascii_digit()
                        || matches!(c, '"' | '\'' | '“' | '‘' | '(' | '[' | '「' | '（')
                });
                next_is_space && starts_sentence && (ch != '.' || !is_abbreviation(&text[..pos]))
            }
            _ => false,
        };
        if boundary {
            ranges.push((start, end));
            start = end;
        }
    }
    if !text[start..].trim().is_empty() {
        ranges.push((start, text.len()));
    }
    // 先頭の空白は文に入れない。
    ranges
        .into_iter()
        .map(|(s, e)| {
            let lead_ws = text[s..e].len() - text[s..e].trim_start().len();
            (s + lead_ws, e)
        })
        .filter(|(s, e)| s < e)
        .collect()
}

/// `.` の直前の語が略語か、大文字 1 文字（著者のイニシャル）か。
fn is_abbreviation(before: &str) -> bool {
    let word: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '.')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut letters = word.chars();
    if let (Some(c), None) = (letters.next(), letters.next()) {
        if c.is_uppercase() {
            return true;
        }
    }
    ABBREVIATIONS.contains(&word.to_lowercase().as_str())
}

/// 文の数を数える。
#[must_use]
pub fn count_sentences(text: &str) -> usize {
    sentence_ranges(text).len()
}

/// 先頭から 2 文を取り、300 字を超えたら切る。
fn lead_of(text: &str) -> String {
    let ranges = sentence_ranges(text);
    let Some(&(start, _)) = ranges.first() else {
        return String::new();
    };
    let end = ranges.get(1).map_or(ranges[0].1, |r| r.1);
    let lead = text[start..end].trim();
    if lead.chars().count() > LEAD_MAX_CHARS {
        let mut cut: String = lead.chars().take(LEAD_MAX_CHARS).collect();
        cut.push('…');
        cut
    } else {
        lead.to_string()
    }
}

// ---------------------------------------------------------------- 分け方

struct RawPara {
    text: String,
    start: usize,
    end: usize,
}

fn is_small(text: &str) -> bool {
    text.trim() == "<!-- formula-not-decoded -->" || text.chars().count() < PASSAGE_MIN_CHARS
}

/// 段落をつないで、長すぎるものを文の切れ目で切る。
fn make_passages(paras: Vec<RawPara>) -> Vec<PassageDraft> {
    let mut merged: Vec<RawPara> = Vec::new();
    let mut pending: Option<RawPara> = None;
    for para in paras {
        let small = is_small(&para.text);
        let para = match pending.take() {
            Some(p) => RawPara {
                text: format!("{}\n\n{}", p.text, para.text),
                start: p.start,
                end: para.end,
            },
            None => para,
        };
        match (small, merged.last_mut()) {
            (true, Some(prev)) => {
                prev.text = format!("{}\n\n{}", prev.text, para.text);
                prev.end = para.end;
            }
            (true, None) => pending = Some(para),
            (false, _) => merged.push(para),
        }
    }
    if let Some(p) = pending {
        merged.push(p);
    }

    let mut out = Vec::new();
    for para in merged {
        for (text, start) in split_long(&para.text, para.start) {
            let end = start + text.len();
            out.push(PassageDraft {
                pas_no: out.len() as i64 + 1,
                n_sentences: count_sentences(&text) as i64,
                text,
                char_start: start,
                char_end: end,
            });
        }
    }
    out
}

/// 1500 字以下に分ける。文の切れ目で詰め、1 文で超えるものは字数で切る。
fn split_long(text: &str, base: usize) -> Vec<(String, usize)> {
    if text.chars().count() <= PASSAGE_MAX_CHARS {
        return vec![(text.to_string(), base)];
    }
    // 1500 字を超える文は字数で切って、文の列にする。
    let mut units: Vec<(usize, usize)> = Vec::new();
    for (s, e) in sentence_ranges(text) {
        let mut cur = s;
        while text[cur..e].chars().count() > PASSAGE_MAX_CHARS {
            let cut = cur
                + text[cur..e]
                    .char_indices()
                    .nth(PASSAGE_MAX_CHARS)
                    .map_or(e - cur, |(i, _)| i);
            units.push((cur, cut));
            cur = cut;
        }
        units.push((cur, e));
    }
    let mut out = Vec::new();
    let mut group: Option<(usize, usize)> = None;
    for (s, e) in units {
        group = match group {
            Some((gs, ge)) if text[gs..e].chars().count() <= PASSAGE_MAX_CHARS => {
                let _ = ge;
                Some((gs, e))
            }
            Some((gs, ge)) => {
                out.push((text[gs..ge].to_string(), base + gs));
                Some((s, e))
            }
            None => Some((s, e)),
        };
    }
    if let Some((gs, ge)) = group {
        out.push((text[gs..ge].to_string(), base + gs));
    }
    out
}

fn label_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(\d+(?:\.\d+)*)\.?\s+(.*)$").expect("正規表現は正しい"))
}

/// 見出しの先頭の番号を取る。`(label, level, 番号を除いた見出し)`。
fn split_label(heading: &str) -> (Option<String>, i64, String) {
    match label_regex().captures(heading) {
        Some(c) => {
            let label = c[1].to_string();
            let level = label.matches('.').count() as i64 + 1;
            (Some(label), level, c[2].to_string())
        }
        None => (None, 1, heading.to_string()),
    }
}

fn kind_of(rest: &str) -> Option<&'static str> {
    let key = rest
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    match key.as_str() {
        "abstract" => Some("abstract"),
        "references" | "bibliography" | "参考文献" => Some("references"),
        _ => None,
    }
}

/// Markdown を節と塊に分ける（DB を使わない）。
#[must_use]
pub fn build_from_markdown(text: &str) -> Vec<SectionDraft> {
    let body = remove_data_uri_images(text);

    // (heading, 段落) の並び。最初の要素は見出し前の本文（節 0）。
    let mut groups: Vec<(Option<String>, Vec<RawPara>)> = vec![(None, Vec::new())];
    let mut cur = String::new();
    let mut cur_start = 0usize;
    let mut offset = 0usize;
    let flush = |cur: &mut String, start: usize, groups: &mut Vec<(Option<String>, Vec<RawPara>)>| {
        if !cur.trim().is_empty() {
            let text = cur.trim_end().to_string();
            let end = start + text.len();
            groups.last_mut().expect("常に 1 つ以上ある").1.push(RawPara { text, start, end });
        }
        cur.clear();
    };
    for line in body.split_inclusive('\n') {
        let plain = line.trim_end_matches(['\n', '\r']);
        if plain.trim().is_empty() {
            flush(&mut cur, cur_start, &mut groups);
        } else if let Some((_, title)) = markdown_heading(plain) {
            flush(&mut cur, cur_start, &mut groups);
            groups.push((Some(title), Vec::new()));
        } else {
            if cur.is_empty() {
                cur_start = offset;
            }
            cur.push_str(line);
        }
        offset += line.len();
    }
    flush(&mut cur, cur_start, &mut groups);

    let abstract_at = groups.iter().position(|(h, _)| {
        h.as_deref()
            .is_some_and(|h| kind_of(&split_label(h).2) == Some("abstract"))
    });
    let mut sections = Vec::new();
    for (index, (heading, paras)) in groups.into_iter().enumerate() {
        let Some(heading) = heading else {
            if paras.is_empty() {
                continue;
            }
            sections.push(finish(0, String::new(), None, 0, "front", paras));
            continue;
        };
        let (label, level, rest) = split_label(&heading);
        let kind = kind_of(&rest).unwrap_or_else(|| {
            if abstract_at.is_some_and(|a| index < a) {
                "front"
            } else {
                "body"
            }
        });
        sections.push(finish(index as i64, heading, label, level, kind, paras));
    }
    sections
}

fn finish(
    sec_no: i64,
    heading: String,
    label: Option<String>,
    level: i64,
    kind: &str,
    paras: Vec<RawPara>,
) -> SectionDraft {
    let passages = make_passages(paras);
    SectionDraft {
        sec_no,
        heading,
        label,
        level,
        kind: kind.to_string(),
        n_sentences: passages.iter().map(|p| p.n_sentences).sum(),
        n_chars: passages.iter().map(|p| p.text.chars().count() as i64).sum(),
        lead: passages.first().map_or_else(String::new, |p| lead_of(&p.text)),
        passages,
    }
}

// ---------------------------------------------------------------- 保存

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn built_sha(conn: &Connection, paper_id: i64) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT extract_sha256 FROM paper_section_builds WHERE paper_id = ?1",
            [paper_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// 表が無ければ（または `extract.md` が変わっていれば）作る。
///
/// # Errors
///
/// `extract.md` が無いとき、DB・ファイルの読み書きに失敗したとき。
pub fn ensure(conn: &Connection, data_dir: &Path, paper_id: i64) -> Result<BuildState> {
    build(conn, data_dir, paper_id, false)
}

/// `extract.md` が同じでも作り直す。要約は消える。
///
/// # Errors
///
/// [`ensure`] と同じ。
pub fn rebuild(conn: &Connection, data_dir: &Path, paper_id: i64) -> Result<BuildState> {
    build(conn, data_dir, paper_id, true)
}

fn build(conn: &Connection, data_dir: &Path, paper_id: i64, force: bool) -> Result<BuildState> {
    let path = crate::citation::resolve_extract_md(conn, Some(paper_id), None, data_dir)
        .ok_or(SectionsError::NoExtract(paper_id))?;
    let bytes = std::fs::read(&path)?;
    let sha = sha256_hex(&bytes);
    if !force && built_sha(conn, paper_id)?.as_deref() == Some(sha.as_str()) {
        return Ok(BuildState::Fresh);
    }
    let sections = build_from_markdown(&crate::citation::docling::strip_figure_images(
        &String::from_utf8_lossy(&bytes),
    ));

    let tx = conn.unchecked_transaction()?;
    for table in ["paper_passages", "paper_sections", "paper_section_builds"] {
        tx.execute(&format!("DELETE FROM {table} WHERE paper_id = ?1"), [paper_id])?;
    }
    for s in &sections {
        tx.execute(
            "INSERT INTO paper_sections (paper_id, sec_no, heading, label, level, kind,
                 n_passages, n_sentences, n_chars, lead, summary)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
            params![
                paper_id,
                s.sec_no,
                s.heading,
                s.label,
                s.level,
                s.kind,
                s.passages.len() as i64,
                s.n_sentences,
                s.n_chars,
                s.lead
            ],
        )?;
        for p in &s.passages {
            tx.execute(
                "INSERT INTO paper_passages (paper_id, sec_no, pas_no, text, n_sentences,
                     char_start, char_end) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    paper_id,
                    s.sec_no,
                    p.pas_no,
                    p.text,
                    p.n_sentences,
                    p.char_start as i64,
                    p.char_end as i64
                ],
            )?;
        }
    }
    tx.execute(
        "INSERT INTO paper_section_builds (paper_id, extract_path, extract_sha256, built_at,
             summary_model) VALUES (?1, ?2, ?3, ?4, NULL)",
        params![
            paper_id,
            path.to_string_lossy(),
            sha,
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    tx.commit()?;
    Ok(BuildState::Rebuilt)
}

/// 論文の節の数（表が無ければ 0）。
///
/// # Errors
///
/// DB の失敗。
pub fn count_sections(conn: &Connection, paper_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM paper_sections WHERE paper_id = ?1",
        [paper_id],
        |r| r.get(0),
    )?)
}

// ---------------------------------------------------------------- ID

/// 読むときに受け付ける ID。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PassageRef {
    /// 論文全体（`37`）。
    Paper { paper_id: i64 },
    /// 節（`37.5`）。
    Section { paper_id: i64, sec_no: i64 },
    /// 塊（`37.5.2`）。
    Passage { paper_id: i64, sec_no: i64, pas_no: i64 },
    /// 同じ節の塊の範囲（`37.5.2-4`）。
    Range { paper_id: i64, sec_no: i64, from: i64, to: i64 },
}

impl PassageRef {
    /// 論文の番号（papers.id）。
    #[must_use]
    pub fn paper_id(&self) -> i64 {
        match *self {
            Self::Paper { paper_id }
            | Self::Section { paper_id, .. }
            | Self::Passage { paper_id, .. }
            | Self::Range { paper_id, .. } => paper_id,
        }
    }
}

impl std::fmt::Display for PassageRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Paper { paper_id } => write!(f, "#{paper_id}"),
            Self::Section { paper_id, sec_no } => write!(f, "#{paper_id}.{sec_no}"),
            Self::Passage { paper_id, sec_no, pas_no } => write!(f, "#{paper_id}.{sec_no}.{pas_no}"),
            Self::Range { paper_id, sec_no, from, to } => {
                write!(f, "#{paper_id}.{sec_no}.{from}-{to}")
            }
        }
    }
}

/// `37`・`37.5`・`37.5.2`・`37.5.2-4`（先頭の `#` は有っても無くてもよい）を読む。
///
/// # Errors
///
/// 形が違うとき（[`SectionsError::BadRef`]）。
pub fn parse_ref(s: &str) -> Result<PassageRef> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    let num = |t: &str| -> Result<i64> {
        if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
            return Err(SectionsError::BadRef);
        }
        t.parse().map_err(|_| SectionsError::BadRef)
    };
    let (head, to) = match s.split_once('-') {
        Some((h, t)) => (h, Some(num(t)?)),
        None => (s, None),
    };
    let parts: Vec<&str> = head.split('.').collect();
    match (parts.as_slice(), to) {
        ([p], None) => Ok(PassageRef::Paper { paper_id: num(p)? }),
        ([p, s], None) => Ok(PassageRef::Section { paper_id: num(p)?, sec_no: num(s)? }),
        ([p, s, a], None) => Ok(PassageRef::Passage {
            paper_id: num(p)?,
            sec_no: num(s)?,
            pas_no: num(a)?,
        }),
        ([p, s, a], Some(to)) => Ok(PassageRef::Range {
            paper_id: num(p)?,
            sec_no: num(s)?,
            from: num(a)?,
            to,
        }),
        _ => Err(SectionsError::BadRef),
    }
}

// ---------------------------------------------------------------- 読む

/// `outline` の戻り値の 1 節。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OutlineSection {
    /// `#37.5`。
    pub id: String,
    /// 節の番号。
    pub sec_no: i64,
    /// 見出し。
    pub heading: String,
    /// 見出しの番号。
    pub label: Option<String>,
    /// 深さ。
    pub level: i64,
    /// 種別。
    pub kind: String,
    /// 塊の数。
    pub n_passages: i64,
    /// 文の数。
    pub n_sentences: i64,
    /// 文字数。
    pub n_chars: i64,
    /// 最初の 2 文。
    pub lead: String,
    /// LLM の要約。
    pub summary: Option<String>,
}

/// `outline` の戻り値。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Outline {
    /// 論文 ID。
    pub paper_id: i64,
    /// 題名。
    pub title: String,
    /// `extract.md` の sha256 の先頭 12 文字。
    pub extract_sha256: String,
    /// 要約に使った model。
    pub summary_model: Option<String>,
    /// 節。
    pub sections: Vec<OutlineSection>,
}

/// 論文の節の一覧。
///
/// # Errors
///
/// [`ensure`] と同じ。
pub fn outline(conn: &Connection, data_dir: &Path, paper_id: i64) -> Result<Outline> {
    ensure(conn, data_dir, paper_id)?;
    let title: String = conn
        .query_row(
            "SELECT COALESCE(title, '') FROM papers WHERE id = ?1",
            [paper_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_default();
    let (sha, summary_model): (String, Option<String>) = conn.query_row(
        "SELECT extract_sha256, summary_model FROM paper_section_builds WHERE paper_id = ?1",
        [paper_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut stmt = conn.prepare(
        "SELECT sec_no, heading, label, level, kind, n_passages, n_sentences, n_chars, lead, summary
         FROM paper_sections WHERE paper_id = ?1 ORDER BY sec_no",
    )?;
    let sections = stmt
        .query_map([paper_id], |r| {
            let sec_no: i64 = r.get(0)?;
            Ok(OutlineSection {
                id: format!("#{paper_id}.{sec_no}"),
                sec_no,
                heading: r.get(1)?,
                label: r.get(2)?,
                level: r.get(3)?,
                kind: r.get(4)?,
                n_passages: r.get(5)?,
                n_sentences: r.get(6)?,
                n_chars: r.get(7)?,
                lead: r.get(8)?,
                summary: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Outline {
        paper_id,
        title,
        extract_sha256: sha.chars().take(12).collect(),
        summary_model,
        sections,
    })
}

/// `read` の戻り値の 1 塊。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReadPassage {
    /// `#37.5.1`。
    pub id: String,
    /// 論文 ID。
    pub paper_id: i64,
    /// `#37.5`。
    pub section: String,
    /// 節の見出し。
    pub heading: String,
    /// 本文。
    pub text: String,
    /// 文の数。
    pub n_sentences: i64,
}

/// `read` の戻り値。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReadResult {
    /// 読んだ ID（正規の形）。
    pub refs: Vec<String>,
    /// 字数の上限。
    pub max_chars: usize,
    /// 上限で止めたか。
    pub truncated: bool,
    /// 止めたときの、次に読む塊の ID。
    pub next: Option<String>,
    /// 塊。
    pub passages: Vec<ReadPassage>,
    /// 指定した節・論文が本文の無い節だった ID（図や表だけの節など）。
    pub empty_refs: Vec<String>,
    /// 読んだ塊が 1 つも無く、指定がすべて本文の無い節だったか。
    pub empty: bool,
}

/// 塊を読む。`max_chars` を超えたら、そこまでの塊で止めて `next` を返す。
///
/// # Errors
///
/// [`ensure`] と同じ。指定した ID の塊が無く、節としても無いとき（[`SectionsError::NotFound`]）。
/// 節はあるが塊が 0 件のときは `Err` にせず、[`ReadResult::empty_refs`] に入れる。
pub fn read(
    conn: &Connection,
    data_dir: &Path,
    refs: &[PassageRef],
    max_chars: usize,
) -> Result<ReadResult> {
    let mut all: Vec<ReadPassage> = Vec::new();
    let mut empty_refs = Vec::new();
    for r in refs {
        let paper_id = r.paper_id();
        ensure(conn, data_dir, paper_id)?;
        // 範囲の条件は NULL を使い、1 本の SQL にまとめる。
        let (sec, from, to) = match *r {
            PassageRef::Paper { .. } => (None, None, None),
            PassageRef::Section { sec_no, .. } => (Some(sec_no), None, None),
            PassageRef::Passage { sec_no, pas_no, .. } => (Some(sec_no), Some(pas_no), Some(pas_no)),
            PassageRef::Range { sec_no, from, to, .. } => (Some(sec_no), Some(from), Some(to)),
        };
        let mut stmt = conn.prepare(
            "SELECT p.sec_no, p.pas_no, s.heading, p.text, p.n_sentences
             FROM paper_passages p JOIN paper_sections s
               ON s.paper_id = p.paper_id AND s.sec_no = p.sec_no
             WHERE p.paper_id = ?1
               AND (?2 IS NULL OR p.sec_no = ?2)
               AND (?3 IS NULL OR p.pas_no >= ?3)
               AND (?4 IS NULL OR p.pas_no <= ?4)
             ORDER BY p.sec_no, p.pas_no",
        )?;
        let rows = stmt
            .query_map(params![paper_id, sec, from, to], |row| {
                let sec_no: i64 = row.get(0)?;
                let pas_no: i64 = row.get(1)?;
                Ok(ReadPassage {
                    id: format!("#{paper_id}.{sec_no}.{pas_no}"),
                    paper_id,
                    section: format!("#{paper_id}.{sec_no}"),
                    heading: row.get(2)?,
                    text: row.get(3)?,
                    n_sentences: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            // 節はあるが塊が 0 件（本文が空）なら、見つからないとは区別して返す。
            if section_exists(conn, r)? {
                empty_refs.push(r.to_string());
                continue;
            }
            return Err(SectionsError::NotFound(r.to_string()));
        }
        for row in rows {
            if !all.iter().any(|p| p.id == row.id) {
                all.push(row);
            }
        }
    }

    let mut passages = Vec::new();
    let mut used = 0usize;
    let mut next = None;
    for p in all {
        let len = p.text.chars().count();
        // 1 つ目は上限を超えていても必ず返す（返さないと進めない）。
        if !passages.is_empty() && used + len > max_chars {
            next = Some(p.id);
            break;
        }
        used += len;
        passages.push(p);
    }
    let empty = passages.is_empty() && !empty_refs.is_empty();
    Ok(ReadResult {
        refs: refs.iter().map(ToString::to_string).collect(),
        max_chars,
        truncated: next.is_some(),
        next,
        passages,
        empty_refs,
        empty,
    })
}

/// 節（`Section`）または論文（`Paper`）の表の行があるか。塊の数は見ない。
fn section_exists(conn: &Connection, r: &PassageRef) -> Result<bool> {
    let sec = match *r {
        PassageRef::Paper { .. } => None,
        PassageRef::Section { sec_no, .. } => Some(sec_no),
        PassageRef::Passage { .. } | PassageRef::Range { .. } => return Ok(false),
    };
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM paper_sections
           WHERE paper_id = ?1 AND (?2 IS NULL OR sec_no = ?2))",
        params![r.paper_id(), sec],
        |row| row.get(0),
    )?)
}

/// `extract.md` の画像を除いたあと、節・塊の表を作り直さずに済むよう、表の元の sha を新しい sha に付け替える。
/// 表は元の `extract.md` から作った中身と同じ（[`build_from_markdown`] が画像を除く）ので、要約も残す。
///
/// # Errors
///
/// DB の失敗。
pub fn rebind_extract_sha(conn: &Connection, paper_id: i64, old_sha: &str, new_sha: &str) -> Result<()> {
    conn.execute(
        "UPDATE paper_section_builds SET extract_sha256 = ?3
         WHERE paper_id = ?1 AND extract_sha256 = ?2",
        params![paper_id, old_sha, new_sha],
    )?;
    Ok(())
}

/// 節・塊があるか。その論文の表がまだ無ければ `None`（検査する側が飛ばす）。
///
/// # Errors
///
/// DB の失敗。
pub fn passage_exists(
    conn: &Connection,
    paper_id: i64,
    sec_no: i64,
    pas_no: Option<i64>,
) -> Result<Option<bool>> {
    if built_sha(conn, paper_id)?.is_none() {
        return Ok(None);
    }
    let found: i64 = match pas_no {
        Some(pas_no) => conn.query_row(
            "SELECT COUNT(*) FROM paper_passages WHERE paper_id = ?1 AND sec_no = ?2 AND pas_no = ?3",
            params![paper_id, sec_no, pas_no],
            |r| r.get(0),
        )?,
        None => conn.query_row(
            "SELECT COUNT(*) FROM paper_sections WHERE paper_id = ?1 AND sec_no = ?2",
            params![paper_id, sec_no],
            |r| r.get(0),
        )?,
    };
    Ok(Some(found > 0))
}

/// 検索結果の本文から塊の ID を引く。`chunk_id` が鍵。
///
/// RAG の `chunks.char_start` は段落の位置を正しく持たない（`split_markdown_paragraphs` は
/// 見出しと空行の長さだけ進める）ので、位置ではなく本文の先頭で塊を探す。
/// 表がまだ無い論文は飛ばす（検索を遅くしないため、ここでは `ensure` を呼ばない）。
#[must_use]
pub fn passages_for_hits(conn: &Connection, hits: &[Hit]) -> HashMap<i64, String> {
    let mut built: HashMap<i64, bool> = HashMap::new();
    let mut out = HashMap::new();
    for hit in hits {
        let Some(paper_id) = hit.paper_id else {
            continue;
        };
        if hit.source_kind != "body" {
            continue;
        }
        let has_table = *built
            .entry(paper_id)
            .or_insert_with(|| built_sha(conn, paper_id).ok().flatten().is_some());
        if !has_table {
            continue;
        }
        // 先頭 120 字（段落をまたぐ塊でも先頭の段落は 1 つの塊に入る）。無ければ先頭の行だけで探す。
        let text = hit.text.trim_start_matches(|c: char| c == '#' || c.is_whitespace());
        let head: String = text.chars().take(120).collect();
        let first_line: String = text.lines().next().unwrap_or("").chars().take(120).collect();
        let found = [head, first_line].iter().filter(|k| !k.trim().is_empty()).find_map(|key| {
            conn.query_row(
                "SELECT sec_no, pas_no FROM paper_passages
                 WHERE paper_id = ?1 AND instr(text, ?2) > 0
                 ORDER BY sec_no, pas_no LIMIT 1",
                params![paper_id, key],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()
            .ok()
            .flatten()
        });
        if let Some((sec_no, pas_no)) = found {
            out.insert(hit.chunk_id, format!("#{paper_id}.{sec_no}.{pas_no}"));
        }
    }
    out
}

// ---------------------------------------------------------------- 要約（任意）

/// 要約する節: `(sec_no, 見出し, 全文)`。
fn summary_targets(conn: &Connection, paper_id: i64) -> Result<Vec<(i64, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT sec_no, heading FROM paper_sections
         WHERE paper_id = ?1 AND kind IN ('abstract', 'body') AND n_chars >= ?2
           AND summary IS NULL ORDER BY sec_no",
    )?;
    let secs = stmt
        .query_map(params![paper_id, SUMMARY_MIN_CHARS], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for (sec_no, heading) in secs {
        let mut stmt = conn.prepare(
            "SELECT text FROM paper_passages WHERE paper_id = ?1 AND sec_no = ?2 ORDER BY pas_no",
        )?;
        let texts = stmt
            .query_map(params![paper_id, sec_no], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let full: String = texts.join("\n\n").chars().take(SUMMARY_INPUT_CHARS).collect();
        out.push((sec_no, heading, full));
    }
    Ok(out)
}

async fn run_summaries(
    targets: Vec<(i64, String, String)>,
    client: &dyn ChatClient,
    max_concurrent: usize,
) -> Result<Vec<(i64, String)>> {
    let results: Vec<(i64, std::result::Result<String, String>)> =
        stream::iter(targets.into_iter().map(|(sec_no, heading, full)| async move {
            let user = format!("見出し: {heading}\n\n{full}");
            let reply = client
                .chat(SUMMARY_SYSTEM, &user)
                .await
                .map(|r| r.content.trim().to_string())
                .map_err(|e| e.to_string());
            (sec_no, reply)
        }))
        .buffer_unordered(max_concurrent.max(1))
        .collect()
        .await;
    let mut done = Vec::new();
    for (sec_no, reply) in results {
        let text = reply.map_err(SectionsError::Chat)?;
        if !text.is_empty() {
            done.push((sec_no, text));
        }
    }
    Ok(done)
}

fn save_summaries(
    conn: &Connection,
    paper_id: i64,
    done: &[(i64, String)],
    model_name: &str,
) -> Result<usize> {
    for (sec_no, text) in done {
        conn.execute(
            "UPDATE paper_sections SET summary = ?3 WHERE paper_id = ?1 AND sec_no = ?2",
            params![paper_id, sec_no, text],
        )?;
    }
    if !done.is_empty() {
        conn.execute(
            "UPDATE paper_section_builds SET summary_model = ?2 WHERE paper_id = ?1",
            params![paper_id, model_name],
        )?;
    }
    Ok(done.len())
}

/// 節の要約を作る。条件: `kind` が `abstract` か `body`、`n_chars >= 200`、要約がまだ無い。
/// 要約した節の数を返す。
///
/// # Errors
///
/// DB の失敗、チャット API の失敗。
pub async fn summarize(
    conn: &Connection,
    paper_id: i64,
    client: &dyn ChatClient,
    model_name: &str,
    max_concurrent: usize,
) -> Result<usize> {
    let targets = summary_targets(conn, paper_id)?;
    let done = run_summaries(targets, client, max_concurrent).await?;
    save_summaries(conn, paper_id, &done, model_name)
}

/// [`summarize`] の、DB のファイルを渡す版。待っている間は接続を持たない
/// （`Connection` は `Sync` でないので、`tokio::spawn` の中から呼べるようにする）。
///
/// # Errors
///
/// [`summarize`] と同じ。
pub async fn summarize_file(
    db_file: &Path,
    paper_id: i64,
    client: &dyn ChatClient,
    model_name: &str,
    max_concurrent: usize,
) -> Result<usize> {
    let open = || {
        crate::database::Database::open(db_file).map_err(|e| SectionsError::Chat(e.to_string()))
    };
    let targets = summary_targets(open()?.connection(), paper_id)?;
    let done = run_summaries(targets, client, max_concurrent).await?;
    save_summaries(open()?.connection(), paper_id, &done, model_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::evidence::{ChatReply, EvidenceError};

    const B64: &str = "AAAABBBBCCCCDDDDEEEEFFFF";

    fn sample() -> String {
        let long1 = "Convolutional networks have been used widely in vision tasks. We apply them to text and report the results here. ".repeat(2);
        let long2 = "Results of our models against other methods are listed in table 2. Our baseline model is competitive. The multichannel variant helps on small data sets. The gains are consistent across all seven benchmarks in this work.";
        format!(
            "## Convolutional Neural Networks for Sentence Classification\n\n\
             Yoon Kim, New York University, a paper about CNNs for sentences and more words here.\n\n\
             ## Abstract\n\n\
             {long1}\n\n\
             ## 1 Introduction\n\n\
             ![img](data:image/png;base64,{B64})\n\n\
             {long1}\n\n\
             <!-- formula-not-decoded -->\n\n\
             short.\n\n\
             ## 2.1. Model\n\n\
             {long2}\n\n\
             ## References\n\n\
             [1] Kim et al. 2014. A very long reference entry that goes beyond one hundred characters for sure, yes.\n"
        )
    }

    #[test]
    fn count_sentences_skips_abbreviations() {
        assert_eq!(
            count_sentences("Kim et al. (2014) proposed a CNN. It works well. See Fig. 1 for details."),
            3
        );
        assert_eq!(count_sentences("A. Smith wrote this. It is e.g. fine."), 2);
        assert_eq!(count_sentences("これは文。もう一文！"), 2);
    }

    #[test]
    fn build_from_markdown_splits_sections() {
        let s = build_from_markdown(&sample());
        let heads: Vec<_> = s.iter().map(|x| (x.sec_no, x.heading.as_str(), x.kind.as_str())).collect();
        assert_eq!(
            heads,
            vec![
                (1, "Convolutional Neural Networks for Sentence Classification", "front"),
                (2, "Abstract", "abstract"),
                (3, "1 Introduction", "body"),
                (4, "2.1. Model", "body"),
                (5, "References", "references"),
            ]
        );
        assert_eq!(s[2].label.as_deref(), Some("1"));
        assert_eq!(s[2].level, 1);
        assert_eq!(s[3].label.as_deref(), Some("2.1"));
        assert_eq!(s[3].level, 2);
        assert_eq!(s[1].label, None);
        // 画像は消え、式と短い段落は前の塊につながる。
        assert_eq!(s[2].passages.len(), 1);
        assert!(s[2].passages[0].text.contains("formula-not-decoded"));
        assert!(s.iter().all(|x| x.passages.iter().all(|p| !p.text.contains(B64))));
        assert_eq!(s[3].passages[0].n_sentences, 4);
        assert_eq!(s[3].lead, "Results of our models against other methods are listed in table 2. Our baseline model is competitive.");
    }

    #[test]
    fn long_paragraph_is_split() {
        let para = "This is a sentence of moderate length for splitting. ".repeat(60);
        let s = build_from_markdown(&format!("## A\n\n{para}\n"));
        assert!(s[0].passages.len() >= 2);
        assert!(s[0].passages.iter().all(|p| p.text.chars().count() <= 1500));
    }

    #[test]
    fn parse_ref_forms() {
        assert_eq!(parse_ref(" #37 ").unwrap(), PassageRef::Paper { paper_id: 37 });
        assert_eq!(parse_ref("37.5").unwrap(), PassageRef::Section { paper_id: 37, sec_no: 5 });
        assert_eq!(
            parse_ref("#37.5.2").unwrap(),
            PassageRef::Passage { paper_id: 37, sec_no: 5, pas_no: 2 }
        );
        assert_eq!(
            parse_ref("37.5.2-4").unwrap(),
            PassageRef::Range { paper_id: 37, sec_no: 5, from: 2, to: 4 }
        );
        assert!(matches!(parse_ref("#37.5.x"), Err(SectionsError::BadRef)));
        assert!(matches!(parse_ref("37.5-4"), Err(SectionsError::BadRef)));
    }

    /// 論文 1 本と extract.md を持つ一時フォルダ付きの DB を作る。
    fn setup(name: &str, md: &str) -> (Database, std::path::PathBuf, i64) {
        let dir = std::env::temp_dir().join(format!("sections-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::in_memory_with_data_dir(&dir).unwrap();
        let id = db.insert_paper("Test paper", None).unwrap();
        let md_path = dir.join("extract.md");
        std::fs::write(&md_path, md).unwrap();
        db.connection()
            .execute(
                "INSERT INTO paper_extracts (paper_id, status, md_path, papers_id)
                 VALUES ('k', 'ready', ?1, ?2)",
                params![md_path.to_string_lossy(), id],
            )
            .unwrap();
        (db, dir, id)
    }

    #[test]
    fn ensure_is_stable_and_detects_change() {
        let (db, dir, id) = setup("ensure", &sample());
        let conn = db.connection();
        assert_eq!(ensure(conn, &dir, id).unwrap(), BuildState::Rebuilt);
        let first = read(conn, &dir, &[PassageRef::Paper { paper_id: id }], 1_000_000).unwrap();
        assert_eq!(ensure(conn, &dir, id).unwrap(), BuildState::Fresh);
        assert_eq!(rebuild(conn, &dir, id).unwrap(), BuildState::Rebuilt);
        let second = read(conn, &dir, &[PassageRef::Paper { paper_id: id }], 1_000_000).unwrap();
        assert_eq!(first, second);

        std::fs::write(dir.join("extract.md"), sample().replace("Results of our", "Changed our")).unwrap();
        assert_eq!(ensure(conn, &dir, id).unwrap(), BuildState::Rebuilt);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_forms_and_truncation() {
        let (db, dir, id) = setup("read", &sample());
        let conn = db.connection();
        let o = outline(conn, &dir, id).unwrap();
        assert_eq!(o.sections.len(), 5);
        assert_eq!(o.sections[3].id, format!("#{id}.4"));

        let one = read(conn, &dir, &[parse_ref(&format!("{id}.4.1")).unwrap()], 12000).unwrap();
        assert_eq!(one.passages.len(), 1);
        assert_eq!(one.passages[0].id, format!("#{id}.4.1"));
        let range = read(conn, &dir, &[parse_ref(&format!("{id}.4.1-3")).unwrap()], 12000).unwrap();
        assert_eq!(range.passages.len(), 1);
        let sec = read(conn, &dir, &[parse_ref(&format!("{id}.2")).unwrap()], 12000).unwrap();
        assert_eq!(sec.passages[0].heading, "Abstract");

        let all = read(conn, &dir, &[PassageRef::Paper { paper_id: id }], 100).unwrap();
        assert!(all.truncated);
        assert_eq!(all.passages.len(), 1);
        assert_eq!(all.next.as_deref(), Some(format!("#{id}.2.1").as_str()));
        assert!(matches!(
            read(conn, &dir, &[parse_ref(&format!("{id}.9")).unwrap()], 100),
            Err(SectionsError::NotFound(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 本文の無い節は `NotFound` にせず `empty_refs` に入る。存在しない節は今まで通り `NotFound`。
    #[test]
    fn read_reports_empty_section_apart_from_missing() {
        let md = "## Empty paper\n\n## 1 Figures only\n\n## 2 Text\n\nSome body text that is long enough to be a passage here.\n";
        let (db, dir, id) = setup("empty-section", md);
        let conn = db.connection();
        ensure(conn, &dir, id).unwrap();
        let sec_of = |heading: &str| -> i64 {
            conn.query_row(
                "SELECT sec_no FROM paper_sections WHERE paper_id = ?1 AND heading LIKE ?2",
                params![id, format!("%{heading}%")],
                |r| r.get(0),
            )
            .unwrap()
        };
        let (empty, text) = (sec_of("Figures only"), sec_of("Text"));

        let r = read(conn, &dir, &[PassageRef::Section { paper_id: id, sec_no: empty }], 1000).unwrap();
        assert!(r.empty);
        assert!(r.passages.is_empty());
        assert_eq!(r.empty_refs, vec![format!("#{id}.{empty}")]);

        let mixed = [
            PassageRef::Section { paper_id: id, sec_no: empty },
            PassageRef::Section { paper_id: id, sec_no: text },
        ];
        let r = read(conn, &dir, &mixed, 1000).unwrap();
        assert!(!r.empty);
        assert!(!r.passages.is_empty());
        assert_eq!(r.empty_refs, vec![format!("#{id}.{empty}")]);

        let missing = read(conn, &dir, &[PassageRef::Section { paper_id: id, sec_no: 99 }], 1000);
        assert!(matches!(missing, Err(SectionsError::NotFound(_))));
    }

    #[test]
    fn passages_for_hits_by_text() {
        let (db, dir, id) = setup("hits", &sample());
        let conn = db.connection();
        let hit_with = |n: i64, text: &str| Hit {
            score: 1.0,
            chunk_id: n,
            source_kind: "body".into(),
            source_ref: String::new(),
            paper_id: Some(id),
            title: None,
            authors: None,
            year: None,
            text: text.into(),
            // RAG の char_start は当てにならない値でも引けること。
            char_start: Some(0),
            char_end: None,
            md_path: None,
            section: None,
        };
        let probe = hit_with(1, "Results of our models");
        // 表がまだ無いので飛ばす。
        assert!(passages_for_hits(conn, &[probe]).is_empty());
        ensure(conn, &dir, id).unwrap();
        let hits = [hit_with(7, "Results of our models against other methods"), hit_with(8, "存在しない文")];
        let map = passages_for_hits(conn, &hits);
        assert_eq!(map[&7], format!("#{id}.4.1"));
        assert!(!map.contains_key(&8));

        let json = crate::retrieval::hits_to_json_with(&hits, &map);
        assert_eq!(json[0]["passage"], format!("#{id}.4.1"));
        assert_eq!(json[0]["cite"], format!("[#{id}.4.1]"));
        let json = crate::retrieval::hits_to_json_with(&hits, &HashMap::new());
        assert!(json[0]["passage"].is_null());
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct FakeChat;
    #[async_trait::async_trait]
    impl ChatClient for FakeChat {
        async fn chat(&self, _system: &str, user: &str) -> std::result::Result<ChatReply, EvidenceError> {
            Ok(ChatReply { content: format!("要約:{}", user.lines().next().unwrap_or("")), ..Default::default() })
        }
    }

    #[tokio::test]
    async fn summarize_only_target_sections() {
        let (db, dir, id) = setup("sum", &sample());
        let conn = db.connection();
        ensure(conn, &dir, id).unwrap();
        let n = summarize(conn, id, &FakeChat, "fake-model", 2).await.unwrap();
        // abstract・Introduction・Model の 3 節。題名（front）と References は対象外。
        assert_eq!(n, 3);
        let o = outline(conn, &dir, id).unwrap();
        assert_eq!(o.summary_model.as_deref(), Some("fake-model"));
        assert_eq!(o.sections[1].summary.as_deref(), Some("要約:見出し: Abstract"));
        assert!(o.sections[0].summary.is_none() && o.sections[4].summary.is_none());
        assert_eq!(summarize(conn, id, &FakeChat, "fake-model", 2).await.unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
