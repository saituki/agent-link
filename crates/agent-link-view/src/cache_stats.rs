//! Stats の表示データ（キャッシュ / トークン統計）の組み立て。
//!
//! `sai-cache-tui-rs` の解析ロジックを流用し、`phone run --view stats`
//! （Web の中継）が使うデータと JSON 表現を提供する。描画は持たない。
//!
//! 実行（ブロック）の `agent_session_id` からセッション jsonl を解決し、
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
/// 実行レコードの `agent_session_id` から `phone_export::resolve_transcript` で
/// セッション jsonl を解決し、sai-cache-tui-rs と同じ解析（usage 集計）を行う。
///
/// - `store`: 対象プロジェクトの `SaiStore`（read-only でも可）
/// - `exec_id`: 対象実行 ID
///
/// セッションが無い・解析に失敗した場合は `error` にメッセージをセットする。
pub fn load(store: &agent_link_engine::db::SaiStore, exec_id: i64) -> CacheViewState {
    // 実行レコードの agent_session_id から会話を解決する（別の run の会話を拾わない）。
    let exec = match store.get_execution_by_id(exec_id) {
        Ok(e) => e,
        Err(_) => {
            return CacheViewState {
                error: Some(format!("Stats: run record not found (ID{})", exec_id)),
                ..Default::default()
            }
        }
    };
    let root = agent_link_engine::project::project_root();
    let session_file = agent_link_engine::phone_export::resolve_transcript(&exec, &root);

    let Some(session_file) = session_file else {
        return CacheViewState {
            error: Some(format!(
                "Stats: session jsonl not found (ID{})",
                exec_id
            )),
            ..Default::default()
        };
    };

    // sai-cache-tui-rs と同じ解析を実行する
    // cc（Claude Code）: usage フィールドが snake_case
    let (metrics, counts) = match (
        collect_metrics_from_file_cc(&session_file),
        collect_input_counts(&session_file),
    ) {
        (Ok(metrics), Ok(counts)) => (metrics, counts),
        (Err(e), _) | (_, Err(e)) => {
            return CacheViewState {
                error: Some(format!("Stats: cc parse error: {}", e)),
                ..Default::default()
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

#[cfg(test)]
mod tests {
    use super::*;
    use agent_link_engine::db::{BlockMetadata, SaiStore};

    /// `load` は run の `agent_session_id` の jsonl を読む。
    /// 別 id のより新しい jsonl があっても、それを拾わない。
    #[test]
    fn load_reads_own_session_not_newest() {
        let _env = crate::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_home = std::env::var_os("HOME");
        let prev_root = std::env::var_os("AGENT_LINK_PROJECT_ROOT");
        let tmp = std::env::temp_dir().join(format!("sai_stats_load_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("proj");
        std::fs::create_dir_all(&root).unwrap();
        std::env::set_var("HOME", &tmp);
        std::env::set_var("AGENT_LINK_PROJECT_ROOT", &root);

        let slug = agent_link_engine::session::claude_project_slug(&root);
        let dir = tmp.join(".claude/projects").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        let own = dir.join("own-sid.jsonl");
        let newer = dir.join("other-sid.jsonl");
        std::fs::write(&own, "").unwrap();
        std::fs::write(&newer, "").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options().write(true).open(&own).unwrap().set_modified(old).unwrap();

        let store = SaiStore::open_in_memory().unwrap();
        let block = BlockMetadata::new(
            "b".to_string(),
            "b".to_string(),
            "m".to_string(),
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            "blocks/b.md".to_string(),
            None,
        );
        let block_id = store.insert_block(&block).unwrap();
        let run_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_agent_session_id(run_id, "own-sid").unwrap();

        let state = load(&store, run_id);

        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match prev_root {
            Some(v) => std::env::set_var("AGENT_LINK_PROJECT_ROOT", v),
            None => std::env::remove_var("AGENT_LINK_PROJECT_ROOT"),
        }
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(state.error, None);
        assert_eq!(state.session_file, Some(own));
    }
}
