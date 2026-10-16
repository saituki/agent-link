//! task.md 作成ウィザードの共通処理（`agent-link phone card …` 用）。
//!
//! 旧 `agent-link-tui::app::card_wizard`（選択肢読み込み・カード作成）と
//! `agent-link-tui::app::input`（`apply_card_auto_fields` / `write_card_file`）から移設した。
//! `agent-link phone card …` がこの組み立てと検査を使う。

use std::path::PathBuf;

use agent_link_engine::db::SaiStore;

/// cc のモデル選択肢（固定）。
pub const CC_MODELS: [&str; 3] = ["opus", "sonnet", "haiku"];
/// thinking 選択肢（`validation.rs` の新 6 値と一致）。
pub const THINKING_OPTIONS: [&str; 6] = ["none", "low", "medium", "high", "xhigh", "max"];

// ============================================================================
// 選択肢読み込み
// ============================================================================

/// ブロック一覧を読み込む（`(表示名, block_id)`・表示名昇順）。
///
/// `collect_block_files` は `(表示名, パス)` を返すため、block_id は
/// ブロック定義ファイルから読む。読めない定義はファイル stem を ID とする。
/// 警告（`source_blocks` 未指定等）は呼び出し側に出さない
/// （TUI の alternate screen では stderr が画面を崩すため）。
pub fn load_block_choices() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (display, path) in agent_link_engine::common::collect_block_files() {
        let block_id = agent_link_engine::parser::parse_block_file(&path)
            .map(|parsed| parsed.config.block_id().to_string())
            .unwrap_or_else(|_| {
                std::path::Path::new(&path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or(display.clone())
            });
        out.push((display, block_id));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

// ============================================================================
// block 定義の読み込み
// ============================================================================

/// block_id からブロック定義ファイルのパスを解決する。
///
/// 1. そのままパスとして存在するか
/// 2. `.agent-link/blocks/<id>.md` が存在するか
/// 3. 全ブロック定義を走査して `id:` が一致するもの
pub fn resolve_block_path(block_id: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(block_id);
    if direct.exists() {
        return Some(direct);
    }
    let candidate =
        agent_link_engine::project::sai_path(".agent-link/blocks").join(format!("{}.md", block_id));
    if candidate.exists() {
        return Some(candidate);
    }
    for (_, path) in agent_link_engine::common::collect_block_files() {
        if let Ok(parsed) = agent_link_engine::parser::parse_block_file(&path) {
            if parsed.config.block_id() == block_id {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

// ============================================================================
// カード作成
// ============================================================================

/// task.md 作成時の自動追記フィールドを埋める。
///
/// - `model` / `thinking`: `.agent-link/settings.json` のデフォルト
/// - ラウンド系は空欄のまま（利用者が決める）
pub fn apply_card_auto_fields(fm: &mut agent_link_engine::kanban::CardFrontmatter) {
    // settings.json（プロジェクト → グローバルのマージ）からデフォルトを読む。
    // 読み込み警告はここでは扱わない。
    let (settings, _) = agent_link_engine::settings::load_settings_quiet();
    if let Some(model) = settings.default_model {
        fm.model = model;
    }
    if let Some(thinking) = settings.default_thinking_level {
        // 新 6 値のうち `none` は「明示指定なし」と同じ扱いのため書き込まない。
        // 旧値（off / minimal 等）も書き込まない（実行時に警告が出る）。
        let t = thinking.trim().to_lowercase();
        if agent_link_engine::validation::VALID_THINKING_LEVELS.contains(&t.as_str()) && t != "none" {
            fm.thinking = t;
        }
    }
}

/// カード md を `outputs/todo/<run_id>/task.md` に書き出す（R206・R301）。
///
/// 作成時に予約 run を発行して run_id を確定させ、frontmatter に記入する。
/// `run_id: -` は廃止。DB 失敗時はエラーを返す（フォルダを作らない）。
pub fn write_card_file(card: &agent_link_engine::kanban::KanbanCard) -> Result<PathBuf, String> {
    // 予約 run を発行して run_id を確定させる（R301）
    let run_id = reserve_run_for_card(card)?;
    let mut card = card.clone();
    card.frontmatter.run_id = run_id.to_string();
    let dir =
        agent_link_engine::run_dir::ensure_run_dir(agent_link_engine::run_dir::TODO_OWNER, run_id)
            .map_err(|e| e.to_string())?;
    let path = dir.join(agent_link_engine::run_dir::TASK_FILE);
    std::fs::write(&path, agent_link_engine::kanban::render_card(&card))
        .map_err(|e| e.to_string())?;
    Ok(path)
}

/// カード用の予約 run を発行する（`status='todo'`・R301）。
///
/// 関数取得用に `SaiStore` を開く。DB が無い場合はエラーを返す。
fn reserve_run_for_card(card: &agent_link_engine::kanban::KanbanCard) -> Result<i64, String> {
    let db_path = agent_link_engine::project::db_path();
    let store = SaiStore::open(&db_path).map_err(|e| e.to_string())?;
    // 予約行に残す最小入力（description を引き継ぐ）
    let inputs = serde_json::json!({
        "description": card.frontmatter.description,
    });
    let inputs_str = serde_json::to_string(&inputs).map_err(|e| e.to_string())?;
    store
        .create_reserved_run(Some(&inputs_str))
        .map_err(|e| e.to_string())
}

/// カード作成の入力（TUI ウィザード / `phone card create` 共通）。
///
/// 各項目の空文字 = スキップ（TUI と同じく自動値を使う）。
#[derive(Debug, Clone, Default)]
pub struct CardSpec {
    /// block_id（空 = スキップ）
    pub block: String,
    /// model（空 = スキップ）
    pub model: String,
    /// thinking（空 = スキップ）
    pub thinking: String,
    /// `# task_text` 見出しの直下に挿入する本文（TUI は常に空・nvim で書くため）
    pub task_text: String,
}

/// `phone card create` の検査（TUI の段階入力と同じ規則）。
///
/// - block は必須（無いと `card run` が必ず失敗する）
/// - model は空（= 自動）か `normalize_model` が受ける値
/// - thinking は一覧の値（空 = スキップは許容）
/// - block は存在して読める定義であること
pub fn validate_card_spec(spec: &CardSpec) -> Result<(), String> {
    let block = spec.block.trim();
    if block.is_empty() {
        let ids: Vec<String> = load_block_choices().into_iter().map(|(_, id)| id).collect();
        return Err(format!("block is required. Choose one of: {}", ids.join(", ")));
    }
    let model = spec.model.trim();
    if !model.is_empty() && agent_link_engine::cc_command::normalize_model(model).is_none() {
        return Err(format!(
            "Invalid model: {}. Choose one of: {}",
            model,
            CC_MODELS.join(", ")
        ));
    }
    let thinking = spec.thinking.trim();
    if !thinking.is_empty() && !THINKING_OPTIONS.contains(&thinking) {
        return Err(format!(
            "Invalid thinking: {}. Choose one of: {}",
            thinking,
            THINKING_OPTIONS.join(", ")
        ));
    }
    let path =
        resolve_block_path(block).ok_or_else(|| format!("Block not found: {}", block))?;
    agent_link_engine::parser::parse_block_file(&path)
        .map_err(|e| format!("Failed to load the block definition: {}", e))?;
    Ok(())
}

/// ウィザード / CLI の値でカード md を作成する。
///
/// - `block` / `model` / `thinking` は指定値を優先する
///   （空・スキップ時は自動値: model/thinking=settings.json）
/// - `task_text` は `# task_text` 見出しの直下に置く
pub fn create_card(spec: &CardSpec) -> Result<PathBuf, String> {
    let body = if spec.task_text.trim().is_empty() {
        "# task_text\n".to_string()
    } else {
        format!("# task_text\n\n{}", spec.task_text.trim_end())
    };
    let mut fm = agent_link_engine::kanban::CardFrontmatter::new_card(
        agent_link_engine::kanban::card_creation_date(),
    );
    // 自動フィールド（settings.json のデフォルト）
    apply_card_auto_fields(&mut fm);
    // 確定値で上書き（空・スキップ時は自動値を維持）
    if !spec.block.trim().is_empty() {
        fm.block = spec.block.trim().to_string();
    }
    if !spec.model.trim().is_empty() {
        fm.model = spec.model.trim().to_string();
    }
    if !spec.thinking.trim().is_empty() {
        fm.thinking = spec.thinking.trim().to_string();
    }
    write_card_file(&agent_link_engine::kanban::KanbanCard {
        frontmatter: fm,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// block 必須・model は `normalize_model` が受ける値・thinking は一覧の値だけ。
    #[test]
    fn spec_rejects_missing_block_and_unknown_model() {
        let spec = CardSpec::default();
        let err = validate_card_spec(&spec).unwrap_err();
        assert!(err.starts_with("block is required. Choose one of:"), "{err}");
        let spec = CardSpec {
            block: "x".to_string(),
            model: "gpt".to_string(),
            ..Default::default()
        };
        let err = validate_card_spec(&spec).unwrap_err();
        assert_eq!(err, "Invalid model: gpt. Choose one of: opus, sonnet, haiku");
    }

    /// thinking は一覧の値以外を弾く（空 = スキップは許容）。
    #[test]
    fn spec_rejects_unknown_thinking() {
        let mut spec = CardSpec {
            block: "x".to_string(),
            thinking: "ultra".to_string(),
            ..Default::default()
        };
        let err = validate_card_spec(&spec).unwrap_err();
        assert!(err.starts_with("Invalid thinking: ultra."), "{err}");
        // thinking が通れば次は block の定義の読み出しで落ちる（block "x" は無い）
        spec.thinking = "high".to_string();
        assert_eq!(validate_card_spec(&spec).unwrap_err(), "Block not found: x");
        spec.thinking = String::new();
        assert_eq!(validate_card_spec(&spec).unwrap_err(), "Block not found: x");
    }
}
