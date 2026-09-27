//! # runner=user の成果物登録（run 紐付け）
//!
//! TUI から呼ばれる「user タスクの sai-write」のコア処理です。
//!
//! runner=user の実行中（`status='running'`）の run に成果物ファイルを登録します。
//!
//! ## 処理の流れ
//!
//! 1. `runner='user'` / `status='running'` を検証する
//! 2. 保存名は元ファイルの basename。移動先は run フォルダ
//! 3. SHA-256 の重複判定（既存の user 成果物と一致したら登録しない・移動もしない）
//! 4. 元ファイルを run フォルダへ **移動**（rename。失敗時は copy + remove）
//! 5. `WaggleIntegrator::tokenize_file` でトークン化
//! 6. `execution_history.output_waggle_tokens` に追記（`output_type='document'`）
//! 7. 入力トークン辺（`waggle_token`）と来歴辺（`kanban_origin`）を挿入
//!
//! 新規 execution 行は作成しません（run の Card は task.md 作成時に存在するため）。

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::db::models::DbError;
use crate::db::SaiStore;
use crate::run_dir;
use crate::waggle_integration::WaggleIntegrator;

// ============================================================================
// エラー型
// ============================================================================

/// user 成果物登録のエラー型。
#[derive(Debug, thiserror::Error)]
pub enum UserWriteError {
    /// 実行が見つからない。
    #[error("実行が見つかりません: run_id={0}")]
    RunNotFound(i64),

    /// runner が `user` ではない。
    #[error("run_id={run_id} は user タスクではありません（runner={runner}）")]
    NotUserRunner {
        /// 実行 ID
        run_id: i64,
        /// 実際の runner
        runner: String,
    },

    /// 実行中（`running`）ではない。
    #[error("run_id={run_id} は実行中ではありません（status={status}）")]
    NotRunning {
        /// 実行 ID
        run_id: i64,
        /// 実際の status
        status: String,
    },

    /// 対象ファイルが存在しない。
    #[error("対象ファイルが存在しません: {0}")]
    SourceNotFound(PathBuf),

    /// run フォルダ内の固定ファイルなど、登録できないファイル。
    #[error("run フォルダ内の固定ファイルは登録できません: {0}")]
    Protected(PathBuf),

    /// 同一内容の出力が既に存在する（重複判定）。
    #[error("同一の内容の出力が既に存在します（sha256={0}）")]
    Duplicate(String),

    /// ファイル移動に失敗した。
    #[error("ファイル移動に失敗しました: {from} -> {to}: {source}")]
    MoveFailed {
        /// 移動元
        from: PathBuf,
        /// 移動先
        to: PathBuf,
        /// 原因
        source: std::io::Error,
    },

    /// waggle トークン化に失敗した。
    #[error("waggle トークン化に失敗しました: {0}")]
    Tokenize(String),

    /// DB エラー。
    #[error("DB エラー: {0}")]
    Db(#[from] DbError),

    /// I/O エラー。
    #[error("I/O エラー: {0}")]
    Io(#[from] std::io::Error),
}

// ============================================================================
// 結果
// ============================================================================

/// user 成果物登録の結果。
#[derive(Debug, Clone)]
pub struct UserWriteOutcome {
    /// 登録先の実行 ID（run_id）
    pub exec_id: i64,
    /// 発行された waggle トークン
    pub token: String,
    /// 移動後の保存先パス
    pub saved_path: PathBuf,
}

// ============================================================================
// 本体
// ============================================================================

/// runner=user の run に成果物を登録する。
///
/// 詳細はモジュールドキュメントを参照。`input_tokens` は入力 waggle トークン
/// （裸のトークン文字列。`waggle:` 接頭辞は呼び出し側で除去する）。
pub fn register_user_output(
    store: &SaiStore,
    integrator: &WaggleIntegrator,
    run_id: i64,
    source: &Path,
    description: Option<&str>,
    input_tokens: &[String],
) -> Result<UserWriteOutcome, UserWriteError> {
    // ── 1. 実行の検証 ──
    let exec = store
        .get_execution_by_id(run_id)
        .map_err(|_| UserWriteError::RunNotFound(run_id))?;
    if exec.runner != "user" {
        return Err(UserWriteError::NotUserRunner {
            run_id,
            runner: exec.runner.clone(),
        });
    }
    if exec.status != "running" {
        return Err(UserWriteError::NotRunning {
            run_id,
            status: exec.status.clone(),
        });
    }

    // ── 2. ソース検証・保存名・移動先 ──
    if !source.is_file() {
        return Err(UserWriteError::SourceNotFound(source.to_path_buf()));
    }
    let base = run_dir::sanitize_filename(
        source
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default(),
    );

    let run_dir_path = match run_dir::find_run_dir(run_id) {
        Some((_, p)) => p,
        None => run_dir::ensure_run_dir(run_dir::RUNNING_OWNER, run_id)?,
    };

    // 破壊防止: run フォルダの固定ファイル、または run フォルダ配下のファイルは登録しない
    if base == run_dir::TASK_FILE || base == run_dir::REVIEW_FILE || base == run_dir::MESSAGE_FILE {
        return Err(UserWriteError::Protected(source.to_path_buf()));
    }
    let source_for_check = source.canonicalize().unwrap_or_else(|_| source.to_path_buf());
    let run_dir_for_check = run_dir_path
        .canonicalize()
        .unwrap_or_else(|_| run_dir_path.clone());
    if source_for_check.starts_with(&run_dir_for_check) {
        return Err(UserWriteError::Protected(source.to_path_buf()));
    }

    // ── 3. 重複判定（内容の SHA-256） ──
    let bytes = std::fs::read(source)?;
    if bytes.is_empty() {
        // 空ファイルはトークン化できないため、移動前に拒否する
        return Err(UserWriteError::Tokenize("空のファイルです".to_string()));
    }
    let sha = sha256_hex(&bytes);
    if is_duplicate(store, integrator, &sha) {
        return Err(UserWriteError::Duplicate(sha));
    }

    // ── 4. 移動 ──
    let dest = run_dir_path.join(&base);
    if dest == source_for_check {
        // 既に同位置（通常は起きない。保護チェックで弾かれる）
        return Err(UserWriteError::Protected(source.to_path_buf()));
    }
    move_file(source, &dest).map_err(|e| UserWriteError::MoveFailed {
        from: source.to_path_buf(),
        to: dest.clone(),
        source: e,
    })?;

    // ── 5. waggle トークン化 ──
    let tokenized = integrator
        .tokenize_file(&dest, "user", description)
        .map_err(|e| UserWriteError::Tokenize(e.to_string()))?;
    let token = tokenized.token.as_str().to_string();

    // ── 6. DB 記録（既存トークンを保持して追記） ──
    store.append_execution_waggle_token(run_id, &token)?;
    let _ = store.update_execution_output_type(run_id, "document");

    // ── 7. 接続線 ──
    let block_id = exec.block_id;
    // 7-a. 入力トークン辺
    crate::orchestrator::insert_block_connections(store, run_id, block_id, input_tokens);
    // 7-b. 来歴辺（task.md frontmatter の origin_run_id）
    insert_origin_connections(store, run_id, block_id, &run_dir_path);

    Ok(UserWriteOutcome {
        exec_id: run_id,
        token,
        saved_path: dest,
    })
}

// ============================================================================
// 内部ヘルパ
// ============================================================================

/// 既存の user 成果物（`user-block` / `__user__`）のいずれかと内容が一致するか。
fn is_duplicate(store: &SaiStore, integrator: &WaggleIntegrator, sha: &str) -> bool {
    for name in ["user-block", "__user__"] {
        let Ok(block) = store.get_block_by_name(name) else {
            continue;
        };
        let Some(block_id) = block.id else {
            continue;
        };
        let executions = store.list_executions_by_block(block_id).unwrap_or_default();
        for exec in executions {
            let tokens: Vec<String> = exec
                .output_waggle_tokens
                .as_deref()
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or_default();
            for token in &tokens {
                if let Ok(content) = integrator.resolve_content(token) {
                    if sha256_hex(content.as_bytes()) == sha {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// ファイルを移動する。同一ファイルシステム内は rename、失敗時は copy + remove。
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            // 別デバイス等で rename できない場合はコピーしてから削除する
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)?;
            Ok(())
        }
    }
}

/// task.md の `origin_run_id` から `kanban_origin` 辺を挿入し、DB に来歴情報を保存する。
fn insert_origin_connections(
    store: &SaiStore,
    run_id: i64,
    block_id: i64,
    run_dir_path: &Path,
) {
    let task_md = run_dir_path.join(run_dir::TASK_FILE);
    let Ok(text) = std::fs::read_to_string(&task_md) else {
        return;
    };
    let Some(card) = crate::kanban::parse_card(&text) else {
        return;
    };
    let csv = card.frontmatter.origin_run_id.trim().to_string();
    if csv.is_empty() || csv == "-" {
        return;
    }
    let ids = crate::orchestrator::parse_origin_run_ids(&csv);
    if ids.is_empty() {
        return;
    }
    crate::orchestrator::insert_kanban_origin_connections(store, run_id, block_id, &ids);
    let card_rel = task_md
        .strip_prefix(crate::project::project_root())
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    let _ = store.update_execution_kanban_info(run_id, Some(csv.as_str()), card_rel.as_deref(), "kanban");
}

/// バイト列の SHA-256 ハッシュ（64桁 hex 小文字）を計算する。
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::BlockMetadata;

    /// テスト用の一時プロジェクト（`.sai/sai-engine/` 付き）を用意する。
    fn tmp_project(tag: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "sai_user_write_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".sai/sai-engine")).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);
        tmp
    }

    /// runner=user / running の run を作り、(run_id, run_dir) を返す。
    fn make_user_run(store: &SaiStore) -> (i64, PathBuf) {
        let user_block = store.get_or_create_user_block().unwrap();
        let run_id = store.create_reserved_run(None).unwrap();
        store.claim_reserved_run(run_id, user_block, None).unwrap();
        store.update_execution_runner(run_id, "user").unwrap();
        store.update_execution_status(run_id, "running").unwrap();
        let dir = run_dir::ensure_run_dir(run_dir::RUNNING_OWNER, run_id).unwrap();
        (run_id, dir)
    }

    fn sample_block(name: &str) -> BlockMetadata {
        BlockMetadata::new(
            name.to_string(),
            "テスト".to_string(),
            "m".to_string(),
            None,
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            format!("blocks/{}.md", name),
            None
        )
    }

    fn write_source(tmp: &Path, rel: &str, content: &str) -> PathBuf {
        let p = tmp.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
        p
    }

    fn tokens_of(store: &SaiStore, run_id: i64) -> Vec<String> {
        store
            .get_execution_by_id(run_id)
            .unwrap()
            .output_waggle_tokens
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default()
    }

    #[test]
    fn happy_path_moves_and_appends_token() {
        let _env = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tmp_project("happy");
        let store = SaiStore::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let integrator =
            WaggleIntegrator::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let (run_id, dir) = make_user_run(&store);
        let source = write_source(&tmp, "docs/memo.md", "# メモ\n本文\n");

        let out = register_user_output(&store, &integrator, run_id, &source, Some("メモ"), &[])
            .expect("登録に成功する");

        assert_eq!(out.exec_id, run_id);
        assert_eq!(out.saved_path, dir.join("memo.md"));
        assert!(dir.join("memo.md").is_file(), "run フォルダへ移動される");
        assert!(!source.exists(), "元ファイルは移動され消える");
        assert_eq!(tokens_of(&store, run_id).len(), 1);
        assert_eq!(
            store.get_execution_by_id(run_id).unwrap().output_type,
            "document"
        );

        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }

    #[test]
    fn rejects_pi_runner() {
        let _env = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tmp_project("pi");
        let store = SaiStore::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let integrator =
            WaggleIntegrator::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let block_id = store.insert_block(&sample_block("b")).unwrap();
        let run_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(run_id, "running").unwrap();
        let source = write_source(&tmp, "docs/x.md", "x");

        let err = register_user_output(&store, &integrator, run_id, &source, None, &[])
            .expect_err("runner=pi は拒否");
        assert!(matches!(err, UserWriteError::NotUserRunner { .. }));
        assert!(source.exists(), "拒否時は移動しない");

        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }

    #[test]
    fn rejects_not_running() {
        let _env = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tmp_project("notrun");
        let store = SaiStore::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let integrator =
            WaggleIntegrator::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let user_block = store.get_or_create_user_block().unwrap();
        let run_id = store.create_reserved_run(None).unwrap();
        store.claim_reserved_run(run_id, user_block, None).unwrap();
        store.update_execution_runner(run_id, "user").unwrap(); // status は pending
        let source = write_source(&tmp, "docs/x.md", "x");

        let err = register_user_output(&store, &integrator, run_id, &source, None, &[])
            .expect_err("running 以外は拒否");
        assert!(matches!(err, UserWriteError::NotRunning { .. }));

        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }

    #[test]
    fn duplicate_skips_and_keeps_source() {
        let _env = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tmp_project("dup");
        let store = SaiStore::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let integrator =
            WaggleIntegrator::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let (run_id, dir) = make_user_run(&store);
        let first = write_source(&tmp, "docs/a.md", "同じ内容");
        register_user_output(&store, &integrator, run_id, &first, None, &[]).unwrap();
        assert_eq!(tokens_of(&store, run_id).len(), 1);

        // 別名・同一内容 → 重複でスキップ、元ファイルは残る
        let second = write_source(&tmp, "docs/b.md", "同じ内容");
        let err = register_user_output(&store, &integrator, run_id, &second, None, &[])
            .expect_err("重複はエラー");
        assert!(matches!(err, UserWriteError::Duplicate(_)));
        assert!(second.exists(), "重複時は元ファイルを移動しない");
        assert!(dir.join("b.md").exists() == false, "重複分は保存されない");
        assert_eq!(tokens_of(&store, run_id).len(), 1);

        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }

    #[test]
    fn appends_multiple_tokens() {
        let _env = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tmp_project("append");
        let store = SaiStore::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let integrator =
            WaggleIntegrator::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let (run_id, _dir) = make_user_run(&store);
        let a = write_source(&tmp, "docs/a.md", "内容A");
        let b = write_source(&tmp, "docs/b.md", "内容B");
        register_user_output(&store, &integrator, run_id, &a, None, &[]).unwrap();
        register_user_output(&store, &integrator, run_id, &b, None, &[]).unwrap();
        assert_eq!(tokens_of(&store, run_id).len(), 2, "既存トークンを保持して追記");

        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }

    #[test]
    fn rejects_protected_task_md() {
        let _env = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tmp_project("protect");
        let store = SaiStore::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let integrator =
            WaggleIntegrator::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let (_run_id, dir) = make_user_run(&store);
        let task = dir.join(run_dir::TASK_FILE);
        std::fs::write(&task, "x").unwrap();

        let err = register_user_output(&store, &integrator, 1, &task, None, &[])
            .expect_err("保護ファイルは拒否");
        assert!(matches!(err, UserWriteError::Protected(_)));

        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }

    #[test]
    fn creates_waggle_and_origin_connections() {
        let _env = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tmp_project("conn");
        let store = SaiStore::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let integrator =
            WaggleIntegrator::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();

        // 入力トークンの出力元となる実行
        let src_block = store.insert_block(&sample_block("src")).unwrap();
        let src_exec = store.create_execution(src_block, None).unwrap();
        store
            .update_execution_waggle_tokens(src_exec, &["AbCdEfGh".to_string()])
            .unwrap();

        let (run_id, dir) = make_user_run(&store);
        // task.md に来歴（origin_run_id = src_exec）を書く
        std::fs::write(
            dir.join(run_dir::TASK_FILE),
            format!(
                "---\nrunner: user\ncreation_date: 2026/09/17 10:00:00\norigin_run_id: {}\nrun_id: {}\n---\n\n# task_text\n",
                src_exec, run_id
            ),
        )
        .unwrap();

        let source = write_source(&tmp, "docs/c.md", "内容C");
        register_user_output(
            &store,
            &integrator,
            run_id,
            &source,
            None,
            &["AbCdEfGh".to_string()],
        )
        .unwrap();

        assert!(store
            .connection_exists(src_exec, run_id, "waggle_token")
            .unwrap());
        assert!(store
            .connection_exists(src_exec, run_id, "kanban_origin")
            .unwrap());

        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }

    #[test]
    fn rejects_empty_file_without_moving() {
        let _env = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tmp_project("empty");
        let store = SaiStore::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let integrator =
            WaggleIntegrator::open(&tmp.join(".sai/sai-engine/sai-engine.db")).unwrap();
        let (run_id, _dir) = make_user_run(&store);
        let source = write_source(&tmp, "docs/empty.md", "");
        let err = register_user_output(&store, &integrator, run_id, &source, None, &[])
            .expect_err("空ファイルは拒否");
        assert!(matches!(err, UserWriteError::Tokenize(_)));
        assert!(source.exists(), "空ファイルは移動しない");
        assert!(tokens_of(&store, run_id).is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }
}
