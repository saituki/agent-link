//! 研究方針の木（Research-tree, sai-sci）の土台。
//!
//! ikawa の研究を「木」として表し、項目・候補・問い・知見・結び付き・決定の記録を 1 つの DB に置く。
//! 正は `papr.db` の `sci_*` の表で、`state.md` と `tree.md` はそこから書き出したもの。
//!
//! 節点の ID は表では数で持つが、外に出すときの形は `N12`（`N` 節点 / `Q` 問い / `F` 知見 / `L` 結び付き / `P` 提案 / `S` 種）。

pub mod card;
pub mod compute;
pub mod context;
pub mod defs;
pub mod export;
pub mod ops;
pub mod store;

/// sai-sci の操作のエラー。`Invalid` と `NotFound` の文は利用者にそのまま見せる日本語にする。
#[derive(Debug, thiserror::Error)]
pub enum SciError {
    /// 入力が通らない。利用者にはこの文をそのまま見せる。
    #[error("{0}")]
    Invalid(String),
    /// 対象が見つからない。
    #[error("{0}")]
    NotFound(String),
    /// SQLite の操作に失敗した。
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    /// ファイルの読み書きに失敗した。
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON の変換に失敗した。
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// sai-sci の操作の結果型。
pub type Result<T> = std::result::Result<T, SciError>;

/// "N12" と 'N' → 12。形が違えば Invalid("ID の形が正しくありません: <text>")。
///
/// 数の頭は 0 を認めない（`N01` は不可）。頭文字も prefix と一致すること。
pub fn parse_ref(text: &str, prefix: char) -> Result<i64> {
    let invalid = || SciError::Invalid(format!("ID の形が正しくありません: {text}"));
    let mut chars = text.chars();
    let Some(head) = chars.next() else {
        return Err(invalid());
    };
    let digits: String = chars.collect();
    // 形は `^[NQFLPS][1-9][0-9]{0,8}$`（頭は 0 を認めない）。
    if head != prefix
        || digits.is_empty()
        || !digits.chars().all(|c| c.is_ascii_digit())
        || !matches!(digits.chars().next(), Some('1'..='9'))
    {
        return Err(invalid());
    }
    let number: i64 = digits.parse().map_err(|_| invalid())?;
    if !(1..=999_999_999).contains(&number) {
        return Err(invalid());
    }
    Ok(number)
}

/// ('N', 12) → "N12"。
#[must_use]
pub fn fmt_ref(prefix: char, id: i64) -> String {
    format!("{prefix}{id}")
}

/// CURRENT_TIMESTAMP と同じ形（UTC の "YYYY-MM-DD HH:MM:SS"）の今の時刻。
#[must_use]
pub fn now_text() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// task.md の sci キー（C-05）。無いキーは空（`None` / 空文字 / 空の Vec）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct SciKeys {
    /// 対象の節点。
    pub node: Option<i64>,
    /// 答える問い。
    pub question: Option<i64>,
    /// 行動の目的。
    pub purpose: String,
    /// 期待する成果。
    pub expect: String,
    /// 範囲（C-01）。
    pub scope: String,
    /// 仮置き（`N9=発作部位の予測（仮）` のように読む）。
    pub assume: Vec<(i64, String)>,
    /// sci-read の型（C-01）。
    pub mode: String,
    /// 対象の論文の番号。
    pub papers: Vec<i64>,
    /// 読む札の論文の役割（C-01）。
    pub role: String,
    /// 読む札の深さ（C-01）。
    pub depth: String,
}
impl SciKeys {
    /// sci のカードか（`sci_node` か `sci_purpose` のどちらかがある）。
    #[must_use]
    pub fn is_sci(&self) -> bool {
        self.node.is_some() || !self.purpose.is_empty()
    }
}

/// task.md の全文から sci キーを読む（`crate::runs::read_frontmatter` を使う）。形の違う値は空として扱う。
#[must_use]
pub fn task_sci_keys(task_md: &str) -> SciKeys {
    let pairs = crate::runs::read_frontmatter(task_md);
    let get = |name: &str| -> String {
        pairs
            .iter()
            .find(|(key, _)| key == name)
            .map_or_else(String::new, |(_, value)| value.trim().to_string())
    };

    let node = parse_ref(&get("sci_node"), 'N').ok();
    let question = parse_ref(&get("sci_question"), 'Q').ok();

    // 仮置きは "N<id>=<値>" を "; " でつなぐ。読めない形は捨てる。
    let assume: Vec<(i64, String)> = get("sci_assume")
        .split(';')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            let (left, right) = part.split_once('=')?;
            let id = parse_ref(left.trim(), 'N').ok()?;
            Some((id, right.trim().to_string()))
        })
        .collect();

    // 論文番号は ", " でつなぐ。正の整数だけを集める。
    let papers: Vec<i64> = get("sci_papers")
        .split(',')
        .filter_map(|part| part.trim().parse::<i64>().ok())
        .filter(|id| *id > 0)
        .collect();

    SciKeys {
        node,
        question,
        purpose: get("sci_purpose"),
        expect: get("sci_expect"),
        scope: get("sci_scope"),
        assume,
        mode: get("sci_mode"),
        papers,
        role: get("sci_role"),
        depth: get("sci_depth"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ref_accepts_only_prefixed_ids() {
        assert_eq!(parse_ref("N12", 'N').unwrap(), 12);
        assert_eq!(parse_ref("Q1", 'Q').unwrap(), 1);
        assert_eq!(parse_ref("P123456789", 'P').unwrap(), 123_456_789);

        for (text, prefix) in [
            ("12", 'N'),
            ("Q3", 'N'),
            ("N0", 'N'),
            ("N", 'N'),
            ("N01", 'N'),
            ("", 'N'),
        ] {
            let err = parse_ref(text, prefix).unwrap_err();
            let SciError::Invalid(message) = err else {
                panic!("Invalid ではない: {text}");
            };
            assert!(message.contains("ID の形が正しくありません"), "{message}");
        }
    }

    #[test]
    fn fmt_ref_joins_prefix_and_number() {
        assert_eq!(fmt_ref('N', 12), "N12");
        assert_eq!(fmt_ref('F', 31), "F31");
    }

    #[test]
    fn task_sci_keys_reads_all_keys() {
        let task_md = "---\nrunner: pi\nrun_id: 120\nblock: sci-compare\nsci_node: N12\nsci_question: Q7\nsci_purpose: 小目的の候補を比べる\nsci_assume: N9=発作部位の予測（仮）; X1=読めない\nsci_papers: 37, 40, abc\n---\n\n# task_text\n\n本文\n";
        let keys = task_sci_keys(task_md);
        assert_eq!(keys.node, Some(12));
        assert_eq!(keys.question, Some(7));
        assert_eq!(keys.purpose, "小目的の候補を比べる");
        assert_eq!(keys.assume, vec![(9, "発作部位の予測（仮）".to_string())]);
        assert_eq!(keys.papers, vec![37, 40]);
        assert!(keys.is_sci());
        // 無いキーは空のまま。
        assert_eq!(keys.expect, "");
        assert_eq!(keys.scope, "");
        assert_eq!(keys.mode, "");
        assert_eq!(keys.role, "");
        assert_eq!(keys.depth, "");
    }

    #[test]
    fn task_sci_keys_is_empty_for_plain_task() {
        let keys = task_sci_keys("---\nrun_id: 3\nblock: paper-search\n---\n\n本文\n");
        assert!(!keys.is_sci());
        assert_eq!(keys.node, None);
        assert!(keys.assume.is_empty());
        assert!(keys.papers.is_empty());
    }
}
