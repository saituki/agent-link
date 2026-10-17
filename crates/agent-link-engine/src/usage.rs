//! 実行 usage（`execution_usage` テーブル）の管理モジュール（L1）。
//!
//! provider が返す実際の usage（token / cost）を DB に保存し、
//! 横断集計を **DB 読取のみ**で行えるようにする。
//!
//! - 保存: [`backfill_execution_usage`]（CLI `agent-link usage backfill` から使用）。
//!   実行 1 件分を 1 トランザクションで**全置換**するため冪等
//! - 抽出: `crate::session::extract_execution_usage`（jsonl 走査 1 回で
//!   usage 合計 + 種別 × ターン別内訳を同時算出する）
//!
//! ## 種別への振り分けは推定値
//!
//! provider は assistant 応答単位でしか usage を報告しないため、
//! `message` / `thinking` / `toolCall` / `toolResult` への内訳は
//! エントリサイズ比による**推定**である。`total` 行のみが生の provider 値（正）。

use crate::db::models::{DbError, ExecutionUsageRow};
use crate::db::SaiStore;
use crate::session::{extract_execution_usage, find_main_session_file, resolve_session_dir};

/// 実行 1 件分の usage 合計（token 3 種 + 費用）。
///
/// 費用は provider が返していない場合があるため [`Option`] で表す。
/// 不明を `0.0` と見せると「安い」と誤解させるので `None` にする。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UsageTotal {
    /// 入力トークン
    pub input: u64,
    /// 出力トークン
    pub output: u64,
    /// キャッシュ読み込み + キャッシュ書き込み
    pub cache: u64,
    /// 費用（USD）。provider が費用を返していない場合は `None`
    pub cost: Option<f64>,
}

impl UsageTotal {
    /// 加算する。
    ///
    /// `cost` は分かっている側だけを合計する（両方 `None` なら `None`）。
    /// 不明分が混ざっているかは [`UsageSummary::unknown_cost_runs`] で判断する。
    pub fn add(&mut self, other: &UsageTotal) {
        self.input += other.input;
        self.output += other.output;
        self.cache += other.cache;
        self.cost = match (self.cost, other.cost) {
            (Some(a), Some(b)) => Some(a + b),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
    }
}

/// 親実行 1 件と、その直接の子実行の usage をまとめたもの（B02 の画面表示用）。
#[derive(Debug, Clone, PartialEq)]
pub struct UsageSummary {
    /// 親実行自身の usage。usage 未保存（セッション無し等）は `None`
    pub own: Option<UsageTotal>,
    /// 子実行（`parent_run_id = run_id`）の `(execution_id, usage)`。未保存は `None`
    pub children: Vec<(i64, Option<UsageTotal>)>,
    /// 自身 + 子の合計。`cost` は費用の分かった分だけを合計する
    pub total: UsageTotal,
    /// usage はあるが費用が不明だった実行数（自身 + 子）。
    /// 0 でなければ [`Self::total`] の `cost` は不完全である
    pub unknown_cost_runs: usize,
}

/// backfill の結果報告。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackfillReport {
    /// 対象となった実行数（session_dir を持つ実行）
    pub scanned: usize,
    /// usage を保存（または dry-run で保存対象）とした実行数
    pub updated: usize,
    /// session_dir 不在・jsonl 不在・解析失敗で読み飛ばした実行数
    pub skipped: usize,
    /// dry-run かどうか
    pub dry_run: bool,
}

/// 既存実行分の jsonl から usage を再計算して `execution_usage` に保存する（冪等）。
///
/// 実行ごとに **全置換**（既存行を削除してから挿入）するため、
/// 複数回実行しても結果が変わらない。実行単位でコミットするため、
/// 中断後の再開も冪等である。
///
/// # 引数
///
/// * `store` — 自プロジェクトの DB ストア
/// * `dry_run` — `true` の場合は保存せず集計のみ行う
/// * `progress` — 実行ごとに進捗文字列を受け取るコールバック
pub fn backfill_execution_usage(
    store: &SaiStore,
    dry_run: bool,
    progress: &mut dyn FnMut(&str),
) -> Result<BackfillReport, crate::db::models::DbError> {
    let executions = store.list_all_executions()?;
    let mut report = BackfillReport {
        dry_run,
        ..Default::default()
    };

    for exec in &executions {
        let Some(exec_id) = exec.id else { continue };
        // owner 移動追従のため現行正本を解決する（DB の旧記録は古くなる）。
        let Some(session_dir) = resolve_session_dir(exec.session_dir.as_deref(), exec_id) else {
            report.skipped += 1;
            continue;
        };
        report.scanned += 1;
        // メイン jsonl が解決できない場合は読み飛ばす（進捗は報告する）
        if find_main_session_file(&session_dir).is_err() {
            report.skipped += 1;
            progress(&format!(
                "ID{}: session jsonl not found (skipped)",
                exec_id
            ));
            continue;
        }

        if dry_run {
            match extract_execution_usage(&session_dir) {
                Ok(scan) => {
                    progress(&format!(
                        "ID{}: total(in={}, out={}, cache={}, cost=${:.4}), {} breakdown rows",
                        exec_id,
                        scan.total.input,
                        scan.total.output,
                        scan.total.cache_total(),
                        scan.total.cost_total,
                        scan.breakdown.len()
                    ));
                    report.updated += 1;
                }
                Err(e) => {
                    report.skipped += 1;
                    progress(&format!("ID{}: parse failed (skipped): {}", exec_id, e));
                }
            }
            continue;
        }

        // 保存は 1 実行分の共通処理 [`record_execution_usage`] に集約する。
        match record_execution_usage(store, exec_id) {
            Ok(Some(total)) => {
                let cost = match total.cost {
                    Some(c) => format!("{:.4}", c),
                    None => "unknown".to_string(),
                };
                progress(&format!(
                    "ID{}: saved total(in={}, out={}, cache={}, cost=${})",
                    exec_id, total.input, total.output, total.cache, cost
                ));
                report.updated += 1;
            }
            Ok(None) => {
                report.skipped += 1;
                progress(&format!(
                    "ID{}: session not found (skipped)",
                    exec_id
                ));
            }
            Err(e) => {
                report.skipped += 1;
                progress(&format!("ID{}: save failed (skipped): {}", exec_id, e));
            }
        }
    }
    Ok(report)
}

/// 実行 1 件分の usage をセッションの jsonl から再計算して `execution_usage` に保存する。
///
/// 実行完了時に呼ぶ（ベストエフォート）。実行 1 件分を全置換するため冪等。
/// セッションの記録（session_dir / メイン jsonl）が見つからない場合は `Ok(None)`。
///
/// 費用の不明判定は [`cost_or_unknown`] を参照。
///
/// # エラー
///
/// DB 読取・jsonl 解析・保存のいずれかに失敗した場合は、その内容を `String` で返す。
pub fn record_execution_usage(
    store: &SaiStore,
    exec_id: i64,
) -> Result<Option<UsageTotal>, String> {
    let exec = store
        .get_execution_by_id(exec_id)
        .map_err(|e| e.to_string())?;
    // owner 移動追従のため現行正本を解決する。
    let Some(session_dir) = resolve_session_dir(exec.session_dir.as_deref(), exec_id) else {
        return Ok(None);
    };
    if find_main_session_file(&session_dir).is_err() {
        return Ok(None);
    }
    let scan = extract_execution_usage(&session_dir).map_err(|e| e.to_string())?;
    let rows = scan_to_rows(exec_id, &scan);
    store
        .replace_execution_usage(exec_id, &rows)
        .map_err(|e| e.to_string())?;
    Ok(Some(total_from_amount(&scan.total)))
}

/// 親実行 1 件と、その直接の子実行（`parent_run_id = run_id`）の usage をまとめる。
///
/// 孫はたどらない。`total` は自身 + 子の合計で、`cost` は費用の分かった分だけを足す。
/// 費用不明の実行数は [`UsageSummary::unknown_cost_runs`] に入る。
pub fn usage_with_children(store: &SaiStore, run_id: i64) -> Result<UsageSummary, DbError> {
    let own = stored_usage_total(store, run_id)?;
    let mut total = own.unwrap_or_default();
    let mut unknown_cost_runs = 0usize;
    if let Some(t) = &own {
        if t.cost.is_none() {
            unknown_cost_runs += 1;
        }
    }
    let mut children = Vec::new();
    for child_id in store.list_child_execution_ids(run_id)? {
        let usage = stored_usage_total(store, child_id)?;
        if let Some(t) = &usage {
            if t.cost.is_none() {
                unknown_cost_runs += 1;
            }
            total.add(t);
        }
        children.push((child_id, usage));
    }
    Ok(UsageSummary {
        own,
        children,
        total,
        unknown_cost_runs,
    })
}

/// DB に保存済みの `total` 行から [`UsageTotal`] を作る（未保存は `None`）。
fn stored_usage_total(store: &SaiStore, exec_id: i64) -> Result<Option<UsageTotal>, DbError> {
    let rows = store.list_execution_usage(exec_id)?;
    Ok(rows
        .iter()
        .find(|r| r.entry_type == "total")
        .map(total_from_row))
}

/// 抽出結果の合計（[`crate::session::UsageAmount`]）から [`UsageTotal`] を作る。
fn total_from_amount(total: &crate::session::UsageAmount) -> UsageTotal {
    let tokens = total.input + total.output + total.cache_total();
    UsageTotal {
        input: total.input,
        output: total.output,
        cache: total.cache_total(),
        cost: cost_or_unknown(total.cost_total, tokens),
    }
}

/// DB の `total` 行から [`UsageTotal`] を作る（費用の不明判定込み）。
///
/// 画面側（Kanban カード・Project 画面）が `list_execution_usage_totals()` の行から
/// 費用を出すときにも使う。判定（[`cost_or_unknown`]）を二重に持たないための公開である。
pub fn total_from_row(row: &ExecutionUsageRow) -> UsageTotal {
    let input = row.input_tokens.max(0) as u64;
    let output = row.output_tokens.max(0) as u64;
    let cache = row.cache_read.max(0) as u64 + row.cache_write.max(0) as u64;
    UsageTotal {
        input,
        output,
        cache,
        cost: cost_or_unknown(row.cost_total, input + output + cache),
    }
}

/// 費用の不明を判定する。
///
/// `crate::session::extract_execution_usage` が使う `parse_assistant_usage` は、
/// `usage.cost.total` が無い場合に `0.0` を入れる（`session.rs:1257`。欠損と
/// 真の 0 を区別しない）。そのため、トークン数があるのに費用が 0 の場合は
/// 「provider が費用を返していない」＝不明として `None` にする。
/// トークンも 0 なら費用も 0 として `Some(0.0)` を返す。
fn cost_or_unknown(cost_total: f64, tokens: u64) -> Option<f64> {
    if cost_total == 0.0 && tokens > 0 {
        None
    } else {
        Some(cost_total)
    }
}

/// 抽出結果を DB の行一覧へ変換する（先頭が `total` 行）。
pub fn scan_to_rows(
    execution_id: i64,
    scan: &crate::session::ExecutionUsageScan,
) -> Vec<ExecutionUsageRow> {
    let mut rows = Vec::with_capacity(scan.breakdown.len() + 1);
    let mut total = ExecutionUsageRow::total(execution_id);
    total.input_tokens = scan.total.input as i64;
    total.output_tokens = scan.total.output as i64;
    total.cache_read = scan.total.cache_read as i64;
    total.cache_write = scan.total.cache_write as i64;
    total.cost_total = scan.total.cost_total;
    rows.push(total);
    for b in &scan.breakdown {
        let mut row = ExecutionUsageRow::breakdown(execution_id, &b.entry_type, b.turn_no as i64);
        row.input_tokens = b.amount.input as i64;
        row.output_tokens = b.amount.output as i64;
        row.cache_read = b.amount.cache_read as i64;
        row.cache_write = b.amount.cache_write as i64;
        row.cost_total = b.amount.cost_total;
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::BlockMetadata;
    use std::path::{Path, PathBuf};

    /// テスト用のブロック + 実行を作り、session_dir を設定して返す。
    fn setup_exec(store: &SaiStore, session_dir: &Path) -> i64 {
        let block = BlockMetadata::new(
            "usage-test-block".to_string(),
            String::new(),
            "anthropic/claude-sonnet-4-5".to_string(),
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            "blocks/test.md".to_string(),
            None
        );
        let block_id = store.insert_block(&block).unwrap();
        let exec_id = store.create_execution(block_id, None).unwrap();
        store
            .update_execution_session_info(exec_id, &session_dir.to_string_lossy(), "pi")
            .unwrap();
        exec_id
    }

    /// usage 付きセッション jsonl を作成する。
    /// assistant 応答 1 回: usage input=100, output=20, cacheRead=5, cost=0.01。
    fn write_session(dir: &Path) -> PathBuf {
        let path = dir.join("session_01a04b0e-0000-0000-0000-000000000004.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"session","version":3,"id":"x"}"#, "\n",
                r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"質問"}]}}"#, "\n",
                r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"回答"}],"usage":{"input":100,"output":20,"cacheRead":5,"cacheWrite":0,"cost":{"total":0.01}}}}"#, "\n",
            ),
        )
        .unwrap();
        path
    }

    /// cost 欄の無い usage 付きセッション jsonl（provider が費用を返さない場合）。
    /// assistant 応答 1 回: usage input=100, output=20（cost 無し）。
    fn write_session_no_cost(dir: &Path) -> PathBuf {
        let path = dir.join("session_01a04b0e-0000-0000-0000-000000000005.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"session","version":3,"id":"x"}"#, "\n",
                r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"質問"}]}}"#, "\n",
                r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"回答"}],"usage":{"input":100,"output":20,"cacheRead":0,"cacheWrite":0}}}"#, "\n",
            ),
        )
        .unwrap();
        path
    }

    /// テスト用のブロックを 1 つ作り、その id を返す。
    fn test_block_id(store: &SaiStore, name: &str) -> i64 {
        let block = BlockMetadata::new(
            name.to_string(),
            String::new(),
            "anthropic/claude-sonnet-4-5".to_string(),
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            "blocks/test.md".to_string(),
            None
        );
        store.insert_block(&block).unwrap()
    }

    /// DB 保存用の `total` 行を組み立てる。
    fn total_row(
        exec_id: i64,
        input: i64,
        output: i64,
        cache_read: i64,
        cache_write: i64,
        cost: f64,
    ) -> ExecutionUsageRow {
        let mut row = ExecutionUsageRow::total(exec_id);
        row.input_tokens = input;
        row.output_tokens = output;
        row.cache_read = cache_read;
        row.cache_write = cache_write;
        row.cost_total = cost;
        row
    }

    /// backfill: 旧実行分が埋められ、保存内容が正しいこと。
    #[test]
    fn test_backfill_saves_total_and_breakdown() {
        let dir = tempfile::tempdir().unwrap();
        write_session(dir.path());
        let store = SaiStore::open_in_memory().unwrap();
        let exec_id = setup_exec(&store, dir.path());

        let mut progress = |_: &str| {};
        let report = backfill_execution_usage(&store, false, &mut progress).unwrap();
        assert_eq!(report.updated, 1);
        assert_eq!(report.skipped, 0);

        let rows = store.list_execution_usage(exec_id).unwrap();
        let total = rows.iter().find(|r| r.entry_type == "total").unwrap();
        assert_eq!(total.input_tokens, 100);
        assert_eq!(total.output_tokens, 20);
        assert_eq!(total.cache_read, 5);
        assert!((total.cost_total - 0.01).abs() < 1e-9);
        // 内訳（message 1 行 + total 1 行）
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .any(|r| r.entry_type == "message" && r.turn_no == Some(1)));
    }

    /// backfill 冪等性: 複数回実行しても結果が変わらない。
    #[test]
    fn test_backfill_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        write_session(dir.path());
        let store = SaiStore::open_in_memory().unwrap();
        let exec_id = setup_exec(&store, dir.path());

        let mut progress = |_: &str| {};
        backfill_execution_usage(&store, false, &mut progress).unwrap();
        let first = store.list_execution_usage(exec_id).unwrap();
        backfill_execution_usage(&store, false, &mut progress).unwrap();
        let second = store.list_execution_usage(exec_id).unwrap();
        // 主キー（id）は再発番されるため、データ部分のみ比較する
        let strip_id = |rows: Vec<ExecutionUsageRow>| -> Vec<ExecutionUsageRow> {
            rows.into_iter()
                .map(|mut r| {
                    r.id = None;
                    r
                })
                .collect()
        };
        assert_eq!(strip_id(first), strip_id(second));

        // dry-run では保存されない
        let store2 = SaiStore::open_in_memory().unwrap();
        let exec2 = setup_exec(&store2, dir.path());
        backfill_execution_usage(&store2, true, &mut progress).unwrap();
        assert!(store2.list_execution_usage(exec2).unwrap().is_empty());
    }

    /// session_dir 無しの実行はスキップされること。
    #[test]
    fn test_backfill_skips_missing_session_dir() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = BlockMetadata::new(
            "no-session".to_string(),
            String::new(),
            "m".to_string(),
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            "blocks/test.md".to_string(),
            None
        );
        let block_id = store.insert_block(&block).unwrap();
        store.create_execution(block_id, None).unwrap();

        let mut progress = |_: &str| {};
        let report = backfill_execution_usage(&store, false, &mut progress).unwrap();
        assert_eq!(report.updated, 0);
        assert_eq!(report.skipped, 1);
    }

    /// record_execution_usage: セッションから保存でき、2 回呼んでも行が増えない。
    #[test]
    fn test_record_execution_usage_saves_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        write_session(dir.path());
        let store = SaiStore::open_in_memory().unwrap();
        let exec_id = setup_exec(&store, dir.path());

        let total = record_execution_usage(&store, exec_id).unwrap().unwrap();
        assert_eq!(total.input, 100);
        assert_eq!(total.output, 20);
        assert_eq!(total.cache, 5);
        assert_eq!(total.cost, Some(0.01));

        let first = store.list_execution_usage(exec_id).unwrap();
        assert_eq!(first.len(), 2); // total 1 行 + 内訳 1 行

        // 2 回目（全置換なので行数は増えない）
        let total2 = record_execution_usage(&store, exec_id).unwrap().unwrap();
        assert_eq!(total2, total);
        let second = store.list_execution_usage(exec_id).unwrap();
        assert_eq!(second.len(), first.len());
    }

    /// record_execution_usage: session_dir の無い実行は `Ok(None)`（保存しない）。
    #[test]
    fn test_record_execution_usage_none_without_session() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = test_block_id(&store, "no-session-record");
        let exec_id = store.create_execution(block_id, None).unwrap();
        assert!(record_execution_usage(&store, exec_id).unwrap().is_none());
        assert!(store.list_execution_usage(exec_id).unwrap().is_empty());
    }

    /// record_execution_usage: 費用の欄が無い usage は `cost = None`（0 と見せない）。
    #[test]
    fn test_record_execution_usage_cost_unknown_is_none() {
        let dir = tempfile::tempdir().unwrap();
        write_session_no_cost(dir.path());
        let store = SaiStore::open_in_memory().unwrap();
        let exec_id = setup_exec(&store, dir.path());

        let total = record_execution_usage(&store, exec_id).unwrap().unwrap();
        assert_eq!(total.input, 100);
        assert_eq!(total.output, 20);
        assert_eq!(total.cost, None);
    }

    /// usage_with_children: 親 1 + 子 2 の合計と、費用不明の数え上げ。
    #[test]
    fn test_usage_with_children_totals_and_unknown_cost() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = test_block_id(&store, "usage-children");
        let parent = store.create_execution(block_id, None).unwrap();
        let child1 = store
            .create_execution_for_subtask(block_id, None, parent, None)
            .unwrap();
        let child2 = store
            .create_execution_for_subtask(block_id, None, parent, None)
            .unwrap();

        // 親: 費用あり / 子1: 費用あり / 子2: トークンはあるが費用 0（不明）
        store
            .replace_execution_usage(parent, &[total_row(parent, 100, 10, 5, 0, 0.5)])
            .unwrap();
        store
            .replace_execution_usage(child1, &[total_row(child1, 200, 20, 0, 10, 0.3)])
            .unwrap();
        store
            .replace_execution_usage(child2, &[total_row(child2, 300, 30, 0, 0, 0.0)])
            .unwrap();

        let summary = usage_with_children(&store, parent).unwrap();
        assert_eq!(summary.own.as_ref().unwrap().cost, Some(0.5));
        assert_eq!(summary.children.len(), 2);
        assert_eq!(summary.unknown_cost_runs, 1);
        // token は 3 実行の合計
        assert_eq!(summary.total.input, 600);
        assert_eq!(summary.total.output, 60);
        assert_eq!(summary.total.cache, 15);
        // 費用は分かった分（0.5 + 0.3）だけを合計する
        assert_eq!(summary.total.cost, Some(0.8));
    }

    /// usage_with_children: usage 未保存の実行は `own = None`、合計はゼロ。
    #[test]
    fn test_usage_with_children_without_usage() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = test_block_id(&store, "usage-children-empty");
        let parent = store.create_execution(block_id, None).unwrap();
        let summary = usage_with_children(&store, parent).unwrap();
        assert!(summary.own.is_none());
        assert!(summary.children.is_empty());
        assert_eq!(summary.unknown_cost_runs, 0);
        assert_eq!(summary.total, UsageTotal::default());
    }
}
