//! research-tree の新しい形。Node は A（答え・軸の A を含む）・F（根拠）・E（実験）・S（SPROUT）・D（seed）の 5 種。
//!
//! 正は `papr.db` の `rt_*` の表。古い `sci_*` の表は読まないが消さない。
//! Node の ID は表では通し番号の数で持ち、外に出す形は `A12`（頭文字は kind と同じ）。`Q12` は `A12` の別名として読む。
//! SPROUT は `S2`。
//!
//! 木の根（SEED）は廃止した。軸の A（`parent_id` が NULL の A。軸の A）は親の無い A として DB に並ぶ。研究ごとの制約は D（seed）の行で、ID は `D<n>`。

// 行の型の項目名は表の列名そのままなので、項目ごとの説明は付けない。
#![allow(missing_docs)]

pub mod action;
pub mod axes;
pub mod card;
pub mod compare;
pub mod compute;
pub mod context;
pub mod defs;
pub mod export;
pub mod ops;
pub mod scope;
pub mod seedq;
pub mod steps;
pub mod store;

/// research-tree の操作のエラー。`Invalid` と `NotFound` の文は利用者にそのまま見せる日本語。
#[derive(Debug, thiserror::Error)]
pub enum RtError {
    /// 入力が通らない。
    #[error("{0}")]
    Invalid(String),
    /// 対象が見つからない。
    #[error("{0}")]
    NotFound(String),
    /// 状態の衝突（許可が無い・同じ質問の承認が既にある・run_id が無い）。
    #[error("{0}")]
    Conflict(String),
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

/// research-tree の操作の結果型。
pub type Result<T> = std::result::Result<T, RtError>;

/// 廃止した根の ID（`SEED`）。agent・利用者の入力で断り、古い task.md の `sci_node: SEED` は木の全体として読む。
const RETIRED_SEED: &str = "SEED";

/// `SEED` を断るときの案内（終了コード 2）。
pub const RETIRED_SEED_MSG: &str = "SEED は廃止しました。木の全体は sai-sci status、研究の制約は seed（D）の ID（例: sai-sci node D3）を使ってください";

/// 廃止した `SEED` か。
#[must_use]
pub fn is_retired_seed(text: &str) -> bool {
    text == RETIRED_SEED
}

/// Node の 5 種（Q は廃止。軸は親の無い A）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum Kind {
    A,
    F,
    E,
    S,
    D,
}

impl Kind {
    /// "A" / "F" / "E" / "S" / "D"。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::A => "A",
            Kind::F => "F",
            Kind::E => "E",
            Kind::S => "S",
            Kind::D => "D",
        }
    }

    /// DB の kind 列から。
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Kind> {
        match s {
            "A" => Some(Kind::A),
            "F" => Some(Kind::F),
            "E" => Some(Kind::E),
            "S" => Some(Kind::S),
            "D" => Some(Kind::D),
            _ => None,
        }
    }

    /// 入力の頭文字。`Q` は `A` の別名（ID の数字は DB の行番号なので同じ行を指す）。
    fn from_char(c: char) -> Option<Kind> {
        match c {
            'Q' | 'A' => Some(Kind::A),
            'F' => Some(Kind::F),
            'E' => Some(Kind::E),
            'S' => Some(Kind::S),
            'D' => Some(Kind::D),
            _ => None,
        }
    }
}

/// "A12" → (Kind::A, 12)。"Q12" も同じく (Kind::A, 12)。形は `^[QAFESD][1-9][0-9]{0,8}$`。違えば Invalid。
pub fn parse_id(text: &str) -> Result<(Kind, i64)> {
    let invalid = || RtError::Invalid(format!("ID の形が正しくありません: {text}"));
    let mut chars = text.chars();
    let kind = chars.next().and_then(Kind::from_char).ok_or_else(invalid)?;
    let digits = chars.as_str();
    if digits.is_empty()
        || digits.len() > 9
        || digits.starts_with('0')
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    let number: i64 = digits.parse().map_err(|_| invalid())?;
    Ok((kind, number))
}

/// 頭文字まで指定して読む。違う頭文字なら Invalid。
pub fn parse_id_of(text: &str, kind: Kind) -> Result<i64> {
    let (found, id) = parse_id(text)?;
    if found != kind {
        return Err(RtError::Invalid(format!(
            "{text} は {} の ID ではありません",
            kind.as_str()
        )));
    }
    Ok(id)
}

/// (Kind::A, 12) → "A12"。
#[must_use]
pub fn fmt_id(kind: Kind, id: i64) -> String {
    format!("{}{id}", kind.as_str())
}

/// "S2" と 'S' → 2。形は `^<prefix>[1-9][0-9]{0,8}$`。
pub fn parse_ref(text: &str, prefix: char) -> Result<i64> {
    let mut buf = [0u8; 4];
    parse_prefixed(text, prefix.encode_utf8(&mut buf))
}

/// ('S', 2) → "S2"。
#[must_use]
pub fn fmt_ref(prefix: char, id: i64) -> String {
    format!("{prefix}{id}")
}

/// "SQ3" → 3（seed の問い）。形は `^SQ[1-9][0-9]{0,8}$`。違えば Invalid。
pub fn parse_sq(text: &str) -> Result<i64> {
    parse_prefixed(text, "SQ")
}

/// "SA7" → 7（問いへの回答）。形は `^SA[1-9][0-9]{0,8}$`。違えば Invalid。
pub fn parse_sa(text: &str) -> Result<i64> {
    parse_prefixed(text, "SA")
}

/// ("SQ", 3) → "SQ3"。
#[must_use]
pub fn fmt_sq(id: i64) -> String {
    format!("SQ{id}")
}

/// ("SA", 7) → "SA7"。
#[must_use]
pub fn fmt_sa(id: i64) -> String {
    format!("SA{id}")
}

/// 頭が `prefix` で、続きが 1〜9 桁の数（先頭は 0 でない）の ID を数にする。
fn parse_prefixed(text: &str, prefix: &str) -> Result<i64> {
    let invalid = || RtError::Invalid(format!("ID の形が正しくありません: {text}"));
    let digits = text.strip_prefix(prefix).ok_or_else(invalid)?;
    if digits.is_empty()
        || digits.len() > 9
        || digits.starts_with('0')
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    digits.parse().map_err(|_| invalid())
}

/// CURRENT_TIMESTAMP と同じ形（UTC の "YYYY-MM-DD HH:MM:SS"）の今の時刻。
#[must_use]
pub fn now_text() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// task.md の sci キー（C-KEYS）。無いキーは空。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct RtKeys {
    /// 対象の Node。木の全体のときは `None`。
    pub node: Option<(Kind, i64)>,
    /// 対象が木の全体か（`sci_node` が空か、古い `sci_node: SEED`）。
    pub whole: bool,
    /// 行動の目的。
    pub purpose: String,
    /// 期待する成果。
    pub expect: String,
    /// 読み方（purpose_first / blind_first）。
    pub mode: String,
    /// 対象の論文の番号。
    pub papers: Vec<i64>,
    /// 読む深さ（abstract / figures / full）。
    pub depth: String,
    /// 足す子の型（A / F。無い・知らない値は空）。
    pub child: String,
}

impl RtKeys {
    /// sci の CARD か（`sci_node` か `sci_purpose` のどちらかがある）。
    #[must_use]
    pub fn is_sci(&self) -> bool {
        self.node.is_some() || self.whole || !self.purpose.is_empty()
    }
}

/// task.md の全文から sci キーを読む。読めない値は空として扱う。`sci_question`・`sci_role` は読まない。
#[must_use]
pub fn task_keys(task_md: &str) -> RtKeys {
    let pairs = crate::runs::read_frontmatter(task_md);
    let get = |name: &str| -> String {
        pairs
            .iter()
            .find(|(key, _)| key == name)
            .map_or_else(String::new, |(_, value)| value.trim().to_string())
    };

    let papers = get("sci_papers")
        .split(',')
        .filter_map(|part| part.trim().parse::<i64>().ok())
        .filter(|id| *id > 0)
        .collect();

    // `sci_node` が空（木の全体）か、古い `sci_node: SEED`（木の全体として読む）。キーが無いときは全体にしない。
    let node_text = get("sci_node");
    let whole = pairs.iter().any(|(key, _)| key == "sci_node") && (node_text.is_empty() || is_retired_seed(&node_text));
    let node = if whole { None } else { parse_id(&node_text).ok() };

    RtKeys {
        node,
        whole,
        purpose: get("sci_purpose"),
        expect: get("sci_expect"),
        mode: get("sci_mode"),
        papers,
        depth: get("sci_depth"),
        child: match get("sci_child").as_str() {
            kind @ ("A" | "F") => kind.to_string(),
            _ => String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_id_accepts_and_rejects() {
        assert_eq!(parse_id("Q12").unwrap(), (Kind::A, 12));
        assert_eq!(parse_id("A12").unwrap(), (Kind::A, 12));
        assert_eq!(parse_id("A15").unwrap(), (Kind::A, 15));
        assert_eq!(parse_id("F999999999").unwrap(), (Kind::F, 999_999_999));
        assert_eq!(parse_id("F5").unwrap(), (Kind::F, 5));
        assert_eq!(parse_id("E3").unwrap(), (Kind::E, 3));
        assert_eq!(parse_id("S31").unwrap(), (Kind::S, 31));
        assert_eq!(parse_id("D1").unwrap(), (Kind::D, 1));
        for text in ["Q0", "Q01", "N12", "12", "Q", "", "Q1000000000", "Q1a", "O1", "D0", "D", "SEED"] {
            assert!(matches!(parse_id(text), Err(RtError::Invalid(_))), "{text}");
        }
        assert!(matches!(parse_id_of("F5", Kind::A), Err(RtError::Invalid(_))));
        assert_eq!(parse_id_of("Q5", Kind::A).unwrap(), 5);
        assert_eq!(parse_id_of("A5", Kind::A).unwrap(), 5);
        assert_eq!(fmt_id(Kind::A, 13), "A13");
    }

    #[test]
    fn q_is_only_an_alias_of_a() {
        // Q は Node の種類ではない。DB の kind にも無く、入力の頭文字だけ A の別名になる。
        assert_eq!(Kind::from_str("Q"), None);
        assert_eq!(Kind::from_str("A"), Some(Kind::A));
        assert_eq!(parse_id_of("Q7", Kind::A).unwrap(), 7);
        assert_eq!(fmt_id(Kind::A, 7), "A7");
    }

    #[test]
    fn parse_ref_checks_prefix() {
        assert_eq!(parse_ref("S3", 'S').unwrap(), 3);
        assert_eq!(parse_sq("SQ3").unwrap(), 3);
        assert_eq!(parse_sa("SA70").unwrap(), 70);
        assert!(parse_sq("SQ03").is_err() && parse_sa("SA").is_err() && parse_sq("Q3").is_err());
        assert_eq!(fmt_sq(3), "SQ3");
        assert!(parse_ref("Q3", 'S').is_err());
        assert!(parse_ref("S03", 'S').is_err());
        assert_eq!(fmt_ref('S', 2), "S2");
    }

    #[test]
    fn task_keys_reads_all_and_ignores_old_keys() {
        // 古い task.md の範囲と仮置きのキーは読まずに捨てる（名前は文字列を分けて組み立てる）。
        let old_scope = format!("sci_{}", "scope");
        let old_assume = format!("sci_{}", "assume");
        let md = format!("---\nsci_node: A12\nsci_question: Q7\nsci_role: x\nsci_purpose: データの量の根拠を探す\nsci_expect: F を 2 つ以上\n{old_scope}: path\n{old_assume}: Q9=SST-2 を使う（仮）; X1=読めない\nsci_mode: purpose_first\nsci_papers: 37, 40, abc\nsci_depth: full\nsci_child: F\n---\n\n本文\n");
        let keys = task_keys(&md);
        assert_eq!(keys.node, Some((Kind::A, 12)));
        assert!(!keys.whole);
        assert_eq!(keys.purpose, "データの量の根拠を探す");
        assert_eq!(keys.expect, "F を 2 つ以上");
        assert_eq!(keys.mode, "purpose_first");
        assert_eq!(keys.papers, vec![37, 40]);
        assert_eq!(keys.depth, "full");
        assert_eq!(keys.child, "F");
        // 知らない値は空として読む（C7: A・F 以外は無い扱い）。
        assert_eq!(task_keys("---\nsci_node: A1\nsci_child: O\n---\n").child, "");
        assert_eq!(task_keys("---\nsci_node: A1\nsci_child: Q\n---\n").child, "");
        assert!(keys.is_sci());
        assert_eq!(task_keys("---\nrunner: pi\n---\n"), RtKeys::default());
        // 古い `sci_node: SEED` と、空の `sci_node` は木の全体（node は空）。
        for md in ["---\nsci_node: SEED\nsci_purpose: 全体を見る\n---\n", "---\nsci_node:\nsci_purpose: 全体を見る\n---\n"] {
            let whole = task_keys(md);
            assert!(whole.whole && whole.node.is_none(), "{md}");
            assert!(whole.is_sci());
        }
    }
}
