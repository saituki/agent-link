//! Markdown からの References セクション抽出・引用エントリ分割・パース。
//!
//! ジャーナル/出版社ごとに参考文献の形式が異なるため、以下の分割とパースを
//! 多系統で対応する（旧 Node/`pi-paper-search` の `paper_extract.py` +
//! `journal_parsers.py` で実績のあるロジックを移植したもの）。
//!
//! # 分割（`split_reference_entries`）
//! 1. **numeric `[N]`**: `- [1] ...` / `[1] ...` をエントリ起点として分割
//! 2. **番号付き段落**: `12. Gong ...` / `12 Anticevic ...` を `numbered_runs` で
//!    検出（空行スキップ + 最長連番ラン採用。PLOS/Nature 等の空行分断に対応）
//! 3. **著者-年（無番号）**: `-` bullet あるいは空行ブロックでエントリ分割
//!    （Frontiers / bioRxiv / ACM / arXiv 等の `Author Year` / `Author (Year)` 形式）
//!
//! # パース（`parse_references` → `parse_author_year_entry` / `parse_numeric_entry`）
//! - `[text](url)` の分離 → url/doi 抽出
//! - `&amp;`・二重空白の正規化
//! - タイトル境界は**ジャーナル語彙**（`Journal of ...` / `In ...` / `arXiv` /
//!   `Proceedings` 等）と `(Year)`・DOI/URL 位置から判定
//!   （イニシャルのピリオドで誤分割しない）
//! - DOI / journal 欠落を許容

use regex::Regex;
use std::sync::LazyLock;

/// パースされた引用文献（docling 抽出の生値）。
#[derive(Debug, Clone)]
pub struct RawRef {
    /// エントリ先頭の参照番号（`[12]` / `12.` 形式）。
    /// これが `paper_refs.ref_no` となり、本文中の引用番号 `[n]` と
    /// 同じ番号空間に揃う。番号なしスタイルは `None`（行順に採番）。
    pub number: Option<i64>,
    /// 第一著者
    pub first_author: Option<String>,
    /// 出版年
    pub year: Option<i64>,
    /// 年号サフィックス（例: "a", "b"）
    pub year_suffix: Option<String>,
    /// ジャーナル名
    pub journal: Option<String>,
    /// タイトル
    pub title: Option<String>,
    /// DOI
    pub doi: Option<String>,
    /// 抽出元の生テキスト
    pub raw_text: String,
}

/// 参考文献リストの引用スタイル。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefStyle {
    /// 番号付き（`[N]` / `N.`）
    Numeric,
    /// 無番号（著者-年）
    AuthorYear,
}

/// 分割されたエントリ。
#[derive(Debug, Clone)]
pub struct SplitEntry {
    /// エントリ先頭の参照番号（ある場合）
    pub number: Option<i64>,
    /// エントリ本文
    pub text: String,
}

/// Markdown から References セクションを抽出する。
///
/// docling の出力は見出し形式（`## References`）と裸行（`References`）の両方があり得るため、
/// 行単位で緩く照合する。見つからない場合は `None`。
///
/// **打ち切りルール:** References の直後に続く次のセクション見出し
/// （`## Organization of Appendix` / `## A ...` 等）で必ず打ち切る。
/// 多くの論文は References の後に Appendix が続き、そこには番号付き箇条書き
/// （プロンプトテンプレートの `1. ...` 等）が含まれる。これを取り込むと
/// 引用エントリに誤分割されて ref_no が重複し、`paper_refs` の主キー違反で
/// 抽出全体が失敗するため。
pub fn extract_references_section(markdown: &str) -> Option<String> {
    // 例: "## References" / "REFERENCES" / "**References**" / "Bibliography"
    static HEADING_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?im)^[ \t>]*(?:#{1,6}[ \t]*)?(?:\*{0,2})?(?:references|bibliography)(?:\**)?(?:[ \t]*:)?[ \t\*]*$").expect("regex"));
    let heading = &*HEADING_RE;
    // 次のセクション見出し（Appendix 等）
    static SECTION_HEADING_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[ \t]*#{1,6}[ \t]+\S").expect("regex"));
    let section_heading = &*SECTION_HEADING_RE;
    let lines: Vec<&str> = markdown.lines().collect();
    let start = lines.iter().position(|line| heading.is_match(line))?;
    let end = lines[start + 1..]
        .iter()
        .position(|line| section_heading.is_match(line))
        .map_or(lines.len(), |offset| start + 1 + offset);
    Some(lines[start + 1..end].join("\n"))
}

/// 共通前処理: Markdown リンク分離・URL 除去・`&amp;` 正規化・空白正規化・DOI 抽出。
///
/// 戻り値: `(クリーン済みテキスト, url, doi)`
fn clean_raw(raw: &str) -> (String, Option<String>, Option<String>) {
    // 1) `[text](url)` 形式 → text を残し url を回収
    static MD_LINK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\((\S+)\)").expect("md_link regex"));
    let md_link = &*MD_LINK_RE;
    let mut url: Option<String> = None;
    let mut text = String::with_capacity(raw.len());
    let mut last = 0;
    for cap in md_link.captures_iter(raw) {
        let m = cap.get(0).expect("capture 0");
        text.push_str(&raw[last..m.start()]);
        let inner = cap.get(1).map_or("", |x| x.as_str());
        let u = cap.get(2).map_or("", |x| x.as_str());
        if url.is_none() && !u.is_empty() {
            url = Some(u.trim_end_matches(['.', ',', ';', ')']).to_string());
        }
        text.push_str(inner);
        last = m.end();
    }
    text.push_str(&raw[last..]);

    // 2) 裸の URL を除去 + 回収
    static BARE_URL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://\S+").expect("bare_url regex"));
    let bare_url = &*BARE_URL_RE;
    if url.is_none() {
        if let Some(m) = bare_url.find(&text) {
            url = Some(
                m.as_str()
                    .trim_end_matches(['.', ',', ';', ')'])
                    .to_string(),
            );
        }
    }
    let text = bare_url.replace_all(&text, "").into_owned();

    // 3) 実体参照・空白の正規化
    let text = text.replace("&amp;", "&");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");

    // 4) DOI 抽出（本文に無ければ URL からも試す）
    let doi = extract_doi(&text).or_else(|| url.as_deref().and_then(extract_doi));

    (text, url, doi)
}

/// エントリ本文をクリーンにする（行頭の bullet・番号プレフィックスを除去）。
fn clean_entry(raw: &str) -> String {
    let mut t = raw.trim().to_string();
    // `- [1] ...` / `- 1 Anticevic ...` 等の行頭 bullet / 番号を除去
    static PREFIX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[-*•·]?\s*\[?\d{1,4}\]?[:.)]?\s+").expect("prefix regex"));
    let prefix = &*PREFIX_RE;
    t = prefix.replace(&t, "").into_owned();
    // 末尾の DOI/URL と句読点は clean_raw 側で処理するため、ここでは空白のみ整理
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// References セクションを引用エントリに分割する（多系統）。
///
/// 戻り値: `(スタイル, エントリ列)`。`Numeric` のエントリには `number` が付き、
/// `AuthorYear` のエントリは `number=None`（後段で行順に採番）。
pub fn split_reference_entries(section: &str) -> (RefStyle, Vec<SplitEntry>) {
    let lines: Vec<&str> = section.lines().collect();

    // --- 1) numeric `[N]`: `- [1] ...` / `[1] ...` を起点に分割 ---------------
    static NUM_LINE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[-*]?\s*\[(\d{1,3})\]\s*(.*)$").expect("num_line regex"));
    let num_line = &*NUM_LINE_RE;
    let mut starts: Vec<(usize, i64, String)> = Vec::new();
    for (i, ln) in lines.iter().enumerate() {
        if let Some(c) = num_line.captures(ln) {
            let no = c.get(1).and_then(|x| x.as_str().parse().ok()).unwrap_or(0);
            let rest = c.get(2).map_or("", |x| x.as_str()).trim().to_string();
            starts.push((i, no, rest));
        }
    }
    if starts.len() >= 2 {
        let mut entries = Vec::new();
        for k in 0..starts.len() {
            let (i, no, rest) = &starts[k];
            let end = if k + 1 < starts.len() {
                starts[k + 1].0
            } else {
                lines.len()
            };
            let cont = lines[*i + 1..end]
                .iter()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let text = [rest.as_str(), cont.as_str()].join(" ");
            entries.push(SplitEntry {
                number: Some(*no),
                text: text.trim().to_string(),
            });
        }
        return (RefStyle::Numeric, entries);
    }

    // --- 2) 番号付き段落 `12. Gong` / `12 Anticevic ...` ---------------------
    // 空行スキップ + 最長連番ラン採用（PLOS/Nature の空行分断対策）
    let runs = numbered_runs(&lines);
    if let Some(&(start, end)) = runs.iter().max_by_key(|(s, e)| (e - s, *s)) {
        let mut entries = Vec::new();
        let mut cur_no: Option<i64> = None;
        let mut cur: Vec<String> = Vec::new();
        static PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(\d{1,3})\.?\s+(\S.*)$").expect("unheaded regex"));
    let re = &*PATTERN;
        for ln in &lines[start..end] {
            if ln.trim().is_empty() {
                continue;
            }
            if let Some(c) = re.captures(ln) {
                if let Some(n) = cur_no.take() {
                    entries.push(SplitEntry {
                        number: Some(n),
                        text: cur.join(" "),
                    });
                }
                cur_no = c.get(1).and_then(|x| x.as_str().parse().ok());
                cur = vec![c.get(2).map_or("", |x| x.as_str()).to_string()];
            } else if cur_no.is_some() {
                cur.push(ln.trim().to_string());
            }
        }
        if let Some(n) = cur_no.take() {
            entries.push(SplitEntry {
                number: Some(n),
                text: cur.join(" "),
            });
        }
        if entries.len() >= 2 {
            return (RefStyle::Numeric, entries);
        }
    }

    // --- 3) 著者-年（無番号）: bullet / 空行ブロックで分割 --------------------
    static BULLET_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[-*]\s+(.*)$").expect("bullet regex"));
    let bullet = &*BULLET_RE;
    let mut candidates: Vec<String> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    for line in &lines {
        let stripped = line.trim();
        if stripped.is_empty() {
            if !cur.is_empty() {
                candidates.push(cur.join(" "));
                cur = Vec::new();
            }
            continue;
        }
        if let Some(c) = bullet.captures(line) {
            let text = c.get(1).map_or("", |x| x.as_str());
            if !cur.is_empty() {
                candidates.push(cur.join(" "));
                cur = vec![text.to_string()];
            } else {
                cur.push(text.to_string());
            }
        } else {
            cur.push(stripped.to_string());
        }
    }
    if !cur.is_empty() {
        candidates.push(cur.join(" "));
    }

    // 短すぎる断片（ページ番号・URL 切れ等）は除外
    let cleaned: Vec<SplitEntry> = candidates
        .into_iter()
        .filter(|c| c.trim().chars().count() >= 30)
        .map(|c| SplitEntry {
            number: None,
            text: c.trim().to_string(),
        })
        .collect();

    (RefStyle::AuthorYear, cleaned)
}

/// 番号付き段落（`12. ...` / `12 ...`）の連なりを `(start, end_exclusive)` で返す。
///
/// 空行はスキップし、最長の連番ランを優先する（PLOS / Neuron の空行分断対策）。
/// 旧 `paper_extract.py::numbered_runs` の移植。`min_run` は 3 に緩和（Nature 対策）。
fn numbered_runs(lines: &[&str]) -> Vec<(usize, usize)> {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(\d{1,3})\.?\s+(\S.*)$").expect("unheaded regex"));
    let re = &*PATTERN;
    let mut runs = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if re.is_match(lines[i]) {
            let mut j = i;
            let mut nums: Vec<i64> = Vec::new();
            while j < lines.len() {
                let l = lines[j];
                if l.trim().is_empty() {
                    j += 1; // 空行スキップ
                    continue;
                }
                if let Some(c) = re.captures(l) {
                    if let Ok(n) = c.get(1).unwrap().as_str().parse::<i64>() {
                        nums.push(n);
                    }
                    j += 1;
                } else if j > i {
                    j += 1; // 前行の折り返し
                } else {
                    break;
                }
            }
            if nums.len() >= 3 && nums.windows(2).all(|w| w[1] > w[0]) {
                runs.push((i, j));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    runs
}

/// エントリ先頭の参照番号（`[12]` / `12.` / `12)` 形式、箇条書きプレフィックス許容）を抽出する。
#[must_use]
pub fn extract_entry_number(entry: &str) -> Option<i64> {
    let t = entry.trim_start();
    let t = t
        .strip_prefix(['-', '*', '+', '\u{2022}'])
        .map(str::trim_start)
        .unwrap_or(t);
    // `[N]` 形式
    if let Some(rest) = t.strip_prefix('[') {
        let close = rest.find(']')?;
        return rest[..close].trim().parse().ok();
    }
    // `N.` / `N)` / `N ` 形式。参照番号は 1〜3 桁を想定（4桁以上は年号等なので除外）
    let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 3 {
        return None;
    }
    let after = &t[digits.len()..];
    // 次の文字が区切り（. ) 空白）なら番号とみなす。そうでなければ
    // 「2020Smith」のような年始まり等を誤検出しないようにする。
    if after.starts_with(['.', ')']) || after.starts_with(' ') {
        digits.parse().ok()
    } else {
        None
    }
}

/// References セクションをパースする。
///
/// ジャーナル / 出版社の判別子。DOI prefix から `detect_journal` で決定し、
/// パース時にジャーナル固有のルールを適用する（旧 Python `detect_journal` の移植）。
///
/// `Generic` は汎用パーサー（著者-年 / 番号付き）にフォールバックする。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Journal {
    /// 汎用（特定のジャーナル分岐をしない）
    Generic,
    /// bioRxiv（DOI 10.1101）— `Author (Year) Title. Journal` 著者-年
    Biorxiv,
    /// arXiv（DOI 10.48550）— `[N] Title. arXiv ...` 番号付き
    Arxiv,
    /// IEEE — `[N] 'Title,' Journal, vol.` 番号付き・引用符付きタイトル
    Ieee,
    /// Frontiers（DOI 10.3389）— `Author (Year) Title. Journal. Vol:pages. doi` 著者-年
    Frontiers,
    /// PLOS（DOI 10.1371）— `1. Author (Year) Title. Journal. doi` 番号付き段落
    Plos,
    /// Nature / Mol Psychiatry（DOI 10.1038）— `N. Author ... Journal Year; Vol:pages` 番号付き段落
    Nature,
    /// Springer（DOI 10.1007）— `Author, A. (Year). Title. Journal, Volume` 著者-年
    Springer,
    /// Elsevier（DOI 10.1016）— `Author, A., ... . Title. Journal. Year; Vol` 著者-年 bullet
    Elsevier,
}

/// DOI からジャーナルを判定する（旧 Python `detect_journal` の DOI prefix 分岐を移植）。
///
/// DOI が無い / 既知 prefix でない場合は `Generic` を返す。
#[must_use]
pub fn detect_journal(doi: Option<&str>) -> Journal {
    let Some(d) = doi.map(str::trim).filter(|d| !d.is_empty()) else {
        return Journal::Generic;
    };
    let d = d.to_ascii_lowercase();
    if d.contains("10.1101") {
        Journal::Biorxiv
    } else if d.contains("10.48550") {
        Journal::Arxiv
    } else if d.contains("10.3389") {
        Journal::Frontiers
    } else if d.contains("10.1371") {
        Journal::Plos
    } else if d.contains("10.1038") {
        Journal::Nature
    } else if d.contains("10.1007") {
        Journal::Springer
    } else if d.contains("10.1016") {
        Journal::Elsevier
    } else {
        Journal::Generic
    }
}

/// スタイル検出（`split_reference_entries`）に従い、各エントリを
/// ジャーナル固有 / 著者-年 / 番号付きの適切なパーサで分解する。
pub fn parse_references(markdown: &str, journal: Journal) -> Vec<RawRef> {
    let Some(section) = extract_references_section(markdown) else {
        return vec![];
    };
    // ジャーナル固有の共通前処理（PLOS の `Citation:` ヘッダ行の除外等）
    let section = preprocess_section(section, journal);
    let (style, entries) = split_reference_entries(&section);
    entries
        .iter()
        .filter_map(|e| parse_for_journal(journal, style, e))
        .collect()
}

/// ジャーナル / スタイルに応じてエントリのパーサーを選択する。
fn parse_for_journal(journal: Journal, style: RefStyle, e: &SplitEntry) -> Option<RawRef> {
    match journal {
        // 著者-年中心のジャーナル
        Journal::Biorxiv | Journal::Frontiers | Journal::Springer | Journal::Elsevier => {
            // Frontiers / Springer / bioRxiv / Elsevier は `(Year)` 括弧年等形式の著者-年が主流
            match style {
                RefStyle::AuthorYear => parse_author_year_entry(e),
                // 万一 numeric に分割された場合は汎用 numeric に委譲
                RefStyle::Numeric => parse_numeric_entry(e),
            }
        }
        // 番号付き中心のジャーナル
        Journal::Arxiv | Journal::Ieee | Journal::Plos | Journal::Nature => match style {
            RefStyle::Numeric => parse_numeric_entry(e),
            RefStyle::AuthorYear => parse_author_year_entry(e),
        },
        Journal::Generic => match style {
            RefStyle::Numeric => parse_numeric_entry(e),
            RefStyle::AuthorYear => parse_author_year_entry(e),
        },
    }
}

/// 引用セクションに対するジャーナル固有の前処理。
///
/// 例: 一部の DL 元（PMC 等）は References 直前に `Citation: ...` の
/// ヘッダ行を持つ（PLOS / Elsevier で見られる）。これをエントリ分割前に除去する。
fn preprocess_section(section: String, journal: Journal) -> String {
    match journal {
        Journal::Plos | Journal::Elsevier => {
            // `Citation: ...` で始まる行（と直後の空行）を除去
            static PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?im)^[ \t]*(?:(?:Permanent\s+)?Citation|Cite\s+this\s+article)\s*:.*(?:\n|$)").expect("citation regex"));
    let re = &*PATTERN;
            re.replace_all(&section, "").into_owned()
        }
        _ => section,
    }
}

/// 著者-年エントリのパース。
///
/// 対応形式:
/// - `Author (Year) Title. Journal`  （括弧年）
/// - `Author, A., et al., Year. Title. Journal.`（Elsevier 系）
/// - `Authors. Title. Journal Year.` / `Authors. Title. arXiv ...`（arXiv 系）
fn parse_author_year_entry(e: &SplitEntry) -> Option<RawRef> {
    if e.text.trim().is_empty() {
        return None;
    }
    let (clean, _url, doi) = clean_raw(&e.text);
    let clean = clean_entry(&clean);

    // 1) 括弧年 `Author (Year) Title. Journal`
    if let Some(r) = try_paren_year(e.number, &clean, doi.clone()) {
        return Some(r);
    }

    // 2) 一般的な `Authors. Title. Journal/arXiv ...`
    //    `Authors.` を先頭で切り、残りからジャーナル語彙で title 境界を判定
    if let Some((authors, rest)) = split_authors_rest(&clean) {
        let (title, journal) = split_title_journal(&rest);
        let year = extract_year_anywhere(&clean);
        let year_suffix = extract_year_suffix(&clean);
        return Some(RawRef {
            number: e.number,
            first_author: extract_first_author(&authors),
            year,
            year_suffix,
            journal,
            title: Some(title).filter(|t| !t.is_empty() && t.len() >= 5),
            doi,
            raw_text: e.text.clone(),
        });
    }

    // 3) フォールバック（タイトルのみ、先頭 200 字）
    let year = extract_year_anywhere(&clean);
    return Some(RawRef {
        number: e.number,
        first_author: extract_first_author(&clean),
        year,
        year_suffix: None,
        journal: None,
        title: Some(clean.chars().take(200).collect::<String>()).filter(|t| t.len() >= 5),
        doi,
        raw_text: e.text.clone(),
    });
}

/// 括弧年 `Author (Year) Title. Journal` 形式のパース（著者-年 / 番号付き共通）。
///
/// PLOS / Nature `/ bioRxiv / Frontiers / Springer` 等、`Author (Year) Title` を
/// 使う形式に適用できる。番号付きエントリでも `N. Author (Year) ...` を扱えるよう
/// `parse_numeric_entry` からも呼ばれる。
fn try_paren_year(number: Option<i64>, clean: &str, doi: Option<String>) -> Option<RawRef> {
    static PAREN_YEAR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?P<authors>.+?)\s*\((?P<year>(?:19|20)\d{2})(?P<suffix>[a-z])?\)[\.\s]+(?P<rest>.+?)\s*$").expect("paren regex"));
    let paren_year = &*PAREN_YEAR_RE;
    let c = paren_year.captures(clean)?;
    let authors = c.name("authors").map_or("", |x| x.as_str());
    let year = c.name("year").and_then(|x| x.as_str().parse().ok());
    let year_suffix = c.name("suffix").map(|x| x.as_str().to_string());
    let rest = c.name("rest").map_or("", |x| x.as_str());
    let (title, journal) = split_title_journal(rest);
    Some(RawRef {
        number,
        first_author: extract_first_author(authors),
        year,
        year_suffix,
        journal,
        title: Some(title).filter(|t| !t.is_empty() && t.len() >= 5),
        doi,
        raw_text: clean.to_string(),
    })
}

/// 番号付きエントリのパース。
///
/// 対応形式: `[1] Authors. Title. Journal, vol. ...`（IEEE） / `Authors. Title. arXiv ...`
fn parse_numeric_entry(e: &SplitEntry) -> Option<RawRef> {
    if e.text.trim().is_empty() {
        return None;
    }
    let (clean, _url, doi) = clean_raw(&e.text);
    let clean = clean_entry(&clean);

    // 括弧年 `Author (Year) Title. Journal`（PLOS / Nature の `N. Author (Year) ...`
    // など、番号付きでも `(Year)` を使う形式に対応）
    if let Some(r) = try_paren_year(e.number, &clean, doi.clone()) {
        return Some(r);
    }

    // シングルクォート / スマートクォートで囲まれた title（IEEE / ACM 等）
    if let Some((q_start, q_end, quoted_title)) = find_quoted(&clean) {
        let title = quoted_title;
        let authors = clean[..q_start]
            .trim()
            .trim_start_matches([',', ' '])
            .to_string();
        let tail = &clean[q_end..];
        let year = extract_year_anywhere(tail);
        // journal は `, vol.` / `, pp.` 直前まで
        static VOL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r",\s*vol\.|\s+vol\.|, pp\.|,\s*\d{4}\.").expect("vol regex"));
    let vol = &*VOL_RE;
        let jm = vol.find(tail).map(|m| m.start()).unwrap_or(tail.len());
        let journal = tail[..jm].trim().trim_end_matches(',').to_string();
        return Some(RawRef {
            number: e.number,
            first_author: extract_first_author(&authors),
            year: year.or_else(|| extract_year_anywhere(&clean)),
            year_suffix: None,
            journal: Some(journal).filter(|j| !j.is_empty() && j.len() >= 2),
            title: Some(title),
            doi,
            raw_text: e.text.clone(),
        });
    }

    // `Authors. Title. Journal/arXiv ...`
    if let Some((authors, rest)) = split_authors_rest(&clean) {
        let (title, journal) = split_title_journal(&rest);
        let year = extract_year_anywhere(&clean);
        let year_suffix = extract_year_suffix(&clean);
        return Some(RawRef {
            number: e.number,
            first_author: extract_first_author(&authors),
            year,
            year_suffix,
            journal,
            title: Some(title).filter(|t| !t.is_empty() && t.len() >= 5),
            doi,
            raw_text: e.text.clone(),
        });
    }

    let year = extract_year_anywhere(&clean);
    return Some(RawRef {
        number: e.number,
        first_author: extract_first_author(&clean),
        year,
        year_suffix: None,
        journal: None,
        title: Some(clean.chars().take(200).collect::<String>()).filter(|t| t.len() >= 5),
        doi,
        raw_text: e.text.clone(),
    });
}

/// テキスト中の引用符（`'` / `‘` / `“`）で囲まれた部分を探す。
///
/// 戻り値: `(開き引用符の開始位置, 閉じ引用符の直後位置, 内部テキスト)`。
fn find_quoted(text: &str) -> Option<(usize, usize, String)> {
    for (i, c) in text.char_indices() {
        let close = match c {
            '\'' => '\'',
            '\u{2018}' => '\u{2019}',
            '\u{201c}' => '\u{201d}',
            _ => continue,
        };
        let after = i + c.len_utf8();
        let rest = &text[after..];
        if let Some(rel) = rest.find(close) {
            let inner = &rest[..rel];
            if inner.chars().count() >= 10 {
                let q_end = after + rel + close.len_utf8();
                return Some((i, q_end, inner.trim().to_string()));
            }
        }
    }
    None
}

/// `Authors. Title...` の著者 / タイトル境界を分割する。
///
/// イニシャル先頭の著者リスト（`V. Botti, F. Barber, ...`）で誤って
/// 「`V.` | `Botti...`」と分割しないよう、境界の直前トークンが単一の
/// 大文字イニシャルであるものはスキップする。
fn split_authors_rest(text: &str) -> Option<(String, String)> {
    static DOT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.\s+").expect("dot-space regex"));
    let dot = &*DOT_RE;
    static YEAR_START_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:19|20)\d{2}").expect("year-start regex"));
    let year_start = &*YEAR_START_RE;
    // 残りがまだ著者リストの続き（"Botti, ..." / "D. Gallardo" / "Hernández." / "et al."）かどうか
    static CONTINUES_AUTHOR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Z][\w'’-]*,\s*|^[A-Z]\.|^[A-Z][\w'’-]*\.\s|^(?:et\s+al\.?,?\s*|and\s+)").expect("cont regex"));
    let continues_author = &*CONTINUES_AUTHOR_RE;
    for m in dot.find_iter(text) {
        let after = &text[m.end()..];
        let next = after.chars().next();
        let is_cap = next.is_some_and(|c| c.is_ascii_uppercase());
        let is_year = year_start.is_match(after);
        if !is_cap && !is_year {
            continue;
        }
        // 残りがまだ著者リストの続きだったら境界候補から除外
        if continues_author.is_match(after) {
            continue;
        }
        let before = text[..m.start()].trim_end();
        if before.is_empty() {
            continue;
        }
        let rest = &text[m.end()..];
        return Some((before.to_string(), rest.to_string()));
    }
    None
}

/// タイトルとジャーナルの境界を判定する。
///
/// ジャーナル語彙（`Journal of ...` / `In ...` / `arXiv` / `Proceedings` /
/// `Nature` / `Science` / `PLoS` / `IEEE` 等）の直前にピリオドがある位置を
/// タイトル終端とみなす。見つからない場合は最初の `. ` で分割する。
fn split_title_journal(rest: &str) -> (String, Option<String>) {
    // 先頭の年号（`2023. ` / `2023 `）を除去（`Authors. 2023. Title ...` 形式対応）
    static LEADING_YEAR_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?:19|20)\d{2}[ab]?[.、;]?\s+").expect("leading year regex"));
    let rest = LEADING_YEAR_RE.replace(rest, "").into_owned();
    let rest = rest.trim();

    // ジャーナル語彙の直前に `. ` がある位置をタイトル終端とみなす。
    // （`regex` は look-ahead 非対応のため、語彙の出現位置の直前が `. `
    //   かどうかを手動で判定する）
    static KW_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:In\s+|Preprint|arXiv|bioRxiv|Proceedings|Advances in|Journal of|Nature|Science|Physical|PNAS|Frontiers|Front\.|IEEE|ACM|PLoS|Neuron|Cell|NeuroImage|Schizophr|Cereb|Brain Research|Trends in|Annual Review|Current Opinion|eLife|ELife|Wiley|MIT Press|Springer|Nat\.|Geosci\.|Sci\. Data|Sci\. Rep|IEEE Trans|PLOS|Comput\.)[\s,;)]").expect("kw regex"));
    let kw_re = &*KW_RE;
    let mut dot: Option<usize> = None;
    for m in kw_re.find_iter(rest) {
        let pos = m.start();
        let before = &rest[..pos];
        if let Some(d) = before.rfind('.') {
            let between = &before[d + 1..];
            if !between.is_empty() && between.chars().all(|c| c.is_whitespace()) {
                dot = Some(d);
                break; // 最初に合致した位置が最前（find_iter は出現順）
            }
        }
    }
    if let Some(d) = dot {
        let title = rest[..d].trim().trim_end_matches(['.', ',', ';']).trim();
        let journal = clean_journal(&rest[d + 1..]);
        return (strip_trailing_year(&title), journal);
    }

    // フォールバック: 最初の `. ` で title / journal を分割
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.\s+").expect("dot regex"));
    let re = &*PATTERN;
    if let Some(m) = re.find(rest) {
        let title = rest[..m.start()].trim();
        let journal = clean_journal(&rest[m.end()..]);
        return (strip_trailing_year(&title), journal);
    }
    (
        strip_trailing_year(rest.trim().trim_end_matches(['.', ',', ';'])),
        None,
    )
}

/// タイトルの末尾の年号（`, 2022` / ` 2022`）を除去する（arXiv 形式 `Title, 2022` 対応）。
fn strip_trailing_year(title: &str) -> String {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:,\s*|\s+)(?:19|20)\d{2}[ab]?\s*$").expect("trailing year regex"));
    let re = &*PATTERN;
    let out = re.replace(title, "").into_owned();
    out.trim()
        .trim_end_matches(['.', ',', ';'])
        .trim()
        .to_string()
}

/// ジャーナル候補をクリーンにする。巻・ページ・年・DOI・URL・`Available online
/// at` 記述・出版社記述等を除去し、ジャーナルとして不適切なら `None` を返す。
///
/// Frontiers / Elsevier 等に見られる以下のパターンを処理する:
/// - `Nat. Comput. Sci. 4,  154-157.  doi: ...` → `Nat. Comput. Sci.`
/// - `Geosci. Model Dev. 9,  1937-1958.  doi: ...` → `Geosci. Model Dev.`
/// - `Available online at: URL (Accessed ...)` → `None`（プレプリント）
/// - `Proceedings of ... . New York, NY: ... 161-170.` → `Proceedings of ...`
fn clean_journal(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    // プレプリント記述（“Available online at” / “Available at”）は journal にしない
    static AVAIL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:available\s+online\s+at|available\s+at|available\s+from)").expect("avail regex"));
    let avail = &*AVAIL_RE;
    if avail.is_match(raw) {
        return None;
    }
    // 巻・ページ・年・URL・DOI・出版社・アクセス日等の区切りまでを取得
    static CUT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*d(?:oi)?\s*[:.]?\s*10\.|,?\s*doi\s*:|https?|\s*arXiv:\s*\S+|\s*\(Accessed|Available\s+online\s+at|\s+vol\.|,?\s*pp\.|\s+\d{4}[ab]?;|\s+\d{4},\s*\d+|\s+\d{4}\b|\s+\d+:\s*\d+|\s+\d+:\S+|\s+\d+,\s*\d+|;\s*\d+|\s+\d+\.\s+\d").expect("journal cut regex"));
    let cut = &*CUT_RE;
    let jm = cut.find(raw).map(|m| m.start()).unwrap_or(raw.len());
    let mut journal = raw[..jm]
        .trim()
        .trim_end_matches([',', ';', '.', ':'])
        .trim()
        .to_string();
    // 出版社記述（`. New York, NY: ...` / `. Oxford: ...` / `. Berlin: ...`）を除去
    // （"Proceedings of the 17th ACM ... . New York, NY: ..." → "Proceedings of ..."）
    static PUB_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.\s+[A-Z][A-Za-z ]*?,\s+[A-Z]{1,3}\.?\s*:\s+|\s+[A-Z][A-Za-z]+,\s+[A-Z]{1,3}\.?\s*:\s+").expect("publisher regex"));
    let pub_re = &*PUB_PATTERN;
    if let Some(m) = pub_re.find(&journal) {
        // 出版社コロン位置より前にある `. ` を検出して切り捨てる
        let prefix = &journal[..m.start()];
        if let Some(dot) = prefix.rfind(". ") {
            journal.truncate(dot);
        } else {
            journal.truncate(m.start());
        }
        journal = journal
            .trim_end_matches(['.', ',', ';', ':'])
            .trim()
            .to_string();
    }
    if journal.is_empty() {
        return None;
    }
    // ジャーナルとして不適切なもの（URL 断片・"URL"・記号のみ）は None
    let lower = journal.to_lowercase();
    let looks_like_url = lower.starts_with("url")
        || lower.contains("http")
        || lower.contains(".com")
        || (lower.contains('.') && !lower.contains(' '));
    if looks_like_url || journal.chars().all(|c| !c.is_alphabetic()) {
        None
    } else {
        Some(journal.chars().take(120).collect())
    }
}

// --- ヘルパー関数 ---

/// 括弧内の4桁年号を抽出する（例: "(2020)"）。
pub fn extract_year_from_parens(s: &str) -> Option<i64> {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\((\d{4})\)").expect("regex"));
    let re = &*PATTERN;
    re.captures(s)?.get(1)?.as_str().parse().ok()
}

/// 文中の4桁年号（19xx / 20xx）を抽出する。
pub fn extract_year_anywhere(s: &str) -> Option<i64> {
    // `2024a` のように年の後ろに字が付いたものも年として読む（字は extract_year_suffix）
    static PATTERN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b((?:19|20)\d{2})[a-z]?\b").expect("regex"));
    let re = &*PATTERN;
    re.captures(s)?.get(1)?.as_str().parse().ok()
}

/// 文中の4桁年号サフィックス（`2017a` 等）を抽出する。
fn extract_year_suffix(s: &str) -> Option<String> {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(19|20)\d{2}([a-z])\b").expect("regex"));
    let re = &*PATTERN;
    re.captures(s)?.get(2).map(|x| x.as_str().to_string())
}

/// 第一著者を抽出する。
///
/// - `Smith, J.` → `Smith`
/// - `J. N. Onuchic` → `Onuchic`（イニシャル先頭の順序）
/// - `Byrne ´ A` → `Byrne`（OCR 崩れ `´` を許容）
/// - `T. Brown` → `Brown`（IEEE）
/// - `Open X-Embodiment Collaboration`（団体名）
pub fn extract_first_author(s: &str) -> Option<String> {
    let body = strip_number_prefix(s).trim();
    if body.is_empty() {
        return None;
    }
    // 先頭のセグメント（カンマ区切りの最初）を基本とする
    let first_seg = body.split(',').next().unwrap_or("").trim();
    if first_seg.is_empty() {
        return None;
    }
    let words: Vec<&str> = first_seg.split_whitespace().collect();
    let first_word = words[0].trim_matches(|c| c == '.' || c == ',' || c == ';');

    // 先頭がイニシャル（"J." / "T."）なら姓（英大文字始まりの最後の単語）を返す
    let is_initial = {
        let t: String = words[0].chars().filter(|c| *c != '.').collect();
        t.chars().count() == 1 && t.chars().next().is_some_and(|c| c.is_ascii_uppercase())
    };
    if is_initial {
        let surname = words
            .iter()
            .rev()
            .find(|w| {
                let t: String = w
                    .chars()
                    .filter(|c| *c != '.' && *c != ',' && *c != ';')
                    .collect();
                t.chars().count() > 1 && t.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            })
            .unwrap_or(&words[0]);
        return Some(
            surname
                .trim_matches(|c| c == '.' || c == ',' || c == ';')
                .to_string(),
        );
    }
    Some(first_word.to_string())
}

/// DOI を抽出する（例: `10.1038/nature12373`）。
pub fn extract_doi(s: &str) -> Option<String> {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(10\.\d{4,9}/[-._;()/:A-Za-z0-9]+)").expect("regex"));
    let re = &*PATTERN;
    let m = re.find(s)?;
    let doi = m.as_str().trim_end_matches('.').to_string();
    if doi.ends_with('/') || doi.ends_with(')') {
        return doi.trim_end_matches(['/', ')']).to_string().into();
    }
    Some(doi)
}

/// 行頭のプレフィックス（箇条書き `- `/`* `、番号 `[12]` / `12.` / `12 `）を除去する。
fn strip_number_prefix(s: &str) -> &str {
    let t = s.trim_start();
    // docling は引用エントリを箇条書き ("- [1] ...") で出力することがある
    let t = t
        .strip_prefix(['-', '*', '•'])
        .map(str::trim_start)
        .unwrap_or(t);
    let mut chars = t.char_indices();
    // `[N] ` 形式
    if let Some((_, '[')) = chars.next() {
        if let Some(close) = t.find(']') {
            return t[close + 1..].trim_start();
        }
    }
    // `N. ` / `N) ` 形式
    let mut saw_digit = false;
    for (i, c) in t.char_indices() {
        if c.is_ascii_digit() {
            saw_digit = true;
        } else if saw_digit && (c == '.' || c == ')' || c == ' ') {
            // 区切り文字も消費する（"12. Smith" → "Smith"）
            return t[i + c.len_utf8()..].trim_start();
        } else {
            break;
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Intro text.\n\n## References\n\n[1] Smith, J. Attention Is All You Need. NeurIPS (2017). doi: 10.1234/abc\n[2] Doe, A. BERT. arXiv (2019).\n";

    #[test]
    fn extract_references_section_handles_heading() {
        let section = extract_references_section(SAMPLE).unwrap();
        assert!(section.contains("[1] Smith"));
        assert!(!section.contains("Intro text."));
    }

    #[test]
    fn extract_references_section_stops_at_next_heading() {
        // References の後の Appendix は取り込まない（番号付き箇条書きの誤分割防止）
        let md = "## References\n\n[1] Foo. Bar (2020).\n\n## Organization of Appendix\n\n1. Step one.\n2. Step two.\n";
        let section = extract_references_section(md).unwrap();
        assert!(section.contains("[1] Foo"));
        assert!(!section.contains("Appendix"));
        assert!(!section.contains("Step one"));
        let md2 = "## References\n\n[1] Foo. Bar (2020).\n";
        let section2 = extract_references_section(md2).unwrap();
        assert!(section2.contains("Foo. Bar"));
    }

    #[test]
    fn extract_references_section_handles_bare_line() {
        let md = "Body\nReferences\n[1] Foo, 2020.";
        assert_eq!(
            extract_references_section(md).unwrap(),
            "[1] Foo, 2020.".to_string()
        );
        assert!(extract_references_section("no refs here").is_none());
    }

    #[test]
    fn split_reference_entries_splits_on_numbers() {
        let section = "[1] One\ncontinued line\n[2] Two\n[10] Ten";
        let (style, entries) = split_reference_entries(section);
        assert_eq!(style, RefStyle::Numeric);
        assert_eq!(entries.len(), 3);
        assert!(entries[0].text.starts_with("One"));
        assert!(entries[0].text.contains("continued"));
        assert!(entries[2].text.starts_with("Ten"));
        assert_eq!(entries[2].number, Some(10));
    }

    #[test]
    fn split_reference_entries_handles_bullet_numbers() {
        let section = "- [1] Jimmy Lei Ba. Layer normalization. arXiv (2016).\n- [2] Dzmitry Bahdanau. Neural machine translation. CoRR (2014).\n- [3] Denny Britz. Massive exploration (2017).";
        let (style, entries) = split_reference_entries(section);
        assert_eq!(style, RefStyle::Numeric);
        assert_eq!(entries.len(), 3);
        assert!(entries[1].text.starts_with("Dzmitry Bahdanau"));
    }

    #[test]
    fn split_reference_entries_detects_author_year_bullets() {
        // 著者-年 bullet（本プロジェクトで失敗していた形式）
        let section = "- Ryan Holbrook Addison Howard, Ashley Chow. Spaceship titanic, 2022. URL https:// kaggle.com/competitions/spaceship-titanic .\n- DataCanary Anna Montoya. House prices - advanced regression techniques, 2016. URL https://kaggle.com/competitions/ house-prices-advanced-regression-techniqu";
        let (style, entries) = split_reference_entries(section);
        assert_eq!(style, RefStyle::AuthorYear);
        assert_eq!(entries.len(), 2);
        assert!(entries[0].text.starts_with("Ryan Holbrook"));
    }

    #[test]
    fn split_reference_entries_handles_numbered_paragraphs() {
        // PLOS / Neuron 形式（番号付き段落、空行分断）
        let section = "1. Deco G, Kringelbach ML (2014) Great expectations... Neuron 84:892-905.\n\n2. Biswal B, ... Functional connectivity... Magnetic Resonance in Medicine. 1995; 34:537.\n\n3. Third author et al (2020) Some title. Nature 100:100-110.";
        let (style, entries) = split_reference_entries(section);
        assert_eq!(style, RefStyle::Numeric);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].number, Some(1));
    }

    #[test]
    fn extract_entry_number_handles_all_prefix_forms() {
        assert_eq!(extract_entry_number("- [12] Foo. Bar (2020)."), Some(12));
        assert_eq!(extract_entry_number("[3] Foo. Bar."), Some(3));
        assert_eq!(extract_entry_number("* 5. Foo. Bar."), Some(5));
        assert_eq!(extract_entry_number("7) Foo. Bar."), Some(7));
        assert_eq!(extract_entry_number("Foo. Bar (2020)."), None);
        assert_eq!(extract_entry_number("2016 was a good year."), None);
    }

    #[test]
    fn parse_references_extracts_fields() {
        let refs = parse_references(SAMPLE, Journal::Generic);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].year, Some(2017));
        assert_eq!(refs[0].doi.as_deref(), Some("10.1234/abc"));
        assert!(refs[0].first_author.as_deref().unwrap().contains("Smith"));
    }

    #[test]
    fn parse_references_splits_author_year_bullets() {
        // 本プロジェクトで失敗していた SPIO 形式
        let md = "Body text.\n\n## References\n\n- Josh Achiam, Steven Adler, Sandhini Agarwal. 2023. Gpt-4 technical report. arXiv preprint arXiv:2303.08774 .\n- Sonnet Anthropic. 2024. Model card addendum: Claude 3.5 Sonnet.\n- Ryan Holbrook Addison Howard, Ashley Chow. Spaceship titanic, 2022.";
        let refs = parse_references(md, Journal::Generic);
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].year, Some(2023));
        assert!(
            refs[0]
                .first_author
                .as_deref()
                .unwrap_or("")
                .contains("Josh")
        );
        assert!(
            refs[0]
                .title
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains("gpt-4 technical report")
        );
    }

    #[test]
    fn parse_references_handles_parenthesized_year() {
        // Frontiers / Springer 形式
        let md = "## References\n\n- Abbott, L. F., & Chance, F. S. (2005). Drivers and modulators. Progress in Brain Research, 149, 147-155.\n- Elijah, D.H., Samengo, I., Montemurro, M.A. (2015) Thalamic neuron models. Front. Comput. Neurosci. 9:113.";
        let refs = parse_references(md, Journal::Generic);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].year, Some(2005));
        assert_eq!(refs[1].year, Some(2015));
        assert!(
            refs[0]
                .first_author
                .as_deref()
                .unwrap_or("")
                .contains("Abbott")
        );
        assert!(
            refs[0]
                .title
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains("drivers")
        );
    }

    #[test]
    fn parse_references_handles_paren_year_no_period_after() {
        // bioRxiv 形式（`(Year) Title. Journal` 括弧後のピリオド無し）
        let md = "## References\n\nByrne ´ A, Brookes MJ, Coombes S (2017) A mean field model for movement. Journal of Computational Neuroscience\n\nCucu O (2021) Title Available at: https://osf.io/3c6tv .";
        let refs = parse_references(md, Journal::Generic);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].year, Some(2017));
        assert_eq!(refs[1].year, Some(2021));
    }

    #[test]
    fn parse_references_handles_quoted_title_numeric() {
        // IEEE 形式
        let md = "## References\n\n[1] T. Brown, B. Mann. 'Language models are few-shot learners,' Advances in neural information processing systems, vol.33, 2020.\n[2] D. E. Rumelhart, G. E. Hinton, and R. J. Williams, 'Learning representations,' Nature, vol. 323, 1986.";
        let refs = parse_references(md, Journal::Generic);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].year, Some(2020));
        assert!(
            refs[0]
                .title
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains("few-shot")
        );
        assert!(
            refs[0]
                .first_author
                .as_deref()
                .unwrap_or("")
                .contains("Brown")
        );
    }

    #[test]
    fn parse_references_handles_initial_first_author_list() {
        // イニシャル先頭の著者リスト（arXiv / ACM 形式）— 誤って `V.` で分割しない
        let md = "## References\n\n- V. Botti, F. Barber, A. Crespo, E. Onaindia, A. Garcia-Fornes, I. Ripoll, D. Gallardo, and L. Hernández. A temporal blackboard for a multi-agent environment. Data &amp; Knowledge Engineering, 15(3): 189-211, 1995.\n- T. B. Brown, B. Mann, N. Ryder, M. Subbiah, and D. Amodei. Language models are few-shot learners. Advances in Neural Information Processing Systems, 33, 2020.";
        let refs = parse_references(md, Journal::Generic);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].year, Some(1995));
        // タイトルは著者の後から始まる
        assert!(
            refs[0]
                .title
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains("temporal blackboard")
        );
        // 第一著者（イニシャル先頭 → 姓）
        assert_eq!(refs[0].first_author.as_deref(), Some("Botti"));
        assert!(
            refs[1]
                .title
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains("few-shot")
        );
    }

    #[test]
    fn parse_references_strips_trailing_year_from_title() {
        // arXiv 形式 `Title, 2022` のタイトル末尾年号を除去する
        let md = "## References\n\n- Ryan Holbrook Addison Howard, Ashley Chow. Spaceship titanic, 2022. URL https://kaggle.com/competitions/spaceship-titanic";
        let refs = parse_references(md, Journal::Generic);
        assert_eq!(refs.len(), 1);
        let t = refs[0].title.as_deref().unwrap_or("").to_lowercase();
        assert_eq!(t.trim_end_matches(','), "spaceship titanic");
        assert!(!t.contains("2022"));
    }

    #[test]
    fn parse_references_handles_frontiers_author_year() {
        // Frontiers / Elsevier 形式（Accelerating earth science 論文）:
        // `Author, A., ..., and Author. (Year). Title. Journal. Volume:pages. doi: ...`
        let md = "## References\n\nAgashe, S., Fan, Y., Reyna, A., and Wang, X. E. (2024). Llm-coordination: evaluating and analyzing multi-agent coordination abilities. Available online at: https://arXiv.org/abs/2310.03903 (Accessed October 27, 2025).\n\nBai, J., Bai, S., Chu, Y ., Cui, Z., Dang, K., Deng, X., et al. (2023). Qwen technical report. arXiv preprint arXiv:2309.16609. doi: 10.48550/arXiv.2309.16609\n\nBauer, P ., Hoefler, T., Stevens, B., and Hazeleger, W . (2024). Digital twins of earth and the computing challenge of human interaction. Nat. Comput. Sci. 4,  154-157.  doi: 10.1038/s43588-024-00599-3\n\nDeng, C., Zhang, T., He, Z., Chen, Q., Shi, Y., Xu, Y., et al. (2024). K2: a foundation language model for geoscience knowledge understanding and utilization. Proceedings of the 17th ACM International Conference on Web Search and Data Mining. New York, NY: Association for Computing Machinery. 161-170.";
        let refs = parse_references(md, Journal::Generic);
        assert_eq!(refs.len(), 4);
        // 1) プレプリント: journal は None
        assert_eq!(refs[0].year, Some(2024));
        assert!(
            refs[0]
                .title
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains("llm-coordination")
        );
        assert_eq!(refs[0].journal, None);
        // 2) arXiv preprint
        assert_eq!(refs[1].journal.as_deref(), Some("arXiv preprint"));
        assert!(refs[1].doi.as_deref().unwrap_or("").contains("10.48550"));
        // 3) Frontiers 略称ジャーナル + 巻ページ/doi 除去
        assert_eq!(refs[2].journal.as_deref(), Some("Nat. Comput. Sci"));
        assert!(refs[2].doi.as_deref().unwrap_or("").contains("10.1038"));
        // 4) 出版社記述を除去した短いプロシーディング名
        assert_eq!(
            refs[3].journal.as_deref(),
            Some(
                "Proceedings of the 17th ACM International Conference on Web Search and Data Mining"
            )
        );
    }

    #[test]
    fn detect_journal_maps_doi_prefixes() {
        // 旧 Python detect_journal の DOI prefix 分岐を移植したもの
        assert_eq!(
            detect_journal(Some("10.1101/2025.10.20.683434")),
            Journal::Biorxiv
        );
        assert_eq!(
            detect_journal(Some("10.48550/arXiv.2410.20424")),
            Journal::Arxiv
        );
        assert_eq!(
            detect_journal(Some("10.3389/frai.2025.1674927")),
            Journal::Frontiers
        );
        assert_eq!(
            detect_journal(Some("10.1371/journal.pcbi.1012647")),
            Journal::Plos
        );
        assert_eq!(
            detect_journal(Some("10.1038/s41586-019-1716-z")),
            Journal::Nature
        );
        assert_eq!(
            detect_journal(Some("10.1007/s10827-024-00874-2")),
            Journal::Springer
        );
        assert_eq!(
            detect_journal(Some("10.1016/j.neuron.2021.06.026")),
            Journal::Elsevier
        );
        // 未知 DOI / 無 DOI → Generic
        assert_eq!(detect_journal(Some("10.1234/unknown")), Journal::Generic);
        assert_eq!(detect_journal(None), Journal::Generic);
        // arXiv DOI の arXiv 論文は Arxiv（原文の引用スタイル）
        assert_eq!(
            detect_journal(Some("10.48550/arXiv.2505.10978")),
            Journal::Arxiv
        );
    }

    #[test]
    fn parse_references_plos_removes_citation_header() {
        // PLOS: References 内の `Citation:` ヘッダ行がエントリとして拾われない
        let md = "## References\n\nCitation: Forrester M, Petros S, et al. (2024) Whole brain dynamics. PLoS Comput Biol 20(12): e1012647. https://doi.org/10.1371/journal.pcbi.1012647\n\n1. Deco G, Kringelbach ML (2014) Great expectations. Neuron 84:892-905.\n2. Biswal B, et al. (1995) Functional connectivity. Magnetic Resonance in Medicine. 1995; 34:537.\n3. Power JD, et al. (2011) Functional network organization. Neuron 72:665-678.";
        let refs = parse_references(md, Journal::Plos);
        // Citation 行は除外され、番号付き段落が 3 件になる
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].number, Some(1));
        assert_eq!(refs[1].number, Some(2));
        assert_eq!(refs[2].number, Some(3));
        // ジャーナルが正しく抽出される（`Neuron 84:892-905` → `Neuron`）
        assert_eq!(refs[0].journal.as_deref(), Some("Neuron"));
    }

    #[test]
    fn extract_year_from_parens_works() {
        assert_eq!(extract_year_from_parens("(2020)"), Some(2020));
        assert_eq!(extract_year_from_parens("no year"), None);
    }

    #[test]
    fn extract_doi_works() {
        assert_eq!(
            extract_doi("doi: 10.1038/nature12373."),
            Some("10.1038/nature12373".to_string())
        );
    }

    #[test]
    fn extract_first_author_handles_initials() {
        // イニシャル先頭形式
        let a = extract_first_author("J. N. Onuchic, Z. Luthey-Schulten, P. G. Wolynes").unwrap();
        assert_eq!(a, "Onuchic");
        // 姓, イニシャル形式（OCR 崩れ `´` 耐性）
        let b = extract_first_author("Byrne ´ A, Brookes MJ, Coombes S").unwrap();
        assert_eq!(b, "Byrne");
        // 団体著者
        let c = extract_first_author("Open X-Embodiment Collaboration.").unwrap();
        assert_eq!(c, "Open");
    }

    #[test]
    fn clean_raw_strips_markdown_links_and_entities() {
        let (text, url, doi) =
            clean_raw("Smith (2020) Title. Journal. [doi](https://doi.org/10.1234/abc)");
        assert!(doi.as_deref() == Some("10.1234/abc"));
        assert_eq!(url.as_deref(), Some("https://doi.org/10.1234/abc"));
        assert!(!text.contains("[doi]"));
        let (text2, _, _) = clean_raw("A &amp; B title.");
        assert!(text2.contains("& B"));
    }
}
