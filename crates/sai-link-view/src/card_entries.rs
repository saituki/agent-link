//! Kanban カード・レーン一覧の組み立て（TUI / CLI 共通・実装計画 12 §3.1）。
//!
//! 旧 `sai-link-tui::app::data`（todo カード走査・description 解決）と
//! `sai-link-tui::app::kanban`（`CardEntry` / `collect_card_entries_from`）から
//! 移設した。TUI の見た目・挙動は変えず、`sai-link phone`（CLI）からも同じ
//! 組み立て結果を呼べるようにするための共通部品である。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use sai_link_engine::db::models::ExecutionHistory;
use sai_link_engine::db::SaiStore;
use sai_link_engine::kanban::KanbanLane;

// ============================================================================
// todo カード走査（旧 app/data.rs）
// ============================================================================

/// 未実行カード情報（todo レーン用・A1/A3 対応）。
///
/// 1行目書式は他レーンと同一（`ID<run_id>/<block名>`）。`block:` 未記載は `-`（A2）。
/// `run_id:` とディレクトリ名の不一致は `run_id=None` ＋ `run_id_mismatch=true`（A3）。
#[derive(Debug, Clone)]
pub struct TodoCardInfo {
    /// カード md のパス
    pub path: PathBuf,
    /// frontmatter `creation_date`
    pub creation_date: String,
    /// frontmatter `description:`
    pub description: String,
    /// frontmatter `block:`（空は `-` に正規化済み）
    pub block: String,
    /// 表示用 run_id（不一致時は `None`）
    pub run_id: Option<i64>,
    /// ディレクトリ名と `run_id:` が不一致か（エラー表示用）
    pub run_id_mismatch: bool,
    /// カードの frontmatter `executer:`（`human` / `professor`・未記載は `human`）
    pub executer: String,
}

impl TodoCardInfo {
    /// 1行目タイトル（`ID<run_id>/<block名>`・不一致時は `ID?/<block名> ⚠run_id不一致`）。
    pub fn title(&self) -> String {
        match self.run_id {
            Some(rid) => format!("ID{}/{}", rid, self.block),
            None => format!("ID?/{} ⚠run_id不一致", self.block),
        }
    }

    /// 2行目日付（他レーンと同一 `MM/DD HH:MM:SS`・A4）。
    pub fn date(&self) -> String {
        crate::fmt::format_creation_date_short(&self.creation_date)
    }
}

/// `.sai/todo/*.md` と `outputs/todo/<run_id>/task.md` を 1 回走査し、カード情報を辞書化する。
///
/// 戻り値は `(run_id → description, 未紐付けカード一覧)`。
/// 未紐付けカードは作成日時順。
///
/// 実行要求送信済み・未開始のカード（`pending/` に `block_run_request` あり）は
/// 未実行ファイルでも `unlinked` に含めない（送信直後の todo 重複を避ける）。
pub fn scan_todo_cards() -> (HashMap<i64, String>, Vec<TodoCardInfo>) {
    let mut by_run_id: HashMap<i64, String> = HashMap::new();
    let mut unlinked: Vec<TodoCardInfo> = Vec::new();
    // 新：outputs/todo/<run_id>/task.md（予約 run 方式・run_id 確定済み）
    let outputs_todo = sai_link_engine::project::sai_path(sai_link_engine::run_dir::OUTPUTS_DIR)
        .join(sai_link_engine::run_dir::TODO_OWNER);
    if let Ok(entries) = std::fs::read_dir(&outputs_todo) {
        for entry in entries.flatten() {
            let task = entry.path().join(sai_link_engine::run_dir::TASK_FILE);
            if !task.is_file() {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&task) else {
                continue;
            };
            let Some(card) = sai_link_engine::kanban::parse_card(&text) else {
                continue;
            };
            // 送信済み・未開始の要求があれば todo に載せない（running 予定地）
            if pending_run_ids()
                .contains(&card.frontmatter.run_id.trim().parse::<i64>().unwrap_or(-1))
            {
                continue;
            }
            if let Some(info) = todo_card_info(&task, &card) {
                unlinked.push(info);
            }
        }
    }
    // 旧：.sai/todo/*.md（移行期のみ）
    let todo_dir = sai_link_engine::project::sai_path(sai_link_engine::kanban::TODO_DIR);
    let Ok(entries) = std::fs::read_dir(&todo_dir) else {
        return (by_run_id, unlinked);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(card) = sai_link_engine::kanban::parse_card(&text) else {
            continue;
        };
        if card.frontmatter.is_unexecuted() {
            // 送信済み・未開始の要求があれば todo に載せない（running 予定地）。
            // パス正規化の揺れに備え、絶対パス・ファイル名の両方で照合する。
            // ファイル名では照合しない（全カードが `task.md` で同名のため、
            // 送信 1 枚で他の全件が送信済みと誤判定されて todo から全消えしていた）。
            if pending_card_paths().iter().any(|p| *p == path) {
                continue;
            }
            if let Some(info) = todo_card_info(&path, &card) {
                unlinked.push(info);
            }
        } else if let Ok(run_id) = card.frontmatter.run_id.trim().parse::<i64>() {
            by_run_id
                .entry(run_id)
                .or_insert_with(|| card.frontmatter.description.clone());
        }
    }
    unlinked.sort_by(|a, b| a.creation_date.cmp(&b.creation_date));
    (by_run_id, unlinked)
}

/// カード md から `TodoCardInfo` を構築する（A1/A2/A3）。
///
/// - 1行目用 `block:` は空なら `-`（A2）
/// - `outputs/todo/<run_id>/task.md` のディレクトリ名と frontmatter `run_id:` が
///   不一致なら `run_id=None` ＋ `run_id_mismatch=true`（A3・エラー表示用）
/// - 旧 `.sai/todo/*.md`（ディレクトリ名なし）は frontmatter のみで判定する
pub fn todo_card_info(
    path: &std::path::Path,
    card: &sai_link_engine::kanban::KanbanCard,
) -> Option<TodoCardInfo> {
    let block = {
        let b = card.frontmatter.block.trim();
        if b.is_empty() {
            "-".to_string()
        } else {
            b.to_string()
        }
    };
    let fm_run_id = card.frontmatter.run_id.trim().parse::<i64>().ok();
    // ディレクトリ名 run_id（`outputs/todo/<run_id>/task.md` のみ）
    let dir_run_id = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .and_then(|n| n.parse::<i64>().ok());
    let (run_id, run_id_mismatch) = match (dir_run_id, fm_run_id) {
        // 両方あり・不一致 → エラー表示（A3）
        (Some(dir), Some(fm)) if dir != fm => (None, true),
        // frontmatter 優先（旧カードはディレクトリ名なし）
        (_, Some(fm)) => (Some(fm), false),
        // frontmatter 未確定だがディレクトリ名あり → ディレクトリ名を使う
        (Some(dir), None) => (Some(dir), false),
        // 両方なし → エラー表示
        (None, None) => (None, true),
    };
    Some(TodoCardInfo {
        path: path.to_path_buf(),
        creation_date: card.frontmatter.creation_date.clone(),
        description: card.frontmatter.description.clone(),
        block,
        run_id,
        run_id_mismatch,
        executer: card.frontmatter.executer().to_string(),
    })
}

/// カードの frontmatter から起動者（`human` / `professor`）を読む。
///
/// カードが読めない・frontmatter が無いときは `human`（人が起こす）。
pub fn executer_of_card(card_path: Option<&std::path::Path>) -> String {
    let Some(path) = card_path else {
        return "human".to_string();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| sai_link_engine::kanban::parse_card(&text))
        .map(|card| card.frontmatter.executer().to_string())
        .unwrap_or_else(|| "human".to_string())
}

/// run フォルダの task.md から起動者（`human` / `professor`）を読む。
///
/// どのレーン（todo / running / review / done）でも run フォルダは
/// `find_run_dir` で引ける。カードが無い run は `human`。
pub fn executer_for_run(run_id: i64) -> String {
    let Some((_, dir)) = sai_link_engine::run_dir::find_run_dir(run_id) else {
        return "human".to_string();
    };
    executer_of_card(Some(&dir.join(sai_link_engine::run_dir::TASK_FILE)))
}

/// `pending/` 内の未処理 `block_run_request` が指す予約 run_id 一覧（R302）。
///
/// 予約 run 方式ではカードの run_id が確定済みのため、送信直後の todo 重複を
/// run_id 照合で防ぐ。旧カード（`card_path` のみ）もパス照合で防ぐ。
pub fn pending_run_ids() -> Vec<i64> {
    pending_card_paths()
        .iter()
        .filter_map(|p| {
            std::fs::read_to_string(p).ok().and_then(|text| {
                sai_link_engine::kanban::parse_card(&text)
                    .and_then(|c| c.frontmatter.run_id.trim().parse::<i64>().ok())
            })
        })
        .collect()
}

/// `pending/` 内の未処理 `block_run_request` が指すカード相対パス一覧。
///
/// 通知 JSON の `card_path`（`.sai/todo/xxx.md`）を絶対パス化して返す。
/// 読み失敗・パース失敗時は空（表示を止めない）。
pub fn pending_card_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let pending = sai_link_engine::common::notify_pending_dir();
    let Ok(entries) = std::fs::read_dir(&pending) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("block_run_request") {
            continue;
        }
        if let Some(rel) = value.get("card_path").and_then(|v| v.as_str()) {
            out.push(sai_link_engine::project::project_root().join(rel));
        }
    }
    out
}

/// 辞書引き版：`kanban_card_path` 優先・なければ run フォルダ・todo 辞書で解決する。
///
/// 実行ごとの走査を最小限にする（`load_data` 時の 1 回走査を使い回す）。
/// 解決順:
/// 1. `kanban_card_path`（DB 記録の相対パス）。ただし実行開始後は run フォルダが
///    status 移動するため記録パスが実在しなくなる（その場合は 2. へフォールバック）
/// 2. run フォルダ（`.sai/outputs/<owner>/<run_id>/task.md`）
/// 3. todo 辞書（事前走査済み。todo レーンの予約カード用）
pub fn description_for_execution_cached(
    _store: &SaiStore,
    card_rel: Option<&str>,
    run_id: i64,
    todo_by_run_id: &HashMap<i64, String>,
) -> String {
    if let Some(rel) = card_rel {
        let path = sai_link_engine::project::project_root().join(rel);
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(card) = sai_link_engine::kanban::parse_card(&text) {
                return card.frontmatter.description.clone();
            }
        }
    }
    // run フォルダ（status 移動後の正本）から task.md を読む
    if run_id > 0 {
        if let Some((_, dir)) = sai_link_engine::run_dir::find_run_dir(run_id) {
            let task = dir.join(sai_link_engine::run_dir::TASK_FILE);
            if let Ok(text) = std::fs::read_to_string(&task) {
                if let Some(card) = sai_link_engine::kanban::parse_card(&text) {
                    return card.frontmatter.description.clone();
                }
            }
        }
    }
    todo_by_run_id.get(&run_id).cloned().unwrap_or_default()
}

// ============================================================================
// カード一覧（レーン分類・旧 app/kanban.rs）
// ============================================================================

/// カード一覧エントリ（ボード表示用の中間形式）。
///
/// Card は実行記録の表示単位とする。
/// todo レーンは `outputs/todo/<run_id>/task.md`（旧 `.sai/todo/*.md` 併読）、
/// それ以外は実行履歴（DB）から作る。
#[derive(Debug, Clone)]
pub struct CardEntry {
    /// 1行目（全レーン共通 `ID<run_id>/<block名>`・todo の `block:` 未記載は `-`）
    pub title: String,
    /// 2行目（日付。todo は `creation_date`、実行済みは `created_at`）
    pub date: String,
    /// 3行目以降（対応カードの frontmatter `description:`。無ければ空）
    pub description: String,
    /// 所属レーン
    pub lane: KanbanLane,
    /// カード md のパス（todo レーンのみ `Some`）
    pub card_path: Option<PathBuf>,
    /// 対応 execution id（実行済みのみ `Some`）
    pub exec_id: Option<i64>,
    /// runner（pi / cc など。todo は未実行で空）
    pub runner: String,
    /// 起動者（`human` / `professor`）。カードが無い run は `human`。
    pub executer: String,
}

/// 実行に対応するカード md の絶対パスを解決する。
///
/// 1. `kanban_card_path`（DB記録の相対パス）があればそのファイルを優先
/// 2. なければ `outputs/<owner>/<run_id>/task.md` を探す
/// 3. なければ旧 `.sai/todo/*.md` を走査して `run_id` 一致のカードを探索（移行期のみ）
/// 4. どちらも無ければ `None`
pub fn card_path_for_execution(card_rel: Option<&str>, run_id: i64) -> Option<PathBuf> {
    if let Some(rel) = card_rel {
        let path = sai_link_engine::project::project_root().join(rel);
        if path.exists() {
            return Some(path);
        }
    }
    // outputs 統一後の正本
    if let Some((_, dir)) = sai_link_engine::run_dir::find_run_dir(run_id) {
        let task = dir.join(sai_link_engine::run_dir::TASK_FILE);
        if task.exists() {
            return Some(task);
        }
    }
    let todo_dir = sai_link_engine::project::sai_path(sai_link_engine::kanban::TODO_DIR);
    let Ok(entries) = std::fs::read_dir(&todo_dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(card) = sai_link_engine::kanban::parse_card(&text) else {
            continue;
        };
        if let Ok(rid) = card.frontmatter.run_id.trim().parse::<i64>() {
            if rid == run_id {
                return Some(path);
            }
        }
    }
    None
}

/// ボード表示用のカード一覧を構築する（取得済みの実行履歴と todo 辞書を使い回す版）。
///
/// Kanban が所有するものだけを返す（Workflow との排他）：
/// * todo: 未実行カードファイル（事前走査済み・作成日時順）
/// * running / review / done: 未移動（`workflow_moved=false`）の実行。
///   実行中は running、終端後は DB `review_done` の有無で review / done
///
/// `d` で移動済み（`workflow_moved=true`）の実行は含めない（Workflow へ）。
pub fn collect_card_entries_from(
    store: &SaiStore,
    executions: &[ExecutionHistory],
    todo_by_run_id: &HashMap<i64, String>,
    todo_unlinked: &[TodoCardInfo],
    mb_replica_execs: &HashSet<i64>,
) -> Vec<CardEntry> {
    let mut out: Vec<CardEntry> = Vec::new();
    // ブロック名解決用キャッシュ
    let mut block_name_cache: HashMap<i64, String> = HashMap::new();
    // 実行ID → 対応カード description のキャッシュ
    let mut desc_cache: HashMap<i64, String> = HashMap::new();
    let block_name_of = |store: &SaiStore, cache: &mut HashMap<i64, String>, block_id: i64| {
        if let Some(n) = cache.get(&block_id) {
            return n.clone();
        }
        let n = store
            .get_block_by_id(block_id)
            .map(|b| b.display_name)
            .unwrap_or_else(|_| format!("block#{}", block_id));
        cache.insert(block_id, n.clone());
        n
    };
    for e in executions {
        let Some(run_id) = e.id else { continue };
        // Kanban / Workflow の排他：`d` で移動済みはボードに載せない。
        // mb 複製（U13・R25）は R/r ツリー同様に載せない。
        // 予約行（`status='todo'`）は todo レーンで表示するためここでは載せない（R303）。
        if e.workflow_moved || mb_replica_execs.contains(&run_id) || e.status == "todo" {
            continue;
        }
        let lane = match e.status.as_str() {
            // 実行開始を契機に実行中へ
            "running" | "pending" => KanbanLane::Running,
            // 確定後は DB の `review_done`（run-tui `e` で更新）で振り分ける。
            // review.md 本文の記入では自動移動しない。
            _ => {
                if e.review_done {
                    KanbanLane::Done
                } else {
                    KanbanLane::Review
                }
            }
        };
        let name = block_name_of(store, &mut block_name_cache, e.block_id);
        // description：対応カードの frontmatter から取得（無ければ空）。
        let description = if let Some(d) = desc_cache.get(&run_id) {
            d.clone()
        } else {
            let d = description_for_execution_cached(
                store,
                e.kanban_card_path.as_deref(),
                run_id,
                todo_by_run_id,
            );
            desc_cache.insert(run_id, d.clone());
            d
        };
        let date = e
            .created_at
            .get(..19)
            .map(|s| {
                if s.len() >= 16 {
                    format!("{} {}", s[5..10].replace('-', "/"), &s[11..19.min(s.len())])
                } else {
                    s.to_string()
                }
            })
            .unwrap_or_else(|| e.created_at.clone());
        let card_path = card_path_for_execution(e.kanban_card_path.as_deref(), run_id);
        out.push(CardEntry {
            title: format!("ID{}/{}", run_id, name),
            date,
            description,
            lane,
            card_path,
            exec_id: Some(run_id),
            runner: e.runner.clone(),
            executer: executer_for_run(run_id),
        });
    }
    // todo: 未実行カードファイル（`load_data` で事前走査済み・作成日時順）。
    // 排他：DB 上で kanban 実行に紐付いたカード（`kanban_card_path`）は
    // ファイル側の `run_id` 書戻し前でも todo に載せない。
    let claimed: HashSet<PathBuf> = executions
        .iter()
        .filter_map(|e| e.kanban_card_path.as_deref())
        .map(|rel| sai_link_engine::project::project_root().join(rel))
        .collect();
    for info in todo_unlinked {
        if claimed.contains(&info.path) {
            continue;
        }
        out.push(CardEntry {
            title: info.title(),
            date: info.date(),
            description: info.description.clone(),
            lane: KanbanLane::Todo,
            card_path: Some(info.path.clone()),
            exec_id: None,
            runner: String::new(),
            executer: info.executer.clone(),
        });
    }
    out
}

// ============================================================================
// CLI 用の事前準備
// ============================================================================

/// `phone tree` / `phone card options` 用の事前準備（TUI `load_data` の部品と同一）。
///
/// 実行履歴の取得 → run フォルダの status 同期 → mb 複製集合 → todo カード走査 →
/// カード一覧の組み立てまでを 1 度に行う。
pub struct ViewPrep {
    /// 直近 100 件と Workflow 移動済み全件の実行履歴（フォルダ同期済み）
    pub executions: Vec<ExecutionHistory>,
    /// メインブロック複製（mb1.x）の実行 ID 集合
    pub mb_replica_execs: HashSet<i64>,
    /// Kanban ボード用カード一覧（レーン別）
    pub kanban_entries: Vec<CardEntry>,
    /// todo カードの走査結果（run ID → 説明文）。
    ///
    /// `load_workflow`（Workflow の DAG 材料）が block の description 解決に
    /// 同じ辞書を使い回すため公開する（再走査しない）。
    pub todo_by_run_id: HashMap<i64, String>,
}

/// ビュー組み立ての事前準備を行う（TUI `load_data` と同じ手順の部品）。
pub fn prepare_view(store: &SaiStore) -> ViewPrep {
    let mut executions = match store.list_view_executions(100) {
        Ok(v) => v,
        Err(_) => Vec::new(),
    };
    // run フォルダを状態（todo/running/review/done/workflow）に合わせて同期する。
    for e in executions.iter_mut() {
        sai_link_engine::run_dir::sync_run_owner(store, e);
    }
    let mb_replica_execs: HashSet<i64> = store
        .list_all_run_batch_mb_execs()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let (todo_by_run_id, todo_unlinked) = scan_todo_cards();
    let kanban_entries = collect_card_entries_from(
        store,
        &executions,
        &todo_by_run_id,
        &todo_unlinked,
        &mb_replica_execs,
    );
    ViewPrep {
        executions,
        mb_replica_execs,
        kanban_entries,
        todo_by_run_id,
    }
}
