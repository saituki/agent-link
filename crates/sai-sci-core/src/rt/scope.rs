//! `--paper` `--papers` `--node` `--subtree` から、検索の論文の集合と、
//! 検索の記録（`rt_searches.scope`）に書く範囲の文字列を決める。
//!
//! Node の扱いは研究方針の木の物なのでここに置く。論文の番号の集まりだけの部分は
//! `sai_paper_core::retrieval::resolve_paper_scope` にある。

use rusqlite::Connection;

use super::store;
use super::{Kind, RtError, fmt_id, fmt_ref};
use sai_paper_core::retrieval::{RetrievalError, resolve_paper_scope};

/// `--paper` `--papers` `--node` `--subtree` を、検索の論文の集合と、
/// 検索の記録（`rt_searches.scope`）に書く範囲の文字列にする。
///
/// どれも無ければ `(None, "library")`。
/// `--node` は `rt_nodes.id`。その Node と直の子（`subtree` なら子孫すべて）の生きた F の
/// `paper_id` と `cites` の論文番号を集める。論文が 0 本なら `(Some(空), "node:Q7")`
/// （0 本の Node では Library 全体を返さない）。
///
/// # Errors
///
/// 生きた Node が無い、または DB エラー。
pub fn resolve_scope(
    conn: &Connection,
    paper: Option<i64>,
    papers: &[i64],
    node: Option<i64>,
    subtree: bool,
) -> Result<(Option<Vec<i64>>, String), RetrievalError> {
    let Some(node_id) = node else {
        // Node が無ければ、論文の番号の集まりだけの部分と同じ結果になる。
        return resolve_paper_scope(conn, paper, papers);
    };

    // Node が無いときは、そのことを伝える（黙って Library 全体を返さない）。
    let row = store::get_node(conn, node_id)
        .ok()
        .filter(|row| row.removed_at.is_none())
        .ok_or_else(|| {
            RetrievalError::Other(format!("{} はありません", fmt_ref('N', node_id)))
        })?;
    let node_name = fmt_id(row.kind, row.id);
    let read_err =
        |e: RtError| RetrievalError::Other(format!("{node_name} を読めません: {e}"));

    // `--paper` `--papers` の分（Node が無いときと同じ集め方）。
    let mut ids = resolve_paper_scope(conn, paper, papers)?
        .0
        .unwrap_or_default();

    let targets = if subtree {
        store::subtree_ids(conn, node_id).map_err(read_err)?
    } else {
        let mut targets = vec![node_id];
        for child in store::children(conn, node_id).map_err(read_err)? {
            targets.push(child.id);
        }
        targets
    };
    for id in targets {
        let found = store::get_node(conn, id).map_err(read_err)?;
        if found.kind != Kind::F {
            continue;
        }
        ids.extend(found.paper_id);
        ids.extend(
            found
                .cites
                .iter()
                .filter_map(|cite| sai_paper_core::textcheck::parse_cite(cite))
                .map(|cite| cite.paper),
        );
    }
    ids.sort_unstable();
    ids.dedup();

    // 記録に書く範囲の文字列。
    let prefix = if subtree { "subtree" } else { "node" };
    Ok((Some(ids), format!("{prefix}:{node_name}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sai_paper_core::Database;

    /// Q の下に O を置き、O の F（論文 1 は paper_id・論文 3 は cites）と、
    /// 孫の F（論文 4）と、✕ した F（論文 5）を作る。
    fn scope_db() -> Database {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let add = |kind, parent: Option<i64>, text: &str, paper: Option<i64>, cites: &[String]| {
            store::insert_node(
                conn,
                &store::NewNode {
                    kind,
                    parent_id: parent,
                    guide_key: None,
                    text,
                    body: "",
                    answer_kind: (kind == Kind::O).then_some("answer"),
                    cites,
                    paper_id: paper,
                    run_id: None,
                    report_id: None,
                    mlflow_ref: None,
                    proposal_id: None,
                    paths: &[],
                    created_by: "user",
                },
            )
            .unwrap()
        };
        let q = add(Kind::Q, None, "問い", None, &[]);
        let node = add(Kind::O, Some(q), "手法", None, &[]);
        add(Kind::F, Some(node), "根拠 1", Some(1), &[]);
        add(Kind::F, Some(node), "根拠 3", None, &["#3 §2.1".to_string()]);
        let removed = add(Kind::F, Some(node), "外した根拠", Some(5), &[]);
        store::remove_subtree(conn, removed).unwrap();
        let child_q = add(Kind::Q, Some(node), "下の問い", None, &[]);
        let child_o = add(Kind::O, Some(child_q), "下の答え", None, &[]);
        add(Kind::F, Some(child_o), "根拠 4", Some(4), &[]);
        db
    }

    /// `paper` / `papers` / `node` を、論文の集合と範囲の文字列にする。
    #[test]
    fn resolve_scope_unions_and_names_scope() {
        let db = scope_db();
        let conn = db.connection();

        // 木の中の位置を調べる（Q → O → 子の Q → 子の O）。
        let q: i64 = conn
            .query_row(
                "SELECT id FROM rt_nodes WHERE kind = 'Q' AND parent_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let node: i64 = conn
            .query_row(
                "SELECT id FROM rt_nodes WHERE kind = 'O' AND parent_id = ?1",
                [q],
                |r| r.get(0),
            )
            .unwrap();
        let child_q: i64 = conn
            .query_row(
                "SELECT id FROM rt_nodes WHERE kind = 'Q' AND parent_id = ?1",
                [node],
                |r| r.get(0),
            )
            .unwrap();

        // O の直の F（1, 3）に `--paper 2` を足した集合。
        let (ids, scope) = resolve_scope(conn, Some(2), &[], Some(node), false).unwrap();
        assert_eq!(ids, Some(vec![1, 2, 3]));
        assert_eq!(scope, format!("node:O{node}"));

        // 子孫もなら `subtree:O…` で、孫の F の論文 4 も入る。
        let (ids, scope) = resolve_scope(conn, None, &[], Some(node), true).unwrap();
        assert_eq!(ids, Some(vec![1, 3, 4]));
        assert_eq!(scope, format!("subtree:O{node}"));

        // 節点が無ければ `papers:1,2`。
        let (ids, scope) = resolve_scope(conn, None, &[2, 1, 2], None, false).unwrap();
        assert_eq!(ids, Some(vec![1, 2]));
        assert_eq!(scope, "papers:1,2");

        // どれも無ければ絞らない。
        let (ids, scope) = resolve_scope(conn, None, &[], None, false).unwrap();
        assert_eq!(ids, None);
        assert_eq!(scope, "library");

        // Node に論文が無くても `Some(空)`（Library 全体を返さない）。
        let (ids, scope) = resolve_scope(conn, None, &[], Some(child_q), true).unwrap();
        assert_eq!(ids, Some(vec![4]));
        assert_eq!(scope, format!("subtree:Q{child_q}"));
        let (ids, scope) = resolve_scope(conn, None, &[], Some(q), false).unwrap();
        assert_eq!(ids, Some(Vec::new()));
        assert_eq!(scope, format!("node:Q{q}"));

        // ✕ した Node と、無い Node は失敗する。
        let removed: i64 = conn
            .query_row(
                "SELECT id FROM rt_nodes WHERE removed_at IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let err = resolve_scope(conn, None, &[], Some(removed), false).unwrap_err();
        assert!(err.to_string().contains(&format!("N{removed} はありません")), "{err}");
        let err = resolve_scope(conn, None, &[], Some(9999), false).unwrap_err();
        assert!(err.to_string().contains("N9999 はありません"), "{err}");
    }
}
