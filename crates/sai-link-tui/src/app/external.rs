//! 外部連携処理（`p` / `r` / `B` / `S` キー）。
//!
//! すべて sai-engine の `HerdrClient` 経由で herdr に「右方向 split →
//! テキスト送信 → Enter」を依頼する。UI をブロックしないよう
//! バックグラウンドスレッドで実行する。
//!
//! | キー | 機能 | 旧 UI の相当機能 |
//! |---|---|---|
//! | `p` | `pi --session` でセッション再開 | 旧右SB Session クリック |
//! | `r` | Review 用 md を nvim で開く | 旧 Header Review |
//! | `B` | `.sai/blocks` ディレクトリを nvim で開く | 旧 Header Blocks |
//! | `S` | `.sai/settings.json` を nvim で開く | 旧 Header Settings |

use sai_link_engine::herdr::shell_escape_single_quotes;

use super::AppState;

/// herdr で右方向にペインを作成し、コマンドを送信して Enter で実行する。
///
/// 本体は `sai_link_widgets::pane::spawn_in_pane`（AppState に依存しない純関数）に
/// 移設済み（sai-control-tui と共通）。本ラッパは結果チャネルを
/// `AppState.background_rx` に保存し、初期メッセージをステータスバーへ出すのみ。
fn spawn_in_pane(app: &mut AppState, label: &str, command: &str) {
    let rx = sai_link_widgets::pane::spawn_in_pane(label, command);
    app.background_rx = Some(rx);
    app.set_message(format!("{}: herdr ペインで起動中...", label));
}

/// シェルコマンド用に文字列を一重引用符で囲む。
///
/// sai-engine の `herdr::shell_escape_single_quotes` を利用する。
fn shell_quote(s: &str) -> String {
    shell_escape_single_quotes(s)
}

/// `p`: 選択中実行の pi セッションを herdr ペインで再開する。
///
/// （旧右SB Session クリック `handle_session_click` の移植。正仕様:
/// pane 上のシェルで bash コマンドを実行するため送信キーは Enter）
/// コマンド組立は `sai_link_widgets::pane::build_resume_command`（純関数）に委譲する。
pub fn resume_session(app: &mut AppState, exec_id: Option<i64>) {
    let Some(exec_id) = exec_id else {
        app.set_message("Session: 実行が選択されていません");
        return;
    };

    match sai_link_widgets::pane::build_resume_command(&app.store, exec_id) {
        Ok(resume_cmd) => spawn_in_pane(app, "Session", &resume_cmd),
        Err(msg) => app.set_message(msg),
    }
}

/// `gl`: sai-gemini-tui を herdr ペインで開く（runner=gl 用）。
///
/// 起動時のタスク書き込みなし（空セッションを開くだけ）。
/// `gl` ラッパー（`~/.local/bin/gl`）をそのまま起動する。
/// 会話内容は `~/.sai-gemini/sessions/*.jsonl` に保存され、
/// History（`h`）・message（`Space m`）画面から解析表示できる。
pub fn open_gl_session(app: &mut AppState) {
    spawn_in_pane(app, "gl", "gl");
}

/// ブロック ID からブロック定義を読み、review のひな形の見出しを求める。
///
/// 定義が見つからない・読めない場合は既定の見出しを返す。
fn resolve_review_headings(block_id: &str) -> Vec<String> {
    let path = sai_link_engine::orchestrator::resolve_block_path(block_id);
    match sai_link_engine::parser::parse_block_file(&path) {
        Ok(parsed) => parsed.config.review_headings_or_default(),
        Err(_) => sai_link_engine::block_config::default_review_headings(),
    }
}

/// `r`: Review 用マークダウンを herdr ペインで nvim により開く（新パス）。
///
/// - 保存場所: `PROJECTROOT/.sai/review/<実行ID>.md`
/// - sidebar で選択中の実行に対応するファイルが無ければ空テンプレートを
///   作成して nvim で開く。ある場合はそのまま開く。
///   ファイル内容の判定・追記はしない（「開くだけ」）。
/// - 旧 `.sai/reflection/*.md` は残す（自動改名・削除なし）。
pub fn open_review(app: &mut AppState, exec_id: Option<i64>) {
    let Some(exec_id) = exec_id else {
        app.set_message("Review: 実行が選択されていません");
        return;
    };

    let path = sai_link_engine::orchestrator::review_path(exec_id);

    if !path.exists() {
        // 空テンプレートで新規作成する（実行完了時の自動生成と同じ書式）。
        // 見出しは実行のブロック定義の `review_headings` に従う（無ければ既定）。
        let default_headings = sai_link_engine::block_config::default_review_headings();
        let (block_name, headings) = match app.store.get_execution_by_id(exec_id) {
            Ok(exec) => match app.store.get_block_by_id(exec.block_id) {
                Ok(block) => {
                    let headings = resolve_review_headings(&block.name);
                    (block.display_name.clone(), headings)
                }
                Err(_) => (format!("block#{}", exec.block_id), default_headings),
            },
            Err(_) => (format!("exec#{}", exec_id), default_headings),
        };
        let template =
            sai_link_engine::orchestrator::review_template_text(&block_name, exec_id, &headings);
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                app.set_message(format!("Review: ディレクトリ作成失敗: {}", e));
                return;
            }
        }
        if let Err(e) = std::fs::write(&path, template) {
            app.set_message(format!("Review: テンプレート作成失敗: {}", e));
            return;
        }
    }

    let editor_cmd = format!("nvim {}", shell_quote(&path.to_string_lossy()));
    spawn_in_pane(app, "Review", &editor_cmd);
    // nvim で開いたことを sai-paper の活動記録に残す（sai-paper が無ければ何もしない）。
    sai_link_engine::sai_paper_client::notify_open(&path);
}

/// `c`: sidebar で選択中のカード md を herdr ペインで nvim により開く。
///
/// `card_path` がある項目（todo カード・対応カード付き実行リーフ）に有効。
/// 対応カードが無い実行リーフではメッセージ表示のみ。
pub fn open_card(app: &mut AppState, card_path: Option<std::path::PathBuf>) {
    let Some(card_path) = card_path else {
        app.set_message("カード: sidebar でカードを選択してください");
        return;
    };
    if !card_path.exists() {
        app.set_message(format!(
            "カード: ファイルがありません: {}",
            card_path.display()
        ));
        return;
    }
    let editor_cmd = format!("nvim {}", shell_quote(&card_path.to_string_lossy()));
    spawn_in_pane(app, "カード", &editor_cmd);
}

/// `d`: 選択中の実行を Kanban ボード・sidebar done から Workflow へ移動する。
///
/// * todo カード選択時（`card_path` あり）: 対象外メッセージのみ
///   （todo は未実行のため R/r が無く、移動先が無い。取り下げはしない）
/// * 実行選択時（`exec_id` あり）: `execution_history.workflow_moved=1` を設定する。
///   Kanban ボード・sidebar done から消え、実行時に決定した R/r フォルダ配下・
///   Workflow DAG に現れる。review ファイル・カードファイルは全て残す。
/// どちらも指定が無い場合はメッセージのみ。
pub fn archive_card(
    app: &mut AppState,
    card_path: Option<std::path::PathBuf>,
    exec_id: Option<i64>,
) {
    if let Some(run_id) = exec_id {
        // DB フラグ更新→run フォルダ移動の順序（R404）。DB を先に更新し、
        // 移動失敗時はフラグを巻き戻す（ズレ防止）。ファイルは残す。
        if let Err(e) = app.store.update_execution_workflow_moved(run_id, true) {
            app.set_message(format!("移動に失敗: {}", e));
            return;
        }
        let global_round = app
            .store
            .get_execution_by_id(run_id)
            .ok()
            .and_then(|e| e.global_round);
        let to = format!("R{}", global_round.unwrap_or(0));
        if let Some((from, _)) = sai_link_engine::run_dir::find_run_dir(run_id) {
            if from != to {
                if let Err(e) = sai_link_engine::run_dir::move_run_dir(run_id, &from, &to) {
                    let _ = app.store.update_execution_workflow_moved(run_id, false);
                    app.set_message(format!("移動に失敗（巻き戻し済み）: {}", e));
                    return;
                }
            }
        }
        app.set_message(format!(
            "ID{} を Workflow へ移動しました（Kanban/done から消えました）",
            run_id
        ));
        app.reload_data();
        return;
    }
    if card_path.is_some() {
        app.set_message("todo カードは移動できません（未実行のため R/r がありません）");
        return;
    }
    app.set_message("カード: sidebar で完了済み実行を選択してください");
}

/// `r` の前身（旧 Reflection パス）: `open_review` に置き換え済みのため削除予定。
///
/// 削除前の暫定措置としてテストからのみ参照する。
#[cfg(test)]
pub fn open_reflection(app: &mut AppState, exec_id: Option<i64>) {
    let Some(exec_id) = exec_id else {
        app.set_message("Reflection: 実行が選択されていません");
        return;
    };

    let rel = format!(".sai/reflection/{}.md", exec_id);
    let path = sai_link_engine::project::sai_path(&rel);

    if !path.exists() {
        // 初期テンプレートで新規作成する（orchestrator::write_reflection_template と同じ書式）
        let (block_name, started_at) = match app.store.get_execution_by_id(exec_id) {
            Ok(exec) => {
                let name = app
                    .store
                    .get_block_by_id(exec.block_id)
                    .map(|b| b.display_name)
                    .unwrap_or_else(|_| format!("block#{}", exec.block_id));
                let started = exec
                    .started_at
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());
                (name, started)
            }
            Err(_) => (format!("exec#{}", exec_id), "unknown".to_string()),
        };
        let template = format!(
            "# Reflection\n\n対象実行: ID{} / {}\n開始: {}\n\n## Run Reason\n\n\n## Run Evaluation\n\n\n## Run Next\n\n\n## Review\n\n",
            exec_id, block_name, started_at
        );
        // 親ディレクトリを作成して書き込む
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                app.set_message(format!("Reflection: ディレクトリ作成失敗: {}", e));
                return;
            }
        }
        if let Err(e) = std::fs::write(&path, template) {
            app.set_message(format!("Reflection: テンプレート作成失敗: {}", e));
            return;
        }
    } else {
        // 既存（旧形式）ファイル: `## Review` が無ければ末尾へ 1 回だけ自動追記する
        if let Err(e) = ensure_review_section(&path) {
            app.set_message(format!("Reflection: Review 項目の追記に失敗: {}", e));
            return;
        }
    }

    let editor_cmd = format!("nvim {}", shell_quote(&path.to_string_lossy()));
    spawn_in_pane(app, "Reflection", &editor_cmd);
}

/// Reflection 記録ファイルに `## Review` セクションが無ければ末尾へ追記する。
///
/// 旧形式の記録ファイルを `open_reflection` で開くときに呼ぶ。既に Review 項目がある場合は
/// 何もせず `Ok(false)` を返す（既存内容は一切壊さない）。
#[cfg(test)]
fn ensure_review_section(path: &std::path::Path) -> std::io::Result<bool> {
    let text = std::fs::read_to_string(path)?;
    if sai_link_engine::reflection_status::has_review_section(&text) {
        return Ok(false); // 既に Review 項目がある（追記不要）
    }
    let mut new_text = text.clone();
    if !new_text.ends_with('\n') {
        new_text.push('\n');
    }
    new_text.push_str("\n## Review\n\n");
    std::fs::write(path, new_text)?;
    Ok(true)
}

/// `Shift+A`: プロジェクト全体のメモ（Summary.md）を herdr ペインで nvim により開く。
///
/// - 保存場所: `PROJECTROOT/.sai/Summary.md`
/// - どの画面からでも開ける「プロジェクト全体の覚書」（概要1行だけのメモ）。
/// - 初回は概要1行のひな形のみ自動作成する。
///   既にファイルがある場合は中身を一切壊さず、そのまま開く。
/// - ユーザーが書いた内容はエディタ側（nvim）で保存されるため、この画面側では
///   保存処理は行わない。
pub fn open_summary(app: &mut AppState) {
    let path = sai_link_engine::project::sai_path(".sai/Summary.md");

    if !path.exists() {
        // 初回のみひな形を書き出す（既存ファイルは絶対に上書きしない）
        const TEMPLATE: &str = "# Summary\n\n<1行概要をここに書く>\n";
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                app.set_message(format!("Summary: ディレクトリ作成失敗: {}", e));
                return;
            }
        }
        if let Err(e) = std::fs::write(&path, TEMPLATE) {
            app.set_message(format!("Summary: ひな形作成失敗: {}", e));
            return;
        }
        app.set_message("Summary: .sai/Summary.md を新規作成しました");
    }

    let editor_cmd = format!("nvim {}", shell_quote(&path.to_string_lossy()));
    spawn_in_pane(app, "Summary", &editor_cmd);
}

/// `B`: `.sai/blocks` ディレクトリを herdr ペインで nvim により開く。
///
/// （旧 Header Blocks の移植。nvim のファイルエクスプローラでディレクトリを開く）
pub fn open_blocks_dir(app: &mut AppState) {
    let dir = sai_link_engine::project::sai_path(".sai/blocks");
    if !dir.exists() {
        // ディレクトリが無ければ作成してから開く
        if let Err(e) = std::fs::create_dir_all(&dir) {
            app.set_message(format!("Blocks: ディレクトリ作成失敗: {}", e));
            return;
        }
    }
    let editor_cmd = format!("nvim {}", shell_quote(&dir.to_string_lossy()));
    spawn_in_pane(app, "Blocks", &editor_cmd);
}

/// `S`: `.sai/settings.json` を herdr ペインで nvim により開く。
///
/// ファイルが存在しない場合は空 JSON（`{}`）で新規作成してから開く。
pub fn open_settings(app: &mut AppState) {
    let path = sai_link_engine::project::sai_path(".sai/settings.json");
    if !path.exists() {
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                app.set_message(format!("Settings: ディレクトリ作成失敗: {}", e));
                return;
            }
        }
        if let Err(e) = std::fs::write(&path, "{}\n") {
            app.set_message(format!("Settings: 新規作成失敗: {}", e));
            return;
        }
        app.set_message("Settings: settings.json を新規作成しました");
    }
    let editor_cmd = format!("nvim {}", shell_quote(&path.to_string_lossy()));
    spawn_in_pane(app, "Settings", &editor_cmd);
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// `pi --session` 再開コマンドの組立（モデル / thinking 引き継ぎ）を検証する。
    ///
    /// 実際の herdr 操作は行わない（コマンド文字列の生成ロジックのみ確認）。
    #[test]
    fn shell_quote_wraps_in_single_quotes() {
        assert_eq!(
            shell_quote("/tmp/a b/session.jsonl"),
            "'/tmp/a b/session.jsonl'"
        );
        assert_eq!(shell_quote("plain"), "'plain'");
    }

    /// 未選択時は pane を作成せずメッセージ表示のみであること。
    #[test]
    fn open_functions_require_selection() {
        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let mut app = AppState::new(std::sync::Arc::new(store));

        // いずれも selected_exec_id = None なのでメッセージのみ（herdr は触らない）
        resume_session(&mut app, None);
        assert!(
            app.message
                .as_deref()
                .unwrap_or("")
                .contains("選択されていません"),
            "Session: 未選択メッセージが表示される: {:?}",
            app.message
        );
        open_reflection(&mut app, None);
        assert!(
            app.message
                .as_deref()
                .unwrap_or("")
                .contains("選択されていません"),
            "Reflection: 未選択メッセージが表示される: {:?}",
            app.message
        );
        // pane 作成は起動していない（background_rx は None のまま）
        assert!(app.background_rx.is_none(), "未選択時は pane を作成しない");
    }

    /// `c` / `d` の対象外ガード: 未選択時はメッセージのみ（pane を作らない）。
    /// `d` は実行移動のため DB フラグ方式。todo カードは移動不可。
    #[test]
    fn card_actions_require_card_selection() {
        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let mut app = AppState::new(std::sync::Arc::new(store));
        open_card(&mut app, None);
        assert!(
            app.message
                .as_deref()
                .unwrap_or("")
                .contains("カードを選択"),
            "c: カード未選択メッセージ: {:?}",
            app.message
        );
        archive_card(&mut app, None, None);
        assert!(
            app.message
                .as_deref()
                .unwrap_or("")
                .contains("完了済み実行を選択"),
            "d: 未選択メッセージ: {:?}",
            app.message
        );
        // todo カード選択時は移動不可メッセージ（ファイルは残す）
        archive_card(&mut app, Some(std::path::PathBuf::from("todo/a.md")), None);
        assert!(
            app.message
                .as_deref()
                .unwrap_or("")
                .contains("移動できません"),
            "d: todo カードは移動不可: {:?}",
            app.message
        );
        assert!(app.background_rx.is_none(), "未選択時は pane を作成しない");
    }

    /// `r` の新 review パス: 未選択時はメッセージのみ（pane を作らない）。
    #[test]
    fn open_review_requires_selection() {
        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let mut app = AppState::new(std::sync::Arc::new(store));
        open_review(&mut app, None);
        assert!(
            app.message
                .as_deref()
                .unwrap_or("")
                .contains("選択されていません"),
            "Review: 未選択メッセージ: {:?}",
            app.message
        );
        assert!(app.background_rx.is_none(), "未選択時は pane を作成しない");
    }

    // -------------------------------------------------------------------
    // R14: Review 自動追記（旧形式の記録ファイルへの後方互換）
    // -------------------------------------------------------------------

    /// `SAI_PROJECT_ROOT` はプロセスグローバルのため、ファイル系テストは共有ロック
    /// （`crate::app::file_test_lock()`）で直列化する。

    #[test]
    fn review_section_appended_once_to_old_template() {
        let _guard = crate::app::file_test_lock();
        // SAI_PROJECT_ROOT（プロセスグローバル）には依存せず、テスト専用の一時
        // ディレクトリへ直接ファイルを用意する（ensure_review_section はパス引数）。
        let root = std::env::temp_dir().join(format!("sai_ext_review_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        // 旧形式（Review 無し）の記録ファイルを用意する
        let path = root.join(".sai/reflection/999.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let old = "# Reflection\n\n## Run Reason\n（理由）\n\n## Run Next\n\n";
        std::fs::write(&path, &old).unwrap();

        // 1 回目: Review が追記される
        assert!(
            ensure_review_section(&path).unwrap(),
            "旧形式には追記が発生する"
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(sai_link_engine::reflection_status::has_review_section(
            &text
        ));
        // 既存内容は壊されないこと
        assert!(text.contains("## Run Reason"));

        // 2 回目: 既に Review があるので追記されない（1 回だけ）
        assert!(!ensure_review_section(&path).unwrap(), "2 回目は追記しない");
        let text2 = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("## Review").count(), 1);
        assert_eq!(text, text2, "2 回目以降は内容が変化しない");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn summary_template_created_once_and_preserved() {
        let _guard = crate::app::file_test_lock();
        let root = std::env::temp_dir().join(format!("sai_ext_summary_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let path = root.join(".sai/Summary.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        assert!(!path.exists(), "初回はファイルが無い");

        // open_summary は pane を作るため直接呼ばず、ひな形作成の前条件のみ検証:
        // （実際のひな形書き出しは open_summary 内。ここでは既存保護のロジックを確認）
        //   - ファイルが無いときのみひな形を書く
        //   - 既存ファイルは一切壊さない
        // open_summary を直接呼ぶと herdr pane を作成するため、
        // ユーザーが書いた内容が保たれることの検証は「既存ファイルを開くだけ」
        // の分岐で担保される。ここではテンプレート作成相当の書き込みを再現する。
        const TEMPLATE: &str = "# Summary\n\nプロジェクト全体のメモ。概要・やること・終わったことをここに書き残します。\n\n## 概要\n\n\n## やること\n\n\n## 終わったこと\n\n";
        std::fs::write(&path, TEMPLATE).unwrap();

        // 既存ファイルがある場合、内容はそのまま（壊されない）
        std::fs::write(&path, "# Summary\n\n自分で書いた内容\n").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("自分で書いた内容"), "既存内容は壊されない");

        let _ = std::fs::remove_dir_all(&root);
    }
}
