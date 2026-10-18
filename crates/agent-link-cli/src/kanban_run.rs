//! # カードの実行要求
//!
//! `card run` の本体。カードファイル（`task.md`）の `# task_text` を入力にし、
//! ブロック定義と合わせて実行要求をデーモンへ送る。
//!
//! ## カード v2（汎用コンテナ）
//!
//! - 入力は本文の `# task_text` セクション 1 つ（旧 `inputs:` は廃止）
//! - 本文 `## Review run_id <id>` は埋め込みレビュー（入力には含めない）
//!
//! ## カード保存（R32 廃止）
//!
//! 実行後にカードファイルは削除しない。要求送信時に `execution_date` を
//! 要求時刻で更新し、デーモン側で `run_id` 確定後に書き戻す（二段階更新）。
//! `list_todo_cards` は `run_id:-` のみ返すため、記入済みは todo から消えつつ
//! ファイルは履歴として残る。
//!
//! 要件 R51: 事前定義方式の接続線は Card 作成時に選択した Review 済み記録とする
//! （`origin_run_id` を user_inputs に残し、デーモン側で `kanban_origin` 接続化）。

use std::collections::HashMap;
use std::path::PathBuf;

/// カードパスを直接指定して実行要求を送る（`card run` の入口。非対話のみ）。
///
/// 送信メッセージは出さない（呼び出し側が自分で出す）。戻り値はブロック表示名。
#[allow(clippy::too_many_arguments)]
pub fn run_card_request(
    card_path: &std::path::Path,
    block: Option<&str>,
    fields: &[String],
    think: Option<String>,
    model: Option<String>,
    block_timeout: Option<u32>,
) -> Result<String, Box<dyn std::error::Error>> {
    let card_path = card_path.to_path_buf();
    // カード読み出し
    let text = std::fs::read_to_string(&card_path)?;
    let card = agent_link_engine::kanban::parse_card(&text)
        .ok_or("Cannot parse the frontmatter of the card md.")?;

    // エージェント（executer: agent）が起こすカードは、人が起こす道（`card run`・
    // `block run --kanban`・常駐の予約実行）では動かさない。起こすと root の run になり、
    // 報告が起こす側（エージェント）に届かないため。エージェント自身が子の run として
    // 起こすのは別の入口（task_create）。旧カードの値も `agent` として同じに扱う。
    if card.frontmatter.is_agent_executer() {
        return Err(agent_link_engine::kanban::AGENT_EXECUTER_MESSAGE.into());
    }

    // ブロック選択：`--block` 指定優先、無ければ task.md の `block:` を使用
    let block_selector: String = match block {
        Some(b) => b.to_string(),
        None => {
            let b = card.frontmatter.block.trim().to_string();
            if b.is_empty() {
                return Err(
                    "`block:` is missing in task.md. Add `block: <block_id>` or pass --block."
                        .into(),
                );
            }
            b
        }
    };

    // ブロック定義ファイルを解決する
    let definition_file = resolve_block_file(&block_selector)?;
    let parsed = agent_link_engine::parser::parse_block_file(&definition_file)?;
    let block_id = parsed.config.block_id().to_string();

    // `--field id=value` をパース（CLI 引数での上書き用）
    let field_overrides = parse_field_args(fields);

    // task.md の `# task_text` を実行時の入力にする
    let user_inputs = resolve_card_inputs(&card, &field_overrides)?;

    // outputs 統一：予約 run_id を要求に添付する（R302・新カードのみ）。
    // task.md の run_id が数値なら予約行の使い回し、旧カード（`-`）なら従来どおり新規発行。
    let reserved_run_id: Option<i64> = card.frontmatter.run_id.trim().parse().ok();

    // 実行設定：task.md 優先、未記載は block.md のデフォルト。
    let non_empty = |s: &str| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    };
    let thinking = think
        .or_else(|| non_empty(&card.frontmatter.thinking))
        .or_else(|| parsed.config.thinking.clone());
    let model = model
        .or_else(|| non_empty(&card.frontmatter.model))
        .or_else(|| parsed.config.model.clone());
    // timeout は廃止（block.md の値を使用。task.md では指定しない）。
    let timeout = block_timeout.or(parsed.config.timeout);
    let mut overrides = agent_link_engine::orchestrator::ExecutionOverrides::default();
    overrides.thinking = thinking;
    overrides.model = model;
    overrides.timeout = timeout;
    stamp_card_execution_date(&card_path)?;
    let card_rel = card_relative_path(&card_path);
    agent_link_engine::daemon::write_block_run_request_full(
        &definition_file,
        &block_id,
        &user_inputs,
        &overrides,
        card_rel.as_deref(),
        reserved_run_id,
    )?;
    Ok(parsed.config.display_name().to_string())
}

/// task.md の `# task_text` を実行時の入力にする（CLI 薄ラッパー）。
///
/// 実体は `agent_link_engine::kanban::resolve_card_inputs`。
pub fn resolve_card_inputs(
    card: &agent_link_engine::kanban::KanbanCard,
    field_overrides: &HashMap<String, String>,
) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
    agent_link_engine::kanban::resolve_card_inputs(card, field_overrides).map_err(|e| e.into())
}

/// `--field id=value` 引数をパースする。
fn parse_field_args(fields: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for f in fields {
        match f.split_once('=') {
            Some((id, value)) => {
                map.insert(id.to_string(), value.to_string());
            }
            None => {
                eprintln!(
                    "Warning: --field '{}' is invalid. Use the id=value form.",
                    f
                );
            }
        }
    }
    map
}

/// カードの `execution_date` を要求時刻で更新する（Q6-A・一段階目）。
///
/// `run_id` は `-` のまま残し、デーモンが採番確定後に書き戻す。
/// frontmatter のみ更新し、本文には触らない。失敗時は警告に留める。
fn stamp_card_execution_date(card_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(card_path)?;
    let mut card = agent_link_engine::kanban::parse_card(&text)
        .ok_or("Cannot parse the frontmatter of the card md.")?;
    let now = chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string();
    card.frontmatter.execution_date = now;
    std::fs::write(card_path, agent_link_engine::kanban::render_card(&card))?;
    Ok(())
}

/// カードパスのプロジェクト相対パス（`.agent-link/todo/xxx.md`）を返す。
///
/// DB の `kanban_card_path` 保存用。相対化できなければ `None`。
fn card_relative_path(card_path: &PathBuf) -> Option<String> {
    let root = agent_link_engine::project::project_root();
    card_path
        .strip_prefix(&root)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

/// ブロック名またはパスから定義ファイルを解決する（判定は `card_wizard::resolve_block_path`）。
fn resolve_block_file(selector: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    agent_link_view::card_wizard::resolve_block_path(selector)
        .ok_or_else(|| format!("Block definition {} not found.", selector).into())
}
