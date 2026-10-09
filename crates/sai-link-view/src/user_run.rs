//! 利用者の札（`runner: user`）の開始・終了の処理（TUI と `sai-link phone user` 共通・実装計画 U17）。
//!
//! TUI の CLI モード（`sai-link-tui/src/app/cli_mode.rs`）に
//! あった「todo のカードを running にする」「実行中の札を review へ移す」を
//! そのまま移したもの。TUI と `sai-link phone user start|finish` が
//! 同じ関数を使うので、両者の挙動が食い違わない。
//!
//! 利用者の札は常駐（デーモン）を要らない。`phone card run`（常駐に実行を
//! 依頼する命令）とは別の道筋で、同じ置き場所へ動かす。

use sai_link_engine::db::SaiStore;
use sai_link_engine::kanban::KanbanCard;

// ============================================================================
// 始める（todo → running）
// ============================================================================

/// `runner: user` のカードを running にする（TUI の Enter と `phone user start` が共有）。
///
/// やり方は TUI の今までの処理と同じ。
///
/// 1. `task.md` の `run_id:` を読む
/// 2. プレースホルダーブロック `__user__`（表示名 `user`）に紐付ける
/// 3. 予約行を確定させる（`claim_reserved_run`）
/// 4. `runner` を `user` にし、`round` / `global_round` を DB に反映する
/// 5. `status` を `running` にしてから run フォルダを running へ移す
///    （移せなければ `todo` に戻す）
///
/// `runner: user` でなければ `Err`。
pub fn start_user_run(store: &SaiStore, run_id: i64, card: &KanbanCard) -> Result<(), String> {
    if !card.frontmatter.is_user_runner() {
        return Err("runner: user のカードではありません".to_string());
    }
    // user タスクはプレースホルダーブロック `__user__`（表示名 `user`）に紐付ける。
    let block_id = store
        .get_or_create_user_block()
        .map_err(|e| format!("failed to resolve user block: {}", e))?;
    store
        .claim_reserved_run(run_id, block_id, None)
        .map_err(|e| format!("failed to claim reserved run: {}", e))?;
    let _ = store.update_execution_runner(run_id, "user");
    // task.md frontmatter の round / global_round を DB に反映する。
    // 未指定なら変更しない（sidebar が R?/r? のままになるのは許容）。
    let round = parse_round_field(&card.frontmatter.round, "round")?;
    let global_round = parse_round_field(&card.frontmatter.global_round, "global_round")?;
    if round.is_some() || global_round.is_some() {
        store
            .update_execution_rounds(run_id, round, global_round)
            .map_err(|e| format!("failed to update rounds: {}", e))?;
    }
    store
        .update_execution_status(run_id, "running")
        .map_err(|e| format!("failed to update status: {}", e))?;
    if let Some((from, _)) = sai_link_engine::run_dir::find_run_dir(run_id) {
        if from != sai_link_engine::run_dir::RUNNING_OWNER {
            if let Err(e) = sai_link_engine::run_dir::move_run_dir(
                run_id,
                &from,
                sai_link_engine::run_dir::RUNNING_OWNER,
            ) {
                let _ = store.update_execution_status(run_id, "todo");
                return Err(format!("failed to move folder (rolled back): {}", e));
            }
        }
    }
    Ok(())
}

// ============================================================================
// 終える（running → review）
// ============================================================================

/// 実行中の利用者の札を review へ移す（`phone user finish`）。
///
/// 実行が見つからない場合・`runner` が `user` で無い場合・`running` で無い場合は
/// `Err`。通れば [`move_running_to_review`] と同じ動きをする。
pub fn finish_user_run(store: &SaiStore, run_id: i64) -> Result<(), String> {
    let exec = store
        .get_execution_by_id(run_id)
        .map_err(|_| "実行が見つかりません".to_string())?;
    if exec.runner != "user" || exec.status != "running" {
        return Err("実行中の利用者の札ではありません".to_string());
    }
    move_running_to_review(store, run_id)
}

/// running → review。DB `status=completed` にしてから run フォルダを移動する。
///
/// `runner: user` でも `runner: pi` でも使える（`pi` は本来
/// sai-finalize が担当するが、finalize 失敗時のエラー対応として TUI から
/// 確定できる）。
///
/// サブタスク（`parent_run_id` あり・未確定）は移動しない。子の pane は親の
/// `task_manage(finalize)` / `delete` まで生きており、`session/*.jsonl` へ
/// 書き続けるため、ここで `review` へ移すと `ENOENT` で停止する。確定後に
/// task_manager 側が移動する。
pub fn move_running_to_review(store: &SaiStore, id: i64) -> Result<(), String> {
    // サブタスクの明示移動を拒否する
    if let Ok(exec) = store.get_execution_by_id(id) {
        if exec.parent_run_id.is_some() {
            let confirmed = store
                .get_task_by_execution_id(id)
                .ok()
                .flatten()
                .map(|t| t.finalized || t.status == "finalized" || t.status == "deleted")
                .unwrap_or(false);
            if !confirmed {
                return Err(format!(
                    "ID{} は実行中のサブタスクのため移動できません（親が task_manage finalize/delete で確定後に移動します）",
                    id
                ));
            }
        }
    }
    store
        .update_execution_status(id, "completed")
        .map_err(|e| format!("move failed: {}", e))?;
    if let Some((from, _)) = sai_link_engine::run_dir::find_run_dir(id) {
        if from != sai_link_engine::run_dir::REVIEW_OWNER {
            if let Err(e) = sai_link_engine::run_dir::move_run_dir(
                id,
                &from,
                sai_link_engine::run_dir::REVIEW_OWNER,
            ) {
                let _ = store.update_execution_status(id, "running");
                return Err(format!("move failed (rolled back): {}", e));
            }
        }
    }
    Ok(())
}

// ============================================================================
// 補助
// ============================================================================

/// task.md frontmatter の round / global_round をパースする。
///
/// 空欄・`-` は `None`（未指定・変更なし）。数値以外はエラー文字列を返す。
pub fn parse_round_field(value: &str, label: &str) -> Result<Option<i64>, String> {
    let t = value.trim();
    if t.is_empty() || t == "-" {
        return Ok(None);
    }
    t.parse::<i64>()
        .map(Some)
        .map_err(|_| format!("task.md `{}:` is not a number: {}", label, t))
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::MutexGuard;

    /// テスト用の排他（ロックが壊れていてもテストは続行する）。
    fn env_lock() -> MutexGuard<'static, ()> {
        crate::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 一時フォルダのテスト用プロジェクト（`SAI_PROJECT_ROOT` を向ける）。
    ///
    /// run フォルダは `SAI_PROJECT_ROOT` を見るため、テスト中だけここへ
    /// 向ける（Drop で元に戻す）。同じクレート内で env を触るのはこの
    /// テストだけなので、並行実行で衝突しない。
    struct Project {
        root: std::path::PathBuf,
        _guard: EnvGuard,
    }

    /// `SAI_PROJECT_ROOT` を一時ディレクトリに向けるガード（Drop で元に戻す）。
    struct EnvGuard(Option<std::ffi::OsString>);

    impl EnvGuard {
        fn set(path: &Path) -> Self {
            let prev = std::env::var_os("SAI_PROJECT_ROOT");
            std::env::set_var("SAI_PROJECT_ROOT", path);
            Self(prev)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.0.take() {
                Some(v) => std::env::set_var("SAI_PROJECT_ROOT", v),
                None => std::env::remove_var("SAI_PROJECT_ROOT"),
            }
        }
    }

    impl Project {
        /// 一時フォルダのプロジェクトを作る。
        fn new() -> Project {
            let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
            let root =
                std::env::temp_dir().join(format!("sai_user_run_{}_{}", std::process::id(), nanos));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let guard = EnvGuard::set(&root);
            Project {
                root,
                _guard: guard,
            }
        }

        /// todo に `runner: user` のカード（予約行つき）を書き出す。
        fn write_user_card(&self, run_id: i64, runner: &str) -> KanbanCard {
            let dir = sai_link_engine::run_dir::ensure_run_dir(
                sai_link_engine::run_dir::TODO_OWNER,
                run_id,
            )
            .unwrap();
            let text = format!(
                "---\nrunner: {}\ncreation_date: 2026/09/24 00:00:00\nexecution_date: -\norigin_run_id: -\nrun_id: {}\nround: 3\nglobal_round: 7\ndescription: テストカード\n---\n\n# task_text\n\nテスト\n",
                runner, run_id
            );
            std::fs::write(dir.join(sai_link_engine::run_dir::TASK_FILE), text).unwrap();
            sai_link_engine::kanban::parse_card(
                &std::fs::read_to_string(dir.join(sai_link_engine::run_dir::TASK_FILE)).unwrap(),
            )
            .expect("カードを読める")
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// `start` と `finish` が DB と run フォルダを、順番に動かすこと。
    #[test]
    fn start_and_finish_user_run_move_folders() {
        let _env = env_lock();
        let project = Project::new();
        let store = SaiStore::open_in_memory().unwrap();
        let run_id = store.create_reserved_run(None).unwrap();
        let card = project.write_user_card(run_id, "user");

        start_user_run(&store, run_id, &card).expect("始められる");
        let exec = store.get_execution_by_id(run_id).unwrap();
        assert_eq!(exec.status, "running");
        assert_eq!(exec.runner, "user");
        assert_eq!(exec.round, Some(3));
        assert_eq!(exec.global_round, Some(7));
        let user_block = store.get_block_by_name("__user__").unwrap();
        assert_eq!(exec.block_id, user_block.id.unwrap(), "__user__ に紐付く");
        assert_eq!(
            sai_link_engine::run_dir::find_run_dir(run_id).unwrap().0,
            sai_link_engine::run_dir::RUNNING_OWNER
        );

        finish_user_run(&store, run_id).expect("終えられる");
        let exec = store.get_execution_by_id(run_id).unwrap();
        assert_eq!(exec.status, "completed");
        assert_eq!(
            sai_link_engine::run_dir::find_run_dir(run_id).unwrap().0,
            sai_link_engine::run_dir::REVIEW_OWNER
        );
    }

    /// `runner: user` でないカードは始めないこと。
    #[test]
    fn start_rejects_non_user_card() {
        let _env = env_lock();
        let project = Project::new();
        let store = SaiStore::open_in_memory().unwrap();
        let run_id = store.create_reserved_run(None).unwrap();
        let card = project.write_user_card(run_id, "pi");

        let err = start_user_run(&store, run_id, &card).unwrap_err();
        assert_eq!(err, "runner: user のカードではありません");
        // 何も動かない（予約行の確定にも到達しない）
        assert_eq!(store.get_execution_by_id(run_id).unwrap().status, "todo");
        assert_eq!(
            sai_link_engine::run_dir::find_run_dir(run_id).unwrap().0,
            sai_link_engine::run_dir::TODO_OWNER
        );
    }

    /// 終えるときは runner と状態を確かめること。
    #[test]
    fn finish_rejects_wrong_runner_or_status() {
        // ファイルも環境変数も使わない（メモリ内 DB だけで確かめる）。
        let store = SaiStore::open_in_memory().unwrap();
        let user_block = store.get_or_create_user_block().unwrap();

        // 存在しない実行
        let err = finish_user_run(&store, 999_999_999).unwrap_err();
        assert_eq!(err, "実行が見つかりません");

        // runner が user でない
        let pi_run = store.create_execution(user_block, None).unwrap();
        store.update_execution_status(pi_run, "running").unwrap();
        let err = finish_user_run(&store, pi_run).unwrap_err();
        assert_eq!(err, "実行中の利用者の札ではありません");
        assert_eq!(store.get_execution_by_id(pi_run).unwrap().status, "running");

        // runner は user でも running でない
        let user_run = store.create_reserved_run(None).unwrap();
        store
            .claim_reserved_run(user_run, user_block, None)
            .unwrap();
        store.update_execution_runner(user_run, "user").unwrap();
        let err = finish_user_run(&store, user_run).unwrap_err();
        assert_eq!(err, "実行中の利用者の札ではありません");
        assert_ne!(
            store.get_execution_by_id(user_run).unwrap().status,
            "completed"
        );
    }

    /// `round:` / `global_round:` の読み方（空欄・`-` は未指定・数値以外はエラー）。
    #[test]
    fn parse_round_field_accepts_dash_and_numbers() {
        assert_eq!(parse_round_field("", "round").unwrap(), None);
        assert_eq!(parse_round_field("  ", "round").unwrap(), None);
        assert_eq!(parse_round_field("-", "round").unwrap(), None);
        assert_eq!(parse_round_field(" 12 ", "round").unwrap(), Some(12));
        let err = parse_round_field("x", "global_round").unwrap_err();
        assert_eq!(err, "task.md `global_round:` is not a number: x");
    }
}
