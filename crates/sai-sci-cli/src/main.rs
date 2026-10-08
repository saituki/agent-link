//! `sai-sci` — 研究の CLI。research-tree・研究の記録・
//! エージェントの報告の登録と検査を 1 つの実行ファイルにまとめる。
//!
//! 失敗の形は CLI 共通の約束に従う（終了コード 2・3・4。失敗は標準エラーの最後の行に
//! 日本語 1 行。`--json` の命令でも標準出力には出さない）。論文の検索は `sai-paper`
//! の CLI を子として呼び、sai-sci は範囲の決定と記録だけを行う。

use anyhow::{Context, Result};
use chrono::{Duration, Local, NaiveDate};
use clap::{Parser, Subcommand};
use sai_paper_core::{Config, Database, Paths};
use sai_sci_core::bins;
use sai_sci_core::rt::{self, card, compute, defs, store};
use serde_json::json;
use std::path::{Path, PathBuf};

mod agent_assets;
mod attention;
mod cli_error;
mod dsr;
mod find_search;
mod phone_sci;
mod progress;
mod report;
mod schema;
mod sci;
mod understanding;

/// `sai-sci` の命令。
#[derive(Parser, Debug)]
#[command(
    name = "sai-sci",
    version,
    about = "sai-sci — research-tree・研究の記録・エージェントの報告"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// 研究プロジェクトを初期化する（sai-link・sai-paper の初期化と研究の assets を配る）
    Init {
        /// RESEARCH_TYPE（problem / phenomenon / method / data / improve）
        #[arg(long = "type", value_name = "型")]
        type_id: Option<String>,
        /// 分野の観点（medical / signal-ml / general）
        #[arg(long, default_value = "general", value_parser = ["medical", "signal-ml", "general"])]
        domain: String,
        /// sai-link の初期化で使うプロジェクト名
        #[arg(long)]
        name: Option<String>,
        /// assets を新しい版へ更新する（古い物は退避する）
        #[arg(long)]
        update: bool,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 現況（進み具合・SEED・research-tree・Next Action）を出す
    Status {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
        /// Node の数・Next Action だけを出す（いつも JSON）
        #[arg(long)]
        brief: bool,
    },
    /// 使える RESEARCH_TYPE を一覧する
    Types {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// RESEARCH_TYPE を利用者全体の置き場に保存する（標準入力に RESEARCH_TYPE の Markdown）
    #[command(name = "type-save")]
    TypeSave,
    /// Node 1 つの詳細を出す
    Node {
        /// Node の ID（`Q12`・`A5` など）
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
        /// blind（sci_mode: blind_first）でも research-tree を出す
        #[arg(long)]
        tree: bool,
    },
    /// agent の op を出す（標準入力に `{"ops":[…]}`。全部通れば書き、1 件でも通らなければ何も書かない）
    Op {
        /// 実行 ID（無ければ環境変数 SAI_PARENT_RUN_ID）
        #[arg(long)]
        run: Option<i64>,
        /// 出したブロックの id（無ければ環境変数 SAI_BLOCK_NAME）
        #[arg(long)]
        block: Option<String>,
    },
    /// 質問で取った回答を承認として記録する（質問フックが呼ぶ。出力は常に JSON）
    Approval {
        /// サブコマンド
        #[command(subcommand)]
        cmd: ApprovalCmd,
    },
    /// 利用者の操作を 1 つ反映する（標準入力に 1 件の JSON）
    Apply {
        /// JSON で出力する（出力は常に JSON。Web が付けるので受け付ける）
        #[arg(long)]
        json: bool,
    },
    /// Node のための実行（CARD）を作る（標準入力の C-CARD は --from-json のときだけ読む）
    Card {
        /// CARD 命令の引数
        #[command(flatten)]
        args: sci::SciCardArgs,
        /// Web から受け取る C-CARD の JSON を標準入力から読む
        #[arg(long)]
        from_json: bool,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// READ_CARD（runner が user の CARD）の中身を出す
    Read {
        /// 実行 ID
        run_id: i64,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// state.md と tree.md を書き出す
    Export,
    /// 内部を検索する。環境変数 `SAI_TARGET_NODE` があれば、その Node に記録を結ぶ（論文の検索は sai-paper が行う）
    Find {
        #[command(flatten)]
        args: find_search::FindArgs,
    },
    /// 論文を検索する。環境変数 `SAI_TARGET_NODE` があれば、その Node に記録を結ぶ（検索は sai-paper が行う）
    Search {
        #[command(flatten)]
        args: find_search::SearchArgs,
    },
    /// エージェントに渡すプロジェクトの状況のまとめを作る
    Brief {
        /// `.sai/template/project-brief.md` に書く（既定は標準出力）
        #[arg(long)]
        write: bool,
    },
    /// 日ごとの研究日誌を出す（記録を並べるだけで LLM は使わない）
    Journal {
        /// 今日を含む日数（既定 7）
        #[arg(long, default_value_t = 7)]
        days: i64,
        /// この日だけを出す（YYYY-MM-DD）。指定すると `--days` は無視される
        #[arg(long)]
        date: Option<String>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// プロジェクトの進み具合と「読むべき論文」を出す
    Progress {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// トピックごとの「理解の状態」の記録
    Understanding {
        /// サブコマンド
        #[command(subcommand)]
        cmd: understanding::UnderstandingCmd,
    },
    /// 要対応の一覧と「対応済み」の記録
    Attention {
        /// サブコマンド
        #[command(subcommand)]
        cmd: attention::AttentionCmd,
    },
    /// 現況（state.md）・考え（mind.md）・メモ
    Project {
        /// サブコマンド
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    /// DSR の report を確かめる（LLM は使わない）
    Dsr {
        /// サブコマンド
        #[command(subcommand)]
        cmd: dsr::DsrCmd,
    },
    /// agent の report の登録・参照
    Report {
        /// サブコマンド
        #[command(subcommand)]
        cmd: report::ReportCmd,
    },
    /// 命令の一覧を JSON で出す
    Schema {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
}

/// `sai-sci approval` のサブコマンド。
#[derive(Subcommand, Debug)]
enum ApprovalCmd {
    /// 質問の回答を 1 件記録する（同じ qid は 2 回書けない・終了コード 4）
    Record {
        /// 質問の ID
        #[arg(long)]
        qid: String,
        /// 質問の文
        #[arg(long)]
        question: String,
        /// 回答（空白を除いて「許可する」のときだけ op に使える）
        #[arg(long)]
        answer: String,
        /// 回答した人
        #[arg(long)]
        answered_by: String,
        /// 実行 ID（無ければ環境変数 SAI_PARENT_RUN_ID）
        #[arg(long)]
        run: Option<i64>,
        /// JSON で出力する（出力は常に JSON。Web が付けるので受け付ける）
        #[arg(long)]
        json: bool,
    },
}

/// `sai-sci project` のサブコマンド。
#[derive(Subcommand, Debug)]
enum ProjectCmd {
    /// 現況（state.md）・考え（mind.md）・状況のまとめ・要対応を出す
    Show {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// mind.md の「## メモ」に 1 行足す
    Memo {
        /// 1 行の文（改行を含む・空の文は終了コード 2）
        text: String,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// mind.md の全文を標準入力の内容で置き換える
    Mind {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() {
    let code = run().await;
    std::process::exit(code);
}

/// CLI の入口。終了コードを返す（CLI 共通の約束）。
async fn run() -> i32 {
    // プロジェクトルートの .env を読み込む（既存の環境変数は上書きしない）。
    sai_paper_core::paths::load_project_dotenv();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => return clap_exit_code(err),
    };
    // 記録の出どころ（SAI_ORIGIN、無ければ cli）。
    sai_paper_core::jobs::set_origin(origin_name());
    // `init` と `schema` 以外は研究プロジェクト（`.sai/paper`）が要る。
    if !matches!(cli.cmd, None | Some(Cmd::Init { .. }) | Some(Cmd::Schema { .. }))
        && let Err(err) = require_paper_dir()
    {
        return report_error(&err);
    }
    match dispatch(cli.cmd).await {
        Ok(()) => 0,
        Err(err) => report_error(&err),
    }
}

/// 記録の出どころ（CLI 共通の約束の 6）。
fn origin_name() -> &'static str {
    match std::env::var("SAI_ORIGIN") {
        Ok(value) if value == "phone" => "phone",
        _ => "cli",
    }
}

/// 研究プロジェクト（`.sai/paper`）があるか確かめる（`init`・`schema` 以外）。
fn require_paper_dir() -> Result<()> {
    let Some(root) = sai_paper_core::paths::project_root() else {
        return Err(cli_error::CliError::input(
            "プロジェクトが見つかりません（`sai-link init` を実行してください）",
        )
        .into());
    };
    if !root.join(".sai").join("paper").is_dir() {
        return Err(cli_error::CliError::input(
            "研究プロジェクト（.sai/paper）がありません（`sai-sci init` を実行してください）",
        )
        .into());
    }
    Ok(())
}

/// papr.db を開く。
pub(crate) fn open_database() -> Result<Database> {
    let paths = Paths::discover().context("研究プロジェクト（.sai/paper）が見つかりません")?;
    Database::open(&paths.database_file).context("papr.db を開けませんでした")
}

/// clap の使い方の誤りの終了コード。`--help`・`--version` は 0。
/// 誤りのときは clap の複数行の文を出さず、日本語の 1 行だけを標準エラーに書く。
fn clap_exit_code(err: clap::Error) -> i32 {
    use clap::error::ErrorKind;
    let kind = err.kind();
    if matches!(kind, ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
        let _ = err.print();
        return 0;
    }
    // 命令が足りないときは使い方を出す（従来どおり）
    if kind == ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
        let _ = err.print();
    }
    let text = err.to_string();
    // clap の文の最初の段落（空行まで）を 1 行にまとめる
    let first = text
        .trim_start()
        .split("\n\n")
        .next()
        .unwrap_or("")
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");
    let stripped = first.strip_prefix("error: ").unwrap_or(&first);
    eprintln!("引数の誤り: {stripped}");
    2
}

/// エラーを標準エラーの最後の行に 1 行で出し、約束の終了コードを返す。
fn report_error(err: &anyhow::Error) -> i32 {
    let code = exit_code_of(err);
    let message = format!("{err:#}").replace('\n', " ");
    eprintln!("{message}");
    code
}

/// 約束の終了コードを決める（2・3・4。それ以外は 1）。
fn exit_code_of(err: &anyhow::Error) -> i32 {
    for cause in err.chain() {
        if let Some(cli_error) = cause.downcast_ref::<cli_error::CliError>() {
            return cli_error.code;
        }
        if let Some(rt_error) = cause.downcast_ref::<rt::RtError>() {
            return match rt_error {
                rt::RtError::Invalid(_) => 2,
                rt::RtError::NotFound(_) => 3,
                rt::RtError::Conflict(_) => 4,
                _ => 1,
            };
        }
        if let Some(config_error) = cause.downcast_ref::<sai_paper_core::config::ConfigError>() {
            return match config_error {
                sai_paper_core::config::ConfigError::ProjectNotFound
                | sai_paper_core::config::ConfigError::PaperDirMissing => 2,
                _ => 1,
            };
        }
    }
    1
}

/// 子の実行ファイルを呼び、失敗を約束の終了コードに直す。
pub(crate) fn run_child(
    bin: &Path,
    name: &str,
    command: &str,
    args: &[String],
    root: &Path,
) -> Result<String> {
    bins::run(bin, args, root, None).map_err(|err| {
        // 子の 2・3・4 はそのまま。ほかは 1。
        let code = if matches!(err.code, 2 | 3 | 4) { err.code } else { 1 };
        let message = if err.message.is_empty() {
            format!("{name} {command} に失敗しました")
        } else {
            format!("{name} {command} に失敗しました: {}", err.message)
        };
        cli_error::CliError { code, message }.into()
    })
}

/// 命令を振り分ける。
async fn dispatch(cmd: Option<Cmd>) -> Result<()> {
    match cmd {
        Some(Cmd::Init {
            type_id,
            domain,
            name,
            update,
            json,
        }) => handle_init(type_id.as_deref(), &domain, name.as_deref(), update, json),
        Some(Cmd::Status { json, brief }) => print_out(sci::status(json, brief)?),
        Some(Cmd::Types { json }) => print_out(sci::types(json)?),
        Some(Cmd::TypeSave) => print_out(sci::type_save()?),
        Some(Cmd::Node { node, json }) => print_out(sci::node_detail(&node, json)?),
        Some(Cmd::Context { run, tree }) => print_out(sci::context(run, tree)?),
        Some(Cmd::Op { run, block }) => print_out(sci::op(run, block)?),
        Some(Cmd::Apply { json: _ }) => print_out(sci::apply()?),
        Some(Cmd::Approval {
            cmd: ApprovalCmd::Record { qid, question, answer, answered_by, run, json: _ },
        }) => print_out(sci::approval_record(&qid, &question, &answer, &answered_by, run)?),
        Some(Cmd::Card {
            args,
            from_json,
            json,
        }) => handle_card(&args, from_json, json),
        Some(Cmd::Read { run_id, json }) => print_out(sci::read(run_id, json)?),
        Some(Cmd::Export) => print_out(sci::export()?),
        Some(Cmd::Find { args }) => find_search::find(&args),
        Some(Cmd::Search { args }) => find_search::search(&args),
        Some(Cmd::Brief { write }) => handle_brief(write),
        Some(Cmd::Journal { days, date, json }) => handle_journal(days, date.as_deref(), json),
        Some(Cmd::Progress { json }) => progress::handle(json),
        Some(Cmd::Understanding { cmd }) => understanding::handle(cmd),
        Some(Cmd::Attention { cmd }) => attention::handle(cmd),
        Some(Cmd::Project { cmd }) => handle_project(cmd),
        Some(Cmd::Dsr { cmd }) => dsr::handle(cmd),
        Some(Cmd::Report { cmd }) => report::handle_report(cmd).await,
        Some(Cmd::Schema { json }) => schema::print_schema(json),
        None => {
            println!(
                "sai-sci — 研究の CLI\n使い方:\n  sai-sci init [--type problem] [--domain general]\n  sai-sci status\n  sai-sci node Q12\n  sai-sci card --node Q12 --purpose …\n  sai-sci find \"語\" --node Q12"
            );
            Ok(())
        }
    }
}

/// `sci::Out` を標準出力へ出す（`--json` は 1 行、文はそのまま）。
fn print_out(out: sci::Out) -> Result<()> {
    match out {
        sci::Out::Text(text) => {
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
        }
        sci::Out::Json(value) => println!("{}", serde_json::to_string(&value)?),
    }
    Ok(())
}

/// `sai-sci card` — Node のための実行（CARD）を作る。
fn handle_card(args: &sci::SciCardArgs, from_json: bool, json: bool) -> Result<()> {
    let spec = if from_json {
        phone_sci::card_spec(&sci::read_stdin_text()?)?
    } else {
        args.to_spec()?
    };
    let value = sci::make_card(&spec)?;
    // `--from-json` は Web の `phone sci card` と同じ JSON を返す。
    if json || from_json {
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!(
            "run {} の CARD を作りました: {}",
            value["run_id"],
            value["path"].as_str().unwrap_or("")
        );
    }
    Ok(())
}

/// `sai-sci init` — 基盤・論文・研究の部品をこの順で配る。
///
/// 1. `--type` を確かめる（知らない RESEARCH_TYPE → 2、research-tree が既にある → 4）。
/// 2. `sai-link init` を呼ぶ。
/// 3. 研究の部品と設定を配る（[`agent_assets::install_sci_init`]）。
/// 4. `sai-paper init` を呼ぶ（`--domain` は渡さない）。
/// 5. `state.md` などと `project-brief.md` を作る（失敗は警告だけ）。
/// 6. `--type` があれば research-tree を作る。
///
/// 3 を 4 より前に行う理由: 古い `sai-paper init` が、無い部品を自分の版で
/// 書くのを避ける（先に sai-sci の物を置けば「ある」と見てそのままにする）。
fn handle_init(
    type_id: Option<&str>,
    domain: &str,
    name: Option<&str>,
    update: bool,
    json: bool,
) -> Result<()> {
    let root = init_root()?;
    let sai = root.join(".sai");

    // 1. 検査（RESEARCH_TYPE と、既にある research-tree）。
    if let Some(type_id) = type_id {
        let defs = defs::load(&root);
        sci::type_name(&defs, type_id)
            .map_err(|err| cli_error::CliError::input(format!("{err}")))?;
        let db_path = sai.join("paper").join("papr.db");
        if db_path.is_file() {
            let db = Database::open(&db_path).context("papr.db を開けませんでした")?;
            if store::is_initialized(db.connection()).map_err(|err| anyhow_err(&err))? {
                let roots = roots_at(&root);
                let existing = compute::status(db.connection(), &roots, &defs)
                    .ok()
                    .and_then(|status| status.type_info)
                    .map_or_else(|| type_id.to_string(), |found| found.name);
                return Err(cli_error::CliError::conflict(format!(
                    "既に始めています（RESEARCH_TYPE: {existing}）"
                ))
                .into());
            }
        }
    }

    let bin = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_else(|_| PathBuf::from("sai-sci"));
    let mut messages: Vec<String> = Vec::new();

    // 2. sai-link の初期化。
    let mut link_args = vec!["init".to_string()];
    if let Some(name) = name {
        link_args.push("--name".to_string());
        link_args.push(name.to_string());
    }
    if update {
        link_args.push("--update".to_string());
    }
    let link_out = run_child(&bins::link_bin(), bins::LINK_BIN_NAME, "init", &link_args, &root)?;
    messages.extend(lines_of(&link_out));

    // 3. 研究の部品と設定。
    messages.extend(agent_assets::install_sci_init(
        &root,
        &bin,
        domain,
        domain != "general",
        update,
    )?);

    // 4. sai-paper の初期化（`--domain` は渡さない）。
    let mut paper_args = vec!["init".to_string()];
    if update {
        paper_args.push("--update".to_string());
    }
    let paper_out = run_child(
        &bins::paper_bin(),
        bins::PAPER_BIN_NAME,
        "init",
        &paper_args,
        &root,
    )?;
    messages.extend(lines_of(&paper_out));

    // 5. state.md などと状況まとめ（失敗は警告だけ）。
    messages.extend(ensure_project_files_and_brief(&root));

    // 6. research-tree。
    let tree = if let Some(type_id) = type_id {
        let outcome = sci::init_tree_and_export(&root, type_id)?;
        if outcome.already {
            return Err(cli_error::CliError::conflict(format!(
                "既に始めています（RESEARCH_TYPE: {}）",
                outcome.type_name
            ))
            .into());
        }
        push_text(&mut messages, &outcome.text());
        Some(outcome.json())
    } else {
        None
    };

    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({ "messages": messages, "tree": tree }))?
        );
    } else {
        for line in &messages {
            println!("{line}");
        }
    }
    Ok(())
}

/// `init` の root（`SAI_PROJECT_ROOT`（空でない値）→ cwd。上へたどらない）。
fn init_root() -> Result<PathBuf> {
    let root = match std::env::var("SAI_PROJECT_ROOT") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value),
        _ => std::env::current_dir().context("cwd が取得できません")?,
    };
    if !root.is_dir() {
        return Err(cli_error::CliError::input(format!(
            "プロジェクトのフォルダがありません: {}",
            root.display()
        ))
        .into());
    }
    Ok(root)
}

/// `root` の研究プロジェクトの置き場。
fn roots_at(root: &Path) -> sai_paper_core::paths::PaperRoots {
    sai_paper_core::paths::PaperRoots {
        root: root.to_path_buf(),
        data_dir: root.join(".sai").join("paper"),
    }
}

/// mind.md / state.md のひな形を配り、状況まとめ（`project-brief.md`）を 1 回作る。
/// 失敗しても init は成功扱いにし、警告の行だけを返す。
fn ensure_project_files_and_brief(root: &Path) -> Vec<String> {
    let mut messages = Vec::new();
    let paper_dir = root.join(".sai").join("paper");
    match sai_sci_core::project_files::ensure(&paper_dir) {
        Ok(written) => {
            for path in written {
                messages.push(format!("✅ {} を作りました", path.display()));
            }
        }
        Err(error) => messages.push(format!("⚠ mind.md / state.md を作れませんでした: {error}")),
    }
    let roots = roots_at(root);
    let config = Config::load(&paper_dir.join(sai_paper_core::config::SETTINGS_FILE))
        .unwrap_or_default();
    let db = Database::open(&paper_dir.join("papr.db")).ok();
    let text = sai_sci_core::brief::build_brief(
        &roots,
        db.as_ref().map(Database::connection),
        &config.brief,
        &config.rag.default_index,
    );
    match sai_sci_core::brief::write_brief(&roots, &text) {
        Ok(path) => messages.push(format!("✅ 状況まとめを作りました: {}", path.display())),
        Err(error) => messages.push(format!("⚠ 状況まとめを作れませんでした: {error}")),
    }
    messages
}

/// 複数行の文字を 1 行ずつの一覧にする（空行は飛ばす）。
fn lines_of(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// `InitOutcome::text()` を表示用の行にする（最初の行だけ `✅ ` を付ける）。
fn push_text(messages: &mut Vec<String>, text: &str) {
    let mut first = true;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if first && !line.starts_with('⚠') {
            messages.push(format!("✅ {line}"));
        } else {
            messages.push(line.to_string());
        }
        first = false;
    }
}

/// `rt::RtError` を anyhow にする（型は保たないが、文だけ使う）。
fn anyhow_err(err: &rt::RtError) -> anyhow::Error {
    anyhow::anyhow!("{err}")
}

/// `sai-sci brief` — プロジェクトの状況のまとめを作る。
fn handle_brief(write: bool) -> Result<()> {
    let paths = Paths::discover()?;
    let roots =
        sai_paper_core::paths::paper_roots().context("研究プロジェクトのルートを解決できません")?;
    let config = Config::load(&paths.config_file).unwrap_or_default();
    let db = if paths.database_file.exists() {
        Database::open(&paths.database_file).ok()
    } else {
        None
    };
    // `--write` のときは、最初に成果物のフォルダを走査して report を登録する。
    if write
        && let Some(db) = db.as_ref()
        && let Err(error) = sai_sci_core::reports::scan(db.connection(), &roots.root)
    {
        eprintln!("report scan: {error}");
    }
    let text = sai_sci_core::brief::build_brief(
        &roots,
        db.as_ref().map(Database::connection),
        &config.brief,
        &config.rag.default_index,
    );
    if write {
        let path = sai_sci_core::brief::write_brief(&roots, &text)?;
        eprintln!("{}", path.display());
    } else {
        print!("{text}");
    }
    Ok(())
}

/// `sai-sci journal` — 日ごとの研究日誌を出す。
fn handle_journal(days: i64, date: Option<&str>, json: bool) -> Result<()> {
    let db = open_database()?;
    let roots = sai_paper_core::paths::paper_roots()
        .context("プロジェクトルート（.sai/paper）が見つかりません")?;
    let today = Local::now().date_naive();
    let (from, to) = match date {
        Some(value) => {
            let day = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
                cli_error::CliError::input(format!(
                    "--date は YYYY-MM-DD の形で指定してください: {value}"
                ))
            })?;
            (day, day)
        }
        None => {
            let days = days.max(1);
            (today - Duration::days(days - 1), today)
        }
    };
    let journal = sai_sci_core::journal::collect(db.connection(), &roots, from, to)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&journal)?);
    } else {
        print!("{}", sai_sci_core::journal::render_markdown(&journal));
    }
    Ok(())
}

/// `sai-sci project` — 現況・考え・メモ。
fn handle_project(cmd: ProjectCmd) -> Result<()> {
    match cmd {
        ProjectCmd::Show { json } => project_show(json),
        ProjectCmd::Memo { text, json } => project_memo(&text, json),
        ProjectCmd::Mind { json } => project_mind(json),
    }
}

/// `project show` — 現況・考え・状況のまとめ・要対応。
fn project_show(json: bool) -> Result<()> {
    let env = sci::open_env()?;
    let roots = &env.roots;
    let state =
        std::fs::read_to_string(sai_paper_core::paths::state_path(&roots.data_dir)).unwrap_or_default();
    let mind =
        std::fs::read_to_string(sai_paper_core::paths::mind_path(&roots.data_dir)).unwrap_or_default();
    let brief = sai_sci_core::brief::build_brief(
        roots,
        Some(env.conn()),
        &env.config.brief,
        &env.config.rag.default_index,
    );
    let attention = sai_sci_core::attention::collect(env.conn(), roots)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "state": state,
                "mind": mind,
                "brief": brief,
                "attention": attention,
            }))?
        );
    } else {
        println!("# 現況（state.md）\n{state}");
        println!("\n# 考え（mind.md）\n{mind}");
    }
    Ok(())
}

/// `project memo` — mind.md の「## メモ」に 1 行足す。
fn project_memo(text: &str, json: bool) -> Result<()> {
    let text = text.trim_end_matches(['\n', '\r']);
    if text.trim().is_empty() || text.contains(['\n', '\r']) {
        return Err(cli_error::CliError::input("メモは空でない 1 行にしてください").into());
    }
    let roots = sai_paper_core::paths::paper_roots()
        .context("プロジェクトルート（.sai/paper）が見つかりません")?;
    let stamp = Local::now().format("%m/%d %H:%M").to_string();
    let line = sai_sci_core::project_files::append_memo(&roots.data_dir, text, &stamp)?;
    if json {
        println!("{}", json!({ "ok": true, "line": line }));
    } else {
        println!("{line}");
    }
    Ok(())
}

/// `project mind` — 標準入力の全文で mind.md を置き換える。空・NUL は終了コード 2。
fn project_mind(json: bool) -> Result<()> {
    let text = std::io::read_to_string(std::io::stdin()).context("標準入力を読めません")?;
    if text.trim().is_empty() {
        return Err(cli_error::CliError::input("mind.md を空にはできません").into());
    }
    if text.contains('\0') {
        return Err(cli_error::CliError::input("mind.md に NUL は書けません").into());
    }
    let roots = sai_paper_core::paths::paper_roots()
        .context("プロジェクトルート（.sai/paper）が見つかりません")?;
    let bytes = sai_sci_core::project_files::write_mind(&roots.data_dir, &text)?;
    if json {
        println!("{}", json!({ "ok": true, "bytes": bytes }));
    } else {
        println!("mind.md を保存しました（{bytes} バイト）");
    }
    Ok(())
}

/// 使っていない値の警告を避けるため、`card` の型を参照しておく。
#[allow(dead_code)]
fn _assert_card_spec(_: &card::CardSpec) {}
