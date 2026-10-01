//! `load_research_dag`（research-tree の実行の図）のテスト。
//!
//! 図は 4 層（seed / root / sprout / branch）だけ`、`block_connections` にある
//! 3 種類の線だけを出す。一時 DB（メモリ上）と一時フォルダのカードで確かめる。

use std::path::Path;
use std::sync::MutexGuard;

use sai_link_engine::db::models::BlockConnection;
use sai_link_engine::db::SaiStore;

use super::*;
use crate::card_entries::ViewPrep;

/// テスト用の排他（ロックが壊れていてもテストは続行する）。
fn env_lock() -> MutexGuard<'static, ()> {
    crate::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// `SAI_PROJECT_ROOT` を一時ディレクトリに向けるガード（Drop で元に戻す）。
struct RootGuard(Option<std::ffi::OsString>);

impl RootGuard {
    fn set(path: &Path) -> Self {
        let prev = std::env::var_os("SAI_PROJECT_ROOT");
        std::env::set_var("SAI_PROJECT_ROOT", path);
        Self(prev)
    }
}

impl Drop for RootGuard {
    fn drop(&mut self) {
        match self.0.take() {
            Some(v) => std::env::set_var("SAI_PROJECT_ROOT", v),
            None => std::env::remove_var("SAI_PROJECT_ROOT"),
        }
    }
}

/// 図の材料を作るテスト用のプロジェクト（一時フォルダ + メモリ DB）。
///
/// 新しい依存は増やさない（一時フォルダは `std::env::temp_dir` に自分で作る）。
struct Fixture {
    root: std::path::PathBuf,
    _guard: RootGuard,
    store: SaiStore,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn new() -> Fixture {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("sai_research_dag_{}_{}", std::process::id(), nanos));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let guard = RootGuard::set(&root);
        let store = SaiStore::open_in_memory().unwrap();
        Fixture {
            root,
            _guard: guard,
            store,
        }
    }

    /// 層付きのブロックを登録して、ブロック ID を返す。
    fn block(&self, name: &str, display: &str, layer: Option<&str>) -> i64 {
        let mut block = sai_link_engine::db::BlockMetadata::new(
            name.to_string(),
            "テスト用のブロック".to_string(),
            "anthropic/claude-sonnet-4-5".to_string(),
            None,
            "none".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            format!("blocks/{}.md", name),
            None,
        );
        block.display_name = display.to_string();
        block.layer = layer.map(|s| s.to_string());
        self.store.insert_block(&block).unwrap()
    }

    /// 実行を 1 件作る（`workflow_moved = false` = Kanban 側の run）。
    fn run(&self, block_id: i64) -> i64 {
        self.store.create_execution(block_id, None).unwrap()
    }

    /// 線（`task_delegation` / `waggle_token` / `task_output`）を 1 本作る。
    fn conn(&self, from: i64, from_block: i64, to: i64, to_block: i64, conn_type: &str) {
        let c = match conn_type {
            "task_delegation" => {
                BlockConnection::new_task_delegation(from, from_block, to, to_block)
            }
            "task_output" => BlockConnection::new_task_output(from, from_block, to, to_block),
            other => BlockConnection::new_waggle(
                from,
                from_block,
                to,
                to_block,
                format!("tok-{}", other),
            ),
        };
        self.store.insert_connection(&c).unwrap();
    }

    /// run のカード（task.md）を書き、実行に紐付ける。返り値は相対パス。
    fn card(&self, run_id: i64, text: &str) -> String {
        let rel = format!(".sai/outputs/todo/{}/task.md", run_id);
        let path = self.root.join(&rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        self.store
            .update_execution_kanban_info(run_id, Some(&rel), "kanban")
            .unwrap();
        rel
    }

    /// 図を組み立てる（`prep` は説明文の辞書に todo の走査結果を使うため空でよい）。
    fn load(&self) -> ResearchDag {
        let prep = ViewPrep {
            executions: Vec::new(),
            mb_replica_execs: Default::default(),
            kanban_entries: Vec::new(),
            todo_by_run_id: Default::default(),
        };
        load_research_dag(&self.store, &prep)
    }
}

/// テスト用のカード本文（frontmatter に `sci_node:` を書き込んだもの）。
fn card_text(run_id: i64, description: &str, sci_node: Option<&str>) -> String {
    let sci_line = match sci_node {
        Some(v) => format!("sci_node: {}\n", v),
        None => String::new(),
    };
    format!(
        "---\nrunner: pi\ncreation_date: 2026/09/30 00:00:00\nexecution_date: -\nrun_id: {}\n\
         block: sci-exp-sprout\ndescription: {}\n{}---\n\n# task_text\n\nテスト\n",
        run_id, description, sci_line
    )
}

/// 層の有る run だけ node になり、両端が node に入っている線だけ edge になること。
///
/// 用意するもの:
/// - seed の run 1 件、branch の run 2 件（うち 1 件は Kanban 側 = `workflow_moved = false`）
/// - 層の無いブロックの run 1 件
/// - 線: `task_delegation` 2 本・`waggle_token` 1 本・層の無い run への線 1 本
#[test]
fn dag_has_layered_nodes_and_edges_between_them() {
    let _env = env_lock();
    let f = Fixture::new();
    let seed_block = f.block("sci-exp-sprout", "exp-seed", Some("seed"));
    let branch_block = f.block("sci-exp-branch", "exp-branch", Some("branch"));
    let plain_block = f.block("plain", "平日のブロック", None);

    let seed = f.run(seed_block);
    let branch = f.run(branch_block);
    let kanban_branch = f.run(branch_block);
    let plain = f.run(plain_block);

    // 2 本の委譲（seed → branch、seed → Kanban 側の branch）
    f.conn(seed, seed_block, branch, branch_block, "task_delegation");
    f.conn(
        seed,
        seed_block,
        kanban_branch,
        branch_block,
        "task_delegation",
    );
    // 統合する agent と元の node の線
    f.conn(
        branch,
        branch_block,
        kanban_branch,
        branch_block,
        "waggle_token",
    );
    // 層の無い run への線（図には出ない）
    f.conn(seed, seed_block, plain, plain_block, "task_delegation");

    let dag = f.load();
    assert_eq!(dag.nodes.len(), 3, "層の有る run だけ: {:?}", dag.nodes);
    assert_eq!(dag.edges.len(), 3, "両端が node の線だけ: {:?}", dag.edges);

    // node は run_id の昇順
    let ids: Vec<i64> = dag.nodes.iter().map(|n| n.run_id).collect();
    assert_eq!(ids, vec![seed, branch, kanban_branch]);
    // 層の無い run は出ない（Kanban 側の run は出る）
    assert!(!ids.contains(&plain));

    // 層と表示名
    assert_eq!(dag.nodes[0].layer, "seed");
    assert_eq!(dag.nodes[0].block, "sci-exp-sprout");
    assert_eq!(dag.nodes[0].name, "exp-seed");
    assert_eq!(dag.nodes[1].layer, "branch");

    // 線の種類（task_delegation は JSON で delegation）
    let kinds: Vec<&str> = dag.edges.iter().map(|e| e.kind.as_str()).collect();
    assert!(kinds.contains(&"delegation"), "{:?}", kinds);
    assert!(kinds.contains(&"waggle_token"), "{:?}", kinds);
    for e in &dag.edges {
        assert!(
            ["delegation", "waggle_token", "task_output"].contains(&e.kind.as_str()),
            "出してはいけない線: {:?}",
            e
        );
        assert!(ids.contains(&e.from) && ids.contains(&e.to), "{:?}", e);
    }
}

/// 同じ `(from, to, kind)` の線は 1 本にまとめられること。
#[test]
fn dag_merges_same_edges() {
    let _env = env_lock();
    let f = Fixture::new();
    let seed_block = f.block("sci-exp-sprout", "exp-seed", Some("seed"));
    let branch_block = f.block("sci-exp-branch", "exp-branch", Some("branch"));
    let seed = f.run(seed_block);
    let branch = f.run(branch_block);

    f.conn(seed, seed_block, branch, branch_block, "task_delegation");
    f.conn(seed, seed_block, branch, branch_block, "task_delegation");
    f.conn(seed, seed_block, branch, branch_block, "task_output");

    let dag = f.load();
    assert_eq!(dag.nodes.len(), 2);
    assert_eq!(
        dag.edges.len(),
        2,
        "同じ線の重複はまとめる: {:?}",
        dag.edges
    );
}

/// カードの frontmatter に `sci_node: N12` があれば、その値が node に出ること。
#[test]
fn dag_reads_sci_node_from_card_frontmatter() {
    let _env = env_lock();
    let f = Fixture::new();
    let seed_block = f.block("sci-exp-sprout", "exp-seed", Some("seed"));
    let branch_block = f.block("sci-exp-branch", "exp-branch", Some("branch"));
    let with_sci = f.run(seed_block);
    let without_sci = f.run(branch_block);
    f.card(
        with_sci,
        &card_text(with_sci, "sci_node があるカード", Some("N12")),
    );
    f.card(
        without_sci,
        &card_text(without_sci, "sci_node が無いカード", None),
    );

    let dag = f.load();
    let a = dag.nodes.iter().find(|n| n.run_id == with_sci).unwrap();
    assert_eq!(a.description, "sci_node があるカード");
    assert_eq!(a.sci_node.as_deref(), Some("N12"));
    let b = dag.nodes.iter().find(|n| n.run_id == without_sci).unwrap();
    assert_eq!(b.sci_node, None);
    assert_eq!(b.description, "sci_node が無いカード");
}

/// 実行が 0 件でも空の DAG（node 0・edge 0）を返すこと。
#[test]
fn dag_is_empty_without_executions() {
    let _env = env_lock();
    let f = Fixture::new();
    let dag = f.load();
    assert!(dag.nodes.is_empty() && dag.edges.is_empty());
}
