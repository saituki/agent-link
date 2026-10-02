//! `sai-moon-paper report` — agent の report の登録・参照コマンド。
//!
//! report の md は sai-link の出力フォルダに置かれたまま扱う。
//! **md を書き換えるコマンドは用意しない**（要件 R3.3）。

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use sai_paper_core::citation_check::{self, CheckOptions, CheckSummary};
use sai_paper_core::{NewReport, ReportSource, reports};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::open_database;

/// `sai-moon-paper report` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum ReportCmd {
    /// report を登録し、採番した report_id を出力する
    Add {
        /// md ファイルのパス
        #[arg(long)]
        path: PathBuf,
        /// タイトル（省略時は md の見出しから推定）
        #[arg(long)]
        title: Option<String>,
        /// 生成元の agent 実行 ID
        #[arg(long)]
        run_id: Option<String>,
        /// 実行した block id
        #[arg(long)]
        block: Option<String>,
        /// 参照した論文 ID（papers.id、繰り返し指定可）
        #[arg(long)]
        paper: Vec<i64>,
        /// 参照した引用箇所（chunks.id、繰り返し指定可）
        #[arg(long)]
        chunk: Vec<i64>,
        /// 入力元の別 report（report_id、繰り返し指定可）
        #[arg(long = "source-report")]
        source_report: Vec<String>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// report の一覧
    List {
        /// 論文 ID で絞り込む（papers.id）
        #[arg(long)]
        paper: Option<i64>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// report 1 件の情報
    Show {
        /// report_id（例: R-0001）
        report_id: String,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// report の md 本文を標準出力へ出す
    Content {
        /// report_id（例: R-0001）
        report_id: String,
    },
    /// 成果物のフォルダを読んで report を自動登録する
    Scan {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// report ごとの要確認（error）と注意（warning）を出す
    Issues {
        /// report_id（例: R-0007。省略すると全 report）
        id: Option<String>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// report を取り除く（md は消さない）
    Remove {
        /// report_id（例: R-0007）
        id: String,
    },
    /// 保存前の検査。通れば frontmatter を付けた本文を標準出力へ出す（本文は標準入力、description は SAI_WRITE_DESCRIPTION）
    Prepare {
        /// 実行 ID
        #[arg(long)]
        run: Option<i64>,
        /// ブロック id
        #[arg(long)]
        block: Option<String>,
        /// 保存するファイル名
        #[arg(long)]
        filename: Option<String>,
    },
    /// 外部の引用（Library に無い論文）を公開の API で照合する
    Check {
        /// report_id（例: R-0007）
        id: Option<String>,
        /// 外部の引用がある report をすべて照合する
        #[arg(long)]
        all: bool,
    },
}

/// `sai-moon-paper report` を実行する。
pub async fn handle_report(cmd: ReportCmd) -> Result<()> {
    if let ReportCmd::Prepare { run, block, filename } = &cmd {
        std::process::exit(prepare(*run, block.clone(), filename.clone()));
    }
    let db = open_database()?;
    let conn = db.connection();
    match cmd {
        ReportCmd::Add {
            path,
            title,
            run_id,
            block,
            paper,
            chunk,
            source_report,
            json,
        } => {
            if !path.is_file() {
                bail!("md ファイルが見つかりません: {}", path.display());
            }
            let mut sources = Vec::new();
            for id in paper {
                sources.push(ReportSource {
                    kind: "paper".into(),
                    ref_id: id.to_string(),
                    role: None,
                });
            }
            for id in chunk {
                sources.push(ReportSource {
                    kind: "chunk".into(),
                    ref_id: id.to_string(),
                    role: None,
                });
            }
            for id in source_report {
                sources.push(ReportSource {
                    kind: "report".into(),
                    ref_id: id,
                    role: None,
                });
            }
            let report_id = reports::create_report(
                conn,
                &NewReport {
                    title,
                    md_path: path.to_string_lossy().to_string(),
                    run_id,
                    block,
                    sources,
                    summary: None,
                },
            )
            .context("report の登録に失敗")?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "report_id": report_id,
                        "path": path.to_string_lossy(),
                    })
                );
            } else {
                println!("{report_id}");
            }
        }
        ReportCmd::List { paper, json } => {
            let rows = reports::list_reports(conn, paper).context("report 一覧の取得に失敗")?;
            // 説明は md の frontmatter の description。無ければ title。
            let description = |md_path: &str| -> Option<String> {
                let text = std::fs::read_to_string(md_path).ok()?;
                sai_paper_core::runs::read_frontmatter(&text)
                    .into_iter()
                    .find(|(key, value)| key == "description" && !value.is_empty())
                    .map(|(_, value)| value)
            };
            if json {
                let mut value = serde_json::to_value(&rows)?;
                for (item, row) in value.as_array_mut().into_iter().flatten().zip(&rows) {
                    item["description"] = serde_json::json!(description(&row.md_path));
                }
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else if rows.is_empty() {
                println!("report はありません");
            } else {
                for row in rows {
                    let mark = match reports::report_check(conn, &row.id)? {
                        Some(check) if !check.header_ok => "\t[形が読めない]",
                        _ => "",
                    };
                    println!(
                        "{}\t{}\t{}{}",
                        row.id,
                        description(&row.md_path)
                            .as_deref()
                            .or(row.title.as_deref())
                            .unwrap_or("-"),
                        row.md_path,
                        mark
                    );
                }
            }
        }
        ReportCmd::Show { report_id, json } => {
            let report = reports::get_report(conn, &report_id)
                .context("report の取得に失敗")?
                .with_context(|| format!("report が見つかりません: {report_id}"))?;
            let sources = reports::report_sources(conn, &report_id)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "report": report,
                        "sources": sources,
                    })
                );
            } else {
                println!("report_id : {}", report.id);
                println!("title     : {}", report.title.as_deref().unwrap_or("-"));
                println!("md_path   : {}", report.md_path);
                println!("run_id    : {}", report.run_id.as_deref().unwrap_or("-"));
                println!("block     : {}", report.block.as_deref().unwrap_or("-"));
                println!("created_at: {}", report.created_at);
                if let Some(resolved) = reports::resolve_md_path(&report) {
                    println!("resolved  : {}", resolved.display());
                }
                println!("sources   : {}", sources.len());
                for source in sources {
                    println!("  {} {}", source.kind, source.ref_id);
                }
            }
        }
        ReportCmd::Content { report_id } => {
            let text = reports::read_report_markdown(conn, &report_id)
                .with_context(|| format!("report 本文の読み出しに失敗: {report_id}"))?;
            print!("{text}");
        }
        ReportCmd::Scan { json } => {
            let root = sai_paper_core::paths::project_root()
                .context("研究プロジェクトのルートを解決できません")?;
            let summary = reports::scan(conn, &root).context("report の走査に失敗")?;
            if json {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                println!(
                    "report: 新規 {}・更新 {}・変化なし {}（開発用を {} 件飛ばしました）・先頭情報の問題 {}",
                    summary.added,
                    summary.updated,
                    summary.unchanged,
                    summary.skipped_dev,
                    summary.header_errors,
                );
            }
        }
        ReportCmd::Issues { id, json } => {
            let rows = reports::list_issues(conn, id.as_deref())
                .context("report の問題の取得に失敗")?;
            let shown: Vec<_> = rows
                .into_iter()
                .filter(|row| !row.issues.is_empty())
                .collect();
            let file_name = |md_path: &str| {
                std::path::Path::new(md_path)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default()
            };
            if json {
                let value: Vec<_> = shown
                    .iter()
                    .map(|row| {
                        serde_json::json!({
                            "report_id": row.report.id,
                            "block": row.report.block,
                            "run_id": row.report.run_id,
                            "file": file_name(&row.report.md_path),
                            "issues": row.issues,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else if shown.is_empty() {
                let total = reports::list_reports(conn, None)
                    .context("report 一覧の取得に失敗")?
                    .len();
                println!("出典の問題はありません（report {total} 本）");
            } else {
                let mut errors = 0usize;
                let mut warnings = 0usize;
                for row in &shown {
                    println!(
                        "{}  {}  run {}  {}",
                        row.report.id,
                        row.report.block.as_deref().unwrap_or("-"),
                        row.report.run_id.as_deref().unwrap_or("-"),
                        file_name(&row.report.md_path),
                    );
                    for issue in &row.issues {
                        let label = if issue.level == "error" {
                            "要確認"
                        } else {
                            "注意"
                        };
                        let line = match issue.line {
                            Some(line) => format!("{line} 行"),
                            None => "      ".to_string(),
                        };
                        println!("  {label}  {line}  {}", issue.detail);
                        if issue.level == "error" {
                            errors += 1;
                        } else {
                            warnings += 1;
                        }
                    }
                }
                println!(
                    "合計: 要確認 {}・注意 {}（report {} 本）",
                    errors,
                    warnings,
                    shown.len()
                );
            }
        }
        ReportCmd::Remove { id } => {
            let removed = conn.execute("DELETE FROM reports WHERE id = ?1", [id.as_str()])?;
            if removed == 0 {
                bail!("report が見つかりません: {id}");
            }
            println!("{id} を取り除きました");
        }
        ReportCmd::Check { id, all } => {
            check_reports(conn, id.as_deref(), all).await?;
        }
        ReportCmd::Prepare { .. } => unreachable!("先に処理済み"),
    }
    Ok(())
}

/// 実験ループの文書（出力の形・引き継ぎの検査をしない）。
const EXP_PREFIXES: [&str; 5] = ["exp-spec", "exp-run", "review", "branch-order", "exp-final"];

/// `report prepare` の本体。終了コードを返す（0 通過・1 違反・2 使い方の誤り）。
fn prepare(run: Option<i64>, block: Option<String>, filename: Option<String>) -> i32 {
    use sai_paper_core::reports::{CitationTarget, parse_citations};
    use sai_paper_core::{dsr_check, runs, sections, textcheck};
    use std::io::Read;

    let (Some(run), Some(block), Some(filename)) = (run, block, filename) else {
        eprintln!("使い方: report prepare --run <run_id> --block <block> --filename <name>");
        return 2;
    };
    if !filename.ends_with(".md") {
        return 0;
    }
    let mut body = String::new();
    if let Err(err) = std::io::stdin().read_to_string(&mut body) {
        eprintln!("標準入力を読めません: {err}");
        return 2;
    }
    let description = std::env::var("SAI_WRITE_DESCRIPTION").unwrap_or_default();
    let description = description.trim();
    let mut errors: Vec<String> = Vec::new();

    // 1. description
    if description.is_empty() {
        errors.push("description が空です。sai_write の description に『<文書の種類> <ID>: <内容を 1 文で>』の形で書いてください".into());
    } else if description.contains('\n')
        || description.chars().count() > 200
        || description.starts_with("```")
        || description.starts_with('#')
    {
        errors.push("description は 1 行・200 字以内で、``` や # で始めないでください".into());
    }

    // 2. 文字種
    for f in textcheck::foreign_chars(&body) {
        errors.push(format!("{}行目: {}「{}」が混ざっています", f.line, f.script, f.ch));
    }
    if let Some(message) = textcheck::foreign_message(description) {
        errors.push(format!("description: {message}"));
    }

    // 3. 引用
    let citations = parse_citations(&body);
    let mut papers: Vec<i64> = citations
        .iter()
        .filter_map(|c| match c.target {
            CitationTarget::Paper(n) => Some(n),
            _ => None,
        })
        .collect();
    papers.sort_unstable();
    papers.dedup();
    match open_database() {
        Ok(db) => {
            let conn = db.connection();
            for citation in &citations {
                let CitationTarget::Paper(n) = citation.target else { continue };
                let exists = conn
                    .query_row("SELECT 1 FROM papers WHERE id = ?1", [n], |_| Ok(()))
                    .is_ok();
                if !exists {
                    errors.push(format!("{}行目: Library に #{n} がありません", citation.line));
                    continue;
                }
                let Some(locator) = &citation.locator else { continue };
                let Ok(parsed) = sections::parse_ref(&format!("{n}{locator}")) else { continue };
                let (sec, pass): (i64, Vec<i64>) = match parsed {
                    sections::PassageRef::Passage { sec_no, pas_no, .. } => (sec_no, vec![pas_no]),
                    sections::PassageRef::Range { sec_no, from, to, .. } => (sec_no, vec![from, to]),
                    _ => continue,
                };
                for pas in pass {
                    if sections::passage_exists(conn, n, sec, Some(pas)).ok().flatten() == Some(false) {
                        errors.push(format!(
                            "{}行目: 塊 #{n}.{sec}.{pas} は論文 {n} にありません",
                            citation.line
                        ));
                    }
                }
            }
        }
        Err(err) => eprintln!("注意: papr.db を開けないので引用の存在は確かめていません: {err}"),
    }

    // 4・5. 出力の形と引き継ぎ（実験ループの文書は見ない）
    let root = sai_paper_core::paths::project_root();
    let stem_is_exp = EXP_PREFIXES.iter().any(|p| filename.starts_with(p));
    let block_md = root.as_ref().and_then(|root| {
        let mut paths = vec![root.join(".sai/blocks").join(format!("{block}.md"))];
        paths.extend(sai_paper_core::paths::sai_home_path("blocks").map(|d| d.join(format!("{block}.md"))));
        paths.iter().find_map(|p| std::fs::read_to_string(p).ok())
    });
    if let (false, Some(block_md)) = (stem_is_exp, &block_md) {
        let expected = dsr_check::expected_headings(block_md);
        let result = dsr_check::check(&body, &expected);
        if result.missing.len() < expected.len() {
            for heading in &result.missing {
                errors.push(format!("見出し『{heading}』がありません"));
            }
        }
        let sci = runs::read_frontmatter(block_md)
            .iter()
            .any(|(k, v)| k == "templates" && v.split(',').any(|t| t.trim() == "sci-core"));
        if sci && result.has_handoff {
            errors.push("sci のブロックの文書に『## 引き継ぎ』は書きません".into());
        }
    }

    if !errors.is_empty() {
        for message in errors {
            eprintln!("- {message}");
        }
        return 1;
    }

    // frontmatter を付ける
    let sci_node = root.as_ref().and_then(|root| {
        let dir = runs::list_run_dirs(root).into_iter().find(|d| d.run_id == run)?;
        let task = std::fs::read_to_string(dir.path.join("task.md")).ok()?;
        runs::read_frontmatter(&task)
            .into_iter()
            .find(|(k, v)| k == "sci_node" && !v.is_empty())
            .map(|(_, v)| v)
    });
    let papers = papers.iter().map(i64::to_string).collect::<Vec<_>>().join(", ");
    let run = run.to_string();
    let mut pairs = vec![("block", block.as_str(), true), ("run", run.as_str(), true), ("description", description, true)];
    if let Some(node) = &sci_node {
        pairs.push(("sci_node", node, false));
    }
    if !papers.is_empty() {
        pairs.push(("papers", &papers, false));
    }
    print!("{}", textcheck::with_frontmatter(&body, &pairs));
    0
}

/// `report check` の本体。
///
/// `id` を指定するとその report だけ、`--all` なら外部の引用がある report を順に照合する。
async fn check_reports(
    conn: &rusqlite::Connection,
    id: Option<&str>,
    all: bool,
) -> Result<()> {
    if !all && id.is_none() {
        bail!("report_id か --all を指定してください");
    }
    // 照合器は 1 つを使い回す（`--all` でも問い合わせの間隔を保つ）。
    let opts = CheckOptions::default();
    if let Some(id) = id {
        let summary = citation_check::check_report(conn, id, &opts)
            .await
            .with_context(|| format!("report の照合に失敗: {id}"))?;
        print_summary(id, &summary);
        return Ok(());
    }

    // `--all`: 外部の引用がある report を集める（先に問い合わせる件数を出す）。
    let mut targets: Vec<String> = Vec::new();
    let mut total = 0usize;
    for report in reports::list_reports(conn, None)? {
        let Ok(markdown) = reports::read_report_markdown(conn, &report.id) else {
            continue;
        };
        let refs: HashSet<String> = reports::parse_citations(&markdown)
            .into_iter()
            .filter_map(|citation| match citation.target {
                reports::CitationTarget::External { scheme, id } => Some(format!("{scheme}:{id}")),
                _ => None,
            })
            .collect();
        if !refs.is_empty() {
            total += refs.len();
            targets.push(report.id);
        }
    }
    if targets.is_empty() {
        println!("外部の引用はありません");
        return Ok(());
    }
    println!("外部の引用 {total} 件を問い合わせます（約 {total} 秒）");

    let mut totals = CheckSummary::default();
    for report_id in &targets {
        let summary = citation_check::check_report(conn, report_id, &opts)
            .await
            .with_context(|| format!("report の照合に失敗: {report_id}"))?;
        print_summary(report_id, &summary);
        totals.total += summary.total;
        totals.found += summary.found;
        totals.not_found += summary.not_found;
        totals.offline += summary.offline;
    }
    if targets.len() > 1 {
        println!(
            "合計: 外部の引用 {} 件 — 確認済み {}・見つからない {}・未確認（オフライン）{}",
            totals.total, totals.found, totals.not_found, totals.offline
        );
    }
    Ok(())
}

/// 照合の結果を 1 行で出す。
fn print_summary(report_id: &str, summary: &CheckSummary) {
    // `Error` は結果に含まれないので、残りを「照合の失敗」として見せる。
    let failed = summary
        .total
        .saturating_sub(summary.found + summary.not_found + summary.offline);
    let suffix = if failed > 0 {
        format!("・照合の失敗 {failed}")
    } else {
        String::new()
    };
    println!(
        "{report_id}: 外部の引用 {} 件 — 確認済み {}・見つからない {}・未確認（オフライン）{}。詳しくは sai-paper report issues {report_id}{suffix}",
        summary.total, summary.found, summary.not_found, summary.offline
    );
}
