//! History の表示データ（Token Usage by Category・Timeline）の組み立て。
//!
//! 描画は持たない。`phone run --view history`（Web の中継）が使う
//! データと JSON 表現だけを提供する。
//!
//! - **token は近似値を含む**: cc の assistant は usage の実値を使う。user・agent・thinking・
//!   道具の結果は文字数換算の近似である。provider の実際の usage（`execution_usage`）は使わない
//!   （Project 画面のみ正確値）
//! - 種別: `user` / `agent` / `toolCall` / `toolResult` / `thinking`
//! - `cli`（C11）: 道具の呼び出し（`sai-paper`・`sai-sci`・`sai-link`）と台帳の集計

use std::collections::{BTreeMap, BTreeSet};

use sai_link_engine::session::{HistoryEvent, HistoryTypeTokens};

/// History 画面の種別（表示順）。
pub const HISTORY_TYPES: [&str; 5] = ["user", "agent", "toolCall", "toolResult", "thinking"];

/// 失敗の分類（表示順・C11）。
const FAILURE_CLASSES: [&str; 5] = ["ledger", "input", "not_found", "conflict", "other"];

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
    /// CLI の呼び出しと台帳の集計（C11）
    pub cli: CliSummary,
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
                    call_id: e.call_id.as_deref(),
                    command: e.command.as_deref(),
                    preview: e.preview.as_deref(),
                    is_error: e.is_error,
                    ids: e.ids.iter().map(String::as_str).collect(),
                    result_tokens: e.result_tokens,
                })
                .collect(),
            cli: &self.cli,
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
    /// CLI の呼び出しと台帳の集計（C11）
    pub cli: &'a CliSummary,
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

/// Timeline イベント 1 件（`time` / `type` リネーム付き・C11 の項目）。
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
    /// 呼び出しの番号
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<&'a str>,
    /// Bash 系の道具の命令（`sai-paper content read` など）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<&'a str>,
    /// 結果の本文の先頭 200 字
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<&'a str>,
    /// 結果が失敗か
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// 呼び出しの入力に出る番号（`#x.y.z`）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<&'a str>,
    /// 結果の文字数による近似
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_tokens: Option<u64>,
}

/// `cli`（C11）: CLI の呼び出しと台帳の集計。
#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub struct CliSummary {
    /// 命令ごとの呼び出し数（多い順・同数は命令名の順）
    pub calls: Vec<CliCallCount>,
    /// 失敗の分類ごとの数（`ledger`・`input`・`not_found`・`conflict`・`other` の順・0 は出さない）
    pub failures: Vec<CliFailureCount>,
    /// 台帳の番号と、使った番号
    pub ids: CliIdCount,
}

/// `cli.calls` の 1 件。
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct CliCallCount {
    /// 命令名（`sai-paper content read` など）
    pub command: String,
    /// 呼び出し数（`--help` を除く）
    pub count: u64,
    /// そのうち失敗（`is_error` が true）の数（`--help` を除く）
    pub errors: u64,
    /// `--help`・`-h` で呼んだ回数（`count` に入らない）
    pub help: u64,
}

/// `cli.failures` の 1 件（分類と命令の組ごとの件数）。
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct CliFailureCount {
    /// 失敗の分類
    pub class: &'static str,
    /// 命令名。命令が読めない `sai-*` の呼び出しは `(不明)`
    pub command: String,
    /// 件数
    pub count: u64,
}

/// `cli.ids`: `seen` は台帳の異なる番号、`used` は台帳にあって呼び出しの入力に出た番号、
/// `unused` は台帳にあって使わなかった番号の数（`used + unused == seen`）。
#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub struct CliIdCount {
    /// 台帳（見せた番号）の数
    pub seen: u64,
    /// 使った番号の数（台帳にある番号だけ）
    pub used: u64,
    /// 見せたのに使わなかった番号の数
    pub unused: u64,
}

// ============================================================================
// ビルダ純関数（DB → 表示データ変換）
// ============================================================================

/// 実行 1 件分の History の表示データを DB から構築する（ビルダ純関数）。
///
/// token は cc の assistant の usage の実値と、文字数換算の近似を含む。provider の
/// 実際の usage（`execution_usage`）は使わない。
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
    let root = sai_link_engine::project::project_root();

    // 近似統計（種別別トークン + Timeline イベント）を jsonl 走査 1 回で取得する。
    // DB の旧パスが残っていても outputs 側へフォールバックする。
    // runner=gl は `~/.sai-gemini/sessions/` の最新セッション（空でない最初の1件）。
    // runner=cc は この run の transcript（全体で一番新しいセッションは使わない）。
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
        sai_link_engine::phone_export::resolve_transcript(&exec, &root)
            .and_then(|path| sai_link_engine::session::extract_cc_history_stats(&path).ok())
            .map(|s| (s.type_tokens, s.events))
            .unwrap_or_default()
    } else {
        sai_link_engine::session::resolve_session_dir(exec.session_dir.as_deref(), exec_id)
            .and_then(|dir| sai_link_engine::session::extract_history_stats(&dir).ok())
            .map(|s| (s.type_tokens, s.events))
            .unwrap_or_default()
    };

    let cli = cli_summary(
        &events,
        &sai_link_engine::seen_ids::read_all(&root, exec_id),
    );

    Some(HistoryViewData {
        title: format!("Session #{}", exec_id),
        subtitle: format!(
            "{} | started: {} | status: {}",
            block_name, start_time, exec.status
        ),
        type_tokens,
        events,
        cli,
    })
}

/// 呼び出しの行と台帳から `cli` の集計を作る（純関数・C11）。
///
/// `ledger` は台帳（`seen_ids::read_all`）の番号の集合。`events` の `command`・`is_error`・
/// `failure`・`ids` から数える。
pub fn cli_summary(events: &[HistoryEvent], ledger: &BTreeSet<String>) -> CliSummary {
    /// 命令ごとの集計（`--help` を除く）。
    #[derive(Default)]
    struct Tally {
        count: u64,
        errors: u64,
        help: u64,
    }
    let mut calls: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut failures: BTreeMap<(&'static str, String), u64> = BTreeMap::new();
    let mut used: BTreeSet<&str> = BTreeSet::new();

    for e in events {
        used.extend(e.ids.iter().map(String::as_str));
        // `--help` は呼び出しに数えない（help だけ増やし、失敗にも数えない）
        if e.help {
            if let Some(command) = &e.command {
                calls.entry(command.as_str()).or_default().help += 1;
            }
            continue;
        }
        if let Some(command) = &e.command {
            let c = calls.entry(command.as_str()).or_default();
            c.count += 1;
            if e.is_error == Some(true) {
                c.errors += 1;
            }
        }
        // 失敗は CLI（`sai-*`）の呼び出しだけ数える。命令が読めなくても入力に `sai-*` があれば
        // `(不明)` で数える（U04）。`Edit` など他の道具の失敗は数えない。
        if let (Some(true), Some(class)) = (e.is_error, e.failure) {
            if e.command.is_some() || e.cli_mention {
                let command = e.command.clone().unwrap_or_else(|| "(不明)".to_string());
                *failures.entry((class, command)).or_default() += 1;
            }
        }
    }

    let mut calls: Vec<CliCallCount> = calls
        .into_iter()
        .map(|(command, t)| CliCallCount {
            command: command.to_string(),
            count: t.count,
            errors: t.errors,
            help: t.help,
        })
        .collect();
    calls.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.command.cmp(&b.command))
    });

    let mut failures: Vec<CliFailureCount> = failures
        .into_iter()
        .map(|((class, command), count)| CliFailureCount {
            class,
            command,
            count,
        })
        .collect();
    failures.sort_by_key(|f| {
        (
            FAILURE_CLASSES.iter().position(|c| *c == f.class),
            f.command.clone(),
        )
    });

    // 使った番号は台帳にある番号だけ数える（`used + unused == seen`）
    let used_in_ledger = ledger
        .iter()
        .filter(|id| used.contains(id.as_str()))
        .count() as u64;
    let seen = ledger.len() as u64;
    CliSummary {
        calls,
        failures,
        ids: CliIdCount {
            seen,
            used: used_in_ledger,
            unused: seen - used_in_ledger,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sai_link_engine::session::failure_class;

    fn event(
        command: Option<&str>,
        is_error: Option<bool>,
        failure: Option<&'static str>,
        ids: &[&str],
    ) -> HistoryEvent {
        HistoryEvent {
            kind: "toolCall".to_string(),
            tokens: 1,
            command: command.map(String::from),
            is_error,
            failure,
            ids: ids.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    /// 見本の events と台帳から `calls`・`failures`・`ids` を数えること（C11）。
    #[test]
    fn cli_summary_counts() {
        let events = vec![
            event(Some("sai-paper content read"), Some(false), None, &["#37"]),
            event(
                Some("sai-paper content read"),
                Some(true),
                Some(failure_class("台帳に無い番号です: #37.5.2")),
                &["#37.5.2"],
            ),
            event(
                Some("sai-sci op"),
                Some(true),
                Some(failure_class("Exit code 3\n見つからない")),
                &[],
            ),
            event(
                Some("sai-sci op"),
                Some(true),
                Some(failure_class("Exit code 1\n何か")),
                &["#37"],
            ),
            // 命令でない行（user・agent など）は calls に入らない
            event(None, None, None, &[]),
            // `sai-*` 以外の道具（Edit など）の失敗は、calls にも failures にも入らない
            event(None, Some(true), Some(failure_class("Exit code 1\n何か")), &[]),
            // `--help` は呼び出しに数えず、失敗にも数えない（help だけ増える）
            HistoryEvent {
                command: Some("sai-link run finalize".into()),
                is_error: Some(true),
                failure: Some("other"),
                help: true,
                ..event(None, None, None, &[])
            },
            // 命令が読めない `sai-*` の失敗は `(不明)` で数える（U04）
            HistoryEvent {
                cli_mention: true,
                is_error: Some(true),
                failure: Some("other"),
                ..event(None, None, None, &[])
            },
        ];
        let ledger: BTreeSet<String> = ["#37", "#40", "#41"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let got = cli_summary(&events, &ledger);
        assert_eq!(
            got,
            CliSummary {
                calls: vec![
                    CliCallCount {
                        command: "sai-paper content read".into(),
                        count: 2,
                        errors: 1,
                        help: 0,
                    },
                    CliCallCount {
                        command: "sai-sci op".into(),
                        count: 2,
                        errors: 2,
                        help: 0,
                    },
                    CliCallCount {
                        command: "sai-link run finalize".into(),
                        count: 0,
                        errors: 0,
                        help: 1,
                    },
                ],
                failures: vec![
                    CliFailureCount {
                        class: "ledger",
                        command: "sai-paper content read".into(),
                        count: 1
                    },
                    CliFailureCount {
                        class: "not_found",
                        command: "sai-sci op".into(),
                        count: 1
                    },
                    CliFailureCount {
                        class: "other",
                        command: "(不明)".into(),
                        count: 1
                    },
                    CliFailureCount {
                        class: "other",
                        command: "sai-sci op".into(),
                        count: 1
                    },
                ],
                // 台帳は #37・#40・#41。使ったのは #37 と #37.5.2（台帳に無い）。
                // 台帳にあって使ったのは #37 だけ。使わなかったのは #40・#41
                ids: CliIdCount {
                    seen: 3,
                    used: 1,
                    unused: 2
                },
            }
        );
        assert_eq!(got.ids.used + got.ids.unused, got.ids.seen);
    }
}
