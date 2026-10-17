//! Workflow の DAG 組み立て。
//!
//! - [`BlockDisplayData`] / [`ConnectionDisplayData`] — DAG の表示データ
//! - [`build_block_graph`] — 実行データから `DagGraph` と `LayoutGraph` を組み立てる
//!
//! CLI（`workflow`）がこの関数で DAG の形を作る。

use std::collections::HashMap;

use super::dag::{DagGraph, EdgeData, EdgeKind};
use super::layered_layout::LayoutGraph;

// ============================================================================
// BlockDisplayData / ConnectionDisplayData
// ============================================================================

/// メイン画面で表示するブロックデータ。
#[derive(Debug, Clone)]
pub struct BlockDisplayData {
    /// 実行ID（execution_history.id）。ノードの一意キーとして使用。
    pub exec_id: i64,
    /// ブロック定義ID（block_metadata.id）
    pub block_id: i64,
    /// ブロック名
    pub name: String,
    /// 日付フォルダ名（`yymmdd`・一番上の親 run の日付）。DAG の行グループと区切り線に使用
    pub date: String,
    /// 親実行ID（サブブロックの場合）
    pub parent_exec_id: Option<i64>,
    /// タスクバッチID（`task create` 1 回ごとに付く。子の段の決定に使用）
    pub batch_id: Option<String>,
    /// 実行ステータス
    pub last_status: String,
    /// 所要時間
    pub last_duration_ms: Option<i64>,
    /// 実行日時（ISO 8601）。DATE 順ソートに使用
    pub created_at: String,
    /// Card 表示用の説明文（対応カードの frontmatter `description:`）。
    ///
    /// 対応カードが無い実行は空。ノード 3 行目以降に枠内表示する。
    pub description: String,
    /// モデル名（`execution_history.model`）。札に出す
    pub model: Option<String>,
    /// thinking（effort）の値（`execution_history.thinking`）。札に出す
    pub thinking: Option<String>,
}

/// メイン画面で表示する接続データ。
///
/// ブロック線図は**実行単位**で接続するため、実行 ID の組を保持する。
/// （block_id 単位で接続すると、入出力関係のない実行同士まで接続されてしまう）
#[derive(Debug, Clone)]
pub struct ConnectionDisplayData {
    /// 出力元の実行 ID
    pub source_execution_id: i64,
    /// 入力先の実行 ID
    pub target_execution_id: i64,
    /// 接続タイプ: "waggle_token" / "task_delegation" / "task_output"（線を引くのは waggle_token だけ）
    pub connection_type: String,
}

/// 子（子・孫…）を、`task create` 1 回ごとの段に割り当てる（段は 1 始まり）。
///
/// 親 run（`parent_exec_id` が無い run）ごとに、子孫を `(parent_exec_id, batch_id)` の組で
/// まとめる（`batch_id` が無い子は 1 人で 1 組）。組を「組の中の最小の exec_id」の昇順に
/// 並べ、1 番目を段 1、2 番目を段 2… とする。exec_id は作った順に増える。
fn assign_batch_rows(blocks: &[BlockDisplayData]) -> HashMap<i64, u32> {
    let parent_of: HashMap<i64, i64> = blocks
        .iter()
        .filter_map(|b| b.parent_exec_id.map(|p| (b.exec_id, p)))
        .collect();
    // 親 run ごとの組: root -> ((parent, batch_id or 自分の id) -> 子の exec_id 一覧)
    let mut by_root: HashMap<i64, HashMap<(i64, String), Vec<i64>>> = HashMap::new();
    for b in blocks {
        let Some(parent) = b.parent_exec_id else { continue };
        let mut root = b.exec_id;
        // 循環しても止まるよう、たどる回数はノード数までにする
        for _ in 0..blocks.len() {
            match parent_of.get(&root) {
                Some(&p) => root = p,
                None => break,
            }
        }
        let key = (
            parent,
            b.batch_id
                .clone()
                .unwrap_or_else(|| format!("#{}", b.exec_id)),
        );
        by_root
            .entry(root)
            .or_default()
            .entry(key)
            .or_default()
            .push(b.exec_id);
    }
    let mut rows: HashMap<i64, u32> = HashMap::new();
    for groups in by_root.into_values() {
        let mut groups: Vec<Vec<i64>> = groups.into_values().collect();
        groups.sort_by_key(|g| g.iter().copied().min());
        for (i, g) in groups.iter().enumerate() {
            for id in g {
                rows.insert(*id, i as u32 + 1);
            }
        }
    }
    rows
}

/// ブロックデータからレイアウトグラフを構築する。
///
/// 接続ルール:
/// - エッジは**実行単位**（source_execution_id → target_execution_id）で張る
/// - `task_delegation`: 子の `parent_exec_id` から必ず親 → 子（子どうしの鎖は作らない・
///   左右辺接地 `side: true`）
/// - `waggle_token`: 文書の受け渡し。両端が親どうし・子どうしなら上下辺接地、
///   親と子なら左右辺接地
/// - 両端の札が無い辺は引かない
///
/// 子の配置: `task create` 1 回 = 1 段（[`assign_batch_rows`]）。
pub fn build_block_graph(
    blocks: &[BlockDisplayData],
    connections: &[ConnectionDisplayData],
) -> (DagGraph<BlockDisplayData>, LayoutGraph) {
    let mut dag: DagGraph<BlockDisplayData> = DagGraph::new();
    let mut layout = LayoutGraph::new();

    // 1. ノード登録（exec_id を一意キーに）
    for block in blocks {
        let id = block.exec_id as u64;
        let idx = dag.add_node(id, block.clone());

        let name_len = block.name.chars().count() as u16;
        // 幅: ID行（`ID<run_id>/<block名>`）+ 余白で十分。
        // ただし日付行「MM/DD HH:MM:SS」（14文字）を途切れさせないため min 18 を確保。
        // ユーザー指定: Card を 1.5 倍に横長化する（既定幅 × 1.5・上限も拡大）。
        let width = (((name_len.max(14) + 4) as f64 * 1.5) as u16).clamp(27, 40);
        // 高さ: ID行 / 日付 / description(1行分) の表示（高さ 6）。
        // description は残り行数で切り詰めるため、枠外に出ない。
        let height = 6u16;
        layout.add_node(
            idx,
            id,
            width,
            height,
            block.date.clone(),
            block.created_at.clone(),
            block.parent_exec_id.map(|p| p as u64),
        );
    }

    // 2. 子の段
    for (id, row) in assign_batch_rows(blocks) {
        layout.set_batch_row(id as u64, row);
    }

    // 3. エッジ登録
    let block_by_exec: HashMap<i64, &BlockDisplayData> =
        blocks.iter().map(|b| (b.exec_id, b)).collect();

    // 3-1. 委譲エッジ（親 → 子。左右辺接地）
    for s in blocks {
        let Some(parent) = s.parent_exec_id else { continue };
        if let (Some(from_idx), Some(to_idx)) = (
            dag.index_of(parent as u64),
            dag.index_of(s.exec_id as u64),
        ) {
            dag.add_edge(
                from_idx,
                to_idx,
                EdgeData {
                    side: true,
                    kind: EdgeKind::Delegation,
                },
            );
            layout.add_edge(parent as u64, s.exec_id as u64, true);
        }
    }

    // 3-2. 実行単位の接続（waggle_token のみ。委譲は上で処理済み）。
    // 線は親子と waggle の受け渡しだけにする。
    for conn in connections {
        if conn.connection_type != "waggle_token" {
            continue;
        }

        let from_eid = conn.source_execution_id as u64;
        let to_eid = conn.target_execution_id as u64;
        if from_eid == to_eid {
            continue; // 自己ループはスキップ
        }
        let (Some(from_idx), Some(to_idx)) = (dag.index_of(from_eid), dag.index_of(to_eid)) else {
            continue; // 実行レコードが表示範囲に無い場合はスキップ
        };

        // waggle_token: 両方子 or 両方親 → 上下辺中点、親と子の混合のみ左右辺接地
        let src_is_sub = block_by_exec
            .get(&(from_eid as i64))
            .map(|b| b.parent_exec_id.is_some())
            .unwrap_or(false);
        let tgt_is_sub = block_by_exec
            .get(&(to_eid as i64))
            .map(|b| b.parent_exec_id.is_some())
            .unwrap_or(false);
        let side = src_is_sub != tgt_is_sub; // 両方サブ or 両方メイン → 上下辺
        dag.add_edge(
            from_idx,
            to_idx,
            EdgeData {
                side,
                kind: EdgeKind::WaggleToken,
            },
        );
        layout.add_edge(from_eid, to_eid, side);
    }

    // 4. ルートノードを設定（親を持たないノード）
    for block in blocks {
        let id = block.exec_id as u64;
        if block.parent_exec_id.is_none() && dag.root.is_none() {
            dag.root = dag.index_of(id);
        }
    }

    (dag, layout)
}
