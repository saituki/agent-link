//! run の中身（md / waggle 成果物）の解決（TUI / CLI 共通・実装計画 12 §3.2）。
//!
//! 旧 `sai-link-tui::app::data`（`load_md_view` のパス解決・`waggle_choices_for`・
//! `load_waggle_token_view`）から移設した。TUI は解決結果を描画用ビューへ、
//! CLI（`sai-link phone md` / `phone waggle`）は JSON へ組み替える。

use sai_link_engine::db::SaiStore;
use std::path::PathBuf;

/// md の種別（`Space t` / `Space r` / `Space m`・`phone md --kind`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdKind {
    /// review.md
    Review,
    /// task.md
    Task,
    /// message.md（session から再生成）
    Message,
}

impl MdKind {
    /// タイトル表示用のラベル。
    pub fn label(self) -> &'static str {
        match self {
            MdKind::Review => "review",
            MdKind::Task => "task",
            MdKind::Message => "message",
        }
    }

    /// ファイル名（`phone md` の `name` 用）。
    pub fn file_name(self) -> &'static str {
        match self {
            MdKind::Review => "review.md",
            MdKind::Task => "task.md",
            MdKind::Message => "message.md",
        }
    }
}

/// md パス解決の結果。
#[derive(Debug)]
pub enum MdResolve {
    /// 読めるファイルが見つかった
    Found(std::path::PathBuf),
    /// ファイルが無い（TUI はメッセージ表示・CLI は終了コード 3「まだありません」）
    Missing(String),
    /// 生成・読み込みに失敗した（メッセージ付き）
    Failed(String),
}

/// run の md パスを解決する（旧 `load_md_view` のパス解決部分）。
///
/// - `message` は session から作り直して上書きする（毎回最新）
/// - `task` は run フォルダの正本 → 未実行 todo カードの順に探索
pub fn resolve_md_path(store: &SaiStore, run_id: i64, kind: MdKind) -> MdResolve {
    let path = match kind {
        MdKind::Message => match sai_link_engine::orchestrator::write_message_file(store, run_id) {
            Ok(p) => p,
            Err(e) => return MdResolve::Failed(format!("message: 保存に失敗: {}", e)),
        },
        MdKind::Review => sai_link_engine::orchestrator::review_path(run_id),
        MdKind::Task => match crate::card_entries::card_path_for_execution(None, run_id) {
            Some(p) => p,
            None => match task_path_for_unexecuted(run_id) {
                Some(p) => p,
                None => {
                    return MdResolve::Missing(format!(
                        "task: task.md が見つかりません (run_id {})",
                        run_id
                    ))
                }
            },
        },
    };
    match kind {
        MdKind::Message => {
            if path.exists() {
                MdResolve::Found(path)
            } else {
                MdResolve::Missing(format!(
                    "message: message.md がありません ({})",
                    path.display()
                ))
            }
        }
        MdKind::Review => {
            if path.exists() {
                MdResolve::Found(path)
            } else {
                MdResolve::Missing(format!(
                    "review: review.md がありません ({})",
                    path.display()
                ))
            }
        }
        MdKind::Task => {
            if path.exists() {
                MdResolve::Found(path)
            } else {
                MdResolve::Missing(format!("task: task.md がありません ({})", path.display()))
            }
        }
    }
}

/// 未実行 todo カードの task.md を run_id で探索する（DB行なし用）。
fn task_path_for_unexecuted(run_id: i64) -> Option<std::path::PathBuf> {
    let root = sai_link_engine::project::sai_path(sai_link_engine::run_dir::OUTPUTS_DIR)
        .join(sai_link_engine::run_dir::TODO_OWNER);
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let task = entry.path().join(sai_link_engine::run_dir::TASK_FILE);
            if !task.is_file() {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&task) {
                if let Some(card) = sai_link_engine::kanban::parse_card(&text) {
                    if let Ok(rid) = card.frontmatter.run_id.trim().parse::<i64>() {
                        if rid == run_id {
                            return Some(task);
                        }
                    }
                }
            }
        }
    }
    None
}

/// waggle トークン選択肢 1 件（md トークンのみ）。
#[derive(Debug, Clone)]
pub struct WaggleChoiceData {
    /// waggle トークン文字列
    pub token: String,
    /// 元ファイル名（basename）
    pub file_name: String,
    /// 出力の説明（マニフェストの description）
    pub description: String,
    /// 出力元ブロック名
    pub block_name: String,
    /// 実行状態（running / completed 等。U03 で追加）
    #[allow(dead_code)]
    pub status: String,
    /// この文書を保存した実行の番号（U08）。
    ///
    /// 参照 `waggle:<tok>:path:.sai/outputs/*/<run_id>/<file>` の `<run_id>` には
    /// この値を使う（origin 実行の番号ではない）。
    pub run_id: i64,
}

/// 文書候補 1 件を作るための入力（実行番号を明示したもの）。
///
/// DB の `WaggleTokenEntry` / `execution_history` の行から作る。
#[derive(Debug, Clone)]
pub struct DocTokenEntry {
    /// waggle トークン文字列
    pub token: String,
    /// 文書を保存した実行の番号（`execution_history.id` = run_id）
    pub run_id: i64,
    /// 出力元ブロック名
    pub block_name: String,
    /// 実行状態（表示用）
    pub status: String,
}

/// 文書候補を参照記法 `waggle:<tok>:path:.sai/outputs/*/<run_id>/<file>` にする。
///
/// `<run_id>` は origin 実行ではなく **その文書を保存した実行の番号**（`c.run_id`）。
/// 正はトークンで、パスは人間向けの目安（共有契約）。
pub fn doc_ref_line(c: &WaggleChoiceData) -> String {
    format!(
        "waggle:{}:path:.sai/outputs/*/{}/{}",
        c.token, c.run_id, c.file_name
    )
}

/// 文書選択の出力先（全 task 実行経路で共通）。
///
/// - `Single`: TUI `w` キー・`phone waggle <id>` 用。指定実行の文書だけ
/// - `Origin`: task.md 作成時（TUI Step4 / CLI / phone 共通）。origin 実行の文書＋実行中を含む全体
#[derive(Debug, Clone)]
pub enum DocSelectScope {
    /// 指定実行の文書だけ（従来の `waggle_choices` と同じ）
    Single(i64),
    /// origin 実行の文書を先頭に、実行中を含む全体を後ろに並べる
    Origin(i64),
}

/// 選択実行の md waggle トークン一覧を返す（`w` / `phone waggle` 用）。
///
/// `output_waggle_tokens` を解決し、`.md` のトークンのみを対象にする。
/// 解決できないトークン・非 md トークンはスキップする。
/// U03: 実行中（running / pending・仮 completed）のトークンも状態付きで返す。
pub fn waggle_choices(store: &SaiStore, run_id: i64) -> Vec<WaggleChoiceData> {
    select_docs(store, &DocSelectScope::Single(run_id))
}

/// 全 task 実行経路で使う文書選択の共通関数。
///
/// - TUI `w` キー: `Single(選択中 run_id)`
/// - task.md 作成（TUI Step4 / `phone card inputs --origin-run-id` / CLI `--field` 解決）: `Origin(origin_run_id)`
/// - 実行時 `task_text` 内の `waggle:` 展開は `placeholder.rs` が行う（ここでは候補提示のみ）
///
/// `Origin` では origin の文書を先頭に、全体（実行中含む）を重複排除で後ろに並べる。
/// 状態は DB の `status` をそのまま載せる（running / review / completed 等）。
pub fn select_docs(store: &SaiStore, scope: &DocSelectScope) -> Vec<WaggleChoiceData> {
    match scope {
        DocSelectScope::Single(run_id) => choices_for_run(store, *run_id),
        DocSelectScope::Origin(origin_id) => {
            let mut out = choices_for_run(store, *origin_id);
            let mut seen: std::collections::HashSet<String> =
                out.iter().map(|c| c.token.clone()).collect();
            // 全体（実行中含む）から md のみ追加。origin 重複は除外
            let all = choices_all_including_running(store);
            for c in all {
                if seen.insert(c.token.clone()) {
                    out.push(c);
                }
            }
            out
        }
    }
}

/// 指定実行の文書だけを返す（従来の `waggle_choices` 本体）。
fn choices_for_run(store: &SaiStore, run_id: i64) -> Vec<WaggleChoiceData> {
    let (tokens, status) = store
        .get_execution_by_id(run_id)
        .ok()
        .map(|e| {
            let toks: Vec<String> = e
                .output_waggle_tokens
                .as_deref()
                .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
                .unwrap_or_default();
            (toks, e.status.clone())
        })
        .unwrap_or_default();
    if tokens.is_empty() {
        return Vec::new();
    }
    let db_path = sai_link_engine::project::resolve_db_path();
    let Ok(integrator) = sai_link_engine::waggle_integration::WaggleIntegrator::open(&db_path)
    else {
        return Vec::new();
    };
    let entries: Vec<DocTokenEntry> = tokens
        .into_iter()
        .map(|token| DocTokenEntry {
            token,
            run_id,
            block_name: String::new(),
            status: status.clone(),
        })
        .collect();
    build_choices(&integrator, entries)
}

/// 実行番号つきの候補列から md のみの `WaggleChoiceData` を作る。
///
/// 解決できないトークン・非 md トークンは除外する（`run_id` はそのまま保持）。
fn build_choices(
    integrator: &sai_link_engine::waggle_integration::WaggleIntegrator,
    entries: Vec<DocTokenEntry>,
) -> Vec<WaggleChoiceData> {
    let mut out = Vec::new();
    for e in entries {
        let Ok(view) = integrator.resolve_token(&e.token) else {
            continue;
        };
        let file_name = view
            .manifest
            .target
            .as_str()
            .strip_prefix("file://")
            .and_then(|p| p.rsplit('/').next())
            .unwrap_or("")
            .to_string();
        if !file_name.to_ascii_lowercase().ends_with(".md") {
            continue;
        }
        // ブロック名はマニフェストのラベルを優先し、無ければ DB の値を入れる
        let block_name = view
            .manifest
            .labels
            .get("block_name")
            .cloned()
            .filter(|s| !s.is_empty())
            .unwrap_or(e.block_name);
        out.push(WaggleChoiceData {
            token: e.token,
            file_name,
            description: view.manifest.meta.description.clone().unwrap_or_default(),
            block_name,
            status: e.status,
            run_id: e.run_id,
        });
    }
    out
}

/// 最近の文書（実行中を含む全体の md）を新しい順に `limit` 件返す（Web の task_text の候補）。
///
/// 並びは `list_waggle_tokens` と同じ（実行の作成日時の新しい順）。
pub fn recent_docs(store: &SaiStore, limit: usize) -> Vec<WaggleChoiceData> {
    let mut out = choices_all_including_running(store);
    out.truncate(limit);
    out
}

/// 全体（実行中含む）の md 文書を DB 状態付きで返す（`Origin` の後半用）。
///
/// `list_waggle_tokens(include_running=true)` のトークンを integrator で解決し、
/// md のみ・解決不可は除外。状態はトークン行の `status` を載せる。
fn choices_all_including_running(store: &SaiStore) -> Vec<WaggleChoiceData> {
    let entries = store.list_waggle_tokens(None, None, -1, true).unwrap_or_default();
    if entries.is_empty() {
        return Vec::new();
    }
    let db_path = sai_link_engine::project::resolve_db_path();
    let Ok(integrator) = sai_link_engine::waggle_integration::WaggleIntegrator::open(&db_path)
    else {
        return Vec::new();
    };
    let entries: Vec<DocTokenEntry> = entries
        .into_iter()
        .map(|e| DocTokenEntry {
            token: e.token,
            // `WaggleTokenEntry.execution_id` は `execution_history.id` = run_id
            run_id: e.execution_id,
            block_name: e.block_name,
            status: e.status,
        })
        .collect();
    build_choices(&integrator, entries)
}

/// waggle トークン 1 件の内容（`phone waggle --token` 用）。
#[derive(Debug, Clone)]
pub struct WaggleDoc {
    /// waggle トークン文字列
    pub token: String,
    /// 元ファイル名（basename）
    pub file_name: String,
    /// 出力の説明
    pub description: String,
    /// 出力元ブロック名
    pub block_name: String,
    /// 本文（元ファイルが無い場合は Blob ストアから復元）
    pub content: String,
    /// 元ファイルの絶対パス（`file://` 以外のトークンでは `None`）。
    ///
    /// sai-paper の活動記録（`activity open --path`）に渡すために使う。
    /// 元ファイルが既に消えていても記録できるよう、存在確認はしない。
    pub path: Option<PathBuf>,
}

/// waggle トークンの内容を解決する（元ファイルが無ければ Blob ストアから復元）。
///
/// エラー文字列は旧 `load_waggle_token_view` のメッセージと同一。
pub fn load_waggle_token(token: &str) -> Result<WaggleDoc, String> {
    let db_path = sai_link_engine::project::resolve_db_path();
    let integrator = sai_link_engine::waggle_integration::WaggleIntegrator::open(&db_path)
        .map_err(|e| format!("waggle: integrator 起動失敗: {}", e))?;
    let view = integrator
        .resolve_token(token)
        .map_err(|e| format!("waggle: 解決エラー: {}", e))?;
    let content = integrator
        .resolve_content(token)
        .map_err(|e| format!("waggle: 内容解決失敗: {}", e))?;
    let target = view.manifest.target.as_str();
    let file_name = target
        .strip_prefix("file://")
        .and_then(|p| p.rsplit('/').next())
        .unwrap_or("")
        .to_string();
    // sai-paper の `activity open --path` 用に元ファイルの絶対パスを取り出す。
    let path = target
        .strip_prefix("file://localhost")
        .or_else(|| target.strip_prefix("file://"))
        .map(|p| PathBuf::from(p.replace("%20", " ")));
    Ok(WaggleDoc {
        token: token.to_string(),
        file_name,
        description: view.manifest.meta.description.clone().unwrap_or_default(),
        block_name: view
            .manifest
            .labels
            .get("block_name")
            .cloned()
            .unwrap_or_default(),
        content,
        path,
    })
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use sai_link_engine::waggle_integration::WaggleIntegrator;

    /// 一時ファイル（`.md`）を作り、トークン化してトークンを返す。
    fn tokenize_temp(integrator: &WaggleIntegrator, name: &str, body: &str) -> String {
        let path = std::env::temp_dir().join(format!("sai-link-u08-{}-{}.md", name, body.len()));
        std::fs::write(&path, body).expect("一時ファイルを書けない");
        let result = integrator
            .tokenize_file(&path, "block-a", Some("説明"))
            .expect("トークン化に失敗");
        result.token.as_str().to_string()
    }

    /// 全体の文書（別の実行のものを含む）でも、その文書を保存した実行番号が
    /// 候補の `run_id` に入る（AC1）。
    #[test]
    fn build_choices_keeps_each_docs_run_id() {
        let integrator = WaggleIntegrator::open_in_memory().expect("インメモリ DB");
        let t1 = tokenize_temp(&integrator, "a", "実行 1 の文書");
        let t3 = tokenize_temp(&integrator, "b", "実行 3 の文書");

        let choices = build_choices(
            &integrator,
            vec![
                DocTokenEntry {
                    token: t1.clone(),
                    run_id: 1,
                    block_name: "block-a".to_string(),
                    status: "completed".to_string(),
                },
                DocTokenEntry {
                    token: t3.clone(),
                    run_id: 3,
                    block_name: "block-a".to_string(),
                    status: "running".to_string(),
                },
            ],
        );

        assert_eq!(choices.len(), 2);
        let c1 = choices.iter().find(|c| c.token == t1).expect("実行 1 の候補");
        let c3 = choices.iter().find(|c| c.token == t3).expect("実行 3 の候補");
        assert_eq!(c1.run_id, 1);
        assert_eq!(c3.run_id, 3, "origin ではなく文書の実行番号が入る");
    }

    /// 参照の `<run_id>` に origin ではなく文書の実行番号が入る（AC2）。
    #[test]
    fn doc_ref_line_uses_doc_run_id() {
        let c = WaggleChoiceData {
            token: "W9knAR4G".to_string(),
            file_name: "paper-search-eeg-llm-2026.md".to_string(),
            description: "計画書".to_string(),
            block_name: "paper-search".to_string(),
            status: "completed".to_string(),
            run_id: 3,
        };
        assert_eq!(
            doc_ref_line(&c),
            "waggle:W9knAR4G:path:.sai/outputs/*/3/paper-search-eeg-llm-2026.md"
        );
    }

    /// 解決できないトークンは候補から除外される（実行番号は=inline で決まる）。
    #[test]
    fn build_choices_skips_unresolvable_token() {
        let integrator = WaggleIntegrator::open_in_memory().expect("インメモリ DB");
        let choices = build_choices(
            &integrator,
            vec![DocTokenEntry {
                token: "AbCdEfGh".to_string(),
                run_id: 2,
                block_name: "block-a".to_string(),
                status: "completed".to_string(),
            }],
        );
        assert!(choices.is_empty());
    }
}
