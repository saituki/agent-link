//! Terminal setup with reliable restoration on every return path.

use std::io::{self, Stdout};

use crossterm::{
    cursor::SetCursorStyle,
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

/// Owns raw mode and alternate-screen lifetime.
pub struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    keyboard_enhancement_enabled: bool,
    command_cursor_active: bool,
}

impl TerminalSession {
    /// Enter the alternate screen and start a ratatui terminal.
    pub fn start() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        // ホイールのスクロールに必要。
        // 他画面ではマウスイベントを無視する。
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture
        ) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        // Enhanced keyboard input makes Ctrl+Backspace distinguishable from
        // Backspace. Request alternate keycodes as well: Crossterm uses the
        // terminal-provided, layout-resolved shifted character from that
        // field, so printable input remains correct without per-key fallbacks.
        // Unsupported terminals stay on their existing input path.
        //
        // 注意: supports_keyboard_enhancement() は CSI ? u 応答を最大 2 秒待つ。
        // kitty プロトコル非対応の端末（Terminal.app / iTerm2 等）では必ず
        // タイムアウトまで待ち、起動が数秒遅くなる。対応端末に限定して問い合わせる。
        let keyboard_enhancement_enabled = terminal_supports_kitty_protocol()
            && supports_keyboard_enhancement().unwrap_or(false)
            && execute!(
                stdout,
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS,
                )
            )
            .is_ok();
        let terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(error) => {
                let mut stdout = io::stdout();
                if keyboard_enhancement_enabled {
                    let _ = execute!(stdout, PopKeyboardEnhancementFlags);
                }
                let _ = execute!(stdout, LeaveAlternateScreen);
                let _ = disable_raw_mode();
                return Err(error);
            }
        };
        Ok(Self {
            terminal,
            keyboard_enhancement_enabled,
            command_cursor_active: false,
        })
    }

    /// Borrow the ratatui terminal for drawing.
    pub fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }

    /// Use a blinking bar only while the command popup owns text input.
    pub fn set_command_cursor_active(&mut self, active: bool) -> io::Result<()> {
        if self.command_cursor_active == active {
            return Ok(());
        }
        execute!(
            self.terminal.backend_mut(),
            if active {
                SetCursorStyle::BlinkingBar
            } else {
                SetCursorStyle::BlinkingBlock
            }
        )?;
        self.command_cursor_active = active;
        Ok(())
    }
}

/// kitty keyboard protocol をサポートすると推定できる端末かどうか。
///
/// 非対応端末で supports_keyboard_enhancement() を呼ぶと応答待ちのタイムアウト
/// （最大 2 秒）だけ起動が遅くなるため、対応が確実な端末名のときだけ問い合わせる。
/// 誤って「非対応」と判定した場合の影響は「拡張キーボード入力が無効になる」だけで、
/// 従来動作と同じため安全側に倒している。
fn terminal_supports_kitty_protocol() -> bool {
    let term = std::env::var("TERM")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
    let known = [
        "kitty",
        "ghostty",
        "wezterm",
        "foot",
        "contour",
        "rio",
        "alacritty",
    ];
    known.iter().any(|n| term.contains(n)) || known.iter().any(|n| program.eq_ignore_ascii_case(n))
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), DisableMouseCapture);
        let _ = execute!(self.terminal.backend_mut(), DisableBracketedPaste);
        if self.keyboard_enhancement_enabled {
            let _ = execute!(self.terminal.backend_mut(), PopKeyboardEnhancementFlags);
        }
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = execute!(
            self.terminal.backend_mut(),
            crossterm::cursor::SetCursorStyle::BlinkingBlock
        );
        let _ = self.terminal.show_cursor();
    }
}
