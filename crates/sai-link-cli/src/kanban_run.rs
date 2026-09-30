//! # Kanban 統一実行モジュール
//!
//! `sai-link block run --kanban` のフローを実装します（実行方式の統一先）。
//! `todo` の未実行カード一覧を dialoguer Select で提示 → 選択 →
//! ブロック選択（2段階）→ task.md の `# task_text` を入力にし、
//! 実行要求をデーモンへ送る。
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

use dialoguer::Select;

/// `--kanban` 実行のエントリポイント。
///
/// 対話的にカード選択 → ブロック選択を行い、実行要求をデーモンへ送る。
/// `--block/--field/--yes` 指定時は非対話で実行する（pi agent 自動化用）。
#[allow(clippy::too_many_arguments)]
pub fn kanban_block_run(
    block: Option<&str>,
    fields: &[String],
    round: Option<i64>,
    global_round: Option<i64>,
    tag: Option<String>,
    think: Option<String>,
    model: Option<String>,
    block_timeout: Option<u32>,
    yes: bool,
    card: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    // task 駆動・非対話のみ。入力フォーム（対話プロンプト）は廃止。
    // 全て task.md に事前記載し、未記載は block.md のデフォルトで補う。
    let _ = yes;
    let cards = list_reserved_cards();
    if cards.is_empty() {
        println!("todo に未実行カードがありません（TUI `Shift+C` で作成してください）");
        return Ok(());
    }

    // カード選択：`--card` 指定または先頭カード（1件のみ時）
    let card_path: PathBuf = match card {
        Some(c) => resolve_card_file(&cards, c)?,
        None => {
            if cards.len() == 1 {
                cards[0].clone()
            } else {
                return Err(format!(
                    "未実行カードが {} 件あります。--card <ファイル名> で指定してください",
                    cards.len()
                )
                .into());
            }
        }
    };

    let display_name = run_card_request(
        &card_path,
        block,
        fields,
        round,
        global_round,
        tag,
        think,
        model,
        block_timeout,
    )?;
    print_sent(&display_name, &card_path);
    Ok(())
}

/// カードパスを直接指定して実行要求を送る（非対話・`phone card run` もこの入口を使う）。
///
/// `kanban_block_run` のカード解決以降の本体で、二重実装しない。
/// 送信メッセージは出さない（呼び出し側が自分で出す）。戻り値はブロック表示名。
#[allow(clippy::too_many_arguments)]
pub fn run_card_request(
    card_path: &std::path::Path,
    block: Option<&str>,
    fields: &[String],
    round: Option<i64>,
    global_round: Option<i64>,
    tag: Option<String>,
    think: Option<String>,
    model: Option<String>,
    block_timeout: Option<u32>,
) -> Result<String, Box<dyn std::error::Error>> {
    let card_path = card_path.to_path_buf();
    // カード読み出し
    let text = std::fs::read_to_string(&card_path)?;
    let card = sai_link_engine::kanban::parse_card(&text)
        .ok_or("カード md の frontmatter をパースできません")?;

    // professor が起こすカードは、人が起こす道（`phone card run`・`block run --kanban`・
    // 常駐の予約実行）では動かさない。起こすと root の run になり、報告が professor に
    // 届かないため。professor 自身が子の run として起こすのは別の入口（task_create）。
    if card.frontmatter.is_professor_executer() {
        return Err(sai_link_engine::kanban::PROFESSOR_EXECUTER_MESSAGE.into());
    }

    // ブロック選択：`--block` 指定優先、無ければ task.md の `block:` を使用
    let block_selector: String = match block {
        Some(b) => b.to_string(),
        None => {
            let b = card.frontmatter.block.trim().to_string();
            if b.is_empty() {
                return Err(
                    "task.md の `block:` が未記載です。`block: <block_id>` を記載するか --block で指定してください"
                        .into(),
                );
            }
            b
        }
    };

    // ブロック定義ファイルを解決する
    let definition_file = resolve_block_file(&block_selector)?;
    let parsed = sai_link_engine::parser::parse_block_file(&definition_file)?;
    let block_id = parsed.config.block_id().to_string();

    // `--field id=value` をパース（CLI 引数での上書き用）
    let field_overrides = parse_field_args(fields);

    // task.md の `# task_text` を実行時の入力にする
    let mut user_inputs = resolve_card_inputs(&card, &field_overrides)?;

    // outputs 統一：予約 run_id を要求に添付する（R302・新カードのみ）。
    // task.md の run_id が数値なら予約行の使い回し、旧カード（`-`）なら従来どおり新規発行。
    let reserved_run_id: Option<i64> = card.frontmatter.run_id.trim().parse().ok();

    // 実行設定：task.md 優先、未記載は block.md のデフォルト。
    // round / global_round は block.md にデフォルトが無いため task.md 必須。tag は任意。
    let round = round
        .map(|v| v.to_string())
        .unwrap_or_else(|| card.frontmatter.round.trim().to_string());
    let round: i64 = if round.is_empty() {
        return Err("task.md の `round:` が未記載です。`round: <番号>` を記載してください".into());
    } else {
        round.parse().map_err(|_| "task.md の `round:` が数値ではありません".to_string())?
    };
    let global_round = global_round
        .map(|v| v.to_string())
        .unwrap_or_else(|| card.frontmatter.global_round.trim().to_string());
    let global_round: i64 = if global_round.is_empty() {
        return Err(
            "task.md の `global_round:` が未記載です。`global_round: <番号>` を記載してください"
                .into(),
        );
    } else {
        global_round
            .parse()
            .map_err(|_| "task.md の `global_round:` が数値ではありません".to_string())?
    };
    let non_empty = |s: &str| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    };
    let tag = tag.or_else(|| non_empty(&card.frontmatter.tag));
    let thinking = think
        .or_else(|| non_empty(&card.frontmatter.thinking))
        .or_else(|| parsed.config.thinking.clone());
    let model = model
        .or_else(|| non_empty(&card.frontmatter.model))
        .or_else(|| parsed.config.model.clone());
    let provider = non_empty(&card.frontmatter.provider);
    // timeout は廃止（block.md の値を使用。task.md では指定しない）。
    let timeout = block_timeout.or(parsed.config.timeout);
    let mut overrides = sai_link_engine::orchestrator::ExecutionOverrides::default();
    overrides.round = Some(round);
    overrides.global_round = Some(global_round);
    overrides.tag = tag;
    overrides.thinking = thinking;
    overrides.model = model;
    overrides.provider = provider;
    overrides.timeout = timeout;
    // 実行主体（`cc` 等）。未記載は Pi（後方互換）。
    overrides.runner = sai_link_engine::runner::RunnerKind::parse(&card.frontmatter.runner);
    // runner=pi の task.md には runner 記載が無い旧カードもあるため、ここでは分岐しない。
    // runner=user の場合は TUI 側でデーモン要求を送らずフォルダ移動のみ行う。
    stamp_card_execution_date(&card_path)?;
    let card_rel = card_relative_path(&card_path);
    sai_link_engine::daemon::write_block_run_request_full(
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
/// 実体は `sai_link_engine::kanban::resolve_card_inputs`。
pub fn resolve_card_inputs(
    card: &sai_link_engine::kanban::KanbanCard,
    field_overrides: &HashMap<String, String>,
) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
    sai_link_engine::kanban::resolve_card_inputs(card, field_overrides).map_err(|e| e.into())
}

/// `serde_yaml::Value` の default 値を文字列化する。
fn yaml_value_to_string(v: &Option<serde_yaml::Value>) -> Option<String> {
    match v {
        None => None,
        Some(serde_yaml::Value::String(s)) => Some(s.clone()),
        Some(serde_yaml::Value::Bool(b)) => Some(b.to_string()),
        Some(serde_yaml::Value::Number(n)) => Some(n.to_string()),
        Some(other) => serde_yaml::to_string(other)
            .ok()
            .map(|s| s.trim().to_string()),
    }
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
                    "警告: --field '{}' の形式が不正です。id=value 形式で指定してください。",
                    f
                );
            }
        }
    }
    map
}

/// 予約カード一覧（`outputs/todo/<run_id>/task.md`＋旧 `.sai/todo/*.md`）。
///
/// 新方式では `task.md` の `run_id` が確定済み。旧カード（`run_id: -`）も移行期のみ併読する。
fn list_reserved_cards() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    // 新：outputs/todo/<run_id>/task.md
    let root = sai_link_engine::project::sai_path(sai_link_engine::run_dir::OUTPUTS_DIR);
    let todo_root = root.join(sai_link_engine::run_dir::TODO_OWNER);
    if let Ok(entries) = std::fs::read_dir(&todo_root) {
        for entry in entries.flatten() {
            let task = entry.path().join(sai_link_engine::run_dir::TASK_FILE);
            if task.is_file() {
                out.push(task);
            }
        }
    }
    // 旧：.sai/todo/*.md（移行期のみ）
    let todo_dir = sai_link_engine::project::sai_path(sai_link_engine::kanban::TODO_DIR);
    out.extend(sai_link_engine::kanban::list_todo_cards(&todo_dir));
    out.sort();
    out
}

/// カード一覧から `--card` 指定のカードを探す（ファイル名・部分一致）。
fn resolve_card_file(
    cards: &[PathBuf],
    selector: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // ファイル名の完全一致を優先
    for p in cards {
        if let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) {
            if name == selector {
                return Ok(p.clone());
            }
        }
    }
    // 部分一致（1件のみ）
    let matches: Vec<&PathBuf> = cards
        .iter()
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().contains(selector))
                .unwrap_or(false)
        })
        .collect();
    if matches.len() == 1 {
        return Ok(matches[0].clone());
    }
    if matches.len() > 1 {
        return Err(format!(
            "カード '{}' が曖昧です。ファイル名で指定してください",
            selector
        )
        .into());
    }
    Err(format!("カード '{}' が todo にありません", selector).into())
}

/// カードの `execution_date` を要求時刻で更新する（Q6-A・一段階目）。
///
/// `run_id` は `-` のまま残し、デーモンが採番確定後に書き戻す。
/// frontmatter のみ更新し、本文には触らない。失敗時は警告に留める。
fn stamp_card_execution_date(card_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(card_path)?;
    let mut card = sai_link_engine::kanban::parse_card(&text)
        .ok_or("カード md の frontmatter をパースできません")?;
    let now = chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string();
    card.frontmatter.execution_date = now;
    std::fs::write(card_path, sai_link_engine::kanban::render_card(&card))?;
    Ok(())
}

/// カードパスのプロジェクト相対パス（`.sai/todo/xxx.md`）を返す。
///
/// DB の `kanban_card_path` 保存用。相対化できなければ `None`。
fn card_relative_path(card_path: &PathBuf) -> Option<String> {
    let root = sai_link_engine::project::project_root();
    card_path
        .strip_prefix(&root)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

/// 送信完了メッセージを表示する。
fn print_sent(block_display_name: &str, card_path: &PathBuf) {
    println!(
        "📨 kanban 実行要求を送信しました: {} (card={})",
        block_display_name,
        card_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| card_path.display().to_string())
    );
}

/// ブロック選択（2段階目の対話）。
fn select_block() -> Result<String, Box<dyn std::error::Error>> {
    let blocks_dir = sai_link_engine::project::sai_path(".sai/blocks");
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&blocks_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();
    if names.is_empty() {
        return Err("ブロック定義が `.sai/blocks` にありません".into());
    }
    let idx = Select::new()
        .with_prompt("実行ブロックを選択")
        .items(&names)
        .default(0)
        .interact()?;
    Ok(names[idx].clone())
}

/// ブロック名またはパスから定義ファイルを解決する。
fn resolve_block_file(selector: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let p = PathBuf::from(selector);
    if p.exists() {
        return Ok(p);
    }
    let candidate =
        sai_link_engine::project::sai_path(".sai/blocks").join(format!("{}.md", selector));
    if candidate.exists() {
        return Ok(candidate);
    }
    Err(format!("ブロック定義が見つかりません: {}", selector).into())
}

/// 数値入力を求める（必須。空欄や数値以外は再入力させる）。
///
/// `default` 指定時は空欄 Enter でその値を使う（Round の frontmatter フォールバック用）。
fn prompt_number(prompt: &str, default: Option<i64>) -> Result<i64, Box<dyn std::error::Error>> {
    use std::io::{BufRead, Write};
    loop {
        let suffix = match default {
            Some(v) => format!("（デフォルト: {}）", v),
            None => "（必須）".to_string(),
        };
        print!("{} {}: ", prompt, suffix);
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if let Some(v) = default {
                return Ok(v);
            }
            println!("⚠ 必須です。整数で入力してください。");
            continue;
        }
        match trimmed.parse::<i64>() {
            Ok(v) => return Ok(v),
            Err(_) => println!("⚠ 無効な数値です。整数で入力してください。"),
        }
    }
}

/// Tag 入力（デフォルト＋入力のカンマ結合。`--interactive` と同等・R125）。
fn prompt_tag(default_tag: Option<&str>) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let default_display = default_tag.unwrap_or("(なし)");
    let prompt = format!("Tag（デフォルト: {}）", default_display);
    let input = prompt_optional_with_default(&prompt, None)?;
    match input {
        None => Ok(default_tag.map(String::from)),
        Some(v) => {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                Ok(default_tag.map(String::from))
            } else {
                match default_tag {
                    Some(def) => Ok(Some(format!("{}, {}", def, trimmed))),
                    None => Ok(Some(trimmed.to_string())),
                }
            }
        }
    }
}

/// Thinking budget 選択（6値。`validation.rs` と一致・R123）。
fn prompt_think(default_think: Option<&str>) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let options = &["none", "low", "medium", "high", "xhigh", "max"];
    prompt_select("Thinking budget", options, default_think)
}

/// Model 選択（`~/.pi/agent/models.json`＋`inherit`。`--interactive` と同等・R124）。
fn prompt_model(default_model: Option<&str>) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let available = sai_link_engine::common::load_available_models();
    let default_model = default_model.unwrap_or("inherit");
    let mut options: Vec<String> = vec!["inherit".to_string()];
    if default_model != "inherit" && !options.contains(&default_model.to_string()) {
        options.push(default_model.to_string());
    }
    for m in &available {
        if !options.contains(m) {
            options.push(m.clone());
        }
    }
    let default_idx = options.iter().position(|o| o == default_model).unwrap_or(0);
    let refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
    let idx = Select::new()
        .with_prompt(format!("Model（デフォルト: {}）", options[default_idx]))
        .items(&refs)
        .default(default_idx)
        .interact()?;
    Ok(Some(options[idx].clone()))
}

/// 選択肢から1つ選ぶ（dialoguer Select。デフォルトは frontmatter 値）。
fn prompt_select(
    prompt: &str,
    options: &[&str],
    default: Option<&str>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let default_idx = default
        .and_then(|d| options.iter().position(|o| *o == d))
        .unwrap_or(0);
    let idx = Select::new()
        .with_prompt(format!(
            "{}（デフォルト: {}）",
            prompt, options[default_idx]
        ))
        .items(options)
        .default(default_idx)
        .interact()?;
    Ok(Some(options[idx].to_string()))
}

/// 任意文字列の入力を求める（デフォルト付き。空欄はデフォルト採用）。
fn prompt_optional_with_default(
    prompt: &str,
    default: Option<&str>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    use std::io::{BufRead, Write};
    let suffix = match default {
        Some(v) => format!("（デフォルト: {}）", v),
        None => "（空欄可。そのまま Enter でなし）".to_string(),
    };
    print!("{} {}: ", prompt, suffix);
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let trimmed = line.trim().to_string();
    if trimmed.is_empty() {
        Ok(default.map(|v| v.to_string()))
    } else {
        Ok(Some(trimmed))
    }
}

/// u32 数値の入力を求める（デフォルト付き。空欄はデフォルト採用）。
fn prompt_number_u32(
    prompt: &str,
    default: Option<u32>,
) -> Result<Option<u32>, Box<dyn std::error::Error>> {
    use std::io::{BufRead, Write};
    loop {
        let suffix = match default {
            Some(v) => format!("（デフォルト: {}）", v),
            None => "（空欄可。そのまま Enter でなし）".to_string(),
        };
        print!("{} {}: ", prompt, suffix);
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(default);
        }
        match trimmed.parse::<u32>() {
            Ok(v) => return Ok(Some(v)),
            Err(_) => println!("⚠ 無効な数値です。整数で入力してください。"),
        }
    }
}
