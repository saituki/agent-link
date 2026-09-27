//! sai-paper-tui — citation list TUI companion for papr (herdr pane)
//! Usage: sai-paper-tui citation <paperId>  |  sai-paper-tui md <paperId>
//! UI is deliberately papr-identical: Header + List, PAPER detail + CITED IN.
//!
//! データは papr.db（docling 引用抽出結果）のみから読む。
//! S2 フォールバックは行わない（docling 抽出済み論文が前提）。

use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use sai_paper_core::{Database, LibraryMatchEntry, RemotePaper, S2Client};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[path = "../../sai-paper-tui/src/theme.rs"]
mod theme;
use theme::Theme;

// ---------- CLI ----------
#[derive(Parser, Debug)]
#[command(
    name = "sai-moon-paper-cite",
    version,
    about = "sai-paper citation TUI (papr companion)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Direct paper id (shorthand for `citation <id>`)
    paper_id: Option<String>,
    /// Extra args (handles unquoted title fallback from papr s key)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    extra: Vec<String>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Show citation list for a paper
    Citation {
        paper_id: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        extra: Vec<String>,
    },
}

// ---------- Data models (mirrors core DB) ----------
#[derive(Debug, Clone)]
struct PaperMeta {
    paper_id: String,
    title: String,
    year: Option<i32>,
    venue: Option<String>,
    authors: Vec<String>,
    abstract_text: Option<String>,
    doi: Option<String>,
    arxiv_id: Option<String>,
    pdf_path: Option<String>,
}

#[derive(Debug, Clone)]
struct RefRow {
    ref_no: i32,
    first_author: Option<String>,
    year: Option<i32>,
    journal: Option<String>,
    title: Option<String>,
    doi: Option<String>,
    raw_text: String,
    s2_paper_id: Option<String>,
    s2_title: Option<String>,
    s2_year: Option<i32>,
}

#[derive(Debug, Clone)]
struct SentenceRow {
    sent_no: i32,
    text: String,
    cite_refs: Vec<i32>,
}

// ---------- Helpers: core integration (papr.db + S2) ----------
/// papr.db へのアクセス。開けない場合は S2 フォールバックのみで動作する。
struct Store {
    db_path: PathBuf,
    /// db_path の親（data ディレクトリ。docling Markdown 出力先の解決に使用）
    data_dir: PathBuf,
    db: Option<Database>,
}

impl Store {
    fn open() -> Self {
        // sai_paper_core::Paths::discover() がプロジェクトルートの data/papr.db を解決し、
        // Database::open がマイグレーション（0016 引用テーブル含む）まで適用する
        let (db, db_path) = match sai_paper_core::Paths::discover() {
            Ok(paths) => (
                Database::open(&paths.database_file).ok(),
                paths.database_file,
            ),
            Err(_) => (None, PathBuf::from("data/papr.db")),
        };
        let data_dir = db_path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        Self {
            db_path,
            data_dir,
            db,
        }
    }

    /// 抽出キーで論文メタデータを取得する（papr.db の papers を照合）。
    fn load_meta(&self, pid: &str) -> Option<PaperMeta> {
        let db = self.db.as_ref()?;
        let conn = db.connection();
        let m = sai_paper_core::citation::db::get_paper_meta_by_extract_key(conn, pid).ok()??;
        self.to_meta(db, m)
    }

    /// Library の番号（`papers.id`）で論文メタデータを取得する（段階 2c）。
    fn load_meta_by_papers_id(&self, papers_id: i64) -> Option<PaperMeta> {
        let db = self.db.as_ref()?;
        let conn = db.connection();
        let m =
            sai_paper_core::citation::db::get_paper_meta_by_papers_id(conn, papers_id).ok()??;
        self.to_meta(db, m)
    }

    fn to_meta(
        &self,
        db: &Database,
        m: sai_paper_core::citation::db::PaperMeta,
    ) -> Option<PaperMeta> {
        Some(PaperMeta {
            paper_id: m.paper_id,
            title: m.title,
            year: m.year.map(|y| y as i32),
            venue: m.venue,
            authors: m.authors,
            abstract_text: m.abstract_text,
            doi: m.doi,
            arxiv_id: m.arxiv_id,
            // DB 保存形（相対）を実ファイルパスへ直す（段階 2b）。
            pdf_path: m
                .pdf_path
                .map(|stored| db.resolve_path(&stored).to_string_lossy().into_owned()),
        })
    }

    /// 引用文献リストを取得する（ref_no 昇順）。
    ///
    /// 記録はまず `papers_id` で、無ければ抽出キーで探す（段階 2c）。
    fn load_refs(&self, papers_id: Option<i64>, pid: &str) -> Vec<RefRow> {
        let Some(db) = self.db.as_ref() else {
            return vec![];
        };
        match sai_paper_core::citation::db::get_refs_for(db.connection(), papers_id, Some(pid)) {
            Ok(rows) => rows
                .into_iter()
                .map(|r| RefRow {
                    ref_no: r.ref_no as i32,
                    first_author: r.first_author,
                    year: r.year.map(|y| y as i32),
                    journal: r.journal,
                    title: r.title,
                    doi: r.doi,
                    raw_text: r.raw_text,
                    s2_paper_id: r.s2_paper_id,
                    s2_title: r.s2_title,
                    s2_year: r.s2_year.map(|y| y as i32),
                })
                .collect(),
            Err(_) => vec![],
        }
    }

    /// 本文中の引用文を取得する（sent_no 昇順）。
    ///
    /// 記録はまず `papers_id` で、無ければ抽出キーで探す（段階 2c）。
    fn load_sentences(&self, papers_id: Option<i64>, pid: &str) -> Vec<SentenceRow> {
        let Some(db) = self.db.as_ref() else {
            return vec![];
        };
        match sai_paper_core::citation::db::get_sentences_for(db.connection(), papers_id, Some(pid))
        {
            Ok(rows) => rows
                .into_iter()
                .map(|s| SentenceRow {
                    sent_no: s.sent_no as i32,
                    text: s.text,
                    cite_refs: serde_json::from_str(&s.cite_refs).unwrap_or_default(),
                })
                .collect(),
            Err(_) => vec![],
        }
    }
}

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

/// docling Markdown の保存先パス（<db_dir>/extracts/<正規化キー>.md）。
/// プロジェクトルートからの相対パス（`data/xxx`）で返す。
fn docling_md_path(paper_id: &str, data_dir: &std::path::Path) -> String {
    let key = sai_paper_core::citation::normalize_paper_id(paper_id);
    data_dir
        .join("extracts")
        .join(format!(
            "{}.md",
            sai_paper_core::citation::docling::sanitize_filename_component(&key)
        ))
        .display()
        .to_string()
}

/// プロジェクトルートからの相対パスで表示する（`data/xxx`、`papers/xxx`）。
fn shorten_path(path: &str, data_dir: &std::path::Path) -> String {
    let p = std::path::Path::new(path);
    if let Some(root) = data_dir.parent()
        && let Ok(rel) = p.strip_prefix(root)
    {
        return rel.display().to_string();
    }
    if let Ok(rel) = p.strip_prefix(data_dir) {
        return format!("data/{}", rel.display());
    }
    path.to_string()
}

/// 同梱の sai-paper CLI バイナリを探す（exe と同じ階層 → PATH）。
fn sai_paper_cli_bin() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for name in ["sai-moon-paper", "sai-moon-paper.exe"] {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    if let Ok(out) = Command::new("which").arg("sai-moon-paper").output() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return Some(PathBuf::from(s));
        }
    }
    None
}

// ---------- App state ----------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    List,
    Paper,
}

/// 下部 LOG ペインに表示する最大行数
const LOG_VISIBLE_LINES: usize = 6;
/// ログリングバッファの上限（古い行から捨てる）
const LOG_BUFFER_LINES: usize = 500;
/// Esc 誤判定（分離 ESC[B 断片）検出のための追加入力待ち窓。
/// キーボードオートリピートの最小間隔（約 33ms）より長く取る。
const ESC_SEQUENCE_WINDOW: Duration = Duration::from_millis(50);

struct App {
    paper_id: String,
    data_dir: PathBuf,
    meta: Option<PaperMeta>,
    refs: Vec<RefRow>,
    sentences: Vec<SentenceRow>,
    selected: usize,
    paper_scroll: u16,
    mode: Mode,
    theme: Theme,
    status: String,
    filter: String,
    /// バックグラウンド処理（get-pdf 等）のログ。古い順に保持
    logs: Vec<String>,
    /// ログ受信チャネル（バックグラウンドタスク → TUI ループ）
    log_rx: Option<UnboundedReceiver<String>>,
    /// papr.db（R5: Library 照合・手動取り込みに使用）。
    db: Option<Database>,
    /// papr.db の papers 一覧（arxiv_id / doi / s2_paper_id / title / pdf_path）。
    lib_entries: Vec<LibraryMatchEntry>,
    /// 手動 PDF 取り込みモーダル（R5: `i`）。None なら非表示。
    import: Option<ManualImportState>,
}

/// 手動 PDF 取り込みモーダルの状態（R5）。
struct ManualImportState {
    /// 候補 PDF の絶対パス（新しい順）。
    candidates: Vec<PathBuf>,
    /// カーソル位置。
    selected: usize,
}

impl App {
    /// 選択中の引用文献に対応する論文が Library に存在し、
    /// ローカル PDF が実在するかどうか（R5: 緑色表示の判定基準）。
    ///
    /// 照合方法は Discover と同一（s2_paper_id / DOI / タイトル一致）。
    fn in_library(&self, r: &RefRow) -> bool {
        let ref_title = r
            .title
            .as_deref()
            .or(r.s2_title.as_deref())
            .map(|t| t.trim().to_lowercase());
        let ref_doi = r.doi.as_deref().map(str::trim);
        self.lib_entries.iter().any(|entry| {
            // 判定基準は「ローカル PDF が実際に存在する論文」（Discover と同じ）
            if !entry
                .pdf_path
                .as_deref()
                .is_some_and(|path| std::path::Path::new(path).is_file())
            {
                return false;
            }
            if let (Some(s1), Some(s2_id)) = (&entry.s2_paper_id, &r.s2_paper_id)
                && s1 == s2_id
            {
                return true;
            }
            if let (Some(d1), Some(d2)) = (&entry.doi, ref_doi)
                && d1.eq_ignore_ascii_case(d2)
            {
                return true;
            }
            if let Some(t2) = ref_title.as_deref()
                && entry.title.trim().to_lowercase() == t2
            {
                return true;
            }
            false
        })
    }

    /// R5: papr.db の papers から Library 照合情報を再読込する。
    ///
    /// ダウンロード / 手動取り込み後に呼び、行の緑色表示を最新化する。
    fn refresh_lib_entries(&mut self) {
        if let Some(db) = &self.db {
            self.lib_entries = db.library_match_entries().unwrap_or_default();
        }
    }

    /// チャネルから到着済みのログ行をすべて取り込む（TUI ループの各反復で呼ぶ）。
    fn drain_logs(&mut self) {
        let mut completed = false;
        if let Some(rx) = self.log_rx.as_mut() {
            while let Ok(line) = rx.try_recv() {
                // R5: get-pdf 完了後に Library 照合（緑色表示）を最新化する
                if line.contains("[get-pdf] 完了") {
                    completed = true;
                }
                self.logs.push(line);
            }
        }
        if completed {
            self.refresh_lib_entries();
        }
        if self.logs.len() > LOG_BUFFER_LINES {
            let drop = self.logs.len() - LOG_BUFFER_LINES;
            self.logs.drain(0..drop);
        }
    }
}

impl App {
    fn filtered_indices(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.refs.len()).collect();
        }
        let f = self.filter.to_lowercase();
        self.refs
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                let t = r
                    .title
                    .as_deref()
                    .or(r.s2_title.as_deref())
                    .unwrap_or(&r.raw_text)
                    .to_lowercase();
                let a = r.first_author.as_deref().unwrap_or("").to_lowercase();
                let y = r.year.map(|y| y.to_string()).unwrap_or_default();
                t.contains(&f)
                    || a.contains(&f)
                    || y.contains(&f)
                    || r.raw_text.to_lowercase().contains(&f)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

// ---------- UI rendering (papr-identical PAPER) ----------
fn paper_detail_lines<'a>(
    meta: &'a PaperMeta,
    theme: &Theme,
    sentences_for_ref: &[&SentenceRow],
    data_dir: &std::path::Path,
) -> Vec<Line<'a>> {
    let title = meta.title.clone();
    let authors = if meta.authors.is_empty() {
        "Not available".to_string()
    } else {
        meta.authors.join(", ")
    };
    let year = meta
        .year
        .map(|y| y.to_string())
        .unwrap_or_else(|| "Not available".to_string());
    let venue = meta
        .venue
        .clone()
        .unwrap_or_else(|| "Not available".to_string());
    let doi = meta
        .doi
        .clone()
        .unwrap_or_else(|| "Not available".to_string());
    let abs = meta
        .abstract_text
        .clone()
        .unwrap_or_else(|| "Not available".to_string());
    let pdf = meta
        .pdf_path
        .clone()
        .unwrap_or_else(|| "Not available".to_string());
    let mut lines = vec![
        Line::styled(
            title,
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::styled(authors, Style::default().fg(theme.secondary)),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Year       ", Style::default().fg(theme.muted)),
            Span::styled(year, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled("Venue      ", Style::default().fg(theme.muted)),
            Span::styled(venue, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled("DOI        ", Style::default().fg(theme.muted)),
            Span::styled(doi, Style::default().fg(theme.accent)),
        ]),
        Line::from(vec![
            Span::styled("URL        ", Style::default().fg(theme.muted)),
            Span::styled(
                paper_site_url(
                    meta.doi.as_deref(),
                    meta.arxiv_id.as_deref(),
                    &meta.paper_id,
                ),
                Style::default().fg(theme.accent),
            ),
        ]),
        Line::from(vec![
            Span::styled("PDF        ", Style::default().fg(theme.muted)),
            Span::styled(pdf.clone(), Style::default().fg(theme.accent)),
        ]),
        Line::raw(""),
        Line::styled(
            "ABSTRACT",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::styled(abs, Style::default().fg(theme.text)),
        Line::raw(""),
        Line::styled(
            "RESOURCES",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(format!("PDF    {}", pdf), Style::default().fg(theme.accent)),
        Line::styled(
            format!(
                "docling  {}",
                shorten_path(&docling_md_path(&meta.paper_id, data_dir), data_dir)
            ),
            Style::default().fg(theme.accent),
        ),
        Line::raw(""),
    ];
    // --- CITED IN (sai-paper-tui exclusive) ---
    lines.push(Line::styled(
        "─ CITED IN (文中の対応箇所) ─",
        Style::default()
            .fg(theme.muted)
            .add_modifier(Modifier::BOLD),
    ));
    if sentences_for_ref.is_empty() {
        lines.push(Line::styled(
            "No in-text citations found (docling extract未実行)",
            Style::default().fg(theme.muted),
        ));
    } else {
        for s in sentences_for_ref {
            let txt = s.text.chars().take(300).collect::<String>();
            lines.push(Line::from(vec![
                Span::styled(
                    format!("[{}] ", s.sent_no),
                    Style::default().fg(theme.warning),
                ),
                Span::styled(txt, Style::default().fg(theme.text)),
            ]));
        }
    }
    lines
}

fn render_app(frame: &mut Frame, app: &mut App) {
    let theme = &app.theme;
    let area = frame.area();
    // ログ行数に応じて下部 LOG ペインの高さを決める（未使用時は 0）
    let log_h = if app.logs.is_empty() {
        0
    } else {
        (app.logs.len().min(LOG_VISIBLE_LINES) as u16).saturating_add(2) // 枠線分
    };
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(log_h),
        Constraint::Length(1),
    ])
    .split(area);
    let title = app
        .meta
        .as_ref()
        .map(|m| m.title.clone())
        .unwrap_or_else(|| app.paper_id.clone());
    let header = Paragraph::new(vec![
        Line::styled(
            format!(" sai-paper-tui — {}", title),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            format!(
                " {} citations  |  {} / {}  |  filter: {}  |  ENTER:PAPER d:download p:pdf q:quit",
                app.refs.len(),
                app.selected + 1,
                app.refs.len().max(1),
                if app.filter.is_empty() {
                    "(none)"
                } else {
                    &app.filter
                }
            ),
            Style::default().fg(theme.muted),
        ),
        Line::styled(
            "─".repeat(area.width as usize),
            Style::default().fg(theme.border),
        ),
    ])
    .block(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme.background)),
    );
    frame.render_widget(header, chunks[0]);

    if app.mode == Mode::Paper {
        if let Some(meta_ref) = app.refs.get(app.selected) {
            // Build a meta for the selected reference as a pseudo paper
            let ref_meta = PaperMeta {
                paper_id: meta_ref
                    .s2_paper_id
                    .clone()
                    .unwrap_or_else(|| format!("ref:{}", meta_ref.ref_no)),
                title: meta_ref
                    .title
                    .clone()
                    .or(meta_ref.s2_title.clone())
                    .unwrap_or_else(|| meta_ref.raw_text.clone()),
                year: meta_ref.year.or(meta_ref.s2_year),
                venue: meta_ref.journal.clone(),
                authors: meta_ref
                    .first_author
                    .clone()
                    .map(|a| vec![a])
                    .unwrap_or_default(),
                abstract_text: Some(meta_ref.raw_text.clone()),
                doi: meta_ref.doi.clone(),
                arxiv_id: None,
                pdf_path: None,
            };
            let sents: Vec<&SentenceRow> = app
                .sentences
                .iter()
                .filter(|s| s.cite_refs.contains(&meta_ref.ref_no))
                .collect();
            let lines = paper_detail_lines(&ref_meta, theme, &sents, &app.data_dir);
            let _h = lines.len() as u16;
            let para = Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((app.paper_scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(theme.border))
                        .title(format!(
                            " PAPER [{}/{}] — ESC:戻る d:download p:pdf ",
                            app.selected + 1,
                            app.refs.len()
                        ))
                        .style(Style::default().bg(theme.background)),
                );
            frame.render_widget(para, chunks[1]);
        }
    } else {
        // List
        let indices = app.filtered_indices();
        let items: Vec<ListItem> = indices
            .iter()
            .enumerate()
            .map(|(vis_idx, &real_idx)| {
                let r = &app.refs[real_idx];
                let s2 = if r.s2_paper_id.is_some() {
                    "●"
                } else {
                    "○"
                };
                let y = r
                    .year
                    .or(r.s2_year)
                    .map(|y| y.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let t = r
                    .title
                    .as_deref()
                    .or(r.s2_title.as_deref())
                    .unwrap_or(&r.raw_text);
                let t短 = if t.chars().count() > 80 {
                    t.chars().take(80).collect::<String>() + "…"
                } else {
                    t.to_string()
                };
                let a = r.first_author.as_deref().unwrap_or("-");
                let is_sel = vis_idx == {
                    // map selected real index to vis position
                    let pos = indices.iter().position(|&x| x == app.selected).unwrap_or(0);
                    pos
                };
                let style = if is_sel {
                    Style::default()
                        .bg(theme.accent)
                        .fg(theme.background)
                        .add_modifier(Modifier::BOLD)
                } else if app.in_library(r) {
                    // R5: Library に既に存在する論文（ローカル PDF 実在）は緑色で表示する
                    Style::default().fg(theme.success)
                } else {
                    Style::default().fg(theme.text)
                };
                let line = Line::from(vec![
                    Span::styled(format!("{:3} {} ", r.ref_no, s2), style),
                    Span::styled(
                        format!("{:<12}", a.chars().take(12).collect::<String>()),
                        style,
                    ),
                    Span::styled(format!(" {:<5}", y), style),
                    Span::styled(t短, style),
                ]);
                ListItem::new(line)
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.border))
                    .title(" CITATIONS — j/k:移動 Enter:PAPER d:download p:pdf /:filter q:quit ")
                    .style(Style::default().bg(theme.background)),
            )
            .highlight_style(Style::default().bg(theme.accent).fg(theme.background));
        let mut state = ListState::default();
        let vis_pos = app
            .filtered_indices()
            .iter()
            .position(|&x| x == app.selected)
            .unwrap_or(0);
        state.select(Some(vis_pos));
        frame.render_stateful_widget(list, chunks[1], &mut state);
    }
    // R5: 手動 PDF 取り込みモーダル（~/Downloads の候補から選択）。
    if let Some(state) = &app.import {
        let modal_w = (area.width.saturating_sub(8)).min(80).max(30);
        let list_h = state.candidates.len() as u16 + 2;
        let modal_h = list_h.saturating_add(2).min(area.height.saturating_sub(4));
        let rect = centered_rect(modal_w, modal_h, area);
        frame.render_widget(Clear, rect);
        let items: Vec<ListItem> = state
            .candidates
            .iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string();
                ListItem::new(name)
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.accent))
                    .title(" MANUAL IMPORT — j/k:select Enter:import Esc:cancel ")
                    .style(Style::default().bg(theme.background)),
            )
            .highlight_style(Style::default().bg(theme.accent).fg(theme.background));
        let mut list_state = ListState::default();
        list_state.select(Some(state.selected));
        frame.render_stateful_widget(list, rect, &mut list_state);
    }
    // 下部 LOG ペイン（バックグラウンド処理の出力。papr の Downloads 相当）
    if !app.logs.is_empty() {
        let visible: Vec<Line> = app
            .logs
            .iter()
            .rev()
            .take(LOG_VISIBLE_LINES)
            .rev()
            .map(|l| Line::styled(l.clone(), Style::default().fg(theme.warning)))
            .collect();
        let log_pane = Paragraph::new(visible)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.border))
                    .title(" LOG — バックグラウンド処理 ")
                    .style(Style::default().bg(theme.background)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(log_pane, chunks[2]);
    }
    let footer = Paragraph::new(Line::styled(
        app.status.clone(),
        Style::default().fg(theme.muted),
    ))
    .block(Block::default().borders(Borders::NONE));
    frame.render_widget(footer, chunks[3]);
}

// ---------- Download / PDF ----------
/// 引用文献の PDF を取得する。
///
/// `sai-paper get-pdf` を子プロセスとして起動するが、stdout/stderr は必ず
/// pipe で受けて TUI 内の LOG ペインに表示する（papr の DownloadManager 相当）。
/// pipe しないと子プロセスが TTY を継承し、alternate screen を迂回して
/// 直接描画されて TUI が崩壊するため。
fn do_download(ref_row: &RefRow, log_tx: &UnboundedSender<String>) {
    // 直接解決できる ID（S2 40hex / DOI）。引用文献は docling 抽出のみで S2 未マッチの
    // ものが多く、ID が無いと以前は「タイトル文字列を paperId として」get-pdf に渡して
    // S2 404 で必ず失敗していた。ID が無い場合はタイトル検索で S2 paperId を解決する。
    let direct_id = ref_row.s2_paper_id.clone().or_else(|| {
        ref_row
            .doi
            .clone()
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .map(|d| format!("DOI:{d}"))
    });
    let title = ref_row
        .s2_title
        .clone()
        .or_else(|| ref_row.title.clone())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    if direct_id.is_none() && title.is_none() {
        return;
    }
    // sai-paper CLI（core 統合版）に委譲。無い場合はブラウザで開く
    let bin = sai_paper_cli_bin();
    let tx = log_tx.clone();
    tokio::spawn(async move {
        let pid = match direct_id {
            Some(id) => id,
            None => {
                let t = title.unwrap_or_default();
                let _ = tx.send(format!(
                    "[get-pdf] S2 未マッチのためタイトル検索で解決: {t}"
                ));
                match resolve_paper_id_by_title(&t).await {
                    Ok(Some(id)) if !id.is_empty() => id,
                    Ok(_) => {
                        let _ =
                            tx.send("[get-pdf] S2 検索に一致する論文が見つからず中止".to_string());
                        return;
                    }
                    Err(e) => {
                        let _ = tx.send(format!("[get-pdf] S2 検索エラー: {e}"));
                        return;
                    }
                }
            }
        };
        let Some(bin) = bin else {
            let _ = tx.send("[get-pdf] sai-paper CLI が見つからず中止".to_string());
            return;
        };
        {
            let _ = tx.send(format!("[get-pdf] 開始: {pid}"));
            match TokioCommand::new(&bin)
                .arg("get-pdf")
                .arg(&pid)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(mut child) => {
                    let stdout = child.stdout.take();
                    let stderr = child.stderr.take();
                    // stdout / stderr を行単位で読み、チャネルへ流す
                    let mut readers = Vec::new();
                    if let Some(out) = stdout {
                        let tx = tx.clone();
                        readers.push(tokio::spawn(async move {
                            let mut lines = BufReader::new(out).lines();
                            while let Ok(Some(l)) = lines.next_line().await {
                                let _ = tx.send(format!("[get-pdf] {l}"));
                            }
                        }));
                    }
                    if let Some(err) = stderr {
                        let tx = tx.clone();
                        readers.push(tokio::spawn(async move {
                            let mut lines = BufReader::new(err).lines();
                            while let Ok(Some(l)) = lines.next_line().await {
                                let _ = tx.send(format!("[get-pdf] {l}"));
                            }
                        }));
                    }
                    for h in readers {
                        let _ = h.await;
                    }
                    match child.wait().await {
                        Ok(s) if s.success() => {
                            let _ = tx.send(format!("[get-pdf] 完了: {pid}"));
                        }
                        Ok(s) => {
                            let _ = tx.send(format!("[get-pdf] 失敗 (exit {s}): {pid}"));
                        }
                        Err(e) => {
                            let _ = tx.send(format!("[get-pdf] エラー: {e}"));
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(format!("[get-pdf] 起動失敗: {e}"));
                }
            }
        }
    });
}

/// タイトルで S2 を検索し、先頭候補の paperId を返す。
async fn resolve_paper_id_by_title(title: &str) -> Result<Option<String>, String> {
    let client = S2Client::new(None).map_err(|e: sai_paper_core::S2Error| e.to_string())?;
    let papers = client
        .search(title, 1)
        .await
        .map_err(|e: sai_paper_core::S2Error| e.to_string())?;
    Ok(papers.into_iter().next().map(|p| p.id))
}

/// ブラウザで URL を開く（macOS / Linux / Windows に対応）。
fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    #[cfg(windows)]
    let program = "cmd";
    let _ = Command::new(program)
        .args({
            #[cfg(windows)]
            {
                vec!["/C", "start", url]
            }
            #[cfg(not(windows))]
            {
                vec![url]
            }
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
fn do_open_pdf(ref_row: &RefRow) {
    // fallback open doi or title search
    if let Some(doi) = &ref_row.doi {
        open_in_browser(&format!("https://doi.org/{}", doi));
    } else if let Some(t) = &ref_row.title {
        open_in_browser(&format!("https://scholar.google.com/scholar?q={}", t));
    }
}

// ---------- Main ----------
#[tokio::main]
async fn main() -> Result<()> {
    // プロジェクトルートの .env を読み込む（S2 API キーのみ使用。既存の環境変数は上書きしない）
    sai_paper_core::paths::load_project_dotenv();
    let cli = Cli::parse();
    let raw_pid = match cli.cmd {
        Some(Cmd::Citation { paper_id, extra: _ }) => paper_id,
        None => cli.paper_id.unwrap_or_else(|| {
            eprintln!("Usage: sai-moon-paper-cite citation <paperId>");
            std::process::exit(1);
        }),
    };
    // extra (title fallback with spaces) is intentionally ignored — pid is first token
    // 引数は抽出キーのまま受け付ける。数字だけの引数は Library の番号（papers.id）と
    // して受け付ける（段階 2c）。表示は変えない。
    let store = Store::open();
    let numeric_id: Option<i64> =
        if !raw_pid.is_empty() && raw_pid.chars().all(|c| c.is_ascii_digit()) {
            raw_pid.parse::<i64>().ok()
        } else {
            None
        };
    // 読むキー（pid）と、対応する Library の番号をまとめて決める。
    let (pid, papers_id) = match numeric_id {
        // Library の番号 → その論文の抽出キーも合わせて得る。
        Some(id) => {
            let key = store
                .db
                .as_ref()
                .and_then(|db| db.citation_key_for_paper(id))
                .map(|raw| sai_paper_core::citation::normalize_paper_id(&raw))
                .unwrap_or_else(|| sai_paper_core::citation::normalize_paper_id(&raw_pid));
            (key, Some(id))
        }
        // 抽出キー → そのキーの papers 行の番号を引く（段階 2c）。
        None => {
            let key = sai_paper_core::citation::normalize_paper_id(&raw_pid);
            let papers_id = store
                .db
                .as_ref()
                .and_then(|db| Database::papers_id_for_extract_key(db.connection(), &key));
            (key, papers_id)
        }
    };
    // papr.db（core 経由・マイグレーション適用済み）から読む。
    // docling 引用抽出結果（paper_refs / paper_sentences）のみを表示データとする。
    // S2 フォールバックは行わない（docling 抽出済み論文が前提）。
    let mut meta = papers_id
        .and_then(|id| store.load_meta_by_papers_id(id))
        .or_else(|| store.load_meta(&pid));
    let refs = store.load_refs(papers_id, &pid);
    let sents = store.load_sentences(papers_id, &pid);
    // If still no meta, create minimal
    if meta.is_none() {
        meta = Some(PaperMeta {
            paper_id: pid.clone(),
            title: pid.clone(),
            year: None,
            venue: None,
            authors: vec![],
            abstract_text: None,
            doi: None,
            arxiv_id: None,
            pdf_path: None,
        });
    }
    let theme = Theme::load("catppuccin-mocha").unwrap_or_else(|_| Theme::load("gruvbox").unwrap());
    // バックグラウンド処理（get-pdf 等）のログを TUI 内に表示するためのチャネル
    let (log_tx, log_rx) = unbounded_channel();
    // R5: Library 照合情報（papr.db の papers 一覧）を初期読込する
    let db = store.db;
    let lib_entries = db
        .as_ref()
        .map(|db| db.library_match_entries().unwrap_or_default())
        .unwrap_or_default();
    let db_path_display = store.db_path.display().to_string();
    let mut app = App {
        paper_id: pid.clone(),
        data_dir: store.data_dir.clone(),
        meta,
        refs,
        sentences: sents,
        selected: 0,
        paper_scroll: 0,
        mode: Mode::List,
        theme,
        status: String::new(),
        filter: String::new(),
        logs: Vec::new(),
        log_rx: Some(log_rx),
        db,
        lib_entries,
        import: None,
    };
    app.status = format!(
        "{} refs | DB: {} | q:quit Enter:PAPER d:download i:import p:pdf j/k:move",
        app.refs.len(),
        db_path_display
    );

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    // マウス操作は廃止。移動はキーボード（j/k・矢印キー）のみ。
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_loop(&mut terminal, &mut app, log_tx).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    if let Err(e) = res {
        eprintln!("error: {e:?}");
    }
    Ok(())
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    log_tx: UnboundedSender<String>,
) -> Result<()> {
    loop {
        // 到着済みのバックグラウンドログを取り込んでから描画
        app.drain_logs();
        // poll=0 で到着済みイベントを全件消化（sai-paper-tui と同じバッチ処理）。
        // 1 イベントごとに描画しないため、大量のスクロール入力が
        // 逆方向の操作を遅延させない。
        while event::poll(Duration::ZERO)? {
            if process_terminal_event(event::read()?, app, &log_tx).await? {
                return Ok(());
            }
        }
        // 必要なら待機して 1 件処理
        if event::poll(Duration::from_millis(100))? {
            if process_terminal_event(event::read()?, app, &log_tx).await? {
                return Ok(());
            }
        }
        // イベント処理後に描画は 1 回だけ
        terminal.draw(|f| render_app(f, app))?;
    }
}

/// 端末イベント 1 件を処理する。戻り値 true は TUI ループの終了（q / Esc）。
async fn process_terminal_event(
    event: Event,
    app: &mut App,
    log_tx: &UnboundedSender<String>,
) -> Result<bool> {
    let Event::Key(k) = event else {
        return Ok(false);
    };
    if k.kind != KeyEventKind::Press {
        return Ok(false);
    }
    // R5: 手動 PDF 取り込みモーダルが開いている間はモーダル側のキー処理を優先する。
    if app.import.is_some() {
        handle_manual_import_key(app, k).await;
        return Ok(false);
    }
    // filter input when in filter mode? simple: '/' starts filter, Esc clears
    match app.mode {
        Mode::List => match k.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                if !app.filter.is_empty() {
                    app.filter.clear();
                } else if k.code == KeyCode::Esc && escape_sequence_fragment().await? {
                    // ESC[B が読み込み境界で分離されると、crossterm は
                    // 単独 Esc を先に返す（仕様上の曖昧性）。直後に追加入力が
                    // 到達しているなら分離シーケンスの断片なので終了しない。
                    // これがないと高速スクロール中に TUI がサイレント終了する。
                } else {
                    return Ok(true);
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                // 矢印キーと同一の移動ロジック（両端でクランプ）
                move_list_selection_down(app);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                move_list_selection_up(app);
            }
            KeyCode::Char('g') if k.modifiers.contains(KeyModifiers::NONE) => {
                // gg handled as two presses? simplify: g -> top
                app.selected = *app.filtered_indices().first().unwrap_or(&0);
            }
            KeyCode::Char('G') => {
                app.selected = *app.filtered_indices().last().unwrap_or(&0);
            }
            KeyCode::Enter => {
                if !app.refs.is_empty() {
                    app.mode = Mode::Paper;
                    app.paper_scroll = 0;
                }
            }
            KeyCode::Char('d') => {
                if let Some(r) = app.refs.get(app.selected) {
                    do_download(r, &log_tx);
                    app.status = format!("download triggered for ref {}", r.ref_no);
                }
            }
            KeyCode::Char('i') => {
                // R5: 手動 PDF 取り込み（自動 DL 不可のジャーナル向け）。
                start_manual_import(app);
            }
            KeyCode::Char('p') => {
                if let Some(r) = app.refs.get(app.selected) {
                    do_open_pdf(r);
                    app.status = format!("open pdf for ref {}", r.ref_no);
                }
            }
            KeyCode::Char('/') => {
                app.filter.clear();
                app.status = "filter: type to filter, Esc to clear".to_string();
            }
            KeyCode::Backspace => {
                app.filter.pop();
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                // if filter mode or slash previously, append; we treat any typing when filter non-empty or after '/' as filter
                // Heuristic: if filter was set or user typed '/', we already handle. For simplicity, if user types and not in special, treat as filter
                // Only when filter is non-empty or last key was '/'
                if !app.filter.is_empty() || c == '/' {
                    if c != '/' {
                        app.filter.push(c);
                    }
                } else if app.filter.is_empty() && c.is_ascii_alphabetic() {
                    // optionally start filter on any letter? keep papr behavior: '>' for filter, but we use '/'
                    // ignore
                }
            }
            _ => {}
        },
        Mode::Paper => match k.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('h') => {
                app.mode = Mode::List;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                app.paper_scroll = app.paper_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.paper_scroll = app.paper_scroll.saturating_sub(1);
            }
            KeyCode::Char('d') => {
                if let Some(r) = app.refs.get(app.selected) {
                    do_download(r, &log_tx);
                    app.status = format!("download triggered for ref {}", r.ref_no);
                }
            }
            KeyCode::Char('i') => {
                // R5: 手動 PDF 取り込み（PAPER 詳細画面からも）。
                start_manual_import(app);
            }
            KeyCode::Char('p') => {
                if let Some(r) = app.refs.get(app.selected) {
                    do_open_pdf(r);
                    app.status = format!("open pdf for ref {}", r.ref_no);
                }
            }
            _ => {}
        },
    }
    Ok(false)
}

/// 選択を 1 行下へ移動する（末尾でクランプ、副作用なし）。
fn move_list_selection_down(app: &mut App) {
    if app.refs.is_empty() {
        return;
    }
    let indices = app.filtered_indices();
    if indices.is_empty() {
        return;
    }
    let vis = indices.iter().position(|&x| x == app.selected).unwrap_or(0);
    let next_vis = (vis + 1).min(indices.len() - 1);
    app.selected = indices[next_vis];
}

// ---------- R5: 手動 PDF 取り込み ----------

/// 手動取り込み候補に含める mtime の窓（5 分。Discover と同じ）。
const MANUAL_IMPORT_WINDOW: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// ユーザーのダウンロードディレクトリ（ブラウザ保存先）を解決する。
/// XDG_DOWNLOAD_DIR → ~/.config/user-dirs.dirs → ~/Downloads → ~/ダウンロード。
fn manual_downloads_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok().map(PathBuf::from)?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(raw) = std::env::var("XDG_DOWNLOAD_DIR") {
        candidates.push(
            raw.strip_prefix("$HOME/")
                .map_or_else(|| home.join(&raw), |rest| home.join(rest)),
        );
    }
    if let Ok(contents) = std::fs::read_to_string(home.join(".config").join("user-dirs.dirs")) {
        for line in contents.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("XDG_DOWNLOAD_DIR=") {
                let value = rest.trim().trim_matches('"');
                candidates.push(
                    value
                        .strip_prefix("$HOME/")
                        .map_or_else(|| home.join(value), |r| home.join(r)),
                );
                break;
            }
        }
    }
    candidates.push(home.join("Downloads"));
    candidates.push(home.join("ダウンロード"));
    candidates.into_iter().find(|dir| dir.is_dir())
}

/// 指定ディレクトリ直下の PDF のうち mtime が window 以内のものを新しい順で返す。
fn recent_download_pdfs(dir: &Path, window: std::time::Duration) -> Vec<PathBuf> {
    let cutoff = std::time::SystemTime::now()
        .checked_sub(window)
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut items: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
        })
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            (modified >= cutoff).then_some((modified, entry.path()))
        })
        .collect();
    items.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    items.into_iter().map(|(_, path)| path).collect()
}

/// R5: 手動取り込みモーダルを開く（~/Downloads の直近 5 分の PDF を候補にする）。
fn start_manual_import(app: &mut App) {
    if app.refs.is_empty() {
        return;
    }
    let Some(dir) = manual_downloads_dir() else {
        app.status = "ダウンロードディレクトリが見つかりません".to_string();
        return;
    };
    let candidates = recent_download_pdfs(&dir, MANUAL_IMPORT_WINDOW);
    if candidates.is_empty() {
        app.status = format!(
            "{} に直近 5 分以内の PDF がありません。ブラウザで保存してからもう一度 i を押してください",
            dir.display()
        );
        return;
    }
    app.import = Some(ManualImportState {
        candidates,
        selected: 0,
    });
}

/// R5: 手動取り込みモーダル内のキー処理（j/k / Enter / Esc）。
async fn handle_manual_import_key(app: &mut App, k: crossterm::event::KeyEvent) {
    let Some(state) = &mut app.import else {
        return;
    };
    let count = state.candidates.len();
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => app.import = None,
        KeyCode::Char('j') | KeyCode::Down => {
            if count > 0 {
                state.selected = (state.selected + 1).min(count - 1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.selected = state.selected.saturating_sub(1);
        }
        KeyCode::Enter => {
            let selected = state.selected;
            let source = app
                .import
                .as_ref()
                .and_then(|s| s.candidates.get(selected))
                .cloned();
            app.import = None;
            if let Some(source) = source {
                confirm_manual_import(app, &source).await;
            }
        }
        _ => {}
    }
}

/// 自動 DL と同じ正規名で衝突しない保存先を決める。
fn unique_pdf_destination(dir: &Path, title: &str) -> Result<PathBuf> {
    let base = sai_paper_core::sanitize_download_filename_component(title);
    let base = if base.is_empty() {
        "paper".to_owned()
    } else {
        base
    };
    let mut candidate = dir.join(format!("{base}.pdf"));
    let mut suffix = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{base} ({suffix}).pdf"));
        suffix += 1;
    }
    Ok(candidate)
}

/// ライブラリの正規ディレクトリ（`<project>/papers/`）を解決する。
fn library_dir() -> Option<PathBuf> {
    sai_paper_core::Paths::discover()
        .ok()
        .map(|p| p.downloads_dir)
}

/// R5: 候補 PDF を検証・移動し、選択中の引用文献を論文として Library に登録する。
///
/// Discover の `i`（手動 PDF 取り込み）と同じ流れ:
/// %PDF マジック検証 → `<project>/papers/` へ正規名で移動 →
/// `Database::attach_download` で papers テーブルに紐付け。
async fn confirm_manual_import(app: &mut App, source: &Path) {
    let Some(r) = app.refs.get(app.selected) else {
        return;
    };
    // %PDF マジック検証（自動 DL と同一の水準）
    let mut magic = [0_u8; 5];
    match std::fs::File::open(source) {
        Ok(mut file) => {
            use std::io::Read;
            if let Err(e) = file.read_exact(&mut magic) {
                app.status = format!("import failed: {e}");
                return;
            }
        }
        Err(e) => {
            app.status = format!("import failed: {e}");
            return;
        }
    }
    if &magic != b"%PDF-" {
        app.status = format!("not a PDF file: {}", source.display());
        return;
    }
    // 登録用 ID（S2 paperId → DOI の優先順。無い場合は登録不可）
    let direct_id = r.s2_paper_id.clone().or_else(|| {
        r.doi
            .clone()
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .map(|d| format!("DOI:{d}"))
    });
    let Some(id) = direct_id else {
        app.status =
            "import requires S2 paperId or DOI (この引用文献は手動登録できません)".to_string();
        return;
    };
    let Some(title) = r
        .s2_title
        .clone()
        .or_else(|| r.title.clone())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
    else {
        app.status = "import requires a title".to_string();
        return;
    };
    let Some(dir) = library_dir() else {
        app.status = "ライブラリディレクトリ (papers/) が解決できません".to_string();
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        app.status = format!("import failed: {e}");
        return;
    }
    let destination = match unique_pdf_destination(&dir, &title) {
        Ok(dest) => dest,
        Err(e) => {
            app.status = format!("import failed: {e}");
            return;
        }
    };
    if let Err(e) = sai_paper_core::move_pdf_file(source, &destination) {
        app.status = format!("import failed: {e}");
        return;
    }
    let pdf = match sai_paper_core::LibraryIndexer::inspect(&destination) {
        Ok(pdf) => pdf,
        Err(e) => {
            app.status = format!("import failed (inspect): {e}");
            return;
        }
    };
    let remote = RemotePaper {
        id,
        title: title.clone(),
        authors: r.first_author.clone().map(|a| vec![a]).unwrap_or_default(),
        abstract_text: String::new(),
        published: {
            let year = r.year.or(r.s2_year).unwrap_or(2000);
            chrono::NaiveDate::from_ymd_opt(year, 1, 1)
                .unwrap_or_default()
                .and_hms_opt(0, 0, 0)
                .unwrap_or_default()
                .and_utc()
        },
        updated: chrono::Utc::now(),
        categories: vec![],
        pdf_url: None,
        doi: r.doi.clone(),
        journal_ref: r.journal.clone(),
        citation_count: None,
    };
    // 置き場（論文フォルダ）は attach_download 内の paths::paper_folder が決める。
    let data_dir = match sai_paper_core::Paths::discover() {
        Ok(paths) => paths.data_dir(),
        Err(e) => {
            app.status = format!("import failed: {e}");
            return;
        }
    };
    match app.db.as_mut() {
        Some(db) => match db.attach_download(&remote, &pdf, &data_dir) {
            Ok(_paper_id) => {
                // 緑色表示を最新化する
                app.refresh_lib_entries();
                app.status = format!(
                    "imported: {} — Library に登録しました",
                    destination
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("PDF")
                );
            }
            Err(e) => app.status = format!("import failed (db): {e}"),
        },
        None => app.status = "papr.db が開けないため登録できません".to_string(),
    }
}

/// 選択を 1 行上へ移動する（先頭でクランプ、副作用なし）。
fn move_list_selection_up(app: &mut App) {
    if app.refs.is_empty() {
        return;
    }
    let indices = app.filtered_indices();
    if indices.is_empty() {
        return;
    }
    let vis = indices.iter().position(|&x| x == app.selected).unwrap_or(0);
    app.selected = indices[vis.saturating_sub(1)];
}

/// Esc キー判定の直後に追加入力が到達したかどうか。
///
/// crossterm は ESC[B が読み込み境界で分離されると、先頭の ESC を単独の Esc キーと
/// して返す（真の Esc と区別できない）。直後の短い窓内に追加入力があれば分離断片の
/// 誤検出とみなす（true を返す）。
async fn escape_sequence_fragment() -> Result<bool> {
    Ok(event::poll(ESC_SEQUENCE_WINDOW)?)
}

/// モーダル等で使う画面中央の矩形。
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
