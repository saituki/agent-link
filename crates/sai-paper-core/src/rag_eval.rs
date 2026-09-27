//! RAG（Library の検索）の成績を測る。
//!
//! ユーザーが作る質問集（`.sai/paper/eval/rag-questions.json`）の問いごとに検索を実行し、
//! 「正解の論文が上位 1・5・10 件に入った割合」と「最初の正解の順位の逆数の平均（MRR）」を
//! 索引ごとに出す。LLM は使わない（検索を実行して数えるだけ）。
//!
//! 問いは英語だけを前提にする（エージェントは日本語の問いを英語の検索語に直してから
//! 検索する決まりなので、英語の問いで測れば足りる）。

use crate::retrieval::{FindRequest, Hit, Result, find};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// 質問集の置き場所（`<root>/.sai/paper/` からの相対パス）。
pub const QUESTIONS_RELATIVE_PATH: &str = "eval/rag-questions.json";

/// 質問集の見本（ファイルが無いときに CLI が出す）。
pub const SAMPLE_QUESTIONS_JSON: &str = r#"[
  {
    "q": "Which seizure prediction studies evaluate across subjects?",
    "expect": [
      { "paper": 37 },
      { "paper": 12, "section": "Evaluation" }
    ]
  },
  {
    "q": "Why is a notch filter used in EEG preprocessing?",
    "expect": [
      { "paper": 5 }
    ]
  }
]"#;

/// 質問集の 1 問。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct EvalQuestion {
    /// 英語の問い。
    pub q: String,
    /// 上位に出てほしい論文（と、分かれば章）。
    #[serde(default)]
    pub expect: Vec<Expect>,
}

/// 上位に出てほしい論文 1 件。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Expect {
    /// 論文の番号（Library で出典を示す `paper_id`）。
    pub paper: i64,
    /// 章の名前。指定があり、結果にも章があるときは、結果の章がこれを含むことを求める。
    #[serde(default)]
    pub section: Option<String>,
}

/// 1 問分の成績。
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionScore {
    /// 上位から見て、正解に最初に当たった順位（1 始まり）。圏外は `None`。
    pub first_hit_rank: Option<usize>,
}

/// 索引 1 つ分の成績。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvalSummary {
    /// 索引の名前。
    pub index: String,
    /// 検索モード（word/vector/hybrid）。
    pub mode: String,
    /// 問いの数。
    pub n: usize,
    /// 正解が 1 位に入った割合。
    pub hit_at_1: f64,
    /// 正解が上位 5 件に入った割合。
    pub hit_at_5: f64,
    /// 正解が上位 10 件に入った割合。
    pub hit_at_10: f64,
    /// 最初の正解の順位の逆数の平均。
    pub mrr: f64,
}

/// 質問集の読み込み・形のエラー。
#[derive(Debug, Error)]
pub enum EvalError {
    /// 質問集のファイルが無い。
    #[error("質問集がありません: {}", .path.display())]
    NotFound {
        /// 探したパス。
        path: PathBuf,
    },
    /// 質問集のファイルが読めない。
    #[error("質問集を読めません: {}（{source}）", .path.display())]
    Read {
        /// 読もうとしたパス。
        path: PathBuf,
        /// 元の IO エラー。
        source: std::io::Error,
    },
    /// JSON として読めない。
    #[error("質問集の JSON が壊れています: {}: {source}", .path.display())]
    Parse {
        /// 読もうとしたパス。
        path: PathBuf,
        /// 元の JSON エラー（行・列を含む）。
        source: serde_json::Error,
    },
    /// 問いの形が違う。
    #[error("質問集の形が違います: {}: {} 番目の問い: {message}", .path.display(), .index + 1)]
    Question {
        /// 読もうとしたパス。
        path: PathBuf,
        /// 0 始まりの問いの位置。
        index: usize,
        /// 何が足りないか。
        message: String,
    },
}

/// 質問集のファイルを読む。
///
/// # Errors
///
/// ファイルが無い（[`EvalError::NotFound`]）・読めない・JSON が壊れている・
/// 問いの形が違う。
pub fn load_questions(path: &Path) -> std::result::Result<Vec<EvalQuestion>, EvalError> {
    if !path.exists() {
        return Err(EvalError::NotFound {
            path: path.to_path_buf(),
        });
    }
    let text = std::fs::read_to_string(path).map_err(|source| EvalError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|source| EvalError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    let Some(items) = value.as_array() else {
        return Err(EvalError::Question {
            path: path.to_path_buf(),
            index: 0,
            message: "一番外側は問いの配列（`[ ... ]`）にしてください".to_string(),
        });
    };

    let mut questions = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let question_error = |message: &str| EvalError::Question {
            path: path.to_path_buf(),
            index,
            message: message.to_string(),
        };
        let q = item
            .get("q")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| question_error("q（英語の問い）がありません"))?;
        let expect_value = item
            .get("expect")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| question_error("expect（正解の論文の配列）がありません"))?;
        let mut expect = Vec::with_capacity(expect_value.len());
        for entry in expect_value {
            let paper = entry
                .get("paper")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| question_error("expect の中の paper（論文の番号）がありません"))?;
            let section = entry
                .get("section")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            expect.push(Expect { paper, section });
        }
        questions.push(EvalQuestion {
            q: q.to_string(),
            expect,
        });
    }
    Ok(questions)
}

/// 上位から見て、正解に最初に当たった順位を求める。
///
/// 一致の条件:
/// - 論文の番号が同じこと。
/// - `section` の指定があり、結果にも章があるときは、結果の章が `section` を含むこと
///   （大文字小文字を区別しない）。指定が無い・結果に章が無いときは論文だけで判定する。
///
/// 同じ論文が何度も出ても、順位は論文ごとに最初に出た位置で数える（重複で順位が
/// 水増しされない）。章の指定は、その論文のどの結果の章で一致してもよい。
#[must_use]
pub fn score(ranked_papers: &[(i64, Option<String>)], expect: &[Expect]) -> QuestionScore {
    // 論文ごとの順位（重複を除いて最初に出た位置）。
    let mut rank_of: HashMap<i64, usize> = HashMap::new();
    for (paper_id, section) in ranked_papers {
        let next = rank_of.len() + 1;
        let rank = *rank_of.entry(*paper_id).or_insert(next);
        let matched = expect
            .iter()
            .any(|e| matches_expect(e, *paper_id, section.as_deref()));
        if matched {
            return QuestionScore {
                first_hit_rank: Some(rank),
            };
        }
    }
    QuestionScore {
        first_hit_rank: None,
    }
}

/// 検索結果から `(論文, 章)` の並びを作る。同じ論文も全部残す（順位の重複は `score` が除く）。
fn ranked_papers(hits: &[Hit]) -> Vec<(i64, Option<String>)> {
    hits.iter()
        .filter_map(|hit| Some((hit.paper_id?, hit.section.clone())))
        .collect()
}

/// 1 件の `expect` が (論文, 章) に一致するか。
fn matches_expect(expect: &Expect, paper_id: i64, section: Option<&str>) -> bool {
    if expect.paper != paper_id {
        return false;
    }
    match (&expect.section, section) {
        (Some(want), Some(got)) => got.to_lowercase().contains(&want.to_lowercase()),
        // 章の指定が無い、または結果に章が無いときは論文だけで判定する。
        _ => true,
    }
}

/// 問いごとの成績を索引 1 つ分の成績に集計する。
#[must_use]
pub fn summarize(index: &str, mode: &str, scores: &[QuestionScore]) -> EvalSummary {
    let n = scores.len();
    let within = |k: usize| {
        scores
            .iter()
            .filter(|s| matches!(s.first_hit_rank, Some(rank) if rank <= k))
            .count()
    };
    let ratio = |hits: usize| {
        if n == 0 { 0.0 } else { hits as f64 / n as f64 }
    };
    let mrr = if n == 0 {
        0.0
    } else {
        scores
            .iter()
            .map(|s| s.first_hit_rank.map_or(0.0, |rank| 1.0 / rank as f64))
            .sum::<f64>()
            / n as f64
    };
    EvalSummary {
        index: index.to_string(),
        mode: mode.to_string(),
        n,
        hit_at_1: ratio(within(1)),
        hit_at_5: ratio(within(5)),
        hit_at_10: ratio(within(10)),
        mrr,
    }
}

/// 質問集の各問いで検索を実行し、索引 1 つ分の成績を出す。
///
/// 検索は [`find`] を `scope: "all"` で呼ぶ（索引の名前とモードは引数で決める）。
/// 同じ論文の結果が何度も出ても、順位は論文ごとに最初の 1 回で数える。
///
/// # Errors
///
/// 索引が無い・モードが不明・埋め込みの失敗・DB エラー（[`find`] と同じ）。
pub async fn run(
    db: &Connection,
    config: &crate::Config,
    questions: &[EvalQuestion],
    index: &str,
    mode: &str,
    top_k: i64,
) -> Result<(EvalSummary, Vec<QuestionScore>)> {
    let mut scores = Vec::with_capacity(questions.len());
    for question in questions {
        let hits = find(
            db,
            config,
            FindRequest {
                query: question.q.clone(),
                mode: Some(mode.to_string()),
                top_k: Some(top_k),
                scope: Some("all".to_string()),
                index: Some(index.to_string()),
                ..FindRequest::default()
            },
        )
        .await?;
        // 論文ごとに最初の 1 回だけを順位の対象にする。
        let ranked = ranked_papers(&hits);
        scores.push(score(&ranked, &question.expect));
    }
    Ok((summarize(index, mode, &scores), scores))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect(paper: i64) -> Vec<Expect> {
        vec![Expect {
            paper,
            section: None,
        }]
    }

    fn hit(paper_id: i64, section: Option<&str>) -> Hit {
        Hit {
            score: 0.5,
            chunk_id: paper_id,
            source_kind: "body".to_string(),
            source_ref: format!("extract_{paper_id}"),
            paper_id: Some(paper_id),
            title: None,
            authors: None,
            year: None,
            text: "text".to_string(),
            char_start: None,
            char_end: None,
            md_path: None,
            section: section.map(str::to_string),
        }
    }

    #[test]
    fn section_matches_any_hit_of_paper_at_first_rank() {
        // 論文 7 の最初の結果は別の章でも、後の結果の章が合えば一致。順位は最初に出た位置。
        let ranked = ranked_papers(&[
            hit(7, Some("2 Methods")),
            hit(3, None),
            hit(7, Some("3 Results")),
        ]);
        let want = vec![Expect {
            paper: 7,
            section: Some("results".to_string()),
        }];
        assert_eq!(score(&ranked, &want).first_hit_rank, Some(1));
        // 章がどれも合わなければ圏外。
        let miss = vec![Expect {
            paper: 7,
            section: Some("discussion".to_string()),
        }];
        assert_eq!(score(&ranked, &miss).first_hit_rank, None);
    }

    #[test]
    fn score_finds_rank_of_first_hit() {
        // 1 位に正解がある。
        assert_eq!(
            score(&[(1, None), (2, None), (3, None)], &expect(1)).first_hit_rank,
            Some(1)
        );
        // 3 位に正解がある。
        assert_eq!(
            score(&[(1, None), (2, None), (3, None)], &expect(3)).first_hit_rank,
            Some(3)
        );
    }

    #[test]
    fn score_returns_none_when_out_of_range() {
        assert_eq!(
            score(&[(1, None), (2, None)], &expect(9)).first_hit_rank,
            None
        );
        // expect が空でも圏外。
        assert_eq!(score(&[(1, None)], &[]).first_hit_rank, None);
    }

    #[test]
    fn score_counts_duplicate_papers_once() {
        // 同じ論文が 1・2 位に出ても、順位は最初の 1 回だけ数える。
        assert_eq!(
            score(&[(7, None), (7, None), (3, None)], &expect(7)).first_hit_rank,
            Some(1)
        );
        // 重複は後ろの論文の順位を水増ししない（3 は重複を除いて 2 位）。
        assert_eq!(
            score(&[(7, None), (7, None), (3, None)], &expect(3)).first_hit_rank,
            Some(2)
        );
    }

    #[test]
    fn score_uses_section_when_both_have_one() {
        let expect_with_section = vec![Expect {
            paper: 12,
            section: Some("Evaluation".to_string()),
        }];
        // 章が含まれれば一致（大文字小文字を区別しない）。
        assert_eq!(
            score(
                &[(12, Some("3. Evaluation and Results".to_string()))],
                &expect_with_section
            )
            .first_hit_rank,
            Some(1)
        );
        // 章が違えば一致しない。
        assert_eq!(
            score(
                &[(12, Some("Introduction".to_string()))],
                &expect_with_section
            )
            .first_hit_rank,
            None
        );
    }

    #[test]
    fn score_falls_back_to_paper_when_section_is_missing() {
        let expect_with_section = vec![Expect {
            paper: 12,
            section: Some("Evaluation".to_string()),
        }];
        // 結果に章が無いときは論文だけで判定する。
        assert_eq!(
            score(&[(12, None)], &expect_with_section).first_hit_rank,
            Some(1)
        );
    }

    #[test]
    fn score_matches_any_of_multiple_expects() {
        let expects = vec![
            Expect {
                paper: 37,
                section: None,
            },
            Expect {
                paper: 12,
                section: None,
            },
        ];
        // 2 番目の expect（paper 12）が 3 位で最初に一致する。
        assert_eq!(
            score(&[(1, None), (2, None), (12, None)], &expects).first_hit_rank,
            Some(3)
        );
    }

    #[test]
    fn summarize_computes_hit_rates_and_mrr() {
        let scores = vec![
            QuestionScore {
                first_hit_rank: Some(1),
            },
            QuestionScore {
                first_hit_rank: Some(3),
            },
            QuestionScore {
                first_hit_rank: None,
            },
        ];
        let summary = summarize("default", "hybrid", &scores);
        assert_eq!(summary.n, 3);
        assert!((summary.hit_at_1 - 1.0 / 3.0).abs() < 1e-9);
        assert!((summary.hit_at_5 - 2.0 / 3.0).abs() < 1e-9);
        assert!((summary.hit_at_10 - 2.0 / 3.0).abs() < 1e-9);
        // (1 + 1/3 + 0) / 3 = 4/9
        assert!((summary.mrr - 4.0 / 9.0).abs() < 1e-9);
    }

    #[test]
    fn summarize_handles_empty_question_set() {
        let summary = summarize("default", "word", &[]);
        assert_eq!(summary.n, 0);
        assert_eq!(summary.hit_at_1, 0.0);
        assert_eq!(summary.mrr, 0.0);
    }

    #[test]
    fn load_questions_reports_missing_file() {
        let path = std::env::temp_dir().join("sai-paper-rag-eval-missing.json");
        let _ = std::fs::remove_file(&path);
        let error = load_questions(&path).unwrap_err();
        assert!(matches!(error, EvalError::NotFound { .. }));
    }

    #[test]
    fn load_questions_reports_broken_json() {
        let path = std::env::temp_dir().join("sai-paper-rag-eval-broken.json");
        std::fs::write(&path, "{ not json").unwrap();
        let error = load_questions(&path).unwrap_err();
        assert!(matches!(error, EvalError::Parse { .. }));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_questions_reports_bad_question_shape() {
        let path = std::env::temp_dir().join("sai-paper-rag-eval-bad-shape.json");
        std::fs::write(&path, r#"[{ "expect": [ { "paper": 1 } ] }]"#).unwrap();
        let error = load_questions(&path).unwrap_err();
        match error {
            EvalError::Question { index, .. } => assert_eq!(index, 0),
            other => panic!("unexpected error: {other}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_questions_reads_sample_shape() {
        let path = std::env::temp_dir().join("sai-paper-rag-eval-sample.json");
        std::fs::write(&path, SAMPLE_QUESTIONS_JSON).unwrap();
        let questions = load_questions(&path).unwrap();
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].expect.len(), 2);
        assert_eq!(questions[0].expect[1].paper, 12);
        assert_eq!(
            questions[0].expect[1].section.as_deref(),
            Some("Evaluation")
        );
        assert_eq!(questions[1].expect[0].paper, 5);
        let _ = std::fs::remove_file(&path);
    }
}
