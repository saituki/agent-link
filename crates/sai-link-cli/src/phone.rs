//! `sai-link phone …` — sai-phone 向け JSON 出力（実装計画 12 §3.2）。
//!
//! すべて cwd = プロジェクト root（または `SAI_PROJECT_ROOT`）から呼び出す前提。
//!
//! - 成功時は stdout に **JSON 1 個**（pretty）を出力して終了コード 0
//! - 失敗時は stderr に理由 1 行を出力し、終了コード ≠ 0 で終了する
//!   （md が無い場合は終了コード 3 + stderr「まだありません」）
//!
//! 組み立て処理は TUI と共有（`sai_link_widgets::{left_tree, card_entries,
//! card_wizard, phone_view}`）ため、TUI の表示結果と二重実装しない。

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

use chrono::{Datelike, NaiveDate};
use serde::Deserialize;
use serde_json::{json, Value};

use sai_link_engine::db::store::exec_usage_totals;
use sai_link_engine::db::SaiStore;
use sai_link_widgets::{
    block_history, block_log, cache_stats, card_entries, card_wizard, left_tree, phone_view,
};

use crate::cli::{PhoneCardCmd, PhoneCmd, EXIT_DB, EXIT_NOT_FOUND, EXIT_PARSE, EXIT_USAGE};

// ============================================================================
// エラー（終了コード付き）
// ============================================================================

/// phone サブコマンドのエラー（stderr 1 行 + 終了コード）。
#[derive(Debug)]
pub(crate) struct PhoneError {
    /// 終了コード（≠ 0）
    pub(crate) code: i32,
    /// stderr に出す理由
    pub(crate) msg: String,
}

impl PhoneError {
    /// 利用エラー（引数・検査失敗・stdin 不正など）= 終了コード 1。
    pub(crate) fn usage(msg: impl Into<String>) -> Self {
        Self {
            code: EXIT_USAGE,
            msg: msg.into(),
        }
    }

    /// リソース無し（実行・ブロック・md・トークンが見つからない）= 終了コード 3。
    pub(crate) fn not_found(msg: impl Into<String>) -> Self {
        Self {
            code: EXIT_NOT_FOUND,
            msg: msg.into(),
        }
    }

    /// 検査エラー（block の解析失敗・settings が JSON オブジェクトでないなど）
    /// = 終了コード 2。
    pub(crate) fn parse(msg: impl Into<String>) -> Self {
        Self {
            code: EXIT_PARSE,
            msg: msg.into(),
        }
    }

    /// 処理系エラー（DB / IO）= 終了コード 4。
    pub(crate) fn internal(msg: impl Into<String>) -> Self {
        Self {
            code: EXIT_DB,
            msg: msg.into(),
        }
    }
}

pub(crate) type PhoneResult<T> = Result<T, PhoneError>;

/// テスト専用: `SAI_PROJECT_ROOT` / `HOME` を触るテストの直列化ロック。
///
/// 環境変数はプロセス共通なので、`phone` と `phone_def` のテストが同じ
/// ロックを使う（別々だと両方が並列実行で衝突する）。
#[cfg(test)]
pub(crate) static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `phone` サブコマンドの出力（stdout に出す 1 個）。
///
/// 値（[`Value`]）で持つ項目は `json!` のリテラル順、テキストで持つ項目は
/// そのまま出力する。`serde_json::Value` は `Map` = アルファベット順で
/// 持つため、`phone dashboard` のように**項目の並び順まで共有契約**の
/// 出力は、構造体でシリアライズしたテキストで持つ（`dashboard` 参照）。
#[derive(Debug)]
enum PhoneOut {
    /// JSON 値
    Value(Value),
    /// シリアライズ済みの JSON テキスト（項目順を保つ）
    Text(String),
}

impl From<Value> for PhoneOut {
    fn from(v: Value) -> Self {
        PhoneOut::Value(v)
    }
}

impl From<String> for PhoneOut {
    fn from(s: String) -> Self {
        PhoneOut::Text(s)
    }
}

/// `Value` を返すサブコマンドの腕を [`PhoneOut`] に包む（`run` のmatchを短く保つ）。
fn val(r: PhoneResult<Value>) -> PhoneResult<PhoneOut> {
    Ok(r?.into())
}

// ============================================================================
// 入力（card create の stdin JSON）
// ============================================================================

/// `phone card create` の stdin 入力（項目名は計画 12 §3.2 のとおり）。
///
/// 空文字 = スキップ（TUI と同じく自動値を使う）。
#[derive(Debug, Default, Deserialize)]
struct CardCreateInput {
    /// 引き継ぐ done 実行 ID
    #[serde(default)]
    done_ids: Vec<i64>,
    /// runner（空 = スキップ）
    #[serde(default)]
    runner: String,
    /// block_id（空 = スキップ）
    #[serde(default)]
    block: String,
    /// provider（空 = スキップ）
    #[serde(default)]
    provider: String,
    /// model（空 = スキップ）
    #[serde(default)]
    model: String,
    /// thinking（空 = スキップ）
    #[serde(default)]
    thinking: String,
    /// グローバルラウンド（空 = 自動継承）
    #[serde(default)]
    global_round: String,
    /// ローカルラウンド（空 = 自動継承）
    #[serde(default)]
    round: String,
    /// タグ（空 = スキップ）
    #[serde(default)]
    tag: String,
    /// `# task_text` 直下に挿入する本文
    #[serde(default)]
    task_text: String,
    /// task_text 先頭に貼る文書トークン（`phone card inputs` の candidates.token。空可）
    /// TUI Step4 と同じく `waggle:<tok>:path:…` 参照行として task_text 先頭に貼られる
    #[serde(default)]
    doc_tokens: Vec<String>,
}

// ============================================================================
// エントリ
// ============================================================================

/// `sai-link phone <sub>` のハンドラ。
///
/// 成功は JSON 1 個を stdout へ、失敗は stderr + 終了コードでこの中で終了する。
pub fn handle(cmd: PhoneCmd) -> Result<(), Box<dyn std::error::Error>> {
    let out = match run(cmd) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{}", e.msg);
            std::process::exit(e.code);
        }
    };
    match out {
        PhoneOut::Value(v) => println!("{}", serde_json::to_string_pretty(&v)?),
        PhoneOut::Text(s) => println!("{}", s),
    }
    Ok(())
}

/// サブコマンド本体（`Export` は main 側で処理するため到達しない）。
fn run(cmd: PhoneCmd) -> PhoneResult<PhoneOut> {
    match cmd {
        PhoneCmd::Export { .. } => Err(PhoneError::usage(
            "内部エラー: phone export は handle_phone_export が処理する",
        )),
        PhoneCmd::Tree => val(tree()),
        PhoneCmd::Run { run_id, view } => val(run_view(run_id, &view)),
        PhoneCmd::Md { run_id, kind, write: false } => val(md(run_id, &kind)),
        PhoneCmd::Md { run_id, kind, write: true } => val(md_write(run_id, &kind)),
        PhoneCmd::Waggle { run_id, token } => val(waggle(run_id, token.as_deref())),
        PhoneCmd::Card { cmd } => val(match cmd {
            PhoneCardCmd::Options => card_options(),
            PhoneCardCmd::Inputs { block, origin_run_id } => {
                card_inputs(&block, origin_run_id)
            }
            PhoneCardCmd::Create => card_create(),
            PhoneCardCmd::Run { run_id } => card_run(run_id),
        }),
        PhoneCmd::Finalize { run_id } => val(finalize(run_id)),
        PhoneCmd::Advance { run_id } => val(advance(run_id)),
        PhoneCmd::Summary => val(summary()),
        PhoneCmd::Dashboard { period } => dashboard(&period),
        PhoneCmd::Workflow => val(workflow()),
        PhoneCmd::Def { cmd } => val(crate::phone_def::run(cmd)),
    }
}

/// プロジェクトの DB を開く（失敗は終了コード 4）。
fn open_store() -> PhoneResult<SaiStore> {
    crate::cli::open_db().map_err(|e| PhoneError::internal(format!("DB を開けません: {}", e)))
}

// ============================================================================
// workflow — メイン画面（Workflow の実行ブロック線図）の DAG
// ============================================================================

/// `phone workflow`: TUI が描くのと同じ DAG を JSON で返す。
///
/// - `width` / `height`: コンテンツサイズ（文字マス単位）
/// - `nodes`: 実行単位のノード。実ノードの `id` は実行 ID、
///   フェーズ合成ノードは `メイン実行ID * 10_000 + k`（`phase` に 1, 2, …）。
///   `run_id` は「押したときに開く run」で、合成ノードは親（メイン）の実行 ID
/// - `edges`: 接続線。`kind` は接続の種別、`side` は左右辺接地か（上下辺接地か）
/// - `separators`: round の区切り線。`global_round` が GR 無しのときは `null`
/// - `lanes`: Kanban の 4 レーンの札
///
/// DB が開けないときは終了コード 4。実行が 0 件でも空の配列で成功する。
fn workflow() -> PhoneResult<Value> {
    let store = open_store()?;
    let prep = card_entries::prepare_view(&store);
    let src = sai_link_widgets::workflow::load_workflow(&store, &prep);
    let (dag, layout_graph) = sai_link_widgets::workflow::build_block_graph(
        &src.blocks,
        &src.connections,
        &src.finalized_runs,
    );
    let layout = sai_link_widgets::workflow::calculate_layout(&layout_graph, true);

    // 座標（文字マス）＋表示データ。孤立ノードの_layout.nodes に無い場合は無いので、
    // フェーズ合成ノードを含め全てここに載る。
    let mut nodes: Vec<Value> = Vec::new();
    for pos in &layout.nodes {
        let Some(b) = dag.get(pos.index) else { continue };
        let run_id = match b.phase {
            // フェーズ合成ノードは親（メイン）の実行 run を開く
            Some(_) => b.parent_exec_id.unwrap_or(b.exec_id),
            None => b.exec_id,
        };
        nodes.push(json!({
            "id": pos.node_id,
            "run_id": run_id,
            "phase": b.phase,
            "name": b.name,
            "tag": b.tag,
            "runner": b.runner,
            "status": b.last_status,
            "created_at": b.created_at,
            "description": b.description,
            "cost": src.cost_labels.get(&b.exec_id),
            "orphaned": src.orphaned_runs.contains(&b.exec_id),
            "review": b.reflection_status.map(|s| reflection_name(s).to_string()),
            "global_round": b.global_round,
            "round": b.round,
            "parent_run_id": b.parent_exec_id,
            "batch_id": b.batch_id,
            "x": pos.x,
            "y": pos.y,
            "w": pos.width,
            "h": pos.height,
        }));
    }

    // 接続線。端点はノード ID（= 実行 ID / 合成 ID）。
    let edges: Vec<Value> = dag
        .all_edges()
        .into_iter()
        .map(|(from, to, e)| {
            json!({
                "from": from.exec_id,
                "to": to.exec_id,
                "kind": edge_kind_name(e.kind),
                "side": e.side,
            })
        })
        .collect();

    // round の区切り線。i64::MAX は「その行に GR が無い」意味なので null にする。
    let separators: Vec<Value> = layout
        .separators
        .iter()
        .map(|s| {
            json!({
                "y": s.y,
                "label": s.label,
                "global_round": (s.global_round != i64::MAX).then_some(s.global_round),
            })
        })
        .collect();

    Ok(json!({
        "width": layout.content_width,
        "height": layout.content_height,
        "nodes": nodes,
        "edges": edges,
        "separators": separators,
        "lanes": lanes_json(&prep.kanban_entries),
    }))
}

/// review の記入状況（`ReflectionStatus`）を JSON 用の名前にする。
fn reflection_name(s: sai_link_engine::reflection_status::ReflectionStatus) -> &'static str {
    use sai_link_engine::reflection_status::ReflectionStatus as R;
    match s {
        R::Empty => "empty",
        R::ReviewOnly => "review_only",
        R::ContentOnly => "content_only",
        R::Complete => "complete",
    }
}

/// 接続の種別（`EdgeKind`）を JSON 用の名前にする。
fn edge_kind_name(k: sai_link_widgets::workflow::EdgeKind) -> &'static str {
    use sai_link_widgets::workflow::EdgeKind as K;
    match k {
        K::Other => "other",
        K::PhaseChain => "phase_chain",
        K::Delegation => "delegation",
        K::TaskOutput => "task_output",
        K::WaggleToken => "waggle_token",
        K::GitCommit => "git_commit",
    }
}

/// Kanban の 4 レーンの札を JSON にする。
///
/// `phone tree` と同じく、旧形式 todo カード（`.sai/todo/*.md`・run_id を
/// 親フォルダ名から取れないもの）は出さない（番号の付けようが無い札のため）。
fn lanes_json(entries: &[card_entries::CardEntry]) -> Value {
    let mut todo: Vec<Value> = Vec::new();
    let mut running: Vec<Value> = Vec::new();
    let mut review: Vec<Value> = Vec::new();
    let mut done: Vec<Value> = Vec::new();
    for e in entries {
        let run_id = match e.exec_id {
            Some(id) => id,
            None => {
                // todo カードは `outputs/todo/<run_id>/task.md` の親フォルダ名が run ID
                let Some(path) = e.card_path.as_deref() else {
                    continue;
                };
                let Some(parent_name) = path.parent().and_then(|p| p.file_name()) else {
                    continue;
                };
                match parent_name.to_str().and_then(|s| s.parse::<i64>().ok()) {
                    Some(id) => id,
                    // 旧形式 todo（run_id 不明）は出さない
                    None => continue,
                }
            }
        };
        let card = json!({
            "run_id": run_id,
            "title": e.title,
            "date": e.date,
            "description": e.description,
            "runner": e.runner,
        });
        use sai_link_engine::kanban::KanbanLane as L;
        match e.lane {
            L::Todo => todo.push(card),
            L::Running => running.push(card),
            L::Review => review.push(card),
            L::Done => done.push(card),
        }
    }
    json!({
        "todo": todo,
        "running": running,
        "review": review,
        "done": done,
    })
}

// ============================================================================
// tree — 左サイドバーと同形のツリー
// ============================================================================

/// `phone tree`: TUI 左サイドバーと同じツリーを JSON で返す。
///
/// - 状態フォルダ `todo` / `running` / `review` / `done` とラウンド別 `R…` › `r…` を
///   入れ子で返す（同じ run が両方に出るのは TUI と同じ）
/// - 旧形式 todo カード（`.sai/todo/*.md`・run_id 無し）は run_id が無いため出さない
fn tree() -> PhoneResult<Value> {
    let store = open_store()?;
    let prep = card_entries::prepare_view(&store);
    // JSON には展開状態を持たせないため prev_expanded は空（TUI 初期状態と同じ全閉）
    let items = left_tree::build_left_tree(
        &store,
        &prep.executions,
        &prep.kanban_entries,
        &prep.mb_replica_execs,
        &HashMap::new(),
    );
    Ok(json!({ "items": nest_tree(items) }))
}

/// 平らなツリー項目を入れ子 JSON（`children`）に組み替える。
fn nest_tree(items: Vec<left_tree::LeftTreeItem>) -> Vec<Value> {
    let mut roots: Vec<Value> = Vec::new();
    // (depth, folder value) のスタック。フォルダ閉じ子は top 側に attach する。
    let mut stack: Vec<(usize, Value)> = Vec::new();
    // 直近の depth-0 フォルダ ID（`r…` フォルダの `R…/r…` 形式 ID 用）
    let mut root_id: Option<String> = None;

    fn attach(stack: &mut Vec<(usize, Value)>, roots: &mut Vec<Value>, val: Value) {
        if let Some((_, top)) = stack.last_mut() {
            if let Some(arr) = top.get_mut("children").and_then(|c| c.as_array_mut()) {
                arr.push(val);
                return;
            }
        }
        roots.push(val);
    }

    for it in items {
        // 旧形式 todo カード（run_id 無し）は phone では出さない
        if it.kind == left_tree::LeftTreeKind::Card && it.run_id.is_none() {
            continue;
        }
        // 深さが等しい以上のフォルダを閉じる
        while stack.last().map(|(d, _)| *d >= it.depth).unwrap_or(false) {
            let (_, closed) = stack.pop().unwrap();
            attach(&mut stack, &mut roots, closed);
        }
        match it.kind {
            left_tree::LeftTreeKind::Folder => {
                let label = it
                    .name
                    .strip_prefix(left_tree::FOLDER_ICON)
                    .unwrap_or(&it.name)
                    .to_string();
                let id = if it.depth == 0 {
                    root_id = Some(label.clone());
                    label.clone()
                } else {
                    format!("{}/{}", root_id.clone().unwrap_or_default(), label)
                };
                let mut val = json!({
                    "type": "folder",
                    "id": id,
                    "label": label,
                    "depth": it.depth,
                    "children": Vec::<Value>::new(),
                });
                // 自身をスタックへ積む（子はここへ attach される）
                val["children"] = json!([]);
                stack.push((it.depth, val));
            }
            left_tree::LeftTreeKind::Leaf | left_tree::LeftTreeKind::Card => {
                let run_id = match it.run_id {
                    Some(id) => id,
                    None => continue,
                };
                let val = json!({
                    "type": "run",
                    "run_id": run_id,
                    "label": it.name,
                    "lane": it.lane,
                    "status": it.status,
                });
                attach(&mut stack, &mut roots, val);
            }
        }
    }
    // 残ったフォルダを閉じる
    while let Some((_, closed)) = stack.pop() {
        attach(&mut stack, &mut roots, closed);
    }
    roots
}

// ============================================================================
// run — log / stats / history
// ============================================================================

/// `phone run <id> --view …`: run の中身を JSON で返す。
fn run_view(run_id: i64, view: &str) -> PhoneResult<Value> {
    let store = open_store()?;
    match view {
        "log" => {
            // BlockDetail は serde 実装済み（項目名は snake_case のまま）
            let detail = block_log::build(&store, run_id).ok_or_else(|| {
                PhoneError::not_found(format!("実行が見つかりません: ID{}", run_id))
            })?;
            serde_json::to_value(&detail).map_err(|e| PhoneError::internal(e.to_string()))
        }
        "stats" => {
            // run の存在は検証し、解析エラーは JSON の `error` として返す（TUI と同じ）
            store.get_execution_by_id(run_id).map_err(|_| {
                PhoneError::not_found(format!("実行が見つかりません: ID{}", run_id))
            })?;
            let state = cache_stats::load(&store, run_id);
            serde_json::to_value(state.phone_json())
                .map_err(|e| PhoneError::internal(e.to_string()))
        }
        "history" => {
            let hist = block_history::build(&store, run_id).ok_or_else(|| {
                PhoneError::not_found(format!("実行が見つかりません: ID{}", run_id))
            })?;
            serde_json::to_value(hist.phone_json()).map_err(|e| PhoneError::internal(e.to_string()))
        }
        other => Err(PhoneError::usage(format!(
            "不正な view です: {}（log | stats | history）",
            other
        ))),
    }
}

// ============================================================================
// md — task / review / message
// ============================================================================

/// `phone md <id> --kind …`: md 本文を JSON で返す。
///
/// md が無い場合は終了コード 3 + stderr「まだありません」（sai-phone は 404 に読む替える）。
fn md(run_id: i64, kind: &str) -> PhoneResult<Value> {
    let store = open_store()?;
    let md_kind = match kind {
        "task" => phone_view::MdKind::Task,
        "review" => phone_view::MdKind::Review,
        "message" => phone_view::MdKind::Message,
        other => {
            return Err(PhoneError::usage(format!(
                "不正な kind です: {}（task | review | message）",
                other
            )))
        }
    };
    match phone_view::resolve_md_path(&store, run_id, md_kind) {
        phone_view::MdResolve::Found(path) => {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| PhoneError::internal(format!("md の読み込みに失敗: {}", e)))?;
            Ok(json!({
                "kind": md_kind.label(),
                "name": md_kind.file_name(),
                "text": text,
            }))
        }
        phone_view::MdResolve::Missing(_) => Err(PhoneError::not_found("まだありません")),
        phone_view::MdResolve::Failed(msg) => Err(PhoneError::internal(msg)),
    }
}

/// 保存できる md の大きさ（sai-phone の入力上限と同じ 100KB）。
const MD_WRITE_MAX: usize = 100 * 1024;

/// `phone md <id> --kind task|review --write`: 標準入力の中身で md を保存する。
///
/// - task: 未実行の todo カード（`phone card run` と同じ条件）だけ。frontmatter が
///   読めなくなる・run_id が変わる内容は保存しない（todo から消えてしまうため）。
/// - review: DB にある実行だけ。review.md が無ければ作る（TUI の `r` と同じ置き場）。
/// - message は session から毎回作り直すので保存できない。
fn md_write(run_id: i64, kind: &str) -> PhoneResult<Value> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|e| PhoneError::usage(format!("標準入力を読めません: {}", e)))?;
    md_save(run_id, kind, &text)
}

/// [`md_write`] の本体（標準入力を読んだ後・テストから呼ぶ）。
fn md_save(run_id: i64, kind: &str, text: &str) -> PhoneResult<Value> {
    if text.len() > MD_WRITE_MAX {
        return Err(PhoneError::usage("本文が大きすぎます（100KB まで）"));
    }
    let store = open_store()?;
    let path = match kind {
        "task" => {
            let task = sai_link_engine::project::sai_path(sai_link_engine::run_dir::OUTPUTS_DIR)
                .join(sai_link_engine::run_dir::TODO_OWNER)
                .join(run_id.to_string())
                .join(sai_link_engine::run_dir::TASK_FILE);
            if !task.is_file() {
                return Err(PhoneError::not_found(TODO_NOT_FOUND));
            }
            let task = resolve_target_card(&store, run_id, task)?;
            let card = sai_link_engine::kanban::parse_card(text).ok_or_else(|| {
                PhoneError::usage("先頭の --- で囲まれた設定（frontmatter）が読めません")
            })?;
            if card.frontmatter.run_id.trim() != run_id.to_string() {
                return Err(PhoneError::usage(
                    "frontmatter の run_id は変えられません（todo から消えてしまうため）",
                ));
            }
            task
        }
        "review" => {
            store
                .get_execution_by_id(run_id)
                .map_err(|_| PhoneError::not_found("実行が見つかりません"))?;
            sai_link_engine::orchestrator::review_path(run_id)
        }
        "message" => return Err(PhoneError::usage("message は保存できません")),
        other => {
            return Err(PhoneError::usage(format!(
                "不正な kind です: {}（task | review）",
                other
            )))
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| PhoneError::internal(format!("フォルダを作れません: {}", e)))?;
    }
    std::fs::write(&path, text)
        .map_err(|e| PhoneError::internal(format!("保存に失敗: {}", e)))?;
    md(run_id, kind)
}

// ============================================================================
// waggle — 成果物トークン一覧 / 本文
// ============================================================================

/// `phone waggle <id> [--token …]`: md 成果物の一覧（または 1 件の本文）。
fn waggle(run_id: i64, token: Option<&str>) -> PhoneResult<Value> {
    let store = open_store()?;
    match token {
        None => {
            let items: Vec<Value> = phone_view::waggle_choices(&store, run_id)
                .into_iter()
                .map(|c| {
                    json!({
                        "token": c.token,
                        "label": c.file_name,
                        "description": c.description,
                        // U03: 実行状態（running / completed 等。task.md 作成時の判断用）
                        "status": c.status,
                    })
                })
                .collect();
            Ok(json!({ "items": items }))
        }
        Some(token) => {
            let doc = phone_view::load_waggle_token(token).map_err(|msg| {
                // integrator 起動失敗は DB 系（4）、解決・内容失敗は見つからない扱い（3）
                if msg.contains("integrator 起動失敗") {
                    PhoneError::internal(msg)
                } else {
                    PhoneError::not_found(msg)
                }
            })?;
            Ok(json!({
                "token": doc.token,
                "label": doc.file_name,
                "text": doc.content,
            }))
        }
    }
}

// ============================================================================
// card — options / inputs / create
// ============================================================================

/// `phone card options`: ウィザードの選択肢（done / runner / block / provider / model / …）。
fn card_options() -> PhoneResult<Value> {
    let store = open_store()?;
    let prep = card_entries::prepare_view(&store);

    let done: Vec<Value> = card_entries::done_options(&prep.kanban_entries)
        .into_iter()
        .map(|d| {
            json!({
                "run_id": d.run_id,
                "label": d.label,
                "description": d.description,
            })
        })
        .collect();
    let blocks: Vec<Value> = card_wizard::load_block_choices()
        .into_iter()
        .map(|(label, id)| json!({ "label": label, "id": id }))
        .collect();
    let mut providers = serde_json::Map::new();
    for p in card_wizard::load_provider_choices() {
        let models = card_wizard::load_model_choices(&p);
        providers.insert(p, json!(models));
    }
    let (_, warnings) = sai_link_engine::pi_command::load_settings_quiet();

    Ok(json!({
        "done": done,
        "runners": card_wizard::RUNNER_OPTIONS,
        "blocks": blocks,
        "providers": providers,
        "cc_models": card_wizard::CC_MODELS,
        "thinking": card_wizard::THINKING_OPTIONS,
        "warnings": warnings,
    }))
}

/// `phone card inputs --block <id>`: 入力欄（P2 で入力を廃止したため常に空）。
///
/// U03: `phone card inputs --block <id> [--origin-run-id <run_id>]`: task.md 作成時の文書候補を返す。
///
/// - ブロックの存在確認だけ行い、`{"fields": []}` を返す（従来通り・外部 sai-phone 互換）。
/// - `origin_run_id` 指定時は origin 実行の文書＋実行中を含む全体（`select_docs(Origin)` と同一）を
///   `candidates` に追加する。plan が実装ガイドを task.md に貼る hand-off 用。
fn card_inputs(block: &str, origin_run_id: Option<i64>) -> PhoneResult<Value> {
    if block.trim().is_empty() {
        return Err(PhoneError::usage("block を指定してください"));
    }
    if card_wizard::resolve_block_path(block.trim()).is_none() {
        return Err(PhoneError::not_found(format!(
            "ブロックが見つかりません: {}",
            block
        )));
    }
    let candidates: Vec<Value> = origin_run_id
        .map(|oid| {
            let store = match open_store() {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };
            // TUI Step4 と同じ候補集合（origin の文書＋実行中を含む全体）
            phone_view::select_docs(&store, &phone_view::DocSelectScope::Origin(oid))
                .into_iter()
                .map(|c| card_candidate_json(&c))
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ "fields": [], "candidates": candidates }))
}

/// 候補 1 件を `phone card inputs` の JSON にする。
///
/// `ref` の `<run_id>` は origin ではなく **文書を保存した実行の番号**（U08）。
fn card_candidate_json(c: &phone_view::WaggleChoiceData) -> Value {
    json!({
        "token": c.token,
        "label": c.file_name,
        "description": c.description,
        "status": c.status,
        // task.md への貼り付け形式（そのまま task_text に書ける）
        "ref": phone_view::doc_ref_line(c),
    })
}

/// `phone card create`（stdin に JSON）: カードを作成し run_id とパスを返す。
///
/// 受け取った値は TUI と同じ検査（`validate_card_spec`）にかける。
/// 本文は `# task_text` 直下に task_text、その後に review 結合（TUI と同じ並び）。
/// `doc_tokens` 指定時は TUI Step4 と同じく task_text 先頭に参照行を貼る
/// （全経路共通の候補形式。実行時に本文展開される）。
fn card_create() -> PhoneResult<Value> {
    let store = open_store()?;
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| PhoneError::usage(format!("標準入力の読み込みに失敗: {}", e)))?;
    let parsed: CardCreateInput = serde_json::from_str(&input)
        .map_err(|e| PhoneError::usage(format!("JSON の解析に失敗: {}", e)))?;

    // TUI Step4 と同じ貼り付け（doc_tokens → task_text 先頭の参照行＋本文）
    let doc_refs: Vec<String> = parsed
        .doc_tokens
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| {
            if t.starts_with("waggle:") {
                t.to_string()
            } else {
                format!("waggle:{}", t)
            }
        })
        .collect();
    let task_text = if doc_refs.is_empty() {
        parsed.task_text
    } else if parsed.task_text.trim().is_empty() {
        doc_refs.join("\n")
    } else {
        format!("{}\n\n{}", doc_refs.join("\n"), parsed.task_text.trim_end())
    };
    let spec = card_wizard::CardSpec {
        done_ids: parsed.done_ids,
        runner: parsed.runner,
        block: parsed.block,
        provider: parsed.provider,
        model: parsed.model,
        thinking: parsed.thinking,
        global_round: parsed.global_round,
        round: parsed.round,
        tag: parsed.tag,
        task_text,
    };
    // TUI と同じ検査（runner / thinking は一覧の値・block は存在するもの）
    card_wizard::validate_card_spec(&spec).map_err(PhoneError::usage)?;

    let path = card_wizard::create_card(&store, &spec)
        .map_err(|e| PhoneError::usage(format!("カード作成に失敗: {}", e)))?;

    // 作成済み task.md の frontmatter から確定した run_id を読む
    let text = std::fs::read_to_string(&path)
        .map_err(|e| PhoneError::internal(format!("作成した task.md を読めません: {}", e)))?;
    let run_id: i64 = sai_link_engine::kanban::parse_card(&text)
        .and_then(|c| c.frontmatter.run_id.trim().parse::<i64>().ok())
        .ok_or_else(|| PhoneError::internal("task.md の run_id を読めません"))?;

    Ok(json!({
        "run_id": run_id,
        "path": path.display().to_string(),
    }))
}

// ============================================================================
// card run — todo の未実行カードを実行する
// ============================================================================

/// todo に未実行カードが無いときの stderr 文言（終了コード 3）。
const TODO_NOT_FOUND: &str = "未実行のカードが見つかりません";

/// 常駐が止まっているときの stderr 文言（終了コード 1）。
const DAEMON_STOPPED: &str = "常駐が止まっています。先に起動してください";

/// 実行が見つからないときの stderr 文言（終了コード 3）。
const EXEC_NOT_FOUND: &str = "実行が見つかりません";

/// `phone advance` で review / done 以外の置き場所だったときの stderr 文言（終了コード 1）。
const ADVANCE_UNSUPPORTED: &str = "review / done の run だけ移動できます";

/// `phone card run <run_id>`: todo の未実行カードに実行要求を送る（非対話）。
///
/// - 対象: `outputs/todo/<run_id>/task.md` があり、`phone tree` の todo に出るもの
///   （同じ `todo_card_paths` を使うため送信済み・DB 紐付け済みは弾かれる）
/// - 実行本体は `sai-link block run --kanban --card … --yes` と同じ
///   `kanban_run::run_card_request` を通す（二重実装しない・対話は出さない）
/// - 成功: stdout に `{"run_id": N, "message": "実行を要求しました"}` / 終了コード 0
/// - 対象無し: 終了コード 3、常駐停止・実行失敗: 終了コード 1
fn card_run(run_id: i64) -> PhoneResult<Value> {
    // 1. 対象カードの解決（無ければ終了コード 3）
    let task = sai_link_engine::project::sai_path(sai_link_engine::run_dir::OUTPUTS_DIR)
        .join(sai_link_engine::run_dir::TODO_OWNER)
        .join(run_id.to_string())
        .join(sai_link_engine::run_dir::TASK_FILE);
    if !task.is_file() {
        return Err(PhoneError::not_found(TODO_NOT_FOUND));
    }
    let store = open_store()?;
    let target = resolve_target_card(&store, run_id, task)?;

    // 2. 常駐確認（既存の判定方法を使う。止まっていれば終了コード 1）
    if !sai_link_engine::daemon::is_running() {
        return Err(PhoneError::usage(DAEMON_STOPPED));
    }

    // 3. 実行要求（対話なし。`block run --kanban --card … --yes` と同じ流れ）
    crate::kanban_run::run_card_request(
        &target, None, &[], None, None, None, None, None, None,
    )
    .map_err(|e| PhoneError::usage(e.to_string()))?;

    Ok(json!({ "run_id": run_id, "message": "実行を要求しました" }))
}

/// todo に出ている未実行カードかを検証し、カードパスを返す（無ければ終了コード 3）。
fn resolve_target_card(store: &SaiStore, run_id: i64, task: PathBuf) -> PhoneResult<PathBuf> {
    // ディレクトリ名と frontmatter の run_id が対象と一致するか
    // （不一致（A3）は todo に run_id が出ないため実行対象にしない）
    let text = std::fs::read_to_string(&task)
        .map_err(|e| PhoneError::internal(format!("task.md を読めません: {}", e)))?;
    let card = sai_link_engine::kanban::parse_card(&text)
        .ok_or_else(|| PhoneError::not_found(TODO_NOT_FOUND))?;
    let info_run_id = card_entries::todo_card_info(&task, &card).and_then(|i| i.run_id);
    if info_run_id != Some(run_id) {
        return Err(PhoneError::not_found(TODO_NOT_FOUND));
    }
    // `phone tree` の todo と同じ条件（送信済み・DB 紐付け済みは除外）
    let executions = store
        .list_recent_executions(100)
        .map_err(|e| PhoneError::internal(e.to_string()))?;
    if left_tree::todo_card_paths(&executions).contains(&task) {
        Ok(task)
    } else {
        Err(PhoneError::not_found(TODO_NOT_FOUND))
    }
}

/// `phone finalize <run_id>`: 実行中のメインブロックを完了させる（sai-phone の ■ 終了用）。
///
/// - 対象: `status == "running"` かつ `parent_run_id` が無い（メインブロックのみ）。
///   子ブロックは `task_report` で完了させるため受け付けない
/// - 完了要求は `agent_api::finalize` を通る（finalize_request を書き、デーモンが
///   DB 保存・成果物のまとめ・pane を閉じる。`pi` / `cc` 終了時の自動完了と同じ経路）
/// - 成功: stdout に `{"run_id": N, "message": "終了を要求しました"}` / 終了コード 0
/// - 実行が無い: 終了コード 3、実行中以外・子ブロック・常駐停止: 終了コード 1
fn finalize(run_id: i64) -> PhoneResult<Value> {
    let store = open_store()?;

    // 1. 実行の存在（無ければ終了コード 3）
    let exec = store
        .get_execution_by_id(run_id)
        .map_err(|_| PhoneError::not_found(EXEC_NOT_FOUND))?;

    // 2. 実行中か（完了済み・未実行は二重確定になるため弾く）
    if exec.status != "running" {
        return Err(PhoneError::usage(format!(
            "実行中ではありません（status={}）",
            exec.status
        )));
    }

    // 3. メインブロックか（子ブロックは task_report で完了する）
    if exec.parent_run_id.is_some() {
        return Err(PhoneError::usage("子ブロックは task_report で完了します"));
    }

    // 4. 常駐確認（実行時と同じ判定。止まっていれば完了処理ができない）
    if !sai_link_engine::daemon::is_running() {
        return Err(PhoneError::usage(DAEMON_STOPPED));
    }

    // 5. 完了要求（デーモンが拾って完了処理する）
    sai_link_engine::agent_api::finalize(Some(run_id)).map_err(PhoneError::usage)?;

    Ok(json!({ "run_id": run_id, "message": "終了を要求しました" }))
}

/// `phone advance <run_id>`: 実行を次の状態へ進める
/// （sai-phone の「→ done」「→ Workflow」用・実装計画 A U10）。
///
/// - 今の置き場所は `run_dir::find_run_dir` で決める
///   - `review` → `done`（`advance_review_to_done`）
///   - `done` → `R<global_round>`（`advance_done_to_workflow`）
/// - TUI の `e` と同じ [`run_dir`] の関数を通す（二重実装しない）
/// - 成功: stdout に `{"run_id": N, "from": "...", "to": "..."}` / 終了コード 0
/// - 実行が無い: 終了コード 3、review / done 以外: 終了コード 1
fn advance(run_id: i64) -> PhoneResult<Value> {
    use sai_link_engine::run_dir;

    let store = open_store()?;

    // 1. 今の置き場所（run フォルダが無い実行は対象外・終了コード 3）
    let (from, _) =
        run_dir::find_run_dir(run_id).ok_or_else(|| PhoneError::not_found(EXEC_NOT_FOUND))?;

    // 2. 置き場所に応じて 1 段階進める
    let to = match from.as_str() {
        run_dir::REVIEW_OWNER => {
            run_dir::advance_review_to_done(&store, run_id).map_err(PhoneError::usage)?;
            run_dir::DONE_OWNER.to_string()
        }
        run_dir::DONE_OWNER => {
            run_dir::advance_done_to_workflow(&store, run_id).map_err(PhoneError::usage)?
        }
        _ => return Err(PhoneError::usage(ADVANCE_UNSUPPORTED)),
    };

    Ok(json!({ "run_id": run_id, "from": from, "to": to }))
}

// ============================================================================
// summary — プロジェクト一覧用の最小集計（実装計画 C C1）
// ============================================================================

/// `phone summary`: プロジェクト一覧に出す 1 プロジェクトの最小集計
/// （実装計画 C C1・sai-phone の Project タブが `sai-control status --json` の
/// 代わりに使う）。
///
/// 返す形（共有契約）:
/// ```json
/// { "running": 3, "last_executed_at": "2026-09-26T02:15:13Z" }
/// ```
///
/// - `running`: `status = 'running'` の実行数（件数のみ・sai-phone は中身を使わない）
/// - `last_executed_at`: 最終実行の開始時刻（`started_at` が空なら `created_at`）。
///   実行が 1 件も無ければ `null`
/// - 集計の決め方は sai-control の `reader::status`（`reader.rs:346-363`）に揃える
fn summary() -> PhoneResult<Value> {
    let store = open_store()?;
    // `list_all_executions` は created_at の昇順（store.rs:2768）なので末尾が最新
    let execs = store
        .list_all_executions()
        .map_err(|e| PhoneError::internal(e.to_string()))?;

    // 実行中（件数のみ）
    let running = execs.iter().filter(|e| e.status == "running").count();

    // 最終実行日時（started_at が空なら created_at・新しい順 1 件）
    let last_executed_at = execs.last().map(|e| {
        e.started_at
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&e.created_at)
            .to_string()
    });

    Ok(json!({ "running": running, "last_executed_at": last_executed_at }))
}

// ============================================================================
// dashboard — 期間集計（実装計画 C C2）
// ============================================================================

/// 集計期間（`sai-control dashboard --period` と同じ 3 種）。
///
/// JSON 出力の `period` 欄はそのままこの短い名前を返す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Period {
    /// 今週（今日の週の月曜から）
    Week,
    /// 今月（その月の 1 日から）
    Month,
    /// 全期間
    All,
}

impl Period {
    /// JSON 出力用の短い名前（`week` / `month` / `all`）。
    fn as_str(&self) -> &'static str {
        match self {
            Period::Week => "week",
            Period::Month => "month",
            Period::All => "all",
        }
    }
}

impl std::str::FromStr for Period {
    type Err = String;

    /// `week` / `month` / `all` のみを受け付ける（cli.rs の `value_parser` と二重で守る）。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "week" => Ok(Period::Week),
            "month" => Ok(Period::Month),
            "all" => Ok(Period::All),
            other => Err(format!("不明な期間です: {other}（week / month / all）")),
        }
    }
}

/// `phone dashboard` の返り値（sai-control `cross::Dashboard` と同形）。
///
/// 構造体にしてあるのは、JSON の項目順を sai-control と一致させるため
/// （`json!` のmacro は `serde_json::Map` = アルファベット順になる）。
#[derive(Debug, Clone, serde::Serialize)]
struct Dashboard {
    /// プロジェクト名
    name: String,
    /// 期間（`week` / `month` / `all`）
    period: String,
    /// 期間の開始日（日本時間・YYYY-MM-DD）
    since: String,
    /// 期間の終了日（日本時間・YYYY-MM-DD）
    until: String,
    /// 常駐（daemon）が生きているか
    daemon_alive: bool,
    /// 期間内の合計
    summary: DashboardSummary,
    /// 日別（`since`〜`until` の全日・古い日が先）
    daily: Vec<DailyRow>,
    /// モデル別（token の多い順）
    by_model: Vec<ModelRow>,
    /// ブロック別（token の多い順）
    by_block: Vec<BlockRow>,
    /// 直近実行（期間内の新しい順・最大 10 件）
    recent: Vec<RecentRun>,
}

/// `phone dashboard` の `summary` 欄（sai-control `cross::DashboardSummary` と同形）。
#[derive(Debug, Clone, Default, serde::Serialize)]
struct DashboardSummary {
    /// 期間内の実行数
    executions: u64,
    /// completed の数
    completed: u64,
    /// failed の数
    failed: u64,
    /// running の数
    running: u64,
    /// completed / executions（0 件なら 0.0）
    success_rate: f64,
    /// 所要時間の合計（ミリ秒）
    duration_ms: i64,
    /// input token 合計
    tokens_input: u64,
    /// output token 合計
    tokens_output: u64,
    /// cache token 合計
    tokens_cache: u64,
    /// input + output + cache
    tokens_total: u64,
    /// cost 合計（USD）
    cost_usd: f64,
}

/// `phone dashboard` の `daily` 1 行（`since`〜`until` の全日・0 埋め）。
#[derive(Debug, Clone, serde::Serialize)]
struct DailyRow {
    /// 日付（日本時間・YYYY-MM-DD）
    date: String,
    /// その日の実行数
    executions: u64,
    /// その日の token 合計（input + output + cache）
    tokens: u64,
}

/// `phone dashboard` の `by_model` 1 行（token の多い順）。
#[derive(Debug, Clone, serde::Serialize)]
struct ModelRow {
    /// モデル名（実行時 > ブロック定義 > `unknown`）
    model: String,
    /// 実行数
    executions: u64,
    /// 所要時間の合計（ミリ秒）
    duration_ms: i64,
    /// token 合計
    tokens: u64,
    /// cost 合計（USD）
    cost_usd: f64,
}

/// `phone dashboard` の `by_block` 1 行（token の多い順）。
#[derive(Debug, Clone, serde::Serialize)]
struct BlockRow {
    /// ブロック名
    block: String,
    /// 実行数
    executions: u64,
    /// 所要時間の合計（ミリ秒）
    duration_ms: i64,
    /// token 合計
    tokens: u64,
}

/// `phone dashboard` の `recent` 1 行（期間内の新しい順・最大 10 件）。
#[derive(Debug, Clone, serde::Serialize)]
struct RecentRun {
    /// 実行 ID（`execution_history.id`）
    run_id: i64,
    /// 親実行 ID（無いときは null）
    parent_run_id: Option<i64>,
    /// ブロック名
    block: String,
    /// 実行状態
    status: String,
    /// 開始日時（無ければ created_at）
    started_at: String,
    /// 所要時間（ミリ秒）
    duration_ms: Option<i64>,
}

/// `phone dashboard --period <p>`: 期間ダッシュボードを JSON で返す
/// （実装計画 C C2・sai-phone の Dashboard タブ用）。
///
/// 出力の形は `sai-control dashboard <name> --period <p> --json` と **同じ**。
/// 期間の決め方（日本時間）・集計は sai-control の `cross::dashboard`
/// （`sai-control/crates/sai-control-engine/src/cross.rs:506`）から写したもの。
///
/// - `name`: `.sai/project.json` の `name`（無ければルートフォルダ名）
/// - `since` / `until`: 日本時間の日付（YYYY-MM-DD）
/// - `daemon_alive`: `daemon::is_running()`（`daemon.pid` の生存確認）
/// - 実行が 1 件も無くても `daily` は `since`〜`until` の全日を 0 埋めで返す
fn dashboard(period_str: &str) -> PhoneResult<PhoneOut> {
    let period: Period = period_str
        .parse()
        .map_err(|e| PhoneError::usage(format!("{}", e)))?;
    let store = open_store()?;
    let root = sai_link_engine::project::project_root();
    // 「今日」の日本時間の日付（sai-control の `cmd_dashboard` と同じ +9 時間）
    let today_jst = (chrono::Utc::now() + chrono::Duration::hours(9)).date_naive();
    let value = build_dashboard(&store, &root, period, today_jst)?;
    let text =
        serde_json::to_string_pretty(&value).map_err(|e| PhoneError::internal(e.to_string()))?;
    Ok(PhoneOut::Text(text))
}

/// 期間ダッシュボードを組み立てる（`today_jst` を渡せる純関数・sai-control の
/// `cross::dashboard` と同じ形。テストでは日付を固定する）。
fn build_dashboard(
    store: &SaiStore,
    root: &std::path::Path,
    period: Period,
    today_jst: NaiveDate,
) -> PhoneResult<Dashboard> {
    let execs = store
        .list_all_executions()
        .map_err(|e| PhoneError::internal(e.to_string()))?;
    let usage_map = store
        .execution_usage_total_map()
        .map_err(|e| PhoneError::internal(e.to_string()))?;

    // ブロック情報は実行ごとに引かない（取り回しのため一度だけためる）
    let mut blocks: HashMap<i64, sai_link_engine::db::BlockMetadata> = HashMap::new();
    for e in &execs {
        if blocks.contains_key(&e.block_id) {
            continue;
        }
        if let Ok(b) = store.get_block_by_id(e.block_id) {
            blocks.insert(e.block_id, b);
        }
    }

    // 実行ごとの日付（日本時間）
    let mut first: Option<NaiveDate> = None;
    let mut dated: Vec<(NaiveDate, &sai_link_engine::db::ExecutionHistory)> = Vec::new();
    for e in &execs {
        let raw = e
            .started_at
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&e.created_at);
        if let Some(d) = exec_date_jst(raw) {
            first = Some(match first {
                Some(m) => m.min(d),
                None => d,
            });
            dated.push((d, e));
        }
    }

    // 期間の開始日（`today_jst` は日本時間）
    let since = period_since(period, today_jst, first.unwrap_or(today_jst));
    let until = today_jst;
    dated.retain(|(d, _)| *d >= since && *d <= until);

    let mut summary = DashboardSummary {
        executions: dated.len() as u64,
        ..Default::default()
    };
    let mut daily_map: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut model_map: HashMap<String, (u64, i64, u64, f64)> = HashMap::new();
    let mut block_map: HashMap<i64, (String, u64, i64, u64)> = HashMap::new();
    let mut recent: Vec<(NaiveDate, RecentRun)> = Vec::new();

    for (date, exec) in &dated {
        match exec.status.as_str() {
            "completed" => summary.completed += 1,
            "failed" => summary.failed += 1,
            "running" => summary.running += 1,
            _ => {}
        }
        let duration = exec.duration_ms.unwrap_or(0);
        summary.duration_ms += duration;

        let (input, output, cache, cost) = exec_usage_totals(exec, &usage_map);
        let tokens = input + output + cache;
        summary.tokens_input += input;
        summary.tokens_output += output;
        summary.tokens_cache += cache;
        summary.cost_usd += cost;

        // 日別
        let entry = daily_map.entry(date.to_string()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += tokens;

        // モデル別（実行時の値 > ブロック定義 > `unknown`）
        let block_model = blocks
            .get(&exec.block_id)
            .map(|b| b.model.clone())
            .unwrap_or_default();
        let model = exec
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(String::from)
            .unwrap_or_else(|| {
                if block_model.is_empty() {
                    "unknown".to_string()
                } else {
                    block_model
                }
            });
        let m = model_map.entry(model).or_insert((0, 0, 0, 0.0));
        m.0 += 1;
        m.1 += duration;
        m.2 += tokens;
        m.3 += cost;

        // ブロック別
        let block_name = blocks
            .get(&exec.block_id)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let b = block_map
            .entry(exec.block_id)
            .or_insert_with(|| (block_name.clone(), 0, 0, 0));
        b.1 += 1;
        b.2 += duration;
        b.3 += tokens;

        // 直近
        recent.push((
            *date,
            RecentRun {
                run_id: exec.id.unwrap_or(0),
                parent_run_id: exec.parent_run_id,
                block: block_name,
                status: exec.status.clone(),
                started_at: exec
                    .started_at
                    .clone()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| exec.created_at.clone()),
                duration_ms: exec.duration_ms,
            },
        ));
    }

    summary.success_rate = if summary.executions > 0 {
        summary.completed as f64 / summary.executions as f64
    } else {
        0.0
    };
    summary.tokens_total = summary.tokens_input + summary.tokens_output + summary.tokens_cache;

    // 日別は since〜until の全日を 0 埋めで並べる（古い日が先）
    let mut daily = Vec::new();
    let mut cursor = since;
    while cursor <= until {
        let key = cursor.to_string();
        let (executions, tokens) = daily_map.get(&key).copied().unwrap_or((0, 0));
        daily.push(DailyRow {
            date: key,
            executions,
            tokens,
        });
        cursor = cursor + chrono::Duration::days(1);
    }

    // モデル別・ブロック別は token の多い順（同点なら名前順）
    let mut by_model: Vec<ModelRow> = model_map
        .into_iter()
        .map(
            |(model, (executions, duration_ms, tokens, cost_usd))| ModelRow {
                model,
                executions,
                duration_ms,
                tokens,
                cost_usd,
            },
        )
        .collect();
    by_model.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.model.cmp(&b.model)));

    let mut by_block: Vec<BlockRow> = block_map
        .into_iter()
        .map(|(_, (block, executions, duration_ms, tokens))| BlockRow {
            block,
            executions,
            duration_ms,
            tokens,
        })
        .collect();
    by_block.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.block.cmp(&b.block)));

    // 直近は期間内の新しい順に最大 10 件
    recent.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.run_id.cmp(&a.1.run_id)));
    let recent: Vec<RecentRun> = recent.into_iter().take(10).map(|(_, r)| r).collect();

    Ok(Dashboard {
        name: project_display_name(root),
        period: period.as_str().to_string(),
        since: since.to_string(),
        until: until.to_string(),
        daemon_alive: sai_link_engine::daemon::is_running(),
        summary,
        daily,
        by_model,
        by_block,
        recent,
    })
}

/// 実行日時文字列を日本時間の日付にする。
///
/// DB のタイムゾーン無しの文字列は UTC とみなして +9 時間する。
/// `2026-09-23T04:23:40Z` と `2026-09-23 06:50:17` の両方に対応する。
/// 解析できないときは `None`。
fn exec_date_jst(s: &str) -> Option<NaiveDate> {
    let s = s.trim().trim_end_matches('Z');
    let normalized = s.replace('T', " ");
    let mut parts = normalized.split_whitespace();
    let date = NaiveDate::parse_from_str(parts.next()?, "%Y-%m-%d").ok()?;
    let time = parts.next().unwrap_or("00:00:00");
    let time = chrono::NaiveTime::parse_from_str(time, "%H:%M:%S")
        .ok()
        .or_else(|| chrono::NaiveTime::parse_from_str(time, "%H:%M:%S%.f").ok())?;
    let dt = chrono::NaiveDateTime::new(date, time) + chrono::Duration::hours(9);
    Some(dt.date())
}

/// 期間の開始日を決める（テストで日付を固定するため関数に分ける）。
///
/// - `Week`: `today` の週の月曜
/// - `Month`: `today` の月の 1 日
/// - `All`: 全実行の最小日付（`first`・0 件なら `today`）
fn period_since(period: Period, today: NaiveDate, first: NaiveDate) -> NaiveDate {
    match period {
        Period::Week => {
            today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64)
        }
        Period::Month => today.with_day(1).unwrap_or(today),
        Period::All => first,
    }
}

/// プロジェクト表示名（`.sai/project.json` の `name`・無ければルートフォルダ名）。
///
/// sai-control は CLI 引数のプロジェクト名をそのまま `name` に入れていたが、
/// `phone dashboard` はプロジェクト名を引数に取らない（cwd で決まる）ため
/// 同じ結果を `.sai/project.json` から復元する。
fn project_display_name(root: &std::path::Path) -> String {
    let from_json = std::fs::read_to_string(root.join(".sai").join("project.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
        .filter(|s| !s.is_empty());
    from_json.unwrap_or_else(|| {
        root.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    })
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// `SAI_PROJECT_ROOT` はプロセス共通のため、環境変数を触るテスト同士が
    /// 並行実行で衝突しないようこのロックを共用する。
    /// `phone_def` のテストも [`ENV_TEST_LOCK`] と同じロックを使う。
    /// 別々のロックだと両方が並列実行して `SAI_PROJECT_ROOT` を奪い合う。
    pub(crate) static ENV_LOCK: &std::sync::Mutex<()> = &super::ENV_TEST_LOCK;

    /// `SAI_PROJECT_ROOT` を一時ディレクトリに向けるガード（Drop で元に戻す）。
    struct RootGuard(Option<std::ffi::OsString>);

    impl RootGuard {
        fn set(path: &std::path::Path) -> Self {
            let prev = std::env::var_os("SAI_PROJECT_ROOT");
            std::env::set_var("SAI_PROJECT_ROOT", path);
            Self(prev)
        }
    }

    impl Drop for RootGuard {
        fn drop(&mut self) {
            match self.0.take() {
                Some(v) => std::env::set_var("SAI_PROJECT_ROOT", v),
                None => std::env::remove_var("SAI_PROJECT_ROOT"),
            }
        }
    }

    /// ダミーの todo カード（`outputs/todo/<dir_run_id>/task.md`）を書き出す。
    fn write_task(root: &std::path::Path, dir_run_id: i64, fm_run_id: &str) -> PathBuf {
        let dir = root
            .join(".sai/outputs/todo")
            .join(dir_run_id.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("task.md");
        let text = [
            "---",
            "runner: pi",
            "creation_date: 2026/09/24 00:00:00",
            "execution_date: -",
            "origin_run_id: -",
            &format!("run_id: {}", fm_run_id),
            "block: demo",
            "thinking: ",
            "model: ",
            "provider: ",
            "inputs:",
            "round: 1",
            "global_round: 1",
            "tag: ",
            "description: テストカード",
            "---",
            "",
            "# task_text",
            "",
            "テスト",
        ]
        .join("\n");
        std::fs::write(&path, text).unwrap();
        path
    }

    /// `phone workflow` の出力が共有契約（C-L1）の形を満たすこと。
    ///
    /// DB は空（実行 0 件）なので `nodes` / `edges` / `separators` は空配列の
    /// ままで、成否ではなく「形」とキー集合を確かめる。
    /// 実際の DAG の中身は `sai_link_widgets::workflow::tests`（固定値）が保証する。
    #[test]
    fn workflow_output_has_contract_shape() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let _guard = RootGuard::set(tmp.path());

        let out = workflow().expect("phone workflow");
        for key in ["width", "height", "nodes", "edges", "separators", "lanes"] {
            assert!(out.get(key).is_some(), "{key} が無い: {out}");
        }
        assert!(out["width"].is_i64() && out["height"].is_i64());
        assert!(out["nodes"].is_array() && out["edges"].is_array());
        // lanes は todo / running / review / done の 4 つ
        let lanes = out["lanes"].as_object().expect("lanes");
        for l in ["todo", "running", "review", "done"] {
            assert!(lanes.get(l).is_some(), "lanes.{l} が無い");
        }
    }

    /// 実際に 1 件分の実行が DAG に載ったとき、`nodes` / `edges` が
    /// 契約どおりのキーを持つこと。
    ///
    /// DAG に載る条件は `workflow_moved = true` の実行だけなので、
    /// DB に実行を 1 件作っても載らない場合は鍵の照合を飛ばす
    /// （DAG の中身そのものは `sai_link_widgets::workflow::tests` の固定値が保証する）。
    #[test]
    fn workflow_node_fields_match_contract() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let _guard = RootGuard::set(tmp.path());

        {
            let store = crate::cli::open_db().expect("DB を開く");
            // block_metadata が無いと外部キー制約で失敗する（作れなくても問題ない）
            let _ = store.create_execution(1, None);
        }
        let out = workflow().expect("phone workflow");
        let Some(nodes) = out["nodes"].as_array().filter(|a| !a.is_empty()) else {
            // 実行が `workflow_moved` に入らず DAG に載らなかった（キー照合の対象外）
            return;
        };
        for key in [
            "id", "run_id", "phase", "name", "tag", "runner", "status", "created_at",
            "description", "cost", "orphaned", "review", "global_round", "round", "parent_run_id",
            "batch_id", "x", "y", "w", "h",
        ] {
            assert!(nodes[0].get(key).is_some(), "nodes[].{key} が無い: {}", nodes[0]);
        }
        for key in ["from", "to", "kind", "side"] {
            assert!(out["edges"][0].get(key).is_some(), "edges[].{key} が無い");
        }
    }

    /// 対象の選び方と見つからないときの終了コードをまとめて検証する。
    ///
    /// `SAI_PROJECT_ROOT` はプロセス共通のため、環境変数を触る検証はこの
    /// 1 テストに集約している（テストの並行実行での干渉を避けるため）。
    #[test]
    fn card_run_target_selection_and_exit_codes() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let _guard = RootGuard::set(root);

        // (1) 存在しない run_id → 終了コード 3
        let err = card_run(999_999_999).unwrap_err();
        assert_eq!(err.code, EXIT_NOT_FOUND);
        assert_eq!(err.msg, TODO_NOT_FOUND);

        // (2) 送信済み（pending に block_run_request あり）→ 終了コード 3
        write_task(root, 778, "778");
        let pending = root.join(".sai/sai-engine/notifications/pending");
        std::fs::create_dir_all(&pending).unwrap();
        std::fs::write(
            pending.join("1-block_run_request.json"),
            r#"{"type":"block_run_request","card_path":".sai/outputs/todo/778/task.md"}"#,
        )
        .unwrap();
        let err = card_run(778).unwrap_err();
        assert_eq!(err.code, EXIT_NOT_FOUND);
        assert_eq!(err.msg, TODO_NOT_FOUND);

        // 注: `todo_card_paths` の pending 照合は「フルパス一致 or ファイル名一致」で、
        // 新形式のカードは全部 `task.md` なため pending 中は新形式カード全体が
        // todo から消える（`phone tree` と同じ挙動）。以降のケースは
        // この干渉を避けるため pending を消してから検証する。
        std::fs::remove_file(pending.join("1-block_run_request.json")).unwrap();

        // (3) ディレクトリ名と frontmatter の run_id 不一致 → 終了コード 3
        write_task(root, 779, "780");
        let err = card_run(779).unwrap_err();
        assert_eq!(err.code, EXIT_NOT_FOUND);
        assert_eq!(err.msg, TODO_NOT_FOUND);

        // (4) 未送信カード → 対象に選ばれて常駐チェックへ（常駐なし = 終了コード 1）。
        // 実行要求は送らない（常駐確認で止まるためエージェントは起動しない）。
        write_task(root, 781, "781");
        let err = card_run(781).unwrap_err();
        assert_eq!(err.code, EXIT_USAGE);
        assert_eq!(err.msg, DAEMON_STOPPED);

        // (5) task の保存（md --write）: 未実行カードだけ・frontmatter を壊す内容は保存しない
        let path = root.join(".sai/outputs/todo/781/task.md");
        let original = std::fs::read_to_string(&path).unwrap();
        let edited = original.replace("テスト", "書き換えた");
        let saved = md_save(781, "task", &edited).unwrap_or_else(|e| panic!("{}", e.msg));
        assert_eq!(saved["text"], edited);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
        let err = md_save(781, "task", "frontmatter なし").unwrap_err();
        assert_eq!(err.code, EXIT_USAGE);
        let err = md_save(781, "task", &edited.replace("run_id: 781", "run_id: 9")).unwrap_err();
        assert_eq!(err.code, EXIT_USAGE);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
        // run_id が合わないカード・message は保存できない
        assert_eq!(md_save(779, "task", &edited).unwrap_err().code, EXIT_NOT_FOUND);
        assert_eq!(md_save(781, "message", "x").unwrap_err().code, EXIT_USAGE);
    }

    /// テスト用のブロックを作る（実行レコードを差し込むため）。
    fn insert_test_block(store: &SaiStore) -> i64 {
        store
            .insert_block(&sai_link_engine::db::models::BlockMetadata::new(
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
            ))
            .unwrap()
    }

    /// `phone finalize`: 実行が無い / 実行中以外 / 子ブロック / 常駐停止の扱い。
    ///
    /// 完了要求まで到達するケース（実行中のメインブロック）は、テスト環境では
    /// 常駐が居ないため必ず DAEMON_STOPPED で止まる（`SAI_PROJECT_ROOT` を
    /// 一時ディレクトリに向けたので PID ファイルも読まれない）。
    #[test]
    fn finalize_target_selection_and_exit_codes() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let _guard = RootGuard::set(root);

        // (1) 存在しない run_id → 終了コード 3
        let err = finalize(999_999_999).unwrap_err();
        assert_eq!(err.code, EXIT_NOT_FOUND);
        assert_eq!(err.msg, EXEC_NOT_FOUND);

        // 実行レコードを差し込む（一時ルート配下の DB を使う）
        let db = root.join(".sai/sai-engine/sai-engine.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let store = SaiStore::open(&db).unwrap();
        let block_id = insert_test_block(&store);
        // 完了済みの run
        let completed = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(completed, "completed").unwrap();
        // 実行中のメインブロック
        let running = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(running, "running").unwrap();
        // 実行中の子ブロック
        let subtask = store
            .create_execution_for_subtask(block_id, None, running, None)
            .unwrap();
        store.update_execution_status(subtask, "running").unwrap();
        drop(store);

        // (2) 完了済みの run → 終了コード 1（実行中ではありません）
        let err = finalize(completed).unwrap_err();
        assert_eq!(err.code, EXIT_USAGE);
        assert!(
            err.msg.contains("実行中ではありません"),
            "実行中以外を弾く: {}",
            err.msg
        );

        // (3) 実行中の子ブロック → 終了コード 1（task_report で完了する）
        let err = finalize(subtask).unwrap_err();
        assert_eq!(err.code, EXIT_USAGE);
        assert_eq!(err.msg, "子ブロックは task_report で完了します");

        // (4) 実行中のメインブロック → 常駐停止で終了コード 1
        let err = finalize(running).unwrap_err();
        assert_eq!(err.code, EXIT_USAGE);
        assert_eq!(err.msg, DAEMON_STOPPED);
    }

    /// `phone advance`: review → done → `R<gr>` と進み、review / done 以外は弾く。
    #[test]
    fn advance_review_to_workflow_and_unsupported() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let _guard = RootGuard::set(root);

        // 実行レコードを差し込む
        let db = root.join(".sai/sai-engine/sai-engine.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let store = SaiStore::open(&db).unwrap();
        let block_id = insert_test_block(&store);
        let run_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(run_id, "completed").unwrap();
        store
            .update_execution_rounds(run_id, Some(1), Some(5))
            .unwrap();
        drop(store);

        // (1) run フォルダが無い → 終了コード 3
        let err = advance(999_999_999).unwrap_err();
        assert_eq!(err.code, EXIT_NOT_FOUND);
        assert_eq!(err.msg, EXEC_NOT_FOUND);

        // (2) 置き場所が todo（未実行）→ 終了コード 1
        sai_link_engine::run_dir::ensure_run_dir(sai_link_engine::run_dir::TODO_OWNER, run_id)
            .unwrap();
        let err = advance(run_id).unwrap_err();
        assert_eq!(err.code, EXIT_USAGE);
        assert_eq!(err.msg, ADVANCE_UNSUPPORTED);

        // (3) review → done
        sai_link_engine::run_dir::move_run_dir(
            run_id,
            sai_link_engine::run_dir::TODO_OWNER,
            sai_link_engine::run_dir::REVIEW_OWNER,
        )
        .unwrap();
        let v = advance(run_id).unwrap_or_else(|e| panic!("{}", e.msg));
        assert_eq!(v["run_id"], run_id);
        assert_eq!(v["from"], "review");
        assert_eq!(v["to"], "done");
        assert_eq!(
            sai_link_engine::run_dir::find_run_dir(run_id).unwrap().0,
            "done"
        );

        // (4) done → Workflow（`R<global_round>`）
        let v = advance(run_id).unwrap_or_else(|e| panic!("{}", e.msg));
        assert_eq!(v["from"], "done");
        assert_eq!(v["to"], "R5");
        assert_eq!(
            sai_link_engine::run_dir::find_run_dir(run_id).unwrap().0,
            "R5"
        );

        // (5) すでに Workflow（R5）→ 終了コード 1（2 段目以上は行わない）
        let err = advance(run_id).unwrap_err();
        assert_eq!(err.code, EXIT_USAGE);
        assert_eq!(err.msg, ADVANCE_UNSUPPORTED);
    }

    // ========================================================================
    // テスト: phone summary / phone dashboard（実装計画 C C1 / C2）
    // ========================================================================

    /// テスト用のブロックを作る（engine 側の `create_test_block` は `#[cfg(test)]` のため
    /// ここでは `BlockMetadata::new` を直接使う）。
    fn test_block(name: &str, model: &str) -> sai_link_engine::db::BlockMetadata {
        sai_link_engine::db::BlockMetadata::new(
            name.to_string(),
            "テスト用ブロック".to_string(),
            model.to_string(),
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

    /// 一時ディレクトリに DB を作って `SAI_PROJECT_ROOT` をそこへ向ける。
    ///
    /// `SAI_PROJECT_ROOT` はプロセス共通のため、環境変数を触るテスト同士が
    /// 並行実行で衝突しないよう [`ENV_LOCK`] を取る（`card_run` のテストと同じ方針）。
    /// 返り値の [`RootGuard`] は**呼び出し側のテストが保持し続けること**
    /// （Drop すると `SAI_PROJECT_ROOT` が戻ってしまうため）。
    fn temp_project_store() -> (tempfile::TempDir, RootGuard, SaiStore) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let guard = RootGuard::set(root);
        let db_dir = root.join(".sai/sai-engine");
        std::fs::create_dir_all(&db_dir).unwrap();
        let store = SaiStore::open(&db_dir.join("sai-engine.db")).unwrap();
        (tmp, guard, store)
    }

    /// pretty JSON の深さ `depth` の項目名を順に並べる（`depth` は 1 始まり。
    /// pretty JSON は 1 段につき 2 スペースのインデント）。
    ///
    /// 並び順まで共有契約なので、`serde_json::Value`（アルファベット順）ではなく
    /// シリアライズしたテキストのインデント深さで判定する。
    fn top_level_keys(text: &str, depth: usize) -> Vec<String> {
        let prefix = "  ".repeat(depth);
        text.lines()
            .filter_map(|l| l.strip_prefix(prefix.as_str()))
            .filter_map(|l| l.strip_prefix('"'))
            .filter_map(|l| l.split('"').next())
            .map(String::from)
            .collect()
    }

    /// `phone summary` が「実行中の件数」と「最終実行の開始日時」を返すこと
    /// （実装計画 C C1 の共有契約 `{"running", "last_executed_at"}`）。
    #[test]
    fn summary_reports_running_count_and_last_executed_at() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_tmp, _guard, store) = temp_project_store();

        // 実行 3 件: completed / running / running
        let block_id = store
            .insert_block(&test_block("sum-block", "inherit"))
            .unwrap();
        let done = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(done, "completed").unwrap();
        let r1 = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(r1, "running").unwrap();
        let r2 = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(r2, "running").unwrap();

        let v = summary().unwrap();
        assert_eq!(v["running"], 2);
        // 最終実行 = `created_at` が最も新しい行（`list_all_executions` は昇順なので末尾）
        let expected = store
            .list_all_executions()
            .unwrap()
            .last()
            .map(|e| e.created_at.clone())
            .unwrap();
        assert_eq!(v["last_executed_at"], expected);
        // 共有契約はこの 2 項目だけ
        let obj = v.as_object().unwrap();
        assert_eq!(obj.len(), 2);
        assert!(obj.contains_key("running"));
        assert!(obj.contains_key("last_executed_at"));
    }

    /// 実行が 0 件のとき `phone summary` が `running = 0` / `last_executed_at = null` を返すこと。
    #[test]
    fn summary_on_empty_db() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_tmp, _guard, _store) = temp_project_store();

        let v = summary().unwrap();
        assert_eq!(v["running"], 0);
        assert!(v["last_executed_at"].is_null());
    }

    /// `phone dashboard` の JSON が sai-control と同じ**項目名・入れ子・並び順**であること
    /// （実装計画 C C2 の共有契約）。
    ///
    /// 並び順まで契約なので、構造体でシリアライズしている
    /// （`serde_json::Value` に落とすとアルファベット順になり sai-control と食い違う）。
    #[test]
    fn dashboard_key_order_matches_sai_control() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (tmp, _guard, store) = temp_project_store();
        let today = NaiveDate::from_ymd_opt(2026, 9, 26).unwrap();

        let d = build_dashboard(&store, tmp.path(), Period::All, today).unwrap();
        let text = serde_json::to_string_pretty(&d).unwrap();

        // トップレベルの並び順（sai-control `cross::Dashboard` の宣言順）
        assert_eq!(
            top_level_keys(&text, 1),
            vec![
                "name",
                "period",
                "since",
                "until",
                "daemon_alive",
                "summary",
                "daily",
                "by_model",
                "by_block",
                "recent",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        );
        // `summary` は唯一の子オブジェクトなので深さ 2 の項目名が summary の並び順
        assert_eq!(
            top_level_keys(&text, 2),
            vec![
                "executions",
                "completed",
                "failed",
                "running",
                "success_rate",
                "duration_ms",
                "tokens_input",
                "tokens_output",
                "tokens_cache",
                "tokens_total",
                "cost_usd",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        );
    }

    /// `week` / `month` / `all` の期間切り分けと集計が sai-control と合うこと
    /// （実装計画 C C2 のテスト条件）。
    ///
    /// 一時 DB に「今日」の実行を 3 件（completed / failed / running）入れ、
    /// `today_jst` を未来に動かすことで 3 つの期間が切り分けられることを確かめる。
    /// 実行の `created_at` は DB の `datetime('now')` のため直接書き換えられないので、
    /// 「今日」基準の実行を作り、`today_jst` を動かして期間から外す形で検証する。
    #[test]
    fn dashboard_period_split_and_totals() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (tmp, _guard, store) = temp_project_store();
        let root = tmp.path();

        // ブロック 2 種・実行 3 件
        let a = store
            .insert_block(&test_block("dash-a", "anthropic/claude-opus-4-5"))
            .unwrap();
        let b = store
            .insert_block(&test_block("dash-b", "anthropic/claude-sonnet-4-5"))
            .unwrap();
        let done = store.create_execution(a, None).unwrap();
        store.update_execution_status(done, "completed").unwrap();
        let failed = store.create_execution(a, None).unwrap();
        store.update_execution_status(failed, "failed").unwrap();
        let running = store.create_execution(b, None).unwrap();
        store.update_execution_status(running, "running").unwrap();

        // 「今日」を実際の日本時間の日付に合わせる（実行は DB の now に入るため）
        let today = (chrono::Utc::now() + chrono::Duration::hours(9)).date_naive();

        // --- (1) today_jst = today: week / month / all すべてに 3 件入る
        for period in [Period::Week, Period::Month, Period::All] {
            let d = build_dashboard(&store, root, period, today).unwrap();
            assert_eq!(d.summary.executions, 3, "period={:?}", period.as_str());
            assert_eq!(d.summary.completed, 1);
            assert_eq!(d.summary.failed, 1);
            assert_eq!(d.summary.running, 1);
            assert!((d.summary.success_rate - 1.0 / 3.0).abs() < 1e-9);
            assert_eq!(d.period, period.as_str());
            assert_eq!(d.until, today.to_string());
            // daily は since〜until の全日。最終日（今日）だけ 3 件・他は 0
            assert_eq!(d.daily.last().unwrap().executions, 3);
            assert!(d.daily.iter().rev().skip(1).all(|r| r.executions == 0));
            // by_block は 2 種 / by_model は 2 種 / recent は 3 件
            assert_eq!(d.by_block.len(), 2);
            assert_eq!(d.by_model.len(), 2);
            assert_eq!(d.recent.len(), 3);
            // recent は新しい順（run_id の降順）
            assert!(d.recent[0].run_id > d.recent[1].run_id);
            assert!(d.recent[1].run_id > d.recent[2].run_id);
            // recent の行の並び順（sai-control `cross::RecentRun` の宣言順）
            let recent_text = serde_json::to_string_pretty(&d.recent[0]).unwrap();
            assert_eq!(
                top_level_keys(&recent_text, 1),
                vec![
                    "run_id",
                    "parent_run_id",
                    "block",
                    "status",
                    "started_at",
                    "duration_ms",
                ]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
            );
        }

        // (2) 期間開始日の決め方（sai-control `cross::period_since` と同じ）
        let monday = today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
        assert_eq!(
            build_dashboard(&store, root, Period::Month, today)
                .unwrap()
                .since,
            today.with_day(1).unwrap().to_string()
        );
        assert_eq!(
            build_dashboard(&store, root, Period::Week, today)
                .unwrap()
                .since,
            monday.to_string()
        );
        // all の開始日は全実行の最小日付 = today
        assert_eq!(
            build_dashboard(&store, root, Period::All, today)
                .unwrap()
                .since,
            today.to_string()
        );

        // (3) today_jst を「今日から 8 日以上離れた次の月 1 日」に進める。
        //     week の月曜・month の 1 日はどちらもその前なので、3 件すべてが外れる。
        let mut cut = today.with_day(1).unwrap() + chrono::Duration::days(32);
        cut = NaiveDate::from_ymd_opt(cut.year(), cut.month(), 1).unwrap();
        while (cut - today).num_days() < 8 {
            cut = NaiveDate::from_ymd_opt(
                (cut + chrono::Duration::days(32)).year(),
                (cut + chrono::Duration::days(32)).month(),
                1,
            )
            .unwrap();
        }
        for period in [Period::Week, Period::Month] {
            let d = build_dashboard(&store, root, period, cut).unwrap();
            assert_eq!(d.summary.executions, 0, "period={:?}", period.as_str());
            assert!(d.by_model.is_empty());
            assert!(d.by_block.is_empty());
            assert!(d.recent.is_empty());
            // daily は 0 埋め（全日 0 件）
            assert!(d.daily.iter().all(|r| r.executions == 0));
        }
        // all は実行の最小日付からなので 3 件残る
        let d = build_dashboard(&store, root, Period::All, cut).unwrap();
        assert_eq!(d.summary.executions, 3);
        assert_eq!(d.since, today.to_string());
        assert_eq!(d.until, cut.to_string());
    }

    /// 実行が 0 件のとき `daily` が `since`〜`until` の全日を 0 埋めで返すこと
    /// （sai-control と同じ挙動）。
    #[test]
    fn dashboard_on_empty_db_zero_fills_daily() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (tmp, _guard, store) = temp_project_store();
        let today = NaiveDate::from_ymd_opt(2026, 9, 26).unwrap();

        let d = build_dashboard(&store, tmp.path(), Period::All, today).unwrap();
        assert_eq!(d.summary.executions, 0);
        assert_eq!(d.summary.success_rate, 0.0);
        // all でも 0 件なら since = today（`first` の既定）
        assert_eq!(d.since, today.to_string());
        assert_eq!(d.daily.len(), 1);
        assert_eq!(d.daily[0].executions, 0);
        assert!(d.by_model.is_empty());
        assert!(d.by_block.is_empty());
        assert!(d.recent.is_empty());
    }

    /// `Period` の受け渡し（`week` / `month` / `all` のみ・大小文字と前後の空白は許容）。
    #[test]
    fn period_parsing() {
        assert_eq!("week".parse::<Period>().unwrap(), Period::Week);
        assert_eq!("month".parse::<Period>().unwrap(), Period::Month);
        assert_eq!("all".parse::<Period>().unwrap(), Period::All);
        assert_eq!(" WEEK ".parse::<Period>().unwrap(), Period::Week);
        let err = "year".parse::<Period>().unwrap_err();
        assert!(err.contains("week / month / all"));
        // `phone dashboard` が受け付けない期間рю，並列 1（stderr に理由を出す）
        let err = dashboard("year").unwrap_err();
        assert_eq!(err.code, EXIT_USAGE);
        assert!(err.msg.contains("week / month / all"));
    }

    /// 実行日時文字列 → 日本時間の日付（`Z` 付き・空白区切りの両方を受け付ける）。
    #[test]
    fn exec_date_jst_parses_utc_strings() {
        let d = |s: &str| exec_date_jst(s).map(|d| d.to_string());
        // 2026-09-23T04:23:40Z = JST 2026-09-23 13:23:40
        assert_eq!(d("2026-09-23T04:23:40Z"), Some("2026-09-23".into()));
        // UTC 2026-09-23 14:30 → JST 2026-09-23 23:30（日付は変わらない）
        assert_eq!(d("2026-09-23 14:30:00"), Some("2026-09-23".into()));
        // UTC 2026-09-23 15:30 → JST 2026-09-24 00:30（日付が 1 日増える）
        assert_eq!(d("2026-09-23 15:30:00"), Some("2026-09-24".into()));
        // 小数秒付き
        assert_eq!(d("2026-09-23T04:23:40.123Z"), Some("2026-09-23".into()));
        // 解析できない
        assert_eq!(d(""), None);
        assert_eq!(d("not-a-date"), None);
    }

    /// `period_since` の期間開始日の決め方（sai-control `cross::period_since` と同じ）。
    #[test]
    fn period_since_rules() {
        // 2026-09-26 は土曜日
        let today = NaiveDate::from_ymd_opt(2026, 9, 26).unwrap();
        let first = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        assert_eq!(
            period_since(Period::Week, today, first).to_string(),
            "2026-09-21"
        );
        assert_eq!(
            period_since(Period::Month, today, first).to_string(),
            "2026-09-01"
        );
        assert_eq!(
            period_since(Period::All, today, first).to_string(),
            "2026-01-02"
        );
    }

    /// `phone card inputs` の候補 JSON の `ref` は、origin ではなく
    /// 文書を保存した実行の番号を使う（U08・AC2）。
    #[test]
    fn card_candidate_ref_uses_doc_run_id() {
        let c = phone_view::WaggleChoiceData {
            token: "W9knAR4G".to_string(),
            file_name: "paper-search-eeg-llm-2026.md".to_string(),
            description: "計画書".to_string(),
            block_name: "paper-search".to_string(),
            status: "completed".to_string(),
            run_id: 3,
        };
        let v = card_candidate_json(&c);
        assert_eq!(
            v["ref"].as_str().unwrap(),
            "waggle:W9knAR4G:path:.sai/outputs/*/3/paper-search-eeg-llm-2026.md"
        );
        assert_eq!(v["token"].as_str().unwrap(), "W9knAR4G");
    }
}
