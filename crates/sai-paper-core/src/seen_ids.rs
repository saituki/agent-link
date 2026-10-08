//! 台帳（seen_ids）。run ごとに「CLI が標準出力に出した番号」を残し、`cites` がその中にあるかを照合する。
//!
//! 置き場は `<root>/.sai/seen_ids/<run_id>`（1 行に 1 つの番号）。
//! sai-link-engine の `seen_ids.rs` と同じ正規表現・同じ置き場（共有契約 C10）。両方を直すときは揃える。

use std::collections::HashSet;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::paths;
use crate::textcheck::parse_cite;

/// 1 つの塊の範囲で開く番号の上限（`#37.5.1-100` は 50 個で止める）。
const MAX_RANGE: i64 = 50;

static NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:^|[^0-9A-Za-z&#/])#([1-9][0-9]{0,8})(?:\.([1-9][0-9]{0,4})(?:\.([1-9][0-9]{0,4})(?:-([1-9][0-9]{0,4}))?)?)?",
    )
    .expect("seen_ids regex")
});

/// C10 の正規表現で文から番号を拾う。範囲は塊ごとに開く（1 つの範囲は 50 個まで）。順は出た順、重複は除く。
#[must_use]
pub fn extract(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for caps in NUMBER_RE.captures_iter(text) {
        let num = |i: usize| caps.get(i).and_then(|m| m.as_str().parse::<i64>().ok());
        let Some(paper) = num(1) else { continue };
        let found: Vec<String> = match (num(2), num(3)) {
            (None, _) => vec![format!("#{paper}")],
            (Some(sec), None) => vec![format!("#{paper}.{sec}")],
            (Some(sec), Some(from)) => {
                let to = num(4)
                    .map_or(from, |t| t.max(from))
                    .min(from + MAX_RANGE - 1);
                (from..=to).map(|k| format!("#{paper}.{sec}.{k}")).collect()
            }
        };
        for n in found {
            if seen.insert(n.clone()) {
                out.push(n);
            }
        }
    }
    out
}

/// 台帳のパス `<root>/.sai/seen_ids/<run_id>`。
#[must_use]
pub fn ledger_path(root: &Path, run_id: i64) -> PathBuf {
    root.join(".sai").join("seen_ids").join(run_id.to_string())
}

/// `SAI_PARENT_RUN_ID` を正の整数として読む。
#[must_use]
pub fn run_id_from_env() -> Option<i64> {
    std::env::var("SAI_PARENT_RUN_ID")
        .ok()?
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|n| *n > 0)
}

/// `text` の番号を `run_id` の台帳に追記する。番号が無ければ何もしない。失敗は標準エラーに 1 行出して戻る。
pub fn record_for(root: &Path, run_id: i64, text: &str) {
    let numbers = extract(text);
    if numbers.is_empty() {
        return;
    }
    let path = ledger_path(root, run_id);
    if let Err(e) = append(&path, &numbers) {
        eprintln!("台帳に書けませんでした: {}: {e}", path.display());
    }
}

/// 1 回の書き込みで追記する（フォルダが無ければ作る）。
fn append(path: &Path, numbers: &[String]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut body = String::new();
    for n in numbers {
        body.push_str(n);
        body.push('\n');
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(body.as_bytes())
}

/// `run_id_from_env()` と `paths::project_root()` が両方あるときだけ [`record_for`]。
pub fn record(text: &str) {
    if let (Some(run_id), Some(root)) = (run_id_from_env(), paths::project_root()) {
        record_for(&root, run_id, text);
    }
}

/// `cites`（`#37.5.2` や `#37 §3.2` の形）を台帳で照合する。通らない最初の 1 つで Err（C10 の文）。
/// 台帳が無い run は、何も見ていない run なので空の台帳として照合する（読める cite があれば Err）。
/// 台帳が読めないとき（権限など）は、標準エラーに 1 行出して Ok にする。読めない cite は照合しない。
///
/// # Errors
///
/// 台帳に無い番号があるとき、その番号を含む 1 行の文。
pub fn check_for(root: &Path, run_id: i64, cites: &[String]) -> Result<(), String> {
    let path = ledger_path(root, run_id);
    let body = match std::fs::read_to_string(&path) {
        Ok(body) => body,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => {
            eprintln!(
                "台帳を読めませんでした（検査を飛ばします）: {}: {e}",
                path.display()
            );
            return Ok(());
        }
    };
    let lines: HashSet<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    for cite in cites {
        let Some(parsed) = parse_cite(cite.trim()) else {
            continue;
        };
        let paper = format!("#{}", parsed.paper);
        // 範囲 `37.5.2-4` は始めの塊で見る。論文 `#37` だけは `#37.` で始まる行でも通す。
        let known = match (parsed.section, parsed.passage) {
            (None, _) => {
                lines.contains(paper.as_str())
                    || lines.iter().any(|l| l.starts_with(&format!("{paper}.")))
            }
            (Some(sec), None) => lines.contains(format!("{paper}.{sec}").as_str()),
            (Some(sec), Some(pas)) => lines.contains(format!("{paper}.{sec}.{pas}").as_str()),
        };
        if !known {
            return Err(format!(
                "台帳に無い番号です: {}（この run で CLI が出した番号だけを使えます）",
                cite.trim()
            ));
        }
    }
    Ok(())
}

/// `run_id_from_env()` と root があるときだけ [`check_for`]。run の ID が無い（手で打つ）ときは Ok。
///
/// # Errors
///
/// [`check_for`] と同じ。
pub fn check(cites: &[String]) -> Result<(), String> {
    match (run_id_from_env(), paths::project_root()) {
        (Some(run_id), Some(root)) => check_for(&root, run_id, cites),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一時フォルダを空にして作る（テストの名前ごとに分ける）。
    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("seen-ids-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn extract_forms_and_boundaries() {
        assert_eq!(
            extract("見出し #37.5（序論）と [#40.2.1-3]、色 #fff と &#123;"),
            vec!["#37.5", "#40.2.1", "#40.2.2", "#40.2.3"]
        );
        // `#0` は拾わない。`#37.0` は節が形を外れるので論文の `#37` だけになる（照合が緩むだけで害は無い）。
        let zero = extract("#0 と #37.0");
        assert!(!zero.iter().any(|n| n == "#0" || n == "#37.0"), "{zero:?}");
        // 範囲は 50 個で止める。逆向きの範囲は始めの塊だけ。
        let long = extract("#37.5.1-100");
        assert_eq!(long.len(), 50);
        assert_eq!(long.last().map(String::as_str), Some("#37.5.50"));
        assert_eq!(extract("#37.5.4-2"), vec!["#37.5.4"]);
        assert_eq!(extract("#37 と #37"), vec!["#37"]);
    }

    #[test]
    fn check_rules() {
        let root = temp_root("check");
        std::fs::create_dir_all(root.join(".sai/seen_ids")).unwrap();
        std::fs::write(ledger_path(&root, 7), "#37.5.2\n#40\n").unwrap();
        let ok = |c: &str| check_for(&root, 7, &[c.to_string()]);
        assert!(ok("#37.5.2").is_ok());
        assert!(ok("#37").is_ok(), "`#37.` で始まる行があれば論文は通す");
        assert!(ok("#37.5").is_err(), "節は完全一致");
        assert!(ok("#40.1.1").is_err());
        assert!(ok("#40").is_ok());
        assert!(ok("#37.5.2-4").is_ok(), "範囲は始めの塊で見る");
        assert!(ok("#37 §3.2").is_ok(), "補足は照合しない");
        assert!(ok("#37.5.3").is_err());
        let err = ok("#37.5.3").unwrap_err();
        assert_eq!(
            err,
            "台帳に無い番号です: #37.5.3（この run で CLI が出した番号だけを使えます）"
        );
        // 読めない cite は照合しない。
        assert!(ok("#37.5.2（序論）").is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_ledger_is_empty() {
        let root = temp_root("missing");
        // 台帳が無い run は何も見ていない。cite があれば止まり、無ければ通る。
        assert!(check_for(&root, 9, &["#1".to_string()]).is_err());
        assert!(check_for(&root, 9, &[]).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn record_appends_and_creates_dir() {
        let root = temp_root("record");
        record_for(&root, 7, "a #37.5 b");
        record_for(&root, 7, "[#40.2.1-2]");
        assert_eq!(
            std::fs::read_to_string(ledger_path(&root, 7)).unwrap(),
            "#37.5\n#40.2.1\n#40.2.2\n"
        );
        // 番号が無ければファイルを作らない。
        record_for(&root, 8, "番号は無い");
        assert!(!ledger_path(&root, 8).exists());
        // 書けないときは止まらない（`.sai` が普通のファイルのとき）。
        let blocked = temp_root("record-blocked");
        std::fs::write(blocked.join(".sai"), "x").unwrap();
        record_for(&blocked, 7, "#1");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&blocked);
    }
}
