//! 検索結果の被引用数を OpenAlex でまとめて補う。
//!
//! S2・PubMed・Europe PMC・OpenAlex の検索結果は被引用数を持つが、arXiv などは持たない。
//! 検索のあと、`citation_count` が `None` の論文だけを OpenAlex に問い合わせて埋める。
//! LLM は使わない。

use std::collections::HashMap;
use std::time::Duration;

use crate::models::RemotePaper;
use serde_json::Value;

use super::openalex::{strip_doi_prefix, OpenAlexClient};

/// 1 つの filter に詰められる問い合わせ先の最大個数。
const BATCH_SIZE: usize = 100;
/// 1 回の要求の待ち時間の上限。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// `citation_count` が `None` の論文に、OpenAlex で引いた被引用数を入れる。
///
/// 問い合わせ先は、DOI があればその DOI、無ければ arXiv の ID から作った
/// `10.48550/arxiv.<id>`。どちらも無ければ補わない。
///
/// # Arguments
///
/// - `cfg`: 設定。OpenAlex の API キー（`discover.openalex_api_key_env`）を読む。
/// - `papers`: 補完の対象。値を書き換える。
///
/// # Returns
///
/// 補えた論文の件数。失敗（ネットワーク・429・読み取り失敗）は無視する。
pub async fn fill_citation_counts(cfg: &crate::Config, papers: &mut [RemotePaper]) -> usize {
    // 被引用数が既にある論文は問い合わせない
    let (dois, arxiv_ids) = lookup_keys(papers);
    if dois.is_empty() && arxiv_ids.is_empty() {
        return 0;
    }
    let arxiv_dois = arxiv_dois(&arxiv_ids);

    let client = OpenAlexClient::from_config(cfg);

    // 1 段目: DOI で引く（100 個ずつ）
    let mut doi_counts: HashMap<String, i32> = HashMap::new();
    for batch in dois.chunks(BATCH_SIZE) {
        if let Some(json) = get_json(&client, &format!("doi:{}", batch.join("|"))).await {
            doi_counts.extend(counts_from_response(&json));
        }
    }

    // 2 段目: DOI で引けなかった arXiv 論文を、arXiv のページの URL で引き直す
    // （arXiv の DOI で見つからない論文がある。1706.03762 がそうだった）
    let remaining: Vec<String> = arxiv_ids
        .iter()
        .filter(|id| !doi_counts.contains_key(&arxiv_dois[id.as_str()]))
        .cloned()
        .collect();
    let mut landing_counts: HashMap<String, i32> = HashMap::new();
    for batch in remaining.chunks(BATCH_SIZE) {
        let urls: Vec<String> = batch
            .iter()
            .map(|id| format!("http://arxiv.org/abs/{id}"))
            .collect();
        let filter = format!("locations.landing_page_url:{}", urls.join("|"));
        if let Some(json) = get_json(&client, &filter).await {
            landing_counts.extend(landing_counts_from_response(&json));
        }
    }

    apply_counts(papers, &doi_counts, &arxiv_dois, &landing_counts)
}

/// 補完が必要な論文の問い合わせ先を集める（DOI は小文字、重複は除く）。
fn lookup_keys(papers: &[RemotePaper]) -> (Vec<String>, Vec<String>) {
    let mut dois: Vec<String> = Vec::new();
    let mut arxiv_ids: Vec<String> = Vec::new();
    for paper in papers {
        if paper.citation_count.is_some() {
            continue;
        }
        match lookup_key(paper) {
            Some(Lookup::Doi(doi)) => {
                if !dois.contains(&doi) {
                    dois.push(doi);
                }
            }
            Some(Lookup::Arxiv(id)) => {
                if !arxiv_ids.contains(&id) {
                    arxiv_ids.push(id);
                }
            }
            None => continue,
        }
    }
    (dois, arxiv_ids)
}

/// arXiv ID から arXiv の DOI を作る。
fn arxiv_dois(arxiv_ids: &[String]) -> HashMap<String, String> {
    arxiv_ids
        .iter()
        .map(|id| (id.clone(), format!("10.48550/arxiv.{}", id.to_lowercase())))
        .collect()
}

/// 引いた被引用数を論文に入れる（`citation_count` が `None` の論文だけ）。
///
/// 返り値は入れた件数。arXiv の URL で引いた値（`landing_counts`）を先に、
/// 無ければ DOI で引いた値を使う。
fn apply_counts(
    papers: &mut [RemotePaper],
    doi_counts: &HashMap<String, i32>,
    arxiv_dois: &HashMap<String, String>,
    landing_counts: &HashMap<String, i32>,
) -> usize {
    let mut filled = 0;
    for paper in papers.iter_mut() {
        if paper.citation_count.is_some() {
            continue;
        }
        let count = match lookup_key(paper) {
            Some(Lookup::Doi(doi)) => doi_counts.get(&doi).copied(),
            Some(Lookup::Arxiv(id)) => {
                landing_counts.get(&id).copied().or_else(|| {
                    arxiv_dois
                        .get(&id)
                        .and_then(|doi| doi_counts.get(doi))
                        .copied()
                })
            }
            None => None,
        };
        if let Some(count) = count {
            paper.citation_count = Some(count);
            filled += 1;
        }
    }
    filled
}

/// 論文の問い合わせ先。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Lookup {
    /// その DOI（小文字）。
    Doi(String),
    /// 版番号を落とした arXiv ID。
    Arxiv(String),
}

/// 論文の問い合わせ先を決める。DOI が無ければ arXiv の ID を見る。
fn lookup_key(paper: &RemotePaper) -> Option<Lookup> {
    if let Some(doi) = paper
        .doi
        .as_deref()
        .and_then(strip_doi_prefix)
        .map(|doi| doi.to_lowercase())
        .filter(|doi| !doi.is_empty())
    {
        return Some(Lookup::Doi(doi));
    }
    arxiv_id(&paper.id).map(Lookup::Arxiv)
}

/// `http://arxiv.org/abs/2005.14165v1` や `2005.14165v2` から、版番号を落とした
/// arXiv ID を取り出す。arXiv の ID でなければ `None`。
fn arxiv_id(id: &str) -> Option<String> {
    let id = id.trim();
    let tail = match id.split_once("arxiv.org/abs/") {
        Some((_, tail)) => tail,
        None => id,
    };
    // 版番号（`v1` など）を落とす
    let number = match tail.split_once('v') {
        Some((number, rest))
            if is_arxiv_number(number)
                && !rest.is_empty()
                && rest.chars().all(|c| c.is_ascii_digit()) =>
        {
            number
        }
        _ => tail,
    };
    is_arxiv_number(number).then(|| number.to_string())
}

/// `2005.14165`（4 桁の年月 + 4〜5 桁の連番）の形か。
fn is_arxiv_number(text: &str) -> bool {
    text.split_once('.').is_some_and(|(prefix, number)| {
        prefix.len() == 4
            && prefix.chars().all(|c| c.is_ascii_digit())
            && (4..=5).contains(&number.len())
            && number.chars().all(|c| c.is_ascii_digit())
    })
}

/// `GET /works?filter=…` を投げ、JSON を返す。失敗したら `None`。
async fn get_json(client: &OpenAlexClient, filter: &str) -> Option<Value> {
    let mut url = url::Url::parse("https://api.openalex.org/works").ok()?;
    url.query_pairs_mut()
        .append_pair("filter", filter)
        .append_pair("per-page", &BATCH_SIZE.to_string());
    let response = client
        .request(url)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

/// DOI で引く応答の JSON から、素の DOI（小文字）と被引用数を取り出す。
fn counts_from_response(json: &Value) -> HashMap<String, i32> {
    let mut counts = HashMap::new();
    for work in results(json) {
        let (Some(doi), Some(count)) = (
            work.get("doi")
                .and_then(Value::as_str)
                .and_then(strip_doi_prefix),
            work.get("cited_by_count").and_then(Value::as_i64),
        ) else {
            continue;
        };
        counts.insert(doi.to_lowercase(), count as i32);
    }
    counts
}

/// arXiv のページの URL で引く応答の JSON から、arXiv ID と被引用数を取り出す。
///
/// 結果との対応は、`locations[].landing_page_url` に ID が含まれるかで決める。
fn landing_counts_from_response(json: &Value) -> HashMap<String, i32> {
    let mut counts = HashMap::new();
    for work in results(json) {
        let Some(count) = work.get("cited_by_count").and_then(Value::as_i64) else {
            continue;
        };
        let Some(locations) = work.get("locations").and_then(Value::as_array) else {
            continue;
        };
        for location in locations {
            let Some(url) = location.get("landing_page_url").and_then(Value::as_str) else {
                continue;
            };
            if let Some(id) = arxiv_id(url) {
                counts.entry(id).or_insert(count as i32);
            }
        }
    }
    counts
}

/// 応答 JSON の `results` 配列を取り出す。形式が違えば空。
fn results(json: &Value) -> impl Iterator<Item = &Value> {
    json.get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|value| value.is_object())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn paper(id: &str, doi: Option<&str>, citation_count: Option<i32>) -> RemotePaper {
        RemotePaper {
            id: id.to_string(),
            title: "T".to_string(),
            authors: Vec::new(),
            abstract_text: String::new(),
            published: Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap(),
            updated: Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap(),
            categories: Vec::new(),
            pdf_url: None,
            doi: doi.map(str::to_string),
            journal_ref: None,
            citation_count,
        }
    }

    #[test]
    fn lookup_key_prefers_doi_then_arxiv_id() {
        // DOI があればその DOI（小文字）
        assert_eq!(
            lookup_key(&paper(
                "http://arxiv.org/abs/2005.14165v1",
                Some("https://doi.org/10.1/ABC"),
                None
            )),
            Some(Lookup::Doi("10.1/abc".to_string()))
        );
        // DOI が無ければ arXiv の ID（版番号を落とす）
        assert_eq!(
            lookup_key(&paper("http://arxiv.org/abs/2005.14165v1", None, None)),
            Some(Lookup::Arxiv("2005.14165".to_string()))
        );
        assert_eq!(
            lookup_key(&paper("2005.14165v2", None, None)),
            Some(Lookup::Arxiv("2005.14165".to_string()))
        );
        // どちらも無ければ補わない
        assert_eq!(lookup_key(&paper("W123", None, None)), None);
        assert_eq!(
            lookup_key(&paper("s2:abc", Some("10.1/abc"), None)),
            Some(Lookup::Doi("10.1/abc".to_string()))
        );
    }

    #[test]
    fn arxiv_id_accepts_only_arxiv_shaped_ids() {
        assert_eq!(
            arxiv_id("http://arxiv.org/abs/1706.03762v1").as_deref(),
            Some("1706.03762")
        );
        assert_eq!(arxiv_id("1706.03762").as_deref(), Some("1706.03762"));
        assert_eq!(arxiv_id("W2741809807"), None);
        assert_eq!(arxiv_id("http://arxiv.org/abs/notanid"), None);
        assert_eq!(arxiv_id("10.1/abc"), None);
    }

    #[test]
    fn lookup_keys_skips_papers_that_already_have_counts() {
        let papers = vec![
            paper("http://arxiv.org/abs/2005.14165v1", None, None),
            paper("W1", Some("10.1/known"), Some(7)),
            paper("W2", Some("10.1/known"), None),
        ];
        let (dois, arxiv_ids) = lookup_keys(&papers);
        assert_eq!(dois, vec!["10.1/known".to_string()]);
        assert_eq!(arxiv_ids, vec!["2005.14165".to_string()]);
    }

    #[test]
    fn counts_from_response_reads_doi_and_cited_by_count() {
        let json: Value = serde_json::from_str(
            r#"{"results":[
                 {"doi":"https://doi.org/10.1038/NATURE14539","cited_by_count":85067},
                 {"doi":"https://doi.org/10.48550/arxiv.2005.14165","cited_by_count":3001},
                 {"doi":"https://doi.org/10.1/missing-count"},
                 {"cited_by_count":5}
               ]}"#,
        )
        .unwrap();
        let counts = counts_from_response(&json);
        assert_eq!(counts.get("10.1038/nature14539"), Some(&85067));
        assert_eq!(counts.get("10.48550/arxiv.2005.14165"), Some(&3001));
        assert_eq!(counts.len(), 2);
    }

    #[test]
    fn landing_counts_from_response_matches_arxiv_urls() {
        let json: Value = serde_json::from_str(
            r#"{"results":[
                 {"cited_by_count":26907,
                  "locations":[{"landing_page_url":"https://doi.org/10.65215/2q58a426"},
                               {"landing_page_url":"http://arxiv.org/abs/1706.03762v5"}]},
                 {"cited_by_count":7,
                  "locations":[{"landing_page_url":"http://example.com/x"}]}
               ]}"#,
        )
        .unwrap();
        let counts = landing_counts_from_response(&json);
        assert_eq!(counts.get("1706.03762"), Some(&26907));
        assert_eq!(counts.len(), 1);
    }

    #[test]
    fn apply_counts_only_touches_papers_without_counts() {
        let mut papers = vec![
            paper("http://arxiv.org/abs/2005.14165v1", None, None),
            paper("W1", Some("10.1/known"), Some(7)),
            paper("W2", Some("10.1/other"), None),
            paper("W3", None, None),
        ];
        let doi_counts: HashMap<String, i32> = HashMap::from([
            ("10.1/known".to_string(), 111),
            ("10.1/other".to_string(), 222),
            ("10.48550/arxiv.2005.14165".to_string(), 3001),
        ]);
        let arxiv_dois = arxiv_dois(&["2005.14165".to_string()]);
        // arXiv の ID を持つ論文は、arXiv の DOI を見て値が入る
        let filled = apply_counts(&mut papers, &doi_counts, &arxiv_dois, &HashMap::new());
        assert_eq!(filled, 2);
        assert_eq!(papers[0].citation_count, Some(3001));
        // すでに値のある論文は変わらない
        assert_eq!(papers[1].citation_count, Some(7));
        assert_eq!(papers[2].citation_count, Some(222));
        // 問い合わせ先が無い論文は変わらない
        assert_eq!(papers[3].citation_count, None);

        // arXiv の URL で引けた値があれば、そちらを使う
        let mut papers = vec![paper("http://arxiv.org/abs/2005.14165v1", None, None)];
        let landing = HashMap::from([("2005.14165".to_string(), 999)]);
        assert_eq!(apply_counts(&mut papers, &doi_counts, &arxiv_dois, &landing), 1);
        assert_eq!(papers[0].citation_count, Some(999));
    }
}
