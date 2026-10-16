//! スキーマ定義と初期化
//!
//! waggle の `schema::init()` パターンに倣い、agent-link 専用テーブルを
//! `CREATE TABLE IF NOT EXISTS` で作成します。waggle と同一の SQLite DB
//! ファイル上にテーブルを追加するため、テーブル名には `block_` / `execution_` /
//! `git_` プレフィックスを使用して衝突を回避します。
//!
//! # プラグマ設定
//!
//! - `journal_mode = WAL`: 読み取りが書き込みをブロックしない
//! - `synchronous = FULL`: 耐障害性を確保
//! - `foreign_keys = ON`: 参照整合性を強制

use rusqlite::Connection;

use super::models::DbError;

/// agent-link 用のテーブルを初期化する。
///
/// プラグマ設定を行い、5 つのテーブルとインデックスを
/// `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS` で作成する。
/// 冪等であり、複数回呼び出しても安全。
///
/// # 作成されるテーブル
///
/// - `block_metadata` — ブロック定義メタデータ
/// - `block_inputs` — ブロック入力フィールド定義
/// - `block_output` — ブロック出力設定
/// - `execution_history` — ブロック実行履歴（output_content カラムを含む）
/// - `git_commits` — Git コミット管理
pub fn init_sai_tables(conn: &Connection) -> Result<(), DbError> {
    // WAL モード: 読み取りが書き込みをブロックしない
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(map_rusqlite)?;
    // FULL 同期: コミットが確認されたら永続化を保証
    conn.pragma_update(None, "synchronous", "FULL")
        .map_err(map_rusqlite)?;
    // 外部キー制約を有効化
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(map_rusqlite)?;

    conn.execute_batch(
        "
        -- ブロックメタデータ
        CREATE TABLE IF NOT EXISTS block_metadata(
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            name        TEXT    NOT NULL UNIQUE,
            display_name TEXT   NOT NULL DEFAULT '',
            description TEXT    NOT NULL,
            model       TEXT    NOT NULL DEFAULT 'inherit',
            thinking    TEXT    NOT NULL DEFAULT 'none',
            tools       TEXT    NOT NULL DEFAULT 'true',
            extensions  TEXT    NOT NULL DEFAULT 'true',
            skills      TEXT,
            max_turns   INTEGER,
            timeout     INTEGER NOT NULL DEFAULT 300,
            worktree    TEXT    NOT NULL DEFAULT 'none',
            enable      INTEGER NOT NULL DEFAULT 1,
            file_path   TEXT    NOT NULL,
            body_hash   TEXT,
            created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
            updated_at  TEXT    NOT NULL DEFAULT (datetime('now'))
        );

        -- ブロック入力定義
        CREATE TABLE IF NOT EXISTS block_inputs(
            id                    INTEGER PRIMARY KEY AUTOINCREMENT,
            block_id              INTEGER NOT NULL REFERENCES block_metadata(id) ON DELETE CASCADE,
            input_id              TEXT    NOT NULL,
            input_type            TEXT    NOT NULL,
            label                 TEXT    NOT NULL,
            description           TEXT,
            required              INTEGER NOT NULL DEFAULT 1,
            default_value         TEXT,
            options_json          TEXT,
            source_blocks_json    TEXT,
            source_categories_json TEXT,
            UNIQUE(block_id, input_id)
        );

        -- ブロック出力定義
        CREATE TABLE IF NOT EXISTS block_output(
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            block_id        INTEGER NOT NULL UNIQUE REFERENCES block_metadata(id) ON DELETE CASCADE,
            format          TEXT    NOT NULL DEFAULT 'markdown',
            waggle_tokenize INTEGER NOT NULL DEFAULT 1,
            description     TEXT
        );

        -- ブロック実行履歴
        CREATE TABLE IF NOT EXISTS execution_history(
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            block_id            INTEGER NOT NULL REFERENCES block_metadata(id),
            status              TEXT    NOT NULL DEFAULT 'pending',
            user_inputs_json    TEXT,
            resolved_prompt     TEXT,
            input_waggle_tokens  TEXT,  -- JSON配列: 入力として使ったwaggleトークン
            output_waggle_token TEXT,
            output_waggle_tokens TEXT,  -- JSON配列: 複数waggleトークン（sai-write用）
            output_type         TEXT    NOT NULL DEFAULT 'document',  -- 'document'|'code'|'both'
            git_commit_before   TEXT,   -- pi起動時のHEADコミットハッシュ（再現性用）
            git_commit_after    TEXT,   -- pi終了時のHEADコミットハッシュ（再現性用）
            herdr_pane_id       TEXT,
            pid                 INTEGER,
            started_at          TEXT,
            finished_at         TEXT,
            duration_ms         INTEGER,
            exit_code           INTEGER,
            error_message       TEXT,
            output_content      TEXT,  -- FN-012: エージェント出力の全文（TUI確認用）
            parent_run_id       INTEGER REFERENCES execution_history(id),  -- 親実行ID（サブタスク用）
            batch_id            TEXT,   -- P3-A: タスクバッチID（DAG のフェーズ合成に使用）
            finalized           INTEGER NOT NULL DEFAULT 0,  -- ユーザー明示完了（Ctrl+D）で finalize 済みか（U4）
            created_at          TEXT    NOT NULL DEFAULT (datetime('now'))
        );

        -- 実行 usage（provider が返す実際の token / cost・L1）。
        -- 'total' 行 1 本 = provider usage の実行合計（コスト計算の正）。
        -- 種別 × ターンの行群 = 内訳（分析用・エントリサイズ比による推定振り分け）。
        -- 既存の近似 token（execution_history.session_stats_json）は互換用にそのまま維持する。
        CREATE TABLE IF NOT EXISTS execution_usage(
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            execution_id  INTEGER NOT NULL,           -- execution_history.id への参照
            entry_type    TEXT NOT NULL,              -- 'total' | 'message' | 'thinking' | 'toolCall' | 'toolResult'
            turn_no       INTEGER,                    -- 種別行のターン番号（1 始まり）。'total' 行は NULL
            input_tokens  INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read    INTEGER NOT NULL DEFAULT 0,
            cache_write   INTEGER NOT NULL DEFAULT 0,
            cost_total    REAL    NOT NULL DEFAULT 0,
            UNIQUE(execution_id, entry_type, turn_no)
        );
        CREATE INDEX IF NOT EXISTS idx_execution_usage_exec
            ON execution_usage(execution_id);
        -- SQLite の UNIQUE は NULL を重複とみなさないため、'total' 行（turn_no = NULL）の
        -- 1 実行 1 行制約を部分 UNIQUE インデックスで強制する
        CREATE UNIQUE INDEX IF NOT EXISTS idx_execution_usage_total
            ON execution_usage(execution_id) WHERE entry_type = 'total';

        -- Git コミット管理
        CREATE TABLE IF NOT EXISTS git_commits(
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            execution_id    INTEGER NOT NULL REFERENCES execution_history(id) ON DELETE CASCADE,
            commit_hash     TEXT    NOT NULL,
            commit_message  TEXT,
            author_name     TEXT,
            author_email    TEXT,
            branch_name     TEXT,
            committed_at    TEXT    NOT NULL DEFAULT (datetime('now'))
        );

        -- サブタスク管理
        CREATE TABLE IF NOT EXISTS tasks(
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            parent_run_id   INTEGER NOT NULL REFERENCES execution_history(id),
            subject         TEXT    NOT NULL,
            description     TEXT    NOT NULL,
            status          TEXT    NOT NULL DEFAULT 'pending',
            block_type      TEXT,
            inputs_json     TEXT,
            depends_on_json TEXT,
            worktree        TEXT    NOT NULL DEFAULT '',  -- '' = 未指定（ブロック定義に従う）/ 'none' / 'isolation' / ブランチ名
            cwd             TEXT,              -- サブタスクの作業フォルダ（絶対パス。NULL = プロジェクトルート）
            timeout         INTEGER,          -- 子の時間上限（秒）。超えると親へ stalled(timeout) を繰り返し通知する。NULL = ブロック定義に従う
            card_run_id     INTEGER,          -- 起こす todo カードの run_id（task_create の card 引数。NULL = カード無し）
            execution_id    INTEGER REFERENCES execution_history(id),
            batch_id        TEXT,
            client_id       TEXT,              -- クライアント側一時ID（carried_over 照合・依存解決用）
            round           INTEGER NOT NULL DEFAULT 1,  -- 修正ラウンド番号（task_create 再送のたびに増加）
            supersedes_batch_id TEXT,          -- 置き換え元バッチID（修正版バッチ作成時）
            approval_comment TEXT,             -- 否認/保留時のユーザーコメント
            finalized       INTEGER NOT NULL DEFAULT 0,  -- task_finalize 済みか（0/1）
            deleted_by      TEXT,              -- 削除者（user / main）
            deleted_at      TEXT,              -- 削除日時
            created_at      TEXT    NOT NULL DEFAULT (datetime('now')),
            updated_at      TEXT    NOT NULL DEFAULT (datetime('now'))
        );

        -- 実行バッチ（メインブロック複製・mb1.x）管理
        CREATE TABLE IF NOT EXISTS run_batches(
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            parent_run_id   INTEGER NOT NULL REFERENCES execution_history(id), -- 実メイン mb1
            batch_id        TEXT    NOT NULL,             -- 実行バッチID（E1, E2...）
            seq             INTEGER NOT NULL DEFAULT 1,   -- 実行バッチ連番（mb1.x の x）
            status          TEXT    NOT NULL DEFAULT 'open', -- open|running|complete|finalized
            mb_execution_id INTEGER,                        -- メイン複製用実行レコードID（接続端点）
            created_at      TEXT    NOT NULL DEFAULT (datetime('now')),
            finished_at     TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_run_batches_parent
            ON run_batches(parent_run_id);
        CREATE INDEX IF NOT EXISTS idx_run_batches_batch
            ON run_batches(batch_id);

        -- サブタスク用インデックス
        CREATE INDEX IF NOT EXISTS idx_tasks_parent_run_id
            ON tasks(parent_run_id);
        CREATE INDEX IF NOT EXISTS idx_tasks_status
            ON tasks(status);
        CREATE INDEX IF NOT EXISTS idx_tasks_batch_id
            ON tasks(batch_id);

        -- ブロック間接続（線図描画用）
        CREATE TABLE IF NOT EXISTS block_connections(
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            source_execution_id INTEGER NOT NULL REFERENCES execution_history(id),
            source_block_id     INTEGER NOT NULL REFERENCES block_metadata(id),
            target_execution_id INTEGER NOT NULL REFERENCES execution_history(id),
            target_block_id     INTEGER NOT NULL REFERENCES block_metadata(id),
            connection_type     TEXT    NOT NULL CHECK(connection_type IN ('waggle_token','git_commit','task_delegation','task_output','kanban_origin','review')),
            waggle_token        TEXT,
            commit_hash         TEXT,
            created_at          TEXT    NOT NULL DEFAULT (datetime('now'))
        );

        -- インデックス
        CREATE INDEX IF NOT EXISTS idx_execution_history_block_id
            ON execution_history(block_id);
        CREATE INDEX IF NOT EXISTS idx_execution_history_status
            ON execution_history(status);
        CREATE INDEX IF NOT EXISTS idx_git_commits_execution_id
            ON git_commits(execution_id);
        CREATE INDEX IF NOT EXISTS idx_git_commits_commit_hash
            ON git_commits(commit_hash);
        CREATE INDEX IF NOT EXISTS idx_connections_source_block
            ON block_connections(source_block_id);
        CREATE INDEX IF NOT EXISTS idx_connections_target_block
            ON block_connections(target_block_id);
        CREATE INDEX IF NOT EXISTS idx_connections_source_exec
            ON block_connections(source_execution_id);
        CREATE INDEX IF NOT EXISTS idx_connections_target_exec
            ON block_connections(target_execution_id);
        ",
    )
    .map_err(map_rusqlite)?;

    // マイグレーション: 既存テーブルへのカラム追加
    // SQLite には ALTER TABLE ADD COLUMN IF NOT EXISTS が存在しないため、
    // PRAGMA table_info でカラム存在確認後に ALTER TABLE を実行する
    migrate_add_column(conn, "execution_history", "output_content", "TEXT")?;
    migrate_add_column(conn, "execution_history", "output_waggle_tokens", "TEXT")?;
    migrate_add_column(
        conn,
        "execution_history",
        "output_type",
        "TEXT NOT NULL DEFAULT 'document'",
    )?;
    migrate_add_column(conn, "execution_history", "input_waggle_tokens", "TEXT")?;
    migrate_add_column(conn, "execution_history", "git_commit_before", "TEXT")?;
    migrate_add_column(conn, "execution_history", "git_commit_after", "TEXT")?;
    migrate_add_column(conn, "execution_history", "session_stats_json", "TEXT")?;
    migrate_add_column(conn, "block_metadata", "round", "INTEGER")?;
    // id/name 分離（表示名）: sai-tui 等の表示用に display_name を追加
    migrate_add_column(
        conn,
        "block_metadata",
        "display_name",
        "TEXT NOT NULL DEFAULT ''",
    )?;

    // 右SB表示用: 実行インスタンス単位のオーバーライド値とセッション情報
    migrate_add_column(conn, "execution_history", "round", "INTEGER")?;
    // グローバルラウンド（GR）: ブロック配置（DAG の行）と区切り線に使用
    migrate_add_column(conn, "execution_history", "global_round", "INTEGER")?;
    migrate_add_column(conn, "execution_history", "model", "TEXT")?;
    migrate_add_column(conn, "execution_history", "thinking", "TEXT")?;
    migrate_add_column(conn, "execution_history", "timeout", "INTEGER")?;
    migrate_add_column(conn, "execution_history", "max_turns", "INTEGER")?;
    migrate_add_column(conn, "execution_history", "session_dir", "TEXT")?;
    migrate_add_column(conn, "execution_history", "pi_command", "TEXT")?;
    migrate_add_column(
        conn,
        "execution_history",
        "parent_run_id",
        "INTEGER REFERENCES execution_history(id)",
    )?;
    // P3-A: タスクバッチID（DAG のフェーズ合成用）
    migrate_add_column(conn, "execution_history", "batch_id", "TEXT")?;
    // U4: メインブロックのユーザー明示完了（Ctrl+D finalize）フラグ
    migrate_add_column(
        conn,
        "execution_history",
        "finalized",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    // Kanban: カード由来の実行を記録する3カラム
    migrate_add_column(conn, "execution_history", "origin_run_id", "TEXT")?;
    migrate_add_column(conn, "execution_history", "kanban_card_path", "TEXT")?;
    migrate_add_column(conn, "execution_history", "execution_mode", "TEXT")?;
    // Kanban→Workflow 移動フラグ（`d` で設定。ファイルは残す）。
    // review ファイル削除による排他判定の代替。0 = Kanban 所有、1 = Workflow 所有。
    migrate_add_column(
        conn,
        "execution_history",
        "workflow_moved",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    // エージェント側セッション ID（Claude Code の `--session-id` の UUID）。
    // pi 実行では NULL（pi は JSONL パスで再開するため）。
    migrate_add_column(conn, "execution_history", "agent_session_id", "TEXT")?;
    // Kanban: レビュー済みフラグ。`completed` のうち 1 なら done レーン、
    // 0 なら review レーン。従来の review.md 本文判定を廃止し、
    // run-tui の `e` で明示的に遷移させるための状態カラム。
    migrate_add_column(
        conn,
        "execution_history",
        "review_done",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_execution_history_batch_id ON execution_history(batch_id)",
        [],
    )
    .map_err(map_rusqlite)?;

    // タスクワークフロー拡張（P1）: tasks テーブルの新カラム
    migrate_add_column(conn, "tasks", "client_id", "TEXT")?;
    migrate_add_column(conn, "tasks", "round", "INTEGER NOT NULL DEFAULT 1")?;
    migrate_add_column(conn, "tasks", "supersedes_batch_id", "TEXT")?;
    migrate_add_column(conn, "tasks", "approval_comment", "TEXT")?;
    migrate_add_column(conn, "tasks", "finalized", "INTEGER NOT NULL DEFAULT 0")?;
    migrate_add_column(conn, "tasks", "deleted_by", "TEXT")?;
    migrate_add_column(conn, "tasks", "deleted_at", "TEXT")?;
    // サブタスクの作業フォルダ（task_create の cwd 引数。絶対パスで保存）
    migrate_add_column(conn, "tasks", "cwd", "TEXT")?;
    // 子の時間上限（task_create の timeout 引数・秒）
    migrate_add_column(conn, "tasks", "timeout", "INTEGER")?;
    // 起こす todo カードの run_id（task_create の card 引数）
    migrate_add_column(conn, "tasks", "card_run_id", "INTEGER")?;
    // 子の model / thinking（task_create の引数。NULL はブロック定義・settings.json に従う）
    migrate_add_column(conn, "tasks", "model", "TEXT")?;
    migrate_add_column(conn, "tasks", "thinking", "TEXT")?;

    // tasks の新カラム（実行バッチ所属）
    migrate_add_column(conn, "tasks", "run_batch_id", "INTEGER")?;

    // supersedes_batch_id インデックスはカラム追加後に作成する
    // （既存 DB ではカラムが存在しないと CREATE INDEX が失敗するため・順序厳守）
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tasks_supersedes_batch_id ON tasks(supersedes_batch_id)",
        [],
    )
    .map_err(map_rusqlite)?;

    // 旧メイン複製レコードのクリーンアップ（P ノード仮想化の後方互換対応）。
    // フェーズノード（P1, P2...）は TUI 側で run_batches から合成する方式に変更したため、
    // task_execute 時に実行レコードとして作成されていた旧「メイン複製」（mb_execution_id）
    // は不要。関連する接続線ごと削除し、mb_execution_id を NULL に戻す。
    // （毎回実行されるが、2 回目以降は該当行が存在しないため冪等）
    conn.execute(
        "DELETE FROM block_connections \
         WHERE source_execution_id IN (SELECT mb_execution_id FROM run_batches WHERE mb_execution_id IS NOT NULL) \
            OR target_execution_id IN (SELECT mb_execution_id FROM run_batches WHERE mb_execution_id IS NOT NULL)",
        [],
    )
    .map_err(map_rusqlite)?;
    conn.execute(
        "DELETE FROM execution_history \
         WHERE id IN (SELECT mb_execution_id FROM run_batches WHERE mb_execution_id IS NOT NULL)",
        [],
    )
    .map_err(map_rusqlite)?;
    conn.execute("UPDATE run_batches SET mb_execution_id = NULL", [])
        .map_err(map_rusqlite)?;

    // block_connections の CHECK 制約を更新（task_delegation / task_output 追加）
    migrate_block_connections_constraint(conn)?;

    Ok(())
}

/// `rusqlite::Error` を `DbError` に変換する。
///
/// waggle の `schema::sql()` と同様のマッピング関数。
#[allow(clippy::needless_pass_by_value)]
fn map_rusqlite(e: rusqlite::Error) -> DbError {
    DbError::Rusqlite(e)
}

/// 指定テーブルにカラムが存在しない場合のみ ALTER TABLE ADD COLUMN を実行する。
///
/// 既存 DB ファイルへの後方互換マイグレーション用。
fn migrate_add_column(
    conn: &Connection,
    table: &str,
    column: &str,
    column_def: &str,
) -> Result<(), DbError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({})", table))
        .map_err(map_rusqlite)?;
    let exists = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(map_rusqlite)?
        .filter_map(|r| r.ok())
        .any(|col| col == column);

    if !exists {
        conn.execute(
            &format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, column_def),
            [],
        )
        .map_err(map_rusqlite)?;
    }
    Ok(())
}

/// `serde_json::Error` を `DbError` に変換する。
///
/// waggle の `schema::codec()` と同様のマッピング関数。
#[allow(clippy::needless_pass_by_value)]
pub fn map_json(e: serde_json::Error) -> DbError {
    DbError::Json(e)
}

/// block_connections テーブルの CHECK 制約を更新する（task_delegation /
/// task_output / kanban_origin 追加）。
/// SQLite は ALTER TABLE で CHECK 制約を変更できないため、テーブル再作成で対応する。
fn migrate_block_connections_constraint(conn: &Connection) -> Result<(), DbError> {
    // 現在の CHECK 制約を確認
    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='block_connections'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    // 既に kanban_origin と review が含まれていればスキップ
    if sql.contains("kanban_origin") && sql.contains("review") {
        return Ok(());
    }

    // テーブル再作成によるマイグレーション
    conn.execute_batch(
        "
        -- 1. 新しいテーブルを作成
        CREATE TABLE IF NOT EXISTS block_connections_new(
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            source_execution_id INTEGER NOT NULL REFERENCES execution_history(id),
            source_block_id     INTEGER NOT NULL REFERENCES block_metadata(id),
            target_execution_id INTEGER NOT NULL REFERENCES execution_history(id),
            target_block_id     INTEGER NOT NULL REFERENCES block_metadata(id),
            connection_type     TEXT    NOT NULL CHECK(connection_type IN ('waggle_token','git_commit','task_delegation','task_output','kanban_origin','review')),
            waggle_token        TEXT,
            commit_hash         TEXT,
            created_at          TEXT    NOT NULL DEFAULT (datetime('now'))
        );

        -- 2. データをコピー
        INSERT INTO block_connections_new SELECT * FROM block_connections;

        -- 3. 古いテーブルを削除
        DROP TABLE block_connections;

        -- 4. 新しいテーブルにリネーム
        ALTER TABLE block_connections_new RENAME TO block_connections;

        -- 5. インデックスを再作成
        CREATE INDEX IF NOT EXISTS idx_connections_source_block
            ON block_connections(source_block_id);
        CREATE INDEX IF NOT EXISTS idx_connections_target_block
            ON block_connections(target_block_id);
        CREATE INDEX IF NOT EXISTS idx_connections_source_exec
            ON block_connections(source_execution_id);
        CREATE INDEX IF NOT EXISTS idx_connections_target_exec
            ON block_connections(target_execution_id);
        "
    ).map_err(map_rusqlite)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn test_init_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        // 複数回の初期化呼び出しが安全であることを確認
        init_sai_tables(&conn).unwrap();
        init_sai_tables(&conn).unwrap();
        init_sai_tables(&conn).unwrap();

        // テーブル一覧を取得して期待されるテーブルが存在することを確認
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"block_metadata".to_string()));
        assert!(tables.contains(&"block_inputs".to_string()));
        assert!(tables.contains(&"block_output".to_string()));
        assert!(tables.contains(&"execution_history".to_string()));
        assert!(tables.contains(&"execution_usage".to_string()));
        assert!(tables.contains(&"git_commits".to_string()));
        assert!(tables.contains(&"block_connections".to_string()));
        assert!(tables.contains(&"run_batches".to_string()));
    }

    /// U3 AC4: `timeout` 列の無い古い tasks テーブルでも、移行で列が足されること。
    #[test]
    fn test_migration_adds_tasks_timeout_column() {
        let conn = Connection::open_in_memory().unwrap();
        // timeout 列を持たない古い tasks テーブルを作る
        // （実際の CREATE TABLE から timeout だけ抜いたもの）
        conn.execute(
            "CREATE TABLE tasks(
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                parent_run_id   INTEGER NOT NULL REFERENCES execution_history(id),
                subject         TEXT    NOT NULL,
                description     TEXT    NOT NULL,
                status          TEXT    NOT NULL DEFAULT 'pending',
                block_type      TEXT,
                inputs_json     TEXT,
                depends_on_json TEXT,
                worktree        TEXT    NOT NULL DEFAULT '',
                cwd             TEXT,
                execution_id    INTEGER REFERENCES execution_history(id),
                batch_id        TEXT,
                client_id       TEXT,
                round           INTEGER NOT NULL DEFAULT 1,
                supersedes_batch_id TEXT,
                approval_comment TEXT,
                finalized       INTEGER NOT NULL DEFAULT 0,
                deleted_by      TEXT,
                deleted_at      TEXT,
                run_batch_id    INTEGER,
                created_at      TEXT    NOT NULL DEFAULT (datetime('now')),
                updated_at      TEXT    NOT NULL DEFAULT (datetime('now'))
            )",
            [],
        )
        .unwrap();

        // 移行でエラーにならず timeout 列が足される
        init_sai_tables(&conn).unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(tasks)").unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            columns.contains(&"timeout".to_string()),
            "移行で tasks に timeout 列が足されること（実際: {:?}）",
            columns
        );
    }

    /// L1: execution_usage のカラム・UNIQUE 制約が仕様どおりであること。
    #[test]
    fn test_execution_usage_schema() {
        let conn = Connection::open_in_memory().unwrap();
        init_sai_tables(&conn).unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(execution_usage)").unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        for expected in [
            "id",
            "execution_id",
            "entry_type",
            "turn_no",
            "input_tokens",
            "output_tokens",
            "cache_read",
            "cache_write",
            "cost_total",
        ] {
            assert!(
                columns.contains(&expected.to_string()),
                "execution_usage にカラム {expected} が必要"
            );
        }

        // UNIQUE(execution_id, entry_type, turn_no) の重複挿入はエラーになること
        conn.execute(
            "INSERT INTO execution_usage \
             (execution_id, entry_type, turn_no, input_tokens, output_tokens, cache_read, cache_write, cost_total) \
             VALUES (1, 'total', NULL, 0, 0, 0, 0, 0.0)",
            [],
        )
        .unwrap();
        assert!(conn
            .execute(
                "INSERT INTO execution_usage \
                 (execution_id, entry_type, turn_no, input_tokens, output_tokens, cache_read, cache_write, cost_total) \
                 VALUES (1, 'total', NULL, 1, 1, 1, 1, 1.0)",
                [],
            )
            .is_err());
    }

    #[test]
    fn test_tables_have_expected_columns() {
        let conn = Connection::open_in_memory().unwrap();
        init_sai_tables(&conn).unwrap();

        // block_metadata テーブルのカラムを確認
        let mut stmt = conn.prepare("PRAGMA table_info(block_metadata)").unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(columns.contains(&"name".to_string()));
        assert!(columns.contains(&"display_name".to_string()));
        assert!(columns.contains(&"description".to_string()));
        assert!(columns.contains(&"model".to_string()));

        // execution_history テーブルに output_content カラムが存在することを確認（FN-012）
        let mut exec_stmt = conn
            .prepare("PRAGMA table_info(execution_history)")
            .unwrap();
        let exec_columns: Vec<String> = exec_stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            exec_columns.contains(&"output_content".to_string()),
            "execution_history テーブルに output_content カラムが存在しません"
        );
    }

    /// Kanban: 新3カラム（origin_run_id / kanban_card_path / execution_mode）が
    /// 存在し、`block_connections` の CHECK に `kanban_origin` が含まれること。
    #[test]
    fn test_kanban_columns_and_connection_type() {
        let conn = Connection::open_in_memory().unwrap();
        init_sai_tables(&conn).unwrap();
        // 冪等: 3回呼び出し後も新カラムが存在すること
        init_sai_tables(&conn).unwrap();
        init_sai_tables(&conn).unwrap();

        let mut stmt = conn
            .prepare("PRAGMA table_info(execution_history)")
            .unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        for expected in ["origin_run_id", "kanban_card_path", "execution_mode"] {
            assert!(
                cols.contains(&expected.to_string()),
                "execution_history にカラム {expected} が必要"
            );
        }

        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='block_connections'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_default();
        assert!(
            sql.contains("kanban_origin"),
            "block_connections の CHECK に kanban_origin が必要"
        );
    }

    #[test]
    fn test_foreign_keys_enabled() {
        let conn = Connection::open_in_memory().unwrap();
        init_sai_tables(&conn).unwrap();
        let fk_enabled: i32 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(fk_enabled, 1);
    }

    #[test]
    fn test_wal_mode() {
        let conn = Connection::open_in_memory().unwrap();
        init_sai_tables(&conn).unwrap();
        // インメモリ DB では WAL モードは変更不可だがエラーにならないことを確認
        // （実際のファイル DB で WAL が設定される）
        // ここではエラーなく通ることをテスト（assert なし: init 成功で十分）
    }
}
