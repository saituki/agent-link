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
use sai_paper_core::sci::{self, card, ops, store};
use serde_json::{Value, json};

use crate::sci::{open_env, parse_assume, parse_id, parse_stdin_text, proposal_json};

/// `sai-moon-paper phone sci` のサブコマンド（C-09）。
#[derive(Subcommand, Debug)]
pub enum PhoneSciCmd {
    /// 現況を JSON で返す（`sci status --json` と同じ）
    Status,
    /// 研究方針の木を始める
    Init {
        /// RESEARCH_TYPE（problem / phenomenon / method / data / improve）
        #[arg(long = "type", value_name = "型")]
        type_id: String,
    },
    /// ROOT_GRAPH の論文の葉を JSON で返す
    Graph,
    /// NODE 1 つの詳細を JSON で返す
    Node {
        /// NODE の ID（`N12`）
        node: String,
    },
    /// 利用者の操作を 1 つ反映する（標準入力に C-07 の JSON）
    Apply,
    /// TRIAGE 待ちの PROPOSAL を JSON で返す
    Proposals,
    /// PROPOSAL を TRIAGE する（採用のときの中身の上書きは標準入力。空なら無し）
    Triage {
        /// PROPOSAL の ID（`P56`）
        proposal: String,
        /// TRIAGE（accept / reject / revert）
        #[arg(long)]
        action: String,
        /// TRIAGE のメモ
        #[arg(long)]
        note: Option<String>,
    },
    /// NODE のための実行（CARD）を作る（標準入力に C-09 の JSON）
    Card,
    /// READ_CARD の中身を JSON で返す
    Read {
        /// 実行 ID
        run_id: i64,
    },
    /// READ_CARD を終える
    FinishRead {
        /// 実行 ID
        run_id: i64,
        /// 反映しない理由（FINDING が無いとき必要）
        #[arg(long)]
        no_reflect: Option<String>,
    },
}

/// `sai-moon-paper phone sci` を実行する（出力と失敗は [`crate::phone::handle`] が扱う）。
pub fn run(cmd: PhoneSciCmd) -> Result<Value> {
    let input = read_stdin_for(&cmd)?;
    match cmd {
        PhoneSciCmd::Status => status(),
        PhoneSciCmd::Graph => graph(),
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

/// 標準入力を読むか（`apply`・`card`・採用の `triage` だけ）。
///
/// 採用のときだけ「直してから採用」の JSON を読む（C-09）。捨てと巻き戻しは
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
    let status = sci::compute::status(env.conn(), &env.roots, &env.defs)
        .map_err(|err| anyhow!("{err}"))
        .context("現況を集められませんでした")?;
    Ok(serde_json::to_value(status)?)
}

/// `phone sci graph` — 論文の葉（`{"papers": [...]}`）。
fn graph() -> Result<Value> {
    let env = open_env()?;
    let papers = sci::compute::paper_leaves(env.conn())
        .map_err(|err| anyhow!("{err}"))
        .context("論文の葉を集められませんでした")?;
    Ok(serde_json::json!({ "papers": papers }))
}

/// `phone sci init` — 部品を配って木を始める。
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

    // 部品（ブロック・テンプレート・`context`・土壌のひな形）を配り、木を作って書き出す。
    let outcome = crate::sci::init_with_assets(&env.roots.root, type_id, false)
        .map_err(|err| anyhow!("{err}"))
        .context("研究方針の木を作れませんでした")?;
    if outcome.already {
        anyhow::bail!(
            "既に始めています（型: {}）。型を変えるには `sai-moon-paper sci init --type <型> --change-type`",
            outcome.type_name
        );
    }

    Ok(json!({
        "type": outcome.type_id,
        "created_items": outcome.created_items,
        "created_edges": outcome.created_edges,
        "warnings": outcome.warnings,
        "backup": outcome.backup.as_ref().map(|path| path.to_string_lossy().to_string()),
        "assets": outcome.assets,
    }))
}

/// `phone sci node <N>`。
fn node_detail(node: &str) -> Result<Value> {
    let env = open_env()?;
    let id = parse_id(node, 'N')?;
    let detail = sci::compute::node_detail(env.conn(), &env.roots, &env.defs, id)
        .map_err(|err| anyhow!("{err}"))
        .context("NODE の詳細を集められませんでした")?;
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
        .context("PROPOSAL を読めませんでした")?;
    let list: Vec<Value> = rows.iter().map(proposal_json).collect();
    Ok(json!({ "proposals": list }))
}

/// `phone sci triage <P> --action …` — 仕分け。
fn triage(proposal: &str, action: &str, note: Option<&str>, input: &str) -> Result<Value> {
    let env = open_env()?;
    let id = parse_id(proposal, 'P')?;
    let action = triage_action(action, input)?;
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

/// 仕分けの指定を action にする（採用のときだけ標準入力の「直してから採用」を重ねる）。
fn triage_action(action: &str, input: &str) -> Result<ops::TriageAction> {
    match action {
        "accept" => Ok(ops::TriageAction::Accept { edit: edit_from_input(input)? }),
        "reject" => Ok(ops::TriageAction::Reject),
        "revert" => Ok(ops::TriageAction::Revert),
        other => anyhow::bail!("知らない TRIAGE です: {other}（accept / reject / revert）"),
    }
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
        // Web からは文書を受け取らない（C-05 の `--input` は CLI の引数だけ）。
        inputs: Vec::new(),
        // Web の画面から作るカードは人が起こすもの（professor が起こすカードは CLI だけ）。
        executer: String::new(),
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

    /// 採用のときだけ標準入力の「直してから採用」の JSON を読む。
    ///
    /// 捨てと巻き戻しのときは標準入力が閉じていなくても困るので読まない。
    #[test]
    fn reads_stdin_only_where_it_is_needed() {
        assert!(reads_stdin(&parse_phone_sci(&["apply"])));
        assert!(reads_stdin(&parse_phone_sci(&["card"])));
        assert!(reads_stdin(&parse_phone_sci(&["triage", "P1", "--action", "accept"])));
        // 捨てと巻き戻しは読まない（端末のままだと標準入力が終わらない）。
        assert!(!reads_stdin(&parse_phone_sci(&["triage", "P1", "--action", "reject"])));
        assert!(!reads_stdin(&parse_phone_sci(&["triage", "P1", "--action", "revert"])));
        // 読む必要の無い命令は読まない。
        assert!(!reads_stdin(&parse_phone_sci(&["status"])));
        assert!(!reads_stdin(&parse_phone_sci(&["proposals"])));
        assert!(!reads_stdin(&parse_phone_sci(&["init", "--type", "problem"])));
        assert!(!reads_stdin(&parse_phone_sci(&["read", "7"])));
    }

    /// 採用のときだけ標準入力の「直してから採用」が action に入る。
    #[test]
    fn triage_action_accept_carries_the_edit_from_stdin() {
        let action = triage_action(
            "accept",
            "{\"op\":\"set_title\",\"node\":\"N12\",\"title\":\"別の題\"}",
        )
        .unwrap();
        let ops::TriageAction::Accept { edit: Some(edit) } = &action else {
            panic!("採用の action になってほしい: {action:?}");
        };
        assert_eq!(edit["op"], "set_title");
        assert_eq!(edit["title"], "別の題");

        // 標準入力が空なら上書き無し。
        assert_eq!(triage_action("accept", "").unwrap(), ops::TriageAction::Accept { edit: None });

        // 捨てと巻き戻しは上書きを読まない（accept だけ）。
        assert_eq!(
            triage_action("reject", "{\"op\":\"set_title\"}").unwrap(),
            ops::TriageAction::Reject
        );
        assert_eq!(
            triage_action("revert", "{\"op\":\"set_title\"}").unwrap(),
            ops::TriageAction::Revert
        );

        // 知らない仕分けは日本語で断つ。
        let err = triage_action("取り消し", "").unwrap_err();
        assert!(format!("{err:#}").contains("知らない TRIAGE です"), "{err:#}");
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
