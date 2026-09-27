//! Project ダッシュボードビュー（ratatui ネイティブウィジェット描画）。
//!
//! `shift+p`（Project 画面）を `s`（Stats 画面 / `cache_view.rs`）の配置に
//! 準拠させて可視化する:
//!
//! ```text
//! ┌ ヘッダ（name / Start / Runs / Completed / ... 英語 stat 行） ────────┐
//! ┌ Daily Tokens（横軸=日時系列・縦軸=tokens・全ブロック合計の折れ線） ─┐
//! ┌ Tokens by Model（円グラフ） ┐┌ Tokens by Block（円グラフ） ┐         │
//! ┌ Token Detail（テーブル） ───┐┌ Recent（スクロール可能テーブル） ┐    │
//! ┌ Run Next（任意） ──────────────────────────────────────────────────┐
//! ```
//!
//! - 表示データ名は英語で統一する
//! - 円グラフは `pie` モジュールの `MiniPie`（Braille ドット描画 + 凡例）を再利用
//! - `Recent` は固定高ビューポートで内部スクロール（`j/k`・`↑/↓`）する。
//!   ページ全体のスクロールは `PgUp/PgDn`・マウスホイール（`md_scroll` 枠組み）
//! - 描画は仮想バッファに全セクションを描いてから可視ウィンドウを転写する
//!   （セクション単位クリップだと、画面より高いセクションの中間行が
//!     どのスクロール位置でも表示できないため）

use std::collections::BTreeMap;

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table, Widget, Wrap,
    },
};

use crate::pie::{MiniPie, PieSliceData};
use crate::theme::AppTheme;

/// Recent セクションの固定高（枠 2 + ヘッダ 1 + データ行 9）。
pub(crate) const RECENT_SECTION_H: u16 = 12;
/// Recent テーブルの可視データ行数。
pub(crate) const RECENT_VISIBLE_ROWS: usize = 9;

// ============================================================================
// データ型（store.rs の集計結果を TUI 側で保持しやすい形に写し替えたもの）
// ============================================================================

/// 実行サマリ（ヘッダ stat 行用）。
#[derive(Debug, Clone, Default)]
pub struct DashSummary {
    /// 全実行数
    pub total: usize,
    /// 完了実行数
    pub completed: usize,
    /// 失敗実行数
    pub failed: usize,
    /// 実行中の数
    pub running: usize,
    /// 待機中の数
    pub pending: usize,
    /// finalize 済みの数
    pub finalized: usize,
    /// 総所要時間（ミリ秒）
    pub total_duration_ms: i64,
    /// 総トークン数（input + output + cache）
    pub total_tokens: u64,
    /// 総コスト（USD）
    pub total_cost_usd: f64,
    /// コミット数（git から取得）
    pub commit_count: u64,
}

/// モデル別・ブロック別の集計 1 行（円グラフのスライス元）。
#[derive(Debug, Clone)]
pub struct DashAggRow {
    /// ラベル（モデル名 or ブロック名）
    pub label: String,
    /// 実行数
    pub execution_count: usize,
    /// input トークン
    pub input_tokens: u64,
    /// output トークン
    pub output_tokens: u64,
    /// cache トークン
    pub cache_tokens: u64,
}

impl DashAggRow {
    /// トークン合計（input + output + cache）。
    fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_tokens
    }
}

/// 期間（今週・今月）の費用合計（B02・Project 画面表示用）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PeriodUsage {
    /// 期間内の費用合計（USD）。期間内に費用が分かった実行が無ければ `None`
    pub cost: Option<f64>,
    /// 費用が不明だった実行数（usage 未保存も含む）
    pub unknown: usize,
    /// 期間内の完了実行数（`completed` か finalized）
    pub runs: usize,
}

/// Recent（直近の実行）1 行。
#[derive(Debug, Clone)]
pub struct DashRecentRow {
    /// 作成日時（表示用に整形済み）
    pub created: String,
    /// ブロック名
    pub block_name: String,
    /// R<GR>.<LR> 表示用
    pub round_label: String,
    /// 実行ステータス
    pub status: String,
    /// 所要時間（表示用に整形済み）
    pub duration: String,
    /// input トークン（execution_usage の total 行。未保存時は近似値・L1）
    pub input_tokens: u64,
    /// output トークン（同上）
    pub output_tokens: u64,
    /// cache トークン（同上）
    pub cache_tokens: u64,
}

/// Project ダッシュボードの表示データ一式。
///
/// `app/data.rs::load_project_view()` が構築し、`MainPanelState` 経由で
/// `render_dashboard()` に渡す。
#[derive(Debug, Clone, Default)]
pub struct ProjectDashData {
    /// プロジェクト名（`.sai/project.json`）。空なら "Project Overview"
    pub name: String,
    /// プロジェクト概要
    pub description: String,
    /// 開始日（YYYY-MM-DD・自動判定）
    pub start_date: Option<String>,
    /// 実行サマリ
    pub summary: DashSummary,
    /// 今週（月曜 0 時以降）の費用合計（B02）
    pub week_usage: PeriodUsage,
    /// 今月（1 日 0 時以降）の費用合計（B02）
    pub month_usage: PeriodUsage,
    /// 日別・ブロック別トークン `(date, block_name, total_tokens)`
    pub daily: Vec<(String, String, u64)>,
    /// モデル別集計
    pub by_model: Vec<DashAggRow>,
    /// ブロック別集計
    pub by_block: Vec<DashAggRow>,
    /// Run Next（Reflection の次アクション。無ければ None）
    pub run_next: Option<String>,
    /// Summary メモ（`.sai/Summary.md` の全文。表示のみ・ファイルが無ければ空）
    pub summary_md: String,
    /// 直近の実行（最大 50 件）
    pub recent: Vec<DashRecentRow>,
    /// Recent テーブルの内部スクロール位置（先頭からの行数）
    pub recent_offset: usize,
    /// ページ全体のスクロール位置（行）
    pub scroll: usize,
    /// ページスクロール上限（描画時に実測値へ更新される）
    pub max_scroll: usize,
}

// ============================================================================
// ビルダ（DB / ファイルから表示データを構築する共通純関数）
// ============================================================================

/// ダッシュボード表示データを構築する（sai-link-tui / sai-control-tui 共通ビルダ）。
///
/// `<root>/.sai/` 配下の project.json / Summary.md / reflection と、`store`
/// （対象プロジェクトの [`sai_link_engine::db::SaiStore`]・read-only で良い）
/// から全データを収集する。各 TUI は「store を開く → 本関数を呼ぶ →
/// 状態へ格納」のみを行い、`ProjectDashData` の形が変わっても修正は
/// 本クレート 1 箇所で済む（block_history::build 等と同じ方針）。
///
/// - `root`: 対象プロジェクトのルート（`.sai/` の親）
///   （sai-link-tui は [`sai_link_engine::project::project_root()`]、
///   sai-control-tui は選択プロジェクトのルートを渡す）
/// - `fallback_name`: `.sai/project.json` に名前が無い場合の表示名
///   （sai-control はプロジェクト一覧行の名前を渡す・sai-link は空文字）
pub fn build_dashboard(
    store: &sai_link_engine::db::SaiStore,
    root: &std::path::Path,
    fallback_name: &str,
) -> ProjectDashData {
    // プロジェクト名・概要（project.json）
    let (name, description) = read_project_config(root, fallback_name);

    // 開始日（最初の実行日を自動判定）
    let start_date = store.project_start_date().ok().flatten();

    // 実行サマリ（カード行用）。コミット数は git から取得
    let summary = store
        .project_run_summary()
        .map(|s| DashSummary {
            total: s.total,
            completed: s.completed,
            failed: s.failed,
            running: s.running,
            pending: s.pending,
            finalized: s.finalized,
            total_duration_ms: s.total_duration_ms,
            total_tokens: s.total_tokens,
            total_cost_usd: s.total_cost_usd,
            commit_count: count_git_commits(root),
        })
        .unwrap_or_default();

    // モデル別・ブロック別集計
    let (by_model, by_block) = match store.aggregate_project_stats() {
        Ok(agg) => (
            agg.by_model
                .iter()
                .map(|m| DashAggRow {
                    label: m.model.clone(),
                    execution_count: m.execution_count,
                    input_tokens: m.input_tokens,
                    output_tokens: m.output_tokens,
                    cache_tokens: m.cache_tokens,
                })
                .collect(),
            agg.by_block
                .iter()
                .map(|b| DashAggRow {
                    label: b.block_name.clone(),
                    execution_count: b.execution_count,
                    input_tokens: b.input_tokens,
                    output_tokens: b.output_tokens,
                    cache_tokens: b.cache_tokens,
                })
                .collect(),
        ),
        Err(_) => (Vec::new(), Vec::new()),
    };

    // 日別・ブロック別トークン
    let daily: Vec<(String, String, u64)> = store
        .daily_block_tokens()
        .map(|rows| {
            rows.into_iter()
                .map(|d| (d.date, d.block_name, d.total_tokens))
                .collect()
        })
        .unwrap_or_default();

    // Run Next（メインブロック末端の Reflection の次アクション）
    let run_next = store
        .latest_main_execution()
        .ok()
        .flatten()
        .and_then(|main_exec| main_exec.id)
        .and_then(|id| {
            let path = root
                .join(".sai")
                .join("reflection")
                .join(format!("{id}.md"));
            std::fs::read_to_string(path).ok()
        })
        .and_then(|text| extract_run_next(&text));

    // Summary メモ（`.sai/Summary.md`）。ファイルが無ければ空（セクション非表示）
    let summary_md =
        std::fs::read_to_string(root.join(".sai").join("Summary.md")).unwrap_or_default();

    // Recent 用の実行ごとの usage（execution_usage の total 行）
    let usage_map = store.execution_usage_total_map().unwrap_or_default();

    // B02: 今週（月曜 0 時）・今月（1 日 0 時）の費用合計。
    // 対象は完了した実行（`completed` か finalized）。日付は started_at → created_at の順、
    // 費用不明の判定は engine の `usage::total_from_row` に寄せる。
    let today = chrono::Utc::now().date_naive();
    let period_rows: Vec<(String, Option<f64>)> = store
        .list_all_executions()
        .unwrap_or_default()
        .iter()
        .filter(|e| e.status == "completed" || e.finalized)
        .map(|e| {
            let date = e
                .started_at
                .as_deref()
                .unwrap_or(e.created_at.as_str())
                .get(0..10)
                .unwrap_or("")
                .to_string();
            let cost = e
                .id
                .and_then(|id| usage_map.get(&id))
                .map(|row| sai_link_engine::usage::total_from_row(row).cost)
                .unwrap_or(None);
            (date, cost)
        })
        .collect();
    let week_usage = period_usage(&period_rows, week_start(today));
    let month_usage = period_usage(&period_rows, month_start(today));

    // Recent（直近の実行。テーブル内部スクロール用に 50 件取得）
    let mut recent = Vec::new();
    if let Ok(executions) = store.list_recent_executions(50) {
        for e in executions {
            let block_name = store
                .get_block_by_id(e.block_id)
                .map(|b| b.display_name)
                .unwrap_or_else(|_| format!("block#{}", e.block_id));
            let created = e.created_at.trim().to_string();
            let gr = e
                .global_round
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into());
            let lr = e.round.map(|v| v.to_string()).unwrap_or_else(|| "?".into());
            let (in_t, out_t, cache_t) = recent_exec_tokens(&e, &usage_map);
            recent.push(DashRecentRow {
                created,
                block_name,
                round_label: format!("R{}.{}", gr, lr),
                status: e.status.clone(),
                duration: e
                    .duration_ms
                    .map(format_duration_ms)
                    .unwrap_or_else(|| "-".into()),
                input_tokens: in_t,
                output_tokens: out_t,
                cache_tokens: cache_t,
            });
        }
    }

    ProjectDashData {
        name,
        description,
        start_date,
        summary,
        week_usage,
        month_usage,
        daily,
        by_model,
        by_block,
        run_next,
        summary_md,
        recent,
        recent_offset: 0,
        scroll: 0,
        max_scroll: 0,
    }
}

/// 今週の開始日（月曜 0 時）。
fn week_start(today: chrono::NaiveDate) -> chrono::NaiveDate {
    use chrono::Datelike;
    today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64)
}

/// 今月の開始日（1 日 0 時）。
fn month_start(today: chrono::NaiveDate) -> chrono::NaiveDate {
    use chrono::Datelike;
    chrono::NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today)
}

/// `from` 以降の日付の実行だけを集計する（B02・純関数）。
///
/// `rows` は `(日付 YYYY-MM-DD, 費用)`。`None` は費用不明として件数のみ数える。
/// 日付が解析できない行は無視する。
fn period_usage(rows: &[(String, Option<f64>)], from: chrono::NaiveDate) -> PeriodUsage {
    let mut p = PeriodUsage::default();
    for (date, cost) in rows {
        let Ok(d) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
            continue;
        };
        if d < from {
            continue;
        }
        p.runs += 1;
        match cost {
            Some(c) => p.cost = Some(p.cost.unwrap_or(0.0) + c),
            None => p.unknown += 1,
        }
    }
    p
}

/// 期間合計の表示行を作る（例: `今週  $3.20（費用不明 2 件）` / `今月  $12.80`）。
///
/// 期間内に実行が無ければ `実行なし`。費用が分かった分があるときだけ
/// `（費用不明 N 件）` を付ける（全部不明のときは費用部分が `費用不明` になる）。
fn format_period_line(label: &str, p: &PeriodUsage) -> String {
    if p.runs == 0 {
        return format!("{}  実行なし", label);
    }
    let cost = crate::fmt::format_cost(p.cost);
    if p.unknown > 0 && p.cost.is_some() {
        format!("{}  {}（費用不明 {} 件）", label, cost, p.unknown)
    } else {
        format!("{}  {}", label, cost)
    }
}

/// `<root>/.sai/project.json` から名前・概要を読む。
///
/// ファイルが無い・JSON が不正でも既定値（fallback）を返し、表示を続行する。
fn read_project_config(root: &std::path::Path, fallback: &str) -> (String, String) {
    let path = root.join(".sai").join("project.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .map(|v| {
            let name = v
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(fallback)
                .to_string();
            let desc = v
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            (name, desc)
        })
        .unwrap_or_else(|| (fallback.to_string(), String::new()))
}

/// Markdown テキストから `## Run Next` セクションの本文を抽出する（R18 / KTD6）。
///
/// `## Run Next` 見出しに続く、次の見出し（`##`/`#`）までのテキストを返す。
/// セクションが無い場合は `None`。
fn extract_run_next(md: &str) -> Option<String> {
    let mut in_section = false;
    let mut buf: Vec<String> = Vec::new();
    for line in md.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## Run Next") {
            in_section = true;
            continue;
        }
        if in_section && (trimmed.starts_with("# ") || trimmed.starts_with("## ")) {
            break;
        }
        if in_section {
            let t = trimmed.trim();
            if !t.is_empty() {
                buf.push(t.to_string());
            }
        }
    }
    if buf.is_empty() {
        return None;
    }
    Some(buf.join(" "))
}

/// プロジェクトリポジトリのコミット数を git から取得する。
///
/// `git -C <root> rev-list --count HEAD` を実行する。git リポジトリでない・
/// コミットが無い・git コマンドが使えない場合は 0 を返す。
fn count_git_commits(root: &std::path::Path) -> u64 {
    std::process::Command::new("git")
        .args(["-C"].as_slice())
        .arg(root)
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

/// Recent 行用の実行ごとの token（input / output / cache）を取得する（L1）。
///
/// `execution_usage` の `total` 行（provider の実際の値）を**優先**し、
/// 未保存・全 0 の旧実行は `session_stats_json` の近似値へフォールバックする
/// （Log / History 画面と同じ近似値規則）。
fn recent_exec_tokens(
    exec: &sai_link_engine::db::models::ExecutionHistory,
    usage_map: &std::collections::HashMap<i64, sai_link_engine::db::models::ExecutionUsageRow>,
) -> (u64, u64, u64) {
    if let Some(t) = usage_map.get(&exec.id.unwrap_or(0)) {
        let sum = t.input_tokens + t.output_tokens + t.cache_read + t.cache_write;
        if sum > 0 {
            return (
                t.input_tokens.max(0) as u64,
                t.output_tokens.max(0) as u64,
                t.cache_total().max(0) as u64,
            );
        }
    }
    // フォールバック: session_stats_json の近似値（R28 相当のトップレベル値）
    let parse = |json: Option<&str>, key: &str| -> u64 {
        json.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.get(key).and_then(|x| x.as_u64()))
            .unwrap_or(0)
    };
    let json = exec.session_stats_json.as_deref();
    (parse(json, "input_tokens"), parse(json, "output_tokens"), 0)
}

// ============================================================================
// 描画
// ============================================================================

/// 系列色（円グラフ・凡例で共通。並び順に応じて割当）。
fn series_color(i: usize) -> Color {
    const C: [Color; 8] = [
        Color::Cyan,
        Color::Green,
        Color::Yellow,
        Color::Magenta,
        Color::Blue,
        Color::Red,
        Color::LightCyan,
        Color::LightGreen,
    ];
    C[i % C.len()]
}

/// ミリ秒を短い時刻表現へ整形する（例: `3900000` → `1h5m`）。
pub fn format_duration_ms(ms: i64) -> String {
    if ms <= 0 {
        return "-".to_string();
    }
    let total_sec = ms / 1000;
    let h = total_sec / 3600;
    let m = (total_sec % 3600) / 60;
    let s = total_sec % 60;
    if h > 0 {
        format!("{}h{}m", h, m)
    } else if m > 0 {
        format!("{}m{}s", m, s)
    } else {
        format!("{}s", s)
    }
}

/// 数値を k / M 付きの短い表現へ整形する（軸ラベル用）。
fn format_num_k(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}G", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// ステータスの表示色。
fn status_color(status: &str, theme: &AppTheme) -> Color {
    match status {
        "completed" => theme.success,
        "failed" => theme.error,
        "running" => theme.info,
        _ => theme.muted,
    }
}

/// Recent テーブルの内部スクロール位置を変更する（`j/k`・`↑/↓`）。
pub fn recent_scroll(d: &mut ProjectDashData, delta: i32) {
    let max = d.recent.len().saturating_sub(RECENT_VISIBLE_ROWS);
    d.recent_offset = ((d.recent_offset as i64 + i64::from(delta)).clamp(0, max as i64)) as usize;
}

/// 円グラフ用スライスを作る（合計降順・上位 `max_slices` 件 + 残りは "other"）。
fn pie_slices(rows: &[DashAggRow], max_slices: usize) -> Vec<PieSliceData> {
    let mut sorted: Vec<&DashAggRow> = rows.iter().collect();
    sorted.sort_by(|a, b| b.total_tokens().cmp(&a.total_tokens()));
    let mut slices: Vec<PieSliceData> = sorted
        .iter()
        .take(max_slices)
        .enumerate()
        .map(|(i, r)| PieSliceData {
            label: r.label.clone(),
            value: r.total_tokens() as f64,
            color: series_color(i),
        })
        .collect();
    if sorted.len() > max_slices {
        let other: u64 = sorted[max_slices..].iter().map(|r| r.total_tokens()).sum();
        if other > 0 {
            slices.push(PieSliceData {
                label: "other".into(),
                value: other as f64,
                color: Color::DarkGray,
            });
        }
    }
    slices
}

/// 日別の全ブロック合計トークンを日付順に返す。
fn daily_totals(daily: &[(String, String, u64)]) -> Vec<(String, u64)> {
    let mut totals: BTreeMap<String, u64> = BTreeMap::new();
    for (date, _, tokens) in daily {
        *totals.entry(date.clone()).or_insert(0) += tokens;
    }
    totals.into_iter().collect()
}

/// Project ダッシュボードを描画する。
///
/// `d.max_scroll` を実測値で更新するため `&mut` を受け取る。
/// 幅が極端に狭い場合はヘッダのみの簡易表示へフォールバックする。
pub fn render_dashboard(area: Rect, buf: &mut Buffer, d: &mut ProjectDashData, theme: &AppTheme) {
    if area.width < 40 || area.height < 6 {
        // 最小幅未満: 情報だけ表示して崩壊を防ぐ
        let text = format!(
            "{} / Start: {} / Runs: {} (area too small)",
            if d.name.is_empty() {
                "Project Overview"
            } else {
                &d.name
            },
            d.start_date.as_deref().unwrap_or("-"),
            d.summary.total
        );
        Paragraph::new(text)
            .wrap(Wrap { trim: true })
            .render(area, buf);
        return;
    }

    // ── セクション定義（`s` 画面準拠: chart → pies → tables） ──
    let mut sections: Vec<Section> = Vec::new();
    sections.push(Section {
        h: header_height(d, area.width),
        f: SectionFn::Header,
    });
    // sai-paper の Dashboard 風 stat カード（枠 + 数値）5 枚
    sections.push(Section {
        h: 4,
        f: SectionFn::Cards,
    });
    if !d.daily.is_empty() {
        sections.push(Section {
            h: 12,
            f: SectionFn::DailyChart,
        });
    }
    if !d.by_model.is_empty() || !d.by_block.is_empty() {
        sections.push(Section {
            h: 12,
            f: SectionFn::Pies,
        });
    }
    if !d.recent.is_empty() {
        sections.push(Section {
            h: RECENT_SECTION_H,
            f: SectionFn::Bottom,
        });
    }
    if d.run_next.is_some() {
        sections.push(Section {
            h: run_next_height(d, area.width),
            f: SectionFn::RunNext,
        });
    }
    // Summary メモ（`.sai/Summary.md`）。ファイルが無い・空のときは非表示
    if !d.summary_md.is_empty() {
        sections.push(Section {
            h: summary_height(d, area.width),
            f: SectionFn::Summary,
        });
    }

    let total_h: u16 = sections.iter().map(|s| s.h).sum();
    d.max_scroll = total_h.saturating_sub(area.height) as usize;
    // scroll は usize のままクランプする（u16 への縮小で wrap しないように）
    let scroll = d.scroll.min(d.max_scroll);
    d.scroll = scroll; // クランプ結果を書き戻す
    let scroll = scroll as u16;

    // ── 仮想バッファへ全セクションを描画し、可視ウィンドウを転写する ──
    //
    // セクション矩形を画面に直接インターセクトさせると、画面より高い
    // セクションの「中間の行」をどのスクロール位置でも表示できない。
    // そのため全セクションを一旦 1 枚の仮想バッファに描画してから、
    // `scroll` 位置のウィンドウだけを実バッファへコピーする。
    let mut virt = Buffer::empty(Rect::new(0, 0, area.width, total_h));
    for cell in virt.content.iter_mut() {
        cell.set_style(Style::default().bg(theme.main_bg));
    }

    let mut y = 0u16;
    for section in &sections {
        let sec_rect = Rect {
            x: 0,
            y,
            width: area.width,
            height: section.h,
        };
        y += section.h;
        match section.f {
            SectionFn::Header => render_header(sec_rect, &mut virt, d, theme),
            SectionFn::Cards => render_cards(sec_rect, &mut virt, d, theme),
            SectionFn::DailyChart => render_daily_chart(sec_rect, &mut virt, d, theme),
            SectionFn::Pies => render_pies(sec_rect, &mut virt, d, theme),
            SectionFn::Bottom => render_bottom(sec_rect, &mut virt, d, theme),
            SectionFn::RunNext => render_run_next(sec_rect, &mut virt, d, theme),
            SectionFn::Summary => render_summary(sec_rect, &mut virt, d, theme),
        }
    }

    // 可視ウィンドウ（scroll .. scroll+area.height）を実バッファへ転写
    let win_bottom = (scroll + area.height).min(total_h);
    for vy in scroll..win_bottom {
        let dy = area.y + (vy - scroll);
        for x in 0..area.width {
            if let Some(dst) = buf.cell_mut((area.x + x, dy)) {
                *dst = virt[(x, vy)].clone();
            }
        }
    }
}

/// セクション種別。
#[derive(Clone, Copy)]
enum SectionFn {
    Header,
    Cards,
    DailyChart,
    Pies,
    Bottom,
    RunNext,
    Summary,
}

/// 縦積みセクション（高さ + 描画関数種別）。
struct Section {
    h: u16,
    f: SectionFn,
}

/// Summary セクションの高さ（本文折返し行数の合計 + 枠）。
fn summary_height(d: &ProjectDashData, width: u16) -> u16 {
    // 枠の内側幅（左右ボーダー分を引く）
    let inner = width.saturating_sub(2).max(1) as usize;
    let body: usize = d
        .summary_md
        .lines()
        .map(|line| {
            // 表示幅 1 文字 = 1 セル、全角は 2 セルとして折り返し行数を概算
            let w = line
                .chars()
                .map(|c| if c.is_ascii() { 1 } else { 2 })
                .sum::<usize>();
            w.div_ceil(inner).max(1)
        })
        .sum();
    body as u16 + 2
}

/// ヘッダ高さ（名前 1 + 概要折り返し行数）。
///
/// サマリ stat（Runs / Completed / ...）はカード行（`render_cards`）へ移行済み。
fn header_height(d: &ProjectDashData, width: u16) -> u16 {
    let desc_width = width.saturating_sub(2) as usize;
    let desc_lines = if d.description.is_empty() {
        0
    } else {
        // 概要は折り返しを考慮（概算: 表示幅 1 文字 = 1 セル、全角は 2 セル）
        let w = d
            .description
            .chars()
            .map(|c| if c.is_ascii() { 1 } else { 2 })
            .sum::<usize>()
            .max(1);
        w.div_ceil(desc_width.max(1)) as u16
    };
    1 + 2 + desc_lines // name+start / 今週・今月の費用 / description
}

/// Run Next セクションの高さ（本文折返し + 枠）。
fn run_next_height(d: &ProjectDashData, width: u16) -> u16 {
    let text = d.run_next.as_deref().unwrap_or("");
    let w = text
        .chars()
        .map(|c| if c.is_ascii() { 1 } else { 2 })
        .sum::<usize>() as u16;
    let body = w.div_ceil(width.saturating_sub(4).max(1)).max(1);
    body + 2
}

/// ブロック枠 + タイトルの共通ビルダー。
fn block_titled(title: &str, color: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .title(Span::styled(
            title.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
}

/// ヘッダ（name / Start / description / サマリ stat 行。ラベルは英語）。
fn render_header(area: Rect, buf: &mut Buffer, d: &ProjectDashData, theme: &AppTheme) {
    let start = d.start_date.clone().unwrap_or_else(|| "n/a".into());
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            if d.name.is_empty() {
                "Project Overview".to_string()
            } else {
                d.name.clone()
            },
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  Start: {}", start),
            Style::default().fg(theme.muted),
        ),
    ]));
    if !d.description.is_empty() {
        lines.push(Line::from(Span::styled(
            d.description.clone(),
            Style::default().fg(theme.muted),
        )));
    }
    lines.push(Line::from(Span::styled(
        format_period_line("今週", &d.week_usage),
        Style::default().fg(theme.muted),
    )));
    lines.push(Line::from(Span::styled(
        format_period_line("今月", &d.month_usage),
        Style::default().fg(theme.muted),
    )));
    Paragraph::new(lines).render(area, buf);
}

/// sai-paper の Dashboard 風 stat カード（枠 + 数値）。
///
/// 5 枚: BLOCK（実行数合計・ステータス不問）/ TOKENS（input+output+cache）/
/// COST（USD）/ COMMITS（git から取得）/ TIME（総所要時間 hour）。
fn render_cards(area: Rect, buf: &mut Buffer, d: &ProjectDashData, theme: &AppTheme) {
    let cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 5); 5])
        .spacing(1)
        .split(area);
    let hours = d.summary.total_duration_ms as f64 / 3_600_000.0;
    let data = [
        ("BLOCK", d.summary.total.to_string(), theme.info),
        (
            "TOKENS",
            format_num_k(d.summary.total_tokens),
            theme.success,
        ),
        (
            "COST",
            format!("${:.1}", d.summary.total_cost_usd),
            theme.warning,
        ),
        ("COMMITS", d.summary.commit_count.to_string(), theme.fg),
        ("TIME", format!("{:.1}h", hours), theme.muted),
    ];
    for (card_area, (label, value, color)) in cards.iter().zip(data) {
        let card = Paragraph::new(vec![
            Line::styled(label, Style::default().fg(theme.muted)),
            Line::styled(
                value,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.border)),
        )
        .alignment(Alignment::Center);
        card.render(*card_area, buf);
    }
}

/// Daily Tokens チャート（横軸=日時系列・縦軸=tokens・全ブロック合計）。
fn render_daily_chart(area: Rect, buf: &mut Buffer, d: &ProjectDashData, theme: &AppTheme) {
    let totals = daily_totals(&d.daily);
    if totals.is_empty() {
        return;
    }
    let data: Vec<(f64, f64)> = totals
        .iter()
        .enumerate()
        .map(|(i, (_, t))| (i as f64, *t as f64))
        .collect();
    let y_max = totals.iter().map(|(_, t)| *t).max().unwrap_or(1) as f64;
    let x_max = (totals.len().saturating_sub(1)).max(1) as f64;

    // x 軸ラベル: 先頭 / 中央 / 末尾の日付
    let n = totals.len();
    let x_labels = vec![
        Span::raw(totals[0].0.clone()),
        Span::raw(if n > 2 {
            totals[n / 2].0.clone()
        } else {
            String::new()
        }),
        Span::raw(totals[n - 1].0.clone()),
    ];
    let y_labels = vec![
        Span::raw("0".to_string()),
        Span::raw(format_num_k((y_max / 2.0) as u64)),
        Span::raw(format_num_k(y_max as u64)),
    ];

    let ds = Dataset::default()
        .name("tokens")
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(theme.info))
        .data(&data);
    let chart = Chart::new(vec![ds])
        .block(block_titled("Daily Tokens", theme.info))
        .x_axis(
            Axis::default()
                .title("date")
                .bounds([0.0, x_max])
                .labels(x_labels),
        )
        .y_axis(
            Axis::default()
                .title("tokens")
                .bounds([0.0, y_max * 1.1])
                .labels(y_labels),
        );
    chart.render(area, buf);
}

/// 中段: モデル別・ブロック別のトークン構成比の円グラフ 2 枚。
fn render_pies(area: Rect, buf: &mut Buffer, d: &ProjectDashData, _theme: &AppTheme) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let model_pie = MiniPie {
        title: " Tokens by Model ".to_string(),
        slices: pie_slices(&d.by_model, 7),
    };
    let block_pie = MiniPie {
        title: " Tokens by Block ".to_string(),
        slices: pie_slices(&d.by_block, 7),
    };

    if !d.by_model.is_empty() {
        model_pie.render(cols[0], buf);
    }
    if !d.by_block.is_empty() {
        block_pie.render(cols[1], buf);
    }
}

/// 下段: Recent スクロール可能テーブル（全幅・Token Detail 廃止に伴い画面左から右端まで）。
fn render_bottom(area: Rect, buf: &mut Buffer, d: &mut ProjectDashData, theme: &AppTheme) {
    if !d.recent.is_empty() {
        render_recent_table(area, buf, d, theme);
    }
}

/// Recent（直近の実行）テーブル。固定高ビューポートで内部スクロールする。
///
/// スクロール位置は `d.recent_offset`（`j/k`・`↑/↓` で変更）。タイトルに
/// 現在の表示範囲（`start-end/total`）を表示する。トークン列
/// （input / output / cache / Total）は `execution_usage` の total 行
/// （provider の実際の値・L1）ベース。未保存の旧実行は近似値が入る。
fn render_recent_table(area: Rect, buf: &mut Buffer, d: &mut ProjectDashData, theme: &AppTheme) {
    let visible = area.height.saturating_sub(3) as usize; // 枠 2 + ヘッダ 1
    let visible = visible.max(1);
    let max_offset = d.recent.len().saturating_sub(visible);
    d.recent_offset = d.recent_offset.min(max_offset);

    let start = d.recent_offset;
    let end = (start + visible).min(d.recent.len());
    let header = Row::new([
        "Time", "R", "Block", "Status", "Duration", "input", "output", "cache", "Total",
    ])
    .style(Style::default().fg(theme.info).add_modifier(Modifier::BOLD));
    let rows: Vec<Row> = d.recent[start..end]
        .iter()
        .map(|r| {
            Row::new(vec![
                Cell::from(r.created.clone()),
                Cell::from(r.round_label.clone()),
                Cell::from(r.block_name.clone()),
                Cell::from(Span::styled(
                    r.status.clone(),
                    Style::default().fg(status_color(&r.status, theme)),
                )),
                Cell::from(r.duration.clone()),
                Cell::from(crate::fmt::format_num(r.input_tokens)),
                Cell::from(crate::fmt::format_num(r.output_tokens)),
                Cell::from(crate::fmt::format_num(r.cache_tokens)),
                Cell::from(crate::fmt::format_num(
                    r.input_tokens + r.output_tokens + r.cache_tokens,
                )),
            ])
        })
        .collect();
    let widths = [
        Constraint::Percentage(14),
        Constraint::Percentage(6),
        Constraint::Percentage(22),
        Constraint::Percentage(10),
        Constraint::Percentage(10),
        Constraint::Percentage(10),
        Constraint::Percentage(10),
        Constraint::Percentage(9),
        Constraint::Percentage(9),
    ];
    let title = format!(" Recent {}-{}/{} ", start + 1, end, d.recent.len());
    let table = Table::new(rows, widths)
        .header(header)
        .block(block_titled(&title, theme.info));
    table.render(area, buf);
}

/// Run Next セクション。
fn render_run_next(area: Rect, buf: &mut Buffer, d: &ProjectDashData, theme: &AppTheme) {
    let text = d.run_next.clone().unwrap_or_default();
    let para = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .block(block_titled("Run Next", theme.warning));
    para.render(area, buf);
}

/// Summary メモ（`.sai/Summary.md`）を表示のみで描画する（編集は `Shift+A` の nvim）。
fn render_summary(area: Rect, buf: &mut Buffer, d: &ProjectDashData, theme: &AppTheme) {
    let para = Paragraph::new(d.summary_md.clone())
        .wrap(Wrap { trim: false })
        .block(block_titled("Summary", theme.info));
    para.render(area, buf);
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;

    /// バッファ全文を空白なしで連結した文字列を返す。
    ///
    /// 全角文字は 2 セルに分かれて格納されるため、素の連結では
    /// 「実 行 完 了 率」のように空白が挟まる。空白を除去して比較する。
    fn flat_content(term: &Terminal<TestBackend>) -> String {
        term.backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    /// テスト用ダッシュボードデータ。
    fn sample_data() -> ProjectDashData {
        ProjectDashData {
            name: "myproject".into(),
            description: "test project".into(),
            start_date: Some("2026-08-01".into()),
            summary: DashSummary {
                total: 10,
                completed: 7,
                failed: 1,
                running: 2,
                pending: 0,
                finalized: 5,
                total_duration_ms: 3_660_000,
                total_tokens: 45_600_000,
                total_cost_usd: 12.34,
                commit_count: 42,
            },
            week_usage: PeriodUsage {
                cost: Some(3.2),
                unknown: 2,
                runs: 5,
            },
            month_usage: PeriodUsage {
                cost: Some(12.8),
                unknown: 0,
                runs: 9,
            },
            daily: vec![
                ("2026-08-01".into(), "plan".into(), 100),
                ("2026-08-01".into(), "code".into(), 200),
                ("2026-08-02".into(), "plan".into(), 300),
            ],
            by_model: vec![
                DashAggRow {
                    label: "opus".into(),
                    execution_count: 6,
                    input_tokens: 100,
                    output_tokens: 50,
                    cache_tokens: 10,
                },
                DashAggRow {
                    label: "sonnet".into(),
                    execution_count: 4,
                    input_tokens: 30,
                    output_tokens: 20,
                    cache_tokens: 0,
                },
            ],
            by_block: vec![DashAggRow {
                label: "plan".into(),
                execution_count: 5,
                input_tokens: 90,
                output_tokens: 40,
                cache_tokens: 5,
            }],
            run_next: Some("next action".into()),
            summary_md: "# Summary\n\n- todo: write tests\n".into(),
            recent: (0..20)
                .map(|i| DashRecentRow {
                    created: format!("10:{:02}", i),
                    block_name: "plan".into(),
                    round_label: format!("R1.{}", i + 1),
                    status: "completed".into(),
                    duration: "1m30s".into(),
                    input_tokens: 100 + i,
                    output_tokens: 50,
                    cache_tokens: 10,
                })
                .collect(),
            recent_offset: 0,
            scroll: 0,
            max_scroll: 0,
        }
    }

    /// ダッシュボードが panic せず、全セクション（英語タイトル）が描画されること。
    /// 実行完了率（Gauge）は廃止済みで表示されないこと。
    #[test]
    fn dashboard_renders_sections_in_english() {
        let backend = TestBackend::new(100, 60);
        let mut term = Terminal::new(backend).unwrap();
        let mut data = sample_data();
        term.draw(|f| {
            render_dashboard(f.area(), f.buffer_mut(), &mut data, &AppTheme::dark());
        })
        .unwrap();
        let content = flat_content(&term);
        assert!(content.contains("myproject"), "名前が表示される: {content}");
        assert!(
            content.contains("Start:2026-08-01"),
            "Start ラベル（英語）が表示される"
        );
        // sai-paper 風 stat カード（枠 + 数値）
        assert!(content.contains("BLOCK"), "BLOCK カードが表示される");
        assert!(content.contains("TOKENS"), "TOKENS カードが表示される");
        assert!(content.contains("45.6M"), "トークン数は M 付きで表示される");
        assert!(content.contains("COST"), "COST カードが表示される");
        assert!(content.contains("$12.3"), "コストは $ 付きで表示される");
        assert!(content.contains("COMMITS"), "COMMITS カードが表示される");
        assert!(content.contains("TIME"), "TIME カードが表示される");
        assert!(content.contains("1.0h"), "所要時間は hour 単位で表示される");
        assert!(
            content.contains("DailyTokens"),
            "Daily Tokens チャートが表示される"
        );
        assert!(
            content.contains("TokensbyModel"),
            "モデル別円グラフが表示される"
        );
        assert!(
            content.contains("TokensbyBlock"),
            "ブロック別円グラフが表示される"
        );
        // Token Detail は廃止（L1 + ユーザー指定）: Recent に input/output/cache/Total 列が移行
        assert!(
            !content.contains("TokenDetail"),
            "Token Detail テーブルは廃止済み"
        );
        assert!(content.contains("input"), "Recent に input 列が表示される");
        assert!(content.contains("Total"), "Recent に Total 列が表示される");
        assert!(content.contains("100"), "Recent の input 値が表示される");
        assert!(
            content.contains("Recent1-9/20"),
            "Recent はスクロール範囲付きタイトルで表示される"
        );
        assert!(content.contains("RunNext"), "Run Next が表示される");
        // 廃止されたセクション
        assert!(
            !content.contains("CompletedRate"),
            "実行完了率 Gauge は廃止済み"
        );
        // 日本語ラベルは廃止済み
        assert!(!content.contains("実行完了率"), "日本語ラベルは使わない");
        assert!(
            !content.contains("モデル別トークン内訳"),
            "日本語ラベルは使わない"
        );
    }

    /// Daily Tokens チャートは全ブロック合計の時系列であること
    /// （y 軸上端が日別合計の最大値になる）。
    #[test]
    fn daily_chart_shows_total_per_day() {
        // 最大日 = 2026-08-02（300）。y 軸ラベルに 300 が現れる
        let mut data = sample_data();
        data.by_model.clear();
        data.by_block.clear();
        data.recent.clear();
        data.run_next = None;

        let backend = TestBackend::new(100, 20);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            render_dashboard(f.area(), f.buffer_mut(), &mut data, &AppTheme::dark());
        })
        .unwrap();
        let content = flat_content(&term);
        assert!(
            content.contains("DailyTokens"),
            "チャート枠が表示される: {content}"
        );
        assert!(
            content.contains("300"),
            "y 軸に日別合計の最大値が表示される"
        );
    }

    /// 円グラフで上位 7 件以外が "other" に統合されること。
    #[test]
    fn pie_merges_overflow_into_other() {
        let mut data = sample_data();
        data.by_block = (0..9)
            .map(|i| DashAggRow {
                label: format!("blk{}", i),
                execution_count: 1,
                input_tokens: 100,
                output_tokens: 0,
                cache_tokens: 0,
            })
            .collect();
        data.by_model.clear();

        let backend = TestBackend::new(100, 40);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            render_dashboard(f.area(), f.buffer_mut(), &mut data, &AppTheme::dark());
        })
        .unwrap();
        let content = flat_content(&term);
        assert!(
            content.contains("TokensbyBlock"),
            "ブロック別円グラフが表示される"
        );
        assert!(
            content.contains("other"),
            "上位 7 件以外は other に統合される: {content}"
        );
    }

    /// Recent テーブルの内部スクロール: recent_offset を変えると
    /// 表示行が切り替わり、範囲がクランプされること。
    #[test]
    fn recent_table_scrolls_internally() {
        let mut data = sample_data(); // recent 20 件・可視 9 行
        let backend = TestBackend::new(100, 60);
        let mut term = Terminal::new(backend).unwrap();

        // 先頭: 1-9/20
        term.draw(|f| {
            render_dashboard(f.area(), f.buffer_mut(), &mut data, &AppTheme::dark());
        })
        .unwrap();
        let content = flat_content(&term);
        assert!(content.contains("10:00"), "先頭行が見える");
        assert!(data.recent_offset == 0);

        // 下へ: j/k 相当
        recent_scroll(&mut data, 5);
        assert_eq!(data.recent_offset, 5);
        term.draw(|f| {
            render_dashboard(f.area(), f.buffer_mut(), &mut data, &AppTheme::dark());
        })
        .unwrap();
        let content = flat_content(&term);
        assert!(
            content.contains("Recent6-14/20"),
            "タイトルに表示範囲が出る: {content}"
        );
        assert!(content.contains("10:13"), "スクロール後の行が見える");
        assert!(!content.contains("10:00"), "先頭行は画面外");

        // 過大スクロールは末尾にクランプ（20 件・可視 9 行 → max offset 11）
        recent_scroll(&mut data, 100);
        assert_eq!(data.recent_offset, 11);
        term.draw(|f| {
            render_dashboard(f.area(), f.buffer_mut(), &mut data, &AppTheme::dark());
        })
        .unwrap();
        let content = flat_content(&term);
        assert!(
            content.contains("Recent12-20/20"),
            "末尾範囲が表示される: {content}"
        );
    }

    /// ページ全体のスクロール: 過大 scroll は実測 max_scroll にクランプされる。
    #[test]
    fn dashboard_scroll_clamps_and_updates_max() {
        let backend = TestBackend::new(100, 12);
        let mut term = Terminal::new(backend).unwrap();
        let mut data = sample_data();
        data.scroll = 10_000; // 過大値は実測 max にクランプされる
        term.draw(|f| {
            render_dashboard(f.area(), f.buffer_mut(), &mut data, &AppTheme::dark());
        })
        .unwrap();
        assert!(
            data.max_scroll > 0,
            "実測の max_scroll が設定される: {}",
            data.max_scroll
        );
        assert!(
            data.scroll <= data.max_scroll,
            "scroll は max_scroll 以下にクランプされる"
        );
    }

    /// 極端に狭い領域でも panic しない（フォールバック表示）。
    #[test]
    fn dashboard_falls_back_on_narrow_area() {
        let backend = TestBackend::new(30, 4);
        let mut term = Terminal::new(backend).unwrap();
        let mut data = sample_data();
        term.draw(|f| {
            render_dashboard(f.area(), f.buffer_mut(), &mut data, &AppTheme::dark());
        })
        .unwrap();
        let content = flat_content(&term);
        assert!(
            content.contains("myproject"),
            "フォールバック表示に名前が出る: {content}"
        );
    }

    /// Summary セクション: `.sai/Summary.md` の内容が最下部に表示され、
    /// 空のときはセクション自体が非表示になること（表示のみ・編集は `Shift+A`）。
    #[test]
    fn summary_section_renders_and_hides_when_empty() {
        // 内容あり: タイトルと本文が表示される
        let backend = TestBackend::new(100, 80);
        let mut term = Terminal::new(backend).unwrap();
        let mut data = sample_data();
        term.draw(|f| {
            render_dashboard(f.area(), f.buffer_mut(), &mut data, &AppTheme::dark());
        })
        .unwrap();
        let content = flat_content(&term);
        assert!(
            content.contains("Summary"),
            "Summary セクションが表示される: {content}"
        );
        assert!(
            content.contains("todo"),
            "Summary の本文が表示される: {content}"
        );

        // 内容なし（ファイル未作成相当）: セクションごと非表示
        data.summary_md.clear();
        let mut term = Terminal::new(TestBackend::new(100, 80)).unwrap();
        term.draw(|f| {
            render_dashboard(f.area(), f.buffer_mut(), &mut data, &AppTheme::dark());
        })
        .unwrap();
        let content = flat_content(&term);
        assert!(
            content.contains("RunNext"),
            "他セクションは表示される: {content}"
        );
        // 枠タイトル "Summary" はセクションタイトルのみで構成されるため、
        // 非表示なら本文 "todo" もタイトルも出ない
        assert!(!content.contains("todo"), "Summary は非表示: {content}");
    }

    /// 所要時間の整形。
    #[test]
    fn format_duration_ms_formats() {
        assert_eq!(format_duration_ms(-1), "-");
        assert_eq!(format_duration_ms(0), "-");
        assert_eq!(format_duration_ms(45_000), "45s");
        assert_eq!(format_duration_ms(3_660_000), "1h1m");
    }

    // -------------------------------------------------------------------
    // ビルダ: Run Next 抽出（R18）
    // -------------------------------------------------------------------

    #[test]
    fn extract_run_next_extracts_section() {
        let md = "---\nblock: a\n---\n\n## Run Reason\n\n理由\n\n## Run Evaluation\n\n評価\n\n## Run Next\n\n次のアクションを実行する。\n";
        let next = extract_run_next(md).expect("Run Next を抽出できること");
        assert!(next.contains("次のアクションを実行する"));
    }

    #[test]
    fn extract_run_next_returns_none_when_missing() {
        assert!(extract_run_next("## Run Reason\n\nなし").is_none());
    }

    #[test]
    fn extract_run_next_stops_at_next_heading() {
        let md = "## Run Next\n\nアクションA\n\n## Other\n\n別の内容\n";
        let next = extract_run_next(md).expect("抽出できること");
        assert!(next.contains("アクションA"));
        assert!(!next.contains("別の内容"));
    }

    // -------------------------------------------------------------------
    // B02: 今週・今月の費用合計
    // -------------------------------------------------------------------

    /// 週の境目: 月曜（開始日）は入り、前日の日曜は入らない。
    #[test]
    fn period_usage_week_boundary() {
        // 2026-09-26 は土曜。今週の月曜は 2026-09-21。
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 26).unwrap();
        let from = week_start(today);
        assert_eq!(
            from,
            chrono::NaiveDate::from_ymd_opt(2026, 9, 21).unwrap(),
            "週の開始は月曜"
        );
        let rows = vec![
            ("2026-09-20".to_string(), Some(1.00)), // 前日の日曜 → 対象外
            ("2026-09-21".to_string(), Some(2.00)), // 月曜 0 時 → 対象
            ("2026-09-26".to_string(), Some(0.20)), // 当日
        ];
        let p = period_usage(&rows, from);
        assert_eq!(p.runs, 2);
        assert_eq!(p.cost, Some(2.20));
        assert_eq!(p.unknown, 0);
    }

    /// 月の境目: 1 日は入り、前月末日は入らない。費用不明は件数のみ数える。
    #[test]
    fn period_usage_month_boundary() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 26).unwrap();
        let from = month_start(today);
        assert_eq!(
            from,
            chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()
        );
        let rows = vec![
            ("2026-08-31".to_string(), Some(5.00)), // 前月末 → 対象外
            ("2026-09-01".to_string(), Some(1.50)), // 今月 1 日 → 対象
            ("2026-09-10".to_string(), None),       // 費用不明
            ("2026-09-26".to_string(), Some(1.30)), // 当日
        ];
        let p = period_usage(&rows, from);
        assert_eq!(p.runs, 3);
        assert_eq!(p.cost, Some(2.80));
        assert_eq!(p.unknown, 1);
    }

    /// 全部費用不明なら cost は None（0 と見せない）、期間に実行が無ければ runs=0。
    #[test]
    fn period_usage_unknown_and_empty() {
        let from = chrono::NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let all_unknown = vec![
            ("2026-09-22".to_string(), None),
            ("2026-09-23".to_string(), None),
        ];
        let p = period_usage(&all_unknown, from);
        assert_eq!(p.runs, 2);
        assert_eq!(p.cost, None);
        assert_eq!(p.unknown, 2);

        let empty = period_usage(&[], from);
        assert_eq!(empty.runs, 0);
        assert_eq!(empty.cost, None);
        assert_eq!(empty.unknown, 0);
    }

    /// 表示行の書式。費用不明件数は費用が分かった分があるときだけ付ける。
    #[test]
    fn format_period_line_formats() {
        assert_eq!(
            format_period_line(
                "今週",
                &PeriodUsage {
                    cost: Some(3.2),
                    unknown: 2,
                    runs: 5,
                }
            ),
            "今週  $3.20（費用不明 2 件）"
        );
        assert_eq!(
            format_period_line(
                "今月",
                &PeriodUsage {
                    cost: Some(12.8),
                    unknown: 0,
                    runs: 9,
                }
            ),
            "今月  $12.80"
        );
        assert_eq!(
            format_period_line("今週", &PeriodUsage::default()),
            "今週  実行なし"
        );
    }
}
