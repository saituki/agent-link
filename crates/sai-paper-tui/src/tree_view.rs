//! 研究方針（sai-sci の木）を読むだけのページ。
//!
//! U23 で Canvas のページを置き換えてできたもの。画面には種の文・関門・
//! 負債・項目の一覧・選んだ項目の中身を出す。木の形は変えない。
//! 操作は Web の研究方針の画面か `sai-moon-paper sci` で行う（R-M3）。

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

/// 研究方針のページの状態。
#[derive(Debug, Default)]
pub struct TreeState {
    /// 木全体の様子（`status.nodes` が一覧）。
    pub status: Option<papr_core::sci::compute::Status>,
    /// 選んだ節点の中身。
    pub detail: Option<papr_core::sci::compute::NodeDetail>,
    /// `status.nodes` の中の位置。
    pub selected: usize,
    /// 次に読み直したとき選ぶ節点（要対応から来たとき。`N12` の数）。
    pub pending_select: Option<i64>,
    /// 読めなかったときの文。
    pub error: Option<String>,
    /// true なら status と中身を読み直す。
    pub stale: bool,
    /// true なら選んだ節点の中身だけを読み直す。
    pub detail_stale: bool,
}

/// 木を読み直す。
///
/// `stale` なら status を作り直し、`pending_select` があればその節点を選ぶ。
/// どちらかが立っていれば選んだ節点の中身を読み直し、2 つの旗を下ろす。
/// 木が無ければ status と detail を None にする。読めなければ error に文を入れる。
pub fn reload(state: &mut TreeState, db: &papr_core::Database, roots: &papr_core::PaperRoots) {
    if !state.stale && !state.detail_stale {
        return;
    }
    let full = state.stale;
    state.stale = false;
    state.detail_stale = false;
    state.error = None;

    if full {
        let defs = papr_core::sci::defs::load(&roots.root);
        let initialized = papr_core::sci::store::is_initialized(db.connection()).unwrap_or(false);
        if !initialized {
            state.status = None;
            state.detail = None;
            state.selected = 0;
            return;
        }
        match papr_core::sci::compute::status(db.connection(), roots, &defs) {
            Ok(status) => {
                if let Some(wanted) = state.pending_select.take()
                    && let Some(index) = status.nodes.iter().position(|node| {
                        papr_core::sci::parse_ref(&node.id, 'N').ok() == Some(wanted)
                    })
                {
                    state.selected = index;
                }
                state.selected = clamp_selected(state.selected, &status);
                state.status = Some(status);
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

/// 選んだ節点の中身だけを読む。
fn reload_detail(state: &mut TreeState, db: &papr_core::Database, roots: &papr_core::PaperRoots) {
    let Some(status) = state.status.as_ref() else {
        state.detail = None;
        return;
    };
    let Some(node) = status.nodes.get(state.selected) else {
        state.detail = None;
        return;
    };
    let defs = papr_core::sci::defs::load(&roots.root);
    let Ok(id) = papr_core::sci::parse_ref(&node.id, 'N') else {
        state.detail = None;
        return;
    };
    match papr_core::sci::compute::node_detail(db.connection(), roots, &defs, id) {
        Ok(detail) => state.detail = Some(detail),
        Err(error) => {
            state.detail = None;
            state.error = Some(error.to_string());
        }
    }
}

/// 選択位置を木の高さに収める。
fn clamp_selected(index: usize, status: &papr_core::sci::compute::Status) -> usize {
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

    // 上の 3 行（種・関門・負債）は常に全体に出す。
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
            Layout::horizontal([Constraint::Percentage(38), Constraint::Min(20)]).split(body);
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

/// 上の 3 行（種の文・関門・負債）。
fn build_header_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![seed_line(app.tree.status.as_ref(), theme)];
    lines.push(gate_line(app.tree.status.as_ref(), theme));
    lines.push(debts_line(app.tree.status.as_ref(), theme));
    lines
}

/// 種の文。空欄の部分は強調の色にする。
fn seed_line(status: Option<&papr_core::sci::compute::Status>, theme: &Theme) -> Line<'static> {
    let Some(seed) = status.and_then(|s| s.seed.as_ref()) else {
        return Line::styled(" 種: まだありません", Style::default().fg(theme.muted));
    };
    let mut spans: Vec<Span<'static>> =
        vec![Span::styled(" 種 ", Style::default().fg(theme.secondary))];
    for part in &seed.parts {
        let color = if part.filled {
            if part.tentative {
                theme.warning
            } else {
                theme.text
            }
        } else {
            theme.accent
        };
        let style = if part.filled {
            Style::default().fg(color)
        } else {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        };
        spans.push(Span::styled(part.text.clone(), style));
    }
    Line::from(spans)
}

/// 関門の進み（例「関門 2/7 小目的」）。
fn gate_line(status: Option<&papr_core::sci::compute::Status>, theme: &Theme) -> Line<'static> {
    let Some(gate) = status.and_then(|s| s.gate.as_ref()) else {
        return Line::styled(" 関門: —", Style::default().fg(theme.muted));
    };
    Line::from(vec![
        Span::styled(" 関門 ", Style::default().fg(theme.secondary)),
        Span::styled(
            format!("{}/{} ", gate.current, gate.total),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Span::styled(gate.name.clone(), Style::default().fg(theme.text)),
        Span::styled(
            format!(
                "  （{}/{} 通過）",
                gate.list.iter().filter(|row| row.passed).count(),
                gate.total
            ),
            Style::default().fg(theme.muted),
        ),
    ])
}

/// 負債の 4 つの数。
fn debts_line(status: Option<&papr_core::sci::compute::Status>, theme: &Theme) -> Line<'static> {
    let Some(status) = status else {
        return Line::styled(" 負債: —", Style::default().fg(theme.muted));
    };
    let debts = status.debts;
    let parts: [(&str, i64); 4] = [
        ("仕分けていない提案", debts.pending_proposals),
        ("読んだが反映していない論文", debts.unreflected_papers),
        ("選別していない検索", debts.unscreened_sets),
        ("書いていない振り返り", debts.empty_reviews),
    ];
    let mut spans: Vec<Span<'static>> =
        vec![Span::styled(" 負債 ", Style::default().fg(theme.secondary))];
    let mut first = true;
    for (name, count) in parts {
        if !first {
            spans.push(Span::styled("  ", Style::default()));
        }
        first = false;
        if count > 0 {
            spans.push(Span::styled(
                format!("{name} {count}"),
                Style::default().fg(theme.warning),
            ));
        } else {
            spans.push(Span::styled(
                format!("{name} 0"),
                Style::default().fg(theme.muted),
            ));
        }
    }
    Line::from(spans)
}

/// 状態の印（C-01 の値から）。
fn state_mark(state: &str) -> &'static str {
    match state {
        "decided" => "●",
        "tentative" => "◐",
        "open" => "○",
        "excluded" => "×",
        "candidate" => "◇",
        "selected" => "◆",
        "rejected" => "✕",
        "dormant" => "◌",
        _ => " ",
    }
}

/// 空欄の種類と印の表示名（C-01）。
fn blank_label(blank: Option<&str>) -> Option<String> {
    blank.map(|value| match value {
        "untouched" => "未着手".to_string(),
        "in_progress" => "調査中".to_string(),
        "insufficient" => "情報不足".to_string(),
        "awaiting_decision" => "判断待ち".to_string(),
        "conflict" => "対立".to_string(),
        other => other.to_string(),
    })
}

/// 印の表示名（C-01）。
fn mark_label(mark: &str) -> String {
    match mark {
        "weak_evidence" => "根拠不足".to_string(),
        "recheck" => "要再確認".to_string(),
        other => other.to_string(),
    }
}

/// 項目の型の表示名（C-01）。
fn type_label(value: &str) -> &'static str {
    match value {
        "choose" => "選ぶ",
        "list" => "並べる",
        "write" => "書く",
        "verify" => "確かめる",
        _ => "",
    }
}

/// 左の欄（項目の一覧）。
fn build_list_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let Some(status) = app.tree.status.as_ref() else {
        return empty_lines(app, theme);
    };
    let mut lines: Vec<Line<'static>> = vec![Line::raw("")];
    if status.nodes.is_empty() {
        lines.push(Line::styled(
            " 項目がまだありません".to_string(),
            Style::default().fg(theme.muted),
        ));
    }
    for (index, node) in status.nodes.iter().enumerate() {
        lines.push(list_line(node, index == app.tree.selected, theme));
    }
    if !status.next.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            " 次の一手 ".to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        for next in status.next.iter().take(5) {
            let node = next
                .node
                .as_ref()
                .map_or_else(String::new, |id| format!(" {id} "));
            lines.push(Line::from(vec![
                Span::styled("  • ".to_string(), Style::default().fg(theme.muted)),
                Span::styled(node, Style::default().fg(theme.secondary)),
                Span::styled(next.purpose.clone(), Style::default().fg(theme.text)),
            ]));
        }
    }
    lines
}

/// 木が無いときの案内。status が無ければ「始めてください」を出す。
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

/// 一覧の 1 行（深さで字下げ、状態印、ID・題・空欄の種類か印）。
fn list_line(
    node: &papr_core::sci::compute::NodeRow,
    selected: bool,
    theme: &Theme,
) -> Line<'static> {
    let indent = "  ".repeat(usize::try_from(node.depth.max(0)).unwrap_or(0) + 1);
    let mark = state_mark(&node.state);
    let mark_color = match node.state.as_str() {
        "decided" => theme.success,
        "tentative" => theme.warning,
        "excluded" | "rejected" => theme.muted,
        "open" | "candidate" => theme.text,
        "selected" => theme.success,
        _ => theme.text,
    };
    let tail = blank_label(node.blank.as_deref())
        .map(|label| format!("［{label}］"))
        .or_else(|| {
            node.marks
                .first()
                .map(|mark| format!("［{}］", mark_label(mark)))
        })
        .unwrap_or_else(|| {
            let types: Vec<&str> = node
                .types
                .iter()
                .map(|t| type_label(t))
                .filter(|s| !s.is_empty())
                .collect();
            if types.is_empty() {
                String::new()
            } else {
                format!("（{}）", types.join("・"))
            }
        });

    let base = Style::default().fg(theme.text);
    let style = if selected {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    };
    let prefix = if selected { "▶ " } else { "  " };
    let counts = if node.counts.options > 0 {
        format!("  候補{}", node.counts.options)
    } else if node.counts.papers > 0 {
        format!("  論文{}", node.counts.papers)
    } else {
        String::new()
    };
    Line::from(vec![
        Span::styled(
            format!("{prefix}{indent}"),
            Style::default().fg(theme.accent),
        ),
        Span::styled(mark.to_string(), Style::default().fg(mark_color)),
        Span::styled(
            format!(" {} ", node.id),
            Style::default().fg(theme.secondary),
        ),
        Span::styled(node.title.clone(), style),
        Span::styled(counts, Style::default().fg(theme.muted)),
        Span::styled(tail, Style::default().fg(theme.warning)),
    ])
}

/// 右の欄（選んだ節点の中身）。
fn build_detail_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let Some(detail) = app.tree.detail.as_ref() else {
        return vec![Line::raw("")];
    };
    let mut lines = vec![Line::raw("")];

    // 本文
    if !detail.node.body.trim().is_empty() {
        for line in detail.node.body.lines() {
            lines.push(Line::styled(
                format!(" {line}"),
                Style::default().fg(theme.text),
            ));
        }
    }

    // 問い
    if !detail.questions.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section_heading("問い", theme));
        for question in detail.questions.iter().take(8) {
            let state = if question.state == "open" {
                "未解決"
            } else {
                "解決"
            };
            let color = if question.state == "open" {
                theme.warning
            } else {
                theme.muted
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", question.id),
                    Style::default().fg(theme.secondary),
                ),
                Span::styled(question.text.clone(), Style::default().fg(theme.text)),
                Span::styled(format!("［{state}］"), Style::default().fg(color)),
            ]));
        }
    }

    // 候補
    if !detail.options.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section_heading("候補", theme));
        for option in detail.options.iter().take(10) {
            let label = match option.state.as_str() {
                "selected" => "選択",
                "rejected" => "却下",
                "dormant" => "休眠",
                _ => "候補",
            };
            let color = match option.state.as_str() {
                "selected" => theme.success,
                "rejected" => theme.muted,
                "dormant" => theme.warning,
                _ => theme.text,
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", option.id),
                    Style::default().fg(theme.secondary),
                ),
                Span::styled(
                    state_mark(&option.state).to_string(),
                    Style::default().fg(color),
                ),
                Span::styled(format!(" {} ", option.title), Style::default().fg(color)),
                Span::styled(
                    format!(
                        "{label}  支持 {} / 反証 {} / 論文 {}",
                        option.support, option.refute, option.papers
                    ),
                    Style::default().fg(theme.muted),
                ),
            ]));
        }
    }

    // 知見
    if !detail.findings.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section_heading("知見", theme));
        for finding in detail.findings.iter().take(10) {
            let polarity = match finding.polarity.as_str() {
                "support" => "支持",
                "refute" => "反証",
                "neutral" => "中立",
                _ => "該当なし",
            };
            let color = match finding.polarity.as_str() {
                "support" => theme.success,
                "refute" => theme.error,
                "neutral" => theme.text,
                _ => theme.muted,
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", finding.id),
                    Style::default().fg(theme.secondary),
                ),
                Span::styled(format!("{polarity} "), Style::default().fg(color)),
                Span::styled(finding.text.clone(), Style::default().fg(theme.text)),
                Span::styled(
                    format!("（{}）", finding.source),
                    Style::default().fg(theme.muted),
                ),
            ]));
        }
    }

    // 論文
    if !detail.links.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section_heading("論文", theme));
        for link in detail.links.iter().take(10) {
            let role = link.role.as_deref().map_or("", paper_role_label);
            let read = link.read_state.as_deref().map_or("", read_state_label);
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", link.id),
                    Style::default().fg(theme.secondary),
                ),
                Span::styled(link.title.clone(), Style::default().fg(theme.text)),
                Span::styled(
                    format!("（{role} / {read}）"),
                    Style::default().fg(theme.muted),
                ),
            ]));
        }
    }

    // 次の一手
    if !detail.blocks.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section_heading("次の一手", theme));
        for block in detail.blocks.iter().take(3) {
            lines.push(Line::from(vec![
                Span::styled("  • ".to_string(), Style::default().fg(theme.muted)),
                Span::styled(
                    block
                        .block
                        .clone()
                        .unwrap_or_else(|| "人の作業".to_string()),
                    Style::default().fg(theme.secondary),
                ),
                Span::styled(
                    format!(" {}", block.purpose),
                    Style::default().fg(theme.text),
                ),
                Span::styled(
                    format!("（{}）", block.why),
                    Style::default().fg(theme.warning),
                ),
            ]));
        }
    }

    // 直近の決定の記録 1 件
    if let Some(decision) = detail.decisions.first() {
        lines.push(Line::raw(""));
        lines.push(section_heading("直近の決定", theme));
        let action = decision_action_label(&decision.action);
        let option = decision.option.clone().unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {action} {option} "),
                Style::default().fg(theme.success),
            ),
            Span::styled(decision.reason.clone(), Style::default().fg(theme.text)),
        ]));
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

/// 論文の役割の表示名（C-01）。
fn paper_role_label(role: &str) -> &str {
    match role {
        "exemplar" => "代表例",
        "compare" => "比較対象",
        "issue_evidence" => "課題の根拠",
        "method_source" => "手法の供給元",
        "data_source" => "データの出典",
        "background" => "背景",
        "counter" => "反証",
        _ => "",
    }
}

/// 論文の読み具合の表示名（C-01）。
fn read_state_label(state: &str) -> &str {
    match state {
        "unread" => "未読",
        "read" => "読んだ・未反映",
        "reflected" => "反映済み",
        "irrelevant" => "関係なし",
        _ => "",
    }
}

/// 決定の記録の種類の表示名（C-01）。
fn decision_action_label(action: &str) -> &str {
    match action {
        "decide" => "決定",
        "tentative" => "仮決定",
        "reopen" => "再開",
        "exclude_item" => "対象外にする",
        "reject_option" => "候補の却下",
        "dormant_option" => "候補の休眠",
        "restore_option" => "候補に戻す",
        _ => action,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use papr_core::sci::compute::{NodeDetail, NodeRow, Status};

    /// 一時フォルダのルート（`defs::load` は `<root>/.sai/paper` を読む）。
    ///
    /// 終えたら呼び出し側が [`remove_temp_roots`] で消す。
    fn temp_roots(tag: &str) -> papr_core::PaperRoots {
        let root = std::env::temp_dir().join(format!(
            "papr-tree-{}-{}-{tag}",
            std::process::id(),
            unique_suffix(),
        ));
        let data_dir = root.join(".sai").join("paper");
        std::fs::create_dir_all(&data_dir).expect("データフォルダ");
        papr_core::PaperRoots { root, data_dir }
    }

    /// 一時フォルダを消す。
    fn remove_temp_roots(roots: &papr_core::PaperRoots) {
        let _ = std::fs::remove_dir_all(&roots.root);
    }

    /// 同じテストを 2 回走らせても衝突しないよう、実行ごとに違う名前を付ける。
    fn unique_suffix() -> u128 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let clock = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        clock ^ u128::from(n) << 32
    }

    /// 節点 3 つを持つ status を作る。
    fn status_with(ids: &[&str]) -> Status {
        Status {
            nodes: ids
                .iter()
                .enumerate()
                .map(|(index, id)| NodeRow {
                    id: (*id).to_string(),
                    title: format!("項目{id}"),
                    depth: i64::try_from(index).unwrap_or(0),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    /// U23: `pending_select` の節点が選ばれ、中身が入ること。
    #[test]
    fn reload_reads_tree_and_pending_select() -> Result<(), Box<dyn std::error::Error>> {
        let db = papr_core::Database::in_memory()?;
        let roots = temp_roots("select");
        let defs = papr_core::sci::defs::load(&roots.root);
        papr_core::sci::defs::init_tree(db.connection(), &defs, "problem")?;

        let mut state = TreeState {
            stale: true,
            // 一番上から 3 番目の節点を選ぶ。
            pending_select: Some(3),
            ..Default::default()
        };
        reload(&mut state, &db, &roots);

        let status = state.status.as_ref().expect("木が読める");
        assert!(status.initialized);
        assert!(!status.nodes.is_empty());
        assert!(
            !state.stale && !state.detail_stale,
            "旗は読み直したあと下りる"
        );
        // pending_select の節点が選ばれている。
        let picked = status.nodes[state.selected].id.clone();
        assert_eq!(
            papr_core::sci::parse_ref(&picked, 'N').ok(),
            Some(3),
            "pending_select の節点が選ばれている: {picked}"
        );
        assert!(state.detail.is_some(), "選んだ節点の中身が入る");
        assert!(state.error.is_none(), "{:?}", state.error);

        let detail = state.detail.as_ref().expect("中身");
        assert_eq!(detail.node.id, picked);
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

    /// 上の 3 行（種・関門・負債）と一覧の行が出る。
    #[test]
    fn render_tree_shows_seed_gate_and_list() {
        use ratatui::{Terminal, backend::TestBackend};

        let theme = Theme::load("nord").expect("テーマ");
        let mut app = App::default();
        let mut status = status_with(&["N1", "N2"]);
        status.initialized = true;
        status.seed = Some(papr_core::sci::compute::SeedSentence {
            text: "頭皮脳波で".into(),
            parts: vec![papr_core::sci::compute::SeedPart {
                text: "頭皮脳波で".into(),
                filled: true,
                ..Default::default()
            }],
        });
        status.gate = Some(papr_core::sci::compute::GateState {
            current: 2,
            total: 7,
            name: "小目的".into(),
            ..Default::default()
        });
        app.tree.status = Some(status);
        app.tree.detail = Some(NodeDetail {
            node: NodeRow {
                id: "N1".into(),
                title: "出発点".into(),
                body: "先行研究を做的事".into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let mut terminal = Terminal::new(TestBackend::new(140, 30)).expect("端末");
        terminal
            .draw(|frame| render_tree(frame, frame.area(), &mut app, &theme))
            .expect("描く");

        let buffer = terminal.backend().buffer().clone();
        let text = compact(buffer.content().iter().map(|cell| cell.symbol()));
        assert!(text.contains("頭皮脳波で"), "種の文が出ていない: {text}");
        assert!(text.contains("関門"), "関門が出ていない: {text}");
        assert!(text.contains("2/7"), "関門の数が出ていない: {text}");
        assert!(text.contains("負債"), "負債が出ていない: {text}");
        assert!(text.contains("N1"), "項目の一覧が出ていない: {text}");
        assert!(
            text.contains("先行研究を做的事"),
            "選んだ項目の中身が出ていない: {text}"
        );
        assert!(text.contains("r:読み直す"), "案内が出ていない: {text}");
    }

    /// 木が無いときは「まだありません」の案内が出る。
    #[test]
    fn render_tree_shows_hint_without_tree() {
        use ratatui::{Terminal, backend::TestBackend};

        let theme = Theme::load("nord").expect("テーマ");
        let mut app = App::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("端末");
        terminal
            .draw(|frame| render_tree(frame, frame.area(), &mut app, &theme))
            .expect("描く");

        let buffer = terminal.backend().buffer().clone();
        let text = compact(buffer.content().iter().map(|cell| cell.symbol()));
        assert!(
            text.contains("研究方針の木はまだありません"),
            "案内の文が出ていない: {text}"
        );
    }

    /// 読めなかったときはエラーの文が出る。
    #[test]
    fn render_tree_shows_error() {
        use ratatui::{Terminal, backend::TestBackend};

        let theme = Theme::load("nord").expect("テーマ");
        let mut app = App::default();
        app.tree.error = Some("閉じている".into());
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("端末");
        terminal
            .draw(|frame| render_tree(frame, frame.area(), &mut app, &theme))
            .expect("描く");

        let buffer = terminal.backend().buffer().clone();
        let text = compact(buffer.content().iter().map(|cell| cell.symbol()));
        assert!(
            text.contains("研究方針の木を読めませんでした"),
            "エラーの文が出ていない: {text}"
        );
    }

    /// ワイド文字のセル分割を吸収して、空白を詰めた文字列にする。
    fn compact<'a>(symbols: impl Iterator<Item = &'a str>) -> String {
        symbols
            .flat_map(str::chars)
            .filter(|c| !c.is_whitespace())
            .collect()
    }
}
