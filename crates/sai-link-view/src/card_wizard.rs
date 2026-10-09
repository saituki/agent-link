//! task.md 作成ウィザードの共通処理（`sai-link phone card …` 用）。
//!
//! 旧 `sai-link-tui::app::card_wizard`（選択肢読み込み・カード作成）と
//! `sai-link-tui::app::input`（`apply_card_auto_fields` / `write_card_file`）から移設した。
//! `sai-link phone card …` がこの組み立てと検査を使う。

use std::path::PathBuf;

use sai_link_engine::db::SaiStore;

/// cc のモデル選択肢（固定）。
pub const CC_MODELS: [&str; 3] = ["opus", "sonnet", "haiku"];
/// runner の選択肢（pi は廃止）。
pub const RUNNER_OPTIONS: [&str; 2] = ["user", "cc"];
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
    for (display, path) in sai_link_engine::common::collect_block_files() {
        let block_id = sai_link_engine::parser::parse_block_file(&path)
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

/// pi の使用可能 provider 一覧（`models.json` 由来・昇順・重複なし）。
pub fn load_provider_choices() -> Vec<String> {
    let mut providers: Vec<String> = sai_link_engine::common::load_available_models()
        .into_iter()
        .filter_map(|full| full.split_once('/').map(|(p, _)| p.to_string()))
        .collect();
    providers.sort();
    providers.dedup();
    providers
}

/// 指定 provider 配下の model 一覧（`/` より後ろ・昇順）。
pub fn load_model_choices(provider: &str) -> Vec<String> {
    let prefix = format!("{}/", provider);
    let mut models: Vec<String> = sai_link_engine::common::load_available_models()
        .into_iter()
        .filter_map(|full| full.strip_prefix(&prefix).map(|m| m.to_string()))
        .collect();
    models.sort();
    models.dedup();
    models
}

// ============================================================================
// block 定義の読み込み
// ============================================================================

/// block_id からブロック定義ファイルのパスを解決する。
///
/// 1. そのままパスとして存在するか
/// 2. `.sai/blocks/<id>.md` が存在するか
/// 3. 全ブロック定義を走査して `id:` が一致するもの
pub fn resolve_block_path(block_id: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(block_id);
    if direct.exists() {
        return Some(direct);
    }
    let candidate =
        sai_link_engine::project::sai_path(".sai/blocks").join(format!("{}.md", block_id));
    if candidate.exists() {
        return Some(candidate);
    }
    for (_, path) in sai_link_engine::common::collect_block_files() {
        if let Ok(parsed) = sai_link_engine::parser::parse_block_file(&path) {
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
/// - `model` / `provider` / `thinking`: `.sai/settings.json` のデフォルト
/// - ラウンド系は空欄のまま（利用者が決める）
pub fn apply_card_auto_fields(fm: &mut sai_link_engine::kanban::CardFrontmatter) {
    // settings.json（プロジェクト → グローバルのマージ）からデフォルトを読む。
    // 読み込み警告はここでは扱わない。
    let (settings, _) = sai_link_engine::settings::load_settings_quiet();
    if let Some(model) = settings.default_model {
        fm.model = model;
    }
    if let Some(provider) = settings.default_provider {
        fm.provider = provider;
    }
    if let Some(thinking) = settings.default_thinking_level {
        // 新 6 値のうち `none` は「明示指定なし」と同じ扱いのため書き込まない。
        // 旧値（off / minimal 等）も書き込まない（実行時に警告が出る）。
        let t = thinking.trim().to_lowercase();
        if sai_link_engine::validation::VALID_THINKING_LEVELS.contains(&t.as_str()) && t != "none" {
            fm.thinking = t;
        }
    }
}

/// カード md を `outputs/todo/<run_id>/task.md` に書き出す（R206・R301）。
///
/// 作成時に予約 run を発行して run_id を確定させ、frontmatter に記入する。
/// `run_id: -` は廃止。DB 失敗時はエラーを返す（フォルダを作らない）。
pub fn write_card_file(card: &sai_link_engine::kanban::KanbanCard) -> Result<PathBuf, String> {
    // 予約 run を発行して run_id を確定させる（R301）
    let run_id = reserve_run_for_card(card)?;
    let mut card = card.clone();
    card.frontmatter.run_id = run_id.to_string();
    let dir =
        sai_link_engine::run_dir::ensure_run_dir(sai_link_engine::run_dir::TODO_OWNER, run_id)
            .map_err(|e| e.to_string())?;
    let path = dir.join(sai_link_engine::run_dir::TASK_FILE);
    std::fs::write(&path, sai_link_engine::kanban::render_card(&card))
        .map_err(|e| e.to_string())?;
    Ok(path)
}

/// カード用の予約 run を発行する（`status='todo'`・R301）。
///
/// 関数取得用に `SaiStore` を開く。DB が無い場合はエラーを返す。
fn reserve_run_for_card(card: &sai_link_engine::kanban::KanbanCard) -> Result<i64, String> {
    let db_path = sai_link_engine::project::sai_path(".sai/sai-engine/sai-engine.db");
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
    /// runner（空 = スキップ → 自動値 cc）
    pub runner: String,
    /// block_id（空 = スキップ）
    pub block: String,
    /// provider（空 = スキップ）
    pub provider: String,
    /// model（空 = スキップ）
    pub model: String,
    /// thinking（空 = スキップ）
    pub thinking: String,
    /// グローバルラウンド（空 = 空欄）
    pub global_round: String,
    /// ローカルラウンド（空 = 空欄）
    pub round: String,
    /// タグ（空 = スキップ）
    pub tag: String,
    /// `# task_text` 見出しの直下に挿入する本文（TUI は常に空・nvim で書くため）
    pub task_text: String,
}

/// `phone card create` の検査（TUI の段階入力と同じ規則）。
///
/// - runner / thinking は一覧の値（空 = スキップは許容）
/// - block が指定されていれば存在する定義であること
pub fn validate_card_spec(spec: &CardSpec) -> Result<(), String> {
    let runner = spec.runner.trim();
    if runner == "pi" {
        return Err("runner の pi は廃止しました。cc を使ってください".to_string());
    }
    if !runner.is_empty() && !RUNNER_OPTIONS.contains(&runner) {
        return Err(format!(
            "runner が不正です: {}（選択肢: {}）",
            runner,
            RUNNER_OPTIONS.join(", ")
        ));
    }
    let thinking = spec.thinking.trim();
    if !thinking.is_empty() && !THINKING_OPTIONS.contains(&thinking) {
        return Err(format!(
            "thinking が不正です: {}（選択肢: {}）",
            thinking,
            THINKING_OPTIONS.join(", ")
        ));
    }
    let block = spec.block.trim();
    if block.is_empty() {
        return Ok(());
    }
    let path =
        resolve_block_path(block).ok_or_else(|| format!("ブロックが見つかりません: {}", block))?;
    sai_link_engine::parser::parse_block_file(&path)
        .map_err(|e| format!("ブロック定義の読み込みに失敗: {}", e))?;
    Ok(())
}

/// ウィザード / CLI の値でカード md を作成する。
///
/// - `runner` / `block` / `provider` / `model` / `thinking` は指定値を優先する
///   （空・スキップ時は自動値: runner=cc・model/provider/thinking=settings.json）
/// - `task_text` は `# task_text` 見出しの直下に置く
pub fn create_card(spec: &CardSpec) -> Result<PathBuf, String> {
    let body = if spec.task_text.trim().is_empty() {
        "# task_text\n".to_string()
    } else {
        format!("# task_text\n\n{}", spec.task_text.trim_end())
    };
    let mut fm = sai_link_engine::kanban::CardFrontmatter::new_card(
        sai_link_engine::kanban::card_creation_date(),
    );
    // 自動フィールド（settings.json のデフォルト）
    apply_card_auto_fields(&mut fm);
    // 確定値で上書き（空・スキップ時は自動値を維持）
    if !spec.runner.trim().is_empty() {
        fm.runner = spec.runner.trim().to_string();
    }
    if !spec.block.trim().is_empty() {
        fm.block = spec.block.trim().to_string();
    }
    if !spec.provider.trim().is_empty() {
        fm.provider = spec.provider.trim().to_string();
    }
    if !spec.model.trim().is_empty() {
        fm.model = spec.model.trim().to_string();
    }
    if !spec.thinking.trim().is_empty() {
        fm.thinking = spec.thinking.trim().to_string();
    }
    // rounds 入力（空時は既存値を維持。空のままなら card run が 1 を使う）
    if !spec.global_round.trim().is_empty() {
        fm.global_round = spec.global_round.trim().to_string();
    }
    if !spec.round.trim().is_empty() {
        fm.round = spec.round.trim().to_string();
    }
    if !spec.tag.trim().is_empty() {
        fm.tag = spec.tag.trim().to_string();
    }
    write_card_file(&sai_link_engine::kanban::KanbanCard {
        frontmatter: fm,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// runner / thinking は一覧の値以外を弾く（空 = スキップは許容）。
    #[test]
    fn spec_rejects_unknown_runner_and_thinking() {
        let mut spec = CardSpec {
            runner: "unknown-runner".to_string(),
            ..Default::default()
        };
        assert!(validate_card_spec(&spec).is_err());
        spec.runner = String::new();
        spec.thinking = "ultra".to_string();
        assert!(validate_card_spec(&spec).is_err());
        spec.thinking = "high".to_string();
        assert!(validate_card_spec(&spec).is_ok());
    }

    /// runner の pi は廃止のため、理由を付けて断る。cc と user は通る。
    #[test]
    fn spec_rejects_pi_runner_and_accepts_cc_user() {
        let mut spec = CardSpec {
            runner: "pi".to_string(),
            ..Default::default()
        };
        let err = validate_card_spec(&spec).unwrap_err();
        assert!(err.contains("runner の pi は廃止しました"), "err={}", err);
        spec.runner = "cc".to_string();
        assert!(validate_card_spec(&spec).is_ok());
        spec.runner = "user".to_string();
        assert!(validate_card_spec(&spec).is_ok());
        assert_eq!(RUNNER_OPTIONS, ["user", "cc"]);
    }
}
