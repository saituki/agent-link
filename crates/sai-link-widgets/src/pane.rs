//! herdr pane 操作の純関数群（sai-link-tui `app/external.rs` から分離）。
//!
//! 各 TUI（sai-link-tui / sai-control-tui）から「外部コマンドを herdr ペインで
//! 実行する」ための部品。`AppState` に依存しない純関数として提供し、
//! 呼び出し側が戻り値の Receiver を自 state の `background_rx` に保存して
//! ステータスメッセージへ反映する。
//!
//! - [`spawn_in_pane`] — herdr 右 split → テキスト送信 → Enter（バックグラウンドスレッド）
//! - [`build_resume_command`] — `pi --session` 再開コマンドの組立
//! - [`reflection_path`] — Reflection 記録ファイルのパス組立（存在チェック・生成は呼び出し側）

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use sai_link_engine::db::SaiStore;
use sai_link_engine::herdr::{shell_escape_single_quotes, HerdrClient, SplitDirection};
use sai_link_engine::session::find_main_session_file;

/// herdr で右方向にペインを作成し、コマンドを送信して Enter で実行する。
///
/// herdr 操作はイベントループをブロックしないようバックグラウンドスレッドで
/// 実行し、結果（起動メッセージ / 成功・失敗メッセージ）はチャネル経由で返す。
/// 呼び出し側は戻り値の Receiver を自 state の `background_rx` に保存し、
/// ステータスバーに初期メッセージ（「...: herdr ペインで起動中...」等）を
/// 出してからイベントループに戻る。
///
/// - `label`: ステータスメッセージの接頭辞（例: `"Reflection"`）
/// - `command`: pane 上のシェルで実行するコマンド文字列
///
/// # 戻り値
///
/// 結果メッセージ（1 通）を受信するチャネルの Receiver。
pub fn spawn_in_pane(label: &str, command: &str) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel::<String>();
    let command = command.to_string();
    let label = label.to_string();

    std::thread::spawn(move || {
        let herdr = HerdrClient::new();
        match herdr.split_pane(SplitDirection::Right, None) {
            Ok(pane_info) => {
                // ペイン生成直後のシェル準備を待つ（sai-link-tui と同じ待機時間）
                std::thread::sleep(Duration::from_millis(500));
                if let Err(e) = herdr.send_text(&pane_info.pane_id, &command) {
                    let _ = tx.send(format!("{}: テキスト送信失敗: {}", label, e));
                    return;
                }
                std::thread::sleep(Duration::from_millis(300));
                // Enter で実行（pane 上のシェルでのコマンド実行）
                if let Err(e) = herdr.send_keys(&pane_info.pane_id, "Enter") {
                    let _ = tx.send(format!("{}: Enter 送信失敗: {}", label, e));
                    return;
                }
                let _ = tx.send(format!(
                    "{}: ペイン {} で実行しました",
                    label, pane_info.pane_id
                ));
            }
            Err(e) => {
                let _ = tx.send(format!("{}: herdr エラー: {}", label, e));
            }
        }
    });

    rx
}

/// `p`（Session 再開）用のコマンド文字列を組立てる（ビルダ純関数）。
///
/// sai-link-tui `app/external.rs::resume_session()` のコマンド組立部分を分離した
/// もの。最初の実行コマンド（`orchestrator::prepare_pipeline`）と同じ形になる
/// よう、`SAI_PROJECT_ROOT` / `SAI_PARENT_RUN_ID` / `SAI_BLOCK_NAME` /
/// `SAI_IS_SUBTASK` の環境変数と、ブロック定義ファイルから再パースした
/// `--extension` / `--tools` 等のエージェント引数を必ず前置する。
/// モデル・プロバイダ・thinking は実行時に上書きされた値（`exec.*`）を優先し、
/// 無ければブロック定義のデフォルトを使う。
/// `--provider` が無いと pi は model 接頭辞（例 `z-ai/glm-5.3-flash` → `z-ai`）を
/// provider と誤解釈して `Model not found` で起動失敗するため必ず付与する。
///
/// # エラー
///
/// - 実行レコードが見つからない / `session_dir` 未記録 / ファイル解決失敗の
///   場合は `Err` にユーザー向けメッセージを返す（そのままステータスバーへ表示）。
pub fn build_resume_command(store: &SaiStore, exec_id: i64) -> Result<String, String> {
    // 実行レコードを取得（runner 分岐のため最初に解決する）
    let exec = store
        .get_execution_by_id(exec_id)
        .map_err(|_| "Session: 実行レコードが見つかりません".to_string())?;

    // ブロック定義（SAI_BLOCK_NAME / --extension 等の再構築に使う）
    let block = store.get_block_by_id(exec.block_id).ok();
    let block_config = block.as_ref().and_then(|b| {
        let path = Path::new(&b.file_path);
        let resolved = if path.is_relative() {
            sai_link_engine::project::project_root().join(path)
        } else {
            path.to_path_buf()
        };
        sai_link_engine::parser::parse_block_file(&resolved)
            .map(|parsed| parsed.config)
            .ok()
    });

    // エージェント用の環境変数（最初の実行コマンドと同じ前置）
    let project_root = sai_link_engine::project::project_root().to_string_lossy().to_string();
    let env_prefix = format!(
        "SAI_PROJECT_ROOT={} SAI_PARENT_RUN_ID={} SAI_BLOCK_NAME={} ",
        shell_escape_single_quotes(&project_root),
        exec_id,
        block
            .as_ref()
            .map(|b| shell_escape_single_quotes(&b.name))
            .unwrap_or_else(|| "''".to_string()),
    );

    // 再開する場所: 子タスクは起動したときと同じフォルダ（作業用コピー）。
    // 作業用コピーが既に消えている子・メインブロックはプロジェクトルート。
    let resume_cwd = store
        .get_task_by_execution_id(exec_id)
        .ok()
        .flatten()
        .and_then(|task| {
            let block_worktree = block_config.as_ref().and_then(|c| c.worktree.as_deref());
            sai_link_engine::orchestrator::subtask_run_cwd(&task, block_worktree, exec_id)
        })
        .unwrap_or_else(sai_link_engine::project::project_root);
    let resume_cwd_str = resume_cwd.to_string_lossy().to_string();

    // cc: 起動時と同じ組み立てで `--session-id` を `--resume <uuid>` に替えて再開する
    // （道具・MCP は .claude/settings.json と .mcp.json が効く。セッション実体は ~/.claude 側）
    if exec.runner == "cc" {
        let session_id = exec
            .agent_session_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or("Session: エージェントセッションIDが記録されていません")?;
        let mut builder = sai_link_engine::cc_command::CcCommandBuilder::new(session_id)
            .with_model_thinking(exec.model.clone(), exec.thinking.clone())
            .resume();
        // 起動時と同じく、作業用コピーに設定が無ければプロジェクトルートの設定を渡す
        if let Ok(dir) = sai_link_engine::cc_command::check_project_files(&resume_cwd) {
            if dir != resume_cwd {
                builder = builder.with_config_dir(dir);
            }
        }
        let cc = builder
            .build()
            .map_err(|e| format!("Session: cc コマンド構築失敗: {}", e))?;
        return Ok(format!(
            "cd {} && {}{}",
            shell_escape_single_quotes(&resume_cwd_str),
            env_prefix,
            cc.command
        ));
    }

    // セッションディレクトリの解決(DB の旧パスが残っていても outputs 側へフォールバック)
    let session_dir = match sai_link_engine::session::resolve_session_dir(
        exec.session_dir.as_deref(),
        exec_id,
    ) {
        Some(dir) => dir.to_string_lossy().to_string(),
        None => return Err("Session: session_dir が記録されていません".to_string()),
    };

    // セッションディレクトリ内のメイン .jsonl ファイルを解決
    let session_file = match find_main_session_file(Path::new(&session_dir)) {
        Ok(file) => file.to_string_lossy().to_string(),
        Err(e) => return Err(format!("Session: ファイル解決失敗: {}", e)),
    };

    // ブロック定義から --extension / --tools 等を再構築し、モデル・プロバイダ・
    // thinking は実行時の上書き値があればそちらを優先する（最初の実行コマンドと同一化）。
    let mut effective_config = block_config;
    if let Some(config) = effective_config.as_mut() {
        if exec.provider.is_some() {
            config.provider = exec.provider.clone();
        }
        if exec.model.is_some() {
            config.model = exec.model.clone();
        }
        if exec.thinking.is_some() {
            config.thinking = exec.thinking.clone();
        }
    }

    // 再開用コマンド: pi --session <path/filename.jsonl> [options]
    // U02: pane 内では必ず起動時と同じ場所（メインはプロジェクトルート、子は
    // 作業用コピー）へ `cd` してから起動する。pane が別フォルダで開かれていても、
    // 同じ AGENTS.md・同じ場所で再開できる。
    let mut resume_cmd = format!(
        "cd {} && {}pi --session {}",
        shell_escape_single_quotes(&resume_cwd_str),
        env_prefix,
        shell_escape_single_quotes(&session_file)
    );
    // 作業用コピーに `.mcp.json` が無ければ、プロジェクトルートの設定を渡す（起動時と同じ）
    if let Some(mcp) = sai_link_engine::cc_command::resolve_sai_mcp_config(&resume_cwd) {
        resume_cmd.push_str(" --mcp-config ");
        resume_cmd.push_str(&shell_escape_single_quotes(&mcp.to_string_lossy()));
    }
    if let Some(config) = effective_config.as_ref() {
        for arg in sai_link_engine::pi_command::agent_args_for_config(config) {
            resume_cmd.push(' ');
            if arg.starts_with("--") {
                resume_cmd.push_str(&arg);
            } else {
                resume_cmd.push_str(&shell_escape_single_quotes(&arg));
            }
        }
    } else {
        // ブロック定義が再パースできない場合のフォールバック（従来どおり最小限）
        if let Some(provider) = exec.provider.clone() {
            resume_cmd.push_str(&format!(" --provider {}", shell_escape_single_quotes(&provider)));
        }
        if let Some(model) = exec.model.clone() {
            resume_cmd.push_str(&format!(" --model {}", shell_escape_single_quotes(&model)));
        }
        if let Some(thinking) = exec.thinking.clone() {
            resume_cmd.push_str(&format!(" --thinking {}", shell_escape_single_quotes(&thinking)));
        }
    }
    Ok(resume_cmd)
}

/// Reflection 記録ファイルのパスを返す（`<root>/.sai/reflection/<exec_id>.md`）。
///
/// **存在チェック・テンプレート生成は行わない**（sai-link 仕様 D2: 生成は
/// sai-link 側の役割。sai-control 側は存在する場合のみ開く）。
pub fn reflection_path(root: &Path, exec_id: i64) -> PathBuf {
    root.join(".sai")
        .join("reflection")
        .join(format!("{}.md", exec_id))
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// `pi --session` 再開コマンドの組立（provider / model / thinking 引き継ぎ）を検証する。
    ///
    /// 実際の herdr 操作は行わない（コマンド文字列の生成ロジックのみ確認）。
    #[test]
    fn build_resume_command_with_provider_model_and_thinking() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = sai_link_engine::db::models::BlockMetadata::new(
            "plan-ice".to_string(),
            "テスト用ブロック".to_string(),
            "anthropic/claude-sonnet-4-5".to_string(),
            None,
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            "blocks/plan-ice.md".to_string(),
            None
        );
        let block_id = store.insert_block(&block).unwrap();
        let exec_id = store.create_execution(block_id, None).unwrap();

        // セッションディレクトリ（メイン jsonl 1 件）を用意する
        let dir = std::env::temp_dir().join(format!("sai_pane_resume_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let jsonl = dir.join("2026-08-29T10-00-00-main.jsonl");
        std::fs::write(&jsonl, "{}\n").unwrap();
        store
            .update_execution_session_info(exec_id, &dir.to_string_lossy(), "pi")
            .unwrap();

        // モデル / thinking / provider を実行時に上書きする
        store
            .update_execution_overrides(exec_id, None, None, None, Some("m1"), Some("high"), None)
            .unwrap();
        store
            .update_execution_provider(exec_id, Some("open-router2"))
            .unwrap();

        let cmd = build_resume_command(&store, exec_id).unwrap();
        // U02: pane 内ではプロジェクトルートへ cd してから再開する
        assert!(cmd.starts_with("cd "), "プロジェクトルートへ cd してから起動: {cmd}");
        assert!(
            cmd.contains(&format!("pi --session '{}'", jsonl.to_string_lossy())),
            "メイン jsonl が使われる: {cmd}"
        );
        assert!(
            cmd.contains("SAI_PARENT_RUN_ID="),
            "元の実行コマンドと同じ SAI_PARENT_RUN_ID が前置される: {cmd}"
        );
        assert!(
            cmd.contains("SAI_BLOCK_NAME='plan-ice'"),
            "元の実行コマンドと同じ SAI_BLOCK_NAME が前置される: {cmd}"
        );
        assert!(
            cmd.contains("--provider 'open-router2'"),
            "provider が引き継がれる（model 接頭辞誤解釈を防ぐ）: {cmd}"
        );
        assert!(cmd.contains("--model 'm1'"), "モデルが引き継がれる: {cmd}");
        assert!(
            cmd.contains("--thinking 'high'"),
            "thinking が引き継がれる: {cmd}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `runner=cc` は `cc --resume <uuid>` を返す（agent_session_id 未記録はエラー）。
    #[test]
    fn build_resume_command_cc() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = sai_link_engine::db::models::BlockMetadata::new(
            "plan-cc".to_string(),
            "テスト用ブロック".to_string(),
            "claude-sonnet-4-5".to_string(),
            None,
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            "blocks/plan-cc.md".to_string(),
            None
        );
        let block_id = store.insert_block(&block).unwrap();
        let exec_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_runner(exec_id, "cc").unwrap();
        store
            .update_execution_overrides(
                exec_id,
                None,
                None,
                None,
                Some("anthropic/claude-sonnet-4-5"),
                None,
                None
            )
            .unwrap();

        // agent_session_id 未記録 → エラー
        let err = build_resume_command(&store, exec_id).unwrap_err();
        assert!(err.contains("エージェントセッションID"), "{err}");

        store
            .update_execution_agent_session_id(exec_id, "11111111-2222-4333-8444-555555555555")
            .unwrap();
        let cmd = build_resume_command(&store, exec_id).unwrap();
        // U02: pane 内ではプロジェクトルートへ cd してから再開する
        assert!(cmd.starts_with("cd "), "プロジェクトルートへ cd してから起動: {cmd}");
        assert!(
            cmd.contains("cc --resume '11111111-2222-4333-8444-555555555555'"),
            "{cmd}"
        );
        assert!(
            cmd.contains("SAI_BLOCK_NAME='plan-cc'"),
            "元の実行コマンドと同じ SAI_BLOCK_NAME が前置される: {cmd}"
        );
        assert!(
            cmd.contains("SAI_PARENT_RUN_ID="),
            "元の実行コマンドと同じ SAI_PARENT_RUN_ID が前置される: {cmd}"
        );
        // モデルは anthropic/ 接頭辞が除かれる
        assert!(cmd.contains("--model 'claude-sonnet-4-5'"), "{cmd}");
    }

    /// U01: ブロック定義ファイルを再パースし、ブロックのファイルパスを復元すること
    /// （--extension は付けない・MCP 使用）。
    #[test]
    fn build_resume_command_reads_block_file() {
        let store = SaiStore::open_in_memory().unwrap();

        let dir = std::env::temp_dir().join(format!("sai_pane_resume_block_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ext_path = dir.join("extension.ts");
        std::fs::write(&ext_path, "// dummy extension").unwrap();
        let block_path = dir.join("plan-ext.md");
        std::fs::write(
            &block_path,
            format!(
                "---\nname: plan-ext\ndescription: テスト用ブロック\nmodel: anthropic/claude-sonnet-4-5\nextensions:\n  - {}\n---\n# 本文",
                ext_path.to_string_lossy()
            ),
        )
        .unwrap();

        let block = sai_link_engine::db::models::BlockMetadata::new(
            "plan-ext".to_string(),
            "テスト用ブロック".to_string(),
            "anthropic/claude-sonnet-4-5".to_string(),
            None,
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            block_path.to_string_lossy().to_string(),
            None
        );
        let block_id = store.insert_block(&block).unwrap();
        let exec_id = store.create_execution(block_id, None).unwrap();

        let session_dir = dir.join("session");
        std::fs::create_dir_all(&session_dir).unwrap();
        let jsonl = session_dir.join("2026-08-29T10-00-00-main.jsonl");
        std::fs::write(&jsonl, "{}\n").unwrap();
        store
            .update_execution_session_info(exec_id, &session_dir.to_string_lossy(), "pi")
            .unwrap();

        let cmd = build_resume_command(&store, exec_id).unwrap();
        // ブロック定義ファイルの model が復元される
        assert!(cmd.contains("--model 'anthropic/claude-sonnet-4-5'"), "{cmd}");
        // U01: --extension は付けない（pi は MCP で道具を使う）
        assert!(!cmd.contains("--extension"), "{cmd}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `session_dir` 未記録・実行レコード無しのエラーメッセージ（sai-link と同一文言）。
    #[test]
    fn build_resume_command_error_messages() {
        let store = SaiStore::open_in_memory().unwrap();
        let block = sai_link_engine::db::models::BlockMetadata::new(
            "b".to_string(),
            "d".to_string(),
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
        let exec_id = store.create_execution(block_id, None).unwrap();

        // session_dir 未記録
        let err = build_resume_command(&store, exec_id).unwrap_err();
        assert_eq!(err, "Session: session_dir が記録されていません");

        // 実行レコード無し
        let err = build_resume_command(&store, 999).unwrap_err();
        assert_eq!(err, "Session: 実行レコードが見つかりません");
    }

    /// `reflection_path` はパス組立のみで、ファイルを生成しないこと（D2）。
    #[test]
    fn reflection_path_builds_only() {
        let root = std::env::temp_dir().join(format!("sai_pane_refl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let path = reflection_path(&root, 42);
        assert_eq!(path, root.join(".sai/reflection/42.md"), "パスが正しいこと");
        assert!(!path.exists(), "ファイルを生成しないこと（D2）");
        assert!(!root.exists(), "ディレクトリも作らないこと");

        let _ = std::fs::remove_dir_all(&root);
    }
}
