//! cc の費用（`cc-cost/<session_id>.json`）と枠の値（`cc-rate-limits.json`）。
//!
//! `agent-link cc-statusline`（cc の statusLine の標準入力）が書き、書き出し（`run_export`）が読む。

use std::path::{Path, PathBuf};

use crate::common::now_iso;

/// cc 自身のセッション金額（`~/.agent-link/cc-cost/<session_id>.json`・00 §3.8）を読む。
///
/// 無ければ `None`（呼び出し側は概算にフォールバックする）。
pub fn read_cc_cost(session_id: &str) -> Option<f64> {
    if !is_safe_id(session_id) {
        return None;
    }
    let home = crate::project::sai_home()?;
    let path = home.join("cc-cost").join(format!("{}.json", session_id));
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("total_cost_usd")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v >= 0.0)
}

/// セッション ID として安全な文字だけか（パストラバーセル防止）。
fn is_safe_id(id: &str) -> bool {
    let b = id.as_bytes();
    (8..=64).contains(&b.len())
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'-' || *c == b'_')
}

/// JSON を一時ファイル → rename で書く（読み手が書きかけを読まないようにするため）。
///
/// 親ディレクトリが無ければ作る。
/// `tmp` と `dst` は同じディレクトリに置くこと（rename が原子的なのは同一 FS のため）。
fn write_json_atomic(tmp: &Path, dst: &Path, body: &serde_json::Value) -> std::io::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(body)
        .map_err(|e| std::io::Error::other(format!("JSON serialization failed: {}", e)))?;
    std::fs::write(tmp, text)?;
    std::fs::rename(tmp, dst)?;
    Ok(())
}

/// `~/.agent-link/cc-cost` の出力先（置き場を解決できなければ None）。
pub fn cc_cost_dir() -> Option<PathBuf> {
    crate::project::sai_home().map(|h| h.join("cc-cost"))
}

/// cc の費用を `<dir>/<session_id>.json` に書く（一時ファイル → rename）。
///
/// 形は `{"session_id", "total_cost_usd", "updated_at"}`。
/// headless では statusLine が呼ばれないため、engine が stream-json の `result` 行から
/// この関数で書く（読む側は [`read_cc_cost`] のまま変えない）。
pub fn write_cc_cost(dir: &Path, session_id: &str, total: f64) -> std::io::Result<()> {
    let tmp = dir.join(format!(".{}.json.tmp", session_id));
    let dst = dir.join(format!("{}.json", session_id));
    let body = serde_json::json!({
        "session_id": session_id,
        "total_cost_usd": total,
        "updated_at": now_iso(),
    });
    write_json_atomic(&tmp, &dst, &body)
}

// ============================================================================
// cc の枠の値（cc-rate-limits.json）
// ============================================================================

/// cc の 5 時間枠・7 日枠の値を `<path>` に書く（一時ファイル → rename）。
///
/// `rate_limits` は statusLine 入力の `rate_limits` の値（`five_hour` / `seven_day` を持つ）。
/// `used_percentage` が数の窓だけ残す（欠けた窓はキーごと省く）。1 つも残らなければ書かない。
pub fn write_cc_rate_limits(
    path: &Path,
    rate_limits: &serde_json::Value,
    session_id: &str,
) -> std::io::Result<()> {
    let mut body = serde_json::Map::new();
    for key in ["five_hour", "seven_day"] {
        let Some(window) = rate_limits.get(key).filter(|w| w.is_object()) else {
            continue;
        };
        let Some(pct) = window.get("used_percentage").filter(|v| v.is_number()) else {
            continue;
        };
        body.insert(
            key.to_string(),
            serde_json::json!({
                "used_percentage": pct,
                "resets_at": window.get("resets_at").cloned().unwrap_or(serde_json::Value::Null),
            }),
        );
    }
    if body.is_empty() {
        return Ok(());
    }
    body.insert("session_id".to_string(), serde_json::json!(session_id));
    body.insert("updated_at".to_string(), serde_json::json!(now_iso()));
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(".cc-rate-limits.json.tmp");
    write_json_atomic(&tmp, path, &serde_json::Value::Object(body))
}

/// cc の statusLine 標準入力 1 件を処理する（`agent-link cc-statusline` の本体）。
///
/// 戻り値は cc の画面下に出す 1 行（例 `agent-link #58 $1.27`）。読めない・費用が無い・書けないときは空行。
pub fn handle_cc_statusline(input: &str) -> String {
    let Some(dir) = cc_cost_dir() else {
        return String::new();
    };
    handle_cc_statusline_in(input, &dir)
}

/// [`handle_cc_statusline`] の出力先指定版（テスト用）。
///
/// `dir` は `cc-cost` のフォルダ。枠の値は `dir` の親（`<sai_home>`）の `cc-rate-limits.json` に書く。
pub fn handle_cc_statusline_in(input: &str, dir: &Path) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(input) else {
        return String::new();
    };
    let Some(session_id) = value.get("session_id").and_then(|v| v.as_str()) else {
        return String::new();
    };
    if !is_safe_id(session_id) {
        return String::new();
    }
    // 枠の値は費用の有無と関係なく書く（応答の後にだけ入るため）
    if let (Some(rate_limits), Some(home)) = (value.get("rate_limits"), dir.parent()) {
        if let Err(e) = write_cc_rate_limits(&home.join("cc-rate-limits.json"), rate_limits, session_id) {
            eprintln!("[agent-link] cc-rate-limits の書込みに失敗: {}", e);
        }
    }
    // コストが取れない場合は書かない（概算フォールバックを妨げないため）
    let Some(total) = value
        .pointer("/cost/total_cost_usd")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v >= 0.0)
    else {
        return String::new();
    };
    if let Err(e) = write_cc_cost(dir, session_id, total) {
        eprintln!("[agent-link] cc-cost の書込みに失敗: {}", e);
        return String::new();
    }
    let amount = format!("${:.2}", total);
    match std::env::var("AGENT_LINK_PARENT_RUN_ID") {
        Ok(id) if !id.trim().is_empty() => format!("agent-link #{} {}", id.trim(), amount),
        _ => format!("agent-link {}", amount),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// statusLine 標準入力 JSON から金額を取り出しファイルを書けること。
    #[test]
    fn cc_statusline_writes_cost_file() {
        let dir = tempfile::tempdir().unwrap();
        let session_id = "11111111-2222-4333-8444-555555555555";
        let input = format!(
            r#"{{"session_id":"{}","transcript_path":"/tmp/x.jsonl","cost":{{"total_cost_usd":1.274}}}}"#,
            session_id
        );
        let line = handle_cc_statusline_in(&input, dir.path());
        assert!(line.contains("$1.27"), "画面表示に金額が入る: {}", line);
        assert!(!dir.path().join(format!(".{}.json.tmp", session_id)).exists());

        let text = std::fs::read_to_string(dir.path().join(format!("{}.json", session_id))).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["session_id"], session_id);
        assert!((v["total_cost_usd"].as_f64().unwrap() - 1.274).abs() < 1e-9);
        assert!(v["updated_at"].as_str().unwrap().ends_with('Z'));
    }

    /// 入力が壊れている・session_id / cost が無い・ID が不正なときは空行を返し、ファイルを書かない。
    #[test]
    fn cc_statusline_swallows_bad_input() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(handle_cc_statusline_in("not json", dir.path()), "");
        assert_eq!(
            handle_cc_statusline_in(r#"{"cost":{"total_cost_usd":1.0}}"#, dir.path()),
            ""
        );
        let sid = "11111111-2222-4333-8444-555555555555";
        assert_eq!(
            handle_cc_statusline_in(&format!(r#"{{"session_id":"{}"}}"#, sid), dir.path()),
            ""
        );
        assert_eq!(
            handle_cc_statusline_in(
                r#"{"session_id":"../../etc/passwd","cost":{"total_cost_usd":1.0}}"#,
                dir.path()
            ),
            ""
        );
        assert!(!dir.path().join(format!("{}.json", sid)).exists());
    }

    /// `rate_limits` があれば `<sai_home>/cc-rate-limits.json` に枠の値を書くこと。
    #[test]
    fn cc_statusline_writes_rate_limits() {
        let tmp = tempfile::tempdir().unwrap();
        let cost_dir = tmp.path().join("cc-cost");
        let session_id = "11111111-2222-4333-8444-555555555555";
        let input = format!(
            r#"{{"session_id":"{}","rate_limits":{{"five_hour":{{"used_percentage":23.5,"resets_at":1738425600}},"seven_day":{{"used_percentage":41.2,"resets_at":1738857600}}}}}}"#,
            session_id
        );
        // 費用が無くても枠の値は書く（画面下の 1 行は空）
        assert_eq!(handle_cc_statusline_in(&input, &cost_dir), "");
        let text = std::fs::read_to_string(tmp.path().join("cc-rate-limits.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["five_hour"]["used_percentage"], 23.5);
        assert_eq!(v["five_hour"]["resets_at"], 1738425600_i64);
        assert_eq!(v["seven_day"]["used_percentage"], 41.2);
        assert_eq!(v["seven_day"]["resets_at"], 1738857600_i64);
        assert_eq!(v["session_id"], session_id);
        assert!(v["updated_at"].as_str().unwrap().ends_with('Z'));
        assert!(!tmp.path().join(".cc-rate-limits.json.tmp").exists());
    }

    /// `rate_limits` が無い・数でない入力では枠の値のファイルを書かないこと。
    #[test]
    fn cc_statusline_skips_rate_limits_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let cost_dir = tmp.path().join("cc-cost");
        let sid = "11111111-2222-4333-8444-555555555555";
        handle_cc_statusline_in(
            &format!(r#"{{"session_id":"{}","cost":{{"total_cost_usd":1.0}}}}"#, sid),
            &cost_dir,
        );
        assert!(!tmp.path().join("cc-rate-limits.json").exists());
        handle_cc_statusline_in(
            &format!(
                r#"{{"session_id":"{}","rate_limits":{{"five_hour":{{"used_percentage":"23"}}}}}}"#,
                sid
            ),
            &cost_dir,
        );
        assert!(!tmp.path().join("cc-rate-limits.json").exists());
    }

    /// 片方の窓だけでも書けること（欠けた窓はキーごと省く）。
    #[test]
    fn cc_statusline_writes_single_window() {
        let tmp = tempfile::tempdir().unwrap();
        let cost_dir = tmp.path().join("cc-cost");
        let sid = "11111111-2222-4333-8444-555555555555";
        handle_cc_statusline_in(
            &format!(
                r#"{{"session_id":"{}","rate_limits":{{"seven_day":{{"used_percentage":41.2,"resets_at":1738857600}}}}}}"#,
                sid
            ),
            &cost_dir,
        );
        let text = std::fs::read_to_string(tmp.path().join("cc-rate-limits.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["seven_day"]["used_percentage"], 41.2);
        assert!(v.get("five_hour").is_none(), "欠けた窓はキーごと無い: {}", v);
    }
}
