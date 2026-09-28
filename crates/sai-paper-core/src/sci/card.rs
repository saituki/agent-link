//! sci のカード（task.md の発行）と読む札の中身・終わり。
//!
//! ここで作るカードは「目的の付いた行動」だけである。目的の無いカード（節点の無い実行）は
//! 作れない（要件 R-A2）。読む札（`runner: user`）は利用者が自分で論文を読むための
//! カードで、節点・目的・論文 1 本・役割・深さを必ず書く。
//!
//! カードを作ったら節点と実行を `sci_links`（`target_kind = run`）で結ぶ。
//! 読む札はさらに節点と論文を結び付き（`read_state = unread`）にする。
//!
//! 出口は [`crate::handoff::create_handoff_with_extra`]。frontmatter の固定の 13 個の
//! キーの後ろに `sci_node` から `sci_depth` までを足す（C-05）。

use std::path::PathBuf;

use rusqlite::Connection;

use super::defs::Defs;
use super::store::{self, NewLink};
use super::{Result, SciError, SciKeys, fmt_ref};
use crate::database::Database;
use crate::handoff::{HandoffRequest, create_handoff_with_extra};
use crate::paths::PaperRoots;

// ---- C-01 の値と表示名 -------------------------------------------------------

/// 項目の型。
pub(crate) const TYPE_LABELS: [(&str, &str); 4] = [
    ("choose", "選ぶ"),
    ("list", "並べる"),
    ("write", "書く"),
    ("verify", "確かめる"),
];
/// 項目の状態。
pub(crate) const ITEM_STATE_LABELS: [(&str, &str); 4] = [
    ("open", "未決定"),
    ("tentative", "仮決定"),
    ("decided", "決定"),
    ("excluded", "対象外"),
];
/// 候補の状態。
pub(crate) const OPTION_STATE_LABELS: [(&str, &str); 4] = [
    ("candidate", "候補"),
    ("selected", "選択"),
    ("rejected", "却下"),
    ("dormant", "休眠"),
];
/// 範囲。
pub(crate) const SCOPE_LABELS: [(&str, &str); 6] = [
    ("node", "節点"),
    ("path", "道"),
    ("subtree", "部分木"),
    ("siblings", "兄弟"),
    ("summary", "全体の要約"),
    ("blind", "目隠し"),
];
/// sci-read の型。
pub(crate) const MODE_LABELS: [(&str, &str); 2] = [
    ("purpose_first", "目的先行"),
    ("blind_first", "目的後付け"),
];
/// 論文の役割。
pub(crate) const ROLE_LABELS: [(&str, &str); 7] = [
    ("exemplar", "代表例"),
    ("compare", "比較対象"),
    ("issue_evidence", "課題の根拠"),
    ("method_source", "手法の供給元"),
    ("data_source", "データの出典"),
    ("background", "背景"),
    ("counter", "反証"),
];
/// 読む深さ。
pub(crate) const DEPTH_LABELS: [(&str, &str); 3] = [
    ("abstract", "抄録"),
    ("figures", "図"),
    ("full", "精読"),
];
/// 読み具合。
pub(crate) const READ_STATE_LABELS: [(&str, &str); 4] = [
    ("unread", "未読"),
    ("read", "読んだ・未反映"),
    ("reflected", "反映済み"),
    ("irrelevant", "関係なし"),
];
/// 知見の向き。
pub(crate) const POLARITY_LABELS: [(&str, &str); 4] = [
    ("support", "支持"),
    ("refute", "反証"),
    ("neutral", "中立"),
    ("none", "該当なし"),
];
/// 空欄の種類の表示名。
pub(crate) const BLANK_LABELS: [(&str, &str); 5] = [
    ("untouched", "未着手"),
    ("in_progress", "調査中"),
    ("insufficient", "情報不足"),
    ("awaiting_decision", "判断待ち"),
    ("conflict", "対立"),
];
/// 印の表示名。
pub(crate) const MARK_LABELS: [(&str, &str); 2] =
    [("weak_evidence", "根拠不足"), ("recheck", "要再確認")];

/// 実行者。
const RUNNERS: [&str; 3] = ["pi", "cc", "user"];
/// 読む札のブロックの既定。
const READ_BLOCK: &str = "sci-read";
/// 仮置きの上限。
const MAX_ASSUME: usize = 5;
/// 対象論文の上限。
const MAX_PAPERS: usize = 20;
/// 目的の上限。
const MAX_PURPOSE: usize = 300;
/// 反映しない理由の上限。
const MAX_NO_REFLECT: usize = 500;

/// C-01 の表から表示名を引く。表に無い値はその値そのもの。
#[must_use]
pub(crate) fn label(table: &[(&str, &str)], value: &str) -> String {
    table
        .iter()
        .find(|(key, _)| *key == value)
        .map_or_else(|| value.to_string(), |(_, name)| (*name).to_string())
}

/// 表示名を `・` でつなぐ（空のものは落とす）。
#[must_use]
pub(crate) fn labels(table: &[(&str, &str)], values: &[String]) -> String {
    values
        .iter()
        .map(|value| label(table, value))
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join("・")
}

/// 実行フォルダの task.md を読んだもの（[`read_run_task`] の返り値）。
#[derive(Debug, Clone)]
pub(crate) struct RunTask {
    /// 実行 ID。
    pub run_id: i64,
    /// 置き場（`todo` / `running` / `review` / `done`）。
    pub owner: String,
    /// 実行するブロックの id。
    pub block: String,
    /// 実行者（`pi` / `cc` / `user`）。
    pub runner: String,
    /// sci キー。
    pub keys: SciKeys,
    /// task.md の全文。
    pub text: String,
}

/// 実行フォルダの task.md を読む。無ければ [`SciError::NotFound`]。
pub(crate) fn read_run_task(roots: &PaperRoots, run_id: i64) -> Result<RunTask> {
    let not_found = || SciError::NotFound(format!("実行 {run_id} が見つかりません"));
    let run = crate::runs::list_run_dirs(&roots.root)
        .into_iter()
        .find(|run| run.run_id == run_id)
        .ok_or_else(not_found)?;
    let text = std::fs::read_to_string(run.path.join("task.md")).map_err(|_| not_found())?;
    let front = crate::runs::read_frontmatter(&text);
    let get = |name: &str| -> String {
        front
            .iter()
            .find(|(key, _)| key == name)
            .map_or_else(String::new, |(_, value)| value.trim().to_string())
    };
    Ok(RunTask {
        run_id,
        owner: run.owner,
        block: get("block"),
        runner: get("runner"),
        keys: super::task_sci_keys(&text),
        text,
    })
}

// ---- カードの依頼 -------------------------------------------------------------

/// sci のカードの依頼（C-05）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CardSpec {
    /// 対象の節点。sci のカードでは必ず要る。
    pub node: i64,
    /// 実行するブロックの id。読む札は空でもよい（`sci-read` になる）。
    pub block: String,
    /// 行動の目的。1 行。sci のカードでは必ず要る。
    pub purpose: String,
    /// 答える問い。
    pub question: Option<i64>,
    /// 期待する成果。1 行。
    pub expect: String,
    /// 範囲（C-01）。空ならブロック定義の `sci_scope`、それも無ければ `path`。
    pub scope: String,
    /// 仮置き（`N9=発作部位の予測（仮）`）。
    pub assume: Vec<(i64, String)>,
    /// sci-read の型（C-01）。読む札では空にする。
    pub mode: String,
    /// 対象の論文の番号。読む札はちょうど 1 本。
    pub papers: Vec<i64>,
    /// 実行者（`pi` / `cc` / `user`）。`user` なら読む札。
    pub runner: String,
    /// 推薦した実行の run_id。
    pub origin_run: Option<i64>,
    /// 読む札の論文の役割（C-01）。読む札では必ず要る。
    pub role: String,
    /// 読む札の深さ（C-01）。空なら `full`。
    pub depth: String,
}

/// カードを作った結果。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CardResult {
    /// 予約された run_id。
    pub run_id: i64,
    /// task.md の絶対パス。
    pub path: PathBuf,
}

/// 読む札の中身（C-08 の `read` の JSON と同じ形）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ReadCard {
    /// 実行 ID。
    pub run_id: i64,
    /// 置き場。
    pub owner: String,
    /// 対象の節点。
    pub node: ReadNode,
    /// 行動の目的。
    pub purpose: String,
    /// 読む論文。
    pub paper: ReadPaper,
    /// 論文の役割。
    pub role: String,
    /// 読む深さ。
    pub depth: String,
    /// 読むときの問い。
    pub questions: Vec<String>,
    /// この実行で入れた知見。
    pub findings: Vec<ReadFinding>,
}

/// 読む札の節点。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ReadNode {
    /// 節点の ID（`N12`）。
    pub id: String,
    /// 節点の題。
    pub title: String,
}

/// 読む札の論文。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ReadPaper {
    /// `papers.id`。
    pub id: i64,
    /// 論文の題。
    pub title: String,
    /// 出版年（`published_at` の先頭 4 字）。無ければ `None`。
    pub year: Option<String>,
}

/// 読む札で入れた知見。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ReadFinding {
    /// 知見の ID（`F40`）。
    pub id: String,
    /// 知見の文。
    pub text: String,
    /// 向き（C-01 の値）。
    pub polarity: String,
}

/// 読む札を終える前の確かめの結果。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct FinishRead {
    /// 実行 ID。
    pub run_id: i64,
    /// この実行で入れた知見の数。
    pub findings: usize,
    /// 結び付きの読み具合を `reflected` にしたか。
    pub reflected: bool,
}

// ---- カードを作る -------------------------------------------------------------

/// 目的付きの sci のカードを todo に作り、節点と run を結ぶ。
///
/// 読む札（`runner = user`）のときは `block` が空なら `sci-read` になり、
/// 論文はちょうど 1 本・役割は要る・深さは空なら `full` になる。
///
/// # Errors
///
/// 検査に通らないもの（節点が無い、目的が無い、ブロックが `sci-` でない、範囲・型が
/// 知らない値、仮置きや論文が多すぎる、論文が無い、実行者が知らない）、節点が
/// 見つからない、ブロック定義が無い、DB・ファイルのエラー。
pub fn create_card(
    db: &Database,
    roots: &PaperRoots,
    defs: &Defs,
    spec: &CardSpec,
) -> Result<CardResult> {
    let conn = db.connection();
    let runner = if spec.runner.trim().is_empty() {
        "pi".to_string()
    } else {
        spec.runner.trim().to_string()
    };
    if !RUNNERS.contains(&runner.as_str()) {
        return Err(SciError::Invalid(format!(
            "知らない実行者です: {runner}"
        )));
    }
    let is_read = runner == "user";

    // 読む札の既定を先に決める（ブロックの空欄もここで埋める）。
    let block = if spec.block.trim().is_empty() {
        if is_read {
            READ_BLOCK.to_string()
        } else {
            String::new()
        }
    } else {
        spec.block.trim().to_string()
    };
    if !block.starts_with("sci-") {
        return Err(SciError::Invalid(
            "sci のブロックを指定してください（sci- で始まります）".to_string(),
        ));
    }

    // 目的（sci のカードは目的が要る）。
    let purpose = one_line(&spec.purpose);
    if purpose.is_empty() {
        return Err(SciError::Invalid(
            "目的を書いてください（sci のカードは目的が要ります）".to_string(),
        ));
    }
    if purpose.chars().count() > MAX_PURPOSE {
        return Err(SciError::Invalid(format!(
            "目的は {MAX_PURPOSE} 字以内にしてください（今は {} 字）",
            purpose.chars().count()
        )));
    }

    // 節点。
    let node = store::get_node(conn, spec.node).map_err(|_| {
        SciError::Invalid(format!(
            "対象の節点がなければ sci のカードは作れません: {}",
            fmt_ref('N', spec.node)
        ))
    })?;

    // 問い。
    if let Some(question) = spec.question {
        store::get_question(conn, question).map_err(|_| {
            SciError::Invalid(format!("問いがありません: {}", fmt_ref('Q', question)))
        })?;
    }

    // 範囲。
    let scope = spec.scope.trim().to_string();
    if !scope.is_empty() && !SCOPE_LABELS.iter().any(|(key, _)| *key == scope) {
        return Err(SciError::Invalid(format!("知らない範囲です: {scope}")));
    }

    // 仮置き。
    if spec.assume.len() > MAX_ASSUME {
        return Err(SciError::Invalid(format!(
            "仮置きは {MAX_ASSUME} 個までです"
        )));
    }
    for (id, _) in &spec.assume {
        store::get_node(conn, *id)
            .map_err(|_| SciError::Invalid(format!("仮置きの節点がありません: {}", fmt_ref('N', *id))))?;
    }

    // 読む札の型は空にする。ほかのカードは C-01 の型だけ。
    let mode = if is_read {
        String::new()
    } else {
        let mode = spec.mode.trim().to_string();
        if !mode.is_empty() && !MODE_LABELS.iter().any(|(key, _)| *key == mode) {
            return Err(SciError::Invalid(format!("知らない型です: {mode}")));
        }
        mode
    };

    // 論文。
    if spec.papers.len() > MAX_PAPERS {
        return Err(SciError::Invalid(format!(
            "対象論文は {MAX_PAPERS} 本までです"
        )));
    }
    if is_read && spec.papers.len() != 1 {
        return Err(SciError::Invalid(
            "読む札には論文を 1 本だけ指定してください".to_string(),
        ));
    }
    for paper_id in &spec.papers {
        if db.library_paper_by_id(*paper_id).ok().flatten().is_none() {
            return Err(SciError::Invalid(format!(
                "論文がありません: {paper_id}"
            )));
        }
    }

    // 読む札の役割と深さ。
    let role = spec.role.trim().to_string();
    let depth = spec.depth.trim().to_string();
    if is_read {
        if role.is_empty() {
            return Err(SciError::Invalid(
                "読む札には論文の役割を指定してください（例: exemplar）".to_string(),
            ));
        }
        if !ROLE_LABELS.iter().any(|(key, _)| *key == role) {
            return Err(SciError::Invalid(format!("知らない論文の役割です: {role}")));
        }
    }
    let depth = if depth.is_empty() {
        if is_read { "full".to_string() } else { String::new() }
    } else {
        if !DEPTH_LABELS.iter().any(|(key, _)| *key == depth) {
            return Err(SciError::Invalid(format!("知らない読む深さです: {depth}")));
        }
        depth
    };

    // 依頼文（C-05 の本文）。
    let mut description = String::new();
    description.push_str(&purpose);
    description.push_str("\n\n");
    description.push_str(&format!("- 節点: {} {}\n", fmt_ref('N', node.id), node.title));
    if let Some(question) = spec.question {
        let text = store::get_question(conn, question)?.text;
        description.push_str(&format!("- 問い: {} {}\n", fmt_ref('Q', question), text));
    }
    let expect = one_line(&spec.expect);
    if !expect.is_empty() {
        description.push_str(&format!("- 期待する成果: {expect}\n"));
    }
    if is_read {
        let questions = read_questions(conn, defs, node.id, &role)?;
        description.push_str("\n## 読むときの問い\n");
        for question in &questions {
            description.push_str(&format!("- {question}\n"));
        }
        // 末尾の改行は `build_task_body` がつけるので 1 つ落とす。
        description.pop();
    }

    // frontmatter に足すキー（C-05 の表の順）。空の値は書かない。
    let papers_text = spec
        .papers
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let assume_text = spec
        .assume
        .iter()
        .map(|(id, value)| format!("{}={}", fmt_ref('N', *id), one_line(value)))
        .collect::<Vec<_>>()
        .join("; ");
    let extra: Vec<(&str, String)> = vec![
        ("sci_node", fmt_ref('N', node.id)),
        (
            "sci_question",
            spec.question.map_or_else(String::new, |id| fmt_ref('Q', id)),
        ),
        ("sci_purpose", purpose.clone()),
        ("sci_expect", expect.clone()),
        ("sci_scope", scope.clone()),
        ("sci_assume", assume_text),
        ("sci_mode", mode.clone()),
        ("sci_papers", papers_text),
        ("sci_role", role.clone()),
        ("sci_depth", depth.clone()),
    ];

    let handoff = create_handoff_with_extra(
        db,
        roots,
        &HandoffRequest {
            block_id: &block,
            paper_ids: &spec.papers,
            description: &description,
            tag: None,
            runner: &runner,
            origin_run: spec.origin_run,
            reason: None,
        },
        &extra,
    )
    .map_err(handoff_error)?;

    // 節点と run を結ぶ。読む札はさらに論文と結ぶ。
    let actor = spec
        .origin_run
        .map_or_else(|| "user".to_string(), |id| format!("run:{id}"));
    // 結び付けの 2 つは 1 つのトランザクションにまとめる（片方だけ入るのを防ぐ）。
    // 実行フォルダと `task.md` の書き込みは入らないので、完全に 1 つにはならない。
    let tx = conn.unchecked_transaction()?;
    upsert_run_link(&tx, node.id, handoff.run_id, &purpose, &actor)?;
    if is_read {
        let paper_id = spec.papers[0];
        if find_paper_link(&tx, node.id, paper_id, &role)?.is_none() {
            store::upsert_link(
                &tx,
                &NewLink {
                    node_id: node.id,
                    target_kind: "paper",
                    target_ref: &paper_id.to_string(),
                    role: Some(&role),
                    read_state: Some("unread"),
                    reason: &purpose,
                    proposal_id: None,
                    auto: false,
                    created_by: &actor,
                },
            )?;
        }
    }
    tx.commit()?;

    Ok(CardResult {
        run_id: handoff.run_id,
        path: handoff.path,
    })
}

/// 節点と run の結び付きを足す（既是あれば理由を入れ直す）。
fn upsert_run_link(
    conn: &Connection,
    node_id: i64,
    run_id: i64,
    reason: &str,
    created_by: &str,
) -> Result<i64> {
    store::upsert_link(
        conn,
        &NewLink {
            node_id,
            target_kind: "run",
            target_ref: &run_id.to_string(),
            role: None,
            read_state: None,
            reason,
            proposal_id: None,
            auto: false,
            created_by,
        },
    )
}

/// 節点と論文の結び付き（役割付き）を探す。外したものは除く。
fn find_paper_link(
    conn: &Connection,
    node_id: i64,
    paper_id: i64,
    role: &str,
) -> Result<Option<i64>> {
    Ok(store::list_links(conn, Some(node_id), false)?
        .into_iter()
        .find(|link| {
            link.target_kind == "paper"
                && link.target_ref == paper_id.to_string()
                && link.role.as_deref() == Some(role)
        })
        .map(|link| link.id))
}

/// 読む札の節点に対応する項目ガイドのキーを探す。
///
/// 項目ならその項目キー、候補なら親の項目のキー。
fn guide_key(conn: &Connection, node_id: i64) -> Result<Option<String>> {
    let node = store::get_node(conn, node_id)?;
    if let Some(key) = &node.item_key {
        return Ok(Some(key.clone()));
    }
    for parent in store::parents_of(conn, node_id)? {
        if let Some(key) = store::get_node(conn, parent)?.item_key {
            return Ok(Some(key));
        }
    }
    Ok(None)
}

/// 読むときの問いを作る。
///
/// 1. 項目ガイドの「抜き出す項目」のうち `role` の行の項目を 1 つずつ問いにする
/// 2. 無ければその節点の答えない問いの文
/// 3. それでも無ければ節点の題から作る
fn read_questions(
    conn: &Connection,
    defs: &Defs,
    node_id: i64,
    role: &str,
) -> Result<Vec<String>> {
    if let Some(key) = guide_key(conn, node_id)? {
        if let Some(guide) = defs.guide(&key) {
            if let Some((_, fields)) = guide.extract.iter().find(|(name, _)| name == role) {
                let questions: Vec<String> = fields
                    .split(['・', '、'])
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(|part| format!("{part}は？"))
                    .collect();
                if !questions.is_empty() {
                    return Ok(questions);
                }
            }
        }
    }
    let open: Vec<String> = store::list_questions(conn, Some(node_id))?
        .into_iter()
        .filter(|question| question.state == "open")
        .map(|question| question.text)
        .collect();
    if !open.is_empty() {
        return Ok(open);
    }
    let title = store::get_node(conn, node_id)?.title;
    Ok(vec![format!(
        "この論文は「{title}」について何を言っているか"
    )])
}

// ---- 読む札の中身 -------------------------------------------------------------

/// 読む札の中身を読む（C-08 の `read` の JSON）。
///
/// # Errors
///
/// 実行が無い（[`SciError::NotFound`]）、読む札ではない（`runner` が `user` でない、
/// sci キーが無い、節点が無い）、論文が無い。
pub fn read_card(
    conn: &Connection,
    roots: &PaperRoots,
    _defs: &Defs,
    run_id: i64,
) -> Result<ReadCard> {
    let task = read_run_task(roots, run_id)?;
    if task.runner != "user" || !task.keys.is_sci() {
        return Err(SciError::Invalid("読む札ではありません".to_string()));
    }
    let node_id = task
        .keys
        .node
        .ok_or_else(|| SciError::Invalid("読む札には対象の節点がありません".to_string()))?;
    let node = store::get_node(conn, node_id)?;
    let paper_id = task
        .keys
        .papers
        .first()
        .copied()
        .ok_or_else(|| SciError::Invalid("読む札には論文がありません".to_string()))?;
    let paper = paper_of(conn, paper_id)?
        .ok_or_else(|| SciError::NotFound(format!("論文がありません: {paper_id}")))?;

    let findings = store::list_findings(conn, None, false)?
        .into_iter()
        .filter(|finding| finding.run_id == Some(run_id) && finding.source == "user")
        .map(|finding| ReadFinding {
            id: fmt_ref('F', finding.id),
            text: finding.text,
            polarity: finding.polarity,
        })
        .collect();

    Ok(ReadCard {
        run_id,
        owner: task.owner,
        node: ReadNode {
            id: fmt_ref('N', node.id),
            title: node.title,
        },
        purpose: task.keys.purpose,
        paper: ReadPaper {
            id: paper.0,
            title: paper.1,
            year: paper.2,
        },
        role: task.keys.role,
        depth: task.keys.depth,
        questions: questions_in_body(&task.text),
        findings,
    })
}

/// 読む札を終える前の確かめ（C-09 の `finish-read`）。
///
/// 知見が 1 つも無いときは `no_reflect`（理由）が要る。理由か知見があれば、
/// 論文と節点の結び付きの読み具合を `reflected` にする。
///
/// # Errors
///
/// 実行が無い、読む札ではない、節点・論文が無い、結び付きが無い、
/// 知見が無く理由も無い、理由が長すぎる。
pub fn finish_read(
    conn: &Connection,
    roots: &PaperRoots,
    run_id: i64,
    no_reflect: Option<&str>,
) -> Result<FinishRead> {
    let task = read_run_task(roots, run_id)?;
    if task.runner != "user" || !task.keys.is_sci() {
        return Err(SciError::Invalid("読む札ではありません".to_string()));
    }
    let node_id = task
        .keys
        .node
        .ok_or_else(|| SciError::Invalid("読む札には対象の節点がありません".to_string()))?;
    let paper_id = task
        .keys
        .papers
        .first()
        .copied()
        .ok_or_else(|| SciError::Invalid("読む札には論文がありません".to_string()))?;

    let findings = store::list_findings(conn, None, false)?
        .into_iter()
        .filter(|finding| finding.run_id == Some(run_id) && finding.source == "user")
        .count();

    let reason = no_reflect.map(str::trim).unwrap_or_default();
    if findings == 0 && reason.is_empty() {
        return Err(SciError::Invalid(
            "知見が 1 つもありません。反映しない理由を書いてください".to_string(),
        ));
    }
    if reason.chars().count() > MAX_NO_REFLECT {
        return Err(SciError::Invalid(format!(
            "反映しない理由は {MAX_NO_REFLECT} 字以内にしてください"
        )));
    }

    // 論文の結び付きを「反映済み」にする。理由の記録と同じトランザクションにまとめる。
    let link_id = find_paper_link(conn, node_id, paper_id, &task.keys.role)?.ok_or_else(|| {
        SciError::NotFound(format!(
            "節点 {} と論文 {} の結び付きがありません",
            fmt_ref('N', node_id),
            paper_id
        ))
    })?;
    let tx = conn.unchecked_transaction()?;
    store::set_read_state(&tx, link_id, "reflected")?;

    // 理由は節点と run の結び付きに残す。
    if !reason.is_empty() {
        upsert_run_link(
            &tx,
            node_id,
            run_id,
            &format!("反映なし: {reason}"),
            "user",
        )?;
    }
    tx.commit()?;

    Ok(FinishRead {
        run_id,
        findings,
        reflected: true,
    })
}

/// 本文の `## 読むときの問い` の下にある `- ` の行を問いとして読む。
fn questions_in_body(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            inside = trimmed == "## 読むときの問い";
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            let item = item.trim();
            if !item.is_empty() {
                out.push(item.to_string());
            }
        }
    }
    out
}

// ---- _elevation 小道具 -------------------------------------------------------

/// 値の改行を空白に置き換えて 1 行にする。
fn one_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ").trim().to_string()
}

/// task.md 発行のエラーを sci のエラーに読み替える。
///
/// `SciError` には task.md 発行の変種が無いので（`sci/mod.rs` は他作業の担当）、
/// I/O・SQLite・JSON はそのまま、残りはこの場で読める日本語 1 行にする。
fn handoff_error(error: crate::handoff::HandoffError) -> SciError {
    use crate::handoff::HandoffError;
    match error {
        HandoffError::Io(e) => SciError::Io(e),
        HandoffError::Sqlite(e) => SciError::Db(e),
        HandoffError::Json(e) => SciError::Json(e),
        HandoffError::BlockNotFound(id) => {
            SciError::Invalid(format!("ブロック定義がありません: {id}"))
        }
        other => SciError::Invalid(format!("カードを作れませんでした: {other}")),
    }
}

/// 論文の（番号・題・年）を読む。無ければ `None`。
pub(crate) fn paper_of(
    conn: &Connection,
    paper_id: i64,
) -> Result<Option<(i64, String, Option<String>)>> {
    let mut statement = conn.prepare("SELECT id, title, published_at FROM papers WHERE id = ?1")?;
    let mut rows = statement.query([paper_id])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let id: i64 = row.get(0)?;
    let title: String = row.get(1)?;
    let published: Option<String> = row.get(2)?;
    let year = published.and_then(|text| {
        let head: String = text.chars().take(4).collect();
        head.chars().all(|c| c.is_ascii_digit()).then_some(head)
    });
    Ok(Some((id, title, year)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::sci::defs;
    use std::fs;
    use std::path::Path;

    /// 一時フォルダ。Drop で消す。
    struct Fixture {
        dir: PathBuf,
        roots: PaperRoots,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    /// テスト用の置き場を作る（sai-link の DB・ブロック定義・論文つきのメモリ DB）。
    fn fixture(tag: &str, blocks: &[&str]) -> (Fixture, Database) {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-card-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".sai").join("blocks")).unwrap();
        fs::create_dir_all(dir.join(".sai").join("paper")).unwrap();
        fs::create_dir_all(dir.join(".sai").join("sai-engine")).unwrap();
        for block in blocks {
            fs::write(
                dir.join(".sai").join("blocks").join(format!("{block}.md")),
                format!("---\nid: {block}\nname: {block}\ndescription: テスト\n---\n\n# task_text\n"),
            )
            .unwrap();
        }
        let conn = rusqlite::Connection::open(dir.join(".sai").join("sai-engine").join("sai-engine.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE block_metadata (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, display_name TEXT, description TEXT, model TEXT, thinking TEXT, tools TEXT, extensions TEXT, max_turns INTEGER, timeout INTEGER, worktree TEXT, enable INTEGER, file_path TEXT NOT NULL);\n             CREATE TABLE execution_history (id INTEGER PRIMARY KEY AUTOINCREMENT, block_id INTEGER, status TEXT, user_inputs_json TEXT, execution_mode TEXT);",
        )
        .unwrap();
        drop(conn);

        let db = Database::in_memory().unwrap();
        db.connection()
            .execute("INSERT INTO papers (id, title, published_at) VALUES (37, '論文 37', '2021-05-01')", [])
            .unwrap();
        let roots = PaperRoots {
            root: dir.clone(),
            data_dir: dir.join(".sai").join("paper"),
        };
        (Fixture { dir, roots }, db)
    }

    fn defs() -> Defs {
        defs::load(Path::new("target/sci-card-defs-not-found"))
    }

    /// 項目を 1 つ足す。`key` を書くと項目ガイド机能く。
    fn add_item(conn: &Connection, title: &str, key: Option<&str>) -> i64 {
        store::insert_node(
            conn,
            &store::NewNode {
                kind: "item",
                item_key: key,
                types: &["choose".to_string()],
                title,
                body: "",
                state: "open",
                fixed: false,
                ord: 0,
                created_by: "user",
            },
        )
        .unwrap()
    }

    fn spec(node: i64) -> CardSpec {
        CardSpec {
            node,
            block: "sci-compare".to_string(),
            purpose: "候補を比べる".to_string(),
            question: None,
            expect: "比較表と推奨".to_string(),
            scope: String::new(),
            assume: Vec::new(),
            mode: String::new(),
            papers: Vec::new(),
            runner: "pi".to_string(),
            origin_run: None,
            role: String::new(),
            depth: String::new(),
        }
    }

    /// 目的が無いと作れないこと（既存の実行も残らないこと）。
    #[test]
    fn card_requires_purpose_and_node() {
        let (fix, db) = fixture("purpose", &["sci-compare"]);
        let conn = db.connection();
        let node = add_item(conn, "小目的", Some("purpose.subgoal"));

        let mut bad = spec(node);
        bad.purpose = "   ".to_string();
        let err = create_card(&db, &fix.roots, &defs(), &bad).unwrap_err();
        assert!(err.to_string().contains("目的を書いてください"), "{err}");

        // 節点が無い ID でも先に弾く（検査の 1 段目）。
        let mut no_node = spec(node);
        no_node.node = 9999;
        no_node.purpose = "候補を比べる".to_string();
        let err = create_card(&db, &fix.roots, &defs(), &no_node).unwrap_err();
        assert!(err.to_string().contains("節点"), "{err}");

        // task.md は 1 枚も作られていない。
        let outputs = fix.dir.join(".sai").join("outputs").join("todo");
        let made = fs::read_dir(&outputs)
            .map(|dirs| dirs.count())
            .unwrap_or(0);
        assert_eq!(made, 0);
    }

    /// 読む札は既定のブロックと深さが入り、問いは項目ガイドの「抜き出す項目」から作られる。
    #[test]
    fn read_card_defaults_block_and_depth() {
        let (fix, db) = fixture("read-defaults", &["sci-read"]);
        let conn = db.connection();
        let node = add_item(conn, "小目的", Some("purpose.subgoal"));

        let mut card = spec(node);
        card.block = String::new();
        card.runner = "user".to_string();
        card.papers = vec![37];
        card.role = "exemplar".to_string();
        card.expect = String::new();
        let made = create_card(&db, &fix.roots, &defs(), &card).unwrap();
        let text = fs::read_to_string(&made.path).unwrap();

        assert!(text.contains("runner: user"), "{text}");
        assert!(text.contains("block: sci-read"), "{text}");
        assert!(text.contains(&format!("sci_node: N{node}")), "{text}");
        assert!(text.contains("sci_purpose: 候補を比べる"), "{text}");
        assert!(text.contains("sci_papers: 37"), "{text}");
        assert!(text.contains("sci_role: exemplar"), "{text}");
        assert!(text.contains("sci_depth: full"), "{text}");
        assert!(!text.contains("sci_mode"), "{text}");
        assert!(text.contains("## 読むときの問い"), "{text}");
        // 目的が無い行は書かない。
        assert!(!text.contains("- 期待する成果:"), "{text}");

        let read = read_card(conn, &fix.roots, &defs(), made.run_id).unwrap();
        assert_eq!(read.paper.id, 37);
        assert_eq!(read.paper.year.as_deref(), Some("2021"));
        assert_eq!(read.role, "exemplar");
        assert_eq!(read.depth, "full");
        assert_eq!(read.node.id, format!("N{node}"));
    }

    /// 読む札の問いが項目ガイドの「抜き出す項目」になること。
    #[test]
    fn read_card_gets_questions_from_guide_extract() {
        let (fix, db) = fixture("read-questions", &["sci-read"]);
        let conn = db.connection();
        let node = add_item(conn, "小目的", Some("purpose.subgoal"));

        let mut card = spec(node);
        card.block = String::new();
        card.runner = "user".to_string();
        card.papers = vec![37];
        card.role = "exemplar".to_string();
        let made = create_card(&db, &fix.roots, &defs(), &card).unwrap();
        let text = fs::read_to_string(&made.path).unwrap();
        // 埋め込みのガイド（purpose.subgoal.md）の exemplar の行。
        assert!(
            text.contains("- その研究が 1 回の研究で何を達成したかは？"),
            "{text}"
        );

        let read = read_card(conn, &fix.roots, &defs(), made.run_id).unwrap();
        assert_eq!(read.questions.len(), 1);
        assert!(read.questions[0].ends_with("は？"), "{:?}", read.questions);
    }

    /// カードを作ると節点と run が結び付くこと。
    #[test]
    fn card_links_node_and_run() {
        let (fix, db) = fixture("links", &["sci-compare"]);
        let conn = db.connection();
        let node = add_item(conn, "小目的", Some("purpose.subgoal"));
        let made = create_card(&db, &fix.roots, &defs(), &spec(node)).unwrap();

        let links = store::list_links(conn, Some(node), false).unwrap();
        let run_link = links
            .iter()
            .find(|link| link.target_kind == "run")
            .expect("run の結び付き");
        assert_eq!(run_link.target_ref, made.run_id.to_string());
        assert_eq!(run_link.reason, "候補を比べる");
        assert_eq!(run_link.created_by, "user");
    }

    /// 読む札は節点と論文を結ぶ（`read_state = unread`）。
    #[test]
    fn read_card_links_paper_as_unread() {
        let (fix, db) = fixture("paper-link", &["sci-read"]);
        let conn = db.connection();
        let node = add_item(conn, "小目的", Some("purpose.subgoal"));
        let mut card = spec(node);
        card.block = String::new();
        card.runner = "user".to_string();
        card.papers = vec![37];
        card.role = "exemplar".to_string();
        let made = create_card(&db, &fix.roots, &defs(), &card).unwrap();

        let links = store::list_links(conn, Some(node), false).unwrap();
        let paper_link = links
            .iter()
            .find(|link| link.target_kind == "paper")
            .expect("論文の結び付き");
        assert_eq!(paper_link.target_ref, "37");
        assert_eq!(paper_link.role.as_deref(), Some("exemplar"));
        assert_eq!(paper_link.read_state.as_deref(), Some("unread"));

        // もう一度作っても増やさない。
        let again = create_card(&db, &fix.roots, &defs(), &card).unwrap();
        assert_ne!(again.run_id, made.run_id);
        let links = store::list_links(conn, Some(node), false).unwrap();
        assert_eq!(links.iter().filter(|l| l.target_kind == "paper").count(), 1);
    }

    /// 知見が無く理由も無いと終えられないこと。
    #[test]
    fn finish_read_requires_reason_without_findings() {
        let (fix, db) = fixture("finish-empty", &["sci-read"]);
        let conn = db.connection();
        let node = add_item(conn, "小目的", Some("purpose.subgoal"));
        let mut card = spec(node);
        card.block = String::new();
        card.runner = "user".to_string();
        card.papers = vec![37];
        card.role = "exemplar".to_string();
        let made = create_card(&db, &fix.roots, &defs(), &card).unwrap();

        let err = finish_read(conn, &fix.roots, made.run_id, None).unwrap_err();
        assert!(
            err.to_string().contains("反映しない理由を書いてください"),
            "{err}"
        );
    }

    /// 知見を 1 つ入れた読む札は終えられること。理由だけで終えたときも行が `reflected` になること。
    #[test]
    fn finish_read_marks_reflected() {
        let (fix, db) = fixture("finish", &["sci-read"]);
        let conn = db.connection();
        let node = add_item(conn, "小目的", Some("purpose.subgoal"));
        let mut card = spec(node);
        card.block = String::new();
        card.runner = "user".to_string();
        card.papers = vec![37];
        card.role = "exemplar".to_string();
        let made = create_card(&db, &fix.roots, &defs(), &card).unwrap();
        store::insert_finding(
            conn,
            &store::NewFinding {
                node_id: node,
                question_id: None,
                text: "データ名は XYZ",
                cites: &["#37 §4.2".to_string()],
                polarity: "support",
                level: Some(2),
                source: "user",
                report_id: None,
                paper_id: Some(37),
                run_id: Some(made.run_id),
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .unwrap();

        let read = read_card(conn, &fix.roots, &defs(), made.run_id).unwrap();
        assert_eq!(read.findings.len(), 1);
        assert_eq!(read.findings[0].polarity, "support");

        let done = finish_read(conn, &fix.roots, made.run_id, None).unwrap();
        assert_eq!(done.run_id, made.run_id);
        assert_eq!(done.findings, 1);
        assert!(done.reflected);

        let links = store::list_links(conn, Some(node), false).unwrap();
        let paper_link = links.iter().find(|l| l.target_kind == "paper").unwrap();
        assert_eq!(paper_link.read_state.as_deref(), Some("reflected"));

        // 理由だけで終える読む札も reflected になり、理由は run の結び付きに残る。
        let mut other = card.clone();
        other.purpose = "関係なしの確認".to_string();
        let second = create_card(&db, &fix.roots, &defs(), &other).unwrap();
        let done = finish_read(conn, &fix.roots, second.run_id, Some("_once がある")).unwrap();
        assert_eq!(done.findings, 0);
        assert!(done.reflected);
        let links = store::list_links(conn, Some(node), false).unwrap();
        let run_link = links
            .iter()
            .find(|l| l.target_kind == "run" && l.target_ref == second.run_id.to_string())
            .unwrap();
        assert_eq!(run_link.reason, "反映なし: _once がある");
    }

    /// Web は知見の `run` と `paper` を数で送る。数でも読む札の答えになり、札を終えられること。
    #[test]
    fn finish_read_accepts_finding_with_numeric_run() {
        let (fix, db) = fixture("finish-numeric", &["sci-read"]);
        let conn = db.connection();
        let node = add_item(conn, "小目的", Some("purpose.subgoal"));
        let mut card = spec(node);
        card.block = String::new();
        card.runner = "user".to_string();
        card.papers = vec![37];
        card.role = "exemplar".to_string();
        let made = create_card(&db, &fix.roots, &defs(), &card).unwrap();

        let op = serde_json::json!({
            "op": "add_finding", "node": format!("N{node}"), "text": "データ名は XYZ",
            "cites": ["#37 §4.2"], "polarity": "support", "paper": 37, "run": made.run_id,
        });
        crate::sci::ops::apply_user(conn, &fix.roots, &defs(), &op).unwrap();

        let done = finish_read(conn, &fix.roots, made.run_id, None).unwrap();
        assert_eq!(done.findings, 1);
    }
}
