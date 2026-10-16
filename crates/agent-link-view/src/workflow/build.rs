//! Workflow の DAG 組み立て（agent-link-tui の `ui/main_panel.rs` から移設）。
//!
//! - [`BlockDisplayData`] / [`ConnectionDisplayData`] — DAG の表示データ
//! - [`batch_round_num`] — `batch_<date>_r<N>_<hash>` の N 抽出
//! - [`build_block_graph`] — 実行データから `DagGraph` と `LayoutGraph` を組み立てる
//!
//! TUI（`agent-link-tui`）と CLI（`agent-link phone workflow`）はこの同じ関数を
//! 使い、DAG の形が 2 か所で食い違わないようにする。

use std::collections::{HashMap, HashSet};

use super::dag::{DagGraph, EdgeData, EdgeKind};
use super::layered_layout::LayoutGraph;

// ============================================================================
// BlockDisplayData / ConnectionDisplayData
// ============================================================================

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
    /// グローバルラウンド（GR）。DAG の行グループと区切り線に使用
    pub global_round: Option<i64>,
    /// ラウンド（LR）
    pub round: Option<i64>,
    /// 親実行ID（サブブロックの場合）
    pub parent_exec_id: Option<i64>,
    /// タスクバッチID（P3-A。サブ実行のフェーズ合成に使用）
    pub batch_id: Option<String>,
    /// フェーズ番号（P3-B2。合成小ブロックのみ設定。実実行は None）
    pub phase: Option<u32>,
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
    /// Reflection 記録の記入状況（Notification 色分け用）。
    ///
    /// 起動時・リロード時に判定した結果を引き回す。実行中・待機中など
    /// 色分け対象外の項目は `None`（従来色で表示する）。
    pub reflection_status: Option<agent_link_engine::reflection_status::ReflectionStatus>,
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
    /// 接続タイプ: "waggle_token" / "task_delegation" / "task_output" / "git_commit"
    pub connection_type: String,
}

/// batch_id からラウンド番号を抽出する（`batch_<date>_r<N>_<hash>` の N）。
/// 抽出できない場合は 0（バッチ順ソートで先頭に近く扱う）。
/// 注意: 「最初の `_r<N>` パターン」のみを解析する。ランダム接尾辞（hash）にも
/// `r<数字>` で始まるもの（例: "r5ca"）があるため、全走査・最大値採用にすると
/// バッチ順が入れ替わり、DAG の縦（フェーズ）配置が崩れる。
pub(crate) fn batch_round_num(batch_id: &Option<String>) -> i64 {
    match batch_id {
        Some(id) => {
            // 最初の `_r<N>` パターンを検索（例: "batch_20260901_r1_r5ca" → 1）
            let bytes = id.as_bytes();
            let mut i = 0usize;
            while i + 2 < bytes.len() {
                if bytes[i] == b'_' && bytes[i + 1] == b'r' && bytes[i + 2].is_ascii_digit() {
                    let mut j = i + 2;
                    let mut v: i64 = 0;
                    while j < bytes.len() && bytes[j].is_ascii_digit() {
                        v = v * 10 + i64::from(bytes[j] - b'0');
                        j += 1;
                    }
                    return v;
                }
                i += 1;
            }
            0
        }
        None => 0,
    }
}

/// ブロックデータからレイアウトグラフを構築する。
///
/// 接続ルール（`docs/block-position-rule.md` 準拠）:
/// - エッジは**実行単位**（source_execution_id → target_execution_id）で張る。
///   入力 waggle トークンが実際に使われた実行の組み合わせのみ接続される
/// - `task_delegation`（main→sub）: `parent_exec_id` から必ず接続
/// - `task_output`（sub→main）: 親がサブ出力を受け取った場合に接続
/// - 左右辺接地（`side: true`）: ブロック-サブブロック間の接続（委譲・返却、
///   および端点にサブブロックを含む接続）
/// - 上下辺接地（`side: false`）: main 同士の waggle_token / git_commit 接続
///
/// ## P3-B2: フェーズノードの合成（表示時）
///
/// サブ実行が `batch_id` を持つ場合、メイン実行を縦に分割したフェーズ列を合成する。
/// - フェーズ 0 = 実メイン実行 M（通常サイズ）
/// - フェーズ k (k>=1) = バッチ k のサブ群からの出力を受け取る合成小ブロック
///   （バッチ数 n なら P_1..P_n を生成。P_n が最終出力フェーズ）
/// - 委譲エッジ: フェーズ bg-1 → バッチ bg のサブ群（右辺接地）
/// - task_output エッジ: バッチ bg のサブ群 → フェーズ bg（右辺接地）
/// - サブ間依存（waggle_token）とメイン同士の接続は従来どおり描画する
pub fn build_block_graph(
    blocks: &[BlockDisplayData],
    connections: &[ConnectionDisplayData],
    finalized_runs: &HashSet<i64>,
) -> (DagGraph<BlockDisplayData>, LayoutGraph) {
    let mut dag: DagGraph<BlockDisplayData> = DagGraph::new();
    let mut layout = LayoutGraph::new();

    // P3-B2: 実メインとサブを分離
    let mains: Vec<&BlockDisplayData> = blocks
        .iter()
        .filter(|b| b.parent_exec_id.is_none())
        .collect();
    let subs: Vec<&BlockDisplayData> = blocks
        .iter()
        .filter(|b| b.parent_exec_id.is_some())
        .collect();

    // 1. ノード登録（exec_id を一意キーに。実ブロックのみ）
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
            block.global_round,
            block.round,
            block.created_at.clone(),
            block.parent_exec_id.map(|p| p as u64),
        );
    }

    // 2. P3-B2: フェーズノードの合成
    //    各メイン M について、S_M を batch_id でグループ化してバッチ順に並べ、
    //    フェーズノード列 P_0(=M), P_1..P_n を生成する。
    //    バッチサブには batch_group（1 始まり）を割り当てる。
    let mut batch_group_map: HashMap<i64, u32> = HashMap::new();
    // M.exec_id -> フェーズノード列の exec_id（P_0 = M 自身）
    let mut phase_node_ids: HashMap<i64, Vec<i64>> = HashMap::new();

    for m in &mains {
        let my_subs: Vec<&&BlockDisplayData> = subs
            .iter()
            .filter(|s| s.parent_exec_id == Some(m.exec_id))
            .collect();

        // P3-B2: フェーズ合成は「全サブが batch_id を持つ」場合のみ有効。
        // batch_id を持たないサブ（レガシー・テストデータ）は従来どおり
        // M の右側に縦積みする（フェーズ 0 のみ）。
        let all_batched = !my_subs.is_empty() && my_subs.iter().all(|s| s.batch_id.is_some());
        if my_subs.is_empty() || !all_batched {
            // サブなし、または batch_id なし: フェーズ 0 のみ（従来どおり）
            phase_node_ids.insert(m.exec_id, vec![m.exec_id]);
            continue;
        }

        // batch_id でグループ化（出現順を維持）
        let mut groups: Vec<(Option<String>, Vec<&&BlockDisplayData>)> = Vec::new();
        for s in &my_subs {
            let key = s.batch_id.clone();
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, list)) => list.push(*s),
                None => groups.push((key, vec![*s])),
            }
        }

        // バッチ順: batch_id のラウンド番号（r<N>）→ グループ内最小 created_at
        groups.sort_by(|a, b| {
            let ka = (
                batch_round_num(&a.0),
                a.1.iter().map(|s| s.created_at.as_str()).min(),
            );
            let kb = (
                batch_round_num(&b.0),
                b.1.iter().map(|s| s.created_at.as_str()).min(),
            );
            ka.cmp(&kb)
        });

        // バッチサブへ batch_group を割り当て（1 始まり）
        for (gi, (_, subs_in_group)) in groups.iter().enumerate() {
            for s in subs_in_group {
                batch_group_map.insert(s.exec_id, gi as u32 + 1);
            }
        }

        // フェーズノード個数の決定:
        // - P_1..P_{n-1}: n 回目の task_create（バッチ n+1 の開始）が P_n を生むため、
        //   バッチ数 n から 1 を引いた個数のフェーズノードを作る
        // - P_n: sai-write 検出済み（finalized 実行バッチあり）の場合のみ追加。
        //   sai-write は最終バッチの出力を受け取る最終出力フェーズとして表示する
        // - task_create 未使用で sai-write のみの場合は P は作らない（phase_count = 0 → 従来表示）
        let n_batches = groups.len();
        let finalized = finalized_runs.contains(&m.exec_id);
        let phase_count = if finalized { n_batches } else { n_batches - 1 };
        if phase_count == 0 {
            // フェーズノードなし: 従来どおり M + サブ横並びチェーンのみ
            phase_node_ids.insert(m.exec_id, vec![m.exec_id]);
            continue;
        }

        // フェーズノード列: P_0 = M、P_k (k=1..=phase_count) は合成小ブロック
        // フェーズ k は「バッチ k のサブ群の出力を受け取る」ため、
        // バッチ k の batch_id と実行日時（バッチ内の最小 created_at）を引き継ぐ。
        // 表示名は「メイン実行ID/batch_id」（ユーザー指定）。
        let mut phase_ids: Vec<i64> = vec![m.exec_id];
        for gi in 0..phase_count {
            let k = gi as i64 + 1;
            let (batch_id, subs_in_group) = &groups[gi];
            // 合成ノードの ID は実 exec_id と衝突しない値（M.exec_id * 10_000 + k）
            let phase_id = m.exec_id * 10_000 + k;
            let min_created = subs_in_group
                .iter()
                .map(|s| s.created_at.as_str())
                .min()
                .unwrap_or(m.created_at.as_str())
                .to_string();
            // 表示名: メイン実行 ID / batch_id（長い場合は後半を省略）
            let batch_label = batch_id.clone().unwrap_or_default();
            let name = if batch_label.chars().count() > 14 {
                let head: String = batch_label.chars().take(13).collect();
                format!("id{}/{}…", m.exec_id, head)
            } else {
                format!("id{}/{}", m.exec_id, batch_label)
            };
            let phase_block = BlockDisplayData {
                exec_id: phase_id,
                block_id: m.block_id,
                name,
                global_round: m.global_round,
                round: m.round,
                parent_exec_id: Some(m.exec_id), // フェーズ親
                batch_id: batch_id.clone(),      // バッチ k の batch_id（小ブロック表示用）
                phase: Some(k as u32),
                last_status: m.last_status.clone(),
                // 色判定結果は実メイン（元ノード）から引き継ぐ（Notification）
                reflection_status: m.reflection_status,
                description: m.description.clone(),
                last_duration_ms: m.last_duration_ms,
                created_at: min_created,
            };
            let idx = dag.add_node(phase_id as u64, phase_block);
            let name_len = m.name.chars().count() as u16;
            // 幅は日付行（MM/DD HH:MM:SS・14文字）が途切れないよう確保。
            // メインと同一の 1.5 倍幅規準に合わせる。
            let width = (((name_len.max(14) + 4) as f64 * 1.5) as u16).clamp(27, 40);
            // 小ブロック: メインと同じ高さ 6（ID行 / 日付 / description 1行）
            let height = 6u16;
            layout.add_node(
                idx,
                phase_id as u64,
                width,
                height,
                m.global_round,
                m.round,
                m.created_at.clone(),
                Some(m.exec_id as u64),
            );
            layout.set_phase(
                phase_id as u64,
                Some(m.exec_id as u64),
                Some(k as u32),
                None,
            );
            phase_ids.push(phase_id);
        }

        // 縦接続鎖: mb1 → P_1 → P_2 → ...（上下辺接地）。
        // 「次回の task_create / sai-write で次フェーズへ継続する」ことを表す。
        for w in phase_ids.windows(2) {
            if let (Some(from_idx), Some(to_idx)) =
                (dag.index_of(w[0] as u64), dag.index_of(w[1] as u64))
            {
                dag.add_edge(
                    from_idx,
                    to_idx,
                    EdgeData {
                        side: false,
                        kind: EdgeKind::PhaseChain,
                    },
                );
                layout.add_edge(w[0] as u64, w[1] as u64, false);
            }
        }

        phase_node_ids.insert(m.exec_id, phase_ids);
    }

    // バッチサブの batch_group をレイアウトに反映
    for s in &subs {
        if let Some(bg) = batch_group_map.get(&s.exec_id) {
            layout.set_phase(s.exec_id as u64, None, None, Some(*bg));
        }
    }

    // 3. エッジ登録
    // P15: exec_id → BlockDisplayData の索引を事前構築し、O(N²) の線形探索を排除する（4.2.1）
    let block_by_exec: HashMap<i64, &BlockDisplayData> =
        blocks.iter().map(|b| (b.exec_id, b)).collect();

    // サブ→メインの task_output 接続の存在セット（P3-B2 のエッジ読み替え用）
    let task_output_pairs: HashSet<(i64, i64)> = connections
        .iter()
        .filter(|c| c.connection_type == "task_output")
        .map(|c| (c.source_execution_id, c.target_execution_id))
        .collect();

    // 3-1. 委譲エッジ（フェーズ bg-1 → バッチ bg の最初のサブ + バッチ内チェーン sb_n → sb_n+1）
    // サブを (親, バッチ順, created_at) でソートし、同一バッチ内は横並びチェーンで接続する。
    let mut subs_sorted: Vec<&BlockDisplayData> = subs.iter().copied().collect();
    subs_sorted.sort_by(|a, b| {
        (
            a.parent_exec_id.unwrap_or(0),
            batch_group_map.get(&a.exec_id).copied().unwrap_or(0),
            a.created_at.as_str(),
        )
            .cmp(&(
                b.parent_exec_id.unwrap_or(0),
                batch_group_map.get(&b.exec_id).copied().unwrap_or(0),
                b.created_at.as_str(),
            ))
    });

    // バッチの先頭サブには「フェーズ bg-1 → サブ」の委譲を張り、
    // バッチ内のサブ同士は「sb_n → sb_n+1」のチェーンで接続する（左右辺接地）。
    let mut prev_key: Option<(i64, u32)> = None;
    let mut prev_exec: Option<i64> = None;
    for s in &subs_sorted {
        let m_id = s.parent_exec_id.unwrap_or(0);
        let bg = batch_group_map.get(&s.exec_id).copied().unwrap_or(1);
        let key = (m_id, bg);
        // 委譲元: 同一バッチ内の 2 番目以降は「前のサブ」、先頭は「フェーズ bg-1」
        let from_id: i64 = if prev_key == Some(key) {
            prev_exec.unwrap_or(m_id)
        } else if let Some(pid) = phase_node_ids
            .get(&m_id)
            .and_then(|ids| ids.get((bg as usize).saturating_sub(1)).copied())
        {
            // 委譲元フェーズノード（P_{bg-1}）
            pid
        } else {
            // P_{bg-1} が存在しない（例: 最終バッチに対応する P が sai-write 未検出で未合成）
            // 場合は前バッチの最終サブから委譲する
            prev_exec.unwrap_or(m_id)
        };
        if let (Some(from_idx), Some(to_idx)) =
            (dag.index_of(from_id as u64), dag.index_of(s.exec_id as u64))
        {
            dag.add_edge(
                from_idx,
                to_idx,
                EdgeData {
                    side: true,
                    kind: EdgeKind::Delegation,
                },
            );
            layout.add_edge(from_id as u64, s.exec_id as u64, true);
        }
        prev_key = Some(key);
        prev_exec = Some(s.exec_id);
    }

    // 3-2. task_output エッジ（バッチ bg のサブ → フェーズ bg）
    // 接地方法:
    // - フェーズ合成されたバッチサブ（batch_group あり）→ **上下辺接地**
    //   （sb の下辺中点 → フェーズノードの上辺中点・ユーザー要求）
    // - レガシーサブ（batch_id なし・フェーズ合成なし）→ 左右辺接地（従来どおり）
    for s in &subs {
        let m_id = s.parent_exec_id.unwrap_or(0);
        // 実データで s → m の task_output 接続が存在する場合のみフェーズへ読み替える
        if !task_output_pairs.contains(&(s.exec_id, m_id)) {
            continue;
        }
        let is_batched = batch_group_map.contains_key(&s.exec_id);
        let bg = batch_group_map.get(&s.exec_id).copied().unwrap_or(1);
        let phase_ids = phase_node_ids.get(&m_id);
        let phase_id = match phase_ids.and_then(|ids| ids.get(bg as usize).copied()) {
            Some(pid) => pid,
            None => {
                if is_batched {
                    // バッチサブだがフェーズ bg が未合成（最終バッチかつ sai-write 未検出）:
                    // 接続線を引かない（未取得であることをユーザーに見やすく伝えるため）。
                    continue;
                }
                // レガシー（batch_id なし・フェーズ合成なし）は従来どおり実メインへ
                phase_ids
                    .and_then(|ids| ids.first().copied())
                    .unwrap_or(m_id)
            }
        };
        if let (Some(from_idx), Some(to_idx)) = (
            dag.index_of(s.exec_id as u64),
            dag.index_of(phase_id as u64),
        ) {
            dag.add_edge(
                from_idx,
                to_idx,
                EdgeData {
                    side: !is_batched, // バッチサブ: 上下辺 / レガシーサブ: 左右辺
                    kind: EdgeKind::TaskOutput,
                },
            );
            layout.add_edge(s.exec_id as u64, phase_id as u64, !is_batched);
        }
    }

    // 3-3. 実行単位の接続（waggle_token のみ。task_delegation / task_output は上で処理済み）。
    // 線は親子と waggle の受け渡しだけにする（旧 kanban_origin / review / git_commit は引かない）。
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

        // waggle_token:
        // - **sb 同士（両端点がサブ）→ 上下辺中点で接続**（ユーザー要求）
        // - mb 同士 → 上下辺中点（従来どおり）
        // - mb-sb の混合（レア）のみ左右辺接地のまま
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

#[cfg(test)]
mod tests {
    use super::*;

    /// batch_round_num: 最初の `_r<N>` のみを解析する（ランダム接尾辞を誤読しない）。
    /// 実 DB のバッチ ID 例（2026-09-01 の検証実行）:
    /// - batch_20260901_r1_r5ca: 接尾辞 "r5ca" を "r5" と誤読して r1 より先に
    ///   ソートされ、バッチ順が入れ替わる不具合（DAG 縦配置崩れ）があった。
    #[test]
    fn batch_round_num_parses_only_first_rn() {
        let cases: Vec<(&str, i64)> = vec![
            ("batch_20260901_r1_r5ca", 1),
            ("batch_20260901_r1_irn1", 1),
            ("batch_abc_r2_def", 2),
            ("batch_20260901_r3_xi7z", 3),
            ("batch_20260901_r12_ab1", 12),
            ("batch_20260901_r1_12ab", 1),
            ("batch_no_round_suffix", 0),
        ];
        for (id, expected) in cases {
            let got = batch_round_num(&Some(id.to_string()));
            assert_eq!(got, expected, "batch_id={id}");
        }
        assert_eq!(batch_round_num(&None), 0);
    }
}
