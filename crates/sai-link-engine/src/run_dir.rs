//! run ディレクトリ管理（outputs 統一・R2）。
//!
//! 1回の実行（run）に紐づく全成果物を
//! `PROJECTROOT/.sai/outputs/<owner>/<run_id>/` に集約する。
//!
//! - Kanban 所有中は状態名フォルダ（`todo/running/review/done`）
//! - `d` で Workflow へ送った後は `R<global_round>` フォルダ

use std::path::PathBuf;

/// outputs ルート（プロジェクトルート基準）。
pub const OUTPUTS_DIR: &str = ".sai/outputs";
/// 未実行（Kanban 所有・予約行）。
pub const TODO_OWNER: &str = "todo";
/// 実行中。
pub const RUNNING_OWNER: &str = "running";
/// 完了・レビュー待ち。
pub const REVIEW_OWNER: &str = "review";
/// レビュー済み。
pub const DONE_OWNER: &str = "done";

/// run フォルダ内の固定ファイル名。
pub const TASK_FILE: &str = "task.md";
pub const REVIEW_FILE: &str = "review.md";
pub const MESSAGE_FILE: &str = "message.md";
/// session jsonl 置き場（run フォルダ直下）。
pub const SESSION_SUBDIR: &str = "session";

/// outputs ルートの絶対パスを返す。
pub fn outputs_root() -> PathBuf {
    crate::project::sai_path(OUTPUTS_DIR)
}

/// 指定 owner・run_id の run フォルダ絶対パスを返す（作成はしない）。
pub fn run_dir(owner: &str, run_id: i64) -> PathBuf {
    outputs_root().join(owner).join(run_id.to_string())
}

/// 状態から owner フォルダ名を決める（純関数）。
///
/// - `todo` → `todo`
/// - `pending` / `running` → `running`
/// - `completed` 系 → レビュー記入ありなら `done`、なければ `review`
/// - `workflow_moved=true` → `R<global_round>`（未設定時は `R0`）
pub fn owner_for(
    status: &str,
    review_done: bool,
    workflow_moved: bool,
    global_round: Option<i64>,
) -> String {
    if workflow_moved {
        return format!("R{}", global_round.unwrap_or(0));
    }
    match status {
        "todo" => TODO_OWNER.to_string(),
        "pending" | "running" => RUNNING_OWNER.to_string(),
        "completed" | "failed" | "cancelled" => {
            if review_done {
                DONE_OWNER.to_string()
            } else {
                REVIEW_OWNER.to_string()
            }
        }
        _ => REVIEW_OWNER.to_string(),
    }
}

/// サブタスク実行の run フォルダ owner を算出する（純関数）。
///
/// サブタスクは sai-write 検出時点で DB が `completed`（仮完了）になるが、pane は
/// 親の `task_manage(finalize)` / `delete` まで生きており、`session/*.jsonl` へ
/// 書き続ける。この間に folder を review へ移すと書き込みが `ENOENT` で止まるため、
/// 確定（finalized / deleted）するまで `running` を維持する。
///
/// * 未確定（`confirmed == false`）かつ pane が生き得る状態
///   （`pending` / `running` / `completed`）→ `running`
/// * 確定済み → [`owner_for`] と同じ。ただし実行レコードの status が
///   `running` / `pending` のままでも pane は閉じているため `completed` 相当として
///   扱い `review` へ送る（delete 直後の status 更新競合で巻き戻らないようにする）
/// * `failed` / `cancelled` は pane が閉じているため確定前でも通常どおり `review`
pub fn subtask_owner(
    status: &str,
    confirmed: bool,
    review_done: bool,
    workflow_moved: bool,
    global_round: Option<i64>,
) -> String {
    if !confirmed && matches!(status, "pending" | "running" | "completed") {
        return RUNNING_OWNER.to_string();
    }
    let effective = if confirmed && matches!(status, "pending" | "running") {
        "completed"
    } else {
        status
    };
    owner_for(effective, review_done, workflow_moved, global_round)
}

/// run_id の実在フォルダを探す（全 owner 走査）。
///
/// `todo/running/review/done` の固定4種＋ `R*` を走査し、
/// 最初に見つかった `(owner, path)` を返す。無ければ `None`。
pub fn find_run_dir(run_id: i64) -> Option<(String, PathBuf)> {
    let root = outputs_root();
    let name = run_id.to_string();
    let mut owners = vec![
        TODO_OWNER.to_string(),
        RUNNING_OWNER.to_string(),
        REVIEW_OWNER.to_string(),
        DONE_OWNER.to_string(),
    ];
    if let Ok(entries) = std::fs::read_dir(&root) {
        let mut rounds: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .filter(|s| s.starts_with('R') && s.len() > 1)
            .collect();
        rounds.sort();
        owners.extend(rounds);
    }
    for owner in owners {
        let p = root.join(&owner).join(&name);
        if p.is_dir() {
            return Some((owner, p));
        }
    }
    None
}

/// run フォルダを確保する（無ければ作成）。`find` 優先、無ければ指定 owner に作成。
pub fn ensure_run_dir(owner: &str, run_id: i64) -> std::io::Result<PathBuf> {
    if let Some((_, p)) = find_run_dir(run_id) {
        return Ok(p);
    }
    let p = run_dir(owner, run_id);
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

/// サブタスク実行が確定済み（finalized / deleted）かどうかを判定する。
///
/// メインブロック（`parent_run_id` なし）は `true` を返す。
/// サブタスクは `tasks.execution_id` で紐付くタスクを引いて判定する。
/// タスクが引けない場合は、実行中の（`pending` / `running`）実行のみ
/// 未確定扱いにする（初回実行では実行レコード作成後、パイプライン完了まで
/// `tasks.execution_id` が未設定のため）。それ以外は旧ラウンド等の
/// 完了済み実行とみなし、従来どおり [`owner_for`] の結果に従う。
fn subtask_confirmed(
    store: &crate::db::SaiStore,
    exec: &crate::db::models::ExecutionHistory,
) -> bool {
    if exec.parent_run_id.is_none() {
        return true;
    }
    let Some(run_id) = exec.id else {
        return true;
    };
    match store.get_task_by_execution_id(run_id) {
        Ok(Some(task)) => task.finalized || task.status == "finalized" || task.status == "deleted",
        // タスク未紐付：実行中なら未確定（pane 稼働中）、それ以外は確定扱い
        Ok(None) => !matches!(exec.status.as_str(), "pending" | "running"),
        Err(_) => true,
    }
}

/// 実行の望ましい owner を算出し、現在の run フォルダと異なれば移動する。
///
/// `todo → running → review → done → workflow` の状態進行に合わせて
/// run フォルダを追従させる。TUI のリロード（起動時・`Shift+R`）から呼ばれる。
///
/// - run フォルダが存在しない実行（subtask 未起動等）は何もしない
/// - 移動はベストエフォート（失敗は黙殺し、次回リロードで再試行）
/// - review 記入の有無は **DB の `review_done`** で判定する（review.md の本文判定は廃止）。
///   review→done は run-tui の `e` でのみ遷移する。
/// - 後方互換: 既に `done` フォルダにある実行は `review_done=true` に補正する
///   （旧実装が review.md 本文から自動移動していた分を維持するため）。
/// - サブタスク（`parent_run_id` あり）は、親の `task_manage(finalize)` / `delete` で
///   確定するまで `running` を維持する（[`subtask_owner`]）。確定後は通常どおり
///   `review` へ移す。
pub fn sync_run_owner(
    store: &crate::db::SaiStore,
    exec: &mut crate::db::models::ExecutionHistory,
) {
    let Some(run_id) = exec.id else { return };
    let Some((current, _dir)) = find_run_dir(run_id) else {
        // run フォルダが無い実行（DB のみ・subtask 未起動等）は追跡対象外
        return;
    };
    // 後方互換: done フォルダに実体があるのに DB が review のままなら埋める。
    // （review.md 本文による自動 done 判定を廃止した分の移行処理）
    if current == DONE_OWNER
        && !exec.review_done
        && store.update_execution_review_done(run_id, true).is_ok()
    {
        exec.review_done = true;
    }
    let desired = if exec.parent_run_id.is_some() {
        subtask_owner(
            &exec.status,
            subtask_confirmed(store, exec),
            exec.review_done,
            exec.workflow_moved,
            exec.global_round,
        )
    } else {
        owner_for(
            &exec.status,
            exec.review_done,
            exec.workflow_moved,
            exec.global_round,
        )
    };
    if current != desired {
        let _ = move_run_dir(run_id, &current, &desired);
    }
}

/// タスク確定（`task_manage(finalize)` / `delete`）後に呼ぶ owner 同期。
///
/// pane を閉じた後（書き込みが止まった後）に呼ぶこと。実行レコードを読み直し、
/// [`sync_run_owner`] で望ましい owner（確定済みなので通常 `review`）へ移す。
/// 実行 ID が `None` / 取得失敗の場合は何もしない。
pub fn sync_run_owner_by_execution_id(store: &crate::db::SaiStore, exec_id: i64) {
    if let Ok(mut exec) = store.get_execution_by_id(exec_id) {
        sync_run_owner(store, &mut exec);
    }
}

/// run フォルダを移動する（DB 更新後に呼ぶこと・R404）。
///
/// 同一ディスク内の rename のみ。移動元が無ければ何もしない（Ok）。
/// 移動先が既存の場合はエラーにする（黙殺禁止）。
pub fn move_run_dir(run_id: i64, from_owner: &str, to_owner: &str) -> std::io::Result<PathBuf> {
    if from_owner == to_owner {
        return Ok(run_dir(to_owner, run_id));
    }
    let src = run_dir(from_owner, run_id);
    if !src.exists() {
        // 実体が別 owner にある場合はそちらを正とする
        if let Some((owner, p)) = find_run_dir(run_id) {
            if owner == to_owner {
                return Ok(p);
            }
            let dst = run_dir(to_owner, run_id);
            if dst.exists() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("移動先が既に存在: {}", dst.display()),
                ));
            }
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&p, &dst)?;
            return Ok(dst);
        }
        return Ok(run_dir(to_owner, run_id));
    }
    let dst = run_dir(to_owner, run_id);
    if dst.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("移動先が既に存在: {}", dst.display()),
        ));
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&src, &dst)?;
    Ok(dst)
}

/// run フォルダを全 owner から削除する（全削除・R601）。
///
/// 見つからなければ Ok（冪等）。削除したパス数を返す。
pub fn remove_run_dir(run_id: i64) -> std::io::Result<usize> {
    let root = outputs_root();
    let name = run_id.to_string();
    let mut removed = 0usize;
    let mut owners = vec![
        TODO_OWNER.to_string(),
        RUNNING_OWNER.to_string(),
        REVIEW_OWNER.to_string(),
        DONE_OWNER.to_string(),
    ];
    if let Ok(entries) = std::fs::read_dir(&root) {
        let mut rounds: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .filter(|s| s.starts_with('R') && s.len() > 1)
            .collect();
        rounds.sort();
        owners.extend(rounds);
    }
    for owner in owners {
        let p = root.join(&owner).join(&name);
        if p.exists() {
            std::fs::remove_dir_all(&p)?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// 実行を消す（DB の行と run フォルダ。子の実行も一緒に消す）。
///
/// CLI の `block rm`・TUI の rm・`phone rm` 共通。子の run フォルダも残さない。
pub fn delete_run(
    store: &crate::db::SaiStore,
    id: i64,
) -> Result<crate::db::models::DeleteExecutionSummary, String> {
    // DB から消す前に、子・孫の run_id を集める（消した後では辿れない）
    let mut ids = vec![id];
    let mut i = 0;
    while i < ids.len() {
        ids.extend(store.list_child_execution_ids(ids[i]).unwrap_or_default());
        i += 1;
    }
    let summary = store.delete_execution_cascade(id).map_err(|e| e.to_string())?;
    for rid in ids {
        if let Err(e) = crate::orchestrator::remove_run_dir_files(rid) {
            eprintln!("警告: run フォルダ {} の削除に失敗: {}", rid, e);
        }
    }
    Ok(summary)
}

/// review → done。`review_done=true` にしてから run フォルダを移動する
/// （TUI の `e` / `sai-link phone advance` 共通・実装計画 A U10）。
///
/// review.md の本文内容では自動移動しない（ユーザーの明示操作のみ）。
/// run フォルダの移動に失敗した場合は DB を元に戻して `Err` を返す。
pub fn advance_review_to_done(store: &crate::db::SaiStore, id: i64) -> Result<(), String> {
    store
        .update_execution_review_done(id, true)
        .map_err(|e| e.to_string())?;
    if let Some((from, _)) = find_run_dir(id) {
        if from != DONE_OWNER {
            if let Err(e) = move_run_dir(id, &from, DONE_OWNER) {
                // 移動できないときは DB 側を元に戻す（次回の同期で review へ戻る）
                let _ = store.update_execution_review_done(id, false);
                return Err(e.to_string());
            }
        }
    }
    // 子（task_create の子）も親と一緒に done へ移す。まだ動いている子（running）は動かさない。
    for child in store.list_child_execution_ids(id).unwrap_or_default() {
        if matches!(find_run_dir(child), Some((o, _)) if o == REVIEW_OWNER) {
            let _ = advance_review_to_done(store, child);
        }
    }
    Ok(())
}

/// done → workflow。`workflow_moved=true` にしてから `R<global_round>` へ移動する
/// （TUI の `e` / `sai-link phone advance` 共通・実装計画 A U10）。
///
/// 移動先（`R<gr>`）を返す。run フォルダの移動に失敗した場合は DB を元に戻して `Err`。
pub fn advance_done_to_workflow(store: &crate::db::SaiStore, id: i64) -> Result<String, String> {
    store
        .update_execution_workflow_moved(id, true)
        .map_err(|e| e.to_string())?;
    let global_round = store
        .get_execution_by_id(id)
        .ok()
        .and_then(|e| e.global_round);
    let to = format!("R{}", global_round.unwrap_or(0));
    if let Some((from, _)) = find_run_dir(id) {
        if from != to {
            if let Err(e) = move_run_dir(id, &from, &to) {
                // 移動できないときは DB 側を元に戻す
                let _ = store.update_execution_workflow_moved(id, false);
                return Err(e.to_string());
            }
        }
    }
    // 子も親と一緒に Workflow へ移す（review / done の子だけ。running の子は動かさない）。
    for child in store.list_child_execution_ids(id).unwrap_or_default() {
        if matches!(find_run_dir(child), Some((o, _)) if o == REVIEW_OWNER || o == DONE_OWNER) {
            let _ = store.update_execution_review_done(child, true);
            let _ = advance_done_to_workflow(store, child);
        }
    }
    Ok(to)
}

/// sai-write 用ファイル名のサニタイズ。
///
/// ディレクトリ部分・`../` を除去し、空なら `unnamed.md` を返す。
pub fn sanitize_filename(name: &str) -> String {
    let base = std::path::Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .trim();
    if base.is_empty() || base == "." || base == ".." {
        return "unnamed.md".to_string();
    }
    base.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_for_mapping() {
        assert_eq!(owner_for("todo", false, false, None), "todo");
        assert_eq!(owner_for("pending", false, false, None), "running");
        assert_eq!(owner_for("running", false, false, None), "running");
        assert_eq!(owner_for("completed", false, false, None), "review");
        assert_eq!(owner_for("completed", true, false, None), "done");
        assert_eq!(owner_for("completed", true, true, Some(2)), "R2");
        assert_eq!(owner_for("completed", false, true, None), "R0");
    }

    /// `subtask_owner`: サブタスクは未確定の間 `running` を維持し、
    /// 確定（finalized / deleted）後に `review` へ移る。
    #[test]
    fn subtask_owner_keeps_running_until_confirmed() {
        // 未確定: 仮完了（completed）でも running
        assert_eq!(
            subtask_owner("completed", false, false, false, None),
            "running"
        );
        assert_eq!(
            subtask_owner("running", false, false, false, None),
            "running"
        );
        assert_eq!(
            subtask_owner("pending", false, false, false, None),
            "running"
        );
        // 確定後: 通常の owner_for に従う
        assert_eq!(
            subtask_owner("completed", true, false, false, None),
            "review"
        );
        assert_eq!(subtask_owner("completed", true, true, false, None), "done");
        assert_eq!(subtask_owner("completed", true, true, true, Some(2)), "R2");
        // delete 直後の競合: status が running のままでも確定済みなら review
        assert_eq!(subtask_owner("running", true, false, false, None), "review");
        // 失敗・キャンセルは未確定でも pane が閉じているため review
        assert_eq!(subtask_owner("failed", false, false, false, None), "review");
        assert_eq!(
            subtask_owner("cancelled", false, false, false, None),
            "review"
        );
    }

    #[test]
    fn sanitize_strips_traversal() {
        assert_eq!(sanitize_filename("report.md"), "report.md");
        assert_eq!(sanitize_filename("../evil.md"), "evil.md");
        assert_eq!(sanitize_filename("a/b/c.md"), "c.md");
        assert_eq!(sanitize_filename(""), "unnamed.md");
        assert_eq!(sanitize_filename(".."), "unnamed.md");
    }

    #[test]
    fn move_and_remove_roundtrip() {
        // 環境変数（SAI_PROJECT_ROOT）を設定するため環境テストロックを取得
        let _env = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("sai_rundir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);
        let run_id = 777001;
        let p = ensure_run_dir(TODO_OWNER, run_id).unwrap();
        assert!(p.is_dir());
        std::fs::write(p.join(TASK_FILE), "x").unwrap();
        let moved = move_run_dir(run_id, TODO_OWNER, RUNNING_OWNER).unwrap();
        assert!(moved.join(TASK_FILE).exists());
        assert_eq!(find_run_dir(run_id).unwrap().0, RUNNING_OWNER);
        assert_eq!(remove_run_dir(run_id).unwrap(), 1);
        assert!(find_run_dir(run_id).is_none());
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }

    /// `sync_run_owner`: DB の `review_done` に従って review→done へ移動する。
    /// review.md 本文の記入では自動移動しない（明示 `e` のみ）。
    #[test]
    fn sync_run_owner_moves_review_to_done() {
        // 環境変数（SAI_PROJECT_ROOT）を設定するため環境テストロックを取得
        let _env = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("sai_sync_run_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".sai")).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);

        let store = crate::db::SaiStore::open_in_memory().unwrap();
        let block = crate::db::models::BlockMetadata::new(
            "b".to_string(),
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
            "blocks/b.md".to_string(),
            None
        );
        let block_id = store.insert_block(&block).unwrap();
        let run_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(run_id, "completed").unwrap();

        // run フォルダを review owner に用意する
        let dir = ensure_run_dir(REVIEW_OWNER, run_id).unwrap();
        std::fs::write(dir.join(REVIEW_FILE), "### 追加修正\n\n文章が簡単すぎる\n").unwrap();
        let mut exec = store.get_execution_by_id(run_id).unwrap();

        // review.md を記入しても DB が review_done=false なら review に留まる
        sync_run_owner(&store, &mut exec);
        assert_eq!(find_run_dir(run_id).unwrap().0, REVIEW_OWNER);

        // DB の review_done=true（run-tui `e` 相当）で done へ移動
        store.update_execution_review_done(run_id, true).unwrap();
        exec.review_done = true;
        sync_run_owner(&store, &mut exec);
        assert_eq!(find_run_dir(run_id).unwrap().0, DONE_OWNER);

        // 後方互換: done フォルダに実体があるのに DB が false なら補正される
        store.update_execution_review_done(run_id, false).unwrap();
        exec.review_done = false;
        sync_run_owner(&store, &mut exec);
        assert!(exec.review_done, "done フォルダから review_done を補正");
        assert_eq!(find_run_dir(run_id).unwrap().0, DONE_OWNER);

        // workflow_moved=true → R<global_round> へ移動
        exec.workflow_moved = true;
        exec.global_round = Some(3);
        sync_run_owner(&store, &mut exec);
        assert_eq!(find_run_dir(run_id).unwrap().0, "R3");

        // running に戻しても移動しない（status 優先）
        exec.status = "running".to_string();
        exec.workflow_moved = false;
        sync_run_owner(&store, &mut exec);
        assert_eq!(find_run_dir(run_id).unwrap().0, RUNNING_OWNER);

        remove_run_dir(run_id).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }

    /// `sync_run_owner`: サブタスクは sai-write 仮完了（completed）でも親の
    /// `task_manage(finalize)` / `delete` まで `running` に留まり、確定後に
    /// `review` へ移る。メインブロックの動きは変わらない。
    #[test]
    fn sync_run_owner_keeps_subtask_running_until_finalized() {
        let _env = crate::common::SAI_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("sai_sync_subtask_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".sai")).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);

        let store = crate::db::SaiStore::open_in_memory().unwrap();
        let block = crate::db::models::BlockMetadata::new(
            "b".to_string(),
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
            "blocks/b.md".to_string(),
            None
        );
        let block_id = store.insert_block(&block).unwrap();

        // --- サブタスク（finalize で確定するケース） ---
        let parent_run_id = store.create_execution(block_id, None).unwrap();
        let sub_run_id = store
            .create_execution_for_subtask(block_id, None, parent_run_id, None)
            .unwrap();
        let task_id = store
            .insert_task(&crate::db::models::Task::new(
                parent_run_id,
                "サブ".to_string(),
                "説明".to_string(),
                Some("b".to_string()),
                None,
                None,
                "none".to_string(),
                None,
                Some("s-1".to_string()),
            ))
            .unwrap();
        store.update_task_execution_id(task_id, sub_run_id).unwrap();
        // sai-write 検出直後: DB は completed（仮完了）、タスクは未確定
        store.update_task_status(task_id, "completed").unwrap();
        store
            .update_execution_status(sub_run_id, "completed")
            .unwrap();
        // pane が session/*.jsonl を書けるフォルダを running に用意する
        let dir = ensure_run_dir(RUNNING_OWNER, sub_run_id).unwrap();
        std::fs::create_dir_all(dir.join(SESSION_SUBDIR)).unwrap();

        let mut exec = store.get_execution_by_id(sub_run_id).unwrap();
        assert!(exec.parent_run_id.is_some());

        // 未確定の間は TUI リロード（sync）でも running のまま
        sync_run_owner(&store, &mut exec);
        assert_eq!(find_run_dir(sub_run_id).unwrap().0, RUNNING_OWNER);
        assert!(dir.join(SESSION_SUBDIR).is_dir(), "session フォルダは残る");

        // 親の task_manage(finalize) 相当で review へ
        // （task_manager は `sync_run_owner_by_execution_id` を呼ぶ）
        store.update_task_finalized(task_id, true).unwrap();
        store.update_task_status(task_id, "finalized").unwrap();
        sync_run_owner_by_execution_id(&store, sub_run_id);
        assert_eq!(find_run_dir(sub_run_id).unwrap().0, REVIEW_OWNER);

        // --- サブタスク（delete で確定するケース・status が running のまま） ---
        let sub_run_id2 = store
            .create_execution_for_subtask(block_id, None, parent_run_id, None)
            .unwrap();
        let task_id2 = store
            .insert_task(&crate::db::models::Task::new(
                parent_run_id,
                "サブ2".to_string(),
                "説明".to_string(),
                Some("b".to_string()),
                None,
                None,
                "none".to_string(),
                None,
                Some("s-2".to_string()),
            ))
            .unwrap();
        store
            .update_task_execution_id(task_id2, sub_run_id2)
            .unwrap();
        store.update_task_status(task_id2, "in_progress").unwrap();
        store
            .update_execution_status(sub_run_id2, "running")
            .unwrap();
        ensure_run_dir(RUNNING_OWNER, sub_run_id2).unwrap();
        let mut exec2 = store.get_execution_by_id(sub_run_id2).unwrap();

        // 削除前に running のまま（未確定で running でも no-op）
        sync_run_owner(&store, &mut exec2);
        assert_eq!(find_run_dir(sub_run_id2).unwrap().0, RUNNING_OWNER);

        // task_manage(delete) 相当: exec が running のままでも review へ
        store.mark_task_deleted(task_id2, "main").unwrap();
        sync_run_owner(&store, &mut exec2);
        assert_eq!(find_run_dir(sub_run_id2).unwrap().0, REVIEW_OWNER);

        // --- メインブロックの動きは変わらない ---
        let main_run_id = store.create_execution(block_id, None).unwrap();
        store
            .update_execution_status(main_run_id, "completed")
            .unwrap();
        ensure_run_dir(RUNNING_OWNER, main_run_id).unwrap();
        let mut main_exec = store.get_execution_by_id(main_run_id).unwrap();
        sync_run_owner(&store, &mut main_exec);
        assert_eq!(find_run_dir(main_run_id).unwrap().0, REVIEW_OWNER);

        // --- タスク未紐付のサブタスク（初回実行中）は running を維持する ---
        // `execute_subtask` はパイプライン完了後に `tasks.execution_id` を設定する
        // ため、実行中はタスクを引けない。それでも running を維持できること。
        let sub_run_id3 = store
            .create_execution_for_subtask(block_id, None, parent_run_id, None)
            .unwrap();
        store
            .update_execution_status(sub_run_id3, "running")
            .unwrap();
        ensure_run_dir(RUNNING_OWNER, sub_run_id3).unwrap();
        let mut exec3 = store.get_execution_by_id(sub_run_id3).unwrap();
        sync_run_owner(&store, &mut exec3);
        assert_eq!(find_run_dir(sub_run_id3).unwrap().0, RUNNING_OWNER);

        // 未紐付のまま完了した旧ラウンドは review へ（running に残さない）
        store
            .update_execution_status(sub_run_id3, "completed")
            .unwrap();
        let mut exec3 = store.get_execution_by_id(sub_run_id3).unwrap();
        sync_run_owner(&store, &mut exec3);
        assert_eq!(find_run_dir(sub_run_id3).unwrap().0, REVIEW_OWNER);

        remove_run_dir(sub_run_id).unwrap();
        remove_run_dir(sub_run_id2).unwrap();
        remove_run_dir(main_run_id).unwrap();
        remove_run_dir(sub_run_id3).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }

    /// `advance_review_to_done` / `advance_done_to_workflow`:
    /// review → done → `R<global_round>` と進み、DB と run フォルダが一致すること。
    #[test]
    fn advance_review_to_done_then_to_workflow() {
        let _env = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("sai_advance_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".sai")).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);

        let store = crate::db::SaiStore::open_in_memory().unwrap();
        let block = crate::db::models::BlockMetadata::new(
            "b".to_string(),
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
            "blocks/b.md".to_string(),
            None
        );
        let block_id = store.insert_block(&block).unwrap();
        let run_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(run_id, "completed").unwrap();
        store
            .update_execution_rounds(run_id, Some(1), Some(3))
            .unwrap();
        let dir = ensure_run_dir(REVIEW_OWNER, run_id).unwrap();
        std::fs::write(dir.join(REVIEW_FILE), "### 追加修正\n").unwrap();

        // review → done
        advance_review_to_done(&store, run_id).unwrap();
        assert!(store.get_execution_by_id(run_id).unwrap().review_done);
        assert_eq!(find_run_dir(run_id).unwrap().0, DONE_OWNER);

        // done → workflow（移動先は global_round の `R<gr>`）
        let to = advance_done_to_workflow(&store, run_id).unwrap();
        assert_eq!(to, "R3");
        assert!(store
            .get_execution_by_id(run_id)
            .unwrap()
            .workflow_moved);
        assert_eq!(find_run_dir(run_id).unwrap().0, "R3");

        // 移動先が既にあり移動できないときは DB を元に戻して Err
        let other = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(other, "completed").unwrap();
        store.update_execution_rounds(other, Some(1), Some(3)).unwrap();
        ensure_run_dir(DONE_OWNER, other).unwrap();
        // `R3/other` も作っておき、移動先を塞いで移動失敗・巻き戻しを確かめる
        std::fs::create_dir_all(run_dir("R3", other)).unwrap();
        let err = advance_done_to_workflow(&store, other).unwrap_err();
        assert!(!err.is_empty());
        assert!(
            !store.get_execution_by_id(other).unwrap().workflow_moved,
            "移動できなければ workflow_moved を元に戻す"
        );

        remove_run_dir(run_id).unwrap();
        remove_run_dir(other).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("SAI_PROJECT_ROOT");
    }
}
