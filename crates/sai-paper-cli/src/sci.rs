//! `sai-moon-paper sci …` — 研究方針の木（Research-tree）を扱う命令。
//!
//! 木の計算と書き込みは `sai_sci_core::rt` が担う。ここは外から来る値の検査・
//! core の関数の呼び出し・出力の形だけを持つ。
//!
//! - `--json` の無い命令は人が読む文を標準出力へ、失敗は標準エラー 1 行
//! - `--json` の命令と、いつも JSON の命令（`propose` / `apply` / `triage` /
//!   `type-save`）は失敗も `{"error": "…"}` を標準出力へ
//! - `context` は本文をそのまま標準出力へ出す（sai-link が埋め込むため）

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use rusqlite::Connection;
use sai_paper_core::paths::{self, PaperRoots};
use sai_sci_core::rt::{self, card, compute, context, defs, export, ops, store};
use sai_paper_core::search_sets;
use sai_paper_core::{Config, Database, Paths};
use serde_json::{Value, json};

use crate::open_database;

/// `sai-moon-paper sci` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum SciCmd {
    /// 研究方針の木を始める（型とガイドを .sai/sci/ に置いて、一番上の Q を作る）
    Init {
        /// 研究の型（problem / phenomenon / method / data / improve など。`sci types` で一覧）
        #[arg(long = "type", value_name = "型")]
        type_id: String,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 現況（進み具合・種の下書き・木・次の一手・負債）を出す
    Status {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
        /// 進み具合・種の下書き・次の一手・負債の数だけを出す（いつも JSON）
        #[arg(long)]
        brief: bool,
    },
    /// Node 1 つの詳細を出す
    Node {
        /// Node の ID（`Q12`・`O5` など）
        node: String,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 実行のための入力（`# context`）を標準出力へ出す
    Context {
        /// 実行 ID
        #[arg(long)]
        run: i64,
        /// 範囲（node / path / subtree / siblings / summary / blind）
        #[arg(long)]
        scope: Option<String>,
    },
    /// 提案を出す（標準入力に `{"proposals":[…]}`。いつも JSON）
    Propose {
        /// 実行 ID（無ければ環境変数 SAI_PARENT_RUN_ID）
        #[arg(long)]
        run: Option<i64>,
        /// 出したブロックの id（無ければ環境変数 SAI_BLOCK_NAME）
        #[arg(long)]
        block: Option<String>,
    },
    /// 利用者の操作を 1 つ反映する（標準入力に 1 件の JSON。いつも JSON）
    Apply,
    /// 仕分け待ちの提案を一覧する
    Proposals {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 提案を仕分ける（採用・却下・取り消しの 1 つ。いつも JSON）
    #[command(group(
        clap::ArgGroup::new("triage").required(true).multiple(false).args(["accept", "reject", "revert"]),
    ))]
    Triage {
        /// 提案の ID（`P56`）
        proposal: String,
        /// 採用する
        #[arg(long)]
        accept: bool,
        /// 却下する
        #[arg(long)]
        reject: bool,
        /// 自動で反映したものを取り消す
        #[arg(long)]
        revert: bool,
        /// 採用するとき、中身の上書き（JSON 文字列）
        #[arg(long, requires = "accept")]
        edit: Option<String>,
        /// 仕分けのメモ
        #[arg(long)]
        note: Option<String>,
    },
    /// Node のための実行（カード）を作る
    Card {
        /// カード命令の引数
        #[command(flatten)]
        args: Box<SciCardArgs>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 読む札（runner が user のカード）の中身を出す
    Read {
        /// 実行 ID
        run_id: i64,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// state.md と tree.md を書き出す
    Export,
    /// 使える研究の型を一覧する
    Types {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 研究の型を利用者全体の置き場に保存する（標準入力に型の Markdown。いつも JSON）
    TypeSave,
}

/// `sci card` の引数（[`SciCmd::Card`] から `flatten` で取る）。
#[derive(clap::Args, Debug)]
pub struct SciCardArgs {
    /// 対象の Node（Q・O・F・E。`Q12`）
    #[arg(long)]
    pub node: String,
    /// 実行するブロックの id
    #[arg(long)]
    pub block: Option<String>,
    /// 目的（1 行・300 字まで）
    #[arg(long)]
    pub purpose: String,
    /// 期待する成果
    #[arg(long)]
    pub expect: Option<String>,
    /// 範囲（node / path / subtree / siblings / summary / blind）
    #[arg(long)]
    pub scope: Option<String>,
    /// 仮置き（`Q9=SST-2 を使う（仮）`。繰り返し可）
    #[arg(long)]
    pub assume: Vec<String>,
    /// sci-read の型（purpose_first / blind_first）
    #[arg(long)]
    pub mode: Option<String>,
    /// 対象論文の番号（カンマ区切り）
    #[arg(long, value_delimiter = ',')]
    pub papers: Vec<i64>,
    /// 実行者（pi / cc / user。`user` なら読む札）
    #[arg(long, default_value = "pi")]
    pub runner: String,
    /// 読む札の深さ（abstract / figures / full）
    #[arg(long)]
    pub depth: Option<String>,
    /// 推薦した実行の run_id
    #[arg(long)]
    pub origin_run: Option<i64>,
    /// 依頼文に載せる文書の waggle トークン（繰り返し可）
    #[arg(long = "input")]
    pub input: Vec<String>,
    /// 起動者（human / professor。省略は human＝人が起こす）
    #[arg(long)]
    pub executer: Option<String>,
}

impl SciCardArgs {
    /// 引数を core の [`card::CardSpec`]（C-CARD）にする。
    fn to_spec(&self) -> card::CardSpec {
        card::CardSpec {
            node: self.node.trim().to_string(),
            block: self.block.clone().unwrap_or_default(),
            purpose: self.purpose.clone(),
            expect: self.expect.clone().unwrap_or_default(),
            scope: self.scope.clone().unwrap_or_default(),
            assume: self.assume.clone(),
            mode: self.mode.clone().unwrap_or_default(),
            papers: self.papers.clone(),
            runner: self.runner.clone(),
            depth: self.depth.clone().unwrap_or_default(),
            origin_run: self.origin_run,
            inputs: self.input.clone(),
            executer: self.executer.clone().unwrap_or_default(),
        }
    }
}

/// 命令の出力（人が読む文か JSON）。
pub(crate) enum Out {
    /// 人が読む文（標準出力へ 1 行ずつ出す）。
    Text(String),
    /// JSON（標準出力へ 1 行で出す）。
    Json(Value),
}

/// `sai-moon-paper sci` を実行する。
///
/// 失敗の形は命令ごとに違うので [`wants_json`] で決めてから出す。
pub fn run(cmd: SciCmd) -> Result<()> {
    let as_json = wants_json(&cmd);
    match dispatch(cmd) {
        Ok(Out::Text(text)) => {
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
            Ok(())
        }
        Ok(Out::Json(value)) => {
            println!("{}", serde_json::to_string(&value)?);
            Ok(())
        }
        Err(err) => {
            let message = format!("{err:#}");
            if as_json {
                println!("{}", json!({ "error": message }));
            } else {
                eprintln!("{message}");
            }
            std::process::exit(1);
        }
    }
}

/// 失敗を `{"error": …}` にする命令か。
fn wants_json(cmd: &SciCmd) -> bool {
    match cmd {
        SciCmd::Context { .. } | SciCmd::Export => false,
        // --brief は誰が使っても JSON。
        SciCmd::Status { brief, .. } if *brief => true,
        SciCmd::Init { json, .. }
        | SciCmd::Status { json, .. }
        | SciCmd::Node { json, .. }
        | SciCmd::Proposals { json }
        | SciCmd::Types { json }
        | SciCmd::Card { json, .. }
        | SciCmd::Read { json, .. } => *json,
        // エージェントが読むので、いつも JSON。
        SciCmd::Propose { .. } | SciCmd::Apply | SciCmd::Triage { .. } | SciCmd::TypeSave => true,
    }
}

/// 各命令の中身。
fn dispatch(cmd: SciCmd) -> Result<Out> {
    match cmd {
        SciCmd::Init { type_id, json } => init(&type_id, json),
        SciCmd::Status { json, brief } => status(json, brief),
        SciCmd::Node { node, json } => node_detail(&node, json),
        SciCmd::Context { run, scope } => context(run, scope.as_deref()),
        SciCmd::Propose { run, block } => propose(run, block),
        SciCmd::Apply => apply(),
        SciCmd::Proposals { json } => proposals(json),
        SciCmd::Triage {
            proposal,
            accept,
            reject,
            revert,
            edit,
            note,
        } => triage(&proposal, accept, reject, revert, edit.as_deref(), note.as_deref()),
        SciCmd::Card { args, json } => card(&args.to_spec(), json),
        SciCmd::Read { run_id, json } => read(run_id, json),
        SciCmd::Export => export(),
        SciCmd::Types { json } => types(json),
        SciCmd::TypeSave => type_save(),
    }
}

// ---- 共通の準備 -------------------------------------------------------------

/// sci の命令に必要なもの（DB・置き場・型とガイドの設定）。
pub(crate) struct SciEnv {
    /// papr.db（run の task.md を作る [`card::create_card`] が必要）。
    pub db: Database,
    /// 研究プロジェクトの置き場。
    pub roots: PaperRoots,
    /// 型とガイド。
    pub defs: defs::Defs,
    /// settings.json。
    pub config: Config,
}

impl SciEnv {
    /// DB の接続。
    pub fn conn(&self) -> &Connection {
        self.db.connection()
    }

    /// 木を変えたら state.md と tree.md を書き直す。
    pub fn export(&self) -> Result<()> {
        export::export(self.conn(), &self.roots, &self.defs)
            .context("state.md と tree.md を書き出せませんでした")?;
        Ok(())
    }
}

/// 置き場・DB・設定・型とガイドを開く（`sci.rs` と [`crate::phone_sci`] で使い回す）。
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
/// `sai-moon-paper init --sci` は研究プロジェクトの置き場を `--root` で受け取るので、
/// この関数を使う（`SAI_PAPER_ROOT` に頼らない）。
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
    anyhow!("{err}")
}

// ---- 各命令 -----------------------------------------------------------------

/// `sci init` の返り値（表示と JSON に書く内容）。
pub(crate) struct InitOutcome {
    /// 型の表示名（課題駆動など）。
    pub type_name: String,
    /// 型そのもの（problem など）。
    pub type_id: String,
    /// 作った Q の数（一番上を含む）。
    pub created: usize,
    /// 見つかった注意点。
    pub warnings: Vec<String>,
    /// 残した元の `state.md`。
    pub backup: Option<PathBuf>,
    /// 配った部品の一覧（表示用）。
    pub assets: Vec<String>,
    /// 新しい木が既にあったか。
    pub already: bool,
}

impl InitOutcome {
    /// 人が読む文（警告と、残した state.md の行も足す）。
    pub fn text(&self) -> String {
        let mut text = format!(
            "研究方針の木を作りました（型: {}・Q {}）\n",
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

/// 木を作って state.md と tree.md を書き出す（`sci init` の本体）。
///
/// 新しい木が既にあるときは `already` が真の返り値を返し、エラーにしない。
/// そこから先をどうするかは呼び出し側が決める（`sci init` はエラー、
/// `sai-moon-paper init --sci` は案内だけ出して続ける）。古い木だけがあるときは新しく作る
/// （古い表は触らない）。
///
/// # Errors
/// 研究プロジェクトを開けないとき、木を作れないとき、書き出しに失敗したとき。
pub(crate) fn init_tree_and_export(root: &Path, type_id: &str) -> Result<InitOutcome> {
    let env = open_env_at(root)?;
    let type_name = type_name(&env.defs, type_id)?;
    if store::is_initialized(env.conn()).map_err(rt_err)? {
        // 作ってある木の型の名前を出す（無ければ頼まれた型）。
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
        .context(".sai/sci/ に型とガイドを置けませんでした")?;
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

/// 部品（ブロック・テンプレート・`context`・土壌のひな形）を配ってから木を作る。
///
/// `phone sci init` と `sai-moon-paper init --sci` はここを呼ぶ。
///
/// # Errors
/// 部品を配れないとき、木を作れないとき、書き出しに失敗したとき。
pub(crate) fn init_with_assets(root: &Path, type_id: &str) -> Result<InitOutcome> {
    // 部品を配る前に型を確かめる（知らない型で `.sai/` や `state.pre-sci.md` を残さない）。
    type_name(&defs::load(root), type_id).context("型を確かめられませんでした")?;
    let assets =
        crate::agent_assets::install_sci_here(root).context("sci の部品を配れませんでした")?;
    let mut outcome = init_tree_and_export(root, type_id)?;
    outcome.assets = assets;
    Ok(outcome)
}

/// `sci init` — 部品を配って木を始める。
fn init(type_id: &str, json: bool) -> Result<Out> {
    let outcome = init_with_assets(&open_env()?.roots.root, type_id)?;
    if outcome.already {
        bail!("既に始めています（型: {}）", outcome.type_name);
    }
    if json {
        return Ok(Out::Json(outcome.json()));
    }
    let mut text = String::new();
    for line in &outcome.assets {
        text.push_str(&format!("✅ {line}\n"));
    }
    text.push_str(&outcome.text());
    Ok(Out::Text(text))
}

/// `sci status` — 現況。
fn status(json: bool, brief: bool) -> Result<Out> {
    let env = open_env()?;
    let status =
        compute::status(env.conn(), &env.roots, &env.defs).context("現況を集められませんでした")?;
    // --brief は --json と一緒でも単独でもよく、いつも JSON。
    if brief {
        return Ok(Out::Json(brief_status(&status)));
    }
    if json {
        return Ok(Out::Json(serde_json::to_value(&status)?));
    }
    Ok(Out::Text(status_text(&status)))
}

/// `sci status --brief` と MCP の `sci_status` の `{"brief": true}` が返す JSON。
///
/// 短い文脈で現在地を読めるように、進み具合・種の下書き・次の一手・負債の数だけを出す。
/// 値は [`compute::status`] の結果からそのまま写すだけで、計算し直さない
/// （Web・CLI・MCP で同じ値になる）。
pub fn brief_status(status: &compute::Status) -> Value {
    json!({
        "type": status.type_info.as_ref().map(|found| found.id.clone()),
        "progress": status.progress,
        "seed_draft": status.seed_draft.as_ref().map(|draft| draft.text.clone()),
        "next": status.next.iter().map(|next| json!({
            "kind": next.kind,
            "node": next.node,
            "option": next.option,
            "block": next.block,
            "purpose": next.purpose,
            "expect": next.expect,
            "why": next.why,
        })).collect::<Vec<Value>>(),
        "debt_count": status.debt_count,
        "pending_proposals": status.pending_proposals,
        "running": &status.running,
        "warnings": &status.warnings,
    })
}

/// `sci node <ID>` — Node 1 つの詳細。
fn node_detail(node: &str, json: bool) -> Result<Out> {
    let env = open_env()?;
    let detail = compute::node_detail(env.conn(), &env.roots, &env.defs, node.trim())
        .map_err(rt_err)?;
    if json {
        return Ok(Out::Json(serde_json::to_value(&detail)?));
    }
    Ok(Out::Text(node_text(&detail)))
}

/// `sci context --run <id>` — 実行のための入力（sai-link が本文へ埋め込む）。
fn context(run: i64, scope: Option<&str>) -> Result<Out> {
    let env = open_env()?;
    let text = context::build(
        env.conn(),
        &env.roots,
        &env.defs,
        env.config.sci.context_max_chars,
        run,
        scope,
    )
    .map_err(rt_err)?;
    Ok(Out::Text(text))
}

/// `sci propose` — 提案する（いつも JSON）。
fn propose(run: Option<i64>, block: Option<String>) -> Result<Out> {
    let env = open_env()?;
    let input = read_stdin_json()?;
    let actor = actor_of(run, block);
    let results =
        ops::propose(env.conn(), &env.roots, &env.defs, &actor, &input).map_err(rt_err)?;
    env.export()?;
    Ok(Out::Json(json!({ "results": results })))
}

/// `sci apply` — 利用者の操作を 1 つ反映する（いつも JSON）。
fn apply() -> Result<Out> {
    let env = open_env()?;
    let input = read_stdin_json()?;
    let result = ops::apply_user(env.conn(), &env.roots, &env.defs, &input).map_err(rt_err)?;
    env.export()?;
    Ok(Out::Json(serde_json::to_value(result)?))
}

/// 仕分け待ちの提案を JSON の 1 行にする（`proposals` の `proposals` の要素）。
pub(crate) fn proposal_json(row: &store::ProposalRow) -> Value {
    json!({
        "id": rt::fmt_ref('P', row.id),
        "op": row.op,
        "run": row.run_id,
        "block": row.block,
        "payload": row.payload,
        "reason": row.reason,
        "created_at": row.created_at,
    })
}

/// `sci proposals` — 仕分け待ちの一覧。
fn proposals(json: bool) -> Result<Out> {
    let env = open_env()?;
    let rows = store::list_proposals(env.conn(), Some("pending")).map_err(rt_err)?;
    if json {
        let list: Vec<Value> = rows.iter().map(proposal_json).collect();
        return Ok(Out::Json(json!({ "proposals": list })));
    }
    if rows.is_empty() {
        return Ok(Out::Text("仕分け待ちの提案はありません\n".to_string()));
    }
    let mut text = String::new();
    for row in &rows {
        text.push_str(&format!(
            "{} {} run {}: {}\n",
            rt::fmt_ref('P', row.id),
            row.op,
            row.run_id
                .map_or_else(|| "?".to_string(), |id| id.to_string()),
            one_line(&row.reason)
        ));
    }
    Ok(Out::Text(text))
}

/// `sci triage <P>` — 仕分け。
fn triage(
    proposal: &str,
    accept: bool,
    reject: bool,
    revert: bool,
    edit: Option<&str>,
    note: Option<&str>,
) -> Result<Out> {
    let id = rt::parse_ref(proposal.trim(), 'P').map_err(rt_err)?;
    let edit = match edit {
        Some(text) => Some(
            serde_json::from_str::<Value>(text)
                .map_err(|err| anyhow!("--edit の JSON を読めません: {err}"))?,
        ),
        None => None,
    };
    let action = if accept {
        ops::TriageAction::Accept
    } else if reject {
        ops::TriageAction::Reject
    } else if revert {
        ops::TriageAction::Revert
    } else {
        // clap のグループで 1 つが必ず選ばれる。ここは直接呼んだときの保険。
        bail!("--accept・--reject・--revert の 1 つを指定します");
    };
    Ok(Out::Json(run_triage(id, action, edit.as_ref(), note.unwrap_or(""))?))
}

/// 仕分けを実行して state.md と tree.md を書き直す（`sci triage` と `phone sci triage` で共用）。
pub(crate) fn run_triage(
    id: i64,
    action: ops::TriageAction,
    edit: Option<&Value>,
    note: &str,
) -> Result<Value> {
    let env = open_env()?;
    let result =
        ops::triage(env.conn(), &env.roots, &env.defs, id, action, edit, note).map_err(rt_err)?;
    env.export()?;
    Ok(serde_json::to_value(result)?)
}

/// カードを作って state.md と tree.md を書き直し、`{"run_id","path","executer"}` を返す
/// （`sci card` と `phone sci card` で共用）。
pub(crate) fn make_card(spec: &card::CardSpec) -> Result<Value> {
    let env = open_env()?;
    let result =
        card::create_card(&env.db, &env.roots, &env.defs, spec).map_err(rt_err)?;
    env.export()?;
    // 起動者は省略なら human（検査は create_card がした）。
    let executer = if spec.executer.trim().is_empty() {
        "human"
    } else {
        spec.executer.trim()
    };
    Ok(json!({
        "run_id": result.run_id,
        "path": result.path.to_string_lossy(),
        "executer": executer,
    }))
}

/// `sci card` — Node のための実行（カード）を作る。
fn card(spec: &card::CardSpec, json: bool) -> Result<Out> {
    let value = make_card(spec)?;
    if json {
        return Ok(Out::Json(value));
    }
    Ok(Out::Text(format!(
        "run {} のカードを作りました: {}\n",
        value["run_id"],
        value["path"].as_str().unwrap_or("")
    )))
}

/// `sci read <run_id>` — 読む札の中身。
fn read(run_id: i64, json: bool) -> Result<Out> {
    let env = open_env()?;
    let card =
        card::read_card(env.conn(), &env.roots, &env.defs, run_id).map_err(rt_err)?;
    if json {
        return Ok(Out::Json(serde_json::to_value(card)?));
    }
    Ok(Out::Text(read_text(&card)))
}

/// `sci export` — state.md と tree.md を書き出す。
fn export() -> Result<Out> {
    let env = open_env()?;
    if export::export(env.conn(), &env.roots, &env.defs).context("書き出せませんでした")? {
        Ok(Out::Text("書き出しました: state.md, tree.md\n".to_string()))
    } else {
        Ok(Out::Text("研究方針の木がありません\n".to_string()))
    }
}

/// 型の一覧の JSON（`{"types":[{"id","name","root"}]}`。`sci types` と `phone sci types` で共用）。
pub(crate) fn types_json(defs: &defs::Defs) -> Value {
    let list: Vec<Value> = defs
        .types
        .iter()
        .map(|def| json!({ "id": def.id, "name": def.name, "root": def.root }))
        .collect();
    json!({ "types": list })
}

/// `sci types` — 使える型の一覧。
fn types(json: bool) -> Result<Out> {
    let roots = paths::paper_roots().context("研究プロジェクト（.sai/paper）が見つかりません")?;
    let defs = defs::load(&roots.root);
    if json {
        return Ok(Out::Json(types_json(&defs)));
    }
    let mut text = String::new();
    for def in &defs.types {
        text.push_str(&format!("{} {}（{}）\n", def.id, def.name, one_line(&def.root)));
    }
    Ok(Out::Text(text))
}

/// `sci type-save` — 標準入力の型の Markdown を保存する（いつも JSON）。
fn type_save() -> Result<Out> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|err| anyhow!("標準入力を読めません: {err}"))?;
    if text.trim().is_empty() {
        bail!("標準入力に型の Markdown がありません");
    }
    let path = defs::save_user_type(&text).map_err(rt_err)?;
    Ok(Out::Json(json!({ "path": path.to_string_lossy() })))
}

// ---- 標準入力と外から来る値 -------------------------------------------------

/// 標準入力を全部読んで JSON にする（読めなければ日本語の文でエラー）。
pub(crate) fn read_stdin_json() -> Result<Value> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|err| anyhow!("標準入力を読めません: {err}"))?;
    parse_stdin_text(&text)
}

/// 標準入力のテキストを JSON にする（空か読めなければ日本語の文でエラー）。
pub(crate) fn parse_stdin_text(text: &str) -> Result<Value> {
    if text.trim().is_empty() {
        bail!("標準入力の JSON を読めません");
    }
    serde_json::from_str(text).map_err(|err| anyhow!("標準入力の JSON を読めません: {err}"))
}

/// sai-link が付けた実行 ID（無ければ `None`）。
pub(crate) fn parent_run_id() -> Option<i64> {
    std::env::var("SAI_PARENT_RUN_ID")
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok())
}

/// 提案の実行者を作る（`--run` / `--block`、無ければ環境変数）。
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

// ---- Node に結び付いた検索の記録（find / search / MCP） ------------------------

/// 検索が結び付いた Node（`find` と `search` の `--node`）。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SearchRefs {
    /// 検索した Node の `rt_nodes.id`（検索の範囲を狭めるのにも使う）。
    pub node: Option<i64>,
}

/// `find` / `search` / MCP の `find` の引数検査。
///
/// `--node` は `Q12`・`O5` のような形だけを受け付ける。指定が無ければ何も調べない
/// （木が無いプロジェクトでも `find` / `search` を今までどおり使えるようにするため）。
pub(crate) fn parse_search_refs(conn: &Connection, node: Option<&str>) -> Result<SearchRefs> {
    let Some(text) = node else {
        return Ok(SearchRefs::default());
    };
    let text = text.trim();
    if !store::is_initialized(conn).map_err(rt_err)? {
        bail!("研究方針の木がありません（sai-moon-paper sci init）");
    }
    let (kind, id) = rt::parse_id(text).map_err(rt_err)?;
    let alive = store::get_node_of(conn, kind, id)
        .ok()
        .is_some_and(|row| row.removed_at.is_none());
    if !alive {
        bail!("{text} はありません");
    }
    Ok(SearchRefs { node: Some(id) })
}

/// 検索の記録に残す内容（`find` / `search` / MCP の `find` で同じものを使う）。
pub(crate) struct SearchLog<'a> {
    /// 検索が結び付いた Node。
    pub refs: &'a SearchRefs,
    /// `find` / `search` / `expand`。
    pub tool: &'a str,
    /// 検索語。
    pub query: &'a str,
    /// 検索の範囲（`node:Q12` など）。
    pub scope: &'a str,
    /// 範囲を広げた理由（`--widen`）。
    pub widen_reason: &'a str,
    /// 当たった件数。
    pub hits: i64,
    /// 当たった論文の番号。
    pub hit_papers: &'a [i64],
    /// 作った検索セットの番号。
    pub set_id: Option<i64>,
    /// その 1 回の検索の履歴（`search_sets` の行）の番号。
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
            widen_reason: log.widen_reason.to_string(),
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

/// `find` の範囲の引数の組み合わせを検査する（CLI と MCP の `find` で共用）。
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
        bail!("--subtree は --node と一緒に使います");
    }
    if each.is_some() && papers_empty && !node_given {
        bail!("--each は --papers か --node と一緒に使います");
    }
    Ok(())
}

/// 検索結果から論文の番号を取り出す（重複なし・昇順。検索の記録に残す）。
///
/// CLI の find・MCP の find・phone の find で共用（U01）。
pub(crate) fn hit_paper_ids(hits: &[sai_paper_core::Hit]) -> Vec<i64> {
    let mut ids: Vec<i64> = hits.iter().filter_map(|hit| hit.paper_id).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// 検索セットに何のための検索かを書く Fields を作る（1 つも無ければ `None`）。
pub(crate) fn set_fields(
    refs: &SearchRefs,
    purpose: Option<&str>,
    extract: Option<&str>,
) -> Option<search_sets::SciSetFields> {
    if refs.node.is_none() && purpose.is_none() && extract.is_none() {
        return None;
    }
    Some(search_sets::SciSetFields {
        node: refs.node,
        question: None,
        purpose: purpose.map(str::to_string),
        extract: extract.map(str::to_string),
    })
}

// ---- 人が読む文 -------------------------------------------------------------

/// `sci status` の文（型・進み具合・種の下書き・負債・木・次の一手）。
fn status_text(status: &compute::Status) -> String {
    if !status.initialized {
        let mut text = "研究方針の木がまだありません。`sai-moon-paper sci init --type <型>` で始めてください。\n"
            .to_string();
        for warning in &status.warnings {
            text.push_str(&format!("⚠ {warning}\n"));
        }
        return text;
    }
    let mut text = String::new();
    if let Some(found) = &status.type_info {
        text.push_str(&format!("型: {}\n", found.name));
    }
    text.push_str(&format!(
        "進み具合: {}/{}\n",
        status.progress.answered, status.progress.total
    ));
    match &status.seed_draft {
        Some(draft) if !draft.text.is_empty() => {
            text.push_str(&format!("種の下書き: {}\n", draft.text));
        }
        _ => text.push_str("種の下書きはまだありません\n"),
    }
    text.push_str(&format!(
        "負債 {} 件・仕分け待ちの提案 {} 件\n",
        status.debt_count, status.pending_proposals
    ));
    for node in &status.nodes {
        text.push_str(&node_line(node));
    }
    if !status.next.is_empty() {
        text.push_str("次の一手:\n");
        for (index, next) in status.next.iter().enumerate() {
            text.push_str(&format!("{}. {}\n", index + 1, next_text(next)));
        }
    }
    for warning in &status.warnings {
        text.push_str(&format!("⚠ {warning}\n"));
    }
    text
}

/// 次の一手 1 件の文（Node・ブロック・目的・理由）。
fn next_text(next: &compute::NextAction) -> String {
    let mut head = String::new();
    for part in [&next.node, &next.block].into_iter().flatten() {
        head.push_str(part);
        head.push(' ');
    }
    format!("{head}{}（{}）", next.purpose, next.why)
}

/// Node 1 つの 1 行（深さ分だけ字下げする。Q には Objectivity と Logicality を付ける）。
fn node_line(node: &compute::NodeJson) -> String {
    let mut line = format!("{}- {} ", "  ".repeat(node.depth), node.id);
    if node.starred {
        line.push_str("★ ");
    }
    line.push_str(&one_line(&node.text));
    if let Some(found) = &node.objectivity {
        line.push_str(&format!(" [Objectivity {}/{}", found.count, found.target));
        if node.leaf && found.met {
            line.push_str(" met");
        }
        line.push(']');
    }
    if let Some(found) = &node.logicality {
        line.push_str(&format!(" [Logicality {}]", found.value));
    }
    line.push('\n');
    line
}

/// `sci node <ID>` の文（題・道筋・子・ガイド・確かめ・決定の記録・提案・検索）。
fn node_text(detail: &compute::NodeDetail) -> String {
    let node = &detail.node;
    let mut text = format!("{} {}\n", node.id, one_line(&node.text));
    if !node.body.is_empty() {
        text.push_str(&format!("\n{}\n", node.body));
    }
    if !detail.path.is_empty() {
        let path = detail
            .path
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>()
            .join(" > ");
        text.push_str(&format!("\n道筋: {path}\n"));
    }
    if let Some(guide) = &detail.guide {
        text.push_str(&format!(
            "\nガイド {}: {}（Objectivity の目安 {}）\n",
            guide.name, guide.explain, guide.target
        ));
    }
    if !detail.children.is_empty() {
        text.push_str("\n子:\n");
        for child in &detail.children {
            text.push_str(&format!("  {}", node_line(child).trim_start_matches("- ")));
        }
    }
    if !detail.logic.is_empty() {
        text.push_str("\nLogicality の確かめ:\n");
        for logic in &detail.logic {
            text.push_str(&format!(
                "  {} {}（F {} 件）{}\n",
                logic.created_at,
                logic.value,
                logic.f_count,
                if logic.note.is_empty() {
                    String::new()
                } else {
                    format!(": {}", one_line(&logic.note))
                }
            ));
        }
    }
    if !detail.decisions.is_empty() {
        text.push_str("\n決定の記録:\n");
        for decision in &detail.decisions {
            let mut line = format!("  {} {} {}", decision.at, decision.action, decision.actor);
            if !decision.detail.is_empty() {
                line.push_str(&format!(" {}", one_line(&decision.detail)));
            }
            if !decision.reason.is_empty() {
                line.push_str(&format!(": {}", one_line(&decision.reason)));
            }
            text.push_str(&line);
            text.push('\n');
        }
    }
    if !detail.pending.is_empty() {
        text.push_str("\n仕分け待ち:\n");
        for pending in &detail.pending {
            text.push_str(&format!(
                "  {} {}: {}\n",
                pending.id,
                pending.op,
                one_line(&pending.reason)
            ));
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
        text.push_str("\n次の一手:\n");
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
    for debt in &card.debts {
        text.push_str(&format!("⚠ {debt}\n"));
    }
    text
}

/// 研究の型の表示名。
fn type_name(defs: &defs::Defs, type_id: &str) -> Result<String> {
    defs.type_def(type_id)
        .map(|def| def.name.clone())
        .ok_or_else(|| {
            let known: Vec<&str> = defs.types.iter().map(|def| def.id.as_str()).collect();
            anyhow!(
                "知らない研究の型です: {type_id}（{}）",
                if known.is_empty() {
                    "型が読み込まれていません".to_string()
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
    use clap::Parser as _;

    /// 一時フォルダに研究プロジェクトの置き場と、新しい木を作った DB を用意する。
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

    /// `find` の新しい引数は clap で読める（`--papers` はカンマ区切り）。`--question` はもう無い。
    #[test]
    fn find_parses_new_arguments() {
        let cli = crate::Cli::try_parse_from([
            "sai-moon-paper", "find", "発作", "--node", "Q12", "--subtree", "--each", "2",
            "--widen", "範囲を広げる", "--papers", "37,40", "--json",
        ])
        .unwrap();
        let Some(crate::Cmd::Find { papers, node, subtree, each, widen, json, .. }) = cli.cmd
        else {
            panic!("find として読めた");
        };
        assert_eq!(papers, vec![37, 40]);
        assert_eq!(node.as_deref(), Some("Q12"));
        assert!(subtree);
        assert_eq!(each, Some(2));
        assert_eq!(widen.as_deref(), Some("範囲を広げる"));
        assert!(json);

        // `--question` は clap が断る。
        assert!(
            crate::Cli::try_parse_from(["sai-moon-paper", "find", "発作", "--question", "Q7"]).is_err()
        );
        assert!(
            crate::Cli::try_parse_from(["sai-moon-paper", "search", "発作", "--question", "Q7"]).is_err()
        );
    }

    /// `search` と `set create` の引数は clap で読める。
    #[test]
    fn search_and_set_create_parse_node_arguments() {
        let cli = crate::Cli::try_parse_from([
            "sai-moon-paper", "search", "発作部位", "--node", "Q12", "--purpose", "候補を探す",
            "--extract", "手法の候補", "--widen", "範囲を広げる",
        ])
        .unwrap();
        let Some(crate::Cmd::Search { node, purpose, extract, .. }) = cli.cmd else {
            panic!("search として読めた");
        };
        assert_eq!(node.as_deref(), Some("Q12"));
        assert_eq!(purpose.as_deref(), Some("候補を探す"));
        assert_eq!(extract.as_deref(), Some("手法の候補"));

        assert!(crate::Cli::try_parse_from([
            "sai-moon-paper", "set", "create", "候補", "--query", "発作部位", "--node", "Q12",
            "--purpose", "候補を探す",
        ])
        .is_ok());
        assert!(crate::Cli::try_parse_from([
            "sai-moon-paper", "set", "create", "候補", "--query", "x", "--question", "Q7",
        ])
        .is_err());
    }

    /// 消した命令は無く、足した命令は読める。
    #[test]
    fn sci_command_set() {
        for name in ["retitle", "tombstones", "finish-read"] {
            assert!(crate::Cli::try_parse_from(["sai-moon-paper", "sci", name]).is_err(), "{name}");
        }
        for args in [&["types", "--json"][..], &["type-save"][..], &["init", "--type", "problem"][..]] {
            let mut all = vec!["sai-moon-paper", "sci"];
            all.extend_from_slice(args);
            assert!(crate::Cli::try_parse_from(all).is_ok(), "{args:?}");
        }
        // カードの `--question` / `--role` はもう無い。
        for flag in ["--question", "--role"] {
            assert!(crate::Cli::try_parse_from([
                "sai-moon-paper", "sci", "card", "--node", "Q1", "--purpose", "p", flag, "x",
            ])
            .is_err());
        }
    }

    /// カードの引数は C-CARD の形になる（`--papers` はカンマ区切り）。
    #[test]
    fn card_args_become_card_spec() {
        let cli = crate::Cli::try_parse_from([
            "sai-moon-paper", "sci", "card", "--node", "Q12", "--block", "sci-ideate", "--purpose",
            "候補を出す", "--assume", "Q9=SST-2 を使う（仮）", "--papers", "37,40", "--origin-run",
            "120",
        ])
        .unwrap();
        let Some(crate::Cmd::Sci { cmd: SciCmd::Card { args, .. } }) = cli.cmd else {
            panic!("sci card として読めた");
        };
        let spec = args.to_spec();
        assert_eq!(spec.node, "Q12");
        assert_eq!(spec.block, "sci-ideate");
        assert_eq!(spec.assume, vec!["Q9=SST-2 を使う（仮）".to_string()]);
        assert_eq!(spec.papers, vec![37, 40]);
        assert_eq!(spec.runner, "pi");
        assert_eq!(spec.origin_run, Some(120));
    }

    #[test]
    fn triage_flags_are_exclusive() {
        let cli = crate::Cli::try_parse_from(["sai-moon-paper", "sci", "triage", "P1", "--accept"]).unwrap();
        let Some(crate::Cmd::Sci { cmd: SciCmd::Triage { accept, reject, .. } }) = cli.cmd else {
            panic!("sci triage として読めた");
        };
        assert!(accept);
        assert!(!reject);

        for flags in [&["--accept", "--reject"][..], &["--reject", "--revert"][..], &[][..]] {
            let mut all = vec!["sai-moon-paper", "sci", "triage", "P1"];
            all.extend_from_slice(flags);
            assert!(crate::Cli::try_parse_from(all).is_err(), "{flags:?}");
        }
    }

    /// 新しい木の status は、文にも `--brief` の JSON にも同じ値が出る。
    #[test]
    fn status_text_and_brief_follow_the_tree() {
        let (root, env) = project_with_tree("status");
        let status = compute::status(env.conn(), &env.roots, &env.defs).unwrap();

        let text = status_text(&status);
        assert!(text.contains("型: "), "{text}");
        assert!(text.contains("進み具合: 0/"), "{text}");
        assert!(text.contains("- Q1 "), "{text}");
        assert!(text.contains("[Objectivity "), "{text}");

        let brief = brief_status(&status);
        assert_eq!(brief["type"], "problem");
        assert_eq!(brief["progress"]["answered"], 0);
        assert!(brief["progress"]["total"].as_i64().unwrap() > 0, "{brief}");
        for key in ["seed_draft", "next", "debt_count", "pending_proposals", "running", "warnings"] {
            assert!(brief.get(key).is_some(), "{key}: {brief}");
        }
        // 木そのものは入れない。
        assert!(brief.get("nodes").is_none(), "{brief}");

        // 木が無いときは始め方を案内する。
        let empty = status_text(&compute::Status { initialized: false, ..status });
        assert!(empty.contains("まだありません"), "{empty}");
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
                widen_reason: "",
                hits: 2,
                hit_papers: &[7, 9],
                set_id: None,
                log_id: None,
            },
        )
        .unwrap();
        let line = previous_search_line(conn, &refs, "node:Q1").unwrap();
        assert!(line.contains("「発作」2 件（論文 #7, #9）"), "{line}");
        assert!(previous_search_line(conn, &refs, "node:O1").is_none());
        record_search(
            conn,
            &SearchLog {
                refs: &none,
                tool: "find",
                query: "x",
                scope: "library",
                widen_reason: "",
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

    /// 木が始まっていないプロジェクトで `--node` を指定すると、始め方を案内する。
    #[test]
    fn search_refs_need_a_started_tree() {
        let db = Database::in_memory().unwrap();
        let err = parse_search_refs(db.connection(), Some("Q1")).unwrap_err();
        assert!(format!("{err:#}").contains("研究方針の木がありません"), "{err:#}");
    }

    #[test]
    fn read_stdin_json_rejects_empty_and_broken() {
        assert!(parse_stdin_text("").is_err());
        assert!(parse_stdin_text("  \n").is_err());
        assert!(parse_stdin_text("{oops").is_err());
        let ok = parse_stdin_text("{\"op\":\"star\"}").unwrap();
        assert_eq!(ok["op"], "star");
    }

    /// 知らない型で始めても、部品も `state.pre-sci.md` も作らない。
    #[test]
    fn init_with_assets_checks_type_before_writing() {
        let root = std::env::temp_dir().join(format!("sai-paper-sci-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let err = init_with_assets(&root, "todo").err().unwrap();
        assert!(format!("{err:#}").contains("知らない研究の型です"), "{err:#}");
        assert!(!root.join(".sai").exists(), "型を確かめる前にファイルを置いた");
        assert!(!root.join(".mcp.json").exists(), "型を確かめる前にファイルを置いた");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 2 回目の init は `already` になり、木は増えない。
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
}
