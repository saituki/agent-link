//! `sai-moon-paper phone sci …` — sai-phone（Web）の Research-tree 画面用の JSON。
//!
//! 共通ルール（`crates/sai-paper-cli/src/phone.rs` と同じ）:
//!
//! - 成功: 終了コード 0、標準出力に JSON 1 行
//! - 失敗: 終了コード 1、標準出力に `{"error": "<日本語の文>"}`
//!
//! 中の処理は core（`sai_sci_core::rt`）が担う。ここは外から来る値の検査と
//! JSON の形だけを持つ。準備の [`crate::sci::open_env`] は `sci` 側と共有する。
//! 書き込む命令（`init`・`apply`・`triage`・`card`）は終わる前に state.md と tree.md を書き直す。

use anyhow::{Context, Result, anyhow};
use clap::Subcommand;
use sai_sci_core::rt::{self, card, compute, ops, store};
use serde_json::{Value, json};

use crate::sci::{open_env, parse_stdin_text, proposal_json};

/// `sai-moon-paper phone sci` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum PhoneSciCmd {
    /// 現況を JSON で返す（`sci status --json` と同じ）
    Status,
    /// 研究方針の木を始める
    Init {
        /// 研究の型（problem / phenomenon / method / data / improve など）
        #[arg(long = "type", value_name = "型")]
        type_id: String,
    },
    /// Node 1 つの詳細を JSON で返す
    Node {
        /// Node の ID（`Q12`・`O5` など）
        node: String,
    },
    /// 利用者の操作を 1 つ反映する（標準入力に 1 件の JSON）
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
    /// Node のための実行（カード）を作る（標準入力に C-CARD の JSON）
    Card,
    /// 読む札（runner が user のカード）の中身を JSON で返す
    Read {
        /// 実行 ID
        run_id: i64,
    },
    /// 使える研究の型を JSON で返す
    Types,
}

/// `sai-moon-paper phone sci` を実行する（出力と失敗は [`crate::phone::handle`] が扱う）。
pub fn run(cmd: PhoneSciCmd) -> Result<Value> {
    let input = read_stdin_for(&cmd)?;
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
        PhoneSciCmd::Types => types(),
    }
}

/// 標準入力を読むか（`apply`・`card`・採用の `triage` だけ）。
///
/// 採用のときだけ「直してから採用」の JSON を読む。捨てと巻き戻しは
/// 標準入力を使わないので、閉じていなくても困らない。
fn reads_stdin(cmd: &PhoneSciCmd) -> bool {
    match cmd {
        PhoneSciCmd::Apply | PhoneSciCmd::Card => true,
        PhoneSciCmd::Triage { action, .. } => action == "accept",
        _ => false,
    }
}

/// 必要なときだけ標準入力を読む（端末のままだと終わらないので待たない）。
fn read_stdin_for(cmd: &PhoneSciCmd) -> Result<String> {
    use std::io::IsTerminal as _;
    if !reads_stdin(cmd) || std::io::stdin().is_terminal() {
        return Ok(String::new());
    }
    use std::io::Read as _;
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .context("標準入力を読めませんでした")?;
    Ok(text)
}

/// `phone sci status`。
fn status() -> Result<Value> {
    let env = open_env()?;
    let status = compute::status(env.conn(), &env.roots, &env.defs)
        .map_err(|err| anyhow!("{err}"))
        .context("現況を集められませんでした")?;
    Ok(serde_json::to_value(status)?)
}

/// `phone sci init` — 部品を配って木を始める。
fn init(type_id: &str) -> Result<Value> {
    let env = open_env()?;
    // 部品（ブロック・テンプレート・`context`・土壌のひな形）を配り、木を作って書き出す。
    let outcome = crate::sci::init_with_assets(&env.roots.root, type_id)
        .map_err(|err| anyhow!("{err}"))
        .context("研究方針の木を作れませんでした")?;
    if outcome.already {
        anyhow::bail!("既に始めています（型: {}）", outcome.type_name);
    }
    Ok(outcome.json())
}

/// `phone sci node <ID>`。
fn node_detail(node: &str) -> Result<Value> {
    let env = open_env()?;
    let detail = compute::node_detail(env.conn(), &env.roots, &env.defs, node.trim())
        .map_err(|err| anyhow!("{err}"))?;
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
    let id = rt::parse_ref(proposal.trim(), 'P').map_err(|err| anyhow!("{err}"))?;
    let (action, edit) = triage_action(action, input)?;
    crate::sci::run_triage(id, action, edit.as_ref(), note.unwrap_or(""))
}

/// 仕分けの指定を action にする（採用のときだけ標準入力の「直してから採用」を読む）。
fn triage_action(action: &str, input: &str) -> Result<(ops::TriageAction, Option<Value>)> {
    match action {
        "accept" => Ok((ops::TriageAction::Accept, edit_from_input(input)?)),
        "reject" => Ok((ops::TriageAction::Reject, None)),
        "revert" => Ok((ops::TriageAction::Revert, None)),
        other => anyhow::bail!("知らない仕分けです: {other}（accept / reject / revert）"),
    }
}

/// 採用のときの中身の上書きを読む（標準入力が空なら無し）。
fn edit_from_input(input: &str) -> Result<Option<Value>> {
    if input.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(parse_stdin_text(input)?))
}

/// `phone sci card` — Node のための実行（カード）を作る。
fn make_card(input: &str) -> Result<Value> {
    crate::sci::make_card(&card_spec(input)?)
}

/// `phone sci read <run_id>` — 読む札の中身。
fn read(run_id: i64) -> Result<Value> {
    let env = open_env()?;
    let card = card::read_card(env.conn(), &env.roots, &env.defs, run_id)
        .map_err(|err| anyhow!("{err}"))?;
    Ok(serde_json::to_value(card)?)
}

/// `phone sci types` — 使える型の一覧。
fn types() -> Result<Value> {
    let roots = sai_paper_core::paths::paper_roots()
        .context("研究プロジェクト（.sai/paper）が見つかりません")?;
    Ok(crate::sci::types_json(&rt::defs::load(&roots.root)))
}

// ---- 標準入力の JSON --------------------------------------------------------

/// `phone sci card` の JSON（C-CARD）を [`card::CardSpec`] にする。
fn card_spec(input: &str) -> Result<card::CardSpec> {
    let value = parse_stdin_text(input)?;
    Ok(card::CardSpec {
        node: text_of(&value, "node"),
        block: text_of(&value, "block"),
        purpose: text_of(&value, "purpose"),
        expect: text_of(&value, "expect"),
        scope: text_of(&value, "scope"),
        assume: strings_of(&value, "assume")?,
        mode: text_of(&value, "mode"),
        papers: numbers_of(&value, "papers")?,
        runner: default_runner(&text_of(&value, "runner")),
        depth: text_of(&value, "depth"),
        origin_run: number_of(&value, "origin_run")?,
        // Web からは文書を受け取らない（`--input` は CLI の引数だけ）。
        inputs: Vec::new(),
        // Web の画面から作るカードは人が起こすもの（professor が起こすカードは CLI だけ）。
        executer: String::new(),
    })
}

/// 文字列の配列の項目を読む（無ければ空）。
fn strings_of(value: &Value, key: &str) -> Result<Vec<String>> {
    let Some(items) = value.get(key) else {
        return Ok(Vec::new());
    };
    if items.is_null() {
        return Ok(Vec::new());
    }
    let invalid = || anyhow!("{key} は文字列の配列です");
    items
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(|item| item.as_str().map(str::to_string).ok_or_else(invalid))
        .collect()
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
            "node": "Q12",
            "block": "sci-ideate",
            "purpose": "「データの量は？」の答えの候補（O）を出す",
            "expect": "O を 2 つ以上",
            "scope": "path",
            "assume": ["Q9=SST-2 を使う（仮）"],
            "mode": "purpose_first",
            "papers": [37, 40],
            "runner": "",
            "depth": "full",
            "origin_run": 120
        }"#;
        let spec = card_spec(input).unwrap();
        assert_eq!(spec.node, "Q12");
        assert_eq!(spec.block, "sci-ideate");
        assert_eq!(spec.expect, "O を 2 つ以上");
        assert_eq!(spec.scope, "path");
        assert_eq!(spec.assume, vec!["Q9=SST-2 を使う（仮）".to_string()]);
        assert_eq!(spec.mode, "purpose_first");
        assert_eq!(spec.papers, vec![37, 40]);
        // 実行者が空なら `pi`（読む札は `user` を書く）。
        assert_eq!(spec.runner, "pi");
        assert_eq!(spec.depth, "full");
        assert_eq!(spec.origin_run, Some(120));
        // Web からは文書も起動者も受け取らない。
        assert!(spec.inputs.is_empty());
        assert!(spec.executer.is_empty());
    }

    #[test]
    fn card_spec_from_phone_json_minimal() {
        let spec = card_spec(r#"{"node":"Q12","purpose":"候補を足す"}"#).unwrap();
        assert_eq!(spec.node, "Q12");
        assert!(spec.assume.is_empty());
        assert!(spec.papers.is_empty());
        assert_eq!(spec.runner, "pi");
    }

    #[test]
    fn card_spec_from_phone_json_rejects_bad_shape() {
        // 論文が数でない。
        assert!(card_spec(r#"{"node":"Q12","papers":["37"]}"#).is_err());
        // 仮置きが文字列でない。
        assert!(card_spec(r#"{"node":"Q12","assume":[9]}"#).is_err());
        // JSON として読めない。
        assert!(card_spec("{oops").is_err());
    }

    #[test]
    fn edit_from_input_is_none_when_stdin_is_empty() {
        assert!(edit_from_input("").unwrap().is_none());
        assert!(edit_from_input("  \n").unwrap().is_none());
        let edit = edit_from_input("{\"text\":\"別の文\"}").unwrap().unwrap();
        assert_eq!(edit["text"], "別の文");
    }

    /// 採用のときだけ標準入力の「直してから採用」の JSON を読む。
    ///
    /// 捨てと巻き戻しのときは標準入力が閉じていなくても困るので読まない。
    #[test]
    fn reads_stdin_only_where_it_is_needed() {
        assert!(reads_stdin(&parse_phone_sci(&["apply"])));
        assert!(reads_stdin(&parse_phone_sci(&["card"])));
        assert!(reads_stdin(&parse_phone_sci(&["triage", "P1", "--action", "accept"])));
        assert!(!reads_stdin(&parse_phone_sci(&["triage", "P1", "--action", "reject"])));
        assert!(!reads_stdin(&parse_phone_sci(&["triage", "P1", "--action", "revert"])));
        assert!(!reads_stdin(&parse_phone_sci(&["status"])));
        assert!(!reads_stdin(&parse_phone_sci(&["proposals"])));
        assert!(!reads_stdin(&parse_phone_sci(&["types"])));
        assert!(!reads_stdin(&parse_phone_sci(&["init", "--type", "problem"])));
        assert!(!reads_stdin(&parse_phone_sci(&["read", "7"])));
    }

    /// 採用のときだけ標準入力の「直してから採用」が付く。
    #[test]
    fn triage_action_accept_carries_the_edit_from_stdin() {
        let (action, edit) = triage_action("accept", "{\"text\":\"別の文\"}").unwrap();
        assert_eq!(action, ops::TriageAction::Accept);
        assert_eq!(edit.unwrap()["text"], "別の文");

        // 標準入力が空なら上書き無し。
        assert_eq!(triage_action("accept", "").unwrap(), (ops::TriageAction::Accept, None));
        // 捨てと巻き戻しは上書きを読まない。
        assert_eq!(triage_action("reject", "{\"x\":1}").unwrap(), (ops::TriageAction::Reject, None));
        assert_eq!(triage_action("revert", "{\"x\":1}").unwrap(), (ops::TriageAction::Revert, None));

        // 知らない仕分けは日本語で断る。
        let err = triage_action("取り消し", "").unwrap_err();
        assert!(format!("{err:#}").contains("知らない仕分けです"), "{err:#}");
    }

    /// `graph` と `finish-read` はもう無い。
    #[test]
    fn removed_commands_are_gone() {
        use clap::Parser as _;
        for name in ["graph", "finish-read"] {
            assert!(
                crate::Cli::try_parse_from(["sai-moon-paper", "phone", "sci", name]).is_err(),
                "{name}"
            );
        }
    }

    /// `sai-moon-paper phone sci …` を clap で読む（テスト用の読み込み）。
    fn parse_phone_sci(args: &[&str]) -> PhoneSciCmd {
        use clap::Parser as _;
        let mut all: Vec<String> =
            ["sai-moon-paper", "phone", "sci"].iter().map(|s| (*s).to_string()).collect();
        all.extend(args.iter().map(|s| (*s).to_string()));
        let cli = crate::Cli::try_parse_from(all).unwrap();
        let Some(crate::Cmd::Phone { cmd: crate::phone::PhoneCmd::Sci { cmd } }) = cli.cmd else {
            panic!("phone sci として読めた");
        };
        cmd
    }
}
