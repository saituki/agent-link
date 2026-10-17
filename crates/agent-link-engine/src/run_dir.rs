//! run ディレクトリ管理（outputs 統一・R2）。
//!
//! 1回の実行（run）に紐づく全成果物を
//! `PROJECTROOT/.agent-link/outputs/<owner>/<run_id>/` に集約する。
//!
//! - Kanban 所有中は状態名フォルダ（`todo/running/review`）
//! - Workflow へ移した後は日付フォルダ（`<yymmdd>`・[`run_date`]）

use std::path::PathBuf;

/// outputs ルート（プロジェクトルート基準）。
pub const OUTPUTS_DIR: &str = ".agent-link/outputs";
/// 未実行（Kanban 所有・予約行）。
pub const TODO_OWNER: &str = "todo";
/// 実行中。
pub const RUNNING_OWNER: &str = "running";
/// 完了・レビュー待ち。
pub const REVIEW_OWNER: &str = "review";

/// run フォルダ内の固定ファイル名。
pub const TASK_FILE: &str = "task.md";
pub const MESSAGE_FILE: &str = "message.md";
/// 固定部分（system prompt）の置き場（run フォルダ直下）。
/// cc は `--append-system-prompt-file` でこのファイルを渡す。
pub const SYSTEM_PROMPT_FILE: &str = "system-prompt.md";
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

/// DB の時刻（UTC の `YYYY-MM-DD HH:MM:SS`）をローカル時刻に直す関数を差し込んで日付 `yymmdd` を作る。
fn run_date_with(
    started_at: Option<&str>,
    created_at: &str,
    to_local: impl Fn(chrono::NaiveDateTime) -> chrono::NaiveDateTime,
) -> String {
    let parse = |s: &str| chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S").ok();
    match started_at.and_then(parse).or_else(|| parse(created_at)) {
        Some(utc) => to_local(utc).format("%y%m%d").to_string(),
        None => "000000".to_string(),
    }
}

/// DB の時刻（UTC の "YYYY-MM-DD HH:MM:SS"）をローカル時刻の日付 "yymmdd" にする。
/// started_at が None か読めなければ created_at を使う。どちらも読めなければ "000000"。
pub fn run_date(started_at: Option<&str>, created_at: &str) -> String {
    use chrono::TimeZone;
    run_date_with(started_at, created_at, |utc| {
        chrono::Local.from_utc_datetime(&utc).naive_local()
    })
}

/// 一番上の親 run（parent_run_id を上へたどる。最大 32 段）の run_date を返す。
pub fn root_run_date(
    store: &crate::db::SaiStore,
    exec: &crate::db::models::ExecutionHistory,
) -> String {
    let mut cur = exec.clone();
    for _ in 0..32 {
        let Some(parent) = cur.parent_run_id else { break };
        match store.get_execution_by_id(parent) {
            Ok(p) => cur = p,
            Err(_) => break,
        }
    }
    run_date(cur.started_at.as_deref(), &cur.created_at)
}

/// フォルダ名が日付フォルダ（ASCII の数字 6 桁）かどうか。
pub fn is_date_owner(name: &str) -> bool {
    name.len() == 6 && name.bytes().all(|b| b.is_ascii_digit())
}

/// 状態から owner フォルダ名を決める（純関数）。
///
/// - `workflow_moved=true` → `date`（`yymmdd`）
/// - `todo` → `todo`
/// - `pending` / `running` → `running`
/// - それ以外 → `review`
pub fn owner_for(status: &str, workflow_moved: bool, date: &str) -> String {
    if workflow_moved {
        return date.to_string();
    }
    match status {
        "todo" => TODO_OWNER.to_string(),
        "pending" | "running" => RUNNING_OWNER.to_string(),
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
    workflow_moved: bool,
    date: &str,
) -> String {
    if !confirmed && matches!(status, "pending" | "running" | "completed") {
        return RUNNING_OWNER.to_string();
    }
    let effective = if confirmed && matches!(status, "pending" | "running") {
        "completed"
    } else {
        status
    };
    owner_for(effective, workflow_moved, date)
}

/// 走査する owner の並び（`todo/running/review` → 日付フォルダの昇順）。
fn scan_owners(root: &std::path::Path) -> Vec<String> {
    let mut owners = vec![
        TODO_OWNER.to_string(),
        RUNNING_OWNER.to_string(),
        REVIEW_OWNER.to_string(),
    ];
    if let Ok(entries) = std::fs::read_dir(root) {
        let mut dates: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .filter(|s| is_date_owner(s))
            .collect();
        dates.sort();
        owners.extend(dates);
    }
    owners
}

/// run_id の実在フォルダを探す（全 owner 走査）。
///
/// `todo/running/review` ＋日付フォルダ（昇順）を走査し、
/// 最初に見つかった `(owner, path)` を返す。無ければ `None`。
pub fn find_run_dir(run_id: i64) -> Option<(String, PathBuf)> {
    let root = outputs_root();
    let name = run_id.to_string();
    scan_owners(&root).into_iter().find_map(|owner| {
        let p = root.join(&owner).join(&name);
        p.is_dir().then_some((owner, p))
    })
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
/// `todo → running → review → workflow（日付フォルダ）` の状態進行に合わせて
/// run フォルダを追従させる。`run tree` / `run workflow` の表示のたびに呼ばれる。
///
/// - run フォルダが存在しない実行（subtask 未起動等）は何もしない
/// - 移動はベストエフォート（失敗は黙殺し、次回に再試行）
/// - review → Workflow は明示操作（[`advance_review_to_workflow`]）でのみ遷移する。
/// - サブタスク（`parent_run_id` あり）は、親の `task_manage(finalize)` / `delete` で
///   確定するまで `running` を維持する（[`subtask_owner`]）。確定後は通常どおり
///   `review` へ移す。ただし親が既に Workflow へ移っているときは、確定した子にも
///   `workflow_moved` を付けて親と同じ日付フォルダへ移す（review に取り残さない）。
pub fn sync_run_owner(
    store: &crate::db::SaiStore,
    exec: &mut crate::db::models::ExecutionHistory,
) {
    let Some(run_id) = exec.id else { return };
    let Some((current, _dir)) = find_run_dir(run_id) else {
        // run フォルダが無い実行（DB のみ・subtask 未起動等）は追跡対象外
        return;
    };
    let confirmed = subtask_confirmed(store, exec);
    let desired_for = |exec: &crate::db::models::ExecutionHistory| {
        // 日付は Workflow に移った run だけが使う
        let date = if exec.workflow_moved {
            root_run_date(store, exec)
        } else {
            String::new()
        };
        if exec.parent_run_id.is_some() {
            subtask_owner(&exec.status, confirmed, exec.workflow_moved, &date)
        } else {
            owner_for(&exec.status, exec.workflow_moved, &date)
        }
    };
    let mut desired = desired_for(exec);
    if desired == REVIEW_OWNER && confirmed {
        let parent_moved = exec
            .parent_run_id
            .and_then(|p| store.get_execution_by_id(p).ok())
            .is_some_and(|p| p.workflow_moved);
        if parent_moved && store.update_execution_workflow_moved(run_id, true).is_ok() {
            exec.workflow_moved = true;
            desired = desired_for(exec);
        }
    }
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
                    format!("Destination already exists: {}", dst.display()),
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
            format!("Destination already exists: {}", dst.display()),
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
    for owner in scan_owners(&root) {
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
/// CLI の `block rm`・`run rm` 共通。子の run フォルダも残さない。
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
            eprintln!("Warning: failed to delete run folder {}: {}", rid, e);
        }
    }
    Ok(summary)
}

/// review → Workflow。`workflow_moved=true` にしてから日付フォルダへ移す
/// （`agent-link run advance` 用）。
///
/// 移した先（`yymmdd`）を返す。run フォルダの移動に失敗した場合は DB を元に戻して `Err`。
/// 子（`list_child_execution_ids`。再帰で孫も）のうち review にあるものも同じ日付フォルダへ移す。
/// running の子は動かさない。
pub fn advance_review_to_workflow(store: &crate::db::SaiStore, id: i64) -> Result<String, String> {
    store
        .update_execution_workflow_moved(id, true)
        .map_err(|e| e.to_string())?;
    let to = match store.get_execution_by_id(id) {
        Ok(exec) => root_run_date(store, &exec),
        Err(e) => {
            let _ = store.update_execution_workflow_moved(id, false);
            return Err(e.to_string());
        }
    };
    if let Some((from, _)) = find_run_dir(id) {
        if from != to {
            if let Err(e) = move_run_dir(id, &from, &to) {
                // 移動できないときは DB 側を元に戻す
                let _ = store.update_execution_workflow_moved(id, false);
                return Err(e.to_string());
            }
        }
    }
    for child in store.list_child_execution_ids(id).unwrap_or_default() {
        if matches!(find_run_dir(child), Some((o, _)) if o == REVIEW_OWNER) {
            let _ = advance_review_to_workflow(store, child);
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
        assert_eq!(owner_for("todo", false, ""), "todo");
        assert_eq!(owner_for("pending", false, ""), "running");
        assert_eq!(owner_for("running", false, ""), "running");
        assert_eq!(owner_for("completed", false, ""), "review");
        assert_eq!(owner_for("failed", false, ""), "review");
        assert_eq!(owner_for("completed", true, "261016"), "261016");
    }

    #[test]
    fn run_date_converts_utc_to_local_and_falls_back() {
        let jst = |n: chrono::NaiveDateTime| n + chrono::Duration::hours(9);
        // UTC 23:30 は JST で翌日
        assert_eq!(
            run_date_with(Some("2026-10-15 23:30:00"), "2026-10-01 00:00:00", jst),
            "261016"
        );
        // started_at が None / 読めない → created_at
        assert_eq!(run_date_with(None, "2026-10-01 00:00:00", jst), "261001");
        assert_eq!(
            run_date_with(Some("bad"), "2026-10-01 15:00:00", jst),
            "261002"
        );
        // どちらも読めない
        assert_eq!(run_date_with(None, "", jst), "000000");
        // 公開関数は少なくとも 6 桁の数字を返す
        assert!(is_date_owner(&run_date(None, "2026-10-01 00:00:00")));
    }

    #[test]
    fn is_date_owner_checks_six_ascii_digits() {
        assert!(is_date_owner("261016"));
        assert!(!is_date_owner("26101"));
        assert!(!is_date_owner("2610160"));
        assert!(!is_date_owner("R1"));
        assert!(!is_date_owner("done"));
        assert!(!is_date_owner("26101a"));
        assert!(!is_date_owner("２６１０１６"));
    }

    /// `subtask_owner`: サブタスクは未確定の間 `running` を維持し、
    /// 確定（finalized / deleted）後に `review` へ移る。
    #[test]
    fn subtask_owner_keeps_running_until_confirmed() {
        // 未確定: 仮完了（completed）でも running
        assert_eq!(subtask_owner("completed", false, false, ""), "running");
        assert_eq!(subtask_owner("running", false, false, ""), "running");
        assert_eq!(subtask_owner("pending", false, false, ""), "running");
        // 確定後: 通常の owner_for に従う
        assert_eq!(subtask_owner("completed", true, false, ""), "review");
        assert_eq!(subtask_owner("completed", true, true, "261016"), "261016");
        // delete 直後の競合: status が running のままでも確定済みなら review
        assert_eq!(subtask_owner("running", true, false, ""), "review");
        // 失敗・キャンセルは未確定でも pane が閉じているため review
        assert_eq!(subtask_owner("failed", false, false, ""), "review");
        assert_eq!(subtask_owner("cancelled", false, false, ""), "review");
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
        // 環境変数（AGENT_LINK_PROJECT_ROOT）を設定するため環境テストロックを取得
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("sai_rundir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::set_var("AGENT_LINK_PROJECT_ROOT", &tmp);
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
        std::env::remove_var("AGENT_LINK_PROJECT_ROOT");
    }

    /// テスト用: メモリ DB とブロックを作り、`(store, block_id)` を返す。
    fn test_store() -> (crate::db::SaiStore, i64) {
        let store = crate::db::SaiStore::open_in_memory().unwrap();
        let block = crate::db::models::BlockMetadata::new(
            "b".to_string(),
            "テスト".to_string(),
            "m".to_string(),
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            "blocks/b.md".to_string(),
            None,
        );
        let block_id = store.insert_block(&block).unwrap();
        (store, block_id)
    }

    /// テスト用: 一時プロジェクトルートを環境変数に設定して返す。
    fn set_tmp_project(tag: &str) -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!("{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".agent-link")).unwrap();
        std::env::set_var("AGENT_LINK_PROJECT_ROOT", &tmp);
        tmp
    }

    /// `sync_run_owner`: review に留まり、`workflow_moved` で日付フォルダへ移る。
    #[test]
    fn sync_run_owner_moves_review_to_date_folder() {
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = set_tmp_project("sai_sync_run");
        let (store, block_id) = test_store();
        let run_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(run_id, "completed").unwrap();
        ensure_run_dir(REVIEW_OWNER, run_id).unwrap();
        let mut exec = store.get_execution_by_id(run_id).unwrap();

        // 明示操作が無ければ review のまま
        sync_run_owner(&store, &mut exec);
        assert_eq!(find_run_dir(run_id).unwrap().0, REVIEW_OWNER);

        // workflow_moved=true → 日付フォルダへ
        exec.workflow_moved = true;
        sync_run_owner(&store, &mut exec);
        let date = root_run_date(&store, &exec);
        assert!(is_date_owner(&date));
        assert_eq!(find_run_dir(run_id).unwrap().0, date);

        // running に戻しても移動しない（status 優先）
        exec.status = "running".to_string();
        exec.workflow_moved = false;
        sync_run_owner(&store, &mut exec);
        assert_eq!(find_run_dir(run_id).unwrap().0, RUNNING_OWNER);

        remove_run_dir(run_id).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("AGENT_LINK_PROJECT_ROOT");
    }

    /// `find_run_dir` / `remove_run_dir`: `R*`・`done` は探さず、日付フォルダは探す。
    #[test]
    fn find_run_dir_scans_date_folders_only() {
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = set_tmp_project("sai_find_date");
        std::fs::create_dir_all(run_dir("R3", 777101)).unwrap();
        std::fs::create_dir_all(run_dir("done", 777102)).unwrap();
        std::fs::create_dir_all(run_dir("261016", 777103)).unwrap();
        assert!(find_run_dir(777101).is_none());
        assert!(find_run_dir(777102).is_none());
        assert_eq!(find_run_dir(777103).unwrap().0, "261016");
        assert_eq!(remove_run_dir(777103).unwrap(), 1);
        assert_eq!(remove_run_dir(777101).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("AGENT_LINK_PROJECT_ROOT");
    }

    /// `sync_run_owner`: サブタスクは sai-write 仮完了（completed）でも親の
    /// `task_manage(finalize)` / `delete` まで `running` に留まり、確定後に
    /// `review` へ移る。メインブロックの動きは変わらない。
    #[test]
    fn sync_run_owner_keeps_subtask_running_until_finalized() {
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("sai_sync_subtask_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".agent-link")).unwrap();
        std::env::set_var("AGENT_LINK_PROJECT_ROOT", &tmp);

        let store = crate::db::SaiStore::open_in_memory().unwrap();
        let block = crate::db::models::BlockMetadata::new(
            "b".to_string(),
            "テスト".to_string(),
            "m".to_string(),
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

        // 未確定の間は sync でも running のまま
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
        std::env::remove_var("AGENT_LINK_PROJECT_ROOT");
    }

    /// `root_run_date`: 子は一番上の親の日付になる（子自身の日時は使わない）。
    #[test]
    fn root_run_date_uses_top_parent() {
        let (store, block_id) = test_store();
        let parent = store.create_execution(block_id, None).unwrap();
        let child = store
            .create_execution_for_subtask(block_id, None, parent, None)
            .unwrap();
        let grandchild = store
            .create_execution_for_subtask(block_id, None, child, None)
            .unwrap();
        let parent_exec = store.get_execution_by_id(parent).unwrap();
        let expected = run_date(parent_exec.started_at.as_deref(), &parent_exec.created_at);
        let mut g = store.get_execution_by_id(grandchild).unwrap();
        g.started_at = Some("2000-01-01 12:00:00".to_string());
        g.created_at = "2000-01-01 12:00:00".to_string();
        assert_ne!(run_date(g.started_at.as_deref(), &g.created_at), expected);
        assert_eq!(root_run_date(&store, &g), expected);
        assert_eq!(root_run_date(&store, &parent_exec), expected);
    }

    /// `advance_review_to_workflow`: review の親と子（孫も）が同じ日付フォルダへ移り、
    /// running の子は動かない。移動できなければ DB を戻して Err。
    #[test]
    fn advance_review_to_workflow_moves_review_children() {
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = set_tmp_project("sai_advance");
        let (store, block_id) = test_store();
        let parent = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(parent, "completed").unwrap();
        let child = store
            .create_execution_for_subtask(block_id, None, parent, None)
            .unwrap();
        let grandchild = store
            .create_execution_for_subtask(block_id, None, child, None)
            .unwrap();
        let running_child = store
            .create_execution_for_subtask(block_id, None, parent, None)
            .unwrap();
        ensure_run_dir(REVIEW_OWNER, parent).unwrap();
        ensure_run_dir(REVIEW_OWNER, child).unwrap();
        ensure_run_dir(REVIEW_OWNER, grandchild).unwrap();
        ensure_run_dir(RUNNING_OWNER, running_child).unwrap();

        let to = advance_review_to_workflow(&store, parent).unwrap();
        let exec = store.get_execution_by_id(parent).unwrap();
        assert_eq!(to, root_run_date(&store, &exec));
        assert!(is_date_owner(&to));
        assert!(exec.workflow_moved);
        for id in [parent, child, grandchild] {
            assert_eq!(find_run_dir(id).unwrap().0, to);
            assert!(store.get_execution_by_id(id).unwrap().workflow_moved);
        }
        assert_eq!(find_run_dir(running_child).unwrap().0, RUNNING_OWNER);
        assert!(!store.get_execution_by_id(running_child).unwrap().workflow_moved);

        // 移動先が既にあり移動できないときは DB を元に戻して Err
        let other = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(other, "completed").unwrap();
        ensure_run_dir(REVIEW_OWNER, other).unwrap();
        std::fs::create_dir_all(run_dir(&to, other)).unwrap();
        let err = advance_review_to_workflow(&store, other).unwrap_err();
        assert!(!err.is_empty());
        assert!(
            !store.get_execution_by_id(other).unwrap().workflow_moved,
            "移動できなければ workflow_moved を元に戻す"
        );

        for id in [parent, child, grandchild, running_child, other] {
            remove_run_dir(id).unwrap();
        }
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("AGENT_LINK_PROJECT_ROOT");
    }

    /// 親が Workflow にいる間に確定した子は、review に取り残されず親と同じ日付フォルダへ移る。
    #[test]
    fn confirmed_child_follows_parent_in_workflow() {
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = set_tmp_project("sai_child_follow");
        let (store, block_id) = test_store();
        let parent = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(parent, "completed").unwrap();
        ensure_run_dir(REVIEW_OWNER, parent).unwrap();
        let child = store
            .create_execution_for_subtask(block_id, None, parent, None)
            .unwrap();
        let task_id = store
            .insert_task(&crate::db::models::Task::new(
                parent,
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
        store.update_task_execution_id(task_id, child).unwrap();
        store.update_task_status(task_id, "completed").unwrap();
        store.update_execution_status(child, "completed").unwrap();
        ensure_run_dir(RUNNING_OWNER, child).unwrap();

        // 子が未確定のまま親が Workflow へ → 子は running のまま
        let to = advance_review_to_workflow(&store, parent).unwrap();
        assert_eq!(find_run_dir(child).unwrap().0, RUNNING_OWNER);

        // 子を確定 → 親と同じ日付フォルダへ
        store.update_task_finalized(task_id, true).unwrap();
        store.update_task_status(task_id, "finalized").unwrap();
        sync_run_owner_by_execution_id(&store, child);
        assert_eq!(find_run_dir(child).unwrap().0, to);
        assert!(store.get_execution_by_id(child).unwrap().workflow_moved);

        remove_run_dir(parent).unwrap();
        remove_run_dir(child).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("AGENT_LINK_PROJECT_ROOT");
    }
}
