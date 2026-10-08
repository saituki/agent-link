//! # CLI インターフェース
//!
//! clap 4 を用いたコマンドライン引数定義と、各サブコマンドのハンドラを提供します。
//!
//! ## サブコマンド
//!
//! - `card …` — カード（実行予約）の参照・作成・実行
//! - `run …` — 実行の参照・操作
//! - `doc …` — 文書（waggle トークン）の読み書き
//! - `def …` — 定義ファイル（block / template / settings）の参照・編集
//! - `task …` — サブタスクの操作
//! - `project` / `session` / `workflow` / `schema` — 集計・検索・画面・命令一覧

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use serde_json;

use sai_link_engine::daemon;
use sai_link_engine::db::models::DbError;
use sai_link_engine::db::SaiStore;
use sai_link_engine::herdr::SplitDirection;
use sai_link_engine::orchestrator::{self, ExecutionOverrides, PipelineConfig};
use sai_link_engine::parser::parse_block_file;
use std::sync::Arc;

// ============================================================================
// 終了コードの分類（P18・5.7 の修正。定義は sai_link_engine::common に一元化）
// ============================================================================

/// 一般的な利用エラー（引数不足・前提条件違反など）。旧来の終了コード 1 と互換。
#[allow(dead_code)]
pub const EXIT_USAGE: i32 = sai_link_engine::common::EXIT_USAGE;
/// ブロック定義のパース・バリデーションエラー。
#[allow(dead_code)]
pub const EXIT_PARSE: i32 = sai_link_engine::common::EXIT_PARSE;
/// ファイル・ブロックが見つからない。
pub const EXIT_NOT_FOUND: i32 = sai_link_engine::common::EXIT_NOT_FOUND;
/// DB 初期化・操作エラー。
pub const EXIT_DB: i32 = sai_link_engine::common::EXIT_DB;

// ============================================================================
// CLI 引数定義
// ============================================================================

/// ルート CLI 構造体。
#[derive(Parser)]
#[command(
    name = "sai-link",
    version = env!("CARGO_PKG_VERSION"),
    about = "Sai-Engine: ブロック定義ファイルに基づく AI コーディングエージェント実行エンジン",
    long_about = None,
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// トップレベルのサブコマンド。
#[derive(Subcommand)]
pub enum Cmd {
    /// サブタスク操作のサブコマンド群
    Task {
        #[command(subcommand)]
        cmd: TaskCmd,
    },
    /// sai デーモンの操作
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCmd,
    },
    /// 実行 usage（provider の実際の token / cost）の管理（L1）
    Usage {
        #[command(subcommand)]
        cmd: UsageCmd,
    },
    /// プロジェクトの初期化（開始時に 1 度のみ使用）
    ///
    /// `PROJECTROOT/.sai/` と必要なフォルダ・`settings.json` を作成し、
    /// `sai-control registry add` でプロジェクトを登録する。
    Init {
        /// プロジェクト名（省略時はカレントディレクトリ名）
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// 配った block・template をこの版の中身に置き換える
        /// （変わる物は .bak に退避。block の model・provider・thinking は残す）
        #[arg(long)]
        update: bool,
    },
    /// カード（実行予約）の参照・作成・実行（新しい木）
    Card {
        #[command(subcommand)]
        cmd: CardCmd,
    },
    /// 実行（run）の参照・操作（新しい木）
    Run {
        #[command(subcommand)]
        cmd: RunCmd,
    },
    /// 文書（waggle トークン）の読み書き（新しい木）
    Doc {
        #[command(subcommand)]
        cmd: DocCmd,
    },
    /// 定義ファイルの参照・編集（新しい木）
    Def {
        #[command(subcommand)]
        cmd: DefCmd,
    },
    /// プロジェクトの集計（新しい木）
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    /// セッション履歴の検索（新しい木）
    Session {
        #[command(subcommand)]
        cmd: SessionCmd,
    },
    /// メイン画面（Workflow の実行ブロック線図）の DAG を JSON で返す
    ///
    /// この命令は常に JSON。`--json` を付けても同じ。
    Workflow {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// CLI の命令一覧（Web の「コマンド」画面が読む）
    ///
    /// `--json` で「schema の形」の JSON を出す。
    Schema {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// waggle トークン操作（内部用）
    #[command(hide = true)]
    Waggle {
        #[command(subcommand)]
        cmd: WaggleCmd,
    },
    /// cc の UserPromptSubmit hook（内部用）。
    ///
    /// 標準入力の JSON の `prompt` が `/sai-finalize` なら、LLM に送らずに完了処理を行い、
    /// 送信を止める JSON を出す。それ以外は何も出さない（終了コードは常に 0）。
    #[command(hide = true)]
    CcPromptHook,
    /// cc の状態フック（内部用）。標準入力は読み捨て、tmux 経路の pane の状態を `working` / `idle` にする。
    ///
    /// 何があっても終了コードは 0・標準出力は空（cc を止めない）。
    #[command(hide = true)]
    CcStateHook {
        /// `working` または `idle`
        state: String,
    },
    /// cc の PermissionRequest フック（内部用）。承認は常に allow を出す（AskUserQuestion は何も出さない）。
    #[command(hide = true)]
    CcPermissionHook,
    /// cc の statusLine（内部用）。費用を `cc-cost` へ書き、画面下に 1 行出す。
    #[command(hide = true)]
    CcStatusline,
}

/// サブタスク操作のサブコマンド（新しい木）。
///
/// 旧 `task list --parent-run-id` / `task show` / `task capture` / `task poll` は
/// なくした。旧 `task manage` は隠した別名として残す。
#[derive(Subcommand)]
pub enum TaskCmd {
    /// サブタスクをバッチ作成する（標準入力に JSON）
    ///
    /// 標準入力は子の配列 `[…]`。`{"tasks":[…]}` も受ける。
    Create {
        /// 修正ラウンド番号（1 始まり）
        #[arg(long)]
        round: Option<i64>,
        /// 置き換え元バッチ ID
        #[arg(long)]
        supersedes_batch_id: Option<String>,
        #[arg(long)]
        run_id: Option<i64>,
        /// JSON で出力する（この命令は常に JSON。付いても同じ）
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 承認済みタスクを実行キューへ送る
    Execute {
        #[arg(long)]
        batch: Option<String>,
        #[arg(long, value_name = "N")]
        task: Option<i64>,
        #[arg(long)]
        run_id: Option<i64>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 子タスクを複数まとめて待つ（1〜50 件）
    Wait {
        /// 待つ子タスクの ID（1 件以上）
        #[arg(value_name = "タスクID", required = true)]
        task_ids: Vec<i64>,
        /// 最大待機時間（ミリ秒）。0 で待たない
        #[arg(long)]
        timeout_ms: Option<u64>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// サブタスク一覧を表示する
    List {
        #[arg(long)]
        status: Option<String>,
        /// ソート順: id | recent | oldest
        #[arg(long)]
        sort: Option<String>,
        #[arg(long)]
        run_id: Option<i64>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// サブタスクの詳細を表示する
    Show {
        #[arg(value_name = "タスクID")]
        task_id: i64,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// タスクの pane 出力をキャプチャする
    Capture {
        #[arg(value_name = "タスクID")]
        task_id: i64,
        /// 読み取る行数
        #[arg(long)]
        lines: Option<u32>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 追加指示を送る
    Send {
        #[arg(value_name = "タスクID")]
        task_id: i64,
        #[arg(long, value_name = "文")]
        message: String,
        #[arg(long)]
        run_id: Option<i64>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 作業中の子タスクを止める（esc）
    Interrupt {
        #[arg(value_name = "タスクID")]
        task_id: i64,
        #[arg(long)]
        run_id: Option<i64>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 子タスクを消す
    Delete {
        #[arg(value_name = "タスクID")]
        task_id: i64,
        #[arg(long)]
        run_id: Option<i64>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 子タスクの完了を確定する
    Finalize {
        #[arg(value_name = "タスクID")]
        task_id: i64,
        #[arg(long)]
        run_id: Option<i64>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 失敗・削除した子タスクを再実行する
    Requeue {
        #[arg(value_name = "タスクID")]
        task_id: i64,
        #[arg(long)]
        run_id: Option<i64>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 完了報告を保存する（本文は標準入力）
    Report {
        #[arg(long)]
        run_id: Option<i64>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 親へ質問を通知する
    Question {
        /// 質問の ID（Web の質問一覧の ID）
        #[arg(long, value_name = "ID")]
        qid: String,
        /// 質問の見出し（1 行にする）
        #[arg(long, value_name = "文")]
        text: String,
        #[arg(long)]
        run_id: Option<i64>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

}

/// カード（実行予約）のサブコマンド（新しい木）。
#[derive(Subcommand)]
pub enum CardCmd {
    /// ウィザードの選択肢（runner / block / provider / model / thinking）
    Options {
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 最近の文書（running / review / done / Workflow の md 成果物）を新しい順に返す
    Docs {
        /// 返す件数の上限
        #[arg(long, default_value_t = 40)]
        limit: usize,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// カードを作成する（標準入力に JSON）
    Create {
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// todo のカードに実行予定時刻を書く（`--at` 省略で予定を消す）
    Schedule {
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
        /// 実行予定時刻（`YYYY-MM-DD HH:MM`）
        #[arg(long)]
        at: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// todo のカードを、ジョブが終わったら実行するようにする（`--job` 省略で解除）
    Wait {
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
        /// 待つジョブの ID
        #[arg(long)]
        job: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// todo の未実行カードを実行する（非対話）
    Run {
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 利用者の札（runner: user）を始める（running にする）
    Start {
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 利用者の札（runner: user）を review へ移す
    Finish {
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

/// 実行（run）のサブコマンド（新しい木）。
#[derive(Subcommand)]
pub enum RunCmd {
    /// 実行履歴を一覧表示する（`--json` で sai-sci の形）
    List {
        /// 表示件数（既定 20。0 で全部）
        #[arg(long, default_value = "20")]
        limit: i64,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 特定の実行の詳細を表示する（文字）
    Show {
        #[arg(value_name = "実行ID")]
        run_id: i64,
    },

    /// 左サイドバーと同形のツリーを JSON で返す
    Tree {
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// run の log を JSON で返す
    Log {
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// run の stats を JSON で返す
    Stats {
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// run の history を JSON で返す
    History {
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// task / message / report / system の md 本文を返す（`--write` は標準入力に本文）
    Md {
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
        /// md の種別（task / message / report / system）
        #[arg(long, value_parser = ["task", "message", "report", "system"])]
        kind: String,
        /// 標準入力の中身で保存する
        #[arg(long, default_value_t = false)]
        write: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// run の完了を要求する（run_id 省略時は自分の run）
    Finalize {
        #[arg(value_name = "RUN_ID")]
        run_id: Option<i64>,
        /// 報告の本文（要点）
        #[arg(long, value_name = "文")]
        report: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 実行を次の状態へ進める（review → done → Workflow）
    Advance {
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 実行中の run に続きの文を送る（チャットの続き）
    ///
    /// `--text` を省くと標準入力の全文を送る。
    Send {
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
        /// 送る文（省略時は標準入力の全文）
        #[arg(long, value_name = "文")]
        text: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 実行中の run を中断する（人間の esc と同じ）
    Interrupt {
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 実行を消す（DB の行と run フォルダ・子の実行も一緒に）
    Rm {
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
        /// 確認をスキップする（無ければ終了コード 2）
        #[arg(long, default_value_t = false)]
        yes: bool,
    },

    /// 現在のプロジェクトの実行を sai-phone 向け JSON に書き出す（文字）
    Export {
        /// 1 件だけ書き出す（省略時は全件）
        #[arg(long, value_name = "RUN_ID")]
        run: Option<i64>,
        /// 書かずに JSON を標準出力へ
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
}

/// 文書（waggle トークン）のサブコマンド（新しい木）。
#[derive(Subcommand)]
pub enum DocCmd {
    /// 成果物を run フォルダへ保存する（本文は標準入力）
    Write {
        /// 保存ファイル名（run フォルダ直下）
        #[arg(long, value_name = "ファイル名")]
        filename: String,
        /// 成果物の説明（waggle トークンのラベル）
        #[arg(long)]
        description: Option<String>,
        /// 実行 ID（省略時は SAI_PARENT_RUN_ID）
        #[arg(long)]
        run_id: Option<i64>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// waggle 文書の本文を読む
    Read {
        #[arg(value_name = "トークン")]
        token: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// waggle 文書の一覧（`--all` は実行中も含める）
    List {
        /// 実行 ID（省略時は全体）
        #[arg(long, value_name = "N")]
        run: Option<i64>,
        /// 実行中（running / review）の文書も含める
        #[arg(long, default_value_t = false)]
        all: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 文書（task.md / message.md / waggle の md 成果物）の本文を検索する
    Search {
        /// 検索語（空白区切りで AND）
        #[arg(long)]
        word: String,
        /// 対象の種類（カンマ区切り。task / message / waggle）
        #[arg(long, default_value = "task,message,waggle")]
        category: String,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        block: Option<String>,
        #[arg(long)]
        from_id: Option<i64>,
        #[arg(long)]
        to_id: Option<i64>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

/// 定義ファイル（block / template / settings）のサブコマンド（新しい木）。
#[derive(Subcommand)]
pub enum DefCmd {
    /// 定義ファイルの一覧（`--parent` で呼べる子の一覧）
    List {
        /// 親ブロック名（subblocks 制限の確認元）
        #[arg(long, value_name = "ブロック名")]
        parent: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 定義ファイルの中身（`--kind block` で `--scope` 省略時は解決済みの定義）
    Show {
        #[arg(long, value_parser = ["block", "template", "settings"])]
        kind: String,
        /// 置き場（project / global。省略時は解決済みの block 定義）
        #[arg(long, value_parser = ["project", "global"])]
        scope: Option<String>,
        #[arg(long, value_name = "名前")]
        name: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 定義ファイルを標準入力の内容で保存する
    Write {
        #[arg(long, value_parser = ["block", "template", "settings"])]
        kind: String,
        #[arg(long, value_parser = ["project", "global"])]
        scope: String,
        #[arg(long, value_name = "名前")]
        name: String,
        /// 新規作成する（既に同名があるなら終了コード 4）
        #[arg(long, default_value_t = false)]
        create: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 定義ファイルを削除する
    Delete {
        #[arg(long, value_parser = ["block", "template", "settings"])]
        kind: String,
        #[arg(long, value_parser = ["project", "global"])]
        scope: String,
        #[arg(long, value_name = "名前")]
        name: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 実行時の pi 起動コマンドをプレビュー表示する（実行はしない・文字）
    Preview {
        /// ブロック定義ファイル（.md）のパス
        #[arg(value_name = "定義ファイル")]
        definition_file: PathBuf,
        /// ブロックへのユーザー入力（テキスト）
        #[arg(value_name = "ユーザー入力")]
        user_input: Option<String>,
        #[arg(long, default_value = "right", value_parser = ["right", "down"])]
        direction: String,
        #[arg(long)]
        timeout: Option<u64>,
        #[arg(long, value_name = "N")]
        round: Option<i64>,
        #[arg(long = "global-round", value_name = "N")]
        global_round: Option<i64>,
        #[arg(long, value_name = "TAG")]
        tag: Option<String>,
        #[arg(long, value_name = "BUDGET", value_parser = ["none", "low", "medium", "high", "xhigh", "max"])]
        think: Option<String>,
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,
        #[arg(long = "block-timeout", value_name = "SECS")]
        block_timeout: Option<u32>,
    },
}

/// プロジェクトの集計のサブコマンド（新しい木）。
#[derive(Subcommand)]
pub enum ProjectCmd {
    /// プロジェクト一覧に出す最小集計を JSON で返す
    Summary {
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// 期間ダッシュボードを JSON で返す
    Dashboard {
        /// 集計期間
        #[arg(long, value_parser = ["week", "month", "all"])]
        period: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

/// セッション履歴検索のサブコマンド（新しい木）。
#[derive(Subcommand)]
pub enum SessionCmd {
    /// セッション履歴を検索する
    Search {
        /// Round（グローバルラウンド）N または N-M
        #[arg(short = 'R', long = "global-round", value_name = "N|N-M")]
        global_round: Option<String>,
        /// round（実行ラウンド）N または N-M
        #[arg(short = 'r', long, value_name = "N|N-M")]
        round: Option<String>,
        /// ブロック ID 範囲
        #[arg(long, value_name = "N|N-M")]
        id: Option<String>,
        /// ブロック名の部分一致（大小無視）
        #[arg(long, value_name = "NAME")]
        block: Option<String>,
        /// 実行日 YYYY/MM/DD または YYYY/MM/DD-YYYY/MM/DD
        #[arg(long, value_name = "D|D-D")]
        date: Option<String>,
        /// ステータス絞り込み（カンマ区切り）
        #[arg(long, value_name = "S[,S...]")]
        status: Option<String>,
        /// 種別（カンマ区切り）
        #[arg(long, value_name = "T[,T...]")]
        r#type: Option<String>,
        /// 検索語（カンマ区切り・AND・大小無視）
        #[arg(short = 'w', long, value_name = "W[,W...]")]
        word: Option<String>,
        /// プロジェクト名（結果の出自表示）
        #[arg(long, value_name = "NAME")]
        project: Option<String>,
        /// JSON で出力する
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

/// 実行 usage 管理（L1）のサブコマンド。
#[derive(Subcommand)]
pub enum UsageCmd {
    /// 既存実行分の jsonl から usage を再計算して `execution_usage` へ保存する（冪等）
    Backfill {
        /// 保存せず集計結果のみ表示する
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
}

/// sai デーモン操作のサブコマンド。
#[derive(Subcommand)]
pub enum DaemonCmd {
    /// デーモンをバックグラウンドで起動する（プロジェクトルートで実行すること）
    Start,

    /// デーモンを停止する（実行中の作業がある場合は制止する・P0-3）
    Stop {
        /// 実行中の作業があっても強制停止する（停止時の確定処理が走る）
        #[arg(long, default_value_t = false)]
        force: bool,
    },

    /// デーモンの状態を表示する
    Status,

    /// ブロック定義キャッシュを再読込する
    Reload,

    /// 取り残し（幽霊）の確認・確定を行う（デーモン停止中専用・P2-2）
    Recover {
        /// 幽霊を `failed` + 理由付きで確定する（省略時は一覧表示のみ）
        #[arg(long, default_value_t = false)]
        apply: bool,
    },

    /// 【内部用】デーモン本体のポーリングループ（start がバックグラウンド起動する）
    #[command(hide = true, name = "_run")]
    Run,
}

/// waggle トークン操作のサブコマンド（内部用）。
#[derive(Subcommand)]
pub enum WaggleCmd {
    /// waggle トークンを解決し、ファイル内容を出力する
    Resolve {
        /// waggle トークン文字列
        #[arg(value_name = "TOKEN")]
        token: String,
    },
}

/// sai-phone への書き出しサブコマンド（実装計画 01 §2.4 / 12 §3.2）。
#[derive(Subcommand)]
pub enum PhoneCmd {
    /// 現在のプロジェクトの実行を sai-phone 向け JSON に書き出す
    Export {
        /// 1 件だけ書き出す（省略時は全件）
        #[arg(long, value_name = "RUN_ID")]
        run: Option<i64>,

        /// 書かずに JSON を標準出力へ（1 件指定時はオブジェクト、
        /// 全件時は配列。各要素が 00 §3.2 の形）
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },

    /// 左サイドバーと同形のツリーを JSON で返す（実装計画 12 §3.2）
    Tree,

    /// run の中身（log / stats / history）を JSON で返す（実装計画 12 §3.2）
    Run {
        /// 対象の実行 ID
        #[arg(value_name = "RUN_ID")]
        run_id: i64,

        /// 表示種別
        #[arg(long, value_parser = ["log", "stats", "history"])]
        view: String,
    },

    /// task / review / message の md 本文を JSON で返す
    /// （無ければ終了コード 3・stderr「まだありません」・実装計画 12 §3.2）
    Md {
        /// 対象の実行 ID
        #[arg(value_name = "RUN_ID")]
        run_id: i64,

        /// md の種別（task / message / report / system）
        #[arg(long, value_parser = ["task", "message", "report", "system"])]
        kind: String,

        /// 標準入力の中身で保存する（task は未実行の todo カードだけ。
        /// message と report は保存できない）。成功すると保存後の本文を同じ形で返す
        #[arg(long)]
        write: bool,
    },

    /// run の成果物 md トークン一覧（`--token` でその本文）を JSON で返す
    Waggle {
        /// 対象の実行 ID
        #[arg(value_name = "RUN_ID")]
        run_id: i64,

        /// 本文を返すトークン（省略時は一覧）
        #[arg(long, value_name = "TOKEN")]
        token: Option<String>,
    },

    /// カード（task.md）作成ウィザード用の JSON（実装計画 12 §3.2）
    Card {
        #[command(subcommand)]
        cmd: PhoneCardCmd,
    },

    /// 実行中のメインブロックを完了させる（sai-phone の ■ 終了用）
    ///
    /// 成功は stdout に JSON（`run_id` / `message`）・終了コード 0、
    /// 実行が無い場合は終了コード 3、実行中以外・子ブロック・常駐停止は終了コード 1。
    Finalize {
        /// 対象の実行 ID
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
    },

    /// 実行を次の状態へ進める（review → done → Workflow・sai-phone の「→ done」「→ Workflow」用）
    ///
    /// 成功は stdout に JSON（`run_id` / `from` / `to`）・終了コード 0、
    /// 実行が無い場合は終了コード 3、review / done 以外は終了コード 1。
    Advance {
        /// 対象の実行 ID
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
    },

    /// 実行を消す（DB の行と run フォルダ・子の実行も一緒に。`block rm --yes` と同じ関数）
    ///
    /// 実行中（running / pending）は消さない（終了コード 1）。実行が無い場合は終了コード 3。
    Rm {
        /// 対象の実行 ID
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
    },

    /// プロジェクト一覧に出す最小集計を JSON で返す（実行中の件数・最終実行日時）
    ///
    /// 成功は stdout に `{"running": N, "last_executed_at": "…"|null}`・終了コード 0。
    /// DB が開けない場合は終了コード 4。
    Summary,

    /// 期間ダッシュボードを JSON で返す（sai-phone の Dashboard 用）
    ///
    /// 出力の形は `sai-control dashboard <name> --period <p> --json` と同じ
    /// （name / period / since / until / daemon_alive / summary / daily /
    ///  by_model / by_block / recent）。
    Dashboard {
        /// 集計期間
        #[arg(long, value_parser = ["week", "month", "all"])]
        period: String,
    },

    /// 文書（task.md / message.md / waggle の md 成果物）の本文を検索して JSON で返す
    ///
    /// 検索語は空白区切りの AND（大文字小文字は区別しない）。新しい run から順に見て、
    /// `--limit` 件で打ち切る（`truncated`）。message.md は run フォルダにあるものだけを見る
    /// （開いたときに作り直す文書なので、まだ作られていない run は当たらない）。
    /// 結果は `{items: [{run_id, category, file, token?, block, date, snippet}], truncated}`。
    Search {
        /// 検索語（空白区切りで AND）
        #[arg(long)]
        word: String,

        /// 対象の種類（カンマ区切り。task / message / waggle）
        #[arg(long, default_value = "task,message,waggle")]
        category: String,

        /// この日以降の run（`YYYY-MM-DD`・日本時間の実行日）
        #[arg(long)]
        since: Option<String>,

        /// この日以前の run（`YYYY-MM-DD`）
        #[arg(long)]
        until: Option<String>,

        /// ブロック名（完全一致）
        #[arg(long)]
        block: Option<String>,

        /// run_id の下限
        #[arg(long)]
        from_id: Option<i64>,

        /// run_id の上限
        #[arg(long)]
        to_id: Option<i64>,

        /// 返す件数の上限
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },

    /// メイン画面（Workflow の実行ブロック線図）の DAG を JSON で返す
    ///
    /// - `nodes`: TUI が描くのと同じ座標（文字マス単位・`x` `y` `w` `h`）で返す
    /// - `edges`: 接続線（`kind` = phase_chain / delegation / task_output /
    ///   waggle_token / git_commit / other・`side` = 左右辺接地か）
    /// - `separators`: round の区切り線
    /// - `lanes`: Kanban の 4 レーン（todo / running / review / done）の札
    ///
    /// 座標の縮め方（横 1 マス = 何 px）は sai-phone 側の取り決めであり、
    /// sai-link は文字マスのまま返す。DB が開けない場合は終了コード 4。
    Workflow,

    /// 定義ファイル（block / template / settings）の一覧・読み書き・削除
    ///
    /// 対象はプロジェクト（`<project_root>/.sai/`）とグローバル（`~/.sai/`）の
    /// 2 か所。保存前に sai-link が検査し、壊れた定義は保存しない（終了コード 2）。
    /// 検査でエラーにならない警告（frontmatter の id とファイル名の不一致など）は
    /// そのまま保存し、`warnings` で返す。
    Def {
        #[command(subcommand)]
        cmd: PhoneDefCmd,
    },

    /// 利用者の札（`runner: user`）を始める・終える（sai-phone の READ_CARD 用・U17）
    ///
    /// 成功は stdout に JSON（`run_id` / `status`）・終了コード 0。
    /// 常駐（デーモン）は要らない。
    User {
        #[command(subcommand)]
        cmd: PhoneUserCmd,
    },
}

/// `sai-link phone user` のサブコマンド。
#[derive(Subcommand)]
pub enum PhoneUserCmd {
    /// todo の `runner: user` のカードを running にする
    ///
    /// 成功は `{"run_id": N, "status": "running"}`・終了コード 0。
    /// カードが無ければ終了コード 3、`runner: user` で無ければ終了コード 1。
    Start {
        /// 対象の予約 run ID
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
    },

    /// 実行中の `runner: user` の実行を review へ移す
    ///
    /// 成功は `{"run_id": N, "status": "review"}`・終了コード 0。
    /// 実行が無ければ終了コード 3、実行中の利用者の札でなければ終了コード 1。
    Finish {
        /// 対象の実行 ID
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
    },
}

/// `sai-link phone card` のサブコマンド。
#[derive(Subcommand)]
pub enum PhoneCardCmd {
    /// ウィザードの選択肢（done / runner / block / provider / model / thinking）
    Options,

    /// 指定ブロックの入力欄（入力は task_text に統一したため常に空。ブロックの存在確認に使う）
    ///
    /// 文書の候補は `phone card docs` を使う。
    Inputs {
        /// ブロック ID
        #[arg(long, value_name = "BLOCK_ID")]
        block: String,
    },

    /// 最近の文書（running / review / done / Workflow の実行の md 成果物）を新しい順に返す
    ///
    /// task_text に `waggle:<tok>:path:<path>` を貼るための候補（`ref` がその行）。
    Docs {
        /// 返す件数の上限
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },

    /// カードを作成する（標準入力に JSON・作成後は todo に配置）
    Create,

    /// todo のカードに実行予定時刻を書く（常駐がその時刻に実行する）
    ///
    /// `--at` は `YYYY-MM-DD HH:MM`（Mac の時刻）。省略すると予定を消す。
    Schedule {
        /// 対象の予約 run ID
        #[arg(value_name = "RUN_ID")]
        run_id: i64,

        /// 実行予定時刻（`YYYY-MM-DD HH:MM`）
        #[arg(long)]
        at: Option<String>,
    },

    /// todo のカードを、ジョブが終わったら実行するようにする
    ///
    /// `--job` を省くと解除する。常駐が約 60 秒ごとにジョブの状態を見回る。
    Wait {
        /// 対象の予約 run ID
        #[arg(value_name = "RUN_ID")]
        run_id: i64,

        /// 待つジョブの ID
        #[arg(long)]
        job: Option<String>,
    },

    /// todo の未実行カード（`outputs/todo/<run_id>/task.md`）を実行する（非対話）
    ///
    /// 成功は stdout に JSON（`run_id` / `message`）・終了コード 0、
    /// 対象無しは終了コード 3、常駐停止・実行失敗は終了コード 1。
    Run {
        /// 対象の予約 run ID
        #[arg(value_name = "RUN_ID")]
        run_id: i64,
    },
}

/// `sai-link phone def` のサブコマンド。
///
/// 定義ファイル（`.sai/blocks/*.md`・`.sai/template/*.md`・`.sai/settings.json`）の
/// 閲覧・編集用。Web（sai-phone）から definitions を編集するために使う。
///
/// 保存（`write`）は必ず検査を通してから行う。検査に失敗したら**ファイルを
/// 書き換えず**に終了コード 2 で終わる（常駐が壊れた定義を読むのを防ぐ）。
#[derive(Subcommand)]
pub enum PhoneDefCmd {
    /// 定義ファイルの一覧を JSON で返す
    ///
    /// 並びは kind（block → template → settings）→ scope（project → global）→
    /// name の昇順。`shadowed` はグローバルがプロジェクトで上書きされるか。
    List,

    /// 定義ファイルの中身を JSON で返す
    ///
    /// 存在しないファイルは終了コード 3。settings は無ければ `exists: false`・
    /// `text: "{}\n"`（終了コード 0）で返す。
    Show {
        /// 定義の種別
        #[arg(long, value_parser = ["block", "template", "settings"])]
        kind: String,

        /// 置き場（project = プロジェクト内 / global = `~/.sai/`）
        #[arg(long, value_parser = ["project", "global"])]
        scope: String,

        /// 定義の名前（拡張子 `.md` は付けない。settings は必ず `settings`）
        #[arg(long, value_name = "名前")]
        name: String,
    },

    /// 定義ファイルを標準入力の内容で保存する
    ///
    /// 保存前に検査し、壊れていれば書き換えず終了コード 2。
    /// 既に同じ名前があるときは `--create` を付けないと終了コード 1。
    Write {
        /// 定義の種別
        #[arg(long, value_parser = ["block", "template", "settings"])]
        kind: String,

        /// 置き場（project = プロジェクト内 / global = `~/.sai/`）
        #[arg(long, value_parser = ["project", "global"])]
        scope: String,

        /// 定義の名前（拡張子 `.md` は付けない。settings は必ず `settings`）
        #[arg(long, value_name = "名前")]
        name: String,

        /// 新規作成する（既に同名があるなら終了コード 1）
        #[arg(long, default_value_t = false)]
        create: bool,
    },

    /// 定義ファイルを削除する
    ///
    /// 対象が無い場合は終了コード 3。settings は消せない（終了コード 1）。
    /// `used_by`（この定義を使っている block の id）は削除前に数えて返す。
    /// 確認は取らない（呼び出す側（Web）が `used_by` を見て判断する）。
    Delete {
        /// 定義の種別（settings は削除できない）
        #[arg(long, value_parser = ["block", "template", "settings"])]
        kind: String,

        /// 置き場（project = プロジェクト内 / global = `~/.sai/`）
        #[arg(long, value_parser = ["project", "global"])]
        scope: String,

        /// 定義の名前（拡張子 `.md` は付けない。settings は必ず `settings`）
        #[arg(long, value_name = "名前")]
        name: String,
    },
}

// ============================================================================
// ブロック実行ハンドラ（後方互換・kanban 統一後は未使用のため削除済み）
// ============================================================================
// `handle_block_run` / `handle_block_run_non_interactive` は削除した。
// 定義ファイル直指定の旧経路は `main.rs` で kanban 誘導メッセージ付きで終了する。
// 非対話実行は `kanban_run::kanban_block_run` に一本化されている。

/// 実行時の pi 起動コマンドをプレビュー表示する（実行はしない）。
///
/// 実行時設定（オーバーライド）が反映された pi 起動コマンドを表示します（D19・§7.1.2）。
/// ユーザーが作成したドキュメントを user-block ブロックの実行として記録する（ユーザーブロック）。
///
/// 処理:
/// 1. 入力ファイルを waggle トークン化（Blob 保存込み）
/// 2. block_metadata（user-block）を取得/作成
/// 3. execution_history に新規レコード（completed）を作成（入力/出力トークン・round/tag 付き）
/// 4. 入力トークンがあれば、その出力元実行 → user-block 実行の waggle_token 接続（線図の入力辺）
///
/// 同一内容（既存の user-block 出力と同じ SHA-256）の場合は、その旨を出力し、
/// トークン化・実行記録を行わない。
pub fn handle_user_block_non_interactive(
    input_tokens: &[String],
    input_file: Option<&PathBuf>,
    description: Option<&str>,
    round: Option<i64>,
    global_round: Option<i64>,
    tag: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    // input-file は must（user-block の出力 = トークン化対象）
    let input_file = input_file.ok_or_else(|| {
        "user-block には --input-file (-f) でトークン化するファイルを指定してください".to_string()
    })?;
    handle_block_user_block(
        input_tokens,
        input_file,
        description,
        round,
        global_round,
        tag,
    )
}

/// 同一内容（既存の user-block 出力と同じ SHA-256）の場合は、その旨を出力し、
/// トークン化・実行記録を行わない。
pub fn handle_block_user_block(
    input_tokens: &[String],
    input_file: &Path,
    description: Option<&str>,
    round: Option<i64>,
    global_round: Option<i64>,
    tag: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    // user-block ブロック名（block_metadata / トークン化ラベルに使用）
    const BLOCK_NAME: &str = "user-block";
    if !input_file.exists() {
        return Err(format!("入力ファイルが存在しません: {}", input_file.display()).into());
    }

    // DB と waggle インテグレーターを開く
    let store = Arc::new(open_db()?);
    let db_path = sai_link_engine::project::sai_path(".sai/sai-engine/sai-engine.db");
    let integrator = sai_link_engine::waggle_integration::WaggleIntegrator::open(&db_path)
        .map_err(|e| format!("waggle DB 初期化失敗: {}", e))?;

    // ── 1. 同一判定: 既存 user-block 出力の内容（SHA-256）と比較 ──
    // トークンは非決定性（毎回ランダム）のため、トークン一致ではなく内容で判定する。
    if let Ok(block) = store.get_block_by_name(BLOCK_NAME) {
        if let Some(block_id) = block.id {
            let executions = store.list_executions_by_block(block_id).unwrap_or_default();
            let new_sha = sha256_hex(&std::fs::read(input_file)?);
            let mut same = false;
            'exec: for exec in &executions {
                if exec.status != "completed" {
                    continue;
                }
                let tokens: Vec<String> = exec
                    .output_waggle_tokens
                    .as_deref()
                    .and_then(|j| serde_json::from_str(j).ok())
                    .unwrap_or_default();
                for token in &tokens {
                    if let Ok(content) = integrator.resolve_content(token) {
                        if sha256_hex(content.as_bytes()) == new_sha {
                            same = true;
                            break 'exec;
                        }
                    }
                }
            }
            if same {
                println!(
                    "⚠ 同一の内容の出力が既に存在します: {}",
                    input_file.display()
                );
                println!("  sha256: {}", new_sha);
                println!("  waggle トークンを作成せず、実行も記録しませんでした。");
                return Ok(());
            }
        }
    }

    // 説明が未指定ならファイル名から生成
    let description = description.map(String::from).unwrap_or_else(|| {
        input_file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "user-block".to_string())
    });
    let description = description.as_str();

    // ── 2. 入力ファイルをトークン化 ──
    let tokenized = integrator
        .tokenize_file(input_file, BLOCK_NAME, Some(description))
        .map_err(|e| format!("waggle トークン化に失敗 ({}): {}", input_file.display(), e))?;
    let output_tokens = vec![tokenized.token.as_str().to_string()];
    println!(
        "  tokenized: {} -> {}",
        input_file.display(),
        tokenized.token.as_str()
    );

    // ── 3. block_metadata（user-block）を取得/作成 ──
    let block_id = match store.get_block_by_name(BLOCK_NAME) {
        Ok(b) => b.id.ok_or("block_metadata id がありません")?,
        Err(_) => {
            let meta = sai_link_engine::db::models::BlockMetadata::new(
                BLOCK_NAME.to_string(),
                "ユーザーが作成したドキュメントを waggle トークン化するユーザーブロック"
                    .to_string(),
                "inherit".to_string(),
                None,
                "off".to_string(),
                "false".to_string(),
                "false".to_string(),
                None,
                300,
                "none".to_string(),
                true,
                format!("({})", BLOCK_NAME),
                None,
            );
            store.insert_block(&meta)?
        }
    };

    // ── 4. 実行レコードを作成（completed・入出力トークン・設定）──
    // user_inputs_json に入力ファイルパスと説明（右SB の [Output] 表示用）を保存する。
    let user_inputs_map = serde_json::json!({
        "file_path": input_file.to_string_lossy().to_string(),
        "description": description,
    });
    let exec_id = store.create_execution(block_id, Some(&user_inputs_map.to_string()))?;
    store.update_execution_status(exec_id, "completed")?;
    store.update_execution_output_type(exec_id, "document")?;
    if !input_tokens.is_empty() {
        store.update_execution_input_tokens(exec_id, input_tokens)?;
    }
    store.update_execution_waggle_tokens(exec_id, &output_tokens)?;
    let tag_ref = tag.as_deref();
    store.update_execution_overrides(exec_id, tag_ref, round, global_round, None, None, None)?;

    // ── 5. 入力辺の接続（線図: 入力トークンの出力元実行 → user-block 実行）──
    // 入力トークンが指定された場合のみ入力辺が存在する。
    sai_link_engine::orchestrator::insert_block_connections(
        &store,
        exec_id,
        block_id,
        input_tokens,
    );

    println!(
        "✅ user-block 実行を記録しました: execution_id={}, block_id={}",
        exec_id, block_id
    );
    println!("   入力トークン: {:?}", input_tokens);
    println!("   出力トークン: {:?}", output_tokens);
    Ok(())
}

/// 指定されたバイト列の SHA-256 ハッシュ（hex）を計算する。
fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

pub fn handle_block_preview(
    definition_file: &Path,
    user_input: Option<&str>,
    direction: &str,
    timeout_ms: Option<u64>,
    round: Option<i64>,
    global_round: Option<i64>,
    tag: Option<String>,
    think: Option<String>,
    model: Option<String>,
    block_timeout: Option<u32>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !definition_file.exists() {
        eprintln!(
            "エラー: ファイル '{}' が見つかりません",
            definition_file.display()
        );
        std::process::exit(EXIT_NOT_FOUND);
    }

    // ブロック定義を読む直前に準備コマンドを走らせる（作り直したテンプレートを
    // そのまま差し込むため）。失敗しても前回のファイルのままプレビューを続ける。
    if let sai_link_engine::pre_run::PreRunOutcome::Failed(msg) =
        sai_link_engine::pre_run::run_if_configured()
    {
        eprintln!(
            "[sai-engine] 警告: 実行直前の準備コマンドに失敗しました（前回のファイルのまま続けます）: {msg}"
        );
    }

    let parsed = parse_block_file(definition_file)
        .map_err(|e| format!("ブロック定義のパースに失敗: {}", e))?;

    // ユーザー入力（`task_text` のみ）
    let mut user_inputs_map: HashMap<String, String> = HashMap::new();
    if let Some(input) = user_input {
        user_inputs_map.insert("task_text".into(), input.to_string());
    }

    // 実行時設定の解決（§6）
    let overrides = ExecutionOverrides {
        model,
        provider: None,
        thinking: think,
        timeout: block_timeout.or_else(|| timeout_ms.map(|ms| (ms / 1000) as u32)),
        round,
        global_round,
        tag,
        direction: Some(parse_direction(direction)),
        worktree: None,
        cwd: None,
        keep_pane: false,
        runner: sai_link_engine::runner::RunnerKind::Pi,
    };
    let resolved = orchestrator::resolve_execution_settings(&parsed.config, &overrides);

    // pi 起動コマンドを構築（プレビュー用。run_id なし → 環境変数プレフィックスなし）
    let pipeline_config = PipelineConfig {
        block_config: resolved.config,
        block_body: String::new(),
        user_inputs: user_inputs_map.clone(),
        cwd: Some(std::env::current_dir()?),
        direction: resolved.direction,
        timeout_ms: resolved.timeout_ms,
        keep_pane: false,
        run_id: None,
        tab_id: None,       // 単体実行: タブ未使用（U2）
        base_pane_id: None, // U2・R4 案X: 分割基準ペインなし
        pane_id: None,      // U2・R4 案X: 分割済みペインなし
        is_subtask: false,
        target_node: None,
        runner: resolved.runner,
    };
    let prepared = orchestrator::prepare_pipeline(&pipeline_config)?;

    // プレビュー用に作成したセッションディレクトリを削除
    sai_link_engine::session::cleanup_session_dir(&prepared.session_dir);

    println!("{}", prepared.pi_command);

    // 実際に送る 2 通（固定部分＝system prompt と最初の文）を見せる（U6・C10）。
    // 実行時は固定部分を run フォルダの `system-prompt.md` に書いて
    // `--append-system-prompt`（pi）/ `--append-system-prompt-file`（cc）で渡すため、
    // プレビューのコマンドにはそのフラグが付きません。
    // `context: true` のブロックは `run_id` が無いので、context の節の中身は
    // 「範囲を作れませんでした」の 1 行になる（実行時と同じ形を見せられる）。
    let preview_context = if parsed.config.context == Some(true) {
        Some(sai_link_engine::pre_run::context_text(
            None,
            parsed.config.block_id(),
        ))
    } else {
        None
    };
    let (system_prompt, first_message) = sai_link_engine::placeholder::split_fixed_and_variable(
        &parsed.body,
        &user_inputs_map,
        preview_context.as_deref(),
    );
    println!();
    println!("## system prompt（固定部分）");
    println!("{}", system_prompt);
    println!();
    println!("## 最初の文");
    println!("{}", first_message);
    Ok(())
}

// ============================================================================
// ブロック定義の参照（P2: block_discover / block_inspect 用）
//
// JSON 構造体・変換ロジックの実体は sai_link_engine::agent_api に移動した
// （plan-03。出力形式は 1 文字も変えていない）。
// ============================================================================

use sai_link_engine::agent_api::{block_def_to_json, collect_block_definitions, BlockDefListJson};

/// ブロック定義の一覧を表示する（block_discover 用）。
///
/// `--parent <親ブロック名>` を指定すると、親ブロック定義の `subblocks` 制限を
/// 考慮して呼び出し可能なブロックのみに絞り込む。
pub fn handle_block_def_list(
    parent: Option<&str>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut defs = collect_block_definitions();

    // 親ブロックの subblocks 制限を適用（親ブロックは ID で解決）
    if let Some(parent_name) = parent {
        let parent_path = sai_link_engine::orchestrator::resolve_block_path(parent_name);
        if parent_path.exists() {
            if let Ok(parsed) = parse_block_file(&parent_path) {
                match parsed.config.subblocks {
                    // subblocks が定義されていない → 呼び出し可能なサブブロックは「なし」
                    None => {
                        defs.clear();
                    }
                    Some(allowed) => {
                        defs.retain(|(_, c)| allowed.iter().any(|b| b == c.block_id()));
                    }
                }
            }
        }
    }

    if json {
        let entries: Vec<BlockDefListJson> = defs
            .iter()
            .map(|(_, c)| BlockDefListJson {
                id: c.block_id().to_string(),
                name: c.display_name().to_string(),
                description: c.description.clone(),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        if defs.is_empty() {
            println!("ブロック定義が見つかりません。");
            return Ok(());
        }
        println!("{:<20} {:<28} {}", "ID", "名前", "説明");
        println!("{:-<20}-+-{:-<28}-+--", "", "");
        for (_, c) in &defs {
            println!(
                "{:<20} {:<28} {}",
                c.block_id(),
                c.display_name(),
                c.description.clone().unwrap_or_default()
            );
        }
    }
    Ok(())
}

/// ブロック定義の詳細を表示する（block_inspect 用）。
///
/// description / inputs（型・required・options・source_blocks）/ output / subblocks を返す。
pub fn handle_block_def_show(name: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    // ID で解決（表示名が指定された場合は表示名マッチでフォールバック）
    let mut path = sai_link_engine::orchestrator::resolve_block_path(name);
    if !path.exists() {
        // 表示名（name）でのフォールバック検索
        let fallback = collect_block_definitions()
            .into_iter()
            .find(|(_, c)| c.display_name() == name)
            .map(|(p, _)| p);
        if let Some(p) = fallback {
            path = p;
        }
    }
    if !path.exists() {
        eprintln!(
            "エラー: ブロック定義が見つかりません: {} (ID または表示名で指定)",
            name
        );
        std::process::exit(EXIT_NOT_FOUND);
    }

    let parsed =
        parse_block_file(&path).map_err(|e| format!("ブロック定義のパースに失敗: {}", e))?;

    if json {
        let entry = block_def_to_json(&parsed.config);
        println!("{}", serde_json::to_string_pretty(&entry)?);
    } else {
        println!("ID: {}", parsed.config.block_id());
        println!("名前: {}", parsed.config.display_name());
        println!(
            "説明: {}",
            parsed.config.description.clone().unwrap_or_default()
        );
        if let Some(ref subblocks) = parsed.config.subblocks {
            println!("呼び出し可能サブブロック: {}", subblocks.join(", "));
        }
        if let Some(ref worktree) = parsed.config.worktree {
            println!("worktree: {}", worktree);
        }
        if let Some(ref templates) = parsed.config.templates {
            if !templates.is_empty() {
                println!("テンプレート: {}", templates.join(", "));
            }
        }
    }
    Ok(())
}

// ============================================================================
// 履歴一覧ハンドラ
// ============================================================================

/// 過去のブロック実行履歴を一覧表示する。
pub fn handle_block_list(limit: i64, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let store = open_db()?;

    // R303: 予約行（status='todo'）は既定で除外する（履歴・件数に含めない）。
    let records: Vec<_> = store
        .list_recent_executions(limit)
        .map_err(|e| format!("DB エラー (履歴取得): {}", e))?
        .into_iter()
        .filter(|e| e.status != "todo")
        .collect();

    if records.is_empty() {
        if !json {
            println!(
                "実行履歴はありません。'sai block run <定義ファイル>' でブロックを実行してください。"
            );
        } else {
            println!("[]");
        }
        return Ok(());
    }

    // P15: N+1 を解消（4.2.3）。ブロック一覧を 1 回の SELECT で取得し HashMap 化する。
    let block_map: std::collections::HashMap<i64, sai_link_engine::db::BlockMetadata> = store
        .list_blocks()
        .ok()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|b| b.id.map(|id| (id, b)))
        .collect();

    // 各実行レコードにブロック名を付与
    let display_records: Vec<DisplayRunRecord> = records
        .iter()
        .map(|exec| {
            let block_data = block_map.get(&exec.block_id);
            let block_name = block_data
                .map(|b| b.name.clone())
                .unwrap_or_else(|| format!("(block_id={})", exec.block_id));
            let tag = block_data.and_then(|b| b.tag.clone()).unwrap_or_default();

            DisplayRunRecord {
                id: exec.id.unwrap_or(0),
                block_name,
                tag,
                status: exec.status.clone(),
                started_at: exec.started_at.clone().unwrap_or_default(),
                elapsed_ms: exec.duration_ms,
            }
        })
        .collect();

    if json {
        // P1: pi-sai / デバッグ用の JSON 出力
        println!("{}", serde_json::to_string_pretty(&display_records)?);
    } else {
        print_list_table(&display_records);
    }
    Ok(())
}

/// 一覧表示用の簡易レコード構造体。
#[derive(serde::Serialize)]
struct DisplayRunRecord {
    id: i64,
    block_name: String,
    tag: String,
    status: String,
    started_at: String,
    elapsed_ms: Option<i64>,
}

/// 実行履歴をテーブル形式で表示する。
fn print_list_table(records: &[DisplayRunRecord]) {
    // ヘッダー
    println!(
        "{:<6} | {:<20} | {:<8} | {:<12} | {:<20} | {:<10}",
        "ID", "ブロック名", "タグ", "ステータス", "日時", "所要時間"
    );
    println!(
        "{:-<6}-+-{:-<20}-+-{:-<8}-+-{:-<12}-+-{:-<20}-+-{:-<10}",
        "", "", "", "", "", ""
    );
    // 各行
    for record in records {
        println!(
            "{:<6} | {:<20} | {:<8} | {:<12} | {:<20} | {:<10}",
            record.id,
            truncate_str(&record.block_name, 20),
            truncate_str(&record.tag, 8),
            format_status(&record.status),
            format_datetime(&record.started_at),
            format_elapsed(record.elapsed_ms),
        );
    }
}

// ============================================================================
// 詳細表示ハンドラ
// ============================================================================

/// 特定のブロック実行の詳細を表示する。
pub fn handle_block_show(run_id: i64) -> Result<(), Box<dyn std::error::Error>> {
    let store = open_db()?;

    let exec = match store.get_execution_by_id(run_id) {
        Ok(e) => e,
        Err(DbError::NotFound(_)) => {
            eprintln!("エラー: 実行ID '{}' は見つかりません", run_id);
            std::process::exit(EXIT_NOT_FOUND);
        }
        Err(e) => {
            eprintln!("DB エラー: {}", e);
            std::process::exit(EXIT_DB);
        }
    };

    let block = store.get_block_by_id(exec.block_id).ok();

    println!("{}", "═".repeat(55));
    println!("  実行 ID:       {}", exec.id.unwrap_or(0));
    println!(
        "  ブロック名:    {}",
        block.as_ref().map(|b| &b.name).unwrap_or(&"-".to_string())
    );
    println!(
        "  定義ファイル:  {}",
        block
            .as_ref()
            .map(|b| &b.file_path)
            .unwrap_or(&"-".to_string())
    );
    println!("  ステータス:    {}", format_status(&exec.status));
    println!(
        "  開始日時:      {}",
        exec.started_at
            .as_deref()
            .map(format_datetime)
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "  終了日時:      {}",
        exec.finished_at
            .as_deref()
            .map(format_datetime)
            .unwrap_or_else(|| "-".to_string())
    );
    println!("  所要時間:      {}", format_elapsed(exec.duration_ms));
    println!(
        "  終了コード:    {}",
        exec.exit_code
            .map_or_else(|| "-".to_string(), |c| c.to_string())
    );
    println!(
        "  ペイン ID:     {}",
        exec.herdr_pane_id.as_deref().unwrap_or("-")
    );
    println!(
        "  エラーメッセージ: {}",
        exec.error_message.as_deref().unwrap_or("-")
    );
    println!(
        "  Git (起動前):  {}",
        exec.git_commit_before.as_deref().unwrap_or("-")
    );
    println!(
        "  Git (終了後):  {}",
        exec.git_commit_after.as_deref().unwrap_or("-")
    );

    // ブロックの tag / round 表示
    if let Some(ref b) = block {
        if let Some(ref tag) = b.tag {
            println!("  タグ:          {}", tag);
        }
        if let Some(round) = b.round {
            println!("  ラウンド:      {}", round);
        }
    }

    // セッション統計の表示
    if let Some(ref stats_json) = exec.session_stats_json {
        if let Ok(stats) = serde_json::from_str::<serde_json::Value>(stats_json) {
            println!("{} ", "─".repeat(55));
            println!("  [セッション統計]");
            if let Some(v) = stats.get("turn_count").and_then(|v| v.as_u64()) {
                println!("  ターン数:      {}", v);
            }
            if let Some(v) = stats.get("tool_call_count").and_then(|v| v.as_u64()) {
                println!("  ツール呼出:    {}", v);
            }
            if let Some(v) = stats.get("input_tokens").and_then(|v| v.as_u64()) {
                println!("  入力トークン:  {} (近似)", v);
            }
            if let Some(v) = stats.get("output_tokens").and_then(|v| v.as_u64()) {
                println!("  出力トークン:  {} (近似)", v);
            }
            if let Some(v) = stats.get("thinking_tokens").and_then(|v| v.as_u64()) {
                println!("  思考トークン:  {} (近似)", v);
            }
            if let Some(v) = stats.get("total_tokens").and_then(|v| v.as_u64()) {
                println!("  総トークン:    {} (近似)", v);
            }
            if let Some(model) = stats.get("model_name").and_then(|v| v.as_str()) {
                println!("  モデル:        {}", model);
            }
        }
    }

    // waggle トークン表示
    if let Some(ref tokens_json) = exec.output_waggle_tokens {
        if let Ok(tokens) = serde_json::from_str::<Vec<String>>(tokens_json) {
            if !tokens.is_empty() {
                println!("  出力トークン:  {}", tokens.join(", "));
            }
        }
    }
    if let Some(ref tokens_json) = exec.input_waggle_tokens {
        if let Ok(tokens) = serde_json::from_str::<Vec<String>>(tokens_json) {
            if !tokens.is_empty() {
                println!("  入力トークン:  {}", tokens.join(", "));
            }
        }
    }

    // FN-012: 出力内容の表示
    println!("{}", "═".repeat(55));
    println!("  出力内容:");
    println!("{}", "─".repeat(55));
    match exec.output_content {
        Some(content) if !content.is_empty() => {
            println!("{}", content);
        }
        _ => {
            println!("  （出力内容は保存されていません）");
        }
    }
    println!("{}", "─".repeat(55));

    Ok(())
}

// ============================================================================
// ブロック削除ハンドラ
// ============================================================================

/// 指定された実行IDの実行記録を削除する。
/// ブロック定義は削除しない。
pub fn handle_block_rm(exec_id: i64, yes: bool) -> Result<(), Box<dyn std::error::Error>> {
    let store = open_db()?;

    // 実行レコードを取得
    let exec = match store.get_execution_by_id(exec_id) {
        Ok(e) => e,
        Err(_) => {
            eprintln!("エラー: 実行ID '{}' は見つかりません", exec_id);
            std::process::exit(EXIT_NOT_FOUND);
        }
    };

    let block_name = store
        .get_block_by_id(exec.block_id)
        .map(|b| b.name)
        .unwrap_or_else(|_| format!("block#{}", exec.block_id));

    // R602: 確認必須（--yes でスキップ可）。削除対象を明示する。
    if !yes {
        let run_dir = sai_link_engine::run_dir::find_run_dir(exec_id)
            .map(|(_, p)| p.display().to_string())
            .unwrap_or_else(|| "（run フォルダなし）".to_string());
        eprintln!(
            "削除対象: 実行 #{} '{}' ({})\n  DB行＋run フォルダ: {}\n本当に削除しますか？ [y/N]",
            exec_id, block_name, exec.status, run_dir
        );
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if line.trim().to_lowercase() != "y" {
            eprintln!("削除を中止しました");
            return Ok(());
        }
    }

    // 依存レコード（子実行・接続・タスク）と run フォルダ（子の分も）をまとめて削除
    let summary = sai_link_engine::run_dir::delete_run(&store, exec_id)?;

    println!(
        "🗑 実行 #{} '{}' ({}) を削除しました。",
        exec_id, block_name, exec.status
    );
    if summary.executions > 1 {
        println!("   サブ実行: {} 件", summary.executions - 1);
    }
    if summary.connections > 0 {
        println!("   接続: {} 件", summary.connections);
    }
    if summary.tasks > 0 {
        println!("   タスク: {} 件", summary.tasks);
    }

    Ok(())
}

// ============================================================================
// ヘルパー関数
// ============================================================================

/// ステータス文字列を表示用の絵文字付き文字列に変換する。
///
/// 状態の正規化（5.9）: herdr の agent_status（idle/working/done/blocked）と
/// DB の実行ステータス（pending/running/completed/failed/cancelled）を
/// この関数で一元マッピングする。
pub(crate) fn format_status(status: &str) -> &str {
    match status {
        "completed" => "✅ 完了",
        "failed" => "❌ 失敗",
        "cancelled" => "⚠ キャンセル",
        "running" => "🔄 実行中",
        "pending" => "⏳ 待機中",
        "todo" => "📝 未実行（予約）",
        // herdr の agent_status: ユーザーがペインを操作・割り込み中
        "blocked" => "🚧 待機中（blocked）",
        "idle" => "⏸ アイドル",
        "working" => "🔄 作業中",
        "done" => "✅ 完了（done）",
        _ => status,
    }
}

/// ミリ秒を人間可読形式に変換する。
///
/// - `Some(45200)` → `"45.2s"`
/// - `Some(90000)` → `"1m 30s"`
/// - `None` → `"-"`
pub(crate) fn format_elapsed(ms: Option<i64>) -> String {
    match ms {
        None => "-".to_string(),
        Some(v) if v < 1000 => format!("{}ms", v),
        Some(v) if v < 60_000 => {
            let secs = v as f64 / 1000.0;
            format!("{:.1}s", secs)
        }
        Some(v) => {
            let mins = v / 60_000;
            let secs = (v % 60_000) as f64 / 1000.0;
            format!("{}m {:.0}s", mins, secs)
        }
    }
}

/// ISO 8601 文字列を `YYYY-MM-DD HH:MM:SS` 形式に変換する。
pub(crate) fn format_datetime(iso: &str) -> String {
    if iso.len() >= 19 {
        // "2026-07-30T14:23:05" → "2026-07-30 14:23:05"
        let mut result = iso.to_string();
        // 'T' を ' ' に置換
        if let Some(t_pos) = result.find('T') {
            result.replace_range(t_pos..=t_pos, " ");
        }
        // 時間部分のみ（秒まで）に切り詰め
        if result.len() > 19 {
            result.truncate(19);
        }
        result
    } else {
        iso.to_string()
    }
}

/// 文字列を指定文字数で切り詰め、超過時に `...` を付与する。
pub(crate) fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

/// 方向文字列を `SplitDirection` に変換する。
fn parse_direction(s: &str) -> SplitDirection {
    match s {
        "down" => SplitDirection::Down,
        _ => SplitDirection::Right,
    }
}

// ============================================================================
// DB ヘルパー
// ============================================================================

/// デフォルトの DB ファイルパスを取得する。
/// PROJECTROOT/.sai/sai-engine/sai-engine.db に配置する（D15）。
fn default_db_path() -> PathBuf {
    sai_link_engine::project::sai_path(".sai/sai-engine/sai-engine.db")
}

/// DB を開く（`phone` ハンドラからも使うため `pub(crate)`）。
pub(crate) fn open_db() -> Result<SaiStore, Box<dyn std::error::Error>> {
    let db_path = default_db_path();
    // DB ディレクトリが存在しない場合は作成
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let store = SaiStore::open(&db_path)?;
    Ok(store)
}

// ============================================================================
// サブタスク操作ハンドラ
// ============================================================================

/// タスクを操作する（task_manage 用・P4）。
///
/// `agent_api::task_manage` に委譲する（通知を書く処理を二重に持たない）。
/// interrupt と作業中（in_progress）タスクへの send は、デーモンを経由せず
/// MCP サーバが herdr を直接呼ぶ。
pub fn handle_task_manage(
    action: &str,
    task_id: i64,
    message: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let v = sai_link_engine::agent_api::task_manage(action, task_id, message, None)
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    println!(
        "{}",
        v.get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("(結果なし)")
    );
    Ok(())
}

/// waggle トークンを解決してファイル内容を標準出力に書き出す。
/// 隠しコマンド。pi-sai から呼ばれる。
// ============================================================================
// phone 書き出しハンドラ（実装計画 01 §2.4 / 00 §3）
// ============================================================================

/// `sai-link phone export` のハンドラ。
///
/// - 引数なし: 現在のプロジェクトの全実行を書き出す
/// - `--run <id>`: 1 件だけ
/// - `--dry-run`: 書かずに JSON を標準出力へ
pub fn handle_phone_export(
    run: Option<i64>,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use sai_link_engine::phone_export;

    let store = open_db()?;
    let project_root = sai_link_engine::project::project_root();

    if dry_run {
        // ファイルは書かず JSON を標準出力へ出す（確認・02 への引き渡し用）
        match run {
            Some(run_id) => {
                let exec = store.get_execution_by_id(run_id)?;
                if !phone_export::should_export(&exec.status, &exec.runner) {
                    eprintln!(
                        "run {} は書き出し対象外です（status={} runner={}）",
                        run_id, exec.status, exec.runner
                    );
                    return Ok(());
                }
                let name = phone_export::project_name_for(&project_root);
                let doc = phone_export::build_run_doc(&store, &exec, &project_root, &name);
                println!("{}", serde_json::to_string_pretty(&doc)?);
            }
            None => {
                let execs = store.list_all_executions()?;
                let docs = phone_export::build_docs(&store, &execs, &project_root);
                println!("{}", serde_json::to_string_pretty(&docs)?);
            }
        }
        return Ok(());
    }

    match run {
        Some(run_id) => match phone_export::export_one(&store, run_id)? {
            Some(true) => println!("run {} を書き出しました", run_id),
            Some(false) => println!("run {} は内容が同一のため省略しました", run_id),
            None => println!("run {} は書き出し対象外です（todo / pending / user）", run_id),
        },
        None => {
            let summary = phone_export::export_all(&store);
            println!("phone 書き出し: {}", summary.brief());
        }
    }
    Ok(())
}

/// `sai-link cc-prompt-hook`（cc の UserPromptSubmit hook 用）。
pub fn handle_cc_prompt_hook() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;

    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_ok() {
        if let Some(out) = sai_link_engine::cc_command::handle_prompt_hook(&input) {
            println!("{}", out);
        }
    }
    Ok(())
}

/// `sai-link cc-state-hook <working|idle>`（cc の Stop / SessionStart hook 用）。
pub fn handle_cc_state_hook(state: &str) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    sai_link_engine::tmux::set_state_from_hook(state);
    Ok(())
}

/// `sai-link cc-permission-hook`（cc の PermissionRequest hook 用）。
pub fn handle_cc_permission_hook() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    if let Some(out) = sai_link_engine::cc_command::permission_hook_output(&input) {
        println!("{}", out);
    }
    Ok(())
}

/// `sai-link cc-statusline`（cc の statusLine 用）。
pub fn handle_cc_statusline() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    println!("{}", sai_link_engine::phone_export::handle_cc_statusline(&input));
    Ok(())
}

// ============================================================================
// waggle リゾルブ
// ============================================================================

pub fn handle_waggle_resolve(token: &str) -> Result<(), Box<dyn std::error::Error>> {
    use sai_link_engine::waggle_integration::WaggleIntegrator;

    // プロジェクトルート基準（worktree 実行時は pi の cwd が worktree に変わるため）
    let db_path = sai_link_engine::project::sai_path(".sai/sai-engine/sai-engine.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let integrator =
        WaggleIntegrator::open(&db_path).map_err(|e| format!("waggle DB 初期化失敗: {}", e))?;

    match integrator.resolve_content(token) {
        Ok(content) => {
            println!("{}", content);
            Ok(())
        }
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}


// 非対話実行は `kanban_run::kanban_block_run` に一本化されているため、
// `handle_block_run_non_interactive`（後方互換ラッパー）は削除済み。

// ============================================================================
// プロジェクト初期化ハンドラ（`sai-link init`）
// ============================================================================

/// プロジェクトを初期化する。
///
/// 1. `PROJECTROOT/.sai/` と必要なフォルダ・`settings.json` を作成する（既存は触らない）。
///    cc と pi の設定 `.mcp.json` / `.claude/settings.json` / `.pi/settings.json` を作る（既存は `.bak` へ退避）
///    実行の記録の DB（`.sai/sai-engine/sai-engine.db`）も作る（デーモンを起動する前に READ_CARD を作れるように）
/// 2. 登録簿（`~/.sai/registry.json`）にプロジェクトを登録する
pub fn handle_init(name: Option<String>, update: bool) -> Result<(), Box<dyn std::error::Error>> {
    use sai_link_engine::assets::AssetOutcome;
    use sai_link_engine::init::init_project;

    // プロジェクトルート（SAI_PROJECT_ROOT またはカレントディレクトリ）
    let project_root = sai_link_engine::project::project_root();

    // 1. `.sai/` と cc 設定の作成（何度実行してもよい。足りないものだけ作る）
    let report = init_project(&project_root, update)?;
    // DB が無いと、デーモンを一度も起動していないプロジェクトで READ_CARD の予約が失敗する
    open_db()?;

    println!(
        "✅ プロジェクトを初期化しました: {}",
        project_root.display()
    );
    for (from, to) in &report.backed_up {
        println!("   退避: {} → {}", from.display(), to.display());
    }
    for path in &report.created {
        println!("   作成: {}", path.display());
    }
    // 基盤の block・template は、作成・更新だけ 1 行ずつ出し、変わらない物は件数だけ出す
    let mut unchanged = 0usize;
    for (path, outcome) in &report.assets {
        match outcome {
            AssetOutcome::Created => println!("   配布: {}（作成）", path.display()),
            AssetOutcome::Updated { backup } => println!(
                "   配布: {}（更新・退避 {}）",
                path.display(),
                backup.display()
            ),
            AssetOutcome::Unchanged | AssetOutcome::Kept => unchanged += 1,
        }
    }
    if unchanged > 0 {
        println!("   配布: そのまま {unchanged} 件");
    }
    println!(
        "   次: .mcp.json・.claude/・.pi/ を git commit してください（worktree の cc と pi が読むため）"
    );

    // 2. 登録簿（`~/.sai/registry.json`）への登録（apps/web がプロジェクトを名前で引くため）
    let project_name = match name {
        Some(n) if !n.is_empty() => n,
        _ => project_root
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string()),
    };
    match sai_link_engine::project::sai_home().map(|h| h.join("registry.json")) {
        Some(registry) => match sai_link_engine::phone_export::register_project(&registry, &project_name, &project_root) {
            Ok(sai_link_engine::phone_export::RegistryAdd::Added) => {
                println!("✅ 登録簿に登録しました: {} ({})", project_name, registry.display());
            }
            Ok(sai_link_engine::phone_export::RegistryAdd::AlreadyRegistered(existing)) => {
                println!("✅ 登録簿には既に登録されています: {existing}");
            }
            Err(e) => println!("⚠ 登録簿に登録できませんでした: {e}"),
        },
        None => println!("⚠ HOME が分からないため、登録簿に登録できませんでした"),
    }

    Ok(())
}

// ============================================================================
// sai デーモン操作ハンドラ（Phase 2・§7.1.1 / §7.1.2）
// ============================================================================

/// デーモンをバックグラウンドで起動する。
///
/// 自身を子プロセスとして spawn（nohup 相当）し、PID を `.sai/sai-engine/daemon.pid`
/// に書き込んで即終了します（D2）。ログは `.sai/sai-engine/daemon.log` に出力されます。
pub fn handle_daemon_start() -> Result<(), Box<dyn std::error::Error>> {
    use std::process::Stdio;

    // 既に起動中なら何もしない
    if daemon::is_running() {
        if let Some(pid) = daemon::read_pid() {
            println!("⚠ sai デーモンは既に起動中です (PID: {})", pid);
        }
        return Ok(());
    }

    // ログファイルを用意（project_root は SAI_PROJECT_ROOT 基準に統一）
    let project_root = sai_link_engine::project::project_root();
    let exe = std::env::current_exe()?;

    // ログファイルを用意
    let log_path = project_root
        .join(".sai")
        .join("sai-engine")
        .join("daemon.log");
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    // 子プロセスとしてデーモン本体（daemon _run）を起動
    // P1-5: pi pane内から起動した場合に親の実行系変数を引き継がないよう除去する。
    // 除去対象: SAI_PARENT_RUN_ID / SAI_BLOCK_NAME / SAI_MAKASERU / SAI_IS_SUBTASK。
    // SAI_PROJECT_ROOT のみ当該プロジェクトで上書きする。
    let child = std::process::Command::new(&exe)
        .arg("daemon")
        .arg("_run")
        .current_dir(&project_root)
        .env("SAI_PROJECT_ROOT", &project_root)
        .env_remove("SAI_PARENT_RUN_ID")
        .env_remove("SAI_BLOCK_NAME")
        .env_remove("SAI_MAKASERU")
        .env_remove("SAI_IS_SUBTASK")
        .stdin(Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file)
        .spawn()?;

    let pid = child.id() as i32;
    daemon::write_pid(pid)?;

    println!("✅ sai デーモンを起動しました (PID: {})", pid);
    println!("   プロジェクトルート: {}", project_root.display());
    println!("   ログ: {}", log_path.display());
    println!("   停止: `sai daemon stop`");

    Ok(())
}

/// デーモンのポーリングループをフォアグラウンドで実行する（内部用）。
///
/// `daemon start` がバックグラウンド起動する子プロセスの本体です。
pub fn handle_daemon_run() -> Result<(), Box<dyn std::error::Error>> {
    let project_root = std::env::var("SAI_PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    println!(
        "🚀 sai デーモン起動 (project root: {})",
        project_root.display()
    );
    daemon::run_foreground(&project_root)
}

/// デーモンを停止する（P0-3・P0-2）。
///
/// 実行中の作業（`daemon.status` の実行中ブロック・タスク＋DBの `running`）が
/// ある場合は停止せず警告する（終了コード非ゼロ相当のエラーを返す）。
/// `--force` でのみ SIGTERM を送り、デーモン側の確定終了（P0-2）が走る。
/// 終了後はプロセス消滅を最大10秒待ってから PID ファイルを削除する（P1-4）。
pub fn handle_daemon_stop(force: bool) -> Result<(), Box<dyn std::error::Error>> {
    match daemon::read_pid() {
        Some(pid) if daemon::process_alive(pid) => {
            if !force {
                let busy = daemon_busy_description();
                if !busy.is_empty() {
                    eprintln!(
                        "⚠ 実行中の作業があります。先に完了させてください: {}",
                        busy.join(" / ")
                    );
                    eprintln!("  強制停止は `sai-link daemon stop --force` を付けてください。");
                    return Err("実行中の作業があるため停止しませんでした".into());
                }
            }
            // SIGTERM を送信（デーモン側で確定終了が走る）
            std::process::Command::new("kill")
                .arg(pid.to_string())
                .status()?;
            // 終了待ち（最大10秒ポーリング。P0-2の確定処理の猶予）
            for _ in 0..100 {
                if !daemon::process_alive(pid) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = daemon::remove_pid_file();
            println!("🛑 sai デーモンを停止しました (PID: {})", pid);
        }
        _ => {
            // 残骸（PID ファイル）があれば掃除
            let _ = daemon::remove_pid_file();
            println!("⚠ sai デーモンは起動していません。");
        }
    }
    Ok(())
}

/// 停止可否の判定用に「実行中の作業」の説明文を集める（P0-3）。
///
/// `daemon.status` の `running_executions`・`active_tasks`・`queued_tasks` と、
/// DBの `status='running'`（`execution_history`）・`in_progress`／`queued`
/// （`tasks`）を照会する。いずれも空なら空ベクタを返す。
fn daemon_busy_description() -> Vec<String> {
    let mut out = Vec::new();
    let status_path = sai_link_engine::project::sai_path(".sai/sai-engine/daemon.status");
    if let Ok(content) = std::fs::read_to_string(&status_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            let ids = |key: &str| -> Vec<i64> {
                v.get(key)
                    .and_then(|a| a.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
                    .unwrap_or_default()
            };
            let running = ids("running_executions");
            if !running.is_empty() {
                out.push(format!("実行中ブロック {:?}", running));
            }
            let active = ids("active_tasks");
            if !active.is_empty() {
                out.push(format!("実行中タスク {:?}", active));
            }
            let queued = ids("queued_tasks");
            if !queued.is_empty() {
                out.push(format!("キュー待ちタスク {:?}", queued));
            }
        }
    }
    if let Ok(store) = open_db() {
        if let Ok(running) = store.list_running_executions() {
            if !running.is_empty() {
                let ids: Vec<i64> = running.iter().filter_map(|e| e.id).collect();
                out.push(format!("DB実行中 {:?}", ids));
            }
        }
        for st in ["in_progress", "queued"] {
            if let Ok(tasks) = store.list_tasks_by_status(&[st]) {
                if !tasks.is_empty() {
                    out.push(format!("タスク({}) {} 件", st, tasks.len()));
                }
            }
        }
    }
    out
}

/// デーモンの状態を表示する。
///
/// PID ファイルの生存確認 + ステータスファイル（`.sai/sai-engine/daemon.status`）の
/// 実行中/キュー待ちタスク数を表示します（§7.1.1）。
/// 幽霊（P1-1）がある場合は取り残し行と復旧案内（P2-2）を追加表示する。
/// デーモン停止中でもDB直読みで幽霊を表示する（TUIと同一方針）。
pub fn handle_daemon_status() -> Result<(), Box<dyn std::error::Error>> {
    // 幽霊一覧はDB照会を正とする（停止中も表示できるよう）。
    let orphans: Vec<sai_link_engine::daemon::OrphanedRun> = open_db()
        .map(|s| sai_link_engine::daemon::list_orphaned_runs(&s))
        .unwrap_or_default();
    // is_running(): process_alive + is_daemon_process（/proc cmdline 確認）を経由し、
    // PID 再利用による誤判定を防ぐ（daemon.rs に一元化）。
    if daemon::is_running() {
        if let Some(pid) = daemon::read_pid() {
            println!("✅ sai デーモン: 実行中 (PID: {})", pid);

            // ステータスファイルを読み取り
            let status_path = sai_link_engine::project::sai_path(".sai/sai-engine/daemon.status");
            if let Ok(content) = std::fs::read_to_string(&status_path) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    let fmt_ids = |arr: &serde_json::Value| -> String {
                        arr.as_array()
                            .map(|a| {
                                a.iter()
                                    .map(|x| x.as_i64().map(|i| i.to_string()).unwrap_or_default())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| "(なし)".to_string())
                    };
                    println!("   実行中タスク:   {}", fmt_ids(&v["active_tasks"]));
                    println!("   キュー待ちタスク: {}", fmt_ids(&v["queued_tasks"]));
                    println!("   実行中ブロック:  {}", fmt_ids(&v["running_executions"]));
                    // 鮮度表示: last_heartbeat との差が5秒超なら古い旨を出す（P1-4）。
                    if let Some(hb) = v.get("last_heartbeat").and_then(|x| x.as_u64()) {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;
                        if now.saturating_sub(hb) > 5000 {
                            println!("   ※ 状態表示が古い可能性があります（最終更新から5秒超）");
                        }
                    }
                }
            }
        }
    } else {
        println!("⚠ sai デーモンは起動していません。");
        println!("   `sai daemon start` で起動してください。");
    }
    if !orphans.is_empty() {
        println!("   取り残し（幽霊）: {} 件", orphans.len());
        for o in &orphans {
            println!(
                "     - run {}（{}・開始 {}）",
                o.run_id, o.block_name, o.started_at
            );
        }
        println!("   管理者は `sai-link daemon recover` を参照してください。");
    }
    Ok(())
}

/// 取り残し（幽霊）の確認・確定を行う（P2-2）。
///
/// デーモン稼働中は対象外（生きている実行と区別できないため停止してから使う）。
/// `--apply` 無しでは一覧表示のみ（DBを触らない）。`--apply` でのみ幽霊を
/// `failed` + 理由付きで確定する。自動確定はしない。
pub fn handle_daemon_recover(apply: bool) -> Result<(), Box<dyn std::error::Error>> {
    if daemon::is_running() {
        eprintln!("⚠ デーモン稼働中は recover できません。先に `sai-link daemon stop` してください。");
        return Err("デーモン稼働中のため recover しませんでした".into());
    }
    let store = open_db()?;
    let orphans = sai_link_engine::daemon::list_orphaned_runs(&store);
    // pending 残量の表示
    let pending_dir = sai_link_engine::project::sai_path(".sai/sai-engine/notifications/pending");
    let pending_count = std::fs::read_dir(&pending_dir)
        .map(|it| {
            it.filter(|e| {
                e.as_ref()
                    .ok()
                    .map(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
                    .unwrap_or(false)
            })
            .count()
        })
        .unwrap_or(0);
    if orphans.is_empty() {
        println!("取り残しはありません（pending 残量: {} 件）。", pending_count);
        return Ok(());
    }
    println!("取り残し（幽霊）: {} 件（pending 残量: {} 件）", orphans.len(), pending_count);
    for o in &orphans {
        println!(
            "  - run {}（{}・開始 {}・理由 {}）",
            o.run_id, o.block_name, o.started_at, o.reason
        );
    }
    if !apply {
        println!("確定するには `sai-link daemon recover --apply` を実行してください（DBを変更します）。");
        return Ok(());
    }
    for o in &orphans {
        store.update_execution_error(o.run_id, "デーモン停止中の手動確定（daemon recover）", 1)?;
        println!("  ✅ run {} を failed に確定しました", o.run_id);
    }
    Ok(())
}

/// ブロック定義キャッシュの再読込を要求する。
///
/// `.sai/sai-engine/reload.flag` を書き込み、デーモンが次回ポーリングで処理します。
pub fn handle_daemon_reload() -> Result<(), Box<dyn std::error::Error>> {
    if !daemon::is_running() {
        println!("⚠ sai デーモンが起動していません。");
        return Ok(());
    }

    let flag_path = sai_link_engine::project::sai_path(".sai/sai-engine/reload.flag");
    if let Some(parent) = flag_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&flag_path, "1")?;
    println!("🔄 ブロック定義の再読込を要求しました。");
    Ok(())
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- format_status のテスト ----

    #[test]
    fn test_format_status_completed() {
        assert_eq!(format_status("completed"), "✅ 完了");
    }

    #[test]
    fn test_format_status_failed() {
        assert_eq!(format_status("failed"), "❌ 失敗");
    }

    #[test]
    fn test_format_status_cancelled() {
        assert_eq!(format_status("cancelled"), "⚠ キャンセル");
    }

    #[test]
    fn test_format_status_running() {
        assert_eq!(format_status("running"), "🔄 実行中");
    }

    #[test]
    fn test_format_status_pending() {
        assert_eq!(format_status("pending"), "⏳ 待機中");
    }

    // ---- format_elapsed のテスト ----

    #[test]
    fn test_format_elapsed_seconds() {
        assert_eq!(format_elapsed(Some(45200)), "45.2s");
    }

    #[test]
    fn test_format_elapsed_minutes() {
        assert_eq!(format_elapsed(Some(90000)), "1m 30s");
    }

    #[test]
    fn test_format_elapsed_none() {
        assert_eq!(format_elapsed(None), "-");
    }

    #[test]
    fn test_format_elapsed_milliseconds() {
        assert_eq!(format_elapsed(Some(500)), "500ms");
    }

    #[test]
    fn test_format_elapsed_exact_minute() {
        assert_eq!(format_elapsed(Some(60000)), "1m 0s");
    }

    // ---- truncate_str のテスト ----

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_str_long() {
        assert_eq!(
            truncate_str("hello world this is long", 10),
            "hello worl..."
        );
    }

    #[test]
    fn test_truncate_str_exact() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    // ---- format_datetime のテスト ----

    #[test]
    fn test_format_datetime_iso() {
        assert_eq!(
            format_datetime("2026-07-30T14:23:05"),
            "2026-07-30 14:23:05"
        );
    }

    #[test]
    fn test_format_datetime_with_timezone() {
        assert_eq!(
            format_datetime("2026-07-30T14:23:05+09:00"),
            "2026-07-30 14:23:05"
        );
    }

    #[test]
    fn test_format_datetime_empty() {
        assert_eq!(format_datetime(""), "");
    }

    // ---- parse_direction のテスト ----

    #[test]
    fn test_parse_direction_right() {
        assert!(matches!(parse_direction("right"), SplitDirection::Right));
    }

    #[test]
    fn test_parse_direction_down() {
        assert!(matches!(parse_direction("down"), SplitDirection::Down));
    }

    #[test]
    fn test_parse_direction_default() {
        assert!(matches!(parse_direction("unknown"), SplitDirection::Right));
    }

    // ---- 統合テスト: print_list_table の出力チェック ----

    #[test]
    fn test_print_list_table_empty() {
        // 空のテーブルはヘッダーのみを表示
        let records: Vec<DisplayRunRecord> = vec![];
        print_list_table(&records);
        // エラーなしで実行できれば OK
    }

    #[test]
    fn test_print_list_table_with_records() {
        let records = vec![
            DisplayRunRecord {
                id: 3,
                block_name: "sai-brst-grill".to_string(),
                tag: "brst".to_string(),
                status: "completed".to_string(),
                started_at: "2026-07-30T14:23:05".to_string(),
                elapsed_ms: Some(45200),
            },
            DisplayRunRecord {
                id: 2,
                block_name: "sai-plan-synth".to_string(),
                tag: "plan".to_string(),
                status: "failed".to_string(),
                started_at: "2026-07-30T13:10:00".to_string(),
                elapsed_ms: Some(12100),
            },
            DisplayRunRecord {
                id: 1,
                block_name: "simple-task".to_string(),
                tag: "".to_string(),
                status: "cancelled".to_string(),
                started_at: "2026-07-30T12:01:00".to_string(),
                elapsed_ms: None,
            },
        ];
        print_list_table(&records);
        // エラーなしで実行できれば OK
    }
}
