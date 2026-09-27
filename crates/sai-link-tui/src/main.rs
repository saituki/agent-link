//! Sai-TUI エントリポイント。
//!
//! ```bash
//! cargo run --bin sai-link-tui
//! ```

use std::sync::Arc;

use app::AppState;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use sai_link_engine::db::SaiStore;

// ============================================================================
// アプリケーションモジュール（同一クレート内）
// ============================================================================

mod app;
mod graph_layout;
mod paths;
mod ui;
mod widgets;

use app::run_event_loop;

/// メインエントリポイント。
fn main() -> std::io::Result<()> {
    // パニック時にターミナル状態を復元する（raw mode 解除・alternate screen 脱出）。
    // これを入れないと、パニックでプロセスが落ちたときターミナルが
    // 制御不能な状態（raw mode のまま / バックグラウンド色が変わったまま）に残る。
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        );
        default_hook(info);
    }));

    // ターミナルのセットアップ
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // DB を探す（sai-engine と同じ DB を共有・SAI_PROJECT_ROOT 優先）
    let db_path = paths::resolve_db_path();

    eprintln!("[sai-link-tui] Using DB: {}", db_path.display());
    let store = match SaiStore::open(&db_path) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            // クリーンアップしてからエラー出力
            disable_raw_mode()?;
            execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
            eprintln!("DB の初期化に失敗しました: {}", e);
            std::process::exit(1);
        }
    };

    // アプリケーション状態の初期化
    let mut state = AppState::new(Arc::clone(&store));
    state.load_data();

    // イベントループの実行
    let result = run_event_loop(&mut terminal, &mut state);

    // クリーンアップ
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;

    result
}
