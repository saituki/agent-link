//! `sai-moon-paper sci …` — 研究方針の木（sai-sci）を扱う命令。
//!
//! 木（Research-tree）の計算と書き込みは `sai_paper_core::sci`（U06〜U11）が担う。ここは
//! 外から来る値の検査・core の関数の呼び出し・出力の形だけを持つ。
//!
//! - `--json` の無い命令は人が読む文を標準出力へ、失敗は標準エラー 1 行
//! - `--json` の命令と、いつも JSON の命令（`propose` / `apply` / `triage` /
//!   `finish-read`）は失敗も `{"error": "…"}` を標準出力へ
//! - `context` は本文をそのまま標準出力へ出す（sai-link が埋め込むため）

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use rusqlite::Connection;
use sai_paper_core::paths::{self, PaperRoots};
use sai_paper_core::sci::{self, card, compute, context, defs, export, ops, store};
use sai_paper_core::search_sets;
use sai_paper_core::{Config, Database, Paths};
use serde_json::{Value, json};

use crate::open_database;

/// `sai-moon-paper sci` のサブコマンド（C-09）。
#[derive(Subcommand, Debug)]
pub enum SciCmd {
    /// 研究方針の木を始める（型と項目ガイドを .sai/sci/ に置いて骨組みを作る）
    Init {
        /// 研究の型（problem / phenomenon / method / data / improve）
        #[arg(long = "type", value_name = "型")]
        type_id: String,
        /// 型を変える（木は壊さず、無い項目だけ足す）
        #[arg(long = "change-type")]
        change_type: bool,
    },
    /// 現況（種の文・関門・負債・木・次の一手）を出す
    Status {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 節点 1 つの詳細を出す
    Node {
        /// 節点の ID（`N12`）
        node: String,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 実行のための入力（sai-sci より）を標準出力へ出す
    Context {
        /// 実行 ID
        #[arg(long)]
        run: i64,
        /// 範囲（node / path / subtree / siblings / summary / blind）
        #[arg(long)]
        scope: Option<String>,
    },
    /// 提案する（標準入力に C-07 の JSON。いつも JSON）
    Propose {
        /// 実行 ID（無ければ環境変数 SAI_PARENT_RUN_ID）
        #[arg(long)]
        run: Option<i64>,
        /// 出したブロックの id（無ければ環境変数 SAI_BLOCK_NAME）
        #[arg(long)]
        block: Option<String>,
    },
    /// 利用者の操作を 1 つ反映する（標準入力に C-07 の JSON。いつも JSON）
    Apply,
    /// 仕分け待ちの提案を一覧する
    Proposals {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 提案を仕分ける（採用・却下・取り消しの 1 つ）
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
    /// 節点のための実行（カード）を作る
    Card {
        /// カード命令の引数
        #[command(flatten)]
        args: Box<SciCardArgs>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 読む札の中身を出す
    Read {
        /// 実行 ID
        run_id: i64,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 読む札を終える（この実行で入れた知見を反映済みにする。いつも JSON）
    FinishRead {
        /// 実行 ID
        run_id: i64,
        /// 反映しない理由（知見が無いとき必要）
        #[arg(long)]
        no_reflect: Option<String>,
    },
    /// state.md と tree.md を書き出す
    Export,
    /// 墓標（外した論文と候補）を一覧する
    Tombstones {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
}

/// `sci card` の引数（[`SciCmd::Card`] から `flatten` で取る）。
#[derive(clap::Args, Debug)]
pub struct SciCardArgs {
    /// 対象の節点（`N12`）
    #[arg(long)]
    pub node: String,
    /// 実行するブロックの id
    #[arg(long)]
    pub block: Option<String>,
    /// 行動の目的
    #[arg(long)]
    pub purpose: String,
    /// 答える問い（`Q7`）
    #[arg(long)]
    pub question: Option<String>,
    /// 期待する成果
    #[arg(long)]
    pub expect: Option<String>,
    /// 範囲（node / path / subtree / siblings / summary / blind）
    #[arg(long)]
    pub scope: Option<String>,
    /// 仮置き（`N9=発作部位の予測（仮）`。繰り返し可）
    #[arg(long)]
    pub assume: Vec<String>,
    /// sci-read の型（purpose_first / blind_first）
    #[arg(long)]
    pub mode: Option<String>,
    /// 対象論文の番号（繰り返し可）
    #[arg(long)]
    pub paper: Vec<i64>,
    /// 実行者（pi / cc / user。`user` なら読む札）
    #[arg(long, default_value = "pi")]
    pub runner: String,
    /// 読む札の論文の役割
    #[arg(long)]
    pub role: Option<String>,
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
    /// 引数を core の [`card::CardSpec`] にする。
    fn to_spec(&self) -> Result<card::CardSpec> {
        Ok(card::CardSpec {
            node: parse_id(&self.node, 'N')?,
            block: self.block.clone().unwrap_or_default(),
            purpose: self.purpose.clone(),
            question: match &self.question {
                Some(text) => Some(parse_id(text, 'Q')?),
                None => None,
            },
            expect: self.expect.clone().unwrap_or_default(),
            scope: self.scope.clone().unwrap_or_default(),
            assume: parse_assume(&self.assume)?,
            mode: self.mode.clone().unwrap_or_default(),
            papers: self.paper.clone(),
            runner: self.runner.clone(),
            origin_run: self.origin_run,
            role: self.role.clone().unwrap_or_default(),
            depth: self.depth.clone().unwrap_or_default(),
            inputs: self.input.clone(),
            executer: self.executer.clone().unwrap_or_default(),
        })
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
/// 失敗の形は命令마다違うので [`wants_json`] で決ってから出す。
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
        SciCmd::Init { .. } | SciCmd::Context { .. } | SciCmd::Export => false,
        SciCmd::Status { json }
        | SciCmd::Node { json, .. }
        | SciCmd::Proposals { json }
        | SciCmd::Tombstones { json }
        | SciCmd::Card { json, .. }
        | SciCmd::Read { json, .. } => *json,
        // エージェントと pi-litbase が読むので、いつも JSON。
        SciCmd::Propose { .. }
        | SciCmd::Apply
        | SciCmd::Triage { .. }
        | SciCmd::FinishRead { .. } => true,
    }
}

/// 各命令の中身。
fn dispatch(cmd: SciCmd) -> Result<Out> {
    match cmd {
        SciCmd::Init {
            type_id,
            change_type,
        } => init(&type_id, change_type),
        SciCmd::Status { json } => status(json),
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
        } => triage(
            &proposal,
            accept,
            reject,
            revert,
            edit.as_deref(),
            note.as_deref(),
        ),
        SciCmd::Card { args, json } => card(args.to_spec()?, json),
        SciCmd::Read { run_id, json } => read(run_id, json),
        SciCmd::FinishRead { run_id, no_reflect } => finish_read(run_id, no_reflect.as_deref()),
        SciCmd::Export => export(),
        SciCmd::Tombstones { json } => tombstones(json),
    }
}

// ---- 共通の準備 -------------------------------------------------------------

/// sci の命令に必要なもの（DB・置き場・型とガイドの設定）。
pub(crate) struct SciEnv {
    /// papr.db（節点と run の結び付きを作る [`card::create_card`] が必要）。
    pub db: Database,
    /// 研究プロジェクトの置き場。
    pub roots: PaperRoots,
    /// 型と項目ガイド。
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

// ---- 各命令 -----------------------------------------------------------------

/// `sci init` の返り値（表示と JSON に書く内容）。
pub(crate) struct InitOutcome {
    /// 型の表示名（課題駆動など）。
    pub type_name: String,
    /// 型そのもの（problem など）。
    pub type_id: String,
    /// 作った項目の数。
    pub created_items: usize,
    /// 作った線の本数。
    pub created_edges: usize,
    /// 見つかった注意点。
    pub warnings: Vec<String>,
    /// 残した元の `state.md`。
    pub backup: Option<PathBuf>,
    /// 配った部品の一覧（表示用）。
    pub assets: Vec<String>,
    /// 木が既に作ってあったか（`change_type` が `false` のときだけ意味を持つ）。
    pub already: bool,
}

impl InitOutcome {
    /// 人が読む 1 行（警告と、残した state.md の行も足す）。
    pub fn text(&self, change_type: bool) -> String {
        let head = if change_type {
            format!(
                "型を変えました（型: {}・追加 項目 {}・線 {}）",
                self.type_name, self.created_items, self.created_edges
            )
        } else {
            format!(
                "研究方針の木を作りました（型: {}・項目 {}・線 {}）",
                self.type_name, self.created_items, self.created_edges
            )
        };
        let mut text = format!("{head}\n");
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
}

/// 木を作って state.md と tree.md を書き出す（`sci init` の本体）。
///
/// 木が既に作ってあるときは `already` が真の返り値を返し、エラーにしない。
/// そこから先をどうするかは呼び出し側が決める（`sci init` はエラー、
/// `sai-moon-paper init --sci` は案内だけ出して続ける）。
///
/// # Errors
/// 研究プロジェクトを開けないとき、木を作れないとき、書き出しに失敗したとき。
pub(crate) fn init_tree_and_export(
    root: &Path,
    type_id: &str,
    change_type: bool,
) -> Result<InitOutcome> {
    let env = open_env_at(root)?;
    let type_name = type_name(&env.defs, type_id)?;
    if store::is_initialized(env.conn())? && !change_type {
        return Ok(InitOutcome {
            type_name,
            type_id: type_id.to_string(),
            created_items: 0,
            created_edges: 0,
            warnings: Vec::new(),
            backup: None,
            assets: Vec::new(),
            already: true,
        });
    }

    defs::write_local_copies(&env.roots.root)
        .context(".sai/sci/ に型と項目ガイドを置けませんでした")?;
    let backup = export::backup_state_once(&env.roots.data_dir).context("今の state.md を残せませんでした")?;
    let report = if change_type {
        defs::change_type(env.conn(), &env.defs, type_id).context("型を変えられませんでした")?
    } else {
        defs::init_tree(env.conn(), &env.defs, type_id).context("骨組みを作れませんでした")?
    };
    env.export()?;

    Ok(InitOutcome {
        type_name,
        type_id: report.type_id.clone(),
        created_items: report.created_items,
        created_edges: report.created_edges,
        warnings: report.warnings.clone(),
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
pub(crate) fn init_with_assets(
    root: &Path,
    type_id: &str,
    change_type: bool,
) -> Result<InitOutcome> {
    // 部品を配る前に型を確かめる（知らない型で `.sai/` や `state.pre-sci.md` を残さない）。
    type_name(&defs::load(root), type_id).context("型を確かめられませんでした")?;
    let assets =
        crate::agent_assets::install_sci_here(root).context("sci の部品を配れませんでした")?;
    let mut outcome = init_tree_and_export(root, type_id, change_type)?;
    outcome.assets = assets;
    Ok(outcome)
}

/// `sci init` — 部品を配って木を始める。
fn init(type_id: &str, change_type: bool) -> Result<Out> {
    let outcome = init_with_assets(&open_env()?.roots.root, type_id, change_type)?;
    if outcome.already && !change_type {
        bail!("既に始めています（型: {}）。型を変えるには --change-type", outcome.type_name);
    }
    let mut text = String::new();
    for line in &outcome.assets {
        text.push_str(&format!("✅ {line}\n"));
    }
    text.push_str(&outcome.text(change_type));
    Ok(Out::Text(text))
}

/// `sci status` — 現況。
fn status(json: bool) -> Result<Out> {
    let env = open_env()?;
    let status =
        compute::status(env.conn(), &env.roots, &env.defs).context("現況を集められませんでした")?;
    if json {
        return Ok(Out::Json(serde_json::to_value(&status)?));
    }
    Ok(Out::Text(status_text(&status)))
}

/// `sci node <N>` — 節点 1 つの詳細。
fn node_detail(node: &str, json: bool) -> Result<Out> {
    let env = open_env()?;
    let id = parse_id(node, 'N')?;
    let detail = compute::node_detail(env.conn(), &env.roots, &env.defs, id)
        .context("節点の詳細を集められませんでした")?;
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
    .context("入力の範囲を組み立てられませんでした")?;
    Ok(Out::Text(text))
}

/// `sci propose` — 提案する（いつも JSON）。
fn propose(run: Option<i64>, block: Option<String>) -> Result<Out> {
    let env = open_env()?;
    let input = read_stdin_json()?;
    let actor = actor_of(run, block);
    let results = ops::propose(env.conn(), &env.roots, &env.defs, &actor, &input)
        .map_err(|err| anyhow!("{err}"))?;
    env.export()?;
    Ok(Out::Json(json!({ "results": results })))
}

/// `sci apply` — 利用者の操作を 1 つ反映する（いつも JSON）。
fn apply() -> Result<Out> {
    let env = open_env()?;
    let input = read_stdin_json()?;
    let result = ops::apply_user(env.conn(), &env.roots, &env.defs, &input)
        .map_err(|err| anyhow!("{err}"))?;
    env.export()?;
    Ok(Out::Json(serde_json::to_value(result)?))
}

/// `sci proposals` — 仕分け待ちの一覧。
fn proposals(json: bool) -> Result<Out> {
    let env = open_env()?;
    let rows =
        store::list_proposals(env.conn(), Some("pending")).context("提案を読めませんでした")?;
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
            "{} {} {} run {}: {}\n",
            sci::fmt_ref('P', row.id),
            row.op,
            row.node_id
                .map_or_else(|| "-".to_string(), |id| sci::fmt_ref('N', id)),
            row.run_id
                .map_or_else(|| "?".to_string(), |id| id.to_string()),
            row.reason
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
    let env = open_env()?;
    let id = parse_id(proposal, 'P')?;
    let action = if accept {
        let edit = match edit {
            Some(text) => Some(
                serde_json::from_str(text)
                    .map_err(|err| anyhow!("--edit の JSON を読めません: {err}"))?,
            ),
            None => None,
        };
        ops::TriageAction::Accept { edit }
    } else if reject {
        ops::TriageAction::Reject
    } else if revert {
        ops::TriageAction::Revert
    } else {
        // clap のグループで 1 つが必ず選ばれる。ここは直接呼んだときの保険。
        bail!("--accept・--reject・--revert の 1 つを指定します");
    };
    let result = ops::triage(
        env.conn(),
        &env.roots,
        &env.defs,
        id,
        action,
        note.unwrap_or(""),
    )
    .map_err(|err| anyhow!("{err}"))?;
    env.export()?;
    Ok(Out::Json(serde_json::to_value(result)?))
}

/// `sci card` — 節点のための実行（カード）を作る。
fn card(spec: sci::card::CardSpec, json: bool) -> Result<Out> {
    let env = open_env()?;
    // 起動者は省略なら human（検査は create_card がする）。
    let executer = sci::card::normalize_executer(&spec.executer);
    let result =
        card::create_card(&env.db, &env.roots, &env.defs, &spec).map_err(|err| anyhow!("{err}"))?;
    env.export()?;
    let value = json!({
        "run_id": result.run_id,
        "path": result.path.to_string_lossy(),
        "executer": executer,
    });
    if json {
        return Ok(Out::Json(value));
    }
    Ok(Out::Text(format!(
        "run {} のカードを作りました: {}\n",
        result.run_id,
        result.path.display()
    )))
}

/// `sci read <run_id>` — 読む札の中身。
fn read(run_id: i64, json: bool) -> Result<Out> {
    let env = open_env()?;
    let card = card::read_card(env.conn(), &env.roots, &env.defs, run_id)
        .map_err(|err| anyhow!("{err}"))?;
    if json {
        return Ok(Out::Json(serde_json::to_value(card)?));
    }
    Ok(Out::Text(read_text(&card)))
}

/// `sci finish-read <run_id>` — 読む札を終える（いつも JSON）。
fn finish_read(run_id: i64, no_reflect: Option<&str>) -> Result<Out> {
    let env = open_env()?;
    let result = card::finish_read(env.conn(), &env.roots, run_id, no_reflect)
        .map_err(|err| anyhow!("{err}"))?;
    env.export()?;
    Ok(Out::Json(serde_json::to_value(result)?))
}

/// `sci export` — state.md と tree.md を書き出す。
fn export() -> Result<Out> {
    let env = open_env()?;
    if export::export(env.conn(), &env.roots, &env.defs).context("書き出せませんでした")?
    {
        Ok(Out::Text("書き出しました: state.md, tree.md\n".to_string()))
    } else {
        Ok(Out::Text("研究方針の木がありません\n".to_string()))
    }
}

/// `sci tombstones` — 墓標の一覧。
fn tombstones(json: bool) -> Result<Out> {
    let env = open_env()?;
    let rows = store::list_tombstones(env.conn()).context("墓標を読めませんでした")?;
    if json {
        return Ok(Out::Json(serde_json::to_value(rows)?));
    }
    if rows.is_empty() {
        return Ok(Out::Text("墓標はありません\n".to_string()));
    }
    let mut text = String::new();
    for row in rows {
        let target = if row.kind == "paper" {
            match row.paper_id {
                Some(id) => format!("#{id} {}", one_line(&row.title)),
                None => one_line(&row.title),
            }
        } else {
            one_line(&row.dedupe_key.unwrap_or_else(|| row.title.clone()))
        };
        text.push_str(&format!(
            "{} {target}（{}）\n",
            target_kind_label(&row.kind),
            row.reason
        ));
    }
    Ok(Out::Text(text))
}

// ---- 標準入力と外から来る値 -------------------------------------------------

/// 標準入力を全部読んで C-07 の JSON にする（読めなければ日本語の文でエラー）。
pub(crate) fn read_stdin_json() -> Result<Value> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|err| anyhow!("標準入力を読めません: {err}"))?;
    parse_stdin_text(&text)
}

/// 標準入力のテキストを C-07 の JSON にする（空か読めなければ日本語の文でエラー）。
pub(crate) fn parse_stdin_text(text: &str) -> Result<Value> {
    if text.trim().is_empty() {
        bail!("標準入力の JSON を読めません");
    }
    serde_json::from_str(text).map_err(|err| anyhow!("標準入力の JSON を読めません: {err}"))
}

/// `N12` のような ID を読む。
pub(crate) fn parse_id(text: &str, prefix: char) -> Result<i64> {
    sci::parse_ref(text.trim(), prefix).map_err(|err| anyhow!("{err}"))
}

/// 仮置きの `N9=値` を読む（`--assume` と phone の `card` の `assume` で使う）。
pub(crate) fn parse_assume(items: &[String]) -> Result<Vec<(i64, String)>> {
    let invalid = || anyhow!("--assume は N<番号>=<値> の形で書きます");
    let mut out = Vec::new();
    for item in items {
        let (left, right) = item.split_once('=').ok_or_else(invalid)?;
        let id = sci::parse_ref(left.trim(), 'N').map_err(|_| invalid())?;
        out.push((id, right.trim().to_string()));
    }
    Ok(out)
}

/// sai-link が付けた実行 ID（無ければ `None`）。
pub(crate) fn parent_run_id() -> Option<i64> {
    std::env::var("SAI_PARENT_RUN_ID")
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok())
}

/// 提案の実行者を作る（`--run` / `--block`、無ければ環境変数）。
pub(crate) fn actor_of(run: Option<i64>, block: Option<String>) -> ops::Actor {
    let run_id = run.or_else(parent_run_id);
    let block = block
        .or_else(|| std::env::var("SAI_BLOCK_NAME").ok())
        .map(|text| text.trim().to_string())
        .unwrap_or_default();
    ops::Actor::Run { run_id, block }
}

// ---- 問いに結び付いた検索の記録（find / search / MCP） ------------------------

/// 検索が結び付いた節点と問い（`find` と `search` の `--node` / `--question`）。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SearchRefs {
    /// 検索した節点。`--question` だけなら、その問いの節点。
    pub node: Option<i64>,
    /// 検索した問い。
    pub question: Option<i64>,
    /// `--node` で明示した節点（検索の範囲を狭めるのに使う。`--question` からは引かない）。
    pub given: Option<i64>,
}

impl SearchRefs {
    /// 節点か問いのどちらかがある（= 検索の記録に残す）。
    pub(crate) fn is_recorded(&self) -> bool {
        self.node.is_some() || self.question.is_some()
    }
}

/// `find` / `search` / MCP の `find` の引数検査。
///
/// `--node` と `--question` は `N12`・`Q7` の形だけを受け付ける。
/// `--question` だけなら、その問いの節点を節点にする（記録に残すため）。
pub(crate) fn parse_search_refs(
    conn: &Connection,
    node: Option<&str>,
    question: Option<&str>,
) -> Result<SearchRefs> {
    let given_node = node.map(|text| parse_id(text, 'N')).transpose()?;
    let question = question.map(|text| parse_id(text, 'Q')).transpose()?;
    let node = match (given_node, question) {
        (Some(node), _) => Some(node),
        (None, Some(id)) => Some(
            store::get_question(conn, id)
                .map_err(|err| anyhow!("問いの節点を読めません: {err}"))?
                .node_id,
        ),
        (None, None) => None,
    };
    Ok(SearchRefs {
        node,
        question,
        given: given_node,
    })
}

/// 検索の記録に残す内容（`find` / `search` / MCP の `find` で同じものを使う）。
pub(crate) struct SearchLog<'a> {
    /// 検索が結び付いた節点と問い。
    pub refs: &'a SearchRefs,
    /// `find` / `search` / `expand`。
    pub tool: &'a str,
    /// 検索語。
    pub query: &'a str,
    /// 検索の範囲（C-02 の書き方）。
    pub scope: &'a str,
    /// 範囲を広げた理由（`--widen`）。
    pub widen_reason: &'a str,
    /// 当たった件数。
    pub hits: i64,
    /// 当たった論文の番号。
    pub hit_papers: &'a [i64],
    /// 作った検索セットの番号。
    pub set_id: Option<i64>,
}

/// 検索の記録（`sci_searches` に 1 行）を残す（R-R3 / R-R4）。
///
/// 節点も問いも無いときは記録しない（`find` / `search` を今までどおり使えるようにするため）。
pub(crate) fn record_search(conn: &Connection, log: &SearchLog<'_>) -> Result<()> {
    if !log.refs.is_recorded() {
        return Ok(());
    }
    store::insert_search(
        conn,
        &store::NewSearch {
            run_id: parent_run_id(),
            node_id: log.refs.node,
            question_id: log.refs.question,
            tool: log.tool,
            query: log.query,
            scope: log.scope,
            widen_reason: log.widen_reason,
            hits: log.hits,
            hit_papers: log.hit_papers,
            set_id: log.set_id,
        },
    )
    .map(|_| ())
    .context("検索の記録を残せませんでした")
}

/// 同じ問い・同じ範囲の前回の検索を 1 行にする（R-R4。無ければ `None`）。
///
/// 節点か問いのどちらも無いときは「前」を見せない（何が前かが決まらないため）。
pub(crate) fn previous_search_line(
    conn: &Connection,
    refs: &SearchRefs,
    scope: &str,
) -> Option<String> {
    let node = refs.node?;
    let rows = store::list_searches(conn, Some(node)).ok()?;
    let found = rows
        .iter()
        .find(|row| row.question_id == refs.question && row.scope == scope)?;
    let papers = if found.hit_papers.is_empty() {
        String::new()
    } else {
        let list = found
            .hit_papers
            .iter()
            .map(i64::to_string)
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
/// 節点や論文の集合が無いのに `--subtree` / `--each` が来たときは、黙って Library
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

/// 検索セットに何のための検索かを書く Fields を作る（1 つも無ければ `None`）。
pub(crate) fn set_fields(
    refs: &SearchRefs,
    purpose: Option<&str>,
    extract: Option<&str>,
) -> Option<search_sets::SciSetFields> {
    if refs.node.is_none() && refs.question.is_none() && purpose.is_none() && extract.is_none() {
        return None;
    }
    Some(search_sets::SciSetFields {
        node: refs.node,
        question: refs.question,
        purpose: purpose.map(str::to_string),
        extract: extract.map(str::to_string),
    })
}

// ---- 人が読む文 -------------------------------------------------------------

/// `sci status` の文（種の文・関門・負債・木・次の一手）。
fn status_text(status: &compute::Status) -> String {
    if !status.initialized {
        return "研究方針の木がまだありません。`sai-moon-paper sci init --type <型>` で始めてください。\n"
            .to_string();
    }
    let mut text = String::new();
    let seed = status
        .seed
        .as_ref()
        .map(|seed| seed.text.clone())
        .unwrap_or_default();
    if seed.is_empty() {
        text.push_str("種の文がまだありません\n");
    } else {
        text.push_str(&format!("{seed}\n"));
    }
    if let Some(gate) = &status.gate {
        text.push_str(&format!(
            "関門 {}/{} {}\n",
            gate.current, gate.total, gate.name
        ));
    }
    let debts = &status.debts;
    text.push_str(&format!(
        "負債: 仕分け {}・未反映 {}・未選別 {}・振り返り {}\n",
        debts.pending_proposals,
        debts.unreflected_papers,
        debts.unscreened_sets,
        debts.empty_reviews
    ));
    for node in &status.nodes {
        text.push_str(&node_line(node));
    }
    if !status.next.is_empty() {
        text.push_str("次の一手:\n");
        for (index, next) in status.next.iter().enumerate() {
            let mut head = String::new();
            if let Some(node) = &next.node {
                head.push_str(node);
                head.push(' ');
            }
            if let Some(block) = &next.block {
                head.push_str(block);
                head.push(' ');
            }
            text.push_str(&format!(
                "{}. {}{}（{}）\n",
                index + 1,
                head,
                next.purpose,
                next.why
            ));
        }
    }
    text
}

/// status の 1 節点 1 行（深さ分だけ字下げする）。
fn node_line(node: &compute::NodeRow) -> String {
    let indent = "  ".repeat(node.depth.max(0) as usize);
    let mut mark = state_label(&node.state);
    if let Some(blank) = &node.blank {
        mark.push('・');
        mark.push_str(&blank_label(blank));
    }
    if !node.marks.is_empty() {
        mark.push('・');
        mark.push_str(
            &node
                .marks
                .iter()
                .map(|mark| blank_label(mark))
                .collect::<Vec<_>>()
                .join("・"),
        );
    }
    let counts = &node.counts;
    let mut extra = Vec::new();
    if counts.questions_open > 0 {
        extra.push(format!("問い {}", counts.questions_open));
    }
    if counts.options > 0 {
        extra.push(format!("候補 {}", counts.options));
    }
    if counts.support > 0 {
        extra.push(format!("支持 {}", counts.support));
    }
    if counts.refute > 0 {
        extra.push(format!("反証 {}", counts.refute));
    }
    if counts.papers > 0 {
        extra.push(format!("論文 {}", counts.papers));
    }
    if counts.pending > 0 {
        extra.push(format!("仕分け {}", counts.pending));
    }
    if extra.is_empty() {
        format!("{indent}{} {} [{mark}]\n", node.id, node.title)
    } else {
        format!(
            "{indent}{} {} [{mark}] {}\n",
            node.id,
            node.title,
            extra.join("・")
        )
    }
}

/// `sci node <N>` の文（題・状態・問い・候補・知見・論文・決定の記録）。
fn node_text(detail: &compute::NodeDetail) -> String {
    let node = &detail.node;
    let mut text = format!("{} {}", node.id, node.title);
    text.push_str(&format!(" [{}]\n", state_label(&node.state)));
    if let Some(blank) = &node.blank {
        text.push_str(&format!("空欄: {}\n", blank_label(blank)));
    }
    for mark in &node.marks {
        text.push_str(&format!("印: {}\n", blank_label(mark)));
    }
    if !node.body.is_empty() {
        text.push_str(&format!("\n{}\n", node.body));
    }
    if let Some(guide) = &detail.guide {
        text.push_str(&format!(
            "\n項目ガイド {}: {}{}\n",
            guide.name,
            guide.explain,
            if guide.condition_met {
                "（埋まった条件は満たしている）"
            } else {
                "（埋まった条件はまだ）"
            }
        ));
    }
    if !detail.questions.is_empty() {
        text.push_str("\n問い:\n");
        for question in &detail.questions {
            text.push_str(&format!(
                "  {} {}（{}）\n",
                question.id,
                question.text,
                question_state_label(&question.state)
            ));
        }
    }
    if !detail.options.is_empty() {
        text.push_str("\n候補:\n");
        for option in &detail.options {
            text.push_str(&format!(
                "  {} {} [{}] 支持 {}・反証 {}・論文 {}\n",
                option.id,
                option.title,
                state_label(&option.state),
                option.support,
                option.refute,
                option.papers
            ));
        }
    }
    if !detail.findings.is_empty() {
        text.push_str("\n知見:\n");
        for finding in &detail.findings {
            let mut line = format!(
                "  {}［{}］{}",
                finding.id,
                polarity_label(&finding.polarity),
                one_line(&finding.text)
            );
            if !finding.cites.is_empty() {
                line.push_str(&format!(" [{}]", finding.cites.join(" ")));
            }
            text.push_str(&line);
            text.push('\n');
        }
    }
    if !detail.links.is_empty() {
        text.push_str("\n論文:\n");
        for link in &detail.links {
            let mut line = format!("  {} {} {}", link.id, link.target, one_line(&link.title));
            if let Some(role) = &link.role {
                line.push_str(&format!(
                    "（{}・{}）",
                    role_label(role),
                    read_label(link.read_state.as_deref().unwrap_or("unread"))
                ));
            } else if let Some(state) = &link.read_state {
                line.push_str(&format!("（{}）", read_label(state)));
            }
            text.push_str(&line);
            text.push('\n');
        }
    }
    if !detail.decisions.is_empty() {
        text.push_str("\n決定の記録:\n");
        for decision in &detail.decisions {
            let mut line = format!(
                "  {} {} {}",
                decision.created_at,
                action_label(&decision.action),
                decision.actor
            );
            if let Some(option) = &decision.option {
                line.push_str(&format!(" {option}"));
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
    text
}

/// `sci read <run_id>` の文（論文・役割・深さ・問い）。
fn read_text(card: &card::ReadCard) -> String {
    let mut text = format!(
        "run {}（{}）{} {}\n",
        card.run_id, card.owner, card.node.id, card.node.title
    );
    let year = card.paper.year.as_deref().unwrap_or("");
    let year = if year.is_empty() {
        String::new()
    } else {
        format!("（{year}）")
    };
    text.push_str(&format!(
        "論文: #{} {}{}\n",
        card.paper.id,
        one_line(&card.paper.title),
        year
    ));
    text.push_str(&format!("役割: {}\n", role_label(&card.role)));
    text.push_str(&format!("深さ: {}\n", depth_label(&card.depth)));
    if !card.purpose.is_empty() {
        text.push_str(&format!("目的: {}\n", one_line(&card.purpose)));
    }
    if !card.questions.is_empty() {
        text.push_str("問い:\n");
        for question in &card.questions {
            text.push_str(&format!("  {question}\n"));
        }
    }
    if !card.findings.is_empty() {
        text.push_str("知見:\n");
        for finding in &card.findings {
            text.push_str(&format!(
                "  {}［{}］{}\n",
                finding.id,
                polarity_label(&finding.polarity),
                one_line(&finding.text)
            ));
        }
    }
    text
}

/// 提案の行を phone と同じ形にする（C-08 の `pending` に `node` を足す）。
pub(crate) fn proposal_json(row: &store::Proposal) -> Value {
    json!({
        "id": sci::fmt_ref('P', row.id),
        "op": row.op,
        "node": row.node_id.map(|id| sci::fmt_ref('N', id)),
        "payload": row.payload,
        "reason": row.reason,
        "run_id": row.run_id,
        "block": row.block,
        "created_at": row.created_at,
    })
}

/// 墓標の種類の表示名。
fn target_kind_label(kind: &str) -> &'static str {
    if kind == "paper" { "論文" } else { "候補" }
}

// ---- 表示名 -----------------------------------------------------------------

/// 節点の状態の表示名（項目と候補）。
fn state_label(state: &str) -> String {
    const LABELS: [(&str, &str); 7] = [
        ("open", "未決定"),
        ("tentative", "仮決定"),
        ("decided", "決定"),
        ("excluded", "対象外"),
        ("candidate", "候補"),
        ("selected", "選択"),
        ("rejected", "却下"),
    ];
    label_of(&LABELS, state)
}

/// 空欄の種類の表示名（印も同じ表を使う）。
fn blank_label(blank: &str) -> String {
    const LABELS: [(&str, &str); 7] = [
        ("untouched", "未着手"),
        ("in_progress", "調査中"),
        ("insufficient", "情報不足"),
        ("awaiting_decision", "判断待ち"),
        ("conflict", "対立"),
        ("weak_evidence", "根拠不足"),
        ("recheck", "要再確認"),
    ];
    label_of(&LABELS, blank)
}

/// 問いの状態の表示名。
fn question_state_label(state: &str) -> String {
    const LABELS: [(&str, &str); 3] = [
        ("open", "答え待ち"),
        ("answered", "回答済み"),
        ("withdrawn", "取り下げ"),
    ];
    label_of(&LABELS, state)
}

/// 知見の向きの表示名。
fn polarity_label(polarity: &str) -> String {
    const LABELS: [(&str, &str); 4] = [
        ("support", "支持"),
        ("refute", "反証"),
        ("neutral", "中立"),
        ("none", "該当なし"),
    ];
    label_of(&LABELS, polarity)
}

/// 論文の役割の表示名。
fn role_label(role: &str) -> String {
    const LABELS: [(&str, &str); 7] = [
        ("exemplar", "代表例"),
        ("compare", "比較対象"),
        ("issue_evidence", "課題の根拠"),
        ("method_source", "手法の供給元"),
        ("data_source", "データの出典"),
        ("background", "背景"),
        ("counter", "反証"),
    ];
    label_of(&LABELS, role)
}

/// 論文の読み具合の表示名。
fn read_label(state: &str) -> String {
    const LABELS: [(&str, &str); 4] = [
        ("unread", "未読"),
        ("read", "読んだ・未反映"),
        ("reflected", "反映済み"),
        ("irrelevant", "関係なし"),
    ];
    label_of(&LABELS, state)
}

/// 読む深さの表示名。
fn depth_label(depth: &str) -> String {
    const LABELS: [(&str, &str); 3] = [("abstract", "抄録"), ("figures", "図"), ("full", "精読")];
    label_of(&LABELS, depth)
}

/// 決定の記録の種類の表示名。
fn action_label(action: &str) -> String {
    const LABELS: [(&str, &str); 7] = [
        ("decide", "決定"),
        ("tentative", "仮決定"),
        ("reopen", "再開"),
        ("exclude_item", "対象外にする"),
        ("reject_option", "候補の却下"),
        ("dormant_option", "候補の休眠"),
        ("restore_option", "候補に戻す"),
    ];
    label_of(&LABELS, action)
}

/// 表に無い値はその値そのものを返す。
fn label_of(table: &[(&str, &str)], value: &str) -> String {
    table
        .iter()
        .find(|(key, _)| *key == value)
        .map_or_else(|| value.to_string(), |(_, name)| (*name).to_string())
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

    /// `find` の範囲の引数は `--node` / `--papers` が要る（`--subtree` と `--each`）。
    #[test]
    fn find_subtree_requires_node() {
        // 範囲の指定があれば通る。
        assert!(check_find_scope(true, true, Some(2), true).is_ok());
        assert!(check_find_scope(false, false, Some(2), false).is_ok());
        assert!(check_find_scope(false, false, None, true).is_ok());

        // `--subtree` と `--each` は範囲の指定が無いと受理しない。
        for (subtree, each, papers_empty, message) in [
            (true, None, true, "--subtree は --node と一緒に使います"),
            (false, Some(2), true, "--each は --papers か --node と一緒に使います"),
        ] {
            let err = check_find_scope(subtree, false, each, papers_empty).unwrap_err();
            assert_eq!(format!("{err:#}"), message);
        }
    }

    /// `find` の新しい引数は clap で読める（`--papers` はカンマ区切り）。
    #[test]
    fn find_parses_new_arguments() {
        let cli = crate::Cli::try_parse_from([
            "sai-moon-paper",
            "find",
            "発作",
            "--node",
            "N12",
            "--subtree",
            "--each",
            "2",
            "--question",
            "Q7",
            "--widen",
            "範囲を広げる",
            "--papers",
            "37,40",
            "--json",
        ])
        .unwrap();
        let Some(crate::Cmd::Find {
            papers,
            node,
            subtree,
            each,
            question,
            widen,
            json,
            ..
        }) = cli.cmd
        else {
            panic!("find として読めた");
        };
        assert_eq!(papers, vec![37, 40]);
        assert_eq!(node.as_deref(), Some("N12"));
        assert!(subtree);
        assert_eq!(each, Some(2));
        assert_eq!(question.as_deref(), Some("Q7"));
        assert_eq!(widen.as_deref(), Some("範囲を広げる"));
        assert!(json);
    }

    /// `search` と `set create` の新しい引数は clap で読める。
    #[test]
    fn search_and_set_create_parse_sci_arguments() {
        let cli = crate::Cli::try_parse_from([
            "sai-moon-paper",
            "search",
            "発作部位",
            "--node",
            "N12",
            "--question",
            "Q7",
            "--purpose",
            "発作部位の候補を探す",
            "--extract",
            "手法の候補",
            "--widen",
            "範囲を広げる",
        ])
        .unwrap();
        let Some(crate::Cmd::Search {
            node,
            question,
            purpose,
            extract,
            ..
        }) = cli.cmd
        else {
            panic!("search として読めた");
        };
        assert_eq!(node.as_deref(), Some("N12"));
        assert_eq!(question.as_deref(), Some("Q7"));
        assert_eq!(purpose.as_deref(), Some("発作部位の候補を探す"));
        assert_eq!(extract.as_deref(), Some("手法の候補"));

        assert!(crate::Cli::try_parse_from([
            "sai-moon-paper",
            "set",
            "create",
            "候補",
            "--query",
            "発作部位",
            "--node",
            "N12",
            "--purpose",
            "候補を探す",
        ])
        .is_ok());
    }

    #[test]
    fn parse_assume_accepts_node_value() {
        let parsed = parse_assume(&["N9=発作部位の予測（仮）".to_string()]).unwrap();
        assert_eq!(parsed, vec![(9, "発作部位の予測（仮）".to_string())]);

        for bad in ["9=値", "N=値", "値", "Q9=値"] {
            let err = parse_assume(&[bad.to_string()]).unwrap_err();
            assert!(
                format!("{err:#}").contains("--assume は N<番号>=<値>"),
                "{bad}: {err:#}"
            );
        }
    }

    #[test]
    fn triage_flags_are_exclusive() {
        // 1 つだけなら通る。
        let cli = crate::Cli::try_parse_from(["sai-moon-paper", "sci", "triage", "P1", "--accept"])
            .unwrap();
        let Some(crate::Cmd::Sci {
            cmd: SciCmd::Triage { accept, reject, .. },
        }) = cli.cmd
        else {
            panic!("sci triage として読めた");
        };
        assert!(accept);
        assert!(!reject);

        // 2 つ指定すると失敗する。
        assert!(
            crate::Cli::try_parse_from([
                "sai-moon-paper",
                "sci",
                "triage",
                "P1",
                "--accept",
                "--reject"
            ])
            .is_err()
        );
        assert!(
            crate::Cli::try_parse_from([
                "sai-moon-paper",
                "sci",
                "triage",
                "P1",
                "--reject",
                "--revert"
            ])
            .is_err()
        );
        // どれも指定しないと失敗する。
        assert!(crate::Cli::try_parse_from(["sai-moon-paper", "sci", "triage", "P1"]).is_err());
    }

    #[test]
    fn status_text_has_seed_gate_debts_and_next() {
        let status = compute::Status {
            initialized: true,
            type_info: Some(compute::TypeInfo {
                id: "problem".to_string(),
                name: "課題駆動".to_string(),
            }),
            seed: Some(compute::SeedSentence {
                text: "「頭皮脳波で［？小目的］を行う。」".to_string(),
                parts: Vec::new(),
            }),
            gate: Some(compute::GateState {
                current: 2,
                total: 7,
                name: "小目的".to_string(),
                list: Vec::new(),
            }),
            debts: compute::Debts {
                pending_proposals: 3,
                unreflected_papers: 2,
                unscreened_sets: 1,
                empty_reviews: 0,
            },
            nodes: vec![
                compute::NodeRow {
                    id: "N1".to_string(),
                    kind: "item".to_string(),
                    title: "背景".to_string(),
                    state: "decided".to_string(),
                    depth: 0,
                    ..Default::default()
                },
                compute::NodeRow {
                    id: "N12".to_string(),
                    kind: "item".to_string(),
                    title: "小目的".to_string(),
                    state: "open".to_string(),
                    depth: 1,
                    blank: Some("awaiting_decision".to_string()),
                    counts: compute::NodeCounts {
                        options: 2,
                        support: 3,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ],
            next: vec![
                compute::NextAction {
                    node: None,
                    block: None,
                    purpose: "仕分けていない提案が 3 件あります".to_string(),
                    expect: String::new(),
                    why: "負債".to_string(),
                    kind: "debt".to_string(),
                },
                compute::NextAction {
                    node: Some("N12".to_string()),
                    block: Some("sci-compare".to_string()),
                    purpose: "小目的の候補を価値の軸で比べる".to_string(),
                    expect: "比較表と推奨".to_string(),
                    why: "判断待ち".to_string(),
                    kind: "blank".to_string(),
                },
            ],
            ..Default::default()
        };
        let text = status_text(&status);
        assert!(
            text.starts_with("「頭皮脳波で［？小目的］を行う。」\n"),
            "{text}"
        );
        assert!(text.contains("関門 2/7 小目的\n"), "{text}");
        assert!(
            text.contains("負債: 仕分け 3・未反映 2・未選別 1・振り返り 0\n"),
            "{text}"
        );
        assert!(text.contains("N1 背景 [決定]\n"), "{text}");
        assert!(
            text.contains("  N12 小目的 [未決定・判断待ち] 候補 2・支持 3\n"),
            "{text}"
        );
        assert!(
            text.contains("1. 仕分けていない提案が 3 件あります（負債）\n"),
            "{text}"
        );
        assert!(
            text.contains("2. N12 sci-compare 小目的の候補を価値の軸で比べる（判断待ち）\n"),
            "{text}"
        );
    }

    #[test]
    fn status_text_says_not_started_before_init() {
        let text = status_text(&compute::Status::default());
        assert!(text.contains("まだありません"), "{text}");
    }

    #[test]
    fn read_stdin_json_rejects_empty_and_broken() {
        assert!(parse_stdin_text("").is_err());
        assert!(parse_stdin_text("  \n").is_err());
        assert!(parse_stdin_text("{oops").is_err());
        let ok = parse_stdin_text("{\"op\":\"decide\"}").unwrap();
        assert_eq!(ok["op"], "decide");
    }

    /// 知らない型で始めても、部品も `state.pre-sci.md` も作らない。
    ///
    /// 型を確かめる前に書くと、失敗したのに `.sai/` が残る。
    #[test]
    fn init_with_assets_checks_type_before_writing() {
        let root = std::env::temp_dir()
            .join(format!("sai-paper-sci-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let err = init_with_assets(&root, "todo", false).err().unwrap();
        assert!(format!("{err:#}").contains("知らない研究の型です"), "{err:#}");
        // 何も置いていない（型を確かめたところで止まる）。
        assert!(!root.join(".sai").exists(), "型を確かめる前にファイルを置いた");
        assert!(!root.join(".mcp.json").exists(), "型を確かめる前にファイルを置いた");
        let _ = std::fs::remove_dir_all(&root);
    }
}
