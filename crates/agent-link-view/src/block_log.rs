//! Log の表示データ（`[Inputs] [Meta] [Output] [Stats]` 相当）の組み立て。
//!
//! 描画は持たない。`phone run --view log`（Web の中継）が使う
//! `BlockDetail` とその JSON 表現、および DB からの組み立てを提供する。

use crate::fmt::{format_cost, format_tokens};

/// 表示用ブロック詳細データ（旧 right_sidebar::BlockDetail の移植）。
///
/// `phone run --view log` 用に JSON 化可能（項目名は snake_case のまま、
/// `input_fields` のタプルのみ `[{"label","value"}]` に変換）。
#[derive(Debug, Clone, Default)]
pub struct BlockDetail {
    /// 対象実行 ID（タイトル表示用）
    pub exec_id: i64,
    /// 対象ブロック名（タイトル表示用）
    pub block_name: String,
    /// この実行が user-block ブロックか（true のとき [Meta] / [Output] のみ表示）
    pub is_user_block: bool,
    /// user-block の説明（[Output] セクションで表示）
    pub user_block_description: Option<String>,
    /// user-block の入力ファイルパス（[Output] セクションで表示）
    pub user_block_file: Option<String>,
    /// 実行時のモデル（execution_history 優先、なければ block_metadata）
    pub model: Option<String>,
    /// 実行時の thinking（execution_history 優先、なければ block_metadata）
    pub thinking: Option<String>,
    pub timeout: Option<i32>,
    pub connections_from: Vec<String>,
    pub connections_to: Vec<String>,
    /// 接続を作られた順（時系列）に並べた構造化版（phone の Connection 表示用）。
    /// TUI は `connections_from` / `connections_to` の文字列を使う。
    pub connections: Vec<ConnectionEntry>,
    /// この実行が出力した waggle トークン一覧（sai-write 使用時）
    pub output_waggle_tokens: Vec<String>,
    /// セッション統計情報
    pub session_stats: Option<SessionStatsData>,
    /// pi起動時のHEADコミットハッシュ
    pub git_commit_before: Option<String>,
    /// pi終了時のHEADコミットハッシュ
    pub git_commit_after: Option<String>,
    /// セッションディレクトリのパス
    pub session_dir: Option<String>,
    /// 実行開始日時（ISO 8601）
    pub started_at: Option<String>,
    /// 実行終了日時（ISO 8601）
    pub finished_at: Option<String>,
    /// 実行レコード作成日時（CREATE 表示用。DB `created_at`）
    pub created_at: Option<String>,
    /// 入力フィールド（ラベル, 値）一覧。[Inputs] セクションで表示する。
    pub input_fields: Vec<(String, String)>,
    /// 費用の表示行（B02。先頭付近に出す）。
    ///
    /// [`usage_lines`] が `usage_with_children` の結果から作り、Log 画面が
    /// そのままの行を描画する。空 = 実行の usage 記録が無い（描画しない）。
    pub usage_lines: Vec<String>,
}

/// 接続 1 件（phone の Connection 表示用）。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ConnectionEntry {
    /// `from`（この実行へ来た線）/ `to`（この実行から出る線）
    pub direction: String,
    /// `task_text`（waggle の受け渡し）/ `task_create`（子の作成）/ `task_output`（子の出力）
    pub kind: String,
    /// 相手の実行 ID
    pub run_id: i64,
    /// 相手のブロック名
    pub block_name: String,
    /// 相手の task.md の description（`task_create` のときだけ入る）
    pub description: Option<String>,
    /// waggle トークン（`task_text` のときだけ入る）
    pub waggle_token: Option<String>,
    /// waggle 文書のファイルパス（`task_text` のときだけ。プロジェクト内なら相対）
    pub file_path: Option<String>,
}

/// セッション統計データ。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SessionStatsData {
    pub turn_count: u64,
    pub tool_call_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub thinking_tokens: u64,
    pub total_tokens: u64,
}

/// Log 画面に出す費用の表示行（B02）を作る。
///
/// 先頭付近に出す 1〜2 行（子がいない場合は 1 行）を返す。
///
/// ```text
/// Cost  $0.42 (input 120k, output 8.1k, cache 300k)
/// Total with children  $1.35 (3 children, 1 with unknown cost)
/// ```
///
/// - `own` が `None`（usage 未保存）の場合は
///   `Cost  not recorded (create it with agent-link usage backfill)` の 1 行。
/// - 子がある場合のみ「子を含めた合計」の行を追加する。
/// - 子の費用不明件数は「usage 行が無い子」も含める（どちらも費用不明のため）。
/// - 費用・トークンの書式は [`crate::fmt::format_cost`] / [`crate::fmt::format_tokens`]。
pub fn usage_lines(summary: &agent_link_engine::usage::UsageSummary) -> Vec<String> {
    let mut lines = Vec::new();
    match &summary.own {
        Some(t) => lines.push(format!(
            "Cost  {} (input {}, output {}, cache {})",
            format_cost(t.cost),
            format_tokens(t.input),
            format_tokens(t.output),
            format_tokens(t.cache)
        )),
        None => lines.push(
            "Cost  not recorded (create it with agent-link usage backfill)".to_string(),
        ),
    }
    if !summary.children.is_empty() {
        let unknown = summary
            .children
            .iter()
            .filter(|(_, u)| u.as_ref().and_then(|t| t.cost).is_none())
            .count();
        let detail = if unknown > 0 {
            format!(
                "({} children, {} with unknown cost)",
                summary.children.len(),
                unknown
            )
        } else {
            format!("({} children)", summary.children.len())
        };
        lines.push(format!(
            "Total with children  {} {}",
            format_cost(summary.total.cost),
            detail
        ));
    }
    lines
}

/// `BlockDetail` の `input_fields` 用（タプル → `{label, value}`）。
#[derive(serde::Serialize)]
struct InputRef<'a> {
    label: &'a str,
    value: &'a str,
}

/// `phone run --view log` 用の JSON 化（実装計画 12 §3.2）。
///
/// フィールド名は `BlockDetail` の snake_case をそのまま使い、
/// 描画用の内部状態は持たないため出力対象は全公開フィールド。
impl serde::Serialize for BlockDetail {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let len = 22;
        let mut st = serializer.serialize_struct("BlockDetail", len)?;
        st.serialize_field("exec_id", &self.exec_id)?;
        st.serialize_field("block_name", &self.block_name)?;
        st.serialize_field("is_user_block", &self.is_user_block)?;
        st.serialize_field("user_block_description", &self.user_block_description)?;
        st.serialize_field("user_block_file", &self.user_block_file)?;
        st.serialize_field("model", &self.model)?;
        st.serialize_field("thinking", &self.thinking)?;
        st.serialize_field("timeout", &self.timeout)?;
        st.serialize_field("connections_from", &self.connections_from)?;
        st.serialize_field("connections_to", &self.connections_to)?;
        st.serialize_field("connections", &self.connections)?;
        st.serialize_field("output_waggle_tokens", &self.output_waggle_tokens)?;
        st.serialize_field("session_stats", &self.session_stats)?;
        st.serialize_field("git_commit_before", &self.git_commit_before)?;
        st.serialize_field("git_commit_after", &self.git_commit_after)?;
        st.serialize_field("session_dir", &self.session_dir)?;
        st.serialize_field("started_at", &self.started_at)?;
        st.serialize_field("finished_at", &self.finished_at)?;
        st.serialize_field("created_at", &self.created_at)?;
        let input_fields: Vec<InputRef<'_>> = self
            .input_fields
            .iter()
            .map(|(label, value)| InputRef { label, value })
            .collect();
        st.serialize_field("input_fields", &input_fields)?;
        st.serialize_field("usage_lines", &self.usage_lines)?;
        st.end()
    }
}

// ============================================================================
// ビルダ純関数（DB → 表示データ変換）
// ============================================================================

/// 実行 1 件分の表示用詳細データを DB から構築する（ビルダ純関数）。
///
/// agent-link-tui `app/data.rs::build_block_detail()` 本体を分離したもの。
/// `sai_control_tui` 等の第 2 クライアントからも同一ビルダを使い、同一画面を
/// 表示できるようにする。UI 側の副作用（選択実行 ID の覚え置き等）は
/// 呼び出し側（各 TUI）が行う。
///
/// - `store`: 対象プロジェクトの `SaiStore`（read-only でも可）
/// - `exec_id`: 対象実行 ID
///
/// ブロック定義が見つからない場合は `None`（実行レコードのみ見つからない場合は
/// agent-link と同じ `block_id = exec_id` フォールバックを適用する）。
pub fn build(store: &agent_link_engine::db::SaiStore, exec_id: i64) -> Option<BlockDetail> {
    let block_id = match store.get_execution_by_id(exec_id) {
        Ok(exec) => exec.block_id,
        Err(_) => exec_id,
    };

    let output_tokens = get_waggle_tokens_for_execution(store, exec_id);
    let session_stats = get_session_stats_for_execution(store, exec_id);
    let exec = store.get_execution_by_id(exec_id).ok();

    let block = store.get_block_by_id(block_id).ok()?;

    // user-block ブロックかどうか（[Meta] / [Output] のみ表示するため）
    let is_user_block = block.display_name == "user-block";

    // 接続情報
    let format_block_ref = |bid: i64, eid: i64| -> String {
        if store.get_block_by_id(bid).is_ok() {
            format!("ID{}", eid)
        } else {
            format!("ID{}/id:{}", eid, bid)
        }
    };

    let all_connections = store
        .list_connections_by_block(block_id)
        .ok()
        .unwrap_or_default();

    // 接続 1 件分の表示行を整形する。
    // waggle_token 接続はトークンを先頭に付与し、
    // task_delegation / task_output（親子）は型ラベルを付ける。
    // 線は親子と waggle の受け渡しだけ（review / kanban_origin / git_commit は出さない）。
    let format_conn_line = |c: &agent_link_engine::db::models::BlockConnection,
                            other_bid: i64,
                            other_eid: i64|
     -> String {
        let ref_str = format_block_ref(other_bid, other_eid);
        match c.connection_type.as_str() {
            "waggle_token" => match &c.waggle_token {
                Some(t) => format!("{} {}", t, ref_str),
                None => ref_str,
            },
            "task_delegation" => format!("[task] {}", ref_str),
            "task_output" => format!("[out] {}", ref_str),
            _ => ref_str,
        }
    };

    let is_shown = |c: &&agent_link_engine::db::models::BlockConnection| {
        matches!(
            c.connection_type.as_str(),
            "waggle_token" | "task_delegation" | "task_output"
        )
    };
    let from: Vec<String> = all_connections
        .iter()
        .filter(|c| c.target_execution_id == exec_id && is_shown(c))
        .map(|c| format_conn_line(c, c.source_block_id, c.source_execution_id))
        .collect();

    let to: Vec<String> = all_connections
        .iter()
        .filter(|c| c.source_execution_id == exec_id && is_shown(c))
        .map(|c| format_conn_line(c, c.target_block_id, c.target_execution_id))
        .collect();

    // 構造化版（作られた順。`list_connections_by_block` が created_at 順で返す）
    let connections = structured_connections(store, &all_connections, exec_id);

    let model = exec
        .as_ref()
        .and_then(|e| e.model.clone())
        .or(Some(block.model.clone()));
    let thinking = exec
        .as_ref()
        .and_then(|e| e.thinking.clone())
        .or(Some(block.thinking.clone()));
    let timeout_override = exec
        .as_ref()
        .and_then(|e| e.timeout)
        .or(Some(block.timeout));

    let git_commit_before = exec.as_ref().and_then(|e| e.git_commit_before.clone());
    let git_commit_after = exec.as_ref().and_then(|e| e.git_commit_after.clone());
    let session_dir = exec.as_ref().and_then(|e| e.session_dir.clone());
    let started_at = exec.as_ref().and_then(|e| e.started_at.clone());
    let finished_at = exec.as_ref().and_then(|e| e.finished_at.clone());
    let created_at = exec.as_ref().and_then(|e| {
        let c = e.created_at.trim();
        if c.is_empty() {
            None
        } else {
            Some(c.to_string())
        }
    });

    // 入力フィールド（[Inputs] 用）: user_inputs_json の値を
    // ブロック定義の入力フィールド順（ラベル付き）で並べる
    let input_fields: Vec<(String, String)> = {
        let mut pairs: Vec<(String, String)> = Vec::new();
        if let Some(json) = exec.as_ref().and_then(|e| e.user_inputs_json.as_deref()) {
            if let Ok(map) =
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json)
            {
                let mut used_keys: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                // ブロック定義（block_inputs）のフィールド順でラベルを付ける
                if let Ok(defs) = store.get_inputs_by_block_id(block_id) {
                    for inp in defs {
                        if let Some(v) = map.get(&inp.input_id) {
                            pairs.push((inp.label.clone(), json_value_to_string(v)));
                            used_keys.insert(inp.input_id.clone());
                        }
                    }
                }
                // 定義にないキーは末尾に追加
                for (k, v) in map.iter() {
                    if !used_keys.contains(k) {
                        pairs.push((k.clone(), json_value_to_string(v)));
                    }
                }
            }
        }
        pairs
    };

    // user-block の説明・入力ファイル（[Output] セクションで表示）
    let (user_block_description, user_block_file) = {
        let mut desc = None;
        let mut file = None;
        if let Some(json) = exec.as_ref().and_then(|e| e.user_inputs_json.as_deref()) {
            if let Ok(map) =
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json)
            {
                if let Some(v) = map.get("description") {
                    desc = v.as_str().map(|s| s.to_string());
                }
                if let Some(v) = map.get("file_path") {
                    file = v.as_str().map(|s| s.to_string());
                }
            }
        }
        (desc, file)
    };

    let block_name = block.display_name.clone();
    // B02: 費用の表示行（`usage_with_children` から。記録が無ければ空でも良い）。
    // 費用不明（記録あり・cost 無し）と記録無しは `usage_lines` 側で区別する。
    let cost_lines = agent_link_engine::usage::usage_with_children(store, exec_id)
        .map(|s| usage_lines(&s))
        .unwrap_or_default();
    Some(BlockDetail {
        exec_id,
        block_name,
        is_user_block,
        user_block_description,
        user_block_file,
        model,
        thinking,
        timeout: timeout_override,
        connections_from: from,
        connections_to: to,
        connections,
        output_waggle_tokens: output_tokens,
        input_fields,
        usage_lines: cost_lines,
        session_stats,
        git_commit_before,
        git_commit_after,
        session_dir,
        started_at,
        finished_at,
        created_at,
    })
}

/// 接続を phone 用の構造化した並びにする（`is_shown` と同じ 3 種だけ）。
fn structured_connections(
    store: &agent_link_engine::db::SaiStore,
    all: &[agent_link_engine::db::models::BlockConnection],
    exec_id: i64,
) -> Vec<ConnectionEntry> {
    let mut integrator = None; // waggle の解決が要るときだけ開く
    let mut out = Vec::new();
    for c in all {
        let kind = match c.connection_type.as_str() {
            "waggle_token" => "task_text",
            "task_delegation" => "task_create",
            "task_output" => "task_output",
            _ => continue,
        };
        let (direction, other_bid, other_eid) = if c.target_execution_id == exec_id {
            ("from", c.source_block_id, c.source_execution_id)
        } else if c.source_execution_id == exec_id {
            ("to", c.target_block_id, c.target_execution_id)
        } else {
            continue;
        };
        let block_name = store
            .get_block_by_id(other_bid)
            .map(|b| b.display_name)
            .unwrap_or_default();
        let mut entry = ConnectionEntry {
            direction: direction.to_string(),
            kind: kind.to_string(),
            run_id: other_eid,
            block_name,
            ..Default::default()
        };
        if kind == "task_create" {
            entry.description = run_description(other_eid);
        }
        if kind == "task_text" {
            if let Some(t) = &c.waggle_token {
                entry.waggle_token = Some(t.clone());
                if integrator.is_none() {
                    integrator = agent_link_engine::waggle_integration::WaggleIntegrator::open(
                        &agent_link_engine::project::resolve_db_path(),
                    )
                    .ok();
                }
                entry.file_path = integrator.as_ref().and_then(|i| waggle_file_path(i, t));
            }
        }
        out.push(entry);
    }
    out
}

/// run の task.md の `description:`（空・読めないときは `None`）。
fn run_description(run_id: i64) -> Option<String> {
    let (_, dir) = agent_link_engine::run_dir::find_run_dir(run_id)?;
    let text = std::fs::read_to_string(dir.join(agent_link_engine::run_dir::TASK_FILE)).ok()?;
    let d = agent_link_engine::kanban::parse_card(&text)?.frontmatter.description;
    let d = d.trim();
    (!d.is_empty()).then(|| d.to_string())
}

/// waggle トークンの元ファイルのパス（プロジェクトの中なら相対パス）。
fn waggle_file_path(
    integrator: &agent_link_engine::waggle_integration::WaggleIntegrator,
    token: &str,
) -> Option<String> {
    let view = integrator.resolve_token(token).ok()?;
    let target = view.manifest.target.as_str();
    let path = target
        .strip_prefix("file://localhost")
        .or_else(|| target.strip_prefix("file://"))?
        .replace("%20", " ");
    let root = agent_link_engine::project::project_root();
    let rel = std::path::Path::new(&path).strip_prefix(&root).ok();
    Some(rel.map_or(path.clone(), |r| r.display().to_string()))
}

/// 実行レコードから出力 waggle トークン一覧を取得する。
fn get_waggle_tokens_for_execution(
    store: &agent_link_engine::db::SaiStore,
    exec_id: i64,
) -> Vec<String> {
    store
        .get_execution_by_id(exec_id)
        .ok()
        .and_then(|exec| {
            exec.output_waggle_tokens
                .as_deref()
                .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
        })
        .unwrap_or_default()
}

/// 実行レコードからセッション統計を取得する（Log 画面用・近似値）。
fn get_session_stats_for_execution(
    store: &agent_link_engine::db::SaiStore,
    exec_id: i64,
) -> Option<SessionStatsData> {
    let exec = store.get_execution_by_id(exec_id).ok()?;
    let json_str = exec.session_stats_json.as_deref()?;
    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;

    Some(SessionStatsData {
        turn_count: parsed.get("turn_count")?.as_u64().unwrap_or(0),
        tool_call_count: parsed.get("tool_call_count")?.as_u64().unwrap_or(0),
        input_tokens: parsed.get("input_tokens")?.as_u64().unwrap_or(0),
        output_tokens: parsed.get("output_tokens")?.as_u64().unwrap_or(0),
        thinking_tokens: parsed.get("thinking_tokens")?.as_u64().unwrap_or(0),
        total_tokens: parsed.get("total_tokens")?.as_u64().unwrap_or(0),
    })
}

/// JSON 値を表示用文字列に変換する（文字列は引用符なし）。
fn json_value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "-".to_string(),
        other => other.to_string(),
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の in-memory store（ブロック 1 件）を作成する。
    fn setup_store(block_name: &str) -> (agent_link_engine::db::SaiStore, i64) {
        let store = agent_link_engine::db::SaiStore::open_in_memory().unwrap();
        let block = agent_link_engine::db::models::BlockMetadata::new(
            block_name.to_string(),
            "テスト用ブロック".to_string(),
            "anthropic/claude-sonnet-4-5".to_string(),
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            format!("blocks/{}.md", block_name),
            None
        );
        let block_id = store.insert_block(&block).unwrap();
        (store, block_id)
    }

    /// 存在しない実行 ID では None を返すこと。
    #[test]
    fn build_returns_none_for_missing_execution() {
        let (store, _block_id) = setup_store("plan-ice");
        assert!(build(&store, 999).is_none());
    }

    /// 通常ブロック（接続 0 件・入力なし）の詳細が構築されること。
    #[test]
    fn build_ordinary_block_with_no_connections() {
        let (store, block_id) = setup_store("plan-ice");
        let exec_id = store.create_execution(block_id, None).unwrap();

        let d = build(&store, exec_id).expect("詳細が構築されること");
        assert_eq!(d.exec_id, exec_id);
        assert_eq!(d.block_name, "plan-ice");
        assert!(!d.is_user_block, "通常ブロックは user-block ではない");
        assert!(d.connections_from.is_empty(), "接続 0 件");
        assert!(d.connections_to.is_empty(), "接続 0 件");
        assert!(d.output_waggle_tokens.is_empty());
        assert!(d.input_fields.is_empty(), "入力フィールドなし");
        assert!(d.session_stats.is_none());
        // ブロック定義の既定値がフォールバックで入ること
        assert_eq!(d.model.as_deref(), Some("anthropic/claude-sonnet-4-5"));
        assert_eq!(d.thinking.as_deref(), Some("off"));
        assert_eq!(d.timeout, Some(300));
    }

    /// user-block は is_user_block = true となり、説明・入力ファイルが
    /// user_inputs_json から抽出されること。
    #[test]
    fn build_user_block_extracts_description_and_file() {
        let (store, block_id) = setup_store("user-block");
        let json = r#"{"description":"ユーザーの説明","file_path":"docs/memo.md"}"#;
        let exec_id = store.create_execution(block_id, Some(json)).unwrap();

        let d = build(&store, exec_id).unwrap();
        assert!(d.is_user_block, "user-block ブロック名なら true");
        assert_eq!(d.user_block_description.as_deref(), Some("ユーザーの説明"));
        assert_eq!(d.user_block_file.as_deref(), Some("docs/memo.md"));
    }

    /// 入力フィールド定義（block_inputs）がある場合、user_inputs_json の値が
    /// ラベル付きで input_fields に並ぶこと。定義にないキーは末尾に追加される。
    #[test]
    fn build_with_input_fields() {
        let (store, block_id) = setup_store("with-inputs");
        let inp = agent_link_engine::db::models::BlockInput::new(
            block_id,
            "question".to_string(),
            "text".to_string(),
            "質問".to_string(),
            None,
            true,
            None,
            None,
            None,
            None,
        );
        store.insert_inputs(block_id, &[inp]).unwrap();
        let json = r#"{"question":"確認テスト","extra":"追加項目"}"#;
        let exec_id = store.create_execution(block_id, Some(json)).unwrap();

        let d = build(&store, exec_id).unwrap();
        assert_eq!(d.block_name, "with-inputs");
        // 定義順（ラベル付き）で最初に並ぶ
        assert_eq!(
            d.input_fields[0],
            ("質問".to_string(), "確認テスト".to_string())
        );
        // 定義にないキーは末尾に追加される
        assert_eq!(
            d.input_fields[1],
            ("extra".to_string(), "追加項目".to_string())
        );
    }

}

/// Log の接続行のテスト。
#[cfg(test)]
mod conn_line_tests {
    use super::*;
    use agent_link_engine::db::models::BlockMetadata;

    fn test_block(store: &agent_link_engine::db::SaiStore, name: &str) -> i64 {
        let b = BlockMetadata::new(
            name.to_string(),
            String::new(),
            "m".to_string(),
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            format!("blocks/{}.md", name),
            None
        );
        store.insert_block(&b).unwrap()
    }

    /// waggle 接続は `waggle: <token> ID<id>` 形式になること。
    #[test]
    fn waggle_conn_line_format() {
        let store = agent_link_engine::db::SaiStore::open_in_memory().unwrap();
        let b1 = test_block(&store, "a");
        let b2 = test_block(&store, "b");
        let e1 = store.create_execution(b1, None).unwrap();
        let e2 = store.create_execution(b2, None).unwrap();
        store
            .insert_connection(&agent_link_engine::db::BlockConnection::new_waggle(
                e1,
                b1,
                e2,
                b2,
                "waggle:abc".to_string(),
            ))
            .unwrap();
        let d = build(&store, e2).unwrap();
        assert_eq!(d.connections_from.len(), 1);
        assert!(d.connections_from[0].starts_with("waggle:abc ID"));
        // phone 用の構造化版: waggle は task_text、相手は e1（block a）
        assert_eq!(d.connections.len(), 1);
        let c = &d.connections[0];
        assert_eq!((c.direction.as_str(), c.kind.as_str()), ("from", "task_text"));
        assert_eq!(c.run_id, e1);
        assert_eq!(c.block_name, "a");
        assert_eq!(c.waggle_token.as_deref(), Some("waggle:abc"));
        // 相手側から見ると同じ線が to になる
        let d1 = build(&store, e1).unwrap();
        assert_eq!(d1.connections[0].direction, "to");
    }

}

/// 費用表示行（B02）のテスト。
#[cfg(test)]
mod usage_lines_tests {
    use super::*;
    use agent_link_engine::usage::{UsageSummary, UsageTotal};

    fn total(input: u64, output: u64, cache: u64, cost: Option<f64>) -> UsageTotal {
        UsageTotal {
            input,
            output,
            cache,
            cost,
        }
    }

    /// 記録あり・費用あり・子なし → 1 行だけ。
    #[test]
    fn usage_lines_with_cost() {
        let own = total(120_000, 8_100, 300_000, Some(0.42));
        let s = UsageSummary {
            own: Some(own),
            children: Vec::new(),
            total: own,
            unknown_cost_runs: 0,
        };
        let lines = usage_lines(&s);
        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0],
            "Cost  $0.42 (input 120k, output 8.1k, cache 300k)"
        );
    }

    /// usage の記録が無い（own が None）→ backfill の案内。
    #[test]
    fn usage_lines_without_record() {
        let s = UsageSummary {
            own: None,
            children: Vec::new(),
            total: UsageTotal::default(),
            unknown_cost_runs: 0,
        };
        let lines = usage_lines(&s);
        assert_eq!(
            lines,
            vec!["Cost  not recorded (create it with agent-link usage backfill)".to_string()]
        );
    }

    /// 記録はあるが費用が不明（cost が None）→ 費用不明（0 と出さない）。
    #[test]
    fn usage_lines_unknown_cost() {
        let own = total(10_000, 0, 0, None);
        let s = UsageSummary {
            own: Some(own),
            children: Vec::new(),
            total: own,
            unknown_cost_runs: 1,
        };
        let lines = usage_lines(&s);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("Cost  cost unknown ("), "{}", lines[0]);
    }

    /// 子あり → 2 行目に「子を含めた合計」と費用不明件数。
    #[test]
    fn usage_lines_with_children() {
        let own = total(120_000, 8_100, 0, Some(0.42));
        let c1 = total(1_000, 0, 0, Some(0.93));
        let c2 = total(1_000, 0, 0, None);
        let mut sum = own;
        sum.add(&c1);
        sum.add(&c2);
        let s = UsageSummary {
            own: Some(own),
            children: vec![(2, Some(c1)), (3, Some(c2))],
            total: sum,
            unknown_cost_runs: 1,
        };
        let lines = usage_lines(&s);
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[1],
            "Total with children  $1.35 (2 children, 1 with unknown cost)"
        );
    }

}
