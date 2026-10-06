//! `sai-moon-paper phone` — sai-phone（iPhone 窓口）向けの JSON 出力。
//!
//! 全サブコマンドが `Result<serde_json::Value>` を返す内部関数にまとまっており、
//! `handle` が 1 か所で出力と失敗処理を担う。
//!
//! - `Ok(v)` → JSON 1 行を標準出力へ
//! - `Err(e)` → `{"error": "<メッセージ>"}` を標準出力へ出し、終了コード 1
//!
//! 読み取り用サブコマンド（library / tree / paper / log / providers）は DB に
//! 書き込まない（02 の計画）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use chrono::Datelike;
use clap::Subcommand;
use rusqlite::OptionalExtension;
use sai_paper_core::citation;
use sai_paper_core::citation::db::get_refs_for;
use sai_paper_core::{
    Config, Database, LibraryIndexer, LibraryPaper, Paths, S2Client, figures, find_log,
    library_tree, paths, podcast_client, search_log, search_sets, translation,
};
use serde_json::{Value, json};

use crate::open_database;
use crate::sci_bridge;

/// `api::PaperSourceResolver::resolve` が受け付ける DB ID（resolver を参照）。
const AVAILABLE_PROVIDERS: [&str; 6] = ["s2", "pubmed", "biorxiv", "medrxiv", "openalex", "arxiv"];

/// `sai-moon-paper phone` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum PhoneCmd {
    /// Library の論文一覧（DB 登録日の新しい順）
    Library,
    /// Library の階層表示（検索ログごと → 日付ごとの Import）
    Tree,
    /// 論文 1 件の詳細（ファイル・メモ・引用文献・音声化状態つき）
    Paper {
        /// 論文 ID（papers.id）
        paper_id: i64,
    },
    /// 検索履歴 1 件とヒット一覧
    Log {
        /// 検索ログ ID（search_logs.id）
        log_id: i64,
    },
    /// 検索に使う DB 一覧（既定の DB と、使える全 DB）
    Providers,
    /// 論文を検索し、TUI の Discover と同じ検索履歴に記録する
    Search {
        /// 検索ワード
        query: String,
        /// 検索する DB（繰り返し指定可。無ければ config の既定）
        #[arg(long)]
        db: Vec<String>,
        /// 開始年（指定年の下限）
        #[arg(long)]
        year_from: Option<u16>,
        /// 終了年（指定年の上限）
        #[arg(long)]
        year_to: Option<u16>,
        /// 取得件数（既定 20、1〜100 に丸める）
        #[arg(long, default_value_t = 20)]
        limit: u16,
    },
    /// 要旨を日本語に訳して保存する（TUI の `a` / `A` キーと同じ）
    Translate {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// 訳済みでももう一度訳す
        #[arg(long)]
        force: bool,
    },
    /// 音声化する（TUI の `r` キーと同じ。数分かかり、終わるまで戻らない）
    Radio {
        /// 論文 ID（papers.id）
        paper_id: i64,
    },
    /// PDF を日本語に訳す（TUI の `T` キーと同じ。数分かかり、終わるまで戻らない）
    #[command(name = "translate-pdf")]
    TranslatePdf {
        /// 論文 ID（papers.id）
        paper_id: i64,
    },
    /// Dashboard（TUI と同じ数字カード＋ Radio の出来事を混ぜた RECENT ACTIVITY）
    Dashboard,
    /// ノート（note.md）を標準入力の中身で保存する（無ければ TUI の `n` と同じ場所に作る）
    Note {
        /// 論文 ID（papers.id）
        paper_id: i64,
    },
    /// 現況（state.md）・考え（mind.md）・状況のまとめ・要対応をまとめて出す（S05）
    Project,
    /// mind.md の「## メモ」に標準入力の 1 行を足す（S05。書き込みはここだけ）
    #[command(name = "mind-append")]
    MindAppend,
    /// BibTeX の取り込み（sai-phone の Import 画面用）
    Import {
        #[command(subcommand)]
        cmd: crate::phone_import::PhoneImportCmd,
    },
    /// 論文の PDF を取得して Library に登録する（sai-phone の Download 画面用）
    #[command(group(
        clap::ArgGroup::new("target").required(true).multiple(false).args(["log", "import_id", "cite"]),
    ))]
    Download {
        /// 検索履歴 ID（`--position` とセットで指定）
        #[arg(long, requires = "position")]
        log: Option<i64>,
        /// 検索履歴の中での位置（0 始まり。`--log` とセットで指定）
        #[arg(long, requires = "log")]
        position: Option<i64>,
        /// BibTeX からの `RemotePaper.id`（例: `ArXiv:1706.03762`）
        #[arg(long = "import-id")]
        import_id: Option<String>,
        /// 引用を持つ論文 ID（`--ref` とセットで指定）
        #[arg(long)]
        cite: Option<i64>,
        /// 引用の番号（`--cite` とセットで指定）
        #[arg(long = "ref", requires = "cite")]
        ref_no: Option<i64>,
    },
    /// 引用一覧（読み取りだけ。抜き出す前の論文にも空の形で返る）
    Citations {
        /// 論文 ID（papers.id）
        paper_id: i64,
    },
    /// 引用を PDF から抜き出す（docling を使うので数十秒。`sai-progress step=1` を出す）
    #[command(name = "citations-extract")]
    CitationsExtract {
        /// 論文 ID（papers.id）
        paper_id: i64,
    },
    /// ダウンロードフォルダの最近の PDF 一覧（手動取り込みの候補）
    #[command(name = "manual-candidates")]
    ManualCandidates {
        /// 何分前まで候補に出すか（既定 5 分）
        #[arg(long, default_value_t = 5)]
        minutes: u64,
    },
    /// 手動で保存した PDF（またはアップロード）を Library に入れる
    #[command(name = "manual-import")]
    ManualImport {
        #[command(flatten)]
        args: crate::phone_cite::ManualImportArgs,
    },
    /// Download の後の自動処理（図の抽出 → PDF の翻訳。`phone download` のあとで呼ぶ）
    AfterDownload {
        /// 論文 ID（papers.id）
        paper_id: i64,
    },
    /// 要対応の 1 件を「対応済み」にする（`sai-moon-paper attention resolve` と同じ。JSON で返す）
    AttentionResolve {
        /// 対応済みにする key（例: `question:41:対象は?`）
        key: String,
    },
    /// 研究方針の木（phone sci。古い別名。`sai-sci` に渡す）
    Sci {
        #[command(subcommand)]
        cmd: PhoneSciCmd,
    },
    /// 検索セット（一覧・候補・選別。`sai-moon-paper set …` と同じ核を呼ぶ）
    Set {
        #[command(subcommand)]
        cmd: PhoneSetCmd,
    },
    /// Library の中を検索し、内部検索の履歴に残す（U01。要約なし）
    Find {
        /// 検索モード（word / vector / hybrid。無ければ設定の既定）
        #[arg(long)]
        mode: Option<String>,
        /// 上位 K 件
        #[arg(long)]
        top_k: Option<i64>,
        /// 検索対象（papers / notes / figures / all。無ければ設定の既定）
        #[arg(long)]
        scope: Option<String>,
        /// 論文 ID（指定時はその論文に絞り込む。繰り返し指定可）
        #[arg(long)]
        paper: Vec<i64>,
        /// 研究方針の Node の ID（`Q12`・`O5` など）。その Node の下の F の論文に絞る
        #[arg(long)]
        node: Option<String>,
        /// `--node` の子孫の Node も含める
        #[arg(long)]
        subtree: bool,
        /// 範囲の論文ごとに上位 M 件を返す（`--paper` か `--node` と一緒に使う）
        #[arg(long)]
        each: Option<i64>,
        /// 検索ワード
        query: String,
    },
    /// 内部検索の履歴 1 件とヒット一覧（U01）
    #[command(name = "find-log")]
    FindLog {
        /// 内部検索の履歴 ID（find_logs.id）
        log_id: i64,
    },
}

/// `sai-moon-paper phone sci` のサブコマンド（古い別名。`sai-sci` に渡す。ガイド 4 で消す）。
#[derive(Subcommand, Debug)]
pub enum PhoneSciCmd {
    /// 現況を JSON で返す
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
    /// 提案を仕分ける（採用のときの中身の上書きは標準入力）
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

/// `sai-moon-paper phone set` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum PhoneSetCmd {
    /// セット一覧（判定ごとの件数つき）
    List,
    /// セット 1 つと候補の一覧
    Show {
        /// セット ID
        id: i64,
    },
    /// 候補の判定を付ける（`imported` の候補は変えない）
    Mark {
        /// セット ID
        id: i64,
        /// 位置（0 始まり。コンマ区切りで複数）
        #[arg(long, value_delimiter = ',', required = true)]
        position: Vec<i64>,
        /// 判定（keep / drop）
        #[arg(long)]
        decision: String,
        /// 理由
        #[arg(long)]
        reason: Option<String>,
    },
    /// `pending` の候補をすべて drop にする
    #[command(name = "drop-pending")]
    DropPending {
        /// セット ID
        id: i64,
        /// 理由（既定は「まとめて drop」）
        #[arg(long)]
        reason: Option<String>,
    },
}

/// `sai-moon-paper phone` を実行し、JSON 1 行を標準出力へ出す。
///
/// 失敗はどのサブコマンドでも同じ形（`{"error": …}` + 終了コード 1）で返す。
pub async fn handle(cmd: PhoneCmd, client: &S2Client) -> Result<()> {
    // 重い処理の記録（jobs）に、この窓口を phone として残す。
    sai_paper_core::jobs::set_origin("phone");
    let outcome = match cmd {
        PhoneCmd::Library => library(),
        PhoneCmd::Tree => tree(),
        PhoneCmd::Paper { paper_id } => paper(paper_id),
        PhoneCmd::Log { log_id } => {
            let (db, cfg, paths) = open_context()?;
            log(&db, &cfg, &paths, log_id)
        }
        PhoneCmd::Providers => providers(),
        PhoneCmd::Search {
            query,
            db,
            year_from,
            year_to,
            limit,
        } => search(&query, &db, year_from, year_to, limit).await,
        PhoneCmd::Translate { paper_id, force } => translate(paper_id, force).await,
        PhoneCmd::Radio { paper_id } => radio(paper_id).await,
        PhoneCmd::TranslatePdf { paper_id } => translate_pdf(paper_id).await,
        PhoneCmd::Dashboard => dashboard(),
        PhoneCmd::Note { paper_id } => save_note(paper_id),
        PhoneCmd::Project => sci_bridge::run_json(&sci_args(&["project", "show", "--json"]), None),
        PhoneCmd::MindAppend => {
            // 標準入力の 1 行を `project memo` の引数にする。
            use std::io::BufRead as _;
            let mut line = String::new();
            let _ = std::io::stdin().lock().read_line(&mut line);
            let text = line.trim_end_matches(['\n', '\r']);
            sci_bridge::run_json(&sci_args(&["project", "memo", text, "--json"]), None)
        }
        PhoneCmd::Import { cmd } => crate::phone_import::run_import(cmd),
        PhoneCmd::Download {
            log,
            position,
            import_id,
            cite,
            ref_no,
        } => {
            crate::phone_import::download(
                client,
                log,
                position,
                import_id.as_deref(),
                cite,
                ref_no,
            )
            .await
        }
        PhoneCmd::Citations { paper_id } => crate::phone_cite::citations(paper_id),
        PhoneCmd::CitationsExtract { paper_id } => {
            crate::phone_cite::citations_extract(paper_id, client).await
        }
        PhoneCmd::ManualCandidates { minutes } => crate::phone_cite::manual_candidates(minutes),
        PhoneCmd::ManualImport { args } => crate::phone_cite::manual_import_command(&args),
        PhoneCmd::AfterDownload { paper_id } => crate::phone_import::after_download(paper_id).await,
        PhoneCmd::AttentionResolve { key } => sci_bridge::run_json(
            &sci_args(&["attention", "resolve", &key, "--json"]),
            None,
        ),
        PhoneCmd::Sci { cmd } => phone_sci_bridge(cmd),
        PhoneCmd::Set { cmd } => set_run(cmd),
        PhoneCmd::Find {
            mode,
            top_k,
            scope,
            paper,
            node,
            subtree,
            each,
            query,
        } => {
            // Node に結び付ける検索は sai-sci が行う（古い別名。ガイド 4 で消す）。
            if node.is_some() || subtree {
                let mut args = vec!["find".to_string(), query.clone()];
                push_opt(&mut args, "--mode", mode.as_deref());
                if let Some(top_k) = top_k {
                    args.push("--top-k".to_string());
                    args.push(top_k.to_string());
                }
                push_opt(&mut args, "--scope", scope.as_deref());
                for one in &paper {
                    args.push("--paper".to_string());
                    args.push(one.to_string());
                }
                push_opt(&mut args, "--node", node.as_deref());
                if subtree {
                    args.push("--subtree".to_string());
                }
                if let Some(each) = each {
                    args.push("--each".to_string());
                    args.push(each.to_string());
                }
                args.push("--json".to_string());
                sci_bridge::run_json(&args, None);
            }
            find(FindArgs {
                mode: &mode,
                top_k,
                scope: &scope,
                papers: &paper,
                each,
                query: &query,
            })
            .await
        }
        PhoneCmd::FindLog { log_id } => {
            let (db, _cfg, _paths) = open_context()?;
            find_log_view(&db, log_id)
        }
    };
    match outcome {
        Ok(value) => {
            println!("{}", serde_json::to_string(&value)?);
            Ok(())
        }
        Err(err) => {
            println!("{}", json!({ "error": format!("{err:#}") }));
            std::process::exit(1);
        }
    }
}

// ---- `sai-sci` への別名（ガイド 4 で消す） ------------------------------------

/// 文字列の並びを `sai-sci` に渡す引数にする。
fn sci_args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_string()).collect()
}

/// 値のある任意の引数を足す。
fn push_opt(out: &mut Vec<String>, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        out.push(name.to_string());
        out.push(value.to_string());
    }
}

/// 標準入力の中身を読む（端末のままなら読まずに空を返す。書く命令だけが使う）。
fn read_stdin_text() -> String {
    use std::io::{IsTerminal as _, Read as _};
    if std::io::stdin().is_terminal() {
        return String::new();
    }
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    text
}

/// `phone sci` を `sai-sci` の子として呼び、JSON をそのまま出す（形 A）。
fn phone_sci_bridge(cmd: PhoneSciCmd) -> ! {
    match cmd {
        PhoneSciCmd::Status => sci_bridge::run_json(&sci_args(&["status", "--json"]), None),
        PhoneSciCmd::Types => sci_bridge::run_json(&sci_args(&["types", "--json"]), None),
        PhoneSciCmd::Proposals => sci_bridge::run_json(&sci_args(&["proposals", "--json"]), None),
        PhoneSciCmd::Node { node } => {
            sci_bridge::run_json(&sci_args(&["node", &node, "--json"]), None)
        }
        PhoneSciCmd::Read { run_id } => {
            let run = run_id.to_string();
            sci_bridge::run_json(&sci_args(&["read", &run, "--json"]), None)
        }
        PhoneSciCmd::Init { type_id } => {
            sci_bridge::run_init_tree(&sci_args(&["init", "--type", &type_id, "--json"]))
        }
        PhoneSciCmd::Apply => {
            let input = read_stdin_text();
            sci_bridge::run_json(&sci_args(&["apply", "--json"]), Some(&input))
        }
        PhoneSciCmd::Card => {
            let input = read_stdin_text();
            sci_bridge::run_json(&sci_args(&["card", "--from-json", "--json"]), Some(&input))
        }
        PhoneSciCmd::Triage {
            proposal,
            action,
            note,
        } => {
            let mut out = vec!["triage".to_string(), proposal];
            match action.as_str() {
                "accept" => {
                    out.push("--accept".to_string());
                    let edit = read_stdin_text();
                    if !edit.trim().is_empty() {
                        out.push(format!("--edit={edit}"));
                    }
                }
                "reject" => out.push("--reject".to_string()),
                "revert" => out.push("--revert".to_string()),
                other => {
                    // 知らない仕分けは、今の文で形 A の失敗にする。
                    println!(
                        "{}",
                        json!({ "error": format!("知らない仕分けです: {other}（accept / reject / revert）") })
                    );
                    std::process::exit(1);
                }
            }
            if let Some(note) = note {
                out.push(format!("--note={note}"));
            }
            sci_bridge::run_json(&out, None)
        }
    }
}

// ---- 検索セット（phone set） ------------------------------------------------

/// `phone set …` の入口。中身は `sai_paper_core::search_sets` を呼ぶだけ。
fn set_run(cmd: PhoneSetCmd) -> Result<Value> {
    let db = open_database()?;
    let conn = db.connection();
    match cmd {
        PhoneSetCmd::List => set_list(conn),
        PhoneSetCmd::Show { id } => set_show(conn, id),
        PhoneSetCmd::Mark {
            id,
            position,
            decision,
            reason,
        } => set_mark(conn, id, &position, &decision, reason.as_deref()),
        PhoneSetCmd::DropPending { id, reason } => set_drop_pending(conn, id, reason.as_deref()),
    }
}

/// 取り込み済みの判定（保存値は `downloaded`）。
fn is_imported(item: &search_sets::SearchSetItem) -> bool {
    item.decision == "downloaded"
}

/// セット 1 件の JSON（`rt_node`・`purpose`・`extract` は核の関数が無いのでここで読む）。
pub(crate) fn set_json(conn: &rusqlite::Connection, set: &search_sets::SearchSet) -> Result<Value> {
    let (rt_node, purpose, extract): (Option<i64>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT rt_node, purpose, extract FROM search_sets WHERE id = ?1",
            [set.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    // 木から消えた Node も、`rt_label` は今までどおり返す（`get_node` と同じ）。
    let node = rt_node.and_then(|id| sai_paper_core::search_sets::rt_label(conn, id));
    let items = search_sets::items(conn, set.id)?;
    let count = |name: &str| items.iter().filter(|i| i.decision == name).count();
    Ok(json!({
        "id": set.id,
        "name": set.name,
        "query": set.query,
        "source_dbs": set.source_dbs,
        "created_at": set.created_at,
        "node": node,
        "purpose": purpose,
        "extract": extract,
        "counts": {
            "pending": count("pending"),
            "keep": count("keep"),
            "drop": count("drop"),
            "imported": items.iter().filter(|i| is_imported(i)).count(),
            "total": items.len(),
        },
    }))
}

/// `phone set list`。
pub(crate) fn set_list(conn: &rusqlite::Connection) -> Result<Value> {
    let mut sets = Vec::new();
    for set in search_sets::list(conn)? {
        let mut one = set_json(conn, &set)?;
        // 一覧には長い `extract` を出さない（`show` で見る）
        one.as_object_mut().map(|o| o.remove("extract"));
        sets.push(one);
    }
    Ok(json!({ "sets": sets }))
}

/// `phone set show <id>`。
pub(crate) fn set_show(conn: &rusqlite::Connection, id: i64) -> Result<Value> {
    let set = search_sets::get(conn, id)?;
    let items: Vec<Value> = search_sets::items(conn, id)?
        .into_iter()
        .map(|item| {
            json!({
                "position": item.position,
                "title": item.remote.title,
                "authors": item.remote.authors,
                "year": remote_year(&item.remote.published),
                "venue": item.remote.journal_ref,
                "doi": item.remote.doi,
                "abstract": item.remote.abstract_text,
                "decision": item.decision,
                "reason": item.reason,
                "paper_id": item.paper_id,
            })
        })
        .collect();
    Ok(json!({ "set": set_json(conn, &set)?, "items": items }))
}

/// 位置の列に判定を付ける。取り込み済みは変えず、数えない。
pub(crate) fn mark_positions(
    conn: &rusqlite::Connection,
    id: i64,
    positions: &[i64],
    decision: &str,
    reason: Option<&str>,
) -> Result<Value> {
    if decision != "keep" && decision != "drop" {
        return Err(crate::cli_error::CliError::input(
            "decision は keep か drop にしてください",
        )
        .into());
    }
    // 無いセットは先に断る
    search_sets::get(conn, id)?;
    let items = search_sets::items(conn, id)?;
    let mut changed = 0;
    for &position in positions {
        let Some(item) = items.iter().find(|i| i.position == position) else {
            return Err(crate::cli_error::CliError::not_found(format!(
                "候補 set_id={id}, position={position} が見つかりません"
            ))
            .into());
        };
        if is_imported(item) {
            continue;
        }
        search_sets::mark(conn, id, position, decision, reason)?;
        changed += 1;
    }
    Ok(json!({ "set_id": id, "changed": changed }))
}

/// `phone set mark`。
fn set_mark(
    conn: &rusqlite::Connection,
    id: i64,
    positions: &[i64],
    decision: &str,
    reason: Option<&str>,
) -> Result<Value> {
    mark_positions(conn, id, positions, decision, reason)
}

/// `phone set drop-pending`。
pub(crate) fn set_drop_pending(conn: &rusqlite::Connection, id: i64, reason: Option<&str>) -> Result<Value> {
    search_sets::get(conn, id)?;
    let pending: Vec<i64> = search_sets::items(conn, id)?
        .into_iter()
        .filter(|i| i.decision == "pending")
        .map(|i| i.position)
        .collect();
    mark_positions(conn, id, &pending, "drop", Some(reason.unwrap_or("まとめて drop")))
}

/// 読み取り系サブコマンド共通の準備（DB・config・パス）。
pub(crate) fn open_context() -> Result<(Database, Config, Paths)> {
    let db = open_database()?;
    let paths = Paths::discover()?;
    let cfg = Config::load_or_create(&paths)?;
    Ok((db, cfg, paths))
}

/// Library の論文一覧（`phone library` / `tree` / `log` で共通、`phone download` も使う）。
///
/// TUI の Library 表示と同じ対象フォルダで絞り込む。並びは DB の
/// `created_at` の新しい順（同刻は ID の新しい順）。
pub(crate) fn library_papers(
    db: &Database,
    cfg: &Config,
    paths: &Paths,
) -> Result<Vec<LibraryPaper>> {
    let roots = library_tree::library_roots(cfg, paths);
    let mut papers = db.library_papers_in_roots(&roots)?;
    let created = created_at_map(db)?;
    papers.sort_by(|a, b| {
        let key_a = (created.get(&a.id).map(String::as_str).unwrap_or(""), a.id);
        let key_b = (created.get(&b.id).map(String::as_str).unwrap_or(""), b.id);
        key_b.cmp(&key_a)
    });
    Ok(papers)
}

/// 論文 ID → DB 登録時刻（`papers.created_at` を 1 回だけ読む）。
fn created_at_map(db: &Database) -> Result<HashMap<i64, String>> {
    let mut stmt = db.connection().prepare("SELECT id, created_at FROM papers")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
}

/// 論文 ID → 音声化ステータス（`paper_podcasts.status` を 1 回だけ読む）。
fn podcast_status_map(db: &Database) -> Result<HashMap<i64, String>> {
    let mut stmt = db
        .connection()
        .prepare("SELECT paper_id, status FROM paper_podcasts")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
}

/// 実在するファイルの絶対パス（無ければ `None`）。
fn abs_if_file(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    Some(absolute.to_string_lossy().into_owned())
}

/// 論文の抽出テキスト（extract.md）の場所（実在するものだけ）。
///
/// TUI / `content paper` と同じ解決順（引用抽出の記録 → 論文フォルダ →
/// 旧 `extracts/`）を優先し、無ければ論文フォルダ（`paths::paper_folder`）も試す
/// （extract.md が 2 系統に分かれている実データに対応するため）。
fn extract_file(db: &Database, paper: &LibraryPaper, data_dir: &Path) -> Option<PathBuf> {
    citation::resolve_extract_md(db.connection(), Some(paper.id), None, data_dir).or_else(|| {
        let roots = paths::paper_roots()?;
        let dir = paths::paper_folder(&roots.data_dir, paper.id, &paper.title);
        let file = paths::paper_file_path(&dir, "extract");
        file.is_file().then_some(file)
    })
}

/// 論文のノート（note.md）の場所（実在するものだけ）。
///
/// 論文フォルダ（`paths::paper_folder`。段階 2a で置き場はここだけ）→
/// PDF のあるフォルダ（DB の絶対パス。段階 2b で整理）の順に探す。
fn note_file(paths: &Paths, paper: &LibraryPaper) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    dirs.push(paths::paper_folder(
        &paths.data_dir(),
        paper.id,
        &paper.title,
    ));
    if let Some(parent) = paper
        .pdf_path
        .as_deref()
        .map(Path::new)
        .and_then(Path::parent)
    {
        dirs.push(parent.to_path_buf());
    }
    dirs.into_iter()
        .map(|dir| paths::paper_file_path(&dir, "note"))
        .find(|file| file.is_file())
}

/// `published_at` 文字列から年の 4 桁文字列を作る（00 §7.2）。
///
/// 先頭 4 文字が数字で読めない、または 1900 未満（未設定の既定値）なら `None`（JSON `null`）。
fn published_year(published_at: Option<&str>) -> Option<String> {
    let raw = published_at?.trim();
    let year: u16 = raw.get(..4)?.parse().ok()?;
    if year < 1900 { None } else { Some(year.to_string()) }
}

/// リモート論文の出版年（`RemotePaper.published`）。1900 未満（未設定の既定値）なら `None`。
pub(crate) fn remote_year(published: &chrono::DateTime<chrono::Utc>) -> Option<String> {
    let year = published.year();
    if (1900..=9999).contains(&year) {
        Some(year.to_string())
    } else {
        None
    }
}

/// `phone library`: Library の論文一覧（00 §7.2）。
///
/// `created_at` は `papers` から、`podcast` は `paper_podcasts` から 1 回ずつ読む。
pub(crate) fn library() -> Result<Value> {
    let (db, cfg, paths) = open_context()?;
    let papers = library_papers(&db, &cfg, &paths)?;
    let created = created_at_map(&db)?;
    let podcasts = podcast_status_map(&db)?;
    let data_dir = paths.data_dir();

    let mut items = Vec::with_capacity(papers.len());
    for paper in &papers {
        items.push(json!({
            "id": paper.id,
            "title": paper.title,
            "authors": paper.authors,
            "year": published_year(paper.published_at.as_deref()),
            "journal": paper.journal,
            "reading_status": paper.reading_status,
            "is_favorite": paper.is_favorite,
            "created_at": created.get(&paper.id),
            "has_pdf": library_tree::paper_pdf_exists(paper),
            "has_extract": extract_file(&db, paper, &data_dir).is_some(),
            "podcast": podcasts.get(&paper.id),
        }));
    }
    Ok(json!({ "papers": items }))
}

/// `phone tree`: 階層グループ一覧（TUI の階層表示と同じまとめ方）。
pub(crate) fn tree() -> Result<Value> {
    let (db, cfg, paths) = open_context()?;
    let papers = library_papers(&db, &cfg, &paths)?;
    let groups = library_tree::build_groups(db.connection(), &papers);
    // 内部検索の履歴（U01。新しい順。`groups` は変えない）。
    let find_groups = find_log::groups(db.connection(), &papers)?;
    Ok(json!({
        "groups": serde_json::to_value(groups)?,
        "find_groups": serde_json::to_value(find_groups)?,
    }))
}

/// `phone paper <id>`: 論文 1 件の詳細（00 §7.2）。
pub(crate) fn paper(paper_id: i64) -> Result<Value> {
    let (db, _cfg, paths) = open_context()?;
    let Some(paper) = db.library_paper_by_id(paper_id)? else {
        return Err(crate::cli_error::CliError::not_found("論文が見つかりません").into());
    };

    // s2_paper_id は LibraryPaper に無いため DB から直接読む。
    let s2_paper_id: Option<String> = db
        .connection()
        .query_row(
            "SELECT s2_paper_id FROM papers WHERE id = ?1",
            [paper_id],
            |row| row.get(0),
        )
        .optional()?;

    // files: 実在するファイルだけ絶対パス（無ければ null）。
    let pdf_path = paper
        .pdf_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let pdf = pdf_path.and_then(|p| abs_if_file(Path::new(p)));
    let pdf_ja = pdf_path
        .and_then(paths::translated_pdf_path)
        .and_then(|p| abs_if_file(&p));
    let data_dir = paths.data_dir();
    let extract = extract_file(&db, &paper, &data_dir).and_then(|p| abs_if_file(&p));
    let note_path = note_file(&paths, &paper);
    let note = note_path.as_deref().and_then(abs_if_file);

    // note: note.md の本文（trim して空なら null）。
    let note_text = note_path
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());

    // refs: 引用文献抽出済みのときだけ（無ければ []）。
    //    記録はまず papers_id で、無ければ抽出キーで探す（段階 2c）。
    //    JSON の形は変えない。
    let refs = {
        // paper_refs のキーは正規化済み（`ArXiv:…` / `DOI:…` / 40hex）なので揃える。
        let key = db
            .citation_key_for_paper(paper_id)
            .map(|raw_key| citation::normalize_paper_id(&raw_key));
        // local_paper_id 判定用の表を 1 回だけ読む。
        let mut stmt = db
            .connection()
            .prepare("SELECT id, s2_paper_id, doi FROM papers")?;
        let local_rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        get_refs_for(db.connection(), Some(paper_id), key.as_deref())?
            .into_iter()
            .map(|row| {
                let local_paper_id =
                    local_paper_id(&local_rows, row.s2_paper_id.as_deref(), row.doi.as_deref());
                json!({
                    "ref_no": row.ref_no,
                    "title": row.s2_title.or(row.title),
                    "authors": row.first_author,
                    "year": row.s2_year.or(row.year),
                    "journal": row.s2_venue.or(row.journal),
                    "doi": row.doi,
                    "s2_paper_id": row.s2_paper_id,
                    "local_paper_id": local_paper_id,
                })
            })
            .collect::<Vec<_>>()
    };

    // podcast: paper_podcasts の行（無ければ null）。
    // radio_id は音声の名前（<フォルダ名>-<番号>）。再生画面への飛び先に使う。
    let podcast = db
        .connection()
        .query_row(
            "SELECT status, generated_at, error FROM paper_podcasts WHERE paper_id = ?1",
            [paper_id],
            |row| {
                Ok(json!({
                    "status": row.get::<_, String>(0)?,
                    "generated_at": row.get::<_, Option<String>>(1)?,
                    "error": row.get::<_, Option<String>>(2)?,
                    "radio_id": podcast_client::radio_paper_id(paper_id),
                }))
            },
        )
        .optional()?;

    // figures: PAPER 画面の FIGURES と同じ中身（画像が消えた図は出さない）。
    // state は ready（図あり）/ empty（抽出済みで 0 件）/ not_extracted（未抽出・抽出中・失敗）。
    // image_path は data_dir からの相対なので、実パスへ直してから返す（段階 2b）。
    let (figure_state, figure_rows) =
        match figures::figure_state_for_paper(db.connection(), paper_id, &data_dir)? {
            figures::FigureState::Ready(rows) => ("ready", rows),
            figures::FigureState::Empty => ("empty", Vec::new()),
            figures::FigureState::NotExtracted => ("not_extracted", Vec::new()),
        };
    let figure_items: Vec<Value> = figure_rows
        .iter()
        .map(|f| {
            json!({
                "fig_no": f.fig_no,
                "label": f.label,
                "title": f.title,
                "caption": f.caption,
                "mention": f.mention,
                "page": f.page,
                "image": f.image_path,
            })
        })
        .collect();

    Ok(json!({
        "figures": { "state": figure_state, "items": figure_items },
        "paper": {
            "id": paper.id,
            "title": paper.title,
            "authors": paper.authors,
            "year": published_year(paper.published_at.as_deref()),
            "journal": paper.journal,
            "doi": paper.doi,
            "arxiv_id": paper.arxiv_id,
            "s2_paper_id": s2_paper_id,
            "citation_count": paper.citation_count,
            "reading_status": paper.reading_status,
            "abstract": paper.abstract_text,
            "abstract_ja": paper.abstract_ja,
        },
        "files": {
            "pdf": pdf,
            "pdf_ja": pdf_ja,
            "extract": extract,
            "note": note,
        },
        "note": note_text,
        "refs": refs,
        "podcast": podcast,
    }))
}

/// 引用文献に対応する Library 論文 ID（S2 ID → DOI（大文字小文字無視）の順に探す）。
fn local_paper_id(
    rows: &[(i64, Option<String>, Option<String>)],
    s2_paper_id: Option<&str>,
    doi: Option<&str>,
) -> Option<i64> {
    if let Some(s2) = s2_paper_id.map(str::trim).filter(|s| !s.is_empty())
        && let Some(hit) = rows.iter().find(|(_, s2_of_row, _)| {
            s2_of_row.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(s2))
        })
    {
        return Some(hit.0);
    }
    let doi = doi?.trim();
    if doi.is_empty() {
        return None;
    }
    rows.iter()
        .find(|(_, _, doi_of_row)| {
            doi_of_row
                .as_deref()
                .is_some_and(|d| d.eq_ignore_ascii_case(doi))
        })
        .map(|(id, _, _)| *id)
}

/// `phone log <log_id>`: 検索履歴とヒット一覧（00 §7.2）。
///
/// `phone search`（03）も記録直後のログをこの関数で出力する。
pub(crate) fn log(db: &Database, cfg: &Config, paths: &Paths, log_id: i64) -> Result<Value> {
    let conn = db.connection();
    let Some(log_meta) = search_log::list(conn)?
        .into_iter()
        .find(|entry| entry.id == log_id)
    else {
        return Err(crate::cli_error::CliError::not_found("検索履歴が見つかりません").into());
    };

    let papers = library_papers(db, cfg, paths)?;
    let mut items = Vec::new();
    for item in search_log::items(conn, log_id)? {
        let remote = item.remote;
        let local_paper_id = library_tree::match_downloaded_paper(&papers, &remote)
            .map(|index| papers[index].id);
        items.push(json!({
            "position": item.position,
            "id": remote.id,
            "title": remote.title,
            "authors": remote.authors,
            "year": remote_year(&remote.published),
            "journal": remote.journal_ref,
            "doi": remote.doi,
            "abstract": remote.abstract_text,
            "citation_count": remote.citation_count,
            "local_paper_id": local_paper_id,
        }));
    }

    let (actor, node) = search_actor(conn, log_id);
    Ok(json!({
        "log": {
            "id": log_meta.id,
            "searched_at": log_meta.searched_at,
            "date": log_meta.log_date,
            "query": log_meta.query,
            "year": log_meta.year,
            "source_dbs": log_meta.source_dbs,
            "actor": actor,
            "node": node,
        },
        "items": items,
    }))
}

/// 検索の実行者と、結び付いた研究方針の Node（`log.actor` / `log.node`）。
///
/// 検索の記録は `search_sets` の 1 行（`origin` が窓口）。agent の検索は `rt_searches` に
/// run_id と Node が残る（`set_id` が検索のログ ID）。読めないものは空にする（エラーにしない）。
/// - `actor`: `{"kind": "human"}` か `{"kind": "agent", "run_id": 数 | null, "block": 名前 | null}`
/// - `node`: `{"id": "O5"}`（Node が結び付いていなければ `null`）
fn search_actor(conn: &rusqlite::Connection, log_id: i64) -> (Value, Value) {
    let (origin, set_node, set_run): (Option<String>, Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT origin, rt_node, run_id FROM search_sets WHERE id = ?1",
            [log_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap_or((None, None, None));
    let rt: Option<(Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT run_id, node_id FROM rt_searches WHERE log_id = ?1 OR set_id = ?1 ORDER BY id LIMIT 1",
            [log_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    let run_id = set_run.or(rt.and_then(|(run, _)| run));
    let node_id = set_node.or(rt.and_then(|(_, node)| node));

    let actor = if origin.as_deref() == Some("agent") || run_id.is_some() {
        // block の名前は、sai-paper が sai-link のデータを読まないので出さない（null）。
        json!({ "kind": "agent", "run_id": run_id, "block": Value::Null })
    } else {
        json!({ "kind": "human" })
    };
    let node = node_id
        .and_then(|id| sai_paper_core::search_sets::rt_label(conn, id))
        .map(|label| json!({ "id": label }));
    (actor, node.unwrap_or(Value::Null))
}

/// `phone find` の引数（`PhoneCmd::Find` から写す。clippy の引数過多を避けるため）。
struct FindArgs<'a> {
    mode: &'a Option<String>,
    top_k: Option<i64>,
    scope: &'a Option<String>,
    papers: &'a [i64],
    each: Option<i64>,
    query: &'a str,
}

/// `phone find …`: Library の中を検索し、内部検索の履歴に残す（U01。要約なし）。
///
/// 出力は `phone find-log <新しい log_id>` と同じ形。結果 0 件でも履歴は残す。
/// 研究方針の Node（`--node`）が付いた検索は、別名として `sai-sci` が行う。
async fn find(args: FindArgs<'_>) -> Result<Value> {
    let FindArgs {
        mode,
        top_k,
        scope,
        papers,
        each,
        query,
    } = args;
    let query = query.trim();
    if query.is_empty() {
        return Err(crate::cli_error::CliError::input("検索語が空です").into());
    }
    if let Some(mode) = mode
        && !matches!(mode.as_str(), "word" | "vector" | "hybrid")
    {
        return Err(crate::cli_error::CliError::input(
            "検索モードが正しくありません（word / vector / hybrid）",
        )
        .into());
    }
    if let Some(scope) = scope
        && !matches!(scope.as_str(), "papers" | "notes" | "figures" | "all")
    {
        return Err(crate::cli_error::CliError::input(
            "検索対象が正しくありません（papers / notes / figures / all）",
        )
        .into());
    }
    // `--each` には範囲の指定が要る（Node の指定は別名が先に取る）。
    if each.is_some() && papers.is_empty() {
        return Err(crate::cli_error::CliError::input("--each は --paper と一緒に使います").into());
    }

    let (db, cfg, _paths) = open_context()?;
    let conn = db.connection();
    let (paper_ids, range_text) =
        sai_paper_core::retrieval::resolve_paper_scope(conn, None, papers)?;
    let widen = "";
    let hits = sai_paper_core::retrieval::find(
        conn,
        &cfg,
        sai_paper_core::retrieval::FindRequest {
            query: query.to_string(),
            mode: mode.clone(),
            top_k,
            per_paper: None,
            scope: scope.clone(),
            paper_id: None,
            paper_ids: paper_ids.clone(),
            each,
            index: None,
            alpha: None,
            fusion: None,
            fts_expr: false,
        },
    )
    .await
    .map_err(|err| anyhow::anyhow!("{err}"))?;
    let passages = sai_paper_core::sections::passages_for_hits(conn, &hits);
    let range_papers: Vec<i64> = paper_ids.unwrap_or_default();
    let log_id = find_log::record(
        conn,
        &find_log::FindLogInput {
            query,
            mode: mode.as_deref(),
            top_k,
            scope: scope.as_deref(),
            range_text: &range_text,
            papers: &range_papers,
            node_id: None,
            subtree: false,
            each,
            widen_reason: widen,
            origin: "phone",
            run_id: None,
        },
        &hits,
        &passages,
    )?;
    find_log_view(&db, log_id)
}

/// 内部検索の履歴 1 件を、使い回しの JSON（C-2）にする。
///
/// `phone find`（検索直後）と `phone find-log`（過去の履歴）で共用。
pub(crate) fn find_log_json(
    conn: &rusqlite::Connection,
    row: &find_log::FindLogRow,
    rows: &[(i64, Option<i64>, Value)],
) -> Value {
    // 実行者: `run_id` ありか `origin = 'mcp'` なら agent（`search_actor` と同じ作り）。
    let run_id = row.run_id;
    let actor = if find_log::is_agent(&row.origin, run_id) {
        // block の名前は、sai-paper が sai-link のデータを読まないので出さない（null）。
        json!({ "kind": "agent", "run_id": run_id, "block": Value::Null })
    } else {
        json!({ "kind": "human" })
    };
    // `find_log` に Node がなければ、`sai-sci find` が残した `rt_searches` から読む。
    let node_id = row.node_id.or_else(|| {
        conn.query_row(
            "SELECT node_id FROM rt_searches WHERE tool = 'find' AND log_id = ?1 ORDER BY id DESC LIMIT 1",
            [row.id],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .ok()
        .flatten()
    });
    let node = node_id
        .and_then(|id| sai_paper_core::search_sets::rt_label(conn, id))
        .map(|label| json!({ "id": label }));
    let hits: Vec<Value> = rows
        .iter()
        .map(|(position, paper_id, item)| {
            let map = item.as_object();
            let authors: Vec<Value> = map
                .and_then(|map| map.get("authors"))
                .and_then(|value| value.as_str())
                .and_then(|text| serde_json::from_str::<Vec<String>>(text).ok())
                .unwrap_or_default()
                .into_iter()
                .map(Value::String)
                .collect();
            let get = |key: &str| map.and_then(|map| map.get(key)).cloned().unwrap_or(Value::Null);
            json!({
                "position": position,
                "score": get("score"),
                "chunk_id": get("chunk_id"),
                "source_kind": get("source_kind"),
                "source_ref": get("source_ref"),
                "paper_id": paper_id.map_or(Value::Null, Value::from),
                "title": get("title"),
                "authors": authors,
                "year": get("year"),
                "section": get("section"),
                "text": get("text"),
                "cite": get("cite"),
                "passage": get("passage"),
            })
        })
        .collect();
    json!({
        "log": {
            "id": row.id,
            "date": row.log_date,
            "searched_at": row.searched_at,
            "query": row.query,
            "mode": row.mode,
            "top_k": row.top_k,
            "scope": row.scope,
            "range": row.range_text,
            "papers": row.papers,
            "node": node.unwrap_or(Value::Null),
            "subtree": row.subtree,
            "each": row.each,
            "widen": row.widen_reason,
            "actor": actor,
        },
        "hits": hits,
    })
}

/// `phone find-log <id>`: 内部検索の履歴 1 件とヒット一覧（U01）。
pub(crate) fn find_log_view(db: &Database, log_id: i64) -> Result<Value> {
    let conn = db.connection();
    let Some(row) = find_log::get(conn, log_id)? else {
        return Err(crate::cli_error::CliError::not_found("検索履歴が見つかりません").into());
    };
    let rows = find_log::hits(conn, log_id)?;
    Ok(find_log_json(conn, &row, &rows))
}

/// `phone providers`: 既定の DB（設定の `discover.providers`）と全 DB の一覧。
pub(crate) fn providers() -> Result<Value> {
    let paths = Paths::discover()?;
    let cfg = Config::load_or_create(&paths)?;
    Ok(json!({
        "default": cfg.discover.providers,
        "available": AVAILABLE_PROVIDERS,
    }))
}

/// 検索年のラベル（`search_logs.year` に保存する文字列）。
///
/// 両方指定 `2010-2020` / 下限だけ `2010-` / 上限だけ `-2020` / 指定なしは空文字。
pub(crate) fn year_label(from: Option<u16>, to: Option<u16>) -> String {
    match (from, to) {
        (Some(from), Some(to)) => format!("{from}-{to}"),
        (Some(from), None) => format!("{from}-"),
        (None, Some(to)) => format!("-{to}"),
        (None, None) => String::new(),
    }
}

/// `phone search <query>`: 検索して検索履歴に記録し、そのログを返す（00 §7.2）。
///
/// 出力は `phone log <新しい log_id>` と同じ形。結果 0 件でも履歴は記録する（TUI と同じ）。
async fn search(
    query: &str,
    db: &[String],
    year_from: Option<u16>,
    year_to: Option<u16>,
    limit: u16,
) -> Result<Value> {
    let query = query.trim();
    if query.is_empty() {
        return Err(crate::cli_error::CliError::input("検索語が空です").into());
    }
    let limit = limit.clamp(1, 100);

    let (database, cfg, paths) = open_context()?;
    // core の `run_search` を直接呼ぶ。失敗した DB と理由を受け取りたいから
    // （`crate::run_search` はそれを標準エラーへ出して捨ててしまう）。
    let (dbs, papers, failed) =
        sai_paper_core::api::run_search(&cfg, query, db, limit, year_from, year_to, None).await;
    // 従来と同じ形（`<db>: <理由>`）で標準エラーにも出す。
    for (db_id, reason) in &failed {
        eprintln!("{db_id}: {reason}");
    }
    let year = year_label(year_from, year_to);
    let log_id = search_log::record_search(
        database.connection(),
        "phone",
        query,
        &year,
        &limit.to_string(),
        &dbs.join(","),
        &papers,
    )?;
    // Dashboard の出来事に「discover」として残す（TUI の検索と同じ）
    database.record_activity("discover", None, Some(query))?;
    with_failed(log(&database, &cfg, &paths, log_id)?, &failed)
}

/// 検索の応答に `failed` を足す（`log` と `items` はそのまま・08 §3）。
///
/// 失敗した DB と理由を `[{"db": "s2", "reason": "…"}]` の形で入れる。
/// 1 件も失敗していなければ空の配列。`phone log <id>`（過去の検索結果）には
/// 使わない（履歴には理由を残していないため）。
pub(crate) fn with_failed(value: Value, failed: &[(String, String)]) -> Result<Value> {
    let Value::Object(mut map) = value else {
        // 想定より別の形が来たら、そのまま返す（検索の応答は常に object）。
        return Ok(value);
    };
    map.insert(
        "failed".to_owned(),
        Value::Array(
            failed
                .iter()
                .map(|(db_id, reason)| json!({ "db": db_id, "reason": reason }))
                .collect(),
        ),
    );
    Ok(Value::Object(map))
}

/// `phone translate <id>`: 要旨の日本語訳（TUI の `a` / `A` キーと同じ）。
///
/// 訳文は `papers.abstract_ja` に保存され、次の呼び出しでは再利用される。
pub(crate) async fn translate(paper_id: i64, force: bool) -> Result<Value> {
    let (db, _cfg, _paths) = open_context()?;
    let Some(paper) = db.library_paper_by_id(paper_id)? else {
        return Err(crate::cli_error::CliError::not_found("論文が見つかりません").into());
    };
    let abstract_en = paper.abstract_text.as_deref().unwrap_or("").trim();
    if abstract_en.is_empty() {
        bail!("要旨がありません");
    }
    // 訳済みなら --force が無ければそれを返す（翻訳しない）。
    if !force
        && paper
            .abstract_ja
            .as_deref()
            .is_some_and(|ja| !ja.trim().is_empty())
    {
        return Ok(json!({ "id": paper_id, "abstract_ja": paper.abstract_ja }));
    }
    let service = Config::load_or_create(&Paths::discover()?)?.translate.abstract_;
    match translation::ensure_abstract_ja(db.connection(), paper_id, abstract_en, force, &service)
        .await
    {
        Ok(Some(ja)) => Ok(json!({ "id": paper_id, "abstract_ja": ja })),
        Ok(None) => bail!("要旨がありません"),
        Err(err) => bail!("翻訳できませんでした: {err}"),
    }
}

/// `phone radio <id>`: 音声化（TUI の `r` キーと同じ流れ。数分かかる）。
///
/// `paper_podcasts` を generating → ready / failed に更新するのは core の共通処理。
/// `radio_id` は sai-radio に登録された音声の名前（<フォルダ名>-<番号>）。
pub(crate) async fn radio(paper_id: i64) -> Result<Value> {
    let paths = Paths::discover()?;
    let cfg = Config::load_or_create(&paths)?;
    let radio_id = podcast_client::generate_for_paper(&paths.database_file, paper_id, &cfg.radio)
        .await
        .map_err(|message| anyhow::anyhow!(message))?;
    Ok(json!({ "id": paper_id, "radio_id": radio_id, "status": "ready" }))
}

/// `phone translate-pdf`: PDF を訳し、できた翻訳 PDF の場所を返す。
pub(crate) async fn translate_pdf(paper_id: i64) -> Result<Value> {
    let paths = Paths::discover()?;
    let cfg = Config::load_or_create(&paths)?;
    let path = sai_paper_core::pipeline::translate_pdf(&paths.database_file, paper_id, &cfg.translate.pdf)
        .await
        .map_err(|message| anyhow::anyhow!(message))?;
    Ok(json!({ "id": paper_id, "pdf_ja_path": path, "status": "ready" }))
}

/// Dashboard の出来事の最大件数（iPhone に送る量を抑える）。
const DASHBOARD_ACTIVITY_MAX: usize = 300;

/// `phone dashboard`: TUI の Dashboard と同じ数字カードと RECENT ACTIVITY。
///
/// - 数字は TUI と同じ数え方（論文は Library フォルダの PDF 数、ノートは論文フォルダの note.md の数）。
/// - 出来事には sai-radio の「聴いた・質問した」を混ぜる（止まっていれば sai-paper の分だけ）。
pub(crate) fn dashboard() -> Result<Value> {
    let (db, cfg, paths) = open_context()?;
    let collection_roots: Vec<PathBuf> = cfg
        .library.import_folders
        .iter()
        .map(|root| std::fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
        .collect();
    let tags: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM tags", [], |row| row.get(0))?;
    let notes = sai_paper_core::paths::note_count(&paths.data_dir());
    let mut activity = db.all_activity()?;
    podcast_client::merge_radio_activity(&mut activity, &cfg.radio, db.connection());
    let mut timed: Vec<(chrono::DateTime<chrono::Utc>, Value)> = activity
        .iter()
        .map(|a| {
            (
                a.occurred_at,
                json!({
                    "kind": a.kind,
                    "label": a.label,
                    "occurred_at": a.occurred_at.to_rfc3339(),
                    "paper_id": a.paper_id,
                    "radio_id": a.radio_id,
                }),
            )
        })
        .collect();
    // 研究の run（Action）の行は、sai-sci が持つのでここでは混ぜない（要件定義書 A8）。
    timed.sort_by(|a, b| b.0.cmp(&a.0));
    timed.truncate(DASHBOARD_ACTIVITY_MAX);
    let items: Vec<Value> = timed.into_iter().map(|(_, v)| v).collect();
    Ok(json!({
        "cards": {
            "papers": LibraryIndexer::count_pdfs(&collection_roots),
            "tags": tags,
            "notes": notes,
            "pdf_bytes": LibraryIndexer::pdf_storage_size(&collection_roots),
            "db_bytes": std::fs::metadata(&paths.database_file).map_or(0, |m| m.len()),
        },
        "activity": items,
    }))
}

/// `phone note <id>`: 標準入力の中身でノート（note.md）を保存する。
///
/// 既にあるノートはその場所で上書きする。無ければ論文フォルダ
/// （`paths::paper_folder`）に作る。保存したら出来事に「notes」を残す。
pub(crate) fn save_note(paper_id: i64) -> Result<Value> {
    use std::io::Read as _;
    let (db, _cfg, paths) = open_context()?;
    let Some(paper) = db.library_paper_by_id(paper_id)? else {
        return Err(crate::cli_error::CliError::not_found("論文が見つかりません").into());
    };
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let file = match note_file(&paths, &paper) {
        Some(file) => file,
        None => {
            // 書き込む前に論文フォルダを作る。
            let dir = paths::paper_folder(&paths.data_dir(), paper.id, &paper.title);
            std::fs::create_dir_all(&dir)?;
            paths::paper_file_path(&dir, "note")
        }
    };
    std::fs::write(&file, &text)?;
    db.record_activity("notes", Some(paper_id), None)?;
    let note = text.trim();
    Ok(json!({ "id": paper_id, "note": (!note.is_empty()).then_some(note) }))
}

#[cfg(test)]
mod tests {
    /// 検索の実行者: 窓口の検索は human、agent の検索は rt_searches の run_id と Node を返す。
    #[test]
    fn search_actor_is_human_or_agent_with_node() {
        use super::search_actor;
        use sai_paper_core::search_log::record_search;

        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let human = record_search(conn, "phone", "q", "", "10", "s2", &[]).unwrap();
        assert_eq!(search_actor(conn, human).0["kind"], "human");
        assert!(search_actor(conn, human).1.is_null());

        let agent = record_search(conn, "agent", "q2", "", "10", "s2", &[]).unwrap();
        conn.execute(
            "INSERT INTO rt_searches (run_id, tool, query, set_id) VALUES (7, 'search', 'q2', ?1)",
            [agent],
        )
        .unwrap();
        let (actor, _node) = search_actor(conn, agent);
        assert_eq!(actor["kind"], "agent");
        assert_eq!(actor["run_id"], 7);

        // search_sets に run_id が残っていれば、rt_searches が無くても agent。
        let by_set = record_search(conn, "cli", "q3", "", "10", "s2", &[]).unwrap();
        sai_paper_core::search_log::set_run_id(conn, by_set, Some(9)).unwrap();
        let (actor, _) = search_actor(conn, by_set);
        assert_eq!((actor["kind"].as_str(), actor["run_id"].as_i64()), (Some("agent"), Some(9)));
    }

    /// 内部検索の履歴: `find-log` は人と agent を見分け、`find_groups` は新しい順に並ぶ。
    #[test]
    fn find_log_view_distinguishes_human_and_agent() {
        use super::find_log_view;
        use sai_paper_core::find_log::{self, FindLogInput};
        use std::collections::HashMap;

        /// 記録用のヒット 1 件。
        fn hit(chunk_id: i64, paper_id: Option<i64>) -> sai_paper_core::Hit {
            sai_paper_core::Hit {
                score: 0.8,
                chunk_id,
                source_kind: "body".to_string(),
                source_ref: "3".to_string(),
                paper_id,
                title: Some("題".to_string()),
                authors: Some("[\"A. Smith\"]".to_string()),
                year: Some(2024),
                text: "本文".to_string(),
                char_start: None,
                char_end: None,
                md_path: None,
                section: Some("Methods".to_string()),
            }
        }

        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let empty: HashMap<i64, String> = HashMap::new();
        // 古い履歴（人）→新しい履歴（agent）の順に作る。
        let human = find_log::record(
            conn,
            &FindLogInput {
                query: "人の検索",
                mode: Some("hybrid"),
                top_k: Some(10),
                scope: Some("all"),
                range_text: "library",
                papers: &[3],
                node_id: None,
                subtree: false,
                each: None,
                widen_reason: "",
                origin: "phone",
                run_id: None,
            },
            &[hit(1, Some(3))],
            &empty,
        )
        .unwrap();
        let agent = find_log::record(
            conn,
            &FindLogInput {
                query: "agent の検索",
                mode: Some("hybrid"),
                top_k: Some(10),
                scope: Some("all"),
                range_text: "library",
                papers: &[],
                node_id: None,
                subtree: false,
                each: None,
                widen_reason: "",
                origin: "mcp",
                run_id: Some(5),
            },
            &[hit(2, None)],
            &empty,
        )
        .unwrap();

        let shown = find_log_view(&db, human).unwrap();
        assert_eq!(shown["log"]["query"], "人の検索");
        assert_eq!(shown["log"]["actor"]["kind"], "human");
        assert_eq!(shown["hits"][0]["position"], 0);
        assert_eq!(shown["hits"][0]["paper_id"], 3);
        assert_eq!(shown["hits"][0]["authors"][0], "A. Smith");
        assert!(shown["log"]["node"].is_null());

        let shown = find_log_view(&db, agent).unwrap();
        assert_eq!(shown["log"]["actor"]["kind"], "agent");
        assert_eq!(shown["log"]["actor"]["run_id"], 5);
        // block の名前は sai-link のデータを読まないので null。
        assert!(shown["log"]["actor"]["block"].is_null());
        assert!(find_log_view(&db, human + agent + 99).is_err());

        // `rt_searches` に `tool='find'` の行があれば、`find_log` の node が無くても
        // `rt_label` で Node を出す（U13 の `sai-sci find` がこう書く）。
        conn.execute(
            "INSERT INTO rt_nodes (id, kind, text, created_by) VALUES (5, 'Q', '問い', 'user')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO rt_searches (node_id, tool, query, log_id) VALUES (5, 'find', '人の検索', ?1)",
            [human],
        )
        .unwrap();
        let with_node = find_log_view(&db, human).unwrap();
        assert_eq!(with_node["log"]["node"]["id"], "Q5");

        let found = find_log::groups(conn, &[]).unwrap();
        assert_eq!(found.len(), 2);
        // 新しい順（agent が先）。論文に結び付かないヒットは数えるが論文一覧には入れない。
        assert_eq!(found[0].log_id, agent);
        assert_eq!(found[0].hit_count, 1);
        assert!(found[0].paper_ids.is_empty());
        assert_eq!(found[0].actor.kind, "agent");
        assert_eq!(found[1].log_id, human);
        assert_eq!(found[1].actor.kind, "human");
    }

    /// 検索セットの 4 つの命令: 一覧・詳細・選別・まとめて drop（imported は変えない）。
    #[test]
    fn set_commands_list_show_mark_and_drop_pending() {
        use super::{set_drop_pending, set_list, set_mark, set_show};
        use chrono::Utc;
        use sai_paper_core::RemotePaper;
        use sai_paper_core::search_sets::{self, CreateSearchSetParams};

        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let id = search_sets::create(
            conn,
            CreateSearchSetParams {
                name: "s".into(),
                source_dbs: "s2".into(),
                query: "q".into(),
                raw_query: None,
                created_by: "user".into(),
                note: None,
            },
        )
        .unwrap();
        let papers: Vec<RemotePaper> = (0..4)
            .map(|n| RemotePaper {
                id: format!("p{n}"),
                title: format!("題{n}"),
                authors: vec!["A".into()],
                abstract_text: "要旨".into(),
                published: Utc::now(),
                updated: Utc::now(),
                categories: vec![],
                pdf_url: None,
                doi: None,
                journal_ref: None,
                citation_count: None,
            })
            .collect();
        search_sets::add_items(conn, id, papers).unwrap();
        // 位置 3 は取り込み済み
        search_sets::mark(conn, id, 3, "downloaded", None).unwrap();

        let list = set_list(conn).unwrap();
        let one = &list["sets"][0];
        assert_eq!(one["id"], id);
        assert!(one["node"].is_null());
        assert_eq!(one["counts"]["pending"], 3);
        assert_eq!(one["counts"]["imported"], 1);
        assert_eq!(one["counts"]["total"], 4);

        let shown = set_show(conn, id).unwrap();
        assert_eq!(shown["items"].as_array().unwrap().len(), 4);
        assert_eq!(shown["items"][0]["title"], "題0");
        assert!(shown["set"].get("extract").is_some());

        // keep / drop 以外は失敗。imported は数えない
        assert!(set_mark(conn, id, &[0], "pending", None).is_err());
        let got = set_mark(conn, id, &[0, 3], "keep", Some("要る")).unwrap();
        assert_eq!(got["changed"], 1);

        let got = set_drop_pending(conn, id, None).unwrap();
        assert_eq!(got["changed"], 2);
        let after = set_list(conn).unwrap();
        assert_eq!(after["sets"][0]["counts"]["pending"], 0);
        assert_eq!(after["sets"][0]["counts"]["drop"], 2);
        let shown = set_show(conn, id).unwrap();
        assert_eq!(shown["items"][1]["reason"], "まとめて drop");
    }

    use super::{with_failed, year_label};
    use clap::Parser;
    use serde_json::json;
    use sai_paper_core::Database;

    /// `sai-moon-paper phone download --cite <id> --ref <no>` が引数として通ること（AC5）。
    ///
    /// `--ref` は `--cite` との組み合わせで 1 つだけ受け入れられる。
    #[test]
    fn download_accepts_the_cite_target() {
        use crate::{Cli, Cmd, phone::PhoneCmd};
        let cli = Cli::parse_from([
            "sai-moon-paper",
            "phone",
            "download",
            "--cite",
            "12",
            "--ref",
            "3",
        ]);
        let Some(Cmd::Phone {
            cmd: PhoneCmd::Download {
                cite, ref_no, ..
            },
        }) = cli.cmd
        else {
            panic!("phone download として読めた");
        };
        assert_eq!(cite, Some(12));
        assert_eq!(ref_no, Some(3));

        // 対象が無いので読み取れない（組み合わせの検査は `download` 側でもする）。
        assert!(Cli::try_parse_from(["sai-moon-paper", "phone", "download", "--ref", "3"]).is_err());
    }

    /// `phone manual-import` は対象 1 つ・中身 1 つを要求する。
    #[test]
    fn manual_import_needs_one_target_and_one_source() {
        use crate::phone_cite::ManualImportArgs;
        use crate::{Cli, Cmd, phone::PhoneCmd};

        let parsed = |args: &[&str]| {
            Cli::try_parse_from(["sai-moon-paper", "phone", "manual-import"].into_iter().chain(args.iter().copied()))
                .map(|cli| match cli.cmd {
                    Some(Cmd::Phone {
                        cmd: PhoneCmd::ManualImport { args },
                    }) => args,
                    other => panic!("manual-import として読めた: {other:?}"),
                })
        };

        // 対象も中身も無いと受け付けない。
        assert!(parsed(&[]).is_err());
        // 対象だけ：中身が無いので受け付けない。
        assert!(parsed(&["--paper-id", "1"]).is_err());
        // 中身だけ：対象が無いので受け付けない。
        assert!(parsed(&["--stdin"]).is_err());
        // 対象 2 つ：中身が無くても受け付けない。
        assert!(parsed(&["--paper-id", "1", "--cite", "2", "--ref", "1"]).is_err());
        // 対象 1 つ・中身 1 つは読める。
        let args: ManualImportArgs = parsed(&["--paper-id", "1", "--stdin"]).expect("読める");
        assert_eq!(args.paper_id, Some(1));
        assert!(args.stdin);
        assert_eq!(args.minutes, 5, "既定は 5 分");

        let args = parsed(&["--cite", "7", "--ref", "2", "--file", "paper.pdf", "--minutes", "30"])
            .expect("読める");
        assert_eq!(args.cite, Some(7));
        assert_eq!(args.ref_no, Some(2));
        assert_eq!(args.file.as_deref(), Some("paper.pdf"));
        assert_eq!(args.minutes, 30);
    }

    /// `search` の年のラベル（両方・下限だけ・上限だけ・指定なしの 4 通り）。
    #[test]
    fn year_label_covers_both_ends_and_nothing() {
        assert_eq!(year_label(Some(2010), Some(2020)), "2010-2020");
        assert_eq!(year_label(Some(2010), None), "2010-");
        assert_eq!(year_label(None, Some(2020)), "-2020");
        assert_eq!(year_label(None, None), "");
    }

    /// `phone search` の `failed` は DB と理由の形。0 件なら空の配列（AC1）。
    #[test]
    fn with_failed_adds_db_and_reason() {
        let base = json!({ "log": { "id": 7 }, "items": [] });
        let none = with_failed(base.clone(), &[]).unwrap();
        assert_eq!(none["failed"], json!([]));
        assert_eq!(none["log"]["id"], 7);

        let one = with_failed(
            base,
            &[(
                "s2".to_owned(),
                "search error: invalid S2 response: S2 status 429 (rate limited)".to_owned(),
            )],
        )
        .unwrap();
        assert_eq!(one["failed"][0]["db"], "s2");
        assert!(one["failed"][0]["reason"].as_str().unwrap().contains("429"));
        // 既存の項目はそのまま
        assert!(one["items"].as_array().unwrap().is_empty());
    }

    /// 検索セットの Node の表示は、`rt_label` が `rt_nodes` を読んで作る。
    #[test]
    fn set_json_uses_rt_label_for_the_node() {
        use super::set_json;
        use sai_paper_core::search_sets::{self, CreateSearchSetParams};

        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let id = search_sets::create(
            conn,
            CreateSearchSetParams {
                name: "set".into(),
                source_dbs: "s2".into(),
                query: "q".into(),
                raw_query: None,
                created_by: "user".into(),
                note: None,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO rt_nodes (id, kind, text, created_by) VALUES (5, 'Q', '問い', 'user')",
            [],
        )
        .unwrap();
        conn.execute("UPDATE search_sets SET rt_node = 5 WHERE id = ?1", [id])
            .unwrap();
        assert_eq!(search_sets::rt_label(conn, 5).as_deref(), Some("Q5"));
        assert_eq!(set_json(conn, &search_sets::get(conn, id).unwrap()).unwrap()["node"], "Q5");
        // 行が無ければ `None`（表が無い・行が無いときも同じ）。
        assert_eq!(search_sets::rt_label(conn, 9999), None);
    }
}
