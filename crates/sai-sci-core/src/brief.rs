//! エージェントに渡す「プロジェクトの状況のまとめ」（brief）を作る。
//!
//! LLM のエージェントは記憶を持たない。研究用のブロック（`paper-*`）が起動する
//! たびに、mind.md・state.md・最近の引き継ぎ・最近のユーザーの review を 1 つの
//! md にして渡す。それがこのモジュールの出力である。
//!
//! ここには **データだけ** を入れる。エージェントへの決まり（引き継ぎ節の書き方
//! など）は sai-link の md テンプレート `research-core.md` に書く（ユーザーが自分で
//! 直せるようにするため）。コードの中に指示の文章を持たない。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sai_paper_core::config::BriefConfig;
use sai_paper_core::paths::{PaperRoots, mind_path, note_count, state_path};
use crate::reports::{IssueCounts, issue_counts, list_reports};
use crate::runs::{RunDir, is_research_block, list_run_dirs, run_block_id, run_tag};
use crate::understanding::{self, UnderstandingEntry};

/// 引き継ぎ節に書ける項目名（`sai-docs.md` の共有契約）。
const HANDOFF_ITEMS: [&str; 6] = [
    "決めたこと",
    "分かったこと",
    "未解決",
    "使った論文",
    "ユーザーへの質問",
    "現況の更新案",
];

/// 実行フォルダ直下で「引き継ぎ」を探すときに除く md（役割が決まっている）。
const SKIPPED_FILES: [&str; 4] = ["task.md", "review.md", "message.md", "finalize-report.md"];

/// 研究用ブロックの実行が書いた引き継ぎ 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffEntry {
    /// 実行 ID（フォルダ名の数字）。
    pub run_id: i64,
    /// 実行したブロック ID。
    pub block_id: String,
    /// 実行のタグ（仮説の名前など）。無ければ `None`。
    pub tag: Option<String>,
    /// 引き継ぎが書かれた md の絶対パス。
    pub file: PathBuf,
    /// md の更新時刻。
    pub modified: SystemTime,
    /// 引き継ぎ節の本文（`## 引き継ぎ` の行は含まない）。
    pub raw: String,
    /// 読めた項目 `(項目名, 値)`。値が空の項目も入る。
    pub items: Vec<(String, String)>,
    /// 項目が 1 つ以上読めたか。`false` は「形が読めない引き継ぎ」。
    pub readable: bool,
}

impl HandoffEntry {
    /// 「現況の更新案」が空でも「なし」でもなければ、その案を返す。
    #[must_use]
    pub fn state_update(&self) -> Option<&str> {
        self.items
            .iter()
            .find(|(name, _)| name == "現況の更新案")
            .map(|(_, value)| value.trim())
            .filter(|value| !value.is_empty() && *value != "なし")
    }
}

/// 研究用ブロックの実行が書いた review 1 件（記入済みのものだけ）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewEntry {
    /// 実行 ID（フォルダ名の数字）。
    pub run_id: i64,
    /// 実行したブロック ID。
    pub block_id: String,
    /// `review.md` の絶対パス。
    pub file: PathBuf,
    /// `review.md` の更新時刻。
    pub modified: SystemTime,
    /// frontmatter を外した本文（`###` 見出しはそのまま）。
    pub body: String,
}

/// 研究用ブロックの実行の引き継ぎを新しい順に最大 `limit` 件返す。
///
/// 拾うのは実行フォルダ直下の `*.md` から [`SKIPPED_FILES`] を除いたもののうち、
/// `## 引き継ぎ` の見出しがあるもの。形が読めない節も `readable == false` として
/// 返す（黙って捨てないため。表示の取舍は [`build_brief`] が行う）。
///
/// `limit` が 0 なら空を返す。
#[must_use]
pub fn recent_handoffs(root: &Path, limit: usize) -> Vec<HandoffEntry> {
    if limit == 0 {
        return Vec::new();
    }
    let runs = research_runs(root);
    let mut entries: Vec<HandoffEntry> = Vec::new();

    for (run, block_id) in &runs {
        let tag = run_tag(root, run);
        let Ok(files) = fs::read_dir(&run.path) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if !path.is_file() || path.extension().is_none_or(|ext| ext != "md") {
                continue;
            }
            let name = file.file_name().to_string_lossy().to_string();
            if SKIPPED_FILES.contains(&name.as_str()) {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Some(raw) = extract_handoff_section(&text) else {
                continue;
            };
            let items = parse_handoff_items(&raw);
            entries.push(HandoffEntry {
                run_id: run.run_id,
                block_id: block_id.clone(),
                tag: tag.clone(),
                file: path.clone(),
                modified: modified_at(&path),
                readable: !items.is_empty(),
                raw,
                items,
            });
        }
    }

    sort_newest_first(&mut entries, |entry| {
        (entry.modified, entry.run_id, entry.file.clone())
    });
    entries.truncate(limit);
    entries
}

/// 研究用ブロックの実行の review を新しい順に最大 `limit` 件返す。
///
/// `###` の見出しの下に空白以外の文字がある（記入済みの）`review.md` だけを拾う。
/// frontmatter は外す。`limit` が 0 なら空を返す。
#[must_use]
pub fn recent_reviews(root: &Path, limit: usize) -> Vec<ReviewEntry> {
    if limit == 0 {
        return Vec::new();
    }
    let runs = research_runs(root);
    let mut entries: Vec<ReviewEntry> = Vec::new();

    for (run, block_id) in &runs {
        let path = run.path.join("review.md");
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let body = strip_frontmatter(&text).trim().to_string();
        if !review_has_content(&body) {
            continue;
        }
        entries.push(ReviewEntry {
            run_id: run.run_id,
            block_id: block_id.clone(),
            file: path.clone(),
            modified: modified_at(&path),
            body,
        });
    }

    sort_newest_first(&mut entries, |entry| {
        (entry.modified, entry.run_id, entry.file.clone())
    });
    entries.truncate(limit);
    entries
}

/// 研究用ブロックの実行だけを `(実行, ブロック ID)` の形で返す。
///
/// ブロック定義の読み込みは同じブロックで何度も起きるので `HashMap` に覚える。
fn research_runs(root: &Path) -> Vec<(RunDir, String)> {
    let mut cache: HashMap<String, bool> = HashMap::new();
    let mut runs = Vec::new();
    for run in list_run_dirs(root) {
        let Some(block_id) = run_block_id(root, &run) else {
            continue;
        };
        let research = *cache
            .entry(block_id.clone())
            .or_insert_with(|| is_research_block(root, &block_id));
        if research {
            runs.push((run, block_id));
        }
    }
    runs
}

/// 新しい順（更新時刻 → 実行 ID → パス）に並べ替える。
fn sort_newest_first<T, F, K>(entries: &mut [T], key: F)
where
    F: Fn(&T) -> K,
    K: Ord,
{
    entries.sort_by_key(|entry| std::cmp::Reverse(key(entry)));
}

/// `## 引き継ぎ` の次の行から、次の `## `（`###` は含まない）またはファイルの終わりまで。
///
/// 見出しが無ければ `None`。コード欄（```` ``` ```` / `~~~`）の中の `## ` は区切りと
/// みなさない。コード欄の中の `## 引き継ぎ` も見出しとみなさない。
fn extract_handoff_section(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut in_code = false;
    let mut start: Option<usize> = None;
    let mut end = lines.len();

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if start.is_none() {
            if trimmed == "## 引き継ぎ" {
                start = Some(index + 1);
            }
        } else if trimmed.starts_with("## ") {
            end = index;
            break;
        }
    }

    let start = start?;
    let body = lines[start..end].join("\n");
    Some(body.trim_matches('\n').to_string())
}

/// 引き継ぎ節の本文から `(項目名, 値)` を読む。
///
/// 項目名は [`HANDOFF_ITEMS`] だけ（コロンは全角 `：` も可）。値はその行のコロンの
/// 後ろと、続く字下げされた行（次の `- <項目名>:` まで）。
fn parse_handoff_items(raw: &str) -> Vec<(String, String)> {
    let mut items: Vec<(String, String)> = Vec::new();
    let mut current: Option<usize> = None;
    let mut in_code = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            current = None;
            continue;
        }
        if in_code {
            continue;
        }
        if let Some((name, value)) = handoff_item_line(trimmed) {
            items.push((name.to_string(), value.to_string()));
            current = Some(items.len() - 1);
            continue;
        }
        if trimmed.is_empty() {
            current = None;
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        match current {
            Some(index) if indented => {
                if items[index].1.is_empty() {
                    items[index].1 = trimmed.to_string();
                } else {
                    items[index].1.push(' ');
                    items[index].1.push_str(trimmed);
                }
            }
            _ => current = None,
        }
    }
    items
}

/// `- 決めたこと: 値` の行を `(項目名, 値)` にする。項目名が既知でなければ `None`。
fn handoff_item_line(trimmed: &str) -> Option<(&str, &str)> {
    let rest = trimmed.strip_prefix("- ")?;
    let (index, separator) = rest
        .char_indices()
        .find(|(_, ch)| *ch == ':' || *ch == '：')?;
    let name = rest[..index].trim();
    if !HANDOFF_ITEMS.contains(&name) {
        return None;
    }
    Some((name, rest[index + separator.len_utf8()..].trim()))
}

/// `review.md` の frontmatter（先頭の `---` から次の `---` まで）を外す。
fn strip_frontmatter(text: &str) -> &str {
    let first_end = text.find('\n').map_or(text.len(), |index| index + 1);
    if text[..first_end].trim() != "---" {
        return text;
    }
    let mut offset = first_end;
    for line in text[first_end..].split_inclusive('\n') {
        offset += line.len();
        if line.trim() == "---" {
            break;
        }
    }
    text[offset..].trim_start_matches('\n')
}

/// `###` の見出しの下に空白以外の文字があるか。
fn review_has_content(body: &str) -> bool {
    let mut in_section = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("### ") {
            in_section = true;
            continue;
        }
        if trimmed.starts_with("## ") {
            in_section = false;
            continue;
        }
        if in_section && !trimmed.is_empty() {
            return true;
        }
    }
    false
}

/// brief 全体を組み立てる。
///
/// 並びは 現況（state.md）→ ユーザーの考え（mind.md）→ 最近の引き継ぎ →
/// 最近のユーザーのレビュー → 数字。全体が `cfg.max_chars` を超えたら、
/// 古い review → 古い引き継ぎの順に 1 件ずつ外す（[`BriefConfig`] を参照）。
///
/// `conn` は `papr.db`。開いていない（`None`）ときは数字の行を
/// 「（papr.db を開けません）」にして、ほかは出す。brief の失敗でエージェントの
/// 起動を止めないため、ここはエラーを返さない。
#[must_use]
pub fn build_brief(
    roots: &PaperRoots,
    conn: Option<&rusqlite::Connection>,
    cfg: &BriefConfig,
    default_index: &str,
) -> String {
    let state_file = state_path(&roots.data_dir);
    let mind_file = mind_path(&roots.data_dir);
    let state = truncate(
        &read_doc(&state_file, "（state.md がありません）"),
        cfg.state_chars,
        &rel_path(&roots.root, &state_file),
    );
    let mind = truncate(
        &read_doc(&mind_file, "（mind.md がありません）"),
        cfg.mind_chars,
        &rel_path(&roots.root, &mind_file),
    );

    let handoffs = recent_handoffs(&roots.root, cfg.handoffs);
    let reviews = recent_reviews(&roots.root, cfg.reviews);
    // 理解の状態（トピックごとの最新 1 件）。DB が無いときは空。
    let understanding_entries = conn
        .map(|conn| understanding::latest_per_topic(conn).unwrap_or_default())
        .unwrap_or_default();
    // 引き継ぎの見出しに添える出典の要確認（`(run_id, file_name)` で report を特定）。
    // DB が無いときは空のまま（件数を添えない）。
    let handoff_issues = conn.map(handoff_issue_counts).unwrap_or_default();
    let unreadable: Vec<String> = handoffs
        .iter()
        .filter(|entry| !entry.readable)
        .map(|entry| format!("run {}", entry.run_id))
        .collect();

    let mut kept_handoffs: Vec<&HandoffEntry> =
        handoffs.iter().filter(|entry| entry.readable).collect();
    let mut kept_reviews: Vec<&ReviewEntry> = reviews.iter().collect();

    let numbers = numbers_line(conn, &roots.data_dir, default_index);
    let mut dropped = 0usize;

    loop {
        let text = assemble(
            roots,
            cfg,
            &state,
            &mind,
            &kept_handoffs,
            &handoff_issues,
            &unreadable,
            &kept_reviews,
            &understanding_entries,
            &numbers,
            dropped,
        );
        if char_len(&text) <= cfg.max_chars || (kept_handoffs.is_empty() && kept_reviews.is_empty())
        {
            return text;
        }
        // 新しい順に並んでいるので、末尾が古い。
        if kept_reviews.pop().is_none() {
            kept_handoffs.pop();
        }
        dropped += 1;
    }
}

/// brief を `<root>/.sai/template/project-brief.md` に書く。
///
/// sai-link はテンプレート名 `project-brief` でこのファイルを読む。書きかけの
/// ファイルが読まれないよう、同じフォルダの一時ファイルに書いてから rename で
/// 置き換える（エージェントが並んで起動しても壊れないため）。
///
/// # Errors
///
/// `.sai/template/` の作成・書き込み・名前の変更に失敗したときに返る。
pub fn write_brief(roots: &PaperRoots, text: &str) -> std::io::Result<PathBuf> {
    let dir = roots.root.join(".sai").join("template");
    fs::create_dir_all(&dir)?;
    let path = dir.join("project-brief.md");
    let tmp = dir.join(format!("project-brief.md.tmp-{}", std::process::id()));
    fs::write(&tmp, text)?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// 理解の状態の節（1 件も無ければ空文字）。
///
/// 上限 `limit` を超えたら `ok` の古い方から 1 件ずつ外す（`vague` と
/// `unknown` は残す。理解の薄いトピックを次のエージェントが深掘りできるように）。
fn render_understanding(entries: &[UnderstandingEntry], limit: usize) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut kept: Vec<&UnderstandingEntry> = entries.iter().collect();
    loop {
        let text = format_understanding(&kept);
        if char_len(&text) <= limit || !kept.iter().any(|entry| entry.state == "ok") {
            return text;
        }
        // 並びは vague → unknown → ok なので、最後の `ok` が古い方。
        if let Some(index) = kept.iter().rposition(|entry| entry.state == "ok") {
            kept.remove(index);
        }
    }
}

/// 理解の状態の節を md にする。
fn format_understanding(entries: &[&UnderstandingEntry]) -> String {
    let mut out = String::from("## 理解の状態（sai-paper understanding）\n");
    for entry in entries {
        out.push_str(&format!(
            "- {}: {}",
            understanding::state_label(&entry.state),
            entry.topic
        ));
        if !entry.evidence.is_empty() {
            out.push_str(&format!(" — {}", entry.evidence));
        }
        if let Some(run_id) = entry.run_id {
            out.push_str(&format!("（run {run_id}）"));
        }
        out.push('\n');
    }
    out
}

/// 各節を並べて 1 つの md にする。
#[allow(clippy::too_many_arguments)]
fn assemble(
    roots: &PaperRoots,
    cfg: &BriefConfig,
    state: &str,
    mind: &str,
    handoffs: &[&HandoffEntry],
    handoff_issues: &HashMap<(String, String), IssueCounts>,
    unreadable: &[String],
    reviews: &[&ReviewEntry],
    understanding_entries: &[UnderstandingEntry],
    numbers: &str,
    dropped: usize,
) -> String {
    let mut out = String::new();
    out.push_str("<!-- sai-paper brief が実行のたびに作り直す。手で直しても上書きされる -->\n");
    out.push_str("# プロジェクトの状況（sai-paper より）\n\n");

    out.push_str("## 現況（.sai/paper/state.md）\n");
    out.push_str(state);
    out.push_str("\n\n");

    out.push_str("## ユーザーの考え（.sai/paper/mind.md）\n");
    out.push_str(mind);
    out.push_str("\n\n");

    out.push_str("## 最近の引き継ぎ（新しい順）\n");
    for entry in handoffs {
        out.push_str(&render_handoff(
            roots,
            entry,
            cfg.handoff_chars,
            handoff_issues,
        ));
        out.push('\n');
    }
    if !unreadable.is_empty() {
        out.push_str(&format!(
            "形が読めなかった引き継ぎ: {}\n",
            unreadable.join(", ")
        ));
    }
    out.push('\n');

    out.push_str("## 最近のユーザーのレビュー（新しい順）\n");
    for entry in reviews {
        out.push_str(&render_review(roots, entry, cfg.review_chars));
        out.push('\n');
    }
    out.push('\n');

    // 理解の状態の節（記録が無ければ節ごと出さない）。
    let understanding_section =
        render_understanding(understanding_entries, cfg.understanding_chars);
    if !understanding_section.is_empty() {
        out.push_str(&understanding_section);
        out.push('\n');
    }

    out.push_str("## 数字\n");
    out.push_str(numbers);
    out.push('\n');
    if dropped > 0 {
        out.push_str(&format!("（長さの上限で省いた記録: {dropped} 件）\n"));
    }
    out
}

/// `(run_id, file_name)` → その report の問題の件数。
///
/// 登録済みの report を新しい順に見て作る。ファイル名は `md_path` の末尾を使う
/// （実行フォルダが running → review → R<n> と動いてもファイル名は変わらない）。
fn handoff_issue_counts(conn: &rusqlite::Connection) -> HashMap<(String, String), IssueCounts> {
    let mut map = HashMap::new();
    let Ok(reports) = list_reports(conn, None) else {
        return map;
    };
    for report in reports {
        let Some(run_id) = report.run_id.clone() else {
            continue;
        };
        let Some(file_name) = std::path::Path::new(&report.md_path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
        else {
            continue;
        };
        let counts = issue_counts(conn, &report.id).unwrap_or_default();
        map.insert((run_id, file_name), counts);
    }
    map
}

/// 引き継ぎ 1 件に対応する report の問題の件数。
fn handoff_issue_counts_for<'a>(
    map: &'a HashMap<(String, String), IssueCounts>,
    entry: &HandoffEntry,
) -> Option<&'a IssueCounts> {
    let file_name = entry.file.file_name()?.to_string_lossy().to_string();
    map.get(&(entry.run_id.to_string(), file_name))
}

/// 引き継ぎ 1 件を md にする（上限を超えたら切って続きの場所を書く）。
///
/// 対応する report が登録済みで要確認（error）があれば、見出しの後ろに
/// `・出典の要確認 N 件` を添える。
fn render_handoff(
    roots: &PaperRoots,
    entry: &HandoffEntry,
    limit: usize,
    handoff_issues: &HashMap<(String, String), IssueCounts>,
) -> String {
    let rel = rel_path(&roots.root, &entry.file);
    let mut heading = match &entry.tag {
        Some(tag) => format!(
            "### run {} / {} / {} / {}（{}）",
            entry.run_id,
            entry.block_id,
            tag,
            format_time(entry.modified),
            rel
        ),
        None => format!(
            "### run {} / {} / {}（{}）",
            entry.run_id,
            entry.block_id,
            format_time(entry.modified),
            rel
        ),
    };
    if let Some(counts) = handoff_issue_counts_for(handoff_issues, entry)
        && counts.errors > 0
    {
        heading.push_str(&format!("・出典の要確認 {} 件", counts.errors));
    }
    let mut out = format!("{heading}\n");
    for (name, value) in &entry.items {
        if value.is_empty() {
            out.push_str(&format!("- {name}:\n"));
        } else {
            out.push_str(&format!("- {name}: {value}\n"));
        }
    }
    truncate(&out, limit, &rel)
}

/// review 1 件を md にする（`###` 見出しは 1 段下げる）。
fn render_review(roots: &PaperRoots, entry: &ReviewEntry, limit: usize) -> String {
    let rel = rel_path(&roots.root, &entry.file);
    let mut out = format!(
        "### run {} / {} / {}\n",
        entry.run_id,
        entry.block_id,
        format_time(entry.modified)
    );
    out.push_str(&demote_headings(&entry.body));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    truncate(&out, limit, &rel)
}

/// `###` で始まる行を `####` にする（brief の中では 1 段下げる）。
fn demote_headings(body: &str) -> String {
    let mut out = String::new();
    for line in body.lines() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = &line[indent..];
        out.push_str(&line[..indent]);
        if trimmed.starts_with("### ") {
            out.push('#');
        }
        out.push_str(trimmed);
        out.push('\n');
    }
    out
}

/// ファイルを読み、`<!-- -->` を除く。読めない・空なら `missing` を返す。
fn read_doc(path: &Path, missing: &str) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return missing.to_string();
    };
    let text = strip_comments(&text);
    let text = text.trim();
    if text.is_empty() {
        return missing.to_string();
    }
    text.to_string()
}

/// `<!-- ... -->`（行をまたぐものも）を除き、それが理由で空になった行は落とす。
fn strip_comments(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_comment = false;
    for raw in text.lines() {
        let had_comment = raw.contains("<!--") || raw.contains("-->");
        let mut rest = raw;
        let mut line = String::new();
        loop {
            if in_comment {
                match rest.find("-->") {
                    Some(index) => {
                        rest = &rest[index + 3..];
                        in_comment = false;
                    }
                    None => break,
                }
            } else {
                match rest.find("<!--") {
                    Some(index) => {
                        line.push_str(&rest[..index]);
                        rest = &rest[index + 4..];
                        in_comment = true;
                    }
                    None => {
                        line.push_str(rest);
                        break;
                    }
                }
            }
        }
        let line = line.trim_end().to_string();
        if line.trim().is_empty() && had_comment {
            continue;
        }
        out.push(line);
    }
    out.join("\n")
}

/// 数字の 1 行を作る。`conn` が無ければその旨を返す。
fn numbers_line(
    conn: Option<&rusqlite::Connection>,
    data_dir: &Path,
    default_index: &str,
) -> String {
    let Some(conn) = conn else {
        return "（papr.db を開けません）".to_string();
    };
    let papers: i64 = conn
        .query_row("SELECT COUNT(*) FROM papers", [], |row| row.get(0))
        .unwrap_or(0);
    let pdfs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM papers WHERE pdf_path IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let unindexed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM papers p WHERE NOT EXISTS ( \
             SELECT 1 FROM chunks c WHERE c.paper_id = p.id AND c.index_name = ?1)",
            rusqlite::params![default_index],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let notes = note_count(data_dir);
    format!(
        "論文 {papers} 本（PDF {pdfs}・note {notes}・未索引 {unindexed}）"
    )
}

/// 文字数が上限を超えていたら切り、末尾に続きの場所を付ける。
fn truncate(text: &str, limit: usize, rel: &str) -> String {
    if char_len(text) <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit).collect();
    format!("{head}\n…（続きは {rel}）")
}

/// 文字数（バイト数ではなく、`char` の数）。
fn char_len(text: &str) -> usize {
    text.chars().count()
}

/// `root` からの相対パス（区切りは `/`）。`root` の外なら絶対パス。
fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 更新時刻を `2026-09-24 21:10` の形にする（表示はローカル時刻）。
fn format_time(time: SystemTime) -> String {
    let datetime: chrono::DateTime<chrono::Local> = time.into();
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

/// ファイルの更新時刻（取れなければ UNIX エポック）。
fn modified_at(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の一時プロジェクトルート（`.sai/paper` と `.sai/outputs` 付き）。
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-brief-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".sai/paper")).unwrap();
        fs::create_dir_all(dir.join(".sai/outputs")).unwrap();
        dir
    }

    fn roots_of(root: &Path) -> PaperRoots {
        PaperRoots {
            root: root.to_path_buf(),
            data_dir: root.join(".sai/paper"),
        }
    }

    fn write(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// ブロック定義を書く。`templates` は frontmatter にそのまま入る。
    fn write_block(root: &Path, id: &str, templates: &str) {
        write(
            root.join(".sai/blocks").join(format!("{id}.md")),
            &format!("---\nname: {id}\ntemplates: {templates}\n---\n\n本文\n"),
        );
    }

    /// 実行フォルダを作る（`task.md` 付き）。
    fn write_run(root: &Path, owner: &str, run_id: i64, block: &str) {
        write(
            root.join(".sai/outputs")
                .join(owner)
                .join(run_id.to_string())
                .join("task.md"),
            &format!("---\nrun_id: {run_id}\nblock: {block}\n---\n\nタスク\n"),
        );
    }

    fn config(max_chars: usize) -> BriefConfig {
        BriefConfig {
            max_chars,
            ..BriefConfig::default()
        }
    }

    const HANDOFF: &str = "## 引き継ぎ\n- 決めたこと: 対象は頭皮脳波\n- 未解決: 前処理の手順\n";

    // ---- AC1: 引き継ぎ節の切り出し ----

    #[test]
    fn extract_stops_at_next_heading() {
        let text = "# 本文\n\n## 引き継ぎ\n- 決めたこと: A\n\n### 補足\n- 分かったこと: B\n\n## 次の節\n無視される\n";
        let raw = extract_handoff_section(text).unwrap();

        assert!(raw.contains("- 決めたこと: A"));
        assert!(raw.contains("### 補足"), "### は区切りにしない: {raw}");
        assert!(raw.contains("- 分かったこと: B"));
        assert!(!raw.contains("無視される"));
    }

    #[test]
    fn extract_runs_to_end_of_file() {
        let text = "## 引き継ぎ\n- 決めたこと: 最後まで\n";
        let raw = extract_handoff_section(text).unwrap();

        assert_eq!(raw, "- 決めたこと: 最後まで");
    }

    #[test]
    fn extract_ignores_headings_in_code() {
        let text = "## 引き継ぎ\n- 決めたこと: 例\n\n```markdown\n## これは区切りではない\n```\n\n- 未解決: 残る\n";
        let raw = extract_handoff_section(text).unwrap();

        assert!(raw.contains("## これは区切りではない"));
        assert!(raw.contains("- 未解決: 残る"));
    }

    #[test]
    fn extract_requires_heading() {
        assert!(extract_handoff_section("# 本文\n- 決めたこと: A\n").is_none());
        // コード欄の中の見出しは見出しとみなさない
        assert!(extract_handoff_section("```\n## 引き継ぎ\n- 決めたこと: A\n```\n").is_none());
    }

    #[test]
    fn parse_items_half_and_full_width() {
        let raw = "- 決めたこと：対象は頭皮脳波\n- 分かったこと: 公開データが多い\n  追加の行\n- 未解決:\n- 使った論文: #37, #40\n";
        let items = parse_handoff_items(raw);

        assert_eq!(
            items,
            vec![
                ("決めたこと".to_string(), "対象は頭皮脳波".to_string()),
                (
                    "分かったこと".to_string(),
                    "公開データが多い 追加の行".to_string()
                ),
                ("未解決".to_string(), String::new()),
                ("使った論文".to_string(), "#37, #40".to_string()),
            ]
        );
    }

    #[test]
    fn parse_items_ignores_unknown_names() {
        let items = parse_handoff_items("- メモ: A\n- 決めたこと: B\n");

        assert_eq!(items, vec![("決めたこと".to_string(), "B".to_string())]);
    }

    #[test]
    fn unreadable_section_is_marked() {
        let items = parse_handoff_items("これは文章で、項目の形ではない。\n");

        assert!(items.is_empty());
        assert!(
            !HandoffEntry {
                run_id: 1,
                block_id: "paper-search".to_string(),
                tag: None,
                file: PathBuf::from("x.md"),
                modified: SystemTime::UNIX_EPOCH,
                raw: String::new(),
                items,
                readable: false,
            }
            .readable
        );
    }

    // ---- AC2: 拾う対象 ----

    #[test]
    fn recent_handoffs_skips_role_files_and_dev_blocks() {
        let root = temp_root("targets");
        write_block(&root, "paper-search", "project-brief,research-core");
        write_block(&root, "main", "sai-link,sai-docs");
        write_run(&root, "review", 41, "paper-search");
        write_run(&root, "review", 42, "main");

        let dir = root.join(".sai/outputs/review/41");
        write(dir.join("answer.md"), HANDOFF);
        write(dir.join("task.md"), "---\nblock: paper-search\n---\n");
        write(
            dir.join("review.md"),
            "---\nblock: paper-search\n---\n\n### 追加実装\nn\n",
        );
        write(dir.join("message.md"), HANDOFF);
        write(dir.join("finalize-report.md"), HANDOFF);
        write(dir.join("notes.txt"), HANDOFF);

        // 開発用ブロックの実行にも引き継ぎを置く（拾ってはいけない）
        write(root.join(".sai/outputs/review/42/report.md"), HANDOFF);

        let handoffs = recent_handoffs(&root, 10);

        assert_eq!(handoffs.len(), 1, "{handoffs:?}");
        assert_eq!(handoffs[0].run_id, 41);
        assert_eq!(handoffs[0].block_id, "paper-search");
        assert!(handoffs[0].file.ends_with("answer.md"));
        assert!(handoffs[0].readable);
    }

    #[test]
    fn recent_handoffs_without_task_md_uses_link_db() {
        let root = temp_root("db-fallback");
        write_block(&root, "paper-library-qa", "project-brief");
        let dir = root.join(".sai/outputs/R9/45");
        write(dir.join("answer.md"), HANDOFF);
        crate::fake_link::write_runs(
            &root,
            &[crate::fake_link::run_value(
                45,
                "R9",
                &dir,
                "paper-library-qa",
                "",
                "completed",
            )],
        );

        let handoffs = recent_handoffs(&root, 10);

        assert_eq!(handoffs.len(), 1);
        assert_eq!(handoffs[0].block_id, "paper-library-qa");
    }

    #[test]
    fn recent_reviews_skips_empty_and_dev_blocks() {
        let root = temp_root("reviews");
        write_block(&root, "paper-search", "project-brief");
        write_block(&root, "sai2", "sai-link,sai-docs");
        write_run(&root, "review", 51, "paper-search");
        write_run(&root, "review", 52, "sai2");

        let empty = "---\nblock: paper-search\ndate: 2026-09-06 12:00:38\nrun_id: 51\n---\n\n### 追加修正\n\n\n### 追加実装\n\n";
        write(root.join(".sai/outputs/review/51/review.md"), empty);
        let filled =
            "---\nblock: sai2\ndate: 2026-09-06 12:01:21\nrun_id: 52\n---\n\n### 追加修正\nn\n";
        write(root.join(".sai/outputs/review/52/review.md"), filled);

        assert!(recent_reviews(&root, 10).is_empty());

        // 記入済みにすると拾う（frontmatter は外れる）
        write(
            root.join(".sai/outputs/review/51/review.md"),
            "---\nblock: paper-search\nrun_id: 51\n---\n\n### 追加修正\nn\n",
        );
        let reviews = recent_reviews(&root, 10);

        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].run_id, 51);
        assert_eq!(reviews[0].body, "### 追加修正\nn");
    }

    // ---- AC3: 順・件数・長さ ----

    #[test]
    fn newest_first_and_limit() {
        let root = temp_root("order");
        write_block(&root, "paper-search", "project-brief");
        for run_id in [61, 62, 63] {
            write_run(&root, "review", run_id, "paper-search");
            write(
                root.join(".sai/outputs/review")
                    .join(run_id.to_string())
                    .join("answer.md"),
                HANDOFF,
            );
        }

        let handoffs = recent_handoffs(&root, 2);

        assert_eq!(handoffs.len(), 2);
        assert_eq!(handoffs[0].run_id, 63);
        assert_eq!(handoffs[1].run_id, 62);
    }

    #[test]
    fn long_entry_is_cut_with_pointer() {
        let root = temp_root("cut");
        write_block(&root, "paper-search", "project-brief");
        write_run(&root, "review", 71, "paper-search");
        let long = format!("## 引き継ぎ\n- 決めたこと: {}\n", "あ".repeat(200));
        write(root.join(".sai/outputs/review/71/answer.md"), &long);

        let cfg = BriefConfig {
            handoff_chars: 80,
            ..BriefConfig::default()
        };
        let text = build_brief(&roots_of(&root), None, &cfg, "default");

        assert!(text.contains("…（続きは .sai/outputs/review/71/answer.md）"));
        assert!(text.contains("（papr.db を開けません）"));
        // 200 文字の「あ」がそのまま出てはいない
        assert!(!text.contains(&"あ".repeat(100)));
    }

    #[test]
    fn mind_comments_are_removed() {
        let root = temp_root("mind");
        write(
            root.join(".sai/paper/mind.md"),
            "# mind\n<!-- HIDDEN-COMMENT 消える -->\n\n## 大きな問い\n本物の考え\n",
        );
        write(
            root.join(".sai/paper/state.md"),
            "# 現況\n<!-- 段階は 探索 / 絞り込み のどれか -->\n\n- 目標: 発作予測\n",
        );

        let text = build_brief(&roots_of(&root), None, &config(8000), "default");

        assert!(!text.contains("HIDDEN-COMMENT"));
        assert!(text.contains("本物の考え"));
        assert!(text.contains("- 目標: 発作予測"));
    }

    #[test]
    fn missing_files_are_reported() {
        let root = temp_root("missing");
        let text = build_brief(&roots_of(&root), None, &config(8000), "default");

        assert!(text.contains("（state.md がありません）"));
        assert!(text.contains("（mind.md がありません）"));
    }

    #[test]
    fn state_is_cut_with_pointer() {
        let root = temp_root("state-cut");
        write(
            root.join(".sai/paper/state.md"),
            &format!("# 現況\n\n- 目標: {}\n", "い".repeat(200)),
        );
        let cfg = BriefConfig {
            state_chars: 30,
            ..BriefConfig::default()
        };

        let text = build_brief(&roots_of(&root), None, &cfg, "default");

        assert!(text.contains("…（続きは .sai/paper/state.md）"));
    }

    #[test]
    fn max_chars_drops_oldest_review_first() {
        let root = temp_root("drop");
        write_block(&root, "paper-search", "project-brief");
        for run_id in [81, 82] {
            write_run(&root, "review", run_id, "paper-search");
            write(
                root.join(".sai/outputs/review")
                    .join(run_id.to_string())
                    .join("answer.md"),
                HANDOFF,
            );
            write(
                root.join(".sai/outputs/review")
                    .join(run_id.to_string())
                    .join("review.md"),
                &format!("---\nblock: paper-search\n---\n\n### 追加実装\nrun {run_id} の要望\n"),
            );
        }

        let full = build_brief(&roots_of(&root), None, &config(usize::MAX), "default");
        let full_len = char_len(&full);
        // 1 件だけ外せば収まる長さ（外した review は見出しだけでも 20 文字以上ある）
        let text = build_brief(&roots_of(&root), None, &config(full_len - 1), "default");

        assert!(text.contains("（長さの上限で省いた記録: 1 件）"), "{text}");
        // 古い review（run 81）から外れる
        assert!(!text.contains("run 81 の要望"), "{text}");
        assert!(text.contains("run 82 の要望"), "{text}");
        // 引き継ぎは残る
        assert!(text.contains("### run 82 / paper-search"), "{text}");
        assert!(text.contains("### run 81 / paper-search"), "{text}");
        assert!(char_len(&text) <= full_len - 1);
    }

    #[test]
    fn max_chars_drops_handoff_when_no_reviews() {
        let root = temp_root("drop-handoff");
        write_block(&root, "paper-search", "project-brief");
        for run_id in [91, 92] {
            write_run(&root, "review", run_id, "paper-search");
            write(
                root.join(".sai/outputs/review")
                    .join(run_id.to_string())
                    .join("answer.md"),
                HANDOFF,
            );
        }

        let full = build_brief(&roots_of(&root), None, &config(usize::MAX), "default");
        let text = build_brief(
            &roots_of(&root),
            None,
            &config(char_len(&full) - 1),
            "default",
        );

        assert!(text.contains("（長さの上限で省いた記録: 1 件）"), "{text}");
        assert!(!text.contains("### run 91 / paper-search"), "{text}");
        assert!(text.contains("### run 92 / paper-search"), "{text}");
    }

    #[test]
    fn unreadable_handoff_is_reported_not_shown() {
        let root = temp_root("unreadable");
        write_block(&root, "paper-search", "project-brief");
        write_run(&root, "review", 101, "paper-search");
        write(
            root.join(".sai/outputs/review/101/answer.md"),
            "## 引き継ぎ\nこれは項目の形になっていない文章。\n",
        );

        let text = build_brief(&roots_of(&root), None, &config(8000), "default");

        assert!(text.contains("形が読めなかった引き継ぎ: run 101"), "{text}");
        assert!(!text.contains("これは項目の形になっていない文章。"));
    }

    // ---- N04: 理解の状態の節 ----

    #[test]
    fn understanding_section_is_ordered_and_absent_when_empty()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("understanding");
        let roots = roots_of(&root);

        // 記録が無ければ節ごと出さない。
        let text = build_brief(&roots, None, &config(8000), "default");
        assert!(!text.contains("理解の状態"), "{text}");

        let db = sai_paper_core::Database::in_memory()?;
        let conn = db.connection();
        crate::understanding::add(conn, "公開データセットの違い", "ok", "", None)?;
        crate::understanding::add(conn, "評価の分け方", "unknown", "", None)?;
        crate::understanding::add(
            conn,
            "前処理の違い",
            "vague",
            "ノッチフィルタの理由を説明できなかった",
            Some(52),
        )?;

        let text = build_brief(&roots, Some(conn), &config(8000), "default");
        let section = text
            .split("## 理解の状態（sai-paper understanding）")
            .nth(1)
            .unwrap_or_else(|| panic!("理解の状態の節が無い: {text}"));
        let vague = section
            .find("- あいまい: 前処理の違い")
            .unwrap_or_else(|| panic!("あいまいが無い: {section}"));
        let unknown = section
            .find("- まだ確かめていない: 評価の分け方")
            .unwrap_or_else(|| panic!("未確認が無い: {section}"));
        let ok = section
            .find("- 理解している: 公開データセットの違い")
            .unwrap_or_else(|| panic!("ok が無い: {section}"));
        assert!(vague < unknown && unknown < ok, "並びが違う: {section}");
        assert!(
            section.contains("ノッチフィルタの理由を説明できなかった（run 52）"),
            "根拠と run が出ない: {section}"
        );
        // 数字の節より前に出る。
        let section_pos = text.find("## 理解の状態").unwrap();
        let numbers_pos = text.find("## 数字").unwrap();
        assert!(section_pos < numbers_pos);
        Ok(())
    }

    #[test]
    fn understanding_section_drops_ok_when_over_limit() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("understanding-limit");
        let roots = roots_of(&root);
        let db = sai_paper_core::Database::in_memory()?;
        let conn = db.connection();
        crate::understanding::add(conn, "あいまいな話", "vague", "", None)?;
        crate::understanding::add(conn, "確かめていない話", "unknown", "", None)?;
        crate::understanding::add(conn, "理解している長い話", "ok", &"あ".repeat(120), None)?;

        // 全部入れると上限を超える長さ。ok を外せば収まる。
        let full = build_brief(&roots, Some(conn), &config(usize::MAX), "default");
        let section_len = full
            .split("## 理解の状態（sai-paper understanding）")
            .nth(1)
            .and_then(|rest| rest.split("## 数字").next())
            .map(char_len)
            .unwrap();
        assert!(section_len > 100, "前提が崩れた: {section_len}");

        let cfg = BriefConfig {
            understanding_chars: 100,
            ..BriefConfig::default()
        };
        let text = build_brief(&roots, Some(conn), &cfg, "default");

        // ok は古い方から外れ、vague と unknown は残る。
        assert!(!text.contains("理解している長い話"), "{text}");
        assert!(text.contains("- あいまい: あいまいな話"), "{text}");
        assert!(text.contains("- まだ確かめていない: 確かめていない話"), "{text}");
        Ok(())
    }

    // ---- AC4: 数字と書き出し ----

    #[test]
    fn numbers_line_counts_papers() {
        let root = temp_root("numbers");
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE papers(id INTEGER PRIMARY KEY, pdf_path TEXT);
             CREATE TABLE chunks(id INTEGER PRIMARY KEY, paper_id INTEGER, index_name TEXT NOT NULL);",
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO papers(id, pdf_path) VALUES (1, 'a.pdf'), (2, NULL), (3, 'c.pdf');
             INSERT INTO chunks(paper_id, index_name) VALUES (1, 'default'), (3, 'other');",
        )
        .unwrap();
        // note 1 件
        write(root.join(".sai/paper/papers/0001_a/note.md"), "# note\n");

        let text = build_brief(&roots_of(&root), Some(&conn), &config(8000), "default");

        assert!(
            text.contains("論文 3 本（PDF 2・note 1・未索引 2）"),
            "{text}"
        );
    }

    #[test]
    fn write_brief_replaces_atomically() {
        let root = temp_root("write");
        let roots = roots_of(&root);

        let path = write_brief(&roots, "1 回目\n").unwrap();
        assert_eq!(path, root.join(".sai/template/project-brief.md"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "1 回目\n");

        write_brief(&roots, "2 回目\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "2 回目\n");
        // 一時ファイルは残らない
        let leftovers: Vec<String> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with("project-brief.md.tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn state_update_reads_the_plan() {
        let entry = |value: &str| HandoffEntry {
            run_id: 1,
            block_id: "paper-search".to_string(),
            tag: None,
            file: PathBuf::from("x.md"),
            modified: SystemTime::UNIX_EPOCH,
            raw: String::new(),
            items: vec![("現況の更新案".to_string(), value.to_string())],
            readable: true,
        };

        assert_eq!(
            entry("段階を絞り込みにする").state_update(),
            Some("段階を絞り込みにする")
        );
        assert_eq!(entry("なし").state_update(), None);
        assert_eq!(entry("").state_update(), None);
    }

    #[test]
    fn handoff_heading_shows_citation_issue_count() {
        let root = temp_root("handoff-issues");
        write_block(&root, "paper-search", "project-brief");
        write_run(&root, "review", 41, "paper-search");
        write(root.join(".sai/outputs/review/41/answer.md"), HANDOFF);

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../sai-paper-core/migrations/0028_reports.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../../sai-paper-core/migrations/0039_report_scan.sql"))
            .unwrap();
        // run 41 の answer.md を report として登録し、要確認 2 件・注意 1 件を入れる。
        conn.execute(
            "INSERT INTO reports (id, title, md_path, run_id, block, created_at, summary)
             VALUES ('R-0007', 'answer', '/tmp/proj/.sai/outputs/review/41/answer.md', '41',
                     'paper-search', '2026-09-24T21:10:00+00:00', '')",
            [],
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO report_issues (report_id, level, kind, detail, line) VALUES
             ('R-0007', 'error', 'unresolved', 'a', NULL),
             ('R-0007', 'error', 'unresolved', 'b', NULL),
             ('R-0007', 'warning', 'style', 'c', NULL);",
        )
        .unwrap();

        let text = build_brief(&roots_of(&root), Some(&conn), &config(8000), "default");
        let line41 = text
            .lines()
            .find(|line| line.starts_with("### run 41 / paper-search"))
            .unwrap_or_else(|| panic!("見出しが無い: {text}"));
        assert!(line41.contains("・出典の要確認 2 件"), "{line41}");

        // 未登録の run 42 の見出しには件数を添えない。
        write_run(&root, "review", 42, "paper-search");
        write(root.join(".sai/outputs/review/42/answer.md"), HANDOFF);
        crate::link::forget(&root);
        let text = build_brief(&roots_of(&root), Some(&conn), &config(8000), "default");
        let line42 = text
            .lines()
            .find(|line| line.starts_with("### run 42 / paper-search"))
            .unwrap_or_else(|| panic!("見出しが無い: {text}"));
        assert!(!line42.contains("出典の要確認"), "{line42}");
    }

    #[test]
    fn handoff_heading_shows_run_tag() {
        let root = temp_root("handoff-tag");
        write_block(&root, "paper-search", "project-brief");
        write_run(&root, "review", 41, "paper-search");
        // task.md にタグを足す（write_run の frontmatter を上書き）。
        write(
            root.join(".sai/outputs/review/41/task.md"),
            "---\nrun_id: 41\nblock: paper-search\ntag: 仮説B\n---\n\nタスク\n",
        );
        write(root.join(".sai/outputs/review/41/answer.md"), HANDOFF);

        let text = build_brief(&roots_of(&root), None, &config(8000), "default");
        let line41 = text
            .lines()
            .find(|line| line.starts_with("### run 41 / paper-search"))
            .unwrap_or_else(|| panic!("見出しが無い: {text}"));
        assert!(
            line41.starts_with("### run 41 / paper-search / 仮説B / "),
            "{line41}"
        );
    }

    #[test]
    fn handoff_heading_hides_empty_or_missing_tag() {
        let root = temp_root("handoff-no-tag");
        write_block(&root, "paper-search", "project-brief");
        // tag: が空
        write_run(&root, "review", 41, "paper-search");
        write(
            root.join(".sai/outputs/review/41/task.md"),
            "---\nrun_id: 41\nblock: paper-search\ntag:\n---\n\nタスク\n",
        );
        write(root.join(".sai/outputs/review/41/answer.md"), HANDOFF);

        let text = build_brief(&roots_of(&root), None, &config(8000), "default");
        let line41 = text
            .lines()
            .find(|line| line.starts_with("### run 41 / paper-search"))
            .unwrap_or_else(|| panic!("見出しが無い: {text}"));
        assert!(!line41.contains("仮説"), "{line41}");
    }

    #[test]
    fn handoff_heading_reads_tag_from_link_db() {
        let root = temp_root("handoff-tag-db");
        write_block(&root, "paper-library-qa", "project-brief");
        // task.md が無い実行。タグは DB から引く。
        let dir = root.join(".sai/outputs/R9/45");
        write(dir.join("answer.md"), HANDOFF);
        crate::fake_link::write_runs(
            &root,
            &[crate::fake_link::run_value(
                45,
                "R9",
                &dir,
                "paper-library-qa",
                "仮説C",
                "completed",
            )],
        );

        let text = build_brief(&roots_of(&root), None, &config(8000), "default");
        let line45 = text
            .lines()
            .find(|line| line.starts_with("### run 45 / paper-library-qa"))
            .unwrap_or_else(|| panic!("見出しが無い: {text}"));
        assert!(
            line45.starts_with("### run 45 / paper-library-qa / 仮説C / "),
            "{line45}"
        );
    }

}
