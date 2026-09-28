//! 研究方針の木を `state.md`（現在地）と `tree.md`（木の全体）へ書き出す。
//!
//! 正は `papr.db` の `sci_*` の表で、この 2 つのファイルはそこから作った書き出しである。
//! 手で直しても次の書き出しで消える。`state.md` の 5 行（`- 目標:` など）は
//! `crate::project_files::parse_state` が読む形のまま残す（TUI の Project の画面と
//! 進み具合の集計が今の形で読み続けられるようにするため）。
//!
//! 書くときは同じフォルダの一時ファイルへ書いてから名前を変える。途中で読まれても
//! 壊れた中身を見せない。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;
use rusqlite::Connection;

use super::compute::{self, Status};
use super::defs::Defs;
use super::store::{self, Decision, Finding, Link, Node, Question, Seed, Tombstone};
use super::{Result, fmt_ref, parse_ref};
use crate::paths::PaperRoots;

/// 次の 3 手に出す件数。
const NEXT_LIMIT: usize = 3;

/// 1 つの項目について、木の読みやすい書き出しに並べるために一緒に読むもの。
/// 節点ごとに SQL を投げないよう、木ごとに 1 回だけ集める。
#[derive(Default)]
struct NodeExtra {
    /// 本文。
    body: String,
    /// 問い（古い順）。
    questions: Vec<Question>,
    /// 知見（項目なら子の候補の分も入る。外していないもの）。
    findings: Vec<Finding>,
    /// 論文の結び付き（項目なら子の候補の分も入る。外していないもの）。
    links: Vec<Link>,
    /// 決定の記録（古い順）。
    decisions: Vec<Decision>,
}

// ---- 表示名 -----------------------------------------------------------------

/// 項目の型の表示名（C-01）。
const TYPE_LABELS: [(&str, &str); 4] = [
    ("choose", "選ぶ"),
    ("list", "並べる"),
    ("write", "書く"),
    ("verify", "確かめる"),
];

/// 項目の状態の表示名（C-01）。
const ITEM_STATE_LABELS: [(&str, &str); 4] = [
    ("open", "未決定"),
    ("tentative", "仮決定"),
    ("decided", "決定"),
    ("excluded", "対象外"),
];

/// 候補の状態の表示名（C-01）。
const OPTION_STATE_LABELS: [(&str, &str); 4] = [
    ("candidate", "候補"),
    ("selected", "選択"),
    ("rejected", "却下"),
    ("dormant", "休眠"),
];

/// 空欄の種類の表示名（C-01）。
const BLANK_LABELS: [(&str, &str); 5] = [
    ("untouched", "未着手"),
    ("in_progress", "調査中"),
    ("insufficient", "情報不足"),
    ("awaiting_decision", "判断待ち"),
    ("conflict", "対立"),
];

/// 印の表示名（C-01）。
const MARK_LABELS: [(&str, &str); 2] = [
    ("weak_evidence", "根拠不足"),
    ("recheck", "要再確認"),
];

/// 知見の向きの表示名（C-01）。
const POLARITY_LABELS: [(&str, &str); 4] = [
    ("support", "支持"),
    ("refute", "反証"),
    ("neutral", "中立"),
    ("none", "該当なし"),
];

/// 論文の役割の表示名（C-01）。
const ROLE_LABELS: [(&str, &str); 7] = [
    ("exemplar", "代表例"),
    ("compare", "比較対象"),
    ("issue_evidence", "課題の根拠"),
    ("method_source", "手法の供給元"),
    ("data_source", "データの出典"),
    ("background", "背景"),
    ("counter", "反証"),
];

/// 読み具合の表示名（C-01）。
const READ_LABELS: [(&str, &str); 4] = [
    ("unread", "未読"),
    ("read", "読んだ・未反映"),
    ("reflected", "反映済み"),
    ("irrelevant", "関係なし"),
];

/// 問いの状態の表示名。
const QUESTION_LABELS: [(&str, &str); 3] = [
    ("open", "答え待ち"),
    ("answered", "回答済み"),
    ("withdrawn", "取り下げ"),
];

/// 決定の記録の種類の表示名（C-01）。
const ACTION_LABELS: [(&str, &str); 7] = [
    ("decide", "決定"),
    ("tentative", "仮決定"),
    ("reopen", "再開"),
    ("exclude_item", "対象外にする"),
    ("reject_option", "候補の却下"),
    ("dormant_option", "候補の休眠"),
    ("restore_option", "候補に戻す"),
];

/// 負債の行の表（`Debts` の項目と表示名）。
const DEBT_LABELS: [(&str, &str); 4] = [
    ("pending_proposals", "仕分けていない提案"),
    ("unreflected_papers", "読んだが反映していない論文"),
    ("unscreened_sets", "選別していない検索"),
    ("empty_reviews", "書いていない振り返り"),
];

/// 表に無い値はその値そのものを返す。
fn label(table: &[(&str, &str)], value: &str) -> String {
    table
        .iter()
        .find(|(key, _)| *key == value)
        .map_or_else(|| value.to_string(), |(_, name)| (*name).to_string())
}

/// 節点の状態の表示名（項目と候補で表が違う）。
fn state_label(node: &compute::NodeRow) -> String {
    if node.kind == "option" {
        label(&OPTION_STATE_LABELS, &node.state)
    } else {
        label(&ITEM_STATE_LABELS, &node.state)
    }
}

// ---- 書き出し ---------------------------------------------------------------

/// sci を始めたプロジェクトなら `state.md` と `tree.md` を書く。始めていなければ何もしない。
///
/// 書くかどうかは `sci_meta` の `type` で決める（[`store::is_initialized`]）。
/// 戻り値は「書いたか」。
///
/// # Errors
///
/// DB の読み書きとファイルの書き込みに失敗したとき。
pub fn export(conn: &Connection, roots: &PaperRoots, defs: &Defs) -> Result<bool> {
    if !store::is_initialized(conn)? {
        return Ok(false);
    }
    let status = compute::status(conn, roots, defs)?;
    let state = render_state_md(&status);
    let tree = render_tree_md(conn, roots, defs, &status)?;
    let data_dir = &roots.data_dir;
    fs::create_dir_all(data_dir)?;
    write_atomic(&crate::paths::state_path(data_dir), &state)?;
    write_atomic(&data_dir.join("tree.md"), &tree)?;
    Ok(true)
}

/// `state.md` の本文を作る（頭の 5 行は [`crate::project_files::parse_state`] が読む形）。
///
/// 種の文・関門・空欄・印・負債・次の 3 手を順に並べる。空のものは `- なし` にする。
#[must_use]
pub fn render_state_md(status: &Status) -> String {
    let seed = status
        .seed
        .as_ref()
        .map(|seed| seed.text.clone())
        .unwrap_or_default();

    let gate = status
        .gate
        .as_ref()
        .map_or_else(|| "なし".to_string(), |gate| {
            format!("関門 {}/{} {}", gate.current, gate.total, gate.name)
        });

    let current = if status.running.is_empty() {
        "なし".to_string()
    } else {
        status
            .running
            .iter()
            .map(|run| match run.node.as_deref() {
                Some(node) => format!("run {} {}（{node}）", run.run_id, run.block),
                None => format!("run {} {}", run.run_id, run.block),
            })
            .collect::<Vec<_>>()
            .join("、")
    };

    let next = status
        .next
        .first()
        .map_or("なし", |action| action.purpose.as_str());

    let mut out = String::new();
    out.push_str("# 現況\n");
    out.push_str(
        "<!-- 研究方針の木から sai-moon-paper が書き出したファイルです。手で直さないでください\
（次の書き出しで上書きされます）。直すときは Research-tree の画面か sai-moon-paper sci を使います -->\n\n",
    );
    out.push_str(&format!("- 目標: {seed}\n"));
    out.push_str(&format!("- 段階: {gate}\n"));
    out.push_str(&format!("- 今の作業: {current}\n"));
    out.push_str(&format!("- 次の一手: {next}\n"));
    out.push_str(&format!("- 更新日: {}\n", Local::now().date_naive()));

    out.push_str("\n## 種の文\n");
    if seed.is_empty() {
        out.push_str("（まだありません）\n");
    } else {
        out.push_str(&seed);
        out.push('\n');
    }

    out.push_str("\n## 関門\n");
    match status.gate.as_ref() {
        Some(gate) if !gate.list.is_empty() => {
            for (index, row) in gate.list.iter().enumerate() {
                let mark = if row.passed { "x" } else { " " };
                let here = if i64::try_from(index + 1).ok() == Some(gate.current) {
                    "  ← 今の関門"
                } else {
                    ""
                };
                out.push_str(&format!("- [{mark}] {}{here}\n", row.name));
            }
        }
        _ => out.push_str("- なし\n"),
    }

    out.push_str("\n## 空欄\n");
    let blanks: Vec<String> = status
        .nodes
        .iter()
        .filter_map(|node| {
            let blank = node.blank.as_deref()?;
            Some(format!(
                "- {} {}: {}",
                node.id,
                node.title,
                label(&BLANK_LABELS, blank)
            ))
        })
        .collect();
    if blanks.is_empty() {
        out.push_str("- なし\n");
    } else {
        for line in blanks {
            out.push_str(&line);
            out.push('\n');
        }
    }

    out.push_str("\n## 印\n");
    let marks: Vec<String> = status
        .nodes
        .iter()
        .filter(|node| !node.marks.is_empty())
        .map(|node| {
            let names = node
                .marks
                .iter()
                .map(|mark| label(&MARK_LABELS, mark))
                .collect::<Vec<_>>()
                .join("、");
            format!("- {} {}: {names}", node.id, node.title)
        })
        .collect();
    if marks.is_empty() {
        out.push_str("- なし\n");
    } else {
        for line in marks {
            out.push_str(&line);
            out.push('\n');
        }
    }

    out.push_str("\n## 負債\n");
    for (name, count) in [
        ("pending_proposals", status.debts.pending_proposals),
        ("unreflected_papers", status.debts.unreflected_papers),
        ("unscreened_sets", status.debts.unscreened_sets),
        ("empty_reviews", status.debts.empty_reviews),
    ] {
        let name = DEBT_LABELS
            .iter()
            .find(|(key, _)| *key == name)
            .map_or(name, |(_, label)| *label);
        out.push_str(&format!("- {name}: {count}\n"));
    }

    out.push_str("\n## 次の 3 手\n");
    if status.next.is_empty() {
        out.push_str("- なし\n");
    } else {
        for (index, action) in status.next.iter().take(NEXT_LIMIT).enumerate() {
            let block = action.block.as_deref().unwrap_or("人の作業");
            out.push_str(&format!(
                "{}. {}（{block}・{}）\n",
                index + 1,
                action.purpose,
                action.why
            ));
        }
    }
    out
}

/// `tree.md` の本文を作る（木の全体）。
///
/// 一番上の項目から深さ順に見出しを出し、その下に状態・空欄・本文・問い・候補・
/// 知見・論文・直近の決定の記録を並べる。最後に種と墓標の一覧を置く。
/// 見出しの深さは 2 段まで（`##` `###` `####`）。それより深い所は `####` のまま。
///
/// # Errors
///
/// DB の読み出しに失敗したとき。
pub fn render_tree_md(
    conn: &Connection,
    _roots: &PaperRoots,
    _defs: &Defs,
    status: &Status,
) -> Result<String> {
    let extra = extras(conn, status)?;
    let titles = paper_titles(conn, &extra)?;
    let (options, orphans) = options_of(status);

    let mut out = String::new();
    out.push_str("# 研究方針の木\n");
    out.push_str(
        "<!-- 研究方針の木から sai-moon-paper が書き出したファイルです。手で直さないでください -->\n",
    );
    out.push_str(&format!(
        "- 研究の型: {}\n",
        status
            .type_info
            .as_ref()
            .map_or("なし", |info| info.name.as_str())
    ));
    out.push_str(&format!("- 更新日: {}\n", Local::now().date_naive()));

    out.push_str("\n## 種の文\n");
    match status.seed.as_ref() {
        Some(seed) if !seed.text.is_empty() => {
            out.push_str(&seed.text);
            out.push('\n');
        }
        _ => out.push_str("（まだありません）\n"),
    }

    for node in &status.nodes {
        // 候補は親の項目の下の一行として出す（見出しにはしない）。親が無いものだけ後ろへ置く。
        if node.kind == "option" {
            continue;
        }
        let depth = usize::try_from(node.depth).unwrap_or(0);
        let heading = "#".repeat(2 + depth.min(2));
        out.push_str(&format!(
            "\n{heading} {} {}\n",
            node.id,
            node.title.trim()
        ));
        out.push_str(&format!("- 状態: {}\n", state_label(node)));
        if !node.types.is_empty() {
            let types = node
                .types
                .iter()
                .map(|kind| label(&TYPE_LABELS, kind))
                .collect::<Vec<_>>()
                .join("・");
            out.push_str(&format!("- 種類: {types}\n"));
        }
        if let Some(blank) = node.blank.as_deref() {
            out.push_str(&format!("- 空欄: {}\n", label(&BLANK_LABELS, blank)));
        }
        if !node.marks.is_empty() {
            let marks = node
                .marks
                .iter()
                .map(|mark| label(&MARK_LABELS, mark))
                .collect::<Vec<_>>()
                .join("、");
            out.push_str(&format!("- 印: {marks}\n"));
        }

        let Some(node_id) = node_id_of(&node.id) else {
            continue;
        };
        let Some(detail) = extra.get(&node_id) else {
            continue;
        };
        if !detail.body.trim().is_empty() {
            out.push_str("\n");
            out.push_str(detail.body.trim_end());
            out.push('\n');
        }
        for question in &detail.questions {
            let state = label(&QUESTION_LABELS, &question.state);
            let answer = if question.answer.trim().is_empty() {
                String::new()
            } else {
                format!("（答え: {}）", question.answer.trim())
            };
            out.push_str(&format!(
                "- 問い {} {}: {state}{answer}\n",
                fmt_ref('Q', question.id),
                question.text.trim()
            ));
        }
        for finding in &detail.findings {
            let cites = if finding.cites.is_empty() {
                String::new()
            } else {
                format!("（{}）", finding.cites.join("、"))
            };
            out.push_str(&format!(
                "- 知見 {} {}: {}{cites}\n",
                fmt_ref('F', finding.id),
                label(&POLARITY_LABELS, &finding.polarity),
                finding.text.trim()
            ));
        }
        for link in &detail.links {
            let Ok(paper_id) = link.target_ref.trim().parse::<i64>() else {
                continue;
            };
            let title = titles
                .get(&paper_id)
                .map_or_else(String::new, |title| format!("「{}」", title.trim()));
            let role = link
                .role
                .as_deref()
                .map_or_else(String::new, |role| label(&ROLE_LABELS, role));
            let read = link
                .read_state
                .as_deref()
                .map_or_else(String::new, |read| label(&READ_LABELS, read));
            out.push_str(&format!(
                "- 論文 {} {role}・{read}{title}\n",
                fmt_ref('L', link.id)
            ));
        }
        if let Some(decision) = detail.decisions.last() {
            let option = decision
                .option_id
                .map(|id| fmt_ref('N', id))
                .unwrap_or_default();
            let reason = decision.reason.trim();
            let reason = if reason.is_empty() {
                String::new()
            } else {
                format!("（{reason}）")
            };
            out.push_str(&format!(
                "- 直近の決定: {} {}{reason} {}\n",
                label(&ACTION_LABELS, &decision.action),
                option,
                decision.created_at
            ));
        }
        for option in options.get(&node_id).into_iter().flatten() {
            out.push_str(&format!(
                "- 候補 {} {}: {}（支持 {}・反証 {}・論文 {}）\n",
                option.id,
                option.title,
                state_label(option),
                option.counts.support,
                option.counts.refute,
                option.counts.papers
            ));
        }
    }

    // 親を辿れなかった候補は末尾に置く（落とさない）。
    for option in &orphans {
        out.push_str(&format!(
            "- 候補 {} {}: {}\n",
            option.id,
            option.title,
            state_label(option)
        ));
    }

    out.push_str("\n## 種\n");
    let seeds = store::list_seeds(conn)?;
    push_seeds(&mut out, &seeds);

    out.push_str("\n## 墓標\n");
    let tombstones = store::list_tombstones(conn)?;
    push_tombstones(&mut out, &tombstones);

    Ok(out)
}

/// 種の一覧を書き出す。
fn push_seeds(out: &mut String, seeds: &[Seed]) {
    if seeds.is_empty() {
        out.push_str("- なし\n");
        return;
    }
    for seed in seeds {
        let doc = if seed.doc_ref.trim().is_empty() {
            String::new()
        } else {
            format!("　{}", seed.doc_ref.trim())
        };
        out.push_str(&format!(
            "- {} {}（{created}）{doc}\n",
            fmt_ref('S', seed.id),
            seed.title.trim(),
            created = seed.created_at
        ));
    }
}

/// 墓標の一覧を書き出す。
fn push_tombstones(out: &mut String, tombstones: &[Tombstone]) {
    if tombstones.is_empty() {
        out.push_str("- なし\n");
        return;
    }
    for tombstone in tombstones {
        let what = match tombstone.kind.as_str() {
            "paper" => "論文",
            _ => "候補",
        };
        let title = tombstone.title.trim();
        let title = if title.is_empty() {
            String::new()
        } else {
            format!("「{title}」")
        };
        out.push_str(&format!(
            "- {what}{title}（{}）\n",
            tombstone.reason.trim()
        ));
    }
}

/// `state.md` が既定のひな形と違う中身を持ち、`state.pre-sci.md` がまだ無ければ写す。
///
/// 写した先を返す。`state.md` が無い・ひな形のまま・既に `state.pre-sci.md` が
/// あるときは `Ok(None)`（何もしない）。
///
/// # Errors
///
/// ファイルの読み書きに失敗したとき。
pub fn backup_state_once(data_dir: &Path) -> Result<Option<PathBuf>> {
    let state = crate::paths::state_path(data_dir);
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

// ---- 節点ごとに読むもの -----------------------------------------------------

/// 候補の行を親（項目）ごとにまとめる。親が無い候補は 2 つめの戻り値にする。
///
/// `status.nodes` の並びは幅優先なので、候補が親の直後に並ばない。そのため
/// 候補はここで親に寄せてから、項目の中の一行として出す。
fn options_of(status: &Status) -> (HashMap<i64, Vec<&compute::NodeRow>>, Vec<&compute::NodeRow>) {
    let items: HashSet<i64> = status
        .nodes
        .iter()
        .filter(|node| node.kind == "item")
        .filter_map(|node| node_id_of(&node.id))
        .collect();
    let mut grouped: HashMap<i64, Vec<&compute::NodeRow>> = HashMap::new();
    let mut orphans = Vec::new();
    for node in status.nodes.iter().filter(|node| node.kind == "option") {
        let parent = node
            .parents
            .iter()
            .filter_map(|parent| node_id_of(parent))
            .find(|parent| items.contains(parent));
        match parent {
            Some(parent) => grouped.entry(parent).or_default().push(node),
            None => orphans.push(node),
        }
    }
    (grouped, orphans)
}

/// `N12` の形から数（`N12` 以外なら `None`）。
fn node_id_of(id: &str) -> Option<i64> {
    parse_ref(id, 'N').ok()
}

/// 項目ごとに、本文・問い・知見・論文の結び付き・決定の記録を集める。
///
/// 知見と結び付きは、項目なら子の候補の分も足す（C-08 の `node` と同じ決まり）。
fn extras(conn: &Connection, status: &Status) -> Result<HashMap<i64, NodeExtra>> {
    // 項目 → 子の候補。
    let mut options: HashMap<i64, Vec<i64>> = HashMap::new();
    for node in &status.nodes {
        if node.kind != "option" {
            continue;
        }
        for parent in &node.parents {
            let Some(parent) = node_id_of(parent) else {
                continue;
            };
            if let Some(option) = node_id_of(&node.id) {
                options.entry(parent).or_default().push(option);
            }
        }
    }

    let mut out: HashMap<i64, NodeExtra> = HashMap::new();
    for node in status.nodes.iter().filter(|node| node.kind == "item") {
        let Some(id) = node_id_of(&node.id) else {
            continue;
        };
        let mut extra = NodeExtra {
            body: store::get_node(conn, id).map(|found: Node| found.body).unwrap_or_default(),
            questions: store::list_questions(conn, Some(id))?,
            findings: store::list_findings(conn, Some(id), false)?,
            links: store::list_links(conn, Some(id), false)?,
            decisions: store::list_decisions(conn, Some(id))?,
        };
        for option in options.get(&id).into_iter().flatten() {
            extra.findings.extend(store::list_findings(conn, Some(*option), false)?);
            extra
                .links
                .extend(store::list_links(conn, Some(*option), false)?);
        }
        // 論文の結び付きだけを出す（report や run の結び付きは木の読みやすさのため出さない）。
        extra.links.retain(|link| link.target_kind == "paper");
        out.insert(id, extra);
    }
    Ok(out)
}

/// 結び付いている論文の番号 → 題。
fn paper_titles(conn: &Connection, extra: &HashMap<i64, NodeExtra>) -> Result<HashMap<i64, String>> {
    let mut wanted: Vec<i64> = Vec::new();
    for detail in extra.values() {
        for link in &detail.links {
            if let Ok(id) = link.target_ref.trim().parse::<i64>() {
                wanted.push(id);
            }
        }
    }
    wanted.sort_unstable();
    wanted.dedup();
    let mut out = HashMap::new();
    if wanted.is_empty() {
        return Ok(out);
    }
    let mut statement = conn.prepare("SELECT id, title FROM papers")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?;
    for row in rows {
        let (id, title) = row?;
        if wanted.contains(&id) {
            out.insert(id, title);
        }
    }
    Ok(out)
}

/// 一時ファイルへ書いてから rename で置き換える（途中で壊れないように）。
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    // `state.md` → `state.md.tmp-<pid>`。
    let tmp = path.with_extension(format!("md.tmp-{}", std::process::id()));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::sci::defs;
    use crate::sci::store::{NewFinding, NewLink, NewNode, NewTombstone};

    /// テスト用の一時プロジェクトルート（`.sai/paper` と `.sai/outputs` 付き）。
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-export-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".sai/paper")).expect("一時フォルダ");
        fs::create_dir_all(dir.join(".sai/outputs")).expect("実行フォルダ");
        dir
    }

    /// テスト用の `PaperRoots`。
    fn roots_of(root: &Path) -> PaperRoots {
        PaperRoots {
            root: root.to_path_buf(),
            data_dir: root.join(".sai/paper"),
        }
    }

    /// 木を始める（型の埋め込み定義で「課題駆動」の骨組みを作る）。
    fn build_tree(conn: &Connection, root: &Path) -> Defs {
        let defs = defs::load(root);
        defs::init_tree(conn, &defs, "problem").expect("木を始める");
        defs
    }

    /// 候補を作る（親に線の处以て結ぶ）。
    fn option(conn: &Connection, parent: i64, title: &str) -> i64 {
        let id = store::insert_node(
            conn,
            &NewNode {
                kind: "option",
                item_key: None,
                types: &[],
                title,
                body: "",
                state: "candidate",
                fixed: false,
                ord: 0,
                created_by: "user",
            },
        )
        .expect("候補を作る");
        store::insert_edge(conn, "parent", parent, id).expect("親の線");
        id
    }

    // ---- state.md ---------------------------------------------------------

    #[test]
    fn state_md_keeps_parse_state_lines() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("state");
        let roots = roots_of(&root);
        let defs = build_tree(conn, &root);

        let text = render_state_md(&compute::status(conn, &roots, &defs).expect("status"));
        let state = crate::project_files::parse_state(&text);
        // 5 項目が全部取れること。
        let goal = state.goal.expect("目標");
        assert!(goal.starts_with('「'), "{goal}");
        let stage = state.stage.expect("段階");
        assert!(stage.starts_with("関門 1/"), "{stage}");
        assert_eq!(state.current_task.as_deref(), Some("なし"));
        assert!(state.next_action.is_some_and(|next| !next.is_empty()));
        let updated = state.updated.expect("更新日");
        assert_eq!(updated.len(), 10, "{updated}");

        // 次の 3 手は 3 件まで。
        let third = text
            .lines()
            .find(|line| line.starts_with("3. "))
            .expect("次の 3 手");
        assert!(!text.lines().any(|line| line.starts_with("4. ")), "{text}");
        assert!(third.contains('（'), "{third}");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn export_writes_nothing_without_tree() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("no-tree");
        let roots = roots_of(&root);
        let defs = defs::load(&root);
        let state = roots.data_dir.join("state.md");
        fs::write(&state, "# 現況\n\n- 目標: 手書きの現況\n").expect("state.md を書く");

        assert!(!export(conn, &roots, &defs).expect("export"));
        assert_eq!(
            fs::read_to_string(&state).expect("読む"),
            "# 現況\n\n- 目標: 手書きの現況\n"
        );
        assert!(!roots.data_dir.join("tree.md").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn export_writes_state_and_tree() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("both");
        let roots = roots_of(&root);
        let defs = build_tree(conn, &root);

        assert!(export(conn, &roots, &defs).expect("export"));
        let state = fs::read_to_string(crate::paths::state_path(&roots.data_dir)).expect("state.md");
        let tree = fs::read_to_string(roots.data_dir.join("tree.md")).expect("tree.md");
        assert!(state.starts_with("# 現況"), "{state}");
        assert!(tree.starts_with("# 研究方針の木"), "{tree}");
        // 一時ファイルを残さない。
        let leftovers: Vec<_> = fs::read_dir(&roots.data_dir)
            .expect("読む")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");

        let _ = fs::remove_dir_all(&root);
    }

    // ---- tree.md ----------------------------------------------------------

    #[test]
    fn tree_md_lists_items_and_tombstones() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("tree");
        let roots = roots_of(&root);
        let defs = build_tree(conn, &root);

        // 目的地へ候補を 1 つ、知見を 1 つ、論文の結び付きを 1 つ足す。
        let target = store::find_item_by_key(conn, "target")
            .expect("探す")
            .expect("ある")
            .id;
        let choice = option(conn, target, "てんかん患者");
        store::insert_finding(
            conn,
            &NewFinding {
                node_id: choice,
                question_id: None,
                text: "公開データ最多的",
                cites: &["#37 §3.2".to_string()],
                polarity: "support",
                level: Some(2),
                source: "user",
                report_id: None,
                paper_id: None,
                run_id: None,
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .expect("知見");
        conn.execute(
            "INSERT INTO papers (id, title) VALUES (37, 'Scalp EEG for localization')",
            [],
        )
        .expect("論文");
        store::upsert_link(
            conn,
            &NewLink {
                node_id: target,
                target_kind: "paper",
                target_ref: "37",
                role: Some("exemplar"),
                read_state: Some("read"),
                reason: "代表例",
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .expect("結び付き");
        store::insert_seed(conn, "最初の種", "「頭皮脳波で小目的を行う」", &[], "waggle:AAAA", None)
            .expect("種");
        store::insert_tombstone(
            conn,
            &NewTombstone {
                kind: "paper",
                paper_id: Some(41),
                doi: Some("10.1/x"),
                arxiv_id: None,
                dedupe_key: None,
                title: " scalp only",
                reason: "系列的が外れた",
                node_id: None,
                run_id: None,
                created_by: "user",
            },
        )
        .expect("墓標");

        let status = compute::status(conn, &roots, &defs).expect("status");
        let tree = render_tree_md(conn, &roots, &defs, &status).expect("tree.md");

        // 見出しと並び。
        assert!(tree.contains("- 研究の型: 課題駆動"), "{tree}");
        assert!(tree.contains("## 種の文"), "{tree}");
        assert!(tree.contains(&format!("## {} 対象", fmt_ref('N', target))), "{tree}");
        // 候補・知見・論文は項目の中の一行。
        assert!(tree.contains(&format!("- 候補 {} ", fmt_ref('N', choice))), "{tree}");
        assert!(tree.contains("知見 F"), "{tree}");
        assert!(tree.contains("支持: 公開データ最多的（#37 §3.2）"), "{tree}");
        assert!(
            tree.contains("代表例・読んだ・未反映「Scalp EEG for localization」"),
            "{tree}"
        );
        // 種と墓標。
        assert!(tree.contains("## 種"), "{tree}");
        assert!(tree.contains("S1 最初の種"), "{tree}");
        assert!(tree.contains("waggle:AAAA"), "{tree}");
        assert!(tree.contains("## 墓標"), "{tree}");
        assert!(tree.contains("論文「scalp only」（系列的が外れた）"), "{tree}");

        let _ = fs::remove_dir_all(&root);
    }

    // ---- ひな形を壊さないために残す ---------------------------------------

    #[test]
    fn backup_state_once_copies_only_once_and_skips_template() {
        let root = temp_root("backup");
        let data_dir = root.join(".sai/paper");
        let state = crate::paths::state_path(&data_dir);

        // ひな形のままなら写さない。
        fs::write(&state, crate::project_files::STATE_TEMPLATE).expect("state.md を書く");
        assert_eq!(backup_state_once(&data_dir).expect("backup"), None);
        assert!(!data_dir.join("state.pre-sci.md").exists());

        // 手で書いた中身は 1 回だけ写す。
        let mine = "# 現況\n\n- 目標: AGENDA の現況\n";
        fs::write(&state, mine).expect("state.md を書く");
        let saved = backup_state_once(&data_dir).expect("backup");
        assert_eq!(saved, Some(data_dir.join("state.pre-sci.md")));
        assert_eq!(
            fs::read_to_string(saved.expect("保存先")).expect("読む"),
            mine
        );

        // 2 回目は写さない（上書きしない）。
        fs::write(&state, "# 現況\n\n- 目標: 別の現況\n").expect("state.md を書く");
        assert_eq!(backup_state_once(&data_dir).expect("backup"), None);
        assert_eq!(
            fs::read_to_string(data_dir.join("state.pre-sci.md")).expect("読む"),
            mine
        );

        let _ = fs::remove_dir_all(&root);
    }
}
