//! Stats の表示データ（キャッシュ / トークン統計）の組み立て。
//!
//! `sai-cache-tui-rs` の解析ロジックを流用し、`phone run --view stats`
//! （Web の中継）が使うデータと JSON 表現を提供する。描画は持たない。
//!
//! 実行（ブロック）の `session_dir` からメインセッション jsonl を解決し、
//! usage を集計して [`CacheViewState`] に詰める（[`load`]）。

use std::path::PathBuf;

// ============================================================================
// 集計ロジック（engine へ移設・01 §2.1）
// ============================================================================
//
// トークン / コストの集計ロジックと型は `agent_link_engine::usage_metrics` に
// 移設した（デーモン＝engine から使うため）。ここでは再公開のみ行い、
// 呼び出しと数字は変更しない。

pub use agent_link_engine::usage_metrics::*;

// ============================================================================
// ビュー状態
// ============================================================================

/// Stats の表示データ。
#[derive(Debug, Clone, Default)]
pub struct CacheViewState {
    /// 解析対象のセッション jsonl パス
    pub session_file: Option<PathBuf>,
    /// 集計済みメトリクス（読み込み失敗時は None）
    pub metrics: Option<CacheSessionMetrics>,
    /// 累積系列（`metrics` と同時に更新）
    pub cumulative: CumulativeSeries,
    /// メッセージ数カウント
    pub counts: SessionCounts,
    /// 読み込みエラーメッセージ（None = 正常）
    pub error: Option<String>,
}

/// `phone run --view stats` 用の JSON 化（実装計画 12 §3.2）。
///
/// `metrics` が無い（解析失敗・セッション無し）項目は `null`。
#[derive(serde::Serialize)]
pub struct PhoneStatsJson<'a> {
    /// 解析対象のセッション jsonl パス（無ければ null）
    pub session_file: Option<String>,
    /// 読み込みエラー（無ければ null）
    pub error: Option<String>,
    /// ツリー全体の合計（無ければ null）
    pub totals: Option<&'a agent_link_engine::usage_metrics::CacheUsageTotals>,
    /// アクティブブランチの合計（無ければ null）
    pub active_totals: Option<&'a agent_link_engine::usage_metrics::CacheUsageTotals>,
    /// assistant メッセージごとの使用量（無ければ null）
    pub messages: Option<&'a [agent_link_engine::usage_metrics::AssistantUsageMetric]>,
    /// 累積系列（チャート用）
    pub cumulative: &'a agent_link_engine::usage_metrics::CumulativeSeries,
    /// セッション内容のメッセージ数カウント
    pub counts: &'a agent_link_engine::usage_metrics::SessionCounts,
}

impl CacheViewState {
    /// `phone run --view stats` 用の JSON 表現を返す。
    pub fn phone_json(&self) -> PhoneStatsJson<'_> {
        PhoneStatsJson {
            session_file: self
                .session_file
                .as_ref()
                .map(|p| p.display().to_string()),
            error: self.error.clone(),
            totals: self.metrics.as_ref().map(|m| &m.tree_totals),
            active_totals: self.metrics.as_ref().map(|m| &m.active_branch_totals),
            messages: self.metrics.as_ref().map(|m| m.all_messages.as_slice()),
            cumulative: &self.cumulative,
            counts: &self.counts,
        }
    }
}

/// 実行 1 件分の Stats ビュー状態を DB から構築する（ビルダ純関数）。
///
/// 実行レコードの `session_dir` から `find_main_session_file` でメインセッション
/// jsonl を解決し、sai-cache-tui-rs と同じ解析（usage 集計）を行う。
///
/// - `store`: 対象プロジェクトの `SaiStore`（read-only でも可）
/// - `exec_id`: 対象実行 ID
///
/// セッションが無い・解析に失敗した場合は `error` にメッセージをセットする。
pub fn load(store: &agent_link_engine::db::SaiStore, exec_id: i64) -> CacheViewState {
    // 実行レコードから runner と session_dir を取得する。
    // runner=gl: ~/.agent-link-gemini/sessions/
    // runner=cc: ~/.claude/projects/
    // その他: DB の resolve_session_dir
    let Some(exec) = store.get_execution_by_id(exec_id).ok() else {
        return CacheViewState {
            error: Some(format!("Stats: 実行レコードが見つかりません (ID{})", exec_id)),
            ..Default::default()
        };
    };

    let session_file = if exec.runner == "gl" {
        agent_link_engine::session::gemini_session_files(30)
            .into_iter()
            .next()
    } else if exec.runner == "cc" {
        agent_link_engine::session::cc_session_files(30)
            .into_iter()
            .next()
    } else {
        agent_link_engine::session::resolve_session_dir(
            exec.session_dir.as_deref(),
            exec_id,
        )
        .and_then(|dir| agent_link_engine::session::find_main_session_file(&dir).ok())
    };

    let Some(session_file) = session_file else {
        return CacheViewState {
            error: Some(format!(
                "Stats: セッション jsonl が見つかりません (ID{})",
                exec_id
            )),
            ..Default::default()
        };
    };

    // sai-cache-tui-rs と同じ解析を実行する
    let (metrics, counts) = if exec.runner == "cc" {
        // cc（Claude Code）: usage フィールドが snake_case
        match (
            collect_metrics_from_file_cc(&session_file),
            collect_input_counts(&session_file),
        ) {
            (Ok(metrics), Ok(counts)) => (metrics, counts),
            (Err(e), _) | (_, Err(e)) => {
                return CacheViewState {
                    error: Some(format!("Stats: cc 解析エラー: {}", e)),
                    ..Default::default()
                }
            }
        }
    } else {
        // pi / gl: 既存の collect_metrics_from_file
        match (
            collect_metrics_from_file(&session_file),
            collect_input_counts(&session_file),
        ) {
            (Ok(metrics), Ok(counts)) => (metrics, counts),
            (Err(e), _) | (_, Err(e)) => {
                return CacheViewState {
                    error: Some(format!("Stats: 解析エラー: {}", e)),
                    ..Default::default()
                }
            }
        }
    };

    let cumulative = compute_cumulative_series(&metrics.all_messages);
    CacheViewState {
        session_file: Some(session_file),
        metrics: Some(metrics),
        cumulative,
        counts,
        error: None,
    }
}
