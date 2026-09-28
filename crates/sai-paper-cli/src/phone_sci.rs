//! `sai-moon-paper phone sci …` — sai-phone（Web）の Research-tree 画面用の JSON。
//!
//! 共通ルール（`crates/sai-paper-cli/src/phone.rs` と同じ）:
//!
//! - 成功: 終了コード 0、標準出力に JSON 1 行
//! - 失敗: 終了コード 1、標準出力に `{"error": "<日本語の文>"}`
//!
//! 中の処理は core（`sai_paper_core::sci`）が担う。ここは外から来る値の検査と
//! JSON の形だけを持つ。準備の [`crate::sci::open_env`] は `sci` 側と共有する。

use anyhow::{Context, Result, anyhow};
use clap::Subcommand;
use sai_paper_core::sci::{self, card, defs, export, ops, store};
use serde_json::{Value, json};

use crate::sci::{open_env, parse_assume, parse_id, parse_stdin_text, proposal_json};

/// `sai-moon-paper phone sci` のサブコマンド（C-09）。
#[derive(Subcommand, Debug)]
pub enum PhoneSciCmd {
    /// 現況を JSON で返す（`sci status --json` と同じ）
    Status,
    /// 研究方針の木を始める
    Init {
        /// 研究の型（problem / phenomenon / method / data / improve）
        #[arg(long = "type", value_name = "型")]
        type_id: String,
    },
    /// 節点 1 つの詳細を JSON で返す
    Node {
        /// 節点の ID（`N12`）
        node: String,
    },
    /// 利用者の操作を 1 つ反映する（標準入力に C-07 の JSON）
    Apply,
    /// 仕分け待ちの提案を JSON で返す
    Proposals,
    /// 提案を仕分ける（採用のときの中身の上書きは標準入力。空なら無し）
    Triage {
        /// 提案の ID（`P56`）
        proposal: String,
        /// 仕分け（accept / reject / revert）
        #[arg(long)]
        action: String,
        /// 仕分けのメモ
        #[arg(long)]
        note: Option<String>,
    },
    /// 節点のための実行（カード）を作る（標準入力に C-09 の JSON）
    Card,
    /// 読む札の中身を JSON で返す
    Read {
        /// 実行 ID
        run_id: i64,
    },
    /// 読む札を終える
    FinishRead {
        /// 実行 ID
        run_id: i64,
        /// 反映しない理由（知見が無いとき必要）
        #[arg(long)]
        no_reflect: Option<String>,
    },
}

/// `sai-moon-paper phone sci` を実行する（出力と失敗は [`crate::phone::handle`] が扱う）。
pub fn run(cmd: PhoneSciCmd) -> Result<Value> {
    let mut input = String::new();
    if matches!(cmd, PhoneSciCmd::Apply | PhoneSciCmd::Card) {
        use std::io::Read as _;
        std::io::stdin()
            .read_to_string(&mut input)
            .context("標準入力を読めませんでした")?;
    }
    match cmd {
        PhoneSciCmd::Status => status(),
        PhoneSciCmd::Init { type_id } => init(&type_id),
        PhoneSciCmd::Node { node } => node_detail(&node),
        PhoneSciCmd::Apply => apply(&input),
        PhoneSciCmd::Proposals => proposals(),
        PhoneSciCmd::Triage {
            proposal,
            action,
            note,
        } => triage(&proposal, &action, note.as_deref(), &input),
        PhoneSciCmd::Card => make_card(&input),
        PhoneSciCmd::Read { run_id } => read(run_id),
        PhoneSciCmd::FinishRead { run_id, no_reflect } => {
            finish_read(run_id, no_reflect.as_deref())
        }
    }
}

/// `phone sci status`。
fn status() -> Result<Value> {
    let env = open_env()?;
    let status = sci::compute::status(env.conn(), &env.roots, &env.defs)
        .map_err(|err| anyhow!("{err}"))
        .context("現況を集められませんでした")?;
    Ok(serde_json::to_value(status)?)
}

/// `phone sci init` — 木を始める（部品の配りは U15 が足す）。
fn init(type_id: &str) -> Result<Value> {
    let env = open_env()?;
    if store::is_initialized(env.conn()).map_err(|err| anyhow!("{err}"))? {
        let name = env
            .defs
            .type_def(type_id)
            .map_or_else(String::new, |def| def.name.clone());
        anyhow::bail!(
            "既に始めています（型: {name}）。型を変えるには `sai-moon-paper sci init --type <型> --change-type`"
        );
    }

    defs::write_local_copies(&env.roots.root)
        .map_err(|err| anyhow!("{err}"))
        .context(".sai/sci/ に型と項目ガイドを置けませんでした")?;
    let backup = export::backup_state_once(&env.roots.data_dir)
        .map_err(|err| anyhow!("{err}"))
        .context("今の state.md を残せませんでした")?;
    let report = defs::init_tree(env.conn(), &env.defs, type_id)
        .map_err(|err| anyhow!("{err}"))
        .context("骨組みを作れませんでした")?;
    env.export()?;

    Ok(json!({
        "type": report.type_id,
        "created_items": report.created_items,
        "created_edges": report.created_edges,
        "warnings": report.warnings,
        "backup": backup.map(|path| path.to_string_lossy().to_string()),
        // 研究プロジェクトの部品の配り方は U15 がここへ足す。
        "assets": [],
    }))
}

/// `phone sci node <N>`。
fn node_detail(node: &str) -> Result<Value> {
    let env = open_env()?;
    let id = parse_id(node, 'N')?;
    let detail = sci::compute::node_detail(env.conn(), &env.roots, &env.defs, id)
        .map_err(|err| anyhow!("{err}"))
        .context("節点の詳細を集められませんでした")?;
    Ok(serde_json::to_value(detail)?)
}

/// `phone sci apply` — 利用者の操作を 1 つ反映する。
fn apply(input: &str) -> Result<Value> {
    let env = open_env()?;
    let value = parse_stdin_text(input)?;
    let result = ops::apply_user(env.conn(), &env.roots, &env.defs, &value)
        .map_err(|err| anyhow!("{err}"))?;
    env.export()?;
    Ok(serde_json::to_value(result)?)
}

/// `phone sci proposals` — 仕分け待ちの一覧。
fn proposals() -> Result<Value> {
    let env = open_env()?;
    let rows = store::list_proposals(env.conn(), Some("pending"))
        .map_err(|err| anyhow!("{err}"))
        .context("提案を読めませんでした")?;
    let list: Vec<Value> = rows.iter().map(proposal_json).collect();
    Ok(json!({ "proposals": list }))
}

/// `phone sci triage <P> --action …` — 仕分け。
fn triage(proposal: &str, action: &str, note: Option<&str>, input: &str) -> Result<Value> {
    let env = open_env()?;
    let id = parse_id(proposal, 'P')?;
    let action = match action {
        "accept" => ops::TriageAction::Accept {
            edit: edit_from_input(input)?,
        },
        "reject" => ops::TriageAction::Reject,
        "revert" => ops::TriageAction::Revert,
        other => anyhow::bail!("知らない仕分けです: {other}（accept / reject / revert）"),
    };
    let result = ops::triage(
        env.conn(),
        &env.roots,
        &env.defs,
        id,
        action,
        note.unwrap_or(""),
    )
    .map_err(|err| anyhow!("{err}"))?;
    env.export()?;
    Ok(serde_json::to_value(result)?)
}

/// 採用のときの中身の上書きを読む（標準入力が空なら無し）。
fn edit_from_input(input: &str) -> Result<Option<Value>> {
    if input.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(parse_stdin_text(input)?))
}

/// `phone sci card` — 節点のための実行（カード）を作る。
fn make_card(input: &str) -> Result<Value> {
    let env = open_env()?;
    let spec = card_spec(input)?;
    let result =
        card::create_card(&env.db, &env.roots, &env.defs, &spec).map_err(|err| anyhow!("{err}"))?;
    env.export()?;
    Ok(json!({
        "run_id": result.run_id,
        "path": result.path.to_string_lossy(),
    }))
}

/// `phone sci read <run_id>` — 読む札の中身。
fn read(run_id: i64) -> Result<Value> {
    let env = open_env()?;
    let card = card::read_card(env.conn(), &env.roots, &env.defs, run_id)
        .map_err(|err| anyhow!("{err}"))?;
    Ok(serde_json::to_value(card)?)
}

/// `phone sci finish-read <run_id>` — 読む札を終える。
fn finish_read(run_id: i64, no_reflect: Option<&str>) -> Result<Value> {
    let env = open_env()?;
    let result = card::finish_read(env.conn(), &env.roots, run_id, no_reflect)
        .map_err(|err| anyhow!("{err}"))?;
    env.export()?;
    Ok(serde_json::to_value(result)?)
}

// ---- 標準入力の JSON --------------------------------------------------------

/// `phone sci card` の JSON を [`card::CardSpec`] にする。
fn card_spec(input: &str) -> Result<card::CardSpec> {
    let value = parse_stdin_text(input)?;
    let node = text_of(&value, "node");
    let question = text_of(&value, "question");
    Ok(card::CardSpec {
        node: parse_id(&node, 'N')?,
        block: text_of(&value, "block"),
        purpose: text_of(&value, "purpose"),
        question: if question.is_empty() {
            None
        } else {
            Some(parse_id(&question, 'Q')?)
        },
        expect: text_of(&value, "expect"),
        scope: text_of(&value, "scope"),
        assume: assume_of(&value)?,
        mode: text_of(&value, "mode"),
        papers: numbers_of(&value, "papers")?,
        runner: default_runner(&text_of(&value, "runner")),
        origin_run: number_of(&value, "origin_run")?,
        role: text_of(&value, "role"),
        depth: text_of(&value, "depth"),
    })
}

/// `assume` の `["N9=値"]` を読む。
fn assume_of(value: &Value) -> Result<Vec<(i64, String)>> {
    let Some(items) = value.get("assume") else {
        return Ok(Vec::new());
    };
    let list = items
        .as_array()
        .ok_or_else(|| anyhow!("assume は文字列の配列です"))?;
    let texts: Vec<String> = list
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("assume は文字列の配列です"))
        })
        .collect::<Result<Vec<_>>>()?;
    parse_assume(&texts)
}

/// 文字列の項目を読む（無ければ空文字）。
fn text_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// 数が 1 つだけ入っている項目を読む（無ければ `None`）。
fn number_of(value: &Value, key: &str) -> Result<Option<i64>> {
    let Some(item) = value.get(key) else {
        return Ok(None);
    };
    if item.is_null() {
        return Ok(None);
    }
    let number = item.as_i64().ok_or_else(|| anyhow!("{key} は数です"))?;
    Ok(Some(number))
}

/// 数の配列を読む（無ければ空）。
fn numbers_of(value: &Value, key: &str) -> Result<Vec<i64>> {
    let Some(item) = value.get(key) else {
        return Ok(Vec::new());
    };
    if item.is_null() {
        return Ok(Vec::new());
    }
    item.as_array()
        .ok_or_else(|| anyhow!("{key} は数の配列です"))?
        .iter()
        .map(|item| item.as_i64().ok_or_else(|| anyhow!("{key} は数の配列です")))
        .collect()
}

/// 実行者が空なら `pi` にする。
fn default_runner(runner: &str) -> String {
    if runner.is_empty() {
        "pi".to_string()
    } else {
        runner.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_spec_from_phone_json() {
        let input = r#"{
            "node": "N12",
            "block": "sci-compare",
            "purpose": "小目的の候補を比べる",
            "question": "Q7",
            "expect": "比較表",
            "scope": "path",
            "assume": ["N9=発作部位の予測（仮）"],
            "papers": [37, 40],
            "runner": "",
            "role": "compare",
            "depth": "full"
        }"#;
        let spec = card_spec(input).unwrap();
        assert_eq!(spec.node, 12);
        assert_eq!(spec.block, "sci-compare");
        assert_eq!(spec.purpose, "小目的の候補を比べる");
        assert_eq!(spec.question, Some(7));
        assert_eq!(spec.expect, "比較表");
        assert_eq!(spec.scope, "path");
        assert_eq!(spec.assume, vec![(9, "発作部位の予測（仮）".to_string())]);
        assert_eq!(spec.papers, vec![37, 40]);
        // 実行者が空なら `pi`（読む札は `user` を書く）。
        assert_eq!(spec.runner, "pi");
        assert_eq!(spec.role, "compare");
        assert_eq!(spec.depth, "full");
    }

    #[test]
    fn card_spec_from_phone_json_minimal() {
        let spec = card_spec(r#"{"node":"N12","purpose":"候補を足す"}"#).unwrap();
        assert_eq!(spec.node, 12);
        assert_eq!(spec.question, None);
        assert!(spec.assume.is_empty());
        assert!(spec.papers.is_empty());
        assert_eq!(spec.runner, "pi");
    }

    #[test]
    fn card_spec_from_phone_json_rejects_bad_shape() {
        // 節点の指定が要る。
        assert!(card_spec(r#"{"purpose":"のみ"}"#).is_err());
        // 節点の ID の形が違う。
        assert!(card_spec(r#"{"node":"12","purpose":"のみ"}"#).is_err());
        // 仮置きの形が違う。
        assert!(card_spec(r#"{"node":"N12","assume":["9=値"]}"#).is_err());
        // 論文が数でない。
        assert!(card_spec(r#"{"node":"N12","papers":["37"]}"#).is_err());
        // JSON として読めない。
        assert!(card_spec("{oops").is_err());
    }

    #[test]
    fn edit_from_input_is_none_when_stdin_is_empty() {
        assert!(edit_from_input("").unwrap().is_none());
        assert!(edit_from_input("  \n").unwrap().is_none());
        let edit = edit_from_input("{\"title\":\"別の題\"}").unwrap().unwrap();
        assert_eq!(edit["title"], "別の題");
    }
}
