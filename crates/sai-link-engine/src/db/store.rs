//! データベース操作（CRUD）の実装
//!
//! `SaiStore` 構造体を定義し、各テーブルに対する CRUD 操作を提供します。
//! waggle の `SqliteStore` パターンに倣い、`Mutex<Connection>` による
//! スレッドセーフなアクセスとトランザクション内での書き込みを実装します。
//!
//! # 同時実行性
//!
//! SQLite WAL モードでは複数コネクションの同時読み取りが可能です。
//! `SaiStore` は内部で `Mutex` により書き込みを直列化し、
//! waggle の `SqliteStore` とは別の接続として共存します。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, Row, TransactionBehavior};

use super::models::{
    BlockConnection, BlockInput, BlockMetadata, BlockOutput, ConnectionGroup, DbError,
    DeleteExecutionSummary, ExecutionHistory, ExecutionUsageRow, GitCommit, RunBatch,
    RunBatchStatus, Task, WaggleTokenEntry,
};
use super::schema::init_sai_tables;

// ---------------------------------------------------------------------------
// execution_history の SELECT 共通化（P13・4.1.6 の修正）
// ---------------------------------------------------------------------------

/// `execution_history` の全カラムを返す SELECT 列句。
///
/// カラム追加時はここ + `execution_from_row` のマッピングのみ修正すればよく、
/// 4 つのクエリメソッドすべてに反映される。
const EXECUTION_SELECT_COLS: &str = "id, block_id, status, user_inputs_json, resolved_prompt, \
     input_waggle_tokens, output_waggle_token, output_waggle_tokens, output_type, \
     git_commit_before, git_commit_after, \
     herdr_pane_id, pid, started_at, finished_at, \
     duration_ms, exit_code, error_message, output_content, session_stats_json, \
     tag, round, model, provider, thinking, timeout, session_dir, pi_command, \
     parent_run_id, created_at, global_round, batch_id, finalized, \
     origin_run_id, kanban_card_path, execution_mode, workflow_moved, review_done, runner, agent_session_id";

/// `execution_history` の行を `ExecutionHistory` に変換する共通マッパー（P13）。
fn execution_from_row(row: &Row) -> rusqlite::Result<ExecutionHistory> {
    Ok(ExecutionHistory {
        id: Some(row.get(0)?),
        block_id: row.get(1)?,
        status: row.get(2)?,
        user_inputs_json: row.get(3)?,
        resolved_prompt: row.get(4)?,
        input_waggle_tokens: row.get(5)?,
        output_waggle_token: row.get(6)?,
        output_waggle_tokens: row.get(7)?,
        output_type: row
            .get::<_, String>(8)
            .unwrap_or_else(|_| "document".to_string()),
        git_commit_before: row.get(9)?,
        git_commit_after: row.get(10)?,
        herdr_pane_id: row.get(11)?,
        pid: row.get(12)?,
        started_at: row.get(13)?,
        finished_at: row.get(14)?,
        duration_ms: row.get(15)?,
        exit_code: row.get(16)?,
        error_message: row.get(17)?,
        output_content: row.get(18)?,
        session_stats_json: row.get(19)?,
        tag: row.get(20)?,
        round: row.get(21)?,
        model: row.get(22)?,
        provider: row.get(23)?,
        thinking: row.get(24)?,
        timeout: row.get(25)?,
        session_dir: row.get(26)?,
        pi_command: row.get(27)?,
        parent_run_id: row.get(28)?,
        created_at: row.get(29)?,
        global_round: row.get(30)?,
        batch_id: row.get(31)?,
        finalized: row.get::<_, i64>(32).unwrap_or(0) != 0,
        kanban_card_path: row.get(34).ok(),
        execution_mode: row.get(35).ok(),
        // 旧 DB（カラムなし）は 0（Kanban 所有）扱い
        workflow_moved: row.get::<_, i64>(36).unwrap_or(0) != 0,
        // 旧 DB（カラムなし）は 0（review レーン）扱い
        review_done: row.get::<_, i64>(37).unwrap_or(0) != 0,
        // 旧 DB（カラムなし）は `pi` 扱い
        runner: row.get::<_, String>(38).unwrap_or_else(|_| "pi".to_string()),
        // エージェント側セッション ID（cc のみ。旧 DB は NULL）
        agent_session_id: row.get(39).ok(),
    })
}

// ---------------------------------------------------------------------------
// tasks の SELECT 共通化（P1・タスクワークフロー拡張）
// ---------------------------------------------------------------------------

/// `tasks` の全カラムを返す SELECT 列句。
///
/// カラム追加時はここ + `task_from_row` のマッピングのみ修正すればよく、
/// タスク系の全クエリメソッドに反映される。
const TASK_SELECT_COLS: &str = "id, parent_run_id, subject, description, status, \
     block_type, inputs_json, depends_on_json, worktree, \
     execution_id, batch_id, client_id, round, supersedes_batch_id, \
     approval_comment, finalized, deleted_by, deleted_at, run_batch_id, \
     cwd, created_at, updated_at, timeout, card_run_id";

/// `tasks` の行を `Task` に変換する共通マッパー。
fn task_from_row(row: &Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: Some(row.get(0)?),
        parent_run_id: row.get(1)?,
        subject: row.get(2)?,
        description: row.get(3)?,
        status: row.get(4)?,
        block_type: row.get(5)?,
        inputs_json: row.get(6)?,
        depends_on_json: row.get(7)?,
        worktree: row
            .get::<_, String>(8)
            .unwrap_or_else(|_| "none".to_string()),
        execution_id: row.get(9)?,
        batch_id: row.get(10)?,
        client_id: row.get(11)?,
        round: row.get::<_, i64>(12).unwrap_or(1),
        supersedes_batch_id: row.get(13)?,
        approval_comment: row.get(14)?,
        finalized: row.get::<_, i64>(15).unwrap_or(0) != 0,
        deleted_by: row.get(16)?,
        deleted_at: row.get(17)?,
        run_batch_id: row.get(18)?,
        cwd: row.get(19)?,
        created_at: row.get(20)?,
        updated_at: row.get(21)?,
        timeout: row.get(22)?,
        // card_run_id は既存 DB で NULL の行があるため Option として読む
        card_run_id: row.get(23)?,
    })
}

/// `depends_on_json`（client_id の JSON 配列）をパースするヘルパー。
/// パース不能な場合は空配列を返す。
pub fn parse_depends_on(depends_on_json: Option<&str>) -> Vec<String> {
    match depends_on_json {
        Some(json) => serde_json::from_str::<Vec<String>>(json).unwrap_or_default(),
        None => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Project 統計集計（R16 / R17 / R19 / R20 / R21）
// ---------------------------------------------------------------------------

/// カテゴリ（tool / message / think）ごとのトークン使用量サマリ。
#[derive(Debug, Clone, Default)]
pub struct CategoryTokenSummary {
    /// 入力トークン数
    pub input: u64,
    /// 出力トークン数
    pub output: u64,
    /// キャッシュトークン数（cache 取得不能時は 0）
    pub cache: u64,
}

/// tool / message / think カテゴリ別のトークン合算（R21）。
#[derive(Debug, Clone, Default)]
pub struct TokenCategoryTotals {
    /// tool カテゴリ
    pub tool: CategoryTokenSummary,
    /// message カテゴリ
    pub message: CategoryTokenSummary,
    /// think カテゴリ
    pub think: CategoryTokenSummary,
}

/// モデル別のプロジェクト集計（R20）。
#[derive(Debug, Clone, Default)]
pub struct ModelAggregation {
    /// モデル名（実行時オーバーライド `model` 優先、なければブロック定義のモデル）
    pub model: String,
    /// 実行数
    pub execution_count: usize,
    /// 実行時間の合計（ミリ秒）
    pub total_duration_ms: i64,
    /// input トークン合計
    pub input_tokens: u64,
    /// output トークン合計
    pub output_tokens: u64,
    /// cache トークン合計
    pub cache_tokens: u64,
}

/// ブロック別のプロジェクト集計（R20）。
#[derive(Debug, Clone, Default)]
pub struct BlockAggregation {
    /// ブロック ID
    pub block_id: i64,
    /// ブロック名
    pub block_name: String,
    /// 実行数
    pub execution_count: usize,
    /// 実行時間の合計（ミリ秒）
    pub total_duration_ms: i64,
    /// input トークン合計
    pub input_tokens: u64,
    /// output トークン合計
    pub output_tokens: u64,
    /// cache トークン合計
    pub cache_tokens: u64,
}

/// R20 の集計結果（モデル別・ブロック別）。
#[derive(Debug, Clone, Default)]
pub struct ProjectAggregation {
    /// モデル別集計
    pub by_model: Vec<ModelAggregation>,
    /// ブロック別集計
    pub by_block: Vec<BlockAggregation>,
}

/// プロジェクト全体の実行サマリ（Project ダッシュボード用）。
///
/// ステータス別の実行数・finalize 済み数・総所要時間・総トークン数・
/// 総コストを集計する。
#[derive(Debug, Clone, Default)]
pub struct ProjectRunSummary {
    /// 全実行数
    pub total: usize,
    /// 完了（completed）実行数
    pub completed: usize,
    /// 失敗（failed）実行数
    pub failed: usize,
    /// 実行中（running）実行数
    pub running: usize,
    /// 待機（pending）実行数
    pub pending: usize,
    /// ユーザー明示完了（finalized）実行数
    pub finalized: usize,
    /// 総所要時間（ミリ秒）
    pub total_duration_ms: i64,
    /// 総トークン数（input + output + cache の全カテゴリ合計）
    pub total_tokens: u64,
    /// 総コスト（USD・`session_stats_json.cost_usd` の合計）
    pub total_cost_usd: f64,
}

/// 日別・ブロック別のトークン合計（R19）。
#[derive(Debug, Clone, Default)]
pub struct DailyBlockTokens {
    /// 日付（YYYY-MM-DD）
    pub date: String,
    /// ブロック名
    pub block_name: String,
    /// トークン合計（input + output + cache）
    pub total_tokens: u64,
}

/// `session_stats_json` から `token_usage_by_category` を抽出する内部ヘルパー。
///
/// カテゴリ別（tool / message / think）の input / output / cache を返す。
/// 後方互換のため、新フィールド（`token_usage_by_category`）が無い・パース不能・
/// cache が無い場合は 0 として扱う。
///
/// R28（U16）: 新フィールド（`token_usage_by_category`）が無い既存実行では、
/// セッション JSON のトップレベル `input_tokens` / `output_tokens` / `thinking_tokens`
/// へフォールバックして input / output / think を取得する。これにより既存データでも
/// Project 統計（input / output）が 0 にならず表示される。cache は旧データに無いため 0 のまま。
/// `session_stats_json` から合計コスト（USD）を抽出する内部ヘルパー。
///
/// `SessionStats.cost_usd`（engine 側で `usage.cost.total` を合算したもの）を
/// 読む。旧データでキーが無い・パース不能な場合は 0 として扱う。
fn parse_session_cost_usd(stats_json: Option<&str>) -> f64 {
    let Some(json) = stats_json else {
        return 0.0;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return 0.0;
    };
    value
        .get("cost_usd")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
}

fn parse_category_token_usage(stats_json: Option<&str>) -> TokenCategoryTotals {
    let mut totals = TokenCategoryTotals::default();
    let Some(json) = stats_json else {
        return totals;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return totals;
    };
    let Some(t) = value.get("token_usage_by_category") else {
        // R28: 既存実行（新フィールドなし）はトップレベルトークンへフォールバック。
        // カテゴリ不明のため input/output は tool、thinking は think.output に割り当てる。
        totals.tool.input = value
            .get("input_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        totals.tool.output = value
            .get("output_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        totals.think.output = value
            .get("thinking_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        return totals;
    };
    let apply = |cat: &str, out: &mut CategoryTokenSummary| {
        let Some(c) = t.get(cat) else {
            return;
        };
        out.input = c.get("input").and_then(|x| x.as_u64()).unwrap_or(0);
        out.output = c.get("output").and_then(|x| x.as_u64()).unwrap_or(0);
        out.cache = c.get("cache").and_then(|x| x.as_u64()).unwrap_or(0);
    };
    apply("tool", &mut totals.tool);
    apply("message", &mut totals.message);
    apply("think", &mut totals.think);
    totals
}

/// `execution_usage` の行を `ExecutionUsageRow` に変換する共通マッパー（L1）。
fn execution_usage_from_row(row: &Row) -> rusqlite::Result<ExecutionUsageRow> {
    Ok(ExecutionUsageRow {
        id: Some(row.get(0)?),
        execution_id: row.get(1)?,
        entry_type: row.get(2)?,
        turn_no: row.get(3)?,
        input_tokens: row.get(4)?,
        output_tokens: row.get(5)?,
        cache_read: row.get(6)?,
        cache_write: row.get(7)?,
        cost_total: row.get(8)?,
    })
}

/// 実行ごとの token / cost を取得する（L1）。
///
/// `execution_usage` の `total` 行を**優先**し（provider の実際の値）、
/// 未保存・全 0 の旧実行は `session_stats_json` の近似値へフォールバックする。
/// 戻り値は `(input, output, cache, cost)`。
/// sai-control の期間集計（Dashboard）からも使うため公開している。
pub fn exec_usage_totals(
    exec: &ExecutionHistory,
    usage_map: &HashMap<i64, ExecutionUsageRow>,
) -> (u64, u64, u64, f64) {
    if let Some(t) = usage_map.get(&exec.id.unwrap_or(0)) {
        let sum = t.input_tokens + t.output_tokens + t.cache_read + t.cache_write;
        if sum > 0 {
            return (
                t.input_tokens.max(0) as u64,
                t.output_tokens.max(0) as u64,
                t.cache_total().max(0) as u64,
                t.cost_total,
            );
        }
    }
    let usage = parse_category_token_usage(exec.session_stats_json.as_deref());
    (
        usage.tool.input + usage.message.input + usage.think.input,
        usage.tool.output + usage.message.output + usage.think.output,
        usage.tool.cache + usage.message.cache + usage.think.cache,
        parse_session_cost_usd(exec.session_stats_json.as_deref()),
    )
}

// ---------------------------------------------------------------------------
// SaiStore
// ---------------------------------------------------------------------------

/// sai-engine 専用テーブルに対する CRUD 操作を提供するストア。
///
/// waggle の `SqliteStore` とは別の `rusqlite::Connection` を持ち、
/// 同一 DB ファイル上の sai-engine 固有テーブルを管理する。
///
/// # スレッドセーフ
///
/// 内部で `Mutex<Connection>` を使用しており、`Send` かつ `Sync` を実現する。
pub struct SaiStore {
    conn: Mutex<Connection>,
}

impl SaiStore {
    /// 指定されたパスの DB ファイルを開き、テーブルを初期化する。
    ///
    /// ファイルが存在しない場合は新規作成される。
    /// 既存のファイルの場合はテーブルが追加作成される（冪等）。
    ///
    /// 同時実行性: デーモン・CLI・TUI・pi（waggle resolve 経由）が別プロセスで
    /// 同一DBを触るため、ロック競合時の即時 `SQLITE_BUSY` を避けるべく
    /// `busy_timeout`（5秒）を設定する（P1-2）。WAL + `synchronous=FULL` は
    /// `init_sai_tables` 側で維持する。
    pub fn open(path: &Path) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;
        // P1-2: 別プロセス同時アクセス時の SQLITE_BUSY 即エラーを避ける。
        // `busy_timeout` は接続ごとの設定のため open 直後に行う。
        let _ = conn.busy_timeout(std::time::Duration::from_millis(5000));
        init_sai_tables(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// インメモリの SQLite DB を開く（主にテスト用）。
    pub fn open_in_memory() -> Result<Self, DbError> {
        let conn = Connection::open_in_memory()?;
        init_sai_tables(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 既存の DB ファイルを **read-only** で開く（L2: 複数プロジェクト検索用）。
    ///
    /// 他プロジェクトの DB を参照するための専用メソッド。テーブルの初期化・
    /// マイグレーション（書き込み）は一切行わない。テーブルが存在しない場合は
    /// 以降のクエリがエラーになる。ファイルの新規作成も行わない。
    pub fn open_read_only(path: &Path) -> Result<Self, DbError> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // -----------------------------------------------------------------------
    // BlockMetadata CRUD
    // -----------------------------------------------------------------------

    /// 新しいブロック定義を挿入し、生成された `id` を返す。
    ///
    /// `name` の UNIQUE 制約違反時は `DbError::DuplicateName` を返す。
    pub fn insert_block(&self, block: &BlockMetadata) -> Result<i64, DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let result = tx.execute(
            "INSERT INTO block_metadata (name, display_name, description, model, provider, thinking, \
             tools, extensions, skills, timeout, worktree, enable, file_path, \
             body_hash, tag, round, layer) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                block.name,
                block.display_name,
                block.description,
                block.model,
                block.provider,
                block.thinking,
                block.tools,
                block.extensions,
                block.skills,
                block.timeout,
                block.worktree,
                block.enable,
                block.file_path,
                block.body_hash,
                block.tag,
                block.round,
                block.layer,
            ],
        );

        match result {
            Ok(_) => {
                let id = tx.last_insert_rowid();
                tx.commit()?;
                Ok(id)
            }
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                // UNIQUE 制約違反 → name 重複
                Err(DbError::DuplicateName(block.name.clone()))
            }
            Err(e) => {
                tx.rollback()?;
                Err(DbError::Rusqlite(e))
            }
        }
    }

    /// 主キーでブロック定義を検索する。
    ///
    /// 未存在時は `DbError::NotFound` を返す。
    pub fn get_block_by_id(&self, id: i64) -> Result<BlockMetadata, DbError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, display_name, description, model, provider, thinking, tools, extensions, \
             skills, timeout, worktree, enable, file_path, body_hash, \
             tag, round, layer, created_at, updated_at FROM block_metadata WHERE id = ?1",
            params![id],
            |row| {
                Ok(BlockMetadata {
                    id: Some(row.get(0)?),
                    name: row.get(1)?,
                    display_name: row.get(2)?,
                    description: row.get(3)?,
                    model: row.get(4)?,
                    provider: row.get(5)?,
                    thinking: row.get(6)?,
                    tools: row.get(7)?,
                    extensions: row.get(8)?,
                    skills: row.get(9)?,
                    timeout: row.get(10)?,
                    worktree: row.get(11)?,
                    enable: row.get::<_, i32>(12)? != 0,
                    file_path: row.get(13)?,
                    body_hash: row.get(14)?,
                    tag: row.get(15)?,
                    round: row.get(16)?,
                    layer: row.get(17)?,
                    created_at: row.get(18)?,
                    updated_at: row.get(19)?,
                })
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => DbError::NotFound(format!("block id = {}", id)),
            other => DbError::Rusqlite(other),
        })
    }

    /// 名前でブロック定義を検索する。
    pub fn get_block_by_name(&self, name: &str) -> Result<BlockMetadata, DbError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, display_name, description, model, provider, thinking, tools, extensions, \
             skills, timeout, worktree, enable, file_path, body_hash, \
             tag, round, layer, created_at, updated_at FROM block_metadata WHERE name = ?1",
            params![name],
            |row| {
                Ok(BlockMetadata {
                    id: Some(row.get(0)?),
                    name: row.get(1)?,
                    display_name: row.get(2)?,
                    description: row.get(3)?,
                    model: row.get(4)?,
                    provider: row.get(5)?,
                    thinking: row.get(6)?,
                    tools: row.get(7)?,
                    extensions: row.get(8)?,
                    skills: row.get(9)?,
                    timeout: row.get(10)?,
                    worktree: row.get(11)?,
                    enable: row.get::<_, i32>(12)? != 0,
                    file_path: row.get(13)?,
                    body_hash: row.get(14)?,
                    tag: row.get(15)?,
                    round: row.get(16)?,
                    layer: row.get(17)?,
                    created_at: row.get(18)?,
                    updated_at: row.get(19)?,
                })
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                DbError::NotFound(format!("block name = '{}'", name))
            }
            other => DbError::Rusqlite(other),
        })
    }

    /// 全ブロック定義の一覧を取得する（`enable = 1` のもののみ）。
    pub fn list_blocks(&self) -> Result<Vec<BlockMetadata>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, display_name, description, model, provider, thinking, tools, extensions, \
             skills, timeout, worktree, enable, file_path, body_hash, \
             tag, round, layer, created_at, updated_at FROM block_metadata WHERE enable = 1 ORDER BY name",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(BlockMetadata {
                id: Some(row.get(0)?),
                name: row.get(1)?,
                display_name: row.get(2)?,
                description: row.get(3)?,
                model: row.get(4)?,
                provider: row.get(5)?,
                thinking: row.get(6)?,
                tools: row.get(7)?,
                extensions: row.get(8)?,
                skills: row.get(9)?,
                timeout: row.get(10)?,
                worktree: row.get(11)?,
                enable: row.get::<_, i32>(12)? != 0,
                file_path: row.get(13)?,
                body_hash: row.get(14)?,
                tag: row.get(15)?,
                round: row.get(16)?,
                layer: row.get(17)?,
                created_at: row.get(18)?,
                updated_at: row.get(19)?,
            })
        })?;

        let mut blocks = Vec::new();
        for row in rows {
            blocks.push(row?);
        }
        Ok(blocks)
    }

    /// ブロック定義を更新する。
    ///
    /// `updated_at` を現在時刻で更新する。
    pub fn update_block(&self, block: &BlockMetadata) -> Result<(), DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let id = block.id.ok_or_else(|| {
            DbError::InvalidArgument("update_block には id が必要です".to_string())
        })?;

        let result = tx.execute(
            "UPDATE block_metadata SET name = ?1, display_name = ?2, description = ?3, model = ?4, \
             provider = ?5, thinking = ?6, tools = ?7, extensions = ?8, skills = ?9, \
             timeout = ?10, worktree = ?11, enable = ?12, \
             file_path = ?13, body_hash = ?14, tag = ?15, round = ?16, layer = ?17, \
             updated_at = datetime('now') WHERE id = ?18",
            params![
                block.name,
                block.display_name,
                block.description,
                block.model,
                block.provider,
                block.thinking,
                block.tools,
                block.extensions,
                block.skills,
                block.timeout,
                block.worktree,
                block.enable,
                block.file_path,
                block.body_hash,
                block.tag,
                block.round,
                block.layer,
                id,
            ],
        );

        match result {
            Ok(0) => {
                tx.rollback()?;
                Err(DbError::NotFound(format!("block id = {}", id)))
            }
            Ok(_) => {
                tx.commit()?;
                Ok(())
            }
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(DbError::DuplicateName(block.name.clone()))
            }
            Err(e) => {
                tx.rollback()?;
                Err(DbError::Rusqlite(e))
            }
        }
    }

    /// ブロック定義を削除する（block_metadata のみ。入出力は CASCADE 削除）。
    ///
    /// 接続や実行履歴は削除しない。完全削除には `delete_block_full` を使用。
    pub fn delete_block(&self, id: i64) -> Result<(), DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let affected = tx.execute("DELETE FROM block_metadata WHERE id = ?", params![id])?;

        match affected {
            0 => {
                tx.rollback()?;
                Err(DbError::NotFound(format!("block id = {}", id)))
            }
            _ => {
                tx.commit()?;
                Ok(())
            }
        }
    }

    /// ブロック定義と関連データを完全に削除する。
    ///
    /// 削除順: block_connections → execution_history → block_metadata
    pub fn delete_block_full(&self, id: i64) -> Result<(String, usize, usize), DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        // ブロック名を取得
        let name: String = tx
            .query_row(
                "SELECT name FROM block_metadata WHERE id = ?",
                params![id],
                |row| row.get(0),
            )
            .map_err(|_| DbError::NotFound(format!("block id = {}", id)))?;

        // 1. ブロック間接続を削除
        let conn_count: usize = tx.execute(
            "DELETE FROM block_connections WHERE source_block_id = ? OR target_block_id = ?",
            params![id, id],
        )?;

        // 2. 実行履歴を削除（依存レコードごと再帰的に削除）
        let exec_ids: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT id FROM execution_history WHERE block_id = ?1")?;
            let rows = stmt.query_map(params![id], |row| row.get(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };
        let mut exec_count: usize = 0;
        for eid in exec_ids {
            let summary = Self::delete_execution_cascade_inner(&tx, eid)?;
            exec_count += summary.executions;
        }

        // 3. ブロック定義を削除（CASCADE で inputs/outputs も削除）
        tx.execute("DELETE FROM block_metadata WHERE id = ?", params![id])?;

        tx.commit()?;
        Ok((name, conn_count, exec_count))
    }

    /// 指定された実行IDに関連するブロック間接続を削除する。
    pub fn delete_connections_by_execution(&self, exec_id: i64) -> Result<usize, DbError> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "DELETE FROM block_connections WHERE source_execution_id = ? OR target_execution_id = ?",
            params![exec_id, exec_id],
        )?;
        Ok(count)
    }

    /// 指定された実行IDの実行レコードを、依存レコードとともに再帰的に削除する。
    ///
    /// FK 制約を満たすため、以下の順で削除する:
    /// 1. 子実行（`parent_run_id` が対象のサブ実行）を再帰的に削除
    /// 2. `block_connections`（source / target）
    /// 3. `tasks`（parent_run_id / execution_id）
    /// 4. `git_commits`
    /// 5. 実行レコード本体
    pub fn delete_execution_cascade(
        &self,
        exec_id: i64,
    ) -> Result<DeleteExecutionSummary, DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let summary = Self::delete_execution_cascade_inner(&tx, exec_id)?;
        tx.commit()?;
        Ok(summary)
    }

    /// トランザクション内で実行削除を再帰的に行う内部ヘルパー。
    fn delete_execution_cascade_inner(
        tx: &rusqlite::Transaction<'_>,
        exec_id: i64,
    ) -> Result<DeleteExecutionSummary, DbError> {
        // 1. 子実行（サブタスク）を再帰的に削除
        let child_ids: Vec<i64> = {
            let mut stmt =
                tx.prepare("SELECT id FROM execution_history WHERE parent_run_id = ?1")?;
            let rows = stmt.query_map(params![exec_id], |row| row.get(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };

        let mut summary = DeleteExecutionSummary::default();
        for child_id in child_ids {
            let child = Self::delete_execution_cascade_inner(tx, child_id)?;
            summary.merge(child);
        }

        // 2. ブロック間接続を削除
        summary.connections += tx.execute(
            "DELETE FROM block_connections WHERE source_execution_id = ?1 OR target_execution_id = ?1",
            params![exec_id],
        )?;

        // 3. タスクを削除（親実行 / 実行紐付けの両方を対象）
        summary.tasks += tx.execute(
            "DELETE FROM tasks WHERE parent_run_id = ?1 OR execution_id = ?1",
            params![exec_id],
        )?;

        // 4. git コミットを削除（ON DELETE CASCADE でも削除されるが明示的に）
        summary.git_commits += tx.execute(
            "DELETE FROM git_commits WHERE execution_id = ?1",
            params![exec_id],
        )?;

        // 5. 実行レコード本体
        let affected = tx.execute(
            "DELETE FROM execution_history WHERE id = ?1",
            params![exec_id],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound(format!("execution id = {}", exec_id)));
        }
        summary.executions += 1;

        Ok(summary)
    }

    /// 指定された実行IDの実行レコードを削除する。
    pub fn delete_execution(&self, exec_id: i64) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "DELETE FROM execution_history WHERE id = ?",
            params![exec_id],
        )?;
        if affected == 0 {
            Err(DbError::NotFound(format!("execution id = {}", exec_id)))
        } else {
            Ok(())
        }
    }

    // -----------------------------------------------------------------------
    // BlockInput CRUD
    // -----------------------------------------------------------------------

    /// 指定ブロックの入力定義をバルク挿入する。
    ///
    /// トランザクション内で全件を挿入し、既存の入力定義は
    /// `delete_inputs_by_block_id()` で事前削除しておくこと。
    pub fn insert_inputs(&self, block_id: i64, inputs: &[BlockInput]) -> Result<(), DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        for input in inputs {
            tx.execute(
                "INSERT INTO block_inputs (block_id, input_id, input_type, label, \
                 description, required, default_value, options_json, \
                 source_blocks_json, source_categories_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    block_id,
                    input.input_id,
                    input.input_type,
                    input.label,
                    input.description,
                    input.required,
                    input.default_value,
                    input.options_json,
                    input.source_blocks_json,
                    input.source_categories_json,
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// 指定ブロックの全入力定義を取得する。
    pub fn get_inputs_by_block_id(&self, block_id: i64) -> Result<Vec<BlockInput>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, block_id, input_id, input_type, label, description, required, \
             default_value, options_json, source_blocks_json, source_categories_json \
             FROM block_inputs WHERE block_id = ?1 ORDER BY id",
        )?;

        let rows = stmt.query_map(params![block_id], |row| {
            Ok(BlockInput {
                id: Some(row.get(0)?),
                block_id: row.get(1)?,
                input_id: row.get(2)?,
                input_type: row.get(3)?,
                label: row.get(4)?,
                description: row.get(5)?,
                required: row.get::<_, i32>(6)? != 0,
                default_value: row.get(7)?,
                options_json: row.get(8)?,
                source_blocks_json: row.get(9)?,
                source_categories_json: row.get(10)?,
            })
        })?;

        let mut inputs = Vec::new();
        for row in rows {
            inputs.push(row?);
        }
        Ok(inputs)
    }

    /// 指定ブロックの全入力定義を削除する（更新時の置き換え用）。
    pub fn delete_inputs_by_block_id(&self, block_id: i64) -> Result<(), DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM block_inputs WHERE block_id = ?1",
            params![block_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // BlockOutput CRUD
    // -----------------------------------------------------------------------

    /// 出力定義を挿入し、生成された `id` を返す。
    pub fn insert_output(&self, output: &BlockOutput) -> Result<i64, DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute(
            "INSERT INTO block_output (block_id, format, waggle_tokenize, description) \
             VALUES (?1, ?2, ?3, ?4)",
            params![
                output.block_id,
                output.format,
                output.waggle_tokenize,
                output.description,
            ],
        )?;

        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// 指定ブロックの出力定義を取得する。
    pub fn get_output_by_block_id(&self, block_id: i64) -> Result<BlockOutput, DbError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, block_id, format, waggle_tokenize, description \
             FROM block_output WHERE block_id = ?1",
            params![block_id],
            |row| {
                Ok(BlockOutput {
                    id: Some(row.get(0)?),
                    block_id: row.get(1)?,
                    format: row.get(2)?,
                    waggle_tokenize: row.get::<_, i32>(3)? != 0,
                    description: row.get(4)?,
                })
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                DbError::NotFound(format!("output for block id = {}", block_id))
            }
            other => DbError::Rusqlite(other),
        })
    }

    /// 出力定義を更新する。
    pub fn update_output(&self, output: &BlockOutput) -> Result<(), DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let id = output.id.ok_or_else(|| {
            DbError::InvalidArgument("update_output には id が必要です".to_string())
        })?;

        let affected = tx.execute(
            "UPDATE block_output SET format = ?1, waggle_tokenize = ?2, description = ?3 \
             WHERE id = ?4",
            params![
                output.format,
                output.waggle_tokenize,
                output.description,
                id
            ],
        )?;

        match affected {
            0 => {
                tx.rollback()?;
                Err(DbError::NotFound(format!("output id = {}", id)))
            }
            _ => {
                tx.commit()?;
                Ok(())
            }
        }
    }

    // -----------------------------------------------------------------------
    // ExecutionHistory CRUD
    // -----------------------------------------------------------------------

    /// 予約 run を作成する（カード作成時・R301）。
    ///
    /// `status='todo'` の行を作り run_id を確定させる。`block_id` は
    /// `__todo__` プレースホルダーブロックに紐付ける（`block_id` NOT NULL 維持）。
    /// 実行開始時は [`Self::claim_reserved_run`] で `block_id` を確定させる。
    pub fn create_reserved_run(&self, user_inputs_json: Option<&str>) -> Result<i64, DbError> {
        let todo_block_id = self.get_or_create_todo_block()?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO execution_history (block_id, status, user_inputs_json, execution_mode) \
             VALUES (?1, 'todo', ?2, 'kanban')",
            params![todo_block_id, user_inputs_json],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// `__todo__` プレースホルダーブロックを確保する（予約行の FK 用）。
    ///
    /// 予約行はブロック未定のため、実在ブロックを指す必要がある。
    /// 既存なら再利用し、無ければ最小レコードを作成する。
    pub fn get_or_create_todo_block(&self) -> Result<i64, DbError> {
        if let Ok(b) = self.get_block_by_name("__todo__") {
            if let Some(id) = b.id {
                return Ok(id);
            }
        }
        let block = BlockMetadata {
            id: None,
            name: "__todo__".to_string(),
            display_name: "未割当".to_string(),
            description: "カード予約用プレースホルダー".to_string(),
            model: "inherit".to_string(),
            provider: None,
            thinking: "off".to_string(),
            tools: "false".to_string(),
            extensions: "false".to_string(),
            skills: None,
            timeout: 300,
            worktree: "none".to_string(),
            enable: false,
            file_path: "".to_string(),
            body_hash: None,
            tag: None,
            round: None,
            layer: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        match self.insert_block(&block) {
            Ok(id) => Ok(id),
            Err(DbError::DuplicateName(_)) => self
                .get_block_by_name("__todo__")
                .and_then(|b| {
                    b.id.ok_or_else(|| DbError::NotFound("__todo__ id".to_string()))
                }),
            Err(e) => Err(e),
        }
    }

    /// `__user__` プレースホルダーブロックを確保する（runner=user タスク用）。
    ///
    /// user タスク（`runner: user`）は pi を実行せずユーザーが行うため、
    /// ブロック定義を参照しない。DB の `block_id` NOT NULL を満たすための
    /// プレースホルダー。実在すれば再利用、無ければ最小レコードを作成する。
    pub fn get_or_create_user_block(&self) -> Result<i64, DbError> {
        if let Ok(b) = self.get_block_by_name("__user__") {
            if let Some(id) = b.id {
                return Ok(id);
            }
        }
        let block = BlockMetadata {
            id: None,
            name: "__user__".to_string(),
            display_name: "user".to_string(),
            description: "ユーザータスク（runner: user）用プレースホルダー".to_string(),
            model: "inherit".to_string(),
            provider: None,
            thinking: "off".to_string(),
            tools: "false".to_string(),
            extensions: "false".to_string(),
            skills: None,
            timeout: 300,
            worktree: "none".to_string(),
            enable: false,
            file_path: "".to_string(),
            body_hash: None,
            tag: None,
            round: None,
            layer: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        match self.insert_block(&block) {
            Ok(id) => Ok(id),
            Err(DbError::DuplicateName(_)) => self
                .get_block_by_name("__user__")
                .and_then(|b| {
                    b.id.ok_or_else(|| DbError::NotFound("__user__ id".to_string()))
                }),
            Err(e) => Err(e),
        }
    }

    /// 予約行を実行開始用に確定させる（R302）。
    ///
    /// `status='todo'` の行を `pending` に戻し、`block_id` を実ブロックに付け替える。
    /// 予約でない行には何もしない（冪等ではなく事前条件エラー）。
    pub fn claim_reserved_run(
        &self,
        run_id: i64,
        block_id: i64,
        user_inputs_json: Option<&str>,
    ) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE execution_history SET block_id = ?1, status = 'pending', user_inputs_json = ?2 \
             WHERE id = ?3 AND status = 'todo'",
            params![block_id, user_inputs_json, run_id],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound(format!("予約 run がありません: {}", run_id)));
        }
        Ok(())
    }

    /// 予約行をサブタスク（子）の実行開始用に確定させる（U2）。
    ///
    /// [`Self::claim_reserved_run`] と違い、親の `run_id` と `batch_id` を同時に書き込む。
    /// 予約行の id は todo のカードの run_id のまま変わらない（研究方針の木との結び付きを保つ）。
    pub fn claim_reserved_run_for_subtask(
        &self,
        run_id: i64,
        block_id: i64,
        user_inputs_json: Option<&str>,
        parent_run_id: i64,
        batch_id: Option<&str>,
    ) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE execution_history SET block_id = ?1, status = 'pending', user_inputs_json = ?2, \
             parent_run_id = ?3, batch_id = ?4 \
             WHERE id = ?5 AND status = 'todo'",
            params![block_id, user_inputs_json, parent_run_id, batch_id, run_id],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound(format!("予約 run がありません: {}", run_id)));
        }
        Ok(())
    }

    /// 同じ todo のカードを既に起こしている未完了タスクを探す（U2・二重起動の検査）。
    ///
    /// `finalized` / `failed` / `deleted` は終了したタスクなので対象から外す。
    pub fn list_unfinished_tasks_by_card(
        &self,
        card_run_id: i64,
    ) -> Result<Vec<Task>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM tasks WHERE card_run_id = ?1 \
             AND status NOT IN ('finalized', 'failed', 'deleted') ORDER BY id",
            TASK_SELECT_COLS
        ))?;
        let rows = stmt.query_map(params![card_run_id], |row| task_from_row(row))?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }
        Ok(tasks)
    }

    /// 新規実行レコードを作成する（status = `pending`）。
    ///
    /// 生成された `id` を返す。
    pub fn create_execution(
        &self,
        block_id: i64,
        user_inputs_json: Option<&str>,
    ) -> Result<i64, DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute(
            "INSERT INTO execution_history (block_id, status, user_inputs_json) \
             VALUES (?1, 'pending', ?2)",
            params![block_id, user_inputs_json],
        )?;

        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// サブタスク用: 親実行ID付きで新規実行レコードを作成する。
    ///
    /// `batch_id` はタスクバッチID（P3-A: DAG のフェーズ合成に使用）。
    /// 生成された `id` を返す。
    pub fn create_execution_for_subtask(
        &self,
        block_id: i64,
        user_inputs_json: Option<&str>,
        parent_run_id: i64,
        batch_id: Option<&str>,
    ) -> Result<i64, DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute(
            "INSERT INTO execution_history (block_id, status, user_inputs_json, parent_run_id, batch_id) \
             VALUES (?1, 'pending', ?2, ?3, ?4)",
            params![block_id, user_inputs_json, parent_run_id, batch_id],
        )?;

        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// 実行ステータスを更新する。
    ///
    /// `running` 開始時に `started_at` を設定し、`completed` または `failed` 時に
    /// `finished_at` と `duration_ms` を計算する。
    pub fn update_execution_status(&self, id: i64, status: &str) -> Result<(), DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        match status {
            "running" => {
                tx.execute(
                    "UPDATE execution_history SET status = ?1, started_at = datetime('now') \
                     WHERE id = ?2",
                    params![status, id],
                )?;
            }
            "completed" | "failed" => {
                tx.execute(
                    "UPDATE execution_history SET status = ?1, finished_at = datetime('now'), \
                     duration_ms = CAST(\
                       (julianday('now') - julianday(started_at)) * 86400000 AS INTEGER\
                     ) WHERE id = ?2",
                    params![status, id],
                )?;
            }
            _ => {
                tx.execute(
                    "UPDATE execution_history SET status = ?1 WHERE id = ?2",
                    params![status, id],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// ユーザー明示完了（Ctrl+D / pi-sai-finalize）の finalize フラグを更新する（U4）。
    ///
    /// `finalized=true` は「sai-write なしのユーザー明示完了」であることを示し、
    /// `sai block list` の `完了` 表示と右SB の Session 再開を確定させる。
    pub fn update_execution_finalized(&self, id: i64, finalized: bool) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET finalized = ?1 WHERE id = ?2",
            params![finalized as i64, id],
        )?;
        Ok(())
    }

    /// waggle トークン（FN-007 連携用）を記録する。
    pub fn update_execution_waggle_token(&self, id: i64, token: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET output_waggle_token = ?1 WHERE id = ?2",
            params![token, id],
        )?;
        Ok(())
    }

    /// 複数の waggle トークンを JSON 配列として保存する（sai-write 用）。
    ///
    /// tokens が空の場合は NULL に設定される。
    pub fn update_execution_waggle_tokens(
        &self,
        id: i64,
        tokens: &[String],
    ) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        if tokens.is_empty() {
            conn.execute(
                "UPDATE execution_history SET output_waggle_tokens = NULL WHERE id = ?1",
                params![id],
            )?;
        } else {
            let json_array = serde_json::to_string(tokens).map_err(|e| DbError::Json(e))?;
            conn.execute(
                "UPDATE execution_history SET output_waggle_tokens = ?1 WHERE id = ?2",
                params![json_array, id],
            )?;
        }
        Ok(())
    }

    /// `output_waggle_tokens` 配列の末尾に 1 件追記する（runner=user の sai-write 用）。
    ///
    /// 既存トークンは保持したまま新しいトークンを追加する。既存が NULL/空なら
    /// `[token]` になる。単数カラム `output_waggle_token` は変更しない。
    pub fn append_execution_waggle_token(&self, id: i64, token: &str) -> Result<(), DbError> {
        let exec = self.get_execution_by_id(id)?;
        let mut tokens: Vec<String> = exec
            .output_waggle_tokens
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default();
        tokens.push(token.to_string());
        self.update_execution_waggle_tokens(id, &tokens)
    }

    /// 出力タイプを更新する。
    ///
    /// `output_type` は "document" | "code" | "both" のいずれか。
    pub fn update_execution_output_type(&self, id: i64, output_type: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET output_type = ?1 WHERE id = ?2",
            params![output_type, id],
        )?;
        Ok(())
    }

    /// エージェントの出力内容を保存する（FN-012）。
    ///
    /// パイプライン実行完了後、`PipelineResult.output` の内容を
    /// このメソッドで `execution_history` に永続化する。
    ///
    /// # 引数
    ///
    /// * `id` — 実行履歴の ID
    /// * `content` — エージェント出力の全文
    pub fn update_execution_output_content(&self, id: i64, content: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET output_content = ?1 WHERE id = ?2",
            params![content, id],
        )?;
        Ok(())
    }

    /// 入力として使った waggle トークンを JSON 配列として保存する。
    pub fn update_execution_input_tokens(&self, id: i64, tokens: &[String]) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        if tokens.is_empty() {
            conn.execute(
                "UPDATE execution_history SET input_waggle_tokens = NULL WHERE id = ?1",
                params![id],
            )?;
        } else {
            let json_array = serde_json::to_string(tokens).map_err(|e| DbError::Json(e))?;
            conn.execute(
                "UPDATE execution_history SET input_waggle_tokens = ?1 WHERE id = ?2",
                params![json_array, id],
            )?;
        }
        Ok(())
    }

    /// pi起動前/終了時の Git コミットハッシュを保存する（再現性用）。
    pub fn update_execution_git_commits(
        &self,
        id: i64,
        commit_before: Option<&str>,
        commit_after: Option<&str>,
    ) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET git_commit_before = ?1, git_commit_after = ?2 WHERE id = ?3",
            params![commit_before, commit_after, id],
        )?;
        Ok(())
    }

    /// herdr pane 情報を記録する。
    pub fn update_execution_pane(&self, id: i64, pane_id: &str, pid: i32) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET herdr_pane_id = ?1, pid = ?2 WHERE id = ?3",
            params![pane_id, pid, id],
        )?;
        Ok(())
    }

    /// エラー情報を記録する。
    pub fn update_execution_error(
        &self,
        id: i64,
        error: &str,
        exit_code: i32,
    ) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET error_message = ?1, exit_code = ?2, \
             status = 'failed', finished_at = datetime('now') WHERE id = ?3",
            params![error, exit_code, id],
        )?;
        Ok(())
    }

    /// 実行詳細を取得する。
    pub fn get_execution_by_id(&self, id: i64) -> Result<ExecutionHistory, DbError> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} \
             FROM execution_history WHERE id = ?1",
            EXECUTION_SELECT_COLS
        );
        conn.query_row(&sql, params![id], |row| execution_from_row(row))
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    DbError::NotFound(format!("execution id = {}", id))
                }
                other => DbError::Rusqlite(other),
            })
    }

    /// ブロックごとの実行履歴一覧を取得する。
    pub fn list_executions_by_block(
        &self,
        block_id: i64,
    ) -> Result<Vec<ExecutionHistory>, DbError> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} \
             FROM execution_history WHERE block_id = ?1 ORDER BY created_at DESC",
            EXECUTION_SELECT_COLS
        );
        let mut stmt = conn.prepare(&sql)?;

        let rows = stmt.query_map(params![block_id], |row| execution_from_row(row))?;

        let mut executions = Vec::new();
        for row in rows {
            executions.push(row?);
        }
        Ok(executions)
    }

    /// 最近の実行履歴を取得する（`created_at DESC`）。
    pub fn list_recent_executions(&self, limit: i64) -> Result<Vec<ExecutionHistory>, DbError> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} \
             FROM execution_history ORDER BY created_at DESC LIMIT ?1",
            EXECUTION_SELECT_COLS
        );
        let mut stmt = conn.prepare(&sql)?;

        let rows = stmt.query_map(params![limit], |row| execution_from_row(row))?;

        let mut executions = Vec::new();
        for row in rows {
            executions.push(row?);
        }
        Ok(executions)
    }

    // -----------------------------------------------------------------------
    // GitCommit CRUD
    // -----------------------------------------------------------------------

    /// コミット情報を記録し、生成された `id` を返す。
    pub fn insert_commit(&self, commit: &GitCommit) -> Result<i64, DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute(
            "INSERT INTO git_commits (execution_id, commit_hash, commit_message, \
             author_name, author_email, branch_name) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                commit.execution_id,
                commit.commit_hash,
                commit.commit_message,
                commit.author_name,
                commit.author_email,
                commit.branch_name,
            ],
        )?;

        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// 実行に関連付けられた全コミットを取得する。
    pub fn get_commits_by_execution(&self, execution_id: i64) -> Result<Vec<GitCommit>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, execution_id, commit_hash, commit_message, author_name, \
             author_email, branch_name, committed_at \
             FROM git_commits WHERE execution_id = ?1 ORDER BY committed_at",
        )?;

        let rows = stmt.query_map(params![execution_id], |row| {
            Ok(GitCommit {
                id: Some(row.get(0)?),
                execution_id: row.get(1)?,
                commit_hash: row.get(2)?,
                commit_message: row.get(3)?,
                author_name: row.get(4)?,
                author_email: row.get(5)?,
                branch_name: row.get(6)?,
                committed_at: row.get(7)?,
            })
        })?;

        let mut commits = Vec::new();
        for row in rows {
            commits.push(row?);
        }
        Ok(commits)
    }

    /// コミットハッシュで検索する。
    pub fn get_commit_by_hash(&self, hash: &str) -> Result<GitCommit, DbError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, execution_id, commit_hash, commit_message, author_name, \
             author_email, branch_name, committed_at \
             FROM git_commits WHERE commit_hash = ?1",
            params![hash],
            |row| {
                Ok(GitCommit {
                    id: Some(row.get(0)?),
                    execution_id: row.get(1)?,
                    commit_hash: row.get(2)?,
                    commit_message: row.get(3)?,
                    author_name: row.get(4)?,
                    author_email: row.get(5)?,
                    branch_name: row.get(6)?,
                    committed_at: row.get(7)?,
                })
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                DbError::NotFound(format!("commit hash = '{}'", hash))
            }
            other => DbError::Rusqlite(other),
        })
    }

    // -----------------------------------------------------------------------
    // BlockConnection CRUD
    // -----------------------------------------------------------------------

    /// ブロック間接続を1件挿入する（冪等）。
    pub fn insert_connection(&self, conn_record: &BlockConnection) -> Result<i64, DbError> {
        let mut db_conn = self.conn.lock().unwrap();
        let tx = db_conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(ref token) = conn_record.waggle_token {
            let exists: bool = tx
                .query_row(
                    "SELECT COUNT(*) > 0 FROM block_connections \
                     WHERE source_execution_id = ?1 AND target_execution_id = ?2 \
                     AND waggle_token = ?3",
                    params![
                        conn_record.source_execution_id,
                        conn_record.target_execution_id,
                        token
                    ],
                    |row| row.get(0),
                )
                .unwrap_or(false);
            if exists {
                tx.commit()?;
                return Ok(0);
            }
        }

        tx.execute(
            "INSERT INTO block_connections \
             (source_execution_id, source_block_id, target_execution_id, target_block_id, \
              connection_type, waggle_token, commit_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                conn_record.source_execution_id,
                conn_record.source_block_id,
                conn_record.target_execution_id,
                conn_record.target_block_id,
                conn_record.connection_type,
                conn_record.waggle_token,
                conn_record.commit_hash,
            ],
        )?;

        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// 指定の組み合わせ（source → target、指定タイプ）の接続が存在するか確認する。
    ///
    /// `task_delegation` / `task_output` のようなトークンを持たない接続の
    /// 重複挿入防止に使用する。
    pub fn connection_exists(
        &self,
        source_execution_id: i64,
        target_execution_id: i64,
        connection_type: &str,
    ) -> Result<bool, DbError> {
        let conn = self.conn.lock().unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM block_connections \
                 WHERE source_execution_id = ?1 AND target_execution_id = ?2 \
                 AND connection_type = ?3",
                params![source_execution_id, target_execution_id, connection_type],
                |row| row.get(0),
            )
            .unwrap_or(false);
        Ok(exists)
    }

    /// 指定ブロックを起点/終点とする全接続を取得する。
    pub fn list_connections_by_block(
        &self,
        block_id: i64,
    ) -> Result<Vec<BlockConnection>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, source_execution_id, source_block_id, target_execution_id, \
             target_block_id, connection_type, waggle_token, commit_hash, created_at \
             FROM block_connections WHERE source_block_id = ?1 OR target_block_id = ?1 \
             ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![block_id], |row| {
            Ok(BlockConnection {
                id: Some(row.get(0)?),
                source_execution_id: row.get(1)?,
                source_block_id: row.get(2)?,
                target_execution_id: row.get(3)?,
                target_block_id: row.get(4)?,
                connection_type: row.get(5)?,
                waggle_token: row.get(6)?,
                commit_hash: row.get(7)?,
            })
        })?;
        let mut connections = Vec::new();
        for row in rows {
            connections.push(row?);
        }
        Ok(connections)
    }

    /// 全接続を実行単位（source_execution_id → target_execution_id）で取得する。
    ///
    /// TUI のブロック線図描画用。block_connections は実行単位で保存されているため、
    /// ブロック単位の集約ではなく実行 ID の組をそのまま返す。
    pub fn list_connections(&self) -> Result<Vec<BlockConnection>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, source_execution_id, source_block_id, target_execution_id, \
             target_block_id, connection_type, waggle_token, commit_hash, created_at \
             FROM block_connections ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(BlockConnection {
                id: Some(row.get(0)?),
                source_execution_id: row.get(1)?,
                source_block_id: row.get(2)?,
                target_execution_id: row.get(3)?,
                target_block_id: row.get(4)?,
                connection_type: row.get(5)?,
                waggle_token: row.get(6)?,
                commit_hash: row.get(7)?,
            })
        })?;
        let mut connections = Vec::new();
        for row in rows {
            connections.push(row?);
        }
        Ok(connections)
    }

    /// ブロック間接続をブロック単位で集約して返す（線図描画用）。
    pub fn list_connections_grouped(&self) -> Result<Vec<ConnectionGroup>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT c.source_block_id, c.target_block_id, \
                    sb.name, tb.name, COUNT(*) \
             FROM block_connections c \
             JOIN block_metadata sb ON sb.id = c.source_block_id \
             JOIN block_metadata tb ON tb.id = c.target_block_id \
             GROUP BY c.source_block_id, c.target_block_id \
             ORDER BY COUNT(*) DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ConnectionGroup {
                source_block_id: row.get(0)?,
                target_block_id: row.get(1)?,
                source_name: row.get(2)?,
                target_name: row.get(3)?,
                connection_count: row.get(4)?,
            })
        })?;
        let mut groups = Vec::new();
        for row in rows {
            groups.push(row?);
        }
        Ok(groups)
    }

    /// ブロック間接続を connection_type を含めて集約して返す。
    ///
    /// 各グループに connection_type が付与され、
    /// task_delegation と waggle_token/git_commit を区別可能。
    pub fn list_connections_grouped_by_type(
        &self,
    ) -> Result<Vec<(ConnectionGroup, String)>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT c.source_block_id, c.target_block_id, \
                    sb.name, tb.name, c.connection_type, COUNT(*) \
             FROM block_connections c \
             JOIN block_metadata sb ON sb.id = c.source_block_id \
             JOIN block_metadata tb ON tb.id = c.target_block_id \
             GROUP BY c.source_block_id, c.target_block_id, c.connection_type \
             ORDER BY COUNT(*) DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                ConnectionGroup {
                    source_block_id: row.get(0)?,
                    target_block_id: row.get(1)?,
                    source_name: row.get(2)?,
                    target_name: row.get(3)?,
                    connection_count: row.get(5)?,
                },
                row.get::<_, String>(4)
                    .unwrap_or_else(|_| "waggle_token".to_string()),
            ))
        })?;
        let mut groups = Vec::new();
        for row in rows {
            groups.push(row?);
        }
        Ok(groups)
    }

    /// セッション統計情報を JSON として保存する。
    ///
    /// # 引数
    ///
    /// * `id` — 実行履歴の ID
    /// * `stats_json` — セッション統計の JSON 文字列
    pub fn update_execution_session_stats(&self, id: i64, stats_json: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET session_stats_json = ?1 WHERE id = ?2",
            params![stats_json, id],
        )?;
        Ok(())
    }

    // -------------------------------------------------------------------
    // 実行 usage（L1: execution_usage テーブル）
    // -------------------------------------------------------------------

    /// 実行 1 件分の usage 行を **全置換** する（1 トランザクション）。
    ///
    /// 既存行を削除してから `rows` を挿入するため冪等（backfill の再実行や
    /// 実行完了時の再書き込みで結果が変わらない）。空配列を渡すと既存行のみ削除する。
    pub fn replace_execution_usage(
        &self,
        execution_id: i64,
        rows: &[ExecutionUsageRow],
    ) -> Result<(), DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM execution_usage WHERE execution_id = ?1",
            params![execution_id],
        )?;
        for row in rows {
            tx.execute(
                "INSERT INTO execution_usage \
                 (execution_id, entry_type, turn_no, input_tokens, output_tokens, \
                  cache_read, cache_write, cost_total) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    row.execution_id,
                    row.entry_type,
                    row.turn_no,
                    row.input_tokens,
                    row.output_tokens,
                    row.cache_read,
                    row.cache_write,
                    row.cost_total,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 実行 1 件分の usage 行を取得する（`total` 行 + 内訳。未保存時は空）。
    pub fn list_execution_usage(
        &self,
        execution_id: i64,
    ) -> Result<Vec<ExecutionUsageRow>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, execution_id, entry_type, turn_no, input_tokens, output_tokens, \
             cache_read, cache_write, cost_total \
             FROM execution_usage WHERE execution_id = ?1 \
             ORDER BY CASE WHEN entry_type = 'total' THEN 0 ELSE 1 END, turn_no, entry_type",
        )?;
        let rows = stmt
            .query_map(params![execution_id], execution_usage_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 全実行の `total` 行を取得する（横断集計・Project 画面用）。
    ///
    /// 戻り値は `(execution_id, total 行)`。
    pub fn list_execution_usage_totals(&self) -> Result<Vec<(i64, ExecutionUsageRow)>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, execution_id, entry_type, turn_no, input_tokens, output_tokens, \
             cache_read, cache_write, cost_total \
             FROM execution_usage WHERE entry_type = 'total'",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let exec_id: i64 = row.get(1)?;
                Ok((exec_id, execution_usage_from_row(row)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 全実行の `total` 行を実行 ID 辞書で取得する（Project 画面・集計用）。
    pub fn execution_usage_total_map(&self) -> Result<HashMap<i64, ExecutionUsageRow>, DbError> {
        Ok(self.list_execution_usage_totals()?.into_iter().collect())
    }

    /// `parent_run_id` が指定値の実行 ID 一覧を返す（子実行のみ。孫は含めない）。
    ///
    /// 実行 1 件の usage を子実行込みでまとめる（`usage_with_children`）ために使う。
    pub fn list_child_execution_ids(&self, parent_run_id: i64) -> Result<Vec<i64>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT id FROM execution_history WHERE parent_run_id = ?1 ORDER BY id")?;
        let rows = stmt
            .query_map(params![parent_run_id], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 実行時のオーバーライド値（Tag, Round, Model, Thinking, Timeout）を一括保存する。
    /// 右SB表示用に実行インスタンス単位で記録する。
    pub fn update_execution_overrides(
        &self,
        id: i64,
        tag: Option<&str>,
        round: Option<i64>,
        global_round: Option<i64>,
        model: Option<&str>,
        thinking: Option<&str>,
        timeout: Option<i32>,
    ) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET tag = ?1, round = ?2, global_round = ?3, \
             model = ?4, thinking = ?5, timeout = ?6 WHERE id = ?7",
            params![tag, round, global_round, model, thinking, timeout, id],
        )?;
        Ok(())
    }

    /// 実行時の provider（`p` 再開用）を保存する。
    pub fn update_execution_provider(&self, id: i64, provider: Option<&str>) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET provider = ?1 WHERE id = ?2",
            params![provider, id],
        )?;
        Ok(())
    }

    /// 実行時のセッション情報（session_dir, pi_command）を保存する。
    /// 右SBのセッション再開機能で使用する。
    pub fn update_execution_session_info(
        &self,
        id: i64,
        session_dir: &str,
        pi_command: &str,
    ) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET session_dir = ?1, pi_command = ?2 WHERE id = ?3",
            params![session_dir, pi_command, id],
        )?;
        Ok(())
    }

    /// Kanban 実行情報を保存する（`--kanban` 実行時のみ使用）。
    ///
    /// * `kanban_card_path` — 実行に使ったカード md の相対パス（`.sai/todo/xxx.md`）
    /// * `execution_mode` — `kanban`（通常実行は `interactive`）
    pub fn update_execution_kanban_info(
        &self,
        id: i64,
        kanban_card_path: Option<&str>,
        execution_mode: &str,
    ) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET kanban_card_path = ?1, execution_mode = ?2 WHERE id = ?3",
            params![kanban_card_path, execution_mode, id],
        )?;
        Ok(())
    }

    /// Kanban→Workflow 移動フラグを設定する（`d` キー用。ファイルは残す）。
    ///
    /// `moved=true` で Workflow 所有（DAG 表示）、`false` に戻す運用は想定しないが
    /// 受け付ける（テスト・手動戻し用）。
    pub fn update_execution_workflow_moved(&self, id: i64, moved: bool) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET workflow_moved = ?1 WHERE id = ?2",
            params![i64::from(moved), id],
        )?;
        Ok(())
    }

    /// Kanban のレビュー済みフラグを設定する（run-tui `e` の review→done で使用）。
    pub fn update_execution_review_done(&self, id: i64, done: bool) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET review_done = ?1 WHERE id = ?2",
            params![i64::from(done), id],
        )?;
        Ok(())
    }

    /// 実行時のラウンド（`round` / `global_round`）だけを更新する。
    ///
    /// `runner: user` の run-tui `e` のように、その他のオーバーライド値を
    /// 保持したままラウンドだけを反映したい場合に使う。
    pub fn update_execution_rounds(
        &self,
        id: i64,
        round: Option<i64>,
        global_round: Option<i64>,
    ) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET round = ?1, global_round = ?2 WHERE id = ?3",
            params![round, global_round, id],
        )?;
        Ok(())
    }

    /// 実行主体（`runner`）を設定する（`pi` / `user` / `cc`）。
    pub fn update_execution_runner(&self, id: i64, runner: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET runner = ?1 WHERE id = ?2",
            params![runner, id],
        )?;
        Ok(())
    }

    /// エージェント側セッション ID（Claude Code の UUID）を設定する。
    ///
    /// pi 実行では使わない（`session_dir` の JSONL パスで再開するため）。
    pub fn update_execution_agent_session_id(&self, id: i64, session_id: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET agent_session_id = ?1 WHERE id = ?2",
            params![session_id, id],
        )?;
        Ok(())
    }

    /// waggle トークンから出力元の実行を検索する。
    ///
    /// `json_each` で `output_waggle_tokens` を厳密一致検索する（2.2 の修正）。
    /// LIKE の `_` ワイルドカード問題を回避する。
    pub fn find_source_execution_by_waggle_token(
        &self,
        token: &str,
    ) -> Result<Option<ExecutionHistory>, DbError> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT eh.id, eh.block_id, eh.status, eh.user_inputs_json, eh.resolved_prompt, \
             eh.input_waggle_tokens, eh.output_waggle_token, eh.output_waggle_tokens, eh.output_type, \
             eh.git_commit_before, eh.git_commit_after, eh.herdr_pane_id, eh.pid, eh.started_at, eh.finished_at, \
             eh.duration_ms, eh.exit_code, eh.error_message, eh.output_content, eh.session_stats_json, \
             eh.tag, eh.round, eh.model, eh.provider, eh.thinking, eh.timeout, eh.session_dir, eh.pi_command, \
             eh.parent_run_id, eh.created_at, eh.global_round, eh.batch_id, eh.finalized, \
             eh.origin_run_id, eh.kanban_card_path, eh.execution_mode, eh.workflow_moved, eh.review_done, eh.runner, eh.agent_session_id \
             FROM execution_history eh, json_each(eh.output_waggle_tokens) \
             WHERE json_each.value = ?1 \
             ORDER BY eh.created_at DESC LIMIT 1",
            params![token],
            |row| {
                Ok(ExecutionHistory {
                    id: Some(row.get(0)?), block_id: row.get(1)?, status: row.get(2)?,
                    user_inputs_json: row.get(3)?, resolved_prompt: row.get(4)?,
                    input_waggle_tokens: row.get(5)?, output_waggle_token: row.get(6)?,
                    output_waggle_tokens: row.get(7)?,
                    output_type: row.get::<_, String>(8).unwrap_or_else(|_| "document".to_string()),
                    git_commit_before: row.get(9)?, git_commit_after: row.get(10)?,
                    herdr_pane_id: row.get(11)?, pid: row.get(12)?,
                    started_at: row.get(13)?, finished_at: row.get(14)?,
                    duration_ms: row.get(15)?, exit_code: row.get(16)?,
                    error_message: row.get(17)?, output_content: row.get(18)?,
                    parent_run_id: row.get(28)?,
                    session_stats_json: row.get(19)?,
                    tag: row.get(20)?, round: row.get(21)?, model: row.get(22)?,
                    provider: row.get(23)?,
                    thinking: row.get(24)?, timeout: row.get(25)?,
                    session_dir: row.get(26)?, pi_command: row.get(27)?,
                    created_at: row.get(29)?,
                    global_round: row.get(30)?,
                    batch_id: row.get(31)?,
                    finalized: row.get::<_, i64>(32).unwrap_or(0) != 0,
                                kanban_card_path: row.get(34).ok(),
                    execution_mode: row.get(35).ok(),
                    workflow_moved: row.get::<_, i64>(36).unwrap_or(0) != 0,
                    review_done: row.get::<_, i64>(37).unwrap_or(0) != 0,
                    runner: row.get::<_, String>(38).unwrap_or_else(|_| "pi".to_string()),
                    agent_session_id: row.get(39).ok(),
                })
            },
        );
        match result {
            Ok(exec) => Ok(Some(exec)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DbError::Rusqlite(e)),
        }
    }

    /// 現在実行中のメインブロック実行を1件取得する（U4・Ctrl+D finalize 用）。
    ///
    /// 条件: `status == 'running'` かつ `parent_run_id IS NULL`（メインブロック）。
    /// 最新（`created_at DESC`）の1件のみ返す。herdr pane は実行中は未確定のため
    /// ここでは対象に含めない（pane 解決は呼び出し元が herdr のラベル照合で行う）。
    pub fn find_active_main_execution(&self) -> Result<Option<ExecutionHistory>, DbError> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} \
             FROM execution_history \
             WHERE status = 'running' AND parent_run_id IS NULL \
             ORDER BY created_at DESC LIMIT 1",
            EXECUTION_SELECT_COLS
        );
        let result = conn.query_row(&sql, [], |row| execution_from_row(row));
        match result {
            Ok(exec) => Ok(Some(exec)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DbError::Rusqlite(e)),
        }
    }

    /// `status='running'` の実行一覧を取得する（起動時の幽霊検出・P1-1 用）。
    ///
    /// デーモン再起動後に取り残されたレコード（幽霊）の検出に使う。
    /// 自動修復は行わない（表示・`daemon recover` による明示確定に留める）。
    pub fn list_running_executions(&self) -> Result<Vec<ExecutionHistory>, DbError> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} FROM execution_history WHERE status = 'running' ORDER BY id",
            EXECUTION_SELECT_COLS
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| execution_from_row(row))?;
        let mut executions = Vec::new();
        for row in rows {
            executions.push(row?);
        }
        Ok(executions)
    }

    /// `error_message` に追記する（既存内容を残す・幽霊確定時の注記用）。
    ///
    /// 既存メッセージがある場合は改行で区切って追記する。
    /// 該当行が無い場合は `DbError::NotFound` を返す。
    pub fn append_execution_error_note(&self, id: i64, note: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE execution_history SET error_message = \
             CASE WHEN error_message IS NULL OR error_message = '' \
             THEN ?1 ELSE error_message || char(10) || ?1 END \
             WHERE id = ?2",
            params![note, id],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound(format!("execution id = {}", id)));
        }
        Ok(())
    }

    /// 利用可能な waggle トークン一覧を取得する。
    ///
    /// `output_waggle_tokens` が空でない完了済み実行から、
    /// 各トークンを `json_each` で個別行に展開して返す。
    /// `source_blocks` / `source_tags` でフィルタ可能。
    ///
    /// U03: `include_running=true` の場合、実行中（running / pending）・
    /// 仮完了（completed 未確定）のトークンも含める。w キーで作りたて文書を選ぶ用。
    /// 既定（false）は完了済みのみ（従来通り）。
    ///
    /// # 引数
    ///
    /// * `source_blocks` — 出力元ブロック名でフィルタ（`None` または空の場合は全ブロック）
    /// * `source_tags` — 出力元ブロックの tag でフィルタ
    /// * `limit` — 取得上限件数
    pub fn list_available_waggle_tokens(
        &self,
        source_blocks: Option<&[String]>,
        source_tags: Option<&[String]>,
        limit: i64,
    ) -> Result<Vec<WaggleTokenEntry>, DbError> {
        self.list_waggle_tokens(source_blocks, source_tags, limit, false)
    }

    /// U03: 実行中を含む waggle トークン一覧。
    ///
    /// `include_running=true` の場合、`status` を問わず `output_waggle_tokens`
    /// が空でない実行を対象にする（running / pending / completed の作りたて文書）。
    /// `false` の場合は従来通り `completed` のみ。
    pub fn list_waggle_tokens(
        &self,
        source_blocks: Option<&[String]>,
        source_tags: Option<&[String]>,
        limit: i64,
        include_running: bool,
    ) -> Result<Vec<WaggleTokenEntry>, DbError> {
        let conn = self.conn.lock().unwrap();

        // 基本クエリ: waggle トークン展開。完了済みのみか、実行中も含むかで分岐。
        let status_filter = if include_running {
            ""
        } else {
            "eh.status = 'completed' AND "
        };
        let mut sql = format!(
            "SELECT json_each.value AS token, bm.name AS block_name, \
             eh.id AS execution_id, COALESCE(eh.round, bm.round) AS round, \
             eh.created_at, eh.output_type, eh.status \
             FROM execution_history eh \
             JOIN block_metadata bm ON bm.id = eh.block_id \
             CROSS JOIN json_each(eh.output_waggle_tokens) \
             WHERE {}eh.output_waggle_tokens IS NOT NULL \
             AND eh.output_waggle_tokens != '[]'",
            status_filter
        );

        // source_blocks フィルタ
        let mut param_values: Vec<String> = Vec::new();
        if let Some(blocks) = source_blocks {
            if !blocks.is_empty() {
                let placeholders: Vec<String> = blocks
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 1))
                    .collect();
                sql.push_str(&format!(" AND bm.name IN ({})", placeholders.join(",")));
                param_values.extend(blocks.iter().cloned());
            }
        }

        // source_tags フィルタ
        if let Some(tags) = source_tags {
            if !tags.is_empty() {
                let base = param_values.len();
                let placeholders: Vec<String> = tags
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", base + i + 1))
                    .collect();
                sql.push_str(&format!(" AND bm.tag IN ({})", placeholders.join(",")));
                param_values.extend(tags.iter().cloned());
            }
        }

        sql.push_str(" ORDER BY eh.created_at DESC");
        let limit_param_idx = param_values.len() + 1;
        sql.push_str(&format!(" LIMIT ?{}", limit_param_idx));

        let mut stmt = conn.prepare(&sql)?;

        // パラメータを `&dyn rusqlite::types::ToSql` のスライスに変換
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = {
            let mut v: Vec<&dyn rusqlite::types::ToSql> = Vec::new();
            for p in &param_values {
                v.push(p as &dyn rusqlite::types::ToSql);
            }
            v.push(&limit as &dyn rusqlite::types::ToSql);
            v
        };

        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            Ok(WaggleTokenEntry {
                token: row.get(0)?,
                block_name: row.get(1)?,
                execution_id: row.get(2)?,
                round: row.get(3)?,
                created_at: row.get(4)?,
                output_type: row
                    .get::<_, Option<String>>(5)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "document".to_string()),
                // SELECT 7列目 eh.status。旧クエリ互換のため取れなければ空
                status: row.get::<_, Option<String>>(6).ok().flatten().unwrap_or_default(),
            })
        })?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    // -----------------------------------------------------------------------
    // Task CRUD — サブタスク管理
    // -----------------------------------------------------------------------

    /// サブタスクを1件作成し、生成された `id` を返す。
    pub fn insert_task(&self, task: &Task) -> Result<i64, DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute(
            "INSERT INTO tasks (parent_run_id, subject, description, status, \
             block_type, inputs_json, depends_on_json, worktree, batch_id, \
             client_id, round, supersedes_batch_id, approval_comment, finalized, \
             deleted_by, deleted_at, cwd, timeout, card_run_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                task.parent_run_id,
                task.subject,
                task.description,
                task.status,
                task.block_type,
                task.inputs_json,
                task.depends_on_json,
                task.worktree,
                task.batch_id,
                task.client_id,
                task.round,
                task.supersedes_batch_id,
                task.approval_comment,
                task.finalized as i64,
                task.deleted_by,
                task.deleted_at,
                task.cwd,
                task.timeout,
                task.card_run_id,
            ],
        )?;

        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// 複数のサブタスクをバッチ挿入する（1 トランザクションで一括書き込み）。
    pub fn insert_tasks_batch(&self, tasks: &[Task]) -> Result<Vec<i64>, DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut ids = Vec::with_capacity(tasks.len());
        for task in tasks {
            tx.execute(
                "INSERT INTO tasks (parent_run_id, subject, description, status, \
                 block_type, inputs_json, depends_on_json, worktree, batch_id, \
                 client_id, round, supersedes_batch_id, approval_comment, finalized, \
                 deleted_by, deleted_at, cwd, timeout, card_run_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
                params![
                    task.parent_run_id,
                    task.subject,
                    task.description,
                    task.status,
                    task.block_type,
                    task.inputs_json,
                    task.depends_on_json,
                    task.worktree,
                    task.batch_id,
                    task.client_id,
                    task.round,
                    task.supersedes_batch_id,
                    task.approval_comment,
                    task.finalized as i64,
                    task.deleted_by,
                    task.deleted_at,
                    task.cwd,
                    task.timeout,
                    task.card_run_id,
                ],
            )?;
            ids.push(tx.last_insert_rowid());
        }
        tx.commit()?;
        Ok(ids)
    }

    /// タスクIDでサブタスクを取得する。
    pub fn get_task_by_id(&self, task_id: i64) -> Result<Task, DbError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            &format!("SELECT {} FROM tasks WHERE id = ?1", TASK_SELECT_COLS),
            params![task_id],
            |row| task_from_row(row),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                DbError::NotFound(format!("task id = {}", task_id))
            }
            other => DbError::Rusqlite(other),
        })
    }

    /// 親実行IDでサブタスク一覧を取得する。
    pub fn list_tasks_by_parent(&self, parent_run_id: i64) -> Result<Vec<Task>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM tasks WHERE parent_run_id = ?1 ORDER BY id",
            TASK_SELECT_COLS
        ))?;

        let rows = stmt.query_map(params![parent_run_id], |row| task_from_row(row))?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }
        Ok(tasks)
    }

    /// バッチIDでサブタスク一覧を取得する。
    pub fn list_tasks_by_batch(&self, batch_id: &str) -> Result<Vec<Task>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM tasks WHERE batch_id = ?1 ORDER BY id",
            TASK_SELECT_COLS
        ))?;

        let rows = stmt.query_map(params![batch_id], |row| task_from_row(row))?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }
        Ok(tasks)
    }

    /// 親実行内の client_id でサブタスクを検索する（依存解決用）。
    pub fn get_task_by_client_id(
        &self,
        parent_run_id: i64,
        client_id: &str,
    ) -> Result<Option<Task>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM tasks WHERE parent_run_id = ?1 AND client_id = ?2 LIMIT 1",
            TASK_SELECT_COLS
        ))?;
        let mut rows =
            stmt.query_map(params![parent_run_id, client_id], |row| task_from_row(row))?;
        match rows.next() {
            Some(Ok(task)) => Ok(Some(task)),
            Some(Err(e)) => Err(DbError::Rusqlite(e)),
            None => Ok(None),
        }
    }

    /// 実行IDでサブタスクを取得する（`tasks.execution_id` で検索。無ければ `None`）。
    ///
    /// `task_manage(send)` のターン送信は同じ実行レコードを再利用するため、
    /// 最新ラウンドの実行IDで紐付くタスクを 1 件返す。
    /// run フォルダの owner 同期（`run_dir::sync_run_owner`）で、
    /// サブタスクが確定済み（finalized / deleted）かどうかの判定に使用する。
    pub fn get_task_by_execution_id(&self, execution_id: i64) -> Result<Option<Task>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM tasks WHERE execution_id = ?1 ORDER BY id LIMIT 1",
            TASK_SELECT_COLS
        ))?;
        let mut rows = stmt.query_map(params![execution_id], |row| task_from_row(row))?;
        match rows.next() {
            Some(Ok(task)) => Ok(Some(task)),
            Some(Err(e)) => Err(DbError::Rusqlite(e)),
            None => Ok(None),
        }
    }

    /// 指定ステータスのサブタスク一覧を取得する（依存解決・昇格判定用）。
    pub fn list_tasks_by_status(&self, statuses: &[&str]) -> Result<Vec<Task>, DbError> {
        let conn = self.conn.lock().unwrap();
        let placeholders = statuses.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM tasks WHERE status IN ({}) ORDER BY id",
            TASK_SELECT_COLS, placeholders
        ))?;

        let rows = stmt.query_map(rusqlite::params_from_iter(statuses), |row| {
            task_from_row(row)
        })?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }
        Ok(tasks)
    }

    /// タスクのステータスを更新する。
    pub fn update_task_status(&self, task_id: i64, status: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![status, task_id],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound(format!("task id = {}", task_id)));
        }
        Ok(())
    }

    /// タスクの実行IDを設定する（task_execute 時に呼ばれる）。
    pub fn update_task_execution_id(&self, task_id: i64, execution_id: i64) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE tasks SET execution_id = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![execution_id, task_id],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound(format!("task id = {}", task_id)));
        }
        Ok(())
    }

    /// タスクの所属実行バッチ（run_batch_id）を設定する。
    pub fn update_task_run_batch(&self, task_id: i64, run_batch_id: i64) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE tasks SET run_batch_id = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![run_batch_id, task_id],
        )?;
        Ok(())
    }

    /// バッチ内の全タスクのステータスを一括更新する（approve_batch / reject_batch 用）。
    pub fn update_batch_status(&self, batch_id: &str, status: &str) -> Result<Vec<i64>, DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        // まず対象タスクのID一覧を取得（stmt のスコープを限定）
        let ids: Vec<i64> = {
            let mut stmt =
                tx.prepare("SELECT id FROM tasks WHERE batch_id = ?1 AND status = 'pending'")?;
            let rows: Vec<i64> = stmt
                .query_map(params![batch_id], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        if ids.is_empty() {
            tx.commit()?;
            return Ok(Vec::new());
        }

        // 一括ステータス更新
        tx.execute(
            "UPDATE tasks SET status = ?1, updated_at = datetime('now') \
             WHERE batch_id = ?2 AND status = 'pending'",
            params![status, batch_id],
        )?;

        tx.commit()?;
        Ok(ids)
    }

    /// バッチIDのタスクを削除する（拒否時）。
    pub fn delete_tasks_by_batch(&self, batch_id: &str) -> Result<(), DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM tasks WHERE batch_id = ?1", params![batch_id])?;
        tx.commit()?;
        Ok(())
    }

    /// タスクの最終確定状態を更新する（task_manage(finalize) 用）。
    ///
    /// メインブロックがサブブロックの完了を確定したことを記録する。
    pub fn update_task_finalized(&self, task_id: i64, finalized: bool) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE tasks SET finalized = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![finalized as i64, task_id],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound(format!("task id = {}", task_id)));
        }
        Ok(())
    }

    /// タスクを削除済みにする（task_manage(delete) / ユーザー削除用）。
    ///
    /// `deleted_by` で削除者（"user" | "main"）を記録する。
    /// 物理削除ではなく `status='deleted'` で履歴を残す。
    pub fn mark_task_deleted(&self, task_id: i64, deleted_by: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE tasks SET status = 'deleted', deleted_by = ?1, \
             deleted_at = datetime('now'), updated_at = datetime('now') WHERE id = ?2",
            params![deleted_by, task_id],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound(format!("task id = {}", task_id)));
        }
        Ok(())
    }

    /// タスクの修正ラウンド番号を更新する（task_create 再送・supersedes 用）。
    pub fn update_task_round(&self, task_id: i64, round: i64) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE tasks SET round = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![round, task_id],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound(format!("task id = {}", task_id)));
        }
        Ok(())
    }

    /// タスクの説明を追記する（task_manage(send) の追加指示用）。
    ///
    /// 新ラウンドのプロンプトにユーザー/メインブロックの追加入力指示を反映する。
    pub fn update_task_description(&self, task_id: i64, description: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE tasks SET description = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![description, task_id],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound(format!("task id = {}", task_id)));
        }
        Ok(())
    }

    /// 指定タスクに依存する（`depends_on` に client_id を含む）待機中タスクを返す。
    ///
    /// 依存解決（完了時昇格）用。`waiting_dependency` 状態のタスクのうち、
    /// `depends_on_json` に引数タスクの `client_id` を含むものを返す。
    /// ※ `queued` は AgentPool の実行上限待ち（依存とは無関係）のため対象外。
    pub fn get_tasks_waiting_on(&self, task_id: i64) -> Result<Vec<Task>, DbError> {
        // 1. 対象タスクの client_id を取得
        let task = self.get_task_by_id(task_id)?;
        let Some(client_id) = task.client_id.as_deref().filter(|c| !c.is_empty()) else {
            // client_id 未設定（旧形式）なら依存解決不可 → 空を返す
            return Ok(Vec::new());
        };

        // 2. 依存待ち状態のタスクを取得し、depends_on に client_id を含むものをフィルタ
        let candidates = self.list_tasks_by_status(&["waiting_dependency"])?;
        Ok(candidates
            .into_iter()
            .filter(|t| {
                parse_depends_on(t.depends_on_json.as_deref())
                    .iter()
                    .any(|d| d == client_id)
            })
            .collect())
    }

    // -------------------------------------------------------------------
    // 実行バッチ（run_batches）操作（U2・R4 案X）
    // -------------------------------------------------------------------

    /// 実行バッチを作成する。`seq` は親メインごとの連番。
    ///
    /// `batch_id` は実行バッチ ID（E1, E2...）、`mb_execution_id` はメイン複製用
    /// 実行レコード ID（接続線の endpoint。原則 Some）。
    pub fn create_run_batch(
        &self,
        parent_run_id: i64,
        batch_id: &str,
        seq: i64,
        mb_execution_id: Option<i64>,
    ) -> Result<i64, DbError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO run_batches (parent_run_id, batch_id, seq, status, mb_execution_id) \
             VALUES (?1, ?2, ?3, 'open', ?4)",
            params![parent_run_id, batch_id, seq, mb_execution_id],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// 親メインごとの次の実行バッチ連番を取得する。
    ///
    /// seq は 1 始まり（最初の実行バッチが seq=1）。
    pub fn next_run_batch_seq(&self, parent_run_id: i64) -> Result<i64, DbError> {
        let conn = self.conn.lock().unwrap();
        let max: Option<i64> = conn
            .query_row(
                "SELECT MAX(seq) FROM run_batches WHERE parent_run_id = ?1",
                params![parent_run_id],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        Ok(max.unwrap_or(0) + 1)
    }

    /// 実行バッチを ID で取得する。
    pub fn get_run_batch(&self, id: i64) -> Result<Option<RunBatch>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, parent_run_id, batch_id, seq, status, mb_execution_id, created_at, finished_at FROM run_batches WHERE id = ?1")?;
        let mut rows = stmt.query_map(params![id], |row| {
            let status: String = row.get(4)?;
            Ok(RunBatch {
                id: Some(row.get(0)?),
                parent_run_id: row.get(1)?,
                batch_id: row.get(2)?,
                seq: row.get(3)?,
                status: RunBatchStatus::from_str(&status),
                mb_execution_id: row.get(5)?,
                created_at: row.get(6)?,
                finished_at: row.get(7)?,
            })
        })?;
        match rows.next() {
            Some(Ok(b)) => Ok(Some(b)),
            Some(Err(e)) => Err(DbError::Rusqlite(e)),
            None => Ok(None),
        }
    }

    /// 親メインの実行バッチ一覧（連番昇順）を取得する。
    pub fn list_run_batches(&self, parent_run_id: i64) -> Result<Vec<RunBatch>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, parent_run_id, batch_id, seq, status, mb_execution_id, created_at, finished_at FROM run_batches WHERE parent_run_id = ?1 ORDER BY seq")?;
        let rows = stmt.query_map(params![parent_run_id], |row| {
            let status: String = row.get(4)?;
            Ok(RunBatch {
                id: Some(row.get(0)?),
                parent_run_id: row.get(1)?,
                batch_id: row.get(2)?,
                seq: row.get(3)?,
                status: RunBatchStatus::from_str(&status),
                mb_execution_id: row.get(5)?,
                created_at: row.get(6)?,
                finished_at: row.get(7)?,
            })
        })?;
        let mut v = Vec::new();
        for r in rows {
            v.push(r?);
        }
        Ok(v)
    }

    /// 全実行バッチ（メイン複製 mb_execution_id 対応表）を取得する（TUI 用）。
    ///
    /// 返り値は `(mb_execution_id)` 一覧。`mb_execution_id` が設定されている実行のみ。
    pub fn list_all_run_batch_mb_execs(&self) -> Result<Vec<i64>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT mb_execution_id FROM run_batches WHERE mb_execution_id IS NOT NULL")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        let mut v = Vec::new();
        for r in rows {
            v.push(r?);
        }
        Ok(v)
    }

    /// 全実行バッチを取得する（TUI 用。フェーズノード合成に使用）。
    pub fn list_all_run_batches(&self) -> Result<Vec<RunBatch>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, parent_run_id, batch_id, seq, status, mb_execution_id, created_at, finished_at \
             FROM run_batches ORDER BY parent_run_id, seq",
        )?;
        let rows = stmt.query_map([], |row| {
            let status: String = row.get(4)?;
            Ok(RunBatch {
                id: Some(row.get(0)?),
                parent_run_id: row.get(1)?,
                batch_id: row.get(2)?,
                seq: row.get(3)?,
                status: RunBatchStatus::from_str(&status),
                mb_execution_id: row.get(5)?,
                created_at: row.get(6)?,
                finished_at: row.get(7)?,
            })
        })?;
        let mut v = Vec::new();
        for r in rows {
            v.push(r?);
        }
        Ok(v)
    }

    /// 親メインの最新の open 実行バッチを finalized にする（sai-write 検出時）。
    ///
    /// TUI は finalized 実行バッチの存在をもって最終フェーズノード（P_n = sai-write 由来）を
    /// 合成する。更新できた場合は `true` を返す。
    pub fn finalize_latest_open_run_batch(&self, parent_run_id: i64) -> Result<bool, DbError> {
        let conn = self.conn.lock().unwrap();
        let n = conn
            .execute(
                "UPDATE run_batches SET status = 'finalized', finished_at = datetime('now') \
                 WHERE id = (SELECT id FROM run_batches \
                             WHERE parent_run_id = ?1 AND status = 'open' \
                             ORDER BY seq DESC LIMIT 1)",
                params![parent_run_id],
            )
            .map_err(DbError::Rusqlite)?;
        Ok(n > 0)
    }

    /// 実行バッチのステータスを更新する。
    pub fn update_run_batch_status(&self, id: i64, status: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap();
        let finished = if status == "finalized" {
            ", finished_at = datetime('now')"
        } else {
            ""
        };
        let sql = format!(
            "UPDATE run_batches SET status = ?1{}\
             WHERE id = ?2",
            finished
        );
        conn.execute(&sql, params![status, id])?;
        Ok(())
    }

    /// タスクの所属実行バッチを取得する（task_output の逆引き用）。
    ///
    /// `tasks.run_batch_id` が設定されていれば、その実行バッチを返す。
    /// ※ 内部で `get_run_batch` を呼ばず、同一ロック内で行を直接読む
    ///   （Mutex は非再帰のため、ロック保持中に再ロックすると自己デッドロックする）。
    pub fn find_task_run_batch(&self, task_id: i64) -> Result<Option<RunBatch>, DbError> {
        let conn = self.conn.lock().unwrap();
        // tasks.run_batch_id を取得
        let run_batch_id: Option<i64> = conn
            .query_row(
                "SELECT run_batch_id FROM tasks WHERE id = ?1",
                params![task_id],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        let Some(rbid) = run_batch_id else {
            return Ok(None);
        };
        // run_batches の行を同じロック内で直接読む
        let mut stmt = conn.prepare("SELECT id, parent_run_id, batch_id, seq, status, mb_execution_id, created_at, finished_at FROM run_batches WHERE id = ?1")?;
        let mut rows = stmt.query_map(params![rbid], |row| {
            let status: String = row.get(4)?;
            Ok(RunBatch {
                id: Some(row.get(0)?),
                parent_run_id: row.get(1)?,
                batch_id: row.get(2)?,
                seq: row.get(3)?,
                status: RunBatchStatus::from_str(&status),
                mb_execution_id: row.get(5)?,
                created_at: row.get(6)?,
                finished_at: row.get(7)?,
            })
        })?;
        match rows.next() {
            Some(Ok(b)) => Ok(Some(b)),
            Some(Err(e)) => Err(DbError::Rusqlite(e)),
            None => Ok(None),
        }
    }

    // -------------------------------------------------------------------
    // Project 統計集計 API（R16 / R17 / R19 / R20 / R21）
    // -------------------------------------------------------------------

    /// 全実行レコード（parent_run_id 不問）を取得する。
    ///
    /// セッション検索（`search` モジュール）からも使用するため公開している。
    pub fn list_all_executions(&self) -> Result<Vec<ExecutionHistory>, DbError> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} FROM execution_history ORDER BY created_at",
            EXECUTION_SELECT_COLS
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| execution_from_row(row))?;
        let mut executions = Vec::new();
        for row in rows {
            executions.push(row?);
        }
        Ok(executions)
    }

    /// 全ブロックの (id, name, model) を取得する内部ヘルパー。
    fn load_block_info(&self) -> Result<Vec<(i64, String, String)>, DbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, name, model FROM block_metadata")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut blocks = Vec::new();
        for row in rows {
            blocks.push(row?);
        }
        Ok(blocks)
    }

    /// モデル別・ブロック別のプロジェクト統計を取得する（R20）。
    ///
    /// 全実行（メイン・サブタスクを問わない）を対象に、実行数・実行時間の合計・
    /// input / output / cache トークン合計をモデル別とブロック別で集計する。
    /// モデルは実行時オーバーライド（`execution_history.model`）を優先し、
    /// なければブロック定義のモデルを使用する。token は `execution_usage` の
    /// `total` 行（provider の実際の値）を優先し、未保存の旧実行は
    /// `session_stats_json` の近似値へフォールバックする（L1）。
    pub fn aggregate_project_stats(&self) -> Result<ProjectAggregation, DbError> {
        let executions = self.list_all_executions()?;
        let usage_map = self.execution_usage_total_map()?;
        let blocks = self.load_block_info()?;
        let block_map: HashMap<i64, (String, String)> = blocks
            .into_iter()
            .map(|(id, name, model)| (id, (name, model)))
            .collect();

        let mut by_model: HashMap<String, ModelAggregation> = HashMap::new();
        let mut by_block: HashMap<i64, BlockAggregation> = HashMap::new();

        for exec in &executions {
            let (input, output, cache, _cost) = exec_usage_totals(exec, &usage_map);
            let duration = exec.duration_ms.unwrap_or(0);

            // モデル名の決定（実行時オーバーライド優先 → ブロックのモデル → unknown）
            let block_model = block_map
                .get(&exec.block_id)
                .map(|(_, m)| m.clone())
                .unwrap_or_default();
            let model = exec
                .model
                .as_deref()
                .filter(|m| !m.is_empty())
                .map(String::from)
                .unwrap_or_else(|| {
                    if block_model.is_empty() {
                        "unknown".to_string()
                    } else {
                        block_model.clone()
                    }
                });

            let model_stats = by_model.entry(model.clone()).or_default();
            model_stats.model = model.clone();
            model_stats.execution_count += 1;
            model_stats.total_duration_ms += duration;
            model_stats.input_tokens += input;
            model_stats.output_tokens += output;
            model_stats.cache_tokens += cache;

            let block_name = block_map
                .get(&exec.block_id)
                .map(|(n, _)| n.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let block_stats = by_block.entry(exec.block_id).or_default();
            block_stats.block_id = exec.block_id;
            block_stats.block_name = block_name;
            block_stats.execution_count += 1;
            block_stats.total_duration_ms += duration;
            block_stats.input_tokens += input;
            block_stats.output_tokens += output;
            block_stats.cache_tokens += cache;
        }

        let mut by_model = by_model.into_values().collect::<Vec<_>>();
        by_model.sort_by(|a, b| a.model.cmp(&b.model));
        let mut by_block = by_block.into_values().collect::<Vec<_>>();
        by_block.sort_by(|a, b| a.block_id.cmp(&b.block_id));

        Ok(ProjectAggregation { by_model, by_block })
    }

    /// tool / message / think ごとのトークン合算を全ブロックに渡って返す（R21）。
    ///
    /// 全実行（メイン・サブタスクを問わない）の `session_stats_json` の
    /// `token_usage_by_category` をカテゴリ別に合算する。新フィールド不在・
    /// パース不能の実行は対象に含めず、cache 取得不能は 0 として扱う。
    pub fn token_usage_summary(&self) -> Result<TokenCategoryTotals, DbError> {
        let executions = self.list_all_executions()?;
        let mut totals = TokenCategoryTotals::default();
        for exec in &executions {
            let usage = parse_category_token_usage(exec.session_stats_json.as_deref());
            totals.tool.input += usage.tool.input;
            totals.tool.output += usage.tool.output;
            totals.tool.cache += usage.tool.cache;
            totals.message.input += usage.message.input;
            totals.message.output += usage.message.output;
            totals.message.cache += usage.message.cache;
            totals.think.input += usage.think.input;
            totals.think.output += usage.think.output;
            totals.think.cache += usage.think.cache;
        }
        Ok(totals)
    }

    /// プロジェクト全体の実行サマリを返す（Project ダッシュボード用）。
    ///
    /// `execution_history` を status で GROUP BY し、実行数・総所要時間・
    /// finalize 済み数を集計する。
    pub fn project_run_summary(&self) -> Result<ProjectRunSummary, DbError> {
        // 1 パス集計: 実行ごとにステータス・所要時間・トークン・コストを合算する
        let executions = self.list_all_executions()?;
        let usage_map = self.execution_usage_total_map()?;
        let mut summary = ProjectRunSummary::default();
        for exec in &executions {
            summary.total += 1;
            match exec.status.as_str() {
                "completed" => summary.completed += 1,
                "failed" => summary.failed += 1,
                "running" => summary.running += 1,
                "pending" => summary.pending += 1,
                _ => {}
            }
            summary.total_duration_ms += exec.duration_ms.unwrap_or(0);
            if exec.finalized {
                summary.finalized += 1;
            }
            let (input, output, cache, cost) = exec_usage_totals(exec, &usage_map);
            summary.total_tokens += input + output + cache;
            summary.total_cost_usd += cost;
        }
        Ok(summary)
    }

    /// 日別・ブロック別のトークン合計を返す（R19）。
    ///
    /// 全実行を対象とし、各実行の token（input + output + cache）を算出し、
    /// 日付（`started_at`、無ければ `created_at` の日部分）+ ブロック名で合算する。
    /// token は `execution_usage` の `total` 行（provider の実際の値）を優先し、
    /// 未保存の旧実行は `session_stats_json` の近似値へフォールバックする（L1）。
    /// トークン合計が 0 の実行は集計から除外する。
    pub fn daily_block_tokens(&self) -> Result<Vec<DailyBlockTokens>, DbError> {
        let executions = self.list_all_executions()?;
        let usage_map = self.execution_usage_total_map()?;
        let blocks = self.load_block_info()?;
        let block_map: HashMap<i64, String> = blocks
            .into_iter()
            .map(|(id, name, _model)| (id, name))
            .collect();

        // (date, block_name) → total
        let mut map: HashMap<(String, String), u64> = HashMap::new();
        for exec in &executions {
            let (input, output, cache, _cost) = exec_usage_totals(exec, &usage_map);
            let total = input + output + cache;
            if total == 0 {
                continue;
            }
            let date = exec
                .started_at
                .as_deref()
                .or_else(|| Some(exec.created_at.as_str()))
                .and_then(|s| s.get(0..10))
                .unwrap_or("unknown")
                .to_string();
            let block_name = block_map
                .get(&exec.block_id)
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            *map.entry((date.clone(), block_name.clone())).or_insert(0) += total;
        }

        let mut rows: Vec<DailyBlockTokens> = map
            .into_iter()
            .map(|((date, block_name), total_tokens)| DailyBlockTokens {
                date,
                block_name,
                total_tokens,
            })
            .collect();
        rows.sort_by(|a, b| a.date.cmp(&b.date).then(a.block_name.cmp(&b.block_name)));
        Ok(rows)
    }

    /// プロジェクト開始日（全実行の最初の開始日時）を返す（R16）。
    ///
    /// 実行が無い場合や開始日時が未設定の場合は `None` を返す。
    /// 戻り値は日付部分（YYYY-MM-DD）。
    pub fn project_start_date(&self) -> Result<Option<String>, DbError> {
        let conn = self.conn.lock().unwrap();
        let earliest: Option<String> = conn
            .query_row(
                "SELECT started_at FROM execution_history \
                 WHERE started_at IS NOT NULL \
                 ORDER BY started_at ASC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        Ok(earliest.and_then(|s| s.get(0..10).map(String::from)))
    }

    /// Recent 実行（既定 5 件）を取得する（R17）。
    ///
    /// 既存の `list_recent_executions` を再利用し、既定件数を 5 とする。
    pub fn list_recent_executions_default(&self) -> Result<Vec<ExecutionHistory>, DbError> {
        self.list_recent_executions(5)
    }

    /// 最新のメインブロック実行を取得する（R18 / Run Next 用）。
    ///
    /// メインブロックは `parent_run_id IS NULL`（サブタスクでない）実行を指す。
    /// うち最新の 1 件を返す。メインブロック実行が無い場合は `Ok(None)`。
    ///
    /// Run Next は「メインブロック末端の Reflection の次アクション欄」から抽出する
    /// （KTD6）。このメソッドで対象実行 ID を特定するために使用する。
    pub fn latest_main_execution(&self) -> Result<Option<ExecutionHistory>, DbError> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} FROM execution_history \
             WHERE parent_run_id IS NULL \
             ORDER BY created_at DESC LIMIT 1",
            EXECUTION_SELECT_COLS
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query_map([], |row| execution_from_row(row))?;
        if let Some(row) = rows.next() {
            Ok(Some(row?))
        } else {
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::test_helpers::*;

    #[test]
    fn test_insert_and_get_block() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("test-block-1");
        let id = store.insert_block(&block).unwrap();
        assert!(id > 0);

        let fetched = store.get_block_by_id(id).unwrap();
        assert_eq!(fetched.name, "test-block-1");
        assert_eq!(fetched.description, "テスト用ブロック");
    }

    #[test]
    fn test_duplicate_name_error() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("duplicate-test");
        store.insert_block(&block).unwrap();

        let dup = create_test_block("duplicate-test");
        let result = store.insert_block(&dup);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DbError::DuplicateName(_)));
    }

    #[test]
    fn test_insert_inputs_and_retrieve() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("input-test-block");
        let block_id = store.insert_block(&block).unwrap();

        let inputs = vec![
            create_test_input(block_id, "field1"),
            create_test_input(block_id, "field2"),
        ];
        store.insert_inputs(block_id, &inputs).unwrap();

        let fetched = store.get_inputs_by_block_id(block_id).unwrap();
        assert_eq!(fetched.len(), 2);
        assert_eq!(fetched[0].input_id, "field1");
        assert_eq!(fetched[1].input_id, "field2");
    }

    #[test]
    fn test_insert_output_and_retrieve() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("output-test-block");
        let block_id = store.insert_block(&block).unwrap();

        let output = create_test_output(block_id);
        let output_id = store.insert_output(&output).unwrap();
        assert!(output_id > 0);

        let fetched = store.get_output_by_block_id(block_id).unwrap();
        assert_eq!(fetched.format, "markdown");
        assert!(fetched.waggle_tokenize);
    }

    #[test]
    fn test_create_and_update_execution() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("exec-test-block");
        let block_id = store.insert_block(&block).unwrap();

        let exec_id = store.create_execution(block_id, None).unwrap();
        assert!(exec_id > 0);

        let fetched = store.get_execution_by_id(exec_id).unwrap();
        assert_eq!(fetched.status, "pending");
        assert_eq!(fetched.block_id, block_id);
    }

    #[test]
    fn test_execution_lifecycle() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("lifecycle-test");
        let block_id = store.insert_block(&block).unwrap();

        // pending → running
        let exec_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(exec_id, "running").unwrap();
        let running = store.get_execution_by_id(exec_id).unwrap();
        assert_eq!(running.status, "running");
        assert!(running.started_at.is_some());

        // running → completed
        store.update_execution_status(exec_id, "completed").unwrap();
        let completed = store.get_execution_by_id(exec_id).unwrap();
        assert_eq!(completed.status, "completed");
        assert!(completed.finished_at.is_some());
        // duration_ms が設定されている（値は環境依存のため存在確認のみ）
        assert!(completed.duration_ms.is_some());
    }

    /// list_waggle_tokens の SQL が両分岐とも組み立てられ、
    /// 完了済みのみ / 実行中も含む の絞り込みが効くこと。
    #[test]
    fn test_list_waggle_tokens_include_running() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_test_block("waggle-list")).unwrap();
        let done = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(done, "completed").unwrap();
        store.update_execution_waggle_tokens(done, &["DONE0001".to_string()]).unwrap();
        let running = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(running, "running").unwrap();
        store.update_execution_waggle_tokens(running, &["RUN00001".to_string()]).unwrap();

        let completed: Vec<String> = store
            .list_waggle_tokens(None, None, -1, false)
            .unwrap()
            .into_iter()
            .map(|e| e.token)
            .collect();
        assert_eq!(completed, vec!["DONE0001".to_string()]);

        let all = store.list_waggle_tokens(None, None, -1, true).unwrap();
        assert_eq!(all.len(), 2);
    }

    /// U4: Ctrl+D finalize の finalize フラグ（update_execution_finalized）が
    /// 保存・復元できること。
    #[test]
    fn test_update_execution_finalized() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("finalized-test");
        let block_id = store.insert_block(&block).unwrap();

        let exec_id = store.create_execution(block_id, None).unwrap();

        // 初期値は false
        let fetched = store.get_execution_by_id(exec_id).unwrap();
        assert!(!fetched.finalized);

        // true に更新できる
        store.update_execution_finalized(exec_id, true).unwrap();
        let fetched = store.get_execution_by_id(exec_id).unwrap();
        assert!(fetched.finalized);

        // false に戻せる
        store.update_execution_finalized(exec_id, false).unwrap();
        let fetched = store.get_execution_by_id(exec_id).unwrap();
        assert!(!fetched.finalized);
    }

    /// U4: find_active_main_execution が「実行中のメインブロック（parent_run_id なし）」を
    /// 返すこと。サブブロック（parent_run_id あり）や完了済みは対象外。
    #[test]
    fn test_find_active_main_execution() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("active-main-test");
        let block_id = store.insert_block(&block).unwrap();

        // 実行中のメインブロック（対象）
        let main_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(main_id, "running").unwrap();

        // 完了済みメインブロック（対象外）
        let done_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(done_id, "completed").unwrap();

        // 実行中のサブブロック（parent_run_id あり・対象外）
        let sub_id = store
            .create_execution_for_subtask(block_id, None, main_id, Some("batch-test"))
            .unwrap();
        store.update_execution_status(sub_id, "running").unwrap();

        let active = store.find_active_main_execution().unwrap();
        let active = active.expect("実行中のメインブロックが検出されるべき");
        assert_eq!(active.id, Some(main_id));
        assert_eq!(active.status, "running");
        assert!(active.parent_run_id.is_none());
    }

    /// U4: 実行中のメインブロックが無い場合は None を返すこと。
    #[test]
    fn test_find_active_main_execution_none() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("no-active-main-test");
        let block_id = store.insert_block(&block).unwrap();

        // 実行中レコードなし（pending のみ）→ None
        let _id = store.create_execution(block_id, None).unwrap();
        assert!(store.find_active_main_execution().unwrap().is_none());

        // 実行中のサブブロックのみ → None（メインブロックではない）
        let main_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(main_id, "running").unwrap();
        let sub_id = store
            .create_execution_for_subtask(block_id, None, main_id, Some("batch-test"))
            .unwrap();
        store.update_execution_status(sub_id, "running").unwrap();
        let active = store.find_active_main_execution().unwrap();
        // メインブロック（main_id）が実行中のため Some(main_id) になる
        assert_eq!(active.expect("メインブロックが実行中").id, Some(main_id));
    }

    #[test]
    fn test_insert_and_get_commit() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("commit-test-block");
        let block_id = store.insert_block(&block).unwrap();
        let exec_id = store.create_execution(block_id, None).unwrap();

        let commit = create_test_commit(exec_id, "abc123def456");
        let commit_id = store.insert_commit(&commit).unwrap();
        assert!(commit_id > 0);

        let fetched_by_hash = store.get_commit_by_hash("abc123def456").unwrap();
        assert_eq!(fetched_by_hash.execution_id, exec_id);
        assert_eq!(
            fetched_by_hash.commit_message.as_deref(),
            Some("テストコミット")
        );

        let commits = store.get_commits_by_execution(exec_id).unwrap();
        assert_eq!(commits.len(), 1);
    }

    #[test]
    fn test_cascade_delete() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("cascade-test");
        let block_id = store.insert_block(&block).unwrap();

        // 入力と出力を追加
        let inputs = vec![create_test_input(block_id, "cascade-field")];
        store.insert_inputs(block_id, &inputs).unwrap();
        let output = create_test_output(block_id);
        store.insert_output(&output).unwrap();

        // ブロックを削除 → 入力・出力も CASCADE 削除される
        store.delete_block(block_id).unwrap();

        let inputs_after = store.get_inputs_by_block_id(block_id).unwrap();
        assert!(inputs_after.is_empty());

        let output_result = store.get_output_by_block_id(block_id);
        assert!(output_result.is_err());
        assert!(matches!(output_result.unwrap_err(), DbError::NotFound(_)));
    }

    #[test]
    fn test_open_in_memory() {
        let store = SaiStore::open_in_memory().unwrap();
        let blocks = store.list_blocks().unwrap();
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_get_block_by_name() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("name-lookup");
        let id = store.insert_block(&block).unwrap();

        let fetched = store.get_block_by_name("name-lookup").unwrap();
        assert_eq!(fetched.id, Some(id));
    }

    #[test]
    fn test_update_block() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("update-me");
        let id = store.insert_block(&block).unwrap();

        let mut updated = block;
        updated.id = Some(id);
        updated.description = "更新された説明".to_string();
        store.update_block(&updated).unwrap();

        let fetched = store.get_block_by_id(id).unwrap();
        assert_eq!(fetched.description, "更新された説明");
    }

    #[test]
    fn test_list_blocks() {
        let store = SaiStore::open_in_memory().unwrap();
        store.insert_block(&create_test_block("block-a")).unwrap();
        store.insert_block(&create_test_block("block-b")).unwrap();

        let blocks = store.list_blocks().unwrap();
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn test_list_recent_executions() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("recent-exec");
        let block_id = store.insert_block(&block).unwrap();

        store.create_execution(block_id, None).unwrap();
        store.create_execution(block_id, None).unwrap();
        store.create_execution(block_id, None).unwrap();

        let recent = store.list_recent_executions(2).unwrap();
        assert_eq!(recent.len(), 2);
    }

    #[test]
    fn test_update_execution_error() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("error-test");
        let block_id = store.insert_block(&block).unwrap();
        let exec_id = store.create_execution(block_id, None).unwrap();

        store
            .update_execution_error(exec_id, "致命的なエラーが発生しました", 1)
            .unwrap();

        let fetched = store.get_execution_by_id(exec_id).unwrap();
        assert_eq!(fetched.status, "failed");
        assert_eq!(fetched.exit_code, Some(1));
        assert_eq!(
            fetched.error_message.as_deref(),
            Some("致命的なエラーが発生しました")
        );
    }

    #[test]
    fn test_get_block_not_found() {
        let store = SaiStore::open_in_memory().unwrap();
        let result = store.get_block_by_id(99999);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DbError::NotFound(_)));
    }

    #[test]
    fn test_update_execution_output_content() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("output-content-test");
        let block_id = store.insert_block(&block).unwrap();
        let exec_id = store.create_execution(block_id, None).unwrap();

        let sample_output = "# 出力レポート\n\nこれはテスト出力です。\n\n- 項目A\n- 項目B\n\n```rust\nfn main() {}\n```";
        store
            .update_execution_output_content(exec_id, sample_output)
            .unwrap();

        let fetched = store.get_execution_by_id(exec_id).unwrap();
        assert_eq!(fetched.output_content.as_deref(), Some(sample_output));
    }

    #[test]
    fn test_execution_lifecycle_with_output() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("lifecycle-output-test");
        let block_id = store.insert_block(&block).unwrap();

        let exec_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(exec_id, "running").unwrap();
        store.update_execution_status(exec_id, "completed").unwrap();

        // 出力内容を保存
        let output = "エージェントの最終出力テキスト\n複数行にわたる\n出力です。";
        store
            .update_execution_output_content(exec_id, output)
            .unwrap();

        let fetched = store.get_execution_by_id(exec_id).unwrap();
        assert_eq!(fetched.status, "completed");
        assert_eq!(fetched.output_content.as_deref(), Some(output));
    }

    #[test]
    fn test_output_content_null_by_default() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("null-output-test");
        let block_id = store.insert_block(&block).unwrap();
        let exec_id = store.create_execution(block_id, None).unwrap();

        let fetched = store.get_execution_by_id(exec_id).unwrap();
        assert_eq!(fetched.output_content, None);
    }

    #[test]
    fn test_output_content_large_text() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("large-output-test");
        let block_id = store.insert_block(&block).unwrap();
        let exec_id = store.create_execution(block_id, None).unwrap();

        let large_output = "あ".repeat(10_000); // 10,000 文字の日本語テキスト
        store
            .update_execution_output_content(exec_id, &large_output)
            .unwrap();

        let fetched = store.get_execution_by_id(exec_id).unwrap();
        assert_eq!(
            fetched.output_content.as_deref(),
            Some(large_output.as_str())
        );
        assert_eq!(fetched.output_content.unwrap().chars().count(), 10_000);
    }

    #[test]
    fn test_output_content_multibyte() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("multibyte-output-test");
        let block_id = store.insert_block(&block).unwrap();
        let exec_id = store.create_execution(block_id, None).unwrap();

        let output = "🎉 こんにちは世界！\n## マークダウン\n絵文字: 🚀💡✨\nコード: `let x = 42;`";
        store
            .update_execution_output_content(exec_id, output)
            .unwrap();

        let fetched = store.get_execution_by_id(exec_id).unwrap();
        assert_eq!(fetched.output_content.as_deref(), Some(output));
    }

    // ---- BlockConnection CRUD ----

    #[test]
    fn test_insert_and_list_connection() {
        let store = SaiStore::open_in_memory().unwrap();

        // 2つのブロックを作成
        let block_a = create_test_block("source-block");
        let block_b = create_test_block("target-block");
        let source_block_id = store.insert_block(&block_a).unwrap();
        let target_block_id = store.insert_block(&block_b).unwrap();

        // 実行を作成
        let source_exec_id = store.create_execution(source_block_id, None).unwrap();
        let target_exec_id = store.create_execution(target_block_id, None).unwrap();

        // waggle トークンを出力側に設定
        let token = "AbCdEfGh";
        store
            .update_execution_waggle_tokens(source_exec_id, &[token.to_string()])
            .unwrap();

        // 接続を挿入
        let conn = BlockConnection::new_waggle(
            source_exec_id,
            source_block_id,
            target_exec_id,
            target_block_id,
            token.to_string(),
        );
        let conn_id = store.insert_connection(&conn).unwrap();
        assert!(conn_id > 0);

        // ブロック単位で取得
        let connections = store.list_connections_by_block(source_block_id).unwrap();
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].waggle_token.as_deref(), Some(token));

        // グルーピング
        let groups = store.list_connections_grouped().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].source_block_id, source_block_id);
        assert_eq!(groups[0].target_block_id, target_block_id);
        assert_eq!(groups[0].connection_count, 1);
    }

    #[test]
    fn test_connection_idempotent() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_a = create_test_block("idem-source");
        let block_b = create_test_block("idem-target");
        let source_block_id = store.insert_block(&block_a).unwrap();
        let target_block_id = store.insert_block(&block_b).unwrap();
        let source_exec_id = store.create_execution(source_block_id, None).unwrap();
        let target_exec_id = store.create_execution(target_block_id, None).unwrap();

        let conn = BlockConnection::new_waggle(
            source_exec_id,
            source_block_id,
            target_exec_id,
            target_block_id,
            "TokenToken".to_string(),
        );

        // 2回挿入しても重複しない
        let id1 = store.insert_connection(&conn).unwrap();
        let id2 = store.insert_connection(&conn).unwrap();
        assert!(id1 > 0);
        assert_eq!(id2, 0); // 2回目はスキップ

        let connections = store.list_connections_by_block(source_block_id).unwrap();
        assert_eq!(connections.len(), 1);
    }

    #[test]
    fn test_find_source_execution_by_waggle_token() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("token-search-block");
        let block_id = store.insert_block(&block).unwrap();
        let exec_id = store.create_execution(block_id, None).unwrap();

        let token = "XxYyZzWw";
        store
            .update_execution_waggle_tokens(exec_id, &[token.to_string()])
            .unwrap();

        // トークンから実行を検索
        let found = store.find_source_execution_by_waggle_token(token).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().block_id, block_id);

        // 存在しないトークン
        let not_found = store
            .find_source_execution_by_waggle_token("NoSuchTok")
            .unwrap();
        assert!(not_found.is_none());
    }

    /// P3-A: `create_execution_for_subtask` で batch_id が保存され、
    /// `get_execution_by_id` で取得できることを確認する。
    #[test]
    fn test_create_execution_for_subtask_with_batch_id() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = create_test_block("batch-sub");
        let block_id = store.insert_block(&block).unwrap();
        let parent_id = store.create_execution(block_id, None).unwrap();

        // batch_id 付きサブ実行
        let sub_id = store
            .create_execution_for_subtask(block_id, None, parent_id, Some("batch_1_r2"))
            .unwrap();
        let sub = store.get_execution_by_id(sub_id).unwrap();
        assert_eq!(sub.parent_run_id, Some(parent_id), "親実行IDが保存される");
        assert_eq!(
            sub.batch_id.as_deref(),
            Some("batch_1_r2"),
            "batch_id が保存される（P3-A）"
        );

        // batch_id なし（None）でも作成できる
        let sub2 = store
            .create_execution_for_subtask(block_id, None, parent_id, None)
            .unwrap();
        let sub2 = store.get_execution_by_id(sub2).unwrap();
        assert_eq!(sub2.batch_id, None, "batch_id 未指定は None");

        // list_recent_executions にも batch_id が含まれる
        let recent = store.list_recent_executions(10).unwrap();
        let found = recent.iter().find(|e| e.id == Some(sub_id)).unwrap();
        assert_eq!(
            found.batch_id.as_deref(),
            Some("batch_1_r2"),
            "一覧取得でも batch_id が取得できる"
        );
    }

    #[test]
    fn test_connection_from_input_tokens() {
        let store = SaiStore::open_in_memory().unwrap();

        // ブロックA: 出力を生成
        let block_a = create_test_block("producer");
        let source_block_id = store.insert_block(&block_a).unwrap();
        let source_exec_id = store.create_execution(source_block_id, None).unwrap();
        let token = "ProdToken";
        store
            .update_execution_waggle_tokens(source_exec_id, &[token.to_string()])
            .unwrap();

        // ブロックB: ブロックAの出力を入力として使う
        let block_b = create_test_block("consumer");
        let target_block_id = store.insert_block(&block_b).unwrap();
        let target_exec_id = store.create_execution(target_block_id, None).unwrap();
        store
            .update_execution_input_tokens(target_exec_id, &[token.to_string()])
            .unwrap();

        // insert_block_connections を呼び出す
        crate::orchestrator::insert_block_connections(
            &store,
            target_exec_id,
            target_block_id,
            &[token.to_string()],
        );

        // 接続が生成されている
        let connections = store.list_connections_by_block(target_block_id).unwrap();
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].source_block_id, source_block_id);
        assert_eq!(connections[0].waggle_token.as_deref(), Some(token));

        let groups = store.list_connections_grouped().unwrap();
        assert_eq!(groups.len(), 1);
    }

    /// Kanban: `update_execution_kanban_info` でカードパスと実行の種類が保存されること。
    #[test]
    fn test_kanban_info() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store
            .insert_block(&create_test_block("kanban-blk"))
            .unwrap();
        let new_id = store.create_execution(block_id, None).unwrap();
        store
            .update_execution_kanban_info(new_id, Some(".sai/todo/card-x.md"), "kanban")
            .unwrap();
        let exec = store.get_execution_by_id(new_id).unwrap();
        assert_eq!(
            exec.kanban_card_path.as_deref(),
            Some(".sai/todo/card-x.md")
        );
        assert_eq!(exec.execution_mode.as_deref(), Some("kanban"));
    }

    /// 親実行の削除で、サブ実行・タスク・接続がカスケード削除されること。
    #[test]
    fn test_delete_execution_cascade_parent() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store
            .insert_block(&create_test_block("cascade-parent"))
            .unwrap();

        // 親実行
        let parent_id = store.create_execution(block_id, None).unwrap();
        // 子実行（サブタスク）2件
        let child1 = store
            .create_execution_for_subtask(block_id, None, parent_id, None)
            .unwrap();
        let child2 = store
            .create_execution_for_subtask(block_id, None, parent_id, None)
            .unwrap();
        // 委譲接続（親 → 子）
        store
            .insert_connection(&BlockConnection::new_task_delegation(
                parent_id, block_id, child1, block_id,
            ))
            .unwrap();
        // タスク（親実行 / 実行紐付け）
        store
            .insert_task(&Task::new(
                parent_id,
                "タスク1".to_string(),
                "説明".to_string(),
                Some("test-block".to_string()),
                None,
                None,
                "none".to_string(),
                None,
                Some("task-1".to_string()),
            ))
            .unwrap();

        // 親実行を削除 → 子・接続・タスクもまとめて削除される
        let summary = store.delete_execution_cascade(parent_id).unwrap();
        assert_eq!(summary.executions, 3, "親 + 子2件");
        assert_eq!(summary.connections, 1);
        assert_eq!(summary.tasks, 1);

        assert!(store.get_execution_by_id(parent_id).is_err());
        assert!(store.get_execution_by_id(child1).is_err());
        assert!(store.get_execution_by_id(child2).is_err());
        let conns = store.list_connections_by_block(block_id).unwrap();
        assert!(conns.is_empty(), "接続は削除されている");
    }

    /// 子実行（サブタスク）単体の削除でも、参照するタスク・接続が削除されること。
    #[test]
    fn test_delete_execution_cascade_child() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store
            .insert_block(&create_test_block("cascade-child"))
            .unwrap();

        let parent_id = store.create_execution(block_id, None).unwrap();
        let child_id = store
            .create_execution_for_subtask(block_id, None, parent_id, None)
            .unwrap();
        store
            .insert_connection(&BlockConnection::new_task_delegation(
                parent_id, block_id, child_id, block_id,
            ))
            .unwrap();

        // 子実行を削除
        let summary = store.delete_execution_cascade(child_id).unwrap();
        assert_eq!(summary.executions, 1);
        assert_eq!(summary.connections, 1);

        // 親は残る
        assert!(store.get_execution_by_id(parent_id).is_ok());
    }

    /// 存在しない実行IDの削除は NotFound になること。
    #[test]
    fn test_delete_execution_cascade_not_found() {
        let store = SaiStore::open_in_memory().unwrap();
        let err = store.delete_execution_cascade(9999).unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    // ---------------------------------------------------------------------
    // タスクワークフロー拡張（P1）: 新カラム・新 API のテスト
    // ---------------------------------------------------------------------

    /// タスク挿入時に新カラム（client_id / round / supersedes 等）が保存・取得できること。
    #[test]
    fn test_task_insert_with_new_columns() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store
            .insert_block(&create_test_block("task-new-cols"))
            .unwrap();
        let parent_id = store.create_execution(block_id, None).unwrap();

        let mut task = Task::new(
            parent_id,
            "タスク".to_string(),
            "説明".to_string(),
            Some("sai-ui-impl".to_string()),
            Some(r#"{"design_doc":"waggle:AbCd"}"#.to_string()),
            Some(r#"["task-1"]"#.to_string()),
            "isolation".to_string(),
            Some("batch_v2".to_string()),
            Some("task-2".to_string()),
        );
        task.round = 2;
        task.supersedes_batch_id = Some("batch_v1".to_string());
        task.approval_comment = Some("分割して".to_string());

        let id = store.insert_task(&task).unwrap();
        let fetched = store.get_task_by_id(id).unwrap();

        assert_eq!(fetched.client_id.as_deref(), Some("task-2"));
        assert_eq!(fetched.round, 2);
        assert_eq!(fetched.supersedes_batch_id.as_deref(), Some("batch_v1"));
        assert_eq!(fetched.approval_comment.as_deref(), Some("分割して"));
        assert!(!fetched.finalized);
        assert!(fetched.deleted_by.is_none());
        assert_eq!(fetched.batch_id.as_deref(), Some("batch_v2"));
        assert_eq!(fetched.execution_id, None);
    }

    /// list_tasks_by_status が指定ステータスのタスクのみ返すこと。
    #[test]
    fn test_list_tasks_by_status() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store
            .insert_block(&create_test_block("task-status"))
            .unwrap();
        let parent_id = store.create_execution(block_id, None).unwrap();

        let mut approved = Task::new(
            parent_id,
            "A".to_string(),
            "".to_string(),
            None,
            None,
            None,
            "none".to_string(),
            Some("b1".to_string()),
            Some("a".to_string()),
        );
        approved.status = "approved".to_string();
        let mut waiting = Task::new(
            parent_id,
            "W".to_string(),
            "".to_string(),
            None,
            None,
            Some(r#"["a"]"#.to_string()),
            "none".to_string(),
            Some("b1".to_string()),
            Some("w".to_string()),
        );
        waiting.status = "waiting_dependency".to_string();
        let mut queued = Task::new(
            parent_id,
            "Q".to_string(),
            "".to_string(),
            None,
            None,
            None,
            "none".to_string(),
            Some("b1".to_string()),
            Some("q".to_string()),
        );
        queued.status = "queued".to_string();

        store
            .insert_tasks_batch(&[approved, waiting, queued])
            .unwrap();

        let got = store
            .list_tasks_by_status(&["waiting_dependency", "queued"])
            .unwrap();
        assert_eq!(got.len(), 2);
        let got_approved = store.list_tasks_by_status(&["approved"]).unwrap();
        assert_eq!(got_approved.len(), 1);
    }

    /// U3 AC3: `timeout` が insert_tasks_batch → get_task_by_id で往復すること。
    #[test]
    fn test_task_timeout_round_trip() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store
            .insert_block(&create_test_block("task-timeout"))
            .unwrap();
        let parent_id = store.create_execution(block_id, None).unwrap();

        let mut with_timeout = Task::new(
            parent_id,
            "T".to_string(),
            String::new(),
            None,
            None,
            None,
            "none".to_string(),
            Some("b1".to_string()),
            Some("t1".to_string()),
        );
        with_timeout.timeout = Some(900);
        let without_timeout = Task::new(
            parent_id,
            "N".to_string(),
            String::new(),
            None,
            None,
            None,
            "none".to_string(),
            Some("b1".to_string()),
            Some("t2".to_string()),
        );

        let ids = store
            .insert_tasks_batch(&[with_timeout, without_timeout])
            .unwrap();

        assert_eq!(store.get_task_by_id(ids[0]).unwrap().timeout, Some(900));
        assert_eq!(store.get_task_by_id(ids[1]).unwrap().timeout, None);
    }

    /// update_task_finalized / mark_task_deleted / update_task_round が機能すること。
    #[test]
    fn test_task_finalize_delete_round() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_test_block("task-ops")).unwrap();
        let parent_id = store.create_execution(block_id, None).unwrap();

        let id = store
            .insert_task(&Task::new(
                parent_id,
                "タスク".to_string(),
                "".to_string(),
                None,
                None,
                None,
                "none".to_string(),
                Some("b1".to_string()),
                Some("t1".to_string()),
            ))
            .unwrap();

        // finalize
        store.update_task_finalized(id, true).unwrap();
        assert!(store.get_task_by_id(id).unwrap().finalized);
        store.update_task_finalized(id, false).unwrap();
        assert!(!store.get_task_by_id(id).unwrap().finalized);

        // round 更新
        store.update_task_round(id, 3).unwrap();
        assert_eq!(store.get_task_by_id(id).unwrap().round, 3);

        // delete（main による削除）
        store.mark_task_deleted(id, "main").unwrap();
        let deleted = store.get_task_by_id(id).unwrap();
        assert_eq!(deleted.status, "deleted");
        assert_eq!(deleted.deleted_by.as_deref(), Some("main"));
        assert!(deleted.deleted_at.is_some());
    }

    /// get_tasks_waiting_on が depends_on（client_id）で待機タスクを検索できること。
    #[test]
    fn test_get_tasks_waiting_on() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_test_block("task-deps")).unwrap();
        let parent_id = store.create_execution(block_id, None).unwrap();

        // 依存元タスク（client_id = "base"）
        let base_id = store
            .insert_task(&Task::new(
                parent_id,
                "BASE".to_string(),
                "".to_string(),
                None,
                None,
                None,
                "none".to_string(),
                Some("b1".to_string()),
                Some("base".to_string()),
            ))
            .unwrap();

        // 依存先タスク: base に依存する waiting_dependency
        let mut dep = Task::new(
            parent_id,
            "DEP".to_string(),
            "".to_string(),
            None,
            None,
            Some(r#"["base"]"#.to_string()),
            "none".to_string(),
            Some("b1".to_string()),
            Some("dep".to_string()),
        );
        dep.status = "waiting_dependency".to_string();
        let dep_id = store.insert_task(&dep).unwrap();

        // 別タスクに依存する queued タスク（対象外）
        let mut other = Task::new(
            parent_id,
            "OTHER".to_string(),
            "".to_string(),
            None,
            None,
            Some(r#"["zzz"]"#.to_string()),
            "none".to_string(),
            Some("b1".to_string()),
            Some("other".to_string()),
        );
        other.status = "queued".to_string();
        store.insert_task(&other).unwrap();

        let waiting = store.get_tasks_waiting_on(base_id).unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].id, Some(dep_id));

        // client_id 未設定タスクに対する依存検索は空
        let no_client = store
            .insert_task(&Task::new(
                parent_id,
                "NC".to_string(),
                "".to_string(),
                None,
                None,
                None,
                "none".to_string(),
                Some("b1".to_string()),
                None,
            ))
            .unwrap();
        assert!(store.get_tasks_waiting_on(no_client).unwrap().is_empty());
    }

    /// run_batches CRUD と連番（seq）の検証。
    #[test]
    fn test_run_batches_crud() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_test_block("rb-main")).unwrap();
        let parent_id = store.create_execution(block_id, None).unwrap();

        // 最初の seq は 1
        assert_eq!(store.next_run_batch_seq(parent_id).unwrap(), 1);
        let rb1 = store.create_run_batch(parent_id, "E1", 1, None).unwrap();
        // 2 つ目は連番インクリメント
        assert_eq!(store.next_run_batch_seq(parent_id).unwrap(), 2);
        let rb2 = store
            .create_run_batch(parent_id, "E2", 2, Some(999))
            .unwrap();

        let got1 = store.get_run_batch(rb1).unwrap().unwrap();
        assert_eq!(got1.seq, 1);
        assert_eq!(got1.batch_id, "E1");
        assert_eq!(got1.status, RunBatchStatus::Open);

        let list = store.list_run_batches(parent_id).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].seq, 1);
        assert_eq!(list[1].seq, 2);

        // メイン複製実行IDの一覧（TUI 用）
        let execs = store.list_all_run_batch_mb_execs().unwrap();
        assert_eq!(execs, vec![999]);

        // ステータス更新
        store.update_run_batch_status(rb2, "finalized").unwrap();
        assert_eq!(
            store.get_run_batch(rb2).unwrap().unwrap().status,
            RunBatchStatus::Finalized
        );
    }

    // ---------------------------------------------------------------------
    // Project 統計集計（R16 / R17 / R19 / R20 / R21）
    // ---------------------------------------------------------------------

    /// `session_stats_json` 用に category 別トークン使用量を作るテストヘルパー。
    /// tool / message / think の input / output / cache を指定する。
    fn stats_json(
        t_in: u64,
        t_out: u64,
        t_cache: u64,
        m_in: u64,
        m_out: u64,
        m_cache: u64,
        k_in: u64,
        k_out: u64,
        k_cache: u64,
    ) -> String {
        format!(
            r#"{{"token_usage_by_category":{{"tool":{{"input":{t_in},"output":{t_out},"cache":{t_cache}}},"message":{{"input":{m_in},"output":{m_out},"cache":{m_cache}}},"think":{{"input":{k_in},"output":{k_out},"cache":{k_cache}}}}}}}"#,
            t_in = t_in,
            t_out = t_out,
            t_cache = t_cache,
            m_in = m_in,
            m_out = m_out,
            m_cache = m_cache,
            k_in = k_in,
            k_out = k_out,
            k_cache = k_cache,
        )
    }

    /// 実行レコードの各種フィールド（started_at / duration_ms / model / session_stats_json / status）を
    /// 直接更新するテストヘルパー。SQLite の datetime('now') では日付を制御できないため、
    /// テストでは同一モジュール内の private コネクションから直接 UPDATE する。
    fn update_exec_raw(
        store: &SaiStore,
        id: i64,
        started_at: &str,
        duration_ms: i64,
        model: Option<&str>,
        stats_json: Option<&str>,
    ) {
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "UPDATE execution_history SET started_at = ?1, duration_ms = ?2, model = ?3, \
             session_stats_json = ?4, status = 'completed' WHERE id = ?5",
            params![started_at, duration_ms, model, stats_json, id],
        )
        .unwrap();
    }

    /// 実行を作成し、指定フィールドで更新した後の ID を返すテストヘルパー。
    fn make_exec(
        store: &SaiStore,
        block_id: i64,
        started_at: &str,
        duration_ms: i64,
        model: Option<&str>,
        stats_json: Option<&str>,
    ) -> i64 {
        let id = store.create_execution(block_id, None).unwrap();
        update_exec_raw(store, id, started_at, duration_ms, model, stats_json);
        id
    }

    /// R20: モデル別・ブロック別の実行数・実行時間・input / output / cache 合計を検証する。
    #[test]
    fn test_aggregate_project_stats_r20() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_a = store
            .insert_block(&create_test_block("agg-block-a"))
            .unwrap();
        let block_b = store
            .insert_block(&create_test_block("agg-block-b"))
            .unwrap();

        // block_a / model-x: 2 実行
        make_exec(
            &store,
            block_a,
            "2026-01-01 10:00:00",
            100,
            Some("model-x"),
            Some(&stats_json(5, 5, 2, 1, 1, 1, 0, 0, 0)),
        ); // in6 out6 cache3
        make_exec(
            &store,
            block_a,
            "2026-01-02 10:00:00",
            200,
            Some("model-x"),
            Some(&stats_json(0, 0, 0, 10, 5, 0, 0, 0, 0)),
        ); // in10 out5 cache0
           // block_b / model-y: 1 実行
        make_exec(
            &store,
            block_b,
            "2026-01-03 10:00:00",
            300,
            Some("model-y"),
            Some(&stats_json(3, 3, 3, 0, 0, 0, 0, 0, 0)),
        ); // in3 out3 cache3

        let agg = store.aggregate_project_stats().unwrap();

        // モデル別
        assert_eq!(agg.by_model.len(), 2);
        let mx = agg.by_model.iter().find(|m| m.model == "model-x").unwrap();
        assert_eq!(mx.execution_count, 2);
        assert_eq!(mx.total_duration_ms, 300);
        assert_eq!(mx.input_tokens, 16);
        assert_eq!(mx.output_tokens, 11);
        assert_eq!(mx.cache_tokens, 3);
        let my = agg.by_model.iter().find(|m| m.model == "model-y").unwrap();
        assert_eq!(my.execution_count, 1);
        assert_eq!(my.total_duration_ms, 300);
        assert_eq!(my.input_tokens, 3);
        assert_eq!(my.output_tokens, 3);
        assert_eq!(my.cache_tokens, 3);

        // ブロック別
        assert_eq!(agg.by_block.len(), 2);
        let ba = agg.by_block.iter().find(|b| b.block_id == block_a).unwrap();
        assert_eq!(ba.block_name, "agg-block-a");
        assert_eq!(ba.execution_count, 2);
        assert_eq!(ba.total_duration_ms, 300);
        assert_eq!(ba.input_tokens, 16);
        assert_eq!(ba.output_tokens, 11);
        assert_eq!(ba.cache_tokens, 3);
        let bb = agg.by_block.iter().find(|b| b.block_id == block_b).unwrap();
        assert_eq!(bb.block_name, "agg-block-b");
        assert_eq!(bb.execution_count, 1);
        assert_eq!(bb.total_duration_ms, 300);
        assert_eq!(bb.input_tokens, 3);
        assert_eq!(bb.output_tokens, 3);
        assert_eq!(bb.cache_tokens, 3);
    }

    /// R20: 実行時モデル未指定時はブロック定義のモデルで集計されること。
    #[test]
    fn test_aggregate_project_stats_model_fallback() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_a = store
            .insert_block(&create_test_block("agg-fallback"))
            .unwrap();
        // モデル未指定（None）の実行 → ブロックのモデル（create_test_block 既定）で集計
        make_exec(&store, block_a, "2026-01-01 10:00:00", 50, None, None);
        let agg = store.aggregate_project_stats().unwrap();
        assert_eq!(agg.by_model.len(), 1);
        assert_eq!(agg.by_model[0].model, "anthropic/claude-sonnet-4-5");
    }

    /// R19: 日別・ブロック別のトークン合計を検証する。
    #[test]
    fn test_daily_block_tokens_r19() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_a = store
            .insert_block(&create_test_block("daily-block-a"))
            .unwrap();
        let block_b = store
            .insert_block(&create_test_block("daily-block-b"))
            .unwrap();

        // 同日（date1）に block_a / block_b で実行
        make_exec(
            &store,
            block_a,
            "2026-01-01 09:00:00",
            10,
            None,
            Some(&stats_json(0, 0, 0, 4, 6, 0, 0, 0, 0)),
        ); // total 10
        make_exec(
            &store,
            block_b,
            "2026-01-01 11:00:00",
            10,
            None,
            Some(&stats_json(0, 0, 0, 10, 10, 0, 0, 0, 0)),
        ); // total 20
           // 別日（date2）に block_a で実行
        make_exec(
            &store,
            block_a,
            "2026-01-02 09:00:00",
            10,
            None,
            Some(&stats_json(0, 0, 0, 2, 3, 0, 0, 0, 0)),
        ); // total 5
           // トークン 0 の実行は日別集計に含まれない
        make_exec(
            &store,
            block_a,
            "2026-01-03 09:00:00",
            10,
            None,
            Some(&stats_json(0, 0, 0, 0, 0, 0, 0, 0, 0)),
        );

        let rows = store.daily_block_tokens().unwrap();
        assert_eq!(rows.len(), 3);
        // 日付昇順 → ブロック名昇順
        assert_eq!(rows[0].date, "2026-01-01");
        assert_eq!(rows[0].block_name, "daily-block-a");
        assert_eq!(rows[0].total_tokens, 10);
        assert_eq!(rows[1].date, "2026-01-01");
        assert_eq!(rows[1].block_name, "daily-block-b");
        assert_eq!(rows[1].total_tokens, 20);
        assert_eq!(rows[2].date, "2026-01-02");
        assert_eq!(rows[2].block_name, "daily-block-a");
        assert_eq!(rows[2].total_tokens, 5);
    }

    /// R21: 全ブロックに渡る tool / message / think ごとのトークン合算を検証する。
    #[test]
    fn test_token_usage_summary_r21() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_a = store
            .insert_block(&create_test_block("sum-block-a"))
            .unwrap();
        let block_b = store
            .insert_block(&create_test_block("sum-block-b"))
            .unwrap();

        // block_a: tool(5,5,2) message(1,1,1) think(4,4,4)
        make_exec(
            &store,
            block_a,
            "2026-01-01 09:00:00",
            10,
            None,
            Some(&stats_json(5, 5, 2, 1, 1, 1, 4, 4, 4)),
        );
        // block_b: tool(10,10,10) message(2,2,2) think(0,0,0)
        make_exec(
            &store,
            block_b,
            "2026-01-02 09:00:00",
            10,
            None,
            Some(&stats_json(10, 10, 10, 2, 2, 2, 0, 0, 0)),
        );
        // session_stats_json 未設定（後方互換）の実行は合算に影響しない
        make_exec(&store, block_a, "2026-01-03 09:00:00", 10, None, None);

        let totals = store.token_usage_summary().unwrap();
        assert_eq!(totals.tool.input, 15);
        assert_eq!(totals.tool.output, 15);
        assert_eq!(totals.tool.cache, 12);
        assert_eq!(totals.message.input, 3);
        assert_eq!(totals.message.output, 3);
        assert_eq!(totals.message.cache, 3);
        assert_eq!(totals.think.input, 4);
        assert_eq!(totals.think.output, 4);
        assert_eq!(totals.think.cache, 4);
    }

    /// R16: 実行が無い状態と存在する状態でプロジェクト開始日を検証する。
    #[test]
    fn test_project_start_date_r16() {
        let store = SaiStore::open_in_memory().unwrap();

        // 実行が無い → None
        assert_eq!(store.project_start_date().unwrap(), None);

        let block = store
            .insert_block(&create_test_block("start-block"))
            .unwrap();
        // 2 実行（最初の started_at が開始日になる）
        make_exec(&store, block, "2026-03-15 10:00:00", 10, None, None);
        make_exec(&store, block, "2026-03-16 10:00:00", 10, None, None);
        assert_eq!(
            store.project_start_date().unwrap().as_deref(),
            Some("2026-03-15")
        );

        // より早い started_at の実行を追加 → 開始日が更新される
        make_exec(&store, block, "2026-01-02 10:00:00", 10, None, None);
        assert_eq!(
            store.project_start_date().unwrap().as_deref(),
            Some("2026-01-02")
        );
    }

    /// R17: Recent（既定 5 件）が取得できること。既存 list_recent_executions を再利用。
    #[test]
    fn test_list_recent_executions_default_r17() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = store
            .insert_block(&create_test_block("recent-default"))
            .unwrap();
        for _ in 0..7 {
            store.create_execution(block, None).unwrap();
        }
        let recent = store.list_recent_executions_default().unwrap();
        assert_eq!(recent.len(), 5);
    }

    /// R30（U16）: `latest_main_execution()` が最新のメイン実行（parent_run_id が NULL）を返すこと。
    /// メイン実行が無い場合は `None` を返すこと。サブタスク実行は対象外であること。
    #[test]
    fn test_latest_main_execution() {
        let store = SaiStore::open_in_memory().unwrap();
        // メイン実行が無い → None
        assert!(store.latest_main_execution().unwrap().is_none());

        let block = store
            .insert_block(&create_test_block("main-block"))
            .unwrap();

        // メイン実行を 1 件作成（これが最新メインとして返る）
        let main = store.create_execution(block, None).unwrap();
        // サブタスク実行（parent あり）は対象外でなければならない
        let _sub = store
            .create_execution_for_subtask(block, None, main, None)
            .unwrap();

        let latest = store
            .latest_main_execution()
            .unwrap()
            .expect("最新メイン実行が存在");
        assert_eq!(
            latest.id,
            Some(main),
            "最新のメイン実行を返す（サブタスクは除外）"
        );
        assert_eq!(
            latest.parent_run_id, None,
            "メイン実行は parent_run_id が NULL"
        );
    }

    /// R28（U16）: 新フィールド（`token_usage_by_category`）無しの既存実行でも、
    /// トップレベル `input_tokens` / `output_tokens` / `thinking_tokens` へ
    /// フォールバックして input / output / think を返すこと。
    #[test]
    fn test_parse_category_token_usage_legacy_fallback() {
        // 新フィールド無し（旧形式）: トップレベルトークンへフォールバック
        let legacy = r#"{"input_tokens": 100, "output_tokens": 40, "thinking_tokens": 15}"#;
        let totals = parse_category_token_usage(Some(legacy));
        assert_eq!(totals.tool.input, 100, "旧形式 input → tool.input");
        assert_eq!(totals.tool.output, 40, "旧形式 output → tool.output");
        assert_eq!(totals.think.output, 15, "旧形式 thinking → think.output");
        assert_eq!(totals.tool.cache, 0, "旧形式は cache を持たない");

        // 新フィールドがある場合は新フィールド優先
        let new_style = r#"{"token_usage_by_category": {
            "tool": {"input": 5, "output": 6, "cache": 7},
            "message": {"input": 1, "output": 2, "cache": 3},
            "think": {"input": 4, "output": 8, "cache": 0}
        }}"#;
        let totals = parse_category_token_usage(Some(new_style));
        assert_eq!(totals.tool.input, 5);
        assert_eq!(totals.message.output, 2);
        assert_eq!(totals.think.output, 8);

        // None / 不正 JSON は 0
        assert_eq!(parse_category_token_usage(None).tool.input, 0);
        assert_eq!(parse_category_token_usage(Some("not json")).tool.input, 0);
    }

    /// L1: execution_usage の全置換が冪等で、total 行集計に反映されること。
    #[test]
    fn test_execution_usage_replace_and_aggregation_fallback() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = BlockMetadata::new(
            "usage-agg".to_string(),
            String::new(),
            "model-x".to_string(),
            None,
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            "blocks/test.md".to_string(),
            None
        );
        let block_id = store.insert_block(&block).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(e1, "completed").unwrap();

        // usage 未保存: session_stats_json の近似値にフォールバックする
        let stats_json = r#"{"input_tokens":10,"output_tokens":20,"thinking_tokens":5,
            "token_usage_by_category":{"tool":{"input":10,"output":0,"cache":0},
            "message":{"input":0,"output":20,"cache":0},"think":{"input":0,"output":5,"cache":0}}}"#;
        store
            .update_execution_session_stats(e1, stats_json)
            .unwrap();
        let summary = store.project_run_summary().unwrap();
        assert_eq!(summary.total_tokens, 35);

        // usage を保存: total 行（provider の実際の値）に切り替わる
        let rows = vec![
            {
                let mut r = ExecutionUsageRow::total(e1);
                r.input_tokens = 100;
                r.output_tokens = 200;
                r.cache_read = 50;
                r.cost_total = 0.5;
                r
            },
            {
                let mut r =
                    ExecutionUsageRow::breakdown(e1, crate::db::models::USAGE_ENTRY_MESSAGE, 1);
                r.input_tokens = 100;
                r.output_tokens = 200;
                r
            },
        ];
        store.replace_execution_usage(e1, &rows).unwrap();
        let summary = store.project_run_summary().unwrap();
        assert_eq!(summary.total_tokens, 350);
        assert!((summary.total_cost_usd - 0.5).abs() < 1e-9);

        // 全置換の冪等性: 差し替えると行数は変わらず内容のみ更新される
        let mut rows2 = rows.clone();
        rows2[0].input_tokens = 111;
        store.replace_execution_usage(e1, &rows2).unwrap();
        let totals = store.list_execution_usage_totals().unwrap();
        assert_eq!(totals.len(), 1);
        assert_eq!(totals[0].0, e1);
        assert_eq!(totals[0].1.input_tokens, 111);
        let agg = store.aggregate_project_stats().unwrap();
        assert_eq!(agg.by_model[0].input_tokens, 111);
        assert_eq!(agg.by_model[0].cache_tokens, 50);
    }

    // -----------------------------------------------------------------------
    // U2: todo のカードを自分の子として起こす（予約 run の子としての確定）
    // -----------------------------------------------------------------------

    /// U2: 予約 run を子として確定すると、`parent_run_id` と `batch_id` が入り、
    /// `block_id` が実ブロックに付け替わる（run_id 自体は予約のまま変わらない）。
    #[test]
    fn test_claim_reserved_run_for_subtask_fills_parent_and_batch() {
        let store = SaiStore::open_in_memory().unwrap();
        let parent_block_id = store
            .insert_block(&create_test_block("claim-sub-parent"))
            .unwrap();
        let parent_run_id = store.create_execution(parent_block_id, None).unwrap();
        let child_block_id = store
            .insert_block(&create_test_block("claim-sub-child"))
            .unwrap();
        let reserved = store.create_reserved_run(None).unwrap();

        store
            .claim_reserved_run_for_subtask(
                reserved,
                child_block_id,
                Some(r#"{"task_text":"カード本文"}"#),
                parent_run_id,
                Some("batch_20260930_r1_abcd"),
            )
            .unwrap();

        let exec = store.get_execution_by_id(reserved).unwrap();
        assert_eq!(exec.id, Some(reserved), "run_id は予約のまま変わらない");
        assert_eq!(exec.status, "pending");
        assert_eq!(exec.block_id, child_block_id);
        assert_eq!(exec.parent_run_id, Some(parent_run_id));
        assert_eq!(exec.batch_id.as_deref(), Some("batch_20260930_r1_abcd"));
        assert_eq!(
            exec.user_inputs_json.as_deref(),
            Some(r#"{"task_text":"カード本文"}"#)
        );
    }

    /// U2: `todo` 以外の行は確定できない（予約でない行・二確定とも `NotFound`）。
    #[test]
    fn test_claim_reserved_run_for_subtask_rejects_non_todo_rows() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_test_block("claim-sub-twice")).unwrap();
        let parent_run_id = store.create_execution(block_id, None).unwrap();

        // 予約でない行（`pending`）は確定できない
        let pending = store.create_execution(block_id, None).unwrap();
        let err = store
            .claim_reserved_run_for_subtask(pending, block_id, None, parent_run_id, None)
            .expect_err("todo 以外の行は確定できない");
        assert!(matches!(err, DbError::NotFound(_)), "実際のエラー: {:?}", err);

        // 予約行でも 2 度目は確定できない（同じ run_id を 2 つの子にできない）
        let reserved = store.create_reserved_run(None).unwrap();
        store
            .claim_reserved_run_for_subtask(reserved, block_id, None, parent_run_id, None)
            .unwrap();
        let err = store
            .claim_reserved_run_for_subtask(reserved, block_id, None, parent_run_id, None)
            .expect_err("二度目の確定はできない");
        assert!(matches!(err, DbError::NotFound(_)), "実際のエラー: {:?}", err);
    }

    /// U2: タスクの `card_run_id` が保存・読み出しでき、未完了のタスクを引ける。
    #[test]
    fn test_task_card_run_id_round_trip_and_unfinished_lookup() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_test_block("card-task")).unwrap();
        let parent_run_id = store.create_execution(block_id, None).unwrap();

        let mut task = Task::new(
            parent_run_id,
            "カードの件名".to_string(),
            "カードの説明".to_string(),
            Some("card-task".to_string()),
            Some(r#"{"task_text":"本文"}"#.to_string()),
            None,
            "none".to_string(),
            Some("batch_1".to_string()),
            Some("R1".to_string()),
        );
        task.card_run_id = Some(120);
        let task_id = store.insert_task(&task).unwrap();

        // 保存と読み出し（task_list の `"card"` に出る値）
        let fetched = store.get_task_by_id(task_id).unwrap();
        assert_eq!(fetched.card_run_id, Some(120));

        // 未完了なら同じカードで引ける
        let unfinished = store.list_unfinished_tasks_by_card(120).unwrap();
        assert_eq!(unfinished.len(), 1);
        assert_eq!(unfinished[0].id, Some(task_id));

        // 完了（finalized）してからは引けない（同じカードを再び起こせる）
        store.update_task_status(task_id, "finalized").unwrap();
        assert!(store.list_unfinished_tasks_by_card(120).unwrap().is_empty());

        // 別のカードは引っかからない
        assert!(store.list_unfinished_tasks_by_card(121).unwrap().is_empty());
    }
}
