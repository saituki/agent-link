//! task.md 作成ウィザード（`Shift+C`）。
//!
//! `Modal::CardCreate` の中身。run-tui（`x`）と同じ見た目（中央配置・
//! 背景変更なし・角丸ダイアログ）で、段階入力フォームを順次表示する。
//!
//! ## ステップ
//!
//! ステップ番号は 7 固定の地図で、runner や定義の都合でスキップした
//! ステップは番号が飛ぶ（例: `user` / `gl` は Step2 → Step7）。
//!
//! | Step | ステップ | 内容 |
//! |---|---|----|
//! | 1 | `Runner` | `user` / `pi` / `cc` / `gl` を選択 |
//! | 2 | `Block` | 使用可能な block.md をリスト選択 |
//! | 3 | `DocSelect` | 最近の文書（`phone card docs` と同じ）から task_text に貼る参照を選ぶ |
//! | 4 | `Provider` | `runner=pi` のみ。pi の使用可能 provider をリスト選択（`cc` はスキップ） |
//! | 5 | `Model` | `runner=pi` は provider 配下の model をリスト選択。`cc` は opus/sonnet/haiku。`user` / `gl` は無し（自動スキップ） |
//! | 6 | `Thinking` | `none` / `low` / `medium` / `high` / `xhigh` / `max` を選択（実行時は pi → `--thinking`、cc → `--effort`） |
//! | 7 | `Rounds` | `global_round` / `round` / `tag` を同時入力フォームで入力 |
//!
//! `Done` は端末状態（到達しない）。
//!
//! ## 共通操作
//!
//! - 各項目はスキップ可能（`s` = この項目をスキップ）
//! - 前の入力フォームに戻れる（`Esc` / `Backspace` = 1 つ戻る・先頭では閉じる）
//! - `↑↓`/`j`/`k` = 移動・`Enter` = 確定・テキスト欄は `Enter`=編集開始→`Enter`=確定
//! - 最終確定で `outputs/todo/<run_id>/task.md` を作成し nvim で開く

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use super::AppState;

// 移設先（sai-link-widgets）から取り込む（TUI と CLI で同じ組み立て・検査を使う）。
use sai_link_widgets::card_wizard::{
    load_block_choices, load_model_choices, load_provider_choices, CC_MODELS, RUNNER_OPTIONS,
    THINKING_OPTIONS,
};

/// Step3 文書選択の候補1件（`recent_docs` と同じ中身・TUI 表示用）。
#[derive(Debug, Clone, Default)]
pub struct DocChoice {
    /// waggle トークン
    pub token: String,
    /// task.md への貼り付け行（`waggle:<tok>:path:.sai/outputs/*/<run_id>/<file>`）
    pub ref_line: String,
    /// 一覧表示行（`[status] file - description`）
    pub label: String,
}

/// ラウンド入力フォームの項目数（global_round / round / tag）。
pub const ROUND_ROWS: usize = 3;
/// ラウンド入力フォームのラベル。
pub const ROUND_LABELS: [&str; ROUND_ROWS] = ["global_round", "round", "tag"];

/// ウィザードの現在ステップ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WizardStep {
    /// runner 選択（Step1）
    #[default]
    Runner,
    /// block 選択（Step2）
    Block,
    /// 文書選択（Step3）。最近の文書から task_text へ貼る候補を選ぶ
    DocSelect,
    /// provider 選択（pi のみ・Step4）
    Provider,
    /// model 選択（pi / cc。user はスキップ・Step5）
    Model,
    /// thinking 選択（Step6）
    Thinking,
    /// global_round / round / tag の同時入力（Step7）
    Rounds,
    /// 端末状態（到達しない）
    Done,
}

/// task.md 作成ウィザードの状態（`Modal::CardCreate` の中身）。
#[derive(Debug, Clone, Default)]
pub struct CardWizardState {
    /// 現在ステップ
    pub step: WizardStep,
    /// エラー表示（ダイアログ下部・枠内折り返し）
    pub error: Option<String>,
    /// 警告表示（ダイアログ下部・枠内折り返し・モーダルを閉じると消える）
    ///
    /// `settings.json` の読み込み警告など。stderr に出すと枠外に残るため
    /// ここに溜めて枠内描画する。
    pub warnings: Vec<String>,
    // ── runner ──
    /// runner カーソル（`RUNNER_OPTIONS` の index）
    pub runner_cursor: usize,
    /// 確定済み runner（`user` / `pi` / `cc`。未確定・スキップは空）
    pub runner: String,
    // ── block ──
    /// 使用可能なブロック一覧（`(表示名, block_id)`・表示名昇順）
    pub blocks: Vec<(String, String)>,
    /// block カーソル
    pub block_cursor: usize,
    /// 確定済み block_id（未確定・スキップは空）
    pub block: String,
    // ── provider ──
    /// 使用可能な provider 一覧（`models.json` 由来・昇順）
    pub providers: Vec<String>,
    /// provider カーソル
    pub provider_cursor: usize,
    /// 確定済み provider（未確定・スキップは空）
    pub provider: String,
    // ── model ──
    /// 選択可能な model 一覧（provider 配下・または cc 固定）
    pub models: Vec<String>,
    /// model カーソル
    pub model_cursor: usize,
    /// 確定済み model（未確定・スキップは空）
    pub model: String,
    // ── block 定義の保持 ──
    /// 選択中 block.md の `thinking`（Step6 の既定選択に使う）
    pub block_thinking: String,
    // ── Step6: thinking ──
    /// thinking カーソル（`THINKING_OPTIONS` の index）
    pub thinking_cursor: usize,
    /// 確定済み thinking（空 = スキップ）
    pub thinking: String,
    // ── Step3: 文書選択 ──
    /// 候補一覧（最近の文書。`phone_view::recent_docs` と同じ並び）
    pub doc_choices: Vec<DocChoice>,
    /// 文書カーソル
    pub doc_cursor: usize,
    /// 文書選択フラグ（複数可）
    pub doc_checked: Vec<bool>,
    // ── rounds ──
    /// 入力値 `[global_round, round, tag]`
    pub rounds: [String; ROUND_ROWS],
    /// フォーカス（0..ROUND_ROWS + 送信行）
    pub rounds_focus: usize,
    /// 編集中バッファ（`Some` で編集モード）
    pub rounds_buf: Option<String>,
}

impl CardWizardState {
    /// 新規ウィザード（runner 選択から開始）。
    pub fn new() -> Self {
        Self {
            step: WizardStep::Runner,
            error: None,
            runner_cursor: 1,
            ..Default::default()
        }
    }

    /// 選択済み文書の貼り付け行（`waggle:<tok>:path:…` 形式・複数可）。
    pub fn selected_doc_refs(&self) -> Vec<String> {
        self.doc_choices
            .iter()
            .enumerate()
            .filter(|(i, _)| self.doc_checked.get(*i).copied().unwrap_or(false))
            .map(|(_, c)| c.ref_line.clone())
            .collect()
    }

    /// 現在ステップの表示番号（1 始まり・デバッグ用）。
    pub fn step_no(&self) -> usize {
        match self.step {
            WizardStep::Runner => 1,
            WizardStep::Block => 2,
            WizardStep::DocSelect => 3,
            WizardStep::Provider => 4,
            WizardStep::Model => 5,
            WizardStep::Thinking => 6,
            WizardStep::Rounds => 7,
            WizardStep::Done => 8,
        }
    }
}

// ============================================================================
// ウィザード開始・進行
// ============================================================================

/// `Shift+C`: カード作成ウィザードを開く。
///
/// runner / block / 文書 / provider / model / rounds を段階入力で尋ねる。
pub(crate) fn open_card_wizard(state: &mut AppState) {
    let mut wizard = CardWizardState::new();
    // settings.json の読み込み警告は stderr に出さず（枠外に残さず）、
    // ウィザードの間だけダイアログ下部に表示する。
    let (_, warnings) = sai_link_engine::pi_command::load_settings_quiet();
    wizard.warnings = warnings;
    state.modal = super::Modal::CardCreate(wizard);
}

/// ウィザード内のキー処理。処理した場合は `true` を返す。
pub(crate) fn handle_card_wizard_key(state: &mut AppState, key: KeyEvent) -> bool {
    // modal を一旦取り出す（借用競合回避）。確定・Esc 以外では戻す。
    let super::Modal::CardCreate(mut wizard) =
        std::mem::replace(&mut state.modal, super::Modal::None)
    else {
        return true;
    };
    let outcome = handle_wizard_key_inner(state, key, &mut wizard);
    match outcome {
        WizardOutcome::Keep => {
            state.modal = super::Modal::CardCreate(wizard);
        }
        WizardOutcome::Closed => {}
    }
    true
}

/// モーダル継続/終了の判定。
#[derive(PartialEq, Debug)]
enum WizardOutcome {
    /// modal を維持する
    Keep,
    /// modal を閉じた（確定・Esc）
    Closed,
}

/// ウィザード内キーの実体（ステップ別に振り分ける）。
fn handle_wizard_key_inner(
    state: &mut AppState,
    key: KeyEvent,
    wizard: &mut CardWizardState,
) -> WizardOutcome {
    match wizard.step {
        WizardStep::Runner => handle_runner_key(wizard, key),
        WizardStep::Block => handle_block_key(state, key, wizard),
        WizardStep::DocSelect => handle_doc_select_key(key, wizard),
        WizardStep::Provider => handle_provider_key(wizard, key),
        WizardStep::Model => handle_model_key(wizard, key),
        WizardStep::Thinking => handle_thinking_key(wizard, key),
        WizardStep::Rounds => handle_rounds_key(state, key, wizard),
        WizardStep::Done => WizardOutcome::Closed,
    }
}

/// 共通: `s` でスキップ（次のステップへ・値は空のまま）。
fn is_skip_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('s' | 'S'))
}

/// 共通: `Esc`/`Backspace` で 1 つ前に戻る。
fn is_back_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Esc | KeyCode::Backspace)
}

// ── runner 選択 ──────────────────────────────────────────────

/// runner 選択: `↑↓`/`jk`=移動・`Enter`=確定→次へ・`s`=スキップ（空=pi 扱い）・`Esc`=閉じる。
fn handle_runner_key(wizard: &mut CardWizardState, key: KeyEvent) -> WizardOutcome {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            wizard.runner_cursor = wizard.runner_cursor.saturating_sub(1);
            WizardOutcome::Keep
        }
        KeyCode::Down | KeyCode::Char('j') => {
            wizard.runner_cursor = (wizard.runner_cursor + 1).min(RUNNER_OPTIONS.len() - 1);
            WizardOutcome::Keep
        }
        KeyCode::Enter => {
            wizard.runner = RUNNER_OPTIONS
                .get(wizard.runner_cursor)
                .unwrap_or(&"pi")
                .to_string();
            advance_from_runner(wizard);
            WizardOutcome::Keep
        }
        _ if is_skip_key(key) => {
            wizard.runner.clear();
            advance_from_runner(wizard);
            WizardOutcome::Keep
        }
        _ if is_back_key(key) => WizardOutcome::Closed,
        _ => WizardOutcome::Keep,
    }
}

/// runner 確定後の遷移（block 一覧を読み込んで Block へ）。
fn advance_from_runner(wizard: &mut CardWizardState) {
    wizard.blocks = load_block_choices();
    wizard.block_cursor = 0;
    wizard.error = None;
    wizard.step = WizardStep::Block;
}

// ── block 選択 ───────────────────────────────────────────────

/// block 選択: `↑↓`/`jk`=移動・`Enter`=確定→Step3へ・`s`=スキップ（空）・`Esc`/`Backspace`=Runner へ戻る。
///
/// state が必要なため呼び出し側で AppState を渡す（Step3 候補組み立て用）。
fn handle_block_key(state: &AppState, key: KeyEvent, wizard: &mut CardWizardState) -> WizardOutcome {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            wizard.block_cursor = wizard.block_cursor.saturating_sub(1);
            WizardOutcome::Keep
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !wizard.blocks.is_empty() {
                wizard.block_cursor = (wizard.block_cursor + 1).min(wizard.blocks.len() - 1);
            }
            WizardOutcome::Keep
        }
        KeyCode::Enter => {
            wizard.block = wizard
                .blocks
                .get(wizard.block_cursor)
                .map(|(_, id)| id.clone())
                .unwrap_or_default();
            advance_from_block(state, wizard);
            WizardOutcome::Keep
        }
        _ if is_skip_key(key) => {
            wizard.block.clear();
            advance_from_block(state, wizard);
            WizardOutcome::Keep
        }
        _ if is_back_key(key) => {
            wizard.step = WizardStep::Runner;
            wizard.error = None;
            WizardOutcome::Keep
        }
        _ => WizardOutcome::Keep,
    }
}

/// Step3 文書選択の候補を組み立てる（Web の「最近の文書」と同じ `recent_docs`）。
///
/// 候補が空でも Step3 自体は表示し、Enter/s で次へ進める（文書なしカードを許容）。
fn load_doc_choices(state: &AppState, wizard: &mut CardWizardState) {
    let merged: Vec<DocChoice> = sai_link_widgets::phone_view::recent_docs(&state.store, 40)
        .iter()
        .map(|c| DocChoice {
            token: c.token.clone(),
            // 参照の <run_id> は文書を保存した実行の番号
            ref_line: sai_link_widgets::phone_view::doc_ref_line(c),
            label: format!(
                "[{}] {}/{} - {}",
                if c.status.is_empty() { "?" } else { &c.status },
                c.token,
                c.file_name,
                c.description
            ),
        })
        .collect();
    wizard.doc_choices = merged;
    wizard.doc_cursor = 0;
    wizard.doc_checked = vec![false; wizard.doc_choices.len()];
}

/// block 確定後の遷移（runner で分岐）。
///
/// - Step3 文書選択を必ず経由（文書なしでも Enter/s で次へ）
/// - `user` / `gl` → Step4〜Step6 はスキップで Rounds へ
///   （gl は起動時のタスク書き込みなし・空セッションを開くだけ）
/// - `cc` → Model（provider なし）
/// - `pi`（既定・空含む）→ Provider
fn advance_from_block(state: &AppState, wizard: &mut CardWizardState) {
    wizard.error = None;
    prepare_block_thinking(wizard);
    // Step3: 文書選択へ（候補は最近の文書。空でも次へ進める）
    load_doc_choices(state, wizard);
    wizard.step = WizardStep::DocSelect;
    wizard.error = None;
    return;
}

/// Step3 の次へ進む（runner で分岐・旧 advance_from_block の後半）。
fn advance_from_doc(wizard: &mut CardWizardState) {
    wizard.error = None;
    match wizard.runner.trim() {
        "user" | "gl" => {
            wizard.provider.clear();
            wizard.model.clear();
            wizard.step = WizardStep::Rounds;
        }
        "cc" => {
            wizard.provider.clear();
            wizard.models = CC_MODELS.iter().map(|s| s.to_string()).collect();
            wizard.model_cursor = 0;
            wizard.step = WizardStep::Model;
        }
        _ => {
            wizard.providers = load_provider_choices();
            wizard.provider_cursor = 0;
            wizard.step = WizardStep::Provider;
        }
    }
}

/// Step3 文書選択: `↑↓`/`jk`=移動・`Space`=選択切替（複数可）・`Enter`=確定→次へ・
/// `s`=スキップ（文書なし）・`Esc`/`Backspace`=Block へ戻る。
///
/// 候補は origin 先頭＋全体（実行中含む）。選択行は task.md の `# task_text` 先頭に
/// `waggle:<tok>:path:…` 形式で貼られる（実行時に本文展開される）。
fn handle_doc_select_key(key: KeyEvent, wizard: &mut CardWizardState) -> WizardOutcome {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            wizard.doc_cursor = wizard.doc_cursor.saturating_sub(1);
            WizardOutcome::Keep
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !wizard.doc_choices.is_empty() {
                wizard.doc_cursor = (wizard.doc_cursor + 1).min(wizard.doc_choices.len() - 1);
            }
            WizardOutcome::Keep
        }
        KeyCode::Char(' ') => {
            if let Some(v) = wizard.doc_checked.get_mut(wizard.doc_cursor) {
                *v = !*v;
            }
            WizardOutcome::Keep
        }
        KeyCode::Enter => {
            advance_from_doc(wizard);
            WizardOutcome::Keep
        }
        _ if is_skip_key(key) => {
            wizard.doc_checked = vec![false; wizard.doc_choices.len()];
            advance_from_doc(wizard);
            WizardOutcome::Keep
        }
        _ if is_back_key(key) => {
            wizard.step = WizardStep::Block;
            wizard.error = None;
            WizardOutcome::Keep
        }
        _ => WizardOutcome::Keep,
    }
}

/// 選択中 block.md の `thinking` を保持する（Step6 の既定選択に使う）。
///
/// ブロック未選択・定義が読めない場合は空のまま。
fn prepare_block_thinking(wizard: &mut CardWizardState) {
    wizard.block_thinking.clear();
    wizard.block_thinking =
        sai_link_widgets::card_wizard::block_thinking(&wizard.runner, &wizard.block);
}


// ── provider 選択（pi のみ） ──────────────────────────────────

/// provider 選択: `↑↓`/`jk`=移動・`Enter`=確定→Model へ・`s`=スキップ→Model へ・`Esc`/`Backspace`=Block へ戻る。
fn handle_provider_key(wizard: &mut CardWizardState, key: KeyEvent) -> WizardOutcome {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            wizard.provider_cursor = wizard.provider_cursor.saturating_sub(1);
            WizardOutcome::Keep
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !wizard.providers.is_empty() {
                wizard.provider_cursor =
                    (wizard.provider_cursor + 1).min(wizard.providers.len() - 1);
            }
            WizardOutcome::Keep
        }
        KeyCode::Enter => {
            wizard.provider = wizard
                .providers
                .get(wizard.provider_cursor)
                .cloned()
                .unwrap_or_default();
            wizard.models = load_model_choices(&wizard.provider);
            wizard.model_cursor = 0;
            wizard.error = None;
            wizard.step = WizardStep::Model;
            WizardOutcome::Keep
        }
        _ if is_skip_key(key) => {
            // この項目（provider）だけをスキップして Model へ
            wizard.provider.clear();
            wizard.error = None;
            wizard.step = WizardStep::Model;
            WizardOutcome::Keep
        }
        _ if is_back_key(key) => {
            // Step3 の前に戻る（Block ではなく DocSelect）
            wizard.step = WizardStep::DocSelect;
            wizard.error = None;
            WizardOutcome::Keep
        }
        _ => WizardOutcome::Keep,
    }
}

// ── model 選択 ───────────────────────────────────────────────

/// model 選択: `↑↓`/`jk`=移動・`Enter`=確定→Thinking へ・`s`=スキップ（空）・`Esc`/`Backspace`=1 つ前へ戻る。
///
/// 戻り先は Step3 が入ったため DocSelect ではなく Provider/cc は DocSelect（旧 Block 相当）。
fn handle_model_key(wizard: &mut CardWizardState, key: KeyEvent) -> WizardOutcome {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            wizard.model_cursor = wizard.model_cursor.saturating_sub(1);
            WizardOutcome::Keep
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !wizard.models.is_empty() {
                wizard.model_cursor = (wizard.model_cursor + 1).min(wizard.models.len() - 1);
            }
            WizardOutcome::Keep
        }
        KeyCode::Enter => {
            wizard.model = wizard
                .models
                .get(wizard.model_cursor)
                .cloned()
                .unwrap_or_default();
            wizard.error = None;
            enter_thinking(wizard);
            wizard.step = WizardStep::Thinking;
            WizardOutcome::Keep
        }
        _ if is_skip_key(key) => {
            wizard.model.clear();
            wizard.error = None;
            enter_thinking(wizard);
            wizard.step = WizardStep::Thinking;
            WizardOutcome::Keep
        }
        _ if is_back_key(key) => {
            wizard.error = None;
            // 文書選択の挿入前は cc→Block・pi→Provider に戻っていた。挿入後は
            // モデル選択の前は Provider（pi）であり、cc は DocSelect が前段。
            // 戻り先は一律で前段に合わせる: pi→Provider、cc→DocSelect。
            if wizard.runner.trim() == "cc" {
                wizard.step = WizardStep::DocSelect;
            } else {
                wizard.step = WizardStep::Provider;
            }
            WizardOutcome::Keep
        }
        _ => WizardOutcome::Keep,
    }
}

// ── Step6: thinking ─────────────────────────────────

/// Step6 に入るときのカーソルを決める。
///
/// 既定は「確定済みの値 → block.md の `thinking` → settings.json → none」の順。
/// 旧値（`off` / `minimal`）は新 6 値に該当しないため none 扱い（先頭）になる。
fn enter_thinking(wizard: &mut CardWizardState) {
    let mut idx = 0usize;
    if !wizard.thinking.is_empty() {
        if let Some(i) = THINKING_OPTIONS
            .iter()
            .position(|v| *v == wizard.thinking.trim())
        {
            idx = i;
        }
        wizard.thinking_cursor = idx;
        return;
    }
    let (settings, _) = sai_link_engine::pi_command::load_settings_quiet();
    let candidates = [
        wizard.block_thinking.clone(),
        settings.default_thinking_level.unwrap_or_default(),
    ];
    for cand in candidates {
        let t = cand.trim().to_lowercase();
        if let Some(i) = THINKING_OPTIONS.iter().position(|v| *v == t) {
            idx = i;
            break;
        }
    }
    wizard.thinking_cursor = idx;
}

/// Step6 thinking: `↑↓`/`jk`=移動・`Enter`=確定→Rounds へ・`s`=スキップ・`Esc`=Model へ戻る。
fn handle_thinking_key(wizard: &mut CardWizardState, key: KeyEvent) -> WizardOutcome {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            wizard.thinking_cursor = wizard.thinking_cursor.saturating_sub(1);
            WizardOutcome::Keep
        }
        KeyCode::Down | KeyCode::Char('j') => {
            wizard.thinking_cursor =
                (wizard.thinking_cursor + 1).min(THINKING_OPTIONS.len() - 1);
            WizardOutcome::Keep
        }
        KeyCode::Enter => {
            wizard.thinking = THINKING_OPTIONS
                .get(wizard.thinking_cursor)
                .copied()
                .unwrap_or("none")
                .to_string();
            wizard.error = None;
            wizard.step = WizardStep::Rounds;
            WizardOutcome::Keep
        }
        _ if is_skip_key(key) => {
            // 空のまま = 従来通り settings.json / block.md の自動解決に任せる
            wizard.thinking.clear();
            wizard.error = None;
            wizard.step = WizardStep::Rounds;
            WizardOutcome::Keep
        }
        _ if is_back_key(key) => {
            wizard.error = None;
            wizard.step = WizardStep::Model;
            WizardOutcome::Keep
        }
        _ => WizardOutcome::Keep,
    }
}

// ── rounds 同時入力 ──────────────────────────────────────────

/// rounds 入力: `↑↓`/`Tab`=移動・`Enter`=編集開始/確定（送信行で作成）・`s`=スキップして作成・`Esc`=1 つ前へ戻る。
///
/// 戻り先は runner 別（`user` / `gl` → Block・`pi` / `cc` → Thinking）。
fn handle_rounds_key(
    state: &mut AppState,
    key: KeyEvent,
    wizard: &mut CardWizardState,
) -> WizardOutcome {
    // 編集中: `Enter`=確定 / `Esc`=取消 / `Backspace`・文字=編集
    if wizard.rounds_buf.is_some() {
        match key.code {
            KeyCode::Enter => {
                let idx = wizard.rounds_focus;
                if idx < ROUND_ROWS {
                    let buf = wizard.rounds_buf.take().unwrap_or_default();
                    wizard.rounds[idx] = buf;
                } else {
                    wizard.rounds_buf = None;
                }
                WizardOutcome::Keep
            }
            KeyCode::Esc => {
                wizard.rounds_buf = None;
                WizardOutcome::Keep
            }
            KeyCode::Backspace => {
                if let Some(buf) = wizard.rounds_buf.as_mut() {
                    buf.pop();
                }
                WizardOutcome::Keep
            }
            KeyCode::Char(c) => {
                if let Some(buf) = wizard.rounds_buf.as_mut() {
                    buf.push(c);
                }
                WizardOutcome::Keep
            }
            _ => WizardOutcome::Keep,
        }
    } else {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                if wizard.rounds_focus > 0 {
                    wizard.rounds_focus -= 1;
                }
                WizardOutcome::Keep
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                if wizard.rounds_focus < ROUND_ROWS {
                    wizard.rounds_focus += 1;
                }
                WizardOutcome::Keep
            }
            KeyCode::Enter => {
                if wizard.rounds_focus < ROUND_ROWS {
                    let idx = wizard.rounds_focus;
                    wizard.rounds_buf = Some(wizard.rounds[idx].clone());
                    WizardOutcome::Keep
                } else {
                    finish_wizard(state, wizard)
                }
            }
            _ if is_skip_key(key) => finish_wizard(state, wizard),
            _ if is_back_key(key) => {
                wizard.error = None;
                wizard.rounds_buf = None;
                // runner 別に戻り先を変える（user/gl → DocSelect・pi/cc → Thinking）
                match wizard.runner.trim() {
                    "user" | "gl" => wizard.step = WizardStep::DocSelect,
                    _ => {
                        enter_thinking(wizard);
                        wizard.step = WizardStep::Thinking;
                    }
                }
                WizardOutcome::Keep
            }
            _ => WizardOutcome::Keep,
        }
    }
}

/// 最終確定: ウィザード値で task.md を作成し nvim で開く。
fn finish_wizard(state: &mut AppState, wizard: &mut CardWizardState) -> WizardOutcome {
    match create_card_from_wizard(wizard) {
        Ok(path) => {
            state.reload_data();
            super::external::open_card(state, Some(path));
            WizardOutcome::Closed
        }
        Err(e) => {
            wizard.error = Some(e);
            WizardOutcome::Keep
        }
    }
}

// ============================================================================
// カード作成（ウィザード値の反映）
// ============================================================================

/// ウィザード値でカード md を作成する。
///
/// - `runner` / `block` / `provider` / `model` はウィザード確定値を優先する
///   （空・スキップ時は自動値: runner=pi・model/provider/thinking=settings.json）
/// - `thinking` は Step6 の確定値（空・スキップ時は自動値を維持）
/// - `global_round` / `round` / `tag` はウィザード入力のまま
fn create_card_from_wizard(wizard: &CardWizardState) -> Result<std::path::PathBuf, String> {
    // 組み立ては sai_link_widgets::card_wizard::create_card へ移設（`phone card create` と同一）。
    // Step3 で選んだ文書は task_text 先頭に waggle 参照行として貼る（実行時に本文展開）。
    // 未選択なら空のまま（作成後に nvim で書く従来通り）。
    let doc_refs = wizard.selected_doc_refs();
    let task_text = doc_refs.join("\n");
    let spec = sai_link_widgets::card_wizard::CardSpec {
        runner: wizard.runner.clone(),
        block: wizard.block.clone(),
        provider: wizard.provider.clone(),
        model: wizard.model.clone(),
        thinking: wizard.thinking.clone(),
        global_round: wizard.rounds[0].clone(),
        round: wizard.rounds[1].clone(),
        tag: wizard.rounds[2].clone(),
        task_text,
    };
    sai_link_widgets::card_wizard::create_card(&spec)
}


// ============================================================================
// 描画（run-tui と同じ見た目）
// ============================================================================

/// 選択行のハイライト背景（run-tui と統一）。
const SELECTED_ROW_BG: Color = Color::Rgb(40, 40, 20);

/// ウィザードのオーバーレイを描画する（run-tui と同じ見た目）。
///
/// 背景は変更せず、画面中央のダイアログだけを描く。
/// `bg` は Workflow パネルと同じ背景色。
pub fn render_card_wizard(wizard: &CardWizardState, area: Rect, buf: &mut Buffer, bg: Color) {
    match wizard.step {
        WizardStep::Runner => render_runner(wizard, area, buf, bg),
        WizardStep::Block => render_block(wizard, area, buf, bg),
        WizardStep::DocSelect => render_doc_select(wizard, area, buf, bg),
        WizardStep::Provider => render_provider(wizard, area, buf, bg),
        WizardStep::Model => render_model(wizard, area, buf, bg),
        WizardStep::Thinking => render_thinking(wizard, area, buf, bg),
        WizardStep::Rounds => render_rounds(wizard, area, buf, bg),
        WizardStep::Done => {}
    }
}

/// ステップ表示（`Step 2/7` 等・番号は 7 固定・スキップ分は番号が飛ぶ）。
fn step_label(wizard: &CardWizardState) -> String {
    let no = match wizard.step {
        WizardStep::Runner => 1,
        WizardStep::Block => 2,
        WizardStep::DocSelect => 3,
        WizardStep::Provider => 4,
        WizardStep::Model => 5,
        WizardStep::Thinking => 6,
        WizardStep::Rounds | WizardStep::Done => 7,
    };
    format!("Step {}/7", no)
}

/// 汎用リストダイアログ（run-tui のメニューと同じ見た目）。
///
/// `items` は表示行（`>` マーク付きで選択強調）。`footer` は操作ガイド。
/// エラーは枠内に折り返し全文表示し、行数に応じて高さを伸ばす。
fn render_list_dialog(
    area: Rect,
    buf: &mut Buffer,
    title: &str,
    step: &str,
    header: &[String],
    items: &[String],
    cursor: usize,
    footer: &str,
    warnings: &[String],
    error: Option<&str>,
    bg: Color,
) {
    use crate::ui::draw::draw_text_clipped;
    let w = 72u16.min(area.width.saturating_sub(4));
    let inner_w = w.saturating_sub(6) as usize;
    // 警告・エラーはいずれも枠内で折り返して下部に並べる（stderr には出さない）
    let warning_lines = wrap_warnings(warnings, inner_w);
    let error_lines: Vec<String> = match error {
        Some(e) => wrap_text_cjk(&format!("⚠ {}", e), inner_w.max(1)),
        None => Vec::new(),
    };
    let sep_warn = usize::from(!warning_lines.is_empty());
    let sep_err = usize::from(!error_lines.is_empty());
    // 本文行数 = ステップ行 1 + ヘッダ + 項目 + 空行 1 + フッタ 1 + 警告/エラー行
    let body_rows = 1 + header.len() + items.len() + 1 + 1 + sep_warn + warning_lines.len()
        + sep_err
        + error_lines.len();
    let max_h = area.height.saturating_sub(2).max(8);
    let h = ((body_rows as u16) + 4).clamp(8, max_h);
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let dlg = Rect::new(x, y, w, h);
    crate::ui::modals::draw_dialog_frame(buf, dlg, title, Color::Yellow, bg);
    let bottom = dlg.y + dlg.height.saturating_sub(1);
    let mut row = dlg.y + 2;
    // ステップ行
    if row < bottom {
        draw_text_clipped(
            buf,
            dlg,
            dlg.x + 2,
            row,
            step,
            Style::default().fg(Color::Cyan),
        );
        row += 1;
    }
    for line in header {
        if row >= bottom {
            break;
        }
        if !line.is_empty() {
            draw_text_clipped(buf, dlg, dlg.x + 2, row, line, Style::default().fg(Color::White));
        }
        row += 1;
    }
    for (i, text) in items.iter().enumerate() {
        if row >= bottom {
            break;
        }
        let cur = if i == cursor { "> " } else { "  " };
        let style = if i == cursor {
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
            row,
            &format!("{}{}", cur, text),
            style,
        );
        row += 1;
    }
    // フッタ前に空行を 1 行挟む
    if row < bottom {
        row += 1;
    }
    if row < bottom {
        draw_text_clipped(
            buf,
            dlg,
            dlg.x + 2,
            row,
            footer,
            Style::default().fg(Color::Gray),
        );
        row += 1;
    }
    if !warning_lines.is_empty() && row < bottom {
        row += 1;
    }
    for line in &warning_lines {
        if row >= bottom {
            break;
        }
        draw_text_clipped(
            buf,
            dlg,
            dlg.x + 2,
            row,
            line,
            Style::default().fg(Color::LightYellow),
        );
        row += 1;
    }
    if !error_lines.is_empty() && row < bottom {
        row += 1;
    }
    for line in error_lines {
        if row >= bottom {
            break;
        }
        draw_text_clipped(
            buf,
            dlg,
            dlg.x + 2,
            row,
            &line,
            Style::default().fg(Color::Red),
        );
        row += 1;
    }
}

/// runner 選択ステップの描画。
fn render_runner(wizard: &CardWizardState, area: Rect, buf: &mut Buffer, bg: Color) {
    let items: Vec<String> = RUNNER_OPTIONS.iter().map(|s| s.to_string()).collect();
    render_list_dialog(
        area,
        buf,
        " task create: runner ",
        &step_label(wizard),
        &["実行主体を選択".to_string()],
        &items,
        wizard.runner_cursor,
        "Up/Down=move Enter=next s=skip Esc=back",
        &wizard.warnings,
        wizard.error.as_deref(),
        bg,
    );
}

/// block 選択ステップの描画。
fn render_block(wizard: &CardWizardState, area: Rect, buf: &mut Buffer, bg: Color) {
    let header = if wizard.blocks.is_empty() {
        vec!["使用可能な block.md がありません（Enter=空のまま進む）".to_string()]
    } else {
        vec!["使用可能な block.md を選択".to_string()]
    };
    let items: Vec<String> = wizard
        .blocks
        .iter()
        .map(|(display, id)| format!("{} ({})", display, id))
        .collect();
    render_list_dialog(
        area,
        buf,
        " task create: block ",
        &step_label(wizard),
        &header,
        &items,
        wizard.block_cursor,
        "Up/Down=move Enter=next s=skip Esc=back",
        &wizard.warnings,
        wizard.error.as_deref(),
        bg,
    );
}

/// Step4 文書選択ステップの描画。
///
/// origin 先頭＋全体（実行中含む）の候補を複数選択する。選択行は task_text 先頭へ貼られる。
fn render_doc_select(wizard: &CardWizardState, area: Rect, buf: &mut Buffer, bg: Color) {
    let header = if wizard.doc_choices.is_empty() {
        vec!["文書がありません（Enter=文書なしで進む）".to_string()]
    } else {
        vec!["task_text に貼る文書を選択（複数可・実行中含む）".to_string()]
    };
    let items: Vec<String> = wizard
        .doc_choices
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mark = if wizard.doc_checked.get(i).copied().unwrap_or(false) {
                "[x]"
            } else {
                "[ ]"
            };
            format!("{} {}", mark, c.label)
        })
        .collect();
    render_list_dialog(
        area,
        buf,
        " task create: docs ",
        &step_label(wizard),
        &header,
        &items,
        wizard.doc_cursor,
        "Up/Down=move Space=toggle Enter=next s=skip Esc=back",
        &wizard.warnings,
        wizard.error.as_deref(),
        bg,
    );
}

/// provider 選択ステップの描画（pi のみ）。
fn render_provider(wizard: &CardWizardState, area: Rect, buf: &mut Buffer, bg: Color) {
    let header = if wizard.providers.is_empty() {
        vec!["使用可能な provider がありません（Enter=空のまま進む）".to_string()]
    } else {
        vec!["pi の provider を選択".to_string()]
    };
    let items: Vec<String> = wizard.providers.clone();
    render_list_dialog(
        area,
        buf,
        " task create: provider ",
        &step_label(wizard),
        &header,
        &items,
        wizard.provider_cursor,
        "Up/Down=move Enter=next s=skip Esc=back",
        &wizard.warnings,
        wizard.error.as_deref(),
        bg,
    );
}

/// model 選択ステップの描画。
fn render_model(wizard: &CardWizardState, area: Rect, buf: &mut Buffer, bg: Color) {
    let header = if wizard.runner.trim() == "cc" {
        vec!["cc のモデルを選択".to_string()]
    } else if wizard.models.is_empty() {
        vec!["使用可能な model がありません（Enter=空のまま進む）".to_string()]
    } else {
        vec![format!("provider `{}` のモデルを選択", wizard.provider)]
    };
    render_list_dialog(
        area,
        buf,
        " task create: model ",
        &step_label(wizard),
        &header,
        &wizard.models,
        wizard.model_cursor,
        "Up/Down=move Enter=next s=skip Esc=back",
        &wizard.warnings,
        wizard.error.as_deref(),
        bg,
    );
}

/// Step7 thinking ステップの描画。
fn render_thinking(wizard: &CardWizardState, area: Rect, buf: &mut Buffer, bg: Color) {
    let default_src = if wizard.block_thinking.trim().is_empty() {
        "settings.json".to_string()
    } else {
        wizard.block_thinking.trim().to_string()
    };
    let header = vec![
        "thinking を選択（実行時: pi=thinking / cc=effort）".to_string(),
        format!("block.md / settings.json の既定: {}", default_src),
    ];
    let items: Vec<String> = THINKING_OPTIONS.iter().map(|s| s.to_string()).collect();
    render_list_dialog(
        area,
        buf,
        " task create: thinking ",
        &step_label(wizard),
        &header,
        &items,
        wizard.thinking_cursor,
        "Up/Down=move Enter=next s=skip Esc=back",
        &wizard.warnings,
        wizard.error.as_deref(),
        bg,
    );
}

/// rounds 同時入力ステップの描画。
fn render_rounds(wizard: &CardWizardState, area: Rect, buf: &mut Buffer, bg: Color) {
    use crate::ui::draw::draw_text_clipped;
    let w = 72u16.min(area.width.saturating_sub(4));
    let inner_w = w.saturating_sub(6) as usize;
    // 警告・エラーは枠内の下部に並べ、枠内で折り返す
    let warning_lines = wrap_warnings(&wizard.warnings, inner_w);
    let error_lines: Vec<String> = match wizard.error.as_deref() {
        Some(e) => wrap_text_cjk(&format!("⚠ {}", e), inner_w.max(1)),
        None => Vec::new(),
    };
    let sep_warn = usize::from(!warning_lines.is_empty());
    let sep_err = usize::from(!error_lines.is_empty());
    // 本文行数 = ステップ 1 + 説明 1 + 項目 3 + 送信 1 + 空行 1 + フッタ 1 + 警告/エラー行
    let body_rows = 1 + 1 + ROUND_ROWS + 1 + 1 + 1 + sep_warn + warning_lines.len()
        + sep_err
        + error_lines.len();
    let max_h = area.height.saturating_sub(2).max(8);
    let h = ((body_rows as u16) + 4).clamp(8, max_h);
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let dlg = Rect::new(x, y, w, h);
    crate::ui::modals::draw_dialog_frame(buf, dlg, " task create: rounds ", Color::Yellow, bg);
    let bottom = dlg.y + dlg.height.saturating_sub(1);
    let mut row = dlg.y + 2;
    if row < bottom {
        draw_text_clipped(
            buf,
            dlg,
            dlg.x + 2,
            row,
            &step_label(wizard),
            Style::default().fg(Color::Cyan),
        );
        row += 1;
    }
    if row < bottom {
        draw_text_clipped(
            buf,
            dlg,
            dlg.x + 2,
            row,
            "global_round / round / tag を入力（空可）",
            Style::default().fg(Color::White),
        );
        row += 1;
    }
    for i in 0..ROUND_ROWS {
        if row >= bottom {
            break;
        }
        let mark = if wizard.rounds_focus == i { "> " } else { "  " };
        let value = if wizard.rounds_focus == i {
            match wizard.rounds_buf.as_ref() {
                Some(b) => format!("{}_", b),
                None => wizard.rounds[i].clone(),
            }
        } else {
            wizard.rounds[i].clone()
        };
        let color = if wizard.rounds_focus == i {
            Color::Yellow
        } else {
            Color::White
        };
        draw_text_clipped(
            buf,
            dlg,
            dlg.x + 2,
            row,
            &format!("{}{:<13}: {}", mark, ROUND_LABELS[i], value),
            Style::default().fg(color),
        );
        row += 1;
    }
    if row < bottom {
        let send_mark = if wizard.rounds_focus == ROUND_ROWS {
            "> "
        } else {
            "  "
        };
        let send_color = if wizard.rounds_focus == ROUND_ROWS {
            Color::Green
        } else {
            Color::White
        };
        draw_text_clipped(
            buf,
            dlg,
            dlg.x + 2,
            row,
            &format!("{}[ Enter = create ]", send_mark),
            Style::default().fg(send_color),
        );
        row += 1;
    }
    if row < bottom {
        row += 1;
    }
    if row < bottom {
        draw_text_clipped(
            buf,
            dlg,
            dlg.x + 2,
            row,
            "Tab/Up/Down=move Enter=edit/create s=skip Esc=back",
            Style::default().fg(Color::Gray),
        );
        row += 1;
    }
    if !warning_lines.is_empty() && row < bottom {
        row += 1;
    }
    for line in &warning_lines {
        if row >= bottom {
            break;
        }
        draw_text_clipped(
            buf,
            dlg,
            dlg.x + 2,
            row,
            line,
            Style::default().fg(Color::LightYellow),
        );
        row += 1;
    }
    if !error_lines.is_empty() && row < bottom {
        row += 1;
    }
    for line in error_lines {
        if row >= bottom {
            break;
        }
        draw_text_clipped(
            buf,
            dlg,
            dlg.x + 2,
            row,
            &line,
            Style::default().fg(Color::Red),
        );
        row += 1;
    }
}

/// テキストを指定セル幅で折り返す（CJK 全角=2 幅対応・run-tui と同一）。
fn wrap_text_cjk(text: &str, max_width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    if max_width == 0 {
        return vec![text.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        let mut cur_w = 0usize;
        for ch in para.chars() {
            let w = ch.width().unwrap_or(1).max(1);
            if cur_w + w > max_width && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            cur.push(ch);
            cur_w += w;
        }
        out.push(cur);
    }
    out
}

/// ダイアログ下部に出す警告行（`⚠` 付き・枠内で折り返す）。
///
/// stderr に出した警告は alternate screen 上で枠外に残り続けるため、
/// 警告はウィザード状態に溜めてこの関数で枠内描画する。
fn wrap_warnings(warnings: &[String], max_width: usize) -> Vec<String> {
    warnings
        .iter()
        .flat_map(|m| wrap_text_cjk(&format!("⚠ {}", m), max_width.max(1)))
        .collect()
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyModifiers, KeyEvent};
    use ratatui::{buffer::Buffer, layout::Rect};
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

    fn wizard_state() -> CardWizardState {
        CardWizardState::new()
    }

    /// 開いた直後は Runner 選択から始まること（review の引き継ぎ手順は無い）。
    #[test]
    fn wizard_starts_at_runner() {
        assert_eq!(wizard_state().step, WizardStep::Runner);
    }

    /// Runner で pi → Block → DocSelect → Provider → Model → Thinking → Rounds の順に進むこと。
    #[test]
    fn runner_pi_goes_through_provider_and_model() {
        let _guard = crate::app::file_test_lock();
        let mut w = wizard_state();
        let store = SaiStore::open_in_memory().unwrap();
        let mut state = AppState::new(Arc::new(store));
        // pi（index 1）を確定
        w.runner_cursor = 1;
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.step, WizardStep::Block);
        assert_eq!(w.runner, "pi");
        w.blocks.clear(); // テストの決定性（実ブロック定義の影響を受けない）
        // Block を確定（空でも可）→ Step4 文書選択へ
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.step, WizardStep::DocSelect);
        // Step4 をスキップ → Provider へ
        let _ = handle_wizard_key_inner(&mut state, key('s'), &mut w);
        assert_eq!(w.step, WizardStep::Provider);
        // Provider をスキップ → Model へ（この項目だけをスキップ）
        let _ = handle_wizard_key_inner(&mut state, key('s'), &mut w);
        assert_eq!(w.step, WizardStep::Model);
        // Model を確定 → Thinking へ
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.step, WizardStep::Thinking);
        // Thinking を確定 → Rounds へ
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.step, WizardStep::Rounds);
        assert_eq!(w.thinking, "none");
    }

    /// Runner で user → Block → DocSelect → Rounds（provider/model スキップ）になること。
    #[test]
    fn runner_user_skips_provider_and_model() {
        let _guard = crate::app::file_test_lock();
        let mut w = wizard_state();
        let store = SaiStore::open_in_memory().unwrap();
        let mut state = AppState::new(Arc::new(store));
        w.runner_cursor = 0;
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.runner, "user");
        w.blocks.clear(); // テストの決定性（実ブロック定義の影響を受けない）
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.step, WizardStep::DocSelect);
        let _ = handle_wizard_key_inner(&mut state, key('s'), &mut w);
        assert_eq!(w.step, WizardStep::Rounds);
    }

    /// Runner で cc → Block → DocSelect → Model（opus/sonnet/haiku）→ Rounds になること。
    #[test]
    fn runner_cc_uses_fixed_models() {
        let _guard = crate::app::file_test_lock();
        let mut w = wizard_state();
        let store = SaiStore::open_in_memory().unwrap();
        let mut state = AppState::new(Arc::new(store));
        w.runner_cursor = 2;
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.runner, "cc");
        w.blocks.clear(); // テストの決定性（実ブロック定義の影響を受けない）
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.step, WizardStep::DocSelect);
        let _ = handle_wizard_key_inner(&mut state, key('s'), &mut w);
        assert_eq!(w.step, WizardStep::Model);
        assert_eq!(
            w.models,
            vec![
                "opus".to_string(),
                "sonnet".to_string(),
                "haiku".to_string()
            ]
        );
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.step, WizardStep::Thinking);
        // Thinking を確定 → Rounds へ（cc は effort として使われる）
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.step, WizardStep::Rounds);
    }


    /// Runner で gl → Block → DocSelect → Rounds（provider/model スキップ・タスク書き込みなし）になること。
    #[test]
    fn runner_gl_skips_provider_and_model() {
        let _guard = crate::app::file_test_lock();
        let mut w = wizard_state();
        let store = SaiStore::open_in_memory().unwrap();
        let mut state = AppState::new(Arc::new(store));
        w.runner_cursor = 3;
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.runner, "gl");
        w.blocks.clear(); // テストの決定性（実ブロック定義の影響を受けない）
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.step, WizardStep::DocSelect);
        let _ = handle_wizard_key_inner(&mut state, key('s'), &mut w);
        assert_eq!(w.step, WizardStep::Rounds);
        assert!(w.provider.is_empty());
        assert!(w.model.is_empty());
    }

    /// Esc で 1 つ前に戻れること（Model → Provider）。
    #[test]
    fn esc_goes_back_one_step() {
        let _guard = crate::app::file_test_lock();
        let mut w = wizard_state();
        w.step = WizardStep::Model;
        w.runner = "pi".to_string();
        let store = SaiStore::open_in_memory().unwrap();
        let mut state = AppState::new(Arc::new(store));
        let _ = handle_wizard_key_inner(&mut state, esc(), &mut w);
        assert_eq!(w.step, WizardStep::Provider);
    }

    /// 各ステップの描画がパニックしないこと。
    #[test]
    fn render_all_steps_does_not_panic() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        for step in [
            WizardStep::Runner,
            WizardStep::Block,
            WizardStep::DocSelect,
            WizardStep::Provider,
            WizardStep::Model,
            WizardStep::Thinking,
            WizardStep::Rounds,
        ] {
            let mut w = wizard_state();
            w.step = step;
            w.blocks = vec![("表示名".to_string(), "block-id".to_string())];
            w.providers = vec!["test-provider".to_string()];
            w.models = vec!["model-a".to_string()];
            w.error = Some(
                "長いエラーメッセージの折り返し確認: あいうえおかきくけこさしすせそ".to_string(),
            );
            render_card_wizard(&w, area, &mut buf, Color::Black);
        }
    }

    /// Step7: thinking を選択して確定でき、Esc で Model へ戻れること。
    #[test]
    fn thinking_step_selects_level_and_back() {
        let _guard = crate::app::file_test_lock();
        let store = SaiStore::open_in_memory().unwrap();
        let mut state = AppState::new(Arc::new(store));
        let mut w = wizard_state();
        w.step = WizardStep::Thinking;
        w.runner = "pi".to_string();
        let _ = handle_wizard_key_inner(&mut state, key('j'), &mut w);
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.step, WizardStep::Rounds);
        assert_eq!(w.thinking, THINKING_OPTIONS[1], "選択したレベルが保存される");
        // Rounds から Esc で Thinking へ戻る（pi）
        let _ = handle_wizard_key_inner(&mut state, esc(), &mut w);
        assert_eq!(w.step, WizardStep::Thinking);
        // Thinking から Esc で Model へ戻る
        let _ = handle_wizard_key_inner(&mut state, esc(), &mut w);
        assert_eq!(w.step, WizardStep::Model);
    }

    /// Step4 文書選択: Space 切替・Enter 確定で次へ、選択行が ref になること。
    #[test]
    fn doc_select_toggles_and_refs() {
        let _guard = crate::app::file_test_lock();
        let store = SaiStore::open_in_memory().unwrap();
        let mut state = AppState::new(Arc::new(store));
        let mut w = wizard_state();
        w.step = WizardStep::DocSelect;
        w.doc_choices = vec![
            DocChoice { token: "AbCdEfGh".to_string(), ref_line: "waggle:AbCdEfGh:path:.sai/outputs/*/1/a.md".to_string(), label: "[running] a.md - 説明".to_string() },
            DocChoice { token: "IjKlMnOp".to_string(), ref_line: "waggle:IjKlMnOp:path:.sai/outputs/*/1/b.md".to_string(), label: "[completed] b.md - 報告".to_string() },
        ];
        w.doc_checked = vec![false, false];
        // Space で1件目を選択 → Enter で次へ（pi 既定なので Provider）
        w.runner = "pi".to_string();
        let _ = handle_wizard_key_inner(&mut state, key(' '), &mut w);
        assert!(w.doc_checked[0]);
        let _ = handle_wizard_key_inner(&mut state, enter(), &mut w);
        assert_eq!(w.step, WizardStep::Provider);
        assert_eq!(w.selected_doc_refs(), vec!["waggle:AbCdEfGh:path:.sai/outputs/*/1/a.md".to_string()]);
        // Esc 相当の戻りは Rounds/Thinking 経路で確認済み。DocSelect から Block へ戻れること
        w.step = WizardStep::DocSelect;
        let _ = handle_wizard_key_inner(&mut state, esc(), &mut w);
        assert_eq!(w.step, WizardStep::Block);
    }

    /// ステップ見出しが 7 固定マップで表示されること。
    #[test]
    fn step_label_uses_fixed_seven_map() {
        let mut w = wizard_state();
        assert_eq!(step_label(&w), "Step 1/7");
        w.step = WizardStep::Block;
        assert_eq!(step_label(&w), "Step 2/7");
        w.step = WizardStep::DocSelect;
        assert_eq!(step_label(&w), "Step 3/7");
        w.step = WizardStep::Thinking;
        assert_eq!(step_label(&w), "Step 6/7");
        w.step = WizardStep::Rounds;
        assert_eq!(step_label(&w), "Step 7/7");
    }

    /// 警告は各ステップでダイアログ枠の内側に描画されること（枠外へは出さない）。
    #[test]
    fn warnings_rendered_inside_dialog() {
        let area = Rect::new(0, 0, 80, 24);
        let long = "settings.json のパースに失敗しました: /Users/example/project/.sai/settings.json"
            .to_string();
        for step in [
            WizardStep::Runner,
            WizardStep::Block,
            WizardStep::DocSelect,
            WizardStep::Provider,
            WizardStep::Model,
            WizardStep::Thinking,
            WizardStep::Rounds,
        ] {
            let mut buf = Buffer::empty(area);
            let mut w = wizard_state();
            w.step = step;
            w.blocks = vec![("表示名".to_string(), "block-id".to_string())];
            w.providers = vec!["test-provider".to_string()];
            w.models = vec!["model-a".to_string()];
            w.warnings = vec![long.clone()];
            render_card_wizard(&w, area, &mut buf, Color::Black);
            let text: String = buf.content().iter().map(|c| c.symbol()).collect();
            assert!(
                text.contains("settings.json"),
                "{:?}: 警告が枠内に描かれていない",
                step
            );
            // 枠は幅72で中央配置（x = 4..76）。枠外への描画がないこと。
            for (i, cell) in buf.content().iter().enumerate() {
                if cell.symbol() == "⚠" {
                    let x = (i % area.width as usize) as u16;
                    assert!(
                        (4..76).contains(&x),
                        "{:?}: 警告が枠外（x={}）に描画された",
                        step,
                        x
                    );
                }
            }
        }
    }

    /// ウィザードを閉じると警告（ウィザード状態）ごと消えること。
    #[test]
    fn closing_wizard_drops_warnings() {
        let store = SaiStore::open_in_memory().unwrap();
        let mut state = AppState::new(Arc::new(store));
        open_card_wizard(&mut state);
        assert!(matches!(state.modal, crate::app::Modal::CardCreate(_)));
        // Runner で Esc → モーダルごと閉じる（枠も警告も消える）
        let _ = handle_card_wizard_key(&mut state, esc());
        assert!(matches!(state.modal, crate::app::Modal::None));
    }
}
