//! `sai-paper expand` — 引用の網を 1 段だけ広げ、Library に無い候補を外部検索の履歴に残す。
//!
//! S2 から引用（references）と被引用（citations）を 1 段だけ取り、`papers` テーブルの
//! DOI・arXiv ID・S2 paperId と照らして、Library に無いものだけを 1 回の外部検索
//! （`search_history` の 1 行）として記録する。ネットを使うのはこのコマンドを実行したときだけ。
//!
//! 取得（S2 への問い合わせ）と照合（Library との突き合わせ）を分けてある。
//! 照合の部分は `LibraryIndex` と `filter_new_candidates` で、ネット無しで試せる。

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use rusqlite::Connection;
use sai_paper_core::api::s2::{S2Client, S2PaperDetail};
use sai_paper_core::figures;
use sai_paper_core::models::RemotePaper;
use std::collections::HashSet;

/// 引用の網をたどる向き。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// その論文が引用している論文だけ（references）。
    Refs,
    /// その論文を引用している論文だけ（citations）。
    Cited,
    /// 両方。
    Both,
}

impl Direction {
    /// `--direction` の文字列を解釈する（`both` / `refs` / `cited`）。
    ///
    /// 既存の `sai-paper graph` の書き方（`references` / `citations` / `out` / `in`）も
    /// 受け付ける。
    ///
    /// # Errors
    ///
    /// 解釈できない文字列のとき。
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "both" => Ok(Self::Both),
            "refs" | "references" | "out" => Ok(Self::Refs),
            "cited" | "citations" | "in" => Ok(Self::Cited),
            other => bail!("invalid --direction: {other} (both / refs / cited)"),
        }
    }

    /// 引用（references）を取るか。
    fn fetches_refs(self) -> bool {
        matches!(self, Self::Refs | Self::Both)
    }

    /// 被引用（citations）を取るか。
    fn fetches_cited(self) -> bool {
        matches!(self, Self::Cited | Self::Both)
    }
}

/// S2 から取った近傍論文 1 件（Library との照合に使う識別子を添えたもの）。
#[derive(Debug, Clone)]
pub struct ExpandCandidate {
    /// 履歴に残す形の論文。
    pub paper: RemotePaper,
    /// DOI（`externalIds` の `DOI`）。
    pub doi: Option<String>,
    /// arXiv ID（`externalIds` の `ArXiv`）。
    pub arxiv_id: Option<String>,
    /// S2 paperId。
    pub s2_id: Option<String>,
}

/// S2 の論文詳細 1 件を候補に直す。
///
/// S2 paperId・DOI・arXiv ID のどれも無いものは、Library と照らせず後から
/// 取り込むこともできないので `None` にする（`build_graph` が paperId の無い
/// エントリを飛ばすのに合わせる）。
fn candidate_from_s2(detail: S2PaperDetail) -> Option<ExpandCandidate> {
    let s2_id = detail
        .paper_id
        .clone()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let doi = detail
        .external_ids
        .as_ref()
        .and_then(|ids| ids.get("DOI"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let arxiv_id = detail
        .external_ids
        .as_ref()
        .and_then(|ids| ids.get("ArXiv"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if s2_id.is_none() && doi.is_none() && arxiv_id.is_none() {
        return None;
    }
    let title = detail.title.clone().unwrap_or_default();
    // references / citations の応答は年月日を返さないので、年から 1 月 1 日を作る。
    let date = NaiveDate::from_ymd_opt(detail.year.unwrap_or(2000), 1, 1)
        .or_else(|| NaiveDate::from_ymd_opt(2000, 1, 1))
        .expect("2000-01-01 は有効な日付")
        .and_hms_opt(0, 0, 0)
        .expect("00:00:00 は有効な時刻")
        .and_utc();
    let pdf_url = detail
        .open_access_pdf
        .as_ref()
        .and_then(|pdf| pdf.url.clone())
        .filter(|url| !url.trim().is_empty())
        .or_else(|| {
            arxiv_id
                .as_ref()
                .map(|id| format!("https://arxiv.org/pdf/{id}.pdf"))
        });
    let journal_ref = detail
        .journal
        .as_ref()
        .and_then(|journal| journal.name.clone())
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            detail
                .venue
                .clone()
                .filter(|venue| !venue.trim().is_empty())
        });
    let authors = detail
        .authors
        .unwrap_or_default()
        .into_iter()
        .filter_map(|author| author.name)
        .collect();

    Some(ExpandCandidate {
        paper: RemotePaper {
            id: s2_id.clone().unwrap_or_default(),
            title,
            authors,
            abstract_text: detail.abstract_text.unwrap_or_default(),
            published: date,
            updated: date,
            categories: vec![],
            pdf_url,
            doi: doi.clone(),
            journal_ref,
            citation_count: detail.citation_count,
        },
        doi,
        arxiv_id,
        s2_id,
    })
}

/// S2 の論文詳細の並びを候補に直し、同じ論文（`dedupe_key` が同じ）を 1 つにまとめる。
#[must_use]
pub fn candidates_from_details(details: Vec<S2PaperDetail>) -> Vec<ExpandCandidate> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut candidates = Vec::new();
    for detail in details {
        let Some(candidate) = candidate_from_s2(detail) else {
            continue;
        };
        if seen.insert(sai_paper_core::api::dedupe_key(&candidate.paper)) {
            candidates.push(candidate);
        }
    }
    candidates
}

/// Library（`papers` テーブル）に既にある論文の識別子（DOI・arXiv ID・S2 paperId）。
#[derive(Debug, Default, Clone)]
pub struct LibraryIndex {
    dois: HashSet<String>,
    arxiv_ids: HashSet<String>,
    s2_ids: HashSet<String>,
}

impl LibraryIndex {
    /// `papers` テーブルの DOI・arXiv ID・S2 paperId をすべて読み込む。
    ///
    /// # Errors
    ///
    /// SQLite エラーのとき。
    pub fn from_conn(conn: &Connection) -> Result<Self> {
        let mut stmt = conn
            .prepare("SELECT doi, arxiv_id, s2_paper_id FROM papers")
            .context("Library の識別子の読み込みに失敗")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Self::from_rows(rows))
    }

    /// (DOI, arXiv ID, S2 paperId) の並びから作る（テストからも使う）。
    #[must_use]
    pub fn from_rows<I>(rows: I) -> Self
    where
        I: IntoIterator<Item = (Option<String>, Option<String>, Option<String>)>,
    {
        let mut index = Self::default();
        for (doi, arxiv, s2) in rows {
            if let Some(doi) = doi {
                let key = normalize_doi(&doi);
                if !key.is_empty() {
                    index.dois.insert(key);
                }
            }
            if let Some(arxiv) = arxiv {
                let key = normalize_arxiv(&arxiv);
                if !key.is_empty() {
                    index.arxiv_ids.insert(key);
                }
            }
            if let Some(s2) = s2 {
                let key = normalize_s2(&s2);
                if !key.is_empty() {
                    index.s2_ids.insert(key);
                }
            }
        }
        index
    }

    /// 候補が Library に既にあるか（DOI・arXiv ID・S2 paperId のどれかで一致）。
    #[must_use]
    pub fn contains(&self, candidate: &ExpandCandidate) -> bool {
        candidate
            .doi
            .as_deref()
            .map(normalize_doi)
            .is_some_and(|key| self.dois.contains(&key))
            || candidate
                .arxiv_id
                .as_deref()
                .map(normalize_arxiv)
                .is_some_and(|key| self.arxiv_ids.contains(&key))
            || candidate
                .s2_id
                .as_deref()
                .map(normalize_s2)
                .is_some_and(|key| self.s2_ids.contains(&key))
    }
}

/// DOI を照合できる形にそろえる（前後の空白を落とし、小文字にする）。
fn normalize_doi(value: &str) -> String {
    value.trim().to_lowercase()
}

/// arXiv ID を照合できる形にそろえる（`ArXiv:` と版番号 `vN` を落とす）。
fn normalize_arxiv(value: &str) -> String {
    let lower = value.trim().to_lowercase();
    let stripped = lower.strip_prefix("arxiv:").unwrap_or(&lower);
    strip_arxiv_version(stripped).to_string()
}

/// 末尾の版番号（`v2`、`v10`）を落とす。
fn strip_arxiv_version(value: &str) -> &str {
    if let Some(pos) = value.rfind('v') {
        let suffix = &value[pos + 1..];
        if pos > 0 && !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
            return &value[..pos];
        }
    }
    value
}

/// S2 paperId を照合できる形にそろえる（前後の空白を落とし、小文字にする）。
fn normalize_s2(value: &str) -> String {
    value.trim().to_lowercase()
}

/// 候補を「Library に無いもの」と「Library に既にあるもの」に分ける。
///
/// 返り値は (Library に無い候補, Library にあった数)。
#[must_use]
pub fn filter_new_candidates(
    candidates: Vec<ExpandCandidate>,
    library: &LibraryIndex,
) -> (Vec<ExpandCandidate>, usize) {
    let mut new_candidates = Vec::new();
    let mut existing = 0usize;
    for candidate in candidates {
        if library.contains(&candidate) {
            existing += 1;
        } else {
            new_candidates.push(candidate);
        }
    }
    (new_candidates, existing)
}

/// 1 本の論文についての expand の結果。
#[derive(Debug, Default)]
struct PaperOutcome {
    paper_id: i64,
    key: String,
    refs: usize,
    cited: usize,
    already: usize,
    added: usize,
    error: Option<String>,
}

impl PaperOutcome {
    /// 人が読む 1 行。
    fn line(&self) -> String {
        if self.refs == 0 && self.cited == 0 && self.added == 0 {
            if let Some(error) = &self.error {
                return format!("#{}: {error}", self.paper_id);
            }
        }
        let mut line = format!(
            "#{}: 引用 {}・被引用 {} → Library にある {}・新しい候補 {}",
            self.paper_id, self.refs, self.cited, self.already, self.added,
        );
        if let Some(error) = &self.error {
            line.push_str(&format!("（{error}）"));
        }
        line
    }

    /// JSON に出す形。
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "paper_id": self.paper_id,
            "key": self.key,
            "refs": self.refs,
            "cited": self.cited,
            "already": self.already,
            "added": self.added,
            "error": self.error,
        })
    }
}

/// `sai-paper expand` の本体。
///
/// たどって見つけた候補は、全部まとめて 1 回の外部検索として `search_history` に残す。
///
/// # Errors
///
/// DB を開けない、`papers` を 1 つも指定していないとき。
pub async fn handle(
    client: &S2Client,
    papers: Vec<i64>,
    direction: Direction,
    limit: usize,
    json: bool,
) -> Result<()> {
    if papers.is_empty() {
        bail!("論文の番号を 1 つ以上指定してください");
    }

    let (db, cfg, paths) = crate::phone::open_context()?;
    let conn = db.connection();
    let library = LibraryIndex::from_conn(conn)?;

    let mut outcomes = Vec::new();
    let mut found: Vec<RemotePaper> = Vec::new();
    for &paper_id in &papers {
        let (outcome, candidates) =
            expand_one(conn, client, &library, paper_id, direction, limit).await?;
        found.extend(candidates);
        if json {
            outcomes.push(outcome.to_json());
        } else {
            println!("{}", outcome.line());
        }
    }

    // 重複（`dedupe_key`）は `record` の中で除かれる。
    let ids: Vec<String> = papers.iter().map(i64::to_string).collect();
    let executer = crate::executer_name();
    let log_id = sai_paper_core::search_history::record(
        conn,
        crate::origin_name(),
        executer,
        crate::run_id_for(executer),
        &format!("expand: {}", ids.join(",")),
        "",
        &limit.to_string(),
        "s2",
        &found,
    )?;

    if json {
        let mut value = crate::phone::log(&db, &cfg, &paths, log_id)?;
        if let serde_json::Value::Object(map) = &mut value {
            map.insert("results".to_string(), serde_json::Value::Array(outcomes));
        }
        println!("{}", serde_json::to_string(&value)?);
    }
    Ok(())
}

/// 1 本の論文の引用の前後を 1 段たどり、Library に無い候補を返す。
///
/// S2 への問い合わせに失敗しても、そこまでに取れた結果は返し、理由を
/// `error` に残す（コマンド全体は止めない）。
///
/// # Errors
///
/// DB エラーのとき。
async fn expand_one(
    conn: &Connection,
    client: &S2Client,
    library: &LibraryIndex,
    paper_id: i64,
    direction: Direction,
    limit: usize,
) -> Result<(PaperOutcome, Vec<RemotePaper>)> {
    let mut outcome = PaperOutcome {
        paper_id,
        ..PaperOutcome::default()
    };

    let Some(key) = figures::extract_key_for_paper(conn, paper_id)? else {
        outcome.error = Some("s2_paper_id / arxiv_id / doi が未登録です".to_string());
        return Ok((outcome, Vec::new()));
    };
    outcome.key = key.clone();

    let mut details: Vec<S2PaperDetail> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    if direction.fetches_refs() {
        match client.get_paper_references(&key).await {
            Ok(mut refs) => {
                refs.truncate(limit);
                outcome.refs = refs.len();
                details.extend(refs);
            }
            Err(error) => errors.push(format!("引用: {error}")),
        }
    }
    if direction.fetches_cited() {
        match client.get_paper_citations(&key).await {
            Ok(mut cited) => {
                cited.truncate(limit);
                outcome.cited = cited.len();
                details.extend(cited);
            }
            Err(error) => errors.push(format!("被引用: {error}")),
        }
    }

    let candidates = candidates_from_details(details);
    let (new_candidates, existing) = filter_new_candidates(candidates, library);
    outcome.already = existing;
    outcome.added = new_candidates.len();

    if !errors.is_empty() {
        outcome.error = Some(errors.join("、"));
    }
    let papers = new_candidates.into_iter().map(|candidate| candidate.paper).collect();
    Ok((outcome, papers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sai_paper_core::Database;

    /// S2 の references / citations 応答から抜き出した見本（ネットは使わない）。
    const S2_SAMPLE: &str = r#"[
        {
            "paperId": "aaaa000000000000000000000000000000000001",
            "title": "Known by DOI",
            "year": 2019,
            "externalIds": {"DOI": "10.1000/KNOWN.1"}
        },
        {
            "paperId": "bbbb000000000000000000000000000000000002",
            "title": "Known by arXiv",
            "year": 2021,
            "externalIds": {"ArXiv": "2401.00001v2"}
        },
        {
            "paperId": "cccc000000000000000000000000000000000003",
            "title": "Known by S2 id",
            "year": 2018
        },
        {
            "paperId": "dddd000000000000000000000000000000000004",
            "title": "Brand new paper",
            "year": 2024,
            "externalIds": {"DOI": "10.1000/new.9"}
        },
        {
            "title": "No identifier at all"
        }
    ]"#;

    fn sample_details() -> Vec<S2PaperDetail> {
        serde_json::from_str(S2_SAMPLE).expect("見本の JSON を読める")
    }

    fn library_with_three_known() -> LibraryIndex {
        LibraryIndex::from_rows(vec![
            (Some("10.1000/known.1".to_string()), None, None),
            (None, Some("2401.00001".to_string()), None),
            (
                None,
                None,
                Some("CCCC000000000000000000000000000000000003".to_string()),
            ),
        ])
    }

    #[test]
    fn ac1_s2_sample_parses_to_candidates() {
        let candidates = candidates_from_details(sample_details());
        // paperId もタイトルも無い 1 件だけが落ちる。
        assert_eq!(candidates.len(), 4);
        assert_eq!(candidates[0].doi.as_deref(), Some("10.1000/KNOWN.1"));
        assert_eq!(candidates[1].arxiv_id.as_deref(), Some("2401.00001v2"));
        assert_eq!(
            candidates[3].paper.title, "Brand new paper",
            "見本の 4 番目が新しい候補"
        );
    }

    #[test]
    fn ac1_excludes_papers_already_in_library() {
        let candidates = candidates_from_details(sample_details());
        let library = library_with_three_known();

        let (new_candidates, existing) = filter_new_candidates(candidates, &library);

        assert_eq!(existing, 3, "DOI・arXiv・S2 ID で 1 件ずつ既にある");
        assert_eq!(new_candidates.len(), 1);
        assert_eq!(new_candidates[0].paper.title, "Brand new paper");
    }

    #[test]
    fn ac2_candidates_are_recorded_once_in_search_history() {
        let db = Database::in_memory().expect("メモリ DB を作れる");
        let conn = db.connection();

        let candidates = candidates_from_details(sample_details());
        // 同じ候補が 2 本の論文から見つかっても、1 回の履歴に 1 つずつだけ入る。
        let papers: Vec<RemotePaper> = candidates
            .iter()
            .chain(candidates.iter())
            .map(|candidate| candidate.paper.clone())
            .collect();
        let log_id = sai_paper_core::search_history::record(
            conn, "cli", "human", None, "expand: 1,2", "", "20", "s2", &papers,
        )
        .expect("履歴に残せる");

        let items = sai_paper_core::search_history::items(conn, log_id).expect("候補を読める");
        assert_eq!(items.len(), candidates.len(), "候補は 1 つずつだけ");
    }

    #[test]
    fn arxiv_version_suffix_is_ignored() {
        assert_eq!(normalize_arxiv("ArXiv:2401.00001v2"), "2401.00001");
        assert_eq!(normalize_arxiv("2401.00001"), "2401.00001");
        assert_eq!(normalize_doi(" 10.1000/Known.1 "), "10.1000/known.1");
    }
}
