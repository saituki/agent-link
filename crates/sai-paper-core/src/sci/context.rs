//! エージェントに渡す入力の範囲（context）を組み立てる。
//!
//! 木から切り出した範囲だけを渡す（要件 R-D3・R-I1〜R-I3）。全体の状況（project-brief）を
//! そのまま渡すのではなく、木の 1 節点と周りの必要な範囲を C-06 の Markdown にして返す。
//!
//! sai-link が起動の直前に `sci context --run <id>` を呼び、標準出力をメモリ上に詰める。
//! したがって **ファイルは作らない**（標準出力に流すのは呼び出し側の仕事）。
//!
//! 長さは `settings.json` の `sci.context_max_chars`（既定 12000 字）で制限する。
//! はみ出したときは C-06 の ①〜⑥ の順に 1 段ずつ削る。

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use super::card::{
    BLANK_LABELS, ITEM_STATE_LABELS, MARK_LABELS, MODE_LABELS, OPTION_STATE_LABELS,
    POLARITY_LABELS, READ_STATE_LABELS, ROLE_LABELS, SCOPE_LABELS, TYPE_LABELS, label, labels,
    paper_of, read_run_task,
};
use super::compute::{self, NodeRow};
use super::defs::Defs;
use super::store::{self, Proposal, SearchRecord};
use super::{Result, SciError, fmt_ref};
use crate::paths::PaperRoots;

// ---- 木が無い・節点が無いときの 2 行 ------------------------------------------

/// sci を始めていないときの文。
const NO_TREE: &str = "\
# 今回の行動（sai-sci より）
研究方針の木がまだありません。ユーザーに `sai-moon-paper init --assets-only --sci --type <型>` の実行（または Web の Research-tree の画面の「始める」）を頼んでください。";

/// 節点の指定が無いときの文。
const NO_NODE: &str = "\
# 今回の行動（sai-sci より）
節点の指定がありません。この実行は Research-tree の画面か `sai-moon-paper sci card` で作られていません。
依頼文に書かれた範囲だけで作業し、最初にユーザーへ「研究方針のどの項目のための行動か」を確かめてください。";

/// 出せる提案の節の注意書き。
const OPS_NOTE: &str = "提案は MCP の `sci_propose`（pi は `sai-moon-paper sci propose`）で出す。書き方は sci-core の決まりに従う。";

/// 土壌で出す節の並び（この順）。
const SOIL_HEADINGS: [&str; 6] = [
    "始まり",
    "研究の型",
    "与えられていること",
    "制約",
    "価値の軸",
    "興味がある / 興味が無い",
];

/// 長すぎるときに付ける文。
const CUT_NOTE: &str = "…（長いので切りました）";

/// 節点 1 個の全文（対象の節点の節）。
#[derive(Clone)]
struct Detail {
    /// 見出し行（`### N12 小目的（…）`）。
    head: String,
    /// 本文。
    body: Option<String>,
    /// 問いの行。
    questions: Vec<String>,
    /// 候補の行。
    options: Vec<String>,
    /// 知見の行（古い順）。
    findings: Vec<String>,
    /// 結び付きの行。
    links: Vec<String>,
}

/// 対象の節点の節の 1 要素。1 行で書くとき（全体の要約・兄弟）は [`Section::Line`]。
#[derive(Clone)]
enum Section {
    /// 節点 1 個の全文。
    Detail(Detail),
    /// 1 行。
    Line(String),
}

/// 組み立てた範囲。節の並びは C-06 の順で固定。
struct Parts {
    /// 今回の行動。`blind` のときは節点の行を外す。
    action: Vec<String>,
    /// 研究の現在地。`blind` なら空。
    place: Vec<String>,
    /// 土壌（見出しと本文の行。上から 3 行まで）。
    soil: Vec<(String, Vec<String>)>,
    /// 対象の節点の見出し。無い範囲なら空。
    node_head: String,
    /// 対象の節点の中身。
    nodes: Vec<Section>,
    /// 上流の道。無い範囲なら空。
    upstream: Vec<String>,
    /// 仮置き。`blind` なら空。
    assume: Vec<String>,
    /// 範囲外。無い範囲なら空。
    out_scope: Vec<String>,
    /// 項目ガイドの全文。
    guide_full: Vec<String>,
    /// 項目ガイドの「問いと埋まった条件」だけ。
    guide_short: Vec<String>,
    /// 既に調べたことの見出し。
    searched_head: Option<String>,
    /// 既に調べたこと（検索と却下された提案。新しい順）。
    searched: Vec<String>,
    /// 出せる提案。
    ops: Vec<String>,
    /// 対象論文（`blind` だけ）。
    papers: Vec<String>,
}

/// C-06 の Markdown を作る。`scope_override` があれば task.md の `sci_scope` より優先する。
///
/// # Errors
///
/// 実行が見つからない（[`SciError::NotFound`]）、節点が見つからない、知らない範囲を
/// 指定した（[`SciError::Invalid`]）、木を読む途中の DB エラー。
pub fn build(
    conn: &Connection,
    roots: &PaperRoots,
    defs: &Defs,
    max_chars: usize,
    run_id: i64,
    scope_override: Option<&str>,
) -> Result<String> {
    let task = read_run_task(roots, run_id)?;
    if !store::is_initialized(conn)? {
        return Ok(NO_TREE.to_string());
    }
    let Some(target_id) = task.keys.node else {
        return Ok(NO_NODE.to_string());
    };
    let scope = resolve_scope(roots, &task.block, &task.keys.scope, scope_override)?;
    let parts = Parts::collect(conn, roots, defs, &task, target_id, &scope)?;

    // ①〜⑤まで順に 1 段ずつ削って、収まった所で止める。
    for level in 0..=5u8 {
        let text = parts.render(level);
        if text.chars().count() <= max_chars {
            return Ok(text);
        }
    }
    // ⑥ それでも長ければ上限で切って、切ったことを書く。
    Ok(cut(&parts.render(5), max_chars))
}

/// 範囲を決める。`scope_override` → `sci_scope` → ブロック定義の `sci_scope` → `path`。
fn resolve_scope(
    roots: &PaperRoots,
    block: &str,
    keys_scope: &str,
    scope_override: Option<&str>,
) -> Result<String> {
    let from_override = scope_override.map(str::trim).unwrap_or_default();
    let from_keys = keys_scope.trim();
    let scope = if !from_override.is_empty() {
        from_override.to_string()
    } else if !from_keys.is_empty() {
        from_keys.to_string()
    } else {
        let from_block = block_scope(roots, block);
        if from_block.is_empty() {
            "path".to_string()
        } else {
            from_block
        }
    };
    if !SCOPE_LABELS.iter().any(|(key, _)| *key == scope) {
        return Err(SciError::Invalid(format!("知らない範囲です: {scope}")));
    }
    Ok(scope)
}

/// ブロック定義の frontmatter の `sci_scope` を読む。無ければ空。
///
/// 探し方は `ops::allowed_ops` と同じ（`<root>/.sai/blocks/` を先に、`~/.sai-moon/blocks/` を次に）。
fn block_scope(roots: &PaperRoots, block: &str) -> String {
    if block.is_empty() {
        return String::new();
    }
    let mut candidates = vec![roots
        .root
        .join(".sai")
        .join("blocks")
        .join(format!("{block}.md"))];
    if let Some(dir) = crate::paths::sai_home_path("blocks") {
        candidates.push(dir.join(format!("{block}.md")));
    }
    for path in candidates {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some((_, value)) = crate::runs::read_frontmatter(&text)
            .into_iter()
            .find(|(key, _)| key == "sci_scope")
        {
            return value.trim().to_string();
        }
    }
    String::new()
}

impl Parts {
    /// C-06 の各節を集める。範囲外の節は、ほかの節の文から算出して最後に足す。
    fn collect(
        conn: &Connection,
        roots: &PaperRoots,
        defs: &Defs,
        task: &super::card::RunTask,
        target_id: i64,
        scope: &str,
    ) -> Result<Self> {
        let blind = scope == "blind";
        let with_scope = matches!(scope, "node" | "path" | "subtree" | "siblings");
        let status = compute::status(conn, roots, defs)?;
        let rows: Vec<NodeRow> = status.nodes.clone();
        let Some(target_row) = rows
            .iter()
            .find(|row| row.id == fmt_ref('N', target_id))
        else {
            return Err(SciError::NotFound(format!(
                "節点 {} がありません",
                fmt_ref('N', target_id)
            )));
        };

        // 今回の行動。
        let mut action = vec!["# 今回の行動（sai-sci より）".to_string()];
        let block = if task.block.is_empty() {
            "ブロック不明"
        } else {
            task.block.as_str()
        };
        action.push(format!("- 実行: run {}（{block}）", task.run_id));
        if !blind {
            action.push(format!("- 節点: {} {}", target_row.id, target_row.title));
        }
        action.push(format!("- 目的: {}", none_if_empty(&task.keys.purpose)));
        action.push(format!(
            "- 問い: {}",
            question_text(conn, &task.keys.question)?
        ));
        action.push(format!(
            "- 期待する成果: {}",
            none_if_empty(&task.keys.expect)
        ));
        action.push(format!("- 範囲: {}", label(&SCOPE_LABELS, scope)));
        if !task.keys.mode.is_empty() {
            action.push(format!("- 型: {}", label(&MODE_LABELS, &task.keys.mode)));
        }

        // 研究の現在地（`blind` なら出さない）。
        let mut place = Vec::new();
        if !blind {
            place.push("## 研究の現在地".to_string());
            if let Some(type_info) = &status.type_info {
                place.push(format!("- 研究の型: {}", type_info.name));
            }
            if let Some(seed) = &status.seed {
                place.push(format!("- 種の文: 「{}」", seed.text));
            }
            if let Some(gate) = &status.gate {
                place.push(format!("- 関門: {}/{} {}", gate.current, gate.total, gate.name));
            }
        }

        // 土壌（`blind` なら出さない）。
        let soil = if blind {
            Vec::new()
        } else {
            let sections = compute::mind_sections(&roots.data_dir);
            SOIL_HEADINGS
                .iter()
                .filter_map(|heading| {
                    let (_, body) = sections.iter().find(|(name, _)| name == heading)?;
                    Some(((*heading).to_string(), body_lines(body, 3)))
                })
                .collect()
        };

        // 対象の節点（節点そのもの / 子孫 / 兄弟 / 全体の要約）。
        let (node_head, nodes) = match scope {
            "blind" => (String::new(), Vec::new()),
            "summary" => ("## 全体の要約".to_string(), summary_sections(&rows)),
            "siblings" => (
                "## 対象の節点".to_string(),
                sibling_sections(conn, &rows, target_id)?,
            ),
            "subtree" => (
                "## 対象の節点".to_string(),
                detail_sections(conn, roots, defs, &rows, target_id, true)?,
            ),
            _ => (
                "## 対象の節点".to_string(),
                detail_sections(conn, roots, defs, &rows, target_id, false)?,
            ),
        };

        // 上流の道。
        let upstream = if matches!(scope, "path" | "subtree" | "siblings") {
            upstream_lines(conn, &rows, target_id)?
        } else {
            Vec::new()
        };

        // 仮置き（`blind` なら出さない）。
        let mut assume = Vec::new();
        if !blind {
            let by_id: HashMap<&str, &NodeRow> =
                rows.iter().map(|row| (row.id.as_str(), row)).collect();
            for (id, value) in &task.keys.assume {
                let node_id = fmt_ref('N', *id);
                let title = by_id
                    .get(node_id.as_str())
                    .map_or_else(String::new, |row| row.title.clone());
                assume.push(format!("- {node_id} {title} = {value}"));
            }
            if !assume.is_empty() {
                assume.insert(0, "## 仮置き（この議論では問い直さない）".to_string());
            }
        }

        // 項目ガイド。
        let mut guide_full = Vec::new();
        let mut guide_short = Vec::new();
        if with_scope {
            let detail = compute::node_detail(conn, roots, defs, target_id)?;
            if let Some(guide) = &detail.guide {
                guide_full.push(format!("## 項目ガイド: {}", guide.name));
                if !guide.questions.is_empty() {
                    let line = format!("- 問い: {}", guide.questions.join(" / "));
                    guide_full.push(line.clone());
                    guide_short.push(line);
                }
                let condition = if guide.condition_text.trim().is_empty() {
                    guide.condition.clone()
                } else {
                    guide.condition_text.clone()
                };
                if !condition.trim().is_empty() {
                    let line = format!("- 埋まった条件: {}", condition.trim());
                    guide_full.push(line.clone());
                    guide_short.push(line);
                }
                if !guide.extract.is_empty() {
                    let extract: Vec<String> = guide
                        .extract
                        .iter()
                        .map(|row| {
                            format!(
                                "{}（{}）",
                                label(&ROLE_LABELS, &row.role),
                                row.fields.trim()
                            )
                        })
                        .collect();
                    guide_full.push(format!("- 抜き出す項目: {}", extract.join(" / ")));
                }
                if !guide.pitfalls.is_empty() {
                    guide_full.push(format!("- 落とし穴: {}", guide.pitfalls.join(" / ")));
                }
            }
        }

        // 出せる提案。
        let ops = vec![
            "## 出せる提案".to_string(),
            super::ops::allowed_ops(roots, &task.block).join(", "),
            OPS_NOTE.to_string(),
        ];

        // 対象論文（`blind` だけ）。
        let mut papers = Vec::new();
        if blind {
            for id in &task.keys.papers {
                let title = paper_of(conn, *id)?
                    .map_or_else(String::new, |(_, title, _)| title);
                papers.push(format!("- #{id} {title}"));
            }
        }

        // 既に調べたこと。
        let mut searched = Vec::new();
        if !blind {
            searched = searched_lines(conn, target_id, scope)?;
        }

        // 範囲外（ほかの節に出てこない一番上の項目）。
        let mut out_scope = Vec::new();
        if with_scope {
            let mut parts = Self {
                action,
                place,
                soil,
                node_head,
                nodes,
                upstream,
                assume,
                out_scope: Vec::new(),
                guide_full,
                guide_short,
                searched_head: None,
                searched,
                ops,
                papers,
            };
            let mentioned = mentioned_ids(&parts.render(0));
            out_scope = rows
                .iter()
                .filter(|row| row.kind == "item" && row.parents.is_empty())
                .filter(|row| !mentioned.contains(&row.id))
                .map(|row| format!("{} {}", row.id, row.title))
                .collect();
            parts.out_scope = out_scope;
            parts.searched_head = (!parts.searched.is_empty())
                .then(|| "## 既に調べたこと".to_string());
            return Ok(parts);
        }
        let searched_head =
            (!searched.is_empty()).then(|| "## 既に調べたこと".to_string());
        Ok(Self {
            action,
            place,
            soil,
            node_head,
            nodes,
            upstream,
            assume,
            out_scope,
            guide_full,
            guide_short,
            searched_head,
            searched,
            ops,
            papers,
        })
    }

    /// C-06 の Markdown にする。`level` は削った段（0 が全部）。
    fn render(&self, level: u8) -> String {
        let mut blocks: Vec<String> = Vec::new();
        push_block(&mut blocks, &self.action);
        push_block(&mut blocks, &self.place);
        if !self.soil.is_empty() {
            let mut lines = vec!["## 土壌（mind.md から）".to_string()];
            for (heading, body) in &self.soil {
                // ④ 土壌 → 各節 1 行。
                let text = if level >= 4 {
                    body.first().cloned().unwrap_or_default()
                } else {
                    body.join(" ")
                };
                lines.push(format!("- {heading}: {text}"));
            }
            push_block(&mut blocks, &lines);
        }
        if let Some(text) = self.render_nodes(level) {
            blocks.push(text);
        }
        push_block(&mut blocks, &self.upstream);
        push_block(&mut blocks, &self.assume);
        if !self.out_scope.is_empty() {
            // ⑤ 範囲外 → 先頭 20 件。
            let items: Vec<&str> = if level >= 5 {
                self.out_scope.iter().take(20).map(String::as_str).collect()
            } else {
                self.out_scope.iter().map(String::as_str).collect()
            };
            let lines = vec![
                "## 範囲外（この回では議論しない）".to_string(),
                items.join(" / "),
            ];
            push_block(&mut blocks, &lines);
        }
        if !self.guide_full.is_empty() {
            // ③ 項目ガイド → 問いと埋まった条件だけ。
            let lines = if level >= 3 {
                let mut lines = vec![self.guide_full[0].clone()];
                lines.extend(self.guide_short.iter().cloned());
                lines
            } else {
                self.guide_full.clone()
            };
            push_block(&mut blocks, &lines);
        }
        if let Some(head) = &self.searched_head {
            let mut lines = vec![head.clone()];
            // ① 既に調べたこと → 新しい 5 件ずつ。
            if level >= 1 {
                lines.extend(self.searched.iter().take(5).cloned());
            } else {
                lines.extend(self.searched.iter().cloned());
            }
            blocks.push(lines.join("\n"));
        }
        push_block(&mut blocks, &self.ops);
        if !self.papers.is_empty() {
            let mut lines = vec!["## 対象論文".to_string()];
            lines.extend(self.papers.iter().cloned());
            push_block(&mut blocks, &lines);
        }
        blocks.join("\n\n")
    }

    /// 対象の節点の節。
    fn render_nodes(&self, level: u8) -> Option<String> {
        if self.nodes.is_empty() || self.node_head.is_empty() {
            return None;
        }
        let mut lines = vec![self.node_head.clone()];
        for section in &self.nodes {
            match section {
                Section::Line(line) => lines.push(line.clone()),
                Section::Detail(detail) => {
                    lines.push(detail.head.clone());
                    if let Some(body) = &detail.body {
                        lines.push(body.clone());
                    }
                    lines.extend(detail.questions.iter().cloned());
                    lines.extend(detail.options.iter().cloned());
                    // ② 対象の節点の知見 → 新しい 10 件。
                    let from = if level >= 2 && detail.findings.len() > 10 {
                        detail.findings.len() - 10
                    } else {
                        0
                    };
                    lines.extend(detail.findings[from..].iter().cloned());
                    lines.extend(detail.links.iter().cloned());
                }
            }
        }
        Some(lines.join("\n"))
    }
}

/// 空の節は出さない。
fn push_block(blocks: &mut Vec<String>, lines: &[String]) {
    let text = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    if !text.is_empty() {
        blocks.push(text);
    }
}

/// 空文字なら「なし」。
fn none_if_empty(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "なし".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 問いの行。ID と文。無ければ「なし」。
fn question_text(conn: &Connection, question: &Option<i64>) -> Result<String> {
    let Some(id) = question else {
        return Ok("なし".to_string());
    };
    let text = store::get_question(conn, *id)?.text;
    Ok(format!("{} {text}", fmt_ref('Q', *id)))
}

// ---- 節点の中身の行 -----------------------------------------------------------

/// 節点の全文の節を作る。`subtree` なら子孫も入れる。
///
/// 並びは `status` の節点の並び（型の上から深さ優先）にそろえる。
fn detail_sections(
    conn: &Connection,
    roots: &PaperRoots,
    defs: &Defs,
    rows: &[NodeRow],
    target_id: i64,
    subtree: bool,
) -> Result<Vec<Section>> {
    let mut wanted = vec![target_id];
    if subtree {
        wanted.extend(store::descendants(conn, target_id)?);
    }
    let wanted: HashSet<i64> = wanted.into_iter().collect();
    let mut out = Vec::new();
    for row in rows
        .iter()
        .filter(|row| parse_node_id(&row.id).is_some_and(|id| wanted.contains(&id)))
    {
        let id = parse_node_id(&row.id).unwrap_or_default();
        out.push(Section::Detail(detail_of(conn, roots, defs, id, row)?));
    }
    Ok(out)
}

/// 節点 1 個の全文の行を作る。
fn detail_of(
    conn: &Connection,
    roots: &PaperRoots,
    defs: &Defs,
    id: i64,
    row: &NodeRow,
) -> Result<Detail> {
    let detail = compute::node_detail(conn, roots, defs, id)?;

    // 見出しの括弧の中は「型・状態・空欄の種類か印」（無いものは省く）。
    let mut parts: Vec<String> = Vec::new();
    if !row.types.is_empty() {
        parts.push(labels(&TYPE_LABELS, &row.types));
    }
    let state_table = if row.kind == "item" {
        &ITEM_STATE_LABELS
    } else {
        &OPTION_STATE_LABELS
    };
    parts.push(label(state_table, &row.state));
    if let Some(blank) = &row.blank {
        parts.push(label(&BLANK_LABELS, blank));
    } else if !row.marks.is_empty() {
        parts.push(labels(&MARK_LABELS, &row.marks));
    }

    let questions = detail
        .questions
        .iter()
        .map(|question| {
            let state = if question.state == "open" {
                "未回答"
            } else {
                "答えた"
            };
            format!("- 問い: {} {}（{state}）", question.id, question.text)
        })
        .collect();
    let options = detail
        .options
        .iter()
        .map(|option| {
            format!(
                "- 候補: {} {}（{}・支持 {}・反証 {}）",
                option.id,
                option.title,
                label(&OPTION_STATE_LABELS, &option.state),
                option.support,
                option.refute
            )
        })
        .collect();
    let findings = detail.findings.iter().map(finding_line).collect();
    let links = detail.links.iter().map(link_line).collect();

    Ok(Detail {
        head: format!("### {} {}（{}）", row.id, row.title, parts.join("・")),
        body: (!row.body.trim().is_empty()).then(|| row.body.trim().to_string()),
        questions,
        options,
        findings,
        links,
    })
}

/// 知見の行。
fn finding_line(finding: &compute::FindingRow) -> String {
    let mut line = format!(
        "- 知見: {}［{}］{}",
        finding.id,
        label(&POLARITY_LABELS, &finding.polarity),
        finding.text.trim()
    );
    if !finding.cites.is_empty() {
        line.push_str(&format!(" [{}]", finding.cites.join(" ")));
    }
    line
}

/// 結び付きの行。論文は `論文`、それ以外は `結び付き`。
fn link_line(link: &compute::LinkRow) -> String {
    if let Some(paper_id) = link.target.strip_prefix("paper:") {
        let mut parts: Vec<String> = Vec::new();
        if let Some(role) = &link.role {
            parts.push(label(&ROLE_LABELS, role));
        }
        if let Some(read_state) = &link.read_state {
            parts.push(label(&READ_STATE_LABELS, read_state));
        }
        let head = format!("- 論文: #{paper_id} {}", link.title.trim());
        if parts.is_empty() {
            head
        } else {
            format!("{head}（{}）", parts.join("・"))
        }
    } else {
        format!("- 結び付き: {} {}", link.target, link.title.trim())
    }
}

/// 全体の要約（全項目を 1 行ずつ）。
fn summary_sections(rows: &[NodeRow]) -> Vec<Section> {
    rows.iter()
        .filter(|row| row.kind == "item")
        .map(|row| Section::Line(one_liner(row)))
        .collect()
}

/// 対象の節点と、親が同じ兄弟（1 行ずつ）。
fn sibling_sections(conn: &Connection, rows: &[NodeRow], target_id: i64) -> Result<Vec<Section>> {
    let parents: Vec<String> = store::parents_of(conn, target_id)?
        .into_iter()
        .map(|parent| fmt_ref('N', parent))
        .collect();
    let mut out = Vec::new();
    for row in rows.iter().filter(|row| row.kind == "item") {
        let id = parse_node_id(&row.id).unwrap_or_default();
        let is_sibling = id != target_id
            && !parents.is_empty()
            && parents
                .iter()
                .all(|parent| row.parents.contains(parent));
        if id == target_id || is_sibling {
            out.push(Section::Line(one_liner(row)));
        }
    }
    Ok(out)
}

/// 1 行の要約。
fn one_liner(row: &NodeRow) -> String {
    let state = if row.kind == "item" {
        label(&ITEM_STATE_LABELS, &row.state)
    } else {
        label(&OPTION_STATE_LABELS, &row.state)
    };
    format!(
        "- {} {}（{}・支持 {}・反証 {}・論文 {}）",
        row.id, row.title, state, row.counts.support, row.counts.refute, row.counts.papers
    )
}

// ---- 上流の道・既に調べたこと -------------------------------------------------

/// 上流の道（対象の節点から `parent` の線を根までたどった項目と、それらの `depends` の上流）。
fn upstream_lines(conn: &Connection, rows: &[NodeRow], target_id: i64) -> Result<Vec<String>> {
    let mut chain: Vec<i64> = Vec::new();
    let mut current = target_id;
    while let Some(parent) = store::parents_of(conn, current)?.first().copied() {
        if chain.contains(&parent) {
            break;
        }
        chain.push(parent);
        current = parent;
    }
    // 根から順に出す。
    chain.reverse();
    // `depends` の上流を足す（同じ節点は 1 回だけ）。
    let mut with_depends = chain.clone();
    for ancestor in &chain {
        for edge in store::list_edges(conn)? {
            if edge.kind == "depends" && edge.from == *ancestor && !with_depends.contains(&edge.to) {
                with_depends.push(edge.to);
            }
        }
    }
    let mut lines = Vec::new();
    for id in with_depends {
        let node_id = fmt_ref('N', id);
        let Some(row) = rows
            .iter()
            .find(|row| row.kind == "item" && row.id == node_id)
        else {
            continue;
        };
        let state = label(&ITEM_STATE_LABELS, &row.state);
        // 未決の項目は値の欄を出さない（値は `［？…］` になるだけだから）。
        if decided_value(row).is_some() {
            lines.push(format!(
                "- {} {} = {}（{state}）",
                node_id,
                row.title,
                decided_value(row).unwrap_or_default()
            ));
        } else {
            lines.push(format!("- {} {}（{state}）", node_id, row.title));
        }
    }
    if lines.is_empty() {
        return Ok(lines);
    }
    let mut out = vec!["## 上流の道（なぜこの節点を扱うか）".to_string()];
    out.extend(lines);
    Ok(out)
}

/// 決まっている項目の値。まだ決まっていなければ `None`。
fn decided_value(row: &NodeRow) -> Option<String> {
    if matches!(row.state.as_str(), "open" | "excluded") {
        return None;
    }
    let value = row.value.trim();
    if value.is_empty() || value.starts_with('［') {
        return None;
    }
    Some(value.to_string())
}

/// 既に調べたこと（検索の記録と、却下された提案。新しい順）。
fn searched_lines(
    conn: &Connection,
    target_id: i64,
    scope: &str,
) -> Result<Vec<String>> {
    let mut targets = vec![target_id];
    if scope == "subtree" {
        targets.extend(store::descendants(conn, target_id)?);
    }

    let mut searches: Vec<SearchRecord> = Vec::new();
    for id in &targets {
        for search in store::list_searches(conn, Some(*id))? {
            if !searches.iter().any(|known| known.id == search.id) {
                searches.push(search);
            }
        }
    }
    searches.sort_by(|a, b| (b.created_at.as_str(), b.id).cmp(&(a.created_at.as_str(), a.id)));

    let mut rejected: Vec<Proposal> = Vec::new();
    for proposal in store::list_proposals(conn, Some("rejected"))? {
        if proposal.node_id.is_some_and(|id| targets.contains(&id))
            && !rejected.iter().any(|known| known.id == proposal.id)
        {
            rejected.push(proposal);
        }
    }

    let mut lines: Vec<String> = searches.iter().map(search_line).collect();
    lines.extend(rejected.iter().map(rejected_line));
    Ok(lines)
}

/// 検索の行。
fn search_line(search: &SearchRecord) -> String {
    let date: String = search.created_at.chars().take(10).collect();
    let extra = if search.widen_reason.trim().is_empty() {
        String::new()
    } else {
        format!("・広げた理由: {}", search.widen_reason.trim())
    };
    format!(
        "- 検索: {date}「{}」（範囲 {}・{} 件{extra}）",
        search.query.trim(),
        search.scope,
        search.hits
    )
}

/// 却下された提案の行。
fn rejected_line(proposal: &Proposal) -> String {
    let payload = serde_json::to_string(&proposal.payload).unwrap_or_default();
    let payload: String = payload.chars().take(40).collect();
    format!(
        "- 却下された提案: {} {}「{payload}」（理由: {}）",
        fmt_ref('P', proposal.id),
        proposal.op,
        proposal.reason.trim()
    )
}

// ---- 長さの扱い ---------------------------------------------------------------

/// 本文の行を先頭 `take` 行ぶん取り出す（前後の空白を落とし、空の行は捨てる）。
fn body_lines(body: &str, take: usize) -> Vec<String> {
    body.lines()
        .take(take)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// 上限で切って、切ったことを書く。
fn cut(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(CUT_NOTE.chars().count());
    let head: String = text.chars().take(keep).collect();
    format!("{head}{CUT_NOTE}")
}

// ---- 補助 ---------------------------------------------------------------------

/// `N12` を 12 にする。違う形なら `None`。
fn parse_node_id(text: &str) -> Option<i64> {
    text.strip_prefix('N')
        .and_then(|rest| rest.parse::<i64>().ok())
}

/// 文中の `N<数>` を集める（「範囲外」の計算に使う）。
fn mentioned_ids(text: &str) -> HashSet<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = HashSet::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index].is_ascii_uppercase() {
            let start = index;
            while index < chars.len() && chars[index].is_ascii_alphanumeric() {
                index += 1;
            }
            let token: String = chars[start..index].iter().collect();
            if token.len() > 1
                && token.starts_with('N')
                && token[1..].chars().all(|c| c.is_ascii_digit())
            {
                out.insert(token);
            }
        } else {
            index += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::sci::defs;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// 木で使った節点の番号。
    struct Tree {
        background: i64,
        purpose: i64,
        method: i64,
        subgoal: i64,
        option: i64,
    }

    /// テスト用の置き場。Drop で消す。
    struct Fixture {
        dir: PathBuf,
        roots: PaperRoots,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    /// 木のある置き場を作る。
    ///
    /// 一番上に 背景・目的・手法。背景の子が 背景の問い、その子に 最深部の問い。
    /// 目的の子に 小目的（項目ガイドのキー `purpose.subgoal`）、その子に候補。
    fn fixture() -> (Fixture, Database, Tree) {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-context-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".sai").join("blocks")).unwrap();
        fs::create_dir_all(dir.join(".sai").join("paper")).unwrap();
        let roots = PaperRoots {
            root: dir.clone(),
            data_dir: dir.join(".sai").join("paper"),
        };
        fs::write(
            roots.data_dir.join("mind.md"),
            "## 始まり\nてんかん外科の支援から\n焦点切除を目指す\n\n## 制約\n学会が 1 年に 1 回\n\n## メモ\nこれは出さない節\n",
        )
        .unwrap();
        fs::write(
            dir.join(".sai").join("blocks").join("sci-compare.md"),
            "---\nid: sci-compare\nname: 比べる\nsci_scope: path\n---\n\n本文\n",
        )
        .unwrap();

        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        conn.execute(
            "INSERT INTO papers (id, title, published_at) VALUES (37, '論文 37', '2021-05-01')",
            [],
        )
        .unwrap();
        store::meta_set(conn, "type", "problem").unwrap();

        let background = add_item(conn, None, &[], "背景", 0, "てんかん外科の支援");
        let method = add_item(conn, None, &[], "手法", 1, "");
        let purpose = add_item(conn, Some(background), &["choose"], "目的", 1, "");
        let background_child = add_item(conn, Some(background), &[], "背景の問い", 0, "");
        let subgoal = add_item(conn, Some(purpose), &["choose"], "小目的", 0, "");
        add_item(conn, Some(background_child), &[], "最深部の問い", 0, "");
        let option = store::insert_node(
            conn,
            &store::NewNode {
                kind: "option",
                item_key: None,
                types: &[],
                title: "発作部位の予測",
                body: "",
                state: "candidate",
                fixed: false,
                ord: 0,
                created_by: "user",
            },
        )
        .unwrap();
        store::insert_edge(conn, "parent", subgoal, option).unwrap();
        // 背景は決定済み（値は本文の 1 行目）。
        store::set_node_state(conn, background, "decided").unwrap();

        // 既に調べたこと・知見・結び付き。
        for number in 1..=12 {
            store::insert_search(
                conn,
                &store::NewSearch {
                    run_id: None,
                    node_id: Some(subgoal),
                    question_id: None,
                    tool: "search",
                    query: &format!("seizure onset zone {number}"),
                    scope: "library",
                    widen_reason: "",
                    hits: 12,
                    hit_papers: &[37],
                    set_id: None,
                },
            )
            .unwrap();
        }
        for number in 1..=14 {
            store::insert_finding(
                conn,
                &store::NewFinding {
                    node_id: option,
                    question_id: None,
                    text: &format!("知見 {number} の本文"),
                    cites: &["#37 §4.2".to_string()],
                    polarity: if number % 2 == 0 { "refute" } else { "support" },
                    level: Some(2),
                    source: "run",
                    report_id: None,
                    paper_id: Some(37),
                    run_id: Some(120),
                    proposal_id: None,
                    auto: false,
                    created_by: "user",
                },
            )
            .unwrap();
        }
        store::insert_proposal(
            conn,
            &store::NewProposal {
                run_id: None,
                block: "sci-ideate",
                op: "add_option",
                node_id: Some(subgoal),
                payload: &serde_json::json!({"title": "候補"}),
                reason: "候補が 1 つしか無い",
                risk: "human",
                status: "rejected",
            },
        )
        .unwrap();
        store::upsert_link(
            conn,
            &store::NewLink {
                node_id: subgoal,
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
        .unwrap();
        store::upsert_link(
            conn,
            &store::NewLink {
                node_id: subgoal,
                target_kind: "run",
                target_ref: "120",
                role: None,
                read_state: None,
                reason: "調べる",
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .unwrap();

        (
            Fixture { dir, roots },
            db,
            Tree {
                background,
                purpose,
                method,
                subgoal,
                option,
            },
        )
    }

    /// 項目を 1 つ足す。
    fn add_item(
        conn: &Connection,
        parent: Option<i64>,
        types: &[&str],
        title: &str,
        ord: i64,
        body: &str,
    ) -> i64 {
        let types: Vec<String> = types.iter().map(|value| (*value).to_string()).collect();
        let id = store::insert_node(
            conn,
            &store::NewNode {
                kind: "item",
                item_key: if title == "小目的" {
                    Some("purpose.subgoal")
                } else {
                    None
                },
                types: &types,
                title,
                body,
                state: "open",
                fixed: false,
                ord,
                created_by: "user",
            },
        )
        .unwrap();
        if let Some(parent) = parent {
            store::insert_edge(conn, "parent", parent, id).unwrap();
        }
        id
    }

    /// sci キーを持つ実行の task.md を書く。
    fn write_run(roots: &PaperRoots, run_id: i64, body: &str) {
        let dir = roots
            .root
            .join(".sai")
            .join("outputs")
            .join("todo")
            .join(run_id.to_string());
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("task.md"),
            format!(
                "---\nrunner: pi\ncreation_date: 2026/09/27 10:00:00\norigin_run_id: -\nrun_id: {run_id}\nblock: sci-compare\nthinking: \nmodel: \nprovider: \nround: 1\nglobal_round: 1\ntag: \ndescription: 候補を比べる\n{body}---\n\n# task_text\n\n本文\n"
            ),
        )
        .unwrap();
    }

    fn defs() -> Defs {
        defs::load(Path::new("target/sci-context-defs-not-found"))
    }

    fn build_with(fix: &Fixture, db: &Database, max: usize, scope: Option<&str>) -> String {
        build(db.connection(), &fix.roots, &defs(), max, 120, scope).unwrap()
    }

    fn count(text: &str, needle: &str) -> usize {
        text.lines().filter(|line| line.contains(needle)).count()
    }

    /// 6 つの範囲で、節の出し方が C-06 の表のとおりになること。
    #[test]
    fn context_sections_follow_scope_table() {
        let (fix, db, tree) = fixture();
        write_run(
            &fix.roots,
            120,
            &format!(
                "sci_node: {}\nsci_purpose: 候補を比べる\nsci_assume: {}=発作部位の予測（仮）\nsci_papers: 37\n",
                fmt_ref('N', tree.subgoal),
                fmt_ref('N', tree.option)
            ),
        );
        let full = |scope: &str| build_with(&fix, &db, 100_000, Some(scope));
        let subgoal = fmt_ref('N', tree.subgoal);
        let option = fmt_ref('N', tree.option);

        // 節点
        let text = full("node");
        assert!(text.contains("## 研究の現在地"), "{text}");
        assert!(text.contains("## 土壌（mind.md から）"), "{text}");
        assert!(text.contains("## 対象の節点"), "{text}");
        assert!(text.contains(&format!("### {subgoal} 小目的")), "{text}");
        assert!(text.contains("## 仮置き（この議論では問い直さない）"), "{text}");
        assert!(text.contains("## 範囲外（この回では議論しない）"), "{text}");
        assert!(text.contains("## 項目ガイド: 小目的"), "{text}");
        assert!(text.contains("## 既に調べたこと"), "{text}");
        assert!(text.contains("## 出せる提案"), "{text}");
        // 節点では上流の道は出さない。対象論文も出さない。
        assert!(!text.contains("## 上流の道"), "{text}");
        assert!(!text.contains("## 対象論文"), "{text}");

        // 道
        let text = full("path");
        assert!(text.contains("## 上流の道（なぜこの節点を扱うか）"), "{text}");
        assert!(
            text.contains(&format!(
                "- {} 背景 = てんかん外科の支援（決定）",
                fmt_ref('N', tree.background)
            )),
            "{text}"
        );
        assert!(
            text.contains(&format!("- {} 目的（未決定）", fmt_ref('N', tree.purpose))),
            "{text}"
        );

        // 部分木（子孫も全文）
        let text = full("subtree");
        assert!(text.contains(&format!("### {subgoal} 小目的")), "{text}");
        assert!(text.contains(&format!("### {option} 発作部位の予測")), "{text}");
        assert!(text.contains("## 上流の道"), "{text}");

        // 兄弟（1 行ずつ）
        let text = full("siblings");
        assert!(text.contains("## 対象の節点"), "{text}");
        assert!(text.contains(&format!("- {subgoal} 小目的（未決定")), "{text}");
        assert!(!text.contains("## 全体の要約"), "{text}");

        // 全体の要約
        let text = full("summary");
        assert!(text.contains("## 全体の要約"), "{text}");
        assert!(!text.contains("## 対象の節点"), "{text}");
        assert!(!text.contains("## 上流の道"), "{text}");
        assert!(!text.contains("## 範囲外"), "{text}");
        assert!(!text.contains("## 項目ガイド"), "{text}");
        assert!(text.contains(&format!("- {} 背景", fmt_ref('N', tree.background))), "{text}");

        // 目隠し
        let text = full("blind");
        assert!(text.contains("- 範囲: 目隠し"), "{text}");
        assert!(!text.contains("- 節点:"), "{text}");
        assert!(!text.contains("## 研究の現在地"), "{text}");
        assert!(!text.contains("## 土壌"), "{text}");
        assert!(!text.contains("## 対象の節点"), "{text}");
        assert!(!text.contains("## 仮置き"), "{text}");
        assert!(!text.contains("## 範囲外"), "{text}");
        assert!(!text.contains("## 項目ガイド"), "{text}");
        assert!(!text.contains("## 既に調べたこと"), "{text}");
        assert!(text.contains("## 出せる提案"), "{text}");
        assert!(text.contains("## 対象論文"), "{text}");
    }

    /// 仮置きが掛かり、範囲外の節点の中身が入らないこと（R-I3）。
    #[test]
    fn context_assume_and_out_of_scope() {
        let (fix, db, tree) = fixture();
        write_run(
            &fix.roots,
            120,
            &format!(
                "sci_node: {}\nsci_purpose: 候補を比べる\nsci_assume: {}=発作部位の予測（仮）\nsci_papers: 37\n",
                fmt_ref('N', tree.subgoal),
                fmt_ref('N', tree.option)
            ),
        );
        let text = build_with(&fix, &db, 100_000, Some("path"));
        let option = fmt_ref('N', tree.option);

        // 仮置き。
        assert!(
            text.contains(&format!("- {option} 発作部位の予測 = 発作部位の予測（仮）")),
            "{text}"
        );
        // 範囲外は、上流にも当事人にも出てこなかった一番上の項目。
        let out = text
            .split("## 範囲外（この回では議論しない）")
            .nth(1)
            .unwrap()
            .lines()
            .nth(1)
            .unwrap()
            .to_string();
        assert!(out.contains(&format!("{} 手法", fmt_ref('N', tree.method))), "{text}");
        assert!(!out.contains("背景"), "{text}");
        assert!(!out.contains(&option), "{text}");
    }

    /// 木が無いときと節点の指定が無いときの文。
    #[test]
    fn context_messages_for_no_tree_and_no_node() {
        let (fix, db, tree) = fixture();
        let conn = db.connection();
        write_run(
            &fix.roots,
            120,
            &format!(
                "sci_node: {}\nsci_purpose: 候補を比べる\n",
                fmt_ref('N', tree.subgoal)
            ),
        );

        // 木が無い。
        conn.execute("DELETE FROM sci_meta", []).unwrap();
        let text = build(conn, &fix.roots, &defs(), 100_000, 120, None).unwrap();
        assert!(text.starts_with("# 今回の行動（sai-sci より）"), "{text}");
        assert!(text.contains("研究方針の木がまだありません"), "{text}");
        assert_eq!(text.lines().count(), 2, "{text}");

        // 木はあるが節点の指定が無い。
        store::meta_set(conn, "type", "problem").unwrap();
        write_run(&fix.roots, 121, "sci_purpose: 汎用カード\n");
        let text = build(conn, &fix.roots, &defs(), 100_000, 121, None).unwrap();
        assert!(text.contains("節点の指定がありません"), "{text}");
        assert_eq!(text.lines().count(), 3, "{text}");
    }

    /// 長すぎるときは ①→⑥ の順に削れること。
    #[test]
    fn context_trims_in_order() {
        let (fix, db, tree) = fixture();
        write_run(
            &fix.roots,
            120,
            &format!(
                "sci_node: {}\nsci_purpose: 候補を比べる\n",
                fmt_ref('N', tree.subgoal)
            ),
        );
        let parts = Parts::collect(
            db.connection(),
            &fix.roots,
            &defs(),
            &read_run_task(&fix.roots, 120).unwrap(),
            tree.subgoal,
            "path",
        )
        .unwrap();

        let full = parts.render(0);
        assert_eq!(count(&full, "- 検索:"), 12);
        assert_eq!(count(&full, "- 知見:"), 14);

        // ① 既に調べたこと → 新しい 5 件ずつ。
        let level1 = parts.render(1);
        assert_eq!(count(&level1, "- 検索:"), 5);
        assert_eq!(count(&level1, "- 知見:"), 14);
        // ② 対象の節点の知見 → 新しい 10 件。
        let level2 = parts.render(2);
        assert_eq!(count(&level2, "- 検索:"), 5);
        assert_eq!(count(&level2, "- 知見:"), 10);
        assert!(level2.contains("- 抜き出す項目:"), "{level2}");
        // ③ 項目ガイド → 問いと埋まった条件だけ。
        let level3 = parts.render(3);
        assert!(!level3.contains("- 抜き出す項目:"), "{level3}");
        assert!(level3.contains("- 問い: 1 回の研究で何を達成するか"), "{level3}");
        // ④ 土壌 → 各節 1 行。
        assert!(level3.contains("学会が 1 年に 1 回"), "{level3}");
        assert!(level3.contains("- 始まり: てんかん外科の支援から 焦点切除を目指す"), "{level3}");
        let level4 = parts.render(4);
        assert!(!level4.contains("焦点切除を目指す"), "{level4}");
        assert!(level4.contains("- 始まり: てんかん外科の支援から\n"), "{level4}");

        // 収まる上限なら切り詰めない。段の境目の長さなら、その段の文そのものになる。
        let text = build_with(&fix, &db, level1.chars().count(), Some("path"));
        assert_eq!(text, level1);
        let text = build_with(&fix, &db, level2.chars().count(), Some("path"));
        assert_eq!(text, level2);

        // ⑥ それでも長ければ上限で切って、切ったことを書く。
        let tiny = build_with(&fix, &db, 200, Some("path"));
        assert!(tiny.ends_with(CUT_NOTE), "{tiny}");
        assert!(tiny.chars().count() <= 200, "{}", tiny.chars().count());
    }

    /// 目隠しは節点も土壌も見せず、論文の番号と題だけを出すこと。
    #[test]
    fn context_blind_hides_tree() {
        let (fix, db, tree) = fixture();
        write_run(
            &fix.roots,
            120,
            &format!(
                "sci_node: {}\nsci_purpose: 候補を比べる\nsci_papers: 37\n",
                fmt_ref('N', tree.subgoal)
            ),
        );
        let text = build_with(&fix, &db, 100_000, Some("blind"));
        assert!(text.contains("- 実行: run 120（sci-compare）"), "{text}");
        assert!(text.contains("- 目的: 候補を比べる"), "{text}");
        assert!(text.contains("- 範囲: 目隠し"), "{text}");
        assert!(text.contains("## 対象論文\n- #37 論文 37"), "{text}");
        assert!(!text.contains("小目的"), "{text}");
        assert!(!text.contains("てんかん外科"), "{text}");
        assert!(!text.contains("既に調べたこと"), "{text}");
        assert!(text.contains("add_finding"), "{text}");
    }
}
