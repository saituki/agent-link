//! # `sai-link migrate-outputs` — 旧 `.sai/` 構成の outputs 移行（R7・一度だけ）
//!
//! 旧構成（`.sai/todo/*.md`・`.sai/message/<id>.md`・
//! `.sai/sessions/pi-sessions-*/`・`.sai/reflection/`）を
//! `outputs/<owner>/<run_id>/` へ移行する。
//!
//! - `todo/*.md`：`run_id` 確定済み→対応 run の `task.md` へ、
//!   未確定（`-`）→予約行を発行して `todo/<run_id>/task.md` へ
//! - `message/<id>.md`→対応 run の `message.md`（対応 run なしはスキップ）
//! - `sessions/pi-sessions-*/`→対応 run の `session/` へ移動（正本移転）
//! - `reflection/` は削除する（使用しない・R209・A9 で他PJも同様）
//! - 欠損ファイルは作らない。2回目は no-op（冪等）。
//!
//! ## 他プロジェクトへの適用（A7）
//!
//! `MigrateArgs.root` で対象プロジェクトルートを指定する。`SAI_PROJECT_ROOT`
//! 環境変数の一時上書きで実現し、呼び出し後に元に戻す（プロセス全体への影響なし）。
//!
//! ## デーモン停止の強制（A8）
//!
//! 移行前に `daemon::is_running()` で稼働確認し、稼働中は `SIGTERM` 停止→
//! 移行→再起動を自動で行う。`--no-daemon-restart` 指定時は再起動しない。
//! 実行中タスク（`running_executions` 非空）がある場合も `--yes` があれば
//! 強制停止する（なければエラーで中断）。

use std::path::PathBuf;

/// 移行結果レポート。
#[derive(Debug, Default)]
pub struct MigrateReport {
    /// 移動したカード数
    pub cards: usize,
    /// 発行した予約行数
    pub reserved: usize,
    /// 移動した message 数
    pub messages: usize,
    /// 移動した session ディレクトリ数
    pub sessions: usize,
    /// 削除した reflection ファイル数
    pub reflections_removed: usize,
    /// スキップ数（対応 run なし等）
    pub skipped: usize,
}

/// 移行対象・デーモン制御の引数（A7/A8）。
#[derive(Debug, Clone, Default)]
pub struct MigrateArgs {
    /// 対象プロジェクトルート（`None` = 現プロジェクト）。
    ///
    /// `SAI_PROJECT_ROOT` の一時上書きで実現する。相対パスは現 cwd 基準で解決する。
    pub root: Option<std::path::PathBuf>,
    /// 確認プロンプトをスキップする（デーモン停止・実行中タスクありの強制停止を含む）。
    pub yes: bool,
    /// 移行後のデーモン再起動を行わない（停止したままにする）。
    pub no_daemon_restart: bool,
}

/// 移行を実行する。
///
/// 互換用：現プロジェクト（`SAI_PROJECT_ROOT`/cwd 基準）を移行する。
/// 他プロジェクト指定・デーモン制御が必要な場合は `run_migrate_outputs_with` を使うこと。
pub fn run_migrate_outputs() -> Result<MigrateReport, Box<dyn std::error::Error>> {
    run_migrate_outputs_with(&MigrateArgs::default())
}

/// 移行を実行する（対象ルート・デーモン制御付き・A7/A8）。
///
/// 手順：
/// 1. 対象ルートを解決（`--root` があれば絶対化・存在確認）
/// 2. デーモン稼働確認（対象ルートの pid ファイル）。稼働中は停止を強制する（A8）。
///    実行中タスクありでも `--yes` があれば停止する。なければエラーで中断。
/// 3. `SAI_PROJECT_ROOT` を対象ルートに一時上書きして移行本体を実行
/// 4. 停止したデーモンがあれば再起動（`--no-daemon-restart` が無い場合）
/// 5. `SAI_PROJECT_ROOT` を元に戻す
pub fn run_migrate_outputs_with(args: &MigrateArgs) -> Result<MigrateReport, Box<dyn std::error::Error>> {
    // 1. 対象ルート解決
    let root = match args.root.as_deref() {
        Some(p) => {
            let abs = if p.is_absolute() {
                p.to_path_buf()
            } else {
                std::env::current_dir()?.join(p)
            };
            if !abs.is_dir() {
                return Err(format!("移行対象が存在しません: {}", abs.display()).into());
            }
            if !abs.join(".sai").is_dir() {
                return Err(format!(".sai/ がありません（sai プロジェクトではありません）: {}", abs.display()).into());
            }
            abs
        }
        None => sai_link_engine::project::project_root(),
    };
    // 2. デーモン稼働確認（対象ルート基準・A8）
    let was_running = daemon_running_at(&root);
    let mut stopped_by_us = false;
    if was_running {
        let running_execs = daemon_running_executions(&root);
        if !running_execs.is_empty() && !args.yes {
            return Err(format!(
                "デーモンが実行中タスク {:?} を抱えています。`--yes` 付きで強制停止するか、先に実行完了を待ってください: {}",
                running_execs,
                root.display()
            )
            .into());
        }
        if !args.yes {
            eprintln!(
                "デーモンが稼働中です（{}）。停止して移行しますか？ [y/N]",
                root.display()
            );
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            if line.trim().to_lowercase() != "y" {
                eprintln!("移行を中止しました");
                return Ok(MigrateReport::default());
            }
        }
        stop_daemon_at(&root)?;
        stopped_by_us = true;
        eprintln!("🛑 デーモンを停止しました（{}）。移行を開始します。", root.display());
    }
    // 3. SAI_PROJECT_ROOT の一時上書きで移行本体を実行
    let prev_root = std::env::var_os("SAI_PROJECT_ROOT");
    // SAFETY: 単一スレッド CLI の一時的切替。並行テストとは併用しない。
    unsafe { std::env::set_var("SAI_PROJECT_ROOT", &root) };
    let result = run_migrate_body();
    // 5. 環境変数を元に戻す（再起動より先に復元し、自プロジェクト基準に戻す）
    unsafe {
        match prev_root {
            Some(v) => std::env::set_var("SAI_PROJECT_ROOT", v),
            None => std::env::remove_var("SAI_PROJECT_ROOT"),
        }
    };
    let report = result?;
    // 4. デーモン再起動（停止したのが自分たちの場合のみ）
    if stopped_by_us && !args.no_daemon_restart {
        start_daemon_at(&root)?;
        eprintln!("✅ デーモンを再起動しました（{}）", root.display());
    } else if stopped_by_us {
        eprintln!("⚠ デーモンは停止したままです（--no-daemon-restart指定）: {}", root.display());
    }
    Ok(report)
}

/// 移行本体（`SAI_PROJECT_ROOT` 基準・従来の `run_migrate_outputs` と同等）。
fn run_migrate_body() -> Result<MigrateReport, Box<dyn std::error::Error>> {
    let mut report = MigrateReport::default();
    let root = sai_link_engine::project::project_root();
    let db_path = root.join(".sai/sai-engine/sai-engine.db");
    let store = sai_link_engine::db::SaiStore::open(&db_path)?;

    migrate_cards(&store, &root, &mut report)?;
    migrate_simple_files(
        &root,
        "message",
        sai_link_engine::run_dir::MESSAGE_FILE,
        &mut report,
        |r| &mut r.messages,
    )?;
    migrate_sessions(&store, &root, &mut report)?;
    remove_reflections(&root, &mut report)?;

    Ok(report)
}

/// 旧カード（`.sai/todo/*.md`）を `task.md` へ移行する。
fn migrate_cards(
    store: &sai_link_engine::db::SaiStore,
    root: &std::path::Path,
    report: &mut MigrateReport,
) -> Result<(), Box<dyn std::error::Error>> {
    let todo_dir = root.join(".sai/todo");
    let Ok(entries) = std::fs::read_dir(&todo_dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            report.skipped += 1;
            continue;
        };
        let Some(card) = sai_link_engine::kanban::parse_card(&text) else {
            report.skipped += 1;
            continue;
        };
        // 移行済みマーカー：既に outputs 側に同内容の task.md があればスキップ
        if let Ok(rid) = card.frontmatter.run_id.trim().parse::<i64>() {
            if sai_link_engine::run_dir::find_run_dir(rid).is_some() {
                // 既に run フォルダあり→ task.md が無ければ補完、あればスキップ
                let dir = sai_link_engine::run_dir::ensure_run_dir(
                    sai_link_engine::run_dir::REVIEW_OWNER,
                    rid,
                )?;
                let task = dir.join(sai_link_engine::run_dir::TASK_FILE);
                if !task.exists() {
                    std::fs::write(&task, sai_link_engine::kanban::render_card(&card))?;
                    report.cards += 1;
                }
                let _ = std::fs::remove_file(&path);
                continue;
            }
            // 対応 run が DB にあるか確認
            if store.get_execution_by_id(rid).is_ok() {
                let owner = owner_for_execution(store, rid);
                let dir = sai_link_engine::run_dir::ensure_run_dir(&owner, rid)?;
                let task = dir.join(sai_link_engine::run_dir::TASK_FILE);
                if !task.exists() {
                    std::fs::write(&task, sai_link_engine::kanban::render_card(&card))?;
                    report.cards += 1;
                }
                let _ = std::fs::remove_file(&path);
            } else {
                report.skipped += 1;
            }
        } else {
            // 未確定カード：予約行を発行して todo/<run_id>/task.md へ
            let inputs = serde_json::json!({
                "description": card.frontmatter.description,
            });
            let inputs_str = serde_json::to_string(&inputs)?;
            let rid = store.create_reserved_run(Some(&inputs_str))?;
            report.reserved += 1;
            let mut card = card;
            card.frontmatter.run_id = rid.to_string();
            let dir = sai_link_engine::run_dir::ensure_run_dir(
                sai_link_engine::run_dir::TODO_OWNER,
                rid,
            )?;
            std::fs::write(
                dir.join(sai_link_engine::run_dir::TASK_FILE),
                sai_link_engine::kanban::render_card(&card),
            )?;
            report.cards += 1;
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}

/// 実行に対応する owner を DB 状態から決める（移行時の配置用）。
fn owner_for_execution(store: &sai_link_engine::db::SaiStore, run_id: i64) -> String {
    let Ok(exec) = store.get_execution_by_id(run_id) else {
        return sai_link_engine::run_dir::REVIEW_OWNER.to_string();
    };
    // review → done の区別は DB の review_done（利用者が確認済みにしたか）で決める
    sai_link_engine::run_dir::owner_for(
        &exec.status,
        exec.review_done,
        exec.workflow_moved,
        exec.global_round,
    )
}

/// 旧 `.sai/<dir>/<id>.md` を対応 run フォルダへ移行する（message 用）。
fn migrate_simple_files(
    root: &std::path::Path,
    dir_name: &str,
    file_name: &str,
    report: &mut MigrateReport,
    counter: fn(&mut MigrateReport) -> &mut usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = root.join(".sai").join(dir_name);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(());
    };
    let db_path = root.join(".sai/sai-engine/sai-engine.db");
    let store = sai_link_engine::db::SaiStore::open(&db_path)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            report.skipped += 1;
            continue;
        };
        let Ok(run_id) = stem.parse::<i64>() else {
            report.skipped += 1;
            continue;
        };
        // 対応 run なし→スキップ（作らない・R702）
        if store.get_execution_by_id(run_id).is_err() {
            report.skipped += 1;
            continue;
        }
        // 移行先：既存 run フォルダがあればそこ、無ければ状態から決める
        let dest_dir = match sai_link_engine::run_dir::find_run_dir(run_id) {
            Some((_, d)) => d,
            None => {
                let owner = owner_for_execution(&store, run_id);
                sai_link_engine::run_dir::ensure_run_dir(&owner, run_id)?
            }
        };
        let dest = dest_dir.join(file_name);
        if dest.exists() {
            // 冪等：既にあれば旧ファイルを消すだけ
            let _ = std::fs::remove_file(&path);
            continue;
        }
        std::fs::rename(&path, &dest)?;
        *counter(report) += 1;
    }
    Ok(())
}

/// 旧 `.sai/sessions/pi-sessions-*/` を対応 run の `session/` へ移動する。
///
/// 対応付けは DB の `session_dir` を逆引きする。DB に記録のない session はスキップ。
fn migrate_sessions(
    store: &sai_link_engine::db::SaiStore,
    root: &std::path::Path,
    report: &mut MigrateReport,
) -> Result<(), Box<dyn std::error::Error>> {
    let sessions_root = root.join(".sai/sessions");
    let Ok(entries) = std::fs::read_dir(&sessions_root) else {
        return Ok(());
    };
    // DB の session_dir → run_id マップを構築
    let mut session_to_run: std::collections::HashMap<PathBuf, i64> =
        std::collections::HashMap::new();
    if let Ok(execs) = store.list_all_executions() {
        for e in execs {
            if let (Some(id), Some(dir)) = (e.id, e.session_dir.as_deref()) {
                session_to_run.insert(PathBuf::from(dir), id);
                // 相対・絶対の揺れに備え basename でも引けるようにする
                if let Some(name) = PathBuf::from(dir).file_name() {
                    session_to_run.insert(sessions_root.join(name), id);
                }
            }
        }
    }
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(run_id) = session_to_run.get(&path).copied().or_else(|| {
            path.file_name()
                .and_then(|n| n.to_str())
                .and_then(|_| session_to_run.get(&path).copied())
        }) else {
            report.skipped += 1;
            continue;
        };
        let dest_root = match sai_link_engine::run_dir::find_run_dir(run_id) {
            Some((_, d)) => d.join(sai_link_engine::run_dir::SESSION_SUBDIR),
            None => {
                let owner = owner_for_execution(store, run_id);
                sai_link_engine::run_dir::ensure_run_dir(&owner, run_id)?
                    .join(sai_link_engine::run_dir::SESSION_SUBDIR)
            }
        };
        std::fs::create_dir_all(&dest_root)?;
        // 中の jsonl を移動する（ディレクトリごと移動すると session_dir 記録とずれるため中身のみ）
        let mut moved_any = false;
        if let Ok(files) = std::fs::read_dir(&path) {
            for f in files.flatten() {
                let src = f.path();
                let dest = dest_root.join(f.file_name());
                if dest.exists() {
                    continue;
                }
                if std::fs::rename(&src, &dest).is_ok() {
                    moved_any = true;
                }
            }
        }
        if moved_any {
            report.sessions += 1;
            // DB の session_dir を移行先へ書き換える（旧パス参照の残骸を防ぐ）
            let new_dir = dest_root.to_string_lossy().to_string();
            if let Ok(exec) = store.get_execution_by_id(run_id) {
                let _ = store.update_execution_session_info(
                    run_id,
                    &new_dir,
                    exec.pi_command.as_deref().unwrap_or(""),
                );
            }
        }
        // 空になった旧ディレクトリは掃除する
        if std::fs::read_dir(&path).map(|mut d| d.next().is_none()).unwrap_or(false) {
            let _ = std::fs::remove_dir(&path);
        }
    }
    Ok(())
}

/// 旧 `.sai/reflection/` を削除する（使用しない・R209/R702・A9）。
///
/// 他プロジェクト（sai-paper / sai-control / sai-test / sai-web）でも同様に削除する。
/// 空になったディレクトリ自体も削除する（参照互換コードは `create_dir_all` で再作成するため）。
fn remove_reflections(
    root: &std::path::Path,
    report: &mut MigrateReport,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = root.join(".sai/reflection");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        std::fs::remove_file(&path)?;
        report.reflections_removed += 1;
    }
    // 空になったディレクトリ自体も削除する（A9・他PJの残骸掃除）。
    // 参照互換コードは `create_dir_all` で再作成するため問題ない。
    let _ = std::fs::remove_dir(&dir);
    Ok(())
}

// ============================================================================
// デーモン制御（対象ルート基準・A8）
// ============================================================================

/// 対象ルートの pid ファイルパスを返す。
fn pid_file_at(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".sai/sai-engine/daemon.pid")
}

/// 対象ルートの status ファイルパスを返す。
fn status_file_at(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".sai/sai-engine/daemon.status")
}

/// 対象ルートのデーモンが稼働中か（pid ファイル＋`kill -0`＋cmdline 確認）。
///
/// `daemon::is_running()` は `SAI_PROJECT_ROOT` 基準のため、他PJ判定には
/// こちらを使う（自PJの一時切替なしで判定できる）。
fn daemon_running_at(root: &std::path::Path) -> bool {
    let pid: Option<i32> = std::fs::read_to_string(pid_file_at(root))
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let Some(pid) = pid else { return false };
    if !sai_link_engine::daemon::process_alive(pid) {
        return false;
    }
    // cmdline に `daemon` が含まれるか（Linux のみ・macOS は kill -0 にフォールバック）
    let cmdline_path = format!("/proc/{}/cmdline", pid);
    if let Ok(content) = std::fs::read(&cmdline_path) {
        let cmdline = String::from_utf8_lossy(&content).replace('\0', " ");
        return cmdline.contains("daemon");
    }
    true
}

/// 対象ルートのデーモンが抱える実行中タスク一覧（`daemon.status` 読み）。
///
/// 読み失敗時は空（停止判断を止めない）。
fn daemon_running_executions(root: &std::path::Path) -> Vec<i64> {
    std::fs::read_to_string(status_file_at(root))
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .and_then(|v| v.get("running_executions").cloned())
        .and_then(|a| a.as_array().cloned())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_i64())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// 対象ルートのデーモンを停止する（`SIGTERM`＋pid ファイル削除・A8 強制）。
fn stop_daemon_at(root: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let pid: Option<i32> = std::fs::read_to_string(pid_file_at(root))
        .ok()
        .and_then(|s| s.trim().parse().ok());
    if let Some(pid) = pid {
        if sai_link_engine::daemon::process_alive(pid) {
            std::process::Command::new("kill")
                .arg(pid.to_string())
                .status()?;
            // 終了待ち（最大 5 秒ポーリング）
            for _ in 0..50 {
                if !sai_link_engine::daemon::process_alive(pid) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    let pid_file = pid_file_at(root);
    if pid_file.exists() {
        std::fs::remove_file(&pid_file)?;
    }
    Ok(())
}

/// 対象ルートのデーモンを起動する（`sai-link daemon _run` を spawn）。
///
/// 現行実行ファイル（`std::env::current_exe`）を使うため、ビルド済みバイナリと
/// 同一バージョンのデーモンが起動する（旧バイナリ問題の解消）。
fn start_daemon_at(root: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::process::Stdio;
    let exe = std::env::current_exe()?;
    let log_path = root.join(".sai/sai-engine/daemon.log");
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let child = std::process::Command::new(&exe)
        .arg("daemon")
        .arg("_run")
        .current_dir(root)
        .env("SAI_PROJECT_ROOT", root)
        // P1-5: pi pane内からの起動時に親の実行系変数を引き継がない（cli.rs と同順）。
        .env_remove("SAI_PARENT_RUN_ID")
        .env_remove("SAI_BLOCK_NAME")
        .env_remove("SAI_MAKASERU")
        .env_remove("SAI_IS_SUBTASK")
        .stdin(Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file)
        .spawn()?;
    let pid = child.id() as i32;
    std::fs::write(pid_file_at(root), format!("{}\n", pid))?;
    Ok(())
}
