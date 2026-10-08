//! # 台帳（seen_ids）— agent が見た番号の記録（C10）
//!
//! run ごとに `<root>/.sai/seen_ids/<run_id>` へ、agent が見た文の中の番号
//! （`#37`・`#37.5`・`#37.5.2`）を 1 行 1 つ追記する。`sai-sci op` の引用の検査が
//! この台帳を見て、CLI が出していない番号の引用を弾く。
//!
//! 番号を拾う規則は `sai-paper-core::seen_ids` と同じにする（C10）。
//! 同じ見本の文で同じ結果になることを、両方のテストで確かめる。
//!
//! 書けなくても命令は止めない。失敗は標準エラーに 1 行出すだけにする。

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

/// C10 の正規表現。`sai-paper-core::seen_ids` と同じ文字列にする。
const PATTERN: &str = r"(?:^|[^0-9A-Za-z&#/])#([1-9][0-9]{0,8})(?:\.([1-9][0-9]{0,4})(?:\.([1-9][0-9]{0,4})(?:-([1-9][0-9]{0,4}))?)?)?";

/// 1 つの範囲で開く個数の上限（C10）。
const RANGE_LIMIT: u32 = 50;

fn re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(PATTERN).expect("台帳の番号の正規表現"))
}

/// 文から番号を拾う。範囲（`#37.5.2-4`）は塊ごとに開く（50 個まで）。
/// 重複は除き、最初に出た順に返す。
pub fn extract(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in re().captures_iter(text) {
        let parts: Vec<&str> = (1..=3)
            .filter_map(|i| c.get(i))
            .map(|m| m.as_str())
            .collect();
        let ids: Vec<String> = match (parts.len(), c.get(4)) {
            // 範囲 `#40.2.1-3`: 最初の塊から終わりの塊まで開く
            (3, Some(end)) => {
                let start: u32 = parts[2].parse().unwrap_or(0);
                let end: u32 = end.as_str().parse().unwrap_or(0);
                if end < start {
                    vec![format!("#{}", parts.join("."))]
                } else {
                    let last = end.min(start.saturating_add(RANGE_LIMIT - 1));
                    (start..=last)
                        .map(|k| format!("#{}.{}", parts[..2].join("."), k))
                        .collect()
                }
            }
            _ => vec![format!("#{}", parts.join("."))],
        };
        for id in ids {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

/// `<root>/.sai/seen_ids/<run_id>`（拡張子なし）。
pub fn ledger_path(root: &Path, run_id: i64) -> PathBuf {
    root.join(".sai").join("seen_ids").join(run_id.to_string())
}

/// 自分の run_id（環境変数 `SAI_PARENT_RUN_ID`、正の整数）。無ければ `None`。
pub fn parent_run_id_from_env() -> Option<i64> {
    std::env::var("SAI_PARENT_RUN_ID")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|n| *n > 0)
}

/// 文から拾った番号を台帳に追記する。番号が無ければ何もしない。
/// 失敗は標準エラーに 1 行（`台帳に書けませんでした: <path>: <理由>`）出すだけにする。
pub fn record_for(root: &Path, run_id: i64, text: &str) {
    let ids = extract(text);
    if ids.is_empty() {
        return;
    }
    let path = ledger_path(root, run_id);
    if let Err(e) = append(&path, &ids) {
        eprintln!("台帳に書けませんでした: {}: {}", path.display(), e);
    }
}

fn append(path: &Path, ids: &[String]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut body = ids.join("\n");
    body.push('\n');
    file.write_all(body.as_bytes())
}

/// 台帳の異なる番号の集合（U13 の集計が使う）。ファイルが無ければ空。
pub fn read_all(root: &Path, run_id: i64) -> BTreeSet<String> {
    std::fs::read_to_string(ledger_path(root, run_id))
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sai-paper-core::seen_ids` のテストと同じ見本・同じ期待値（C10）。
    #[test]
    fn extract_same_as_paper_core() {
        assert_eq!(
            extract("見出し #37.5（序論）と [#40.2.1-3]、色 #fff と &#123;"),
            vec!["#37.5", "#40.2.1", "#40.2.2", "#40.2.3"]
        );
    }

    #[test]
    fn extract_range_limit_and_dedup() {
        // 範囲は 50 個まで、重複は除く、範囲の終わりが始まりより小さければ始まりだけ
        assert_eq!(extract("#1.2.1-100").len(), 50);
        assert_eq!(extract("#1.2.1 と #1.2.1"), vec!["#1.2.1"]);
        assert_eq!(extract("#1.2.5-3"), vec!["#1.2.5"]);
    }

    #[test]
    fn record_and_read_all() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_all(dir.path(), 7).is_empty());

        record_for(dir.path(), 7, "依頼 #12.3.4 と #12 と #12.3.4");
        record_for(dir.path(), 7, "番号が無い文");
        record_for(dir.path(), 7, "範囲 #5.1.1-2");

        let got: Vec<String> = read_all(dir.path(), 7).into_iter().collect();
        assert_eq!(got, vec!["#12", "#12.3.4", "#5.1.1", "#5.1.2"]);
        // run ごとに別のファイル（run 8 の台帳は作られない）
        assert!(ledger_path(dir.path(), 7).is_file());
        assert!(!ledger_path(dir.path(), 8).exists());
    }
}
