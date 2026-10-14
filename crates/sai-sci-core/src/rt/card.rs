//! 新しい research-tree の CARD（task.md の発行）と、runner=user の READ_CARD の中身。
//!
//! READ_CARD は特別な釦ではなく、`runner: user` の CARD の 1 つ。論文は 1 本で、
//! F を付ける先の A（か Q）を `node` に持つ。LINK は作らない。run と Node の結び付きは
//! task.md の `sci_node` だけで表す（[`super::action::sci_runs`] が読む）。DB には何も書かない。

use std::path::PathBuf;

use rusqlite::Connection;
use serde_json::{Value, json};

use super::defs::Defs;
use super::{Kind, Result, RtError, action, fmt_id, parse_id, store};
use sai_paper_core::database::Database;
use crate::handoff::{HandoffError, HandoffRequest, create_handoff_with_extra};
use sai_paper_core::paths::{PaperRoots, paper_file_names, paper_folder};

const RUNNERS: [&str; 2] = ["cc", "user"];
const MODELS: [&str; 4] = ["opus", "sonnet", "haiku", "inherit"];
const THINKINGS: [&str; 6] = ["none", "low", "medium", "high", "xhigh", "max"];
const MODES: [&str; 2] = ["purpose_first", "blind_first"];
const DEPTHS: [&str; 3] = ["abstract", "figures", "full"];
const READ_BLOCK: &str = "lab-read";
const MAX_PAPERS: usize = 20;
const MAX_PURPOSE: usize = 300;
const VIEW_HEADING: &str = "## 読むときの観点";
const EXTRACT_HEADING: &str = "## 抜き出すもの";

/// CARD の値（C-CARD）。
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct CardSpec {
    /// A・F・E・S・D の ID（Q は A の別名）。空は木の全体（廃止した `SEED` は断る）。
    pub node: String,
    /// 必須。runner が user で空なら `lab-read`。
    pub block: String,
    /// 必須。1 行、300 字まで。
    pub purpose: String,
    #[serde(default)]
    pub expect: String,
    /// 空 / purpose_first / blind_first。
    #[serde(default)]
    pub mode: String,
    /// 20 本まで。user はちょうど 1 本。
    #[serde(default)]
    pub papers: Vec<i64>,
    /// 空は cc。`pi` は廃止（断る）。
    #[serde(default)]
    pub runner: String,
    /// 空は settings.json の既定（sai-link が入れる）。opus / sonnet / haiku / inherit か claude で始まる名前。
    #[serde(default)]
    pub model: String,
    /// 空は settings.json の既定（sai-link が入れる）。none / low / medium / high / xhigh / max。
    #[serde(default)]
    pub thinking: String,
    /// user のときだけ。空は full。
    #[serde(default)]
    pub depth: String,
    #[serde(default)]
    pub origin_run: Option<i64>,
    /// waggle トークン。
    #[serde(default)]
    pub inputs: Vec<String>,
    /// 足す子の型（Q / A / F。空は未指定）。C7 の組だけ通す。
    #[serde(default)]
    pub child: String,
}

/// CARD を作った結果。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CardResult {
    pub run_id: i64,
    pub path: PathBuf,
}

/// READ_CARD の中身。
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
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(RtError::Invalid(message.into()))
}

fn one_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ").trim().to_string()
}

fn foreign(label: &str, text: &str) -> Result<()> {
    match sai_paper_core::textcheck::foreign_message(text) {
        Some(message) => invalid(format!("{label}に{message}")),
        None => Ok(()),
    }
}

fn check_token(raw: &str) -> Result<String> {
    let token = raw.trim();
    let bare = token.strip_prefix("waggle:").unwrap_or(token);
    let ok = (4..=64).contains(&bare.chars().count())
        && bare.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok { Ok(bare.to_string()) } else { invalid(format!("文書のトークンの形が違います: {raw}")) }
}

/// 足す子の型を検査する（C7・K2）。軸の A の下は A、軸でない A の下は A か F。空は未指定として通す。
fn check_child(target: Option<&store::NodeRow>, child: &str) -> Result<String> {
    let child = child.trim();
    if child.is_empty() {
        return Ok(String::new());
    }
    let allowed: &[&str] = match target {
        Some(n) if store::is_axis(n) => &["A"],
        Some(n) if n.kind == Kind::A => &["A", "F"],
        _ => return invalid("足す子の型は A の Node にだけ指定できます"),
    };
    if allowed.contains(&child) {
        Ok(child.to_string())
    } else {
        invalid(format!("足す子の型が違います: {child}（この Node では {}）", allowed.join(" / ")))
    }
}

fn handoff_error(error: HandoffError) -> RtError {
    match error {
        HandoffError::Io(e) => RtError::Io(e),
        HandoffError::Sqlite(e) => RtError::Db(e),
        HandoffError::Json(e) => RtError::Json(e),
        HandoffError::BlockNotFound(id) => RtError::Invalid(format!("ブロック定義がありません: {id}")),
        // sai-link の 3（見つからない）はそのまま伝える。
        HandoffError::Link(error) => {
            let message = format!("CARD を作れませんでした: {error}");
            if error.code == 3 {
                RtError::NotFound(message)
            } else {
                RtError::Invalid(message)
            }
        }
        other => RtError::Invalid(format!("CARD を作れませんでした: {other}")),
    }
}

/// 読むときの観点の対象になる軸の A（軸の A なら自分、軸でない A なら親）。
fn target_axis(conn: &Connection, node: &store::NodeRow) -> Result<Option<store::NodeRow>> {
    Ok(match node.kind {
        Kind::A if store::is_axis(node) => Some(node.clone()),
        Kind::A => match node.parent_id {
            Some(parent) => Some(store::get_node(conn, parent)?),
            None => None,
        },
        _ => None,
    })
}

/// 検査して CARD の task.md を作る。DB（`rt_*`）には何も書かない。
///
/// # Errors
/// 検査に通らない値は `Invalid`。ブロック定義や論文が無いときも `Invalid`。
pub fn create_card(db: &Database, roots: &PaperRoots, defs: &Defs, spec: &CardSpec) -> Result<CardResult> {
    let conn = db.connection();

    // 空は木の全体（node は無し）。廃止した SEED は断る。
    let requested = spec.node.trim();
    if super::is_retired_seed(requested) {
        return invalid(super::RETIRED_SEED_MSG);
    }
    let node = if requested.is_empty() {
        None
    } else {
        let (kind, node_id) = parse_id(requested)?;
        match store::get_node_of(conn, kind, node_id) {
            Ok(node) if node.removed_at.is_none() => Some(node),
            _ => return invalid(format!("対象の Node がありません: {requested}")),
        }
    };

    let runner = if spec.runner.trim().is_empty() { "cc" } else { spec.runner.trim() };
    if runner == "pi" {
        return invalid("runner の pi は廃止しました。cc を使ってください");
    }
    if !RUNNERS.contains(&runner) {
        return invalid(format!("知らない実行者です: {runner}（cc / user）"));
    }
    let model = spec.model.trim();
    if !model.is_empty() && !MODELS.contains(&model) && !model.starts_with("claude") {
        return invalid("model は opus / sonnet / haiku / inherit のどれか、または claude で始まる名前です");
    }
    let thinking = spec.thinking.trim();
    if !thinking.is_empty() && !THINKINGS.contains(&thinking) {
        return invalid("thinking は none / low / medium / high / xhigh / max のどれかです");
    }
    let is_read = runner == "user";

    let block = match spec.block.trim() {
        "" if is_read => READ_BLOCK,
        other => other,
    };
    if !(block.starts_with("sci-") || block.starts_with("lab-")) {
        return invalid("sci のブロックを指定してください（sci- か lab- で始まります）");
    }
    if action::block_def_paths(roots, block).iter().all(|path| !path.is_file()) {
        return invalid(format!("ブロック定義がありません: {block}"));
    }
    // node の型のブロックは対象の Node が要る（木の全体は manage と support だけ）。
    let sci_type = action::block_sci_type(roots, block)
        .filter(|t| action::SCI_TYPES.contains(&t.as_str()))
        .unwrap_or_else(|| "node".to_string());
    if node.is_none() && sci_type == "node" {
        return invalid(format!("{block} は対象の Node が要ります（--node を指定してください）"));
    }

    let purpose = one_line(&spec.purpose);
    if purpose.is_empty() {
        return invalid("目的を書いてください（CARD は目的が要ります）");
    }
    if purpose.chars().count() > MAX_PURPOSE {
        return invalid(format!("目的は {MAX_PURPOSE} 字以内にしてください（今は {} 字）", purpose.chars().count()));
    }
    if spec.expect.contains(['\r', '\n']) {
        return invalid("期待する成果は 1 行にしてください");
    }
    let expect = spec.expect.trim().to_string();

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

    if spec.papers.len() > MAX_PAPERS {
        return invalid(format!("対象論文は {MAX_PAPERS} 本までです"));
    }
    if is_read {
        if spec.papers.len() != 1 {
            return invalid("READ_CARD には論文をちょうど 1 本指定してください");
        }
        if !node.as_ref().is_some_and(|n| n.kind == Kind::A) {
            return invalid("READ_CARD の Node は A にしてください（F を付ける先が要ります）");
        }
    }
    for id in &spec.papers {
        if db.library_paper_by_id(*id).ok().flatten().is_none() {
            return invalid(format!("論文がありません: {id}"));
        }
    }

    foreign("目的", &purpose)?;
    foreign("期待する成果", &expect)?;
    let child = check_child(node.as_ref(), &spec.child)?;
    let inputs = spec.inputs.iter().map(|t| check_token(t)).collect::<Result<Vec<_>>>()?;

    // 依頼文。木の全体のときは `- NODE: （木の全体）`。
    let node_label = match &node {
        Some(n) => format!("{} {}", fmt_id(n.kind, n.id), n.text),
        None => "（木の全体）".to_string(),
    };
    let mut body = format!("{purpose}\n\n- NODE: {node_label}\n");
    if !expect.is_empty() {
        body.push_str(&format!("- 期待する成果: {expect}\n"));
    }
    if is_read {
        let read_node = node.as_ref().expect("READ_CARD は Node が必須");
        let q = target_axis(conn, read_node)?;
        let guide = q.as_ref().and_then(|q| q.guide_key.as_deref()).and_then(|key| defs.guide(key));
        body.push_str(&format!("\n{VIEW_HEADING}\n"));
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
    // 木の全体のときは空の値になり、task.md に書かない（`sci_node` が無い run は対象の Node が無い）。
    let sci_node = node.as_ref().map_or_else(String::new, |n| fmt_id(n.kind, n.id));
    let mut extra: Vec<(&str, String)> = vec![
        ("sci_node", sci_node),
        ("sci_purpose", purpose.clone()),
        ("sci_expect", expect),
        ("sci_mode", mode.to_string()),
        ("sci_papers", papers_text),
        ("sci_depth", depth.to_string()),
    ];
    // 空の足す子の型は書かない（task.md に行を出さない）。
    if !child.is_empty() {
        extra.push(("sci_child", child));
    }
    let handoff = create_handoff_with_extra(
        db,
        roots,
        &HandoffRequest {
            block_id: block,
            paper_ids: &spec.papers,
            description: &description,
            tag: None,
            runner,
            model,
            thinking,
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

/// READ_CARD（runner=user の CARD）の中身を返す。
///
/// # Errors
/// run が無ければ `NotFound`。READ_CARD でなければ `Invalid`。
pub fn read_card(conn: &Connection, roots: &PaperRoots, defs: &Defs, run_id: i64) -> Result<ReadCard> {
    let _ = defs;
    let run = action::sci_runs(roots)
        .into_iter()
        .find(|r| r.run_id == run_id)
        .ok_or_else(|| RtError::NotFound(format!("実行 {run_id} が見つかりません")))?;
    let not_read = || RtError::Invalid("READ_CARD ではありません".to_string());
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
            json!({"id": fmt_id(n.kind, n.id), "parent": n.parent_id.map(|p| fmt_id(Kind::A, p)), "text": n.text, "cites": n.cites})
        })
        .collect();
    let note = paper_folder(&roots.data_dir, paper_id, &title).join(paper_file_names::NOTE).is_file();

    Ok(ReadCard {
        run_id,
        owner: run.owner,
        node: json!({"id": fmt_id(kind, node.id), "kind": kind.as_str(), "text": node.text}),
        purpose: run.keys.purpose,
        paper: json!({"id": paper_id, "title": title, "year": year}),
        depth: run.keys.depth,
        questions: section_lines(&text, VIEW_HEADING),
        extract: section_lines(&text, EXTRACT_HEADING),
        findings,
        note,
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

    /// Q1（GUIDE 付き）・A2・F3（run 99 で付いた）・論文 37 のある一時環境。
    fn env(tag: &str) -> Env {
        let dir = std::env::temp_dir().join(format!("sai-rt-card-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let data_dir = dir.join(".sai/paper");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(dir.join(".sai/blocks")).unwrap();
        for b in ["lab-grow", "lab-read"] {
            fs::write(
                dir.join(format!(".sai/blocks/{b}.md")),
                format!("---\nid: {b}\nname: {b}\ndescription: テスト\n---\n\n# task_text\n"),
            )
            .unwrap();
        }
        let db = Database::open(&data_dir.join("papr.db")).unwrap();
        db.connection()
            .execute("INSERT INTO papers (id, title, published_at) VALUES (37, '論文 37', '2021-05-01')", [])
            .unwrap();
        let c = db.connection();
        let mut n = store::NewNode {
            kind: Kind::A, parent_id: None, guide_key: Some("data"), text: "データの量は？", body: "", stance: None,
            cites: &[], paper_id: None, run_id: None, report_id: None, mlflow_ref: None, dataset_ref: None,
            paths: &[], created_by: "user",
        };
        store::insert_node(c, &n).unwrap();
        n.kind = Kind::A;
        n.guide_key = None;
        n.parent_id = Some(1);
        n.text = "1 万件";
        store::insert_node(c, &n).unwrap();
        n.kind = Kind::F;
        n.parent_id = Some(2);
        n.stance = Some("support");
        n.text = "論文は 1 万件と述べる";
        n.run_id = Some(99);
        store::insert_node(c, &n).unwrap();
        let defs = Defs {
            guides: vec![GuideDef {
                key: "data".into(), name: "データ".into(), explain: String::new(), label: String::new(),
                child_candidates: vec!["規模は？".into()], extract: vec!["件数".into(), "出所".into()],
                pitfalls: vec![],
            }],
            ..Defs::default()
        };
        Env { roots: PaperRoots { root: dir.clone(), data_dir }, dir, db, defs }
    }

    fn spec() -> CardSpec {
        CardSpec {
            node: "Q1".into(), block: "lab-grow".into(), purpose: "答えの候補を出す".into(), expect: "A を 2 つ".into(),
            mode: "purpose_first".into(), papers: vec![37], runner: "cc".into(), origin_run: Some(120), ..CardSpec::default()
        }
    }

    fn err(e: &Env, s: &CardSpec) -> String {
        create_card(&e.db, &e.roots, &e.defs, s).unwrap_err().to_string()
    }

    fn node_rows(e: &Env) -> usize {
        store::list_nodes(e.db.connection(), true).unwrap().len()
    }

    #[test]
    fn child_key_checked_and_written() {
        let e = env("child");
        let with = |node: &str, child: &str| CardSpec { node: node.into(), child: child.into(), ..spec() };
        // 軸の A（Q1 は A1 の別名）の下は A、軸でない A（A2）の下は A か F。それ以外（と知らない値・Q）は入力の誤り。
        for bad in [with("Q1", "F"), with("Q1", "Q"), with("A2", "Q"), with("A2", "O")] {
            assert!(matches!(create_card(&e.db, &e.roots, &e.defs, &bad), Err(RtError::Invalid(_))), "{bad:?}");
        }
        let q = create_card(&e.db, &e.roots, &e.defs, &with("Q1", "A")).unwrap();
        assert!(fs::read_to_string(&q.path).unwrap().contains("sci_child: A"));
        let a = create_card(&e.db, &e.roots, &e.defs, &with("A2", "F")).unwrap();
        assert!(fs::read_to_string(&a.path).unwrap().contains("sci_child: F"));
        let a_under_a = create_card(&e.db, &e.roots, &e.defs, &with("A2", "A")).unwrap();
        assert!(fs::read_to_string(&a_under_a.path).unwrap().contains("sci_child: A"));
        // 未指定なら行を書かない。
        let none = create_card(&e.db, &e.roots, &e.defs, &spec()).unwrap();
        assert!(!fs::read_to_string(&none.path).unwrap().contains("sci_child"));
    }

    #[test]
    fn creates_task_md_with_keys_and_writes_no_rows() {
        let e = env("ok");
        let before = node_rows(&e);
        let r = create_card(&e.db, &e.roots, &e.defs, &spec()).unwrap();
        let text = fs::read_to_string(&r.path).unwrap();
        for key in ["sci_node: A1", "sci_mode: purpose_first", "sci_papers: 37", "sci_expect: A を 2 つ"] {
            assert!(text.contains(key), "{key}\n{text}");
        }
        // 古い範囲と仮置きのキーは task.md に書かない（名前は文字列を分けて組み立てる）。
        assert!(!text.contains(&format!("sci_{}", "scope")) && !text.contains(&format!("sci_{}", "assume")));
        assert!(!text.contains("sci_question") && !text.contains("sci_role") && !text.contains("sci_depth"));
        let k = task_keys(&text);
        assert_eq!(k.node, Some((Kind::A, 1)));
        assert_eq!(k.papers, vec![37]);
        assert_eq!(node_rows(&e), before);
    }

    #[test]
    fn whole_tree_card_has_no_node() {
        let e = env("whole");
        // 木の全体は manage の block だけ。`--node` を省くと sci_node は空、依頼文は（木の全体）。
        fs::write(e.dir.join(".sai/blocks/lab-progress.md"), "---\nid: lab-progress\nname: lab-progress\nsci_type: manage\n---\n\n# task_text\n").unwrap();
        let mut s = spec();
        s.node = String::new();
        s.block = "lab-progress".into();
        let r = create_card(&e.db, &e.roots, &e.defs, &s).unwrap();
        let text = fs::read_to_string(&r.path).unwrap();
        assert!(text.contains("- NODE: （木の全体）"), "{text}");
        // 空の値は task.md に書かない。キーが無いので node は無し（sci_purpose で sci の CARD になる）。
        assert!(!text.contains("sci_node"), "{text}");
        let k = task_keys(&text);
        assert!(k.node.is_none() && !k.whole && k.is_sci());
        // node の型の block は木の全体を断る。
        let mut s = spec();
        s.node = String::new();
        assert!(err(&e, &s).contains("対象の Node が要ります"));
        // 廃止した SEED は、どの block でも断る。
        let mut s = spec();
        s.node = "SEED".into();
        s.block = "lab-progress".into();
        assert!(err(&e, &s).contains("SEED は廃止しました"));
        // READ_CARD（runner=user）は木の全体では作れない（lab-read は node の型で、F を付ける先が要る）。
        let mut s = spec();
        s.node = String::new();
        s.runner = "user".into();
        assert!(err(&e, &s).contains("対象の Node が要ります"));
    }

    /// runner が user で block が空のとき、READ_CARD の既定の block は lab-read。
    #[test]
    fn read_card_without_block_uses_lab_read() {
        let e = env("read-default");
        let mut s = spec();
        s.runner = "user".into();
        s.block = String::new();
        s.papers = vec![37];
        let r = create_card(&e.db, &e.roots, &e.defs, &s).unwrap();
        let text = fs::read_to_string(&r.path).unwrap();
        assert!(text.contains("lab-read"), "{text}");
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
        s.runner = "user".into();
        s.papers = vec![37, 37];
        assert!(err(&e, &s).contains("ちょうど 1 本"));
        let mut s = spec();
        s.runner = "user".into();
        s.node = "F3".into();
        assert!(err(&e, &s).contains("READ_CARD の Node は A にしてください"));
        let mut s = spec();
        s.purpose = "한글".into();
        assert!(err(&e, &s).contains("ハングル"));
    }

    #[test]
    fn read_card_roundtrip() {
        let e = env("read");
        let mut s = spec();
        s.node = "A2".into();
        s.runner = "user".into();
        s.block = String::new();
        s.mode = String::new();
        let r = create_card(&e.db, &e.roots, &e.defs, &s).unwrap();
        let text = fs::read_to_string(&r.path).unwrap();
        assert!(text.contains("## 読むときの観点") && text.contains("- 件数") && text.contains("sci_depth: full"));
        // READ_CARD の run に F を付け替えて、note も置く。
        e.db.connection().execute("UPDATE rt_nodes SET run_id = ?1 WHERE id = 3", [r.run_id]).unwrap();
        let card = read_card(e.db.connection(), &e.roots, &e.defs, r.run_id).unwrap();
        assert_eq!(card.questions, vec!["データの量は？", "規模は？"]);
        assert_eq!(card.extract, vec!["件数", "出所"]);
        assert_eq!(card.findings.len(), 1);
        assert_eq!(card.paper["year"], "2021");
        assert!(!card.note);
        // READ_CARD でない CARD は弾く。
        let r2 = create_card(&e.db, &e.roots, &e.defs, &spec()).unwrap();
        assert!(read_card(e.db.connection(), &e.roots, &e.defs, r2.run_id).unwrap_err().to_string().contains("READ_CARD ではありません"));
    }

    /// runner は cc（省略も cc）か user。pi は理由付きで断る。model・thinking は空なら渡さず、
    /// 外れ値は断る。値は sai-link の `card create` の JSON の最上位に入る。
    #[test]
    fn runner_cc_model_thinking_checked_and_passed() {
        let e = env("runner");

        let mut s = spec();
        s.runner = String::new();
        let r = create_card(&e.db, &e.roots, &e.defs, &s).unwrap();
        assert!(fs::read_to_string(&r.path).unwrap().contains("runner: cc"));

        let mut s = spec();
        s.runner = "pi".into();
        assert!(err(&e, &s).contains("pi は廃止"));
        s.runner = "robot".into();
        assert!(err(&e, &s).contains("知らない実行者"));

        let mut s = spec();
        s.model = "gpt".into();
        assert!(err(&e, &s).contains("model は"));
        s.model = "claude-haiku-5-5".into();
        s.thinking = "ultra".into();
        assert!(err(&e, &s).contains("thinking は none"));

        let mut s = spec();
        s.model = "haiku".into();
        s.thinking = "low".into();
        create_card(&e.db, &e.roots, &e.defs, &s).unwrap();
        let card = crate::fake_link::last_card(&e.dir).expect("card create に JSON を渡した");
        assert_eq!(card["model"], "haiku");
        assert_eq!(card["thinking"], "low");
        assert!(card.get("executer").is_none(), "card={card}");

        // 空は JSON に入れない（sai-link の settings.json の既定を使う）。
        create_card(&e.db, &e.roots, &e.defs, &spec()).unwrap();
        let card = crate::fake_link::last_card(&e.dir).expect("card create に JSON を渡した");
        assert!(card.get("model").is_none() && card.get("thinking").is_none(), "card={card}");
    }

    /// sai-link の終了コード 3（見つからない）は NotFound に、ほかは Invalid に直す。
    #[test]
    fn link_error_keeps_not_found_code() {
        let not_found = handoff_error(HandoffError::Link(crate::bins::BinError {
            code: 3,
            message: "x".into(),
        }));
        assert!(matches!(not_found, RtError::NotFound(_)), "{not_found:?}");
        let invalid = handoff_error(HandoffError::Link(crate::bins::BinError {
            code: 2,
            message: "x".into(),
        }));
        assert!(matches!(invalid, RtError::Invalid(_)), "{invalid:?}");
    }
}
