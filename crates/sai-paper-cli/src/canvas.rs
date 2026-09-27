//! `sai-paper canvas` — CLI から Canvas を作る（V03）。
//!
//! エージェントとユーザーが、CLI から Canvas に札・枠・線を足せるようにする。
//! エージェントは Canvas の JSON を直接書かず、このコマンドを通す（形が壊れない
//! ように）。置き場所は [`sai_paper_core::canvas_edit`] が自動で決める。
//!
//! - `new` でタグと空の Canvas を作る（エージェントは `--draft` を付ける）
//! - `add-*` は作ったノードの id を 1 行で出す（`--json` なら `{"id": "..."}`）
//! - 下書きでない Canvas に足すには `--force` が要る（ユーザーの Canvas を守る）
//! - `add-paper` と `add-report` は、その論文にタグも付ける（TUI の同期用）

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use sai_paper_core::canvas::{Canvas, CanvasNode, CanvasNodeKind, canvas_path_for_tag};
use sai_paper_core::canvas_edit;
use sai_paper_core::paths::paper_file_names;
use sai_paper_core::{Database, Paths, reports, understanding};
use std::path::{Path, PathBuf};

use crate::open_database;

/// `sai-paper canvas` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum CanvasCmd {
    /// 空の Canvas を作る（タグも作る）
    New {
        /// Canvas 名（= タグ名）
        name: String,
        /// 下書きとして作る（`saiDraft: true`）
        #[arg(long)]
        draft: bool,
    },
    /// Canvas の一覧（タグ名・下書きか・ノード数）
    List {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// Canvas の中身（ノードと線）を出す
    Show {
        /// Canvas 名（= タグ名）
        name: String,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// トピックの枠（グループ）を足す
    AddGroup {
        /// Canvas 名（= タグ名）
        name: String,
        /// 枠の見出し
        #[arg(long)]
        label: String,
        /// 色（red/orange/yellow/green/cyan/purple/gray。gray は色なし）
        #[arg(long)]
        color: Option<String>,
        /// トピック名（`saiTopic` に入れる）
        #[arg(long)]
        topic: Option<String>,
        /// 下書きでない Canvas でも足す
        #[arg(long)]
        force: bool,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 論文の札を足す（note.md があれば note の札）
    AddPaper {
        /// Canvas 名（= タグ名）
        name: String,
        /// 論文番号（`papers.id`）
        paper_id: i64,
        /// 入れるグループ（枠）の id
        #[arg(long)]
        group: Option<String>,
        /// 下書きでない Canvas でも足す
        #[arg(long)]
        force: bool,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// note の札を足す（`note.md` が無ければエラー）
    AddNote {
        /// Canvas 名（= タグ名）
        name: String,
        /// 論文番号（`papers.id`）
        paper_id: i64,
        /// 入れるグループ（枠）の id
        #[arg(long)]
        group: Option<String>,
        /// 下書きでない Canvas でも足す
        #[arg(long)]
        force: bool,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// report の札を足す
    AddReport {
        /// Canvas 名（= タグ名）
        name: String,
        /// report ID（例: R-0007）
        report_id: String,
        /// 入れるグループ（枠）の id
        #[arg(long)]
        group: Option<String>,
        /// 下書きでない Canvas でも足す
        #[arg(long)]
        force: bool,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 文字の札を足す
    AddText {
        /// Canvas 名（= タグ名）
        name: String,
        /// 札に書く文
        text: String,
        /// 入れるグループ（枠）の id
        #[arg(long)]
        group: Option<String>,
        /// 下書きでない Canvas でも足す
        #[arg(long)]
        force: bool,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 2 つのノードを線でつなぐ
    Link {
        /// Canvas 名（= タグ名）
        name: String,
        /// 始点のノード id
        from: String,
        /// 終点のノード id
        to: String,
        /// 線のラベル
        #[arg(long)]
        label: Option<String>,
        /// 下書きでない Canvas でも足す
        #[arg(long)]
        force: bool,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
}

/// `sai-paper canvas` を実行する。
pub fn handle(cmd: CanvasCmd) -> Result<()> {
    let paths = Paths::discover().context("プロジェクトのパスを解決できません")?;
    let data_dir = paths.data_dir();
    match cmd {
        CanvasCmd::New { name, draft } => handle_new(&data_dir, &name, draft),
        CanvasCmd::List { json } => handle_list(&data_dir, json),
        CanvasCmd::Show { name, json } => handle_show(&data_dir, &name, json),
        CanvasCmd::AddGroup {
            name,
            label,
            color,
            topic,
            force,
            json,
        } => handle_add_group(
            &data_dir,
            &name,
            &label,
            color.as_deref(),
            topic.as_deref(),
            force,
            json,
        ),
        CanvasCmd::AddPaper {
            name,
            paper_id,
            group,
            force,
            json,
        } => handle_add_paper(&data_dir, &name, paper_id, group.as_deref(), force, json),
        CanvasCmd::AddNote {
            name,
            paper_id,
            group,
            force,
            json,
        } => handle_add_note(&data_dir, &name, paper_id, group.as_deref(), force, json),
        CanvasCmd::AddReport {
            name,
            report_id,
            group,
            force,
            json,
        } => handle_add_report(&data_dir, &name, &report_id, group.as_deref(), force, json),
        CanvasCmd::AddText {
            name,
            text,
            group,
            force,
            json,
        } => handle_add_text(&data_dir, &name, &text, group.as_deref(), force, json),
        CanvasCmd::Link {
            name,
            from,
            to,
            label,
            force,
            json,
        } => handle_link(&data_dir, &name, &from, &to, label.as_deref(), force, json),
    }
}

/// `canvas new` — タグと空の Canvas を作る。
fn handle_new(data_dir: &Path, name: &str, draft: bool) -> Result<()> {
    let path = canvas_path_for_tag(data_dir, name);
    if path.as_os_str().is_empty() {
        bail!("タグ '{name}' は Canvas 名にできません");
    }
    if path.is_file() {
        bail!("Canvas '{name}' は既にあります。足すには add-… を使ってください");
    }
    let db = open_database()?;
    db.ensure_tag(name).context("タグを作れませんでした")?;
    let canvas = canvas_edit::new_canvas(name, draft);
    canvas_edit::save(data_dir, name, &canvas)?;
    println!("{name}");
    Ok(())
}

/// `canvas list` — Canvas の一覧を出す。
fn handle_list(data_dir: &Path, json: bool) -> Result<()> {
    let db = open_database()?;
    let mut entries: Vec<(String, bool, usize)> = Vec::new();
    for tag in db.tags().context("タグの取得に失敗しました")? {
        let path = canvas_path_for_tag(data_dir, &tag.name);
        if path.as_os_str().is_empty() || !path.is_file() {
            continue;
        }
        let canvas = canvas_edit::load_or_new(data_dir, &tag.name)?;
        entries.push((tag.name, canvas_edit::is_draft(&canvas), canvas.nodes.len()));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    if json {
        let array: Vec<serde_json::Value> = entries
            .iter()
            .map(|(name, draft, nodes)| {
                serde_json::json!({ "name": name, "draft": draft, "nodes": nodes })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&array)?);
    } else if entries.is_empty() {
        println!("Canvas はありません");
    } else {
        for (name, draft, nodes) in entries {
            let mark = if draft { "下書き" } else { "確定" };
            println!("{name}\t{mark}\t{nodes} ノード");
        }
    }
    Ok(())
}

/// `canvas show` — Canvas のノードと線を出す。
fn handle_show(data_dir: &Path, name: &str, json: bool) -> Result<()> {
    let path = canvas_path_for_tag(data_dir, name);
    if path.as_os_str().is_empty() {
        bail!("タグ '{name}' は Canvas 名にできません");
    }
    if !path.is_file() {
        bail!("Canvas '{name}' がありません");
    }
    let canvas = canvas_edit::load_or_new(data_dir, name)?;
    if json {
        let nodes: Vec<serde_json::Value> = canvas
            .nodes
            .iter()
            .map(|node| {
                serde_json::json!({
                    "id": node.id,
                    "kind": node_kind_label(node),
                    "heading": node_heading(node),
                    "group": group_of(&canvas, node),
                })
            })
            .collect();
        let edges: Vec<serde_json::Value> = canvas
            .edges
            .iter()
            .map(|edge| {
                serde_json::json!({
                    "id": edge.id,
                    "from": edge.from_node,
                    "to": edge.to_node,
                    "label": edge.label,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "name": name,
                "draft": canvas_edit::is_draft(&canvas),
                "nodes": nodes,
                "edges": edges,
            }))?
        );
    } else {
        let mark = if canvas_edit::is_draft(&canvas) {
            "（下書き）"
        } else {
            ""
        };
        println!("Canvas: {name}{mark}");
        for node in &canvas.nodes {
            let group = group_of(&canvas, node).unwrap_or_else(|| "-".to_string());
            println!(
                "{}\t{}\t{}\tgroup: {}",
                node.id,
                node_kind_label(node),
                node_heading(node),
                group
            );
        }
        println!("線: {}", canvas.edges.len());
        for edge in &canvas.edges {
            println!(
                "  {} -> {} {}",
                edge.from_node,
                edge.to_node,
                edge.label.as_deref().unwrap_or("")
            );
        }
    }
    Ok(())
}

/// 書き換える Canvas を読む。
///
/// 下書きでない Canvas（ユーザーの Canvas）と、まだファイルの無い Canvas は
/// `force` が無ければエラーにする（無いものは `canvas new` で作る）。
fn load_for_edit(data_dir: &Path, name: &str, force: bool) -> Result<Canvas> {
    let path = canvas_path_for_tag(data_dir, name);
    if path.as_os_str().is_empty() {
        bail!("タグ '{name}' は Canvas 名にできません");
    }
    let exists = path.is_file();
    let canvas = canvas_edit::load_or_new(data_dir, name)?;
    if exists {
        canvas_edit::ensure_editable(&canvas, force)?;
    } else if !force {
        // ファイルがまだ無いタグも TUI ではユーザーの Canvas として開ける。
        // 黙って下書きにしないよう、先に `canvas new` で作らせる。
        bail!("Canvas '{name}' がありません。先に `sai-paper canvas new {name} --draft` で作ってください（ユーザーの Canvas に足すなら --force）");
    }
    Ok(canvas)
}

/// 書き換える Canvas と DB をそろえる（タグが無ければ作る）。
fn edit_canvas(data_dir: &Path, name: &str, force: bool) -> Result<(Database, Canvas)> {
    let canvas = load_for_edit(data_dir, name, force)?;
    let db = open_database()?;
    db.ensure_tag(name).context("タグを作れませんでした")?;
    Ok((db, canvas))
}

/// 作った id を出す（`--json` なら `{"id": "..."}`）。
fn print_id(id: &str, json: bool) {
    if json {
        println!("{}", serde_json::json!({ "id": id }));
    } else {
        println!("{id}");
    }
}

/// `canvas add-group` を実行する。
fn handle_add_group(
    data_dir: &Path,
    name: &str,
    label: &str,
    color: Option<&str>,
    topic: Option<&str>,
    force: bool,
    json: bool,
) -> Result<()> {
    let (db, mut canvas) = edit_canvas(data_dir, name, force)?;
    let id = match color {
        // `--color` の指定はその色を使い、理解の記録で決め直さない（V05）。
        Some(color) => {
            let color = canvas_edit::parse_color_name(color)?;
            canvas_edit::add_group(&mut canvas, label, color, topic)
        }
        // 指定が無くトピックがあれば、理解の記録から色を決める（V05）。
        None => match topic {
            Some(topic) => {
                let entries = understanding::latest_per_topic(db.connection())
                    .context("理解の記録の取得に失敗しました")?;
                let states = canvas_edit::states_by_topic(&entries);
                canvas_edit::add_group_from_understanding(
                    &mut canvas,
                    label,
                    topic,
                    states.get(topic).map(String::as_str),
                )
            }
            None => canvas_edit::add_group(&mut canvas, label, None, None),
        },
    };
    canvas_edit::save(data_dir, name, &canvas)?;
    print_id(&id, json);
    Ok(())
}

/// `canvas add-paper` を実行する。
fn handle_add_paper(
    data_dir: &Path,
    name: &str,
    paper_id: i64,
    group: Option<&str>,
    force: bool,
    json: bool,
) -> Result<()> {
    let (db, mut canvas) = edit_canvas(data_dir, name, force)?;
    let title = paper_title(&db, paper_id)?;
    let note_md = note_path_if_exists(data_dir, paper_id, &title);
    check_group(&canvas, group)?;
    let id = canvas_edit::add_paper(&mut canvas, paper_id, note_md.as_deref(), group);
    db.add_paper_tag(paper_id, name)
        .context("論文にタグを付けられませんでした")?;
    canvas_edit::save(data_dir, name, &canvas)?;
    print_id(&id, json);
    Ok(())
}

/// `canvas add-note` を実行する（`note.md` が無ければエラー）。
fn handle_add_note(
    data_dir: &Path,
    name: &str,
    paper_id: i64,
    group: Option<&str>,
    force: bool,
    json: bool,
) -> Result<()> {
    let (db, mut canvas) = edit_canvas(data_dir, name, force)?;
    let title = paper_title(&db, paper_id)?;
    let note_md = note_path_if_exists(data_dir, paper_id, &title)
        .ok_or_else(|| anyhow::anyhow!("論文 #{paper_id} に note.md がありません"))?;
    check_group(&canvas, group)?;
    let id = canvas_edit::add_note(&mut canvas, paper_id, &note_md, group);
    db.add_paper_tag(paper_id, name)
        .context("論文にタグを付けられませんでした")?;
    canvas_edit::save(data_dir, name, &canvas)?;
    print_id(&id, json);
    Ok(())
}

/// `canvas add-report` を実行する。
fn handle_add_report(
    data_dir: &Path,
    name: &str,
    report_id: &str,
    group: Option<&str>,
    force: bool,
    json: bool,
) -> Result<()> {
    let (db, mut canvas) = edit_canvas(data_dir, name, force)?;
    let report = reports::get_report(db.connection(), report_id)
        .context("report の取得に失敗しました")?
        .ok_or_else(|| anyhow::anyhow!("report '{report_id}' がありません"))?;
    let md_path = reports::resolve_md_path(&report);
    check_group(&canvas, group)?;
    let id = canvas_edit::add_report(&mut canvas, report_id, md_path.as_deref(), group);
    // report が引用している論文にタグを付ける（TUI の同期でタグの論文として扱われる）。
    for source in reports::report_sources(db.connection(), report_id)
        .context("report の引用元の取得に失敗しました")?
    {
        if source.kind == "paper" {
            if let Ok(paper_id) = source.ref_id.parse::<i64>() {
                db.add_paper_tag(paper_id, name)
                    .context("論文にタグを付けられませんでした")?;
            }
        }
    }
    canvas_edit::save(data_dir, name, &canvas)?;
    print_id(&id, json);
    Ok(())
}

/// `canvas add-text` を実行する。
fn handle_add_text(
    data_dir: &Path,
    name: &str,
    text: &str,
    group: Option<&str>,
    force: bool,
    json: bool,
) -> Result<()> {
    let (_db, mut canvas) = edit_canvas(data_dir, name, force)?;
    check_group(&canvas, group)?;
    let id = canvas_edit::add_text(&mut canvas, text, group);
    canvas_edit::save(data_dir, name, &canvas)?;
    print_id(&id, json);
    Ok(())
}

/// `canvas link` を実行する。
fn handle_link(
    data_dir: &Path,
    name: &str,
    from: &str,
    to: &str,
    label: Option<&str>,
    force: bool,
    json: bool,
) -> Result<()> {
    let (_db, mut canvas) = edit_canvas(data_dir, name, force)?;
    let id = canvas_edit::link(&mut canvas, from, to, label)?;
    canvas_edit::save(data_dir, name, &canvas)?;
    print_id(&id, json);
    Ok(())
}

/// 論文のタイトルを引く（無ければエラー）。
pub(crate) fn paper_title(db: &Database, paper_id: i64) -> Result<String> {
    use rusqlite::OptionalExtension;
    let title: Option<Option<String>> = db
        .connection()
        .query_row(
            "SELECT title FROM papers WHERE id = ?1",
            [paper_id],
            |row| row.get(0),
        )
        .optional()?;
    match title {
        Some(title) => Ok(title.unwrap_or_default()),
        None => bail!("論文 #{paper_id} がライブラリにありません"),
    }
}

/// 論文フォルダの `note.md`（あるときだけ）。
pub(crate) fn note_path_if_exists(data_dir: &Path, paper_id: i64, title: &str) -> Option<PathBuf> {
    let path =
        sai_paper_core::paths::paper_folder(data_dir, paper_id, title).join(paper_file_names::NOTE);
    path.is_file().then_some(path)
}

/// `--group` で渡されたグループが Canvas にあるか確かめる。
fn check_group(canvas: &Canvas, group: Option<&str>) -> Result<()> {
    if let Some(group_id) = group {
        if !canvas.nodes.iter().any(|node| node.id == group_id) {
            bail!("グループ '{group_id}' が Canvas にありません");
        }
    }
    Ok(())
}

/// ノードの種類（group / text / note / report / paper / file）。
fn node_kind_label(node: &CanvasNode) -> String {
    match &node.kind {
        CanvasNodeKind::Group { .. } => "group".to_string(),
        CanvasNodeKind::Text { .. } => "text".to_string(),
        CanvasNodeKind::Link { .. } => "link".to_string(),
        CanvasNodeKind::File { .. } => node
            .extra
            .get("saiKind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("file")
            .to_string(),
        CanvasNodeKind::Unknown { kind } => kind.clone(),
    }
}

/// ノードの見出し（グループは label、文字は 1 行目、札は md のパス）。
fn node_heading(node: &CanvasNode) -> String {
    match &node.kind {
        CanvasNodeKind::Group { label, .. } => label.clone().unwrap_or_default(),
        CanvasNodeKind::Text { text } => text.lines().next().unwrap_or("").to_string(),
        CanvasNodeKind::Link { url } => url.clone(),
        CanvasNodeKind::File { file, .. } => {
            if file.is_empty() {
                match (
                    node.extra.get("saiTitle").and_then(serde_json::Value::as_str),
                    node.extra.get("saiPaperId").and_then(serde_json::Value::as_i64),
                ) {
                    (Some(title), _) => title.to_string(),
                    (None, Some(paper_id)) => format!("#{paper_id}"),
                    (None, None) => String::new(),
                }
            } else {
                file.clone()
            }
        }
        CanvasNodeKind::Unknown { kind } => kind.clone(),
    }
}

/// そのノードが入っているグループの id（枠の中に完全に入っているとき）。
pub(crate) fn group_of(canvas: &Canvas, node: &CanvasNode) -> Option<String> {
    canvas
        .nodes
        .iter()
        .find(|candidate| {
            matches!(candidate.kind, CanvasNodeKind::Group { .. })
                && candidate.id != node.id
                && node.x >= candidate.x
                && node.y >= candidate.y
                && node.x + node.width <= candidate.x + candidate.width
                && node.y + node.height <= candidate.y + candidate.height
        })
        .map(|candidate| candidate.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai_cli_canvas_{label}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn non_draft_canvas_requires_force() {
        let dir = temp_dir("force");
        let canvas = canvas_edit::new_canvas("user-map", false);
        canvas_edit::save(&dir, "user-map", &canvas).unwrap();

        let error = load_for_edit(&dir, "user-map", false).unwrap_err();
        assert!(
            format!("{error}").contains("--force"),
            "エラー文に --force がありません: {error}"
        );
        // --force なら開ける。
        assert!(load_for_edit(&dir, "user-map", true).is_ok());
        // 無い Canvas は --force が無ければ作らない（canvas new を促す）。
        let error = load_for_edit(&dir, "new-map", false).unwrap_err();
        assert!(format!("{error}").contains("canvas new"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
