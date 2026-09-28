//! Log 画面（旧右サイドバーのメイン画面移植）。
//!
//! 旧 `ui/right_sidebar.rs` の内容（`[Inputs] [Meta] [Output] [Stats]` の
//! 4 セクション構成）をメイン画面いっぱいに表示する。
//!
//! ## 旧右SB からの変更点（仕様書 §4.1）
//!
//! - **色使いは維持**: 見出し `[xxx]` は赤、START / END は緑、
//!   Session / waggle token は青、内容は白。
//! - **クリックアクションは廃止**（Session = `p`、waggle token = `o` ショートカットへ委任）。
//! - **セクション折りたたみは廃止**（常に全セクション展開）。
//! - スクロールは `LogState.scroll`（↑/↓/PgUp/PgDn / マウスホイール）で行う。
//! - 表示内容は `l` 押下時点で左SBカーソル位置にあった実行の詳細で**固定**される。

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Widget,
};
use unicode_width::UnicodeWidthChar;

use super::cjk;
use crate::fmt::{format_cost, format_num, format_tokens};
use crate::theme::AppTheme;

/// 表示用ブロック詳細データ（旧 right_sidebar::BlockDetail の移植）。
///
/// `phone run --view log` 用に JSON 化可能（項目名は snake_case のまま、
/// `input_fields` のタプルのみ `[{"label","value"}]` に変換）。
#[derive(Debug, Clone, Default)]
pub struct BlockDetail {
    /// 対象実行 ID（タイトル表示用）
    pub exec_id: i64,
    /// 対象ブロック名（タイトル表示用）
    pub block_name: String,
    /// この実行が user-block ブロックか（true のとき [Meta] / [Output] のみ表示）
    pub is_user_block: bool,
    /// user-block の説明（[Output] セクションで表示）
    pub user_block_description: Option<String>,
    /// user-block の入力ファイルパス（[Output] セクションで表示）
    pub user_block_file: Option<String>,
    /// 実行時のタグ（execution_history 優先、なければ block_metadata）
    pub tag: Option<String>,
    /// 実行時のラウンド（execution_history 優先、なければ block_metadata）
    pub round: Option<i64>,
    /// 実行時のグローバルラウンド（GR）
    pub global_round: Option<i64>,
    /// 実行時のモデル（execution_history 優先、なければ block_metadata）
    pub model: Option<String>,
    /// 実行時の thinking（execution_history 優先、なければ block_metadata）
    pub thinking: Option<String>,
    pub timeout: Option<i32>,
    pub connections_from: Vec<String>,
    pub connections_to: Vec<String>,
    /// この実行が出力した waggle トークン一覧（sai-write 使用時）
    pub output_waggle_tokens: Vec<String>,
    /// セッション統計情報
    pub session_stats: Option<SessionStatsData>,
    /// pi起動時のHEADコミットハッシュ
    pub git_commit_before: Option<String>,
    /// pi終了時のHEADコミットハッシュ
    pub git_commit_after: Option<String>,
    /// セッションディレクトリのパス
    pub session_dir: Option<String>,
    /// 実行開始日時（ISO 8601）
    pub started_at: Option<String>,
    /// 実行終了日時（ISO 8601）
    pub finished_at: Option<String>,
    /// 実行レコード作成日時（CREATE 表示用。DB `created_at`）
    pub created_at: Option<String>,
    /// 入力フィールド（ラベル, 値）一覧。[Inputs] セクションで表示する。
    pub input_fields: Vec<(String, String)>,
    /// 費用の表示行（B02。先頭付近に出す）。
    ///
    /// [`usage_lines`] が `usage_with_children` の結果から作り、Log 画面が
    /// そのままの行を描画する。空 = 実行の usage 記録が無い（描画しない）。
    pub usage_lines: Vec<String>,
}

/// セッション統計データ。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SessionStatsData {
    pub turn_count: u64,
    pub tool_call_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub thinking_tokens: u64,
    pub total_tokens: u64,
}

/// Log 画面に出す費用の表示行（B02）を作る。
///
/// 先頭付近に出す 1〜2 行（子がいない場合は 1 行）を返す。
///
/// ```text
/// 費用  $0.42（入力 120k・出力 8.1k・キャッシュ 300k）
/// 子を含めた合計  $1.35（子 3 件、うち費用不明 1 件）
/// ```
///
/// - `own` が `None`（usage 未保存）の場合は
///   `費用  記録なし（sai-link usage backfill で作れます）` の 1 行。
/// - 子がある場合のみ「子を含めた合計」の行を追加する。
/// - 子の費用不明件数は「usage 行が無い子」も含める（どちらも費用不明のため）。
/// - 費用・トークンの書式は [`crate::fmt::format_cost`] / [`crate::fmt::format_tokens`]。
pub fn usage_lines(summary: &sai_link_engine::usage::UsageSummary) -> Vec<String> {
    let mut lines = Vec::new();
    match &summary.own {
        Some(t) => lines.push(format!(
            "費用  {}（入力 {}・出力 {}・キャッシュ {}）",
            format_cost(t.cost),
            format_tokens(t.input),
            format_tokens(t.output),
            format_tokens(t.cache)
        )),
        None => lines.push(
            "費用  記録なし（sai-link usage backfill で作れます）".to_string(),
        ),
    }
    if !summary.children.is_empty() {
        let unknown = summary
            .children
            .iter()
            .filter(|(_, u)| u.as_ref().and_then(|t| t.cost).is_none())
            .count();
        let detail = if unknown > 0 {
            format!(
                "（子 {} 件、うち費用不明 {} 件）",
                summary.children.len(),
                unknown
            )
        } else {
            format!("（子 {} 件）", summary.children.len())
        };
        lines.push(format!(
            "子を含めた合計  {}{}",
            format_cost(summary.total.cost),
            detail
        ));
    }
    lines
}

/// Log 画面の状態。
#[derive(Debug, Clone, Default)]
pub struct LogState {
    /// 表示内容（`l` 押下時点の実行詳細で固定。None = 未選択）
    pub detail: Option<BlockDetail>,
    /// スクロール位置
    pub scroll: usize,
}

/// `BlockDetail` の `input_fields` 用（タプル → `{label, value}`）。
#[derive(serde::Serialize)]
struct InputRef<'a> {
    label: &'a str,
    value: &'a str,
}

/// `phone run --view log` 用の JSON 化（実装計画 12 §3.2）。
///
/// フィールド名は `BlockDetail` の snake_case をそのまま使い、
/// 描画用の内部状態は持たないため出力対象は全公開フィールド。
impl serde::Serialize for BlockDetail {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let len = 24;
        let mut st = serializer.serialize_struct("BlockDetail", len)?;
        st.serialize_field("exec_id", &self.exec_id)?;
        st.serialize_field("block_name", &self.block_name)?;
        st.serialize_field("is_user_block", &self.is_user_block)?;
        st.serialize_field("user_block_description", &self.user_block_description)?;
        st.serialize_field("user_block_file", &self.user_block_file)?;
        st.serialize_field("tag", &self.tag)?;
        st.serialize_field("round", &self.round)?;
        st.serialize_field("global_round", &self.global_round)?;
        st.serialize_field("model", &self.model)?;
        st.serialize_field("thinking", &self.thinking)?;
        st.serialize_field("timeout", &self.timeout)?;
        st.serialize_field("connections_from", &self.connections_from)?;
        st.serialize_field("connections_to", &self.connections_to)?;
        st.serialize_field("output_waggle_tokens", &self.output_waggle_tokens)?;
        st.serialize_field("session_stats", &self.session_stats)?;
        st.serialize_field("git_commit_before", &self.git_commit_before)?;
        st.serialize_field("git_commit_after", &self.git_commit_after)?;
        st.serialize_field("session_dir", &self.session_dir)?;
        st.serialize_field("started_at", &self.started_at)?;
        st.serialize_field("finished_at", &self.finished_at)?;
        st.serialize_field("created_at", &self.created_at)?;
        let input_fields: Vec<InputRef<'_>> = self
            .input_fields
            .iter()
            .map(|(label, value)| InputRef { label, value })
            .collect();
        st.serialize_field("input_fields", &input_fields)?;
        st.serialize_field("usage_lines", &self.usage_lines)?;
        st.end()
    }
}

/// Log 画面ウィジェット（旧右SB の描画をメイン画面サイズに移植）。
pub struct LogView<'a> {
    pub state: &'a mut LogState,
    pub theme: &'a AppTheme,
}

/// Log 画面のセクション種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogSection {
    Inputs,
    Meta,
    Stats,
    Connection,
}

impl LogSection {
    /// 全セクション（描画順）
    const ALL: [LogSection; 4] = [
        LogSection::Inputs,
        LogSection::Meta,
        LogSection::Stats,
        LogSection::Connection,
    ];

    fn title(self) -> &'static str {
        match self {
            LogSection::Inputs => "[Inputs]",
            LogSection::Meta => "[Meta]",
            LogSection::Stats => "[Stats]",
            LogSection::Connection => "[Connection]",
        }
    }
}

impl Widget for LogView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = self.theme.main_bg;
        let fg = self.theme.sidebar_fg;

        // 背景
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(Style::default().bg(bg).fg(fg));
                }
            }
        }

        let Some(detail) = self.state.detail.clone() else {
            // 未選択時の案内表示
            cjk::draw_text_cjk(
                buf,
                area.x + 1,
                area.y + 1,
                "Log — ブロックが選択されていません。Workflow でブロックを選択して `l` を押してください。",
                Style::default().fg(self.theme.muted).bg(bg),
            );
            return;
        };

        // スクロール上限をクランプ（コンテンツが可視領域より短い場合は先頭に戻す）
        let visible = area.height.saturating_sub(3) as usize;
        let total = total_content_height(&detail, area);
        if total <= visible {
            self.state.scroll = 0;
        } else {
            self.state.scroll = self.state.scroll.min(total - visible);
        }

        // タイトル
        let title = format!(" Log — ID{} / {}", detail.exec_id, detail.block_name);
        cjk::draw_text_cjk(
            buf,
            area.x + 1,
            area.y + 1,
            &title,
            Style::default().fg(self.theme.info).bg(bg),
        );
        // セパレータ
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, area.y + 2)) {
                cell.set_char('─')
                    .set_style(Style::default().fg(Color::Rgb(60, 60, 80)));
            }
        }

        // コンテンツ先頭行: タイトル（2行）の下 = area.y + 3
        let mut y = i64::from(area.y) + 3 - self.state.scroll as i64;

        // B02: 費用の行を先頭付近（セクションより上）に出す。
        // 記録が無い（空）実行では行を作らないので、従来の表示は変わらない。
        if !detail.usage_lines.is_empty() {
            let style = Style::default().fg(self.theme.info).bg(bg);
            for line in &detail.usage_lines {
                y += draw_wrapped_row(buf, area, area.x + 1, y, line, style) as i64;
            }
        }

        // ── セクション（[xxx] は赤。折りたたみなし・クリックなし）──
        // user-block は入力可能な項目（START / END / Commit / Session）のみ =
        // [Meta] と [Output] を表示する（旧右SB と同じ）。
        let user_block = detail.is_user_block;
        let header_color = self.theme.error;
        for section in LogSection::ALL {
            if user_block && section != LogSection::Meta && section != LogSection::Connection {
                continue;
            }
            draw_section_header(buf, area, &mut y, section.title(), header_color, bg);
            match section {
                LogSection::Inputs => render_inputs_section(&detail, area, buf, &mut y, bg),
                LogSection::Meta => {
                    render_meta_section(&detail, area, buf, &mut y, bg, &self.theme)
                }
                LogSection::Stats => {
                    render_stats_section(&detail, area, buf, &mut y, bg, &self.theme)
                }
                LogSection::Connection => {
                    render_connection_section(&detail, area, buf, &mut y, bg, &self.theme)
                }
            }
        }
    }
}

// ============================================================================
// ビルダ純関数（DB → 表示データ変換）
// ============================================================================

/// 実行 1 件分の表示用詳細データを DB から構築する（ビルダ純関数）。
///
/// sai-link-tui `app/data.rs::build_block_detail()` 本体を分離したもの。
/// `sai_control_tui` 等の第 2 クライアントからも同一ビルダを使い、同一画面を
/// 表示できるようにする。UI 側の副作用（選択実行 ID の覚え置き等）は
/// 呼び出し側（各 TUI）が行う。
///
/// - `store`: 対象プロジェクトの `SaiStore`（read-only でも可）
/// - `exec_id`: 対象実行 ID
///
/// ブロック定義が見つからない場合は `None`（実行レコードのみ見つからない場合は
/// sai-link と同じ `block_id = exec_id` フォールバックを適用する）。
pub fn build(store: &sai_link_engine::db::SaiStore, exec_id: i64) -> Option<BlockDetail> {
    let block_id = match store.get_execution_by_id(exec_id) {
        Ok(exec) => exec.block_id,
        Err(_) => exec_id,
    };

    let output_tokens = get_waggle_tokens_for_execution(store, exec_id);
    let session_stats = get_session_stats_for_execution(store, exec_id);
    let exec = store.get_execution_by_id(exec_id).ok();

    let block = store.get_block_by_id(block_id).ok()?;

    // user-block ブロックかどうか（[Meta] / [Output] のみ表示するため）
    let is_user_block = block.display_name == "user-block";

    // 接続情報
    let format_block_ref = |bid: i64, eid: i64| -> String {
        if let Ok(b) = store.get_block_by_id(bid) {
            // 接続先の実行レコードから GR/LR を取得（なければブロック定義の LR）
            let (gr, lr) = match store.get_execution_by_id(eid) {
                Ok(e) => (e.global_round, e.round.or(b.round)),
                Err(_) => (None, b.round),
            };
            let gr_str = gr.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string());
            let lr_str = lr.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string());
            format!("R{}.{}/ID{}", gr_str, lr_str, eid)
        } else {
            format!("R?.?/ID{}/id:{}", eid, bid)
        }
    };

    let all_connections = store
        .list_connections_by_block(block_id)
        .ok()
        .unwrap_or_default();

    // 接続 1 件分の表示行を整形する。
    // waggle_token 接続はトークンを先頭に付与し、
    // task_delegation / task_output（親子）は型ラベルを付ける。
    // 線は親子と waggle の受け渡しだけ（review / kanban_origin / git_commit は出さない）。
    let format_conn_line = |c: &sai_link_engine::db::models::BlockConnection,
                            other_bid: i64,
                            other_eid: i64|
     -> String {
        let ref_str = format_block_ref(other_bid, other_eid);
        match c.connection_type.as_str() {
            "waggle_token" => match &c.waggle_token {
                Some(t) => format!("{} {}", t, ref_str),
                None => ref_str,
            },
            "task_delegation" => format!("[task] {}", ref_str),
            "task_output" => format!("[out] {}", ref_str),
            _ => ref_str,
        }
    };

    let is_shown = |c: &&sai_link_engine::db::models::BlockConnection| {
        matches!(
            c.connection_type.as_str(),
            "waggle_token" | "task_delegation" | "task_output"
        )
    };
    let from: Vec<String> = all_connections
        .iter()
        .filter(|c| c.target_execution_id == exec_id && is_shown(c))
        .map(|c| format_conn_line(c, c.source_block_id, c.source_execution_id))
        .collect();

    let to: Vec<String> = all_connections
        .iter()
        .filter(|c| c.source_execution_id == exec_id && is_shown(c))
        .map(|c| format_conn_line(c, c.target_block_id, c.target_execution_id))
        .collect();

    let tag = exec
        .as_ref()
        .and_then(|e| e.tag.clone())
        .or(block.tag.clone());
    let round = exec.as_ref().and_then(|e| e.round).or(block.round);
    let global_round = exec.as_ref().and_then(|e| e.global_round);
    let model = exec
        .as_ref()
        .and_then(|e| e.model.clone())
        .or(Some(block.model.clone()));
    let thinking = exec
        .as_ref()
        .and_then(|e| e.thinking.clone())
        .or(Some(block.thinking.clone()));
    let timeout_override = exec
        .as_ref()
        .and_then(|e| e.timeout)
        .or(Some(block.timeout));

    let git_commit_before = exec.as_ref().and_then(|e| e.git_commit_before.clone());
    let git_commit_after = exec.as_ref().and_then(|e| e.git_commit_after.clone());
    let session_dir = exec.as_ref().and_then(|e| e.session_dir.clone());
    let started_at = exec.as_ref().and_then(|e| e.started_at.clone());
    let finished_at = exec.as_ref().and_then(|e| e.finished_at.clone());
    let created_at = exec.as_ref().and_then(|e| {
        let c = e.created_at.trim();
        if c.is_empty() {
            None
        } else {
            Some(c.to_string())
        }
    });

    // 入力フィールド（[Inputs] 用）: user_inputs_json の値を
    // ブロック定義の入力フィールド順（ラベル付き）で並べる
    let input_fields: Vec<(String, String)> = {
        let mut pairs: Vec<(String, String)> = Vec::new();
        if let Some(json) = exec.as_ref().and_then(|e| e.user_inputs_json.as_deref()) {
            if let Ok(map) =
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json)
            {
                let mut used_keys: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                // ブロック定義（block_inputs）のフィールド順でラベルを付ける
                if let Ok(defs) = store.get_inputs_by_block_id(block_id) {
                    for inp in defs {
                        if let Some(v) = map.get(&inp.input_id) {
                            pairs.push((inp.label.clone(), json_value_to_string(v)));
                            used_keys.insert(inp.input_id.clone());
                        }
                    }
                }
                // 定義にないキーは末尾に追加
                for (k, v) in map.iter() {
                    if !used_keys.contains(k) {
                        pairs.push((k.clone(), json_value_to_string(v)));
                    }
                }
            }
        }
        pairs
    };

    // user-block の説明・入力ファイル（[Output] セクションで表示）
    let (user_block_description, user_block_file) = {
        let mut desc = None;
        let mut file = None;
        if let Some(json) = exec.as_ref().and_then(|e| e.user_inputs_json.as_deref()) {
            if let Ok(map) =
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json)
            {
                if let Some(v) = map.get("description") {
                    desc = v.as_str().map(|s| s.to_string());
                }
                if let Some(v) = map.get("file_path") {
                    file = v.as_str().map(|s| s.to_string());
                }
            }
        }
        (desc, file)
    };

    let block_name = block.display_name.clone();
    // B02: 費用の表示行（`usage_with_children` から。記録が無ければ空でも良い）。
    // 費用不明（記録あり・cost 無し）と記録無しは `usage_lines` 側で区別する。
    let cost_lines = sai_link_engine::usage::usage_with_children(store, exec_id)
        .map(|s| usage_lines(&s))
        .unwrap_or_default();
    Some(BlockDetail {
        exec_id,
        block_name,
        is_user_block,
        user_block_description,
        user_block_file,
        tag,
        round,
        global_round,
        model,
        thinking,
        timeout: timeout_override,
        connections_from: from,
        connections_to: to,
        output_waggle_tokens: output_tokens,
        input_fields,
        usage_lines: cost_lines,
        session_stats,
        git_commit_before,
        git_commit_after,
        session_dir,
        started_at,
        finished_at,
        created_at,
    })
}

/// 実行レコードから出力 waggle トークン一覧を取得する。
fn get_waggle_tokens_for_execution(
    store: &sai_link_engine::db::SaiStore,
    exec_id: i64,
) -> Vec<String> {
    store
        .get_execution_by_id(exec_id)
        .ok()
        .and_then(|exec| {
            exec.output_waggle_tokens
                .as_deref()
                .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
        })
        .unwrap_or_default()
}

/// 実行レコードからセッション統計を取得する（Log 画面用・近似値）。
fn get_session_stats_for_execution(
    store: &sai_link_engine::db::SaiStore,
    exec_id: i64,
) -> Option<SessionStatsData> {
    let exec = store.get_execution_by_id(exec_id).ok()?;
    let json_str = exec.session_stats_json.as_deref()?;
    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;

    Some(SessionStatsData {
        turn_count: parsed.get("turn_count")?.as_u64().unwrap_or(0),
        tool_call_count: parsed.get("tool_call_count")?.as_u64().unwrap_or(0),
        input_tokens: parsed.get("input_tokens")?.as_u64().unwrap_or(0),
        output_tokens: parsed.get("output_tokens")?.as_u64().unwrap_or(0),
        thinking_tokens: parsed.get("thinking_tokens")?.as_u64().unwrap_or(0),
        total_tokens: parsed.get("total_tokens")?.as_u64().unwrap_or(0),
    })
}

/// JSON 値を表示用文字列に変換する（文字列は引用符なし）。
fn json_value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "-".to_string(),
        other => other.to_string(),
    }
}

// ============================================================================
// セクション描画（旧 right_sidebar.rs から移植・色使い維持）
// ============================================================================

/// [Inputs] セクションの内容を描画し、`y` を進める（スクロール対応）。
///
/// 入力テキスト（`task_text` 等の実行時入力値）は表示しない。
/// `Global Round` から始まる実行フォーム項目のみを表示する。
fn render_inputs_section(d: &BlockDetail, area: Rect, buf: &mut Buffer, y: &mut i64, bg: Color) {
    let white = Color::White;

    // 入力フォームで指定可能な項目（実行値 or ブロック定義のデフォルト）。
    // 先頭は `Global Round`。入力テキスト（input_fields）は表示しない。
    const PAD: usize = 12;
    let form_lines: Vec<(&str, String)> = vec![
        (
            "Global Round",
            d.global_round
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ),
        (
            "Round",
            d.round
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ),
        ("Tag", d.tag.clone().unwrap_or_else(|| "-".to_string())),
        (
            "Think",
            d.thinking.clone().unwrap_or_else(|| "-".to_string()),
        ),
        ("Model", d.model.clone().unwrap_or_else(|| "-".to_string())),
        (
            "Timeout",
            d.timeout.map_or("-".to_string(), |v| format!("{}s", v)),
        ),
    ];
    for (label, value) in &form_lines {
        if *y >= i64::from(area.bottom()) {
            break;
        }
        let text = format!(" {:<PAD$}: {}", label, value, PAD = PAD);
        *y += draw_wrapped_row(
            buf,
            area,
            area.x + 1,
            *y,
            &text,
            Style::default().fg(white).bg(bg),
        ) as i64;
    }
}

/// [Meta] セクションの内容を描画し、`y` を進める（スクロール対応）。
///
/// `<項目>: <コンテンツ>` 形式で、項目ラベル幅を揃える。
/// CREATE（実行レコード作成日時）/ START / END は緑、
/// Waggle・Session は青、その他は白（旧右SB の配色維持）。
fn render_meta_section(
    d: &BlockDetail,
    area: Rect,
    buf: &mut Buffer,
    y: &mut i64,
    bg: Color,
    theme: &AppTheme,
) {
    let green = theme.success;
    let white = Color::White;
    let link = theme.link;

    // 項目ラベル幅（`Waggle ` に合わせる）
    const PAD: usize = 8;
    let row = |label: &str, value: &str| format!(" {:<PAD$}: {}", label, value);

    let created = d
        .created_at
        .as_deref()
        .map(format_datetime_short)
        .unwrap_or_else(|| "-".to_string());
    let start = d
        .started_at
        .as_deref()
        .map(format_datetime_short)
        .unwrap_or_else(|| "-".to_string());
    let end = d
        .finished_at
        .as_deref()
        .map(format_datetime_short)
        .unwrap_or_else(|| "-".to_string());
    for (label, value) in [
        ("CREATE", created.as_str()),
        ("START", start.as_str()),
        ("END", end.as_str()),
    ] {
        if *y >= i64::from(area.bottom()) {
            break;
        }
        *y += draw_wrapped_row(
            buf,
            area,
            area.x + 1,
            *y,
            &row(label, value),
            Style::default().fg(green).bg(bg),
        ) as i64;
    }

    // Commit before / after（白）
    let cb = d
        .git_commit_before
        .clone()
        .unwrap_or_else(|| "-".to_string());
    let ca = d
        .git_commit_after
        .clone()
        .unwrap_or_else(|| "-".to_string());
    for (label, value) in [("Commit", cb.as_str()), ("Commit", ca.as_str())] {
        if *y >= i64::from(area.bottom()) {
            break;
        }
        *y += draw_wrapped_row(
            buf,
            area,
            area.x + 1,
            *y,
            &row(label, value),
            Style::default().fg(white).bg(bg),
        ) as i64;
    }

    // Waggle（青・旧 Waggle Out を [Meta] に統合）
    if d.output_waggle_tokens.is_empty() {
        if *y < i64::from(area.bottom()) {
            *y += draw_wrapped_row(
                buf,
                area,
                area.x + 1,
                *y,
                &row("Waggle", "(none)"),
                Style::default().fg(theme.muted).bg(bg),
            ) as i64;
        }
    } else {
        for token in &d.output_waggle_tokens {
            if *y >= i64::from(area.bottom()) {
                break;
            }
            *y += draw_wrapped_row(
                buf,
                area,
                area.x + 1,
                *y,
                &row("Waggle", token),
                Style::default().fg(link).bg(bg),
            ) as i64;
        }
    }

    // Session（青・共通プレフィクス除去の短縮名のみ）
    if let Some(ref session_dir) = d.session_dir {
        if *y < i64::from(area.bottom()) {
            let display = format_session_short(session_dir);
            *y += draw_wrapped_row(
                buf,
                area,
                area.x + 1,
                *y,
                &row("Session", &display),
                Style::default().fg(link).bg(bg),
            ) as i64;
        }
    }
}

/// [Connection] セクションの内容を描画し、`y` を進める（スクロール対応）。
///
/// 2区分（from/waggle・to/waggle）で表示する。
/// 項目ラベル幅を揃える。空の区分は `(none)` を出す。
fn render_connection_section(
    d: &BlockDetail,
    area: Rect,
    buf: &mut Buffer,
    y: &mut i64,
    bg: Color,
    theme: &AppTheme,
) {
    let white = Color::White;
    let muted = theme.muted;

    // 項目ラベル幅（`from/waggle` に合わせる）
    const PAD: usize = 11;
    let row = |label: &str, value: &str| format!(" {:<PAD$}: {}", label, value);
    let groups: [(&str, Vec<&str>); 2] = [
        ("from/waggle", d.connections_from.iter().map(String::as_str).collect()),
        ("to/waggle", d.connections_to.iter().map(String::as_str).collect()),
    ];
    // user-block: 説明・入力ファイルを先頭に表示
    if d.is_user_block {
        if let Some(ref desc) = d.user_block_description {
            if *y >= i64::from(area.bottom()) {
                return;
            }
            *y += draw_wrapped_row(
                buf,
                area,
                area.x + 1,
                *y,
                &format!(" {:<PAD$}: {}", "desc", desc),
                Style::default().fg(white).bg(bg),
            ) as i64;
        }
        if let Some(ref file) = d.user_block_file {
            if *y >= i64::from(area.bottom()) {
                return;
            }
            *y += draw_wrapped_row(
                buf,
                area,
                area.x + 1,
                *y,
                &format!(" {:<PAD$}: {}", "file", file),
                Style::default().fg(white).bg(bg),
            ) as i64;
        }
    }

    for (label, items) in groups {
        if *y >= i64::from(area.bottom()) {
            break;
        }
        if items.is_empty() {
            *y += draw_wrapped_row(
                buf,
                area,
                area.x + 1,
                *y,
                &row(label, "(none)"),
                Style::default().fg(muted).bg(bg),
            ) as i64;
        } else {
            for item in items {
                if *y >= i64::from(area.bottom()) {
                    break;
                }
                *y += draw_wrapped_row(
                    buf,
                    area,
                    area.x + 1,
                    *y,
                    &row(label, item),
                    Style::default().fg(white).bg(bg),
                ) as i64;
            }
        }
    }
}

/// サブラベル（白・太字）を描画し、`y` を進める。
fn draw_sub_label(area: Rect, buf: &mut Buffer, y: &mut i64, text: &str, bg: Color) {
    if *y < i64::from(area.bottom()) {
        draw_row(
            buf,
            area,
            area.x + 1,
            *y,
            text,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
                .bg(bg),
        );
    }
    *y += 1;
}

/// [Stats] セクションの内容を描画し、`y` を進める。
///
/// トークン数などの統計のみを表示する（Model は表示しない）。
fn render_stats_section(
    d: &BlockDetail,
    area: Rect,
    buf: &mut Buffer,
    y: &mut i64,
    bg: Color,
    theme: &AppTheme,
) {
    let white = Color::White;
    let muted = theme.muted;

    if let Some(ref stats) = d.session_stats {
        let lines = vec![
            ("Turn count:", stats.turn_count.to_string()),
            ("Tool calls:", stats.tool_call_count.to_string()),
            ("Tokens (in):", format_num(stats.input_tokens)),
            ("Tokens (out):", format_num(stats.output_tokens)),
            ("Tokens (think):", format_num(stats.thinking_tokens)),
            ("Tokens (total):", format_num(stats.total_tokens)),
        ];

        for (label, value) in &lines {
            if *y >= i64::from(area.bottom()) {
                break;
            }
            let text = format!(" {:<14}: {}", label.trim_end_matches(':'), value);
            *y += draw_wrapped_row(
                buf,
                area,
                area.x + 1,
                *y,
                &text,
                Style::default().fg(white).bg(bg),
            ) as i64;
        }
    } else {
        *y += draw_wrapped_row(
            buf,
            area,
            area.x + 1,
            *y,
            "  No session stats available.",
            Style::default().fg(muted).bg(bg),
        ) as i64;
    }
}

// ============================================================================
// 描画ヘルパー（旧 right_sidebar.rs から移植）
// ============================================================================

/// スクロール対応の行描画。領域の外（上下）の行は描画しない。
fn draw_row(buf: &mut Buffer, area: Rect, x: u16, y: i64, text: &str, style: Style) {
    if y < i64::from(area.top()) || y >= i64::from(area.bottom()) {
        return;
    }
    cjk::draw_text_cjk(buf, x, y as u16, text, style);
}

/// 折り返し対応の行描画。
///
/// パネル幅（`area` の右端）を超える文字は次の行に折り返して描画する。
/// CJK 全角文字は 2 セルとして扱う。戻り値は消費した行数（折り返しを含む）。
fn draw_wrapped_row(
    buf: &mut Buffer,
    area: Rect,
    x: u16,
    y: i64,
    text: &str,
    style: Style,
) -> usize {
    let max_width = area.right().saturating_sub(x) as usize;
    if max_width == 0 {
        return 0;
    }

    let mut wrap_count = 0usize;
    let mut width_used = 0usize;
    let mut cx = x;
    let mut cy = y;

    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(1) as usize;
        if w == 0 {
            continue; // 結合文字はスキップ
        }
        if width_used + w > max_width {
            // パネル幅を超える → 次の行へ折り返し
            wrap_count += 1;
            cy += 1;
            width_used = 0;
            cx = x;
        }
        // 可視行のみ描画（スクロールで上下の外にある行はスキップ）
        if cy >= i64::from(area.top()) && cy < i64::from(area.bottom()) {
            cjk::draw_char_cjk(buf, cx, cy as u16, ch, style);
        }
        cx += w as u16;
        width_used += w;
    }

    wrap_count + 1
}

/// テキストが与えられた幅で何行に折り返されるかを返す（CJK 全角= 2 セル）。
fn wrapped_line_count(text: &str, max_width: usize) -> usize {
    if max_width == 0 {
        return 1;
    }
    let mut width_used = 0usize;
    let mut lines = 1usize;
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(1) as usize;
        if w == 0 {
            continue;
        }
        if width_used + w > max_width {
            lines += 1;
            width_used = w;
        } else {
            width_used += w;
        }
    }
    lines
}

/// セクションタイトルのみを描画する（スクロール対応）。行カウンタは常に進める。
fn draw_section_header(
    buf: &mut Buffer,
    area: Rect,
    y: &mut i64,
    title: &str,
    fg: Color,
    bg: Color,
) {
    if *y < i64::from(area.bottom()) {
        draw_row(
            buf,
            area,
            area.x + 1,
            *y,
            title,
            Style::default().fg(fg).bg(bg),
        );
    }
    *y += 1;
}

/// 全セクションのコンテンツ総行数（スクロール上限の計算用）。
///
/// 各セクション見出し 1 行 + 内容（折り返し考慮）の合計。折りたたみはない。
fn total_content_height(d: &BlockDetail, area: Rect) -> usize {
    // 内容幅: 左余白 1 セル分を除いたパネル内幅
    let width = area.right().saturating_sub(area.x + 1) as usize;
    // B02: 先頭付近の費用行（折り返し考慮。空なら 0）
    let usage_h: usize = d
        .usage_lines
        .iter()
        .map(|line| wrapped_line_count(line, width))
        .sum();
    // user-block は [Meta] と [Output] セクションのみ
    if d.is_user_block {
        return usage_h + 1 + meta_content_height(d, width) + 1 + connection_content_height(d, width);
    }
    usage_h
        + 1
        + inputs_content_height(d, width)
        + 1
        + meta_content_height(d, width)
        + 1
        + stats_content_height(d)
        + 1
        + connection_content_height(d, width)
}

/// [Inputs] セクションのコンテンツ行数（折り返し考慮）。
///
/// 入力テキスト（input_fields）は表示しないため、フォーム項目 7 行のみ。
fn inputs_content_height(d: &BlockDetail, width: usize) -> usize {
    let mut h = 0usize;
    const PAD: usize = 12;
    let labels = [
        "Global Round",
        "Round",
        "Tag",
        "Think",
        "Model",
        "Timeout",
    ];
    let values = [
        d.global_round
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string()),
        d.round
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string()),
        d.tag.clone().unwrap_or_else(|| "-".to_string()),
        d.thinking.clone().unwrap_or_else(|| "-".to_string()),
        d.model.clone().unwrap_or_else(|| "-".to_string()),
        d.timeout.map_or("-".to_string(), |v| format!("{}s", v)),
    ];
    for (label, value) in labels.iter().zip(values.iter()) {
        h += wrapped_line_count(&format!(" {:<PAD$}: {}", label, value, PAD = PAD), width);
    }
    h
}

/// [Meta] セクションのコンテンツ行数（折り返し考慮）。
fn meta_content_height(d: &BlockDetail, width: usize) -> usize {
    const PAD: usize = 8;
    let row = |label: &str, value: &str| format!(" {:<PAD$}: {}", label, value);
    let mut h = 0usize;
    let created = d
        .created_at
        .as_deref()
        .map(format_datetime_short)
        .unwrap_or_else(|| "-".to_string());
    let start = d
        .started_at
        .as_deref()
        .map(format_datetime_short)
        .unwrap_or_else(|| "-".to_string());
    let end = d
        .finished_at
        .as_deref()
        .map(format_datetime_short)
        .unwrap_or_else(|| "-".to_string());
    h += wrapped_line_count(&row("CREATE", &created), width);
    h += wrapped_line_count(&row("START", &start), width);
    h += wrapped_line_count(&row("END", &end), width);
    let cb = d
        .git_commit_before
        .clone()
        .unwrap_or_else(|| "-".to_string());
    let ca = d
        .git_commit_after
        .clone()
        .unwrap_or_else(|| "-".to_string());
    h += wrapped_line_count(&row("Commit", &cb), width);
    h += wrapped_line_count(&row("Commit", &ca), width);
    if d.output_waggle_tokens.is_empty() {
        h += wrapped_line_count(&row("Waggle", "(none)"), width);
    } else {
        for token in &d.output_waggle_tokens {
            h += wrapped_line_count(&row("Waggle", token), width);
        }
    }
    if let Some(ref session_dir) = d.session_dir {
        h += wrapped_line_count(&row("Session", &format_session_short(session_dir)), width);
    }
    h
}

/// [Connection] セクションのコンテンツ行数（折り返し考慮）。
fn connection_content_height(d: &BlockDetail, width: usize) -> usize {
    const PAD: usize = 11;
    let row = |label: &str, value: &str| format!(" {:<PAD$}: {}", label, value);
    let mut h = 0usize;
    if d.is_user_block {
        if let Some(ref desc) = d.user_block_description {
            h += wrapped_line_count(&format!(" {:<PAD$}: {}", "desc", desc), width);
        }
        if let Some(ref file) = d.user_block_file {
            h += wrapped_line_count(&format!(" {:<PAD$}: {}", "file", file), width);
        }
    }
    let groups = [("from/waggle", &d.connections_from), ("to/waggle", &d.connections_to)];
    for (label, items) in groups {
        if items.is_empty() {
            h += wrapped_line_count(&row(label, "(none)"), width);
        } else {
            for item in items {
                h += wrapped_line_count(&row(label, item), width);
            }
        }
    }
    h
}

/// [Output] セクションのコンテンツ行数（折り返し考慮）。
fn output_content_height(d: &BlockDetail, width: usize) -> usize {
    let mut h = 0usize;
    // user-block: 説明・入力ファイルの行数
    if d.is_user_block {
        if let Some(ref desc) = d.user_block_description {
            h += wrapped_line_count(&format!(" 説明: {}", desc), width);
        }
        if let Some(ref file) = d.user_block_file {
            h += wrapped_line_count(&format!(" 入力ファイル: {}", file), width);
        }
    }
    // Waggle Out（ラベル 1 行 + トークン or "(none)"）
    h += 1;
    if d.output_waggle_tokens.is_empty() {
        h += 1;
    } else {
        for token in &d.output_waggle_tokens {
            h += wrapped_line_count(&format!("   {}", token), width);
        }
    }
    // Connection from
    h += 1;
    if d.connections_from.is_empty() {
        h += 1;
    } else {
        for conn in d.connections_from.iter() {
            h += wrapped_line_count(&format!("   {}", conn), width);
        }
    }
    // Connection to
    h += 1;
    if d.connections_to.is_empty() {
        h += 1;
    } else {
        for conn in d.connections_to.iter() {
            h += wrapped_line_count(&format!("   {}", conn), width);
        }
    }
    h
}

/// [Stats] セクションのコンテンツ行数。
fn stats_content_height(d: &BlockDetail) -> usize {
    if d.session_stats.is_some() {
        6
    } else {
        1
    }
}

/// セッションパスからブロック名とタイムスタンプを抽出して短く表示する。
fn format_session_display(session_dir: &str) -> String {
    let session_name = session_dir
        .trim_start_matches(".sai/sessions/")
        .trim_start_matches("/tmp/");
    // "pi-sessions-" プレフィックスを除去
    let short = session_name
        .strip_prefix("pi-sessions-")
        .unwrap_or(session_name);
    // タイムスタンプ部分 (YYYYMMDDTHHMMSS) と残りを分離
    if short.len() >= 15 {
        let ts = &short[..15]; // YYYYMMDDTHHMMSS
        let rest = &short[16..]; // {block_name}-{random}
                                 // 日付を読みやすく: 20260801T124258 → 08/01 12:42:58
        let formatted_ts = if ts.len() == 15 {
            format!(
                "{}/{}/{} {}:{}:{}",
                &ts[0..4],
                &ts[4..6],
                &ts[6..8],
                &ts[9..11],
                &ts[11..13],
                &ts[13..15]
            )
        } else {
            ts.to_string()
        };
        format!("{} @ {}", rest, formatted_ts)
    } else {
        short.to_string()
    }
}

/// session_dir から表示用の短縮名を取り出す。
///
/// 現行正本 `outputs/<owner>/<run_id>/session/` は owner 移動で変わるため、
/// `<owner>/<run_id>` を残す（例: `running/40`）。旧 `.sai/sessions/` と
/// `/tmp/`・`pi-sessions-` プレフィクスは除去し、末尾要素のみ返す。
fn format_session_short(session_dir: &str) -> String {
    // 現行正本: `.../outputs/<owner>/<run_id>/session` → `<owner>/<run_id>`
    if let Some(pos) = session_dir.find(".sai/outputs/") {
        let rest = &session_dir[pos + ".sai/outputs/".len()..];
        let mut parts = rest.split('/');
        if let (Some(owner), Some(run)) = (parts.next(), parts.next()) {
            if !owner.is_empty() && !run.is_empty() {
                return format!("{owner}/{run}");
            }
        }
    }
    // 絶対パス内の `outputs/<owner>/<run_id>/session` にも対応する。
    if let Some(pos) = session_dir.find("/outputs/") {
        let rest = &session_dir[pos + "/outputs/".len()..];
        let mut parts = rest.split('/');
        if let (Some(owner), Some(run)) = (parts.next(), parts.next()) {
            if !owner.is_empty() && !run.is_empty() {
                return format!("{owner}/{run}");
            }
        }
    }
    let name = session_dir
        .trim_start_matches(".sai/sessions/")
        .trim_start_matches("/tmp/");
    let short = name.strip_prefix("pi-sessions-").unwrap_or(name);
    short.rsplit('/').next().unwrap_or(short).to_string()
}

/// ISO 8601 形式の日時を `YYYY/MM/DD HH:MM:SS` 形式に変換する。
fn format_datetime_short(iso: &str) -> String {
    if iso.len() >= 19 {
        let mut result = iso.to_string();
        if let Some(t_pos) = result.find('T') {
            result.replace_range(t_pos..=t_pos, " ");
        }
        if result.len() > 19 {
            result.truncate(19);
        }
        result.replace('-', "/")
    } else {
        iso.to_string()
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer as TuiBuffer;

    /// テスト用の in-memory store（ブロック 1 件）を作成する。
    fn setup_store(block_name: &str) -> (sai_link_engine::db::SaiStore, i64) {
        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let block = sai_link_engine::db::models::BlockMetadata::new(
            block_name.to_string(),
            "テスト用ブロック".to_string(),
            "anthropic/claude-sonnet-4-5".to_string(),
            None,
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            format!("blocks/{}.md", block_name),
            None
        );
        let block_id = store.insert_block(&block).unwrap();
        (store, block_id)
    }

    /// 存在しない実行 ID では None を返すこと。
    #[test]
    fn build_returns_none_for_missing_execution() {
        let (store, _block_id) = setup_store("plan-ice");
        assert!(build(&store, 999).is_none());
    }

    /// 通常ブロック（接続 0 件・入力なし）の詳細が構築されること。
    #[test]
    fn build_ordinary_block_with_no_connections() {
        let (store, block_id) = setup_store("plan-ice");
        let exec_id = store.create_execution(block_id, None).unwrap();

        let d = build(&store, exec_id).expect("詳細が構築されること");
        assert_eq!(d.exec_id, exec_id);
        assert_eq!(d.block_name, "plan-ice");
        assert!(!d.is_user_block, "通常ブロックは user-block ではない");
        assert!(d.connections_from.is_empty(), "接続 0 件");
        assert!(d.connections_to.is_empty(), "接続 0 件");
        assert!(d.output_waggle_tokens.is_empty());
        assert!(d.input_fields.is_empty(), "入力フィールドなし");
        assert!(d.session_stats.is_none());
        // ブロック定義の既定値がフォールバックで入ること
        assert_eq!(d.model.as_deref(), Some("anthropic/claude-sonnet-4-5"));
        assert_eq!(d.thinking.as_deref(), Some("off"));
        assert_eq!(d.timeout, Some(300));
    }

    /// user-block は is_user_block = true となり、説明・入力ファイルが
    /// user_inputs_json から抽出されること。
    #[test]
    fn build_user_block_extracts_description_and_file() {
        let (store, block_id) = setup_store("user-block");
        let json = r#"{"description":"ユーザーの説明","file_path":"docs/memo.md"}"#;
        let exec_id = store.create_execution(block_id, Some(json)).unwrap();

        let d = build(&store, exec_id).unwrap();
        assert!(d.is_user_block, "user-block ブロック名なら true");
        assert_eq!(d.user_block_description.as_deref(), Some("ユーザーの説明"));
        assert_eq!(d.user_block_file.as_deref(), Some("docs/memo.md"));
    }

    /// 入力フィールド定義（block_inputs）がある場合、user_inputs_json の値が
    /// ラベル付きで input_fields に並ぶこと。定義にないキーは末尾に追加される。
    #[test]
    fn build_with_input_fields() {
        let (store, block_id) = setup_store("with-inputs");
        let inp = sai_link_engine::db::models::BlockInput::new(
            block_id,
            "question".to_string(),
            "text".to_string(),
            "質問".to_string(),
            None,
            true,
            None,
            None,
            None,
            None,
        );
        store.insert_inputs(block_id, &[inp]).unwrap();
        let json = r#"{"question":"確認テスト","extra":"追加項目"}"#;
        let exec_id = store.create_execution(block_id, Some(json)).unwrap();

        let d = build(&store, exec_id).unwrap();
        assert_eq!(d.block_name, "with-inputs");
        // 定義順（ラベル付き）で最初に並ぶ
        assert_eq!(
            d.input_fields[0],
            ("質問".to_string(), "確認テスト".to_string())
        );
        // 定義にないキーは末尾に追加される
        assert_eq!(
            d.input_fields[1],
            ("extra".to_string(), "追加項目".to_string())
        );
    }

    fn sample_detail() -> BlockDetail {
        BlockDetail {
            exec_id: 1,
            block_name: "plan-ice".to_string(),
            tag: Some("test".into()),
            ..Default::default()
        }
    }

    fn render(state: &mut LogState, w: u16, h: u16) -> TuiBuffer {
        let theme = AppTheme::dark();
        let area = Rect::new(0, 0, w, h);
        let mut buf = TuiBuffer::empty(area);
        LogView {
            state,
            theme: &theme,
        }
        .render(area, &mut buf);
        buf
    }

    /// スクロールオフセットに応じて内容が上下にずれること（旧右SB と同様）。
    #[test]
    fn log_view_content_scrolls() {
        let mut state = LogState {
            detail: Some(sample_detail()),
            scroll: 0,
        };

        let (w, h) = (30u16, 20u16);

        // スクロールなし: [Inputs] 見出しが内容先頭（area.y+3）に表示される
        state.scroll = 0;
        let buf0 = render(&mut state, w, h);
        assert_eq!(
            buf0[(1, 3)].symbol(),
            "[",
            "スクロールなしでは [Inputs] 見出しが表示されるべき"
        );
        assert_eq!(
            buf0[(2, 4)].symbol(),
            "G",
            "スクロールなしでは先頭行（Global Round:）が表示されるべき"
        );

        // スクロールあり: 内容が上方へずれ、先頭行は見えなくなる
        state.scroll = 5;
        let buf1 = render(&mut state, w, h);
        assert_ne!(
            buf1[(2, 4)].symbol(),
            "G",
            "スクロール後は先頭行が表示されないべき"
        );

        // スクロール上限のクランプ: 内容より大きいオフセットは末尾に収まる
        state.scroll = 10_000;
        let buf2 = render(&mut state, w, h);
        assert_ne!(
            buf2[(2, 4)].symbol(),
            "G",
            "過大なスクロールでは先頭行が表示されないべき"
        );
        assert!(
            state.scroll < 10_000,
            "スクロールオフセットがクランプされるべき"
        );
    }

    /// 折りたたみは廃止されたため、全セクションが常に表示されること。
    #[test]
    fn log_view_has_no_collapse() {
        let mut state = LogState {
            detail: Some(sample_detail()),
            scroll: 0,
        };

        let (w, h) = (60u16, 60u16);
        let buf = render(&mut state, w, h);

        // 4 セクションすべての見出しが表示される
        let all: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(all.contains("[Inputs]"), "[Inputs] は常に表示されるべき");
        assert!(all.contains("[Meta]"), "[Meta] は常に表示されるべき");
        assert!(all.contains("[Stats]"), "[Stats] は常に表示されるべき");
        assert!(
            all.contains("[Connection]"),
            "[Connection] は常に表示されるべき"
        );
    }

    /// セクション見出しは赤、START は緑、内容は白で表示されること（旧右SB の配色維持）。
    #[test]
    fn log_view_section_colors() {
        let mut state = LogState {
            detail: Some(sample_detail()),
            scroll: 0,
        };
        state.detail.as_mut().unwrap().started_at = Some("2026-08-01T10:00:00".into());

        let (w, h) = (40u16, 40u16);
        let buf = render(&mut state, w, h);
        let theme = AppTheme::dark();

        // [Inputs] 見出し（y=3）は赤
        assert_eq!(buf[(1, 3)].fg, theme.error, "[Inputs] 見出しは赤色");
        // 内容（Global Round:）は白
        assert_eq!(buf[(2, 4)].fg, Color::White, "内容は白");
        // START 行は緑: [Inputs] 見出し(1) + 内容(7) + [Meta] 見出し(1) = y=11 → CREATE(y=12)の次 y=13
        assert_eq!(buf[(2, 13)].fg, theme.success, "START: は緑色");
    }

    /// user-block は [Meta] セクションのみ表示し、他は表示しないこと。
    #[test]
    fn log_view_user_block_shows_meta_and_output() {
        let mut detail = sample_detail();
        detail.is_user_block = true;
        detail.started_at = Some("2026-08-01T10:00:00".into());
        detail.finished_at = Some("2026-08-01T10:05:00".into());
        detail.git_commit_before = Some("abc123".into());
        detail.git_commit_after = Some("def456".into());
        detail.user_block_description = Some("ユーザーの説明文".into());
        detail.user_block_file = Some("docs/user/memo.md".into());
        detail.output_waggle_tokens = vec!["AbCdEfGh".to_string()];
        let mut state = LogState {
            detail: Some(detail),
            scroll: 0,
        };

        let (w, h) = (60u16, 40u16);
        let buf = render(&mut state, w, h);

        let all: String = buf.content.iter().map(|c| c.symbol()).collect();
        // CJK 全角文字はセル単位で分割描画されるため、日本語の検証のみスペース除去で行う
        let all_nospace: String = all.replace(' ', "");
        assert!(all.contains("[Meta]"), "[Meta] セクションは表示されるべき");
        assert!(all.contains("START"), "START は表示されるべき");
        assert!(all.contains("Commit"), "Commit は表示されるべき");
        assert!(
            all.contains("[Connection]"),
            "user-block は [Connection] を表示するべき"
        );
        assert!(
            all_nospace.contains("ユーザーの説明文"),
            "説明が表示されるべき"
        );
        assert!(
            all.contains("docs/user/memo.md"),
            "入力ファイルが表示されるべき"
        );
        assert!(all.contains("Waggle"), "Waggle が [Meta] に表示されるべき");
        assert!(all.contains("AbCdEfGh"), "出力トークンが表示されるべき");
        assert!(
            !all.contains("[Inputs]"),
            "user-block は [Inputs] を表示しないべき"
        );
        assert!(
            !all.contains("[Stats]"),
            "user-block は [Stats] を表示しないべき"
        );
    }

    /// [Stats] セクションに Model 行が表示されないことを確認する。
    #[test]
    fn log_view_stats_has_no_model_row() {
        let mut detail = sample_detail();
        detail.session_stats = Some(SessionStatsData {
            turn_count: 3,
            tool_call_count: 5,
            input_tokens: 100,
            output_tokens: 200,
            thinking_tokens: 50,
            total_tokens: 350,
            ..Default::default()
        });
        let mut state = LogState {
            detail: Some(detail),
            scroll: 0,
        };

        let (w, h) = (40u16, 60u16);
        let buf = render(&mut state, w, h);

        // [Stats] 見出しの Y 座標を探す
        let stats_y = (0..h)
            .find(|&y| {
                buf[(1, y)].symbol() == "["
                    && buf[(2, y)].symbol() == "S"
                    && buf[(3, y)].symbol() == "t"
            })
            .expect("[Stats] 見出しが見つかるべき");

        // [Stats] セクション内（見出し以降）のテキストを収集
        let mut stats_text = String::new();
        for y in stats_y..h {
            for x in 0..w {
                stats_text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(
            stats_text.contains("Turn count"),
            "Stats 内容は表示されるべき"
        );
        assert!(
            !stats_text.contains("Model"),
            "Stats セクションに Model 行は表示されないべき"
        );
    }

    /// 折り返し行数計算（CJK 全角 = 2 セル）が正しいこと。
    #[test]
    fn wrapped_line_count_is_correct() {
        assert_eq!(wrapped_line_count("1234567890", 10), 1);
        assert_eq!(wrapped_line_count("12345678901", 10), 2);
        assert_eq!(wrapped_line_count("あいうえお", 10), 1);
        assert_eq!(wrapped_line_count("あいうえおか", 10), 2);
        assert_eq!(wrapped_line_count("abc", 0), 1);
    }

    /// 長いテキストはパネル幅で折り返され、パネル外にはみ出さないこと。
    /// （入力テキストは非表示になったため、フォーム項目の Tag の長文で検証する）
    #[test]
    fn long_content_wraps_inside_panel() {
        let mut detail = sample_detail();
        detail.tag = Some(
            "very long tag text that exceeds the panel width and must wrap".to_string(),
        );
        let mut state = LogState {
            detail: Some(detail),
            scroll: 0,
        };

        let (w, h) = (30u16, 40u16);
        let theme = AppTheme::dark();
        let area = Rect::new(0, 0, w, h);
        let mut buf = TuiBuffer::empty(Rect::new(0, 0, w + 20, h));
        LogView {
            state: &mut state,
            theme: &theme,
        }
        .render(area, &mut buf);

        // パネル右端（x >= w）に文字が描画されていないこと（はみ出しなし）
        for x in w..w + 20 {
            for y in 0..h {
                assert_eq!(
                    buf[(x, y)].symbol(),
                    " ",
                    "x={} y={} にテキストがはみ出している",
                    x,
                    y
                );
            }
        }

        // 折り返しが発生し、2 行目に続きが表示される。
        // [Inputs] は Global Round(4) → Round(5) → Tag(6〜) の順。長い Tag は
        // 複数行に折り返される。
        let row1_nonempty = (1..w - 1).any(|x| buf[(x, 6)].symbol() != " ");
        assert!(row1_nonempty, "1 行目（Tag 行）に値が表示されるべき");
        let row2_nonempty = (1..w - 1).any(|x| buf[(x, 7)].symbol() != " ");
        assert!(row2_nonempty, "長い Tag 値は 2 行目に折り返されるべき");
    }

    /// 未選択（detail なし）の場合は案内メッセージが表示されること。
    #[test]
    fn log_view_without_detail_shows_hint() {
        let mut state = LogState {
            detail: None,
            scroll: 0,
        };
        let buf = render(&mut state, 60, 20);
        let all: String = buf.content.iter().map(|c| c.symbol()).collect();
        let nospace: String = all.replace(' ', "");
        assert!(
            nospace.contains("ブロックが選択されていません"),
            "案内メッセージが表示されるべき"
        );
    }
}

/// Log の接続行のテスト。
#[cfg(test)]
mod conn_line_tests {
    use super::*;
    use sai_link_engine::db::models::BlockMetadata;

    fn test_block(store: &sai_link_engine::db::SaiStore, name: &str) -> i64 {
        let b = BlockMetadata::new(
            name.to_string(),
            String::new(),
            "m".to_string(),
            None,
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            format!("blocks/{}.md", name),
            None
        );
        store.insert_block(&b).unwrap()
    }

    /// waggle 接続は `waggle: <token> R<gr>.<lr>/ID<id>` 形式になること。
    #[test]
    fn waggle_conn_line_format() {
        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let b1 = test_block(&store, "a");
        let b2 = test_block(&store, "b");
        let e1 = store.create_execution(b1, None).unwrap();
        let e2 = store.create_execution(b2, None).unwrap();
        store
            .update_execution_overrides(e1, None, Some(3), Some(2), None, None, None)
            .unwrap();
        store
            .insert_connection(&sai_link_engine::db::BlockConnection::new_waggle(
                e1,
                b1,
                e2,
                b2,
                "waggle:abc".to_string(),
            ))
            .unwrap();
        let d = build(&store, e2).unwrap();
        assert_eq!(d.connections_from.len(), 1);
        assert!(d.connections_from[0].starts_with("waggle:abc R2.3/ID"));
    }

}

/// 費用表示行（B02）のテスト。
#[cfg(test)]
mod usage_lines_tests {
    use super::*;
    use sai_link_engine::usage::{UsageSummary, UsageTotal};

    fn total(input: u64, output: u64, cache: u64, cost: Option<f64>) -> UsageTotal {
        UsageTotal {
            input,
            output,
            cache,
            cost,
        }
    }

    /// 記録あり・費用あり・子なし → 1 行だけ。
    #[test]
    fn usage_lines_with_cost() {
        let own = total(120_000, 8_100, 300_000, Some(0.42));
        let s = UsageSummary {
            own: Some(own),
            children: Vec::new(),
            total: own,
            unknown_cost_runs: 0,
        };
        let lines = usage_lines(&s);
        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0],
            "費用  $0.42（入力 120k・出力 8.1k・キャッシュ 300k）"
        );
    }

    /// usage の記録が無い（own が None）→ backfill の案内。
    #[test]
    fn usage_lines_without_record() {
        let s = UsageSummary {
            own: None,
            children: Vec::new(),
            total: UsageTotal::default(),
            unknown_cost_runs: 0,
        };
        let lines = usage_lines(&s);
        assert_eq!(
            lines,
            vec!["費用  記録なし（sai-link usage backfill で作れます）".to_string()]
        );
    }

    /// 記録はあるが費用が不明（cost が None）→ 費用不明（0 と出さない）。
    #[test]
    fn usage_lines_unknown_cost() {
        let own = total(10_000, 0, 0, None);
        let s = UsageSummary {
            own: Some(own),
            children: Vec::new(),
            total: own,
            unknown_cost_runs: 1,
        };
        let lines = usage_lines(&s);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("費用  費用不明（"), "{}", lines[0]);
    }

    /// 子あり → 2 行目に「子を含めた合計」と費用不明件数。
    #[test]
    fn usage_lines_with_children() {
        let own = total(120_000, 8_100, 0, Some(0.42));
        let c1 = total(1_000, 0, 0, Some(0.93));
        let c2 = total(1_000, 0, 0, None);
        let mut sum = own;
        sum.add(&c1);
        sum.add(&c2);
        let s = UsageSummary {
            own: Some(own),
            children: vec![(2, Some(c1)), (3, Some(c2))],
            total: sum,
            unknown_cost_runs: 1,
        };
        let lines = usage_lines(&s);
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[1],
            "子を含めた合計  $1.35（子 2 件、うち費用不明 1 件）"
        );
    }

    /// 費用行が Log 画面の先頭付近に実際に描画される。
    #[test]
    fn log_view_renders_usage_line() {
        let theme = AppTheme::dark();
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        let detail = BlockDetail {
            exec_id: 1,
            usage_lines: vec![
                "費用  $0.42（入力 120k・出力 8.1k・キャッシュ 300k）".to_string()
            ],
            ..Default::default()
        };
        let mut state = LogState {
            detail: Some(detail),
            scroll: 0,
        };
        LogView {
            state: &mut state,
            theme: &theme,
        }
        .render(area, &mut buf);
        let text: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(text.contains("$0.42"), "費用行が描画される: {}", text);
    }
}
