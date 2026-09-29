//! research-tree の実行の図（3 層の DAG）の組み立て（U01）。
//!
//! 利用者は research-tree を「1 run = 1 node」の root / seed / branch の
//! 3 層 DAG として扱う（要件定義書 R-L1・R-L2）。このモジュールはその
//! **材料（node と edge）を DB から集める**ところまでを担当する。
//! Web（`sai-moon-link phone rtree`）が同じ形を使う。
//!
//! - **node**: ブロックの `layer`（`root` / `seed` / `branch`）が書いてある
//!   ブロックの run だけ。1 run = 1 node。`run_id` の昇順に並べる
//! - **edge**: `block_connections` にある次の 3 種類の線だけ。両端が node に
//!   入っているものだけを出す
//!   - `task_delegation`（親 → 子。JSON の `kind` は `delegation`）
//!   - `waggle_token`（統合する agent と元の node の線）
//!   - `task_output`
//!
//! `phone workflow` とは別の図なので、`workflow_moved`（Kanban 側の run）で
//! 載せる・載せないの判定はしない。**1 run を 1 札で出す**ため。

use std::collections::HashMap;

use sai_link_engine::db::SaiStore;

use crate::card_entries::{description_for_execution_cached, ViewPrep};

/// 出せる線（DB の `connection_type` → JSON の `kind`）。
///
/// 新しい connection_type は作らない（要件 R-L2）。
const EDGE_KINDS: [(&str, &str); 3] = [
    ("task_delegation", "delegation"),
    ("waggle_token", "waggle_token"),
    ("task_output", "task_output"),
];

/// research-tree の実行の図（3 層の DAG）。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ResearchDag {
    /// node（1 run = 1 node。`run_id` の昇順）
    pub nodes: Vec<ResearchNode>,
    /// edge（同じ `(from, to, kind)` は 1 本にまとめる）
    pub edges: Vec<ResearchEdge>,
}

/// 図の node（1 run = 1 node）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResearchNode {
    /// 実行 ID（`execution_history.id`）
    pub run_id: i64,
    /// ブロックの ID（`block_metadata.name`・例 `sci-exp-seed`）
    pub block: String,
    /// ブロックの表示名（例 `🌱 exp-seed`）
    pub name: String,
    /// 層（`root` / `seed` / `branch`）
    pub layer: String,
    /// 実行の状態（`completed` など。`execution_history.status`）
    pub status: String,
    /// 作成日時
    pub created_at: String,
    /// 終了日時（未終了は `None`）
    pub finished_at: Option<String>,
    /// 親 run（サブタスクじゃなければ `None`）
    pub parent_run_id: Option<i64>,
    /// カード（task.md）の frontmatter にある説明文（無ければ空文字）
    pub description: String,
    /// カードの frontmatter の `sci_node:` の値（`N12` の形。無いとき `None`）
    pub sci_node: Option<String>,
    /// グローバルラウンド
    pub global_round: Option<i64>,
    /// ローカルラウンド
    pub round: Option<i64>,
}

/// 図の edge（run と run の線）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResearchEdge {
    /// 始点側の実行 ID
    pub from: i64,
    /// 終点側の実行 ID
    pub to: i64,
    /// 線の種類（`delegation` / `waggle_token` / `task_output`）
    pub kind: String,
}

/// research-tree の実行の図を DB から集める。
///
/// - node は、ブロックの `layer` が書いてあるブロックの run だけ。
///   ブロックの情報はブロック ID ごとに 1 回だけ引く（`load_workflow` と同じようにキャッシュする）。
///   `workflow_moved` は見ない（Kanban 側の run も 1 枚で出す）。
/// - edge は `task_delegation` / `waggle_token` / `task_output` のうち
///   **両端が node に入っている**ものだけ。同じ `(from, to, kind)` は 1 本にまとめる。
/// - `sci_node` は run のカードの frontmatter の `sci_node:` の値。カードが無いときや
///   キーが無いときは `None`。
///
/// DB の読み損ねは空の DAG になる（`unwrap_or_default`）。
/// 読み損ねを空に落とすのは `load_workflow` と同じ方針。
pub fn load_research_dag(store: &SaiStore, prep: &ViewPrep) -> ResearchDag {
    // ブロック ID ごとの情報（名前・表示名・層）。1 ブロックにつき 1 回だけ引く。
    let mut block_cache: HashMap<i64, Option<(String, String, String)>> = HashMap::new();

    let executions = store.list_all_executions().unwrap_or_default();
    let mut nodes: Vec<ResearchNode> = Vec::new();
    let mut node_run_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for e in &executions {
        let block_id = e.block_id;
        let block_info = match block_cache.get(&block_id) {
            Some(cached) => cached.clone(),
            None => {
                // 層が書いてあるブロックだけ node にする（無ければ載せない）。
                // 読み損ねのブロックも `None` をキャッシュして引き直さない。
                let info = store
                    .get_block_by_id(block_id)
                    .ok()
                    .and_then(|b| b.layer.map(|layer| (b.name, b.display_name, layer)));
                block_cache.insert(block_id, info.clone());
                info
            }
        };
        let Some((block, name, layer)) = block_info else {
            continue;
        };
        let Some(run_id) = e.id else { continue };
        let description = description_for_execution_cached(
            store,
            e.kanban_card_path.as_deref(),
            run_id,
            &prep.todo_by_run_id,
        );
        let sci_node = sci_node_for_execution(e.kanban_card_path.as_deref(), run_id);
        node_run_ids.insert(run_id);
        nodes.push(ResearchNode {
            run_id,
            block,
            name,
            layer,
            status: e.status.clone(),
            created_at: e.created_at.clone(),
            finished_at: e.finished_at.clone(),
            parent_run_id: e.parent_run_id,
            description,
            sci_node,
            global_round: e.global_round,
            round: e.round,
        });
    }
    // 1 run = 1 node。実行 ID の昇順に並べる（作りたては作成日時順のため並べ替える）。
    nodes.sort_by_key(|n| n.run_id);

    // 線。両端が node に入っているものだけ、同じ (from, to, kind) は 1 本にまとめる。
    let mut edges: Vec<ResearchEdge> = Vec::new();
    let mut seen: std::collections::HashSet<(i64, i64, &str)> = std::collections::HashSet::new();
    for c in store.list_connections().unwrap_or_default() {
        let Some((_, kind)) = EDGE_KINDS
            .iter()
            .find(|(conn_type, _)| *conn_type == c.connection_type.as_str())
        else {
            continue;
        };
        if !node_run_ids.contains(&c.source_execution_id)
            || !node_run_ids.contains(&c.target_execution_id)
        {
            continue;
        }
        if !seen.insert((c.source_execution_id, c.target_execution_id, *kind)) {
            continue;
        }
        edges.push(ResearchEdge {
            from: c.source_execution_id,
            to: c.target_execution_id,
            kind: (*kind).to_string(),
        });
    }

    ResearchDag { nodes, edges }
}

/// run のカードの frontmatter の `sci_node:` の値（`N12` の形）を読む。
///
/// カード（`kanban_card_path`）を先に読み、無ければ run フォルダの task.md を
/// 読む。どちらも読めず・キーが無ければ `None`。
/// 解析は既存の `sai_link_engine::kanban::parse_card` に任せる（新しい解析器は書かない）。
fn sci_node_for_execution(card_rel: Option<&str>, run_id: i64) -> Option<String> {
    if let Some(rel) = card_rel {
        let path = sai_link_engine::project::project_root().join(rel);
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(card) = sai_link_engine::kanban::parse_card(&text) {
                return sci_node_in(&card.frontmatter);
            }
        }
    }
    if run_id > 0 {
        if let Some((_, dir)) = sai_link_engine::run_dir::find_run_dir(run_id) {
            let task = dir.join(sai_link_engine::run_dir::TASK_FILE);
            if let Ok(text) = std::fs::read_to_string(&task) {
                if let Some(card) = sai_link_engine::kanban::parse_card(&text) {
                    return sci_node_in(&card.frontmatter);
                }
            }
        }
    }
    None
}

/// カードの frontmatter から `sci_node:` を取り出す（空文字は `None` にする）。
fn sci_node_in(fm: &sai_link_engine::kanban::CardFrontmatter) -> Option<String> {
    fm.extra
        .get("sci_node")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests;
