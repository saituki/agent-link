//! カラーテーマ定義。
//!
//! `sai-control-tui` の `project_page` が全フィールドを上書き利用するため、
//! 一部が sai-link-tui 内で未使用でも公開フィールドとして維持する。

use ratatui::style::Color;

/// アプリケーションカラーテーマ。
#[derive(Debug, Clone)]
pub struct AppTheme {
    pub fg: Color,
    pub sidebar_bg: Color,
    pub sidebar_fg: Color,
    pub main_bg: Color,
    pub border: Color,
    pub selected: Color,
    pub success: Color,
    pub error: Color,
    pub warning: Color,
    pub info: Color,
    pub muted: Color,
    /// クリック可能な項目（session / waggle トークン）用（P2-2）
    pub link: Color,
    /// コードブロック / インラインコードの背景色（マークダウン表示用）
    pub code_bg: Color,
    /// マークダウン見出し用（H1〜H6 統一色）
    pub heading: Color,
    /// マークダウン太字用
    pub strong: Color,
    /// マークダウンインラインコード用
    pub inline_code: Color,
}

impl AppTheme {
    /// デフォルト（ダーク）テーマ。
    pub fn dark() -> Self {
        Self {
            fg: Color::Rgb(220, 220, 230),
            sidebar_bg: Color::Rgb(22, 22, 30),
            sidebar_fg: Color::Rgb(190, 190, 210),
            main_bg: Color::Rgb(18, 18, 24),
            border: Color::Rgb(60, 60, 80),
            selected: Color::Rgb(255, 200, 50),
            success: Color::Rgb(80, 220, 100),
            error: Color::Rgb(255, 80, 80),
            warning: Color::Rgb(255, 200, 50),
            info: Color::Rgb(80, 160, 255),
            muted: Color::Rgb(100, 100, 120),
            // クリック可能なリンクは青（ユーザー要求: リンク色 = 青）
            link: Color::Rgb(80, 160, 255),
            // コードブロック用の暗い背景（main_bg より少し明るい）
            code_bg: Color::Rgb(28, 28, 38),
            // マークダウン専用色（ユーザー要求）
            heading: Color::Rgb(100, 220, 220),   // シアン
            strong: Color::Rgb(220, 200, 80),    // 黄色
            inline_code: Color::Rgb(220, 120, 220), // マゼンタ
        }
    }
}

impl Default for AppTheme {
    fn default() -> Self {
        Self::dark()
    }
}
