//! 研究方針の木（Research-tree）の新しい形。Node は Q（問い）・O（答えの候補）・F（根拠）・E（実験）の 4 種だけ。
//!
//! 正は `papr.db` の `rt_*` の表。古い `sci_*` の表と [`crate::sci`] は別の作業で外すまで残す。
//! Node の ID は表では通し番号の数で持ち、外に出す形は `Q12`（頭文字は kind と同じ）。
//! 提案は `P3`、種は `S2`。

// 行の型の項目名は表の列名そのままなので、項目ごとの説明は付けない。
#![allow(missing_docs)]

pub mod action;
pub mod defs;
pub mod ops;
pub mod store;

/// 研究方針の木の操作のエラー。`Invalid` と `NotFound` の文は利用者にそのまま見せる日本語。
#[derive(Debug, thiserror::Error)]
pub enum RtError {
    /// 入力が通らない。
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

/// 研究方針の木の操作の結果型。
pub type Result<T> = std::result::Result<T, RtError>;

/// Node の 4 種。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum Kind {
    Q,
    O,
    F,
    E,
}

impl Kind {
    /// "Q" / "O" / "F" / "E"。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Q => "Q",
            Kind::O => "O",
            Kind::F => "F",
            Kind::E => "E",
        }
    }

    /// DB の kind 列から。
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Kind> {
        match s {
            "Q" => Some(Kind::Q),
            "O" => Some(Kind::O),
            "F" => Some(Kind::F),
            "E" => Some(Kind::E),
            _ => None,
        }
    }

    fn from_char(c: char) -> Option<Kind> {
        match c {
            'Q' => Some(Kind::Q),
            'O' => Some(Kind::O),
            'F' => Some(Kind::F),
            'E' => Some(Kind::E),
            _ => None,
        }
    }
}

/// "Q12" → (Kind::Q, 12)。形は `^[QOFE][1-9][0-9]{0,8}$`。違えば Invalid。
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

/// (Kind::Q, 12) → "Q12"。
#[must_use]
pub fn fmt_id(kind: Kind, id: i64) -> String {
    format!("{}{id}", kind.as_str())
}

/// "P3" / "S2" と 'P' → 3。形は `^<prefix>[1-9][0-9]{0,8}$`。
pub fn parse_ref(text: &str, prefix: char) -> Result<i64> {
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

/// ('P', 3) → "P3"。
#[must_use]
pub fn fmt_ref(prefix: char, id: i64) -> String {
    format!("{prefix}{id}")
}

/// CURRENT_TIMESTAMP と同じ形（UTC の "YYYY-MM-DD HH:MM:SS"）の今の時刻。
#[must_use]
pub fn now_text() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// task.md の sci キー（C-KEYS）。無いキーは空。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct RtKeys {
    /// 対象の Node。
    pub node: Option<(Kind, i64)>,
    /// 行動の目的。
    pub purpose: String,
    /// 期待する成果。
    pub expect: String,
    /// 範囲（node / path / subtree / siblings / summary / blind）。
    pub scope: String,
    /// 仮置き（`Q9=SST-2 を使う（仮）` のように読む）。
    pub assume: Vec<((Kind, i64), String)>,
    /// 読み方（purpose_first / blind_first）。
    pub mode: String,
    /// 対象の論文の番号。
    pub papers: Vec<i64>,
    /// 読む深さ（abstract / figures / full）。
    pub depth: String,
}

impl RtKeys {
    /// sci のカードか（`sci_node` か `sci_purpose` のどちらかがある）。
    #[must_use]
    pub fn is_sci(&self) -> bool {
        self.node.is_some() || !self.purpose.is_empty()
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

    let assume = get("sci_assume")
        .split(';')
        .filter_map(|part| {
            let (left, right) = part.trim().split_once('=')?;
            Some((parse_id(left.trim()).ok()?, right.trim().to_string()))
        })
        .collect();

    let papers = get("sci_papers")
        .split(',')
        .filter_map(|part| part.trim().parse::<i64>().ok())
        .filter(|id| *id > 0)
        .collect();

    RtKeys {
        node: parse_id(&get("sci_node")).ok(),
        purpose: get("sci_purpose"),
        expect: get("sci_expect"),
        scope: get("sci_scope"),
        assume,
        mode: get("sci_mode"),
        papers,
        depth: get("sci_depth"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_id_accepts_and_rejects() {
        assert_eq!(parse_id("Q12").unwrap(), (Kind::Q, 12));
        assert_eq!(parse_id("O1").unwrap(), (Kind::O, 1));
        assert_eq!(parse_id("F999999999").unwrap(), (Kind::F, 999_999_999));
        assert_eq!(parse_id("E3").unwrap(), (Kind::E, 3));
        for text in ["Q0", "Q01", "N12", "12", "Q", "", "Q1000000000", "Q1a"] {
            assert!(matches!(parse_id(text), Err(RtError::Invalid(_))), "{text}");
        }
        assert!(matches!(parse_id_of("O5", Kind::Q), Err(RtError::Invalid(_))));
        assert_eq!(parse_id_of("Q5", Kind::Q).unwrap(), 5);
        assert_eq!(fmt_id(Kind::O, 13), "O13");
    }

    #[test]
    fn parse_ref_checks_prefix() {
        assert_eq!(parse_ref("P3", 'P').unwrap(), 3);
        assert!(parse_ref("S3", 'P').is_err());
        assert!(parse_ref("P03", 'P').is_err());
        assert_eq!(fmt_ref('S', 2), "S2");
    }

    #[test]
    fn task_keys_reads_all_and_ignores_old_keys() {
        let md = "---\nsci_node: Q12\nsci_question: Q7\nsci_role: x\nsci_purpose: データの量の根拠を探す\nsci_expect: F を 2 つ以上\nsci_scope: path\nsci_assume: Q9=SST-2 を使う（仮）; X1=読めない\nsci_mode: purpose_first\nsci_papers: 37, 40, abc\nsci_depth: full\n---\n\n本文\n";
        let keys = task_keys(md);
        assert_eq!(keys.node, Some((Kind::Q, 12)));
        assert_eq!(keys.purpose, "データの量の根拠を探す");
        assert_eq!(keys.expect, "F を 2 つ以上");
        assert_eq!(keys.scope, "path");
        assert_eq!(keys.assume, vec![((Kind::Q, 9), "SST-2 を使う（仮）".to_string())]);
        assert_eq!(keys.mode, "purpose_first");
        assert_eq!(keys.papers, vec![37, 40]);
        assert_eq!(keys.depth, "full");
        assert!(keys.is_sci());
        assert_eq!(task_keys("---\nrunner: pi\n---\n"), RtKeys::default());
    }
}
