//! sai-link を CLI だけで使う（run の一覧とカードの作成）。
//!
//! sci-core は sai-link の DB と実行フォルダの並び
//! （`.sai/outputs/`）を直に開かない。run の一覧は
//! `sai-moon-link run list --json --limit 0`、カードの作成は
//! `sai-moon-link card create`（標準入力に JSON）で行う。
//!
//! 子の実行ファイルの見つけ方と呼び方は [`crate::bins`] に任せる。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

use crate::bins::{self, BinError};

/// sai-link の run 1 件（`run list --json` の 1 項目）。
///
/// 値が無い項目は空の文字・`None` にする。`global_round`・`round` は
/// 文字でも数でも受け、文字にする。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRun {
    /// 実行 ID。
    pub run_id: i64,
    /// ブロック名。
    pub block: String,
    /// タグ。
    pub tag: String,
    /// 状態（`todo`・`pending`・`running`・`completed` …）。
    pub status: String,
    /// 実行フォルダの親の名前（`todo`・`running`・`review`・`done`・`R<n>` …）。
    pub lane: String,
    /// 親の実行 ID。
    pub parent_run_id: Option<i64>,
    /// グローバルラウンド（文字にする）。
    pub global_round: String,
    /// ローカルのラウンド（文字にする）。
    pub round: String,
    /// 作成日時。
    pub created_at: String,
    /// 開始日時。
    pub started_at: String,
    /// 終了日時。
    pub finished_at: String,
    /// 1 行の説明。
    pub description: String,
    /// 実行フォルダの絶対パス（無ければ `None`）。
    pub dir: Option<PathBuf>,
}

/// run の一覧を覚えておく（同じプロセスの中で root ごとに 1 回だけ子を呼ぶ）。
fn cache() -> &'static Mutex<HashMap<PathBuf, Vec<LinkRun>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Vec<LinkRun>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 覚えた run の一覧を捨てる。カードを作った後に呼ぶ。
pub fn forget(root: &Path) {
    if let Ok(mut map) = cache().lock() {
        map.remove(root);
    }
}

/// `sai-moon-link run list --json --limit 0` で run の一覧を読む。
///
/// 同じ root について 1 回だけ子を呼び、結果を覚えておく
/// （[`forget`] を呼ぶまで）。形が違えば [`BinError`]。
///
/// # Errors
/// 子を起動できない、子が 0 以外で終わる、出力が読めないとき。
pub fn runs(root: &Path) -> Result<Vec<LinkRun>, BinError> {
    if let Ok(map) = cache().lock()
        && let Some(runs) = map.get(root)
    {
        return Ok(runs.clone());
    }

    let args: Vec<String> = ["run", "list", "--json", "--limit", "0"]
        .iter()
        .map(|arg| (*arg).to_string())
        .collect();
    let out = bins::run(&bins::link_bin(), &args, root, None)?;
    let value: Value = serde_json::from_str(&out).map_err(|error| BinError {
        code: 1,
        message: format!("sai-link の run の一覧を読めませんでした: {error}"),
    })?;
    let runs = parse_runs(&value)?;
    if let Ok(mut map) = cache().lock() {
        map.insert(root.to_path_buf(), runs.clone());
    }
    Ok(runs)
}

/// `{"runs":[…]}` を [`LinkRun`] の並びにする。
fn parse_runs(value: &Value) -> Result<Vec<LinkRun>, BinError> {
    let items = value
        .get("runs")
        .and_then(Value::as_array)
        .ok_or_else(|| BinError {
            code: 1,
            message: "sai-link の run の一覧の形が違います（runs がありません）".to_string(),
        })?;

    let mut runs = Vec::with_capacity(items.len());
    for item in items {
        let run_id = item
            .get("run_id")
            .and_then(Value::as_i64)
            .ok_or_else(|| BinError {
                code: 1,
                message: "sai-link の run の一覧に run_id がありません".to_string(),
            })?;
        runs.push(LinkRun {
            run_id,
            block: string_field(item, "block"),
            tag: string_field(item, "tag"),
            status: string_field(item, "status"),
            lane: string_field(item, "lane"),
            parent_run_id: item.get("parent_run_id").and_then(Value::as_i64),
            global_round: string_field(item, "global_round"),
            round: string_field(item, "round"),
            created_at: string_field(item, "created_at"),
            started_at: string_field(item, "started_at"),
            finished_at: string_field(item, "finished_at"),
            description: string_field(item, "description"),
            dir: item
                .get("dir")
                .and_then(Value::as_str)
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from),
        });
    }
    Ok(runs)
}

/// JSON の項目を文字にする（数・真偽も文字にする。無ければ空）。
fn string_field(item: &Value, key: &str) -> String {
    match item.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    }
}

/// `sai-moon-link card create`（標準入力の JSON）に渡す項目。
#[derive(Debug, Clone)]
pub struct CardCreate {
    /// 実行エージェント（`pi` / `cc`）。
    pub runner: String,
    /// ブロック ID。
    pub block: String,
    /// グローバルラウンド。
    pub global_round: String,
    /// ローカルのラウンド。
    pub round: String,
    /// タグ。
    pub tag: String,
    /// 1 行の説明。
    pub description: String,
    /// タスク本文。
    pub task_text: String,
    /// 起動者（`human` / `agent`）。
    pub executer: String,
    /// frontmatter に足す任意の項目。
    pub meta: BTreeMap<String, String>,
}

/// `sai-moon-link card create` でカードを作る。
///
/// 標準入力に [`CardCreate`] の JSON を渡す（`thinking`・`model`・`provider`・
/// `doc_tokens` は渡さない）。返り値は `(run_id, task.md のパス)`。
/// 成功したら [`forget`] を呼び、覚えていた run の一覧を捨てる。
///
/// # Errors
/// 子を起動できない、子が 0 以外で終わる、出力が読めないとき。
pub fn card_create(root: &Path, card: &CardCreate) -> Result<(i64, PathBuf), BinError> {
    let input = serde_json::json!({
        "runner": card.runner,
        "block": card.block,
        "global_round": card.global_round,
        "round": card.round,
        "tag": card.tag,
        "description": card.description,
        "task_text": card.task_text,
        "executer": card.executer,
        "meta": card.meta,
    });
    let args: Vec<String> = ["card", "create"]
        .iter()
        .map(|arg| (*arg).to_string())
        .collect();
    let out = bins::run(&bins::link_bin(), &args, root, Some(&input.to_string()))?;
    forget(root);

    let value: Value = serde_json::from_str(&out).map_err(|error| BinError {
        code: 1,
        message: format!("sai-link の card create の出力を読めませんでした: {error}"),
    })?;
    let run_id = value
        .get("run_id")
        .and_then(Value::as_i64)
        .ok_or_else(|| BinError {
            code: 1,
            message: "sai-link の card create の出力に run_id がありません".to_string(),
        })?;
    let path = value
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| BinError {
            code: 1,
            message: "sai-link の card create の出力に path がありません".to_string(),
        })?;
    Ok((run_id, PathBuf::from(path)))
}

/// 次の `global_round` を求める（`lane` が `R<数>` の物の最大 + 1。無ければ 1）。
#[must_use]
pub fn next_global_round(runs: &[LinkRun]) -> i64 {
    let max = runs
        .iter()
        .filter_map(|run| run.lane.strip_prefix('R'))
        .filter_map(|rest| rest.parse::<i64>().ok())
        .max()
        .unwrap_or(0);
    max + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-sci-link-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run(lane: &str) -> LinkRun {
        LinkRun {
            run_id: 1,
            block: String::new(),
            tag: String::new(),
            status: String::new(),
            lane: lane.to_string(),
            parent_run_id: None,
            global_round: String::new(),
            round: String::new(),
            created_at: String::new(),
            started_at: String::new(),
            finished_at: String::new(),
            description: String::new(),
            dir: None,
        }
    }

    /// `R<n>` の最大 + 1。無ければ 1。
    #[test]
    fn next_global_round_uses_largest_lane() {
        assert_eq!(next_global_round(&[]), 1);
        assert_eq!(next_global_round(&[run("todo"), run("running")]), 1);
        assert_eq!(next_global_round(&[run("R3"), run("R10"), run("done")]), 11);
        assert_eq!(next_global_round(&[run("R0")]), 1);
    }

    /// 形の違う JSON はエラー。
    #[test]
    fn parse_runs_rejects_wrong_shape() {
        let error = parse_runs(&serde_json::json!({"nope": []})).unwrap_err();
        assert_eq!(error.code, 1);
        let run_id = serde_json::json!({"runs": [{"block": "x"}]});
        assert!(parse_runs(&run_id).is_err());
    }

    /// 数・null・文字を読む。
    #[test]
    fn parse_runs_reads_scalars() {
        let value = serde_json::json!({"runs": [{
            "run_id": 7,
            "block": "sci-read",
            "tag": "仮説",
            "status": "completed",
            "lane": "R10",
            "parent_run_id": 3,
            "global_round": 11,
            "round": "2",
            "created_at": "2026-09-25 06:01:02",
            "started_at": null,
            "finished_at": null,
            "description": "説明",
            "dir": "/tmp/run/7",
        }]});
        let runs = parse_runs(&value).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, 7);
        assert_eq!(runs[0].block, "sci-read");
        assert_eq!(runs[0].parent_run_id, Some(3));
        assert_eq!(runs[0].global_round, "11");
        assert_eq!(runs[0].round, "2");
        assert_eq!(runs[0].started_at, "");
        assert_eq!(runs[0].dir.as_deref(), Some(Path::new("/tmp/run/7")));
    }

    /// 同じ root では子を 1 回しか呼ばない（覚えておく）。
    #[test]
    fn runs_calls_child_once() {
        let root = temp_root("once");
        crate::fake_link::write_runs(&root, &[]);

        crate::bins::with_link_bin(crate::fake_link::script(), || {
            assert!(runs(&root).unwrap().is_empty());
            assert!(runs(&root).unwrap().is_empty());
        });

        let calls = crate::fake_link::calls(&root);
        let list_calls = calls
            .iter()
            .filter(|call| call.starts_with("run list"))
            .count();
        assert_eq!(list_calls, 1, "calls={calls:?}");
        forget(&root);
    }
}
