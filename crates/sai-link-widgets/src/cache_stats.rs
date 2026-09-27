//! Stats ビュー（キャッシュ / トークン統計グラフ）。
//!
//! `sai-cache-tui-rs`（`/home/aidivn2026/workdir/Book/Pi/pi-extension/cache/sai-cache-tui-rs`）
//! の移植版。pi セッション jsonl を解析し、以下をメイン画面に表示する:
//!
//! - Cache / Token trend チャート（Per-turn % / 累積 % / 累積トークン）
//! - Stats テーブル（`1` / `2` / `3` / `s` で切替）
//! - Token / Cost 構成比の円グラフ
//! - Per-message breakdown（PgUp/PgDn でスクロール）+ Summary
//!
//! セッション選択（サイドバーのファイル一覧）は sai-link-tui では不要。
//! 左サイドバーで選択したブロック（実行）の `session_dir` から
//! メインセッション jsonl を解決して入力とする（`app::data::load_cache_view`）。

use std::path::PathBuf;

use crate::pie::{MiniPie, PieSliceData};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style, Stylize},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table, Widget,
    },
};

// ============================================================================
// 集計ロジック（engine へ移設・01 §2.1）
// ============================================================================
//
// トークン / コストの集計ロジックと型は `sai_link_engine::usage_metrics` に
// 移設した（デーモン＝engine から使うため）。ここでは再公開のみ行い、
// 呼び出し（TUI / sai-control）と数字は変更しない。

pub use sai_link_engine::usage_metrics::*;

// ============================================================================
// ビュー状態
// ============================================================================

/// チャート表示モード（`sai-cache-tui-rs` の ViewMode 相当）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheChartMode {
    /// ターンごとのキャッシュヒット率
    #[default]
    PerTurn,
    /// 累積ヒット率
    CumulativePercent,
    /// 累積トークン
    CumulativeTotal,
    /// Stats テーブル
    StatsTable,
}

impl CacheChartMode {
    /// 表示ラベル。
    pub fn label(self) -> &'static str {
        match self {
            CacheChartMode::PerTurn => "Per-turn (%)",
            CacheChartMode::CumulativePercent => "Cumulative (aggregate) %",
            CacheChartMode::CumulativeTotal => "Cumulative (aggregate) total",
            CacheChartMode::StatsTable => "Stats table",
        }
    }

    /// 次のモードへ循環（`1` → `2` → `3` → `s` → `1` ...）。
    pub fn next(self) -> CacheChartMode {
        match self {
            CacheChartMode::PerTurn => CacheChartMode::CumulativePercent,
            CacheChartMode::CumulativePercent => CacheChartMode::CumulativeTotal,
            CacheChartMode::CumulativeTotal => CacheChartMode::StatsTable,
            CacheChartMode::StatsTable => CacheChartMode::PerTurn,
        }
    }
}

/// Stats ビューの状態。
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
    /// チャート表示モード
    pub chart_mode: CacheChartMode,
    /// Per-message breakdown のスクロール位置
    pub per_message_scroll: usize,
    /// 読み込みエラーメッセージ（None = 正常）
    pub error: Option<String>,
    /// 現在表示中の実行 ID（キャッシュ用。同一実行の再読み込みを省略する）
    pub loaded_exec_id: Option<i64>,
}

/// `phone run --view stats` 用の JSON 化（実装計画 12 §3.2）。
///
/// 描画用の内部状態（`chart_mode` / `per_message_scroll` / `loaded_exec_id`）は
/// 出力しない。`metrics` が無い（解析失敗・セッション無し）項目は `null`。
#[derive(serde::Serialize)]
pub struct PhoneStatsJson<'a> {
    /// 解析対象のセッション jsonl パス（無ければ null）
    pub session_file: Option<String>,
    /// 読み込みエラー（無ければ null）
    pub error: Option<String>,
    /// ツリー全体の合計（無ければ null）
    pub totals: Option<&'a sai_link_engine::usage_metrics::CacheUsageTotals>,
    /// アクティブブランチの合計（無ければ null）
    pub active_totals: Option<&'a sai_link_engine::usage_metrics::CacheUsageTotals>,
    /// assistant メッセージごとの使用量（無ければ null）
    pub messages: Option<&'a [sai_link_engine::usage_metrics::AssistantUsageMetric]>,
    /// 累積系列（チャート用）
    pub cumulative: &'a sai_link_engine::usage_metrics::CumulativeSeries,
    /// セッション内容のメッセージ数カウント
    pub counts: &'a sai_link_engine::usage_metrics::SessionCounts,
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
/// sai-link-tui `app/data.rs::load_cache_view()` の解析部分を分離したもの。
/// 実行レコードの `session_dir` から `find_main_session_file` でメインセッション
/// jsonl を解決し、sai-cache-tui-rs と同じ解析（usage 集計）を行う。
///
/// - `store`: 対象プロジェクトの `SaiStore`（read-only でも可）
/// - `exec_id`: 対象実行 ID
///
/// セッションが無い・解析に失敗した場合は `error` にメッセージをセットする
/// （画面にそのまま表示される。呼び出し側の状態更新は不要）。
pub fn load(store: &sai_link_engine::db::SaiStore, exec_id: i64) -> CacheViewState {
    // 実行レコードから runner と session_dir を取得する。
    // runner=gl: ~/.sai-gemini/sessions/
    // runner=cc: ~/.claude/projects/
    // その他: DB の resolve_session_dir
    let Some(exec) = store.get_execution_by_id(exec_id).ok() else {
        return CacheViewState {
            loaded_exec_id: Some(exec_id),
            error: Some(format!("Stats: 実行レコードが見つかりません (ID{})", exec_id)),
            ..Default::default()
        };
    };

    let session_file = if exec.runner == "gl" {
        sai_link_engine::session::gemini_session_files(30)
            .into_iter()
            .next()
    } else if exec.runner == "cc" {
        sai_link_engine::session::cc_session_files(30)
            .into_iter()
            .next()
    } else {
        sai_link_engine::session::resolve_session_dir(
            exec.session_dir.as_deref(),
            exec_id,
        )
        .and_then(|dir| sai_link_engine::session::find_main_session_file(&dir).ok())
    };

    let Some(session_file) = session_file else {
        return CacheViewState {
            loaded_exec_id: Some(exec_id),
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
                    loaded_exec_id: Some(exec_id),
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
                    loaded_exec_id: Some(exec_id),
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
        chart_mode: CacheChartMode::default(),
        per_message_scroll: 0,
        error: None,
        loaded_exec_id: Some(exec_id),
    }
}

// ============================================================================
// 描画
// ============================================================================

/// Stats ビュー本体（メイン画面領域へ描画するウィジェット）。
pub struct CacheView<'a> {
    pub state: &'a CacheViewState,
}

impl Widget for CacheView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // 小さすぎる領域ではメッセージのみ（レイアウト制約のパニック防止）
        if area.width < 30 || area.height < 8 {
            Paragraph::new("Stats: area too small")
                .block(Block::default().borders(Borders::ALL).title("Stats"))
                .render(area, buf);
            return;
        }

        // エラー表示
        if let Some(err) = &self.state.error {
            Paragraph::new(err.clone())
                .block(Block::default().borders(Borders::ALL).title("Stats"))
                .render(area, buf);
            return;
        }
        let Some(metrics) = &self.state.metrics else {
            Paragraph::new("セッションが読み込まれていません")
                .block(Block::default().borders(Borders::ALL).title("Stats"))
                .render(area, buf);
            return;
        };

        // sai-cache-tui-rs と同じ配置:
        //   charts Fill(1) | pies 12 行 | bottom 33%（breakdown 2/3 + summary 1/3）
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Fill(1),
                Constraint::Length(12),
                Constraint::Percentage(33),
            ])
            .split(area);

        // ── 上段: Chart（1/2/3）または Stats テーブル（s） ──
        match self.state.chart_mode {
            CacheChartMode::StatsTable => render_stats_table(buf, chunks[0], metrics),
            _ => render_charts(buf, chunks[0], self.state, metrics),
        }

        // ── 中段: Token / Cost 構成比の円グラフ ──
        render_pies(buf, chunks[1], metrics);

        // ── 下段: Per-message breakdown（2/3）+ Summary（1/3） ──
        let bottom = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
            .split(chunks[2]);
        render_breakdown(buf, bottom[0], self.state, metrics);
        render_summary(buf, bottom[1], self.state, metrics);
    }
}

// ── チャート（Cache trend + Token trend） ────────────────────────────────

fn render_charts(
    buf: &mut Buffer,
    area: Rect,
    state: &CacheViewState,
    metrics: &CacheSessionMetrics,
) {
    if metrics.all_messages.is_empty() {
        Paragraph::new("No assistant messages with usage")
            .block(Block::default().borders(Borders::ALL).title("Chart"))
            .render(area, buf);
        return;
    }

    let halves = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_cache_trend(buf, halves[0], state, metrics);
    render_token_trend(buf, halves[1], state, metrics);
}

/// x 軸（turn）の共通構築。
fn turn_x_axis(x_max: f64) -> Axis<'static> {
    Axis::default()
        .title("turn")
        .bounds([0.0, x_max])
        .labels(vec![
            Span::raw("0"),
            Span::raw(format!("{}", (x_max / 2.0) as usize)),
            Span::raw(format!("{}", x_max as usize)),
        ])
}

/// y 軸（トークン数）の共通構築。
fn token_y_axis(y_max: f64) -> Axis<'static> {
    Axis::default()
        .title("tokens")
        .bounds([0.0, y_max])
        .labels(vec![
            Span::raw("0"),
            Span::raw(format!("{:.0}k", y_max / 2.0 / 1000.0)),
            Span::raw(format!("{:.0}k", y_max / 1000.0)),
        ])
}

/// 左半分: キャッシュトレンド（Per-turn % / 累積 % / 累積トークン）。
fn render_cache_trend(
    buf: &mut Buffer,
    area: Rect,
    state: &CacheViewState,
    metrics: &CacheSessionMetrics,
) {
    let x_max = metrics.all_messages.len() as f64;
    match state.chart_mode {
        CacheChartMode::PerTurn => {
            let data: Vec<(f64, f64)> = metrics
                .all_messages
                .iter()
                .enumerate()
                .map(|(i, m)| (i as f64, m.cache_hit_percent))
                .collect();
            let ds = Dataset::default()
                .name("hit%")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Cyan))
                .data(&data);
            let chart = Chart::new(vec![ds])
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Cache trend — Per-turn hit %"),
                )
                .x_axis(turn_x_axis(x_max))
                .y_axis(
                    Axis::default()
                        .title("hit%")
                        .bounds([0.0, 100.0])
                        .labels(vec![Span::raw("0%"), Span::raw("50%"), Span::raw("100%")]),
                );
            chart.render(area, buf);
        }
        CacheChartMode::CumulativePercent => {
            let data: Vec<(f64, f64)> = state
                .cumulative
                .cum_hit_percent
                .iter()
                .enumerate()
                .map(|(i, v)| (i as f64, *v))
                .collect();
            let ds = Dataset::default()
                .name("cum hit%")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Magenta))
                .data(&data);
            let chart = Chart::new(vec![ds])
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Cache trend — Cumulative hit %"),
                )
                .x_axis(turn_x_axis(x_max))
                .y_axis(
                    Axis::default()
                        .title("hit%")
                        .bounds([0.0, 100.0])
                        .labels(vec![Span::raw("0%"), Span::raw("50%"), Span::raw("100%")]),
                );
            chart.render(area, buf);
        }
        CacheChartMode::CumulativeTotal => {
            let d1: Vec<(f64, f64)> = state
                .cumulative
                .cum_input
                .iter()
                .enumerate()
                .map(|(i, v)| (i as f64, *v))
                .collect();
            let d2: Vec<(f64, f64)> = state
                .cumulative
                .cum_cache_read
                .iter()
                .enumerate()
                .map(|(i, v)| (i as f64, *v))
                .collect();
            let d3: Vec<(f64, f64)> = state
                .cumulative
                .cum_cache_write
                .iter()
                .enumerate()
                .map(|(i, v)| (i as f64, *v))
                .collect();
            let max = d1
                .last()
                .map(|(_, v)| *v)
                .unwrap_or(0.0)
                .max(d2.last().map(|(_, v)| *v).unwrap_or(0.0))
                .max(d3.last().map(|(_, v)| *v).unwrap_or(0.0));
            let y_max = (max * 1.15).max(1.0);
            let ds1 = Dataset::default()
                .name("cum input")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::White))
                .data(&d1);
            let ds2 = Dataset::default()
                .name("cum read")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Green))
                .data(&d2);
            let ds3 = Dataset::default()
                .name("cum write")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Yellow))
                .data(&d3);
            let chart = Chart::new(vec![ds1, ds2, ds3])
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Cache trend — Cumulative tokens"),
                )
                .x_axis(turn_x_axis(x_max))
                .y_axis(token_y_axis(y_max));
            chart.render(area, buf);
        }
        CacheChartMode::StatsTable => {
            unreachable!("render_cache_trend は StatsTable 以外でのみ呼ばれる")
        }
    }
}

/// 右半分: トークントレンド（Per-turn / 累積 input/output）。
fn render_token_trend(
    buf: &mut Buffer,
    area: Rect,
    state: &CacheViewState,
    metrics: &CacheSessionMetrics,
) {
    let x_max = metrics.all_messages.len() as f64;
    match state.chart_mode {
        CacheChartMode::PerTurn => {
            let data_in: Vec<(f64, f64)> = metrics
                .all_messages
                .iter()
                .enumerate()
                .map(|(i, m)| (i as f64, m.input as f64))
                .collect();
            let data_out: Vec<(f64, f64)> = metrics
                .all_messages
                .iter()
                .enumerate()
                .map(|(i, m)| (i as f64, m.output as f64))
                .collect();
            let max = data_in
                .iter()
                .map(|(_, y)| *y)
                .fold(0.0_f64, f64::max)
                .max(data_out.iter().map(|(_, y)| *y).fold(0.0_f64, f64::max));
            let y_max = (max * 1.25).max(10.0);
            let ds_in = Dataset::default()
                .name("input")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Cyan))
                .data(&data_in);
            let ds_out = Dataset::default()
                .name("output")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Green))
                .data(&data_out);
            let chart = Chart::new(vec![ds_in, ds_out])
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Token trend — Per-turn input/output"),
                )
                .x_axis(turn_x_axis(x_max))
                .y_axis(token_y_axis(y_max));
            chart.render(area, buf);
        }
        CacheChartMode::CumulativePercent | CacheChartMode::CumulativeTotal => {
            let mut cum_in: Vec<(f64, f64)> = Vec::with_capacity(metrics.all_messages.len());
            let mut cum_out: Vec<(f64, f64)> = Vec::with_capacity(metrics.all_messages.len());
            let (mut sum_in, mut sum_out) = (0u64, 0u64);
            for (i, m) in metrics.all_messages.iter().enumerate() {
                sum_in += m.input;
                sum_out += m.output;
                cum_in.push((i as f64, sum_in as f64));
                cum_out.push((i as f64, sum_out as f64));
            }
            let max = cum_in
                .last()
                .map(|(_, y)| *y)
                .unwrap_or(0.0)
                .max(cum_out.last().map(|(_, y)| *y).unwrap_or(0.0));
            let y_max = (max * 1.15).max(10.0);
            let ds_in = Dataset::default()
                .name("cum input")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Cyan))
                .data(&cum_in);
            let ds_out = Dataset::default()
                .name("cum output")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Green))
                .data(&cum_out);
            let chart = Chart::new(vec![ds_in, ds_out])
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Token trend — Cumulative input/output"),
                )
                .x_axis(turn_x_axis(x_max))
                .y_axis(token_y_axis(y_max));
            chart.render(area, buf);
        }
        CacheChartMode::StatsTable => {
            unreachable!("render_token_trend は StatsTable 以外でのみ呼ばれる")
        }
    }
}

/// 中段: Token / Cost 構成比の円グラフ 2 枚。
fn render_pies(buf: &mut Buffer, area: Rect, metrics: &CacheSessionMetrics) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let b = &metrics.active_branch_totals;

    // Token 円グラフ: input / output / cache
    let tok_cache = b.cache_read + b.cache_write;
    let tok_total = b.input as f64 + b.output as f64 + tok_cache as f64;
    let (tok_in_v, tok_out_v, tok_cache_v) = if tok_total > 0.0 {
        (b.input as f64, b.output as f64, tok_cache as f64)
    } else {
        (1.0, 1.0, 1.0)
    };
    let tok_pie = MiniPie {
        title: " Token proportion (input/output/cache) ".to_string(),
        slices: vec![
            PieSliceData {
                label: "input".into(),
                value: tok_in_v,
                color: Color::Cyan,
            },
            PieSliceData {
                label: "output".into(),
                value: tok_out_v,
                color: Color::Green,
            },
            PieSliceData {
                label: "cache".into(),
                value: tok_cache_v,
                color: Color::Yellow,
            },
        ],
    };

    // Cost 円グラフ: $in / $out / $cache
    let cost_cache = b.cost.cache_read + b.cost.cache_write;
    let cost_total = b.cost.total;
    let (c_in_v, c_out_v, c_cache_v) = if cost_total > 0.0 {
        (b.cost.input, b.cost.output, cost_cache)
    } else {
        (1.0, 1.0, 1.0)
    };
    let cost_pie = MiniPie {
        title: " Cost proportion ($in/$out/$cache) ".to_string(),
        slices: vec![
            PieSliceData {
                label: "$in".into(),
                value: c_in_v,
                color: Color::Magenta,
            },
            PieSliceData {
                label: "$out".into(),
                value: c_out_v,
                color: Color::Red,
            },
            PieSliceData {
                label: "$cache".into(),
                value: c_cache_v,
                color: Color::Yellow,
            },
        ],
    };

    tok_pie.render(cols[0], buf);
    cost_pie.render(cols[1], buf);
}

// ── テーブル（Stats / Per-message breakdown） ────────────────────────────

/// テーブルのヘッダー行（turn〜$tot の 9 列）。
fn table_header() -> Row<'static> {
    Row::new(vec![
        Cell::from("turn"),
        Cell::from("input"),
        Cell::from("output"),
        Cell::from("cache"),
        Cell::from("cache%"),
        Cell::from("$in"),
        Cell::from("$out"),
        Cell::from("$cac"),
        Cell::from("$tot"),
    ])
    .style(Style::default().fg(Color::Yellow).bold())
    .height(1)
}

/// メトリクス 1 件 → テーブル行。
fn metric_row(m: &AssistantUsageMetric) -> Row<'static> {
    Row::new(vec![
        Cell::from(format!("{}", m.sequence)),
        Cell::from(format!("{}", m.input)),
        Cell::from(format!("{}", m.output)),
        Cell::from(format!("{}", m.cache_read)),
        Cell::from(format!("{:.1}%", m.cache_hit_percent)),
        Cell::from(format!("{:.3}", m.cost.input)),
        Cell::from(format!("{:.3}", m.cost.output)),
        Cell::from(format!("{:.3}", m.cost.cache_read)),
        Cell::from(format!("{:.3}", m.cost.total)),
    ])
    .height(1)
}

/// テーブルの列幅（9 列）。
fn table_widths() -> [Constraint; 9] {
    [
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(9),
    ]
}

/// 上段: Stats テーブル（`s` モード。最新ターンを表示）。
fn render_stats_table(buf: &mut Buffer, area: Rect, metrics: &CacheSessionMetrics) {
    if metrics.all_messages.is_empty() {
        Paragraph::new("No assistant messages with usage")
            .block(Block::default().borders(Borders::ALL).title("Stats table"))
            .render(area, buf);
        return;
    }

    let messages = &metrics.all_messages;
    let visible_h = area.height.saturating_sub(3) as usize; // 枠 + ヘッダー
    let visible = visible_h.max(1);
    let offset = messages.len().saturating_sub(visible);
    let slice = &messages[offset..(offset + visible).min(messages.len())];

    let rows: Vec<Row> = slice.iter().map(metric_row).collect();
    let title = format!(
        "Stats table — per-message breakdown (showing {}/{})",
        slice.len(),
        messages.len()
    );

    let table = Table::new(rows, table_widths())
        .header(table_header())
        .block(Block::default().borders(Borders::ALL).title(title))
        .column_spacing(1);
    table.render(area, buf);
}

/// 下段左: Per-message breakdown（合計行付き・スクロール可能）。
fn render_breakdown(
    buf: &mut Buffer,
    area: Rect,
    state: &CacheViewState,
    metrics: &CacheSessionMetrics,
) {
    if metrics.all_messages.is_empty() {
        Paragraph::new("No assistant messages with usage")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Per-message breakdown"),
            )
            .render(area, buf);
        return;
    }

    // 全行 + 末尾に合計行
    let mut all_rows: Vec<Row> = metrics.all_messages.iter().map(metric_row).collect();
    let totals = &metrics.active_branch_totals;
    all_rows.push(
        Row::new(vec![
            Cell::from("total"),
            Cell::from(format!("{}", totals.input)),
            Cell::from(format!("{}", totals.output)),
            Cell::from(format!("{}", totals.cache_read)),
            Cell::from(format!("{:.1}%", summarize_hit_percent(totals))),
            Cell::from(format!("{:.3}", totals.cost.input)),
            Cell::from(format!("{:.3}", totals.cost.output)),
            Cell::from(format!("{:.3}", totals.cost.cache_read)),
            Cell::from(format!("{:.3}", totals.cost.total)),
        ])
        .style(Style::default().fg(Color::Cyan).bold())
        .height(1),
    );

    let visible_h = area.height.saturating_sub(3) as usize; // 枠 + ヘッダー
    let visible = visible_h.max(1);
    let max_offset = all_rows.len().saturating_sub(visible);
    let offset = state.per_message_scroll.min(max_offset);
    let rows: Vec<Row> = all_rows[offset..(offset + visible).min(all_rows.len())].to_vec();

    let title = format!(
        "Per-message breakdown — {}/{} (PgUp/PgDn scroll) [{}-{}]",
        offset + 1,
        all_rows.len(),
        offset + 1,
        offset + rows.len()
    );

    let table = Table::new(rows, table_widths())
        .header(table_header())
        .block(Block::default().borders(Borders::ALL).title(title))
        .column_spacing(1);
    table.render(area, buf);
}

/// セッションフォルダ名の復号（`--home-user-dir--` → `home/user/dir`）。
fn format_folder(name: &str) -> String {
    let base = name.strip_suffix(".jsonl").unwrap_or(name);
    let stripped = if base.starts_with("--") && base.ends_with("--") && base.len() >= 4 {
        &base[2..base.len() - 2]
    } else {
        base.trim_matches('-')
    };
    if stripped.is_empty() {
        return base.to_string();
    }
    stripped.replace('-', "/")
}

/// ファイル名から日付部分を抽出（`<yyyy-mm-ddThh-mm-ss>-xxx.jsonl` 形式）。
fn parse_date_file(file_name: &str) -> (String, String) {
    let base = file_name.strip_suffix(".jsonl").unwrap_or(file_name);
    if base.len() >= 19 && base.as_bytes().get(10) == Some(&b'T') {
        let date_part = &base[0..19];
        let valid = date_part.as_bytes()[4] == b'-'
            && date_part.as_bytes()[7] == b'-'
            && date_part.as_bytes()[10] == b'T'
            && date_part.as_bytes()[13] == b'-'
            && date_part.as_bytes()[16] == b'-';
        if valid {
            let y = &date_part[0..4];
            let m = &date_part[5..7];
            let d = &date_part[8..10];
            let hh = &date_part[11..13];
            let mm = &date_part[14..16];
            let ss = &date_part[17..19];
            let date_str = format!("{}/{}/{} : {}:{}:{}", y, m, d, hh, mm, ss);
            let remainder = if base.len() > 20 { &base[20..] } else { "" };
            let file_str = if remainder.is_empty() {
                format!("{}.jsonl", base)
            } else {
                format!("{}.jsonl", remainder)
            };
            if !remainder.is_empty() {
                return (date_str, file_str);
            }
        }
    }
    ("unknown".to_string(), file_name.to_string())
}

/// 下段右: Summary（provider / model / folder / date / file / メッセージ数）。
fn render_summary(
    buf: &mut Buffer,
    area: Rect,
    state: &CacheViewState,
    metrics: &CacheSessionMetrics,
) {
    let (provider, model) = metrics
        .all_messages
        .last()
        .map(|m| (m.provider.as_str(), m.model.as_str()))
        .unwrap_or(("unknown", "unknown"));

    let path = state.session_file.clone().unwrap_or_default();
    let folder_encoded = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let folder_inner = format_folder(folder_encoded);
    let folder_display = if folder_inner.is_empty() {
        "/".to_string()
    } else if folder_inner.starts_with('/') {
        folder_inner
    } else {
        format!("/{}", folder_inner)
    };
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let (date_str, file_str) = parse_date_file(file_name);

    let cnt = &state.counts;
    let lines = vec![
        Line::from(vec![
            Span::styled("provider:", Style::default().fg(Color::Cyan).bold()),
            Span::styled(format!(" {}", provider), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("model:   ", Style::default().fg(Color::Green).bold()),
            Span::styled(format!(" {}", model), Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::styled("folder:  ", Style::default().fg(Color::Yellow).bold()),
            Span::styled(
                format!(" {}", folder_display),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::styled("date:    ", Style::default().fg(Color::Magenta).bold()),
            Span::styled(
                format!(" {}", date_str),
                Style::default().fg(Color::Magenta),
            ),
        ]),
        Line::from(vec![
            Span::styled("file:    ", Style::default().fg(Color::White).bold()),
            Span::styled(format!(" {}", file_str), Style::default().fg(Color::White)),
        ]),
        Line::from(Span::raw("")),
        Line::from(vec![
            Span::styled("user:     ", Style::default().fg(Color::Blue).bold()),
            Span::styled(format!("{}", cnt.user), Style::default().fg(Color::Blue)),
            Span::styled("  assistant:", Style::default().fg(Color::Cyan).bold()),
            Span::styled(
                format!(" {}", cnt.assistant),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled("thinking: ", Style::default().fg(Color::Yellow).bold()),
            Span::styled(
                format!("{}", cnt.thinking),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("  toolcall: ", Style::default().fg(Color::Red).bold()),
            Span::styled(format!("{}", cnt.toolcall), Style::default().fg(Color::Red)),
        ]),
    ];
    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Summary "))
        .render(area, buf);
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// テスト用のセッション jsonl を作成する。
    fn write_sample_jsonl(dir: &Path) -> PathBuf {
        let path = dir.join("2026-08-29T10-00-00-main_test.jsonl");
        let lines = [
            r#"{"type":"message","id":"e1","parentId":null,"timestamp":"2026-08-29T10:00:00Z","message":{"role":"user","content":[{"type":"text"}]}}"#,
            r#"{"type":"message","id":"e2","parentId":"e1","timestamp":"2026-08-29T10:00:01Z","message":{"role":"assistant","provider":"anthropic","model":"claude","usage":{"input":100,"output":50,"cacheRead":300,"cacheWrite":100,"totalTokens":550,"cost":{"input":0.001,"output":0.002,"cacheRead":0.0003,"cacheWrite":0.001,"total":0.0043}}}}"#,
            r#"{"type":"message","id":"e3","parentId":"e2","timestamp":"2026-08-29T10:00:02Z","message":{"role":"user","content":[{"type":"text"}]}}"#,
            r#"{"type":"message","id":"e4","parentId":"e3","timestamp":"2026-08-29T10:00:03Z","message":{"role":"assistant","provider":"anthropic","model":"claude","usage":{"input":200,"output":80,"cacheRead":400,"cacheWrite":0,"totalTokens":680,"cost":{"input":0.002,"output":0.003,"cacheRead":0.0004,"cacheWrite":0.0,"total":0.0054}}}}"#,
        ];
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        path
    }

    /// チャートモードが `1 → 2 → 3 → s → 1` の順に循環すること。
    #[test]
    fn chart_mode_cycles() {
        let mut mode = CacheChartMode::PerTurn;
        let mut seen = vec![mode];
        for _ in 0..3 {
            mode = mode.next();
            seen.push(mode);
        }
        assert_eq!(
            seen,
            vec![
                CacheChartMode::PerTurn,
                CacheChartMode::CumulativePercent,
                CacheChartMode::CumulativeTotal,
                CacheChartMode::StatsTable,
            ]
        );
        assert_eq!(mode.next(), CacheChartMode::PerTurn);
    }

    /// CacheView ウィジェットがパニックせず描画できること（円グラフ・テーブル含む）。
    #[test]
    fn cache_view_renders_without_panic() {
        let dir = std::env::temp_dir().join(format!("sai_cache_r{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_sample_jsonl(&dir);

        let metrics = collect_metrics_from_file(&path).unwrap();
        let cumulative = compute_cumulative_series(&metrics.all_messages);
        let counts = collect_input_counts(&path).unwrap();

        for mode in [
            CacheChartMode::PerTurn,
            CacheChartMode::CumulativePercent,
            CacheChartMode::CumulativeTotal,
            CacheChartMode::StatsTable,
        ] {
            let state = CacheViewState {
                session_file: Some(path.clone()),
                metrics: Some(metrics.clone()),
                cumulative: cumulative.clone(),
                counts: counts.clone(),
                chart_mode: mode,
                per_message_scroll: 0,
                error: None,
                loaded_exec_id: None,
            };
            let area = Rect::new(0, 0, 100, 40);
            let mut buf = Buffer::empty(area);
            CacheView { state: &state }.render(area, &mut buf);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// フォルダ名復号・日付抽出のヘルパー。
    #[test]
    fn folder_and_date_helpers() {
        assert_eq!(format_folder("--home-user-dir--"), "home/user/dir");
        assert_eq!(format_folder("plain"), "plain");
        let (d, f) = parse_date_file("2026-08-29T10-00-00-main_test.jsonl");
        assert_eq!(d, "2026/08/29 : 10:00:00");
        assert_eq!(f, "main_test.jsonl");
        let (d, f) = parse_date_file("unknown.jsonl");
        assert_eq!(d, "unknown");
        assert_eq!(f, "unknown.jsonl");
    }
}
