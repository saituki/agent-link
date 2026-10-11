//! `sai-sci` の research-tree の命令。
//!
//! research-tree の計算と書き込みは `sai_sci_core::rt` が担う。ここは外から来る値の検査・
//! core の関数の呼び出し・出力の形だけを持つ。失敗は CLI 共通の約束に従う
//! （`main` が [`crate::cli_error::CliError`] と [`rt::RtError`] から終了コードを決める）。
//!
//! - `context` は本文をそのまま標準出力へ出す（sai-link が埋め込むため）

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use rusqlite::Connection;
use sai_paper_core::paths::{self, PaperRoots};
use sai_sci_core::rt::{self, card, compare, compute, context, defs, export, ops, seedq, store};
use sai_paper_core::search_history;
use sai_paper_core::{Config, Database, Paths};
use serde_json::{Value, json};

use crate::open_database;

/// 命令の出力（人が読む文か JSON）。
pub(crate) enum Out {
    /// 人が読む文（標準出力へ 1 行ずつ出す）。
    Text(String),
    /// JSON（標準出力へ 1 行で出す）。
    Json(Value),
}

/// `card` の引数（CLI から）。
#[derive(clap::Args, Debug)]
pub(crate) struct SciCardArgs {
    /// 対象の Node（Q・A・F・E・S・D。`Q12`）。省くと木の全体
    #[arg(long)]
    pub node: Option<String>,
    /// 実行するブロックの id
    #[arg(long)]
    pub block: Option<String>,
    /// 目的（1 行・300 字まで）
    #[arg(long)]
    pub purpose: Option<String>,
    /// 期待する成果
    #[arg(long)]
    pub expect: Option<String>,
    /// lab-read の型（purpose_first / blind_first）
    #[arg(long)]
    pub mode: Option<String>,
    /// 対象論文の番号（カンマ区切り）
    #[arg(long, value_delimiter = ',')]
    pub papers: Vec<i64>,
    /// 実行者（cc / user。`user` なら READ_CARD。省くと cc）
    #[arg(long, default_value = "cc")]
    pub runner: String,
    /// モデル（opus / sonnet / haiku / inherit。省くと settings.json の既定）
    #[arg(long)]
    pub model: Option<String>,
    /// 思考の量（none / low / medium / high / xhigh / max。省くと settings.json の既定）
    #[arg(long)]
    pub thinking: Option<String>,
    /// READ_CARD の深さ（abstract / figures / full）
    #[arg(long)]
    pub depth: Option<String>,
    /// 推薦した実行の run_id
    #[arg(long)]
    pub origin_run: Option<i64>,
    /// 依頼文に載せる文書の waggle トークン（繰り返し可）
    #[arg(long = "input")]
    pub input: Vec<String>,
    /// 足す子の型（Q・A・F。Q の下は Q か A、A の下は Q か F）
    #[arg(long)]
    pub child: Option<String>,
}

impl SciCardArgs {
    /// 引数を core の [`card::CardSpec`]（C-CARD）にする。
    pub(crate) fn to_spec(&self) -> Result<card::CardSpec> {
        // 省くと木の全体（`--node` は空）。廃止した SEED は core が断る。
        let node = self.node.as_deref().unwrap_or("").trim();
        let purpose = self.purpose.as_deref().unwrap_or("").trim();
        if purpose.is_empty() {
            return Err(
                crate::cli_error::CliError::input("--purpose を指定してください").into(),
            );
        }
        Ok(card::CardSpec {
            node: node.to_string(),
            block: self.block.clone().unwrap_or_default(),
            purpose: purpose.to_string(),
            expect: self.expect.clone().unwrap_or_default(),
            mode: self.mode.clone().unwrap_or_default(),
            papers: self.papers.clone(),
            runner: self.runner.clone(),
            model: self.model.clone().unwrap_or_default(),
            thinking: self.thinking.clone().unwrap_or_default(),
            depth: self.depth.clone().unwrap_or_default(),
            origin_run: self.origin_run,
            inputs: self.input.clone(),
            child: self.child.clone().unwrap_or_default(),
        })
    }
}

// ---- 共通の準備 -------------------------------------------------------------

/// sci の命令に必要なもの（DB・置き場・RESEARCH_TYPE と GUIDE の設定）。
pub(crate) struct SciEnv {
    /// papr.db（run の task.md を作る [`card::create_card`] が必要）。
    pub db: Database,
    /// 研究プロジェクトの置き場。
    pub roots: PaperRoots,
    /// RESEARCH_TYPE と GUIDE。
    pub defs: defs::Defs,
    /// settings.json。
    pub config: Config,
}

impl SciEnv {
    /// DB の接続。
    pub fn conn(&self) -> &Connection {
        self.db.connection()
    }

    /// research-tree を変えたら state.md と tree.md を書き直す。
    pub fn export(&self) -> Result<()> {
        export::export(self.conn(), &self.roots, &self.defs)
            .context("state.md と tree.md を書き出せませんでした")?;
        Ok(())
    }
}

/// 置き場・DB・設定・RESEARCH_TYPE と GUIDE を開く（`sci.rs` と [`crate::phone_sci`] で使い回す）。
pub(crate) fn open_env() -> Result<SciEnv> {
    let roots = paths::paper_roots().context("研究プロジェクト（.sai/paper）が見つかりません")?;
    let db = open_database()?;
    let config = Config::load_or_create(&Paths::discover()?)
        .context("settings.json を読み込めませんでした")?;
    let defs = defs::load(&roots.root);
    Ok(SciEnv {
        db,
        roots,
        defs,
        config,
    })
}

/// 研究プロジェクトのルート（`root`）を指定して [`open_env`] と同じものを開く。
///
/// 呼び出し側が決めた研究プロジェクトのルート（`root`）を使って [`open_env`] と同じものを開く。
/// ルートは引数で受け取る（環境変数に頼らない）。
pub(crate) fn open_env_at(root: &Path) -> Result<SciEnv> {
    let data_dir = root.join(".sai").join("paper");
    let roots = PaperRoots { root: root.to_path_buf(), data_dir: data_dir.clone() };
    let db = Database::open(&data_dir.join("papr.db")).context("papr.db を開けませんでした")?;
    let config = Config::load_or_create(&Paths {
        config_file: data_dir.join(sai_paper_core::config::SETTINGS_FILE),
        database_file: data_dir.join("papr.db"),
        downloads_dir: data_dir.join("papers"),
        plugins_dir: data_dir.join("plugins"),
        plugins_config_file: data_dir.join("plugins.toml"),
    })
    .context("settings.json を読み込めませんでした")?;
    let defs = defs::load(&roots.root);
    Ok(SciEnv {
        db,
        roots,
        defs,
        config,
    })
}

/// rt のエラーを日本語の文のまま anyhow にする。
fn rt_err(err: rt::RtError) -> anyhow::Error {
    // 型を保ったまま anyhow に載せる（main が `downcast_ref::<RtError>()` で終了コードを決める）。
    anyhow::Error::new(err)
}

// ---- 各命令 -----------------------------------------------------------------

/// `sci init` の返り値（表示と JSON に書く内容）。
pub(crate) struct InitOutcome {
    /// RESEARCH_TYPE の表示名（課題駆動など）。
    pub type_name: String,
    /// RESEARCH_TYPE そのもの（problem など）。
    pub type_id: String,
    /// 作った Q の数（一番上を含む）。
    pub created: usize,
    /// 見つかった注意点。
    pub warnings: Vec<String>,
    /// 残した元の `state.md`。
    pub backup: Option<PathBuf>,
    /// 配った部品の一覧（表示用）。
    pub assets: Vec<String>,
    /// 新しい research-tree が既にあったか。
    pub already: bool,
}

impl InitOutcome {
    /// 人が読む文（警告と、残した state.md の行も足す）。
    pub fn text(&self) -> String {
        let mut text = format!(
            "research-tree を作りました（RESEARCH_TYPE: {}・Q {}）\n",
            self.type_name, self.created
        );
        for warning in &self.warnings {
            text.push_str(&format!("⚠ {warning}\n"));
        }
        if let Some(path) = &self.backup {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "state.pre-sci.md".to_string());
            text.push_str(&format!("今の state.md を {name} に残しました\n"));
        }
        text
    }

    /// `--json` と `phone sci init` が返す JSON。
    pub fn json(&self) -> Value {
        json!({
            "type": self.type_id,
            "created": self.created,
            "warnings": self.warnings,
            "backup": self.backup.as_ref().map(|path| path.to_string_lossy().to_string()),
            "assets": self.assets,
        })
    }
}

/// research-tree を作って state.md と tree.md を書き出す（`sci init` の本体）。
///
/// 新しい research-tree が既にあるときは `already` が真の返り値を返し、エラーにしない。
/// そこから先をどうするかは呼び出し側が決める（`sai-sci init` はエラーにする）。
/// 古い research-tree だけがあるときは新しく作る
/// （古い表は触らない）。
///
/// # Errors
/// 研究プロジェクトを開けないとき、research-tree を作れないとき、書き出しに失敗したとき。
pub(crate) fn init_tree_and_export(root: &Path, type_id: &str) -> Result<InitOutcome> {
    let env = open_env_at(root)?;
    let type_name = type_name(&env.defs, type_id)?;
    if store::is_initialized(env.conn()).map_err(rt_err)? {
        // 作ってある research-tree の RESEARCH_TYPE の名前を出す（無ければ頼まれた型）。
        let existing = compute::status(env.conn(), &env.roots, &env.defs)
            .ok()
            .and_then(|status| status.type_info)
            .map_or(type_name, |found| found.name);
        return Ok(InitOutcome {
            type_name: existing,
            type_id: type_id.to_string(),
            created: 0,
            warnings: Vec::new(),
            backup: None,
            assets: Vec::new(),
            already: true,
        });
    }

    defs::write_local_copies(&env.roots.root)
        .context(".sai/sci/ に RESEARCH_TYPE と GUIDE を置けませんでした")?;
    let backup = export::backup_state_once(&env.roots.data_dir)
        .context("今の state.md を残せませんでした")?;
    let report = defs::init_tree(env.conn(), &env.defs, type_id, "user")
        .context("骨組みを作れませんでした")?;
    env.export()?;

    Ok(InitOutcome {
        type_name,
        type_id: type_id.to_string(),
        created: report.created,
        warnings: report.warnings,
        backup,
        assets: Vec::new(),
        already: false,
    })
}

/// 部品（ブロック・テンプレート・`context`・土壌のひな形）を配ってから research-tree を作る。
///
/// `sai-sci init` はここを呼ぶ。
///
/// # Errors
/// 部品を配れないとき、research-tree を作れないとき、書き出しに失敗したとき。
pub(crate) fn init_with_assets(root: &Path, type_id: &str) -> Result<InitOutcome> {
    // 部品を配る前に RESEARCH_TYPE を確かめる（知らない RESEARCH_TYPE で `.sai/` や `state.pre-sci.md` を残さない）。
    type_name(&defs::load(root), type_id).context("RESEARCH_TYPE を確かめられませんでした")?;
    let assets =
        crate::agent_assets::install_sci_here(root).context("sci の assets を配れませんでした")?;
    let mut outcome = init_tree_and_export(root, type_id)?;
    outcome.assets = assets;
    Ok(outcome)
}

/// `sci status` — 現況。`limit` は Next Action の件数（省くと 10、0 は全部）。
pub(crate) fn status(json: bool, brief: bool, limit: Option<usize>) -> Result<Out> {
    let env = open_env()?;
    let limit = match limit {
        Some(0) => usize::MAX,
        Some(n) => n,
        None => 10,
    };
    let status = compute::status_with_limit(env.conn(), &env.roots, &env.defs, limit)
        .context("現況を集められませんでした")?;
    // --brief は --json と一緒でも単独でもよく、いつも JSON。
    if brief {
        return Ok(Out::Json(brief_status(&status)));
    }
    if json {
        return Ok(Out::Json(serde_json::to_value(&status)?));
    }
    Ok(Out::Text(status_text(&status)))
}

/// `sci status --brief`（`{"brief": true}`）が返す JSON。
///
/// 短い文脈で現在地を読めるように、Node の数・Next Action だけを出す。
/// 値は [`compute::status`] の結果からそのまま写すだけで、計算し直さない
/// （Web と CLI で同じ値になる）。
pub fn brief_status(status: &compute::Status) -> Value {
    json!({
        "type": status.type_info.as_ref().map(|found| found.id.clone()),
        "counts": status.counts,
        "next": status.next.iter().map(|next| json!({
            "kind": next.kind,
            "node": next.node,
            "option": next.option,
            "block": next.block,
            "purpose": next.purpose,
            "expect": next.expect,
            "why": next.why,
        })).collect::<Vec<Value>>(),
        "running": &status.running,
        "warnings": &status.warnings,
    })
}

/// `sci node <ID>` — Node 1 つの詳細。
pub(crate) fn node_detail(node: &str, json: bool) -> Result<Out> {
    let env = open_env()?;
    let detail = compute::node_detail(env.conn(), &env.roots, &env.defs, node.trim())
        .map_err(rt_err)?;
    if json {
        return Ok(Out::Json(serde_json::to_value(&detail)?));
    }
    Ok(Out::Text(node_text(&detail)))
}

/// `sci compare <S>… [--json]` — 複数の S を軸ごとに並べる（2〜10 個）。
pub(crate) fn compare(sprouts: &[String], json: bool) -> Result<Out> {
    let env = open_env()?;
    let table = rt::compare::compare(env.conn(), &env.roots, &env.defs, sprouts).map_err(rt_err)?;
    if json {
        return Ok(Out::Json(serde_json::to_value(&table)?));
    }
    Ok(Out::Text(compare_text(&table)))
}

/// `sci context --run <id>` — 実行のための入力（sai-link が本文へ埋め込む）。
///
/// `reveal` が真（`--tree`）なら blind でも research-tree を出す。
pub(crate) fn context(run: i64, reveal: bool) -> Result<Out> {
    let env = open_env()?;
    let text = context::build(
        env.conn(),
        &env.roots,
        &env.defs,
        env.config.sci.context_max_chars,
        run,
        reveal,
    )
    .map_err(rt_err)?;
    Ok(Out::Text(text))
}

/// `sci op` — agent の op を 1 つのトランザクションで行う（いつも JSON）。
pub(crate) fn op(run: Option<i64>, block: Option<String>) -> Result<Out> {
    let env = open_env()?;
    let input = read_stdin_json()?;
    let actor = actor_of(run, block);
    let batch = ops::op(env.conn(), &env.roots, &env.defs, &actor, &input).map_err(rt_err)?;
    env.export()?;
    Ok(Out::Json(serde_json::to_value(batch)?))
}

/// `sci apply` — 利用者の操作を 1 つ反映する（いつも JSON）。
/// 記録の actor は、`SAI_PARENT_RUN_ID` があれば `run:<id>`（Professor の代行）、無ければ `user`。
pub(crate) fn apply() -> Result<Out> {
    let env = open_env()?;
    let input = read_stdin_json()?;
    let by = match parent_run_id().filter(|id| *id > 0) {
        Some(run_id) => format!("run:{run_id}"),
        None => "user".to_string(),
    };
    let result = ops::apply_user(env.conn(), &env.roots, &env.defs, &input, &by).map_err(rt_err)?;
    env.export()?;
    Ok(Out::Json(serde_json::to_value(result)?))
}

/// `sci approval record` — 質問の回答を承認として 1 件記録する（いつも JSON）。
pub(crate) fn approval_record(
    qid: &str,
    question: &str,
    answer: &str,
    answered_by: &str,
    run: Option<i64>,
) -> Result<Out> {
    let Some(run_id) = run.or_else(parent_run_id) else {
        return Err(crate::cli_error::CliError::input("run_id がありません（--run か SAI_PARENT_RUN_ID）").into());
    };
    let env = open_env()?;
    let id = ops::record_approval(env.conn(), run_id, qid, question, answer, answered_by).map_err(rt_err)?;
    Ok(Out::Json(json!({
        "approval": {"id": id, "run_id": run_id, "qid": qid, "consumable": answer.trim() == "許可する"}
    })))
}

/// CARD を作って state.md と tree.md を書き直し、`{"run_id","path"}` を返す
/// （`sci card` と `phone sci card` で共用）。
pub(crate) fn make_card(spec: &card::CardSpec) -> Result<Value> {
    let env = open_env()?;
    let result =
        card::create_card(&env.db, &env.roots, &env.defs, spec).map_err(rt_err)?;
    env.export()?;
    Ok(json!({
        "run_id": result.run_id,
        "path": result.path.to_string_lossy(),
    }))
}

/// `sci card` — Node のための実行（CARD）を作る。
pub(crate) fn card(spec: &card::CardSpec, json: bool) -> Result<Out> {
    let value = make_card(spec)?;
    if json {
        return Ok(Out::Json(value));
    }
    Ok(Out::Text(format!(
        "run {} の CARD を作りました: {}\n",
        value["run_id"],
        value["path"].as_str().unwrap_or("")
    )))
}

/// `sci read <run_id>` — READ_CARD の中身。
pub(crate) fn read(run_id: i64, json: bool) -> Result<Out> {
    let env = open_env()?;
    let card =
        card::read_card(env.conn(), &env.roots, &env.defs, run_id).map_err(rt_err)?;
    if json {
        return Ok(Out::Json(serde_json::to_value(card)?));
    }
    Ok(Out::Text(read_text(&card)))
}

/// `sci export` — state.md と tree.md を書き出す。
pub(crate) fn export() -> Result<Out> {
    let env = open_env()?;
    if export::export(env.conn(), &env.roots, &env.defs).context("書き出せませんでした")? {
        Ok(Out::Text("書き出しました: state.md, tree.md\n".to_string()))
    } else {
        Ok(Out::Text("research-tree がありません\n".to_string()))
    }
}

/// RESEARCH_TYPE の一覧の JSON（`{"types":[{"id","name"}]}`。`sci types` と `phone sci types` で共用）。
pub(crate) fn types_json(defs: &defs::Defs) -> Value {
    let list: Vec<Value> = defs
        .types
        .iter()
        .map(|def| json!({ "id": def.id, "name": def.name }))
        .collect();
    json!({ "types": list })
}

/// `sci types` — 使える RESEARCH_TYPE の一覧。
pub(crate) fn types(json: bool) -> Result<Out> {
    let roots = paths::paper_roots().context("研究プロジェクト（.sai/paper）が見つかりません")?;
    let defs = defs::load(&roots.root);
    if json {
        return Ok(Out::Json(types_json(&defs)));
    }
    let mut text = String::new();
    for def in &defs.types {
        text.push_str(&format!("{} {}\n", def.id, def.name));
    }
    Ok(Out::Text(text))
}

/// `sci type-save` — 標準入力の RESEARCH_TYPE の Markdown を保存する（いつも JSON）。
pub(crate) fn type_save() -> Result<Out> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|err| anyhow!("標準入力を読めません: {err}"))?;
    if text.trim().is_empty() {
        return Err(crate::cli_error::CliError::input("標準入力に RESEARCH_TYPE の Markdown がありません").into());
    }
    let path = defs::save_user_type(&text).map_err(rt_err)?;
    Ok(Out::Json(json!({ "path": path.to_string_lossy() })))
}

// ---- 標準入力と外から来る値 -------------------------------------------------

/// 標準入力を全部読んで JSON にする（読めなければ日本語の文でエラー）。
pub(crate) fn read_stdin_json() -> Result<Value> {
    parse_stdin_text(&read_stdin_text()?)
}

/// 標準入力を全部読む（読めなければ入力の誤り）。
pub(crate) fn read_stdin_text() -> Result<String> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|err| crate::cli_error::CliError::input(format!("標準入力を読めません: {err}")))?;
    Ok(text)
}

/// 標準入力のテキストを JSON にする（空か読めなければ日本語の文でエラー）。
pub(crate) fn parse_stdin_text(text: &str) -> Result<Value> {
    if text.trim().is_empty() {
        return Err(crate::cli_error::CliError::input("標準入力の JSON を読めません").into());
    }
    serde_json::from_str(text).map_err(|err| {
        crate::cli_error::CliError::input(format!("標準入力の JSON を読めません: {err}")).into()
    })
}

/// sai-link が付けた実行 ID（無ければ `None`）。
pub(crate) fn parent_run_id() -> Option<i64> {
    std::env::var("SAI_PARENT_RUN_ID")
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok())
}

/// PROPOSAL の実行者を作る（`--run` / `--block`、無ければ環境変数）。
pub(crate) fn actor_of(run: Option<i64>, block: Option<String>) -> ops::Actor {
    let block = block
        .or_else(|| std::env::var("SAI_BLOCK_NAME").ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    ops::Actor {
        run_id: run.or_else(parent_run_id),
        block,
    }
}

// ---- Node に結び付いた検索の記録（find / search） ------------------------

/// 検索が結び付いた Node（`find` と `search` の `--node`）。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SearchRefs {
    /// 検索した Node の `rt_nodes.id`（検索の範囲を狭めるのにも使う）。
    pub node: Option<i64>,
}

/// `find` / `search` の引数検査。
///
/// `--node` は `Q12`・`A5` のような形だけを受け付ける。指定が無ければ何も調べない
/// （research-tree が無いプロジェクトでも `find` / `search` を今までどおり使えるようにするため）。
pub(crate) fn parse_search_refs(conn: &Connection, node: Option<&str>) -> Result<SearchRefs> {
    let Some(text) = node else {
        return Ok(SearchRefs::default());
    };
    let text = text.trim();
    if !store::is_initialized(conn).map_err(rt_err)? {
        return Err(crate::cli_error::CliError::input(
            "research-tree がありません（sai-sci init）",
        )
        .into());
    }
    let (kind, id) = rt::parse_id(text)
        .map_err(|err| crate::cli_error::CliError::input(format!("{err}")))?;
    let alive = store::get_node_of(conn, kind, id)
        .ok()
        .is_some_and(|row| row.removed_at.is_none());
    if !alive {
        return Err(crate::cli_error::CliError::not_found(format!("{text} はありません")).into());
    }
    Ok(SearchRefs { node: Some(id) })
}

/// 環境変数 `SAI_TARGET_NODE` の Node（検索の記録の結び先）。
///
/// 無い・空なら `None`。形が違う・Node が無い・消えている・research-tree が無いときは、
/// 検索そのものを止めず、標準エラーに 1 行出して `None` にする。
pub(crate) fn target_node_from_env(conn: &Connection) -> Option<i64> {
    let raw = std::env::var("SAI_TARGET_NODE").ok()?;
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    let found = store::is_initialized(conn)
        .ok()
        .filter(|ready| *ready)
        .and_then(|_| rt::parse_id(text).ok())
        .and_then(|(kind, id)| {
            store::get_node_of(conn, kind, id)
                .ok()
                .filter(|row| row.removed_at.is_none())
                .map(|_| id)
        });
    if found.is_none() {
        eprintln!("SAI_TARGET_NODE の {text} はありません。検索を Node に結びません");
    }
    found
}

/// 検索の記録に残す内容（`find` / `search` で同じものを使う）。
pub(crate) struct SearchLog<'a> {
    /// 検索が結び付いた Node。
    pub refs: &'a SearchRefs,
    /// `find` / `search` / `expand`。
    pub tool: &'a str,
    /// 検索語。
    pub query: &'a str,
    /// 検索の範囲（`node:Q12` など）。
    pub scope: &'a str,
    /// 当たった件数。
    pub hits: i64,
    /// 当たった論文の番号。
    pub hit_papers: &'a [i64],
    /// 使わない（常に `None`。廃止した検索セットの番号の列 `rt_searches.set_id`）。
    pub set_id: Option<i64>,
    /// その 1 回の検索の履歴（`search_history` の行）の番号。
    pub log_id: Option<i64>,
}

/// 検索の記録（`rt_searches` に 1 行）を残す。
///
/// Node が無いときは記録しない（`find` / `search` を今までどおり使えるようにするため）。
pub(crate) fn record_search(conn: &Connection, log: &SearchLog<'_>) -> Result<()> {
    if log.refs.node.is_none() {
        return Ok(());
    }
    store::insert_search(
        conn,
        &store::SearchRow {
            // id と created_at は表が振る。
            id: 0,
            run_id: parent_run_id(),
            node_id: log.refs.node,
            tool: log.tool.to_string(),
            query: log.query.to_string(),
            scope: log.scope.to_string(),
            hits: log.hits,
            hit_papers: log.hit_papers.to_vec(),
            set_id: log.set_id,
            created_at: String::new(),
            log_id: log.log_id,
        },
    )
    .map(|_| ())
    .context("検索の記録を残せませんでした")
}

/// 同じ Node・同じ範囲の前回の検索を 1 行にする（無ければ `None`）。
///
/// Node が無いときは「前」を見せない（何が前かが決まらないため）。
pub(crate) fn previous_search_line(
    conn: &Connection,
    refs: &SearchRefs,
    scope: &str,
) -> Option<String> {
    let node = refs.node?;
    let rows = store::list_searches(conn, Some(node)).ok()?;
    let found = rows.iter().find(|row| row.scope == scope)?;
    let papers = if found.hit_papers.is_empty() {
        String::new()
    } else {
        let list = found
            .hit_papers
            .iter()
            .map(|id| format!("#{id}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("（論文 {list}）")
    };
    Some(format!(
        "前回の検索: {}「{}」{} 件{papers}",
        found.created_at, found.query, found.hits
    ))
}

/// `find` の範囲の引数の組み合わせを検査する（CLI で共用）。
///
/// Node や論文の集合が無いのに `--subtree` / `--each` が来たときは、黙って Library
/// 全体を探すことにせず、文で伝える。
pub(crate) fn check_find_scope(
    subtree: bool,
    node_given: bool,
    each: Option<i64>,
    papers_empty: bool,
) -> Result<()> {
    if subtree && !node_given {
        return Err(crate::cli_error::CliError::input(
            "--subtree は --node と一緒に使います",
        )
        .into());
    }
    if each.is_some() && papers_empty && !node_given {
        return Err(crate::cli_error::CliError::input(
            "--each は --papers か --node と一緒に使います",
        )
        .into());
    }
    Ok(())
}

/// 検索結果から論文の番号を取り出す（重複なし・昇順。検索の記録に残す）。
///
/// CLI の find で共用（U01）。
pub(crate) fn hit_paper_ids(hits: &[sai_paper_core::Hit]) -> Vec<i64> {
    let mut ids: Vec<i64> = hits.iter().filter_map(|hit| hit.paper_id).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// 検索履歴に何のための検索かを書く Fields を作る（1 つも無ければ `None`）。
pub(crate) fn set_fields(
    refs: &SearchRefs,
    purpose: Option<&str>,
    extract: Option<&str>,
) -> Option<search_history::SciSetFields> {
    if refs.node.is_none() && purpose.is_none() && extract.is_none() {
        return None;
    }
    Some(search_history::SciSetFields {
        node: refs.node,
        question: None,
        purpose: purpose.map(str::to_string),
        extract: extract.map(str::to_string),
    })
}

// ---- 人が読む文 -------------------------------------------------------------

/// `sci status` の文（RESEARCH_TYPE・Node の数・seed（D）・research-tree・Next Action）。
fn status_text(status: &compute::Status) -> String {
    if !status.initialized {
        let mut text = "research-tree がまだありません。`sai-sci init --type <型>` で始めてください。\n"
            .to_string();
        for warning in &status.warnings {
            text.push_str(&format!("⚠ {warning}\n"));
        }
        return text;
    }
    let mut text = String::new();
    if let Some(found) = &status.type_info {
        text.push_str(&format!("RESEARCH_TYPE: {}\n", found.name));
    }
    text.push_str(&format!("木: {}\n", status.counts.line()));
    let seeds: Vec<&compute::NodeJson> = status.nodes.iter().filter(|n| n.kind == "D").collect();
    if seeds.is_empty() {
        text.push_str("seed（D）はまだありません\n");
    } else {
        let list = seeds.iter().map(|n| format!("{} {}", n.id, one_line(&n.text))).collect::<Vec<_>>().join(" / ");
        text.push_str(&format!("seed（D）: {list}\n"));
    }
    let sprouts: Vec<&compute::NodeJson> = status.nodes.iter().filter(|n| n.kind == "S").collect();
    if sprouts.is_empty() {
        text.push_str("SPROUT はまだありません\n");
    } else {
        let list = sprouts
            .iter()
            .map(|n| format!("{} {}（seed: {}）", n.id, one_line(&n.text), n.parent.as_deref().unwrap_or("なし")))
            .collect::<Vec<_>>()
            .join("、");
        text.push_str(&format!("SPROUT: {list}\n"));
    }
    for node in &status.nodes {
        text.push_str(&node_line(node));
    }
    if !status.next.is_empty() {
        text.push_str("Next Action:\n");
        for (index, next) in status.next.iter().enumerate() {
            text.push_str(&format!("{}. {}\n", index + 1, next_text(next)));
        }
    }
    for warning in &status.warnings {
        text.push_str(&format!("⚠ {warning}\n"));
    }
    text
}

/// Next Action 1 件の文（Node・ブロック・目的・理由）。
fn next_text(next: &compute::NextAction) -> String {
    let mut head = String::new();
    for part in [&next.node, &next.block].into_iter().flatten() {
        head.push_str(part);
        head.push(' ');
    }
    format!("{head}{}（{}）", next.purpose, next.why)
}

/// F の立場の印（support は S、deny は D、issue は I）。
fn stance_mark(stance: &str) -> &str {
    match stance {
        "support" => "S",
        "deny" => "D",
        "issue" => "I",
        other => other,
    }
}

/// `sci compare` の表（行が S、列が 6 軸。全部の S で違う軸の見出しに `*`）。
fn compare_text(table: &compare::Compare) -> String {
    let mut head = vec!["S".to_string()];
    head.extend(table.axes.iter().map(|key| if table.same.get(key).copied().unwrap_or(true) { key.clone() } else { format!("{key}*") }));
    let mut lines = vec![format!("| {} |", head.join(" | ")), format!("|{}", "---|".repeat(head.len()))];
    for s in &table.sprouts {
        let mut row = vec![format!("{} {}", s.id, one_line(&s.title))];
        row.extend(table.axes.iter().map(|key| compare_cell(&s.cells[key])));
        lines.push(format!("| {} |", row.join(" | ")));
    }
    lines.join("\n") + "\n"
}

/// 比べる表の 1 欄（`A12 短い文 [固定]`・`短い答え [新規]`・`（空欄）`）。
fn compare_cell(cell: &compare::Cell) -> String {
    let label = if cell.role == "fixed" { "固定" } else { "新規" };
    let text = match &cell.a {
        Some(a) => {
            let more = cell.more.iter().map(|m| format!(" +{m}")).collect::<String>();
            format!("{a} {} [{label}]{more}", short(&cell.text))
        }
        None if cell.role != "open" => format!("{} [{label}]", short(&cell.text)),
        None => "（空欄）".to_string(),
    };
    text.replace('|', "\\|")
}

/// Node 1 つの 1 行（深さ分だけ字下げする。★ は種類によらず付け、F には立場の印、S には範囲の数を付ける）。
fn node_line(node: &compute::NodeJson) -> String {
    let mut line = format!("{}- {} ", "  ".repeat(node.depth), node.id);
    if node.starred {
        line.push_str("★ ");
    }
    if let Some(stance) = &node.stance {
        line.push_str(&format!("[{}] ", stance_mark(stance)));
    }
    line.push_str(&one_line(&node.text));
    if let Some(range) = &node.range {
        line.push_str(&format!(" [範囲: {}]", range.line()));
    }
    line.push('\n');
    line
}

/// 文を 1 行にして、30 字で切る（道の A の文）。
fn short(text: &str) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= 30 {
        return one;
    }
    format!("{}…", one.chars().take(30).collect::<String>())
}

/// A の道（`Q1 › A7 › Q20 › A12`）。道が無ければ空。
fn chain_text(detail: &compute::NodeDetail, a: &str) -> String {
    detail.chains.get(a).map_or_else(String::new, |items| items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>().join(" › "))
}

/// A の文（30 字）。道が無ければ空。
fn anchor_text(detail: &compute::NodeDetail, a: &str) -> String {
    detail.chains.get(a).and_then(|items| items.last()).map_or_else(String::new, |item| short(&item.text))
}

/// `（…）` の付け方。中身が空なら何も付けない。
fn paren(inner: &[String]) -> String {
    let inner: Vec<&str> = inner.iter().map(String::as_str).filter(|s| !s.is_empty()).collect();
    if inner.is_empty() { String::new() } else { format!("（{}）", inner.join("。")) }
}

/// 節を 1 つ足す（中身が無ければ何も足さない）。
fn push_section(out: &mut String, heading: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    out.push_str(&format!("## {heading}\n"));
    for line in lines {
        out.push_str(&format!("{line}\n"));
    }
}

/// 問いの回答 1 件の行（`- SQ3 <問い>: A12 <A の文>（<道>）` か `- SQ3 <問い>: 「<テキスト>」`）。
fn answer_line(detail: &compute::NodeDetail, q: &seedq::QuestionJson, a: &seedq::AnswerJson) -> String {
    let body = match (&a.a, &a.text) {
        (Some(id), _) => format!("{id} {}{}", anchor_text(detail, id), paren(&[chain_text(detail, id)])),
        (None, Some(text)) => format!("「{}」", one_line(text)),
        (None, None) => String::new(),
    };
    format!("- {} {}: {body}", q.id, one_line(&q.text))
}

/// S の `sci node` の節（seed（固定）・新規性（sprout の回答）・未回答・実験（E））。空の節は出さない。
fn sprout_sections(detail: &compute::NodeDetail) -> String {
    let mut out = String::new();
    let seed_answers: Vec<String> = detail
        .seed
        .as_ref()
        .map_or_else(Vec::new, |seed| {
            seed.questions
                .iter()
                .flat_map(|q| q.answers.iter().filter(|a| a.owner.starts_with('D')).map(move |a| answer_line(detail, q, a)))
                .collect()
        });
    let novel: Vec<String> = detail
        .node
        .questions
        .iter()
        .flat_map(|q| q.answers.iter().filter(|a| a.owner.starts_with('S')).map(move |a| answer_line(detail, q, a)))
        .collect();
    let missing: Vec<String> = detail
        .node
        .missing
        .iter()
        .map(|id| {
            let text = detail.node.questions.iter().find(|q| &q.id == id).map_or("", |q| q.text.as_str());
            format!("- {id} {}", one_line(text))
        })
        .collect();
    let experiments: Vec<String> = detail
        .subtree
        .iter()
        .filter(|n| n.kind == "E")
        .map(|e| {
            let mlflow = e.mlflow.as_deref().filter(|m| !m.is_empty()).map_or_else(String::new, |m| format!("（MLflow: {m}）"));
            format!("- {} {}{mlflow}", e.id, one_line(&e.text))
        })
        .collect();
    let seed_head = detail.seed.as_ref().map_or_else(|| "seed（固定）".to_string(), |seed| format!("seed（固定）  {} {}", seed.id, one_line(&seed.text)));
    push_section(&mut out, &seed_head, &seed_answers);
    push_section(&mut out, "新規性（sprout の回答）", &novel);
    push_section(&mut out, "未回答", &missing);
    push_section(&mut out, "実験（E）", &experiments);
    out
}

/// `sci node <ID>` の文（題・S の節・PATH・children・GUIDE・DECISIONS・検索）。
fn node_text(detail: &compute::NodeDetail) -> String {
    let node = &detail.node;
    let mut text = format!("{} {}\n", node.id, one_line(&node.text));
    if node.kind == "S" {
        text.push_str(&sprout_sections(detail));
        if !node.body.is_empty() {
            text.push_str(&format!("## 本文\n{}\n", node.body));
        }
    } else if !node.body.is_empty() {
        text.push_str(&format!("\n{}\n", node.body));
    }
    if !detail.path.is_empty() {
        let path = detail
            .path
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>()
            .join(" > ");
        text.push_str(&format!("\nPATH: {path}\n"));
    }
    if let Some(guide) = &detail.guide {
        text.push_str(&format!(
            "\nGUIDE {}: {}\n",
            guide.name, guide.explain,
        ));
    }
    if !detail.children.is_empty() {
        text.push_str("\nchildren:\n");
        for child in &detail.children {
            text.push_str(&format!("  {}", node_line(child).trim_start_matches("- ")));
        }
    }
    if !detail.decisions.is_empty() {
        text.push_str("\nDECISIONS:\n");
        for decision in &detail.decisions {
            let mut line = format!("  {} {} {}", decision.at, decision.action, decision.actor);
            if !decision.detail.is_empty() {
                line.push_str(&format!(" {}", one_line(&decision.detail)));
            }
            if let Some(by) = &decision.approved_by {
                line.push_str(&format!(" 承認: {by}"));
            }
            text.push_str(&line);
            text.push('\n');
        }
    }
    if !detail.searches.is_empty() {
        text.push_str("\n検索:\n");
        for search in &detail.searches {
            text.push_str(&format!(
                "  {} {}「{}」{} 件\n",
                search.created_at, search.tool, search.query, search.hits
            ));
        }
    }
    if !detail.next.is_empty() {
        text.push_str("\nNext Action:\n");
        for (index, next) in detail.next.iter().enumerate() {
            text.push_str(&format!("  {}. {}\n", index + 1, next_text(next)));
        }
    }
    text
}

/// `sci read <run_id>` の文（論文・深さ・目的・読むときの Q・抜き出すもの・F）。
fn read_text(card: &card::ReadCard) -> String {
    let field = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let mut text = format!(
        "run {}（{}）{} {}\n",
        card.run_id,
        card.owner,
        field(&card.node, "id"),
        one_line(&field(&card.node, "text"))
    );
    let year = field(&card.paper, "year");
    let year = if year.is_empty() { String::new() } else { format!("（{year}）") };
    text.push_str(&format!(
        "論文: #{} {}{}\n",
        card.paper.get("id").map_or_else(String::new, ToString::to_string),
        one_line(&field(&card.paper, "title")),
        year
    ));
    text.push_str(&format!("深さ: {}\n", card.depth));
    if !card.purpose.is_empty() {
        text.push_str(&format!("目的: {}\n", one_line(&card.purpose)));
    }
    if !card.questions.is_empty() {
        text.push_str("読むときの Q:\n");
        for question in &card.questions {
            text.push_str(&format!("  {question}\n"));
        }
    }
    if !card.extract.is_empty() {
        text.push_str("抜き出すもの:\n");
        for item in &card.extract {
            text.push_str(&format!("  {item}\n"));
        }
    }
    if !card.findings.is_empty() {
        text.push_str("F:\n");
        for finding in &card.findings {
            text.push_str(&format!(
                "  {} {}\n",
                field(finding, "id"),
                one_line(&field(finding, "text"))
            ));
        }
    }
    text
}

/// RESEARCH_TYPE の表示名。
pub(crate) fn type_name(defs: &defs::Defs, type_id: &str) -> Result<String> {
    defs.type_def(type_id)
        .map(|def| def.name.clone())
        .ok_or_else(|| {
            let known: Vec<&str> = defs.types.iter().map(|def| def.id.as_str()).collect();
            anyhow!(
                "知らない RESEARCH_TYPE です: {type_id}（{}）",
                if known.is_empty() {
                    "RESEARCH_TYPE が読み込まれていません".to_string()
                } else {
                    known.join(" / ")
                }
            )
        })
}

/// 改行を空白に、長過ぎれば切る（1 行に出すため）。
fn one_line(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= 80 {
        return joined;
    }
    let head: String = joined.chars().take(79).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一時フォルダに研究プロジェクトの置き場と、新しい research-tree を作った DB を用意する。
    fn project_with_tree(name: &str) -> (PathBuf, SciEnv) {
        let root = std::env::temp_dir().join(format!("sai-paper-sci-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let data_dir = root.join(".sai").join("paper");
        std::fs::create_dir_all(&data_dir).unwrap();
        let db = Database::open(&data_dir.join("papr.db")).unwrap();
        let roots = PaperRoots { root: root.clone(), data_dir };
        let defs = defs::load(&root);
        defs::init_tree(db.connection(), &defs, "problem", "user").unwrap();
        let env = SciEnv { db, roots, defs, config: Config::default() };
        (root, env)
    }

    /// `find` の範囲の引数は `--node` / `--papers` が要る（`--subtree` と `--each`）。
    #[test]
    fn find_subtree_requires_node() {
        assert!(check_find_scope(true, true, Some(2), true).is_ok());
        assert!(check_find_scope(false, false, Some(2), false).is_ok());
        assert!(check_find_scope(false, false, None, true).is_ok());

        for (subtree, each, papers_empty, message) in [
            (true, None, true, "--subtree は --node と一緒に使います"),
            (false, Some(2), true, "--each は --papers か --node と一緒に使います"),
        ] {
            let err = check_find_scope(subtree, false, each, papers_empty).unwrap_err();
            assert_eq!(format!("{err:#}"), message);
        }
    }

    /// 新しい research-tree の status は、文にも `--brief` の JSON にも同じ値が出る。
    #[test]
    fn status_text_and_brief_follow_the_tree() {
        let (root, env) = project_with_tree("status");
        let status = compute::status(env.conn(), &env.roots, &env.defs).unwrap();

        let text = status_text(&status);
        assert!(text.contains("RESEARCH_TYPE: "), "{text}");
        assert!(text.contains("seed（D）はまだありません"), "{text}");
        assert!(text.contains("木: Q "), "{text}");
        assert!(text.contains("- Q1 "), "{text}");
        assert!(!text.contains("Objectivity"), "{text}");

        let brief = brief_status(&status);
        assert_eq!(brief["type"], "problem");
        assert!(brief["counts"]["q"].as_i64().unwrap() > 0, "{brief}");
        assert!(brief.get("seed_draft").is_none(), "{brief}");
        for key in ["counts", "next", "running", "warnings"] {
            assert!(brief.get(key).is_some(), "{key}: {brief}");
        }
        // research-tree そのものは入れない。
        assert!(brief.get("nodes").is_none(), "{brief}");

        // research-tree が無いときは始め方を案内する。
        let empty = status_text(&compute::Status { initialized: false, ..status });
        assert!(empty.contains("まだありません"), "{empty}");
        let _ = std::fs::remove_dir_all(root);
    }

    fn new_node(kind: rt::Kind, parent_id: Option<i64>, text: &str) -> store::NewNode<'_> {
        store::NewNode {
            kind, parent_id, guide_key: None, text, body: "", stance: None, cites: &[],
            paper_id: None, run_id: None, report_id: None, mlflow_ref: None, dataset_ref: None, paths: &[], created_by: "user",
        }
    }

    /// seed（D）は `seed（D）:` の 1 行にまとめ、SPROUT の行には親の seed を付ける。親の無い古い SPROUT は `seed: なし`。
    #[test]
    fn status_text_shows_seed_and_sprout_parent() {
        let (root, env) = project_with_tree("seed-text");
        let d = store::insert_node(env.conn(), &new_node(rt::Kind::D, None, "自分の研究")).unwrap();
        let s = store::insert_node(env.conn(), &new_node(rt::Kind::S, Some(d), "一つ目")).unwrap();
        let old = store::insert_node(env.conn(), &new_node(rt::Kind::S, None, "古い研究")).unwrap();
        let status = compute::status(env.conn(), &env.roots, &env.defs).unwrap();

        let text = status_text(&status);
        let (d, s, old) = (rt::fmt_id(rt::Kind::D, d), rt::fmt_id(rt::Kind::S, s), rt::fmt_id(rt::Kind::S, old));
        assert!(text.contains(&format!("seed（D）: {d} 自分の研究\n")), "{text}");
        assert!(text.contains(&format!("SPROUT: {s} 一つ目（seed: {d}）、{old} 古い研究（seed: なし）\n")), "{text}");
        let _ = std::fs::remove_dir_all(root);
    }

    /// 検索の記録は Node があるときだけ残り、同じ範囲の前回が 1 行になる。
    #[test]
    fn search_record_and_previous_line() {
        let (root, env) = project_with_tree("search");
        let conn = env.conn();

        // Node が無い指定は断る。
        let err = parse_search_refs(conn, Some("Q999")).unwrap_err();
        assert_eq!(format!("{err:#}"), "Q999 はありません");
        assert!(parse_search_refs(conn, Some("12")).is_err());
        // 指定が無ければ記録しない。
        let none = parse_search_refs(conn, None).unwrap();
        assert!(none.node.is_none());

        let refs = parse_search_refs(conn, Some("Q1")).unwrap();
        assert_eq!(refs.node, Some(1));
        assert!(previous_search_line(conn, &refs, "node:Q1").is_none());
        record_search(
            conn,
            &SearchLog {
                refs: &refs,
                tool: "find",
                query: "発作",
                scope: "node:Q1",
                hits: 2,
                hit_papers: &[7, 9],
                set_id: None,
                log_id: None,
            },
        )
        .unwrap();
        let line = previous_search_line(conn, &refs, "node:Q1").unwrap();
        assert!(line.contains("「発作」2 件（論文 #7, #9）"), "{line}");
        assert!(previous_search_line(conn, &refs, "node:A1").is_none());
        record_search(
            conn,
            &SearchLog {
                refs: &none,
                tool: "find",
                query: "x",
                scope: "library",
                hits: 0,
                hit_papers: &[],
                set_id: None,
                log_id: None,
            },
        )
        .unwrap();
        assert_eq!(store::list_searches(conn, None).unwrap().len(), 1);

        let fields = set_fields(&refs, Some("目的"), None).unwrap();
        assert_eq!(fields.node, Some(1));
        assert!(set_fields(&none, None, None).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    /// research-tree が始まっていないプロジェクトで `--node` を指定すると、始め方を案内する。
    #[test]
    fn search_refs_need_a_started_tree() {
        let db = Database::in_memory().unwrap();
        let err = parse_search_refs(db.connection(), Some("Q1")).unwrap_err();
        assert!(format!("{err:#}").contains("research-tree がありません"), "{err:#}");
    }

    #[test]
    fn read_stdin_json_rejects_empty_and_broken() {
        assert!(parse_stdin_text("").is_err());
        assert!(parse_stdin_text("  \n").is_err());
        assert!(parse_stdin_text("{oops").is_err());
        let ok = parse_stdin_text("{\"op\":\"star\"}").unwrap();
        assert_eq!(ok["op"], "star");
    }

    /// 知らない RESEARCH_TYPE で始めても、部品も `state.pre-sci.md` も作らない。
    #[test]
    fn init_with_assets_checks_type_before_writing() {
        let root = std::env::temp_dir().join(format!("sai-paper-sci-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let err = init_with_assets(&root, "todo").err().unwrap();
        assert!(format!("{err:#}").contains("知らない RESEARCH_TYPE です"), "{err:#}");
        assert!(!root.join(".sai").exists(), "RESEARCH_TYPE を確かめる前にファイルを置いた");
        assert!(!root.join(".mcp.json").exists(), "RESEARCH_TYPE を確かめる前にファイルを置いた");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 2 回目の init は `already` になり、research-tree は増えない。
    #[test]
    fn init_tree_twice_reports_already() {
        let root = std::env::temp_dir().join(format!("sai-paper-sci-twice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sai").join("paper")).unwrap();

        let first = init_tree_and_export(&root, "problem").unwrap();
        assert!(!first.already);
        assert!(first.created > 0);
        assert_eq!(first.json()["type"], "problem");
        let second = init_tree_and_export(&root, "problem").unwrap();
        assert!(second.already);
        assert!(root.join(".sai").join("paper").join("tree.md").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `sci node <S>` の節の順（seed（固定）→ 新規性（sprout の回答）→ 未回答 → 実験（E）→ 本文）と、道・`sci compare` の表。
    #[test]
    fn sprout_node_sections_and_compare_table() {
        let (root, env) = project_with_tree("sprout-node");
        let conn = env.conn();
        // 新しい模型の DB として作る（変換を走らせない）。
        store::meta_set(conn, "seed_questions", "1").unwrap();
        let new = |kind: rt::Kind, parent: Option<i64>, guide: Option<&str>, text: &str, body: &str, paths: &[i64]| {
            store::insert_node(
                conn,
                &store::NewNode {
                    kind, parent_id: parent, guide_key: guide, text, body, stance: None, cites: &[],
                    paper_id: None, run_id: None, report_id: None, mlflow_ref: None, dataset_ref: None, paths, created_by: "user",
                },
            )
            .unwrap()
        };
        let q = new(rt::Kind::Q, None, Some("method"), "手法の問い", "", &[]);
        let a94 = new(rt::Kind::A, Some(q), None, "Costa 型", "", &[]);
        let q60 = new(rt::Kind::Q, Some(a94), None, "下位の問い", "", &[]);
        let a385 = new(rt::Kind::A, Some(q60), None, "細かい手法", "", &[]);
        let d = new(rt::Kind::D, None, None, "題", "", &[]);
        // seed の問い（雛形の 7 問）。task と data は seed が答え、method は S が答える（新規）。
        let qs: Vec<i64> = rt::seedq::TEMPLATE.iter().map(|(axis, text)| rt::seedq::insert_question(conn, d, text, *axis).unwrap()).collect();
        rt::seedq::insert_answer(conn, d, qs[0], None, Some("発作部位の特定"), "user").unwrap();
        rt::seedq::insert_answer(conn, d, qs[2], None, Some("記載なし: 実測データを使わない"), "user").unwrap();
        let s = new(rt::Kind::S, Some(d), None, "一つ目", "S の本文", &[]);
        rt::seedq::insert_answer(conn, s, qs[3], Some(a385), None, "user").unwrap();
        let e = new(rt::Kind::E, Some(s), None, "実験の題", "", &[]);
        let s2 = new(rt::Kind::S, Some(d), None, "二つ目", "", &[]);
        rt::seedq::refresh(conn, d).unwrap();

        let detail = compute::node_detail(conn, &env.roots, &env.defs, &rt::fmt_id(rt::Kind::S, s)).unwrap();
        let text = node_text(&detail);
        let at = |heading: &str| text.find(heading).unwrap_or_else(|| panic!("{heading} が無い: {text}"));
        let order = [at("## seed（固定）"), at("## 新規性（sprout の回答）"), at("## 未回答"), at("## 実験（E）"), at("## 本文")];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{text}");
        // seed の回答（固定）は問いの文と答え。path の回答は A の文と道。
        assert!(text.contains(&format!("- {} 解く問題は何か: 「発作部位の特定」\n", rt::fmt_sq(qs[0]))), "{text}");
        assert!(text.contains(&format!("- {} どのデータを使うか: 「記載なし: 実測データを使わない」\n", rt::fmt_sq(qs[2]))), "{text}");
        let chain = format!("Q{q} › A{a94} › Q{q60} › A{a385}");
        assert!(text.contains(&format!("- {} どの手法を使うか: A{a385} 細かい手法（{chain}）", rt::fmt_sq(qs[3]))), "{text}");
        assert!(text.contains(&format!("- {} 対象は誰・何か", rt::fmt_sq(qs[1]))), "{text}");
        assert!(text.contains(&format!("- E{e} 実験の題")), "{text}");

        // 比べる表: task と data は両方とも同じ、method は S1 だけ A があるので `*`。
        let ids = vec![rt::fmt_id(rt::Kind::S, s), rt::fmt_id(rt::Kind::S, s2)];
        let table = compare::compare(conn, &env.roots, &env.defs, &ids).unwrap();
        let table_text = compare_text(&table);
        assert!(table_text.starts_with("| S | task | target | data | method* | output | metric |"), "{table_text}");
        assert!(table_text.contains(&format!("| {} 一つ目 | ", rt::fmt_id(rt::Kind::S, s))), "{table_text}");
        assert!(table_text.contains(&format!("A{a385} 細かい手法 [新規]")), "{table_text}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
