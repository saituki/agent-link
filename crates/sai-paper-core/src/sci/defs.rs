//! 型と項目ガイドの読み込み・条件の読み取り・骨組みづくり。
//!
//! 元の版はリポジトリの `sai-sci/types/` と `sai-sci/guides/` で、ここに埋め込む。
//! 読むときはプロジェクトの写し `<root>/.sai/sci/` を先にし、無ければ埋め込みを使う。
//! 利用者が写しの md を直してよいようにするためと、写しが無いプロジェクトでも動くようにするため。
//!
//! 読み取りの失敗は止めない。読めなかったファイルは [`Defs::warnings`] に 1 行入れて飛ばす。

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use rusqlite::Connection;

use super::store::{self, NewNode};
use super::{Result, SciError};

/// 埋め込みの一覧を `include_str!` の繰り返しなく書くための補助。
macro_rules! embedded_files {
    ($($relative:literal),* $(,)?) => {
        &[$(($relative, include_str!(concat!("../../../../sai-sci/", $relative)))),*]
    };
}

/// sai-sci の元の版（リポジトリの `sai-sci/`）。相対パスと中身。
const EMBEDDED: &[(&str, &str)] = embedded_files![
    // 型（5 本）
    "types/problem.md",
    "types/phenomenon.md",
    "types/method.md",
    "types/data.md",
    "types/improve.md",
    // 一番上の項目のガイド（9 本）
    "guides/background.md",
    "guides/target.md",
    "guides/data.md",
    "guides/purpose.md",
    "guides/existing.md",
    "guides/issue.md",
    "guides/method.md",
    "guides/novelty.md",
    "guides/phenomenon.md",
    // 子の項目のガイド（16 本）
    "guides/background.goal.md",
    "guides/background.problem.md",
    "guides/target.subgroup.md",
    "guides/data.type.md",
    "guides/data.size.md",
    "guides/data.label.md",
    "guides/data.access.md",
    "guides/purpose.subgoal.md",
    "guides/purpose.output.md",
    "guides/purpose.metric.md",
    "guides/issue.limit.md",
    "guides/issue.root.md",
    "guides/method.analog.md",
    "guides/method.adapt.md",
    "guides/novelty.closest.md",
    "guides/novelty.diff.md",
];

/// `Defs::types` の先頭に並べる 5 つの型（この順に並ぶ）。ここに無い型は後ろで id 順。
const TYPE_ORDER: [&str; 5] = ["problem", "phenomenon", "method", "data", "improve"];

/// 骨組みを作るときの実行者（利用者が `sci init` をした扱い）。
const ACTOR: &str = "user";

// ---- 読み込んだ定義 ---------------------------------------------------------

/// 研究の型（`sai-sci/types/<id>.md` を読んだもの）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TypeDef {
    /// 型の id（`problem` など）。
    pub id: String,
    /// 表示名（`課題駆動` など）。
    pub name: String,
    /// この型で最初から与えられることが多い項目のキー。
    pub given: Vec<String>,
    /// 型の説明（見出しの下の本文）。
    pub explain: String,
    /// 一番上の項目のキー（この順に骨組みを作る）。
    pub order: Vec<String>,
    /// 種の文。`{<項目キー>}` と `{mind:<見出し>}` を含む。
    pub seed: String,
    /// 関門（上から順に判定する）。
    pub gates: Vec<Gate>,
}

/// 関門の 1 つ。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Gate {
    /// 関門の表示名（`出発点` など）。
    pub name: String,
    /// 満たすための条件（`parse_condition` で読む）。
    pub when: String,
}

/// 項目ガイド（`sai-sci/guides/<key>.md` を読んだもの）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GuideDef {
    /// 項目キー（ファイル名と同じ）。
    pub key: String,
    /// 項目の表示名。
    pub name: String,
    /// 項目の型（C-01 の順で並べる）。
    pub types: Vec<String>,
    /// 親項目（一番上の項目は `None`）。
    pub parent: Option<String>,
    /// 既定の依存の線の上流のキー。
    pub depends_on: Vec<String>,
    /// 「情報不足」の目安（`min_findings` が無ければ 2）。
    pub min_findings: usize,
    /// 項目の説明（見出しの下の本文）。
    pub explain: String,
    /// 項目を作ると入る既定の問い。
    pub questions: Vec<String>,
    /// 埋まった条件（`parse_condition` で読む）。
    pub condition: String,
    /// 条件の下の、人向けの説明。
    pub condition_text: String,
    /// 抜き出す項目（論文の役割と、抜き出すもの）。
    pub extract: Vec<(String, String)>,
    /// 落とし穴。
    pub pitfalls: Vec<String>,
    /// 次の一手。
    pub next: Vec<NextRule>,
    /// 子の項目のキー（この順に作る）。
    pub children: Vec<String>,
}

/// 次の一手の 1 つ。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NextRule {
    /// どんなときか（空欄の種類または印）。
    pub when: String,
    /// 使うブロックの id。`-` なら `None`（人の作業）。
    pub block: Option<String>,
    /// 目的のひな形。`{title}` は項目の名前に置き換える。
    pub purpose: String,
    /// 期待する成果。
    pub expect: String,
}

/// 型と項目ガイドの読み込み結果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Defs {
    /// 型（5 つの型が既定の順、その後にプロジェクトにだけある型）。
    pub types: Vec<TypeDef>,
    /// 項目ガイド（key の順）。
    pub guides: Vec<GuideDef>,
    /// 読めなかったもの・名前の合わないもの（利用者にそのまま見せる）。
    pub warnings: Vec<String>,
}

impl Defs {
    /// id の型を探す。無ければ `None`。
    #[must_use]
    pub fn type_def(&self, id: &str) -> Option<&TypeDef> {
        self.types.iter().find(|def| def.id == id)
    }

    /// key の項目ガイドを探す。無ければ `None`。
    #[must_use]
    pub fn guide(&self, key: &str) -> Option<&GuideDef> {
        self.guides.iter().find(|guide| guide.key == key)
    }
}

// ---- 条件 -------------------------------------------------------------------

/// 条件の書き方 1 つ。` かつ ` で並べると、すべてを満たすときだけ真になる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cond {
    /// `decided(<key>)` — その項目の状態が `decided`。
    Decided(String),
    /// `count(<key>, <n>)` — その項目の下の候補（`candidate` と `selected`）が n 個以上。
    Count(String, usize),
    /// `saturated(<key>)` — 候補が出たあとに検索が 2 回以上あり、候補に代表例がある。
    Saturated(String),
    /// `evidence(<key>, <n>)` — 支持の知見が n 個以上。
    Evidence(String, usize),
    /// `level(<key>, <n>)` — 根拠の段が n 以上の支持の知見がある。
    Level(String, i64),
    /// `not_empty(<key>)` — その項目の本文が空でない。
    NotEmpty(String),
    /// `mind(<見出し>)` — mind.md のその見出しの節が空でない。
    Mind(String),
}

// ---- 埋め込みと読み込み -----------------------------------------------------

/// 埋め込みの一覧（`"types/problem.md"` のような相対パスと中身）。
#[must_use]
pub fn embedded() -> &'static [(&'static str, &'static str)] {
    EMBEDDED
}

/// `<root>/.sai/sci/` を先に、無ければ埋め込みを読んで型と項目ガイドをまとめる。
///
/// - プロジェクトの写し（`types/` と `guides/` の直下の `.md`）は、同じ相対パスの埋め込みを上書きする。
/// - プロジェクトにだけあるファイルも読む（利用者が足したガイド）。
/// - 読めないファイルは [`Defs::warnings`] に 1 行入れて飛ばす。
#[must_use]
pub fn load(root: &Path) -> Defs {
    let mut warnings: Vec<String> = Vec::new();
    let mut files: BTreeMap<String, String> = embedded()
        .iter()
        .map(|(relative, text)| ((*relative).to_string(), (*text).to_string()))
        .collect();

    // プロジェクトの写しを足す（同じ相対パスは上書き）。無いフォルダはそのままにする。
    let local = root.join(".sai").join("sci");
    for folder in ["types", "guides"] {
        let Ok(entries) = fs::read_dir(local.join(folder)) else {
            continue;
        };
        let mut names: Vec<String> = entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for name in names {
            if !name.ends_with(".md") {
                continue;
            }
            let relative = format!("{folder}/{name}");
            match fs::read_to_string(local.join(folder).join(&name)) {
                Ok(text) => {
                    files.insert(relative, text);
                }
                Err(error) => warnings.push(format!("{relative} を読めません: {error}")),
            }
        }
    }

    let mut types: Vec<TypeDef> = Vec::new();
    let mut guides: Vec<GuideDef> = Vec::new();
    for (relative, text) in &files {
        if relative.starts_with("types/") {
            match parse_type(text) {
                Ok(found) => types.push(found),
                Err(error) => warnings.push(format!("{relative} を読めません: {error}")),
            }
        } else if let Some(name) = relative.strip_prefix("guides/") {
            let stem = name.strip_suffix(".md").unwrap_or(name);
            match parse_guide(text) {
                Ok(mut guide) => {
                    // key はファイル名（拡張子なし）を正とする。
                    if guide.key != stem {
                        warnings.push(format!(
                            "{relative} の key は {} なので、ファイル名の {stem} を key にします",
                            guide.key
                        ));
                        guide.key = stem.to_string();
                    }
                    guides.push(guide);
                }
                Err(error) => warnings.push(format!("{relative} を読めません: {error}")),
            }
        }
    }

    // 5 つの型は既定の順に、プロジェクトにだけある型は後ろで id 順に並べる。
    types.sort_by(|left, right| {
        let rank = |id: &str| {
            TYPE_ORDER
                .iter()
                .position(|known| *known == id)
                .unwrap_or(TYPE_ORDER.len())
        };
        rank(&left.id)
            .cmp(&rank(&right.id))
            .then_with(|| left.id.cmp(&right.id))
    });
    guides.sort_by(|left, right| left.key.cmp(&right.key));

    Defs {
        types,
        guides,
        warnings,
    }
}

/// 埋め込みのうち `<root>/.sai/sci/` に無いものだけ.write、書いた数を返す。
///
/// 既にあるファイルは触らない（利用者が直した内容を守るため）。
pub fn write_local_copies(root: &Path) -> Result<usize> {
    let base = root.join(".sai").join("sci");
    let mut written = 0usize;
    for (relative, text) in embedded() {
        let path = base.join(relative);
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, text)?;
        written += 1;
    }
    Ok(written)
}

// ---- 1 本の md を読む -------------------------------------------------------

/// 型の md を読む。`id` か `## 項目の順序` が無ければ [`SciError::Invalid`]。
pub fn parse_type(text: &str) -> Result<TypeDef> {
    let Doc { explain, sections } = read_doc(text);
    let front = Frontmatter::of(text);
    let id = front.get("id");
    if id.is_empty() {
        return Err(SciError::Invalid("型に id がありません".to_string()));
    }
    let order = bullets(section(&sections, "ITEM_ORDER"));
    if order.is_empty() {
        return Err(SciError::Invalid(format!(
            "型 {id} に ITEM_ORDER がありません"
        )));
    }
    let name = front.get("name");
    let given = split_list(&front.get("given"));
    let seed = first_nonempty(section(&sections, "SEED_SENTENCE")).unwrap_or_default();
    let gates = section(&sections, "GATE")
        .iter()
        .filter_map(|line| {
            // `- <名前>: <条件>`。形が違う行は飛ばす。
            let item = line.trim().strip_prefix("- ")?;
            let (name, when) = item.split_once(':')?;
            Some(Gate {
                name: name.trim().to_string(),
                when: when.trim().to_string(),
            })
        })
        .collect();
    Ok(TypeDef {
        id,
        name,
        given,
        explain,
        order,
        seed,
        gates,
    })
}

/// 項目ガイドの md を読む。`key` が無ければ [`SciError::Invalid`]。
pub fn parse_guide(text: &str) -> Result<GuideDef> {
    let Doc { explain, sections } = read_doc(text);
    let front = Frontmatter::of(text);
    let key = front.get("key");
    if key.is_empty() {
        return Err(SciError::Invalid(
            "ITEM_GUIDE に key がありません".to_string(),
        ));
    }
    let parent = front.get("parent");
    let name = front.get("name");
    let types = split_list(&front.get("type"));
    let depends_on = split_list(&front.get("depends_on"));
    let min_findings = front
        .get("min_findings")
        .parse::<usize>()
        .unwrap_or(DEFAULT_MIN_FINDINGS);
    let questions = bullets(section(&sections, "QUESTION"));
    let (condition, condition_text) = split_condition(section(&sections, "CONDITION"));
    let extract = section(&sections, "EXTRACT_FIELDS")
        .iter()
        .filter_map(|line| {
            // `- <役割>: <抜き出す項目>`。形が違う行は飛ばす。
            let item = line.trim().strip_prefix("- ")?;
            let (role, what) = item.split_once(':')?;
            Some((role.trim().to_string(), what.trim().to_string()))
        })
        .collect();
    let pitfalls = bullets(section(&sections, "PITFALL"));
    let next = section(&sections, "NEXT_ACTION")
        .iter()
        .filter_map(|line| {
            // `- <種類>: <ブロック> | <目的> | <成果>`。
            // `|` が 2 つでない行は飛ばす（ガイドの行の数の検査は U01・U02 が行う）。
            let item = line.trim().strip_prefix("- ")?;
            let (when, rest) = item.split_once(':')?;
            let parts: Vec<&str> = rest.split('|').collect();
            if parts.len() != 3 {
                return None;
            }
            let block = parts[0].trim();
            Some(NextRule {
                when: when.trim().to_string(),
                block: if block == "-" {
                    None
                } else {
                    Some(block.to_string())
                },
                purpose: parts[1].trim().to_string(),
                expect: parts[2].trim().to_string(),
            })
        })
        .collect();
    let children = bullets(section(&sections, "CHILD_ITEM"));
    Ok(GuideDef {
        key,
        name,
        types,
        parent: if parent.is_empty() {
            None
        } else {
            Some(parent)
        },
        depends_on,
        min_findings,
        explain,
        questions,
        condition,
        condition_text,
        extract,
        pitfalls,
        next,
        children,
    })
}

/// 条件の文字列を読む。` かつ ` で分けて 1 つずつ読む。
///
/// 空文字なら空の `Vec`。読めない部分があれば [`SciError::Invalid`]。
pub fn parse_condition(text: &str) -> Result<Vec<Cond>> {
    split_and(text).into_iter().map(parse_one_cond).collect()
}

/// 「情報不足」の目安の既定値。
const DEFAULT_MIN_FINDINGS: usize = 2;

// ---- 骨組み -----------------------------------------------------------------

/// 骨組みを作るときの結果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InitReport {
    /// 作った項目の数（子の項目を含む）。
    pub created_items: usize,
    /// 引いた線の数（`parent` と `depends` の合計）。
    pub created_edges: usize,
    /// この木の型の id。
    pub type_id: String,
    /// ガイドが無い項目など、利用者にそのまま見せる注意。
    pub warnings: Vec<String>,
}

/// 項目キーのガイドがあれば、その名前・型・既定の問いで項目を作り、
/// `## 子の項目` を子として作る。子の並びは `ord` から 0 番で続ける。
///
/// ガイドが無い key でも、題を key にした項目を作る（利用者が型の md に key を足せるようにするため）。
/// 既に同じ key の項目があれば [`SciError::Invalid`]（子の再帰のときは既存の項目を先に調べて飛ばす）。
pub fn create_item_from_guide(
    conn: &Connection,
    defs: &Defs,
    key: &str,
    parent: Option<i64>,
    ord: i64,
    created_by: &str,
) -> Result<i64> {
    if let Some(node) = store::find_item_by_key(conn, key)? {
        return Err(SciError::Invalid(format!(
            "ITEM {key} は既にあります（N{}）",
            node.id
        )));
    }
    let guide = defs.guide(key);
    let title = guide.map_or_else(|| key.to_string(), |found| found.name.clone());
    let types: Vec<String> = guide.map(|found| found.types.clone()).unwrap_or_default();
    let id = store::insert_node(
        conn,
        &NewNode {
            kind: "item",
            item_key: Some(key),
            types: &types,
            title: &title,
            body: "",
            state: "open",
            fixed: false,
            ord,
            created_by,
        },
    )?;

    // 既定の問いはガイドから入れる（手作りの問いと区別する）。
    if let Some(found) = guide {
        for question in &found.questions {
            if question.trim().is_empty() {
                continue;
            }
            store::insert_question(conn, id, question, true, created_by)?;
        }
    }
    if let Some(parent) = parent {
        store::insert_edge(conn, "parent", parent, id)?;
    }

    // 子を作る。別の枝で既に同じ key があれば飛ばす。
    if let Some(found) = guide {
        for (index, child) in found.children.iter().enumerate() {
            if store::find_item_by_key(conn, child)?.is_some() {
                continue;
            }
            let ord = i64::try_from(index).unwrap_or(0);
            create_item_from_guide(conn, defs, child, Some(id), ord, created_by)?;
        }
    }
    Ok(id)
}

/// 型の `## 項目の順序` に沿って骨組みを作る。
///
/// - 節点が 1 つも無いときだけ項目を作る（既にある木は壊さない）。
/// - ガイドの `depends_on` の線（`depends`）を引く。
/// - `sci_meta` に `type` と `initialized_at` を書く。
/// - 途中で失敗したら何も残さない（1 つのトランザクションで行う）。
pub fn init_tree(conn: &Connection, defs: &Defs, type_id: &str) -> Result<InitReport> {
    let Some(type_def) = defs.type_def(type_id) else {
        return Err(SciError::Invalid(format!(
            "知らない RESEARCH_TYPE です: {type_id}"
        )));
    };
    let order = type_def.order.clone();
    let mut warnings = Vec::new();
    for key in &order {
        missing_guides(defs, key, &mut warnings);
    }

    let tx = conn.unchecked_transaction()?;
    let nodes_before = store::list_nodes(&tx)?.len();
    let edges_before = store::list_edges(&tx)?.len();
    if nodes_before == 0 {
        for (index, key) in order.iter().enumerate() {
            let ord = i64::try_from(index).unwrap_or(0);
            create_item_from_guide(&tx, defs, key, None, ord, ACTOR)?;
        }
    }
    add_depends_edges(&tx, defs)?;
    store::meta_set(&tx, "type", type_id)?;
    if store::meta_get(&tx, "initialized_at")?.is_none() {
        store::meta_set(&tx, "initialized_at", &super::now_text())?;
    }
    let report = InitReport {
        created_items: store::list_nodes(&tx)?.len() - nodes_before,
        created_edges: store::list_edges(&tx)?.len() - edges_before,
        type_id: type_id.to_string(),
        warnings,
    };
    tx.commit()?;
    Ok(report)
}

/// 研究の型だけを変える。今の木は壊さない。
///
/// 新しい型の `## 項目の順序` にあって木に無い項目だけを一番上に足し（子の項目も一緒に作る）、
/// 依存の線と `sci_meta` の `type` を入れ直す。既に在る項目・線・状態は変えない。
pub fn change_type(conn: &Connection, defs: &Defs, type_id: &str) -> Result<InitReport> {
    let Some(type_def) = defs.type_def(type_id) else {
        return Err(SciError::Invalid(format!(
            "知らない RESEARCH_TYPE です: {type_id}"
        )));
    };
    let order = type_def.order.clone();
    let mut warnings = Vec::new();
    for key in &order {
        missing_guides(defs, key, &mut warnings);
    }

    let tx = conn.unchecked_transaction()?;
    let nodes_before = store::list_nodes(&tx)?.len();
    let edges_before = store::list_edges(&tx)?.len();
    let mut ord = top_count(&tx)?;
    for key in &order {
        if store::find_item_by_key(&tx, key)?.is_some() {
            continue;
        }
        create_item_from_guide(&tx, defs, key, None, ord, ACTOR)?;
        ord += 1;
    }
    add_depends_edges(&tx, defs)?;
    store::meta_set(&tx, "type", type_id)?;
    let report = InitReport {
        created_items: store::list_nodes(&tx)?.len() - nodes_before,
        created_edges: store::list_edges(&tx)?.len() - edges_before,
        type_id: type_id.to_string(),
        warnings,
    };
    tx.commit()?;
    Ok(report)
}

/// ガイドの `depends_on` に沿って、木にある項目どうしの `depends` の線を引く。
///
/// 両端の項目が無いときは引かない。同じ線は足さない（`insert_edge` が既存の番号を返す）。
fn add_depends_edges(conn: &Connection, defs: &Defs) -> Result<()> {
    for node in store::list_nodes(conn)? {
        let Some(key) = node.item_key.as_deref() else {
            continue;
        };
        let Some(guide) = defs.guide(key) else {
            continue;
        };
        for upstream in &guide.depends_on {
            let Ok(Some(from)) = store::find_item_by_key(conn, upstream) else {
                continue;
            };
            store::insert_edge(conn, "depends", from.id, node.id)?;
        }
    }
    Ok(())
}

/// ガイドが無い key（その子孫も）を警告に並べる。
fn missing_guides(defs: &Defs, key: &str, out: &mut Vec<String>) {
    let Some(guide) = defs.guide(key) else {
        out.push(format!("ITEM_GUIDE がありません: {key}"));
        return;
    };
    for child in &guide.children {
        missing_guides(defs, child, out);
    }
}

/// 一番上の項目（親から出た線が無い項目）の数。次の `ord` の始まりに使う。
fn top_count(conn: &Connection) -> Result<i64> {
    let children: HashSet<i64> = store::list_edges(conn)?
        .iter()
        .filter(|edge| edge.kind == "parent")
        .map(|edge| edge.to)
        .collect();
    let count = store::list_nodes(conn)?
        .iter()
        .filter(|node| !children.contains(&node.id))
        .count();
    Ok(i64::try_from(count).unwrap_or(0))
}

// ---- 条件の読み取りの補助 ---------------------------------------------------

/// 条件を 1 つ読む。
fn parse_one_cond(part: &str) -> Result<Cond> {
    let invalid = || SciError::Invalid(format!("条件を読めません: {part}"));
    let Some((name, rest)) = part.split_once('(') else {
        return Err(invalid());
    };
    let Some(inner) = rest.strip_suffix(')') else {
        return Err(invalid());
    };
    let inner = inner.trim();
    if inner.is_empty() {
        return Err(invalid());
    }
    match name.trim() {
        "decided" => Ok(Cond::Decided(inner.to_string())),
        "saturated" => Ok(Cond::Saturated(inner.to_string())),
        "not_empty" => Ok(Cond::NotEmpty(inner.to_string())),
        "mind" => Ok(Cond::Mind(inner.to_string())),
        "count" => {
            let (key, count) = inner.split_once(',').ok_or_else(invalid)?;
            Ok(Cond::Count(
                key.trim().to_string(),
                count.trim().parse().map_err(|_| invalid())?,
            ))
        }
        "evidence" => {
            let (key, count) = inner.split_once(',').ok_or_else(invalid)?;
            Ok(Cond::Evidence(
                key.trim().to_string(),
                count.trim().parse().map_err(|_| invalid())?,
            ))
        }
        "level" => {
            let (key, level) = inner.split_once(',').ok_or_else(invalid)?;
            Ok(Cond::Level(
                key.trim().to_string(),
                level.trim().parse().map_err(|_| invalid())?,
            ))
        }
        _ => Err(invalid()),
    }
}

/// ` かつ ` で分ける。前後の空白が 1 つでも「かつ」の両側にあるときだけ区切りにする。
fn split_and(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(index) = find_and(rest) {
        let (before, after) = rest.split_at(index);
        parts.push(before.trim());
        // 境目の「かつ」を落としてから次を読む。
        rest = after["かつ".len()..].trim_start();
    }
    parts.push(rest.trim());
    parts.into_iter().filter(|part| !part.is_empty()).collect()
}

/// ` かつ ` の `かつ` の頭の位置。無ければ `None`。
fn find_and(text: &str) -> Option<usize> {
    for (index, _) in text.char_indices() {
        let Some(after) = text[index..].strip_prefix("かつ") else {
            continue;
        };
        if text[..index].ends_with(char::is_whitespace) && after.starts_with(char::is_whitespace) {
            return Some(index);
        }
    }
    None
}

// ---- md の読み取りの補助 ----------------------------------------------------

/// frontmatter の `key: value` を読むための薄い包み。
struct Frontmatter {
    /// `crate::runs::read_frontmatter` が返した組。
    pairs: Vec<(String, String)>,
}

impl Frontmatter {
    /// md の全文から読む。
    fn of(text: &str) -> Self {
        Frontmatter {
            pairs: crate::runs::read_frontmatter(text),
        }
    }

    /// 値を返す。無ければ空文字。
    fn get(&self, key: &str) -> String {
        self.pairs
            .iter()
            .find(|(name, _)| name == key)
            .map_or_else(String::new, |(_, value)| value.trim().to_string())
    }
}

/// md を読んだ結果（frontmatter の後ろの見出しの下と、`## ` の節）。
struct Doc {
    /// 最初の見出し（`# データ`）から最初の `## ` までの本文。見出しの行は含めない。
    explain: String,
    /// `## ` の節（見出しの名前と、その下の行）。
    sections: Vec<(String, Vec<String>)>,
}

/// 型・ガイドの節の見出しの別名（新しい名前, 旧い名前）。新旧どちらでも読む。
const SECTION_ALIASES: [(&str, &str); 9] = [
    ("ITEM_ORDER", "項目の順序"),
    ("SEED_SENTENCE", "種の文"),
    ("GATE", "関門"),
    ("QUESTION", "問い"),
    ("CONDITION", "埋まった条件"),
    ("EXTRACT_FIELDS", "抜き出す項目"),
    ("PITFALL", "落とし穴"),
    ("NEXT_ACTION", "次の一手"),
    ("CHILD_ITEM", "子の項目"),
];

/// mind.md の見出しの別名（新しい名前, 旧い名前）。
const MIND_ALIASES: [(&str, &str); 7] = [
    ("ORIGIN", "始まり"),
    ("RESEARCH_TYPE", "研究の型"),
    ("GIVEN", "与えられていること"),
    ("CONSTRAINTS", "制約"),
    ("VALUE_AXES", "価値の軸"),
    ("INTERESTS", "興味がある / 興味が無い"),
    ("NOTES", "メモ"),
];

/// mind.md の見出しを新しい名前にそろえる。旧い名前なら新しい名前、それ以外はそのまま。
#[must_use]
pub fn canonical_mind_heading(name: &str) -> &str {
    MIND_ALIASES
        .iter()
        .find(|(_, old)| *old == name)
        .map_or(name, |(new, _)| new)
}

/// `## ` の節の行を返す。節が無ければ空。
///
/// `name` は新しい名前で渡す。新しい名前を先に探し、無ければ旧い名前を探す。
fn section<'a>(sections: &'a [(String, Vec<String>)], name: &str) -> &'a [String] {
    let find = |key: &str| sections.iter().find(|(found, _)| found == key);
    find(name)
        .or_else(|| {
            SECTION_ALIASES
                .iter()
                .find(|(new, _)| *new == name)
                .and_then(|(_, old)| find(old))
        })
        .map_or(&[], |(_, lines)| lines.as_slice())
}

/// md を [`Doc`] に分ける。
///
/// ```markdown` で囲んだ中は見出しとして読まない（表の例などを壊さないため）。
fn read_doc(text: &str) -> Doc {
    let mut doc = Doc {
        explain: String::new(),
        sections: Vec::new(),
    };
    let mut explain: Vec<&str> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    let mut fenced = false;
    for line in body_of(text).lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            if let Some(name) = trimmed.strip_prefix("## ") {
                if let Some(section) = current.take() {
                    doc.sections.push(section);
                }
                current = Some((name.trim().to_string(), Vec::new()));
                continue;
            }
            if current.is_none() {
                // 最初の見出し（`# データ`）の行は説明の本文に入れない。
                if trimmed.starts_with("# ") {
                    continue;
                }
                explain.push(line);
                continue;
            }
        }
        if let Some((_, lines)) = current.as_mut() {
            lines.push(line.to_string());
        }
    }
    if let Some(section) = current.take() {
        doc.sections.push(section);
    }
    doc.explain = trim_blank_edges(&explain).join("\n");
    doc
}

/// 先頭の frontmatter（`---` から次の `---` まで）を除いた本文。
fn body_of(text: &str) -> &str {
    let rest = text.strip_prefix("---\n").unwrap_or(text);
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        if line.trim() == "---" {
            return &rest[offset + line.len()..];
        }
        offset += line.len();
    }
    rest
}

/// 両端の空行を落とした行を 1 本の文字列に束ねる。
fn trim_blank_edges<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let start = lines.iter().position(|line| !line.trim().is_empty());
    let Some(start) = start else {
        return Vec::new();
    };
    let end = lines.iter().rposition(|line| !line.trim().is_empty());
    lines[start..=end.unwrap_or(start)].to_vec()
}

/// 節の行から `- ` で始まる行の中身を集める（`- ` で無い行は落とす）。
fn bullets(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// 最初の空でない行。
fn first_nonempty(lines: &[String]) -> Option<String> {
    lines
        .iter()
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// `## 埋まった条件` の節を「条件 1 行」と「そのあとの人向けの説明」に分ける。
fn split_condition(lines: &[String]) -> (String, String) {
    let mut condition = String::new();
    let mut after: Vec<&str> = Vec::new();
    let mut found = false;
    for line in lines {
        if let Some(item) = line.trim().strip_prefix("- ") {
            if !found {
                found = true;
                condition = item.trim().to_string();
            }
            continue;
        }
        if found {
            after.push(line);
        }
    }
    (condition, trim_blank_edges(&after).join("\n"))
}

/// カンマ区切りの値を並べ直す（空の要素は落とす）。
fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use std::path::PathBuf;

    /// 全表のメモリ DB。DB ごと持ち、接続は `database.connection()` で取る。
    fn db() -> Database {
        Database::in_memory().expect("メモリ DB")
    }

    /// 埋め込みだけを読むための Defs（存在しないフォルダを渡して、プロジェクトの写しが無い状態を作る）。
    fn defs() -> Defs {
        load(Path::new("target/sai-sci-defs-not-found"))
    }

    /// テスト用の一時フォルダ。
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-defs-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("一時フォルダ");
        dir
    }

    /// 一時フォルダの写しに 1 本だけ md を書く（親フォルダは作る）。
    fn write_local(root: &Path, relative: &str, text: &str) {
        let path = root.join(".sai").join("sci").join(relative);
        fs::create_dir_all(path.parent().expect("親フォルダ")).expect("親フォルダを作る");
        fs::write(&path, text).expect("md を書く");
    }

    /// 項目キーの並びを並べる（番号の昇順）。
    fn item_keys(conn: &Connection) -> Vec<String> {
        let mut keys: Vec<(i64, String)> = store::list_nodes(conn)
            .expect("節点を読む")
            .into_iter()
            .filter_map(|node| node.item_key.map(|key| (node.id, key)))
            .collect();
        keys.sort();
        keys.into_iter().map(|(_, key)| key).collect()
    }

    #[test]
    fn embedded_files_all_parse() {
        // 埋め込みは型の 5 本と項目ガイドの 25 本。
        assert_eq!(embedded().len(), 30);
        assert_eq!(
            embedded()
                .iter()
                .filter(|(relative, _)| relative.starts_with("types/"))
                .count(),
            5
        );

        let defs = defs();
        assert!(defs.warnings.is_empty(), "{:?}", defs.warnings);
        assert_eq!(
            defs.types
                .iter()
                .map(|found| found.id.as_str())
                .collect::<Vec<_>>(),
            vec!["problem", "phenomenon", "method", "data", "improve"]
        );
        assert_eq!(defs.guides.len(), 25);
        assert!(defs.guides.windows(2).all(|pair| pair[0].key < pair[1].key));

        // 埋め込みの 1 本 1 本が読める。
        for (relative, text) in embedded() {
            if relative.starts_with("types/") {
                let def = parse_type(text).unwrap_or_else(|error| panic!("{relative}: {error}"));
                assert!(!def.name.is_empty(), "{relative}");
                assert!(!def.explain.is_empty(), "{relative}");
                assert!(!def.seed.is_empty(), "{relative}");
                assert!(!def.gates.is_empty(), "{relative}");
                for gate in &def.gates {
                    let conds = parse_condition(&gate.when)
                        .unwrap_or_else(|error| panic!("{relative} の {}: {error}", gate.name));
                    assert!(!conds.is_empty(), "{relative} の {}", gate.name);
                }
            } else {
                let guide = parse_guide(text).unwrap_or_else(|error| panic!("{relative}: {error}"));
                assert_eq!(
                    guide.key,
                    relative
                        .trim_start_matches("guides/")
                        .trim_end_matches(".md")
                );
                assert!(!guide.name.is_empty(), "{relative}");
                assert!(!guide.explain.is_empty(), "{relative}");
                assert!(!guide.questions.is_empty(), "{relative}");
                let conds = parse_condition(&guide.condition)
                    .unwrap_or_else(|error| panic!("{relative}: {error}"));
                assert!(!conds.is_empty(), "{relative}");
            }
        }
    }

    #[test]
    fn problem_skeleton_counts() {
        let database = db();
        let conn = database.connection();
        let defs = defs();
        let report = init_tree(conn, &defs, "problem").expect("骨組みを作る");

        // 一番上 8 個と子 16 個。
        assert_eq!(report.type_id, "problem");
        assert_eq!(report.created_items, 24);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(store::list_nodes(conn).unwrap().len(), 24);
        let edges = store::list_edges(conn).unwrap();

        // 一番上 8 個は型の「項目の順序」のとおりに並ぶ。
        let children: HashSet<i64> = edges
            .iter()
            .filter(|edge| edge.kind == "parent")
            .map(|edge| edge.to)
            .collect();
        let mut top: Vec<(i64, String)> = store::list_nodes(conn)
            .unwrap()
            .into_iter()
            .filter(|node| !children.contains(&node.id))
            .map(|node| (node.ord, node.item_key.unwrap_or_default()))
            .collect();
        top.sort();
        assert_eq!(
            top.iter().map(|(_, key)| key.as_str()).collect::<Vec<_>>(),
            vec![
                "background",
                "target",
                "data",
                "purpose",
                "existing",
                "issue",
                "method",
                "novelty"
            ]
        );
        // 子は 16 個。
        assert_eq!(children.len(), 16);
        let mut all: Vec<String> = item_keys(conn);
        all.sort();
        assert_eq!(
            all,
            vec![
                "background",
                "background.goal",
                "background.problem",
                "data",
                "data.access",
                "data.label",
                "data.size",
                "data.type",
                "existing",
                "issue",
                "issue.limit",
                "issue.root",
                "method",
                "method.adapt",
                "method.analog",
                "novelty",
                "novelty.closest",
                "novelty.diff",
                "purpose",
                "purpose.metric",
                "purpose.output",
                "purpose.subgoal",
                "target",
                "target.subgroup",
            ]
        );

        // 線は parent 16 本・depends 24 本。
        assert_eq!(edges.len(), 40);
        assert_eq!(report.created_edges, 40);
        assert_eq!(
            edges.iter().filter(|edge| edge.kind == "parent").count(),
            16
        );
        assert_eq!(
            edges.iter().filter(|edge| edge.kind == "depends").count(),
            24
        );

        // 親の線と依存の線が、決まった向きに引かれている。
        let id_of = |key: &str| store::find_item_by_key(conn, key).unwrap().unwrap().id;
        assert!(edges.iter().any(|edge| edge.kind == "parent"
            && edge.from == id_of("data")
            && edge.to == id_of("data.type")));
        assert!(edges.iter().any(|edge| edge.kind == "depends"
            && edge.from == id_of("data.type")
            && edge.to == id_of("data.size")));

        // 型は sci_meta に残る。
        assert_eq!(
            store::meta_get(conn, "type").unwrap().as_deref(),
            Some("problem")
        );
        assert!(store::meta_get(conn, "initialized_at").unwrap().is_some());
        assert!(store::is_initialized(conn).unwrap());

        // ガイドの問いは from_guide で入る。
        let questions = store::list_questions(conn, None).unwrap();
        assert!(!questions.is_empty());
        assert!(questions.iter().all(|question| question.from_guide));
        let data = id_of("data");
        assert!(
            questions
                .iter()
                .any(|question| question.node_id == data && question.text == "どのデータを使うか")
        );
    }

    #[test]
    fn init_tree_is_noop_when_nodes_exist() {
        let database = db();
        let conn = database.connection();
        let defs = defs();
        init_tree(conn, &defs, "problem").expect("骨組みを作る");
        let nodes = store::list_nodes(conn).unwrap().len();

        // 2 回目は足さない（既にある木を壊さない）。
        let report = init_tree(conn, &defs, "problem").expect("2 回目の骨組み");
        assert_eq!(report.created_items, 0);
        assert_eq!(report.created_edges, 0);
        assert_eq!(store::list_nodes(conn).unwrap().len(), nodes);
    }

    #[test]
    fn new_headings_parse_and_win_over_old() {
        let old = embedded_type("types/problem.md");
        let new = old
            .replace("## 項目の順序", "## ITEM_ORDER")
            .replace("## 種の文", "## SEED_SENTENCE")
            .replace("## 関門", "## GATE");
        let a = parse_type(old).unwrap();
        let b = parse_type(&new).unwrap();
        assert_eq!(a.order, b.order);
        assert_eq!(a.seed, b.seed);
        assert_eq!(a.gates.len(), b.gates.len());
        // 新旧が混ざったら新しい見出しが先。
        let mixed = format!("{new}\n## 項目の順序\n- zzz\n");
        assert_eq!(parse_type(&mixed).unwrap().order, a.order);
    }

    #[test]
    fn new_headings_parse_guide() {
        let text = "---\nkey: target\nname: 対象\ntype: choose\nparent:\ndepends_on: background\nmin_findings: 2\n---\n# 対象\n\n説明。\n\n## QUESTION\n- 何か\n\n## CONDITION\n- decided(target)\n\n## PITFALL\n- 広い\n";
        let g = parse_guide(text).unwrap();
        assert_eq!(g.questions, vec!["何か".to_string()]);
        assert_eq!(g.pitfalls, vec!["広い".to_string()]);
    }

    #[test]
    fn change_type_adds_only_missing() {
        let database = db();
        let conn = database.connection();
        let defs = defs();
        init_tree(conn, &defs, "improve").expect("改良駆動の骨組み");

        // 1 つだけ決定済みにしておく（変えないことの確かめ）。
        let target = store::find_item_by_key(conn, "target").unwrap().unwrap();
        store::set_node_state(conn, target.id, "decided").unwrap();

        let report = change_type(conn, &defs, "problem").expect("型を変える");
        // 改良駆動に無い一番上の項目は purpose だけ（子 3 個も一緒にできる）。
        assert_eq!(report.type_id, "problem");
        assert_eq!(report.created_items, 4);
        assert!(item_keys(conn).contains(&"purpose".to_string()));
        assert!(item_keys(conn).contains(&"purpose.subgoal".to_string()));
        // 既にある項目は足さない。
        assert_eq!(
            item_keys(conn)
                .iter()
                .filter(|key| *key == "target")
                .count(),
            1
        );
        // 状態は変えない。
        let after = store::find_item_by_key(conn, "target").unwrap().unwrap();
        assert_eq!(after.state, "decided");
        assert_eq!(
            store::meta_get(conn, "type").unwrap().as_deref(),
            Some("problem")
        );

        // 知らない型は通さない。
        let err = change_type(conn, &defs, "nope").unwrap_err();
        assert!(
            matches!(err, SciError::Invalid(message) if message.contains("知らない RESEARCH_TYPE です"))
        );
    }

    #[test]
    fn local_copy_overrides_embedded() {
        let root = temp_root("override");
        // 埋め込みの guides/data.md を、利用者が直した写しで上書きする。
        write_local(
            &root,
            "guides/data.md",
            "---\nkey: data\nname: データ（直した）\ntype: choose\n---\n# データ\n\n直した説明。\n\n## 問い\n- 直した問い\n\n## 埋まった条件\n- decided(data)\n\nこれは埋まった説明。\n",
        );
        // プロジェクトにだけあるガイド。
        write_local(
            &root,
            "guides/extra.md",
            "---\nkey: extra\nname: 足した項目\ntype: write\n---\n# 足した項目\n\n説明。\n\n## 問い\n- 問う\n\n## 埋まった条件\n- decided(extra)\n",
        );
        // 読めないファイル。
        write_local(
            &root,
            "types/broken.md",
            "---\nname: 壊れた型\n---\n# 壊れた型\n\nid が無い。\n",
        );

        let defs = load(&root);
        assert_eq!(defs.warnings.len(), 1, "{:?}", defs.warnings);
        assert!(defs.warnings[0].contains("types/broken.md"));

        let data = defs.guide("data").expect("データのガイド");
        assert_eq!(data.name, "データ（直した）");
        assert_eq!(data.questions, vec!["直した問い"]);
        assert_eq!(data.condition_text, "これは埋まった説明。");
        assert!(defs.guide("extra").is_some());
        // 上書きした data 以外のガイドは埋め込みのまま（書き換えの副作用がないことの確かめ）。
        assert_eq!(defs.guides.len(), 26);
        assert_eq!(defs.guide("novelty").unwrap().name, "NOVELTY");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn write_local_copies_writes_missing_only() {
        let root = temp_root("copies");
        assert_eq!(write_local_copies(&root).unwrap(), 30);
        // 2 回目は 1 本も書かない（利用者が直した内容を守る）。
        assert_eq!(write_local_copies(&root).unwrap(), 0);

        // 書けたものは読み込みで効く。
        let defs = load(&root);
        assert!(defs.warnings.is_empty(), "{:?}", defs.warnings);
        assert_eq!(defs.types.len(), 5);
        assert_eq!(defs.guides.len(), 25);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_condition_accepts_vocabulary_and_rejects_others() {
        assert_eq!(
            parse_condition("count(novelty.closest, 3) かつ decided(novelty.diff)").unwrap(),
            vec![
                Cond::Count("novelty.closest".to_string(), 3),
                Cond::Decided("novelty.diff".to_string())
            ]
        );
        assert_eq!(
            parse_condition("level(issue.limit, 2)").unwrap(),
            vec![Cond::Level("issue.limit".to_string(), 2)]
        );
        assert_eq!(
            parse_condition("mind(価値の軸)").unwrap(),
            vec![Cond::Mind("価値の軸".to_string())]
        );
        assert_eq!(
            parse_condition(
                "saturated(existing) かつ not_empty(background) かつ evidence(issue.root, 1)"
            )
            .unwrap(),
            vec![
                Cond::Saturated("existing".to_string()),
                Cond::NotEmpty("background".to_string()),
                Cond::Evidence("issue.root".to_string(), 1)
            ]
        );
        // 空文字は空の Vec。
        assert!(parse_condition("  ").unwrap().is_empty());

        // 語彙に無い書き方は通さない。
        for text in [
            "evidence>=2",
            "decided(background",
            "decided()",
            "count(novelty.closest)",
            "count(novelty.closest, 多く)",
            "wrote(background)",
        ] {
            let error = parse_condition(text).unwrap_err();
            assert!(
                matches!(&error, SciError::Invalid(message) if message.contains("条件を読めません")),
                "{text}: {error}"
            );
        }
    }

    #[test]
    fn create_item_rejects_duplicate_key() {
        let database = db();
        let conn = database.connection();
        let defs = defs();
        let id = create_item_from_guide(conn, &defs, "data", None, 0, "user").expect("項目を作る");
        assert!(id >= 1);
        // 子（data.type・data.size・data.label・data.access）も 4 つできる。
        assert_eq!(store::list_nodes(conn).unwrap().len(), 5);
        assert_eq!(store::list_edges(conn).unwrap().len(), 4);

        let error = create_item_from_guide(conn, &defs, "data", None, 1, "user").unwrap_err();
        assert!(matches!(error, SciError::Invalid(message) if message.contains("既にあります")));
        // 作り直ししても木は増えない。
        assert_eq!(store::list_nodes(conn).unwrap().len(), 5);
    }

    #[test]
    fn init_tree_rolls_back_on_failure() {
        let database = db();
        let conn = database.connection();

        // 項目の順序に同じ key を 2 回書いた型を作る。2 回目で失敗する。
        let mut type_def = parse_type(embedded_type("types/problem.md")).expect("型を読む");
        type_def.order.push("background".to_string());
        let broken = Defs {
            types: vec![type_def],
            guides: defs().guides,
            warnings: Vec::new(),
        };

        let error = init_tree(conn, &broken, "problem").unwrap_err();
        assert!(matches!(error, SciError::Invalid(_)), "{error}");
        // 失敗したあと、木も線も残さない。
        assert_eq!(store::list_nodes(conn).unwrap().len(), 0);
        assert!(store::list_edges(conn).unwrap().is_empty());
        assert_eq!(store::meta_get(conn, "type").unwrap(), None);
    }

    /// 埋め込みの 1 本の型を読み直す。
    fn embedded_type(relative: &str) -> &'static str {
        embedded()
            .iter()
            .find(|(path, _)| *path == relative)
            .map_or("", |(_, text)| *text)
    }
}
