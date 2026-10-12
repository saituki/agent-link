//! `sai-sci onto` — 医療の ontology（MONDO・MeSH）を読み取り専用で引く命令。
//!
//! 共有データ（環境変数 `SAI_SHARED_DIR` の `onto.db`）だけを開く。研究プロジェクトは要らない。
//! 失敗の終了コードは CLI 共通の約束（2・3）に直してから返す。

use anyhow::Result;
use clap::Subcommand;
use sai_sci_core::onto::{Onto, OntoError};

use crate::cli_error::CliError;

/// `sai-sci onto` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum OntoCmd {
    /// 語を引く（別名を含む。完全一致→前方一致→部分一致の順）
    Lookup {
        /// 引く語（全角英数・大文字小文字は同じに扱う）
        term: String,
        /// 絞る source（無指定は両方）
        #[arg(long, value_parser = ["mondo", "mesh"])]
        source: Option<String>,
        /// 件数（既定 10・最大 50）
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 概念 1 件の詳細（親・子・別名・xref・根までの距離）
    Show {
        /// 概念の ID（`MONDO:<数字>` か `MESH:<UI>`）
        id: String,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 2 概念の関係（same・descendant・ancestor・unrelated・different_source）
    Relation {
        /// 1 つ目の ID
        id_a: String,
        /// 2 つ目の ID
        id_b: String,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 祖先を最短距離の昇順で出す
    Ancestors {
        /// 概念の ID
        id: String,
        /// この段数までに絞る
        #[arg(long)]
        max_depth: Option<usize>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
}

/// `sai-sci onto` を実行する。
pub fn handle(cmd: OntoCmd) -> Result<()> {
    match cmd {
        OntoCmd::Lookup {
            term,
            source,
            limit,
            json,
        } => {
            let out = run(|onto| onto.lookup(&term, source.as_deref(), limit.clamp(1, 50)))?;
            if json {
                println!("{}", serde_json::to_string(&out)?);
                return Ok(());
            }
            if out.candidates.is_empty() {
                println!("該当する概念はありません: {}", out.query);
            }
            for c in &out.candidates {
                println!(
                    "{}  {}  {}  {}  （一致: {}）",
                    c.id, c.source, c.kind, c.label, c.matched
                );
            }
        }
        OntoCmd::Show { id, json } => {
            let out = run(|onto| onto.show(&id))?;
            if json {
                println!("{}", serde_json::to_string(&out)?);
                return Ok(());
            }
            println!("{}  {}  {}", out.id, out.source, out.label);
            println!("別名: {}", out.synonyms.join("、"));
            println!("親: {}", join_links(&out.parents));
            println!("子: {} 件", out.children.total);
            println!("xref: {}", out.xref.join("、"));
            println!("根までの距離（参考）: {}", opt_num(out.depth));
        }
        OntoCmd::Relation { id_a, id_b, json } => {
            let out = run(|onto| onto.relation(&id_a, &id_b))?;
            if json {
                println!("{}", serde_json::to_string(&out)?);
                return Ok(());
            }
            println!(
                "{}（{}）と {}（{}）: {}（距離 {}）",
                out.a.id,
                out.a.label,
                out.b.id,
                out.b.label,
                out.relation,
                opt_num(out.distance)
            );
            for n in &out.nearest_common_ancestors {
                println!(
                    "  共通の祖先: {}（{}）a={} b={}",
                    n.id, n.label, n.a_distance, n.b_distance
                );
            }
        }
        OntoCmd::Ancestors {
            id,
            max_depth,
            json,
        } => {
            let out = run(|onto| onto.ancestors(&id, max_depth))?;
            if json {
                println!("{}", serde_json::to_string(&out)?);
                return Ok(());
            }
            for a in &out.ancestors {
                println!("{}  {}  {}", a.distance, a.id, a.label);
            }
        }
    }
    Ok(())
}

/// 共有データを開いて命令を 1 つ実行し、失敗を約束の終了コード（2・3）に直す。
fn run<T>(f: impl FnOnce(&Onto) -> Result<T>) -> Result<T> {
    Onto::from_env()
        .and_then(|onto| f(&onto))
        .map_err(exit_code_error)
}

/// `onto` のエラーを CLI の約束の終了コードのエラーにする。それ以外はそのまま（終了コード 1）。
fn exit_code_error(err: anyhow::Error) -> anyhow::Error {
    match err.downcast_ref::<OntoError>() {
        Some(OntoError::Input(m)) => CliError::input(m.clone()).into(),
        Some(OntoError::NotFound(m)) => CliError::not_found(m.clone()).into(),
        None => err,
    }
}

/// 概念の並び（`ID（label）`）を読点でつなぐ。
fn join_links(links: &[sai_sci_core::onto::Link]) -> String {
    links
        .iter()
        .map(|l| format!("{}（{}）", l.id, l.label))
        .collect::<Vec<_>>()
        .join("、")
}

/// 距離（無ければ「なし」）。
fn opt_num(value: Option<i64>) -> String {
    value.map_or_else(|| "なし".to_string(), |n| n.to_string())
}
