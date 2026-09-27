//! CLI 実行ポップアップ（`x` モード＝`rui-tui`）。
//!
//! sidebar で `j/k` 移動して選択したブロックを対象に CLI を実行する。
//! 入力フォームは廃止。全て `task.md` に事前記載し、未記載は block.md の
//! デフォルトで補う（`block` 未記載時のみエラー・CLI 引数での上書き可）。
//!
//! ## モード体系
//!
//! 通常画面で sidebar 選択を済ませてから `x` で入る（選択ブロック固定）。
//! `x` / `Esc` で1段階戻る（Menu ではモード終了・元画面に戻る）。
//!
//! | ステップ | キー | 動作 |
//! |---|---|---|
//! | Menu | `e`/`d`/`o` | アクションを選択（カーソル移動） |
//! | Menu | `↑↓`/`j`/`k` | アクション選択を移動 |
//! | Menu | `Enter` | 選択中アクションを確定（確定後は run-tui を閉じる） |
//! | Menu | `x` / `Esc` | モード終了 |
//! | ConfirmDelete | `Enter` | 削除確定（`Esc` で戻る） |
//! | UserWrite | `Enter` | 編集開始/確定（送信行で実行）/ `Esc` で Menu |

use std::collections::HashMap;
use std::path::PathBuf;

use crossterm::event::KeyCode;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

use super::AppState;

// ============================================================================
// 状態
// ============================================================================

/// CLI モードのステップ（`rui-tui`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CliStep {
    /// メニュー（`e`=状態を進める・`d`=rm・`o`=user write）
    #[default]
    Menu,
    /// 削除確認（`block rm` 相当・`Enter` 確定 / `Esc` 戻る）
    ConfirmDelete,
    /// user 成果物登録フォーム（runner=user / running のみ）
    UserWrite,
}

/// 入力フォームの1行（ブロック定義の inputs 由来）。
#[derive(Debug, Clone)]
pub struct CliFieldRow {
    /// フィールド ID
    pub id: String,
    /// 表示ラベル
    pub label: String,
    /// 型名（`text` / `select` / `confirm` / `file_token`）
    pub type_name: String,
    /// 必須か
    pub required: bool,
    /// 現在値（カードから解決済み・編集可）
    pub value: String,
    /// `select` の選択肢
    pub options: Vec<String>,
    /// ブロック定義の `default`（空欄時の代替）
    pub default: Option<String>,
}

/// グローバル上書き行の数（Round / GlobalRound / Tag / Think / Model）。
///
/// timeout は廃止したため含めない。
pub const GLOBAL_ROWS: usize = 5;
/// グローバル行のラベル。
pub const GLOBAL_LABELS: [&str; GLOBAL_ROWS] = [
    "Round*",
    "GlobalRound*",
    "Tag",
    "Think",
    "Model",
];
/// thinking budget の選択肢（`validation.rs` と一致）。
pub const THINK_OPTIONS: [&str; 6] = ["none", "low", "medium", "high", "xhigh", "max"];

/// run-tui メニューのアクション数（e / d / o）。
pub const MENU_ACTIONS: usize = 3;

/// CLI モードの状態（`AppState` に保持）。
#[derive(Debug, Clone, Default)]
pub struct CliModeState {
    /// モード中か（`x` で入る）
    pub active: bool,
    /// 現在のステップ
    pub step: CliStep,
    /// メニューの選択中アクション（0=e, 1=d, 2=o）
    pub action_cursor: usize,
    /// エラー表示（各ステップ下部）
    pub error: Option<String>,
    // ── カード選択 ──
    /// todo 未実行カード一覧
    pub cards: Vec<PathBuf>,
    /// カードカーソル
    pub card_cursor: usize,
    // ── ブロック選択 ──
    /// ブロック名一覧（`.sai/blocks` の stem）
    pub blocks: Vec<String>,
    /// ブロックカーソル
    pub block_cursor: usize,
    // ── 確定済み選択（`x` 押下時の sidebar 選択を固定） ──
    /// 選択中カードのパス
    pub card_path: Option<PathBuf>,
    /// レーン（todo / running / review / done）を選択した場合の対象レーン名。
    ///
    /// `Some` のとき `e` はそのレーンに存在する全てのカードに対して実行される
    /// （`d` / `o` はカード 1 枚のときのみ有効）。
    pub target_lane: Option<String>,
    /// 選択中カードの中身
    pub card: Option<sai_link_engine::kanban::KanbanCard>,
    /// 選択中ブロック名
    pub block_name: Option<String>,
    /// 選択中ブロック定義ファイル
    pub block_path: Option<PathBuf>,
    /// 選択中ブロック設定
    pub block_config: Option<sai_link_engine::block_config::BlockConfig>,
    /// 選択中の実行 ID（実行リーフ選択時）
    pub selected_exec_id: Option<i64>,
    // ── 入力フォーム ──
    /// 入力行（inputs 由来）
    pub fields: Vec<CliFieldRow>,
    /// グローバル上書き値（`GLOBAL_LABELS` と 1:1）
    pub globals: [String; GLOBAL_ROWS],
    /// フォームカーソル（0..fields + GLOBAL_ROWS + 送信行）
    pub form_cursor: usize,
    /// テキスト編集中か
    pub editing: bool,
    /// 編集中バッファ
    pub edit_buf: String,
    /// 削除対象の実行 ID
    pub delete_target: Option<i64>,
    /// 削除対象の表示名
    pub delete_label: String,
    // ── user write（`o`）──
    /// 登録対象 run（`x` 押下時の selected_exec_id）
    pub user_write_run_id: Option<i64>,
    /// 入力値 [filepath, waggle, description]
    pub user_write_values: [String; 3],
    /// フォーカス（0=filepath, 1=waggle, 2=description, 3=送信行）
    pub user_write_focus: usize,
    /// 編集中バッファ（`Some` で編集モード）
    pub user_write_buf: Option<String>,
}

impl CliModeState {
    /// CLI モードに入る（Menu から開始・`x` 押下時の sidebar 選択を固定）。
    pub fn open_with_selection(
        &mut self,
        card_path: Option<PathBuf>,
        card: Option<sai_link_engine::kanban::KanbanCard>,
        exec_id: Option<i64>,
    ) {
        self.active = true;
        self.step = CliStep::Menu;
        self.action_cursor = 0;
        self.error = None;
        self.card_path = card_path;
        self.card = card;
        self.target_lane = None;
        self.selected_exec_id = exec_id;
        self.delete_target = None;
    }

    /// 対象をレーン（todo / running / review / done）全体に設定する。
    ///
    /// sidebar でレーンフォルダを選択して `x` したときに呼ぶ。
    pub fn set_lane_target(&mut self, lane: &str) {
        self.target_lane = Some(lane.to_string());
        self.card_path = None;
        self.card = None;
        self.selected_exec_id = None;
    }

    /// モードを終了する（元画面に戻る）。
    pub fn close(&mut self) {
        self.active = false;
        self.step = CliStep::Menu;
        self.action_cursor = 0;
        self.error = None;
        self.editing = false;
        self.user_write_buf = None;
        self.user_write_run_id = None;
    }

    /// フォームの総行数（入力行 + グローバル行 + 送信行）。
    pub fn form_total_rows(&self) -> usize {
        self.fields.len() + GLOBAL_ROWS + 1
    }

    /// カーソルが送信行か。
    pub fn is_send_row(&self) -> bool {
        self.form_cursor + 1 == self.form_total_rows()
    }

    /// カーソルがグローバル行ならそのインデックス。
    pub fn global_idx(&self) -> Option<usize> {
        let g0 = self.fields.len();
        if self.form_cursor >= g0 && self.form_cursor < g0 + GLOBAL_ROWS {
            Some(self.form_cursor - g0)
        } else {
            None
        }
    }
}

// ============================================================================
// キー処理（CLI モード中の専用キーマップ）
// ============================================================================

/// CLI モード中のキー処理。処理した場合は `true` を返す。
pub fn handle_cli_mode_key(state: &mut AppState, key: crossterm::event::KeyEvent) -> bool {
    let step = state.cli_mode.step;
    match step {
        CliStep::Menu => handle_menu_key(state, key),
        CliStep::ConfirmDelete => handle_delete_key(state, key),
        CliStep::UserWrite => handle_user_write_key(state, key),
    }
}

/// メニュー（`↑↓`/`j`/`k`=選択移動・`e`/`d`/`o`=選択・`Enter`=確定・`x`/`Esc`=終了）。
///
/// 対象は `x` 押下時の sidebar 選択で固定（モード中の `j/k` 移動は廃止）。
/// 確定（`Enter`）後にモードを閉じることで、連続で `e` を押して status が
/// 2 段階進むのを防ぐ。UI 文字は英語表記。
fn handle_menu_key(state: &mut AppState, key: crossterm::event::KeyEvent) -> bool {
    match key.code {
        // 各アクションの先頭キーは「選択」のみ（実行は `Enter`）。
        crossterm::event::KeyCode::Char('e') => {
            state.cli_mode.action_cursor = 0;
            true
        }
        crossterm::event::KeyCode::Char('d') => {
            state.cli_mode.action_cursor = 1;
            true
        }
        crossterm::event::KeyCode::Char('o') => {
            state.cli_mode.action_cursor = 2;
            true
        }
        crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
            state.cli_mode.action_cursor = state.cli_mode.action_cursor.saturating_sub(1);
            true
        }
        crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
            state.cli_mode.action_cursor =
                (state.cli_mode.action_cursor + 1).min(MENU_ACTIONS - 1);
            true
        }
        crossterm::event::KeyCode::Enter => {
            let idx = state.cli_mode.action_cursor;
            execute_menu_action(state, idx);
            true
        }
        crossterm::event::KeyCode::Char('x') | crossterm::event::KeyCode::Esc => {
            state.cli_mode.close();
            true
        }
        _ => true, // モード中は他キーを吸収する（通常 shortcut を無効化）
    }
}

/// メニューで選択されたアクションを実行する。
///
/// - `0` (e): 状態を 1 段階進める。成功したら run-tui を閉じる
/// - `1` (d): 削除確認へ
/// - `2` (o): user 成果物登録フォームへ
fn execute_menu_action(state: &mut AppState, idx: usize) {
    match idx {
        0 => {
            // レーン選択時はそのレーンの全カードを、カード選択時は 1 枚を対象にする
            match state.cli_mode.target_lane.clone() {
                Some(lane) => do_lane_transition(state, &lane),
                None => do_e_transition(state),
            }
            // 確定後は run-tui を閉じる（連続押下による多重遷移を防ぐ）。
            if state.cli_mode.error.is_none() {
                state.cli_mode.close();
            }
        }
        1 => start_delete_flow(state),
        2 => start_user_write_flow(state),
        _ => {}
    }
}

// ============================================================================
// user write フロー（`o`・runner=user の成果物登録）
// ============================================================================

/// `o`: runner=user かつ status=running のときだけ入力フォームを開く。
///
/// 対象 run は `x` 押下時の sidebar 選択で固定済み（`selected_exec_id`）。
fn start_user_write_flow(state: &mut AppState) {
    let Some(id) = state.cli_mode.selected_exec_id else {
        state.cli_mode.error = Some("o: select an execution in the sidebar".to_string());
        return;
    };
    let exec = match state.store.get_execution_by_id(id) {
        Ok(e) => e,
        Err(e) => {
            state.cli_mode.error = Some(format!("o: {}", e));
            return;
        }
    };
    if exec.runner != "user" {
        state.cli_mode.error = Some(format!(
            "o: ID{} is not a user task (runner={})",
            id, exec.runner
        ));
        return;
    }
    if exec.status != "running" {
        state.cli_mode.error = Some(format!(
            "o: ID{} is not running (status={})",
            id, exec.status
        ));
        return;
    }
    state.cli_mode.user_write_run_id = Some(id);
    state.cli_mode.user_write_values = Default::default();
    state.cli_mode.user_write_focus = 0;
    state.cli_mode.user_write_buf = None;
    state.cli_mode.error = None;
    state.cli_mode.step = CliStep::UserWrite;
}

/// user-write フォームのキー処理。
///
/// - 編集中: `Enter`=確定 / `Esc`=取消 / `Backspace`・文字=編集
/// - 非編集: `Tab`・`↑↓`=移動 / `Enter`=編集開始（送信行では実行）/ `Esc`=Menu に戻る
fn handle_user_write_key(state: &mut AppState, key: crossterm::event::KeyEvent) -> bool {
    if state.cli_mode.user_write_buf.is_some() {
        match key.code {
            KeyCode::Enter => {
                let idx = state.cli_mode.user_write_focus;
                if idx < 3 {
                    let buf = state.cli_mode.user_write_buf.take().unwrap_or_default();
                    state.cli_mode.user_write_values[idx] = buf;
                } else {
                    state.cli_mode.user_write_buf = None;
                }
                true
            }
            KeyCode::Esc => {
                state.cli_mode.user_write_buf = None;
                true
            }
            KeyCode::Backspace => {
                if let Some(buf) = state.cli_mode.user_write_buf.as_mut() {
                    buf.pop();
                }
                true
            }
            KeyCode::Char(c) => {
                if let Some(buf) = state.cli_mode.user_write_buf.as_mut() {
                    buf.push(c);
                }
                true
            }
            _ => true,
        }
    } else {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                if state.cli_mode.user_write_focus > 0 {
                    state.cli_mode.user_write_focus -= 1;
                }
                true
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                if state.cli_mode.user_write_focus < 3 {
                    state.cli_mode.user_write_focus += 1;
                }
                true
            }
            KeyCode::Enter => {
                if state.cli_mode.user_write_focus < 3 {
                    let idx = state.cli_mode.user_write_focus;
                    state.cli_mode.user_write_buf =
                        Some(state.cli_mode.user_write_values[idx].clone());
                } else {
                    do_user_write(state);
                }
                true
            }
            KeyCode::Esc => {
                state.cli_mode.step = CliStep::Menu;
                state.cli_mode.error = None;
                state.cli_mode.user_write_buf = None;
                true
            }
            _ => true,
        }
    }
}

/// 送信: エンジンの `register_user_output` を呼ぶ。
fn do_user_write(state: &mut AppState) {
    let Some(run_id) = state.cli_mode.user_write_run_id else {
        state.cli_mode.error = Some("o: no target run".to_string());
        return;
    };
    let filepath = state.cli_mode.user_write_values[0].trim().to_string();
    if filepath.is_empty() {
        state.cli_mode.error = Some("filepath is required".to_string());
        return;
    }
    let source = resolve_input_path(&filepath);
    let description = state.cli_mode.user_write_values[2].trim().to_string();
    let input_tokens = parse_waggle_tokens(&state.cli_mode.user_write_values[1]);

    let db_path = sai_link_engine::project::sai_path(".sai/sai-engine/sai-engine.db");
    let integrator = match sai_link_engine::waggle_integration::WaggleIntegrator::open(&db_path) {
        Ok(i) => i,
        Err(e) => {
            state.cli_mode.error = Some(format!("waggle DB 初期化失敗: {}", e));
            return;
        }
    };
    match sai_link_engine::user_write::register_user_output(
        &state.store,
        &integrator,
        run_id,
        &source,
        if description.is_empty() {
            None
        } else {
            Some(description.as_str())
        },
        &input_tokens,
    ) {
        Ok(out) => {
            state.cli_mode.close();
            state.set_message(format!(
                "✅ ID{} に登録: {} ({})",
                out.exec_id,
                out.saved_path.display(),
                out.token
            ));
            state.reload_data();
        }
        Err(e) => {
            state.cli_mode.error = Some(format!("{}", e));
        }
    }
}

/// 入力パスを解決する（相対はプロジェクトルート基準）。
fn resolve_input_path(input: &str) -> PathBuf {
    let p = PathBuf::from(input);
    if p.is_absolute() {
        p
    } else {
        sai_link_engine::project::project_root().join(p)
    }
}

/// waggle 欄をトークン配列へ正規化する（カンマ/空白区切り・`waggle:` 接頭辞除去）。
fn parse_waggle_tokens(input: &str) -> Vec<String> {
    input
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(|s| s.trim().trim_start_matches("waggle:").to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

// ============================================================================
// run フロー（`sai-link block run`・task 駆動・即実行・フォーム廃止）
// ============================================================================

/// `e`: 固定済み task.md から即実行する（入力フォームなし）。
///
/// - `runner: pi` → task.md 値＋block.md デフォルトで確定し、デーモンへ実行要求
/// - `runner: user` → デーモン要求なし・ブロック定義を参照せず
///   run フォルダを `todo` → `running` に移動するのみ（`__user__` ブロックに claim）
/// - `block:` 未記載 → エラー（`runner: user` を除く）
fn do_run_flow(state: &mut AppState) {
    // 固定済み task.md（`x` 押下時の選択）を取り出す
    let card_path = match state.cli_mode.card_path.clone() {
        Some(p) => p,
        None => {
            state.cli_mode.error =
                Some("run: select a todo card in the sidebar".to_string());
            return;
        }
    };
    let text = match std::fs::read_to_string(&card_path) {
        Ok(t) => t,
        Err(e) => {
            state.cli_mode.error = Some(format!("failed to read task.md: {}", e));
            return;
        }
    };
    let card = match sai_link_engine::kanban::parse_card(&text) {
        Some(c) => c,
        None => {
            state.cli_mode.error = Some("failed to parse task.md frontmatter".to_string());
            return;
        }
    };
    // `runner: gl` → デーモン要求なし・herdr ペインで `gl` を開くのみ。
    // sai-gemini 側で会話するためタスク書き込み・DB claim は行わない。
    // run フォルダは user と同様に todo → running へ移動する。
    if card.frontmatter.runner.trim().eq_ignore_ascii_case("gl") {
        let run_id = match card.frontmatter.run_id.trim().parse::<i64>() {
            Ok(v) => v,
            Err(_) => {
                state.cli_mode.error = Some("task.md `run_id:` is invalid".to_string());
                return;
            }
        };
        let _ = state.store.update_execution_runner(run_id, "gl");
        let round = match parse_round_field(&card.frontmatter.round, "round") {
            Ok(v) => v,
            Err(e) => {
                state.cli_mode.error = Some(e);
                return;
            }
        };
        let global_round = match parse_round_field(&card.frontmatter.global_round, "global_round") {
            Ok(v) => v,
            Err(e) => {
                state.cli_mode.error = Some(e);
                return;
            }
        };
        if round.is_some() || global_round.is_some() {
            if let Err(e) = state
                .store
                .update_execution_rounds(run_id, round, global_round)
            {
                state.cli_mode.error = Some(format!("failed to update rounds: {}", e));
                return;
            }
        }
        if let Err(e) = state.store.update_execution_status(run_id, "running") {
            state.cli_mode.error = Some(format!("failed to update status: {}", e));
            return;
        }
        if let Some((from, _)) = sai_link_engine::run_dir::find_run_dir(run_id) {
            if from != sai_link_engine::run_dir::RUNNING_OWNER {
                if let Err(e) = sai_link_engine::run_dir::move_run_dir(
                    run_id,
                    &from,
                    sai_link_engine::run_dir::RUNNING_OWNER,
                ) {
                    let _ = state.store.update_execution_status(run_id, "todo");
                    state.cli_mode.error =
                        Some(format!("failed to move folder (rolled back): {}", e));
                    return;
                }
            }
        }
        super::external::open_gl_session(state);
        state.cli_mode.close();
        state.reload_data();
        return;
    }
    // `runner: user` → デーモン要求なし・ブロック定義は参照しない（pi 実行しないため）。
    // frontmatter の block/thinking/model/provider/inputs は不要。
    if card.frontmatter.is_user_runner() {
        let run_id = match card.frontmatter.run_id.trim().parse::<i64>() {
            Ok(v) => v,
            Err(_) => {
                state.cli_mode.error = Some("task.md `run_id:` is invalid".to_string());
                return;
            }
        };
        // user タスクはプレースホルダーブロック `__user__`（表示名 `user`）に紐付ける。
        let block_id = match state.store.get_or_create_user_block() {
            Ok(id) => id,
            Err(e) => {
                state.cli_mode.error = Some(format!("failed to resolve user block: {}", e));
                return;
            }
        };
        if let Err(e) = state.store.claim_reserved_run(run_id, block_id, None) {
            state.cli_mode.error = Some(format!("failed to claim reserved run: {}", e));
            return;
        }
        let _ = state.store.update_execution_runner(run_id, "user");
        // task.md frontmatter の round / global_round を DB に反映する。
        // 未指定なら変更しない（sidebar が R?/r? のままになるのは許容）。
        let round = match parse_round_field(&card.frontmatter.round, "round") {
            Ok(v) => v,
            Err(e) => {
                state.cli_mode.error = Some(e);
                return;
            }
        };
        let global_round = match parse_round_field(&card.frontmatter.global_round, "global_round") {
            Ok(v) => v,
            Err(e) => {
                state.cli_mode.error = Some(e);
                return;
            }
        };
        if round.is_some() || global_round.is_some() {
            if let Err(e) = state.store.update_execution_rounds(run_id, round, global_round) {
                state.cli_mode.error = Some(format!("failed to update rounds: {}", e));
                return;
            }
        }
        if let Err(e) = state.store.update_execution_status(run_id, "running") {
            state.cli_mode.error = Some(format!("failed to update status: {}", e));
            return;
        }
        if let Some((from, _)) = sai_link_engine::run_dir::find_run_dir(run_id) {
            if from != sai_link_engine::run_dir::RUNNING_OWNER {
                if let Err(e) = sai_link_engine::run_dir::move_run_dir(
                    run_id,
                    &from,
                    sai_link_engine::run_dir::RUNNING_OWNER,
                ) {
                    let _ = state.store.update_execution_status(run_id, "todo");
                    state.cli_mode.error =
                        Some(format!("failed to move folder (rolled back): {}", e));
                    return;
                }
            }
        }
        state.cli_mode.close();
        state.set_message(format!("ID{} moved to running (user task)", run_id));
        state.reload_data();
        return;
    }
    // ブロック解決：task.md の `block:` を使用（未記載はエラー）
    let block_selector = card.frontmatter.block.trim().to_string();
    if block_selector.is_empty() {
        state.cli_mode.error = Some(
            "task.md `block:` is missing. Set `block: <block_id>`".to_string(),
        );
        return;
    }
    let def_path = match resolve_block_file(&block_selector) {
        Some(p) => p,
        None => {
            state.cli_mode.error = Some(format!("block definition not found: {}", block_selector));
            return;
        }
    };
    let parsed = match sai_link_engine::parser::parse_block_file(&def_path) {
        Ok(p) => p,
        Err(e) => {
            state.cli_mode.error = Some(format!("failed to parse block definition: {}", e));
            return;
        }
    };
    // 警告メッセージを表示
    if !parsed.warnings.is_empty() {
        let warning_msg = parsed.warnings.join(" ");
        state.set_message(format!("[警告] {}", warning_msg));
    }
    // `runner: pi` → task.md 値＋block.md デフォルトで確定し、デーモンへ実行要求
    let mut user_inputs =
        match sai_link_engine::kanban::resolve_card_inputs(&card, &HashMap::new()) {
            Ok(m) => m,
            Err(e) => {
                state.cli_mode.error = Some(e);
                return;
            }
        };
    let origin_csv = card.frontmatter.origin_run_id.trim().to_string();
    if !origin_csv.is_empty() && origin_csv != "-" {
        user_inputs.insert("origin_run_id".to_string(), origin_csv);
    }
    let non_empty = |s: &str| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    };
    // round / global_round は task.md 必須（block.md にデフォルトなし）。tag は任意。
    let round: i64 = match non_empty(&card.frontmatter.round) {
        Some(v) => match v.parse() {
            Ok(n) => n,
            Err(_) => {
                state.cli_mode.error = Some("task.md `round:` is not a number".to_string());
                return;
            }
        },
        None => {
            state.cli_mode.error = Some("task.md `round:` is missing".to_string());
            return;
        }
    };
    let global_round: i64 = match non_empty(&card.frontmatter.global_round) {
        Some(v) => match v.parse() {
            Ok(n) => n,
            Err(_) => {
                state.cli_mode.error =
                    Some("task.md `global_round:` is not a number".to_string());
                return;
            }
        },
        None => {
            state.cli_mode.error = Some("task.md `global_round:` is missing".to_string());
            return;
        }
    };
    let mut overrides = sai_link_engine::orchestrator::ExecutionOverrides::default();
    overrides.round = Some(round);
    overrides.global_round = Some(global_round);
    overrides.tag = non_empty(&card.frontmatter.tag);
    overrides.thinking =
        non_empty(&card.frontmatter.thinking).or_else(|| parsed.config.thinking.clone());
    overrides.model = non_empty(&card.frontmatter.model).or_else(|| parsed.config.model.clone());
    // provider も thinking / model と同様に block.md へフォールバックする。
    // task.md の `provider:` 空欄で block.md の provider（例: `open-router2`）が
    // 落ちると、エンジン側の model からの再解決が走るため（接頭辞誤解釈の温床）。
    overrides.provider = non_empty(&card.frontmatter.provider)
        .or_else(|| parsed.config.provider.clone());
    // timeout は廃止（block.md の値を使用。task.md では指定しない）。
    overrides.timeout = parsed.config.timeout;
    // 実行主体（cc/gl 等）。未記載は Pi（後方互換）。
    overrides.runner = sai_link_engine::runner::RunnerKind::parse(&card.frontmatter.runner);
    if let Err(e) = stamp_card_execution_date(&card_path) {
        state.cli_mode.error = Some(format!("failed to update card: {}", e));
        return;
    }
    let card_rel = card_relative_path(&card_path);
    let reserved_run_id: Option<i64> = card.frontmatter.run_id.trim().parse().ok();
    if let Err(e) = sai_link_engine::daemon::write_block_run_request_full(
        &def_path,
        &parsed.config.block_id().to_string(),
        &user_inputs,
        &overrides,
        card_rel.as_deref(),
        reserved_run_id,
    ) {
        state.cli_mode.error = Some(format!("failed to send run request: {}", e));
        return;
    }
    state.cli_mode.close();
    state.set_message(format!(
        "📨 run request sent: {} (card={})",
        parsed.config.display_name(),
        card_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    ));
    state.reload_data();
}

/// task.md frontmatter の round / global_round をパースする。
///
/// 空欄・`-` は `None`（未指定・変更なし）。数値以外はエラー文字列を返す。
fn parse_round_field(value: &str, label: &str) -> Result<Option<i64>, String> {
    let t = value.trim();
    if t.is_empty() || t == "-" {
        return Ok(None);
    }
    t.parse::<i64>()
        .map(Some)
        .map_err(|_| format!("task.md `{}:` is not a number: {}", label, t))
}

// ============================================================================
// rm フロー（`sai-link block rm` の TUI 再現）
// ============================================================================

/// ブロック名またはパスから定義ファイルを解決する（CLI と同等）。
fn resolve_block_file(selector: &str) -> Option<PathBuf> {
    let p = PathBuf::from(selector);
    if p.exists() {
        return Some(p);
    }
    let candidate =
        sai_link_engine::project::sai_path(".sai/blocks").join(format!("{}.md", selector));
    if candidate.exists() {
        return Some(candidate);
    }
    None
}

/// カードの `execution_date` を要求時刻で更新する（CLI と同等・一段階目）。
fn stamp_card_execution_date(
    card_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(card_path)?;
    let mut card = sai_link_engine::kanban::parse_card(&text)
        .ok_or("カード md の frontmatter をパースできません")?;
    let now = chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string();
    card.frontmatter.execution_date = now;
    std::fs::write(card_path, sai_link_engine::kanban::render_card(&card))?;
    Ok(())
}

/// カードパスのプロジェクト相対パスを返す。
fn card_relative_path(card_path: &std::path::Path) -> Option<String> {
    let root = sai_link_engine::project::project_root();
    card_path
        .strip_prefix(&root)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

/// rm フロー開始：sidebar 選択中の実行/todo カードを削除対象として確認ステップへ。
///
/// - 実行リーフ（DB 行あり）：カスケード削除＋run フォルダ削除（現行通り）
/// - todo カード（DB 行なし）：対応 md ファイルの削除のみ
fn start_delete_flow(state: &mut AppState) {
    // レーン一括対象では削除対象を 1 枚に決められないため対象外（`d` はカード 1 枚のみ）
    if state.cli_mode.target_lane.is_some() {
        state.cli_mode.error = Some("rm: select a card (not a lane)".to_string());
        return;
    }
    // `x` 押下時の固定選択のみを使う（`state.selected_exec_id` は todo カード
    // 選択時に前の実行を引きずる stale のため参照しない。別物削除の防止・Q5）。
    let exec_id = state.cli_mode.selected_exec_id;
    if let Some(id) = exec_id {
        let label = match state.store.get_execution_by_id(id) {
            Ok(exec) => {
                let name = state
                    .store
                    .get_block_by_id(exec.block_id)
                    .map(|b| b.name)
                    .unwrap_or_else(|_| format!("block#{}", exec.block_id));
                format!("#{} '{}' ({})", id, name, exec.status)
            }
            Err(_) => {
                state.cli_mode.error = Some(format!("rm: execution ID '{}' not found", id));
                return;
            }
        };
        state.cli_mode.delete_target = Some(id);
        state.cli_mode.delete_label = label;
        state.cli_mode.error = None;
        state.cli_mode.step = CliStep::ConfirmDelete;
        return;
    }
    // todo カード選択時：md ファイル削除の確認へ（exec_id は None のまま）
    let card_path = state.cli_mode.card_path.clone();
    if let Some(p) = card_path {
        state.cli_mode.delete_target = None;
        state.cli_mode.delete_label = format!("todo card '{}'", p.display());
        state.cli_mode.error = None;
        state.cli_mode.step = CliStep::ConfirmDelete;
        return;
    }
    state.cli_mode.error =
        Some("rm: select an execution leaf or todo card in the sidebar".to_string());
}

/// ConfirmDelete: `Enter` で削除確定・`Esc` で Menu に戻る。
fn handle_delete_key(state: &mut AppState, key: crossterm::event::KeyEvent) -> bool {
    match key.code {
        KeyCode::Enter => {
            do_delete(state);
            true
        }
        KeyCode::Esc => {
            state.cli_mode.step = CliStep::Menu;
            state.cli_mode.error = None;
            true
        }
        _ => true,
    }
}

/// 削除を実行する（CLI の `handle_block_rm` と同等）。
///
/// - 実行リーフ：カスケード削除＋run フォルダ削除
/// - todo カード：md ファイル削除のみ
fn do_delete(state: &mut AppState) {
    // todo カードのみ（exec_id なし）の場合はファイル削除のみ
    if state.cli_mode.delete_target.is_none() {
        if let Some(p) = state.cli_mode.card_path.clone() {
            if let Err(e) = std::fs::remove_file(&p) {
                state.cli_mode.error = Some(format!("rm: failed to delete file: {}", e));
                return;
            }
            // run フォルダ（予約 run の task.md）も併せて削除する。不在時は無視。
            if let Some(parent) = p.parent() {
                if parent.file_name().map(|n| n == "todo").unwrap_or(false) {
                    let _ = std::fs::remove_dir_all(parent);
                } else {
                    let _ = std::fs::remove_file(&p);
                }
            }
            state.cli_mode.delete_target = None;
            state.set_message(format!("🗑 todo card deleted: {}", p.display()));
            // 確定後は run-tui を閉じる。
            state.cli_mode.close();
            state.reload_data();
            return;
        }
        state.cli_mode.error = Some("rm: nothing to delete".to_string());
        return;
    }
    let Some(exec_id) = state.cli_mode.delete_target else {
        state.cli_mode.error = Some("rm: nothing to delete".to_string());
        return;
    };
    let summary = match state.store.delete_execution_cascade(exec_id) {
        Ok(s) => s,
        Err(e) => {
            state.cli_mode.error = Some(format!("rm: failed to delete: {}", e));
            return;
        }
    };
    // run フォルダ（task/review/message/session/成果物）も丸ごと削除する（R603・CLI と同等）。
    // 旧 review/message/reflection の残骸も併せて掃除する。不在時は無視。
    if let Err(e) = sai_link_engine::orchestrator::remove_run_dir_files(exec_id) {
        state.set_message(format!("warning: failed to delete run folder: {}", e));
    }
    if state.selected_exec_id == Some(exec_id) {
        state.selected_exec_id = None;
    }
    let mut msg = format!("🗑 execution #{} deleted.", exec_id);
    if summary.executions > 1 {
        msg.push_str(&format!(" (including {} sub-executions)", summary.executions - 1));
    }
    state.cli_mode.delete_target = None;
    state.set_message(msg);
    // 確定後は run-tui を閉じる。
    state.cli_mode.close();
    state.reload_data();
}

// ============================================================================
// 状態遷移（`x` run-tui の `e`）
// ============================================================================

/// `e`: 現在の状態に応じて 1 段階 status を進める（run-tui の統一移動キー）。
///
/// - todo（予約カード）: run（`do_run_flow`）
/// - running: review へ（`runner: pi` もエラー対応のため許可）
/// - review: done へ（`review_done=true`＋フォルダ移動）
/// - done: workflow へ（`workflow_moved=true`＋`R<gr>` へ移動）
///
/// run フォルダの owner を現在状態の正とし、フォルダが無い場合のみ DB から推定する。
fn do_e_transition(state: &mut AppState) {
    // `x` 押下時の選択で固定した実行 ID のみを使う（`state.selected_exec_id` は
    // todo カード選択時に前の実行を引きずるため参照しない）。
    let Some(id) = state.cli_mode.selected_exec_id else {
        // 予約カード（DB 未 claim・exec_id なし）は run フローへ
        do_run_flow(state);
        return;
    };
    let owner = sai_link_engine::run_dir::find_run_dir(id).map(|(o, _)| o);
    if owner.as_deref() == Some(sai_link_engine::run_dir::TODO_OWNER) {
        do_run_flow(state);
        return;
    }
    let exec = match state.store.get_execution_by_id(id) {
        Ok(e) => e,
        Err(e) => {
            state.cli_mode.error = Some(format!("e: {}", e));
            return;
        }
    };
    let phase = owner.unwrap_or_else(|| {
        sai_link_engine::run_dir::owner_for(
            &exec.status,
            exec.review_done,
            exec.workflow_moved,
            exec.global_round,
        )
    });
    match phase.as_str() {
        "todo" => do_run_flow(state),
        "running" => move_running_to_review(state, id),
        "review" => move_review_to_done(state, id),
        "done" => move_done_to_workflow(state, id),
        other if other.starts_with('R') => {
            state.cli_mode.error = None;
            state.set_message(format!("ID{} is already in Workflow ({})", id, other));
        }
        _ => {
            state.cli_mode.error = Some(format!("e: unknown state '{}'", phase));
        }
    }
}

/// `e`: レーン（todo / running / review / done）対象の一括状態遷移。
///
/// そのレーンに存在する全てのカードを 1 段階進める。
///
/// - todo: 全件を実行開始
/// - running: 全件を review へ
/// - review: 全件を done へ
/// - done: 全件を workflow へ（`workflow_moved=true`）
///
/// 1 枚分の遷移は単一カード用の `do_e_transition` をそのまま使い、
/// 件数と失敗だけをここで集計する。
fn do_lane_transition(state: &mut AppState, lane: &str) {
    use sai_link_engine::kanban::KanbanLane;
    let lane_name = |l: KanbanLane| match l {
        KanbanLane::Todo => "todo",
        KanbanLane::Running => "running",
        KanbanLane::Review => "review",
        KanbanLane::Done => "done",
    };
    // 先頭にスナップショットを取る（実行中に `kanban_entries` が作り直されるため）
    let targets: Vec<(Option<i64>, Option<PathBuf>)> = state
        .kanban_entries
        .iter()
        .filter(|e| lane_name(e.lane) == lane)
        .map(|e| (e.exec_id, e.card_path.clone()))
        .collect();

    if targets.is_empty() {
        state.cli_mode.error = Some(format!("e: lane '{}' is empty", lane));
        return;
    }

    // 単一カード用の遷移を 1 枚ずつ適用（対象固定値を差し替えて呼ぶ）
    let saved_path = state.cli_mode.card_path.clone();
    let saved_exec = state.cli_mode.selected_exec_id;
    let mut ok = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for (exec_id, card_path) in targets {
        state.cli_mode.card_path = card_path;
        state.cli_mode.selected_exec_id = exec_id;
        state.cli_mode.error = None;
        do_e_transition(state);
        match state.cli_mode.error.take() {
            Some(e) => errors.push(e),
            None => ok += 1,
        }
    }
    // 対象固定値を元に戻す（直後にモードを閉じるが、エラー表示用に保持）
    state.cli_mode.card_path = saved_path;
    state.cli_mode.selected_exec_id = saved_exec;
    state.reload_data();

    if errors.is_empty() {
        state.set_message(format!("e: {} card(s) moved (lane {})", ok, lane));
    } else {
        state.cli_mode.error = Some(format!(
            "e: {}/{} moved (lane {}): {}",
            ok,
            ok + errors.len(),
            lane,
            errors.join("; ")
        ));
    }
}

/// running → review。DB `status=completed` にしてから run フォルダを移動する。
///
/// `runner: pi` でも使用可能（本来は pi-sai-finalize が担当するが、
/// finalize 失敗時のエラー対応として TUI から確定できる）。
///
/// U04: サブタスク（`parent_run_id` あり・未確定）は移動しない。
/// 子の pane は親の `task_manage(finalize)` / `delete` まで生きており、
/// `session/*.jsonl` へ書き続けるため、ここで `review` へ移すと
/// `ENOENT` で停止する。確定後に task_manager 側が移動する。
fn move_running_to_review(state: &mut AppState, id: i64) {
    // U04 ガード: 未確定サブタスクの明示移動を拒否する
    if let Ok(exec) = state.store.get_execution_by_id(id) {
        if exec.parent_run_id.is_some() {
            let confirmed = state
                .store
                .get_task_by_execution_id(id)
                .ok()
                .flatten()
                .map(|t| t.finalized || t.status == "finalized" || t.status == "deleted")
                .unwrap_or(false);
            if !confirmed {
                state.cli_mode.error = Some(format!(
                    "ID{} は実行中のサブタスクのため移動できません（親が task_manage finalize/delete で確定後に移動します）",
                    id
                ));
                return;
            }
        }
    }
    if let Err(e) = state.store.update_execution_status(id, "completed") {
        state.cli_mode.error = Some(format!("move failed: {}", e));
        return;
    }
    if let Some((from, _)) = sai_link_engine::run_dir::find_run_dir(id) {
        if from != sai_link_engine::run_dir::REVIEW_OWNER {
            if let Err(e) = sai_link_engine::run_dir::move_run_dir(
                id,
                &from,
                sai_link_engine::run_dir::REVIEW_OWNER,
            ) {
                let _ = state.store.update_execution_status(id, "running");
                state.cli_mode.error = Some(format!("move failed (rolled back): {}", e));
                return;
            }
        }
    }
    state.cli_mode.error = None;
    state.set_message(format!("ID{} moved to review", id));
    state.reload_data();
}

/// review → done。`review_done=true` にしてから run フォルダを移動する。
///
/// 実処理は [`sai_link_engine::run_dir::advance_review_to_done`]（TUI と
/// `sai-link phone advance` で共有・実装計画 A U10）。ここでは結果の表示だけを行う。
fn move_review_to_done(state: &mut AppState, id: i64) {
    if let Err(e) = sai_link_engine::run_dir::advance_review_to_done(&state.store, id) {
        state.cli_mode.error = Some(format!("move failed: {}", e));
        return;
    }
    state.cli_mode.error = None;
    state.set_message(format!("ID{} moved to done", id));
    state.reload_data();
}

/// done → workflow。`workflow_moved=true` にしてから `R<global_round>` へ移動する。
///
/// 実処理は [`sai_link_engine::run_dir::advance_done_to_workflow`]（TUI と
/// `sai-link phone advance` で共有・実装計画 A U10）。ここでは結果の表示だけを行う。
fn move_done_to_workflow(state: &mut AppState, id: i64) {
    if let Err(e) = sai_link_engine::run_dir::advance_done_to_workflow(&state.store, id) {
        state.cli_mode.error = Some(format!("move failed: {}", e));
        return;
    }
    state.cli_mode.error = None;
    state.set_message(format!("ID{} moved to Workflow", id));
    state.reload_data();
}

// ============================================================================
// 描画
// ============================================================================

/// CLI モードのオーバーレイを描画する（非アクティブ時は何もしない）。
/// `bg` は Workflow パネル（`MainPanel`）と同じ背景色（`AppTheme::main_bg`）。
pub fn render_cli_mode(cli: &CliModeState, area: Rect, buf: &mut Buffer, bg: Color) {
    if !cli.active {
        return;
    }
    // 背景は変更せず、画面中央のダイアログだけを描く（waggle 選択と同じ形式）。
    match cli.step {
        CliStep::Menu => render_menu(cli, area, buf, bg),
        CliStep::ConfirmDelete => render_delete_confirm(cli, area, buf, bg),
        CliStep::UserWrite => render_user_write(cli, area, buf, bg),
    }
}

/// メニュー（`e`=advance / `d`=rm / `o`=user write）。
///
/// waggle 選択と同じ形式（中央配置・背景変更なし）で、`↑↓`/`jk` で選択を移動し
/// `Enter` で確定する。確定後はモードが閉じる。
fn render_menu(cli: &CliModeState, area: Rect, buf: &mut Buffer, bg: Color) {
    use crate::ui::draw::draw_text_clipped;
    let actions = [
        "e : advance status (run -> review -> done -> workflow)",
        "d : block rm (delete confirm, todo allowed)",
        "o : register user output (runner=user, running only)",
    ];
    let w = 72u16.min(area.width.saturating_sub(4));
    let h = (actions.len() as u16 + 7 + u16::from(cli.error.is_some()))
        .clamp(8, area.height.saturating_sub(2));
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let dlg = Rect::new(x, y, w, h);
    crate::ui::modals::draw_dialog_frame(buf, dlg, " run-tui (x) ", Color::Yellow, bg);
    // 対象行（`x` 押下時の固定選択）。実行リーフは run_id、todo カードはパス。
    // stale な保持値は使わないため、表示と削除対象は常に一致する（Q5）。
    let target = match cli.target_lane.as_deref() {
        Some(lane) => format!("Target: lane '{}' (all cards)", lane),
        None => match cli.selected_exec_id {
            Some(id) => format!("Target: run_id {}", id),
            None => match cli.card_path.as_deref() {
                Some(pt) => format!("Target: {}", pt.display()),
                None => "Target: (none)".to_string(),
            },
        },
    };
    draw_text_clipped(
        buf,
        dlg,
        dlg.x + 2,
        dlg.y + 1,
        &target,
        Style::default().fg(Color::Cyan),
    );
    draw_text_clipped(
        buf,
        dlg,
        dlg.x + 2,
        dlg.y + 2,
        "Up/Down=move Enter=confirm Esc=close",
        Style::default().fg(Color::White),
    );
    // waggle 選択と同じ選択行ハイライト
    const SELECTED_ROW_BG: Color = Color::Rgb(40, 40, 20);
    for (i, text) in actions.iter().enumerate() {
        let cur = if i == cli.action_cursor { "> " } else { "  " };
        let style = if i == cli.action_cursor {
            Style::default()
                .fg(Color::Yellow)
                .bg(SELECTED_ROW_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        draw_text_clipped(
            buf,
            dlg,
            dlg.x + 2,
            dlg.y + 3 + i as u16,
            &format!("{}{}", cur, text),
            style,
        );
    }
    if let Some(e) = cli.error.as_deref() {
        let row = dlg.y + 3 + actions.len() as u16;
        if row < dlg.y + h.saturating_sub(2) {
            draw_text_clipped(
                buf,
                dlg,
                dlg.x + 2,
                row,
                &format!("⚠ {}", e),
                Style::default().fg(Color::Red),
            );
        }
    }
}

/// user-write フォームの描画。
///
/// `filepath` / `waggle` / `description` の 3 項目と送信行を表示する。
/// 編集中はバッファ＋カーソルを表示し、破壊的操作（移動）の注意書きを添える。
fn render_user_write(cli: &CliModeState, area: Rect, buf: &mut Buffer, bg: Color) {
    let labels = ["filepath", "waggle", "description"];
    let mut lines: Vec<(String, Color)> = Vec::new();
    if let Some(id) = cli.user_write_run_id {
        lines.push((
            format!("Target: execution #{} (user / running)", id),
            Color::Yellow,
        ));
    }
    lines.push((String::new(), Color::White));
    for (i, label) in labels.iter().enumerate() {
        let mark = if i == cli.user_write_focus { "> " } else { "  " };
        let value = if i == cli.user_write_focus {
            match cli.user_write_buf.as_ref() {
                Some(buf) => format!("{}_", buf),
                None => cli.user_write_values[i].clone(),
            }
        } else {
            cli.user_write_values[i].clone()
        };
        let color = if i == cli.user_write_focus {
            Color::Yellow
        } else {
            Color::White
        };
        lines.push((format!("{}{:<12}: {}", mark, label, value), color));
    }
    let send_mark = if cli.user_write_focus == 3 { "> " } else { "  " };
    let send_color = if cli.user_write_focus == 3 {
        Color::Green
    } else {
        Color::White
    };
    lines.push((format!("{}[ Enter = register ]", send_mark), send_color));
    lines.push((String::new(), Color::White));
    lines.push((
        "Source file is MOVED into the run folder.".to_string(),
        Color::Gray,
    ));
    lines.push((
        "Tab/Up/Down=move  Enter=edit/confirm  Esc=back".to_string(),
        Color::Gray,
    ));
    render_dialog(area, buf, " user write (o) ", &lines, cli.error.as_deref(), bg);
}

/// 削除確認。英語表記・ショートカット説明は枠内。
fn render_delete_confirm(cli: &CliModeState, area: Rect, buf: &mut Buffer, bg: Color) {
    let lines = vec![
        ((format!("Delete target: {}", cli.delete_label)), Color::White),
        (String::new(), Color::White),
        (
            "Deletes the DB row + run folder (task/review/message/session/artifacts)."
                .to_string(),
            Color::White,
        ),
        (
            "Child run folders are also deleted recursively (definitions remain)."
                .to_string(),
            Color::White,
        ),
        (String::new(), Color::White),
        ("Enter = delete   Esc = back".to_string(), Color::Gray),
    ];
    render_dialog(
        area,
        buf,
        " block rm: delete confirm ",
        &lines,
        cli.error.as_deref(),
        bg,
    );
}

/// 汎用ダイアログ（中央配置・長方形枠・エラー行）を描画する。
///
/// フッタ（下の枠線上のショートカット文字）は廃止。ショートカット説明は
/// 呼び出し側が本文行として枠内に含める。
fn render_dialog(
    area: Rect,
    buf: &mut Buffer,
    title: &str,
    lines: &[(String, Color)],
    error: Option<&str>,
    bg: Color,
) {
    use crate::ui::draw::draw_text_clipped;
    // 高さ = 枠 2 行 + タイトル直下の空き 1 行 + 本文 +（エラー時はエラー行）
    let body = lines.len() + if error.is_some() { 1 } else { 0 };
    let w = 72u16.min(area.width.saturating_sub(4));
    let h = (body as u16 + 4).clamp(6, area.height.saturating_sub(2));
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let dlg = Rect::new(x, y, w, h);
    crate::ui::modals::draw_dialog_frame(buf, dlg, title, Color::Yellow, bg);
    let mut row = dlg.y + 2;
    let bottom = dlg.y + dlg.height.saturating_sub(2);
    for (text, color) in lines.iter().take(h as usize - 4) {
        if row >= bottom {
            break;
        }
        if !text.is_empty() {
            draw_text_clipped(buf, dlg, dlg.x + 2, row, text, Style::default().fg(*color));
        }
        row += 1;
    }
    if let Some(e) = error {
        if row < bottom {
            draw_text_clipped(
                buf,
                dlg,
                dlg.x + 2,
                row,
                &format!("⚠ {}", e),
                Style::default().fg(Color::Red),
            );
        }
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use sai_link_engine::db::SaiStore;
    use std::sync::Arc;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }

    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }

    /// `x` で CLI モードに入り、`x`/`Esc` で終了すること。
    #[test]
    fn x_opens_and_closes_cli_mode() {
        let mut state = AppState::new(Arc::new(SaiStore::open_in_memory().unwrap()));
        state.view_mode = crate::ui::ViewMode::Workflow;
        assert!(super::super::input::handle_key_event(&mut state, key('x')));
        assert!(state.cli_mode.active);
        assert_eq!(state.cli_mode.step, CliStep::Menu);
        // Menu で `x` → モード終了
        assert!(super::super::input::handle_key_event(&mut state, key('x')));
        assert!(!state.cli_mode.active);
        // 再入場 → `Esc` でも終了
        assert!(super::super::input::handle_key_event(&mut state, key('x')));
        assert!(super::super::input::handle_key_event(&mut state, esc()));
        assert!(!state.cli_mode.active);
    }

    /// CLI モード中は通常 shortcut（旧 `d` の Workflow 移動）が無効化されること。
    #[test]
    fn cli_mode_swallow_normal_shortcuts() {
        let mut state = AppState::new(Arc::new(SaiStore::open_in_memory().unwrap()));
        state.view_mode = crate::ui::ViewMode::Workflow;
        assert!(super::super::input::handle_key_event(&mut state, key('x')));
        // Menu で `d` → Enter → rm フローの未選択エラー（Workflow 移動ではない）
        assert!(super::super::input::handle_key_event(&mut state, key('d')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert!(state.cli_mode.active, "モードは維持される");
        assert!(
            state.cli_mode.error.as_deref().unwrap_or("").contains("rm"),
            "rm フローのエラーになる: {:?}",
            state.cli_mode.error
        );
    }

    /// rm フロー：未選択ではエラー、選択中は確認→`Enter` で削除されること。
    #[test]
    fn rm_flow_deletes_selected_execution() {
        let _guard = super::super::file_test_lock();
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store
            .insert_block(&sai_link_engine::db::models::BlockMetadata::new(
                "b".to_string(),
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
                "blocks/b.md".to_string(),
                None
            ))
            .unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        let mut state = AppState::new(Arc::new(store));
        state.view_mode = crate::ui::ViewMode::Workflow;
        // 未選択で `d` → Enter → エラー（削除されない）
        assert!(super::super::input::handle_key_event(&mut state, key('x')));
        assert!(super::super::input::handle_key_event(&mut state, key('d')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert_eq!(state.cli_mode.step, CliStep::Menu);
        assert!(state.cli_mode.error.is_some());
        // 選択中は確認ステップへ
        state.selected_exec_id = Some(e1);
        state.cli_mode.selected_exec_id = Some(e1);
        assert!(super::super::input::handle_key_event(&mut state, key('d')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert_eq!(state.cli_mode.step, CliStep::ConfirmDelete);
        assert_eq!(state.cli_mode.delete_target, Some(e1));
        // `Esc` で Menu に戻る（削除されない）
        assert!(super::super::input::handle_key_event(&mut state, esc()));
        assert_eq!(state.cli_mode.step, CliStep::Menu);
        assert!(state.store.get_execution_by_id(e1).is_ok());
        // 再度 `d` → Enter（確認ステップ）→ Enter（削除確定）
        assert!(super::super::input::handle_key_event(&mut state, key('d')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert_eq!(state.cli_mode.step, CliStep::ConfirmDelete);
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert!(state.store.get_execution_by_id(e1).is_err(), "削除される");
        assert!(!state.cli_mode.active, "削除確定後は run-tui が閉じる");
        assert_eq!(state.cli_mode.step, CliStep::Menu);
    }

    /// `e`（run）：task.md 未選択では Menu にエラー表示されること。
    #[test]
    fn run_flow_without_card_shows_error() {
        let mut state = AppState::new(Arc::new(SaiStore::open_in_memory().unwrap()));
        state.view_mode = crate::ui::ViewMode::Workflow;
        assert!(super::super::input::handle_key_event(&mut state, key('x')));
        assert!(super::super::input::handle_key_event(&mut state, key('e')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert_eq!(state.cli_mode.step, CliStep::Menu, "未選択では Menu のまま");
        assert!(state.cli_mode.error.is_some());
    }

    /// レーン（sidebar の review フォルダ）を選択して `x` → `e` すると、
    /// そのレーンに存在する全てのカードが 1 段階進むこと（一括移動）。
    #[test]
    fn lane_x_then_e_moves_all_review_cards() {
        let _guard = super::super::file_test_lock();
        let tmp = std::env::temp_dir().join(format!(
            "sai_cli_lane_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".sai/sai-engine")).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);

        let store = Arc::new(
            SaiStore::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap(),
        );
        let block_id = store
            .insert_block(&sai_link_engine::db::models::BlockMetadata::new(
                "b".to_string(),
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
                "blocks/b.md".to_string(),
                None
            ))
            .unwrap();
        // review レーンに 2 枚
        let mut execs = Vec::new();
        for _ in 0..2 {
            let id = store.create_execution(block_id, None).unwrap();
            store
                .update_execution_overrides(id, None, Some(1), Some(1), None, None, None)
                .unwrap();
            store.update_execution_status(id, "completed").unwrap();
            execs.push(id);
        }

        let mut state = AppState::new(Arc::clone(&store));
        state.view_mode = crate::ui::ViewMode::Workflow;
        state.load_data();
        let idx = state
            .left_sidebar
            .tree_items
            .iter()
            .position(|t| t.is_folder() && t.name.contains("review"))
            .expect("sidebar に review フォルダがある");
        state.left_sidebar.selected_index = idx;

        // `x` → 対象がレーンになる
        assert!(super::super::input::handle_key_event(&mut state, key('x')));
        assert!(state.cli_mode.active);
        assert_eq!(
            state.cli_mode.target_lane.as_deref(),
            Some("review"),
            "レーンフォルダを選ぶと対象がレーンになる"
        );

        // `e` → レーン内の全カードが done へ
        assert!(super::super::input::handle_key_event(&mut state, key('e')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert!(state.cli_mode.error.is_none(), "{:?}", state.cli_mode.error);
        assert!(!state.cli_mode.active, "成功後は run-tui を閉じる");
        for id in &execs {
            let e = store.get_execution_by_id(*id).unwrap();
            assert!(e.review_done, "ID{} が done へ移動している", id);
        }

        // `d` はレーンでは使えない（カード 1 枚のときのみ有効）
        state.left_sidebar.selected_index = idx;
        assert!(super::super::input::handle_key_event(&mut state, key('x')));
        assert!(super::super::input::handle_key_event(&mut state, key('d')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert!(state
            .cli_mode
            .error
            .as_deref()
            .unwrap_or("")
            .contains("rm"));

        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }

    /// `runner: user` のカードは `block:` 未記載でもエラーにならず、
    /// `__user__` ブロックに紐付き todo → running へ移動すること（P3）。
    #[test]
    fn run_flow_user_runner_skips_block_resolution() {
        let _guard = crate::app::file_test_lock();
        let tmp = std::env::temp_dir().join(format!(
            "sai_cli_user_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".sai/sai-engine")).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);

        let store = Arc::new(SaiStore::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap());
        let run_id = store.create_reserved_run(None).unwrap();

        // `block:` 未記載・`runner: user` の task.md を outputs/todo に作る
        let dir = sai_link_engine::run_dir::ensure_run_dir(
            sai_link_engine::run_dir::TODO_OWNER,
            run_id,
        )
        .unwrap();
        let task = dir.join(sai_link_engine::run_dir::TASK_FILE);
        std::fs::write(
            &task,
            format!(
                "---\nrunner: user\ncreation_date: 2026/09/08 10:00:00\nexecution_date: -\nrun_id: {}\nround: 3\nglobal_round: 7\n---\n\n# task_text\n手動で確認する\n",
                run_id
            ),
        )
        .unwrap();

        let mut state = AppState::new(Arc::clone(&store));
        state.view_mode = crate::ui::ViewMode::Workflow;
        assert!(super::super::input::handle_key_event(&mut state, key('x')));
        // sidebar 選択を模擬（todo カードを固定）
        state.cli_mode.card_path = Some(task.clone());
        state.cli_mode.card = sai_link_engine::kanban::parse_card(
            &std::fs::read_to_string(&task).unwrap(),
        );
        assert!(super::super::input::handle_key_event(&mut state, key('e')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));

        assert!(
            state.cli_mode.error.is_none(),
            "ブロック解決エラーになってはならない: {:?}",
            state.cli_mode.error
        );
        let exec = state.store.get_execution_by_id(run_id).unwrap();
        assert_eq!(exec.status, "running");
        assert_eq!(exec.runner, "user");
        // task.md の round / global_round が DB に反映される（R?/r? 解消）
        assert_eq!(exec.round, Some(3), "round が反映される");
        assert_eq!(exec.global_round, Some(7), "global_round が反映される");
        let user_block = state.store.get_block_by_name("__user__").unwrap();
        assert_eq!(exec.block_id, user_block.id.unwrap(), "__user__ に紐付く");
        assert_eq!(
            user_block.display_name,
            "user",
            "Card は ID<run_id>/user 形式で表示される"
        );
        // run フォルダが running へ移動している
        assert_eq!(
            sai_link_engine::run_dir::find_run_dir(run_id).unwrap().0,
            sai_link_engine::run_dir::RUNNING_OWNER
        );

        sai_link_engine::run_dir::remove_run_dir(run_id).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }

    /// `e`（状態の移動）：running → review → done → workflow と 1 段階ずつ進むこと。
    #[test]
    fn e_advances_status_running_review_done_workflow() {
        let _guard = crate::app::file_test_lock();
        let tmp = std::env::temp_dir().join(format!(
            "sai_cli_e_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".sai/sai-engine")).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);

        let store = Arc::new(SaiStore::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap());
        let block_id = store
            .insert_block(&sai_link_engine::db::models::BlockMetadata::new(
                "b".to_string(),
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
                "blocks/b.md".to_string(),
                None
            ))
            .unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(e1, "running").unwrap();
        // run フォルダを running に用意する
        sai_link_engine::run_dir::ensure_run_dir(sai_link_engine::run_dir::RUNNING_OWNER, e1)
            .unwrap();

        let mut state = AppState::new(Arc::clone(&store));
        state.view_mode = crate::ui::ViewMode::Workflow;

        // running → review
        state.cli_mode.open_with_selection(None, None, Some(e1));
        assert!(super::super::input::handle_key_event(&mut state, key('e')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert!(
            !state.cli_mode.active,
            "確定後は run-tui が閉じる（連続押下で多重遷移しない）"
        );
        assert_eq!(
            sai_link_engine::run_dir::find_run_dir(e1).unwrap().0,
            sai_link_engine::run_dir::REVIEW_OWNER
        );
        assert_eq!(
            state.store.get_execution_by_id(e1).unwrap().status,
            "completed"
        );

        // review → done（DB `review_done` が立ち、フォルダも移動）
        state.cli_mode.open_with_selection(None, None, Some(e1));
        assert!(super::super::input::handle_key_event(&mut state, key('e')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert_eq!(
            sai_link_engine::run_dir::find_run_dir(e1).unwrap().0,
            sai_link_engine::run_dir::DONE_OWNER
        );
        assert!(state.store.get_execution_by_id(e1).unwrap().review_done);

        // done → workflow
        state.cli_mode.open_with_selection(None, None, Some(e1));
        assert!(super::super::input::handle_key_event(&mut state, key('e')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert!(state.store.get_execution_by_id(e1).unwrap().workflow_moved);
        assert_eq!(sai_link_engine::run_dir::find_run_dir(e1).unwrap().0, "R0");

        sai_link_engine::run_dir::remove_run_dir(e1).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }

    /// CLI モードの描画がパニックしないこと（各ステップ）。
    #[test]
    fn render_cli_mode_does_not_panic() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        for step in [CliStep::Menu, CliStep::ConfirmDelete, CliStep::UserWrite] {
            let mut cli = CliModeState::default();
            cli.active = true;
            cli.step = step;
            cli.delete_label = "#1 'b' (completed)".to_string();
            cli.user_write_run_id = Some(1);
            render_cli_mode(&cli, area, &mut buf, Color::Black);
        }
    }

    /// user/running の実行で `x`→`o` とするとフォームが開くこと。
    #[test]
    fn user_write_flow_opens_form() {
        let store = SaiStore::open_in_memory().unwrap();
        let user_block = store.get_or_create_user_block().unwrap();
        let run_id = store.create_reserved_run(None).unwrap();
        store.claim_reserved_run(run_id, user_block, None).unwrap();
        store.update_execution_runner(run_id, "user").unwrap();
        store.update_execution_status(run_id, "running").unwrap();

        let mut state = AppState::new(Arc::new(store));
        state.cli_mode.open_with_selection(None, None, Some(run_id));
        assert!(super::super::input::handle_key_event(&mut state, key('o')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert_eq!(state.cli_mode.step, CliStep::UserWrite);
        assert_eq!(state.cli_mode.user_write_run_id, Some(run_id));
        assert!(state.cli_mode.error.is_none());
    }

    /// runner=pi では `o` がフォームを開かずエラーになること。
    #[test]
    fn user_write_flow_rejects_non_user() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store
            .insert_block(&sai_link_engine::db::models::BlockMetadata::new(
                "b".to_string(),
                "t".to_string(),
                "m".to_string(),
                None,
                "off".to_string(),
                "true".to_string(),
                "true".to_string(),
                None,
                300,
                "none".to_string(),
                true,
                "blocks/b.md".to_string(),
                None
            ))
            .unwrap();
        let run_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(run_id, "running").unwrap();

        let mut state = AppState::new(Arc::new(store));
        state.cli_mode.open_with_selection(None, None, Some(run_id));
        assert!(super::super::input::handle_key_event(&mut state, key('o')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert_eq!(state.cli_mode.step, CliStep::Menu);
        assert!(state
            .cli_mode
            .error
            .as_deref()
            .unwrap_or("")
            .contains("not a user task"));
    }

    /// running 以外の user run でも `o` はフォームを開かないこと。
    #[test]
    fn user_write_flow_rejects_not_running() {
        let store = SaiStore::open_in_memory().unwrap();
        let user_block = store.get_or_create_user_block().unwrap();
        let run_id = store.create_reserved_run(None).unwrap();
        store.claim_reserved_run(run_id, user_block, None).unwrap();
        store.update_execution_runner(run_id, "user").unwrap();

        let mut state = AppState::new(Arc::new(store));
        state.cli_mode.open_with_selection(None, None, Some(run_id));
        assert!(super::super::input::handle_key_event(&mut state, key('o')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert_eq!(state.cli_mode.step, CliStep::Menu);
        assert!(state
            .cli_mode
            .error
            .as_deref()
            .unwrap_or("")
            .contains("not running"));
    }

    /// フォームの編集開始→入力→確定→送信行が動くこと。
    #[test]
    fn user_write_form_edit_flow() {
        let store = SaiStore::open_in_memory().unwrap();
        let user_block = store.get_or_create_user_block().unwrap();
        let run_id = store.create_reserved_run(None).unwrap();
        store.claim_reserved_run(run_id, user_block, None).unwrap();
        store.update_execution_runner(run_id, "user").unwrap();
        store.update_execution_status(run_id, "running").unwrap();

        let mut state = AppState::new(Arc::new(store));
        state.cli_mode.open_with_selection(None, None, Some(run_id));
        assert!(super::super::input::handle_key_event(&mut state, key('o')));
        assert!(super::super::input::handle_key_event(&mut state, enter()));

        // filepath を編集
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert!(state.cli_mode.user_write_buf.is_some());
        for c in "./docs/memo.md".chars() {
            assert!(super::super::input::handle_key_event(&mut state, key(c)));
        }
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert_eq!(state.cli_mode.user_write_values[0], "./docs/memo.md");

        // Tab で送信行へ → Enter（ファイルが無いのでエラー）
        for _ in 0..3 {
            super::super::input::handle_key_event(
                &mut state,
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            );
        }
        assert_eq!(state.cli_mode.user_write_focus, 3);
        assert!(super::super::input::handle_key_event(&mut state, enter()));
        assert!(state.cli_mode.error.is_some(), "存在しない filepath はエラー");
    }
}
