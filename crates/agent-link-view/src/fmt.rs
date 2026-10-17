//! 数値整形ヘルパー（Project ダッシュボード等で使用）。

/// 費用（USD）を表示用に整形する。
///
/// - `Some(x)` かつ `x >= 0.01` → `$0.42`（小数 2 桁）
/// - `Some(x)` かつ `x < 0.01` → `<$0.01`（0 と見せて「安い」と誤解させない）
/// - `None`（provider が費用を返していない）→ `cost unknown`
pub fn format_cost(cost: Option<f64>) -> String {
    match cost {
        Some(x) if x < 0.01 => "<$0.01".to_string(),
        Some(x) => format!("${:.2}", x),
        None => "cost unknown".to_string(),
    }
}

/// トークン数を短い表現に整形する（例: `120000` → `120k`、`12300` → `12.3k`、`1200000` → `1.2M`）。
///
/// 小数 1 桁まで表示し、`.0` は省く（`12.3k` / `120k` / `1.2M`）。
/// 1000 未満はそのままの数値。
pub fn format_tokens(n: u64) -> String {
    let scaled = if n >= 1_000_000 {
        Some((n as f64 / 1_000_000.0, "M"))
    } else if n >= 1_000 {
        Some((n as f64 / 1_000.0, "k"))
    } else {
        None
    };
    match scaled {
        Some((v, unit)) => {
            let s = format!("{:.1}", v);
            let s = s.strip_suffix(".0").unwrap_or(&s).to_string();
            format!("{}{}", s, unit)
        }
        None => n.to_string(),
    }
}

/// 3 桁区切りの数値文字列（例: `12345` → `12,345`）。
pub fn format_num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `creation_date`（`YYYY/MM/DD HH:MM:SS`）を Kanban todo 表示用
/// `MM/DD HH:MM:SS` に変換する（他レーンと同一書式・A4）。形式外はそのまま返す。
///
/// （`TodoCardInfo::date` が使う）。
pub fn format_creation_date_short(date: &str) -> String {
    let t = date.trim();
    if t.len() >= 19 && t.as_bytes().get(4) == Some(&b'/') {
        // `YYYY/MM/DD HH:MM:SS` → `MM/DD HH:MM:SS`
        format!("{} {}", &t[5..10].replace('-', "/"), &t[11..19])
    } else if t.len() >= 16 && t.as_bytes().get(4) == Some(&b'/') {
        // 秒欠損時は分まで（後方互換）
        format!("{} {}", &t[5..10].replace('-', "/"), &t[11..16])
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{format_cost, format_num, format_tokens};

    #[test]
    fn test_format_num() {
        assert_eq!(format_num(0), "0");
        assert_eq!(format_num(999), "999");
        assert_eq!(format_num(12_345), "12,345");
        assert_eq!(format_num(45_600_000), "45,600,000");
    }

    #[test]
    fn test_format_cost() {
        // 0.01 以上は小数 2 桁でドル表示
        assert_eq!(format_cost(Some(0.42)), "$0.42");
        assert_eq!(format_cost(Some(1.0)), "$1.00");
        assert_eq!(format_cost(Some(12.8)), "$12.80");
        // 0.01 未満は「0 ではない」ことを示す
        assert_eq!(format_cost(Some(0.0)), "<$0.01");
        assert_eq!(format_cost(Some(0.009)), "<$0.01");
        // 不明は 0 と出さない
        assert_eq!(format_cost(None), "cost unknown");
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(12_300), "12.3k");
        assert_eq!(format_tokens(120_000), "120k");
        assert_eq!(format_tokens(300_000), "300k");
        assert_eq!(format_tokens(8_100), "8.1k");
        assert_eq!(format_tokens(1_200_000), "1.2M");
    }
}
