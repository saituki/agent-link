//! `sai-paper paper figures` — 図表（Figure / Table）抽出コマンド。
//!
//! docling を画像出力モードで実行し、`<paper_dir>/figures/` に PNG を保存して
//! `paper_figures` テーブルに登録する。既に抽出済みの論文は `--force` 指定時のみ
//! 再実行する。

use anyhow::{Context, Result, bail};
use sai_paper_core::{Database, Paths, figures};
use std::path::PathBuf;

/// 図抽出を実行する。
pub async fn handle_figures(paper_ids: Vec<i64>, all: bool, force: bool) -> Result<()> {
    let paths = Paths::discover().context("経路の解決に失敗")?;
    let db = Database::open(&paths.database_file).context("DB を開けませんでした")?;

    // 対象論文 (id, title, pdf_path) を決定する。
    // pdf_path は DB 上では data_dir からの相対なので実パスへ直す（段階 2b）。
    let data_dir = paths.data_dir();
    let targets: Vec<(i64, String, String)> = if all {
        let mut stmt = db.connection().prepare(
            "SELECT id, COALESCE(title, ''), pdf_path FROM papers
             WHERE pdf_path IS NOT NULL AND TRIM(pdf_path) <> ''
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(id, title, stored)| {
                let resolved = sai_paper_core::paths::resolve_stored_path(
                    &data_dir,
                    stored.as_deref().unwrap_or_default(),
                );
                (id, title, resolved.to_string_lossy().into_owned())
            })
            .collect()
    } else {
        if paper_ids.is_empty() {
            bail!("--all か論文 ID を 1 つ以上指定してください");
        }
        let mut rows = Vec::new();
        for id in paper_ids {
            let paper = db
                .library_paper_by_id(id)?
                .with_context(|| format!("論文 {id} が見つかりません"))?;
            rows.push((paper.id, paper.title, paper.pdf_path.unwrap_or_default()));
        }
        rows
    };

    if targets.is_empty() {
        println!("対象論文がありません");
        return Ok(());
    }

    let mut ok = 0usize;
    let mut failed = 0usize;
    for (id, title, pdf_path) in targets {
        let pdf = PathBuf::from(&pdf_path);
        if !pdf.is_file() {
            eprintln!("[{id}] skip: PDF が見つかりません ({})", pdf.display());
            failed += 1;
            continue;
        }
        let Some(paper_dir) = pdf.parent().map(std::path::Path::to_path_buf) else {
            eprintln!("[{id}] skip: PDF の親ディレクトリを解決できません");
            failed += 1;
            continue;
        };
        let Some(key) = figures::extract_key_for_paper(db.connection(), id)? else {
            eprintln!("[{id}] skip: s2_paper_id / arxiv_id / doi が未登録です");
            failed += 1;
            continue;
        };
        print!("[{id}] {title} ... ");
        match figures::extract_figures(
            db.connection(),
            &key,
            Some(id),
            &pdf,
            &paper_dir,
            &data_dir,
            force,
        )
        .await
        {
            Ok(count) => {
                println!("{count} 図");
                ok += 1;
            }
            Err(error) => {
                println!("失敗: {error}");
                failed += 1;
            }
        }
    }

    println!("\n完了: 成功 {ok} 件 / 失敗 {failed} 件");
    // 全部失敗したときだけ終了コード 1 にする（1 件ずつの失敗は続ける）。
    if ok == 0 && failed > 0 {
        bail!("すべて失敗しました（{failed} 件）");
    }
    Ok(())
}
