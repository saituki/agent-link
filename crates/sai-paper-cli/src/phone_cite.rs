//! `sai-paper phone citations` / `citations-extract` / `manual-candidates` /
//! `manual-import` — sai-phone（Web）の Citations・Import 画面用（U4）。
//!
//! sai-phone は sai-paper の DB を直接開かず、このコマンドの JSON だけを読む。
//! 共通ルール（`crates/sai-paper-cli/src/phone.rs` と同じ）:
//!
//! - 成功: 終了コード 0、標準出力に JSON 1 行
//! - 失敗: 終了コード 1、標準出力に `{"error": "<日本語の文>"}`
//!
//! 中の処理は core（`sai_paper_core::citation` と `sai_paper_core::manual_import`）が
//! 担う。ここは外から来る値の検査と JSON の形だけを持つ。
//! 図の抽出と PDF の翻訳はここではしない（`phone after-download <paper_id>` が担当）。

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::Connection;
use sai_paper_core::citation;
use sai_paper_core::citation::db::{RefRow, SentenceRow, get_refs_for, get_sentences_for};
use sai_paper_core::manual_import::{ManualTarget, import_pdf};
use sai_paper_core::models::{LibraryMatchEntry, RemotePaper};
use sai_paper_core::{Database, Paths, S2Client, library_tree, manual_import, search_log,
    search_sets};
use serde_json::{Value, json};

use crate::phone::{library_papers, open_context};

/// `phone manual-import --stdin` で受け取る PDF の上限（100MB）。
const MAX_UPLOAD_BYTES: usize = 104_857_600;

/// 引用文を Web へ返すときの長さの上限（文字数）。
const SENTENCE_CHARS: usize = 300;

/// `phone citations <paper_id>`: 引用一覧を JSON で返す（読むだけ）。
///
/// 抜き出し済みの論文と未済みの論文で同じ形を返す。`extracted` と `has_pdf` で
/// Web 側が「引用の抜き出しが要るか」を判断する。
pub fn citations(paper_id: i64) -> Result<Value> {
    let (db, _cfg, _paths) = open_context()?;
    let Some(paper) = db.library_paper_by_id(paper_id)? else {
        bail!("論文が見つかりません");
    };

    // 抜き出すときのキー（`ArXiv:…` / `DOI:…` / 40hex の正規形）。
    let key = db
        .citation_key_for_paper(paper_id)
        .map(|raw| citation::normalize_paper_id(&raw));

    // PDF は DB に残ったパスが実在するときだけ「ある」とする。
    let has_pdf = paper
        .pdf_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .is_some_and(|path| Path::new(path).is_file());

    let ref_rows =
        get_refs_for(db.connection(), Some(paper_id), key.as_deref()).unwrap_or_default();
    let sentences =
        get_sentences_for(db.connection(), Some(paper_id), key.as_deref()).unwrap_or_default();
    let entries = db.library_match_entries().unwrap_or_default();

    let refs: Vec<Value> = ref_rows
        .iter()
        .map(|row| ref_json(row, &sentences, &entries))
        .collect();

    Ok(json!({
        "paper": { "id": paper.id, "title": paper.title },
        "key": key,
        "has_pdf": has_pdf,
        "extracted": !ref_rows.is_empty() || !sentences.is_empty(),
        "refs": refs,
    }))
}

/// 引用 1 行の JSON（C1）。`sentences` はその行を引用している引用文だけ。
fn ref_json(
    row: &RefRow,
    sentences: &[SentenceRow],
    entries: &[LibraryMatchEntry],
) -> Value {
    let title = non_empty(row.s2_title.clone()).or_else(|| non_empty(row.title.clone()));
    // Library にあるか（PDF が実在する論文だけ。引用 TUI の緑色表示と同じ決まり）。
    let local_paper_id = citation::library::library_match(
        entries,
        row.s2_paper_id.as_deref(),
        row.doi.as_deref(),
        title.as_deref(),
    )
    .map(|entry| entry.id);
    json!({
        "ref_no": row.ref_no,
        "title": title,
        "first_author": row.first_author,
        "year": row.year.or(row.s2_year),
        "journal": non_empty(row.journal.clone()).or_else(|| non_empty(row.s2_venue.clone())),
        "doi": non_empty(row.doi.clone()),
        "s2_paper_id": non_empty(row.s2_paper_id.clone()),
        "raw_text": row.raw_text,
        "local_paper_id": local_paper_id,
        "url": citation::library::ref_site_url(row),
        // ID か DOI か題名があれば取りに行ける（題名だけのときは S2 の題名検索で ID を探す）。
        "can_download": row.s2_paper_id.is_some() || row.doi.is_some() || title.is_some(),
        // 取り込みの出し先を作れるか（ID か DOI が要る）。
        "can_import": citation::library::ref_remote_paper(row).is_some(),
        "sentences": sentences_json(row.ref_no, sentences),
    })
}

/// 引用 1 行を引用している引用文（`sent_no` の順・`text` は 300 字まで）。
fn sentences_json(ref_no: i64, sentences: &[SentenceRow]) -> Vec<Value> {
    let mut items: Vec<&SentenceRow> = sentences
        .iter()
        .filter(|sentence| cites_ref(&sentence.cite_refs, ref_no))
        .collect();
    items.sort_by_key(|sentence| sentence.sent_no);
    items
        .into_iter()
        .map(|sentence| {
            json!({
                "sent_no": sentence.sent_no,
                "text": sentence.text.chars().take(SENTENCE_CHARS).collect::<String>(),
            })
        })
        .collect()
}

/// `cite_refs`（引用 ref_no の JSON 配列）の中に ref_no があるか。
fn cites_ref(cite_refs: &str, ref_no: i64) -> bool {
    let Ok(list) = serde_json::from_str::<Vec<i64>>(cite_refs) else {
        return false;
    };
    list.contains(&ref_no)
}

/// 空文字と前後の空白だけの値を除く（引用の行の項目をそろえる）。
fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// `phone citations-extract <paper_id>`: 引用を抜き出す（docling ごと。数十秒）。
///
/// 段は `0 = PDF確認`（この命令の前）、`1 = 引用抽出`、`2 = 完了`（Web 側）。
/// docling が無い・失敗したときは成功として扱い、S2 の引用だけが入ることを
/// `docling_error` で伝える（TUI と同じ扱い）。
pub async fn citations_extract(paper_id: i64, client: &S2Client) -> Result<Value> {
    let (db, _cfg, paths) = open_context()?;
    let Some(paper) = db.library_paper_by_id(paper_id)? else {
        bail!("論文が見つかりません");
    };
    let Some(key) = db
        .citation_key_for_paper(paper_id)
        .map(|raw| citation::normalize_paper_id(&raw))
        .filter(|key| !key.is_empty())
    else {
        bail!("引用を抜き出すには arXiv ID か DOI が要ります");
    };
    let pdf_path = paper
        .pdf_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .ok_or_else(|| anyhow::anyhow!("PDF がありません。先に PDF を取り込んでください"))?;

    eprintln!("sai-progress step=1");
    let data_dir = paths.data_dir();
    match citation::ensure_extract(db.connection(), client, &key, &pdf_path, &data_dir).await {
        Ok(output) => Ok(json!({
            "paper_id": paper_id,
            "refs": output.refs.len(),
            "sentences": output.sentences.len(),
            "docling_error": null,
        })),
        // docling の失敗は部分成功として扱う（S2 の引用だけが入る）。
        Err(citation::ExtractError::Docling(reason)) => Ok(json!({
            "paper_id": paper_id,
            "refs": stored_count(&db, paper_id, &key, true),
            "sentences": stored_count(&db, paper_id, &key, false),
            "docling_error": reason,
        })),
        Err(error) => Err(anyhow::anyhow!("引用を抜き出せませんでした: {error}")),
    }
}

/// DB に残っている引用文献・引用文の件数（docling が失敗したあとで使う）。
fn stored_count(db: &Database, paper_id: i64, key: &str, refs: bool) -> i64 {
    let count = if refs {
        get_refs_for(db.connection(), Some(paper_id), Some(key)).map(|rows| rows.len())
    } else {
        get_sentences_for(db.connection(), Some(paper_id), Some(key)).map(|rows| rows.len())
    };
    i64::try_from(count.unwrap_or(0)).unwrap_or(i64::MAX)
}

/// `phone manual-candidates [--minutes <N>]`: ダウンロードフォルダの最近の PDF 一覧。
///
/// 返すのはファイル名だけ（フルパスは返さない）。フォルダが無いときは
/// `dir` が `null` になる。
pub fn manual_candidates(minutes: u64) -> Result<Value> {
    let Some(dir) = manual_import::downloads_dir() else {
        return Ok(json!({ "dir": null, "minutes": minutes, "items": [] }));
    };
    let window = Duration::from_secs(minutes.saturating_mul(60));
    let items: Vec<Value> = manual_import::recent_pdfs(&dir, window)
        .iter()
        .filter_map(|path| {
            let meta = std::fs::metadata(path).ok()?;
            let name = path.file_name()?.to_string_lossy().into_owned();
            Some(json!({
                "name": name,
                "size": meta.len(),
                "modified_at": modified_at(&meta),
            }))
        })
        .collect();
    Ok(json!({
        "dir": display_dir(&dir),
        "minutes": minutes,
        "items": items,
    }))
}

/// ファイルの更新時刻（Web が見せる形式。ローカルの時刻つき）。
fn modified_at(meta: &std::fs::Metadata) -> Option<String> {
    let modified = meta.modified().ok()?;
    let stamp: chrono::DateTime<chrono::Local> = modified.into();
    Some(stamp.to_rfc3339())
}

/// ホームを `~` に置き換えた表示用のフォルダ名。
fn display_dir(dir: &Path) -> String {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let Some(home) = home else {
        return dir.to_string_lossy().into_owned();
    };
    match dir.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => dir.to_string_lossy().into_owned(),
    }
}

/// `phone manual-import` の引数（clap が受け取る形）。
///
/// 対象（`target`）と中身（`source`）は、それぞれ 1 つだけ必ず指定する。
#[derive(clap::Args, Debug)]
#[command(group(
    clap::ArgGroup::new("target")
        .required(true)
        .multiple(false)
        .args(["log", "import_id", "cite", "paper_id"]),
))]
#[command(group(
    clap::ArgGroup::new("source")
        .required(true)
        .multiple(false)
        .args(["file", "stdin"]),
))]
pub struct ManualImportArgs {
    /// 検索履歴 ID（`--position` とセットで指定）
    #[arg(long)]
    pub log: Option<i64>,
    /// 検索履歴の中での位置（0 始まり。`--log` とセットで指定）
    #[arg(long, requires = "log")]
    pub position: Option<i64>,
    /// BibTeX からの `RemotePaper.id`（例: `ArXiv:1706.03762`）
    #[arg(long = "import-id")]
    pub import_id: Option<String>,
    /// 引用を持つ論文 ID（`--ref` とセットで指定）
    #[arg(long)]
    pub cite: Option<i64>,
    /// 引用の番号（`--cite` とセットで指定）
    #[arg(long = "ref", requires = "cite")]
    pub ref_no: Option<i64>,
    /// Library に既にいる論文 ID（その論文へ PDF を付ける）
    #[arg(long)]
    pub paper_id: Option<i64>,
    /// 取り込むファイル名（ダウンロードフォルダの最近の PDF だけを受ける）
    #[arg(long)]
    pub file: Option<String>,
    /// `--file` の候補を何分前まで見るか（既定 5 分）
    #[arg(long, requires = "file", default_value_t = 5)]
    pub minutes: u64,
    /// 標準入力の中身（Web のアップロード）を取り込む
    #[arg(long)]
    pub stdin: bool,
}

/// `phone manual-import`: ブラウザで保存した PDF（またはアップロード）を
/// Library に入れる。
///
/// 対象は 1 つだけ（`--log` / `--import-id` / `--cite` / `--paper-id`）、
/// 中身は `--file` か `--stdin` の 1 つだけ。図の抽出と PDF の翻訳はしない
/// （Web が続けて `phone after-download <paper_id>` を呼ぶ）。
pub fn manual_import_command(args: &ManualImportArgs) -> Result<Value> {
    let (mut db, cfg, paths) = open_context()?;
    let data_dir = paths.data_dir();
    // PDF の置き場は `phone download` と同じ（設定の library.download_path、無ければ
    // .sai/paper/papers）。取り込んだ先の一時ファイルにもここを使う。
    let download_dir = cfg
        .library
        .download_path
        .clone()
        .unwrap_or_else(|| paths.downloads_dir.clone());
    std::fs::create_dir_all(&download_dir)
        .with_context(|| format!("保存先を作れませんでした: {}", download_dir.display()))?;

    // 取り込み先を決める。Remote で既に Library にあるときは取り込まずに落とす。
    let target = match manual_target(&db, &paths, args)? {
        target @ ManualTarget::Remote(_) => {
            let ManualTarget::Remote(remote) = &target else {
                unreachable!("Remote のときだけここに来る");
            };
            let library = library_papers(&db, &cfg, &paths)?;
            if let Some(index) = library_tree::match_downloaded_paper(&library, remote) {
                bail!("この論文は既に Library にあります（ID {}）", library[index].id);
            }
            target
        }
        local => local,
    };

    // 中身を決める。--file はダウンロードフォルダ（ブラウザの保存先）の候補から 1 つ選ぶ。
    let source = match (&args.file, args.stdin) {
        (Some(name), false) => {
            check_file_name(name)?;
            let browser_dir = manual_import::downloads_dir()
                .context("ダウンロードフォルダが見つかりません")?;
            find_recent_pdf(&browser_dir, name, args.minutes).ok_or_else(|| {
                anyhow::anyhow!(format!(
                    "ダウンロードフォルダの最近の PDF に {name} がありません"
                ))
            })?
        }
        (None, true) => {
            let temporary = download_dir.join(format!(".sai-upload-{}.pdf.part", std::process::id()));
            if let Err(error) = write_upload(&temporary) {
                // 書けなかった一時ファイルは残さない。
                let _ = std::fs::remove_file(&temporary);
                return Err(error);
            }
            temporary
        }
        _ => bail!("中身は --file か --stdin のどちらかを指定してください"),
    };
    let is_upload = args.stdin;

    let imported = match import_pdf(&mut db, &data_dir, &download_dir, &source, &target) {
        Ok(imported) => imported,
        Err(error) => {
            // 取り込めなかったアップロードの一時ファイルも残さない。
            if is_upload {
                let _ = std::fs::remove_file(&source);
            }
            bail!("{error}");
        }
    };
    if is_upload {
        // アップロードの一時ファイルは取り込みの後で消す。
        if let Err(error) = std::fs::remove_file(&source)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("一時ファイルを消せませんでした: {error}");
        }
    }

    // 検索履歴から取り込んだときは、階層表示のためその記録へ紐付ける（TUI と同じ）。
    if let (Some(log_id), ManualTarget::Remote(remote)) = (args.log, &target) {
        link_to_search_log(db.connection(), log_id, remote, imported.paper_id);
    }

    let title = db
        .library_paper_by_id(imported.paper_id)
        .ok()
        .flatten()
        .map(|paper| paper.title)
        .unwrap_or_default();
    Ok(json!({
        "paper_id": imported.paper_id,
        "title": title,
        "status": "imported",
        "pdf_path": imported.path.to_string_lossy(),
    }))
}

/// 取り込み先を作る。検索履歴・BibTeX・引用・既存の論文の 4 通り。
fn manual_target(db: &Database, paths: &Paths, args: &ManualImportArgs) -> Result<ManualTarget> {
    match (args.log, args.import_id.as_deref(), args.cite, args.paper_id) {
        (Some(log_id), None, None, None) => {
            let position = args
                .position
                .context("phone manual-import には --log と --position の両方が必要です")?;
            let remote = search_log::items(db.connection(), log_id)?
                .into_iter()
                .find(|item| item.position == position)
                .map(|item| item.remote)
                .context("論文が見つかりません")?;
            Ok(ManualTarget::Remote(remote))
        }
        (None, Some(import_id), None, None) => {
            let bibtex_dir = paths.data_dir().join("bibtex");
            let remote = sai_paper_core::bibtex::load_bibtex_dir_detailed(&bibtex_dir)?
                .papers
                .into_iter()
                .find(|paper| paper.id == import_id)
                .context("論文が見つかりません")?;
            Ok(ManualTarget::Remote(remote))
        }
        (None, None, Some(paper_id), None) => {
            let row = find_ref_row(db, paper_id, args.ref_no)?;
            let remote = citation::library::ref_remote_paper(&row).ok_or_else(|| {
                anyhow::anyhow!("この引用は S2 の ID も DOI も無いので取り込めません")
            })?;
            Ok(ManualTarget::Remote(remote))
        }
        (None, None, None, Some(paper_id)) => {
            let Some(paper) = db.library_paper_by_id(paper_id)? else {
                bail!("論文が見つかりません");
            };
            Ok(ManualTarget::Local {
                paper_id,
                title: paper.title,
            })
        }
        _ => bail!(
            "対象は --log と --position、--import-id、--cite と --ref、--paper-id のどれか 1 つです"
        ),
    }
}

/// 引用 1 行（`--cite <paper_id> --ref <ref_no>`）を DB から取り出す。
pub fn find_ref_row(db: &Database, paper_id: i64, ref_no: Option<i64>) -> Result<RefRow> {
    let ref_no = ref_no.context("--cite には --ref の両方が必要です")?;
    let key = db
        .citation_key_for_paper(paper_id)
        .map(|raw| citation::normalize_paper_id(&raw));
    get_refs_for(db.connection(), Some(paper_id), key.as_deref())?
        .into_iter()
        .find(|row| row.ref_no == ref_no)
        .with_context(|| format!("引用 {ref_no} が見つかりません"))
}

/// `--file` の名前の検査（ファイル名 1 個だけ。パスも `..` も受け付けない）。
fn check_file_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("ファイル名を指定してください");
    }
    if name.contains(['/', '\\', '\0']) || name.contains("..") {
        bail!("ファイル名だけを指定してください");
    }
    Ok(())
}

/// ダウンロードフォルダの最近の PDF から、名前が一致するもの 1 つを選ぶ。
fn find_recent_pdf(dir: &Path, name: &str, minutes: u64) -> Option<PathBuf> {
    let window = Duration::from_secs(minutes.saturating_mul(60));
    manual_import::recent_pdfs(dir, window)
        .into_iter()
        .find(|path| path.file_name().is_some_and(|file| file == name))
}

/// 標準入力の PDF（100MB まで）を一時ファイルへ書く。
fn write_upload(path: &Path) -> Result<()> {
    let mut buffer = Vec::new();
    std::io::stdin()
        .lock()
        .take(u64::try_from(MAX_UPLOAD_BYTES).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut buffer)
        .context("標準入力を読めませんでした")?;
    if buffer.len() > MAX_UPLOAD_BYTES {
        // 大きすぎるときは書かずに落とす（一時ファイルは残さない）。
        bail!("PDF が大きすぎます（100MB まで）");
    }
    std::fs::write(path, &buffer)
        .with_context(|| format!("一時ファイルを書けませんでした: {}", path.display()))
}

/// 検索履歴へ紐付ける（TUI の `link_to_discover_log` と同じ紐付け）。
///
/// 記録の中に無い論文なら 1 件足してから紐付ける。失敗しても取り込みは失敗にしない。
fn link_to_search_log(conn: &Connection, log_id: i64, remote: &RemotePaper, paper_id: i64) {
    let key = sai_paper_core::api::dedupe_key(remote);
    let position_in_log = || {
        search_log::items(conn, log_id)
            .ok()
            .and_then(|items| {
                items
                    .into_iter()
                    .find(|item| item.dedupe_key == key)
                    .map(|item| item.position)
            })
    };
    let position = match position_in_log() {
        Some(position) => position,
        None => {
            // 記録の候補に無いので、いまの検索の 1 件として足してから紐付ける。
            if search_log::append_items(conn, log_id, std::slice::from_ref(remote)).is_err() {
                return;
            }
            match position_in_log() {
                Some(position) => position,
                None => return,
            }
        }
    };
    if let Err(error) = search_sets::link_paper(conn, log_id, position, paper_id) {
        eprintln!("検索の記録への紐付けに失敗: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_UPLOAD_BYTES, check_file_name, display_dir, sentences_json};
    use sai_paper_core::citation::db::{RefRow, SentenceRow};
    use sai_paper_core::{Database, citation};
    use std::path::{Path, PathBuf};

    /// 引用 1 行だけを持つ論文を用意する（テスト用）。
    fn db_with_refs() -> (PathBuf, Database) {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-phone-cite-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".sai/paper")).unwrap();
        let db = Database::open(&dir.join(".sai/paper/papr.db")).unwrap();
        db.connection()
            .execute(
                "INSERT INTO papers (title, doi) VALUES ('Ref Host', '10.1/host')",
                [],
            )
            .unwrap();
        let paper_id = db.connection().last_insert_rowid();
        citation::db::replace_refs(
            db.connection(),
            "ArXiv:1706.03762",
            Some(paper_id),
            &[RefRow {
                paper_id: "ArXiv:1706.03762".to_owned(),
                ref_no: 1,
                first_author: Some("Vaswani".to_owned()),
                year: Some(2017),
                journal: Some("NeurIPS".to_owned()),
                title: Some("Attention Is All You Need".to_owned()),
                doi: Some("10.5555/3295222".to_owned()),
                raw_text: "Vaswani et al. 2017".to_owned(),
                s2_paper_id: Some("abcdef0123456789abcdef0123456789abcdef01".to_owned()),
                ..RefRow::default()
            }],
        )
        .unwrap();
        (dir, db)
    }

    /// アップロードの上限は 100MB。
    #[test]
    fn upload_limit_is_one_hundred_mib() {
        assert_eq!(MAX_UPLOAD_BYTES, 104_857_600);
    }

    /// 引用 1 行を `--cite` / `--ref` で取り出せる（対象の組み立て）。
    #[test]
    fn cite_target_comes_from_the_ref_row() {
        let (dir, db) = db_with_refs();
        let row = super::find_ref_row(&db, 1, Some(1)).expect("引用 1 行を取り出せる");
        let remote = citation::library::ref_remote_paper(&row).expect("登録用の論文が作れる");
        assert_eq!(remote.id, "abcdef0123456789abcdef0123456789abcdef01");
        assert_eq!(remote.title, "Attention Is All You Need");
        assert_eq!(remote.journal_ref.as_deref(), Some("NeurIPS"));
        // 番号の違う引用は取れない
        assert!(super::find_ref_row(&db, 1, Some(2)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 引用文はその行を引用している物だけを `sent_no` の順で返す。
    #[test]
    fn sentences_are_limited_to_the_ref_and_ordered() {
        let rows = vec![
            SentenceRow {
                paper_id: "k".to_owned(),
                sent_no: 5,
                text: "後の文".to_owned(),
                cite_refs: "[1]".to_owned(),
            },
            SentenceRow {
                paper_id: "k".to_owned(),
                sent_no: 2,
                text: "前の文".to_owned(),
                cite_refs: "[1, 3]".to_owned(),
            },
            SentenceRow {
                paper_id: "k".to_owned(),
                sent_no: 3,
                text: "別の引用".to_owned(),
                cite_refs: "[2]".to_owned(),
            },
        ];
        let items = sentences_json(1, &rows);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["sent_no"], 2);
        assert_eq!(items[1]["sent_no"], 5);
        assert!(sentences_json(9, &rows).is_empty());
    }

    /// ファイル名は 1 個だけ。パスや `..` は受け付けない。
    #[test]
    fn file_name_must_be_a_plain_name() {
        assert!(check_file_name("paper.pdf").is_ok());
        assert!(check_file_name("").is_err());
        assert!(check_file_name("../paper.pdf").is_err());
        assert!(check_file_name("dir/paper.pdf").is_err());
        assert!(check_file_name("dir\\paper.pdf").is_err());
        assert!(check_file_name("a..b.pdf").is_err());
    }

    /// ホーム配下は `~/…` の表示になる。
    #[test]
    fn display_dir_shortens_the_home_prefix() {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .expect("ホームが分かる");
        assert_eq!(
            display_dir(&home.join("Downloads")),
            "~/Downloads".to_owned()
        );
        assert_eq!(
            display_dir(Path::new("/tmp/elsewhere")),
            "/tmp/elsewhere".to_owned()
        );
    }
}
