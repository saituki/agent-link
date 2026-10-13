//! research-tree の複製と変換（`sai-sci convert`）。
//!
//! 既存のプロジェクト（A → Q → A の木）を新しいプロジェクトのフォルダへ複製し、複製先の木で「A の下の Q」を畳んで A の下に A を置く。
//! **元のプロジェクトには書かない。** 元の `papr.db` は読み取り専用で `VACUUM INTO` し、開くのは複製先だけにする（元に移行が走らないように）。
//! 変換は利用者・agent の操作ではないので `ops` を通さず、`store` を直接使う。記録は `rt_log` に actor `convert` で残す。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OpenFlags, params};
use sai_paper_core::{Database, paths::PaperRoots};
use serde::Serialize;

use super::{Kind, RtError, defs, export, fmt_id, store};
use store::NodeRow;

/// 複製しないファイル（`papr.db` は `VACUUM INTO` で作る。`*-journal` は残すと複製先が壊れるので除く）。
const SKIP_FILES: [&str; 7] = [
    "papr.db",
    "papr.db-wal",
    "papr.db-shm",
    "papr.db-journal",
    "state.md",
    "tree.md",
    "tree-idmap.json",
];
/// 中身を写さず、空で作るフォルダ。
const EMPTY_DIRS: [&str; 2] = ["inbox", "outbox"];
/// 対応表のファイル名（複製先の `.sai/paper/` の中）。
pub const IDMAP_FILE: &str = "tree-idmap.json";
/// `rt_log` の actor。
const ACTOR: &str = "convert";

/// 木の数（生きた Node）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub a: usize,
    pub f: usize,
    pub s: usize,
    pub d: usize,
    /// 親の無い生きた Q（軸）。
    pub q_axis: usize,
    /// 親のある生きた Q。
    pub q_other: usize,
}

/// 変換の報告（`--json` の形）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConvertReport {
    pub from: String,
    pub to: String,
    pub before: Counts,
    pub after: Counts,
    /// 畳んだ Q の数。
    pub flattened_q: usize,
    /// 付け替えた A の数。
    pub moved_a: usize,
    /// 対応表のパス（`<to>/.sai/paper/tree-idmap.json`）。
    pub idmap: String,
    /// 変換の前後で、A・F・E・S・D の木の順が同じか。
    pub ord_exact: bool,
}

/// `from_root` の research-tree を `to_root` へ複製し、畳んだ木にする。
///
/// # Errors
///
/// 元の `papr.db` が無いとき（NotFound）・`from` と `to` が同じか入れ子のとき（Invalid）・
/// `to` が既にあるとき（Conflict）・複製か変換に失敗したとき（その他）。失敗したときは `to` を消す。
pub fn convert(from_root: &Path, to_root: &Path) -> Result<ConvertReport> {
    let from_data = from_root.join(".sai").join("paper");
    if !from_data.join("papr.db").is_file() {
        return Err(not_found(format!(
            "元の papr.db がありません: {}",
            from_data.join("papr.db").display()
        )));
    }
    let (from_full, to_full) = (resolve(from_root), resolve(to_root));
    if from_full == to_full || to_full.starts_with(&from_full) || from_full.starts_with(&to_full) {
        return Err(invalid("--from と --to が同じか、入れ子になっています"));
    }
    if to_root.exists() {
        return Err(conflict(format!(
            "--to が既にあります: {}",
            to_root.display()
        )));
    }
    let built = build(
        from_root,
        to_root,
        &from_data,
        &to_root.join(".sai").join("paper"),
    );
    if built.is_err() {
        // `--to` は最初に無かったことを確かめている。作りかけは全部消す。
        let _ = fs::remove_dir_all(to_root);
    }
    built
}

/// 複製・変換・書き出しを順に行う。途中の失敗は呼び出し側で `to` ごと消す。
fn build(
    from_root: &Path,
    to_root: &Path,
    from_data: &Path,
    to_data: &Path,
) -> Result<ConvertReport> {
    fs::create_dir_all(to_data)?;
    copy_tree(from_data, to_data, true)?;
    for dir in EMPTY_DIRS {
        fs::create_dir_all(to_data.join(dir))?;
    }
    let to_db = to_data.join("papr.db");
    snapshot(&from_data.join("papr.db"), &to_db)?;

    let db = Database::open(&to_db).context("複製先の papr.db を開けませんでした")?;
    let conn = db.connection();
    let before = counts(conn)?;
    let order_before = flat_order(conn)?;
    let flat = flatten(conn)?;
    let ord_exact = flat_order(conn)? == order_before;
    let after = counts(conn)?;

    let idmap = to_data.join(IDMAP_FILE);
    fs::write(&idmap, idmap_json(&flat.idmap)?)?;
    let roots = PaperRoots {
        root: to_root.to_path_buf(),
        data_dir: to_data.to_path_buf(),
    };
    // 元の RESEARCH_TYPE と GUIDE は元のプロジェクトから読む（読むだけ）。
    export::export(conn, &roots, &defs::load(from_root))
        .context("state.md と tree.md を書き出せませんでした")?;

    Ok(ConvertReport {
        from: from_root.display().to_string(),
        to: to_root.display().to_string(),
        before,
        after,
        flattened_q: flat.flattened,
        moved_a: flat.moved_a,
        idmap: idmap.display().to_string(),
        ord_exact,
    })
}

/// 元の `papr.db` を読み取り専用で開き、`VACUUM INTO` で一貫した複製を作る。元には書かない。
fn snapshot(from_db: &Path, to_db: &Path) -> Result<()> {
    let copied = Connection::open_with_flags(
        from_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .and_then(|conn| {
        conn.execute(
            "VACUUM INTO ?1",
            params![to_db.to_string_lossy().into_owned()],
        )
    });
    copied.map(|_| ()).map_err(|err| {
        anyhow!("元の papr.db を読み取り専用で複製できませんでした。動いているコンテナを止めてから取り直してください: {err}")
    })
}

/// `.sai/paper/` の中身を写す。`top` のときだけ `SKIP_FILES` と `EMPTY_DIRS` を見る。シンボリックリンクはたどらず、写さない。
fn copy_tree(from: &Path, to: &Path, top: bool) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if top && SKIP_FILES.contains(&name_text.as_ref()) {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink()
            || (top && file_type.is_dir() && EMPTY_DIRS.contains(&name_text.as_ref()))
        {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if file_type.is_dir() {
            copy_tree(&src, &dst, false)?;
        } else if file_type.is_file() {
            fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// 畳んだ結果。`idmap` は（消した Q の ID, 付け替え先の ID）。
#[derive(Default)]
struct Flat {
    flattened: usize,
    moved_a: usize,
    idmap: Vec<(i64, String)>,
}

/// 軸（親の無い Q）以外の生きた Q を、木の順（親の Q を先に）で畳む。1 つの transaction で行う。
fn flatten(conn: &Connection) -> Result<Flat> {
    let tx = conn.unchecked_transaction()?;
    let mut flat = Flat::default();
    let qs: Vec<i64> = preorder(&tx)?
        .iter()
        .filter(|n| n.kind == Kind::Q && n.parent_id.is_some())
        .map(|n| n.id)
        .collect();
    for id in qs {
        // 親の Q を先に畳んでいるので、ここで親を読み直す（子の Q は親の付け替え先へ移っている）。
        let q = store::get_node(&tx, id)?;
        let Some(parent_id) = q.parent_id else {
            continue;
        };
        let parent = store::get_node(&tx, parent_id)?;
        flatten_one(&tx, &q, &parent, &mut flat)?;
    }
    tx.commit()?;
    Ok(flat)
}

/// Q 1 つを畳む。子（A・Q）を Q の位置へ差し込み、Q を消す。
fn flatten_one(conn: &Connection, q: &NodeRow, parent: &NodeRow, flat: &mut Flat) -> Result<()> {
    let kids = store::children(conn, q.id)?;
    if let Some(other) = kids
        .iter()
        .find(|kid| !matches!(kid.kind, Kind::A | Kind::Q))
    {
        bail!(
            "{} の下に {} があるので畳めません",
            fmt_id(Kind::Q, q.id),
            fmt_id(other.kind, other.id)
        );
    }
    // 親の並びで Q をその子に置き換える（移す前の並びで作る）。
    let mut order = Vec::new();
    for sibling in store::children(conn, parent.id)? {
        if sibling.id == q.id {
            order.extend(kids.iter().map(|kid| kid.id));
        } else {
            order.push(sibling.id);
        }
    }
    let detail = format!(
        "{} から {} へ（convert）",
        fmt_id(Kind::Q, q.id),
        fmt_id(parent.kind, parent.id)
    );
    for kid in &kids {
        store::set_parent(conn, kid.id, parent.id)?;
        if kid.kind == Kind::A {
            store::set_rel(conn, kid.id, Some("subtype"))?;
            store::insert_log(conn, kid.id, "move_node", &detail, ACTOR, None)?;
            flat.moved_a += 1;
        }
    }
    for (index, id) in order.iter().enumerate() {
        set_ord(conn, *id, i64::try_from(index + 1)?)?;
    }
    // 「分け方の基準」は親が A のときだけ、親の本文へ 1 行足す（軸の説明は GUIDE が持つ）。
    if parent.kind == Kind::A
        && let Some(line) = basis_line(&q.text)
    {
        let body = add_line_once(&parent.body, &line);
        if body != parent.body {
            store::update_text(conn, parent.id, None, Some(&body))?;
        }
    }
    store::remove_subtree(conn, q.id)?;
    store::insert_log(
        conn,
        q.id,
        "remove",
        "convert: A の下の Q を畳んだ",
        ACTOR,
        None,
    )?;
    flat.flattened += 1;
    flat.idmap.push((q.id, fmt_id(parent.kind, parent.id)));
    Ok(())
}

/// 兄弟の並び（`ord`）を 1 から振り直す。`store` には無い、変換だけの操作。
fn set_ord(conn: &Connection, id: i64, ord: i64) -> Result<()> {
    conn.execute(
        "UPDATE rt_nodes SET ord = ?2 WHERE id = ?1",
        params![id, ord],
    )?;
    Ok(())
}

/// 旧い Q の文から「分け方の基準:」の行を取る。後ろが空なら無し。
fn basis_line(text: &str) -> Option<String> {
    ["分け方の基準:", "分け方の基準："]
        .iter()
        .filter_map(|mark| text.find(mark).map(|at| (at, mark.len())))
        .min()
        .map(|(at, len)| text[at + len..].trim())
        .filter(|rest| !rest.is_empty())
        .map(|rest| format!("分け方の基準: {rest}"))
}

/// 本文の末尾に 1 行足す。同じ行が既にあれば足さない（2 回変換しても 1 行のまま）。
fn add_line_once(body: &str, line: &str) -> String {
    if body.lines().any(|known| known.trim() == line) {
        return body.to_string();
    }
    if body.trim().is_empty() {
        return line.to_string();
    }
    format!("{}\n{line}", body.trim_end())
}

/// 木の全部の生きた Node を、木の順（親の前に子・兄弟は ord 順）で並べる。
fn preorder(conn: &Connection) -> Result<Vec<NodeRow>> {
    let mut out = Vec::new();
    for root in store::list_nodes(conn, false)?
        .iter()
        .filter(|n| n.parent_id.is_none())
    {
        for id in store::subtree_ids(conn, root.id)? {
            out.push(store::get_node(conn, id)?);
        }
    }
    Ok(out)
}

/// 木の順の、Q 以外の ID の並び（畳んだ前後で同じなら、A・F などの順は保たれている）。
fn flat_order(conn: &Connection) -> Result<Vec<i64>> {
    Ok(preorder(conn)?
        .iter()
        .filter(|n| n.kind != Kind::Q)
        .map(|n| n.id)
        .collect())
}

/// 生きた Node の数。
fn counts(conn: &Connection) -> Result<Counts> {
    let mut out = Counts::default();
    for node in store::list_nodes(conn, false)? {
        match node.kind {
            Kind::A => out.a += 1,
            Kind::F => out.f += 1,
            Kind::S => out.s += 1,
            Kind::D => out.d += 1,
            Kind::Q if node.parent_id.is_none() => out.q_axis += 1,
            Kind::Q => out.q_other += 1,
            Kind::E => {}
        }
    }
    Ok(out)
}

/// 対応表の JSON（キーは Q の番号の昇順。`{"Q12":"A5", …}`）。
fn idmap_json(pairs: &[(i64, String)]) -> Result<String> {
    let mut sorted = pairs.to_vec();
    sorted.sort_by_key(|(id, _)| *id);
    let items = sorted
        .iter()
        .map(|(id, to)| {
            Ok(format!(
                "{}:{}",
                serde_json::to_string(&fmt_id(Kind::Q, *id))?,
                serde_json::to_string(to)?
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(format!("{{{}}}\n", items.join(",")))
}

/// 実在の経路を正規化する。無い末尾（`--to` はまだ無いことがある）は親を正規化して付け足す。
fn resolve(path: &Path) -> PathBuf {
    if let Ok(full) = path.canonicalize() {
        return full;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => resolve(parent).join(name),
        _ => path.to_path_buf(),
    }
}

fn invalid(message: impl Into<String>) -> anyhow::Error {
    RtError::Invalid(message.into()).into()
}

fn not_found(message: impl Into<String>) -> anyhow::Error {
    RtError::NotFound(message.into()).into()
}

fn conflict(message: impl Into<String>) -> anyhow::Error {
    RtError::Conflict(message.into()).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use store::NewNode;

    /// 試験の一時フォルダ（並列でも名前がぶつからないように時刻と PID を入れる）。
    fn temp(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir =
            std::env::temp_dir().join(format!("sai-convert-{tag}-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(
        conn: &Connection,
        kind: Kind,
        parent: Option<i64>,
        text: &str,
        body: &str,
        cites: &[String],
    ) -> i64 {
        store::insert_node(
            conn,
            &NewNode {
                kind,
                parent_id: parent,
                guide_key: (kind == Kind::Q).then_some("guide-check"),
                text,
                body,
                stance: (kind == Kind::F).then_some("support"),
                cites,
                paper_id: None,
                run_id: None,
                report_id: None,
                mlflow_ref: None,
                dataset_ref: None,
                paths: &[],
                created_by: "test",
            },
        )
        .unwrap()
    }

    /// 旧い木の元のプロジェクトを作る。移行 0061・0063 を外して、元を移行 60 までの形にする。
    /// 書き込みの接続（`Database`）は開いたまま返す（WAL に未反映の行がある状態を再現する）。
    /// 木: 1 軸 Q / 2 A / 3 Q（分け方の基準つき）/ 4 A / 5 F / 6 A / 7 A（軸の下）/ 8 Q（Q の下の Q）/ 9 A / 10 S（paths=[4]）/ 11 D。
    fn make_origin(root: &Path) -> Database {
        let data = root.join(".sai").join("paper");
        fs::create_dir_all(data.join("papers").join("deep")).unwrap();
        fs::create_dir_all(data.join("outbox")).unwrap();
        fs::write(data.join("papers").join("deep").join("x.pdf"), b"%PDF-test").unwrap();
        fs::write(data.join("mind.md"), "# mind\n").unwrap();
        fs::write(data.join("outbox").join("old.txt"), "x").unwrap();
        fs::write(data.join("state.md"), "old state\n").unwrap();
        std::os::unix::fs::symlink(data.join("mind.md"), data.join("papers").join("link")).unwrap();

        let db = Database::open(&data.join("papr.db")).unwrap();
        let conn = db.connection();
        store::meta_set(conn, "initialized_at", "2026-10-01 00:00:00").unwrap();
        let cites = ["paper:1".to_string()];
        put(conn, Kind::Q, None, "軸", "", &[]); // 1
        put(conn, Kind::A, Some(1), "軸の下の答え", "", &[]); // 2
        put(
            conn,
            Kind::Q,
            Some(2),
            "動物の種を分ける。分け方の基準: 動物の種。",
            "",
            &[],
        ); // 3
        put(conn, Kind::A, Some(3), "ネコ", "", &[]); // 4
        put(conn, Kind::F, Some(4), "根拠", "", &cites); // 5
        put(conn, Kind::A, Some(3), "イヌ", "", &[]); // 6
        put(conn, Kind::A, Some(1), "別の答え", "", &[]); // 7
        put(conn, Kind::Q, Some(3), "細分の問い", "", &[]); // 8
        put(conn, Kind::A, Some(8), "細分の答え", "", &[]); // 9
        let sprout = put(conn, Kind::S, None, "道筋", "", &[]); // 10
        conn.execute("UPDATE rt_nodes SET paths = '[4]' WHERE id = ?1", [sprout])
            .unwrap();
        put(conn, Kind::D, None, "研究の制約", "", &[]); // 11
        // 旧い形にする（rel 列と移行 61 の記録を外す）。
        conn.execute_batch(
            "ALTER TABLE rt_nodes DROP COLUMN rel;
             DROP TABLE IF EXISTS paper_suggest_items;
             DROP TABLE IF EXISTS paper_suggests;
             DELETE FROM schema_migrations WHERE version >= 61;",
        )
        .unwrap();
        db
    }

    /// ファイルの中身。元が変わっていないことの確かめに、バイト列そのものを比べる。
    fn bytes(path: &Path) -> Vec<u8> {
        fs::read(path).unwrap()
    }

    /// 元の移行の最大の版（読み取り専用で開く）。
    fn schema_max(db: &Path) -> i64 {
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn convert_flattens_copy_and_leaves_origin_untouched() {
        let dir = temp("main");
        let (from, to) = (dir.join("from"), dir.join("to"));
        // 書き込みの接続を開いたまま変換する（WAL に未反映の行を含む元の再現）。
        let origin = make_origin(&from);
        let origin_db = from.join(".sai/paper/papr.db");
        let origin_bytes = bytes(&origin_db);
        assert_eq!(schema_max(&origin_db), 60);
        // WAL に未反映の行がある（これが無いと、WAL を読めるかの確かめにならない）。
        assert!(fs::metadata(from.join(".sai/paper/papr.db-wal")).is_ok_and(|m| m.len() > 0));

        let report = convert(&from, &to).unwrap();

        // 元の不変（AC3）。
        assert_eq!(bytes(&origin_db), origin_bytes);
        assert_eq!(schema_max(&origin_db), 60);
        drop(origin);

        assert_eq!(
            report.before,
            Counts {
                a: 5,
                f: 1,
                s: 1,
                d: 1,
                q_axis: 1,
                q_other: 2
            }
        );
        assert_eq!(
            report.after,
            Counts {
                a: 5,
                f: 1,
                s: 1,
                d: 1,
                q_axis: 1,
                q_other: 0
            }
        );
        assert_eq!((report.flattened_q, report.moved_a), (2, 3));
        assert!(report.ord_exact);

        // 複製先の中身（AC1）。
        let to_data = to.join(".sai/paper");
        for name in [
            "papr.db",
            "mind.md",
            "papers/deep/x.pdf",
            "tree.md",
            "tree-idmap.json",
        ] {
            assert!(to_data.join(name).is_file(), "{name}");
        }
        assert!(to_data.join("inbox").is_dir() && to_data.join("outbox").is_dir());
        assert_eq!(fs::read_dir(to_data.join("outbox")).unwrap().count(), 0);
        assert!(
            !to_data.join("papers/link").exists(),
            "シンボリックリンクは写さない"
        );
        assert_ne!(
            fs::read_to_string(to_data.join("state.md")).unwrap(),
            "old state\n",
            "state.md は作り直す"
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &fs::read_to_string(to_data.join(IDMAP_FILE)).unwrap()
            )
            .unwrap(),
            json!({"Q3": "A2", "Q8": "A2"})
        );

        // 複製先の木（AC2・AC5・AC6）。
        let copy = Database::open(&to_data.join("papr.db")).unwrap();
        let conn = copy.connection();
        assert!(
            store::list_nodes(conn, false)
                .unwrap()
                .iter()
                .all(|n| n.kind != Kind::Q || n.parent_id.is_none())
        );
        assert_eq!(counts(conn).unwrap(), report.after);
        assert_eq!(
            store::get_node(conn, 4).unwrap().rel.as_deref(),
            Some("subtype")
        );
        assert_eq!(
            store::get_node(conn, 9).unwrap().rel.as_deref(),
            Some("subtype")
        );
        assert_eq!(
            store::get_node(conn, 2).unwrap().rel,
            None,
            "軸の下の A は付け替えていない"
        );
        assert_eq!(store::get_node(conn, 10).unwrap().paths, vec![4]);
        let fact = store::get_node(conn, 5).unwrap();
        assert_eq!(
            (fact.cites.as_slice(), fact.stance.as_deref()),
            (&["paper:1".to_string()][..], Some("support"))
        );
        // 並び: A2 の下は 4・6・9 の順（Q3 の子の 4・6、Q8 の子の 9）。
        let kids: Vec<i64> = store::children(conn, 2)
            .unwrap()
            .iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(kids, vec![4, 6, 9]);
        // 分け方の基準は親の A の本文に 1 行だけ（AC5）。
        assert_eq!(
            store::get_node(conn, 2).unwrap().body,
            "分け方の基準: 動物の種。"
        );
        let log_moves: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM rt_log WHERE actor = 'convert' AND action = 'move_node'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let log_removes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM rt_log WHERE actor = 'convert' AND action = 'remove'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!((log_moves, log_removes), (3, 2));
    }

    #[test]
    fn convert_twice_gives_the_same_tree_and_converted_tree_is_a_no_op() {
        let dir = temp("twice");
        let (from, first, second) = (dir.join("from"), dir.join("first"), dir.join("second"));
        let _origin = make_origin(&from);
        let one = convert(&from, &first).unwrap();
        let two = convert(&from, &second).unwrap();
        assert_eq!((one.after, one.flattened_q), (two.after, two.flattened_q));
        let body = |to: &Path| {
            let db = Database::open(&to.join(".sai/paper/papr.db")).unwrap();
            store::get_node(db.connection(), 2).unwrap().body
        };
        assert_eq!(body(&first), body(&second));

        // 変換済みの木を変換しても何も変わらない（冪等）。
        let again = convert(&first, &dir.join("again")).unwrap();
        assert_eq!((again.flattened_q, again.moved_a), (0, 0));
        assert_eq!(again.after, one.after);
        assert!(again.ord_exact);
    }

    #[test]
    fn convert_refuses_missing_conflicting_and_nested_inputs() {
        let dir = temp("refuse");
        let from = dir.join("from");
        let _origin = make_origin(&from);
        let missing = convert(&dir.join("none"), &dir.join("to")).unwrap_err();
        assert!(matches!(
            missing.downcast_ref::<RtError>(),
            Some(RtError::NotFound(_))
        ));
        // 同じ・入れ子は、既にある `--to` より先に 2 で断る。
        let same = convert(&from, &from).unwrap_err();
        assert!(matches!(
            same.downcast_ref::<RtError>(),
            Some(RtError::Invalid(_))
        ));
        let nested = convert(&from, &from.join("inner")).unwrap_err();
        assert!(matches!(
            nested.downcast_ref::<RtError>(),
            Some(RtError::Invalid(_))
        ));
        let exists = dir.join("exists");
        fs::create_dir_all(&exists).unwrap();
        let conflict_err = convert(&from, &exists).unwrap_err();
        assert!(matches!(
            conflict_err.downcast_ref::<RtError>(),
            Some(RtError::Conflict(_))
        ));
        assert!(exists.is_dir(), "断ったフォルダは消さない");
    }

    #[test]
    fn failed_snapshot_removes_partial_copy_and_keeps_origin() {
        let dir = temp("broken");
        let (from, to) = (dir.join("from"), dir.join("to"));
        let data = from.join(".sai/paper");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("papr.db"), b"not a database").unwrap();
        let err = convert(&from, &to).unwrap_err();
        assert!(
            err.downcast_ref::<RtError>().is_none(),
            "読み取りの失敗は終了コード 1（その他）"
        );
        assert!(
            err.to_string()
                .contains("読み取り専用で複製できませんでした")
        );
        assert!(!to.exists(), "作りかけの複製は消す");
        assert_eq!(bytes(&data.join("papr.db")), b"not a database");
    }

    #[test]
    fn vacuum_into_reads_a_closed_wal_origin_read_only() {
        // 元を閉じた後（-wal・-shm が無い状態）でも、読み取り専用で複製できること（要確認の項目）。
        let dir = temp("closed");
        let (from, to) = (dir.join("from"), dir.join("to"));
        drop(make_origin(&from));
        assert!(!from.join(".sai/paper/papr.db-wal").exists());
        assert!(!from.join(".sai/paper/papr.db-shm").exists());
        let report = convert(&from, &to).unwrap();
        assert_eq!(report.after.q_other, 0);
        assert_eq!(report.after.a, 5);
    }

    #[test]
    fn basis_line_and_add_line_once() {
        assert_eq!(
            basis_line("問い 分け方の基準: 種。").as_deref(),
            Some("分け方の基準: 種。")
        );
        assert_eq!(
            basis_line("問い 分け方の基準：種。").as_deref(),
            Some("分け方の基準: 種。")
        );
        assert_eq!(basis_line("分け方の基準:   "), None);
        assert_eq!(basis_line("基準は無い"), None);
        assert_eq!(add_line_once("", "x"), "x");
        assert_eq!(add_line_once("本文\n", "x"), "本文\nx");
        assert_eq!(add_line_once("本文\nx", "x"), "本文\nx");
    }
}
