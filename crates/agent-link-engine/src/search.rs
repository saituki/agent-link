//! セッション検索エンジン（仕様書 `docs/spec-session-search.md`）。
//!
//! `.agent-link/` 配下のブロック実行履歴（pi セッション jsonl）を 2 段階で検索する
//! コアロジックを提供する。CLI（`agent-link search`）から利用される。
//! UI ロジックはこのクレートに置かない。
//!
//! ## 検索の流れ
//!
//! 1. [`parse_query`] — 生入力（`RawSearchForm`）を解析し [`SearchQuery`] を生成
//! 2. [`run_search`] — DB で実行履歴を絞り込み（§5.1）→ メイン jsonl を特定
//!    （§5.2）→ type / word 条件でマッチ抽出（§4）
//!
//! ## 使用例
//!
//! ```no_run
//! use agent_link_engine::db::SaiStore;
//! use agent_link_engine::search::{parse_query, run_search, RawSearchForm};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let store = SaiStore::open(std::path::Path::new(".agent-link/engine/agent-link.db"))?;
//! let raw = RawSearchForm {
//!     round_gr: "1".to_string(),
//!     round: String::new(),
//!     id: String::new(),
//!     block: String::new(),
//!     date: "2026/08/10-2026/08/20".to_string(),
//!     status: String::new(),
//!     type_: "user".to_string(),
//!     word: "llm, agent-link-tui".to_string(),
//! };
//! let query = parse_query(&raw)?;
//! let hits = run_search(&store, &query)?;
//! for hit in &hits {
//!     println!("ID{} | {} | {} | {}", hit.block_id, hit.type_.as_str(), hit.turn, hit.snippet);
//! }
//! # Ok(())
//! # }
//! ```

use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde::Serialize;

use crate::db::models::ExecutionHistory;
use crate::db::SaiStore;

// ============================================================================
// データ型
// ============================================================================

/// 検索結果の種別（仕様書 §4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum SearchHitType {
    /// user message の text
    User,
    /// assistant message の text
    Agent,
    /// assistant message の thinking
    Thinking,
    /// assistant の toolCall（name + arguments）
    ToolCall,
    /// ツール名（toolCall.name および toolResult.toolName）
    ToolName,
    /// toolResult の text
    ToolResult,
}

impl SearchHitType {
    /// 文字列表現（表示用）。
    pub fn as_str(self) -> &'static str {
        match self {
            SearchHitType::User => "user",
            SearchHitType::Agent => "agent",
            SearchHitType::Thinking => "thinking",
            SearchHitType::ToolCall => "tool_call",
            SearchHitType::ToolName => "tool_name",
            SearchHitType::ToolResult => "tool_result",
        }
    }

    /// 文字列から種別を生成する。
    ///
    /// `tool` は [`SearchHitType::ToolCall`] / [`SearchHitType::ToolName`] /
    /// [`SearchHitType::ToolResult`] の集合エイリアスとして展開される。
    pub fn parse(s: &str) -> Result<Vec<SearchHitType>, SearchError> {
        match s.trim().to_lowercase().as_str() {
            "user" => Ok(vec![SearchHitType::User]),
            "agent" => Ok(vec![SearchHitType::Agent]),
            "thinking" => Ok(vec![SearchHitType::Thinking]),
            "tool_call" | "toolcall" => Ok(vec![SearchHitType::ToolCall]),
            "tool_name" | "toolname" => Ok(vec![SearchHitType::ToolName]),
            "tool_result" | "toolresult" => Ok(vec![SearchHitType::ToolResult]),
            "tool" => Ok(vec![
                SearchHitType::ToolCall,
                SearchHitType::ToolName,
                SearchHitType::ToolResult,
            ]),
            other => Err(SearchError::Parse(format!(
                "不明な type 値です: `{other}`（user/agent/thinking/tool_call/tool_name/tool_result/tool が指定可能）"
            ))),
        }
    }
}

/// 数値範囲（仕様書 §3.1）。`N` / `N-M` / `-M` / `N-` を表現する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumRange {
    /// 下限（None = 下限なし）
    pub min: Option<i64>,
    /// 上限（None = 上限なし）
    pub max: Option<i64>,
}

impl NumRange {
    /// 値が範囲内か（両端含む）。
    pub fn contains(&self, v: i64) -> bool {
        self.min.is_none_or(|m| v >= m) && self.max.is_none_or(|m| v <= m)
    }
}

/// 日付範囲（仕様書 §3.2）。両端を含む。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    /// 開始日（含む）
    pub start: NaiveDate,
    /// 終了日（含む）
    pub end: NaiveDate,
}

impl DateRange {
    /// 日付が範囲内か（両端含む）。
    pub fn contains(&self, d: NaiveDate) -> bool {
        d >= self.start && d <= self.end
    }
}

/// 検索フォームの生入力（TUI のテキストボックス / CLI の引数と共有）。
///
/// 各項目はまだ検証されていない文字列。空文字列は「条件なし」を表す。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawSearchForm {
    /// Round（グローバルラウンド / GR）。例: `1`、`1-5`
    pub round_gr: String,
    /// round（実行ラウンド）。例: `2`、`1-2`
    pub round: String,
    /// ブロック ID 範囲。例: `10`、`10-20`
    pub id: String,
    /// ブロック名（部分一致・大小無視）
    pub block: String,
    /// 実行日。例: `2026/08/10`、`2026/08/10-2026/08/20`
    pub date: String,
    /// ステータス絞り込み（カンマ区切り・R304。空 = `todo` 除外。`todo` 指定時のみ予約行を含む）
    pub status: String,
    /// 種別（カンマ区切り）。例: `user,agent,tool`
    pub type_: String,
    /// 検索語（カンマ区切り・AND・大小無視）
    pub word: String,
}

/// 解析済みの検索クエリ。
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Round（global_round）の範囲
    pub round_gr: Option<NumRange>,
    /// round の範囲（実行値 NULL 時はブロック定義 round で補完）
    pub round: Option<NumRange>,
    /// ブロック ID の範囲
    pub id: Option<NumRange>,
    /// ブロック名部分一致（小文字化済み）
    pub block: Option<String>,
    /// 実行日の範囲
    pub date: Option<DateRange>,
    /// 種別フィルタ（空 = 全種別）
    pub types: Vec<SearchHitType>,
    /// 検索語（AND・小文字化済み。空 = 条件なし）
    pub words: Vec<String>,
    /// ステータス絞り込み（R304。空 = `todo` を除外した全部。`todo` 指定時のみ予約行を含む）
    pub statuses: Vec<String>,
}

/// 検索結果 1 行（仕様書 §6）。
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    /// 出自プロジェクト名（複数プロジェクト検索・L2。単体 DB 用関数では空）
    #[serde(default)]
    pub project: String,
    /// ブロック ID（`execution_history.block_id`）
    pub block_id: i64,
    /// 実行 ID（`execution_history.id`。ソート安定化・詳細表示用）
    pub exec_id: i64,
    /// 種別
    #[serde(rename = "type")]
    pub type_: SearchHitType,
    /// jsonl 行番号（1 始まり。`session` ヘッダ行を含む全行カウント）
    pub turn: usize,
    /// マッチした content の 1 行抜粋
    pub snippet: String,
    /// マッチした検索語（小文字化。TUI の強調表示用）
    pub matched_words: Vec<String>,
    /// 元 jsonl のパス（詳細表示用）
    pub session_path: PathBuf,
}

/// 検索対象プロジェクトの定義（L2: 複数プロジェクト検索用の入力型）。
///
/// agent-link 単体では常に自プロジェクト 1 件（「プロジェクト内のみ」の原則は維持）。
/// 複数件を渡すのは sai-control だけである（各プロジェクトの DB は read-only で開く）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSource {
    /// 表示名（検索結果行・詳細表示に使用）
    pub name: String,
    /// プロジェクトルート（`<root>/.agent-link/` を走査）
    pub root: PathBuf,
}

impl ProjectSource {
    /// プロジェクトルートの agent-link DB パス（`<root>/.agent-link/engine/agent-link.db`）。
    pub fn db_path(&self) -> PathBuf {
        crate::project::db_path_in(&self.root)
    }
}

/// 検索処理のエラー。
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// 入力書式のエラー（数値範囲・日付・type 値など）
    #[error("検索条件の書式エラー: {0}")]
    Parse(String),
    /// DB アクセスのエラー
    #[error("DB エラー: {0}")]
    Db(#[from] crate::db::models::DbError),
    /// ファイル I/O のエラー
    #[error("I/O エラー: {0}")]
    Io(#[from] std::io::Error),
}

// ============================================================================
// パーサ
// ============================================================================

/// 数値範囲文字列（`N` / `N-M` / `-M` / `N-`）を解析する。
///
/// # 例
///
/// ```
/// # use agent_link_engine::search::{parse_num_range, SearchError};
/// let r = parse_num_range("3-5").unwrap();
/// assert_eq!(r.min, Some(3));
/// assert_eq!(r.max, Some(5));
/// assert!(parse_num_range("x").is_err());
/// ```
pub fn parse_num_range(s: &str) -> Result<NumRange, SearchError> {
    let t = s.trim();
    if t.is_empty() {
        return Err(SearchError::Parse("空の数値範囲です".to_string()));
    }
    // 全角ハイフン・全角数字は扱わない（ASCII のみ許容）
    let (min_s, max_s) = match t.split_once('-') {
        Some((a, b)) => (a.trim(), b.trim()),
        None => (t, t), // 単一値は min == max
    };
    let min = parse_bound(min_s, "下限")?;
    let max = parse_bound(max_s, "上限")?;
    // "3-" や "-5" は片側のみ。両方空は不正
    if min.is_none() && max.is_none() {
        return Err(SearchError::Parse(format!("不正な数値範囲です: `{t}`")));
    }
    if let (Some(a), Some(b)) = (min, max) {
        if a > b {
            return Err(SearchError::Parse(format!(
                "数値範囲の下限が上限を超えています: `{t}`"
            )));
        }
    }
    Ok(NumRange { min, max })
}

/// 範囲の片側の文字列を解析する。空文字列は None（開放端）。
fn parse_bound(s: &str, label: &str) -> Result<Option<i64>, SearchError> {
    if s.is_empty() {
        return Ok(None);
    }
    s.parse::<i64>().map(Some).map_err(|_| {
        SearchError::Parse(format!(
            "不正な{label}です: `{s}`（整数を指定してください）"
        ))
    })
}

/// 日付文字列（`YYYY/MM/DD` または `YYYY/MM/DD-YYYY/MM/DD`）を解析する。
///
/// 月日は 1〜2 桁を許容（例: `2026/8/5`）。範囲の前後が逆の場合は自動で入れ替える。
pub fn parse_date_range(s: &str) -> Result<DateRange, SearchError> {
    let t = s.trim();
    if t.is_empty() {
        return Err(SearchError::Parse("空の日付範囲です".to_string()));
    }
    let parse_one = |v: &str| -> Result<NaiveDate, SearchError> {
        let parts: Vec<&str> = v.trim().split('/').collect();
        if parts.len() != 3 {
            return Err(SearchError::Parse(format!(
                "不正な日付です: `{v}`（YYYY/MM/DD 形式で指定してください）"
            )));
        }
        let year: i32 = parts[0]
            .parse()
            .map_err(|_| SearchError::Parse(format!("不正な年です: `{}`", parts[0])))?;
        let month: u32 = parts[1]
            .parse()
            .map_err(|_| SearchError::Parse(format!("不正な月です: `{}`", parts[1])))?;
        let day: u32 = parts[2]
            .parse()
            .map_err(|_| SearchError::Parse(format!("不正な日です: `{}`", parts[2])))?;
        NaiveDate::from_ymd_opt(year, month, day)
            .ok_or_else(|| SearchError::Parse(format!("存在しない日付です: `{v}`")))
    };
    // 日付は `YYYY/MM/DD`（区切り `/`）、範囲は `-` で接続するため
    // 最初の `-` で分割すればよい（負数は許容しない）
    let (start, end) = match t.split_once('-') {
        Some((a, b)) => (parse_one(a)?, parse_one(b)?),
        None => {
            let d = parse_one(t)?;
            (d, d)
        }
    };
    // 前後が逆の場合は自動で入れ替える（仕様書 §3.2）
    if start > end {
        return Ok(DateRange {
            start: end,
            end: start,
        });
    }
    Ok(DateRange { start, end })
}

/// 生入力フォームを解析して [`SearchQuery`] を生成する。
///
/// すべての項目が空の場合も成功する（全件対象のクエリになる）。
pub fn parse_query(raw: &RawSearchForm) -> Result<SearchQuery, SearchError> {
    let mut q = SearchQuery::default();

    // 数値範囲系
    if !raw.round_gr.trim().is_empty() {
        q.round_gr = Some(parse_num_range(&raw.round_gr)?);
    }
    if !raw.round.trim().is_empty() {
        q.round = Some(parse_num_range(&raw.round)?);
    }
    if !raw.id.trim().is_empty() {
        q.id = Some(parse_num_range(&raw.id)?);
    }

    // ブロック名部分一致（小文字化して保持）
    let block = raw.block.trim().to_lowercase();
    if !block.is_empty() {
        q.block = Some(block);
    }

    // 日付
    if !raw.date.trim().is_empty() {
        q.date = Some(parse_date_range(&raw.date)?);
    }

    // status（カンマ区切り・小文字化。空は指定なし扱い）
    for s in raw.status.split(',') {
        let s = s.trim().to_lowercase();
        if s.is_empty() {
            continue;
        }
        match s.as_str() {
            "todo" | "pending" | "running" | "completed" | "failed" | "cancelled" => {
                q.statuses.push(s);
            }
            _ => {
                return Err(SearchError::Parse(format!("不正な status です: `{s}`")));
            }
        }
    }

    // type（カンマ区切り）
    for t in raw.type_.split(',') {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        q.types.extend(SearchHitType::parse(t)?);
    }

    // word（カンマ区切り・AND・小文字化）
    for w in raw.word.split(',') {
        let w = w.trim().to_lowercase();
        if !w.is_empty() {
            q.words.push(w);
        }
    }

    Ok(q)
}

// ============================================================================
// DB フィルタ（仕様書 §5.1）
// ============================================================================

/// DB フィルタ済みの実行履歴。
struct FilteredExecution {
    exec: ExecutionHistory,
}

/// DB 上の実行履歴を [`SearchQuery`] の DB 条件で絞り込む。
///
/// 対象条件: `round_gr`（global_round）/ `round`（NULL 時はブロック定義 round で
/// 補完・左 SB と同一規則）/ `id`（block_id）/ `block`（名前部分一致）/ `date`
/// （started_at の日付）/ `statuses`（R304。空 = `todo` 除外、`todo` 指定時のみ予約行を含む）。
fn filter_executions(
    store: &SaiStore,
    q: &SearchQuery,
) -> Result<Vec<FilteredExecution>, SearchError> {
    let executions = store.list_all_executions()?;
    let mut result = Vec::new();
    // ブロック定義の round / name キャッシュ（get_block_by_id の問い合わせ削減）
    let mut block_cache: std::collections::HashMap<i64, (String, Option<i64>)> =
        std::collections::HashMap::new();
    for e in executions {
        // status 絞り込み（R304）：空 = `todo` を除外。`todo` 指定時のみ予約行を含む。
        if q.statuses.is_empty() {
            if e.status == "todo" {
                continue;
            }
        } else if !q.statuses.iter().any(|s| s == &e.status) {
            continue;
        }
        let (name, block_round) = match block_cache.get(&e.block_id) {
            Some(v) => v.clone(),
            None => {
                let v = match store.get_block_by_id(e.block_id) {
                    Ok(b) => (b.name.clone(), b.round),
                    Err(_) => (String::new(), None),
                };
                block_cache.insert(e.block_id, v.clone());
                v
            }
        };

        // Round（global_round）
        if let Some(r) = &q.round_gr {
            match e.global_round {
                Some(v) if r.contains(v) => {}
                _ => continue,
            }
        }
        // round（実行値 NULL 時はブロック定義 round で補完）
        if let Some(r) = &q.round {
            match e.round.or(block_round) {
                Some(v) if r.contains(v) => {}
                _ => continue,
            }
        }
        // ID（block_id）
        if let Some(r) = &q.id {
            if !r.contains(e.block_id) {
                continue;
            }
        }
        // block 名部分一致（name と display_name 相当の name のみ。大小無視）
        if let Some(b) = &q.block {
            if !name.to_lowercase().contains(b) {
                continue;
            }
        }
        // Date（started_at の日付部分）
        if let Some(d) = &q.date {
            match exec_start_date(&e.started_at) {
                Some(v) if d.contains(v) => {}
                _ => continue,
            }
        }
        result.push(FilteredExecution { exec: e });
    }
    Ok(result)
}

/// `started_at`（`YYYY-MM-DD HH:MM:SS` 形式）から日付部分を取り出す。
///
/// `None` / パース不能 / NULL は `None` を返す（Date 条件指定時は除外される）。
fn exec_start_date(started_at: &Option<String>) -> Option<NaiveDate> {
    let s = started_at.as_deref()?;
    NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()
}

// ============================================================================
// jsonl スキャン（仕様書 §4・§5.3）
// ============================================================================

/// 抜粋の最大文字数（表示幅に合わせ TUI 側でも切詰めるための上限）。
const SNIPPET_MAX_CHARS: usize = 80;

/// 会話 jsonl を 1 行ずつ走査し、type / word 条件にマッチしたヒットを返す。
///
/// - pi の会話（行の `type` が `message`）と cc の会話（行の `type` が
///   `user` / `assistant`）の両方を同じ走査で扱う。どちらの行も、候補を作ったあとの
///   種別絞り込み・語の AND・抜粋は共通の手順を通る
/// - `turn` は 1 始まりの行番号（`session` ヘッダ行を含む全行カウント）
/// - パースに失敗した行はスキップ（検索は継続）
/// - 1 エントリ内の text / thinking / toolCall は別々のマッチ候補になる
fn scan_session_file(
    path: &Path,
    block_id: i64,
    exec_id: i64,
    q: &SearchQuery,
    project: &str,
) -> std::io::Result<Vec<SearchHit>> {
    let content = std::fs::read_to_string(path)?;
    let mut hits = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let turn = idx + 1;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue; // パース失敗行はスキップ
        };
        // 行の `type` で会話の runner を判断する。`message` = pi、`user`/`assistant` = cc
        let row_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        // (種別, 検索対象テキスト) の候補を収集する
        let mut candidates: Vec<(SearchHitType, String)> = Vec::new();
        match row_type {
            "message" => {
                // pi の会話（message.role で user / assistant / toolResult を見分ける）
                let Some(msg) = v.get("message") else {
                    continue;
                };
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
                let content_val = msg.get("content");
                match role {
                    "user" => {
                        collect_text_items(content_val, SearchHitType::User, &mut candidates);
                    }
                    "assistant" => {
                        if let Some(items) = content_val.and_then(|c| c.as_array()) {
                            for item in items {
                                match item.get("type").and_then(|t| t.as_str()) {
                                    Some("text") => {
                                        if let Some(s) = item.get("text").and_then(|t| t.as_str()) {
                                            candidates.push((SearchHitType::Agent, s.to_string()));
                                        }
                                    }
                                    Some("thinking") => {
                                        if let Some(s) =
                                            item.get("thinking").and_then(|t| t.as_str())
                                        {
                                            candidates
                                                .push((SearchHitType::Thinking, s.to_string()));
                                        }
                                    }
                                    Some("toolCall") => {
                                        let name =
                                            item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                        let args = item
                                            .get("arguments")
                                            .map(|a| a.to_string())
                                            .unwrap_or_default();
                                        // tool_call: ツール名 + 引数 JSON を対象にする
                                        candidates.push((
                                            SearchHitType::ToolCall,
                                            format!("{name} {args}"),
                                        ));
                                        // tool_name: ツール名のみ
                                        if !name.is_empty() {
                                            candidates
                                                .push((SearchHitType::ToolName, name.to_string()));
                                        }
                                    }
                                    _ => {} // 未知の content type は無視
                                }
                            }
                        }
                    }
                    "toolResult" => {
                        // tool_name: toolResult 側の toolName も対象
                        if let Some(name) = msg.get("toolName").and_then(|n| n.as_str()) {
                            if !name.is_empty() {
                                candidates.push((SearchHitType::ToolName, name.to_string()));
                            }
                        }
                        collect_text_items(content_val, SearchHitType::ToolResult, &mut candidates);
                    }
                    _ => {}
                }
            }
            // cc の会話（行の `type` が `user` / `assistant`）
            "user" | "assistant" => {
                if let Some(msg) = v.get("message") {
                    collect_cc_candidates(row_type, msg, &mut candidates);
                }
            }
            _ => {} // ヘッダ行など（`session` / `model_change` など）は無視
        }

        for (type_, text) in candidates {
            // type フィルタ（空 = 全種別）
            if !q.types.is_empty() && !q.types.contains(&type_) {
                continue;
            }
            // word 条件（AND・大小無視）。語が空なら条件なし
            let text_lower = text.to_lowercase();
            let matched: Vec<String> = q
                .words
                .iter()
                .filter(|w| text_lower.contains(w.as_str()))
                .cloned()
                .collect();
            if !q.words.is_empty() && matched.len() != q.words.len() {
                continue; // AND 条件未達
            }
            hits.push(SearchHit {
                project: project.to_string(),
                block_id,
                exec_id,
                type_,
                turn,
                snippet: extract_snippet_line(&text, &q.words, SNIPPET_MAX_CHARS),
                matched_words: matched,
                session_path: path.to_path_buf(),
            });
        }
    }
    Ok(hits)
}

/// cc（Claude Code）の会話 1 行から (種別, 検索対象テキスト) の候補を収集する。
///
/// cc の jsonl は行の `type` が `user` / `assistant` で、`message.content` に
/// `text` / `thinking` / `tool_use` / `tool_result` の項目が並ぶ（content が
/// 文字列の形式もある）。項目ごとに別々の候補にするのは pi 側と同じ。
fn collect_cc_candidates(
    row_type: &str,
    msg: &serde_json::Value,
    out: &mut Vec<(SearchHitType, String)>,
) {
    // 文字列の content は user なら User、assistant なら Agent として扱う
    let text_type = if row_type == "user" {
        SearchHitType::User
    } else {
        SearchHitType::Agent
    };
    match msg.get("content") {
        Some(serde_json::Value::String(s)) => {
            out.push((text_type, s.clone()));
        }
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                match item.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(s) = item.get("text").and_then(|t| t.as_str()) {
                            out.push((text_type, s.to_string()));
                        }
                    }
                    Some("thinking") => {
                        if let Some(s) = item.get("thinking").and_then(|t| t.as_str()) {
                            out.push((SearchHitType::Thinking, s.to_string()));
                        }
                    }
                    Some("tool_use") => {
                        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        let input = item
                            .get("input")
                            .map(|a| a.to_string())
                            .unwrap_or_default();
                        // tool_call: ツール名 + 引数 JSON を対象にする（pi と同じ考え方）
                        out.push((SearchHitType::ToolCall, format!("{name} {input}")));
                        // tool_name: ツール名のみ
                        if !name.is_empty() {
                            out.push((SearchHitType::ToolName, name.to_string()));
                        }
                    }
                    // ツールの返り（cc では user 行に入る）。中身の文字を対象にする
                    Some("tool_result") => {
                        collect_text_items(
                            item.get("content"),
                            SearchHitType::ToolResult,
                            out,
                        );
                    }
                    _ => {} // 未知の項目は無視
                }
            }
        }
        _ => {}
    }
}

/// content（配列 or 文字列）から `type:"text"` のテキストを収集する。
fn collect_text_items(
    content_val: Option<&serde_json::Value>,
    type_: SearchHitType,
    out: &mut Vec<(SearchHitType, String)>,
) {
    match content_val {
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    if let Some(s) = item.get("text").and_then(|t| t.as_str()) {
                        out.push((type_, s.to_string()));
                    }
                }
            }
        }
        // 旧形式互換: content が文字列の場合
        Some(serde_json::Value::String(s)) => {
            out.push((type_, s.clone()));
        }
        _ => {}
    }
}

/// マッチ語を中心とした 1 行抜粋を生成する（仕様書 §6.1）。
///
/// - 改行・タブはスペースに圧縮
/// - 最初のマッチ語（大小無視）の位置を中心に前後を取り 1 行化
/// - 先頭・末尾の省略は `...` を付与
/// - マルチバイト文字を壊さないよう `char` 境界で切詰める
fn extract_snippet_line(text: &str, words: &[String], max_chars: usize) -> String {
    // 改行・タブをスペースに圧縮し連続スペースを 1 つに
    let mut flat = String::with_capacity(text.len());
    let mut prev_space = false;
    for c in text.chars() {
        let c = if c == '\n' || c == '\r' || c == '\t' {
            ' '
        } else {
            c
        };
        if c == ' ' && prev_space {
            continue;
        }
        prev_space = c == ' ';
        flat.push(c);
    }
    let flat = flat.trim();
    if flat.chars().count() <= max_chars {
        return flat.to_string();
    }
    // マッチ位置を探す（大小無視）
    let flat_lower = flat.to_lowercase();
    let center = words
        .iter()
        .find_map(|w| flat_lower.find(w.as_str()))
        .unwrap_or(0);
    let chars: Vec<char> = flat.chars().collect();
    // バイト位置 → 文字位置
    let center_char = flat_lower[..center].chars().count();
    let half = max_chars / 2;
    let start = center_char.saturating_sub(half);
    let end = (start + max_chars).min(chars.len());
    let mut result = String::new();
    if start > 0 {
        result.push_str("...");
    }
    result.extend(chars[start..end].iter());
    if end < chars.len() {
        result.push_str("...");
    }
    result
}

// ============================================================================
// エントリポイント
// ============================================================================

/// 検索を実行する（仕様書 §7.4）。
///
/// 1. DB フィルタで対象実行を決定 → 2. 会話 jsonl を特定（`resolve_transcript`。
///    pi は run の `session_dir`、cc は `~/.claude/projects/…`）→
/// 3. type / word でマッチ抽出。結果は `(block_id, exec_id, turn)` 昇順で返す。
///
/// 会話ファイルが見つからない実行はスキップする
/// （エラーにはしない。スキップ件数が必要な場合は [`run_search_verbose`] を使う）。
pub fn run_search(store: &SaiStore, q: &SearchQuery) -> Result<Vec<SearchHit>, SearchError> {
    run_search_verbose(store, q).map(|(hits, _)| hits)
}

/// [`run_search`] に読み飛ばした実行件数の報告を加えた版。
///
/// 戻り値は `(ヒット一覧, スキップした実行数)`。TUI の警告表示用。
pub fn run_search_verbose(
    store: &SaiStore,
    q: &SearchQuery,
) -> Result<(Vec<SearchHit>, usize), SearchError> {
    let filtered = filter_executions(store, q)?;
    let mut hits = Vec::new();
    let mut skipped = 0usize;
    // 会話ファイルの探し方は Web や `message.md` と同じ `resolve_transcript` にそろえる
    // （cc の run は `~/.claude/projects/…/<session_id>.jsonl` にあるため、
    //   pi 用の `session_dir` だけを見ると必ずスキップになる）。
    // owner 移動追従も `resolve_transcript` の中（`resolve_session_dir`）で行われる。
    let project_root = crate::project::project_root();
    for fe in &filtered {
        let exec_id = fe.exec.id.unwrap_or(0);
        let Some(path) = crate::phone_export::resolve_transcript(&fe.exec, &project_root) else {
            skipped += 1;
            continue;
        };
        match scan_session_file(&path, fe.exec.block_id, exec_id, q, "") {
            Ok(mut h) => hits.append(&mut h),
            Err(_) => {
                skipped += 1;
                continue;
            }
        }
    }
    // ソート: block_id → exec_id → turn 昇順（仕様書 §2.2）
    hits.sort_by_key(|h| (h.block_id, h.exec_id, h.turn, h.type_ as u8));
    Ok((hits, skipped))
}

/// 複数プロジェクトを対象に検索を実行する（L2・sai-control からの呼び出し用）。
///
/// 各プロジェクトの DB を **read-only** で開き（テーブル初期化・書き込みは行わない）、
/// 条件絞り込み → jsonl 走査 → ヒットを日時降順（実行開始時刻の新しい順・同一実行内は
/// turn 昇順）でマージする。DB が存在しない・開けないプロジェクトはスキップする。
///
/// agent-link 単体では [`run_search`]（自プロジェクト 1 件・後方互換）を使用する。
pub fn run_search_projects(
    projects: &[ProjectSource],
    q: &SearchQuery,
) -> Result<Vec<SearchHit>, SearchError> {
    // (started_at, ヒット群)。started_at はマージ時の日時降順ソート用
    let mut collected: Vec<(Option<String>, Vec<SearchHit>)> = Vec::new();
    for project in projects {
        let db_path = project.db_path();
        if !db_path.exists() {
            continue;
        }
        let Ok(store) = SaiStore::open_read_only(&db_path) else {
            continue; // 開けない（権限・破損等）プロジェクトはスキップ
        };
        let filtered = filter_executions(&store, q)?;
        for fe in &filtered {
            // 単一検索と同様に `resolve_transcript` で現行正本を探す
            // （owner 移動追従 + cc の会話ファイルも解決される）。
            let exec_id = fe.exec.id.unwrap_or(0);
            let Some(path) = crate::phone_export::resolve_transcript(&fe.exec, &project.root)
            else {
                continue;
            };
            let hits = scan_session_file(
                &path,
                fe.exec.block_id,
                exec_id,
                q,
                &project.name,
            )?;
            if !hits.is_empty() {
                collected.push((fe.exec.started_at.clone(), hits));
            }
        }
    }

    // マージ: 実行開始日時の降順（新しい実行が先。None は最後）。
    // 同一実行内は turn 昇順を保持する（安定ソートのため collected の順序を保つ）
    collected.sort_by(|a, b| b.0.cmp(&a.0));
    let mut hits: Vec<SearchHit> = Vec::new();
    for (_, mut per_exec) in collected {
        per_exec.sort_by_key(|x| (x.turn, x.type_ as u8));
        hits.append(&mut per_exec);
    }
    Ok(hits)
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::BlockMetadata;

    // -----------------------------------------------------------------------
    // 数値範囲パーサ
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_num_range() {
        let r = parse_num_range("3").unwrap();
        assert_eq!((r.min, r.max), (Some(3), Some(3)));
        assert!(r.contains(3));
        assert!(!r.contains(4));

        let r = parse_num_range("3-5").unwrap();
        assert_eq!((r.min, r.max), (Some(3), Some(5)));
        assert!(r.contains(3) && r.contains(4) && r.contains(5));
        assert!(!r.contains(2) && !r.contains(6));

        // 片側開放
        let r = parse_num_range("-5").unwrap();
        assert_eq!((r.min, r.max), (None, Some(5)));
        let r = parse_num_range("3-").unwrap();
        assert_eq!((r.min, r.max), (Some(3), None));

        // 不正値
        assert!(parse_num_range("x").is_err());
        assert!(parse_num_range("").is_err());
        assert!(parse_num_range("5-3").is_err()); // 下限 > 上限
    }

    // -----------------------------------------------------------------------
    // 日付範囲パーサ
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_date_range() {
        // 単日
        let r = parse_date_range("2026/08/10").unwrap();
        assert_eq!(r.start.to_string(), "2026-08-10");
        assert_eq!(r.end.to_string(), "2026-08-10");

        // 範囲
        let r = parse_date_range("2026/08/10-2026/08/20").unwrap();
        assert_eq!(r.start.to_string(), "2026-08-10");
        assert_eq!(r.end.to_string(), "2026-08-20");
        assert!(r.contains(NaiveDate::from_ymd_opt(2026, 8, 15).unwrap()));
        assert!(!r.contains(NaiveDate::from_ymd_opt(2026, 8, 21).unwrap()));

        // 短縮表記（月日は 1〜2 桁）
        let r = parse_date_range("2026/8/5").unwrap();
        assert_eq!(r.start.to_string(), "2026-08-05");

        // 前後逆は自動入替
        let r = parse_date_range("2026/08/20-2026/08/10").unwrap();
        assert_eq!(r.start.to_string(), "2026-08-10");
        assert_eq!(r.end.to_string(), "2026-08-20");

        // 不正値
        assert!(parse_date_range("2026/13/01").is_err());
        assert!(parse_date_range("2026/08").is_err());
        assert!(parse_date_range("").is_err());
    }

    // -----------------------------------------------------------------------
    // parse_query
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_query() {
        // 空欄は全条件なし
        let q = parse_query(&RawSearchForm::default()).unwrap();
        assert!(q.round_gr.is_none() && q.round.is_none() && q.id.is_none());
        assert!(q.block.is_none() && q.date.is_none());
        assert!(q.types.is_empty() && q.words.is_empty());

        let raw = RawSearchForm {
            round_gr: "1".into(),
            round: "1-2".into(),
            id: "10-20".into(),
            block: "Main-Block".into(),
            date: "2026/08/10-2026/08/20".into(),
            status: "completed".into(),
            type_: "user, tool".into(),
            word: "LLM, agent-link-tui , 移植".into(),
        };
        let q = parse_query(&raw).unwrap();
        assert_eq!(
            q.round_gr.unwrap(),
            NumRange {
                min: Some(1),
                max: Some(1)
            }
        );
        assert_eq!(
            q.round.unwrap(),
            NumRange {
                min: Some(1),
                max: Some(2)
            }
        );
        assert_eq!(
            q.id.unwrap(),
            NumRange {
                min: Some(10),
                max: Some(20)
            }
        );
        assert_eq!(q.block.unwrap(), "main-block"); // 小文字化
                                                    // type: user + tool（3 種に展開）
        assert_eq!(
            q.types,
            vec![
                SearchHitType::User,
                SearchHitType::ToolCall,
                SearchHitType::ToolName,
                SearchHitType::ToolResult,
            ]
        );
        // word: AND・小文字化・トリム
        assert_eq!(q.words, vec!["llm", "agent-link-tui", "移植"]);

        // 不正な type
        let raw = RawSearchForm {
            type_: "hoge".into(),
            ..Default::default()
        };
        assert!(parse_query(&raw).is_err());

        // status: 正常系・todo 除外の既定
        let q = parse_query(&RawSearchForm::default()).unwrap();
        assert!(q.statuses.is_empty());
        let raw = RawSearchForm {
            status: "todo,completed".into(),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        assert_eq!(q.statuses, vec!["todo", "completed"]);
        // status: 不正値はエラー
        let raw = RawSearchForm {
            status: "hoge".into(),
            ..Default::default()
        };
        assert!(parse_query(&raw).is_err());
    }

    // -----------------------------------------------------------------------
    // 抜粋生成
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_snippet_line() {
        // 短いテキストはそのまま
        assert_eq!(
            extract_snippet_line("こんにちは", &["こん".into()], 80),
            "こんにちは"
        );

        // 改行・連続スペースの圧縮
        let s = extract_snippet_line("line1\n\nline2\tline3", &["line2".into()], 80);
        assert_eq!(s, "line1 line2 line3");

        // 長いテキストはマッチ位置中心・... 付き
        let long = format!("{}target{}", "あ".repeat(100), "い".repeat(100));
        let s = extract_snippet_line(&long, &["target".into()], 20);
        assert!(s.starts_with("..."));
        assert!(s.ends_with("..."));
        assert!(s.contains("target"));
        // 文字数上限（... は除く）
        let core = s.trim_start_matches("...").trim_end_matches("...");
        assert!(core.chars().count() <= 20);

        // マルチバイト文字を壊さない
        let long = "日本語".repeat(100);
        let s = extract_snippet_line(&long, &[], 10);
        assert!(s
            .chars()
            .all(|c| c == '日' || c == '本' || c == '語' || c == '.'));
    }

    // -----------------------------------------------------------------------
    // jsonl スキャン
    // -----------------------------------------------------------------------

    /// 実セッション形式のテスト用 jsonl を書き出す。
    fn write_test_jsonl(dir: &std::path::Path) -> PathBuf {
        let path = dir.join("2026-08-29T01-06-45-229Z_01a04b0e-056d-7e81-b03b-00ef35e1526c.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"session","version":3,"id":"abc","timestamp":"2026-08-29T01:06:45.229Z"}"#, "\n",
                r#"{"type":"model_change","provider":"p","modelId":"m"}"#, "\n",
                r#"{"type":"message","id":"m1","message":{"role":"user","content":[{"type":"text","text":"pi-agent-link の 検索機能について\n説明してください"}]}}"#, "\n",
                r#"{"type":"message","id":"m2","message":{"role":"assistant","content":[{"type":"thinking","thinking":"SEARCHWORD を含む思考"},{"type":"text","text":"回答です"},{"type":"toolCall","id":"t1","name":"block_discover","arguments":{"q":"llm"}}]}}"#, "\n",
                r#"{"type":"message","id":"m3","message":{"role":"toolResult","toolCallId":"t1","toolName":"block_discover","content":[{"type":"text","text":"サブブロック 2 件"}]}}"#, "\n",
                "not a json line\n",
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn test_scan_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_test_jsonl(dir.path());
        let q = parse_query(&RawSearchForm::default()).unwrap();

        let hits = scan_session_file(&path, 5, 1, &q, "").unwrap();

        // turn は 1 始まりの行番号（session ヘッダ含む）
        let by_turn: Vec<(usize, SearchHitType)> = hits.iter().map(|h| (h.turn, h.type_)).collect();
        assert!(by_turn.contains(&(3, SearchHitType::User)));
        assert!(by_turn.contains(&(4, SearchHitType::Thinking)));
        assert!(by_turn.contains(&(4, SearchHitType::Agent)));
        assert!(by_turn.contains(&(4, SearchHitType::ToolCall)));
        assert!(by_turn.contains(&(4, SearchHitType::ToolName)));
        assert!(by_turn.contains(&(5, SearchHitType::ToolName)));
        assert!(by_turn.contains(&(5, SearchHitType::ToolResult)));

        // block_id / exec_id / パスの反映
        assert!(hits.iter().all(|h| h.block_id == 5 && h.exec_id == 1));
        assert_eq!(hits[0].session_path, path);

        // word AND 条件
        let raw = RawSearchForm {
            word: "検索機能".into(),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        let hits = scan_session_file(&path, 5, 1, &q, "").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].type_, SearchHitType::User);
        assert_eq!(hits[0].matched_words, vec!["検索機能"]);

        // AND: 両方含むエントリのみ
        let raw = RawSearchForm {
            word: "searchword, 思考".into(),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        let hits = scan_session_file(&path, 5, 1, &q, "").unwrap();
        assert!(hits.iter().all(|h| h.type_ == SearchHitType::Thinking));
    }

    /// cc（Claude Code）の会話のテスト用 jsonl を書き出す。
    ///
    /// pi と違い、行の `type` が `user` / `assistant` で、`message.content` に
    /// `text` / `thinking` / `tool_use` / `tool_result` の項目が並ぶ。
    fn write_test_cc_jsonl(dir: &std::path::Path) -> PathBuf {
        let path = dir.join("cccccccc-3333-4333-8333-333333333333.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"summary","summary":"会話の要約"}"#, "\n",
                r#"{"type":"user","message":{"role":"user","content":"CCWORD について 説明してください"}}"#, "\n",
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"CCWORD を含む思考"},{"type":"text","text":"回答です"},{"type":"tool_use","id":"t1","name":"block_discover","input":{"q":"llm"}}]}}"#, "\n",
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"サブブロック CCWORD 2 件"}]}]}}"#, "\n",
                "not a json line\n",
            ),
        )
        .unwrap();
        path
    }

    /// cc の会話でも各種別が語で当たる（`scan_session_file` 単体）。
    #[test]
    fn test_scan_session_file_cc() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_test_cc_jsonl(dir.path());
        let q = parse_query(&RawSearchForm::default()).unwrap();

        let hits = scan_session_file(&path, 7, 9, &q, "").unwrap();

        // turn は 1 始まりの行番号（summary 行も数える）
        let by_turn: Vec<(usize, SearchHitType)> = hits.iter().map(|h| (h.turn, h.type_)).collect();
        assert!(by_turn.contains(&(2, SearchHitType::User)));
        assert!(by_turn.contains(&(3, SearchHitType::Thinking)));
        assert!(by_turn.contains(&(3, SearchHitType::Agent)));
        assert!(by_turn.contains(&(3, SearchHitType::ToolCall)));
        assert!(by_turn.contains(&(3, SearchHitType::ToolName)));
        assert!(by_turn.contains(&(4, SearchHitType::ToolResult)));

        // block_id / exec_id / パスの反映
        assert!(hits.iter().all(|h| h.block_id == 7 && h.exec_id == 9));
        assert_eq!(hits[0].session_path, path);

        // 種別ごとに語が当たること
        let by_type = |type_: SearchHitType, word: &str| -> Vec<SearchHit> {
            let q = parse_query(&RawSearchForm {
                type_: type_.as_str().to_string(),
                word: word.to_string(),
                ..Default::default()
            })
            .unwrap();
            scan_session_file(&path, 7, 9, &q, "").unwrap()
        };
        assert!(!by_type(SearchHitType::User, "ccword").is_empty());
        assert!(!by_type(SearchHitType::Agent, "回答").is_empty());
        assert!(!by_type(SearchHitType::Thinking, "ccword").is_empty());
        // tool_call はツール名 + 引数 JSON が対象（ツール名も一致する）
        assert!(!by_type(SearchHitType::ToolCall, "block_discover").is_empty());
        // tool_name はツール名のみが対象（引数の値は当たらない）
        assert!(!by_type(SearchHitType::ToolName, "block_discover").is_empty());
        assert!(by_type(SearchHitType::ToolName, "llm").is_empty());
        assert!(!by_type(SearchHitType::ToolResult, "ccword").is_empty());

        // word AND 条件（cc の user 行 1 件だけ）
        let q = parse_query(&RawSearchForm {
            word: "ccword, 説明".into(),
            ..Default::default()
        })
        .unwrap();
        let hits = scan_session_file(&path, 7, 9, &q, "").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].type_, SearchHitType::User);
        assert_eq!(hits[0].matched_words, vec!["ccword", "説明"]);
    }

    // -----------------------------------------------------------------------
    // run_search（DB フィルタ + 結合）
    // -----------------------------------------------------------------------

    /// テスト用 DB にブロックと実行を投入し、実行 ID を返す。
    ///
    /// `db_path` が存在しない場合は新規作成する（複数回呼び出しで 1 つの DB に
    /// 複数実行を投入できる）。round / global_round / started_at / session_dir は
    /// 専用の公開 API がないため別接続で直接 UPDATE する（テスト内のみの利用）。
    fn setup_test_db(
        db_path: &std::path::Path,
        block_name: &str,
        block_round: Option<i64>,
        exec_round: Option<i64>,
        global_round: Option<i64>,
        started_at: &str,
        session_dir: Option<&Path>,
    ) -> (i64, i64) {
        let store = SaiStore::open(db_path).unwrap();
        let block = BlockMetadata::new(
            block_name.to_string(),
            "テスト用ブロック".to_string(),
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
        drop(store);

        // 直接 UPDATE（テスト用）
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE execution_history SET round = ?1, global_round = ?2, \
             started_at = ?3, session_dir = ?4 WHERE id = ?5",
            rusqlite::params![
                exec_round,
                global_round,
                started_at,
                session_dir.map(|p| p.to_string_lossy().to_string()),
                exec_id,
            ],
        )
        .unwrap();
        conn.execute(
            "UPDATE block_metadata SET round = ?1 WHERE id = ?2",
            rusqlite::params![block_round, block_id],
        )
        .unwrap();
        (block_id, exec_id)
    }

    #[test]
    fn test_run_search_filters_and_scan() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let sess_dir = dir.path().join("pi-sessions-test");
        std::fs::create_dir_all(&sess_dir).unwrap();
        write_test_jsonl(&sess_dir);

        let (block_id, exec_id) = setup_test_db(
            &db_path,
            "test-pi-sai-main",
            None,
            Some(2),
            Some(1),
            "2026-08-29 01:06:41",
            Some(&sess_dir),
        );
        // 条件外の実行を 1 件追加（global_round = 9・session_dir 無し）
        let (other_block_id, _) = setup_test_db(
            &db_path,
            "other-block",
            None,
            Some(1),
            Some(9),
            "2026-01-01 00:00:00",
            None,
        );

        let store = SaiStore::open(&db_path).unwrap();

        // 全条件なし → 全実行（session_dir があるもののみスキャン）
        let q = parse_query(&RawSearchForm::default()).unwrap();
        let (hits, skipped) = run_search_verbose(&store, &q).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(skipped, 1); // session_dir 無しの other-block はスキップ
        assert!(hits.iter().all(|h| h.exec_id == exec_id));

        // Round（global_round）フィルタ
        let raw = RawSearchForm {
            round_gr: "1".into(),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        let hits = run_search(&store, &q).unwrap();
        assert!(!hits.is_empty());

        let raw = RawSearchForm {
            round_gr: "9".into(),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        assert!(run_search(&store, &q).unwrap().is_empty());

        // round フィルタ（実行値 2）
        let raw = RawSearchForm {
            round: "2".into(),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        let hits = run_search(&store, &q).unwrap();
        assert!(!hits.is_empty());

        // round: 実行値 NULL → ブロック定義 round で補完
        let (nr_block_id, _) = setup_test_db(
            &db_path,
            "no-round-block",
            Some(3),
            None,
            Some(1),
            "2026-08-29 01:06:41",
            None,
        );
        let raw = RawSearchForm {
            round: "3".into(),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        // session_dir 無しでもフィルタ自体は通る（ヒット 0・スキップ 1）
        let (hits, skipped) = run_search_verbose(&store, &q).unwrap();
        assert!(hits.is_empty() && skipped == 1);
        // round 2 指定では no-round-block（定義 round = 3）は弾かれる
        let raw = RawSearchForm {
            round: "2".into(),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        let (hits, _) = run_search_verbose(&store, &q).unwrap();
        assert!(hits.iter().all(|h| h.exec_id == exec_id));
        let _ = nr_block_id;

        // ID（block_id）フィルタ
        let raw = RawSearchForm {
            id: format!("{}", block_id),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        let hits = run_search(&store, &q).unwrap();
        assert!(hits.iter().all(|h| h.block_id == block_id));
        let raw = RawSearchForm {
            id: format!("{}", other_block_id),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        assert!(run_search(&store, &q).unwrap().is_empty());

        // block 名部分一致（大小無視）
        let raw = RawSearchForm {
            block: "PI-SAI".into(),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        let hits = run_search(&store, &q).unwrap();
        assert!(!hits.is_empty());

        // Date フィルタ（両端含む）
        let raw = RawSearchForm {
            date: "2026/08/29".into(),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        assert!(!run_search(&store, &q).unwrap().is_empty());
        let raw = RawSearchForm {
            date: "2026/08/30".into(),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        assert!(run_search(&store, &q).unwrap().is_empty());

        // word + type 複合
        let raw = RawSearchForm {
            type_: "user".into(),
            word: "検索機能".into(),
            ..Default::default()
        };
        let q = parse_query(&raw).unwrap();
        let hits = run_search(&store, &q).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].type_, SearchHitType::User);
        assert_eq!(hits[0].block_id, block_id);

        // ソート順（block_id → exec_id → turn）
        let q = parse_query(&RawSearchForm::default()).unwrap();
        let hits = run_search(&store, &q).unwrap();
        let keys: Vec<(i64, i64, usize)> = hits
            .iter()
            .map(|h| (h.block_id, h.exec_id, h.turn))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    /// テスト用 DB の実行に agent_session_id を設定する。
    ///
    /// 公開 API がないため別接続で直接 UPDATE する（テスト内のみの利用）。
    fn set_session_id(db_path: &std::path::Path, exec_id: i64, session_id: Option<&str>) {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE execution_history SET agent_session_id = ?1 WHERE id = ?2",
            rusqlite::params![session_id, exec_id],
        )
        .unwrap();
    }

    /// cc の実行が `skipped` にならず、ヒットに入ることの確認。
    ///
    /// cc の会話は run フォルダではなく `~/.claude/projects/…/<session_id>.jsonl`
    /// に置く。`HOME` と `AGENT_LINK_PROJECT_ROOT` を一時フォルダに向けて、
    /// `resolve_transcript` の探索先そのものを一時フォルダ内の物にする。
    #[test]
    fn test_run_search_finds_cc_transcript() {
        // 環境変数（HOME・AGENT_LINK_PROJECT_ROOT）を書き換えるため直列化する
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let root = dir.path().join("project-root");
        std::fs::create_dir_all(&root).unwrap();

        // cc の会話ファイル（`~/.claude/projects/<slug(root)>/<session_id>.jsonl`）
        let sid = "cccccccc-3333-4333-8333-333333333333";
        let cc_dir = dir
            .path()
            .join("home")
            .join(".claude")
            .join("projects")
            .join(crate::session::claude_project_slug(&root));
        std::fs::create_dir_all(&cc_dir).unwrap();
        write_test_cc_jsonl(&cc_dir);

        let (_block_id, exec_id) = setup_test_db(
            &db_path,
            "test-cc-block",
            None,
            Some(1),
            Some(1),
            "2026-08-29 01:06:41",
            None, // cc の実行には session_dir が無い
        );
        set_session_id(&db_path, exec_id, Some(sid));

        let old_home = std::env::var_os("HOME");
        let old_root = std::env::var_os("AGENT_LINK_PROJECT_ROOT");
        std::env::set_var("HOME", dir.path().join("home"));
        std::env::set_var("AGENT_LINK_PROJECT_ROOT", &root);

        let store = SaiStore::open(&db_path).unwrap();
        let q = parse_query(&RawSearchForm {
            word: "ccword".into(),
            ..Default::default()
        })
        .unwrap();
        let result = run_search_verbose(&store, &q);
        drop(store);

        if let Some(v) = old_home {
            std::env::set_var("HOME", v);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(v) = old_root {
            std::env::set_var("AGENT_LINK_PROJECT_ROOT", v);
        } else {
            std::env::remove_var("AGENT_LINK_PROJECT_ROOT");
        }

        let (hits, skipped) = result.unwrap();
        // 会話が見つからないのでスキップされていない（ヒットが出ている）
        assert_eq!(skipped, 0);
        assert!(!hits.is_empty(), "cc の会話がヒットに入ること");
        assert!(hits.iter().all(|h| h.exec_id == exec_id));
        assert_eq!(hits[0].session_path, cc_dir.join(format!("{sid}.jsonl")));
    }

    /// メイン jsonl（UUID パターン）のみが検索対象になることの確認（§5.2）。
    #[test]
    fn test_run_search_uses_main_session_file_only() {
        let dir = tempfile::tempdir().unwrap();
        let sess_dir = dir.path().join("pi-sessions-test");
        std::fs::create_dir_all(&sess_dir).unwrap();
        write_test_jsonl(&sess_dir); // UUID パターン = メイン jsonl
                                     // メタデータのみの起動ファイル（メインではない）
        std::fs::write(
            sess_dir.join("2026-08-29T01-06-41-test-pi-sai-main.jsonl"),
            r#"{"type":"session","version":3,"id":"meta"}"#,
        )
        .unwrap();

        let db_path = dir.path().join("main-test.db");
        setup_test_db(
            &db_path,
            "main-test",
            None,
            Some(1),
            Some(1),
            "2026-08-29 01:06:41",
            Some(&sess_dir),
        );
        let store = SaiStore::open(&db_path).unwrap();
        let q = parse_query(&RawSearchForm::default()).unwrap();
        let hits = run_search(&store, &q).unwrap();
        // メタファイルにはメッセージが無いためヒットはすべてメイン jsonl 由来
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| {
            h.session_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains("01a04b0e"))
                .unwrap_or(false)
        }));
    }

    // -----------------------------------------------------------------------
    // 複数プロジェクト検索（L2）
    // -----------------------------------------------------------------------

    /// 2 プロジェクト分の DB + jsonl を一時ディレクトリに作成し、
    /// 横断検索・マージ順・`project` フィールドを検証する。
    #[test]
    fn test_run_search_projects() {
        // プロジェクト A（新しい実行: 2026-08-29）
        let dir_a = tempfile::tempdir().unwrap();
        let root_a = dir_a.path().join("proj-a");
        let sess_a = root_a.join(".agent-link").join("sessions").join("pi-sessions-x");
        std::fs::create_dir_all(&sess_a).unwrap();
        write_test_jsonl(&sess_a);
        let db_a = root_a.join(".agent-link").join("engine").join("agent-link.db");
        std::fs::create_dir_all(db_a.parent().unwrap()).unwrap();
        setup_test_db(
            &db_a,
            "block-a",
            None,
            Some(1),
            Some(1),
            "2026-08-29 01:06:41",
            Some(&sess_a),
        );

        // プロジェクト B（古い実行: 2026-08-01・別の検索語でマッチさせる）
        let dir_b = tempfile::tempdir().unwrap();
        let root_b = dir_b.path().join("proj-b");
        let sess_b = root_b.join(".agent-link").join("sessions").join("pi-sessions-y");
        std::fs::create_dir_all(&sess_b).unwrap();
        write_test_jsonl(&sess_b);
        let db_b = root_b.join(".agent-link").join("engine").join("agent-link.db");
        std::fs::create_dir_all(db_b.parent().unwrap()).unwrap();
        setup_test_db(
            &db_b,
            "block-b",
            None,
            Some(1),
            Some(1),
            "2026-08-01 10:00:00",
            Some(&sess_b),
        );

        // 存在しない DB のプロジェクト C（スキップされる）
        let dir_c = tempfile::tempdir().unwrap();
        let root_c = dir_c.path().join("proj-c");

        let projects = vec![
            ProjectSource {
                name: "proj-b".into(),
                root: root_b.clone(),
            },
            ProjectSource {
                name: "proj-a".into(),
                root: root_a.clone(),
            },
            ProjectSource {
                name: "proj-c".into(),
                root: root_c.clone(),
            },
        ];
        let q = parse_query(&RawSearchForm::default()).unwrap();
        let hits = run_search_projects(&projects, &q).unwrap();
        assert!(!hits.is_empty());

        // 出自プロジェクトが入る
        assert!(hits
            .iter()
            .all(|h| h.project == "proj-a" || h.project == "proj-b"));
        assert!(hits.iter().any(|h| h.project == "proj-a"));
        assert!(hits.iter().any(|h| h.project == "proj-b"));

        // マージ順: 実行開始日時の降順（proj-a が先・proj-b が後）
        let first_project = &hits[0].project;
        assert_eq!(first_project, "proj-a");
        // 降順の明示検証: proj-a のヒットがすべて proj-b のヒットより前にある
        let last_a = hits.iter().rposition(|h| h.project == "proj-a").unwrap();
        let first_b = hits.iter().position(|h| h.project == "proj-b").unwrap();
        assert!(
            last_a < first_b,
            "proj-a（新しい）が proj-b（古い）より前にマージされる"
        );

        // 単一プロジェクト指定（1 件 = 従来動作相当・日時降順）
        let only_a = run_search_projects(&projects[..1], &q).unwrap();
        assert!(only_a.iter().all(|h| h.project == "proj-b"));
    }
}
