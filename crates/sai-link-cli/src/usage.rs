//! # `sai-link usage` サブコマンド — 実行 usage（L1）の管理
//!
//! provider が返す実際の token / cost を `execution_usage` テーブルへ保存する。
//! 保存ロジックは engine（`sai_link_engine::usage::backfill_execution_usage`）に
//! 一本化されており、このモジュールは CLI 引数の受付と進捗表示のみを担う。
//!
//! ## 使用例
//!
//! ```bash
//! # 既存実行分の usage を再計算して保存（冪等）
//! sai-link usage backfill
//!
//! # 保存せず集計結果のみ確認
//! sai-link usage backfill --dry-run
//! ```

use sai_link_engine::db::SaiStore;
use sai_link_engine::usage::backfill_execution_usage;

/// `sai-link usage backfill` の引数。
#[derive(Debug, Clone, Default)]
pub struct BackfillArgs {
    /// 保存せず集計結果のみ表示する
    pub dry_run: bool,
}

/// `sai-link usage backfill` の実行本体。
///
/// 全実行を対象に jsonl を走査し、`execution_usage` へ全置換保存する（冪等）。
/// 実行単位でコミットするため、中断後の再開も安全である。
pub fn run_usage_backfill(args: BackfillArgs) -> Result<(), Box<dyn std::error::Error>> {
    // DB を開く（`search` と同じパス解決規則）
    let db_path = sai_link_engine::project::sai_path(".sai/sai-engine/sai-engine.db");
    let store = match SaiStore::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("エラー: DB を開けませんでした: {}", db_path.display());
            eprintln!("  {e}");
            std::process::exit(1);
        }
    };

    if args.dry_run {
        println!("dry-run: 保存せず集計結果のみ表示します");
    }
    let mut progress = |msg: &str| println!("{msg}");
    let report = backfill_execution_usage(&store, args.dry_run, &mut progress)?;

    let mode = if args.dry_run {
        "保存対象"
    } else {
        "保存済み"
    };
    println!(
        "--- 完了: 対象 {} 件 / {} {} 件 / スキップ {} 件{}",
        report.scanned,
        mode,
        report.updated,
        report.skipped,
        if report.dry_run {
            "（dry-run・未保存）"
        } else {
            ""
        }
    );
    Ok(())
}
