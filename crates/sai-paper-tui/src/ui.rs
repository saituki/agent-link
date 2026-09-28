//! Ratatui rendering for the application shell.

use crate::state::{
    App, AppMode, DeletionTarget, DiscoverInputFocus, DiscoveryStatus, ImportStatus,
    LibraryTreeKind, LibraryView, ManualImportPaper, Page, RunningOperation,
};
use crate::theme::Theme;
use chrono::Datelike;

use papr_core::RemotePaper;
use papr_core::models::TagSummary;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
    },
};

const LOGO: &str = "[ P A P R ]";
const WORKSPACE_LIST_ITEM_GAP_ROWS: usize = 0;
/// sai-paper-tui のリストと同じ固定幅列（first author / year）。
const LIST_AUTHOR_WIDTH: usize = 12;
const LIST_YEAR_WIDTH: usize = 5;
/// R4: 年月列の幅（例: `2017/06`）。
const LIST_DATE_WIDTH: usize = 7;
/// R4: 被引用数列の幅（例: `80.0k`）。
const LIST_CITE_WIDTH: usize = 6;
/// R4: ジャーナル名列の幅。
const LIST_JOURNAL_WIDTH: usize = 12;
/// R2: Discover のヒット回数列の幅（例: `3*`）。
const LIST_HITS_WIDTH: usize = 5;

fn saturating_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// Build a workspace list item with the standard vertical separation.
///
/// Keeping the gap here means item renderers only describe their content.
fn workspace_list_item(mut lines: Vec<Line<'_>>) -> ListItem<'_> {
    lines.extend(std::iter::repeat_n(
        Line::raw(""),
        WORKSPACE_LIST_ITEM_GAP_ROWS,
    ));
    ListItem::new(lines)
}

fn focus_block<'a>(title: &'a str, focused: bool, theme: &Theme) -> Block<'a> {
    let border_style = if focused {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.border)
    };

    let title_style = if focused {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.muted)
    };

    let mut block = Block::default()
        .title(Line::styled(title, title_style))
        .borders(Borders::ALL)
        .border_style(border_style);

    if focused {
        block = block
            .border_type(BorderType::Thick)
            .border_set(border::THICK);
    }

    block
}

/// Return a list selection only while the Workspace owns keyboard focus.
///
/// The selected index remains in the application state at all times; omitting it
/// here only prevents Ratatui from drawing the selection treatment.
fn workspace_highlight_selection(app: &App, selected: usize) -> Option<usize> {
    app.content_focused.then_some(selected)
}

/// Resolve the user-visible timezone label for a local timestamp.
///
/// Priority:
/// 1. System timezone abbreviation if available from OS (e.g. IST, CEST, EDT, JST).
/// 2. Comprehensive IANA timezone lookup table mapping to standard abbreviations.
/// 3. Fallback to UTC offset enclosed in parentheses, e.g. `(+05:30)` or `(-07:00)`.
#[must_use]
pub fn resolve_timezone_display(local: &chrono::DateTime<chrono::Local>) -> String {
    let raw_tz = local.format("%Z").to_string();
    let trimmed = raw_tz.trim();

    // Check if OS provided a valid alphabetic abbreviation (e.g., IST, CEST, EDT, PDT, JST, UTC, AEST, BST, CET).
    if !trimmed.is_empty()
        && trimmed.len() <= 6
        && trimmed.chars().all(|c| c.is_ascii_alphabetic())
        && !trimmed.eq_ignore_ascii_case("LOCAL")
    {
        return trimmed.to_uppercase();
    }

    // Try mapping detected system IANA timezone identifier (e.g. "Asia/Kolkata", "America/New_York") to standard abbreviation.
    if let Ok(iana_tz) = iana_time_zone::get_timezone()
        && let Some(abbr) = iana_to_abbreviation(&iana_tz, local)
    {
        return abbr.to_string();
    }

    // Fallback: UTC offset enclosed in parentheses (e.g., (+05:30) or (-07:00))
    format!("({})", local.format("%:z"))
}

fn iana_to_abbreviation(
    iana: &str,
    local: &chrono::DateTime<chrono::Local>,
) -> Option<&'static str> {
    use chrono::Offset;
    let offset_secs = local.offset().fix().local_minus_utc();

    asia_pacific_abbreviation(iana, offset_secs)
        .or_else(|| american_abbreviation(iana, offset_secs))
        .or_else(|| european_abbreviation(iana, offset_secs))
}

fn asia_pacific_abbreviation(iana: &str, offset_secs: i32) -> Option<&'static str> {
    match iana {
        "Asia/Kolkata" | "Asia/Calcutta" | "Asia/Colombo" => Some("IST"),
        "Asia/Tokyo" => Some("JST"),
        "Asia/Seoul" => Some("KST"),
        "Asia/Shanghai" | "Asia/Chongqing" | "Asia/Harbin" | "Asia/Urumqi" | "Asia/Taipei" => {
            Some("CST")
        }
        "Asia/Hong_Kong" => Some("HKT"),
        "Asia/Singapore" => Some("SGT"),
        "Asia/Manila" => Some("PST"),
        "Asia/Kuala_Lumpur" | "Asia/Kuching" => Some("MYT"),
        "Australia/Sydney" | "Australia/Melbourne" | "Australia/Canberra" | "Australia/Hobart" => {
            if offset_secs == 11 * 3600 {
                Some("AEDT")
            } else {
                Some("AEST")
            }
        }
        "Australia/Brisbane" => Some("AEST"),
        "Australia/Adelaide" => {
            if offset_secs == 10 * 3600 + 1800 {
                Some("ACDT")
            } else {
                Some("ACST")
            }
        }
        "Australia/Darwin" => Some("ACST"),
        "Australia/Perth" => Some("AWST"),
        "Pacific/Auckland" | "NZ" => {
            if offset_secs == 13 * 3600 {
                Some("NZDT")
            } else {
                Some("NZST")
            }
        }
        "Asia/Dubai" => Some("GST"),
        "Asia/Riyadh" => Some("AST"),
        _ => None,
    }
}

fn american_abbreviation(iana: &str, offset_secs: i32) -> Option<&'static str> {
    match iana {
        "America/New_York"
        | "America/Detroit"
        | "America/Toronto"
        | "America/Montreal"
        | "America/Indiana/Indianapolis"
        | "US/Eastern" => {
            if offset_secs == -4 * 3600 {
                Some("EDT")
            } else {
                Some("EST")
            }
        }
        "America/Chicago" | "America/Winnipeg" | "America/Mexico_City" | "US/Central" => {
            if offset_secs == -5 * 3600 {
                Some("CDT")
            } else {
                Some("CST")
            }
        }
        "America/Denver" | "America/Edmonton" | "America/Boise" | "US/Mountain" => {
            if offset_secs == -6 * 3600 {
                Some("MDT")
            } else {
                Some("MST")
            }
        }
        "America/Phoenix" => Some("MST"),
        "America/Los_Angeles" | "America/Vancouver" | "America/Tijuana" | "US/Pacific" => {
            if offset_secs == -7 * 3600 {
                Some("PDT")
            } else {
                Some("PST")
            }
        }
        "America/Anchorage" => {
            if offset_secs == -8 * 3600 {
                Some("AKDT")
            } else {
                Some("AKST")
            }
        }
        "Pacific/Honolulu" => Some("HST"),
        _ => None,
    }
}

fn european_abbreviation(iana: &str, offset_secs: i32) -> Option<&'static str> {
    match iana {
        "Europe/London" | "Europe/Belfast" | "Europe/Dublin" => {
            if offset_secs == 3600 {
                Some("BST")
            } else {
                Some("GMT")
            }
        }
        "Europe/Berlin" | "Europe/Paris" | "Europe/Rome" | "Europe/Madrid" | "Europe/Amsterdam"
        | "Europe/Brussels" | "Europe/Vienna" | "Europe/Zurich" | "Europe/Stockholm"
        | "Europe/Oslo" | "Europe/Copenhagen" | "Europe/Prague" | "Europe/Warsaw"
        | "Europe/Budapest" | "Europe/Belgrade" => {
            if offset_secs == 2 * 3600 {
                Some("CEST")
            } else {
                Some("CET")
            }
        }
        "Europe/Athens" | "Europe/Helsinki" | "Europe/Bucharest" | "Europe/Kiev"
        | "Europe/Sofia" | "Europe/Tallinn" | "Europe/Riga" | "Europe/Vilnius" => {
            if offset_secs == 3 * 3600 {
                Some("EEST")
            } else {
                Some("EET")
            }
        }
        "Europe/Moscow" => Some("MSK"),
        "Europe/Istanbul" => Some("TRT"),
        "UTC" | "Etc/UTC" | "Etc/GMT" | "GMT" => Some("UTC"),
        _ => None,
    }
}

/// Format a UTC `DateTime` in the system's local timezone with timezone abbreviation or parenthesized offset.
pub fn format_local_datetime(dt: &chrono::DateTime<chrono::Utc>, fmt: &str) -> String {
    let local = dt.with_timezone(&chrono::Local);
    let tz = resolve_timezone_display(&local);
    format!("{} {}", local.format(fmt), tz)
}

/// Render the complete application for the current state.
pub fn render(frame: &mut Frame<'_>, app: &mut App, theme: &Theme) {
    if app.mode == AppMode::PdfView {
        crate::pdf_viewer::draw_pdf_viewer(frame, app);
        return;
    }
    if app.mode == AppMode::MarkdownView {
        let area = frame.area();
        frame.render_widget(
            Block::default().style(Style::default().bg(theme.background)),
            area,
        );
        if let Some(md) = app.md_view.as_mut() {
            crate::md_view::render_markdown_view(frame, area, md, theme);
        }
        crate::pdf_viewer::render_pending_kitty_cleanup(frame);
        return;
    }
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.background)),
        area,
    );
    if area.width < 58 || area.height < 18 {
        render_too_small(frame, area, theme);
        crate::pdf_viewer::render_pending_kitty_cleanup(frame);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(area);
    render_header(frame, rows[0], app, theme);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(32)])
        .split(rows[1]);
    render_sidebar(frame, columns[0], app, theme);
    render_content(frame, columns[1], app, theme);
    render_status(frame, rows[2], app, theme);

    match app.mode {
        AppMode::CommandPalette => render_palette(frame, app, theme),
        AppMode::TerminalCommand => render_terminal_command(frame, app, theme),
        AppMode::Help => render_help(frame, app, theme),
        AppMode::Tasks => render_tasks(frame, app, theme),
        AppMode::AttentionList => render_attention(frame, app, theme),
        AppMode::PaperDetail => render_paper_detail(frame, app, theme),
        AppMode::Prompt => render_metadata_prompt(frame, app, theme),
        AppMode::ConfirmDelete => render_delete_confirmation(frame, app, theme),
        AppMode::ManualImport => render_manual_import(frame, app, theme),
        AppMode::BlockPicker => render_block_picker(frame, app, theme),
        AppMode::RagSearch => render_rag_search(frame, app, theme),
        AppMode::Normal
        | AppMode::Search
        | AppMode::DiscoverFilter
        | AppMode::WorkspaceSearch
        | AppMode::PdfView
        | AppMode::MarkdownView
        | AppMode::QueryDetail => {}
    }

    // カーソルスタイルは常に点滅バー（旧 config editor の insert モード分岐は機能廃止で削除）。
    let cursor_style = crossterm::cursor::SetCursorStyle::BlinkingBar;
    crate::pdf_viewer::render_pending_kitty_cleanup(frame);
    let _ = crossterm::execute!(std::io::stdout(), cursor_style);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let title = Line::from(vec![
        Span::styled(
            LOGO,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(
            app.page.title(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
    ]);
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(theme.border));
    frame.render_widget(Paragraph::new(title).block(block), area);
    let shortcut = Rect::new(area.x + area.width.saturating_sub(18), area.y, 18, 1);
    frame.render_widget(
        Paragraph::new("Ctrl+B  Browse")
            .alignment(Alignment::Right)
            .style(Style::default().fg(theme.muted)),
        shortcut,
    );
}

fn render_sidebar(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let focused = !app.content_focused;
    let block = focus_block(" NAVIGATION ", focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items = Page::ALL
        .iter()
        .map(|page| ListItem::new(format!("  {}", page.title())));
    let list = List::new(items)
        .style(Style::default().fg(theme.muted))
        .highlight_style(if app.content_focused {
            Style::default().fg(theme.text).bg(theme.surface)
        } else {
            Style::default()
                .fg(theme.background)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        })
        .highlight_symbol("›");
    let mut state = ListState::default()
        .with_selected(Some(app.sidebar_index))
        .with_offset(app.sidebar_scroll);
    frame.render_stateful_widget(list, inner, &mut state);
    app.sidebar_scroll = state.offset();
}

fn render_workspace_search_bar(frame: &mut Frame<'_>, area: Rect, app: &App, theme: &Theme) {
    let style = if app.mode == AppMode::WorkspaceSearch {
        Style::default().fg(theme.text).bg(theme.surface)
    } else {
        Style::default().fg(theme.muted)
    };
    let border_style = if app.mode == AppMode::WorkspaceSearch {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.border)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(" Local Search (Press: > to toggle) ");

    let mut text = app.workspace_query.clone();
    if text.is_empty() && app.mode != AppMode::WorkspaceSearch {
        "Type to filter...".clone_into(&mut text);
    }
    frame.render_widget(Paragraph::new(text).style(style).block(block), area);

    if app.mode == AppMode::WorkspaceSearch {
        let cursor_offset = app
            .workspace_query
            .chars()
            .take(app.workspace_query_cursor)
            .count();
        frame.set_cursor_position((
            area.x
                .saturating_add(1)
                .saturating_add(u16::try_from(cursor_offset).unwrap_or(0)),
            area.y.saturating_add(1),
        ));
    }
}

/// Tags ページ（仕様 §3.1）: DB 由来タグ一覧。
fn render_tags(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    // R2: タグ選択中はそのタグが付いた論文一覧を表示する（R4 と共通の行形式）。
    if let Some(tag) = app.active_tag.clone() {
        render_tag_papers(frame, area, app, theme, &tag);
        return;
    }
    if app.tags.is_empty() {
        frame.render_widget(
            Paragraph::new("No tags yet. Select a paper and press t to add one.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.muted)),
            area,
        );
        return;
    }
    let items = app.tags.iter().map(|tag| {
        workspace_list_item(vec![
            Line::styled(
                &tag.name,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::styled(
                format!("{} papers", tag.paper_count),
                Style::default().fg(theme.muted),
            ),
        ])
    });
    let list = List::new(items)
        .block(
            Block::default()
                .title(" TAGS - ENTER LIST PAPERS  t ADD TAG TO PAPER ")
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.border)),
        )
        .highlight_style(Style::default().bg(theme.surface).fg(theme.accent))
        .highlight_symbol("> ");
    let mut state = ListState::default()
        .with_selected(workspace_highlight_selection(app, app.tag_selected))
        .with_offset(app.tag_scroll);
    frame.render_stateful_widget(list, area, &mut state);
    app.tag_scroll = state.offset();
}

/// R2: 選択中タグに紐づく論文一覧（R4 と共通の行形式）。
/// `Esc` / `h` でタグ一覧へ戻る。`t` で論文ごとのタグ設定（R1 と同じ動作）。
fn render_tag_papers(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut App,
    theme: &Theme,
    tag: &TagSummary,
) {
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(4)]).split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Tags / ", Style::default().fg(theme.muted)),
            Span::styled(
                &tag.name,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "   h/Esc back   Enter/p open PDF   t add tag",
                Style::default().fg(theme.muted),
            ),
        ])),
        rows[0],
    );
    if app.tag_papers.is_empty() {
        frame.render_widget(
            Paragraph::new("No papers with this tag.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.muted)),
            rows[1],
        );
        return;
    }
    let available_width = rows[1].width.saturating_sub(2) as usize;
    let items = app.tag_papers.iter().map(|paper| {
        let year_month = published_year_month(paper.published_at.as_deref());
        workspace_list_item(paper_list_columns_line(
            &year_month,
            paper.citation_count,
            paper.journal.as_deref(),
            display_paper_title(paper.display_name()),
            theme,
            available_width,
            None,
        ))
    });
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" {} PAPERS ", app.tag_papers.len()))
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.border)),
        )
        .highlight_style(Style::default().bg(theme.surface).fg(theme.accent))
        .highlight_symbol("> ");
    let mut state = ListState::default()
        .with_selected(workspace_highlight_selection(app, app.tag_paper_selected))
        .with_offset(app.tag_paper_scroll);
    frame.render_stateful_widget(list, rows[1], &mut state);
    app.tag_paper_scroll = state.offset();
}

fn render_authors(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    if app.active_author.is_some() {
        render_author_papers(frame, area, app, theme);
        return;
    }
    let authors = app.filtered_authors();
    if authors.is_empty() {
        frame.render_widget(
            Paragraph::new("No authors found in your library.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.muted)),
            area,
        );
        return;
    }
    let items = authors.iter().map(|author| {
        workspace_list_item(vec![
            Line::styled(
                &author.name,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::styled(
                format!("{} papers", author.paper_count),
                Style::default().fg(theme.muted),
            ),
        ])
    });
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" {} AUTHORS - ENTER VIEW ", authors.len()))
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.border)),
        )
        .highlight_style(Style::default().bg(theme.surface).fg(theme.accent))
        .highlight_symbol("> ");
    let mut state = ListState::default()
        .with_selected(workspace_highlight_selection(app, app.author_selected))
        .with_offset(app.author_scroll);
    frame.render_stateful_widget(list, area, &mut state);
    app.author_scroll = state.offset();
}

fn render_author_papers(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let Some(author) = &app.active_author else {
        return;
    };
    let papers = app.filtered_author_papers();
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(4)]).split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Authors / ", Style::default().fg(theme.muted)),
            Span::styled(
                &author.name,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "   h/Esc back   Enter/Right/l open PDF",
                Style::default().fg(theme.muted),
            ),
        ])),
        rows[0],
    );
    if papers.is_empty() {
        frame.render_widget(
            Paragraph::new("No papers found for this author.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.muted)),
            rows[1],
        );
        return;
    }
    let available_width = rows[1].width.saturating_sub(2) as usize;
    let items = papers.iter().map(|paper| {
        workspace_list_item(paper_list_line(
            first_author_surname(&paper.authors),
            published_year(paper.published_at.as_deref().unwrap_or("")),
            display_paper_title(paper.display_name()),
            theme,
            available_width,
            None,
        ))
    });
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" {} PAPERS ", papers.len()))
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.border)),
        )
        .highlight_style(Style::default().bg(theme.surface).fg(theme.accent))
        .highlight_symbol("> ");
    let mut state = ListState::default()
        .with_selected(workspace_highlight_selection(
            app,
            app.author_paper_selected,
        ))
        .with_offset(app.author_paper_scroll);
    frame.render_stateful_widget(list, rows[1], &mut state);
    app.author_paper_scroll = state.offset();
}

fn render_search_sets(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    // 2 ペイン：左にセット一覧、右に候補一覧
    let columns =
        Layout::horizontal([Constraint::Ratio(1, 3), Constraint::Ratio(2, 3)]).split(area);

    if app.search_sets.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "No search sets yet. Create one with: sai-paper search <query> --save-as <name>",
            )
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.muted)),
            area,
        );
        return;
    }

    render_search_set_list(frame, columns[0], app, theme);
    render_search_set_items(frame, columns[1], app, theme);
}

fn render_search_set_list(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let items = app.search_sets.iter().map(|set| {
        let undecided = app
            .search_set_items
            .iter()
            .filter(|item| item.decision == "pending")
            .count();

        workspace_list_item(vec![
            Line::styled(
                &set.name,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::styled(
                format!("{} items, {} undecided", set.id, undecided),
                Style::default().fg(theme.muted),
            ),
            Line::styled(
                format!("{} · {}", set.created_by, &set.created_at[..10]),
                Style::default().fg(theme.muted),
            ),
        ])
    });

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(
                    " {} SEARCH SETS - TAB/l VIEW ITEMS  r RERUN ",
                    app.search_sets.len()
                ))
                .borders(Borders::ALL)
                .border_style(if !app.search_set_focused_on_items {
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.border)
                }),
        )
        .highlight_style(Style::default().bg(theme.surface).fg(theme.accent))
        .highlight_symbol("> ");

    let mut state = ListState::default()
        .with_selected(workspace_highlight_selection(app, app.search_set_selected))
        .with_offset(app.search_set_scroll);

    frame.render_stateful_widget(list, area, &mut state);
    app.search_set_scroll = state.offset();
}

fn render_search_set_items(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    if app.search_set_items.is_empty() {
        frame.render_widget(
            Paragraph::new("No items in this set.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.muted)),
            area,
        );
        return;
    }

    let _available_width = area.width.saturating_sub(2) as usize;
    let items = app.search_set_items.iter().map(|item| {
        let year = item
            .remote
            .published
            .with_timezone(&chrono::Local)
            .year()
            .to_string();
        let first_author = item
            .remote
            .authors
            .first()
            .map(|a| a.as_str())
            .unwrap_or("-");

        let decision_marker = match item.decision.as_str() {
            "keep" => "✓ ",
            "drop" => "✗ ",
            "downloaded" => "⬇ ",
            _ => "  ",
        };

        workspace_list_item(vec![
            Line::styled(
                format!(
                    "{}{}",
                    decision_marker,
                    display_paper_title(&item.remote.title)
                ),
                match item.decision.as_str() {
                    "keep" => Style::default().fg(theme.accent),
                    "drop" => Style::default().fg(theme.muted),
                    _ => Style::default(),
                },
            ),
            Line::styled(
                format!(
                    "  {} · {} · {}",
                    first_author,
                    year,
                    item.remote.journal_ref.as_deref().unwrap_or("-")
                ),
                Style::default().fg(theme.muted),
            ),
        ])
    });

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(
                    " {} CANDIDATES - Tab/h BACK ",
                    app.search_set_items.len()
                ))
                .borders(Borders::ALL)
                .border_style(if app.search_set_focused_on_items {
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.border)
                }),
        )
        .highlight_style(Style::default().bg(theme.surface).fg(theme.accent))
        .highlight_symbol("> ");

    let mut state = ListState::default()
        .with_selected(workspace_highlight_selection(
            app,
            app.search_set_item_selected,
        ))
        .with_offset(app.search_set_item_scroll);

    frame.render_stateful_widget(list, area, &mut state);
    app.search_set_item_scroll = state.offset();
}

fn render_organization(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let available_width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem<'_>> = match app.page {
        Page::Notes => app
            .filtered_notes_papers()
            .iter()
            .map(|paper| {
                let lines = build_paper_lines(
                    app,
                    theme,
                    PaperLineContext {
                        paper_id: Some(paper.id),
                        title: paper.display_name(),
                        authors: &paper.authors,
                        reading_status: &paper.reading_status,
                        file_size: paper.file_size,
                        availability: None,
                        bookmark_year: None,
                        bookmark_journal: None,
                        bookmark_doi: None,
                        bookmark_page: None,
                        download_label: None,
                        available_width,
                    },
                );
                workspace_list_item(lines)
            })
            .collect(),
        _ => Vec::new(),
    };
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(format!("No {} yet", app.page.title().to_ascii_lowercase()))
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.muted)),
            area,
        );
    } else {
        let (title, selected_idx, scroll_idx) = match app.page {
            Page::Notes => (
                " NOTES - j/k SELECT  ENTER/n EDIT  x DELETE ",
                app.notes_selected,
                app.notes_scroll,
            ),
            _ => ("", 0, 0),
        };
        let list = List::new(items)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(theme.border)),
            )
            .style(Style::default().fg(theme.text))
            .highlight_style(Style::default().bg(theme.surface).fg(theme.accent))
            .highlight_symbol("> ");
        let mut state = ListState::default()
            .with_selected(workspace_highlight_selection(app, selected_idx))
            .with_offset(scroll_idx);
        frame.render_stateful_widget(list, area, &mut state);
        match app.page {
            Page::Notes => app.notes_scroll = state.offset(),
            _ => {}
        }
    }
}

fn chunk_string(s: &str, size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current_chunk = String::new();
    for c in s.chars() {
        if current_chunk.chars().count() == size {
            chunks.push(current_chunk);
            current_chunk = String::new();
        }
        current_chunk.push(c);
    }
    if !current_chunk.is_empty() || chunks.is_empty() {
        chunks.push(current_chunk);
    }
    chunks
}

fn render_delete_confirmation(frame: &mut Frame<'_>, app: &App, theme: &Theme) {
    let Some(target) = &app.delete_confirmation else {
        return;
    };

    let (title, message, item_name) = match target {
        DeletionTarget::Paper { title, .. } => (
            " CONFIRM DELETE PDF ",
            "Are you sure you want to permanently delete this PDF file from your disk?",
            title.as_str(),
        ),
        DeletionTarget::Collection { name, .. } => (
            " CONFIRM DELETE GROUP ",
            "Are you sure you want to permanently delete this group (subdirectory) and ALL of its contents?",
            name.as_str(),
        ),
    };

    let height = 12;
    let width = 64;
    let area = centered(width, height, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.surface)),
        area,
    );

    let lines = vec![
        Line::raw(""),
        Line::styled(message, Style::default().fg(theme.text)),
        Line::raw(""),
        Line::styled(
            format!("  \"{item_name}\""),
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::styled(
            "Press [y/Enter] to confirm, or [n/Esc/q] to cancel.",
            Style::default().fg(theme.muted),
        ),
    ];

    let block = Block::default()
        .title(Line::styled(
            title,
            Style::default()
                .fg(theme.error)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.error))
        .style(Style::default().bg(theme.surface));

    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(theme.text).bg(theme.surface))
            .block(block)
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_metadata_prompt(frame: &mut Frame<'_>, app: &mut App, theme: &Theme) {
    let Some(prompt) = &app.metadata_prompt else {
        return;
    };
    let renaming_pdf = prompt.rename_paper_id.is_some();
    let renaming = prompt.rename_collection_id.is_some();
    let creating = prompt.paper_id.is_none() && !renaming && !renaming_pdf;
    let title = if renaming_pdf {
        " RENAME PDF "
    } else if renaming {
        " RENAME GROUP "
    } else if creating {
        " CREATE GROUP "
    } else {
        " CHOOSE OR CREATE GROUP "
    };
    let prefix = if renaming || creating || renaming_pdf {
        "> "
    } else {
        "New name: "
    };
    let text = format!("{}{}", prefix, prompt.value);
    let chunks = chunk_string(&text, 62);
    let text_lines = if chunks.is_empty() {
        1
    } else {
        saturating_u16(chunks.len())
    };

    let has_current =
        prompt.current_collection.is_some() && !renaming && !creating && !renaming_pdf;
    let extra_rows = if has_current { 2 } else { 0 };

    let height = if renaming || creating || renaming_pdf {
        text_lines.max(1) + 2
    } else {
        text_lines.max(1) + 10 + extra_rows
    };

    let area = centered(64, height, frame.area());
    frame.render_widget(Clear, area);

    let mut lines = Vec::new();
    if let Some(current_collection) = prompt.current_collection.as_ref().filter(|_| has_current) {
        lines.push(Line::styled(
            format!("Current group: {current_collection}"),
            Style::default().fg(theme.muted),
        ));
        lines.push(Line::raw(""));
    }

    for chunk in &chunks {
        lines.push(Line::raw(chunk.clone()));
    }
    if chunks.is_empty() {
        lines.push(Line::raw(prefix));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(theme.text).bg(theme.surface))
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.secondary)),
            ),
        area,
    );

    let prefix_len = prefix.chars().count();
    let cursor_idx = prefix_len + prompt.value[..prompt.cursor].chars().count();
    let cursor_row = u16::try_from(cursor_idx / 62).unwrap_or(0) + extra_rows;
    let cursor_col = u16::try_from(cursor_idx % 62).unwrap_or(0);

    frame.set_cursor_position((
        area.x.saturating_add(1).saturating_add(cursor_col),
        area.y.saturating_add(1).saturating_add(cursor_row),
    ));
}

fn render_library(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    if app.library_view == LibraryView::Hierarchy {
        render_library_hierarchy(frame, area, app, theme);
        return;
    }
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(4)]).split(area);
    let papers = app.filtered_library_papers();
    let message = app
        .library
        .message
        .as_deref()
        .unwrap_or("Local papers from configured library folders. Press r to rescan.");
    frame.render_widget(
        Paragraph::new(message).style(Style::default().fg(if app.library.indexing {
            theme.warning
        } else {
            theme.muted
        })),
        rows[0],
    );
    if papers.is_empty() {
        frame.render_widget(
            Paragraph::new("No local papers indexed")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.muted)),
            rows[1],
        );
        return;
    }
    let available_width = rows[1].width.saturating_sub(2) as usize;
    let items = papers.iter().map(|paper| {
        // R4: `year/month  cite  journal  title` 形式（Library 共通）
        let year_month = published_year_month(paper.published_at.as_deref());
        let mut lines = paper_list_columns_line(
            &year_month,
            paper.citation_count,
            paper.journal.as_deref(),
            display_paper_title(if paper.title.trim().is_empty() {
                paper.display_name()
            } else {
                &paper.title
            }),
            theme,
            available_width,
            None,
        );
        // 撤回された論文は、タイトルの前に `[撤回] ` を `theme.error` で付ける。
        if paper.is_retracted == Some(true)
            && let Some(line) = lines.first_mut()
            && let Some(title) = line.spans.pop()
        {
            line.spans.push(Span::styled(
                "[撤回] ",
                Style::default()
                    .fg(theme.error)
                    .add_modifier(Modifier::BOLD),
            ));
            line.spans.push(title);
        }
        workspace_list_item(lines)
    });
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" {} PAPERS ", papers.len()))
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.border)),
        )
        .highlight_style(Style::default().bg(theme.surface).fg(theme.accent))
        .highlight_symbol("> ");
    let mut state = ListState::default()
        .with_selected(workspace_highlight_selection(app, app.library.selected))
        .with_offset(app.library.scroll);
    frame.render_stateful_widget(list, rows[1], &mut state);
    app.library.scroll = state.offset();
}

/// Library の階層表示（検索履歴 + ダウンロード済み論文 + Import 行）。
fn render_library_hierarchy(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(4)]).split(area);
    frame.render_widget(
        Paragraph::new("HIERARCHY VIEW — shift+K: flat list  space: fold  Enter: open")
            .style(Style::default().fg(theme.muted)),
        rows[0],
    );
    if app.library_tree.is_empty() {
        frame.render_widget(
            Paragraph::new("No search history yet. Run a Discover search to populate this view.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.muted)),
            rows[1],
        );
        return;
    }
    let available_width = rows[1].width.saturating_sub(2) as usize;
    let items: Vec<ListItem<'_>> = app
        .library_tree
        .iter()
        .map(|row| match row.kind {
            LibraryTreeKind::Paper => {
                let mut lines = paper_list_columns_line(
                    &row.year_month,
                    row.citation_count,
                    row.journal.as_deref(),
                    display_paper_title(&row.title),
                    theme,
                    available_width,
                    None,
                );
                if let Some(first) = lines.first_mut() {
                    first.spans.insert(0, Span::raw("  "));
                }
                workspace_list_item(lines)
            }
            LibraryTreeKind::Query | LibraryTreeKind::Import => {
                let marker = if row
                    .fold_key
                    .as_ref()
                    .is_some_and(|key| app.library_tree_folded.contains(key))
                {
                    "▸ "
                } else {
                    "▼ "
                };
                let color = if row.kind == LibraryTreeKind::Import {
                    theme.secondary
                } else {
                    theme.accent
                };
                workspace_list_item(vec![Line::styled(
                    format!("{marker}{}  ({})", row.label, row.child_count),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )])
            }
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" {} QUERIES ", app.library_tree.len()))
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.border)),
        )
        .highlight_style(Style::default().bg(theme.surface).fg(theme.accent))
        .highlight_symbol("> ");
    let mut state = ListState::default()
        .with_selected(workspace_highlight_selection(
            app,
            app.library_tree_selected,
        ))
        .with_offset(app.library_tree_scroll);
    frame.render_stateful_widget(list, rows[1], &mut state);
    app.library_tree_scroll = state.offset();
}

/// QUERY 画面（過去の Discover 検索の再表示）。
fn render_query_detail(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let rows = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(4),
        Constraint::Length(2),
    ])
    .split(area);
    let detail = &app.query_detail;
    let reason_text = if detail.editing_reason {
        format!("{}▏", detail.reason)
    } else {
        if detail.reason.is_empty() {
            "(e で入力)".to_owned()
        } else {
            detail.reason.clone()
        }
    };
    let reason_style = if detail.editing_reason {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.text)
    };
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("query     ", Style::default().fg(theme.muted)),
            Span::raw(detail.query.clone()),
        ]),
        Line::from(vec![
            Span::styled("year      ", Style::default().fg(theme.muted)),
            Span::raw(detail.year.clone()),
            Span::styled("   Limit  ", Style::default().fg(theme.muted)),
            Span::raw(detail.limit_txt.clone()),
            Span::styled("   Database  ", Style::default().fg(theme.muted)),
            Span::raw(detail.source_dbs.clone()),
        ]),
        Line::from(vec![
            Span::styled("Reason    ", Style::default().fg(theme.muted)),
            Span::styled(reason_text, reason_style),
        ]),
    ])
    .block(
        Block::default()
            .title(" QUERY ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border)),
    );
    frame.render_widget(header, rows[0]);

    let available_width = rows[1].width.saturating_sub(2) as usize;
    let items: Vec<ListItem<'_>> = detail
        .items
        .iter()
        .map(|item| {
            let year = published_year_month(Some(
                item.remote
                    .published
                    .format("%Y-%m-%d")
                    .to_string()
                    .as_str(),
            ));
            // ダウンロード済みは success、未ダウンロードは muted で文字色を変える。
            let color = if item.local_id.is_some() {
                theme.success
            } else {
                theme.muted
            };
            let lines = paper_list_columns_line(
                &year,
                item.remote.citation_count.map(i64::from),
                item.remote.journal_ref.as_deref(),
                display_paper_title(&item.remote.title),
                theme,
                available_width,
                Some(color),
            );
            workspace_list_item(lines)
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(
                    " {} PAPERS (green=downloaded) ",
                    detail.items.len()
                ))
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.border)),
        )
        .highlight_style(Style::default().bg(theme.surface).fg(theme.accent))
        .highlight_symbol("> ");
    let mut state = ListState::default()
        .with_selected(workspace_highlight_selection(app, detail.selected))
        .with_offset(detail.scroll);
    frame.render_stateful_widget(list, rows[1], &mut state);
    let offset = state.offset();
    app.query_detail.scroll = offset;

    let guide = if app.query_detail.editing_reason {
        "typing Reason…  Enter: save  Esc: cancel"
    } else {
        "j/k select  Enter: open paper  e: edit Reason  Esc/h: back"
    };
    frame.render_widget(
        Paragraph::new(guide).style(Style::default().fg(theme.muted)),
        rows[2],
    );
}

fn format_bytes(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    const KIB: u64 = 1024;
    if bytes >= MIB {
        format_decimal_bytes(bytes, MIB, "MiB")
    } else if bytes >= KIB {
        format_decimal_bytes(bytes, KIB, "KiB")
    } else {
        format!("{bytes} B")
    }
}

fn format_decimal_bytes(bytes: u64, unit: u64, suffix: &str) -> String {
    let whole = bytes / unit;
    let decimal = bytes % unit * 10 / unit;
    format!("{whole}.{decimal} {suffix}")
}

fn render_discover(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let results_area = render_discover_inputs(frame, area, app, theme);
    render_discover_status(frame, results_area, app, theme);
}

/// Import 画面（Discover から検索入力を除いた構成）。
///
/// - 上段: `data/bibtex/` のパス + `r` 再スキャン案内 + `>` フィルタ
/// - 下部: Discover と共通の 1 行形式リスト（Library 済み行は緑色）
fn render_import(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(4)]).split(area);
    let show_filter = !app.import.results.is_empty();
    let inputs = Layout::horizontal(if show_filter {
        [Constraint::Percentage(55), Constraint::Percentage(45)]
    } else {
        [Constraint::Percentage(100), Constraint::Length(0)]
    })
    .split(rows[0]);

    // 左: スキャン対象ディレクトリと再スキャン案内
    let dir_label = app.data_dir.join("bibtex").display().to_string();
    frame.render_widget(
        Paragraph::new(dir_label)
            .style(Style::default().fg(theme.muted))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.border)),
            ),
        inputs[0],
    );

    // 右: `>` フィルタ（Discover と同じ入力体験）
    if show_filter {
        let filter = if app.import.filter.is_empty() {
            "Filter displayed results"
        } else {
            &app.import.filter
        };
        let filter_style = if app.mode == AppMode::DiscoverFilter {
            Style::default().fg(theme.text).bg(theme.surface)
        } else if app.import.filter.is_empty() {
            Style::default().fg(theme.muted)
        } else {
            Style::default().fg(theme.text)
        };
        frame.render_widget(
            Paragraph::new(format!("> {filter}"))
                .style(filter_style)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(
                            if app.mode == AppMode::DiscoverFilter {
                                theme.accent
                            } else {
                                theme.border
                            },
                        )),
                ),
            inputs[1],
        );
        if app.mode == AppMode::DiscoverFilter {
            let cursor_offset = u16::try_from(
                app.import.filter[..app.import.filter_cursor]
                    .chars()
                    .count(),
            )
            .unwrap_or(0);
            frame.set_cursor_position((
                inputs[1].x.saturating_add(3).saturating_add(cursor_offset),
                inputs[1].y.saturating_add(1),
            ));
        }
    }

    render_import_status(frame, rows[1], app, theme);
}

/// Import 画面のステータス別描画（スキャン中 / エラー / 空 / 一覧）。
fn render_import_status(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    match &app.import.status {
        ImportStatus::Idle | ImportStatus::Loading => {
            frame.render_widget(
                Paragraph::new("Scanning data/bibtex ...")
                    .style(Style::default().fg(theme.accent))
                    .alignment(Alignment::Center),
                area,
            );
        }
        ImportStatus::Error(error) => {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::styled("Scan failed (Import)", Style::default().fg(theme.error)),
                    Line::raw(""),
                    Line::styled(error.as_str(), Style::default().fg(theme.warning)),
                    Line::raw(""),
                    Line::styled("Press r to retry", Style::default().fg(theme.text)),
                ])
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
                area,
            );
        }
        ImportStatus::Ready if app.import.results.is_empty() => {
            // 空状態: S2 ウェブ Cite → BibTeX の手順案内
            frame.render_widget(
                Paragraph::new(vec![
                    Line::styled(
                        "Import papers from BibTeX",
                        Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                    ),
                    Line::raw(""),
                    Line::styled(
                        "1. Semantic Scholar (or publisher) → Cite → BibTeX をコピー",
                        Style::default().fg(theme.muted),
                    ),
                    Line::styled(
                        "2. data/bibtex/ に .bib ファイルとして保存",
                        Style::default().fg(theme.muted),
                    ),
                    Line::styled(
                        "3. この画面に戻ると自動スキャンされます（r で再スキャン）",
                        Style::default().fg(theme.muted),
                    ),
                ])
                .alignment(Alignment::Center),
                area,
            );
        }
        ImportStatus::Ready => {
            render_import_results(frame, area, app, theme);
        }
    }
}

/// Import 画面の一覧描画（Discover のリストと共通の 1 行形式）。
/// Library に PDF がある論文は Discover と同じ緑色で示す。
fn render_import_results(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let fallback = app.import.filter.is_empty() && app.import.filtered_indices.is_empty();
    let indices: Vec<usize> = if fallback {
        (0..app.import.results.len()).collect()
    } else {
        app.import.filtered_indices.clone()
    };
    let items = indices.iter().filter_map(|&index| {
        let paper = app.import.results.get(index)?;
        // (Downloaded) はテキスト表示ではなく行全体の色で示す（Discover と同じ）
        let has_local = app.downloaded_remote_paper(paper).is_some();
        let row_color = has_local.then_some(theme.success);
        Some(workspace_list_item(paper_list_line(
            first_author_surname(&paper.author_line()),
            &format_local_datetime(&paper.published, "%Y"),
            display_paper_title(&paper.title),
            theme,
            area.width.saturating_sub(2) as usize,
            row_color,
        )))
    });

    // パース失敗ファイルがある場合は見出しに警告を添える
    let error_note = if app.import.scan_errors.is_empty() {
        String::new()
    } else {
        format!("  |  {} skipped file(s)", app.import.scan_errors.len())
    };
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(
                    " {}ENTRIES  |  {} RESULTS  |  r: rescan  >: filter{} ",
                    "",
                    indices.len(),
                    error_note,
                ))
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.border)),
        )
        .highlight_style(Style::default().bg(theme.surface).fg(theme.accent))
        .highlight_symbol("> ");
    let mut state = ListState::default()
        .with_selected(workspace_highlight_selection(app, app.import.selected))
        .with_offset(app.import.scroll);
    frame.render_stateful_widget(list, area, &mut state);
    app.import.scroll = state.offset();
}

fn render_discover_inputs(frame: &mut Frame<'_>, area: Rect, app: &App, theme: &Theme) -> Rect {
    // 4 段構成（word 全幅 → year / limit 左右分割 → db → filter）。
    let show_filter = !app.discovery.results.is_empty();
    let mut constraints = vec![
        Constraint::Length(3), // word
        Constraint::Length(3), // year / limit
        Constraint::Length(3), // db
    ];
    if show_filter {
        constraints.push(Constraint::Length(3)); // filter
    }
    constraints.push(Constraint::Min(4));
    let rows = Layout::vertical(constraints).split(area);
    let inputs =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[1]);

    let focused = |target: DiscoverInputFocus| {
        app.mode == AppMode::Search && app.discovery.input_focus == target
    };

    let query = if app.discovery.query.is_empty() {
        "Search words | author: name | title: words | category: gr-qc"
    } else {
        &app.discovery.query
    };
    let search_style = if focused(DiscoverInputFocus::Word) {
        Style::default().fg(theme.text).bg(theme.surface)
    } else if app.discovery.query.is_empty() {
        Style::default().fg(theme.muted)
    } else {
        Style::default().fg(theme.text)
    };
    frame.render_widget(
        Paragraph::new(format!("/ {query}"))
            .style(search_style)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(if focused(DiscoverInputFocus::Word) {
                        theme.accent
                    } else {
                        theme.border
                    })),
            ),
        rows[0],
    );

    // 2 段目: year（発行年区間）と limit（取得件数）
    let year = if app.discovery.year.is_empty() {
        "Year range (e.g. 2010-2020)"
    } else {
        &app.discovery.year
    };
    let year_style = if focused(DiscoverInputFocus::Year) {
        Style::default().fg(theme.text).bg(theme.surface)
    } else if app.discovery.year.is_empty() {
        Style::default().fg(theme.muted)
    } else {
        Style::default().fg(theme.text)
    };
    frame.render_widget(
        Paragraph::new(format!("{year}")).style(year_style).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if focused(DiscoverInputFocus::Year) {
                    theme.accent
                } else {
                    theme.border
                })),
        ),
        inputs[0],
    );
    let limit = if app.discovery.limit.is_empty() {
        "Limit (default 100)"
    } else {
        &app.discovery.limit
    };
    let limit_style = if focused(DiscoverInputFocus::Limit) {
        Style::default().fg(theme.text).bg(theme.surface)
    } else if app.discovery.limit.is_empty() {
        Style::default().fg(theme.muted)
    } else {
        Style::default().fg(theme.text)
    };
    frame.render_widget(
        Paragraph::new(format!("{limit}")).style(limit_style).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if focused(DiscoverInputFocus::Limit) {
                    theme.accent
                } else {
                    theme.border
                })),
        ),
        inputs[1],
    );

    // 3 段目: db（検索対象データベース）
    let db = if app.discovery.db.is_empty() {
        "Database (s2, arxiv, pubmed, biorxiv, medrxiv, openalex)"
    } else {
        &app.discovery.db
    };
    let db_style = if focused(DiscoverInputFocus::Db) {
        Style::default().fg(theme.text).bg(theme.surface)
    } else if app.discovery.db.is_empty() {
        Style::default().fg(theme.muted)
    } else {
        Style::default().fg(theme.text)
    };
    frame.render_widget(
        Paragraph::new(format!("{db}")).style(db_style).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if focused(DiscoverInputFocus::Db) {
                    theme.accent
                } else {
                    theme.border
                })),
        ),
        rows[2],
    );

    // 4 段目: filter（既存の結果絞り込み入力）
    if show_filter {
        let filter = if app.discovery.filter.is_empty() {
            "Filter displayed results"
        } else {
            &app.discovery.filter
        };
        let filter_style = if app.mode == AppMode::DiscoverFilter {
            Style::default().fg(theme.text).bg(theme.surface)
        } else if app.discovery.filter.is_empty() {
            Style::default().fg(theme.muted)
        } else {
            Style::default().fg(theme.text)
        };
        frame.render_widget(
            Paragraph::new(format!("> {filter}"))
                .style(filter_style)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(
                            if app.mode == AppMode::DiscoverFilter {
                                theme.accent
                            } else {
                                theme.border
                            },
                        )),
                ),
            rows[3],
        );
    }

    if focused(DiscoverInputFocus::Word) {
        let cursor_offset = u16::try_from(
            app.discovery.query[..app.discovery.query_cursor]
                .chars()
                .count(),
        )
        .unwrap_or(0);
        frame.set_cursor_position((
            rows[0].x.saturating_add(3).saturating_add(cursor_offset),
            rows[0].y.saturating_add(1),
        ));
    } else if focused(DiscoverInputFocus::Year) {
        let cursor_offset = u16::try_from(
            app.discovery.year[..app.discovery.year_cursor]
                .chars()
                .count(),
        )
        .unwrap_or(0);
        frame.set_cursor_position((
            inputs[0].x.saturating_add(1).saturating_add(cursor_offset),
            inputs[0].y.saturating_add(1),
        ));
    } else if focused(DiscoverInputFocus::Limit) {
        let cursor_offset = u16::try_from(
            app.discovery.limit[..app.discovery.limit_cursor]
                .chars()
                .count(),
        )
        .unwrap_or(0);
        frame.set_cursor_position((
            inputs[1].x.saturating_add(1).saturating_add(cursor_offset),
            inputs[1].y.saturating_add(1),
        ));
    } else if focused(DiscoverInputFocus::Db) {
        let cursor_offset =
            u16::try_from(app.discovery.db[..app.discovery.db_cursor].chars().count()).unwrap_or(0);
        frame.set_cursor_position((
            rows[2].x.saturating_add(1).saturating_add(cursor_offset),
            rows[2].y.saturating_add(1),
        ));
    }
    if show_filter && app.mode == AppMode::DiscoverFilter {
        let cursor_offset = u16::try_from(
            app.discovery.filter[..app.discovery.filter_cursor]
                .chars()
                .count(),
        )
        .unwrap_or(0);
        frame.set_cursor_position((
            rows[3].x.saturating_add(3).saturating_add(cursor_offset),
            rows[3].y.saturating_add(1),
        ));
    }

    // 結果一覧は filter 段の下から
    let results_row = if show_filter { rows[4] } else { rows[3] };
    Rect::new(
        results_row.x,
        results_row.y,
        results_row.width,
        results_row.height,
    )
}

/// Discover の読み込み中の見出しを作る（`Searching arxiv, pubmed...`）。
///
/// DB 名がまだ決まっていない（providers が空）ときは `Searching...`。
#[must_use]
pub fn searching_message(providers: &[String]) -> String {
    if providers.is_empty() {
        "Searching...".to_owned()
    } else {
        format!("Searching {}...", providers.join(", "))
    }
}

/// Discover の失敗の見出しを作る（`Search failed (pubmed)`）。
///
/// DB 名がまだ決まっていない（providers が空）ときは `Search failed`。
#[must_use]
pub fn search_failed_message(providers: &[String]) -> String {
    if providers.is_empty() {
        "Search failed".to_owned()
    } else {
        format!("Search failed ({})", providers.join(", "))
    }
}

fn render_discover_status(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    match &app.discovery.status {
        DiscoveryStatus::Idle => render_discover_empty(frame, area, theme),
        DiscoveryStatus::Loading if app.discovery.results.is_empty() => {
            // 実際に使った DB 名を出す。途中の進捗文があるときはそちらを優先する。
            let msg = app
                .discovery
                .progress_message
                .clone()
                .unwrap_or_else(|| searching_message(&app.discovery.active_providers));
            frame.render_widget(
                Paragraph::new(msg)
                    .style(Style::default().fg(theme.accent))
                    .alignment(Alignment::Center),
                area,
            );
        }
        DiscoveryStatus::Error(e) if app.discovery.results.is_empty() => {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::styled(
                        search_failed_message(&app.discovery.active_providers),
                        Style::default().fg(theme.error),
                    ),
                    Line::raw(""),
                    Line::styled(e.as_str(), Style::default().fg(theme.warning)),
                    Line::raw(""),
                    // キーの種類は U02 のエラー文（環境変数の名前）が出す。
                    Line::styled("Press r to retry", Style::default().fg(theme.text)),
                ])
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
                area,
            );
        }
        DiscoveryStatus::Ready if app.discovery.results.is_empty() => {
            frame.render_widget(
                Paragraph::new("No papers found. Try a broader query.")
                    .style(Style::default().fg(theme.muted))
                    .alignment(Alignment::Center),
                area,
            );
        }
        DiscoveryStatus::Loading | DiscoveryStatus::Error(_) | DiscoveryStatus::Ready => {
            render_search_results(frame, area, app, theme);
        }
    }
}

fn render_discover_empty(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "Discover research",
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
            Line::styled(
                "Press / and enter a query to begin.",
                Style::default().fg(theme.muted),
            ),
        ])
        .alignment(Alignment::Center),
        area,
    );
}

fn render_search_results(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let progress = app
        .discovery
        .progress_message
        .as_deref()
        .unwrap_or_else(|| {
            if app.discovery.status == DiscoveryStatus::Loading {
                "Loading more results..."
            } else {
                ""
            }
        });
    let progress_prefix = if progress.is_empty() {
        String::new()
    } else {
        format!("{progress}  ")
    };
    let items = app.discovery.visible_page_results().map(|paper| {
        // (Downloaded) はテキスト表示ではなく行全体の色で示す
        let has_local = app.downloaded_remote_paper(paper).is_some();
        let row_color = has_local.then_some(theme.success);
        // R2/R4: `year/mo  cite  hits  journal  title` 形式
        let year_month = format_local_datetime(&paper.published, "%Y/%m");
        let citations = paper.citation_count.map(i64::from);
        let hits = app
            .discovery
            .hit_counts
            .get(&papr_core::dedupe_key(paper))
            .copied();
        workspace_list_item(discover_list_columns_line(
            &year_month,
            citations,
            hits,
            !has_local,
            paper.journal_ref.as_deref(),
            display_paper_title(&paper.title),
            theme,
            area.width.saturating_sub(2) as usize,
            row_color,
        ))
    });
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(
                    " {}PAGE {} / {}  |  {} RESULTS  |  Ctrl+Left/Right: page ",
                    progress_prefix,
                    app.discovery.page + 1,
                    app.discovery.page_count(),
                    app.discovery.filtered_result_count(),
                ))
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.border)),
        )
        .highlight_style(Style::default().bg(theme.surface).fg(theme.accent))
        .highlight_symbol("> ");
    let mut state = ListState::default()
        .with_selected(workspace_highlight_selection(app, app.discovery.selected))
        .with_offset(app.discovery.scroll);
    frame.render_stateful_widget(list, area, &mut state);
    app.discovery.scroll = state.offset();
    app.discovery.store_page_view();
}

/// 先頭著者の姓を返す（"Ashish Vaswani, Niki Parmar" → "Vaswani"）。
/// sai-paper-tui のリスト表示と同じ方式。
fn first_author_surname(authors: &str) -> &str {
    authors
        .split(',')
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|first| first.split_whitespace().last())
        .unwrap_or("Unknown")
}

/// 出版年（published_at の先頭4桁）を返す。
fn published_year(published_at: &str) -> &str {
    published_at
        .get(0..4)
        .filter(|y| y.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or("-")
}

/// R4: published_at 文字列（RFC3339 等）から `YYYY/MM` 形式の年月を作る。
/// 月が不明なら `YYYY`、年も不明なら `-`。
pub(crate) fn published_year_month(published_at: Option<&str>) -> String {
    let Some(raw) = published_at.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return "-".to_string();
    };
    let year: String = raw.chars().take_while(|c| c.is_ascii_digit()).collect();
    if year.len() != 4 {
        return "-".to_string();
    }
    let rest = &raw[year.len()..];
    // RFC3339（`2020-03-01T...`）や `2020/03` 表記に対応
    let month: String = rest
        .strip_prefix('-')
        .or_else(|| rest.strip_prefix('/'))
        .unwrap_or("")
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if month.len() == 2 || (month.len() == 1 && !month.is_empty()) {
        format!("{year}/{month:0>2}")
    } else {
        year
    }
}

/// R4: 被引用数の可読表記（例: 80000 → `80.0k`、1200000 → `1.2m`）。不明時は `-`。
fn format_citation_count(citations: Option<i64>) -> String {
    match citations {
        None => "-".to_string(),
        Some(n) if n < 0 => "-".to_string(),
        Some(n) if n < 1000 => n.to_string(),
        Some(n) => {
            let k = n as f64 / 1000.0;
            if k < 1000.0 {
                format!("{k:.1}k")
            } else {
                format!("{:.1}m", k / 1000.0)
            }
        }
    }
}

/// R4: ジャーナル名の短縮表示。空なら `-`。
fn journal_display(journal: Option<&str>) -> String {
    journal
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("-")
        .to_string()
}

/// R4: `year/month  cite  journal  title` 形式の 1 行を作る。
///
/// Discover / Library / タグ論文一覧の共通形式。`row_color` を指定すると
/// 行全体をその色で描画する（Discover のローカル PDF 済み緑色表示など）。
fn paper_list_columns_line<'a>(
    year_month: &str,
    citations: Option<i64>,
    journal: Option<&str>,
    title: &str,
    theme: &Theme,
    available_width: usize,
    row_color: Option<ratatui::style::Color>,
) -> Vec<Line<'a>> {
    let meta_color = row_color.unwrap_or(theme.muted);
    let title_color = row_color.unwrap_or(theme.text);
    let date_col = fit_column(year_month, LIST_DATE_WIDTH);
    let cite_col = fit_column(&format_citation_count(citations), LIST_CITE_WIDTH);
    let journal_col = fit_column(&journal_display(journal), LIST_JOURNAL_WIDTH);
    let fixed = LIST_DATE_WIDTH + LIST_CITE_WIDTH + LIST_JOURNAL_WIDTH + 3;
    let title_width = available_width.saturating_sub(fixed).max(8);
    let truncated = safe_truncate(title, title_width);
    vec![Line::from(vec![
        Span::styled(date_col, Style::default().fg(meta_color)),
        Span::raw(" "),
        Span::styled(cite_col, Style::default().fg(meta_color)),
        Span::raw(" "),
        Span::styled(journal_col, Style::default().fg(meta_color)),
        Span::raw(" "),
        Span::styled(truncated, Style::default().fg(title_color)),
    ])]
}

/// R2: `year/mo  cite  hits  journal  title` 形式の 1 行を作る（Discover 専用）。
///
/// `hits` は過去の検索でのヒット回数（`None` は集計前 → `-`）。
/// `not_downloaded` が true のとき回数の後ろに未 DL 印 `*` を付ける。
#[allow(clippy::too_many_arguments)]
fn discover_list_columns_line<'a>(
    year_month: &str,
    citations: Option<i64>,
    hits: Option<i64>,
    not_downloaded: bool,
    journal: Option<&str>,
    title: &str,
    theme: &Theme,
    available_width: usize,
    row_color: Option<ratatui::style::Color>,
) -> Vec<Line<'a>> {
    let meta_color = row_color.unwrap_or(theme.muted);
    let title_color = row_color.unwrap_or(theme.text);
    let date_col = fit_column(year_month, LIST_DATE_WIDTH);
    let cite_col = fit_column(&format_citation_count(citations), LIST_CITE_WIDTH);
    let hits_text = match hits {
        None => "-".to_string(),
        Some(n) if not_downloaded => format!("{n}*"),
        Some(n) => n.to_string(),
    };
    let hits_col = fit_column(&hits_text, LIST_HITS_WIDTH);
    let journal_col = fit_column(&journal_display(journal), LIST_JOURNAL_WIDTH);
    let fixed = LIST_DATE_WIDTH + LIST_CITE_WIDTH + LIST_HITS_WIDTH + LIST_JOURNAL_WIDTH + 4;
    let title_width = available_width.saturating_sub(fixed).max(8);
    let truncated = safe_truncate(title, title_width);
    vec![Line::from(vec![
        Span::styled(date_col, Style::default().fg(meta_color)),
        Span::raw(" "),
        Span::styled(cite_col, Style::default().fg(meta_color)),
        Span::raw(" "),
        Span::styled(hits_col, Style::default().fg(meta_color)),
        Span::raw(" "),
        Span::styled(journal_col, Style::default().fg(meta_color)),
        Span::raw(" "),
        Span::styled(truncated, Style::default().fg(title_color)),
    ])]
}

/// sai-paper-tui と同じ 1 行形式: `<first author> <year> <title>`
///
/// author / year は固定幅の列に揃える（表のような見た目）。
/// `row_color` を指定すると行全体をその色で描画する
/// （Discover の (Downloaded) 表示の代替）。
fn paper_list_line<'a>(
    first_author: &str,
    year: &str,
    title: &str,
    theme: &Theme,
    available_width: usize,
    row_color: Option<ratatui::style::Color>,
) -> Vec<Line<'a>> {
    let meta_color = row_color.unwrap_or(theme.muted);
    let title_color = row_color.unwrap_or(theme.text);
    // 固定幅列（sai-paper-tui と同じ: author 12 / year 5）
    let author_col = fit_column(first_author, LIST_AUTHOR_WIDTH);
    let year_col = fit_column(year, LIST_YEAR_WIDTH);
    let title_width = available_width
        .saturating_sub(LIST_AUTHOR_WIDTH + 1 + LIST_YEAR_WIDTH + 1)
        .max(8);
    let truncated = safe_truncate(title, title_width);
    vec![Line::from(vec![
        Span::styled(author_col, Style::default().fg(meta_color)),
        Span::raw(" "),
        Span::styled(year_col, Style::default().fg(meta_color)),
        Span::raw(" "),
        Span::styled(truncated, Style::default().fg(title_color)),
    ])]
}

/// 固定幅列に文字列を収める（はみ出しは切り詰め、短ければ空白でパディング）。
fn fit_column(value: &str, width: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() >= width {
        chars[..width].iter().collect()
    } else {
        let mut out: String = chars.iter().collect();
        out.push_str(&" ".repeat(width - chars.len()));
        out
    }
}

/// docling Markdown の保存先パス（data/extracts/<正規化キー>.md）。
fn docling_md_path(paper_id: &str, data_dir: &std::path::Path) -> String {
    let key = papr_core::citation::normalize_paper_id(paper_id);
    data_dir
        .join("extracts")
        .join(format!(
            "{}.md",
            papr_core::citation::docling::sanitize_filename_component(&key)
        ))
        .display()
        .to_string()
}

/// プロジェクトルートからの相対パスで表示する（`papers/xxx`、`data/xxx`）。
fn shorten_path(path: &str, data_dir: &std::path::Path) -> String {
    let p = std::path::Path::new(path);
    // プロジェクトルート（data_dir の親）を省く
    if let Some(root) = data_dir.parent()
        && let Ok(rel) = p.strip_prefix(root)
    {
        return rel.display().to_string();
    }
    // ルート解決不可の場合は data/ からの相対にフォールバック
    if let Ok(rel) = p.strip_prefix(data_dir) {
        return format!("data/{}", rel.display());
    }
    path.to_string()
}

/// PDFMathTranslate (JP) の出力パス慣例: `<stem>-ja.pdf`（元 PDF と同一ディレクトリ）。
fn pdf_ja_path(pdf_path: Option<&str>) -> String {
    match pdf_path {
        Some(p) if !p.trim().is_empty() => {
            format!("{}-ja.pdf", p.trim_end_matches(".pdf"))
        }
        _ => "Not available".to_string(),
    }
}

/// RESOURCES セクションの PDF(ja) 行テキスト。
/// 翻訳 PDF が実在する場合のみパスを表示し、未翻訳ならガイドのみ表示する。
fn pdf_ja_resource_line(pdf_path: Option<&str>, data_dir: &std::path::Path) -> String {
    let ja = pdf_path
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|p| pdf_ja_path(Some(p)));
    match ja {
        Some(ja) if std::path::Path::new(&ja).exists() => {
            format!("PDF(ja)  {}  ✓", shorten_path(&ja, data_dir))
        }
        // 未翻訳（実態なし）: パスは表示しない
        _ => "PDF(ja)  (T キーで翻訳)".to_string(),
    }
}

/// 論文を掲載している電子ジャーナルのウェブサイト URL。
/// DOI → arXiv abs → Semantic Scholar の順で解決する。
fn paper_site_url(doi: Option<&str>, arxiv_id: Option<&str>, remote_id: &str) -> String {
    if let Some(d) = doi.map(str::trim).filter(|d| !d.is_empty()) {
        return format!("https://doi.org/{d}");
    }
    if let Some(a) = arxiv_id.map(str::trim).filter(|a| !a.is_empty()) {
        return format!("https://arxiv.org/abs/{a}");
    }
    if let Some(rest) = remote_id.strip_prefix("ArXiv:") {
        return format!("https://arxiv.org/abs/{rest}");
    }
    if remote_id.len() == 40 && remote_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return format!("https://semanticscholar.org/paper/{remote_id}");
    }
    "Not available".to_string()
}

/// 図画像 1 件分の表示行数（見出し・キャプション・言及を除いた画像本体の高さ）。
const FIGURE_IMAGE_ROWS: u16 = 14;

/// `Line<'_>` を所有権付きに変換する（描画中に `&mut App` を使うため）。
fn own_lines(lines: Vec<Line<'_>>) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| {
            let spans: Vec<Span<'static>> = line
                .spans
                .into_iter()
                .map(|span| Span::styled(span.content.into_owned(), span.style))
                .collect();
            let mut owned = Line::from(spans);
            owned.style = line.style;
            owned.alignment = line.alignment;
            owned
        })
        .collect()
}

/// `text` を `width` 幅で折り返したときの行数。
fn wrapped_rows(text: &str, width: u16) -> usize {
    Paragraph::new(text.to_string())
        .wrap(Wrap { trim: true })
        .line_count(width)
}

/// 絶対行 `abs_row` から `rows` 行分のうち、ビューポートに見えている部分の
/// 描画矩形と「上にはみ出して隠れた行数」を返す。完全に画面外なら `None`。
///
/// 「全体が収まるときだけ描く」方式にすると、図ブロック（見出し + 画像 14 行 +
/// キャプション + 言及）がビューポートより高いため、画像が写るのは
/// `ビューポート行数 - 14 + 1` 行分のスクロール位置だけになり、
/// 残りは 14 行の空白になってしまう。見えている分だけ描いて常に表示する。
/// 画像は `Resize::Fit` が縦横比を保つので、矩形が低くなっても潰れない。
fn visible_rect(abs_row: usize, rows: usize, scroll: usize, area: Rect) -> Option<(Rect, u16)> {
    if rows == 0 || area.height == 0 {
        return None;
    }
    let end = abs_row + rows;
    let view_end = scroll + area.height as usize;
    if end <= scroll || abs_row >= view_end {
        return None;
    }
    let top = abs_row.saturating_sub(scroll);
    let height = end.min(view_end) - abs_row.max(scroll);
    Some((
        Rect {
            x: area.x,
            y: area.y + saturating_u16(top),
            width: area.width,
            height: saturating_u16(height),
        },
        saturating_u16(scroll.saturating_sub(abs_row)),
    ))
}

/// 図 1 つのキャプションと本文中の言及の折り返し行数を返す。
fn figure_text_rows(figure: &papr_core::figures::FigureRow, width: u16) -> (usize, usize) {
    let caption_rows = figure
        .caption
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .map_or(0, |caption| wrapped_rows(caption, width));
    let mention_rows = figure
        .mention
        .as_deref()
        .filter(|m| !m.trim().is_empty())
        .map_or(0, |mention| wrapped_rows(&format!("本文: \"{mention}\""), width));
    (caption_rows, mention_rows)
}

/// PAPER 画面（Library）の本文を、FIGURES セクションの画像込みで描画する。
///
/// テキスト部は 1 つの `Paragraph` として折り返し行数を数え、その続きに図ブロックを
/// 縦に並べる。スクロール位置はテキスト部と図ブロックで共通の絶対行で扱う。
fn render_library_paper_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut App,
    theme: &Theme,
    mut text_lines: Vec<Line<'static>>,
) {
    // FIGURES 見出し / 「(図は未抽出)」は画像を含まないのでテキスト部に入れる。
    if !app.paper_figures.is_empty() {
        text_lines.push(Line::raw(""));
        text_lines.push(Line::styled(
            "FIGURES",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ));
    } else if app.paper_figures_not_extracted {
        text_lines.push(Line::raw(""));
        text_lines.push(Line::styled(
            "(図は未抽出)",
            Style::default().fg(theme.muted),
        ));
    }

    let width = area.width;
    let text_rows = Paragraph::new(text_lines.clone())
        .wrap(Wrap { trim: true })
        .line_count(width);
    let image_rows = FIGURE_IMAGE_ROWS.min(area.height.saturating_sub(4)).max(4) as usize;
    // 引用の `Fig.3` から開いたときは FIGURES の見出し（テキスト部の最後の行）へ移す（V06）。
    if app.paper_detail_jump_to_figures && !app.paper_figures.is_empty() {
        app.paper_detail_jump_to_figures = false;
        let figure_rows: usize = app
            .paper_figures
            .iter()
            .map(|figure| {
                let (caption_rows, mention_rows) = figure_text_rows(figure, width);
                1 + image_rows + caption_rows + mention_rows
            })
            .sum();
        let max = text_rows
            .saturating_add(figure_rows)
            .saturating_sub(area.height as usize);
        app.paper_detail_scroll = saturating_u16(text_rows.saturating_sub(1).min(max));
    }
    let scroll = app.paper_detail_scroll as usize;

    // 1) テキスト部（見えている範囲だけ）
    if scroll < text_rows {
        let visible = (text_rows - scroll).min(area.height as usize);
        let text_area = Rect {
            height: saturating_u16(visible),
            ..area
        };
        frame.render_widget(
            Paragraph::new(text_lines)
                .wrap(Wrap { trim: true })
                .scroll((saturating_u16(scroll), 0)),
            text_area,
        );
    }

    // 2) 図ブロック（見出し → 画像 → キャプション → 本文中の言及）
    let mut cursor = text_rows;
    let mut total_rows = text_rows;
    for figure in &app.paper_figures {
        let label = figure
            .label
            .clone()
            .unwrap_or_else(|| format!("Figure {}", figure.fig_no));
        let header = match figure.title.as_deref().filter(|t| !t.trim().is_empty()) {
            Some(title) => format!("{label}. {title}"),
            None => format!("{label}."),
        };
        let (caption_rows, mention_rows) = figure_text_rows(figure, width);
        let mention_text = figure
            .mention
            .as_deref()
            .filter(|m| !m.trim().is_empty())
            .map(|mention| format!("本文: \"{mention}\""));

        let header_row = cursor;
        let image_row = header_row + 1;
        let caption_row = image_row + image_rows;
        let mention_row = caption_row + caption_rows;
        cursor = mention_row + mention_rows;
        total_rows = cursor;

        // 見出し
        if let Some((rect, _)) = visible_rect(header_row, 1, scroll, area) {
            frame.render_widget(
                Paragraph::new(Line::styled(
                    header,
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                )),
                rect,
            );
        }
        // 画像（はみ出した分は縮めて描く。スクロール 1 行ごとに再エンコードが走る）
        if let Some((rect, _)) = visible_rect(image_row, image_rows, scroll, area) {
            crate::figure_view::render_figure_image(frame, rect, &figure.image_path);
        }
        // キャプション（上にはみ出した行数を Paragraph のスクロールで送る）
        if caption_rows > 0
            && let Some((rect, hidden)) = visible_rect(caption_row, caption_rows, scroll, area)
            && let Some(caption) = figure.caption.as_deref()
        {
            frame.render_widget(
                Paragraph::new(caption.to_string())
                    .wrap(Wrap { trim: true })
                    .scroll((hidden, 0))
                    .style(Style::default().fg(theme.secondary)),
                rect,
            );
        }
        // 本文中の言及
        if mention_rows > 0
            && let Some((rect, hidden)) = visible_rect(mention_row, mention_rows, scroll, area)
            && let Some(mention) = mention_text.as_deref()
        {
            frame.render_widget(
                Paragraph::new(mention.to_string())
                    .wrap(Wrap { trim: true })
                    .scroll((hidden, 0))
                    .style(Style::default().fg(theme.accent)),
                rect,
            );
        }
    }

    // スクロール上限の計算用に内容行数と表示行数を保持する。
    app.paper_detail_content_rows = total_rows;
    app.paper_detail_view_rows = area.height;
}

fn render_paper_detail(frame: &mut Frame<'_>, app: &mut App, theme: &Theme) {
    // Library PAPER: show local paper details
    // Enter で開いた論文 ID を優先し、選択位置の変化に影響されない。
    // QUERY 画面から未ダウンロード論文を開いた場合は remote 詳細を表示する。
    if app.is_local_paper_detail() {
        let area = frame.area();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default().style(Style::default().bg(theme.background)),
            area,
        );
        let outer = area.inner(ratatui::layout::Margin {
            horizontal: 3,
            vertical: 1,
        });
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(outer);
        frame.render_widget(
            Paragraph::new("PAPER")
                .style(
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )
                .block(
                    Block::default()
                        .borders(Borders::BOTTOM)
                        .border_style(Style::default().fg(theme.border)),
                ),
            rows[0],
        );
        // 描画中に `&mut App`（図画像のキャッシュ更新）を使うため、
        // テキスト行は所有権付きに変換して借用を切る。
        let text_lines = match app.detail_library_paper() {
            Some(paper) => own_lines(library_paper_detail_lines(
                paper,
                theme,
                &app.data_dir,
                &app.paper_reports,
                app.paper_report_total,
                crate::current_paper_db_id(app),
            )),
            None => return,
        };
        render_library_paper_detail(frame, rows[1], app, theme, text_lines);
        frame.render_widget(
            Paragraph::new(
                "j/k/PgUp/PgDn scroll  h back  p PDF  c cite  s citation  m extract  space n/m md-view  a/A 概要訳  T PDF訳  i import",
            )
            .style(Style::default().fg(theme.muted)),
            rows[2],
        );
        if let Some(toast) = &app.toast {
            frame.render_widget(
                Paragraph::new(toast.as_str())
                    .alignment(Alignment::Right)
                    .style(Style::default().fg(theme.success)),
                rows[2],
            );
        }
        return;
    }
    // PAPER 詳細で保持している論文を優先して表示する（Discover / Dashboard / Import）。
    // 保持が無い場合は従来どおり一覧の選択中論文へフォールバックする。
    let paper = match app.paper_detail_remote.as_ref() {
        Some(paper) => Some(paper),
        None => match app.page {
            Page::Dashboard => app.today_papers.get(app.today_selected),
            Page::Import => app.import.selected_paper(),
            _ => app.discovery.selected_paper(),
        },
    };
    let Some(paper) = paper else {
        return;
    };
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.background)),
        area,
    );
    let outer = area.inner(ratatui::layout::Margin {
        horizontal: 3,
        vertical: 1,
    });
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .split(outer);
    frame.render_widget(
        Paragraph::new("PAPER")
            .style(
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(theme.border)),
            ),
        rows[0],
    );
    let is_downloaded = app.downloaded_remote_paper(paper).is_some();
    let local_pdf = app
        .downloaded_remote_paper(paper)
        .and_then(|local| local.pdf_path.clone());
    // 概要訳は OpenPaper 時に DB から取得した値（翻訳完了時は RefreshLibrary で更新）。
    // 詳細保持が無い一覧フォールバック表示では、別論文の訳が誤表示されないよう None を渡す。
    let detail_ja = app
        .paper_detail_remote
        .is_some()
        .then_some(app.paper_detail_remote_ja.as_deref())
        .flatten();
    let paragraph = Paragraph::new(paper_detail_lines(
        paper,
        theme,
        is_downloaded,
        &app.data_dir,
        local_pdf.as_deref(),
        detail_ja,
        &app.paper_reports,
        app.paper_report_total,
        crate::current_paper_db_id(app),
    ))
    .wrap(Wrap { trim: true })
    .scroll((app.paper_detail_scroll, 0));
    // スクロール上限の計算用に内容行数と表示行数を保持する（Library 側と同じ）。
    app.paper_detail_content_rows = paragraph.line_count(rows[1].width);
    app.paper_detail_view_rows = rows[1].height;
    frame.render_widget(paragraph, rows[1]);
    frame.render_widget(
        Paragraph::new(if is_downloaded {
            "j/k/PgUp/PgDn scroll  h back  p PDF  Enter PDF  d download  c cite  s citation  m extract  space n/m md-view  a/A 概要訳  T PDF訳"
        } else {
            "j/k/PgUp/PgDn scroll  h back  d download  c cite  s citation  m extract  space n/m md-view  i import  a/A 概要訳"
        })
        .style(Style::default().fg(theme.muted)),
        rows[2],
    );
    if let Some(toast) = &app.toast {
        frame.render_widget(
            Paragraph::new(toast.as_str())
                .alignment(Alignment::Right)
                .style(Style::default().fg(theme.success)),
            rows[2],
        );
    }
}

fn display_paper_title(title: &str) -> &str {
    title.strip_suffix(".pdf").unwrap_or(title)
}

/// PAPER 画面の REPORTS 欄の本文行（見出しは含まない）を作る。
///
/// `reports` は最大 5 本。`total` はその論文を引用した report の総数。
/// `total` が `reports` より多ければ、最後に「…ほか N 本」の案内を足す。
/// `paper_id` はその案内に出す CLI の引数（`--paper <番号>`）に使う。
pub(crate) fn report_rows(
    reports: &[(papr_core::Report, papr_core::IssueCounts)],
    total: usize,
    paper_id: Option<i64>,
) -> Vec<String> {
    if total == 0 {
        return vec!["（まだありません）".to_string()];
    }
    let mut rows: Vec<String> = Vec::new();
    for (index, (report, counts)) in reports.iter().enumerate() {
        let block = report.block.as_deref().unwrap_or("-");
        let date = report_date(&report.created_at);
        let issues = if counts.errors > 0 || counts.warnings > 0 {
            format!("要確認 {}・注意 {}", counts.errors, counts.warnings)
        } else {
            "問題なし".to_string()
        };
        rows.push(format!(
            " {}  {}  {}  {}  {}",
            index + 1,
            report.id,
            block,
            date,
            issues
        ));
    }
    if total > reports.len() {
        let rest = total - reports.len();
        rows.push(match paper_id {
            Some(id) => format!("…ほか {rest} 本（sai-paper report list --paper {id}）"),
            None => format!("…ほか {rest} 本"),
        });
    }
    rows
}

/// `created_at`（ISO8601）から `MM/DD` を取り出す。
/// 解釈できないときは先頭 10 文字をそのまま使う。
fn report_date(created_at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|at| at.format("%m/%d").to_string())
        .unwrap_or_else(|_| created_at.chars().take(10).collect())
}

/// REPORTS 欄の見出しと本文を `lines` の末尾に足す。
fn push_report_lines<'a>(
    lines: &mut Vec<Line<'a>>,
    reports: &[(papr_core::Report, papr_core::IssueCounts)],
    total: usize,
    paper_id: Option<i64>,
    theme: &Theme,
) {
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "REPORTS（この論文を引用した report）",
        Style::default().fg(theme.muted).add_modifier(Modifier::BOLD),
    ));
    for row in report_rows(reports, total, paper_id) {
        lines.push(Line::styled(row, Style::default().fg(theme.text)));
    }
}

// REPORTS 欄の表示に必要な値を引数で受け取る（描画のための引数が多く、
// 同じ理由で `assemble` にも allow を付けている）。
#[allow(clippy::too_many_arguments)]
fn paper_detail_lines<'a>(
    paper: &'a RemotePaper,
    theme: &'a Theme,
    is_downloaded: bool,
    data_dir: &std::path::Path,
    local_pdf: Option<&str>,
    // DB に保存済みの概要（日本語訳）。翻訳済みなら本文を表示する。
    abstract_ja: Option<&'a str>,
    // REPORTS 欄に出す report（最大 5 本）と、その総数・論文番号。
    reports: &[(papr_core::Report, papr_core::IssueCounts)],
    report_total: usize,
    paper_id: Option<i64>,
) -> Vec<Line<'a>> {
    let doi = paper.doi.as_deref().unwrap_or("Not available");
    let journal = paper.journal_ref.as_deref().unwrap_or("Not available");
    let pdf = paper.pdf_url.as_deref().unwrap_or("Not available");
    let mut title_spans = vec![Span::styled(
        display_paper_title(&paper.title),
        Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
    )];
    if is_downloaded {
        title_spans.push(Span::raw(" "));
        title_spans.push(Span::styled(
            " (Downloaded) ",
            Style::default()
                .fg(theme.success)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ));
    }
    // sai-paper-tui の PAPER 形式に合わせたメタデータ表示
    // （Year / Venue / DOI / URL / PDF → ABSTRACT → RESOURCES）
    let authors = if paper.authors.is_empty() {
        "Not available".to_string()
    } else {
        paper.author_line()
    };
    let year = format_local_datetime(&paper.published, "%Y");
    let mut lines = vec![
        Line::from(title_spans),
        Line::raw(""),
        Line::styled(authors, Style::default().fg(theme.secondary)),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Year       ", Style::default().fg(theme.muted)),
            Span::styled(year, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled("Venue      ", Style::default().fg(theme.muted)),
            Span::styled(journal, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled("DOI        ", Style::default().fg(theme.muted)),
            Span::styled(doi, Style::default().fg(theme.accent)),
        ]),
        Line::from(vec![
            Span::styled("URL        ", Style::default().fg(theme.muted)),
            Span::styled(
                paper_site_url(paper.doi.as_deref(), None, &paper.id),
                Style::default().fg(theme.accent),
            ),
        ]),
        Line::from(vec![
            Span::styled("PDF        ", Style::default().fg(theme.muted)),
            Span::styled(pdf, Style::default().fg(theme.accent)),
        ]),
        Line::raw(""),
        Line::styled(
            "ABSTRACT",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        // DB 保存済みの古い abstract に JATS が残っていても表示時に整形する。
        Line::styled(
            papr_core::sanitize_abstract(&paper.abstract_text),
            Style::default().fg(theme.text),
        ),
        Line::raw(""),
        // 概要訳は DB（papers.abstract_ja）にキャッシュ。Remote 詳細でも
        // `a` / `A` キーでその場で翻訳できる（DB に保存され、以降は常に表示）。
        // 古い訳文に残った改行・JATS は表示時に平坦化する（改行を全てなくす）。
        Line::styled(
            "概要",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        {
            let ja = abstract_ja.map(str::trim).filter(|s| !s.is_empty());
            match ja {
                Some(ja) => Line::styled(
                    papr_core::flatten_abstract_ja(ja),
                    Style::default().fg(theme.text),
                ),
                None => Line::styled(
                    "(未翻訳 — 'a' キーで翻訳 / 'A' で再翻訳)",
                    Style::default().fg(theme.muted),
                ),
            }
        },
        Line::raw(""),
        Line::styled(
            "RESOURCES",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(format!("PDF    {pdf}"), Style::default().fg(theme.accent)),
        Line::styled(
            format!(
                "docling  {}",
                shorten_path(&docling_md_path(&paper.id, data_dir), data_dir)
            ),
            Style::default().fg(theme.accent),
        ),
        Line::styled(
            pdf_ja_resource_line(local_pdf, data_dir),
            Style::default().fg(theme.accent),
        ),
    ];
    push_report_lines(&mut lines, reports, report_total, paper_id, theme);
    lines
}

fn library_paper_detail_lines<'a>(
    paper: &'a papr_core::models::LibraryPaper,
    theme: &Theme,
    data_dir: &std::path::Path,
    // REPORTS 欄に出す report（最大 5 本）と、その総数・論文番号。
    reports: &[(papr_core::Report, papr_core::IssueCounts)],
    report_total: usize,
    paper_id: Option<i64>,
) -> Vec<Line<'a>> {
    let doi = paper.doi.as_deref().unwrap_or("Not available");
    let journal = paper.journal.as_deref().unwrap_or("Not available");
    let pdf = paper.pdf_path.as_deref().unwrap_or("Not available");
    let abstract_text = paper.abstract_text.as_deref().unwrap_or("Not available");
    let published = paper.published_at.as_deref().unwrap_or("Not available");
    let updated = paper.updated_at.as_deref().unwrap_or("Not available");
    let authors = if paper.authors.is_empty() {
        "Not available"
    } else {
        paper.authors.as_str()
    };
    let mut lines = vec![
        Line::styled(
            display_paper_title(&paper.title),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::styled(authors, Style::default().fg(theme.secondary)),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Published  ", Style::default().fg(theme.muted)),
            Span::styled(published, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled("Updated    ", Style::default().fg(theme.muted)),
            Span::styled(updated, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled("DOI        ", Style::default().fg(theme.muted)),
            Span::styled(doi, Style::default().fg(theme.accent)),
        ]),
        Line::from(vec![
            Span::styled("Journal    ", Style::default().fg(theme.muted)),
            Span::styled(journal, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled("URL        ", Style::default().fg(theme.muted)),
            Span::styled(
                paper_site_url(paper.doi.as_deref(), paper.arxiv_id.as_deref(), ""),
                Style::default().fg(theme.accent),
            ),
        ]),
        Line::raw(""),
        Line::styled(
            "ABSTRACT",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        // DB 保存済みの古い abstract に JATS が残っていても表示時に整形する。
        Line::styled(
            papr_core::sanitize_abstract(abstract_text),
            Style::default().fg(theme.text),
        ),
        Line::raw(""),
        Line::styled(
            "概要",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        {
            let ja = paper.abstract_ja.as_deref().unwrap_or("").trim();
            if ja.is_empty() {
                Line::styled(
                    "(未翻訳 — 'a' キーで翻訳 / 'A' で再翻訳)",
                    Style::default().fg(theme.muted),
                )
            } else {
                // 古い訳文に残った改行・JATS は表示時に平坦化する（改行を全てなくす）。
                Line::styled(
                    papr_core::flatten_abstract_ja(ja),
                    Style::default().fg(theme.text),
                )
            }
        },
        Line::raw(""),
        Line::styled(
            "RESOURCES",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            format!("PDF    {}", shorten_path(pdf, data_dir)),
            Style::default().fg(theme.accent),
        ),
        // 抽出キー: arxiv_id / doi から正規化（md は TUI と共通のパス規約）
        Line::styled(
            format!(
                "docling  {}",
                shorten_path(
                    &docling_md_path(
                        paper
                            .arxiv_id
                            .as_deref()
                            .map(|a| format!("ArXiv:{a}"))
                            .or_else(|| paper.doi.clone().map(|d| format!("DOI:{d}")))
                            .as_deref()
                            .unwrap_or(""),
                        data_dir
                    ),
                    data_dir
                )
            ),
            Style::default().fg(theme.accent),
        ),
        Line::styled(
            pdf_ja_resource_line(paper.pdf_path.as_deref(), data_dir),
            Style::default().fg(theme.accent),
        ),
        // Radio の音声（sai-radio が論文フォルダに書く）
        match papr_core::paths::radio_audio_path(&papr_core::paths::paper_folder(
            data_dir,
            paper.id,
            &paper.title,
        )) {
            Some(audio) => Line::styled(
                format!("Radio  {}", shorten_path(&audio.to_string_lossy(), data_dir)),
                Style::default().fg(theme.accent),
            ),
            None => Line::styled(
                "Radio  (未作成 — 'r' キーで作成)",
                Style::default().fg(theme.muted),
            ),
        },
    ];
    // 撤回された論文は、タイトルの下に印を出す（確かめた日付は分かれば添える）。
    if paper.is_retracted == Some(true) {
        let checked = paper
            .retraction_checked_at
            .as_deref()
            .map(|at| at.chars().take(10).collect::<String>())
            .filter(|date| !date.is_empty());
        let warning = match checked {
            Some(date) => format!("⚠ この論文は撤回されています（OpenAlex, {date}）"),
            None => "⚠ この論文は撤回されています（OpenAlex）".to_string(),
        };
        lines.insert(
            1,
            Line::styled(
                warning,
                Style::default()
                    .fg(theme.error)
                    .add_modifier(Modifier::BOLD),
            ),
        );
    }
    push_report_lines(&mut lines, reports, report_total, paper_id, theme);
    lines
}

fn render_content(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let block = focus_block(" WORKSPACE ", app.content_focused, theme);
    let outer = block.inner(area);
    frame.render_widget(block, area);

    let inset = Rect::new(
        outer.x + 1,
        outer.y,
        outer.width.saturating_sub(2),
        outer.height,
    );
    let mut workspace_area = inset;
    if app.mode == AppMode::QueryDetail {
        render_query_detail(frame, inset, app, theme);
        return;
    }
    if app.page.supports_workspace_search() {
        let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(4)]).split(inset);
        render_workspace_search_bar(frame, rows[0], app, theme);
        workspace_area = rows[1];
    }

    if app.page == Page::Dashboard {
        render_dashboard(frame, inset, app, theme);
    } else if app.page == Page::Project {
        render_project(frame, inset, app, theme);
    } else if app.page == Page::Discover {
        render_discover(frame, inset, app, theme);
    } else if app.page == Page::Import {
        render_import(frame, inset, app, theme);
    } else if app.page == Page::Library {
        render_library(frame, workspace_area, app, theme);
    } else if app.page == Page::Collections {
        render_tags(frame, workspace_area, app, theme);
    } else if app.page == Page::Authors {
        render_authors(frame, workspace_area, app, theme);
    } else if app.page == Page::Notes {
        render_organization(frame, workspace_area, app, theme);
    } else if app.page == Page::SearchSets {
        render_search_sets(frame, workspace_area, app, theme);
    } else if app.page == Page::Tree {
        crate::tree_view::render_tree(frame, inset, app, theme);
    }
}

fn render_dashboard(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    // 仕様 §3.2: 見出し無し。カード 5 枚（PAPER / TAG / NOTE / PDF / DB）
    // + RECENT ACTIVITY 全件スクロールリスト。
    let rows = Layout::vertical([Constraint::Length(4), Constraint::Min(6)])
        .spacing(1)
        .split(area);
    let cards = Layout::horizontal([Constraint::Ratio(1, 5); 5])
        .spacing(1)
        .split(rows[0]);
    let note_count = papr_core::paths::note_count(&app.data_dir) as u64;
    let gib = |bytes: u64| format!("{:.2} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0));
    let data = [
        ("PAPER", app.stats.papers.to_string(), theme.accent),
        ("TAG", app.tags.len().to_string(), theme.secondary),
        ("NOTE", note_count.to_string(), theme.success),
        ("PDF", gib(app.dashboard.disk_usage), theme.warning),
        ("DB", gib(app.dashboard.database_size), theme.muted),
    ];
    for (area, (label, value, color)) in cards.iter().zip(data) {
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
        frame.render_widget(card, *area);
    }
    render_recent_activity(frame, rows[1], app, theme);
}

/// Project ページ（現況・最新の引き継ぎ・最近のレビュー）。
///
/// 節を上から順に並べるだけの作りにして、後のテーマが節を足しやすいようにする。
fn render_project(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    // 一番上の節: 要対応（S02）。
    lines.push(project_attention_line(&app.project.attention, theme));
    lines.extend(project_state_lines(app, theme));
    // 現況の節の後ろに理解の 1 行、その次に出典の問題の 1 行。
    lines.push(project_understanding_line(
        &app.project.understanding,
        theme,
    ));
    if app.project.report_count > 0 {
        lines.push(project_citation_line(&app.project.citation_issues, theme));
    }
    lines.extend(project_read_next_lines(app, theme));
    lines.push(Line::raw(""));
    lines.extend(project_handoff_lines(app, theme));
    lines.push(Line::raw(""));
    lines.extend(project_review_lines(app, theme));

    let block = Block::default()
        .title(" PROJECT - M: mind.md  W: state.md  N: 次の一手  U: 理解の確認 ")
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border));
    let inner = block.inner(area);
    let max_scroll = u16::try_from(lines.len().saturating_sub(usize::from(inner.height)))
        .unwrap_or(u16::MAX);
    let scroll = app.project.scroll.min(max_scroll);
    app.project.scroll = scroll;
    frame.render_widget(
        Paragraph::new(lines).block(block).scroll((scroll, 0)),
        area,
    );
}

/// 要対応の 1 行（S02。Project ページの一番上）。
///
/// 0 件なら「要対応はありません」。1 件以上なら件数と `A` の案内を出す。
fn project_attention_line(
    items: &[papr_core::attention::AttentionItem],
    theme: &Theme,
) -> Line<'static> {
    let (text, color) = if items.is_empty() {
        ("  要対応はありません".to_string(), theme.muted)
    } else {
        (
            format!("  要対応 {} 件（A で一覧）", items.len()),
            theme.warning,
        )
    };
    Line::styled(
        text,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

/// `state.md` の現況 5 項目（未記入は薄い色で「未記入（W で書く）」）。
fn project_state_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let state = &app.project.state;
    vec![
        project_field_line("目標", state.goal.as_deref(), theme),
        project_field_line("段階", state.stage.as_deref(), theme),
        project_field_line("今の作業", state.current_task.as_deref(), theme),
        project_field_line("次の一手", state.next_action.as_deref(), theme),
        project_field_line("更新日", state.updated.as_deref(), theme),
    ]
}

/// 理解の 1 行（あいまいなトピックを最大 3 つ、まだ確かめていない件数）。
///
/// 記録が無ければ「まだ記録がありません」を出す。`ok` の記録はここには出さない
/// （理解の薄いトピックを目立たせるため）。
fn project_understanding_line(
    entries: &[papr_core::understanding::UnderstandingEntry],
    theme: &Theme,
) -> Line<'static> {
    let label_width = Line::from("理解").width();
    let pad = 10usize.saturating_sub(label_width);
    let color = if entries.is_empty() {
        theme.muted
    } else {
        theme.text
    };
    Line::from(vec![
        Span::styled(
            format!(" 理解{}", " ".repeat(pad)),
            Style::default().fg(theme.secondary),
        ),
        Span::styled(
            project_understanding_value(entries),
            Style::default().fg(color),
        ),
    ])
}

/// 理解の行の値（ラベルを除いた部分）。
fn project_understanding_value(entries: &[papr_core::understanding::UnderstandingEntry]) -> String {
    if entries.is_empty() {
        return "まだ記録がありません（「理解の確認」カードで確かめる）".to_string();
    }
    let vague: Vec<&str> = entries
        .iter()
        .filter(|entry| entry.state == "vague")
        .map(|entry| entry.topic.as_str())
        .take(3)
        .collect();
    let unknown = entries
        .iter()
        .filter(|entry| entry.state == "unknown")
        .count();
    let mut parts: Vec<String> = Vec::new();
    if !vague.is_empty() {
        parts.push(format!("あいまい: {}", vague.join(" / ")));
    }
    if unknown > 0 {
        parts.push(format!("未確認: {unknown} 件"));
    }
    if parts.is_empty() {
        return "理解しているトピックのみ".to_string();
    }
    parts.join("   ")
}

/// READ NEXT 節（読むべき論文と理由の 1 行。`Enter` でその論文を開く）。
///
/// 並びと点数は `papr_core::progress::read_next` が決める（画面では並べ替えない）。
fn project_read_next_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![project_read_next_header(theme)];
    if app.project.read_next.is_empty() {
        lines.push(Line::styled(
            "  まだありません（PDF のある論文に、引き継ぎ・未読・note なし・古さの点が付くと出ます）"
                .to_string(),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    for (index, entry) in app.project.read_next.iter().enumerate() {
        let selected = index == app.project.read_next_selected;
        lines.push(project_read_next_row(entry, selected, theme));
    }
    lines
}

/// READ NEXT 節の見出し行。
fn project_read_next_header(theme: &Theme) -> Line<'static> {
    Line::styled(
        " READ NEXT ─ 読むべき論文 ────────────────────────────".to_string(),
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )
}

/// READ NEXT の 1 行（選択中の行は強調し、先頭に `>` を付ける）。
fn project_read_next_row(
    entry: &papr_core::progress::ReadNext,
    selected: bool,
    theme: &Theme,
) -> Line<'static> {
    let marker = if selected { ">" } else { " " };
    let title = safe_truncate(&entry.title, 44);
    let title_style = if selected {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text)
    };
    Line::from(vec![
        Span::styled(format!(" {marker} "), Style::default().fg(theme.accent)),
        Span::styled(
            format!("#{:<4} ", entry.paper_id),
            Style::default().fg(theme.secondary),
        ),
        Span::styled(title, title_style),
        Span::styled(
            format!("  {}", entry.reason),
            Style::default().fg(theme.muted),
        ),
    ])
}

/// 出典の問題の 1 行（report が 1 本以上あるときだけ出す）。
fn project_citation_line(counts: &papr_core::IssueCounts, theme: &Theme) -> Line<'static> {
    let (text, color) = if counts.errors == 0 && counts.warnings == 0 {
        ("出典: 問題なし".to_string(), theme.muted)
    } else {
        (
            format!(
                "出典の要確認: {} 件・注意 {} 件（sai-paper report issues）",
                counts.errors, counts.warnings
            ),
            theme.warning,
        )
    };
    Line::from(vec![
        Span::raw(" "),
        Span::styled(text, Style::default().fg(color)),
    ])
}

/// 現況 1 項目の行。ラベルは表示幅 10 桁にそろえる。
fn project_field_line(label: &str, value: Option<&str>, theme: &Theme) -> Line<'static> {
    let label_width = Line::from(label).width();
    let pad = 10usize.saturating_sub(label_width);
    let value_span = match value {
        Some(value) if !value.is_empty() => {
            Span::styled(value.to_string(), Style::default().fg(theme.text))
        }
        _ => Span::styled(
            "未記入（W で書く）".to_string(),
            Style::default().fg(theme.muted),
        ),
    };
    Line::from(vec![
        Span::styled(
            format!(" {label}{}", " ".repeat(pad)),
            Style::default().fg(theme.secondary),
        ),
        value_span,
    ])
}

/// 最新の引き継ぎ節 1 件。
fn project_handoff_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![project_section_line("最新の引き継ぎ", theme)];
    let Some(entry) = app.project.latest_handoff.as_ref() else {
        lines.push(Line::styled(
            "  まだありません（研究用のブロックの report に『## 引き継ぎ』節があると、ここに出ます）"
                .to_string(),
            Style::default().fg(theme.muted),
        ));
        return lines;
    };
    lines.push(project_entry_header(
        entry.run_id,
        &entry.block_id,
        entry.tag.as_deref(),
        entry.modified,
        theme,
    ));
    if !entry.readable {
        lines.push(Line::styled(
            format!(
                "  形が読めない引き継ぎです（ファイル: {}）",
                project_rel_path(app, &entry.file)
            ),
            Style::default().fg(theme.error),
        ));
    } else if entry.state_update().is_some() {
        lines.push(Line::styled(
            "  ・現況の更新案あり（W で state.md を直す）".to_string(),
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ));
    }
    for line in entry.raw.lines() {
        lines.push(Line::raw(format!("  {line}")));
    }
    lines
}

/// 最近のレビュー節（最大 2 件、本文はそのまま）。
fn project_review_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![project_section_line("最近のレビュー", theme)];
    if app.project.recent_reviews.is_empty() {
        lines.push(Line::styled(
            "  まだありません".to_string(),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    for entry in &app.project.recent_reviews {
        lines.push(project_entry_header(
            entry.run_id,
            &entry.block_id,
            None,
            entry.modified,
            theme,
        ));
        for line in entry.body.lines() {
            lines.push(Line::raw(format!("  {line}")));
        }
        lines.push(Line::raw(""));
    }
    lines
}

/// `run 41 / block / 09/24 21:10` の 1 行（タグがあれば `run 41 / block / 仮説B / 09/24 21:10`）。
fn project_entry_header(
    run_id: i64,
    block_id: &str,
    tag: Option<&str>,
    modified: std::time::SystemTime,
    theme: &Theme,
) -> Line<'static> {
    let mut text = format!("run {run_id} / {block_id}");
    if let Some(tag) = tag.filter(|tag| !tag.is_empty()) {
        text.push_str(&format!(" / {tag}"));
    }
    text.push_str(&format!(" / {}", format_short_time(modified)));
    Line::from(vec![
        Span::raw("  "),
        Span::styled(text, Style::default().fg(theme.muted)),
    ])
}

/// 節の見出し行。
fn project_section_line(title: &str, theme: &Theme) -> Line<'static> {
    Line::styled(
        format!(" {title}"),
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )
}

/// 更新時刻を `09/24 21:10` の形にする（表示はローカル時刻）。
fn format_short_time(time: std::time::SystemTime) -> String {
    let datetime: chrono::DateTime<chrono::Local> = time.into();
    datetime.format("%m/%d %H:%M").to_string()
}

/// プロジェクトルートからの相対パス（区切りは `/`）。
fn project_rel_path(app: &App, path: &std::path::Path) -> String {
    let root = app.data_dir.parent().and_then(std::path::Path::parent);
    root.and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// RECENT ACTIVITY（仕様 §3.3）: activity_log 全件の date / action / description 一覧。
fn render_recent_activity(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let activity_scroll = app.activity_scroll;
    if app.dashboard.recent_activity.is_empty() {
        frame.render_widget(
            Paragraph::new("No activity yet.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.muted)),
            area,
        );
        return;
    }
    let items = app.dashboard.recent_activity.iter().map(|item| {
        Line::from(vec![
            Span::styled(
                format!("{} ", item.occurred_at.format("%Y-%m-%d %H:%M:%S")),
                Style::default().fg(theme.muted),
            ),
            {
                let (label, color) = activity_category(&item.kind, theme);
                Span::styled(format!("{label: <10} "), Style::default().fg(color))
            },
            Span::styled(activity_description(item), Style::default().fg(theme.text)),
        ])
    });
    let list = List::new(items)
        .block(
            Block::default()
                .title(" RECENT ACTIVITY - j/k SCROLL  ENTER OPEN ")
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.border)),
        )
        .highlight_style(Style::default().bg(theme.surface).fg(theme.accent))
        .highlight_symbol("> ");
    let mut state = ListState::default()
        .with_selected(workspace_highlight_selection(app, app.activity_selected))
        .with_offset(activity_scroll);
    frame.render_stateful_widget(list, area, &mut state);
    app.activity_scroll = state.offset();
}

/// RECENT ACTIVITY の種別 → 表示名と色（カテゴリごとに色分けする）。
///
/// sai-phone の Papers → Dashboard も同じ種別で色分けしている（static/papers.js の ACTIVITY_KINDS）。
fn activity_category(kind: &str, theme: &Theme) -> (&'static str, ratatui::style::Color) {
    match kind {
        "discover" => ("Discover", theme.accent),
        "download" => ("Download", theme.success),
        "import" => ("Import", theme.secondary),
        "translate" => ("Translate", theme.warning),
        "notes" => ("Note", theme.text),
        "radio" => ("Radio new", theme.error),
        "radio_play" => ("Radio play", ratatui::style::Color::LightMagenta),
        "radio_question" => ("Radio Q&A", ratatui::style::Color::LightCyan),
        "open" => ("Open", ratatui::style::Color::LightBlue),
        "agent_read" => ("Agent read", ratatui::style::Color::LightGreen),
        _ => ("Other", theme.muted),
    }
}

/// RECENT ACTIVITY の説明文（右端の列）。
///
/// `open` / `agent_read` の `label` は、論文が引ければ論文タイトル、引けなければ
/// `detail` の JSON になる（`Database::recent_activity` の `COALESCE`）。
/// 後者のときは `target` とパスのファイル名を出す（例 `report  answer.md`）。
fn activity_description(item: &papr_core::ActivityItem) -> String {
    if !matches!(item.kind.as_str(), "open" | "agent_read") {
        return item.label.clone();
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&item.label) else {
        return item.label.clone();
    };
    let target = value
        .get("target")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let file = value
        .get("path")
        .and_then(serde_json::Value::as_str)
        .and_then(|path| path.rsplit('/').next())
        .unwrap_or_default();
    match (target.is_empty(), file.is_empty()) {
        (false, false) => format!("{target}  {file}"),
        (false, true) => target.to_owned(),
        (true, false) => file.to_owned(),
        (true, true) => item.label.clone(),
    }
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let line = Line::from(vec![
        Span::styled(
            " j/k ",
            Style::default().fg(theme.background).bg(theme.accent),
        ),
        Span::styled(" navigate  ", Style::default().fg(theme.muted)),
        Span::styled(
            " enter ",
            Style::default().fg(theme.background).bg(theme.secondary),
        ),
        Span::styled(" open  ", Style::default().fg(theme.muted)),
        Span::styled(" J ", Style::default().fg(theme.background).bg(theme.accent)),
        Span::styled(" 研究日誌  ", Style::default().fg(theme.muted)),
        Span::styled(
            " ? ",
            Style::default().fg(theme.background).bg(theme.warning),
        ),
        Span::styled(" help  ", Style::default().fg(theme.muted)),
        Span::styled(" q ", Style::default().fg(theme.background).bg(theme.error)),
        Span::styled(" quit", Style::default().fg(theme.muted)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
    // 実行中の長時間処理（`s` / `T`）はトーストとは別に、完了まで表示し続ける。
    if !app.running_operations.is_empty() {
        let labels: Vec<String> = app
            .running_operations
            .iter()
            .map(|op| format!("⏳ {}", op.label))
            .collect();
        frame.render_widget(
            Paragraph::new(labels.join("  |  "))
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.warning)),
            area,
        );
        if let Some(toast) = &app.toast {
            frame.render_widget(
                Paragraph::new(toast.as_str())
                    .alignment(Alignment::Right)
                    .style(Style::default().fg(theme.success)),
                area,
            );
        }
        return;
    }
    if let Some(toast) = &app.toast {
        frame.render_widget(
            Paragraph::new(toast.as_str())
                .alignment(Alignment::Right)
                .style(Style::default().fg(theme.success)),
            area,
        );
    }
}

fn render_terminal_command(frame: &mut Frame<'_>, app: &App, theme: &Theme) {
    let area = centered(76, 20, frame.area());
    frame.render_widget(Clear, area);
    let working_directory = app.terminal_command_directory.as_ref().map_or_else(
        || "current application directory".to_owned(),
        |directory| directory.display().to_string(),
    );
    let output = if app.terminal_command_output.is_empty() {
        "Command output will appear here.".to_owned()
    } else {
        app.terminal_command_output.clone()
    };
    let mut output_lines = Vec::new();
    if !app.terminal_completions.is_empty() {
        output_lines.push(Line::styled(
            format!("Completions ({})", app.terminal_completions.len()),
            Style::default().fg(theme.muted),
        ));
        output_lines.extend(app.terminal_completions.iter().enumerate().map(
            |(index, candidate)| {
                let style = if app.terminal_completion_selected == Some(index) {
                    Style::default()
                        .bg(theme.surface)
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                };
                Line::styled(
                    format!(
                        "{} {candidate}",
                        if app.terminal_completion_selected == Some(index) {
                            "›"
                        } else {
                            " "
                        }
                    ),
                    style,
                )
            },
        ));
        output_lines.push(Line::raw(""));
    }
    output_lines.extend(output.lines().map(|line| Line::raw(line.to_owned())));
    let sections = Layout::vertical([
        Constraint::Min(4),
        Constraint::Length(2),
        Constraint::Length(3),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(output_lines)
            .style(Style::default().bg(theme.surface))
            .wrap(Wrap { trim: false })
            .block(focus_block(" TERMINAL OUTPUT ", false, theme)),
        sections[0],
    );
    frame.render_widget(
        Paragraph::new(format!("cwd: {working_directory}"))
            .style(Style::default().fg(theme.muted).bg(theme.surface)),
        sections[1],
    );
    frame.render_widget(
        Paragraph::new(app.terminal_command.as_str())
            .style(Style::default().bg(theme.surface))
            .block(focus_block(
                " TERMINAL — ENTER RUN  ESC CLOSE ",
                true,
                theme,
            )),
        sections[2],
    );
    let cursor = app.terminal_command
        [..app.terminal_command_cursor.min(app.terminal_command.len())]
        .chars()
        .count();
    let cursor = saturating_u16(cursor);
    frame.set_cursor_position((sections[2].x.saturating_add(1 + cursor), sections[2].y + 1));
}

fn render_palette(frame: &mut Frame<'_>, app: &mut App, theme: &Theme) {
    let area = centered(45, 18, frame.area());
    frame.render_widget(Clear, area);

    // Split the area into Search Bar and Results List
    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(4)]).split(area);

    // 1. Render the search input field
    let search_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(" BROWSE PAPR ");

    let query_text = &app.palette_query;
    frame.render_widget(
        Paragraph::new(if query_text.is_empty() {
            "Search workspaces, papers, groups, settings…"
        } else {
            query_text.as_str()
        })
        .style(Style::default().bg(theme.surface))
        .block(search_block),
        chunks[0],
    );

    // Render text cursor position
    let cursor_offset = app
        .palette_query
        .chars()
        .take(app.palette_query_cursor)
        .count();
    frame.set_cursor_position((
        chunks[0]
            .x
            .saturating_add(1)
            .saturating_add(u16::try_from(cursor_offset).unwrap_or(0)),
        chunks[0].y.saturating_add(1),
    ));

    // 2. Filter items and render list
    let filtered_pages = app.filtered_palette_items();
    let items = filtered_pages
        .iter()
        .map(|page| ListItem::new(Line::styled(page.title(), Style::default().fg(theme.text))));

    let list = List::new(items)
        .block(
            Block::default()
                .title(" DESTINATIONS ")
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                .style(Style::default().bg(theme.surface))
                .border_style(Style::default().fg(theme.accent)),
        )
        .highlight_style(Style::default().bg(theme.border).fg(theme.accent))
        .highlight_symbol("> ");

    // Ensure selected index is valid for filtered items
    app.palette_selected = app
        .palette_selected
        .min(filtered_pages.len().saturating_sub(1));

    let mut state = ListState::default()
        .with_selected(if filtered_pages.is_empty() {
            None
        } else {
            Some(app.palette_selected)
        })
        .with_offset(app.palette_scroll);

    frame.render_stateful_widget(list, chunks[1], &mut state);
    app.palette_scroll = state.offset();
}

/// TASKS 画面（`shift+L`）: 実行中のバックグラウンド処理を一覧する。
///
/// 進捗はステップ単位の近似（docling / pdf2zh は実進捗を報告しない）。
fn render_tasks(frame: &mut Frame<'_>, app: &App, theme: &Theme) {
    let area = centered(96, frame.area().height.saturating_sub(4), frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" TASKS — 実行中のバックグラウンド処理   ESC/q CLOSE ")
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.surface))
        .border_style(Style::default().fg(theme.secondary));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = task_lines(&app.running_operations, theme);
    lines.extend(job_lines(&app.jobs, theme));
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.text)),
        inner,
    );
}

/// TASKS 画面の下半分: `jobs` 表（CLI / phone で動いている処理と、最近終わった処理）。
fn job_lines<'a>(jobs: &[papr_core::jobs::JobRow], theme: &Theme) -> Vec<Line<'a>> {
    // この TUI で動いている処理は上に出しているので、他の窓口の実行中と、終わったものだけ。
    let rows: Vec<_> = jobs
        .iter()
        .filter(|job| job.origin != "tui" || !matches!(job.status.as_str(), "queued" | "running"))
        .collect();
    if rows.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        Line::raw(""),
        Line::styled(
            "  最近の記録（CLI / phone を含む）",
            Style::default().fg(theme.muted).add_modifier(Modifier::BOLD),
        ),
    ];
    for job in rows {
        let (mark, color) = match job.status.as_str() {
            "done" => ("完了", theme.secondary),
            "failed" => ("失敗", theme.error),
            _ => ("実行中", theme.accent),
        };
        let paper = job.paper_id.map_or_else(String::new, |id| format!("論文 {id}  "));
        let error = job.error.as_deref().map_or_else(String::new, |e| {
            format!("  — {}", e.chars().take(60).collect::<String>())
        });
        lines.push(Line::styled(
            format!(
                "  [{mark}] {paper}{}  ({})  {}{error}",
                job.kind,
                job.origin,
                job.started_at.replace('T', " ").trim_end_matches('Z')
            ),
            Style::default().fg(color),
        ));
    }
    lines
}

/// TASKS 画面の本文行を組み立てる（空のときは空状態メッセージ）。
fn task_lines<'a>(operations: &[RunningOperation], theme: &Theme) -> Vec<Line<'a>> {
    if operations.is_empty() {
        return vec![
            Line::raw(""),
            Line::styled(
                "  実行中の処理はありません",
                Style::default().fg(theme.muted),
            ),
        ];
    }
    let mut lines = Vec::new();
    for op in operations {
        lines.push(Line::styled(
            format!("  {}", op.title),
            Style::default().fg(theme.accent),
        ));
        lines.push(Line::styled(
            format!(
                "    {} {:>3}%  {}  ({}/{})  経過 {}",
                op.progress_bar(24),
                op.percent(),
                op.step_label(),
                op.step.min(op.steps.len()) + 1,
                op.steps.len(),
                op.elapsed_text()
            ),
            Style::default().fg(theme.text),
        ));
        lines.push(Line::raw(""));
    }
    lines
}

/// 要対応の一覧（S02。Project ページの `A`）。
///
/// 選んだ行を強調し、種類の印（`[review]` など）と題を出す。
/// 一覧が空なら「要対応はありません」。
fn render_attention(frame: &mut Frame<'_>, app: &App, theme: &Theme) {
    let area = centered(96, frame.area().height.saturating_sub(4), frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" 要対応 — j/k 選ぶ   Enter 開く   x 対応済み   ESC/q 閉じる ")
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.surface))
        .border_style(Style::default().fg(theme.secondary));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items = &app.project.attention;
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![Line::styled(
                "  要対応はありません",
                Style::default().fg(theme.muted),
            )])
            .style(Style::default().fg(theme.text)),
            inner,
        );
        return;
    }

    // 選んだ行が見える範囲だけ出す。
    let visible = usize::from(inner.height).max(1);
    let selected = app.attention_selected.min(items.len() - 1);
    let start = selected.saturating_sub(visible - 1).min(items.len() - 1);
    let end = (start + visible).min(items.len());

    let mut lines: Vec<Line<'_>> = Vec::new();
    for (index, item) in items.iter().enumerate().take(end).skip(start) {
        let marker = if index == selected { ">" } else { " " };
        let style = if index == selected {
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text)
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {marker} "), Style::default().fg(theme.accent)),
            Span::styled(
                format!("[{}] ", item.kind.label()),
                Style::default().fg(theme.secondary),
            ),
            Span::styled(safe_truncate(&item.title, 72), style),
        ]));
    }
    // 空の残り行を埋める（`Paragraph` は上詰めでよいので何もしない）。
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.text)),
        inner,
    );
}

fn render_help(frame: &mut Frame<'_>, app: &mut App, theme: &Theme) {
    let max_area = centered(160, frame.area().height.saturating_sub(4), frame.area());
    let sections = keyboard_reference();
    // A column needs enough room for a key and a useful description.  Reflow
    // whole sections at narrower sizes rather than squeezing the old fixed
    // three-column layout until the descriptions are unusable.
    let inner_width = max_area.width.saturating_sub(2);
    let column_count = match inner_width {
        0..=71 => 1,
        72..=104 => 2,
        _ => 3,
    };
    let groups = help_column_groups(sections, column_count, inner_width);
    let layout_area = Rect::new(0, 0, inner_width, 0);
    let columns = help_columns(layout_area, &groups);
    let rendered = groups
        .iter()
        .zip(columns.iter())
        .map(|(group, column)| format_help_column(group, column.width, theme))
        .collect::<Vec<_>>();
    let total_rows = rendered.iter().map(Vec::len).max().unwrap_or(0);
    let desired_height = u16::try_from(total_rows.saturating_add(2)).unwrap_or(u16::MAX);
    let height = desired_height.clamp(12, max_area.height);
    let area = centered(max_area.width, height, frame.area());
    frame.render_widget(Clear, area);
    let title = if area.width < 86 {
        " KEYBOARD HELP — ↑/↓ SCROLL  ?/ESC CLOSE "
    } else {
        " KEYBOARD REFERENCE — ↑/↓ SCROLL  PGUP/PGDN PAGE  HOME/END JUMP  ?/ESC/q CLOSE "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.surface))
        .border_style(Style::default().fg(theme.secondary));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let viewport_rows = inner.height as usize;
    let columns = help_columns(inner, &groups);
    app.help_scroll = app
        .help_scroll
        .min(total_rows.saturating_sub(viewport_rows));
    for (column, lines) in columns.iter().zip(rendered) {
        frame.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(app.help_scroll)
                    .take(viewport_rows)
                    .collect::<Vec<_>>(),
            )
            .style(Style::default().fg(theme.text)),
            *column,
        );
    }
}

#[derive(Clone)]
struct HelpSection {
    title: &'static str,
    scope: &'static [&'static str],
    entries: &'static [(&'static str, &'static str)],
}

fn section_group_height(group: &[HelpSection], col_width: u16) -> usize {
    if group.is_empty() {
        return 0;
    }
    let key_width = group
        .iter()
        .flat_map(|section| section.entries)
        .map(|(key, _)| key.len())
        .max()
        .unwrap_or(0);
    let prefix_width = key_width + 4;
    let desc_width = usize::from(col_width).saturating_sub(prefix_width).max(1);

    let mut height = 0;
    for (index, section) in group.iter().enumerate() {
        height += wrap_help_text(section.title, usize::from(col_width)).len();
        for scope in section.scope {
            height += wrap_help_text(scope, usize::from(col_width).saturating_sub(2)).len();
        }
        for (_key, desc) in section.entries {
            height += wrap_help_text(desc, desc_width).len();
        }
        if index + 1 < group.len() {
            height += 1;
        }
    }
    height
}

fn help_column_groups(
    sections: Vec<HelpSection>,
    columns: usize,
    width: u16,
) -> Vec<Vec<HelpSection>> {
    if columns <= 1 || sections.is_empty() {
        return vec![sections];
    }
    let columns = columns.min(sections.len());
    let column_count = saturating_u16(columns).max(1);
    let column_width =
        width.saturating_sub(column_count.saturating_sub(1).saturating_mul(2)) / column_count;
    let mut units = help_section_units(&sections);
    let global_unit = units
        .iter()
        .position(|unit| {
            unit.first()
                .is_some_and(|section| section.title == "GLOBAL & NAVIGATION")
        })
        .map(|index| units.remove(index));
    units.sort_by_key(|unit| std::cmp::Reverse(section_group_height(unit, column_width)));

    let mut groups = vec![Vec::new(); columns];
    let mut heights = vec![0usize; columns];
    if let Some(global_unit) = global_unit {
        heights[0] = section_group_height(&global_unit, column_width);
        groups[0] = global_unit;
    }
    for unit in units {
        let target = heights
            .iter()
            .enumerate()
            .min_by_key(|(_, height)| *height)
            .map_or(0, |(index, _)| index);
        if !groups[target].is_empty() {
            heights[target] += 1;
        }
        heights[target] += section_group_height(&unit, column_width);
        groups[target].extend(unit);
    }
    groups
}

fn help_section_units(sections: &[HelpSection]) -> Vec<Vec<HelpSection>> {
    let mut units = Vec::new();
    let mut index = 0;
    while index < sections.len() {
        let next_title = sections.get(index + 1).map(|section| section.title);
        let keep_together = matches!(
            (sections[index].title, next_title),
            ("DOWNLOADS", Some("PROJECT LIST"))
        );
        if keep_together {
            units.push(vec![sections[index].clone(), sections[index + 1].clone()]);
            index += 2;
        } else {
            units.push(vec![sections[index].clone()]);
            index += 1;
        }
    }
    units
}

fn help_columns_widths(area_width: u16, groups: &[Vec<HelpSection>]) -> Vec<u16> {
    let count = groups.len();
    if count == 0 {
        return vec![];
    }
    if count == 1 {
        return vec![area_width];
    }

    let gap = 2u16;
    let count_u16 = saturating_u16(count).max(1);
    let total_gaps = count_u16.saturating_sub(1).saturating_mul(gap);
    let avail_width = area_width.saturating_sub(total_gaps);

    let mut prefix_widths = Vec::with_capacity(count);
    let mut ideal_desc_widths = Vec::with_capacity(count);
    let mut min_widths = Vec::with_capacity(count);
    let mut ideal_widths = Vec::with_capacity(count);

    for g in groups {
        let key_width = g
            .iter()
            .flat_map(|section| section.entries)
            .map(|(key, _)| key.len())
            .max()
            .unwrap_or(0);
        let prefix = saturating_u16(key_width.saturating_add(4));
        let max_desc = g
            .iter()
            .flat_map(|section| section.entries)
            .map(|(_, desc)| desc.len())
            .max()
            .map_or(16, saturating_u16);

        let ideal_desc = max_desc;
        let min_w = prefix + 10;
        let ideal_w = prefix + ideal_desc;

        prefix_widths.push(prefix);
        ideal_desc_widths.push(ideal_desc);
        min_widths.push(min_w);
        ideal_widths.push(ideal_w);
    }

    let min_sum: u16 = min_widths.iter().sum();
    let ideal_sum: u16 = ideal_widths.iter().sum();

    let mut widths = vec![0u16; count];

    if avail_width <= min_sum {
        let total_min = u32::from(min_sum.max(1));
        let mut allocated = 0u16;
        for i in 0..count {
            let w = u16::try_from((u32::from(min_widths[i]) * u32::from(avail_width)) / total_min)
                .unwrap_or(u16::MAX);
            widths[i] = w;
            allocated += w;
        }
        let mut rem = avail_width.saturating_sub(allocated);
        let mut i = 0;
        while rem > 0 && count > 0 {
            widths[i % count] += 1;
            rem -= 1;
            i += 1;
        }
    } else if avail_width <= ideal_sum {
        let extra = avail_width - min_sum;
        let needed_extra: Vec<u16> = ideal_widths
            .iter()
            .zip(min_widths.iter())
            .map(|(&ideal, &min)| ideal.saturating_sub(min))
            .collect();
        let total_needed_extra: u32 = needed_extra
            .iter()
            .map(|&x| u32::from(x))
            .sum::<u32>()
            .max(1);

        let mut allocated = 0u16;
        for i in 0..count {
            let add =
                u16::try_from((u32::from(extra) * u32::from(needed_extra[i])) / total_needed_extra)
                    .unwrap_or(u16::MAX);
            widths[i] = min_widths[i] + add;
            allocated += widths[i];
        }
        let mut rem = avail_width.saturating_sub(allocated);
        let mut i = 0;
        while rem > 0 && count > 0 {
            widths[i % count] += 1;
            rem -= 1;
            i += 1;
        }
    } else {
        let extra = avail_width - ideal_sum;
        let base_add = extra / count_u16;
        let mut rem = extra % count_u16;

        for i in 0..count {
            widths[i] = ideal_widths[i]
                + base_add
                + if rem > 0 {
                    rem -= 1;
                    1
                } else {
                    0
                };
        }
    }

    widths
}

fn help_columns(area: Rect, groups: &[Vec<HelpSection>]) -> Vec<Rect> {
    if groups.is_empty() {
        return vec![];
    }
    if groups.len() == 1 {
        return vec![area];
    }
    let widths = help_columns_widths(area.width, groups);
    let gap = 2u16;
    let mut rects = Vec::with_capacity(groups.len());
    let mut x = area.x;
    for &w in &widths {
        rects.push(Rect::new(x, area.y, w, area.height));
        x = x.saturating_add(w).saturating_add(gap);
    }
    rects
}

fn wrap_help_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let separator = usize::from(!line.is_empty());
        if !line.is_empty() && line.len() + separator + word.len() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        if word.len() <= width {
            line.push_str(word);
        } else {
            // No shortcut text currently needs this, but it keeps even an
            // unusually long future word inside the allocated column.
            for chunk in word.as_bytes().chunks(width) {
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                lines.push(String::from_utf8_lossy(chunk).into_owned());
            }
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn format_help_column(group: &[HelpSection], width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let key_width = group
        .iter()
        .flat_map(|section| section.entries)
        .map(|(key, _)| key.len())
        .max()
        .unwrap_or(0);
    let prefix_width = key_width + 4; // two-space left indent and key/description gap
    let description_width = usize::from(width).saturating_sub(prefix_width).max(1);
    let mut lines = Vec::new();
    for (index, section) in group.iter().enumerate() {
        lines.extend(
            wrap_help_text(section.title, usize::from(width))
                .into_iter()
                .map(|line| {
                    Line::styled(
                        line,
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD),
                    )
                }),
        );
        for scope in section.scope {
            lines.extend(
                wrap_help_text(scope, usize::from(width).saturating_sub(2))
                    .into_iter()
                    .map(|line| {
                        Line::styled(format!("  {line}"), Style::default().fg(theme.muted))
                    }),
            );
        }
        for (key, description) in section.entries {
            let description = wrap_help_text(description, description_width);
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {key:<key_width$}  "),
                    Style::default().fg(theme.secondary),
                ),
                Span::styled(description[0].clone(), Style::default().fg(theme.text)),
            ]));
            for continuation in description.into_iter().skip(1) {
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(prefix_width)),
                    Span::styled(continuation, Style::default().fg(theme.text)),
                ]));
            }
        }
        if index + 1 < group.len() {
            lines.push(Line::raw(""));
        }
    }
    lines
}

/// The help overlay is deliberately organized by input behavior rather than
/// by destination. Each row has one command or one pair of equivalent keys,
/// making the reference readable without sacrificing accuracy.
fn keyboard_reference() -> Vec<HelpSection> {
    general_help_sections()
}

fn general_help_sections() -> Vec<HelpSection> {
    vec![
        HelpSection {
            title: "GLOBAL & NAVIGATION",
            scope: &[],
            entries: &[
                ("?", "toggle reference"),
                ("shift+L", "toggle TASKS (background jobs)"),
                ("M", "open mind.md (your thoughts)"),
                ("W", "open state.md (project status)"),
                ("J", "研究日誌 (research journal, last 7 days)"),
                ("/", "toggle arXiv search"),
                ("Ctrl+b", "toggle navigator"),
                ("Ctrl+t", "terminal palette"),
                ("Esc", "close local search"),
                ("Enter / → / l", "open selected item"),
                ("← / h", "return one level"),
                ("q", "quit outside text input"),
            ],
        },
        HelpSection {
            title: "DASHBOARD & PROJECT",
            scope: &["Dashboard and Project screens."],
            entries: &[
                (
                    "N",
                    "create a 'next step' card (paper-next) in sai-link todo",
                ),
                ("U", "create an 'understanding check' card (paper-check)"),
                ("A", "attention list (Project, j/k Enter x)"),
            ],
        },
        HelpSection {
            title: "PAPER ACTIONS",
            scope: &[
                "Paper rows: Library, Downloads, Bookmarks, Notes,",
                "Reading Queue, and paper rows in Groups or Authors.",
            ],
            entries: &[
                ("Enter / → / l", "open PAPER screen"),
                ("shift+K", "toggle hierarchy view (search history)"),
                ("o", "open paper online"),
                ("B", "toggle bookmark"),
                (">", "toggle local search"),
                ("n / m", "open note.md / extract.md in nvim"),
                ("t", "edit tags (comma separated)"),
                ("g", "assign to group"),
                ("R", "rename PDF"),
                ("c", "copy citation"),
                ("x", "confirm then delete PDF"),
                ("u", "mark PDF unread"),
            ],
        },
        HelpSection {
            title: "INTERNAL PDF VIEWER",
            scope: &[],
            entries: &[("Esc / q", "close internal viewer")],
        },
        HelpSection {
            title: "CREDITS",
            scope: &[],
            entries: &[("Enter", "open selected link")],
        },
        HelpSection {
            title: "DISCOVER & DASHBOARD",
            scope: &[],
            entries: &[
                ("/", "toggle arXiv search"),
                ("d", "download paper"),
                ("Enter", "open downloaded PDF"),
            ],
        },
        HelpSection {
            title: "DISCOVER",
            scope: &[],
            entries: &[
                ("/", "toggle arXiv search"),
                ("Tab / Shift+Tab", "word / year / limit fields"),
                ("year", "publish year range (e.g. 2010-2020)"),
                ("limit", "max results (default 100)"),
                ("Ctrl+←", "previous cached page"),
                ("Ctrl+→", "next cached page"),
                ("r", "retry loading failure"),
            ],
        },
        HelpSection {
            title: "PAPER DETAIL",
            scope: &["Library or Discover/Dashboard paper detail view."],
            entries: &[
                ("j/k", "scroll"),
                ("h/l back", "return to list"),
                ("p/Enter", "open PDF"),
                ("c", "copy citation"),
                ("o", "open online"),
                ("s", "citation extract (tui)"),
                ("n / m", "open note.md / extract.md in nvim"),
                (
                    "space n / space m",
                    "built-in Markdown viewer (note / extract)",
                ),
                ("a", "translate abstract (EN→JA)"),
                ("A", "re-translate abstract"),
                ("T", "translate PDF (pdf2zh) / open ja-PDF"),
                ("r", "make Radio audio (sai-radio, minutes)"),
                ("shift+L", "show running background jobs"),
            ],
        },
        HelpSection {
            title: "MARKDOWN VIEWER",
            scope: &[],
            entries: &[
                ("Enter", "open the citation on the line (V06)"),
                ("n / N", "next / previous citation line (V06)"),
            ],
        },
        HelpSection {
            title: "QUERY SCREEN",
            scope: &["Past Discover search opened from the Library hierarchy view."],
            entries: &[
                ("j/k", "select paper"),
                ("Enter", "open PAPER screen"),
                ("e", "edit Reason (Enter save / Esc cancel)"),
                ("Esc / h", "back to Library"),
                ("green", "downloaded paper"),
                ("muted", "not downloaded yet"),
            ],
        },
        HelpSection {
            title: "GROUPS",
            scope: &[],
            entries: &[("g", "create a group")],
        },
        HelpSection {
            title: "QUEUE",
            scope: &[],
            entries: &[
                ("Shift+↑", "move queued paper up"),
                ("Shift+↓", "move queued paper down"),
            ],
        },
        HelpSection {
            title: "DOWNLOADS",
            scope: &[],
            entries: &[("r", "retry failed download")],
        },
        HelpSection {
            title: "SEARCH SETS",
            scope: &[],
            entries: &[
                ("j/k", "move down/up"),
                ("Tab/h/l", "switch pane"),
                ("Enter", "open detail view"),
                ("d", "download paper"),
                ("i", "manual import"),
                ("K", "keep candidate"),
                ("x", "drop candidate"),
                ("o", "open online"),
                ("r", "rerun the saved search"),
            ],
        },
        HelpSection {
            title: "RAG SEARCH",
            scope: &["Library, Notes screens."],
            entries: &[
                ("f", "RAG natural language search (hybrid / vector / word)"),
                ("Esc", "close search results"),
                ("Enter", "toggle search / result view"),
            ],
        },
    ]
}

fn render_too_small(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    frame.render_widget(
        Paragraph::new("papr needs a terminal at least 58 x 18")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.warning)),
        Rect::new(area.x, area.y + area.height / 2, area.width, 1),
    );
}

fn centered(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// 手動 PDF 取り込みモーダル（~/Downloads の最近 5 分の PDF から選択）。
fn render_manual_import(frame: &mut Frame<'_>, app: &App, theme: &Theme) {
    let Some(paper) = &app.manual_import.paper else {
        return;
    };
    let paper_title = match paper {
        ManualImportPaper::Local { title, .. } => title.clone(),
        ManualImportPaper::Remote(remote) => remote.title.clone(),
    };
    let candidates = &app.manual_import.candidates;
    if candidates.is_empty() {
        return;
    }
    // カーソルが見えるように最大 8 件の窓を表示する。
    const MAX_VISIBLE: usize = 8;
    let selected = app.manual_import.selected;
    let start = selected
        .saturating_sub(MAX_VISIBLE - 1)
        .min(candidates.len().saturating_sub(MAX_VISIBLE));
    let end = (start + MAX_VISIBLE).min(candidates.len());

    let mut lines: Vec<Line<'_>> = vec![
        Line::raw(""),
        Line::styled(
            paper_title.clone(),
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            "Select the PDF you just saved in your browser:",
            Style::default().fg(theme.muted),
        ),
        Line::raw(""),
    ];
    for (index, path) in candidates.iter().enumerate().skip(start).take(end - start) {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let size = std::fs::metadata(path)
            .map(|meta| format!("  {:.1} MiB", meta.len() as f64 / (1024.0 * 1024.0)))
            .unwrap_or_default();
        let line = if index == selected {
            Line::styled(
                format!("▶ {name}{size}"),
                Style::default()
                    .fg(theme.accent)
                    .bg(theme.surface)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Line::styled(format!("  {name}{size}"), Style::default().fg(theme.text))
        };
        lines.push(line);
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "j/k select  Enter import  Esc cancel",
        Style::default().fg(theme.muted),
    ));

    let height = lines.len() as u16 + 2;
    let width = 76_u16;
    let area = centered(width, height, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(Line::styled(
                " MANUAL IMPORT ",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(theme.surface)),
        area,
    );
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().padding(ratatui::widgets::Padding::horizontal(1)))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_block_picker(frame: &mut Frame<'_>, app: &App, theme: &Theme) {
    if app.block_picker.blocks.is_empty() {
        return;
    }

    let blocks = &app.block_picker.blocks;
    let selected = app.block_picker.selected;
    let scroll = app.block_picker.scroll;
    const MAX_VISIBLE: usize = 12;
    let end = (scroll + MAX_VISIBLE).min(blocks.len());

    let mut lines: Vec<Line<'_>> = vec![
        Line::raw(""),
        Line::styled(
            "実行ブロックを選択",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
    ];

    for (index, block) in blocks.iter().enumerate().skip(scroll).take(end - scroll) {
        let display = format!(
            "{} — {}",
            block.name,
            if block.description.is_empty() {
                "(説明なし)".to_string()
            } else {
                block.description.clone()
            }
        );
        let line = if index == selected {
            Line::styled(
                format!("▶ {}", display),
                Style::default()
                    .fg(theme.accent)
                    .bg(theme.surface)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Line::styled(format!("  {}", display), Style::default().fg(theme.text))
        };
        lines.push(line);
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "j/k 選択  Enter 決定  Esc キャンセル",
        Style::default().fg(theme.muted),
    ));

    let height = lines.len() as u16 + 2;
    let width = 80_u16;
    let area = centered(width, height, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(Line::styled(
                " BLOCK PICKER ",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(theme.surface)),
        area,
    );
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().padding(ratatui::widgets::Padding::horizontal(1)))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn get_paper_collection_name(_app: &App, _paper_id: i64) -> Option<String> {
    // Collections は廃止（仕様 §3.1）。
    None
}

fn safe_truncate(s: &str, max_width: usize) -> String {
    if s.chars().count() <= max_width {
        s.to_string()
    } else {
        let mut truncated: String = s.chars().take(max_width.saturating_sub(3)).collect();
        truncated.push_str("...");
        truncated
    }
}

#[derive(Clone, Copy)]
struct PaperLineContext<'a> {
    paper_id: Option<i64>,
    title: &'a str,
    authors: &'a str,
    reading_status: &'a str,
    file_size: Option<u64>,
    availability: Option<&'a str>,
    bookmark_year: Option<&'a str>,
    bookmark_journal: Option<&'a str>,
    bookmark_doi: Option<&'a str>,
    bookmark_page: Option<u64>,
    download_label: Option<(&'a str, ratatui::style::Color)>,
    available_width: usize,
}

fn build_paper_lines<'a>(app: &App, theme: &Theme, context: PaperLineContext<'a>) -> Vec<Line<'a>> {
    let PaperLineContext {
        title,
        authors,
        available_width,
        ..
    } = context;
    let stats_spans = paper_stats_spans(app, theme, context);

    let stats_len: usize = stats_spans.iter().map(ratatui::prelude::Span::width).sum();
    let authors_str = if authors.is_empty() {
        "Unknown authors"
    } else {
        authors
    };
    let separator_len = if stats_spans.is_empty() { 0 } else { 5 };
    let max_authors_width = available_width
        .saturating_sub(stats_len)
        .saturating_sub(separator_len);

    if stats_spans.is_empty()
        || authors_str.chars().count() <= max_authors_width
        || max_authors_width >= 20
    {
        let display_authors = if authors_str.chars().count() <= max_authors_width {
            authors_str.to_string()
        } else {
            safe_truncate(authors_str, max_authors_width)
        };

        let mut line_spans = vec![Span::styled(
            display_authors,
            Style::default().fg(theme.muted),
        )];
        if !stats_spans.is_empty() {
            line_spans.push(Span::styled("  |  ", Style::default().fg(theme.border)));
            line_spans.extend(stats_spans);
        }
        vec![
            Line::styled(
                title.to_string(),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
            Line::from(line_spans),
        ]
    } else {
        let mut lines = vec![
            Line::styled(
                title.to_string(),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
            Line::styled(
                safe_truncate(authors_str, available_width),
                Style::default().fg(theme.muted),
            ),
        ];
        if !stats_spans.is_empty() {
            lines.push(Line::from(stats_spans));
        }
        lines
    }
}

fn paper_stats_spans<'a>(app: &App, theme: &Theme, context: PaperLineContext<'a>) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    if !context.reading_status.is_empty() {
        push_paper_stat(
            &mut spans,
            context.reading_status.to_owned(),
            theme.muted,
            theme,
        );
    }
    if let Some(year) = context.bookmark_year {
        push_paper_stat(&mut spans, year.to_owned(), theme.muted, theme);
    }
    if let Some(journal) = context.bookmark_journal {
        push_paper_stat(&mut spans, journal.to_owned(), theme.muted, theme);
    } else if let Some(doi) = context.bookmark_doi {
        push_paper_stat(&mut spans, format!("DOI {doi}"), theme.muted, theme);
    }
    if let Some(page) = context.bookmark_page {
        push_paper_stat(&mut spans, format!("page {page}"), theme.muted, theme);
    }
    if let Some(name) = context
        .paper_id
        .and_then(|id| get_paper_collection_name(app, id))
    {
        push_paper_separator(&mut spans, theme);
        spans.push(Span::styled("(", Style::default().fg(theme.muted)));
        spans.push(Span::styled(
            name,
            Style::default()
                .bg(theme.surface)
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(")", Style::default().fg(theme.muted)));
    }
    if let Some(size) = context.file_size {
        push_paper_stat(&mut spans, format_bytes(size), theme.muted, theme);
    } else if context.bookmark_year.is_none()
        && context.download_label.is_none()
        && context.paper_id.is_some()
    {
        push_paper_stat(&mut spans, "metadata only".to_owned(), theme.muted, theme);
    }
    if let Some(availability) = context.availability {
        push_paper_stat(&mut spans, availability.to_owned(), theme.muted, theme);
    }
    if let Some((label, color)) = context.download_label {
        push_paper_stat(&mut spans, label.to_owned(), color, theme);
    }
    spans
}

fn push_paper_separator(spans: &mut Vec<Span<'_>>, theme: &Theme) {
    if !spans.is_empty() {
        spans.push(Span::styled("  |  ", Style::default().fg(theme.border)));
    }
}

fn push_paper_stat(
    spans: &mut Vec<Span<'_>>,
    value: String,
    color: ratatui::style::Color,
    theme: &Theme,
) {
    push_paper_separator(spans, theme);
    spans.push(Span::styled(value, Style::default().fg(color)));
}

/// RAG ハイブリッド検索のレンダリング。
fn render_rag_search(frame: &mut Frame<'_>, app: &mut App, theme: &Theme) {
    // 画面中央に検索ボックスと結果を表示。
    let area = frame.area();
    let width = (area.width as usize).max(40).min(120);
    let height = (area.height as usize).max(10).min(30);
    let x = (area.width as usize - width) / 2;
    let y = (area.height as usize - height) / 2;
    let dialog_area = Rect::new(x as u16, y as u16, width as u16, height as u16);

    // 背景。
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.surface).fg(theme.border)),
        dialog_area,
    );

    let border = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(if app.rag_search.evidence {
            " RAG Search [要約付き] (Esc to exit) "
        } else {
            " RAG Search (Esc to exit) "
        });
    let inner = border.inner(dialog_area);
    frame.render_widget(border, dialog_area);

    // 検索ボックス（高さ 3）。
    let search_box_area = Rect::new(inner.x, inner.y, inner.width, 3);
    let search_block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(theme.border));

    let search_text = if app.rag_search.query.is_empty() {
        Span::styled("Type to search...", Style::default().fg(theme.muted))
    } else {
        Span::styled(
            app.rag_search.query.clone(),
            Style::default().fg(theme.text),
        )
    };
    frame.render_widget(
        Paragraph::new(search_text).block(search_block),
        search_box_area,
    );

    // カーソルを表示。
    if !app.rag_search.query.is_empty() {
        let cursor_x = search_box_area.x
            + (app.rag_search.query_cursor as u16).min(search_box_area.width.saturating_sub(1));
        frame.set_cursor_position((cursor_x, search_box_area.y + 1));
    }

    // 検索結果・メッセージ領域。
    let results_area = Rect::new(
        inner.x,
        inner.y + 3,
        inner.width,
        inner.height.saturating_sub(5), // フッタースペース確保
    );

    if app.rag_search.searching {
        // 検索中…
        frame.render_widget(
            Paragraph::new("検索中…").style(Style::default().fg(theme.muted)),
            results_area,
        );
    } else if app.rag_search.results.is_empty() {
        let msg = if app.rag_search.query.is_empty() {
            "Type to search..."
        } else {
            "No results found"
        };
        frame.render_widget(
            Paragraph::new(msg).style(Style::default().fg(theme.muted)),
            results_area,
        );
    } else {
        // 検索結果の一覧。
        let items: Vec<ListItem> = app
            .rag_search
            .results
            .iter()
            .enumerate()
            .map(|(i, hit)| {
                let title = hit.title.as_deref().unwrap_or(&hit.source_ref);
                let year_str = hit.year.map(|y| format!(" ({})", y)).unwrap_or_default();
                let snippet = hit.text.lines().next().unwrap_or("").trim();
                let score_str = format!("{:.2}", hit.score);
                let kind_label = match hit.source_kind.as_str() {
                    "abstract" => "abs",
                    "body" => "body",
                    "note" => "note",
                    "figure" => "fig",
                    "document" => "doc",
                    _ => "other",
                };
                // フォーマット: [score] kind | title (year) > snippet
                let display_text = if snippet.len() > 40 {
                    format!(
                        "[{}] {} | {}{} > {}",
                        score_str,
                        kind_label,
                        title,
                        year_str,
                        &snippet[..40]
                    )
                } else {
                    format!(
                        "[{}] {} | {}{} > {}",
                        score_str, kind_label, title, year_str, snippet
                    )
                };
                // 図の結果は薄い色で `図 <label>` を足す。その他の結果は章があれば `§…` を足す。
                let mut spans = Vec::new();
                // 要約付きのときは関連度を行の先頭に出す。
                if app.rag_search.evidence {
                    let mark = match app.rag_search.relevances.get(i).copied().flatten() {
                        Some(relevance) => format!("関連度 {relevance}/10 "),
                        None => "関連度 ? ".to_string(),
                    };
                    spans.push(Span::styled(mark, Style::default().fg(theme.accent)));
                }
                spans.push(Span::styled(display_text, Style::default()));
                if hit.source_kind == "figure" {
                    let label = hit.section.as_deref().unwrap_or("");
                    let mark = if label.is_empty() {
                        " 図".to_string()
                    } else {
                        format!(" 図 {label}")
                    };
                    spans.push(Span::styled(mark, Style::default().fg(theme.muted)));
                } else if let Some(section) = &hit.section {
                    spans.push(Span::styled(
                        format!(" §{section}"),
                        Style::default().fg(theme.muted),
                    ));
                }
                // 選んだ行の下に要約を出す（要約付きのときだけ）。
                let summary = if i == app.rag_search.selected && app.rag_search.evidence {
                    app.rag_search
                        .summaries
                        .get(i)
                        .and_then(|s| s.as_deref())
                        .map(str::to_string)
                } else {
                    None
                };
                let item = match summary {
                    Some(summary) => ListItem::new(vec![
                        Line::from(spans),
                        Line::from(Span::styled(
                            format!("   要約: {summary}"),
                            Style::default().fg(theme.muted),
                        )),
                    ]),
                    None => ListItem::new(Line::from(spans)),
                };
                item.style(if i == app.rag_search.selected {
                    Style::default().fg(theme.accent).bg(theme.surface)
                } else {
                    Style::default().fg(theme.text)
                })
            })
            .collect();
        let list = List::new(items).highlight_symbol("› ");
        let mut state = ListState::default()
            .with_selected(Some(app.rag_search.selected))
            .with_offset(app.rag_search.scroll);
        frame.render_stateful_widget(list, results_area, &mut state);
        app.rag_search.scroll = state.offset();
    }

    // フッター：キーヒント
    let footer_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(2),
        inner.width,
        2,
    );
    let footer = if app.rag_search.results.is_empty() {
        if app.rag_search.evidence {
            "Enter: 検索（要約付き） | Ctrl+e: 要約付きの切り替え | Esc: 閉じる".to_string()
        } else {
            "Enter: 検索 | Ctrl+e: 要約付き | Esc: 閉じる".to_string()
        }
    } else if app.rag_search.evidence {
        "Enter: 開く | m: markdown | ↑↓: 移動 | Ctrl+e: 要約付きの切り替え | Esc: 閉じる"
            .to_string()
    } else {
        "Enter: 開く | m: markdown | Ctrl+e: 要約付き | ↑↓: 移動 | Esc: 閉じる".to_string()
    };
    let mut footer_lines = vec![Line::from(Span::styled(
        footer,
        Style::default().fg(theme.muted),
    ))];
    // 要約付きのときは、最後の行に使ったトークン数（費用）を出す。
    if app.rag_search.evidence && !app.rag_search.results.is_empty() {
        let total = app.rag_search.results.len() + app.rag_search.dropped;
        footer_lines.push(Line::from(Span::styled(
            format!(
                "要約付き: {total} 件中 {} 件を外しました。トークン 入力 {}・出力 {}（失敗 {}）",
                app.rag_search.dropped,
                app.rag_search.prompt_tokens,
                app.rag_search.completion_tokens,
                app.rag_search.failed,
            ),
            Style::default().fg(theme.muted),
        )));
    }
    frame.render_widget(
        Paragraph::new(footer_lines).style(Style::default().fg(theme.muted)),
        footer_area,
    );
}

#[cfg(test)]
mod tests {
    /// バッファの全セル記号を連結する（ワイド文字の継続セルは空白で入る）。
    fn buffer_symbols(buffer: &ratatui::buffer::Buffer) -> String {
        buffer
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    /// バッファ記号から空白を除く（ワイド文字のセル分割を吸収して日本語を照合する）。
    fn compact_symbols(text: &str) -> String {
        text.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// バッファ内で記号が最初に現れる (index, row) を返す。
    fn find_symbol(buffer: &ratatui::buffer::Buffer, symbol: &str) -> Option<(usize, u16)> {
        let width = usize::from(buffer.area.width);
        buffer
            .content
            .iter()
            .position(|cell| cell.symbol() == symbol)
            .map(|index| (index, u16::try_from(index / width).unwrap_or(0)))
    }

    fn symbol_row(buffer: &ratatui::buffer::Buffer, symbol: &str) -> Option<(usize, u16)> {
        find_symbol(buffer, symbol)
    }

    use crate::state::*;
    use crate::theme::*;
    use papr_core::LibraryPaper;
    use ratatui::{
        Terminal, backend::TestBackend, layout::Rect,
    };

    use super::{
        activity_category, activity_description, format_help_column, help_column_groups,
        help_columns, keyboard_reference, render, render_tasks, workspace_highlight_selection,
    };

    #[test]
    fn activity_category_labels_open_and_agent_read() {
        let theme = Theme::load("nord").unwrap();
        assert_eq!(activity_category("open", &theme).0, "Open");
        assert_eq!(activity_category("agent_read", &theme).0, "Agent read");
        // 知らない種類はこれまでどおり Other にする。
        assert_eq!(activity_category("unknown", &theme).0, "Other");
    }

    #[test]
    fn activity_description_shows_target_and_file_for_open_detail() {
        let item = papr_core::ActivityItem {
            kind: "open".into(),
            paper_id: None,
            label: r#"{"path":".sai/outputs/R9/41/answer.md","surface":"tui","target":"report"}"#
                .into(),
            occurred_at: chrono::Utc::now(),
            radio_id: None,
        };
        assert_eq!(activity_description(&item), "report  answer.md");

        // 論文タイトルが引けている行は、そのまま出す。
        let titled = papr_core::ActivityItem {
            kind: "open".into(),
            paper_id: Some(1),
            label: "Attention Is All You Need".into(),
            occurred_at: chrono::Utc::now(),
            radio_id: None,
        };
        assert_eq!(activity_description(&titled), "Attention Is All You Need");
    }

    #[test]
    fn discover_row_shows_hits_and_undownloaded_marker() {
        let theme = Theme::load("nord").unwrap();
        let line = super::discover_list_columns_line(
            "2017/06",
            Some(80000),
            Some(3),
            true,
            Some("NeurIPS"),
            "Attention Is All You Need",
            &theme,
            90,
            None,
        );
        let text: String = line[0]
            .spans
            .iter()
            .map(|span| span.content.to_string())
            .collect();
        // hits 列（数値 + 未 DL 印）
        assert!(text.contains("3*"), "{text}");
        // 年/月・被引用数・ジャーナルは従来どおり
        assert!(text.contains("2017/06"));
        assert!(text.contains("80.0k"));
        assert!(text.contains("NeurIPS"));

        // ダウンロード済みなら * は付かない
        let line = super::discover_list_columns_line(
            "2017/06",
            Some(80000),
            Some(3),
            false,
            Some("NeurIPS"),
            "Attention Is All You Need",
            &theme,
            90,
            Some(theme.success),
        );
        let text: String = line[0]
            .spans
            .iter()
            .map(|span| span.content.to_string())
            .collect();
        assert!(!text.contains("3*"));

        // 集計前（None）は -
        let line = super::discover_list_columns_line(
            "2017/06",
            None,
            None,
            true,
            None,
            "Title",
            &theme,
            90,
            None,
        );
        let text: String = line[0]
            .spans
            .iter()
            .map(|span| span.content.to_string())
            .collect();
        assert!(text.contains('-'));
    }

    #[test]
    fn library_detail_renders_figures_section() -> Result<(), Box<dyn std::error::Error>> {
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend)?;
        let theme = Theme::load("nord")?;
        let mut app = App::default();
        app.paper_figures = vec![papr_core::figures::FigureRow {
            paper_id: "ArXiv:2401.00001".into(),
            fig_no: 1,
            label: Some("Figure 1".into()),
            caption: Some("Workflow overview.".into()),
            image_path: "/nonexistent/fig-001.png".into(),
            mention: Some("As shown in Figure 1, it works.".into()),
            ..Default::default()
        }];
        let lines = vec![ratatui::text::Line::raw("Title line")];
        terminal.draw(|frame| {
            let area = frame.area();
            super::render_library_paper_detail(frame, area, &mut app, &theme, lines);
        })?;
        let text = buffer_symbols(terminal.backend().buffer());
        let compact = compact_symbols(&text);
        assert!(text.contains("FIGURES"), "FIGURES 見出しが出ていない");
        assert!(text.contains("Figure 1."), "図ラベルが出ていない");
        assert!(
            text.contains("Workflow overview."),
            "キャプションが出ていない"
        );
        assert!(compact.contains("本文:"), "本文中の言及行が出ていない");
        // 内容行数 = テキスト 3 行 + 見出し/画像/キャプション/言及
        assert_eq!(app.paper_detail_content_rows, 3 + 1 + 14 + 1 + 1);
        assert_eq!(app.paper_detail_view_rows, 40);
        Ok(())
    }

    /// 図の画像は、ブロック全体が収まらないスクロール位置でも見えている分だけ描く。
    /// （旧実装は「全体が収まるときだけ」描いたため、ビューポートより高い図ブロックでは
    /// スクロール位置のほとんどで 14 行の空白になり、2 枚目以降が見つからなかった）
    #[test]
    fn figure_image_clipped_instead_of_skipped() {
        let area = Rect::new(0, 0, 100, 19);
        let image_rows = usize::from(super::FIGURE_IMAGE_ROWS);
        // 図ブロック: 見出し 1 + 画像 14 + キャプション 10 + 言及 3 = 28 行
        let header_row = 40;
        let image_row = header_row + 1;
        let block_end = image_row + image_rows + 10 + 3;
        let shown = (0..block_end + 20)
            .filter(|scroll| super::visible_rect(image_row, image_rows, *scroll, area).is_some())
            .count();
        // 見えている分を描くので「ビューポート行数 + 画像行数 - 1」箇所で写る。
        // 「全体が収まるときだけ」描く旧実装では 19 - 14 + 1 = 6 箇所しかなかった。
        assert_eq!(shown, usize::from(area.height) + image_rows - 1);
        // 上下にはみ出した分は縮めて描く（隠れた行数も返す）
        let (rect, hidden) = super::visible_rect(image_row, image_rows, image_row + 4, area)
            .expect("上にはみ出していても描く");
        assert_eq!((rect.y, rect.height, hidden), (0, 10, 4));
        // 完全に画面外なら None
        assert!(super::visible_rect(image_row, image_rows, block_end + 50, area).is_none());
        assert!(super::visible_rect(image_row, image_rows, 0, Rect::new(0, 0, 100, 0)).is_none());
    }

    /// ビューポートより高いキャプションも、隠れずに見えている分が描かれる。
    #[test]
    fn library_detail_shows_caption_taller_than_viewport()
    -> Result<(), Box<dyn std::error::Error>> {
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend)?;
        let theme = Theme::load("nord")?;
        let mut app = App::default();
        let caption = "caption ".repeat(120);
        app.paper_figures = vec![papr_core::figures::FigureRow {
            paper_id: "ArXiv:2401.00001".into(),
            fig_no: 1,
            label: Some("Figure 1".into()),
            caption: Some(caption),
            image_path: "/nonexistent/fig-001.png".into(),
            ..Default::default()
        }];
        let lines = vec![ratatui::text::Line::raw("Title line")];
        // キャプション先頭（テキスト 3 行 + 見出し 1 + 画像 14）が画面上端に来る位置
        app.paper_detail_scroll = 18;
        terminal.draw(|frame| {
            let area = frame.area();
            super::render_library_paper_detail(frame, area, &mut app, &theme, lines);
        })?;
        let text = buffer_symbols(terminal.backend().buffer());
        assert!(text.contains("caption"), "長いキャプションが出ていない");
        Ok(())
    }

    #[test]
    fn library_detail_shows_not_extracted_placeholder_without_figures()
    -> Result<(), Box<dyn std::error::Error>> {
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend)?;
        let theme = Theme::load("nord")?;
        let mut app = App::default();
        app.paper_figures_not_extracted = true;
        let lines = vec![ratatui::text::Line::raw("Title line")];
        terminal.draw(|frame| {
            let area = frame.area();
            super::render_library_paper_detail(frame, area, &mut app, &theme, lines);
        })?;
        let compact = compact_symbols(&buffer_symbols(terminal.backend().buffer()));
        assert!(compact.contains("(図は未抽出)"));
        assert!(!compact.contains("FIGURES"));
        Ok(())
    }

    #[test]
    fn keyboard_reference_reflows_without_exceeding_each_column()
    -> Result<(), Box<dyn std::error::Error>> {
        let theme = Theme::load("nord")?;
        for (width, count) in [(54, 1), (76, 2), (130, 3)] {
            let groups = help_column_groups(keyboard_reference(), count, width);
            let columns = help_columns(Rect::new(0, 0, width, 30), &groups);
            for (group, column) in groups.iter().zip(columns) {
                assert!(
                    format_help_column(group, column.width, &theme)
                        .iter()
                        .all(|line| line.width() <= usize::from(column.width))
                );
            }
        }
        Ok(())
    }

    #[test]
    fn keyboard_reference_balances_column_heights_and_widths()
    -> Result<(), Box<dyn std::error::Error>> {
        let theme = Theme::load("nord")?;

        // Check 3-column layout on wide screen
        let groups_3col = help_column_groups(keyboard_reference(), 3, 130);
        assert_eq!(groups_3col.len(), 3);
        let cols_3col = help_columns(Rect::new(0, 0, 130, 30), &groups_3col);
        let heights_3col: Vec<usize> = groups_3col
            .iter()
            .zip(&cols_3col)
            .map(|(g, col)| format_help_column(g, col.width, &theme).len())
            .collect();
        let max_h3 = heights_3col.iter().max().copied().ok_or("missing column")?;
        let min_h3 = heights_3col.iter().min().copied().ok_or("missing column")?;
        // Heights should be well balanced: max height should be <= 52 lines and diff <= 10 lines
        // （52 は S02 の要対応の一覧の 1 行を足したあとの上限。ヘルプはスクロールできる）
        assert!(
            max_h3 <= 52,
            "Max height in 3-col layout should be <= 52, got {max_h3}"
        );
        assert!(
            max_h3 - min_h3 <= 10,
            "Height diff in 3-col layout should be <= 10, got {}",
            max_h3 - min_h3
        );

        // Check 2-column layout on medium screen
        let groups_2col = help_column_groups(keyboard_reference(), 2, 86);
        assert_eq!(groups_2col.len(), 2);
        let cols_2col = help_columns(Rect::new(0, 0, 86, 30), &groups_2col);
        let heights_2col: Vec<usize> = groups_2col
            .iter()
            .zip(&cols_2col)
            .map(|(g, col)| format_help_column(g, col.width, &theme).len())
            .collect();
        let max_h2 = heights_2col.iter().max().copied().ok_or("missing column")?;
        let min_h2 = heights_2col.iter().min().copied().ok_or("missing column")?;
        assert!(
            max_h2 - min_h2 <= 10,
            "Height diff in 2-col layout should be <= 10, got {}",
            max_h2 - min_h2
        );
        Ok(())
    }

    #[test]
    fn workspace_selection_highlight_is_only_rendered_while_content_has_focus() {
        let mut app = App {
            content_focused: false,
            ..App::default()
        };

        assert_eq!(workspace_highlight_selection(&app, 4), None);

        app.content_focused = true;
        assert_eq!(workspace_highlight_selection(&app, 4), Some(4));
    }

    #[test]
    fn inactive_workspace_hides_the_row_highlight_without_changing_selection_or_scroll()
    -> Result<(), Box<dyn std::error::Error>> {
        let backend = TestBackend::new(100, 32);
        let mut terminal = Terminal::new(backend)?;
        let theme = Theme::load("nord")?;
        let mut app = App {
            page: Page::Library,
            content_focused: false,
            library: LibraryState {
                selected: 0,
                scroll: 0,
                papers: vec![LibraryPaper {
                    id: 1,
                    title: "Focus Aware Paper".into(),
                    authors: "Test Author".into(),
                    doi: None,
                    arxiv_id: None,
                    pdf_path: None,
                    file_size: None,
                    reading_status: "unread".into(),
                    is_favorite: false,
                    abstract_text: None,
                    journal: None,
                    published_at: None,
                    abstract_ja: None,
                    abstract_ja_translated_at: None,
                    citation_count: None,
                    is_retracted: None,
                    retraction_checked_at: None,
                    updated_at: None,
                }],
                ..LibraryState::default()
            },
            ..App::default()
        };

        // 1 行形式（<first author> <year> <title>）ではタイトルが行頭に来ないため、
        // タイトル "F" の行にハイライト背景が付くかどうかで検証する
        let row_has_highlight = |buffer: &ratatui::buffer::Buffer, row: u16| {
            let width = usize::from(buffer.area.width);
            let start = usize::from(row) * width;
            buffer.content[start..start + width]
                .iter()
                .any(|cell| cell.style().bg == Some(theme.surface))
        };

        terminal.draw(|frame| render(frame, &mut app, &theme))?;
        let buffer = terminal.backend().buffer();
        let (fi, fy) = symbol_row(buffer, "F").ok_or("paper title should be rendered")?;
        assert!(
            !row_has_highlight(buffer, fy),
            "no highlight while unfocused"
        );
        assert_ne!(buffer.content[fi].style().bg, Some(theme.surface));
        assert_eq!(app.library.selected, 0);
        assert_eq!(app.library.scroll, 0);

        app.content_focused = true;
        terminal.draw(|frame| render(frame, &mut app, &theme))?;
        let buffer = terminal.backend().buffer();
        let (fi, fy) = symbol_row(buffer, "F").ok_or("paper title should be rendered")?;
        assert!(row_has_highlight(buffer, fy), "highlight while focused");
        assert_eq!(buffer.content[fi].style().bg, Some(theme.surface));
        assert_eq!(app.library.selected, 0);
        assert_eq!(app.library.scroll, 0);
        Ok(())
    }

    fn running_op_fixture(title: &str, steps: &[&str], step: usize) -> RunningOperation {
        RunningOperation {
            key: format!("pipeline:{title}"),
            label: String::new(),
            title: title.into(),
            steps: steps.iter().map(|s| (*s).to_string()).collect(),
            step,
            started_at: std::time::Instant::now(),
        }
    }

    #[test]
    fn tasks_screen_lists_running_jobs_with_step_and_percentage()
    -> Result<(), Box<dyn std::error::Error>> {
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend)?;
        let theme = Theme::load("nord")?;
        let app = App {
            mode: AppMode::Tasks,
            running_operations: vec![
                running_op_fixture("Deep Residual Learning", &["PDF取得", "図抽出", "翻訳"], 2),
                running_op_fixture("Attention Paper", &["PDF取得", "引用抽出"], 1),
            ],
            ..App::default()
        };
        terminal.draw(|frame| render_tasks(frame, &app, &theme))?;
        let compact = compact_symbols(&buffer_symbols(terminal.backend().buffer()));
        assert!(compact.contains("DeepResidualLearning"));
        assert!(compact.contains("翻訳"));
        assert!(compact.contains("66%"));
        assert!(compact.contains("AttentionPaper"));
        assert!(compact.contains("引用抽出"));
        assert!(compact.contains("50%"));
        Ok(())
    }

    #[test]
    fn tasks_screen_shows_empty_state() -> Result<(), Box<dyn std::error::Error>> {
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend)?;
        let theme = Theme::load("nord")?;
        let app = App {
            mode: AppMode::Tasks,
            ..App::default()
        };
        terminal.draw(|frame| render_tasks(frame, &app, &theme))?;
        let compact = compact_symbols(&buffer_symbols(terminal.backend().buffer()));
        assert!(compact.contains("実行中の処理はありません"));
        Ok(())
    }

    #[test]
    fn help_reference_documents_the_tasks_key() {
        let has_tasks_key = keyboard_reference()
            .iter()
            .flat_map(|section| section.entries)
            .any(|(key, _)| *key == "shift+L");
        assert!(has_tasks_key);
    }

    #[test]
    fn help_reference_documents_the_project_file_keys() {
        let keys: Vec<&str> = keyboard_reference()
            .iter()
            .flat_map(|section| section.entries)
            .map(|(key, _)| *key)
            .collect();
        assert!(keys.contains(&"M"), "M（mind.md）がヘルプに無い");
        assert!(keys.contains(&"W"), "W（state.md）がヘルプに無い");
        assert!(keys.contains(&"J"), "J（研究日誌）がヘルプに無い");
    }

    #[test]
    fn project_page_shows_state_handoff_and_reviews() -> Result<(), Box<dyn std::error::Error>> {
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend)?;
        let theme = Theme::load("nord")?;
        let mut app = App {
            page: Page::Project,
            content_focused: true,
            project: ProjectView {
                state: papr_core::project_files::ProjectState {
                    goal: Some("てんかんの発作前兆".into()),
                    stage: Some("探索".into()),
                    current_task: Some("公開データの比較".into()),
                    next_action: None,
                    updated: Some("2026-09-24".into()),
                },
                state_exists: true,
                latest_handoff: Some(papr_core::brief::HandoffEntry {
                    run_id: 41,
                    block_id: "paper-library-qa".into(),
                    tag: None,
                    file: std::path::PathBuf::from("/tmp/proj/.sai/outputs/*/41/report.md"),
                    modified: std::time::SystemTime::UNIX_EPOCH,
                    raw: "- 決めたこと: 対象を絞る\n- 現況の更新案: 目標を書き直す".into(),
                    items: vec![
                        ("決めたこと".into(), "対象を絞る".into()),
                        ("現況の更新案".into(), "目標を書き直す".into()),
                    ],
                    readable: true,
                }),
                recent_reviews: vec![papr_core::brief::ReviewEntry {
                    run_id: 41,
                    block_id: "paper-library-qa".into(),
                    file: std::path::PathBuf::from("/tmp/proj/.sai/outputs/*/41/review.md"),
                    modified: std::time::SystemTime::UNIX_EPOCH,
                    body: "### 疑問に思った点\nデータの選び方".into(),
                }],
                understanding: Vec::new(),
                read_next: Vec::new(),
                read_next_selected: 0,
                citation_issues: papr_core::IssueCounts::default(),
                report_count: 0,
                attention: Vec::new(),
                scroll: 0,
            },
            ..App::default()
        };
        terminal.draw(|frame| render(frame, &mut app, &theme))?;
        let compact = compact_symbols(&buffer_symbols(terminal.backend().buffer()));
        assert!(compact.contains("てんかんの発作前兆"), "目標が出ていない");
        assert!(compact.contains("探索"), "段階が出ていない");
        assert!(compact.contains("未記入（Wで書く）"), "未記入の表示が出ていない");
        assert!(compact.contains("最新の引き継ぎ"), "引き継ぎの見出しが無い");
        assert!(compact.contains("現況の更新案あり"), "更新案ありの表示が出ていない");
        assert!(compact.contains("決めたこと"), "引き継ぎの本文が出ていない");
        assert!(compact.contains("最近のレビュー"), "レビューの見出しが無い");
        assert!(compact.contains("疑問に思った点"), "レビューの本文が出ていない");
        assert!(compact.contains("READNEXT"), "READ NEXT の見出しが無い");
        assert!(compact.contains("まだありません"), "READ NEXT の空表示が出ていない");
        Ok(())
    }

    /// S02: Project ページの「要対応 N 件（A で一覧）」と、一覧画面。
    #[test]
    fn project_shows_attention_line_and_list() -> Result<(), Box<dyn std::error::Error>> {
        let item = |key: &str, kind, title: &str| papr_core::attention::AttentionItem {
            key: key.into(),
            kind,
            title: title.into(),
            run_id: Some(41),
            open_path: Some(std::path::PathBuf::from("/tmp/x")),
            report_id: None,
        };
        let attention = vec![
            item(
                "review:41",
                papr_core::attention::AttentionKind::EmptyReview,
                "run 41 の review が未記入",
            ),
            item(
                "card:42",
                papr_core::attention::AttentionKind::RecommendCard,
                "推薦のカード: 候補を絞る",
            ),
        ];
        let theme = Theme::load("nord")?;
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend)?;

        // Project ページの一番上に件数と案内が出る。
        let mut app = App {
            page: Page::Project,
            content_focused: true,
            project: ProjectView {
                attention: attention.clone(),
                ..ProjectView::default()
            },
            ..App::default()
        };
        terminal.draw(|frame| render(frame, &mut app, &theme))?;
        let compact = compact_symbols(&buffer_symbols(terminal.backend().buffer()));
        assert!(compact.contains("要対応2件"), "{compact}");
        assert!(compact.contains("Aで一覧"), "{compact}");

        // 一覧画面に印と題が出る。
        let mut app = App {
            page: Page::Project,
            mode: AppMode::AttentionList,
            attention_selected: 1,
            project: ProjectView {
                attention,
                ..ProjectView::default()
            },
            ..App::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(100, 40))?;
        terminal.draw(|frame| render(frame, &mut app, &theme))?;
        let compact = compact_symbols(&buffer_symbols(terminal.backend().buffer()));
        assert!(compact.contains("[review]"), "{compact}");
        assert!(compact.contains("[推薦]"), "{compact}");
        assert!(compact.contains("候補を絞る"), "{compact}");
        Ok(())
    }

    /// READ NEXT の一覧が行と選択の印（`>`）つきで出る。
    #[test]
    fn project_read_next_shows_rows_and_selection() -> Result<(), Box<dyn std::error::Error>> {
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend)?;
        let theme = Theme::load("nord")?;
        let mut app = App {
            page: Page::Project,
            content_focused: true,
            project: ProjectView {
                read_next: vec![
                    papr_core::progress::ReadNext {
                        paper_id: 37,
                        title: "Large Language Model".into(),
                        score: 5,
                        reason: "引き継ぎで 3 回・note なし".into(),
                    },
                    papr_core::progress::ReadNext {
                        paper_id: 12,
                        title: "Signal Processing".into(),
                        score: 2,
                        reason: "残した候補・未読".into(),
                    },
                ],
                read_next_selected: 1,
                ..ProjectView::default()
            },
            ..App::default()
        };
        terminal.draw(|frame| render(frame, &mut app, &theme))?;
        let compact = compact_symbols(&buffer_symbols(terminal.backend().buffer()));
        assert!(compact.contains("READNEXT"), "READ NEXT の見出しが無い");
        assert!(compact.contains("#37"), "1 本目が出ていない: {compact}");
        assert!(compact.contains("LargeLanguageModel"), "タイトルが出ていない");
        assert!(compact.contains("#12"), "2 本目が出ていない");
        assert!(compact.contains("SignalProcessing"), "2 本目のタイトルが出ていない");
        assert!(compact.contains(">"), "選択中の印が無い");
        Ok(())
    }

    /// 理解の 1 行の値（あいまい最大 3 つ・未確認の件数・記録なし）。
    #[test]
    fn project_understanding_line_shows_vague_unknown_and_empty() {
        let theme = Theme::load("nord").unwrap();
        let entry =
            |id: i64, topic: &str, state: &str| papr_core::understanding::UnderstandingEntry {
                id,
                topic: topic.to_string(),
                state: state.to_string(),
                evidence: String::new(),
                run_id: None,
                created_at: "2026-09-25 12:00:00".to_string(),
            };
        let line_text = |line: ratatui::text::Line<'static>| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };

        // 記録が無いとき。
        let text = line_text(super::project_understanding_line(&[], &theme));
        assert!(text.contains("理解"), "{text}");
        assert!(text.contains("まだ記録がありません"), "{text}");

        // あいまい 4 つ → 3 つまで。未確認は件数だけ。ok は出さない。
        let entries = vec![
            entry(1, "A", "vague"),
            entry(2, "B", "vague"),
            entry(3, "C", "vague"),
            entry(4, "D", "vague"),
            entry(5, "E", "unknown"),
            entry(6, "F", "unknown"),
            entry(7, "G", "ok"),
        ];
        let text = line_text(super::project_understanding_line(&entries, &theme));
        assert!(text.contains("あいまい: A / B / C"), "{text}");
        assert!(!text.contains("D"), "4 つ目は出さない: {text}");
        assert!(text.contains("未確認: 2 件"), "{text}");
        assert!(!text.contains("G"), "ok は出さない: {text}");

        // ok だけのとき。
        let text = line_text(super::project_understanding_line(
            &[entry(8, "H", "ok")],
            &theme,
        ));
        assert!(text.contains("理解しているトピックのみ"), "{text}");
    }

    /// テスト用の report（`id` と `created_at` だけ使う）。
    fn test_report(id: &str, block: &str, created_at: &str) -> papr_core::Report {
        papr_core::Report {
            id: id.into(),
            block: Some(block.into()),
            created_at: created_at.into(),
            ..papr_core::Report::default()
        }
    }

    #[test]
    fn report_rows_cover_empty_two_and_more_than_five() {
        use papr_core::IssueCounts;

        // 0 本
        assert_eq!(
            super::report_rows(&[], 0, Some(7)),
            vec!["（まだありません）".to_string()]
        );

        // 2 本: id・block・日付・件数が出る（問題なしも）
        let rows = super::report_rows(
            &[
                (
                    test_report("R-0007", "paper-critic", "2026-09-24T21:10:00+00:00"),
                    IssueCounts {
                        errors: 1,
                        warnings: 2,
                    },
                ),
                (
                    test_report("R-0005", "paper-map", "2026-09-20T00:00:00+00:00"),
                    IssueCounts::default(),
                ),
            ],
            2,
            Some(7),
        );
        assert_eq!(rows.len(), 2);
        assert!(rows[0].contains("R-0007"), "{rows:?}");
        assert!(rows[0].contains("paper-critic"));
        assert!(rows[0].contains("09/24"));
        assert!(rows[0].contains("要確認 1・注意 2"));
        assert!(rows[1].contains("R-0005"));
        assert!(rows[1].contains("問題なし"));

        // 6 本以上: 5 本だけ出し、残りを案内する
        let five: Vec<(papr_core::Report, IssueCounts)> = (0..5)
            .map(|i| {
                (
                    test_report(
                        &format!("R-{:04}", i + 1),
                        "paper-critic",
                        "2026-09-24T21:10:00+00:00",
                    ),
                    IssueCounts::default(),
                )
            })
            .collect();
        let rows = super::report_rows(&five, 6, Some(42));
        assert_eq!(rows.len(), 6);
        assert!(rows[5].contains("…ほか 1 本"), "{rows:?}");
        assert!(rows[5].contains("--paper 42"));
    }

    /// 撤回の印のテスト用に最小の `LibraryPaper` を作る。
    fn retraction_paper(is_retracted: Option<bool>) -> LibraryPaper {
        LibraryPaper {
            id: 12,
            title: "Retracted work".into(),
            authors: String::new(),
            doi: None,
            arxiv_id: None,
            pdf_path: None,
            file_size: None,
            reading_status: "unread".into(),
            is_favorite: false,
            abstract_text: None,
            journal: None,
            published_at: None,
            abstract_ja: None,
            abstract_ja_translated_at: None,
            citation_count: None,
            is_retracted,
            retraction_checked_at: Some("2026-09-25T12:00:00+00:00".into()),
            updated_at: None,
        }
    }

    #[test]
    fn retracted_paper_detail_shows_warning_line() {
        let theme = Theme::load("nord").unwrap();
        let data_dir = std::path::Path::new("/tmp");
        let paper = retraction_paper(Some(true));
        let lines = super::library_paper_detail_lines(&paper, &theme, data_dir, &[], 0, Some(12));
        let warning = lines
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains("撤回されています"))
            })
            .expect("撤回の印が無い");
        assert_eq!(
            warning.spans[0].content,
            "⚠ この論文は撤回されています（OpenAlex, 2026-09-25）"
        );
        assert_eq!(warning.style.fg, Some(theme.error));

        // 撤回されていない（Some(false)）・未確認（None）のときは出ない。
        for state in [Some(false), None] {
            let paper = retraction_paper(state);
            let lines =
                super::library_paper_detail_lines(&paper, &theme, data_dir, &[], 0, Some(12));
            assert!(
                !lines.iter().any(|line| line
                    .spans
                    .iter()
                    .any(|span| span.content.contains("撤回されています"))),
                "{state:?} で印が出ている"
            );
        }
    }

    #[test]
    fn library_list_marks_retracted_paper() -> Result<(), Box<dyn std::error::Error>> {
        let theme = Theme::load("nord")?;
        let mut app = App {
            page: Page::Library,
            content_focused: true,
            library: LibraryState {
                papers: vec![retraction_paper(Some(true))],
                ..LibraryState::default()
            },
            ..App::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(100, 20))?;
        terminal.draw(|frame| super::render(frame, &mut app, &theme))?;
        let compact: String = terminal
            .backend()
            .to_string()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(compact.contains("[撤回]"), "撤回の印が無い: {compact}");

        // 撤回されていないときは出ない。
        app.library.papers[0].is_retracted = Some(false);
        terminal.draw(|frame| super::render(frame, &mut app, &theme))?;
        let text = terminal.backend().to_string();
        assert!(!text.contains("[撤回]"), "印が残っている: {text}");
        Ok(())
    }


    /// 検索結果が 1 件以上あるとき、Database 欄の文字が絞り込み欄に飲まれて消えない（AC1）。
    #[test]
    fn discover_db_field_visible_with_results() -> Result<(), Box<dyn std::error::Error>> {
        let mut terminal = Terminal::new(TestBackend::new(100, 40))?;
        let theme = Theme::load("nord")?;
        let mut app = App {
            mode: AppMode::Search,
            discovery: crate::state::DiscoveryState {
                db: "arxiv".to_owned(),
                db_cursor: 5,
                query: "multimodal medical LLM".to_owned(),
                status: DiscoveryStatus::Ready,
                results: vec![test_remote_paper("2401.00001", "A Paper")],
                ..Default::default()
            },
            ..Default::default()
        };
        app.discovery.rebuild_filter();

        terminal.draw(|frame| {
            let area = frame.area();
            super::render_discover(frame, area, &mut app, &theme);
        })?;
        let text: String = buffer_symbols(terminal.backend().buffer());
        assert!(text.contains("arxiv"), "Database 欄が結果で消えた: {text}");
        // 絞り込み欄は Database 欄の下に出る（重なっていない）。
        assert!(
            text.contains("Filter displayed results"),
            "絞り込み欄が出ていない: {text}"
        );
        Ok(())
    }

    /// 検索の見出しは、実際に使った DB 名を出す（AC2）。
    #[test]
    fn discover_error_heading_names_actual_providers() -> Result<(), Box<dyn std::error::Error>> {
        let mut terminal = Terminal::new(TestBackend::new(100, 40))?;
        let theme = Theme::load("nord")?;
        let mut app = App {
            mode: AppMode::Search,
            discovery: crate::state::DiscoveryState {
                active_providers: vec!["pubmed".to_owned()],
                status: DiscoveryStatus::Error("ネットワークエラー".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        };

        terminal.draw(|frame| {
            let area = frame.area();
            super::render_discover(frame, area, &mut app, &theme);
        })?;
        let text: String = buffer_symbols(terminal.backend().buffer());
        assert!(
            text.contains("Search failed (pubmed)"),
            "DB 名が固定の S2 のまま: {text}"
        );
        assert!(
            !text.contains("Search failed (S2)"),
            "従来の固定の文が残っている: {text}"
        );

        // 読み込み中の文にも実際の DB 名を出す。
        app.discovery.status = DiscoveryStatus::Loading;
        terminal.draw(|frame| {
            let area = frame.area();
            super::render_discover(frame, area, &mut app, &theme);
        })?;
        let text: String = buffer_symbols(terminal.backend().buffer());
        assert!(
            text.contains("Searching pubmed..."),
            "読み込み中の文に DB 名が出ない: {text}"
        );
        Ok(())
    }

    /// Discover などから開いた PAPER 画面でも、描画後に内容行数と表示行数が入る
    /// （入らないと `j` / ホイール / PageDown が途中で止まる）。
    #[test]
    fn remote_paper_detail_records_content_and_view_rows() -> Result<(), Box<dyn std::error::Error>>
    {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend)?;
        let theme = Theme::load("nord")?;
        let mut paper = test_remote_paper("2401.00001", "A Paper");
        // 長い要旨で、本文が画面より長く的情形を作る。
        paper.abstract_text = (0..40)
            .map(|i| format!("sentence{i} of the abstract "))
            .collect::<String>();
        let mut app = App {
            page: Page::Discover,
            paper_detail_remote: Some(paper),
            ..App::default()
        };

        terminal.draw(|frame| super::render_paper_detail(frame, &mut app, &theme))?;
        // 縦 margin 1 + 見出し 2 + 案内 1 を引いた分が表示行数。
        assert_eq!(app.paper_detail_view_rows, 15);
        let content_rows = app.paper_detail_content_rows;
        assert!(
            content_rows > usize::from(app.paper_detail_view_rows),
            "要旨が長いのにスクロールできる行数が無い: {content_rows}"
        );

        // 描画された行数が実際の末尾（RESOURCES のあたり）まで届く。
        app.paper_detail_scroll =
            u16::try_from(content_rows - usize::from(app.paper_detail_view_rows)).unwrap_or(0);
        terminal.draw(|frame| super::render_paper_detail(frame, &mut app, &theme))?;
        let text = buffer_symbols(terminal.backend().buffer());
        assert!(
            text.contains("RESOURCES"),
            "末尾までスクロールできない: {text}"
        );
        Ok(())
    }

    /// 描画テスト用のRemotePaper 1 件。
    fn test_remote_paper(id: &str, title: &str) -> papr_core::models::RemotePaper {
        papr_core::models::RemotePaper {
            id: id.to_owned(),
            title: title.to_owned(),
            authors: vec!["A. Author".to_owned()],
            abstract_text: String::new(),
            published: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            categories: Vec::new(),
            pdf_url: None,
            doi: None,
            journal_ref: None,
            citation_count: Some(12),
        }
    }

    /// U23: 研究方針のページに、種の文・関門・項目の一覧が出る。
    #[test]
    fn tree_page_renders_seed_gate_and_rows() -> Result<(), Box<dyn std::error::Error>> {
        use papr_core::sci::compute::{NodeRow, SeedPart, SeedSentence, Status};

        let mut terminal = Terminal::new(TestBackend::new(120, 40))?;
        let theme = Theme::load("nord")?;
        let mut app = App {
            page: Page::Tree,
            content_focused: true,
            ..App::default()
        };
        let mut status = Status {
            initialized: true,
            seed: Some(SeedSentence {
                text: "頭皮脳波で［？小目的］を行う".into(),
                parts: vec![SeedPart {
                    text: "頭皮脳波で".into(),
                    filled: true,
                    ..Default::default()
                }],
            }),
            ..Default::default()
        };
        status.gate = Some(papr_core::sci::compute::GateState {
            current: 2,
            total: 7,
            name: "小目的".into(),
            ..Default::default()
        });
        status.nodes = vec![
            NodeRow {
                id: "N1".into(),
                title: "出発点".into(),
                state: "decided".into(),
                depth: 0,
                ..Default::default()
            },
            NodeRow {
                id: "N12".into(),
                title: "小目的".into(),
                state: "open".into(),
                blank: Some("awaiting_decision".into()),
                depth: 1,
                ..Default::default()
            },
        ];
        app.tree.status = Some(status);
        app.tree.selected = 1;

        terminal.draw(|frame| {
            let area = frame.area();
            crate::tree_view::render_tree(frame, area, &mut app, &theme);
        })?;
        let text = buffer_symbols(terminal.backend().buffer());
        let compact = compact_symbols(&text);
        assert!(compact.contains("頭皮脳波で"), "種の文が出ていない: {compact}");
        assert!(compact.contains("関門"), "関門が出ていない: {compact}");
        assert!(compact.contains("2/7"), "関門の数が出ていない: {compact}");
        assert!(compact.contains("負債"), "負債が出ていない: {compact}");
        assert!(text.contains("N12"), "項目の一覧が出ていない: {text}");
        assert!(compact.contains("判断待ち"), "空欄の種類が出ていない: {compact}");
        Ok(())
    }

    /// U23: 木が無いプロジェクトでは案内の文が出る。
    #[test]
    fn tree_page_shows_init_hint_without_tree() -> Result<(), Box<dyn std::error::Error>> {
        let mut terminal = Terminal::new(TestBackend::new(120, 40))?;
        let theme = Theme::load("nord")?;
        let mut app = App {
            page: Page::Tree,
            content_focused: true,
            ..App::default()
        };
        terminal.draw(|frame| {
            let area = frame.area();
            crate::tree_view::render_tree(frame, area, &mut app, &theme);
        })?;
        let text = buffer_symbols(terminal.backend().buffer());
        let compact = compact_symbols(&text);
        assert!(
            compact.contains("研究方針の木はまだありません"),
            "案内の文が出ていない: {compact}"
        );
        assert!(compact.contains("sai-moon-papersciinit"), "型の案内がない");
        Ok(())
    }
}
