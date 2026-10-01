//! # sai デーモン
//!
//! 常駐プロセスとして通知を一元処理し、メインブロック・サブブロックの
//! 実行を生成・監視・完了処理します（§3・§7.1.1）。
//!
//! ## 役割
//!
//! - `task_*` 通知の処理（`TaskManager::poll_one_event`）
//! - `file_written`（sai-write）通知の処理 → 実行監視スレッドへ完了イベント送信
//! - `block_run_request` 通知の処理（メインブロック実行開始）
//! - ブロック定義の定期リロード（mtime 監視 + `daemon reload`）
//! - AgentPool の非同期ワーカー（サブタスク実行）
//! - PID ファイル / ステータスファイルの管理
//!
//! ## 完了判定（D4）
//!
//! 実行の完了は「sai-write の最初の1回目の検出」で判定します。
//! デーモンのポーリングループが `file_written` 通知を処理（waggle トークン化①）し、
//! 通知の `run_id` に対応する実行監視チャネルへ `CompletionEvent` を送信します。
//! 実行スレッドはそれを受けて順序厳守で完了処理（②③④⑤）を実行します（§4.2 ケース1）。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agent_pool::AgentPool;
use crate::db::SaiStore;
use crate::orchestrator::{self, CompletionEvent, ExecutionOverrides, RunningExecutions};
use crate::parser::parse_block_file;
use crate::phone_export;
use crate::sai_write_notify;
use crate::task_manager::TaskManager;
use crate::waggle_integration::WaggleIntegrator;
/// サブタスク実行ワーカーの最大並列数（AgentPool の max_concurrent と同等）
const MAX_WORKERS: usize = 4;

/// 実行タスクキューのジョブ種別。
///
/// - `Execute`: 通常の新規実行（execute_subtask。新規 worktree / pane でブロックを起動）
/// - `Turn`: 同一 pane へのターン送信（execute_task_turn。task_manage(send) の新実装）
#[derive(Debug, Clone, Copy)]
pub enum WorkerJob {
    /// 新規実行（execute_subtask）
    Execute(i64),
    /// 同一 pane へのターン送信（execute_task_turn）
    Turn(i64),
}

/// ポーリング間隔（アイドル時の見回り周期）。
///
/// 通知の到着はファイルベースのため、短くしても到着は速くならない。
/// 変更時は `handle_daemon_status` の鮮度表示（`last_heartbeat`）の説明も更新する。
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// ステータスファイルの書き込み間隔（ポーリング回数）。
///
/// `daemon.status` は `daemon status` 表示用。毎回書くと変化がなくても
/// ファイル更新・JSON 整形が発生するため、N 回に 1 回に間引く。
const STATUS_WRITE_EVERY: u32 = 10;

// ============================================================================
// block_run_request 通知（メインブロック実行要求・§5.2）
// ============================================================================

/// `block_run_request` 通知の JSON 構造。
///
/// pi-sai ではなく `sai block run` CLI / 対話モードが書き込み、
/// デーモンが `pending/` をスキャンして処理します（§7.1.1）。
/// 実行時オーバーライドは `orchestrator::ExecutionOverrides` に一元化（P8）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BlockRunRequest {
    /// 通知タイプ: "block_run_request"
    #[serde(rename = "type")]
    pub notification_type: String,
    /// 通知タイムスタンプ（ISO 8601・5.3 の修正）
    #[serde(default)]
    pub timestamp: String,
    /// ブロック定義ファイルのパス
    pub block_path: String,
    /// ブロック名
    pub block_name: String,
    /// ユーザー入力（フィールド id → 値）
    #[serde(default)]
    pub user_inputs: HashMap<String, String>,
    /// 実行時設定のオーバーライド（§6・P8 で単一構造体に統合）
    #[serde(default)]
    pub overrides: ExecutionOverrides,
    /// kanban 実行に使ったカード md のプロジェクト相対パス（`.sai/todo/xxx.md`）。
    ///
    /// デーモンが `run_id` 確定後に `run_id` をカードへ書き戻す（二段階更新・R121）。
    /// 旧通知（本フィールド無し）との互換のため `default`（None 許容）。
    #[serde(default)]
    pub card_path: Option<String>,
    /// 予約 run_id（outputs 統一・R302。新カードは `Shift+C` 時に予約済み）。
    ///
    /// `Some` ならデーモンは新規行を作らず予約行を確定させる。
    /// 旧通知（無し）は従来どおり新規発行。
    #[serde(default)]
    pub reserved_run_id: Option<i64>,
}

// ============================================================================
// Daemon 構造体
// ============================================================================

/// 幽霊実行（orphaned_run）の1件分の情報（P1-1）。
///
/// デーモン起動時の `status='running'` スキャンで作り、`daemon.status` の
/// `orphaned_runs` に載せる。自動修復はしない（表示・明示確定に留める）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OrphanedRun {
    /// 取り残された実行 ID
    pub run_id: i64,
    /// ブロック名（解決できなければ `(block_id=N)`）
    pub block_name: String,
    /// 開始日時（`started_at`。無ければ空文字）
    pub started_at: String,
    /// 取り残し理由（当面 `daemon_restart` 固定）
    pub reason: String,
}

/// 幽霊実行一覧を DB から組み立てる（P1-1・`daemon status`／`recover` 共用）。
///
/// `status='running'` の全実行を `orphaned_runs` 形式に変換する。
/// ブロック名は `block_metadata` から解決し、失敗時は `(block_id=N)` とする。
pub fn list_orphaned_runs(store: &SaiStore) -> Vec<OrphanedRun> {
    let running = store.list_running_executions().unwrap_or_default();
    running
        .into_iter()
        .filter_map(|e| {
            let run_id = e.id?;
            let block_name = store
                .get_block_by_id(e.block_id)
                .map(|b| b.name)
                .unwrap_or_else(|_| format!("(block_id={})", e.block_id));
            Some(OrphanedRun {
                run_id,
                block_name,
                started_at: e.started_at.unwrap_or_default(),
                reason: "daemon_restart".to_string(),
            })
        })
        .collect()
}

/// finalize 結果ファイル（`results/finalize-<run_id>-<ms>.json`）を書き込む（P2-1）。
///
/// pi 拡張（`/pi-sai-finalize`）がポーリングして成否表示するための応答である。
/// `task_*` 系の `results/` 応答と同形式のディレクトリ規約に従う。
fn write_finalize_result(run_id: i64, ok: bool, reason: &str) {
    let results_dir = crate::common::notify_results_dir();
    let _ = std::fs::create_dir_all(&results_dir);
    let body = serde_json::json!({
        "type": "finalize_result",
        "run_id": run_id,
        "ok": ok,
        "reason": reason,
    });
    let filename = format!("finalize-{}-{}.json", run_id, unix_millis());
    let path = results_dir.join(filename);
    if let Ok(text) = serde_json::to_string_pretty(&body) {
        let _ = std::fs::write(&path, text);
    }
}

/// `exp_exec.py status` の `state` が「終わった」を表す値か。
fn job_state_finished(state: &str) -> bool {
    matches!(state, "done" | "failed" | "cancelled" | "lost")
}

/// `exp_exec.py status <job_id> --json` の標準出力から `state` を読む。読めなければ `None`。
fn parse_job_state(stdout: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    v.get("state")?.as_str().map(str::to_string)
}

/// ジョブの状態を読む命令を作る。
///
/// 環境変数 `SAI_EXP_STATUS_CMD` があれば `<cmd> status <job_id> --json`、
/// 無ければ `uv run ... exp_exec.py status <job_id> --json`。
fn job_status_command(root: &Path, job_id: &str) -> std::process::Command {
    let mut cmd = match std::env::var_os("SAI_EXP_STATUS_CMD") {
        Some(exe) => std::process::Command::new(exe),
        None => {
            let mut c = std::process::Command::new("uv");
            c.args(["run", "--no-project", "--python", ">=3.11", "python"])
                .arg(root.join(".sai/tools/exp_exec.py"));
            c
        }
    };
    cmd.args(["status", job_id, "--json"])
        .current_dir(root)
        .env("SAI_PROJECT_ROOT", root);
    cmd
}

/// ジョブの状態を読む（90 秒で打ち切る）。失敗は文で返す。
fn read_job_state(root: &Path, job_id: &str) -> Result<String, String> {
    let mut child = job_status_command(root, job_id)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("命令を起動できません: {}", e))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("90 秒で時間切れになりました".to_string());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(format!("命令の待機に失敗: {}", e)),
        }
    };
    if !status.success() {
        return Err(format!("終了コード {}", status.code().unwrap_or(-1)));
    }
    let mut out = String::new();
    if let Some(mut so) = child.stdout.take() {
        use std::io::Read;
        let _ = so.read_to_string(&mut out);
    }
    parse_job_state(&out).ok_or_else(|| "state を読めません".to_string())
}

/// 状態が「終わった」なら、`task.md` を読み直して `after_job` が同じ値のときだけ消し、
/// 起こす run_id を返す。途中で解除・変更されていたら何もしない。
fn release_job_card(path: &Path, job_id: &str, state: Option<&str>) -> Option<i64> {
    if !state.is_some_and(job_state_finished) {
        return None;
    }
    let mut card = crate::kanban::parse_card(&std::fs::read_to_string(path).ok()?)?;
    if card.frontmatter.meta.get("after_job").map(|s| s.trim()) != Some(job_id) {
        return None;
    }
    let run_id = card.frontmatter.run_id.trim().parse::<i64>().ok()?;
    card.frontmatter.meta.remove("after_job");
    std::fs::write(path, crate::kanban::render_card(&card)).ok()?;
    Some(run_id)
}

/// `sai-moon-link phone card run <run_id>` を別プロセスで起こす（終わりは別の糸で待つ）。
fn spawn_card_run(root: &Path, run_id: i64) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let spawned = std::process::Command::new(exe)
        .args(["phone", "card", "run", &run_id.to_string()])
        .current_dir(root)
        .env("SAI_PROJECT_ROOT", root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => eprintln!("[sai-engine] ジョブ後の実行に失敗: run_id={}: {}", run_id, e),
    }
}

/// sai デーモン。
pub struct Daemon {
    /// データベースストア
    store: Arc<SaiStore>,
    /// タスク管理マネージャー（AgentPool 共有）
    task_manager: Arc<TaskManager>,
    /// エージェント実行プール（同時実行制御・ステータス表示用）
    agent_pool: Arc<AgentPool>,
    /// 通知ディレクトリのベースパス
    notify_base: PathBuf,
    /// 実行監視マップ: run_id → 完了通知チャネル（§7.1.1）
    running_executions: RunningExecutions,
    /// ブロック定義キャッシュ: パス → (mtime, サイズ)（定期リロード用・§7.1.1）
    block_def_cache: Mutex<HashMap<PathBuf, (SystemTime, u64)>>,
    /// phone 書き出しの差分管理（実装計画 01 §2.3）
    phone_export: Mutex<PhoneExportState>,
    /// ジョブの状態を読んでいる最中の run_id（同じカードの糸を重ねて立てない）
    job_busy: Arc<Mutex<HashSet<i64>>>,
    /// カードごとの前回の失敗の文（同じ失敗を繰り返しログに出さない）
    job_errs: Arc<Mutex<HashMap<i64, String>>>,
}

/// phone 書き出し（`~/.sai-moon/phone/runs/`）の差分管理状態（01 §2.3）。
struct PhoneExportState {
    /// 書き出し器（プロジェクト名解決済み・初回サイクルで遅延作成）
    exporter: Option<phone_export::Exporter>,
    /// run_id → 前回書き出し時のシグネチャ
    sigs: HashMap<i64, phone_export::ExportSig>,
    /// 終了直後でもう一度だけ書き直す対象
    rewrite_once: HashSet<i64>,
}

impl Default for PhoneExportState {
    fn default() -> Self {
        Self {
            exporter: None,
            sigs: HashMap::new(),
            rewrite_once: HashSet::new(),
        }
    }
}

impl Daemon {
    /// 新しい `Daemon` を作成する。
    pub fn new(store: Arc<SaiStore>) -> Self {
        // 通知ディレクトリは common の定数に一元化（P17・5.5）
        let notify_base = crate::common::notify_base_dir();
        let _ = std::fs::create_dir_all(&notify_base);

        // AgentPool を共有（デーモンはワーカーとステータス表示で使用）
        let agent_pool = Arc::new(AgentPool::new());
        let task_manager = Arc::new(TaskManager::with_agent_pool(
            Arc::clone(&store),
            Arc::clone(&agent_pool),
        ));

        Self {
            store,
            task_manager,
            agent_pool,
            notify_base,
            running_executions: Arc::new(Mutex::new(HashMap::new())),
            block_def_cache: Mutex::new(HashMap::new()),
            phone_export: Mutex::new(PhoneExportState::default()),
            job_busy: Arc::new(Mutex::new(HashSet::new())),
            job_errs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // -----------------------------------------------------------------------
    // メインループ
    // -----------------------------------------------------------------------

    /// ポーリングループを無限に実行する（§7.1.1）。
    ///
    /// 1. `task_*` 通知の処理（Started タスクは実行タスクキューへ）
    /// 2. `file_written` 通知の処理（run_id を照合して完了イベント送信）
    /// 3. `block_run_request` 通知の処理（メインブロック実行開始）
    /// 4. ブロック定義の定期リロード
    /// 5. `daemon reload` フラグの処理
    /// 6. ステータスファイルの更新
    fn loop_forever(&self) -> Result<(), Box<dyn std::error::Error>> {
        let db_path = crate::project::sai_path(".sai/sai-engine/sai-engine.db");
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let integrator = WaggleIntegrator::open(&db_path)?;

        // ── AgentPool の非同期ワーカー（§7.1.1）──
        // Started タスク（および complete() で昇格したタスク）を実行タスクキューから受信し、
        // ワーカースレッド内で同期的に execute_subtask を実行する。
        // task_manage(send) のターン送信（同一 pane 継続）は execute_task_turn で実行する。
        // ※ mpsc::Receiver は Clone できないため、Arc<Mutex<_>> で共有する。
        let (task_tx, task_rx) = mpsc::channel::<WorkerJob>();
        let task_rx = Arc::new(Mutex::new(task_rx));
        let mut workers = Vec::new();
        for _ in 0..MAX_WORKERS {
            let rx = Arc::clone(&task_rx);
            let tx = task_tx.clone();
            let manager = Arc::clone(&self.task_manager);
            let running_executions = Arc::clone(&self.running_executions);
            workers.push(std::thread::spawn(move || loop {
                let job = match rx.lock().unwrap().recv() {
                    Ok(j) => j,
                    Err(_) => break, // デーモン停止
                };
                let promoted = match job {
                    WorkerJob::Execute(task_id) => {
                        manager.execute_subtask(task_id, Some(&running_executions))
                    }
                    WorkerJob::Turn(task_id) => {
                        manager.execute_task_turn(task_id, Some(&running_executions))
                    }
                };
                // 完了処理（complete_task / fail_task）で昇格したタスクを再投入
                // （AgentPool の実行上限待ち + 依存解決で昇格したタスク）
                for p in promoted {
                    let _ = tx.send(WorkerJob::Execute(p));
                }
            }));
        }

        eprintln!(
            "[sai-engine] デーモンのポーリングループを開始します (PID: {})",
            std::process::id()
        );

        // ── メインループ ──
        // `tick` はステータスファイルの間引き書き込み用カウンタ。
        let mut tick: u32 = 0;
        loop {
            // P0-2: 停止要求（SIGTERM/SIGINT）があれば確定終了する。
            // 新規受付を止め、実行中を確定させてから抜ける。
            if shutdown_requested() {
                self.shutdown_gracefully();
                return Ok(());
            }
            // 1. task_* 通知の処理
            match self.task_manager.poll_one_event() {
                Ok(Some(event)) => {
                    eprintln!("[sai-engine] {}", event.message);
                    for task_id in &event.started_task_ids {
                        eprintln!("[sai-engine] 実行タスクキューに投入: task_id={}", task_id);
                        let _ = task_tx.send(WorkerJob::Execute(*task_id));
                    }
                    for task_id in &event.turn_task_ids {
                        eprintln!("[sai-engine] ターン送信キューに投入: task_id={}", task_id);
                        let _ = task_tx.send(WorkerJob::Turn(*task_id));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("[sai-engine] task 通知の処理エラー: {}", e);
                }
            }

            // 2. file_written 通知の処理（① waggle トークン化 + 完了イベント送信）
            match sai_write_notify::poll_and_process_one(&integrator) {
                Ok(Some(processed)) => {
                    if let Some(run_id) = processed.run_id {
                        // P4: エージェント側で発行済みのトークンを実行中でも DB に載せる。
                        // 完了時の update_execution_waggle_tokens で同じ集合に上書きされる。
                        if let Some(token) = processed.waggle_token.as_deref() {
                            let _ = self.store.append_execution_waggle_token(run_id, token);
                        }
                        let event = CompletionEvent::SaiWriteDetected {
                            run_id,
                            waggle_token: processed.waggle_token.clone(),
                            file_path: processed.file_path.clone(),
                        };
                        let map = self.running_executions.lock().unwrap();
                        if let Some(tx) = map.get(&run_id) {
                            let _ = tx.send(event);
                        }
                        drop(map);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("[sai-engine] file_written 通知の処理エラー: {}", e);
                }
            }

            // 3. block_run_request 通知の処理（メインブロック実行開始）
            if let Err(e) = self.process_block_run_request() {
                eprintln!("[sai-engine] block_run_request の処理エラー: {}", e);
            }

            // 3.5 finalize_request 通知の処理（ユーザーによる明示的な完了要求）
            if let Err(e) = self.process_finalize_request() {
                eprintln!("[sai-engine] finalize_request の処理エラー: {}", e);
            }

            // 4. ブロック定義の定期リロード（mtime 比較・§7.1.1）
            if let Err(e) = self.reload_block_definitions_if_changed() {
                eprintln!("[sai-engine] ブロック定義リロードエラー: {}", e);
            }

            // 5. daemon reload フラグ（手動リロード要求）
            let _ = self.check_reload_flag();

            // 6. ステータスファイルの更新（`daemon status` 表示用・間引き書き込み）
            // 変化がなくても毎回書くとファイル更新が起きるため、N 回に 1 回にする。
            // 停止直後の鮮度が気になる場合は `STATUS_WRITE_EVERY` を下げる。
            tick += 1;
            if tick % STATUS_WRITE_EVERY == 0 {
                let _ = self.write_status_file();
                // phone 書き出しも同じ周期で差分を書き出す（実装計画 01 §2.3）
                self.export_phone_runs(false);
                // 実行予定時刻を過ぎた todo のカードを実行する
                self.run_scheduled_cards();
            }
            // ジョブの終わりを待つ todo のカードを見回る（約 60 秒ごと）
            if tick % (STATUS_WRITE_EVERY * 12) == 0 {
                self.run_job_waiting_cards();
            }

            std::thread::sleep(POLL_INTERVAL);
        }
    }

    // -----------------------------------------------------------------------
    // block_run_request の処理（メインブロック実行・§3.1）
    // -----------------------------------------------------------------------

    /// `pending/` 内の `block_run_request` 通知を処理する。
    ///
    /// `TaskManager::poll_one_event` / `sai_write_notify` は block_run_request を
    /// 処理しないため、デーモン側で独自にスキャンします（§7.1.1）。
    fn process_block_run_request(&self) -> Result<(), Box<dyn std::error::Error>> {
        let pending = self.notify_base.join("pending");
        let processed = self.notify_base.join("processed");
        let _ = std::fs::create_dir_all(&pending);
        let _ = std::fs::create_dir_all(&processed);

        let mut entries: Vec<_> = std::fs::read_dir(&pending)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();
        entries.sort_by_key(|e| e.file_name());

        // P7: 1 回のポーリングで backlog をすべて消化する（1.12 の修正）。
        // 複数の block_run_request が溜まっていても 1 件ずつ順番に処理する。
        for entry in entries {
            let notification_path = entry.path();
            let content = match std::fs::read_to_string(&notification_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let type_value: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if type_value.get("type").and_then(|v| v.as_str()) != Some("block_run_request") {
                continue;
            }

            let req: BlockRunRequest = match serde_json::from_str(&content) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[sai-engine] block_run_request のパースに失敗: {}", e);
                    // パース不能な通知は処理済みに移動して再処理を防ぐ
                    let _ = self.move_to_processed(&notification_path, &processed);
                    continue;
                }
            };

            // メインブロック実行を開始（別スレッド）
            self.start_main_block(&req);

            // 処理済みに移動
            let _ = self.move_to_processed(&notification_path, &processed);
        }

        Ok(())
    }

    /// 通知ファイルを processed/ に移動する（ベストエフォート）。
    /// 実行予定時刻（task.md の `scheduled_at:`）を過ぎた todo のカードを実行する。
    ///
    /// 予定を先に消してから、`sai-moon-link phone card run <run_id>`（Web の ▶ 実行と同じ流れ）を
    /// 別プロセスで呼ぶ。失敗しても繰り返さない（予定は消えている）。
    fn run_scheduled_cards(&self) {
        let todo = crate::project::sai_path(crate::run_dir::OUTPUTS_DIR)
            .join(crate::run_dir::TODO_OWNER);
        let Ok(entries) = std::fs::read_dir(&todo) else {
            return;
        };
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let now = chrono::Local::now().naive_local();
        for entry in entries.flatten() {
            let path = entry.path().join(crate::run_dir::TASK_FILE);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(mut card) = crate::kanban::parse_card(&text) else {
                continue;
            };
            let due = card
                .frontmatter
                .meta
                .get("scheduled_at")
                .and_then(|v| crate::kanban::parse_scheduled_at(v))
                .is_some_and(|at| at <= now);
            let Ok(run_id) = card.frontmatter.run_id.trim().parse::<i64>() else {
                continue;
            };
            if !due {
                continue;
            }
            card.frontmatter.meta.remove("scheduled_at");
            if std::fs::write(&path, crate::kanban::render_card(&card)).is_err() {
                continue;
            }
            eprintln!("[sai-engine] 予定の時刻になったので実行します: run_id={}", run_id);
            let root = crate::project::project_root();
            let spawned = std::process::Command::new(&exe)
                .args(["phone", "card", "run", &run_id.to_string()])
                .current_dir(&root)
                .env("SAI_PROJECT_ROOT", &root)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            match spawned {
                // 終わりを待つだけの糸を立てる（ゾンビを残さない）
                Ok(mut child) => {
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                Err(e) => eprintln!("[sai-engine] 予定の実行に失敗: run_id={}: {}", run_id, e),
            }
        }
    }

    /// `after_job:` を持つ todo のカードについて、ジョブが終わっていれば実行する。
    ///
    /// 状態の読み取りは ssh で数秒〜60 秒かかるため、カードごとに別の糸で行う。
    /// 読めなかったときはカードを変えず、次の見回りでやり直す。
    fn run_job_waiting_cards(&self) {
        let todo = crate::project::sai_path(crate::run_dir::OUTPUTS_DIR)
            .join(crate::run_dir::TODO_OWNER);
        let Ok(entries) = std::fs::read_dir(&todo) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path().join(crate::run_dir::TASK_FILE);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(card) = crate::kanban::parse_card(&text) else {
                continue;
            };
            let Some(job) = card.frontmatter.meta.get("after_job").map(|s| s.trim().to_string())
            else {
                continue;
            };
            let Ok(run_id) = card.frontmatter.run_id.trim().parse::<i64>() else {
                continue;
            };
            if !crate::kanban::valid_job_id(&job) {
                continue;
            }
            if !self.job_busy.lock().unwrap().insert(run_id) {
                continue;
            }
            let busy = Arc::clone(&self.job_busy);
            let errs = Arc::clone(&self.job_errs);
            std::thread::spawn(move || {
                let root = crate::project::project_root();
                match read_job_state(&root, &job) {
                    Ok(state) => {
                        errs.lock().unwrap().remove(&run_id);
                        if release_job_card(&path, &job, Some(&state)).is_some() {
                            eprintln!(
                                "[sai-engine] ジョブ {} が {} で終わったので実行します: run_id={}",
                                job, state, run_id
                            );
                            spawn_card_run(&root, run_id);
                        }
                    }
                    Err(e) => {
                        // 失敗の文が前と同じなら出さない
                        let mut map = errs.lock().unwrap();
                        if map.get(&run_id) != Some(&e) {
                            eprintln!(
                                "[sai-engine] ジョブ {} の状態を読めません: run_id={}: {}",
                                job, run_id, e
                            );
                            map.insert(run_id, e);
                        }
                    }
                }
                busy.lock().unwrap().remove(&run_id);
            });
        }
    }

    fn move_to_processed(
        &self,
        source: &Path,
        processed_dir: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let filename = source.file_name().unwrap_or_default();
        let dest = processed_dir.join(filename);
        if fs_rename_safe(source, &dest) {
            Ok(())
        } else {
            // 移動失敗時は削除を試みる
            let _ = std::fs::remove_file(source);
            Ok(())
        }
    }
}

impl Daemon {
    /// メインブロック実行を別スレッドで開始する。
    ///
    /// デーモンは実行レコードの作成・pane 分割・pi 起動・完了監視を
    /// `orchestrator::execute_block_pipeline` に委譲します（§3.1）。
    fn start_main_block(&self, req: &BlockRunRequest) {
        let block_path = PathBuf::from(&req.block_path);
        if !block_path.exists() {
            eprintln!(
                "[sai-engine] ブロック定義ファイルが見つかりません: {}",
                block_path.display()
            );
            return;
        }

        // ブロック定義を読む直前に準備コマンドを走らせる（作り直したテンプレートを
        // そのまま差し込むため）。失敗しても前回のファイルのまま実行を続ける。
        if let crate::pre_run::PreRunOutcome::Failed(msg) = crate::pre_run::run_if_configured() {
            eprintln!(
                "[sai-engine] 警告: 実行直前の準備コマンドに失敗しました（前回のファイルのまま続けます）: {msg}"
            );
        }

        let parsed = match parse_block_file(&block_path) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "[sai-engine] ブロック定義のパースに失敗: {}: {}",
                    block_path.display(),
                    e
                );
                return;
            }
        };

        // 実行時設定のオーバーライド（§6・P8: ExecutionOverrides をそのまま使用）
        let overrides = ExecutionOverrides {
            model: req.overrides.model.clone(),
            provider: req.overrides.provider.clone(),
            thinking: req.overrides.thinking.clone(),
            timeout: req.overrides.timeout,
            round: req.overrides.round,
            global_round: req.overrides.global_round,
            tag: req.overrides.tag.clone(),
            direction: req.overrides.direction,
            worktree: req.overrides.worktree.clone(),
            cwd: req.overrides.cwd.clone(),
            keep_pane: req.overrides.keep_pane,
            runner: req.overrides.runner,
        };

        // プレースホルダー（{{input.*}} / waggle トークン）の展開は
        // prepare_pipeline が解決するため、ここでは展開せず生の本文のまま渡す。
        // （事前展開すると {{input.source_doc}} が "waggle:xxx" のままで置換され、
        //   waggle トークンのファイル内容解決が行われないため）
        let body = parsed.body.clone();

        let store = Arc::clone(&self.store);
        let running_executions = Arc::clone(&self.running_executions);
        let user_inputs = req.user_inputs.clone();
        let block_name = req.block_name.clone();
        let card_path = req.card_path.clone();
        let reserved_run_id = req.reserved_run_id;

        std::thread::spawn(move || {
            let result = orchestrator::execute_block_pipeline_full(
                &store,
                &parsed,
                &block_path,
                body,
                user_inputs,
                overrides,
                None, // メイン: parent_run_id = None
                None, // メイン: batch_id = None（P3-A）
                None, // メイン: タブ未使用（U2・R4）
                None, // メイン: 分割基準ペインなし（既定の分割方式）
                None, // メイン: 分割済みペインなし（既定の分割方式）
                None, // メイン: タスクIDなし（U3・停止通知対象外）
                Some(&running_executions),
                card_path, // kanban: カード相対パス
                reserved_run_id, // outputs 統一：予約 run（R302）
            );

            match result {
                Ok(pr) => {
                    eprintln!(
                        "[sai-engine] ✅ メインブロック完了: block={}, run_id={:?}, elapsed_ms={}",
                        block_name, pr.execution_id, pr.elapsed_ms
                    );
                    // ⑤ pane 自動削除（sai-write 正常完了時のみ・§4.2 ケース1）
                    orchestrator::close_execution_pane(&pr.pane_id);
                }
                Err(e) => {
                    eprintln!(
                        "[sai-engine] ❌ メインブロック失敗: block={}, error={}",
                        block_name, e
                    );
                    // 以降はユーザー手動対応（pane は維持。DB は execute_block_pipeline が更新済み）
                }
            }
        });
    }

    // -----------------------------------------------------------------------
    // finalize_request 通知の処理（ユーザーによる明示的な完了要求・ケース5）
    // -----------------------------------------------------------------------

    /// `pending/` 内の `finalize_request` 通知を処理する。
    ///
    /// ユーザーが `/pi-sai-finalize` スラッシュコマンドを入力した際に pi-sai が
    /// この通知を書き込む。デーモンは対応する実行の監視スレッドに
    /// `FinalizeRequested` イベントを送信し、出力抽出・統計保存・DB更新を
    /// 行わせる。
    fn process_finalize_request(&self) -> Result<(), Box<dyn std::error::Error>> {
        let pending = self.notify_base.join("pending");
        let processed = self.notify_base.join("processed");
        let _ = std::fs::create_dir_all(&pending);
        let _ = std::fs::create_dir_all(&processed);

        let mut entries: Vec<_> = std::fs::read_dir(&pending)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let notification_path = entry.path();
            let content = match std::fs::read_to_string(&notification_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let value: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if value.get("type").and_then(|v| v.as_str()) != Some("finalize_request") {
                continue;
            }

            let run_id = value.get("run_id").and_then(|v| v.as_i64()).unwrap_or(0);
            if run_id == 0 {
                eprintln!("[sai-engine] finalize_request に有効な run_id がありません");
                let _ = self.move_to_processed(&notification_path, &processed);
                continue;
            }

            eprintln!("[sai-engine] finalize_request を処理: run_id={}", run_id);

            // 対応する実行の監視スレッドに FinalizeRequested イベントを送信
            let event = orchestrator::CompletionEvent::FinalizeRequested { run_id };
            let found = {
                let map = self.running_executions.lock().unwrap();
                match map.get(&run_id) {
                    Some(tx) => {
                        let _ = tx.send(event);
                        true
                    }
                    None => false,
                }
            };
            if found {
                // 正常系（マップ在り）は従来どおり。結果応答を残す（P2-1）。
                write_finalize_result(run_id, true, "accepted");
                // P1-4: 確定要求を受け付けたら状態ファイルを即時更新する（鮮度改善）。
                let _ = self.write_status_file();
            } else {
                // P0-1: DBフォールバック。TUIのCtrl+D経路（`request_main_block_finalize`
                // の③）と同順で直接確定する。デーモン再起動でマップが消えても幽霊を
                // `completed` + `finalized=1` にできる。
                match self.store.get_execution_by_id(run_id) {
                    Ok(exec) if exec.status == "running" => {
                        // P6-2: サブブロック（parent_run_id あり）は status だけ completed にし、
                        // finalized は付けない（子の finalized は親の task_manage finalize で付ける）。
                        let is_subtask = exec.parent_run_id.is_some();
                        let _ = self.store.update_execution_status(run_id, "completed");
                        if !is_subtask {
                            let _ = self.store.update_execution_finalized(run_id, true);
                        }
                        let _ = self.store.append_execution_error_note(
                            run_id,
                            "デーモン再起動後のフォールバック確定（finalize_request）",
                        );
                        // L1: フォールバック確定でも実行 usage を自動保存する（ベストエフォート）。
                        if let Err(e) = crate::usage::record_execution_usage(&self.store, run_id) {
                            eprintln!("[sai-engine] 費用の記録に失敗 (run_id={}): {}", run_id, e);
                        }
                        eprintln!(
                            "[sai-engine] finalize_request: run_id={} をフォールバック確定しました（completed{}）",
                            run_id,
                            if is_subtask { "・子は finalize 待ち" } else { " + finalized=1" }
                        );
                        write_finalize_result(run_id, true, "fallback_completed");
                    }
                    Ok(exec) => {
                        eprintln!(
                            "[sai-engine] finalize_request: run_id={} は既に確定済みです（status={}）。何もしません。",
                            run_id, exec.status
                        );
                        write_finalize_result(
                            run_id,
                            false,
                            &format!("already_finished:{}", exec.status),
                        );
                    }
                    Err(_) => {
                        eprintln!(
                            "[sai-engine] finalize_request: run_id={} に対応する実行が見つかりません。既に完了した可能性があります。",
                            run_id
                        );
                        write_finalize_result(run_id, false, "not_found");
                    }
                }
                // P1-4: フォールバック確定直後も即時更新する。
                let _ = self.write_status_file();
            }

            // 処理済みに移動
            let _ = self.move_to_processed(&notification_path, &processed);
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // ブロック定義のリロード（§7.1.1・Phase 6）
    // -----------------------------------------------------------------------

    /// `.sai/blocks/*.md` の mtime を監視し、変更があれば再パースして
    /// `block_metadata` に反映する（自動リロード）。
    fn reload_block_definitions_if_changed(&self) -> Result<(), Box<dyn std::error::Error>> {
        let blocks_dir = crate::project::sai_path(".sai/blocks");
        if !blocks_dir.is_dir() {
            return Ok(());
        }

        let mut cache = self.block_def_cache.lock().unwrap();

        for entry in std::fs::read_dir(&blocks_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                let meta = match std::fs::metadata(&path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let key = (meta.modified().unwrap_or(UNIX_EPOCH), meta.len());
                if cache.get(&path) != Some(&key) {
                    // 変更 or 新規 → 再パースして block_metadata を更新
                    if let Ok(parsed) = parse_block_file(&path) {
                        match orchestrator::upsert_block_metadata(
                            &self.store,
                            &parsed.config,
                            &path,
                        ) {
                            Ok(block_id) => {
                                eprintln!(
                                    "[sai-engine] ブロック定義をリロード: {} (block_id={})",
                                    path.display(),
                                    block_id
                                );
                            }
                            Err(e) => {
                                eprintln!(
                                    "[sai-engine] ブロック定義のリロードに失敗: {}: {}",
                                    path.display(),
                                    e
                                );
                            }
                        }
                    }
                    cache.insert(path, key);
                }
            }
        }

        Ok(())
    }

    /// `daemon reload` コマンドのフラグファイルを処理する（手動リロード）。
    fn check_reload_flag(&self) -> Result<(), Box<dyn std::error::Error>> {
        let flag_path = crate::project::sai_path(".sai/sai-engine/reload.flag");
        if flag_path.exists() {
            let _ = std::fs::remove_file(&flag_path);
            eprintln!("[sai-engine] 手動リロード要求を受け付けました");
            // キャッシュをクリアして次回スキャンで全ファイルを再読込する
            self.block_def_cache.lock().unwrap().clear();
            let _ = self.reload_block_definitions_if_changed();
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // ステータスファイル（daemon status 表示用）
    // -----------------------------------------------------------------------

    /// デーモンのステータスを `.sai/sai-engine/daemon.status` に書き込む。
    ///
    /// `orphaned_runs`（P1-1）は起動時スキャンの結果を保持し、復旧されるまで
    /// 載せ続ける。後方互換のため既存キー名・型は変えず追加のみとする。
    fn write_status_file(&self) -> Result<(), Box<dyn std::error::Error>> {
        let status_path = crate::project::sai_path(".sai/sai-engine/daemon.status");
        if let Some(parent) = status_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let pool = self.agent_pool.status();
        let running_run_ids: Vec<i64> = self
            .running_executions
            .lock()
            .unwrap()
            .keys()
            .copied()
            .collect();
        // 幽霊一覧はDB照会を正とする（停止中のTUI表示と一致させる）。
        // 生きている実行（running_executions）と重なるIDは除外する。
        let orphaned_runs: Vec<OrphanedRun> = list_orphaned_runs(&self.store)
            .into_iter()
            .filter(|o| !running_run_ids.contains(&o.run_id))
            .collect();

        let status = serde_json::json!({
            "pid": std::process::id(),
            "started_at": unix_millis(),
            "active_tasks": pool.active_task_ids,
            "queued_tasks": pool.queued_task_ids,
            "running_executions": running_run_ids,
            "orphaned_runs": orphaned_runs,
            "last_heartbeat": unix_millis(),
        });

        std::fs::write(&status_path, serde_json::to_string_pretty(&status)?)?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // phone 書き出し（実装計画 01 §2.3・status ファイルと同じ周期）
    // -----------------------------------------------------------------------

    /// 変わった実行だけ `~/.sai-moon/phone/runs/<project_name>/<run_id>.json` へ書き出す。
    ///
    /// - `todo` / `pending` と runner=`user` は対象外（`phone_export::should_export`）
    /// - `status` / `finished_at` / `session_stats_json` が変わった実行だけ書き直す
    /// - 終了状態になった実行は **次の周期でもう一度** 書き直す
    ///   （終了直後は `session_stats_json` や transcript の最終行が遅れて入ることがあるため）
    /// - 失敗はログのみ（sai-phone 連携はおまけ機能・エージェント実行を止めない）
    ///
    /// `initial = true` はデーモン起動時（全件を書き出し、シグネチャ表を初期化する）。
    fn export_phone_runs(&self, initial: bool) {
        let execs = match self.store.list_all_executions() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[sai-engine] phone 書き出し: 実行一覧の取得に失敗: {}", e);
                return;
            }
        };
        let project_root = crate::project::project_root();
        let mut state = self.phone_export.lock().unwrap();
        if state.exporter.is_none() {
            match phone_export::Exporter::new(&project_root) {
                Ok(e) => state.exporter = Some(e),
                Err(e) => {
                    // HOME 未設定など。ログのみでエージェント実行は止めない。
                    if initial {
                        eprintln!("[sai-engine] phone 書き出しを開始できません: {}", e);
                    }
                    return;
                }
            }
        }
        // 借用衝突を避けるため書き出し器を一時的に取り出す（ループ後に戻す）
        let exporter = state.exporter.take().expect("直上で確保済み");

        let (mut written, mut skipped, mut failed) = (0usize, 0usize, 0usize);
        for exec in &execs {
            let Some(run_id) = exec.id else { continue };
            if !phone_export::should_export(&exec.status, &exec.runner) {
                continue;
            }
            let sig = phone_export::ExportSig::of(exec);
            let prev = state.sigs.get(&run_id);
            let changed = prev != Some(&sig);
            // 終了状態へ変わった（= changed）実行は次の周期にもう一度書き直す
            // （終了直後は stats / transcript が遅れて入るための 1 回限りの再書込）
            if changed && phone_export::is_finished(&exec.status) {
                state.rewrite_once.insert(run_id);
            }
            // 実行中は毎周期書き直す（開始直後は cc の session_id・pi の実セッション
            // ファイル・pane_id がまだ無く，後から揃うため。同じ内容なら書き込みは省略される）
            if !changed && exec.status != "running" && !state.rewrite_once.remove(&run_id) {
                // 前回から何も変わっていない（かつ再書き出し予定もない）→ 省略
                continue;
            }
            match exporter.export(&self.store, exec) {
                Ok(Some(true)) => written += 1,
                Ok(Some(false)) => skipped += 1,
                Ok(None) => {}
                Err(e) => {
                    failed += 1;
                    eprintln!("[sai-engine] phone 書き出し失敗: run_id={}: {}", run_id, e);
                    continue;
                }
            }
            state.sigs.insert(run_id, sig);
        }
        state.exporter = Some(exporter);
        if initial || written > 0 || failed > 0 {
            eprintln!(
                "[sai-engine] phone 書き出し{}: {} 件書く / {} 件省略 / {} 件失敗",
                if initial { "（起動時・全件）" } else { "" },
                written,
                skipped,
                failed
            );
        }
    }

    /// 停止要求（SIGTERM 受信・P0-2）を受けて確定終了する。
    ///
    /// 1. 新規受付を止める（呼び出し元がループを抜ける）
    /// 2. `running_executions` の全実行へ `DaemonShutdown` を送る
    /// 3. 状態ファイルを即時更新（P1-4）してから終了する
    /// 4. 各実行スレッドは残留出力で完了処理し、無理なら `failed` 確定する
    ///    （`orchestrator::CompletionEvent::DaemonShutdown` の分岐）
    /// 5. それでも残る `running` は `failed` + 理由付きで確定する（取りこぼし防止）
    fn shutdown_gracefully(&self) {
        eprintln!("[sai-engine] 停止要求を受けました。実行中の確定処理を開始します");
        let run_ids: Vec<i64> = self
            .running_executions
            .lock()
            .unwrap()
            .keys()
            .copied()
            .collect();
        for run_id in &run_ids {
            let event = orchestrator::CompletionEvent::DaemonShutdown { run_id: *run_id };
            let sent = self
                .running_executions
                .lock()
                .unwrap()
                .get(run_id)
                .map(|tx| tx.send(event).is_ok())
                .unwrap_or(false);
            if !sent {
                eprintln!(
                    "[sai-engine] 停止時: run_id={} への切断通知に失敗しました",
                    run_id
                );
            }
        }
        // 実行スレッドの確定を最大5秒待つ（待てない分は下の取りこぼし確定で拾う）。
        for _ in 0..50 {
            let remaining = self
                .running_executions
                .lock()
                .unwrap()
                .keys()
                .copied()
                .collect::<Vec<_>>();
            if remaining.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // 取りこぼし: まだ `running` の実行は `failed` + 理由付きで確定する。
        for orphan in list_orphaned_runs(&self.store) {
            // 生きている実行スレッドが残っていればその確定に任せる。
            if self
                .running_executions
                .lock()
                .unwrap()
                .contains_key(&orphan.run_id)
            {
                continue;
            }
            let _ = self
                .store
                .update_execution_error(orphan.run_id, "デーモン停止により中断", 1);
            eprintln!(
                "[sai-engine] 停止時確定: run_id={}（{}）を failed にしました",
                orphan.run_id, orphan.block_name
            );
        }
        let _ = self.write_status_file();
        eprintln!("[sai-engine] 停止時確定処理が完了しました");
    }
}

// ============================================================================
// エントリポイント（daemon _run から呼ばれる）
// ============================================================================

/// デーモンをフォアグラウンドで実行する（`sai-engine daemon _run`）。
///
/// `daemon start` がバックグラウンド起動する子プロセスの本体です。
/// プロジェクトルート（起動パス）を `SAI_PROJECT_ROOT` に設定し、
/// cwd をプロジェクトルートに固定してからポーリングループを開始します。
///
/// 起動時スキャン（P1-1）: `status='running'` の取り残し（幽霊）を検出し
/// `daemon.log` に記録する。自動修復はしない（`daemon recover` で明示確定）。
/// シグナル（P0-2）: SIGTERM/SIGINT を捕捉し、確定終了してから抜ける。
pub fn run_foreground(project_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // 環境変数と cwd をプロジェクトルートに固定（D15・§7.1.7）
    std::env::set_var("SAI_PROJECT_ROOT", project_root);
    std::env::set_current_dir(project_root)?;

    let db_path = crate::project::sai_path(".sai/sai-engine/sai-engine.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let store = Arc::new(SaiStore::open(&db_path)?);
    let daemon = Daemon::new(Arc::clone(&store));

    // P1-1: 起動時の幽霊スキャン（自動修復なし・記録のみ）
    let orphans = list_orphaned_runs(&store);
    if orphans.is_empty() {
        eprintln!("[sai-engine] 起動時スキャン: 取り残しはありません");
    } else {
        eprintln!(
            "[sai-engine] 起動時スキャン: 取り残し {} 件（自動修復しません。`sai-link daemon recover` を参照）",
            orphans.len()
        );
        for o in &orphans {
            eprintln!(
                "[sai-engine] 取り残し: run {}（{}・開始 {}・理由 {}）",
                o.run_id, o.block_name, o.started_at, o.reason
            );
        }
    }
    // 起動直後の `daemon status` に幽霊を載せる（P1-4 即時書込）。
    let _ = daemon.write_status_file();

    // 実装計画 01 §2.3: 起動時にそのプロジェクトの過去の全実行を書き出す
    // （既に同じ内容のファイルがあれば書き直さないため、2 回目以降の起動は軽い）。
    daemon.export_phone_runs(true);

    // P0-2: SIGTERM/SIGINT 捕捉（停止時の確定終了用）。
    // `libc::signal` でフラグのみ立て、ポーリングループ側で安全に確定処理する
    // （ハンドラ内では確保・入出力を行わない）。
    install_shutdown_handler();

    eprintln!(
        "🚀 sai デーモン起動 (project root: {})",
        project_root.display()
    );

    daemon.loop_forever()
}

/// 停止要求フラグ（SIGTERM/SIGINT 受信用・P0-2）。
///
/// シグナルハンドラは非同期シグナル安全な操作（フラグ設定）のみ行う。
/// ポーリングループが各 tick で確認し、確定終了へ進む。
static SHUTDOWN_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// SIGTERM/SIGINT で停止フラグを立てるハンドラを登録する（P0-2）。
///
/// 登録失敗時は警告に留める（従来どおり即時終了になる）。
fn install_shutdown_handler() {
    // SAFETY: ハンドラは `AtomicBool::store`（ロックフリー・非同期シグナル安全）のみ行う。
    unsafe extern "C" fn on_term(_sig: libc::c_int) {
        SHUTDOWN_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    // SAFETY: 関数ポインタは有効で、`signal` の戻り値は無視する（失敗時は従来動作）。
    unsafe {
        libc::signal(libc::SIGTERM, on_term as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_term as libc::sighandler_t);
    }
}

/// 停止要求（SIGTERM/SIGINT）があるか返す。
fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(std::sync::atomic::Ordering::SeqCst)
}

// ============================================================================
// block_run_request の書き込み（CLI / 対話モードから呼ばれる）
// ============================================================================

/// `block_run_request` 通知を `pending/` に書き込む。
///
/// メインブロックの実行要求（§5.2）。`sai block run` CLI（kanban 統一後）が使用します。
/// デーモンが検出して実行します。
pub fn write_block_run_request(
    block_path: &Path,
    block_name: &str,
    user_inputs: &HashMap<String, String>,
    overrides: &ExecutionOverrides,
) -> Result<(), Box<dyn std::error::Error>> {
    write_block_run_request_with_card(block_path, block_name, user_inputs, overrides, None)
}

/// カードパス＋予約 run_id 付きで `block_run_request` 通知を書き込む（kanban 用）。
///
/// `card_path` はプロジェクト相対パス。`reserved_run_id` は outputs 統一の
/// 予約 run（`Shift+C` 時に発行済み）。`None` なら従来どおり新規発行する。
pub fn write_block_run_request_with_card(
    block_path: &Path,
    block_name: &str,
    user_inputs: &HashMap<String, String>,
    overrides: &ExecutionOverrides,
    card_path: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    write_block_run_request_full(
        block_path,
        block_name,
        user_inputs,
        overrides,
        card_path,
        None,
    )
}

/// `block_run_request` 通知の完全版（予約 run_id 付き・R302）。
pub fn write_block_run_request_full(
    block_path: &Path,
    block_name: &str,
    user_inputs: &HashMap<String, String>,
    overrides: &ExecutionOverrides,
    card_path: Option<&str>,
    reserved_run_id: Option<i64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let pending = crate::project::sai_path(".sai/sai-engine/notifications/pending");
    std::fs::create_dir_all(&pending)?;

    let req = BlockRunRequest {
        notification_type: "block_run_request".to_string(),
        timestamp: crate::common::iso_8601_timestamp(),
        block_path: block_path.to_string_lossy().to_string(),
        block_name: block_name.to_string(),
        user_inputs: user_inputs.clone(),
        overrides: overrides.clone(),
        card_path: card_path.map(|s| s.to_string()),
        reserved_run_id,
    };

    let filename = format!("{}-block_run_request.json", unix_millis());
    let path = pending.join(filename);
    std::fs::write(&path, serde_json::to_string_pretty(&req)?)?;
    eprintln!(
        "[sai-engine] block_run_request を書き込みました: {}",
        path.display()
    );
    Ok(())
}

/// メインブロックのユーザー明示完了（finalize）要求を `pending/` に書き込む（U4）。
///
/// TUI の Ctrl+D 2 回目（および pi 上の `/pi-sai-finalize`）から呼ばれ、
/// デーモンの `process_finalize_request` が対応する実行監視スレッドへ
/// `FinalizeRequested` イベントを送信します。
/// これにより `execute_block_pipeline` の正常完了（Ok）パスの DB 更新順序
/// （`update_execution_output_content` → `update_execution_status(completed)` →
/// waggle トークン → Session 情報）が再利用され、`sai block list` で `完了` 表示になります。
pub fn write_finalize_request(run_id: i64) -> Result<(), Box<dyn std::error::Error>> {
    let pending = crate::project::sai_path(".sai/sai-engine/notifications/pending");
    std::fs::create_dir_all(&pending)?;

    let body = serde_json::json!({
        "type": "finalize_request",
        "run_id": run_id,
        "timestamp": crate::common::iso_8601_timestamp(),
    });

    let filename = format!("{}-finalize_request.json", unix_millis());
    let path = pending.join(filename);
    std::fs::write(&path, serde_json::to_string_pretty(&body)?)?;
    eprintln!(
        "[sai-engine] finalize_request を書き込みました: {}",
        path.display()
    );
    Ok(())
}

// ============================================================================
// PID ファイル管理
// ============================================================================

/// PID ファイルのパスを取得する。
pub fn pid_file_path() -> PathBuf {
    crate::project::sai_path(".sai/sai-engine/daemon.pid")
}

/// PID ファイルからデーモンの PID を読み取る。
pub fn read_pid() -> Option<i32> {
    read_pid_at(&pid_file_path())
}

/// PID を PID ファイルに書き込む。
pub fn write_pid(pid: i32) -> Result<(), Box<dyn std::error::Error>> {
    write_pid_at(&pid_file_path(), pid)
}

/// PID ファイルを削除する（ベストエフォート）。
pub fn remove_pid_file() -> Result<(), Box<dyn std::error::Error>> {
    remove_pid_at(&pid_file_path())
}

/// 指定パスの PID ファイルから PID を読み取る（テスト用）。
fn read_pid_at(path: &Path) -> Option<i32> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
}

/// 指定パスの PID ファイルに PID を書き込む（テスト用）。
fn write_pid_at(path: &Path, pid: i32) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", pid))?;
    Ok(())
}

/// 指定パスの PID ファイルを削除する（テスト用）。
fn remove_pid_at(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// プロセスが生存しているか確認する（`kill -0`）。
pub fn process_alive(pid: i32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 指定 PID が sai デーモンのプロセスであるか確認する（2.4 の修正・P1-3）。
///
/// PID 再利用による誤判定（kill -9 後に無関係プロセスへ PID が再利用されるケース）を
/// 防ぐため、`/proc/<pid>/cmdline` に `daemon` が含まれるかを確認する。
/// `/proc` が無い環境（macOS 等）では `ps -p <pid> -o command=` の出力で判定し、
/// `ps` 実行失敗時のみ `kill -0` の結果にフォールバックする。
fn is_daemon_process(pid: i32) -> bool {
    let cmdline_path = format!("/proc/{}/cmdline", pid);
    if let Ok(content) = std::fs::read(&cmdline_path) {
        let cmdline = String::from_utf8_lossy(&content).replace('\0', " ");
        // 例: ".../sai-engine daemon _run"
        return cmdline.contains("daemon");
    }
    // P1-3: /proc が無い環境（macOS 等）は `ps` でコマンド行を確認する。
    if let Ok(output) = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
    {
        if output.status.success() {
            let cmd = String::from_utf8_lossy(&output.stdout);
            return cmd.contains("daemon");
        }
    }
    // `ps` 実行失敗時は kill -0 の結果にフォールバック
    process_alive(pid)
}

/// デーモンが稼働中かどうかを返す（PID ファイル + プロセス生存確認）。
pub fn is_running() -> bool {
    match read_pid() {
        Some(pid) => process_alive(pid) && is_daemon_process(pid),
        None => false,
    }
}

// ============================================================================
// 内部ヘルパー
// ============================================================================
// 内部ヘルパー
// ============================================================================

/// UNIX エポックからのミリ秒を返す。
fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// リネームを安全に実行する（Windows では上書き不可のため削除してから）。
fn fs_rename_safe(source: &Path, dest: &Path) -> bool {
    if dest.exists() {
        let _ = std::fs::remove_file(dest);
    }
    std::fs::rename(source, dest).is_ok()
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::SplitDirection;

    #[test]
    fn parse_job_state_reads_state() {
        assert_eq!(parse_job_state(r#"{"state": "done"}"#).as_deref(), Some("done"));
        assert_eq!(parse_job_state("{壊れ"), None);
        assert_eq!(parse_job_state(r#"{"x": 1}"#), None);
        for s in ["done", "failed", "cancelled", "lost"] {
            assert!(job_state_finished(s), "{s}");
        }
        for s in ["queued", "running"] {
            assert!(!job_state_finished(s), "{s}");
        }
    }

    /// 偽の命令（`SAI_EXP_STATUS_CMD`）が出す状態で、カードを起こす対象に選ぶかが決まる。
    #[test]
    fn job_card_is_released_only_when_job_finished() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let task = root.join("task.md");
        let mut card = crate::kanban::KanbanCard {
            frontmatter: crate::kanban::CardFrontmatter::new_card("2026/10/02 10:00:00".into()),
            body: "# task_text\nやること\n".into(),
        };
        card.frontmatter.run_id = "42".into();
        card.frontmatter.set_meta("after_job".into(), "j1".into());
        let write = |c: &crate::kanban::KanbanCard| {
            std::fs::write(&task, crate::kanban::render_card(c)).unwrap()
        };
        write(&card);

        let fake = |state: &str| {
            let exe = root.join("fake.sh");
            std::fs::write(&exe, format!("#!/bin/sh\necho '{{\"state\":\"{}\"}}'\n", state)).unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::env::set_var("SAI_EXP_STATUS_CMD", &exe);
            read_job_state(root, "j1")
        };
        let running = fake("running").unwrap();
        assert_eq!(release_job_card(&task, "j1", Some(&running)), None);
        assert!(std::fs::read_to_string(&task).unwrap().contains("after_job: j1"));

        // 別の値に変わっていたら何もしない
        let done = fake("done").unwrap();
        assert_eq!(release_job_card(&task, "other", Some(&done)), None);
        assert_eq!(release_job_card(&task, "j1", None), None);

        assert_eq!(release_job_card(&task, "j1", Some(&done)), Some(42));
        assert!(!std::fs::read_to_string(&task).unwrap().contains("after_job"));
        std::env::remove_var("SAI_EXP_STATUS_CMD");
    }

    // ---- PID ファイル管理（実デーモンの PID ファイルに影響しないよう一時パスで実行）----

    #[test]
    fn test_pid_file_write_read_remove() {
        // 一時ディレクトリで検証（実環境の .sai/sai-engine/daemon.pid を汚染しない）
        let temp_dir =
            std::env::temp_dir().join(format!("sai-engine-pid-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let path = temp_dir.join("daemon.pid");

        let pid = 12345;
        write_pid_at(&path, pid).expect("PID ファイル書き込みに失敗");

        assert_eq!(read_pid_at(&path), Some(pid));

        remove_pid_at(&path).expect("PID ファイル削除に失敗");
        assert_eq!(read_pid_at(&path), None);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_process_alive_current() {
        // 自分自身の PID は常に生存
        let pid = std::process::id() as i32;
        assert!(process_alive(pid));
    }

    #[test]
    fn test_process_alive_nonexistent() {
        // 存在しない PID（通常は大きな値）は生存しない
        assert!(!process_alive(999_999));
    }

    // ---- P1-1: 幽霊一覧の組み立て ----

    #[test]
    fn test_list_orphaned_runs_empty() {
        // `running` が無ければ空（既存表示不変）
        let store = SaiStore::open_in_memory().expect("インメモリDBを開く");
        assert!(list_orphaned_runs(&store).is_empty());
    }

    #[test]
    fn test_list_orphaned_runs_collects_running() {
        let store = SaiStore::open_in_memory().expect("インメモリDBを開く");
        let meta = crate::db::models::BlockMetadata::new(
            "ghost-block".to_string(),
            "幽霊テスト".to_string(),
            "inherit".to_string(),
            None,
            "off".to_string(),
            "false".to_string(),
            "false".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            "ghost.md".to_string(),
            None
        );
        let block_id = store.insert_block(&meta).expect("ブロック登録");
        let run_id = store.create_execution(block_id, None).expect("実行作成");
        store
            .update_execution_status(run_id, "running")
            .expect("running化");
        let orphans = list_orphaned_runs(&store);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].run_id, run_id);
        assert_eq!(orphans[0].block_name, "ghost-block");
        assert_eq!(orphans[0].reason, "daemon_restart");
        // 完了後は幽霊から消える
        store
            .update_execution_status(run_id, "completed")
            .expect("完了化");
        assert!(list_orphaned_runs(&store).is_empty());
    }

    // ---- block_run_request のシリアライズ/デシリアライズ ----

    #[test]
    fn test_block_run_request_roundtrip() {
        let mut user_inputs = HashMap::new();
        user_inputs.insert("input".to_string(), "テスト入力".to_string());

        let req = BlockRunRequest {
            notification_type: "block_run_request".to_string(),
            timestamp: "2026-08-02T00:00:00Z".to_string(),
            block_path: ".sai/blocks/test_task_main.md".to_string(),
            block_name: "test_task_main".to_string(),
            user_inputs,
            overrides: ExecutionOverrides {
                model: Some("anthropic/claude-sonnet-4-5".to_string()),
                provider: None,
                thinking: Some("high".to_string()),
                timeout: Some(900),
                round: Some(1),
                global_round: Some(2),
                tag: Some("test".to_string()),
                worktree: None,
                cwd: None,
                direction: Some(SplitDirection::Right),
                keep_pane: true,
                runner: crate::runner::RunnerKind::Cc,
            },
            card_path: Some(".sai/todo/card-20260905-000000-0001.md".to_string()),
            reserved_run_id: Some(7),
        };

        let json = serde_json::to_string_pretty(&req).unwrap();
        let parsed: BlockRunRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.notification_type, "block_run_request");
        assert_eq!(parsed.block_name, "test_task_main");
        assert_eq!(
            parsed.user_inputs.get("input").map(|s| s.as_str()),
            Some("テスト入力")
        );
        assert_eq!(
            parsed.overrides.model.as_deref(),
            Some("anthropic/claude-sonnet-4-5")
        );
        assert_eq!(parsed.overrides.timeout, Some(900));
        assert_eq!(parsed.overrides.round, Some(1));
        assert_eq!(parsed.overrides.global_round, Some(2));
        assert_eq!(parsed.overrides.direction, Some(SplitDirection::Right));
        assert!(parsed.overrides.keep_pane);
        assert_eq!(
            parsed.card_path.as_deref(),
            Some(".sai/todo/card-20260905-000000-0001.md")
        );
    }

    #[test]
    fn test_block_run_request_old_json_without_card_path() {
        // 旧通知（card_path・reserved_run_id 無し）も読める（前方互換・R121/R302）
        let json = r#"{"type": "block_run_request", "block_path": "b.md", "block_name": "b"}"#;
        let parsed: BlockRunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.card_path, None);
        assert_eq!(parsed.reserved_run_id, None);
    }

    #[test]
    fn test_block_run_request_minimal() {
        // overrides なし（デフォルト）でもパースできる
        let json = r#"{
            "type": "block_run_request",
            "timestamp": "2026-08-02T00:00:00Z",
            "block_path": ".sai/blocks/test_task_main.md",
            "block_name": "test_task_main",
            "user_inputs": {}
        }"#;

        let parsed: BlockRunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.block_name, "test_task_main");
        assert!(parsed.user_inputs.is_empty());
        assert_eq!(parsed.overrides.timeout, None);
        assert_eq!(parsed.overrides.direction, None);
    }

    #[test]
    fn test_block_run_request_legacy_timeout_ms_alias() {
        // 旧形式（timeout_ms: ミリ秒）の pending 通知も受容する（移行措置・P8）
        let json = r#"{
            "type": "block_run_request",
            "timestamp": "2026-08-02T00:00:00Z",
            "block_path": ".sai/blocks/test.md",
            "block_name": "test",
            "user_inputs": {},
            "overrides": {
                "timeout_ms": 900000
            }
        }"#;

        let parsed: BlockRunRequest = serde_json::from_str(json).unwrap();
        // alias により timeout フィールドとして解釈される（単位の違いは移行措置として許容）
        assert_eq!(parsed.overrides.timeout, Some(900000));
    }

    /// P6-2: finalize_request の DB フォールバック（running_executions に無いとき）で、
    /// メインブロックは completed + finalized=1 にし、サブブロックは finalized を付けない。
    #[test]
    fn test_finalize_fallback_does_not_finalize_subtask() {
        // SAI_PROJECT_ROOT と通知ディレクトリを触るため直列化する。
        let _env = crate::common::SAI_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "sai-link-daemon-finalize-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &dir);

        let store = Arc::new(crate::common::open_db().unwrap());
        let block = crate::db::models::test_helpers::create_test_block("daemon-fallback-test");
        let block_id = store.insert_block(&block).unwrap();

        // 子（サブブロック）実行とメイン実行を running にしておく
        let child_id = store
            .create_execution_for_subtask(block_id, None, 1, None)
            .unwrap();
        store.update_execution_status(child_id, "running").unwrap();
        let main_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(main_id, "running").unwrap();

        // running_executions に登録しない = デーモン再起動後のフォールバック経路
        let daemon = Daemon::new(Arc::clone(&store));

        // 通知ファイル名はミリ秒精度なので、子→メインの順で 1 件ずつ処理する
        write_finalize_request(child_id).unwrap();
        daemon.process_finalize_request().unwrap();

        let child = store.get_execution_by_id(child_id).unwrap();
        assert_eq!(child.status, "completed");
        assert!(!child.finalized, "子には finalized=1 を付けない");

        write_finalize_request(main_id).unwrap();
        daemon.process_finalize_request().unwrap();

        let main = store.get_execution_by_id(main_id).unwrap();
        assert_eq!(main.status, "completed");
        assert!(main.finalized, "メインは finalized=1 を付ける");

        std::env::remove_var("SAI_PROJECT_ROOT");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
