//! docling title の単語で S2 候補を逐次絞り込むマッチング。
//!
//! 1. word検索: docling タイトルの語で S2 候補を順に絞り、1件に確定したら採用
//! 2. フォールバック: Dice 2-gram 類似度（閾値 0.78 + 年号一致ボーナス 0.05）
//!
//! `match_refs` は使用済み S2 インデックス集合を返す。呼び出し側はこの集合を
//! 使って fallback 追加（S2 にあって docling にない引用の補完）を行うこと。

use std::collections::HashSet;

use crate::api::s2::S2PaperDetail;

use super::db::RefRow;

/// ストップワード（3文字以上の一般的な語）。
/// ※ `into` は重複登録しないこと。
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "from", "that", "this", "have", "has", "were", "are", "was",
    "being", "using", "based", "between", "among", "within", "through", "during", "under", "over",
    "into", "onto", "about", "after", "before", "while", "where", "which", "their", "there",
    "these", "those", "then", "than", "also", "been", "would", "could", "should", "upon", "via",
    "per", "et", "al", "vs",
];

/// Dice フォールバックの採用閾値。
const DICE_THRESHOLD: f64 = 0.78;
/// 年号一致ボーナス。
const YEAR_BONUS: f64 = 0.05;

/// タイトルを正規化する（小文字化 + 英数字以外を空白に変換 + 空白正規化）。
pub fn normalize_title(s: &str) -> String {
    let lower = s.to_lowercase();
    let normalized: String = lower
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect();
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// タイトルをトークナイズする（ストップワード除去 + 3文字以上）。
pub fn tokenize_title(s: &str) -> Vec<String> {
    let normalized = normalize_title(s);
    let stopset: HashSet<&str> = STOPWORDS.iter().copied().collect();
    normalized
        .split_whitespace()
        .filter(|w| w.len() >= 3 && !stopset.contains(w))
        .map(str::to_string)
        .collect()
}

/// S2 タイトルが指定単語を含むかを判定する（正規化後の部分一致）。
pub fn s2_title_contains_word(s2_title: &str, word: &str) -> bool {
    let normalized = normalize_title(s2_title);
    normalized.contains(word)
}

/// Dice 2-gram 類似度を計算する。
pub fn title_similarity(a: &str, b: &str) -> f64 {
    let bg = |s: &str| -> HashSet<String> {
        let norm = normalize_title(s);
        let chars: Vec<char> = norm.chars().collect();
        if chars.len() < 2 {
            return HashSet::new();
        }
        (0..chars.len() - 1)
            .map(|i| format!("{}{}", chars[i], chars[i + 1]))
            .collect()
    };
    let sa = bg(a);
    let sb = bg(b);
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    let inter = sa.iter().filter(|x| sb.contains(*x)).count();
    (2 * inter) as f64 / (sa.len() + sb.len()) as f64
}

/// docling title の単語で S2 候補を逐次絞り込む。
///
/// # 戻り値
/// 1件に確定した場合は `(S2 refs 内のインデックス, マッチした単語リスト)`。
/// 1件に確定しなかった場合は `None`（誤マッチ防止のため未確定とする）。
pub fn match_by_word_search(
    docling_title: &str,
    s2_refs: &[S2PaperDetail],
    used: &HashSet<usize>,
) -> Option<(usize, Vec<String>)> {
    let words = tokenize_title(docling_title);
    if words.is_empty() {
        return None;
    }

    let mut candidates: Vec<usize> = (0..s2_refs.len())
        .filter(|i| !used.contains(i) && s2_refs[*i].title.is_some())
        .collect();
    let mut matched_words: Vec<String> = Vec::new();

    for w in &words {
        let next: Vec<usize> = candidates
            .iter()
            .filter(|&&i| s2_title_contains_word(s2_refs[i].title.as_deref().unwrap_or(""), w))
            .copied()
            .collect();
        if next.is_empty() {
            continue; // この語では誰もヒットしない → スキップ
        }
        matched_words.push(w.clone());
        candidates = next;
        if candidates.len() == 1 {
            break; // 1件に絞れた
        }
    }

    if candidates.len() == 1 {
        Some((candidates[0], matched_words))
    } else {
        None
    }
}

/// `ref_rows` の S2 マッチングを実行し、使用済み S2 インデックス集合を返す。
///
/// word検索で1件に絞れない場合は Dice 類似度で最良候補を採用する
/// （閾値 `DICE_THRESHOLD` + 年号一致ボーナス `YEAR_BONUS`）。
pub fn match_refs(
    ref_rows: &mut [RefRow],
    s2_refs: &[S2PaperDetail],
    mut used: HashSet<usize>,
) -> HashSet<usize> {
    for ref_row in ref_rows.iter_mut() {
        let Some(title) = ref_row.title.as_ref().filter(|t| t.len() >= 8) else {
            continue;
        };
        let title = title.clone();

        // 1) word検索
        if let Some((idx, _matched_words)) = match_by_word_search(&title, s2_refs, &used) {
            used.insert(idx);
            apply_s2_match(ref_row, &s2_refs[idx]);
            continue;
        }

        // 2) フォールバック: Dice 類似度
        let mut best_score = 0.0_f64;
        let mut best_idx: Option<usize> = None;
        for (i, s2) in s2_refs.iter().enumerate() {
            if used.contains(&i) {
                continue;
            }
            let Some(s2_title) = s2.title.as_deref() else {
                continue;
            };
            let mut score = title_similarity(&title, s2_title);
            if ref_row.year.is_some() && ref_row.year == s2.year.map(i64::from) {
                score += YEAR_BONUS;
            }
            if score > best_score {
                best_score = score;
                best_idx = Some(i);
            }
        }
        if let Some(idx) = best_idx
            && best_score >= DICE_THRESHOLD
        {
            used.insert(idx);
            apply_s2_match(ref_row, &s2_refs[idx]);
        }
    }
    used
}

/// S2 マッチ結果を `RefRow` に適用する。
fn apply_s2_match(ref_row: &mut RefRow, s2: &S2PaperDetail) {
    ref_row.s2_paper_id = s2.paper_id.clone();
    ref_row.s2_title = s2.title.clone();
    ref_row.s2_authors = s2
        .authors
        .as_ref()
        .map(|a| serde_json::to_string(a).unwrap_or_default());
    ref_row.s2_year = s2.year.map(i64::from);
    ref_row.s2_venue = s2.venue.clone();
    ref_row.s2_journal = s2
        .journal
        .as_ref()
        .map(|j| serde_json::to_string(j).unwrap_or_default());
    ref_row.s2_external_ids = s2
        .external_ids
        .as_ref()
        .map(|m| serde_json::to_string(m).unwrap_or_default());
    ref_row.s2_citation_count = s2.citation_count.map(i64::from);
    ref_row.s2_url = s2
        .url
        .clone()
        .or_else(|| s2.open_access_pdf.as_ref().and_then(|p| p.url.clone()));
}

/// docling に対応しなかった S2 refs のインデックス集合を求める（fallback 追加用）。
pub fn unmatched_indices(total: usize, used: &HashSet<usize>) -> Vec<usize> {
    (0..total).filter(|i| !used.contains(i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(paper_id: &str, title: &str) -> S2PaperDetail {
        S2PaperDetail {
            paper_id: Some(paper_id.to_string()),
            title: Some(title.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn normalize_title_removes_special_chars() {
        assert_eq!(normalize_title("Hello, World!"), "hello world");
    }

    #[test]
    fn tokenize_title_removes_stopwords() {
        let tokens = tokenize_title("A Survey of Deep Learning for Natural Language Processing");
        assert!(tokens.contains(&"survey".to_string()));
        assert!(tokens.contains(&"deep".to_string()));
        assert!(tokens.contains(&"learning".to_string()));
        assert!(!tokens.contains(&"for".to_string()));
        assert!(!tokens.contains(&"of".to_string()));
    }

    #[test]
    fn title_similarity_identical() {
        assert!(title_similarity("Hello World", "Hello World") > 0.99);
    }

    #[test]
    fn title_similarity_different() {
        assert!(title_similarity("Hello World", "Goodbye Universe") < 0.3);
    }

    #[test]
    fn match_by_word_search_finds_match() {
        let s2_refs = vec![detail(
            "abc123",
            "Deep Learning for Natural Language Processing",
        )];
        let used = HashSet::new();
        let result = match_by_word_search("A Survey of Deep Learning for NLP", &s2_refs, &used);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, 0);
    }

    #[test]
    fn match_refs_marks_used_and_avoids_duplicates() {
        let s2_refs = vec![
            detail("s2-a", "Attention Is All You Need"),
            detail(
                "s2-b",
                "BERT Pretraining of Deep Bidirectional Transformers",
            ),
        ];
        let mut rows = vec![
            RefRow {
                paper_id: "root".into(),
                ref_no: 1,
                title: Some("Attention Is All You Need".into()),
                raw_text: String::new(),
                ..Default::default()
            },
            RefRow {
                paper_id: "root".into(),
                ref_no: 2,
                title: Some("BERT: Pre-training of Deep Bidirectional Transformers".into()),
                raw_text: String::new(),
                ..Default::default()
            },
        ];
        let used = match_refs(&mut rows, &s2_refs, HashSet::new());
        assert_eq!(used.len(), 2);
        assert_eq!(rows[0].s2_paper_id.as_deref(), Some("s2-a"));
        assert_eq!(rows[1].s2_paper_id.as_deref(), Some("s2-b"));
        // 返却された集合から未マッチを求めると空（二重登録の元にならない）
        assert!(unmatched_indices(s2_refs.len(), &used).is_empty());
    }

    #[test]
    fn dice_fallback_applies_with_threshold() {
        let s2_refs = vec![detail("s2-x", "Language Models are Few-Shot Learners")];
        let mut rows = vec![RefRow {
            paper_id: "root".into(),
            ref_no: 1,
            title: Some("Language Models are Few-Shot Learners".into()),
            raw_text: String::new(),
            ..Default::default()
        }];
        let used = match_refs(&mut rows, &s2_refs, HashSet::new());
        assert_eq!(used.len(), 1);
        assert_eq!(rows[0].s2_paper_id.as_deref(), Some("s2-x"));
    }
}
