//! 新しい research-tree から `state.md` と `tree.md` を書き出す。
//!
//! 文は [`compute::status`] の結果だけから作る。research-tree の文は検査済み（op の検査）なので、そのまま写す。
//! 消した Node は `status` に入らないので、ここでも出ない。

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;
use rusqlite::Connection;

use super::compute::{self, NodeJson, Status};
use super::defs::Defs;
use super::{Result, store};
use sai_paper_core::paths::PaperRoots;

/// 新しい research-tree を始めたプロジェクトなら `state.md` と `tree.md` を書く。始めていなければ何もしない。戻り値は「書いたか」。
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

/// `state.md` の本文を作る（頭の 4 行は [`crate::project_files::parse_state`] が読む形）。
#[must_use]
pub fn render_state_md(status: &Status) -> String {
    let counts = status.counts.line();

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
        "- 段階: {counts}\n- 今の作業: {}\n- Next Action: {}\n- 更新日: {}\n\n",
        or_none(current),
        or_none(next),
        Local::now().format("%Y-%m-%d"),
    );
    out.push_str("<!-- このファイルは research-tree から書き出しています。直すときは Web か sai-sci を使います。 -->\n\n");

    // PROJECT（RESEARCH_TYPE と mind.md の見出しの一覧）。
    out.push_str("## PROJECT\n");
    let type_name = status.project.type_info.as_ref().map_or("なし", |t| t.name.as_str());
    out.push_str(&format!("- RESEARCH_TYPE: {type_name}\n"));
    for m in &status.project.mind {
        out.push_str(&format!("- {}\n", m.heading));
    }

    out.push_str("\n## seed（D）\n");
    out.push_str(&seed_section(status));

    out.push_str("\n## SPROUT\n");
    for n in status.nodes.iter().filter(|n| n.kind == "S") {
        out.push_str(&format!("- {}\n", sprout_text(n)));
    }

    out.push_str(&format!("\n## COUNTS\n- {counts}\n"));

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

/// `## seed（D）` の中身。1 行 1 つの D と、その下の SPROUT の ID。D が無ければ「なし」。
fn seed_section(status: &Status) -> String {
    let seeds: Vec<&NodeJson> = status.nodes.iter().filter(|n| n.kind == "D").collect();
    if seeds.is_empty() {
        return "- なし\n".to_string();
    }
    let mut out = String::new();
    for d in seeds {
        let sprouts: Vec<&str> = status
            .nodes
            .iter()
            .filter(|s| s.kind == "S" && s.parent.as_deref() == Some(d.id.as_str()))
            .map(|s| s.id.as_str())
            .collect();
        let list = if sprouts.is_empty() { "なし".to_string() } else { sprouts.join(", ") };
        out.push_str(&format!("- {} {}（SPROUT: {list}）\n", d.id, d.text));
    }
    out
}

/// SPROUT の 1 行（`## SPROUT` の中身。親の seed と PATH を付ける。親が無い古い SPROUT は `seed: なし`）。
fn sprout_text(n: &NodeJson) -> String {
    let seed = n.parent.as_deref().unwrap_or("なし");
    format!("{} {}（seed: {seed}・paths: {}）", n.id, n.text, n.paths.join(", "))
}

/// Node の 1 行（`# context` と同じ形）。字下げは深さ × 2 字。
fn node_line(n: &NodeJson) -> String {
    let mut line = format!("{}- {} ", "  ".repeat(n.depth), n.id);
    if n.starred {
        line.push_str("★ ");
    }
    match n.kind.as_str() {
        // seed（D）と軸の A は題だけ（mlflow を付けない）。
        "D" => {
            line.push_str(&n.text);
        }
        "A" if n.parent.is_none() => {
            line.push_str(&n.text);
        }
        "A" => {
            line.push_str(&format!("{} [F {}]", n.text, n.counts.f));
            if n.rel.as_deref() == Some("condition") {
                line.push_str(" [条件]");
            }
        }
        "F" => {
            if let Some(stance) = &n.stance {
                line.push_str(&format!("[{stance}] "));
            }
            line.push_str(&n.text);
            line.push_str(&format!(" [{}]", n.cites.join(", ")));
        }
        "S" => {
            line.push_str(&n.text);
            line.push_str(&format!("（paths: {}）", n.paths.join(", ")));
            if let Some(range) = &n.range {
                line.push_str(&format!(" [範囲: {}]", range.line()));
            }
        }
        _ => {
            line.push_str(&n.text);
            line.push_str(&format!(" [mlflow: {}]", n.mlflow.as_deref().unwrap_or("")));
            if let Some(dataset) = n.dataset.as_deref().filter(|d| !d.is_empty()) {
                line.push_str(&format!(" [dataset: {dataset}]"));
            }
        }
    }
    line
}

/// `tree.md` の本文を作る。
#[must_use]
pub fn render_tree_md(status: &Status) -> String {
    let name = status.type_info.as_ref().map_or("RESEARCH_TYPE は未設定", |t| t.name.as_str());
    let mut out = format!(
        "# research-tree（{name}）\n\n{} / 書き出した時刻 {}\n\n",
        status.counts.line(),
        Local::now().format("%Y-%m-%d %H:%M"),
    );
    for n in &status.nodes {
        out.push_str(&node_line(n));
        out.push('\n');
    }
    out.push_str("\n## seed（D）\n");
    out.push_str(&seed_section(status));
    out.push_str("\n## SPROUT\n");
    let sprouts: Vec<&NodeJson> = status.nodes.iter().filter(|n| n.kind == "S").collect();
    if sprouts.is_empty() {
        out.push_str("- なし\n");
    }
    for n in sprouts {
        out.push_str(&format!("- {}\n", sprout_text(n)));
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
    use crate::rt::{Kind, fmt_id};

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

    fn add(db: &Database, kind: Kind, parent: Option<i64>, text: &str, stance: Option<&str>, cites: &[String]) -> i64 {
        let n = store::NewNode {
            kind, parent_id: parent, guide_key: None, text, body: "", stance, cites,
            paper_id: None, run_id: None, report_id: None, mlflow_ref: None, dataset_ref: None,
            paths: &[], created_by: "user",
        };
        store::insert_node(db.connection(), &n).unwrap()
    }

    #[test]
    fn tree_and_state() {
        let (roots, db) = env("tree", true);
        let c = db.connection();
        let q0 = add(&db, Kind::A, None, "大目的は？", None, &[]);
        let a1 = add(&db, Kind::A, Some(q0), "予後予測", None, &[]);
        store::set_starred(c, a1, true).unwrap();
        let q2 = add(&db, Kind::A, Some(a1), "データは？", None, &[]);
        let a3 = add(&db, Kind::A, Some(q2), "術前 MRI", None, &[]);
        add(&db, Kind::F, Some(a3), "120 例ある", Some("support"), &["#12 §2".to_string()]);
        let gone = add(&db, Kind::F, Some(a3), "消す根拠", Some("deny"), &[]);
        store::remove_subtree(c, gone).unwrap();

        let status = compute::status(c, &roots, &Defs::default()).unwrap();
        let tree = render_tree_md(&status);
        let lines: Vec<&str> = tree.lines().collect();
        assert!(lines[0].starts_with("# research-tree（"));
        assert!(lines[2].contains("軸 1・A 3・F 1（S 1 / D 0 / I 0）・E 0・S 0"), "{tree}");
        assert!(tree.contains("\n- A1 大目的は？\n"), "{tree}");
        assert!(tree.contains("\n  - A2 ★ 予後予測 [F 0]\n"), "{tree}");
        assert!(tree.contains("\n      - A4 術前 MRI [F 1]\n"), "{tree}");
        assert!(tree.contains("\n        - F5 [support] 120 例ある [#12 §2]\n"), "{tree}");
        assert!(!tree.contains("消す根拠"));

        let state = render_state_md(&status);
        assert!(state.contains("## COUNTS") && !state.contains("PENDING"), "{state}");
        let parsed = crate::project_files::parse_state(&state);
        // 目標の行は無くなり、代わりに PROJECT の節が出る。
        assert_eq!(parsed.goal, None);
        assert!(state.contains("## PROJECT") && state.contains("- RESEARCH_TYPE: なし"), "{state}");
        assert!(parsed.stage.is_some() && parsed.current_task.is_some());
        assert!(parsed.next_action.is_some() && parsed.updated.is_some());

        assert!(export(c, &roots, &Defs::default()).unwrap());
        assert!(roots.data_dir.join("tree.md").exists());
    }

    #[test]
    fn tree_md_shows_a_under_a_and_condition() {
        let (roots, db) = env("tree-a-under-a", true);
        let c = db.connection();
        let q0 = add(&db, Kind::A, None, "大目的は？", None, &[]);
        let a1 = add(&db, Kind::A, Some(q0), "大分類", None, &[]);
        let a2 = add(&db, Kind::A, Some(a1), "細分", None, &[]);
        store::set_rel(c, a2, Some("condition")).unwrap();
        let status = compute::status(c, &roots, &Defs::default()).unwrap();
        let tree = render_tree_md(&status);
        assert!(tree.contains(&format!("\n  - A{a1} 大分類 [F 0]\n")), "{tree}");
        assert!(tree.contains(&format!("\n    - A{a2} 細分 [F 0] [条件]\n")), "{tree}");
    }

    #[test]
    fn seed_and_sprout_lines() {
        let (roots, db) = env("seed", true);
        let c = db.connection();
        let d1 = add(&db, Kind::D, None, "自分の研究", None, &[]);
        let s2 = add(&db, Kind::S, Some(d1), "一つ目", None, &[]);
        let d3 = add(&db, Kind::D, None, "空の研究", None, &[]);
        let old = add(&db, Kind::S, None, "古い研究", None, &[]);
        let status = compute::status(c, &roots, &Defs::default()).unwrap();

        let tree = render_tree_md(&status);
        assert!(tree.contains(&format!("\n- {} 自分の研究\n", fmt_id(Kind::D, d1))), "{tree}");
        assert!(tree.contains(&format!("\n  - {} 一つ目（paths: ）", fmt_id(Kind::S, s2))), "{tree}");
        assert!(tree.contains(&format!("\n- {} 古い研究（paths: ）", fmt_id(Kind::S, old))), "{tree}");
        assert!(!tree.contains("[mlflow"), "{tree}");
        assert!(tree.contains(&format!("\n## seed（D）\n- {} 自分の研究（SPROUT: {}）\n- {} 空の研究（SPROUT: なし）\n", fmt_id(Kind::D, d1), fmt_id(Kind::S, s2), fmt_id(Kind::D, d3))), "{tree}");
        assert!(tree.contains(&format!("## SPROUT\n- {} 一つ目（seed: {}・paths: ）\n- {} 古い研究（seed: なし・paths: ）\n", fmt_id(Kind::S, s2), fmt_id(Kind::D, d1), fmt_id(Kind::S, old))), "{tree}");

        let state = render_state_md(&status);
        assert!(state.contains(&format!("## seed（D）\n- {} 自分の研究（SPROUT: {}）\n", fmt_id(Kind::D, d1), fmt_id(Kind::S, s2))), "{state}");
        assert!(state.contains(&format!("- {} 一つ目（seed: {}・paths: ）\n", fmt_id(Kind::S, s2), fmt_id(Kind::D, d1))), "{state}");
        assert!(state.contains(&format!("- {} 古い研究（seed: なし・paths: ）\n", fmt_id(Kind::S, old))), "{state}");
        // 親の D の行は COUNTS に入らない（S は数える）。
        assert!(state.contains("## COUNTS\n- 軸 0・A 0・F 0（S 0 / D 0 / I 0）・E 0・S 2\n"), "{state}");
        let _ = fs::remove_dir_all(&roots.root);
    }

    #[test]
    fn not_initialized_writes_nothing() {
        let (roots, db) = env("none", false);
        assert!(!export(db.connection(), &roots, &Defs::default()).unwrap());
        assert!(!roots.data_dir.join("tree.md").exists());
        assert!(!sai_paper_core::paths::state_path(&roots.data_dir).exists());
    }
}
