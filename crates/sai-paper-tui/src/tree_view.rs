//! 研究方針（Q・O・F・E の木）を読むだけのページ。
//!
//! 画面には SEED の下書き・進み具合・DEBT と仕分け待ちの数・Node の一覧・
//! 選んだ Node の中身を出す。計算は `rt::compute` の値をそのまま写す。
//! 操作は Web の研究方針の画面か `sai-moon-paper sci` で行う（TUI は読み取り専用）。

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::theme::Theme;
use crate::{App, UiAction};
use crossterm::event::{KeyCode, KeyEvent};

/// 右の欄を出す最小の幅（これより狭ければ中身は下の段に出す）。
const DETAIL_MIN_WIDTH: u16 = 90;

/// 案内行の文言（`r` / `o`）。
const HINT: &str = "操作は Web の研究方針の画面か sai-moon-paper sci で行います（r: 読み直す / o: tree.md を開く）";

/// 進み具合のバーのマス数。
const BAR_CELLS: usize = 10;

/// 研究方針のページの状態。
#[derive(Debug, Default)]
pub struct TreeState {
    /// 木全体の様子（`status.nodes` が一覧）。
    pub status: Option<papr_core::rt::compute::Status>,
    /// 選んだ Node の中身。
    pub detail: Option<papr_core::rt::compute::NodeDetail>,
    /// `status.nodes` の中の位置。
    pub selected: usize,
    /// 次に読み直したとき選ぶ Node（要対応から来たとき。`rt_nodes.id`）。
    pub pending_select: Option<i64>,
    /// 読めなかったときの文。
    pub error: Option<String>,
    /// true なら status と中身を読み直す。
    pub stale: bool,
    /// true なら選んだ Node の中身だけを読み直す。
    pub detail_stale: bool,
}

/// 木を読み直す。
///
/// `stale` なら status を作り直し、`pending_select` があればその Node を選ぶ。
/// どちらかが立っていれば選んだ Node の中身を読み直し、2 つの旗を下ろす。
/// 木が無ければ status と detail を None にする（古い木だけのときは注意を出すため status は残す）。
/// 読めなければ error に文を入れる。
pub fn reload(state: &mut TreeState, db: &papr_core::Database, roots: &papr_core::PaperRoots) {
    if !state.stale && !state.detail_stale {
        return;
    }
    let full = state.stale;
    state.stale = false;
    state.detail_stale = false;
    state.error = None;

    if full {
        let defs = papr_core::rt::defs::load(&roots.root);
        match papr_core::rt::compute::status(db.connection(), roots, &defs) {
            Ok(status) if status.initialized => {
                if let Some(wanted) = state.pending_select.take()
                    && let Some(index) = status.nodes.iter().position(|node| {
                        papr_core::rt::parse_id(&node.id).ok().map(|(_, n)| n) == Some(wanted)
                    })
                {
                    state.selected = index;
                }
                state.selected = clamp_selected(state.selected, &status);
                state.status = Some(status);
            }
            Ok(status) => {
                // 木が無い。古い木だけのときは作り直しの注意を出すため status を残す。
                state.status = status.legacy.then_some(status);
                state.detail = None;
                state.selected = 0;
                return;
            }
            Err(error) => {
                state.status = None;
                state.detail = None;
                state.selected = 0;
                state.error = Some(error.to_string());
                return;
            }
        }
    }
    reload_detail(state, db, roots);
}

/// 選んだ Node の中身だけを読む。
fn reload_detail(state: &mut TreeState, db: &papr_core::Database, roots: &papr_core::PaperRoots) {
    let Some(node) = state
        .status
        .as_ref()
        .and_then(|status| status.nodes.get(state.selected))
    else {
        state.detail = None;
        return;
    };
    let defs = papr_core::rt::defs::load(&roots.root);
    match papr_core::rt::compute::node_detail(db.connection(), roots, &defs, &node.id) {
        Ok(detail) => state.detail = Some(detail),
        Err(error) => {
            state.detail = None;
            state.error = Some(error.to_string());
        }
    }
}

/// 選択位置を木の高さに収める。
fn clamp_selected(index: usize, status: &papr_core::rt::compute::Status) -> usize {
    index.min(status.nodes.len().saturating_sub(1))
}

/// このページだけのキー（`r`: 読み直す、`o`: tree.md を開く）。
///
/// 返り値が `None` なら共通のキー（`j` / `k` など）へ処理を任せる。
pub fn handle_tree_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    match key.code {
        KeyCode::Char('r') => {
            app.tree.stale = true;
            None
        }
        KeyCode::Char('o') => Some(UiAction::OpenTreeMd),
        _ => None,
    }
}

// ---- 描き方 -----------------------------------------------------------------

/// ページを描く（`render_content` の中の `inset` に描く）。
pub fn render_tree(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let block = Block::default()
        .title(" 研究方針 ")
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    // 上の 3 行（SEED・進み具合・DEBT）は常に全体に出す。
    let header_rows = Layout::vertical([Constraint::Length(1); 3]).split(inner);
    for (rect, line) in header_rows
        .iter()
        .zip(build_header_lines(app, theme).into_iter().take(3))
    {
        frame.render_widget(Paragraph::new(line), *rect);
    }
    // 残りは一覧と中身。最後の 1 行は案内。
    let rest_height = inner.height.saturating_sub(4);
    if rest_height == 0 {
        return;
    }
    let rest = Rect::new(inner.x, inner.y + 3, inner.width, rest_height);
    frame.render_widget(Paragraph::new(hint_line(theme)), rest);

    let wide = rest.width >= DETAIL_MIN_WIDTH;
    let body = Rect::new(rest.x, rest.y, rest.width, rest_height.saturating_sub(1));
    if body.height == 0 {
        return;
    }
    let list_lines = build_list_lines(app, theme);
    if wide {
        let columns =
            Layout::horizontal([Constraint::Percentage(45), Constraint::Min(20)]).split(body);
        frame.render_widget(Paragraph::new(list_lines), columns[0]);
        frame.render_widget(Paragraph::new(build_detail_lines(app, theme)), columns[1]);
    } else {
        // 狭いときは一覧の下に中身を出す。
        let rows = Layout::vertical([Constraint::Percentage(45), Constraint::Min(4)]).split(body);
        frame.render_widget(Paragraph::new(list_lines), rows[0]);
        frame.render_widget(Paragraph::new(build_detail_lines(app, theme)), rows[1]);
    }
}

/// 案内行。
fn hint_line(theme: &Theme) -> Line<'static> {
    Line::styled(format!(" {HINT}"), Style::default().fg(theme.muted))
}

/// 上の 3 行（SEED・進み具合・DEBT と仕分け待ち）。
fn build_header_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let status = app.tree.status.as_ref();
    vec![
        seed_line(status, theme),
        progress_line(status, theme),
        debts_line(status, theme),
    ]
}

/// SEED の下書き。埋まっていない差し込みは警告の色にする。
fn seed_line(status: Option<&papr_core::rt::compute::Status>, theme: &Theme) -> Line<'static> {
    let Some(seed) = status.and_then(|s| s.seed_draft.as_ref()) else {
        return Line::styled(" SEED: まだありません", Style::default().fg(theme.muted));
    };
    let mut spans: Vec<Span<'static>> =
        vec![Span::styled(" SEED ", Style::default().fg(theme.secondary))];
    for part in &seed.parts {
        let style = if part.filled {
            Style::default().fg(theme.text)
        } else {
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD)
        };
        spans.push(Span::styled(part.text.clone(), style));
    }
    Line::from(spans)
}

/// 進み具合（例「進み具合 12/24  [■■■■■□□□□□]」）。古い木だけなら作り直しの注意。
fn progress_line(
    status: Option<&papr_core::rt::compute::Status>,
    theme: &Theme,
) -> Line<'static> {
    let Some(status) = status else {
        return Line::styled(" 進み具合: —", Style::default().fg(theme.muted));
    };
    if status.legacy {
        return Line::styled(
            " 古い形式の木です。作り直してください（sai-moon-paper sci init）",
            Style::default().fg(theme.warning),
        );
    }
    let progress = status.progress;
    let filled = if progress.total > 0 {
        usize::try_from(progress.answered.clamp(0, progress.total)).unwrap_or(0) * BAR_CELLS
            / usize::try_from(progress.total).unwrap_or(1)
    } else {
        0
    };
    let bar = format!(
        "[{}{}]",
        "■".repeat(filled),
        "□".repeat(BAR_CELLS - filled)
    );
    Line::from(vec![
        Span::styled(" 進み具合 ", Style::default().fg(theme.secondary)),
        Span::styled(
            format!("{}/{}", progress.answered, progress.total),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {bar}"), Style::default().fg(theme.success)),
    ])
}

/// DEBT・仕分け待ち・次の一手（例「DEBT 3 / PROPOSAL 仕分け待ち 4 / Next: …」）。
fn debts_line(status: Option<&papr_core::rt::compute::Status>, theme: &Theme) -> Line<'static> {
    let Some(status) = status.filter(|s| s.initialized) else {
        return Line::styled(" DEBT: —", Style::default().fg(theme.muted));
    };
    let count_color = |n: usize| if n > 0 { theme.warning } else { theme.muted };
    let next = status
        .next
        .first()
        .map_or("—".to_string(), |next| next.purpose.clone());
    Line::from(vec![
        Span::styled(
            format!(" DEBT {}", status.debt_count),
            Style::default().fg(count_color(status.debt_count)),
        ),
        Span::styled(" / ", Style::default().fg(theme.muted)),
        Span::styled(
            format!("PROPOSAL 仕分け待ち {}", status.pending_proposals),
            Style::default().fg(count_color(status.pending_proposals)),
        ),
        Span::styled(" / ", Style::default().fg(theme.muted)),
        Span::styled(format!("Next: {next}"), Style::default().fg(theme.text)),
    ])
}

/// 左の欄（Node の一覧）。
fn build_list_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let Some(status) = app.tree.status.as_ref().filter(|s| s.initialized) else {
        return empty_lines(app, theme);
    };
    let mut lines: Vec<Line<'static>> = vec![Line::raw("")];
    for (index, node) in status.nodes.iter().enumerate() {
        lines.push(list_line(node, index == app.tree.selected, theme));
    }
    lines
}

/// 木が無いときの案内。古い木だけなら作り直しの注意、読めなければ error を出す。
fn empty_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    if let Some(error) = app.tree.error.as_ref() {
        return vec![
            Line::raw(""),
            Line::styled(
                " 研究方針の木を読めませんでした".to_string(),
                Style::default().fg(theme.error),
            ),
            Line::styled(format!("  {error}"), Style::default().fg(theme.error)),
        ];
    }
    if app.tree.status.as_ref().is_some_and(|s| s.legacy) {
        return vec![
            Line::raw(""),
            Line::styled(
                " 古い形式の木だけがあります".to_string(),
                Style::default().fg(theme.warning),
            ),
            Line::styled(
                "  新しい木を作り直してください".to_string(),
                Style::default().fg(theme.muted),
            ),
        ];
    }
    vec![
        Line::raw(""),
        Line::styled(
            " 研究方針の木はまだありません".to_string(),
            Style::default().fg(theme.text),
        ),
        Line::styled(
            "  sai-moon-paper sci init --type <型> で始めます".to_string(),
            Style::default().fg(theme.muted),
        ),
    ]
}

/// 値の頭を大文字にする（`answer` → `Answer`）。
fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// 一覧の 1 行（深さで字下げ、ID・文、種類ごとの要約）。
fn list_line(
    node: &papr_core::rt::compute::NodeJson,
    selected: bool,
    theme: &Theme,
) -> Line<'static> {
    let indent = "  ".repeat(node.depth);
    let prefix = if selected { "▶ " } else { "  " };
    let base = if node.on_path { theme.text } else { theme.muted };
    let text_style = if selected {
        Style::default().fg(base).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(base)
    };
    let mut spans = vec![
        Span::styled(
            format!("{prefix}{indent}"),
            Style::default().fg(theme.accent),
        ),
        Span::styled(format!("{} ", node.id), Style::default().fg(theme.secondary)),
    ];
    match node.kind.as_str() {
        "O" => {
            let star = if node.starred {
                format!(
                    "★ {} ",
                    node.answer_kind.as_deref().map_or_else(String::new, capitalize)
                )
            } else {
                "  ".to_string()
            };
            spans.push(Span::styled(star, Style::default().fg(theme.warning)));
            spans.push(Span::styled(node.text.clone(), text_style));
            spans.push(Span::styled(
                format!("  F {}", node.counts.f),
                Style::default().fg(theme.muted),
            ));
        }
        "F" => {
            spans.push(Span::styled(node.text.clone(), text_style));
            if !node.cites.is_empty() {
                spans.push(Span::styled(
                    format!("  [{}]", node.cites.join(", ")),
                    Style::default().fg(theme.muted),
                ));
            }
        }
        "E" => {
            spans.push(Span::styled(node.text.clone(), text_style));
            if let Some(mlflow) = &node.mlflow {
                spans.push(Span::styled(
                    format!("  mlflow: {mlflow}"),
                    Style::default().fg(theme.muted),
                ));
            }
        }
        _ => {
            spans.push(Span::styled(node.text.clone(), text_style));
            if let Some(obj) = &node.objectivity {
                let color = if obj.met { theme.success } else { theme.warning };
                let check = if obj.met { " ✓" } else { "" };
                spans.push(Span::styled(
                    format!("  Obj {}/{}{check}", obj.count, obj.target),
                    Style::default().fg(color),
                ));
            }
            if let Some(logic) = &node.logicality {
                spans.push(Span::styled(
                    format!("  Logic {}", logic.value),
                    Style::default().fg(logic_color(&logic.value, theme)),
                ));
            }
        }
    }
    Line::from(spans)
}

/// Logicality の値の色。
fn logic_color(value: &str, theme: &Theme) -> ratatui::style::Color {
    match value {
        "ok" => theme.success,
        "contradiction" => theme.error,
        _ => theme.muted,
    }
}

/// 右の欄（選んだ Node の中身）。
fn build_detail_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let Some(detail) = app.tree.detail.as_ref() else {
        return vec![Line::raw("")];
    };
    let node = &detail.node;
    let mut lines = vec![Line::raw("")];

    // 見出し・本文
    lines.push(Line::from(vec![
        Span::styled(
            format!(" {} ", node.id),
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("［{}］ ", node.kind), Style::default().fg(theme.muted)),
        Span::styled(node.text.clone(), Style::default().fg(theme.text)),
    ]));
    for line in node.body.lines().filter(|l| !l.trim().is_empty()) {
        lines.push(Line::styled(
            format!(" {line}"),
            Style::default().fg(theme.text),
        ));
    }

    // 親までの道
    if detail.path.len() > 1 {
        let path = detail
            .path
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>()
            .join(" › ");
        lines.push(Line::styled(
            format!(" 道: {path}"),
            Style::default().fg(theme.muted),
        ));
    }

    // Q の Objectivity・Logicality・ガイド
    if let Some(obj) = &node.objectivity {
        lines.push(Line::raw(""));
        lines.push(section_heading("Objectivity", theme));
        lines.push(Line::styled(
            format!(
                "  {}/{}{}",
                obj.count,
                obj.target,
                if obj.met { " ✓" } else { "" }
            ),
            Style::default().fg(if obj.met { theme.success } else { theme.warning }),
        ));
    }
    if let Some(logic) = &node.logicality {
        lines.push(Line::raw(""));
        lines.push(section_heading("Logicality", theme));
        let checked = logic
            .checked_at
            .as_deref()
            .map_or(String::new(), |at| format!("（{at}）"));
        lines.push(Line::styled(
            format!("  {}{checked}", logic.value.to_uppercase()),
            Style::default().fg(logic_color(&logic.value, theme)),
        ));
        if !logic.note.trim().is_empty() {
            lines.push(Line::styled(
                format!("  {}", logic.note),
                Style::default().fg(theme.text),
            ));
        }
        if !logic.findings.is_empty() {
            lines.push(Line::styled(
                format!("  指摘の F: {}", logic.findings.join(", ")),
                Style::default().fg(theme.warning),
            ));
        }
    }
    if let Some(guide) = &detail.guide {
        lines.push(Line::raw(""));
        lines.push(section_heading(&format!("ガイド {}", guide.name), theme));
        lines.push(Line::styled(
            format!("  {}", guide.question),
            Style::default().fg(theme.text),
        ));
        for (label, values) in [
            ("Child Q", &guide.child_candidates),
            ("Extract", &guide.extract),
            ("Pitfall", &guide.pitfalls),
        ] {
            if !values.is_empty() {
                lines.push(Line::styled(
                    format!("  {label}: {}", values.join(" / ")),
                    Style::default().fg(theme.muted),
                ));
            }
        }
        lines.push(Line::styled(
            format!("  目安: O {} 個", guide.target),
            Style::default().fg(theme.muted),
        ));
    }

    // 子
    if !detail.children.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section_heading("子", theme));
        for child in detail.children.iter().take(12) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", child.id),
                    Style::default().fg(theme.secondary),
                ),
                Span::styled(child.text.clone(), Style::default().fg(theme.text)),
            ]));
        }
    }

    // Next Action
    if !detail.next.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section_heading("Next Action", theme));
        for next in detail.next.iter().take(3) {
            lines.push(Line::from(vec![
                Span::styled("  • ".to_string(), Style::default().fg(theme.muted)),
                Span::styled(
                    next.block.clone().unwrap_or_else(|| "人の作業".to_string()),
                    Style::default().fg(theme.secondary),
                ),
                Span::styled(format!(" {}", next.purpose), Style::default().fg(theme.text)),
                Span::styled(format!("（{}）", next.why), Style::default().fg(theme.warning)),
            ]));
        }
    }

    // Action の履歴（DEBT があれば ⚠ とその文）
    if !detail.actions.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section_heading("Action", theme));
        for action in detail.actions.iter().take(8) {
            let mark = if action.debt { "⚠ " } else { "  " };
            lines.push(Line::from(vec![
                Span::styled(format!("  {mark}"), Style::default().fg(theme.warning)),
                Span::styled(
                    format!("run {} {} ", action.run_id, action.block),
                    Style::default().fg(theme.secondary),
                ),
                Span::styled(action.purpose.clone(), Style::default().fg(theme.text)),
                Span::styled(
                    format!("  {}", action.occurred_at),
                    Style::default().fg(theme.muted),
                ),
            ]));
            for message in &action.debt_messages {
                lines.push(Line::styled(
                    format!("      {message}"),
                    Style::default().fg(theme.warning),
                ));
            }
        }
    }

    // 仕分け待ち
    if !detail.pending.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section_heading("仕分け待ち", theme));
        for pending in detail.pending.iter().take(8) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} {} ", pending.id, pending.op),
                    Style::default().fg(theme.secondary),
                ),
                Span::styled(pending.reason.clone(), Style::default().fg(theme.text)),
            ]));
        }
    }

    // 検索の記録
    if !detail.searches.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section_heading("検索の記録", theme));
        for search in detail.searches.iter().take(8) {
            lines.push(Line::styled(
                format!("  {} 「{}」 {} 件", search.tool, search.query, search.hits),
                Style::default().fg(theme.text),
            ));
        }
    }

    // 決定の記録
    if !detail.decisions.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section_heading("決定の記録", theme));
        for decision in detail.decisions.iter().take(8) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} {} ", decision.action, decision.detail),
                    Style::default().fg(theme.success),
                ),
                Span::styled(decision.reason.clone(), Style::default().fg(theme.text)),
            ]));
        }
    }

    lines
}

/// 節の見出し。
fn section_heading(text: &str, theme: &Theme) -> Line<'static> {
    Line::styled(
        format!(" {text} ─────────"),
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )
}

/// 他のモジュールのテストが使う、本物の木から作った status。
#[cfg(test)]
pub(crate) mod testing {
    use papr_core::rt::compute::Status;

    /// 一時フォルダのルート（`defs::load` は `<root>/.sai/paper` を読む）。
    ///
    /// 終えたら呼び出し側が [`remove_temp_roots`] で消す。
    pub(crate) fn temp_roots(tag: &str) -> papr_core::PaperRoots {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "papr-tree-{}-{n}-{tag}",
            std::process::id()
        ));
        let data_dir = root.join(".sai").join("paper");
        std::fs::create_dir_all(&data_dir).expect("データフォルダ");
        papr_core::PaperRoots { root, data_dir }
    }

    /// 一時フォルダを消す。
    pub(crate) fn remove_temp_roots(roots: &papr_core::PaperRoots) {
        let _ = std::fs::remove_dir_all(&roots.root);
    }

    /// problem の型で木を作って計算した status（Node は 3 つ以上ある）。
    pub(crate) fn sample_status(tag: &str) -> Status {
        let db = papr_core::Database::in_memory().expect("DB");
        let roots = temp_roots(tag);
        let defs = papr_core::rt::defs::load(&roots.root);
        papr_core::rt::defs::init_tree(db.connection(), &defs, "problem", "user").expect("木");
        let status =
            papr_core::rt::compute::status(db.connection(), &roots, &defs).expect("status");
        remove_temp_roots(&roots);
        status
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{remove_temp_roots, temp_roots};
    use super::*;

    use papr_core::rt::{Kind, store};

    /// 木に Node を 1 つ足す。
    fn add(
        db: &papr_core::Database,
        kind: Kind,
        parent: i64,
        text: &str,
        answer: Option<&str>,
    ) -> i64 {
        let node = store::NewNode {
            kind,
            parent_id: Some(parent),
            guide_key: None,
            text,
            body: "",
            answer_kind: answer,
            cites: &[],
            paper_id: None,
            run_id: None,
            report_id: None,
            mlflow_ref: None,
            proposal_id: None,
            created_by: "user",
        };
        store::insert_node(db.connection(), &node).expect("Node")
    }

    fn draw(app: &mut App, width: u16) -> String {
        use ratatui::{Terminal, backend::TestBackend};

        let theme = Theme::load("nord").expect("テーマ");
        let mut terminal = Terminal::new(TestBackend::new(width, 40)).expect("端末");
        terminal
            .draw(|frame| render_tree(frame, frame.area(), app, &theme))
            .expect("描く");
        let buffer = terminal.backend().buffer().clone();
        compact(buffer.content().iter().map(|cell| cell.symbol()))
    }

    /// `pending_select` の Node が選ばれ、中身が入ること。
    #[test]
    fn reload_reads_tree_and_pending_select() -> Result<(), Box<dyn std::error::Error>> {
        let db = papr_core::Database::in_memory()?;
        let roots = temp_roots("select");
        let defs = papr_core::rt::defs::load(&roots.root);
        papr_core::rt::defs::init_tree(db.connection(), &defs, "problem", "user")?;

        let mut state = TreeState {
            stale: true,
            // 一覧の 2 番目の Node を選ぶ。
            pending_select: None,
            ..Default::default()
        };
        reload(&mut state, &db, &roots);
        let status = state.status.as_ref().expect("木が読める");
        assert!(status.initialized);
        assert!(status.nodes.len() >= 2);
        let target = status.nodes[1].id.clone();
        let wanted = papr_core::rt::parse_id(&target)?.1;

        state.selected = 0;
        state.stale = true;
        state.pending_select = Some(wanted);
        reload(&mut state, &db, &roots);

        assert!(!state.stale && !state.detail_stale, "旗は読み直したあと下りる");
        let status = state.status.as_ref().expect("木が読める");
        assert_eq!(status.nodes[state.selected].id, target);
        assert_eq!(state.detail.as_ref().expect("中身").node.id, target);
        assert!(state.error.is_none(), "{:?}", state.error);
        remove_temp_roots(&roots);
        Ok(())
    }

    /// 木が無いときは status と detail が None になり案内が出る。
    #[test]
    fn reload_without_tree_clears_status() -> Result<(), Box<dyn std::error::Error>> {
        let db = papr_core::Database::in_memory()?;
        let roots = temp_roots("empty");
        let mut state = TreeState {
            stale: true,
            ..Default::default()
        };
        reload(&mut state, &db, &roots);
        assert!(state.status.is_none());
        assert!(state.detail.is_none());
        remove_temp_roots(&roots);
        Ok(())
    }

    /// AC2: 進み具合・Obj・Logic・★・Answer・F の件数が出る。
    #[test]
    fn render_tree_shows_progress_obj_logic_and_star() -> Result<(), Box<dyn std::error::Error>> {
        let db = papr_core::Database::in_memory()?;
        let roots = temp_roots("render");
        let defs = papr_core::rt::defs::load(&roots.root);
        let init = papr_core::rt::defs::init_tree(db.connection(), &defs, "problem", "user")?;
        let o = add(&db, Kind::O, init.root, "術前 MRI と頭皮脳波 120 例", Some("answer"));
        store::set_starred(db.connection(), o, true)?;
        add(&db, Kind::F, o, "施設の記録に 120 例ある", None);

        let mut app = App::default();
        app.tree.stale = true;
        reload(&mut app.tree, &db, &roots);
        remove_temp_roots(&roots);

        let text = draw(&mut app, 160);
        for want in ["進み具合", "Obj", "Logic", "★", "Answer", "DEBT", "PROPOSAL仕分け待ち", "施設の記録に120例ある"] {
            assert!(text.contains(want), "{want} が出ていない: {text}");
        }
        Ok(())
    }

    /// AC3: 木が無ければ始め方、古い木だけなら作り直しの注意が出る。
    #[test]
    fn render_tree_guides_without_tree_and_warns_legacy() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = App::default();
        let text = draw(&mut app, 100);
        assert!(text.contains("研究方針の木はまだありません"), "{text}");

        let db = papr_core::Database::in_memory()?;
        let roots = temp_roots("legacy");
        db.connection().execute_batch(
            "INSERT INTO sci_meta (key, value) VALUES ('initialized_at', '2026-01-01');",
        )?;
        app.tree.stale = true;
        reload(&mut app.tree, &db, &roots);
        remove_temp_roots(&roots);
        let text = draw(&mut app, 100);
        assert!(text.contains("作り直してください"), "{text}");
        assert!(text.contains("古い形式の木だけがあります"), "{text}");
        Ok(())
    }

    /// 読めなかったときはエラーの文が出る。
    #[test]
    fn render_tree_shows_error() {
        let mut app = App::default();
        app.tree.error = Some("閉じている".into());
        let text = draw(&mut app, 100);
        assert!(text.contains("研究方針の木を読めませんでした"), "{text}");
    }

    /// ワイド文字のセル分割を吸収して、空白を詰めた文字列にする。
    fn compact<'a>(symbols: impl Iterator<Item = &'a str>) -> String {
        symbols
            .flat_map(str::chars)
            .filter(|c| !c.is_whitespace())
            .collect()
    }
}
