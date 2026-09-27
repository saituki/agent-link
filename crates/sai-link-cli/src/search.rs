//! # `sai-link search` サブコマンド — セッション履歴検索
//!
//! 検索ロジックは engine（`sai_link_engine::search`）に一本化されており、
//! このモジュールは CLI 引数の受付と結果出力のみを担う。
//! 仕様書: `docs/spec-session-search.md`
//!
//! ## 使用例
//!
//! ```bash
//! # user メッセージから「llm」を検索
//! sai-link search --type user -w llm
//!
//! # ブロック ID 1〜5・Round 1・2026/08 中の実行を対象に AND 検索
//! sai-link search --id 1-5 -R 1 --date 2026/08/01-2026/08/31 -w llm,sai
//!
//! # JSON 出力
//! sai-link search -w 移植 --json
//! ```

use sai_link_engine::search::{
    parse_query, run_search_projects, run_search_verbose, ProjectSource, RawSearchForm, SearchError,
};

// ============================================================================
// 引数構造体
// ============================================================================

/// `sai-link search` の引数（`cli.rs` の `Cmd::Search` から生成する）。
#[derive(Debug, Clone, Default)]
pub struct SearchArgs {
    /// Round（グローバルラウンド）
    pub global_round: Option<String>,
    /// round（実行ラウンド）
    pub round: Option<String>,
    /// ブロック ID 範囲
    pub id: Option<String>,
    /// ブロック名部分一致
    pub block: Option<String>,
    /// 実行日
    pub date: Option<String>,
    /// ステータス絞り込み（カンマ区切り・R304。空 = todo 除外）
    pub status: Option<String>,
    /// 種別（カンマ区切り）
    pub r#type: Option<String>,
    /// 検索語（カンマ区切り）
    pub word: Option<String>,
    /// プロジェクト名（結果の出自表示。自プロジェクト内検索のため表示名のみ・テスト用）
    pub project: Option<String>,
    /// JSON 出力
    pub json: bool,
}

impl SearchArgs {
    /// 引数を生入力フォーム（engine の `RawSearchForm`）に変換する。
    fn to_raw_form(&self) -> RawSearchForm {
        RawSearchForm {
            round_gr: self.global_round.clone().unwrap_or_default(),
            round: self.round.clone().unwrap_or_default(),
            id: self.id.clone().unwrap_or_default(),
            block: self.block.clone().unwrap_or_default(),
            date: self.date.clone().unwrap_or_default(),
            status: self.status.clone().unwrap_or_default(),
            type_: self.r#type.clone().unwrap_or_default(),
            word: self.word.clone().unwrap_or_default(),
        }
    }
}

// ============================================================================
// エントリポイント
// ============================================================================

/// `sai-link search` の実行本体。
///
/// 引数を engine の `parse_query` → `run_search_verbose` に渡し、
/// 結果を標準出力へ出力する。書式エラー時は終了コード 2、
/// DB / I/O エラー時は終了コード 1 で終了する。
pub fn run_search_command(args: SearchArgs) -> Result<(), Box<dyn std::error::Error>> {
    // 検索条件の解析（書式検証）
    let raw = args.to_raw_form();
    let query = match parse_query(&raw) {
        Ok(q) => q,
        // 書式エラーは終了コード 2
        Err(SearchError::Parse(msg)) => {
            eprintln!("エラー: {msg}");
            std::process::exit(2);
        }
        Err(e) => return Err(e.into()),
    };

    // DB を開く（`block list` と同じパス解決規則）
    let db_path = sai_link_engine::project::sai_path(".sai/sai-engine/sai-engine.db");

    // `--project` 指定時は自プロジェクト名付きの ProjectSource 1 件として
    // 複数プロジェクト検索関数（L2）経由で実行する（出自表示・テスト用）
    let (hits, skipped) = if let Some(name) = args.project.as_deref().filter(|s| !s.is_empty()) {
        let projects = vec![ProjectSource {
            name: name.to_string(),
            root: sai_link_engine::project::project_root(),
        }];
        match run_search_projects(&projects, &query) {
            Ok(v) => (v, 0usize),
            Err(SearchError::Parse(msg)) => {
                eprintln!("エラー: {msg}");
                std::process::exit(2);
            }
            Err(e) => return Err(e.into()),
        }
    } else {
        let store = match sai_link_engine::db::SaiStore::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("エラー: DB を開けませんでした: {}", db_path.display());
                eprintln!("  {e}");
                std::process::exit(1);
            }
        };
        match run_search_verbose(&store, &query) {
            Ok(v) => v,
            // 書式エラーはここでは発生しないが、念のため終了コード 2 に振り分ける
            Err(SearchError::Parse(msg)) => {
                eprintln!("エラー: {msg}");
                std::process::exit(2);
            }
            Err(e) => return Err(e.into()),
        }
    };

    // 出力
    if args.json {
        println!("{}", serde_json::to_string_pretty(&hits)?);
    } else {
        print_text_output(&hits, skipped);
    }
    Ok(())
}

// ============================================================================
// 出力
// ============================================================================

/// テキスト出力（仕様書 §6 の結果行書式）。
///
/// ```text
/// ID<N> | <type> | <turn> | <マッチした content の一部>
/// ```
fn print_text_output(hits: &[sai_link_engine::search::SearchHit], skipped: usize) {
    if hits.is_empty() {
        println!("検索結果: 0 件");
        if skipped > 0 {
            println!("（セッション不在のため読み飛ばした実行: {skipped} 件）");
        }
        return;
    }
    for h in hits {
        // `--project` 指定時（出自プロジェクト付き）は先頭カラムに表示する
        if h.project.is_empty() {
            println!(
                "ID{} | {} | {} | {}",
                h.block_id,
                h.type_.as_str(),
                h.turn,
                h.snippet
            );
        } else {
            println!(
                "{} | ID{} | {} | {} | {}",
                h.project,
                h.block_id,
                h.type_.as_str(),
                h.turn,
                h.snippet
            );
        }
    }
    println!(
        "--- {} 件{}",
        hits.len(),
        if skipped > 0 {
            format!("（読み飛ばし {skipped} 件）")
        } else {
            String::new()
        }
    );
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_raw_form() {
        let args = SearchArgs {
            global_round: Some("1".into()),
            word: Some("llm, sai".into()),
            json: true,
            ..Default::default()
        };
        let raw = args.to_raw_form();
        assert_eq!(raw.round_gr, "1");
        assert_eq!(raw.word, "llm, sai");
        assert_eq!(raw.round, "");
        assert_eq!(raw.status, "");
        assert_eq!(raw.type_, "");
    }

    /// 引数 → parse_query までの通し確認（書式エラーの伝播）。
    #[test]
    fn test_args_parse_query() {
        let args = SearchArgs {
            global_round: Some("1-5".into()),
            r#type: Some("user,tool".into()),
            word: Some("LLM".into()),
            ..Default::default()
        };
        let q = parse_query(&args.to_raw_form()).unwrap();
        assert_eq!(q.types.len(), 4); // user + tool 3 種
        assert_eq!(q.words, vec!["llm"]);

        let bad = SearchArgs {
            global_round: Some("x".into()),
            ..Default::default()
        };
        assert!(matches!(
            parse_query(&bad.to_raw_form()),
            Err(SearchError::Parse(_))
        ));
    }
}
