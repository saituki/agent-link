//! History の表示データ（Token Usage by Category・Timeline）の組み立て。
//!
//! 描画は持たない。`phone run --view history`（Web の中継）が使う
//! データと JSON 表現だけを提供する。
//!
//! - **近似値のみ使用する**: token はセッション jsonl の文字数換算（近似）であり、
//!   provider の実際の usage（`execution_usage`）は使わない（Project 画面のみ正確値）
//! - 種別: `user` / `agent` / `toolCall` / `toolResult` / `thinking`

use sai_link_engine::session::{HistoryEvent, HistoryTypeTokens};

/// History 画面の種別（表示順）。
pub const HISTORY_TYPES: [&str; 5] = ["user", "agent", "toolCall", "toolResult", "thinking"];

// ============================================================================
// 状態
// ============================================================================

/// History の表示データ（`build` が構築する）。
#[derive(Debug, Default)]
pub struct HistoryViewData {
    /// タイトル（例: `Session #3`）
    pub title: String,
    /// サブタイトル（例: `block 名 | started: ... | status: ...`）
    pub subtitle: String,
    /// 種別ごとの近似トークン合計（文字数換算）
    pub type_tokens: HistoryTypeTokens,
    /// Timeline イベント（時刻順）
    pub events: Vec<HistoryEvent>,
}

impl HistoryViewData {
    /// 種別の近似トークン合計を取得する。
    pub fn tokens_of(&self, kind: &str) -> u64 {
        match kind {
            "user" => self.type_tokens.user,
            "agent" => self.type_tokens.agent,
            "toolCall" => self.type_tokens.tool_call,
            "toolResult" => self.type_tokens.tool_result,
            "thinking" => self.type_tokens.thinking,
            _ => 0,
        }
    }

    /// `phone run --view history` 用の JSON 表現を返す（実装計画 12 §3.2）。
    ///
    /// 項目名は仕様のとおり（`toolCall` / `toolResult` / `time` / `type`）。
    pub fn phone_json(&self) -> PhoneHistoryJson<'_> {
        PhoneHistoryJson {
            title: &self.title,
            subtitle: &self.subtitle,
            type_tokens: PhoneTypeTokens {
                user: self.type_tokens.user,
                agent: self.type_tokens.agent,
                tool_call: self.type_tokens.tool_call,
                tool_result: self.type_tokens.tool_result,
                thinking: self.type_tokens.thinking,
            },
            events: self
                .events
                .iter()
                .map(|e| PhoneEvent {
                    timestamp: &e.timestamp,
                    kind: &e.kind,
                    name: &e.name,
                    tokens: e.tokens,
                })
                .collect(),
        }
    }
}

/// `phone run --view history` の JSON 形（実装計画 12 §3.2）。
#[derive(serde::Serialize)]
pub struct PhoneHistoryJson<'a> {
    /// タイトル（例: `Session #25`）
    pub title: &'a str,
    /// サブタイトル（`block 名 | started: … | status: …`）
    pub subtitle: &'a str,
    /// 種別ごとの近似トークン合計
    pub type_tokens: PhoneTypeTokens,
    /// Timeline イベント（時刻順）
    pub events: Vec<PhoneEvent<'a>>,
}

/// `type_tokens`（項目名は仕様どおり camelCase の `toolCall` / `toolResult`）。
#[derive(serde::Serialize)]
pub struct PhoneTypeTokens {
    /// user message の text
    pub user: u64,
    /// assistant message の text
    pub agent: u64,
    /// toolCall（名前 + 引数）
    #[serde(rename = "toolCall")]
    pub tool_call: u64,
    /// toolResult の text
    #[serde(rename = "toolResult")]
    pub tool_result: u64,
    /// thinking
    pub thinking: u64,
}

/// Timeline イベント 1 件（`time` / `type` リネーム付き）。
#[derive(serde::Serialize)]
pub struct PhoneEvent<'a> {
    /// ISO 8601 タイムスタンプ
    #[serde(rename = "time")]
    pub timestamp: &'a str,
    /// 種別: `user` / `agent` / `toolCall` / `toolResult` / `thinking`
    #[serde(rename = "type")]
    pub kind: &'a str,
    /// 名前（ツール名・本文冒頭など）
    pub name: &'a str,
    /// 近似トークン数
    pub tokens: u64,
}

// ============================================================================
// ビルダ純関数（DB → 表示データ変換）
// ============================================================================

/// 実行 1 件分の History の表示データを DB から構築する（ビルダ純関数）。
///
/// **近似値のみ使用する**: token はセッション jsonl の文字数換算（近似）であり、
/// provider の実際の usage（`execution_usage`）は使わない。
///
/// - `store`: 対象プロジェクトの `SaiStore`（read-only でも可）
/// - `exec_id`: 対象実行 ID
///
/// 実行レコードが見つからない場合は `None`（呼び出し側でメッセージ表示する）。
/// セッション（`session_dir` 未記録）が無い場合は type_tokens / events が空の
/// データを返す。
pub fn build(store: &sai_link_engine::db::SaiStore, exec_id: i64) -> Option<HistoryViewData> {
    let exec = store.get_execution_by_id(exec_id).ok()?;
    let block_name = store
        .get_block_by_id(exec.block_id)
        .map(|b| b.display_name)
        .unwrap_or_else(|_| format!("block#{}", exec.block_id));
    let start_time = exec.started_at.as_deref().unwrap_or("unknown");

    // 近似統計（種別別トークン + Timeline イベント）を jsonl 走査 1 回で取得する。
    // DB の旧パスが残っていても outputs 側へフォールバックする。
    // runner=gl は `~/.sai-gemini/sessions/` の最新セッション（空でない最初の1件）。
    // runner=cc は `~/.claude/projects/` の最新セッション。
    let (type_tokens, events) = if exec.runner == "gl" {
        if let Some(st) = sai_link_engine::session::gemini_session_files(30)
            .into_iter()
            .find_map(|path| {
                sai_link_engine::session::extract_gemini_history_stats(&path)
                    .ok()
                    .filter(|st| !st.events.is_empty())
            })
        {
            (st.type_tokens, st.events)
        } else {
            Default::default()
        }
    } else if exec.runner == "cc" {
        if let Some(st) = sai_link_engine::session::cc_session_files(30)
            .into_iter()
            .find_map(|path| {
                sai_link_engine::session::extract_cc_history_stats(&path)
                    .ok()
                    .filter(|st| !st.events.is_empty())
            })
        {
            (st.type_tokens, st.events)
        } else {
            Default::default()
        }
    } else {
        sai_link_engine::session::resolve_session_dir(exec.session_dir.as_deref(), exec_id)
            .and_then(|dir| sai_link_engine::session::extract_history_stats(&dir).ok())
            .map(|s| (s.type_tokens, s.events))
            .unwrap_or_default()
    };

    Some(HistoryViewData {
        title: format!("Session #{}", exec_id),
        subtitle: format!(
            "{} | started: {} | status: {}",
            block_name, start_time, exec.status
        ),
        type_tokens,
        events,
    })
}
