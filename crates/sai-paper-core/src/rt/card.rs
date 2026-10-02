//! 新しい木のカード（task.md の発行）と、runner=user の読む札の中身。
//!
//! 読む札は特別な釦ではなく、`runner: user` のカードの 1 つ。論文は 1 本で、
//! F を付ける先の O（か Q）を `node` に持つ。LINK は作らない。run と Node の結び付きは
//! task.md の `sci_node` だけで表す（[`super::action::sci_runs`] が読む）。DB には何も書かない。

use std::path::PathBuf;

use rusqlite::Connection;
use serde_json::{Value, json};

use super::defs::Defs;
use super::{Kind, Result, RtError, action, compute, fmt_id, parse_id, store};
use crate::database::Database;
use crate::handoff::{HandoffError, HandoffRequest, create_handoff_with_extra};
use crate::paths::{PaperRoots, paper_file_names, paper_folder};

const RUNNERS: [&str; 3] = ["pi", "cc", "user"];
const EXECUTERS: [&str; 2] = ["human", "professor"];
const MODES: [&str; 2] = ["purpose_first", "blind_first"];
const DEPTHS: [&str; 3] = ["abstract", "figures", "full"];
const READ_BLOCK: &str = "sci-read";
const MAX_ASSUME: usize = 5;
const MAX_PAPERS: usize = 20;
const MAX_PURPOSE: usize = 300;
const Q_HEADING: &str = "## 読むときの Q";
const EXTRACT_HEADING: &str = "## 抜き出すもの";

/// カードの値（C-CARD）。
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct CardSpec {
    /// Q・O・F・E の ID（必須）。
    pub node: String,
    /// 必須。runner が user で空なら `sci-read`。
    pub block: String,
    /// 必須。1 行、300 字まで。
    pub purpose: String,
    #[serde(default)]
    pub expect: String,
    /// 空ならブロック定義の `sci_scope`、それも無ければ `path`。
    #[serde(default)]
    pub scope: String,
    /// `Q9=SST-2 を使う（仮）` の形。5 つまで。
    #[serde(default)]
    pub assume: Vec<String>,
    /// 空 / purpose_first / blind_first。
    #[serde(default)]
    pub mode: String,
    /// 20 本まで。user はちょうど 1 本。
    #[serde(default)]
    pub papers: Vec<i64>,
    /// 空は pi。
    #[serde(default)]
    pub runner: String,
    /// user のときだけ。空は full。
    #[serde(default)]
    pub depth: String,
    #[serde(default)]
    pub origin_run: Option<i64>,
    /// waggle トークン。
    #[serde(default)]
    pub inputs: Vec<String>,
    /// 空は human。
    #[serde(default)]
    pub executer: String,
}

/// カードを作った結果。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CardResult {
    pub run_id: i64,
    pub path: PathBuf,
}

/// 読む札の中身。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ReadCard {
    pub run_id: i64,
    pub owner: String,
    /// `{"id","kind","text"}`
    pub node: Value,
    pub purpose: String,
    /// `{"id","title","year"}`
    pub paper: Value,
    pub depth: String,
    pub questions: Vec<String>,
    pub extract: Vec<String>,
    /// `{"id","parent","text","cites"}`
    pub findings: Vec<Value>,
    pub note: bool,
    pub debts: Vec<String>,
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(RtError::Invalid(message.into()))
}

fn one_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ").trim().to_string()
}

fn foreign(label: &str, text: &str) -> Result<()> {
    match crate::textcheck::foreign_message(text) {
        Some(message) => invalid(format!("{label}に{message}")),
        None => Ok(()),
    }
}

/// `^[QOFE][1-9][0-9]{0,8}=.+$`。
fn check_assume(item: &str) -> Result<String> {
    let item = one_line(item);
    match item.split_once('=') {
        Some((id, value)) if parse_id(id).is_ok() && !value.trim().is_empty() => Ok(item),
        _ => invalid(format!("仮置きの形が正しくありません（例: Q9=SST-2 を使う（仮））: {item}")),
    }
}

fn check_token(raw: &str) -> Result<String> {
    let token = raw.trim();
    let bare = token.strip_prefix("waggle:").unwrap_or(token);
    let ok = (4..=64).contains(&bare.chars().count())
        && bare.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok { Ok(bare.to_string()) } else { invalid(format!("文書のトークンの形が違います: {raw}")) }
}

fn handoff_error(error: HandoffError) -> RtError {
    match error {
        HandoffError::Io(e) => RtError::Io(e),
        HandoffError::Sqlite(e) => RtError::Db(e),
        HandoffError::Json(e) => RtError::Json(e),
        HandoffError::BlockNotFound(id) => RtError::Invalid(format!("ブロック定義がありません: {id}")),
        other => RtError::Invalid(format!("カードを作れませんでした: {other}")),
    }
}

/// 読むときの Q の対象になる Q（O なら親の Q）。
fn target_q(conn: &Connection, node: &store::NodeRow) -> Result<Option<store::NodeRow>> {
    Ok(match node.kind {
        Kind::Q => Some(node.clone()),
        Kind::O => match node.parent_id {
            Some(parent) => Some(store::get_node(conn, parent)?),
            None => None,
        },
        _ => None,
    })
}

/// 検査してカードの task.md を作る。DB（`rt_*`）には何も書かない。
///
/// # Errors
/// 検査に通らない値は `Invalid`。ブロック定義や論文が無いときも `Invalid`。
pub fn create_card(db: &Database, roots: &PaperRoots, defs: &Defs, spec: &CardSpec) -> Result<CardResult> {
    let conn = db.connection();

    let (kind, node_id) = parse_id(spec.node.trim())?;
    let node = match store::get_node_of(conn, kind, node_id) {
        Ok(node) if node.removed_at.is_none() => node,
        _ => return invalid(format!("対象の Node がありません: {}", spec.node.trim())),
    };

    let runner = if spec.runner.trim().is_empty() { "pi" } else { spec.runner.trim() };
    if !RUNNERS.contains(&runner) {
        return invalid(format!("知らない実行者です: {runner}（pi / cc / user）"));
    }
    let executer = if spec.executer.trim().is_empty() { "human" } else { spec.executer.trim() };
    if !EXECUTERS.contains(&executer) {
        return invalid(format!("知らない起動者です: {executer}（human / professor）"));
    }
    if executer == "professor" && runner != "pi" {
        return invalid("professor が起こすカードは runner が pi だけです（子の run は pi で動くため）");
    }
    let is_read = runner == "user";

    let block = match spec.block.trim() {
        "" if is_read => READ_BLOCK,
        other => other,
    };
    if !block.starts_with("sci-") {
        return invalid("sci のブロックを指定してください（sci- で始まります）");
    }
    if action::block_def_paths(roots, block).iter().all(|path| !path.is_file()) {
        return invalid(format!("ブロック定義がありません: {block}"));
    }

    let purpose = one_line(&spec.purpose);
    if purpose.is_empty() {
        return invalid("目的を書いてください（カードは目的が要ります）");
    }
    if purpose.chars().count() > MAX_PURPOSE {
        return invalid(format!("目的は {MAX_PURPOSE} 字以内にしてください（今は {} 字）", purpose.chars().count()));
    }
    if spec.expect.contains(['\r', '\n']) {
        return invalid("期待する成果は 1 行にしてください");
    }
    let expect = spec.expect.trim().to_string();

    let scope = match spec.scope.trim() {
        "" => action::block_frontmatter(roots, block)
            .and_then(|fm| fm.into_iter().find(|(k, _)| k == "sci_scope").map(|(_, v)| v.trim().to_string()))
            .filter(|v| compute::SCOPES.contains(&v.as_str()))
            .unwrap_or_else(|| "path".to_string()),
        given if compute::SCOPES.contains(&given) => given.to_string(),
        other => return invalid(format!("知らない範囲です: {other}")),
    };
    let mode = spec.mode.trim();
    if !mode.is_empty() && !MODES.contains(&mode) {
        return invalid(format!("知らない読み方です: {mode}（purpose_first / blind_first）"));
    }
    let depth = match spec.depth.trim() {
        "" if is_read => "full",
        "" => "",
        d if DEPTHS.contains(&d) => d,
        other => return invalid(format!("知らない深さです: {other}（abstract / figures / full）")),
    };

    if spec.assume.len() > MAX_ASSUME {
        return invalid(format!("仮置きは {MAX_ASSUME} 個までです"));
    }
    let assume = spec.assume.iter().map(|a| check_assume(a)).collect::<Result<Vec<_>>>()?;

    if spec.papers.len() > MAX_PAPERS {
        return invalid(format!("対象論文は {MAX_PAPERS} 本までです"));
    }
    if is_read {
        if spec.papers.len() != 1 {
            return invalid("読む札には論文をちょうど 1 本指定してください");
        }
        if !matches!(kind, Kind::O | Kind::Q) {
            return invalid("読む札の Node は O か Q にしてください（F を付ける先が要ります）");
        }
    }
    for id in &spec.papers {
        if db.library_paper_by_id(*id).ok().flatten().is_none() {
            return invalid(format!("論文がありません: {id}"));
        }
    }

    foreign("目的", &purpose)?;
    foreign("期待する成果", &expect)?;
    for item in &assume {
        foreign("仮置き", item)?;
    }
    let inputs = spec.inputs.iter().map(|t| check_token(t)).collect::<Result<Vec<_>>>()?;

    // 依頼文。
    let mut body = format!("{purpose}\n\n- NODE: {} {}\n", fmt_id(kind, node.id), node.text);
    if !expect.is_empty() {
        body.push_str(&format!("- 期待する成果: {expect}\n"));
    }
    if is_read {
        let q = target_q(conn, &node)?;
        let guide = q.as_ref().and_then(|q| q.guide_key.as_deref()).and_then(|key| defs.guide(key));
        body.push_str(&format!("\n{Q_HEADING}\n"));
        if let Some(q) = &q {
            body.push_str(&format!("- {}\n", q.text));
        }
        for line in guide.map_or(&[][..], |g| &g.child_candidates[..]) {
            body.push_str(&format!("- {line}\n"));
        }
        if let Some(guide) = guide.filter(|g| !g.extract.is_empty()) {
            body.push_str(&format!("\n{EXTRACT_HEADING}\n"));
            for line in &guide.extract {
                body.push_str(&format!("- {line}\n"));
            }
        }
    }
    if !inputs.is_empty() {
        body.push_str("\n## 受け取る文書\n");
        for token in &inputs {
            body.push_str(&format!("- waggle:{token}\n"));
        }
    }
    // 末尾の改行は `create_handoff_with_extra` 側がつける。
    let description = body.trim_end().to_string();

    let papers_text = spec.papers.iter().map(i64::to_string).collect::<Vec<_>>().join(", ");
    let extra: Vec<(&str, String)> = vec![
        ("executer", executer.to_string()),
        ("sci_node", fmt_id(kind, node.id)),
        ("sci_purpose", purpose.clone()),
        ("sci_expect", expect),
        ("sci_scope", scope),
        ("sci_assume", assume.join("; ")),
        ("sci_mode", mode.to_string()),
        ("sci_papers", papers_text),
        ("sci_depth", depth.to_string()),
    ];
    let handoff = create_handoff_with_extra(
        db,
        roots,
        &HandoffRequest {
            block_id: block,
            paper_ids: &spec.papers,
            description: &description,
            tag: None,
            runner,
            origin_run: spec.origin_run,
            reason: None,
        },
        &extra,
    )
    .map_err(handoff_error)?;
    Ok(CardResult { run_id: handoff.run_id, path: handoff.path })
}

/// 見出し `heading` の下の `- ` の行を抜く（次の `## ` まで）。
fn section_lines(text: &str, heading: &str) -> Vec<String> {
    text.lines()
        .skip_while(|line| line.trim_end() != heading)
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .filter_map(|line| line.strip_prefix("- "))
        .map(|line| line.trim().to_string())
        .collect()
}

/// 読む札（runner=user のカード）の中身を返す。
///
/// # Errors
/// run が無ければ `NotFound`。読む札でなければ `Invalid`。
pub fn read_card(conn: &Connection, roots: &PaperRoots, defs: &Defs, run_id: i64) -> Result<ReadCard> {
    let _ = defs;
    let run = action::sci_runs(roots)
        .into_iter()
        .find(|r| r.run_id == run_id)
        .ok_or_else(|| RtError::NotFound(format!("実行 {run_id} が見つかりません")))?;
    let not_read = || RtError::Invalid("読む札ではありません".to_string());
    if run.runner != "user" {
        return Err(not_read());
    }
    let (Some((kind, node_id)), Some(&paper_id)) = (run.keys.node, run.keys.papers.first()) else {
        return Err(not_read());
    };
    let text = std::fs::read_to_string(run.path.join("task.md"))?;
    let node = store::get_node(conn, node_id)?;

    let (title, published): (String, Option<String>) = conn
        .query_row("SELECT title, published_at FROM papers WHERE id = ?1", [paper_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|_| RtError::NotFound(format!("論文がありません: {paper_id}")))?;
    let year = published
        .map(|p| p.chars().take(4).collect::<String>())
        .filter(|y| y.chars().all(|c| c.is_ascii_digit()));

    let findings = store::list_nodes(conn, false)?
        .into_iter()
        .filter(|n| n.kind == Kind::F && n.run_id == Some(run_id))
        .map(|n| {
            json!({"id": fmt_id(n.kind, n.id), "parent": n.parent_id.map(|p| fmt_id(Kind::O, p)), "text": n.text, "cites": n.cites})
        })
        .collect();
    let note = paper_folder(&roots.data_dir, paper_id, &title).join(paper_file_names::NOTE).is_file();
    let debts = action::debts(conn, roots)?
        .into_iter()
        .filter(|d| d.run_id == run_id)
        .map(|d| d.message)
        .collect();

    Ok(ReadCard {
        run_id,
        owner: run.owner,
        node: json!({"id": fmt_id(kind, node.id), "kind": kind.as_str(), "text": node.text}),
        purpose: run.keys.purpose,
        paper: json!({"id": paper_id, "title": title, "year": year}),
        depth: run.keys.depth,
        questions: section_lines(&text, Q_HEADING),
        extract: section_lines(&text, EXTRACT_HEADING),
        findings,
        note,
        debts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rt::defs::GuideDef;
    use crate::rt::task_keys;
    use std::fs;

    struct Env {
        dir: PathBuf,
        roots: PaperRoots,
        db: Database,
        defs: Defs,
    }
    impl Drop for Env {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    /// Q1（ガイド付き）・O2・F3（run 99 で付いた）・論文 37 のある一時環境。
    fn env(tag: &str) -> Env {
        let dir = std::env::temp_dir().join(format!("sai-rt-card-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let data_dir = dir.join(".sai/paper");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(dir.join(".sai/blocks")).unwrap();
        fs::create_dir_all(dir.join(".sai/sai-engine")).unwrap();
        for b in ["sci-ideate", "sci-read"] {
            fs::write(
                dir.join(format!(".sai/blocks/{b}.md")),
                format!("---\nid: {b}\nname: {b}\ndescription: テスト\n---\n\n# task_text\n"),
            )
            .unwrap();
        }
        let conn = Connection::open(dir.join(".sai/sai-engine/sai-engine.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE block_metadata (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, display_name TEXT, description TEXT, model TEXT, thinking TEXT, tools TEXT, extensions TEXT, max_turns INTEGER, timeout INTEGER, worktree TEXT, enable INTEGER, file_path TEXT NOT NULL);\n CREATE TABLE execution_history (id INTEGER PRIMARY KEY AUTOINCREMENT, block_id INTEGER, status TEXT, user_inputs_json TEXT, execution_mode TEXT);",
        )
        .unwrap();
        drop(conn);
        let db = Database::open(&data_dir.join("papr.db")).unwrap();
        db.connection()
            .execute("INSERT INTO papers (id, title, published_at) VALUES (37, '論文 37', '2021-05-01')", [])
            .unwrap();
        let c = db.connection();
        let mut n = store::NewNode {
            kind: Kind::Q, parent_id: None, guide_key: Some("data"), text: "データの量は？", body: "", answer_kind: None,
            cites: &[], paper_id: None, run_id: None, report_id: None, mlflow_ref: None, proposal_id: None, created_by: "user",
        };
        store::insert_node(c, &n).unwrap();
        n.kind = Kind::O;
        n.guide_key = None;
        n.parent_id = Some(1);
        n.answer_kind = Some("answer");
        n.text = "1 万件";
        store::insert_node(c, &n).unwrap();
        n.kind = Kind::F;
        n.parent_id = Some(2);
        n.answer_kind = None;
        n.text = "論文は 1 万件と述べる";
        n.run_id = Some(99);
        store::insert_node(c, &n).unwrap();
        let defs = Defs {
            guides: vec![GuideDef {
                key: "data".into(), name: "データ".into(), explain: String::new(), question: String::new(),
                child_candidates: vec!["規模は？".into()], extract: vec!["件数".into(), "出所".into()],
                pitfalls: vec![], target: 0,
            }],
            ..Defs::default()
        };
        Env { roots: PaperRoots { root: dir.clone(), data_dir }, dir, db, defs }
    }

    fn spec() -> CardSpec {
        CardSpec {
            node: "Q1".into(), block: "sci-ideate".into(), purpose: "答えの候補を出す".into(), expect: "O を 2 つ".into(),
            scope: "path".into(), assume: vec!["Q9=SST-2 を使う（仮）".into()], mode: "purpose_first".into(),
            papers: vec![37], runner: "pi".into(), origin_run: Some(120), ..CardSpec::default()
        }
    }

    fn err(e: &Env, s: &CardSpec) -> String {
        create_card(&e.db, &e.roots, &e.defs, s).unwrap_err().to_string()
    }

    fn node_rows(e: &Env) -> usize {
        store::list_nodes(e.db.connection(), true).unwrap().len()
    }

    #[test]
    fn creates_task_md_with_keys_and_writes_no_rows() {
        let e = env("ok");
        let before = node_rows(&e);
        let r = create_card(&e.db, &e.roots, &e.defs, &spec()).unwrap();
        let text = fs::read_to_string(&r.path).unwrap();
        for key in ["sci_node: Q1", "sci_scope: path", "sci_assume: Q9=SST-2 を使う（仮）", "sci_mode: purpose_first", "sci_papers: 37", "sci_expect: O を 2 つ"] {
            assert!(text.contains(key), "{key}\n{text}");
        }
        assert!(!text.contains("sci_question") && !text.contains("sci_role") && !text.contains("sci_depth"));
        let k = task_keys(&text);
        assert_eq!(k.node, Some((Kind::Q, 1)));
        assert_eq!(k.papers, vec![37]);
        assert_eq!(k.assume.len(), 1);
        assert_eq!(node_rows(&e), before);
    }

    #[test]
    fn rejects_bad_values() {
        let e = env("bad");
        let mut s = spec();
        s.node = "Q77".into();
        assert!(err(&e, &s).contains("Node がありません"));
        let mut s = spec();
        s.block = "sci-nothing".into();
        assert!(err(&e, &s).contains("ブロック定義がありません"));
        let mut s = spec();
        s.scope = "all".into();
        assert!(err(&e, &s).contains("範囲"));
        let mut s = spec();
        s.runner = "user".into();
        s.papers = vec![37, 37];
        assert!(err(&e, &s).contains("ちょうど 1 本"));
        let mut s = spec();
        s.runner = "user".into();
        s.node = "F3".into();
        assert!(err(&e, &s).contains("O か Q"));
        let mut s = spec();
        s.purpose = "한글".into();
        assert!(err(&e, &s).contains("ハングル"));
        let mut s = spec();
        s.assume = vec!["N9=x".into()];
        assert!(err(&e, &s).contains("仮置き"));
    }

    #[test]
    fn read_card_roundtrip() {
        let e = env("read");
        let mut s = spec();
        s.node = "O2".into();
        s.runner = "user".into();
        s.block = String::new();
        s.mode = String::new();
        let r = create_card(&e.db, &e.roots, &e.defs, &s).unwrap();
        let text = fs::read_to_string(&r.path).unwrap();
        assert!(text.contains("## 読むときの Q") && text.contains("- 件数") && text.contains("sci_depth: full"));
        // 読む札の run に F を付け替えて、note も置く。
        e.db.connection().execute("UPDATE rt_nodes SET run_id = ?1 WHERE id = 3", [r.run_id]).unwrap();
        let card = read_card(e.db.connection(), &e.roots, &e.defs, r.run_id).unwrap();
        assert_eq!(card.questions, vec!["データの量は？", "規模は？"]);
        assert_eq!(card.extract, vec!["件数", "出所"]);
        assert_eq!(card.findings.len(), 1);
        assert_eq!(card.paper["year"], "2021");
        assert!(!card.note);
        // 読む札でないカードは弾く。
        let r2 = create_card(&e.db, &e.roots, &e.defs, &spec()).unwrap();
        assert!(read_card(e.db.connection(), &e.roots, &e.defs, r2.run_id).unwrap_err().to_string().contains("読む札ではありません"));
    }
}
