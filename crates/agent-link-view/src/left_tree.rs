//! 左サイドバー ツリーの組み立て（TUI / CLI 共通・実装計画 12 §3.1 / §3.2）。
//!
//! 旧 `agent-link-tui::app::data::build_left_tree_from_executions_slice` の本体を
//! 移設した。TUI は返り値を `TreeItem`（描画用）へ変換するだけで、見た目・
//! 折りたたみ挙動は変わらない。CLI（`agent-link phone tree`）は同じ組み立て結果を
//! JSON に組み替えて返す。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::card_entries::{
    card_path_for_execution, pending_card_paths, pending_run_ids, todo_card_info, CardEntry,
};
use agent_link_engine::db::models::ExecutionHistory;
use agent_link_engine::db::SaiStore;

/// フォルダ表示用のアイコン（Nerd Font のフォルダ）。
/// 全フォルダ（todo/running/review/日付）名の先頭に付与する。
/// リーフ（`ID<run_id>/<block名>`）・カードには付けない。
pub const FOLDER_ICON: &str = "󰉋 ";

/// `todo` フォルダの固定 ID（実行と衝突しない負値）。
pub const TODO_FOLDER_ID: i64 = -1_000;
/// `running` フォルダの固定 ID。
pub const RUNNING_FOLDER_ID: i64 = -1_001;
/// `review` フォルダの固定 ID。
pub const REVIEW_FOLDER_ID: i64 = -1_002;
/// 日付フォルダ（`yymmdd`）の一意 ID（他のフォルダ・カードと衝突しない負値）。
pub fn date_folder_id(date: &str) -> i64 {
    -1_000_000i64 - date.parse::<i64>().unwrap_or(0)
}

/// カードリーフの ID（連番ベース・他 ID と衝突しない負値）。
pub fn todo_card_id(index: usize) -> i64 {
    -100_000i64 - index as i64
}

/// ツリーアイテムの種別（`TreeItemKind` と対応）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftTreeKind {
    /// フォルダ（todo / running / review / 日付）
    Folder,
    /// 実行リーフ（`ID<run_id>/<block名>`）
    Leaf,
    /// todo カード（`outputs/todo/<run_id>/task.md` / 旧 `.agent-link/todo/*.md`）
    Card,
}

/// ツリーアイテム1件（平らな一覧・TUI `TreeItem` と対応）。
///
/// `lane` / `status` / `run_id` は `phone tree` の JSON 用の追加情報で、
/// TUI の描画には使わない（無い項目は `None`）。
#[derive(Debug, Clone)]
pub struct LeftTreeItem {
    /// フォルダ / 実行 / カードの一意 ID（負値はフォルダ・カード）
    pub id: i64,
    /// 表示名（フォルダは `FOLDER_ICON` 付き）
    pub name: String,
    /// インデントレベル（0 = ルート）
    pub depth: usize,
    /// 展開状態の初期値（`prev_expanded` から解決済み）
    pub expanded_default: bool,
    /// 最終ノードか（ツリー描画用）
    pub is_last_child: bool,
    /// アイテム種別
    pub kind: LeftTreeKind,
    /// 各祖先が「最後の子」か（ツリー接続線の描画用）
    pub ancestor_last: Vec<bool>,
    /// 対応カード md のパス（TUI の `c` / `d` 用）
    pub card_path: Option<PathBuf>,
    /// 実行 ID（カード・実行リーフ。無ければ `None`）
    pub run_id: Option<i64>,
    /// 所属レーン（`phone tree` JSON 用: todo / running / review / workflow）
    pub lane: Option<String>,
    /// 親 run の ID（`phone tree` の入れ子用。親が無い・リーフでなければ `None`）
    pub parent_run_id: Option<i64>,
    /// DB status（`phone tree` JSON 用。無ければ `None`）
    pub status: Option<String>,
}

/// 左サイドバーのツリーを取得済みの実行履歴から構築する（平らな一覧）。
///
/// 構成:
/// - 最上位（depth 0）: `todo` / `running` / `review` フォルダ、続けて日付フォルダ（`yymmdd`・古い順）
/// - 各フォルダの下（depth 1）: `ID<run_id>/<ブロック名>`（親子は `parent_run_id` で表す）
///
/// 日付フォルダは Workflow へ移した run（`workflow_moved`）の置き場で、日付は
/// 一番上の親 run の日付（`run_dir::root_run_date`）。中の並びは `run_id` の昇順。
/// sidebar の重複禁止: 未移動（Kanban 所有）の実行は running/review フォルダのみに載せ、
/// Workflow へ移した実行は日付フォルダのみに載せる。
/// mb 複製（U13・R25）は日付フォルダから除外する。
///
/// - `store` — 対象プロジェクトの `SaiStore`
/// - `executions` — 直近 100 件と Workflow へ移した実行（`prepare_view` 済みを想定）
/// - `lane_entries` — Kanban ボード用カード一覧（running / review の中身）
/// - `mb_replica_execs` — 除外する mb 複製の実行 ID
/// - `prev_expanded` — 前回のフォルダ展開状態（`フォルダID → expanded`）
pub fn build_left_tree(
    store: &SaiStore,
    executions: &[ExecutionHistory],
    lane_entries: &[CardEntry],
    mb_replica_execs: &HashSet<i64>,
    prev_expanded: &HashMap<i64, bool>,
) -> Vec<LeftTreeItem> {
    // 実行 ID → (status, parent_run_id)（phone 用 lane / status / 入れ子の解決）
    let status_of: HashMap<i64, (String, Option<i64>)> = executions
        .iter()
        .filter_map(|e| e.id.map(|id| (id, (e.status.clone(), e.parent_run_id))))
        .collect();

    // (日付, &exec) の一覧を構築する（Workflow へ移した実行だけ）。
    // ソート順: 日付昇順 → run_id 昇順
    let mut entries: Vec<(String, &ExecutionHistory)> = Vec::with_capacity(executions.len());
    for e in executions.iter() {
        // U13・R25: メインブロック複製（mb1.x）は左SB の実行一覧に表示しない
        if mb_replica_execs.contains(&e.id.unwrap_or(0)) {
            continue;
        }
        // 未移動はレーン側のみ（日付フォルダとの重複を避ける）
        if !e.workflow_moved {
            continue;
        }
        entries.push((agent_link_engine::run_dir::root_run_date(store, e), e));
    }
    entries.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.id.unwrap_or(0).cmp(&b.1.id.unwrap_or(0)))
    });

    let mut block_name_cache: HashMap<i64, String> = HashMap::new();

    let mut items: Vec<LeftTreeItem> = Vec::with_capacity(executions.len() + 32);

    // sidebar 最上位: `todo` / `running` / `review` フォルダ（同列）。
    {
        // todo 載せ条件は `todo_card_paths` に共通化（`phone card run` も同じものを使う）。
        let todo_cards: Vec<PathBuf> = todo_card_paths(executions);
        let todo_open = prev_expanded.get(&TODO_FOLDER_ID).copied().unwrap_or(false);
        items.push(LeftTreeItem {
            id: TODO_FOLDER_ID,
            name: format!("{}todo", FOLDER_ICON),
            depth: 0,
            expanded_default: todo_open,
            is_last_child: false,
            kind: LeftTreeKind::Folder,
            ancestor_last: Vec::new(),
            card_path: None,
            run_id: None,
            lane: Some("todo".to_string()),
            parent_run_id: None,
            status: None,
        });
        for (ci, card_path) in todo_cards.iter().enumerate() {
            // todo 子の表示名は Kanban todo Card と同一 `ID<run_id>/<block名>`（A1/A6）。
            // 読み失敗時はファイル名にフォールバック（表示を止めない）。
            let info = std::fs::read_to_string(card_path)
                .ok()
                .and_then(|t| agent_link_engine::kanban::parse_card(&t))
                .and_then(|c| todo_card_info(card_path, &c));
            let (label, run_id) = match &info {
                Some(info) => (info.title(), info.run_id),
                None => (
                    card_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| card_path.display().to_string()),
                    None,
                ),
            };
            let status = run_id.and_then(|rid| status_of.get(&rid).map(|(s, _)| s.clone()));
            let parent_run_id = run_id.and_then(|rid| status_of.get(&rid).and_then(|(_, p)| *p));
            items.push(LeftTreeItem {
                id: todo_card_id(ci),
                name: label,
                depth: 1,
                expanded_default: false,
                is_last_child: ci + 1 == todo_cards.len(),
                kind: LeftTreeKind::Card,
                ancestor_last: Vec::new(),
                card_path: Some(card_path.clone()),
                run_id,
                lane: Some("todo".to_string()),
                parent_run_id,
                status,
            });
        }
    }
    // `running` / `review` フォルダ：Kanban ボードと同一分類。
    // 覚え置き済みの `lane_entries` を使い回す（再計算なし）。
    {
        for (folder_id, folder_name, lane) in [
            (
                RUNNING_FOLDER_ID,
                format!("{}running", FOLDER_ICON),
                agent_link_engine::kanban::KanbanLane::Running,
            ),
            (
                REVIEW_FOLDER_ID,
                format!("{}review", FOLDER_ICON),
                agent_link_engine::kanban::KanbanLane::Review,
            ),
        ] {
            let open = prev_expanded.get(&folder_id).copied().unwrap_or(false);
            let lane_str = match lane {
                agent_link_engine::kanban::KanbanLane::Todo => "todo",
                agent_link_engine::kanban::KanbanLane::Running => "running",
                agent_link_engine::kanban::KanbanLane::Review => "review",
            };
            items.push(LeftTreeItem {
                id: folder_id,
                name: folder_name.to_string(),
                depth: 0,
                expanded_default: open,
                is_last_child: false,
                kind: LeftTreeKind::Folder,
                ancestor_last: Vec::new(),
                card_path: None,
                run_id: None,
                lane: Some(lane_str.to_string()),
                parent_run_id: None,
                status: None,
            });
            // 当該レーンの実行（`exec_id` あり）のみ。mb 複製は除外（日付フォルダと一致）。
            let lane_items: Vec<_> = lane_entries
                .iter()
                .filter(|e| {
                    e.lane == lane
                        && e.exec_id.is_some()
                        && !mb_replica_execs.contains(&e.exec_id.unwrap_or(0))
                })
                .collect();
            for (li, e) in lane_items.iter().enumerate() {
                let exec_id = e.exec_id.unwrap_or(0);
                let status = status_of.get(&exec_id).map(|(s, _)| s.clone());
                let parent_run_id = status_of.get(&exec_id).and_then(|(_, p)| *p);
                items.push(LeftTreeItem {
                    id: exec_id,
                    name: e.title.clone(),
                    depth: 1,
                    expanded_default: false,
                    is_last_child: li + 1 == lane_items.len(),
                    kind: LeftTreeKind::Leaf,
                    ancestor_last: Vec::new(),
                    card_path: e.card_path.clone(),
                    run_id: Some(exec_id),
                    lane: Some(lane_str.to_string()),
                    parent_run_id,
                    status,
                });
            }
        }
    }

    // ── 日付フォルダ（Workflow へ移した実行）────────────────────
    let mut i = 0usize;
    while i < entries.len() {
        let date = entries[i].0.clone();
        let mut j = i;
        while j < entries.len() && entries[j].0 == date {
            j += 1;
        }
        let folder_id = date_folder_id(&date);
        items.push(LeftTreeItem {
            id: folder_id,
            name: format!("{}{}", FOLDER_ICON, date),
            depth: 0,
            expanded_default: prev_expanded.get(&folder_id).copied().unwrap_or(false),
            is_last_child: j == entries.len(),
            kind: LeftTreeKind::Folder,
            ancestor_last: Vec::new(),
            card_path: None,
            run_id: None,
            lane: Some("workflow".to_string()),
            parent_run_id: None,
            status: None,
        });

        // リーフ（実行）: ID<run_id>/<block_name>
        for (li, (_, exec)) in entries[i..j].iter().enumerate() {
            let block_id = exec.block_id;
            let block_name = if let Some(n) = block_name_cache.get(&block_id) {
                n.clone()
            } else {
                let b = store.get_block_by_id(block_id).ok();
                let n = b
                    .as_ref()
                    .map(|b| b.display_name.clone())
                    .unwrap_or_else(|| format!("(block_id={})", block_id));
                block_name_cache.insert(block_id, n.clone());
                n
            };
            let exec_id = exec.id.unwrap_or(0);
            // 日付フォルダ配下の実行リーフも `c` でカードを開けるようパスを解決
            let card_path = card_path_for_execution(exec.kanban_card_path.as_deref(), exec_id);
            items.push(LeftTreeItem {
                id: exec_id,
                name: format!("ID{}/{}", exec_id, block_name),
                depth: 1,
                expanded_default: false,
                is_last_child: li == j - i - 1,
                kind: LeftTreeKind::Leaf,
                ancestor_last: Vec::new(),
                card_path,
                run_id: Some(exec_id),
                lane: Some("workflow".to_string()),
                parent_run_id: exec.parent_run_id,
                status: Some(exec.status.clone()),
            });
        }
        i = j;
    }

    items
}

/// todo フォルダに載せるカードパス一覧（`phone tree` の todo 条件）。
///
/// `phone card run <run_id>` の対象選択からも同じ関数を呼んで二重実装しない。
///
/// - 新: `outputs/todo/<run_id>/task.md` ＋ 旧: `.agent-link/todo/*.md`（移行期のみ）
/// - DB 上で kanban 実行に紐付いたカード（`kanban_card_path`）は載せない
/// - 送信済み（`pending/` の `block_run_request` が指す run_id / カードパス）は載せない
pub fn todo_card_paths(executions: &[ExecutionHistory]) -> Vec<PathBuf> {
    // 新：outputs/todo/<run_id>/task.md（予約 run 方式）＋旧 `.agent-link/todo/*.md`（移行期のみ）
    let mut todo_cards: Vec<PathBuf> = Vec::new();
    let outputs_todo = agent_link_engine::project::sai_path(agent_link_engine::run_dir::OUTPUTS_DIR)
        .join(agent_link_engine::run_dir::TODO_OWNER);
    if let Ok(dir_entries) = std::fs::read_dir(&outputs_todo) {
        for entry in dir_entries.flatten() {
            let task = entry.path().join(agent_link_engine::run_dir::TASK_FILE);
            if task.is_file() {
                todo_cards.push(task);
            }
        }
    }
    let todo_dir = agent_link_engine::project::sai_path(agent_link_engine::kanban::TODO_DIR);
    todo_cards.extend(agent_link_engine::kanban::list_todo_cards(&todo_dir));
    todo_cards.sort();
    // 排他: DB 上で kanban 実行に紐付いたカードは todo に載せない
    let claimed: HashSet<PathBuf> = executions
        .iter()
        .filter_map(|e| e.kanban_card_path.as_deref())
        .map(|rel| agent_link_engine::project::project_root().join(rel))
        .collect();
    // 予約 run 方式：送信済み run_id は todo に載せない（running 予定地）
    let pending_ids = pending_run_ids();
    let pending_paths = pending_card_paths();
    todo_cards
        .into_iter()
        .filter(|p| {
            if claimed.contains(p) {
                return false;
            }
            // task.md の run_id が pending なら除外
            if let Ok(text) = std::fs::read_to_string(p) {
                if let Some(card) = agent_link_engine::kanban::parse_card(&text) {
                    if let Ok(rid) = card.frontmatter.run_id.trim().parse::<i64>() {
                        if pending_ids.contains(&rid) {
                            return false;
                        }
                    }
                }
            }
            // 同名ファイルでの照合はしない（全カードが `task.md` という同じ名前のため、
            // 送信 1 枚で残り全件が「送信済み」と誤判定され todo から全消えしていた）。
            // ファイル1つ1つはフルパスでしか区別できないので、パス完全一致のみで照合する。
            !pending_paths.iter().any(|q| *q == *p)
        })
        .collect()
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agent_link_engine::db::models::ExecutionHistory;

    /// 送信 1 枚で、他の同名カード（全カードが `task.md`）まで todo から消えないこと。
    ///
    /// 以前はファイル名での照合をしており、`outputs/todo/<run_id>/task.md` が
    /// 全件同じ名前のため、1 枚を実行送信しただけで残り全件が「送信済み」と
    /// 誤判定されて todo から一斉に消えていた（常駐が止まっていると消えたまま）。
    #[test]
    fn pending_one_card_keeps_other_task_md_cards() {
        // AGENT_LINK_PROJECT_ROOT を書き換えるので、ほかのテストと同じロックで排他する
        let _env = crate::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!(
            "sai_lefttree_pending_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".agent-link")).unwrap();
        std::env::set_var("AGENT_LINK_PROJECT_ROOT", &tmp);

        // カード 2 枚（どちらもファイル名は task.md）
        for run_id in [91001_i64, 91002_i64] {
            let dir = tmp.join(".agent-link/outputs/todo").join(run_id.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            let text = format!(
                "---\nrun_id: {}\ndescription: カード{}\ncreation_date: 2026-01-01\nblock: b\n---\n本文\n",
                run_id, run_id
            );
            std::fs::write(dir.join("task.md"), text).unwrap();
        }

        // 91001 だけを実行送信済み（pending の block_run_request）
        let pending = tmp
            .join(".agent-link")
            .join("engine")
            .join("notifications")
            .join("pending");
        std::fs::create_dir_all(&pending).unwrap();
        let req = serde_json::json!({
            "type": "block_run_request",
            "card_path": ".agent-link/outputs/todo/91001/task.md",
        });
        std::fs::write(
            pending.join("req1.json"),
            serde_json::to_string(&req).unwrap(),
        )
        .unwrap();

        let no_execs: Vec<ExecutionHistory> = Vec::new();
        let cards = todo_card_paths(&no_execs);
        let names: Vec<String> = cards
            .iter()
            .map(|p| {
                p.parent()
                    .and_then(|d| d.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            })
            .collect();
        assert!(
            !names.iter().any(|n| n == "91001"),
            "送信済みの 91001 は載せない: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n == "91002"),
            "送信していない同名カード 91002 は残る（全消えの回帰防止）: {:?}",
            names
        );

        std::fs::remove_dir_all(&tmp).ok();
        std::env::remove_var("AGENT_LINK_PROJECT_ROOT");
    }
}
