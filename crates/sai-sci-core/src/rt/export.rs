//! 新しい木から `state.md` と `tree.md` を書き出す。
//!
//! 文は [`compute::status`] の結果だけから作る。木の文は検査済み（PROPOSAL の検査）なので、そのまま写す。
//! 消した Node は `status` に入らないので、ここでも出ない。

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;
use rusqlite::Connection;

use super::compute::{self, NodeJson, Status};
use super::defs::Defs;
use super::{Result, store};
use sai_paper_core::paths::PaperRoots;

/// 新しい木を始めたプロジェクトなら `state.md` と `tree.md` を書く。始めていなければ何もしない。戻り値は「書いたか」。
///
/// # Errors
///
/// DB の読み書きとファイルの書き込みに失敗したとき。
pub fn export(conn: &Connection, roots: &PaperRoots, defs: &Defs) -> Result<bool> {
    if !store::is_initialized(conn)? {
        return Ok(false);
    }
    let status = compute::status(conn, roots, defs)?;
    let data_dir = &roots.data_dir;
    fs::create_dir_all(data_dir)?;
    write_atomic(&sai_paper_core::paths::state_path(data_dir), &render_state_md(&status))?;
    write_atomic(&data_dir.join("tree.md"), &render_tree_md(&status))?;
    Ok(true)
}

/// 空なら「なし」。
fn or_none(text: String) -> String {
    if text.is_empty() { "なし".to_string() } else { text }
}

/// `state.md` の本文を作る（頭の 5 行は [`crate::project_files::parse_state`] が読む形）。
#[must_use]
pub fn render_state_md(status: &Status) -> String {
    let progress = format!("{}/{}", status.progress.answered, status.progress.total);

    // 一番上の Q の ★ の O の題。無ければ Q の文。
    let goal = status
        .nodes
        .iter()
        .find(|n| n.kind == "Q" && n.parent.is_none())
        .map(|q| {
            status
                .nodes
                .iter()
                .find(|n| n.kind == "O" && n.starred && n.parent.as_deref() == Some(q.id.as_str()))
                .map_or(&q.text, |o| &o.text)
                .clone()
        })
        .unwrap_or_default();

    let current = status
        .running
        .iter()
        .map(|r| match &r.node {
            Some(node) => format!("run {} {}（{node}）", r.run_id, r.block),
            None => format!("run {} {}", r.run_id, r.block),
        })
        .collect::<Vec<_>>()
        .join("、");

    let next = status.next.first().map_or_else(String::new, |a| match &a.block {
        Some(block) => format!("{}（{block}）", a.purpose),
        None => a.purpose.clone(),
    });

    let mut out = format!(
        "- 目標: {}\n- 段階: 進み具合 {progress}\n- 今の作業: {}\n- 次の一手: {}\n- 更新日: {}\n\n",
        or_none(goal),
        or_none(current),
        or_none(next),
        Local::now().format("%Y-%m-%d"),
    );
    out.push_str("<!-- このファイルは研究方針の木から書き出しています。直すときは Web か sai-sci を使います。 -->\n\n");

    out.push_str("## SEED\n");
    let draft = status.seed_draft.as_ref().map(|d| d.text.clone()).unwrap_or_default();
    out.push_str(&format!("- 下書き: {}\n", or_none(draft)));
    for seed in &status.seeds {
        out.push_str(&format!("- {} {}: {}\n", seed.id, seed.title, seed.sentence));
    }

    out.push_str(&format!(
        "\n## PROGRESS\n- 進み具合: {progress}\n- PENDING: {} 件\n",
        status.pending_proposals
    ));

    out.push_str("\n## NEXT_ACTION\n");
    if status.next.is_empty() {
        out.push_str("- なし\n");
    }
    for (i, a) in status.next.iter().take(3).enumerate() {
        let target = [a.block.as_deref(), a.node.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" / ");
        out.push_str(&format!("{}. [{}] {}（{target}）\n", i + 1, a.why, a.purpose));
    }

    out
}

/// Node の 1 行（`# context` と同じ形）。字下げは深さ × 2 字。
fn node_line(n: &NodeJson) -> String {
    let mut line = format!("{}- {} ", "  ".repeat(n.depth), n.id);
    match n.kind.as_str() {
        "Q" => {
            line.push_str(&n.text);
            if let Some(o) = &n.objectivity {
                line.push_str(&format!(" [Objectivity {}/{}", o.count, o.target));
                if n.leaf && o.met {
                    line.push_str(" met");
                }
                line.push(']');
            }
            if let Some(l) = &n.logicality {
                let date = l.checked_at.as_deref().map_or("", |t| t.get(..10).unwrap_or(t));
                let sep = if date.is_empty() { "" } else { " " };
                line.push_str(&format!(" [Logicality {}{sep}{date}]", l.value));
            }
        }
        "O" => {
            if n.starred {
                line.push_str("★ ");
            }
            if let Some(kind) = &n.answer_kind {
                let mut chars = kind.chars();
                let head = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
                line.push_str(&format!("{head}{} ", chars.as_str()));
            }
            line.push_str(&format!("{} [F {}]", n.text, n.counts.f));
        }
        "F" => {
            line.push_str(&n.text);
            line.push_str(&format!(" [{}]", n.cites.join(", ")));
        }
        _ => {
            line.push_str(&n.text);
            line.push_str(&format!(" [mlflow: {}]", n.mlflow.as_deref().unwrap_or("")));
        }
    }
    line
}

/// `tree.md` の本文を作る。
#[must_use]
pub fn render_tree_md(status: &Status) -> String {
    let name = status.type_info.as_ref().map_or("型は未設定", |t| t.name.as_str());
    let mut out = format!(
        "# 研究方針の木（{name}）\n\n進み具合 {}/{} / 書き出した時刻 {}\n\n",
        status.progress.answered,
        status.progress.total,
        Local::now().format("%Y-%m-%d %H:%M"),
    );
    for n in &status.nodes {
        out.push_str(&node_line(n));
        out.push('\n');
    }
    out.push_str("\n## SEED\n");
    if status.seeds.is_empty() {
        out.push_str("- なし\n");
    }
    for s in &status.seeds {
        out.push_str(&format!("- {} {}: {}（{}）\n", s.id, s.title, s.sentence, s.path.join(", ")));
    }
    out.push_str("\n## DECISIONS\n");
    if status.decisions.is_empty() {
        out.push_str("- なし\n");
    }
    for d in status.decisions.iter().take(30) {
        let at = d.at.get(..16).unwrap_or(&d.at);
        out.push_str(&format!("- {at} {} {}（{}）\n", d.node, d.action, d.actor));
    }
    out
}

/// `state.md` が既定のひな形と違う中身を持ち、`state.pre-sci.md` がまだ無ければ写す（`sci/export.rs` の写し）。
///
/// 写した先を返す。`state.md` が無い・ひな形のまま・既に `state.pre-sci.md` があるときは `Ok(None)`。
///
/// # Errors
///
/// ファイルの読み書きに失敗したとき。
pub fn backup_state_once(data_dir: &Path) -> Result<Option<PathBuf>> {
    let state = sai_paper_core::paths::state_path(data_dir);
    let Ok(text) = fs::read_to_string(&state) else {
        return Ok(None);
    };
    // ひな形のまま（手で何も書いていない）なら、ユーザーの中身は無いので写さない。
    if text.trim() == crate::project_files::STATE_TEMPLATE.trim() {
        return Ok(None);
    }
    let backup = data_dir.join("state.pre-sci.md");
    if backup.exists() {
        return Ok(None);
    }
    fs::write(&backup, &text)?;
    Ok(Some(backup))
}

/// 一時ファイルへ書いてから rename で置き換える（途中で壊れないように）。
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension(format!("md.tmp-{}", std::process::id()));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sai_paper_core::database::Database;
    use crate::rt::Kind;

    fn env(tag: &str, init: bool) -> (PaperRoots, Database) {
        let dir = std::env::temp_dir().join(format!("sai-rt-export-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let data_dir = dir.join(".sai/paper");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(dir.join(".sai/blocks")).unwrap();
        let db = Database::open(&data_dir.join("papr.db")).unwrap();
        if init {
            store::meta_set(db.connection(), "initialized_at", "2026-10-02 00:00:00").unwrap();
        }
        (PaperRoots { root: dir, data_dir }, db)
    }

    fn add(db: &Database, kind: Kind, parent: Option<i64>, text: &str, answer: Option<&str>, cites: &[String]) -> i64 {
        let n = store::NewNode {
            kind, parent_id: parent, guide_key: None, text, body: "", answer_kind: answer, cites,
            paper_id: None, run_id: None, report_id: None, mlflow_ref: None, proposal_id: None, created_by: "user",
        };
        store::insert_node(db.connection(), &n).unwrap()
    }

    #[test]
    fn tree_and_state() {
        let (roots, db) = env("tree", true);
        let c = db.connection();
        let q0 = add(&db, Kind::Q, None, "大目的は？", None, &[]);
        let o1 = add(&db, Kind::O, Some(q0), "予後予測", Some("answer"), &[]);
        store::set_starred(c, o1, true).unwrap();
        let q2 = add(&db, Kind::Q, Some(o1), "データは？", None, &[]);
        let o3 = add(&db, Kind::O, Some(q2), "術前 MRI", Some("answer"), &[]);
        let f4 = add(&db, Kind::F, Some(o3), "120 例ある", None, &["#12 §2".to_string()]);
        let gone = add(&db, Kind::F, Some(o3), "消す根拠", None, &[]);
        store::remove_subtree(c, gone).unwrap();
        let _ = f4;

        let status = compute::status(c, &roots, &Defs::default()).unwrap();
        let tree = render_tree_md(&status);
        let lines: Vec<&str> = tree.lines().collect();
        assert!(lines[0].starts_with("# 研究方針の木（"));
        assert!(lines[4].starts_with("- Q1 大目的は？ [Objectivity "), "{tree}");
        assert!(lines[4].ends_with("[Logicality unchecked]"), "{tree}");
        assert!(tree.contains("\n  - O2 ★ Answer 予後予測 [F 0]\n"), "{tree}");
        assert!(tree.contains("\n      - O4 Answer 術前 MRI [F 1]\n"), "{tree}");
        assert!(tree.contains("\n        - F5 120 例ある [#12 §2]\n"), "{tree}");
        assert!(!tree.contains("消す根拠"));

        let state = render_state_md(&status);
        let parsed = crate::project_files::parse_state(&state);
        assert_eq!(parsed.goal.as_deref(), Some("予後予測"));
        assert!(parsed.stage.is_some() && parsed.current_task.is_some());
        assert!(parsed.next_action.is_some() && parsed.updated.is_some());

        assert!(export(c, &roots, &Defs::default()).unwrap());
        assert!(roots.data_dir.join("tree.md").exists());
    }

    #[test]
    fn not_initialized_writes_nothing() {
        let (roots, db) = env("none", false);
        assert!(!export(db.connection(), &roots, &Defs::default()).unwrap());
        assert!(!roots.data_dir.join("tree.md").exists());
        assert!(!sai_paper_core::paths::state_path(&roots.data_dir).exists());
    }
}
