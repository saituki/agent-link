//! docling CLI を呼び出して PDF → Markdown 変換を行う。
//!
//! 実行形式: `docling <pdf> --to md --output <ディレクトリ>`
//! （`--output` は**ディレクトリ**指定。出力ファイル名は docling の規約に従う）
//!
//! 図抽出を使う場合は [`run_docling_with_images`] を使う。こちらは
//! `docling convert <pdf> --to md --to json --image-export-mode referenced`
//! を実行し、JSON から図の画像パス・キャプション・掲載ページを取り出す。
//!
//! 注意: docling の詳細な動作検証・精度調整は別作業。本モジュールは
//! パイプラインの呼び出し口を提供するのみ。

use std::path::{Path, PathBuf};

use regex::Regex;
use tokio::process::Command;

use super::ExtractError;

/// docling 実行バイナリの解決順序:
/// 1. 環境変数 `DOCLING_BIN`（明示指定）
/// 2. `$PATH` 上の `docling`
/// 3. プロジェクトルート / 親ディレクトリの uv venv（`<root>/.venv/bin/docling`）
fn resolve_docling_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("DOCLING_BIN")
        && !bin.trim().is_empty()
    {
        let p = PathBuf::from(bin);
        if p.is_file() {
            return p;
        }
    }
    if let Ok(out) = std::process::Command::new("which").arg("docling").output()
        && out.status.success()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return PathBuf::from(s);
        }
    }
    // uv venv に入っている場合（Web/.venv 等）
    let mut roots = Vec::new();
    if let Ok(root) = crate::config::Paths::discover() {
        // sai-paper ルート（database_file の親の親）
        if let Some(parent) = root.database_file.parent().and_then(Path::parent) {
            roots.push(parent.to_path_buf());
        }
        roots.push(
            root.config_file
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(".")),
        );
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    for root in roots {
        for candidate in [
            root.join(".venv/bin/docling"),
            // 親ディレクトリ（sai-paper が Web/sai-paper 配下に置かれているケース）
            root.parent()
                .map(|p| p.join(".venv/bin/docling"))
                .unwrap_or_default(),
        ] {
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    // 見つからなければ PATH 解決に委ねる（実行時に明確なエラーになる）
    PathBuf::from("docling")
}

/// docling が出力した図 1 件分の情報。
#[derive(Debug, Clone, Default)]
pub struct DoclingFigure {
    /// docling が書き出した画像ファイルのパス（docling の一時出力先）。
    pub image_path: PathBuf,
    /// 図ラベル（例: `Figure 1` / `Table 2`）。
    pub label: Option<String>,
    /// ラベルを除いたキャプション本文。
    pub caption: Option<String>,
    /// 掲載ページ（取得できる場合）。
    pub page: Option<i64>,
}

/// docling の実行結果。
#[derive(Debug)]
pub struct DoclingOutput {
    /// 抽出した Markdown 全文。
    pub markdown: String,
    /// Markdown を保存したパス（extracts/ 配下の正規化済みファイル名）。
    pub md_path: PathBuf,
    /// ジャーナルスタイル（統合優先のため常に "Generic"）。
    pub style: String,
    /// 抽出した図（`run_docling` では常に空）。
    pub figures: Vec<DoclingFigure>,
}

/// docling を実行して PDF を Markdown に変換する。
///
/// 画像は出力しない（従来どおりの引用抽出向け）。
///
/// # 引数
/// - `pdf_path`: 入力 PDF ファイルパス
/// - `out_dir`: docling の出力先**ディレクトリ**
/// - `save_md_path`: 読み取った Markdown の保存先（正規化済みファイル名）
///
/// # 前提
/// - `docling` が `$PATH` に存在すること
pub async fn run_docling(
    pdf_path: PathBuf,
    out_dir: PathBuf,
    save_md_path: PathBuf,
) -> Result<DoclingOutput, ExtractError> {
    run_docling_inner(pdf_path, out_dir, save_md_path, false).await
}

/// docling を実行して Markdown と図画像の両方を出力する。
///
/// `--image-export-mode referenced` により、図は PNG として
/// `<out_dir>/<stem>_artifacts/` に書き出され、JSON から参照される。
/// `out_dir` は**絶対パス**を渡すこと（Markdown / JSON 内の画像参照が絶対になる）。
pub async fn run_docling_with_images(
    pdf_path: PathBuf,
    out_dir: PathBuf,
    save_md_path: PathBuf,
) -> Result<DoclingOutput, ExtractError> {
    run_docling_inner(pdf_path, out_dir, save_md_path, true).await
}

/// docling 実行の共通部分。
///
/// `with_images` が false のときの引数は従来と完全に同一に保つ
/// （既存の引用抽出パイプラインの挙動を変えないため）。
async fn run_docling_inner(
    pdf_path: PathBuf,
    out_dir: PathBuf,
    save_md_path: PathBuf,
    with_images: bool,
) -> Result<DoclingOutput, ExtractError> {
    let mut cmd = Command::new(resolve_docling_bin());
    if with_images {
        // 画像付き出力は `convert` サブコマンド形式を使う。
        cmd.arg("convert");
    }
    cmd.arg(&pdf_path).arg("--to").arg("md");
    if with_images {
        cmd.arg("--to")
            .arg("json")
            .arg("--image-export-mode")
            .arg("referenced")
            .arg("--quiet");
    }
    cmd.arg("--output").arg(&out_dir);

    let output = cmd.output().await.map_err(|e| {
        ExtractError::Docling(format!("failed to execute docling: {e}（インストール済みか確認してください。DOCLING_BIN 環境変数で明示指定も可能です）"))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ExtractError::Docling(format!(
            "docling exited with {}: {}",
            output.status.code().unwrap_or(-1),
            stderr.chars().take(500).collect::<String>()
        )));
    }

    // 画像行は一時フォルダを指し、後で壊れたリンクになるので保存前に除く（図は別に保存する）。
    let markdown = strip_figure_images(&read_produced_markdown(&pdf_path, &out_dir)?);

    // 正規化済みファイル名で保存（後続の再利用・TUI の md プレビュー用）
    std::fs::write(&save_md_path, &markdown)?;

    let figures = if with_images {
        match find_produced_json(&pdf_path, &out_dir) {
            Some(json_path) => parse_figures_from_json(&json_path, &out_dir)?,
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };

    Ok(DoclingOutput {
        markdown,
        md_path: save_md_path,
        style: "Generic".to_string(),
        figures,
    })
}

/// 画像だけの行（`![Image](…)`・`<!-- image -->`）を行ごと除く。他の行は変えない。
///
/// 文中に画像がある行は触らない。
#[must_use]
pub fn strip_figure_images(md: &str) -> String {
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"^\s*(?:!\[[^\]]*\]\([^)]*\)|<!--\s*image\s*-->)\s*$").expect("画像行の正規表現")
    });
    md.split_inclusive('\n')
        .filter(|line| !RE.is_match(line.trim_end_matches(['\n', '\r'])))
        .collect()
}

/// docling が出力した Markdown を読み取る。
///
/// docling は `<out_dir>/<PDFファイル名(拡張子なし)>.md` を出力する。
/// 規約外の名前の場合は out_dir 内の最新 .md を採用する。
fn read_produced_markdown(pdf_path: &Path, out_dir: &Path) -> Result<String, ExtractError> {
    let stem = pdf_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let produced = out_dir.join(format!("{stem}.md"));
    if produced.exists() {
        return Ok(std::fs::read_to_string(&produced)?);
    }
    let mut candidates = std::fs::read_dir(out_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .map(|e| e.path())
        .collect::<Vec<_>>();
    candidates.sort_by_key(|p| p.metadata().and_then(|m| m.modified()).ok());
    let latest = candidates.pop().ok_or_else(|| {
        ExtractError::Docling(format!(
            "docling produced no markdown in {}",
            out_dir.display()
        ))
    })?;
    Ok(std::fs::read_to_string(&latest)?)
}

/// docling が出力した JSON のパスを返す（見つからない場合は None）。
fn find_produced_json(pdf_path: &Path, out_dir: &Path) -> Option<PathBuf> {
    let stem = pdf_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let produced = out_dir.join(format!("{stem}.json"));
    if produced.exists() {
        return Some(produced);
    }
    std::fs::read_dir(out_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|ext| ext == "json"))
}

/// `#/texts/41` 形式の参照から本文テキストを引く。
fn text_by_ref(texts: &[serde_json::Value], reference: &str) -> Option<String> {
    let index: usize = reference.rsplit('/').next()?.parse().ok()?;
    texts
        .get(index)?
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// docling JSON から図一覧を取り出す。
///
/// キャプションが figure/table 形式（`Figure 1` / `Fig. 1` / `図 1` / `Table 1` 等）の
/// picture のみを図として採用する。ロゴ・アイコン・装飾画像を除外するため。
fn parse_figures_from_json(
    json_path: &Path,
    out_dir: &Path,
) -> Result<Vec<DoclingFigure>, ExtractError> {
    let text = std::fs::read_to_string(json_path)?;
    let doc: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        ExtractError::Docling(format!(
            "docling JSON の解析に失敗 ({}): {e}",
            json_path.display()
        ))
    })?;
    let empty = Vec::new();
    let texts = doc
        .get("texts")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    let pictures = doc
        .get("pictures")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);

    let mut figures = Vec::new();
    for pic in pictures {
        let Some(uri) = pic.pointer("/image/uri").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(image_path) = resolve_image_path(uri, out_dir) else {
            continue;
        };
        // キャプション（docling は captions[0] に "Figure 1: ..." を入れる）
        let raw_caption = pic
            .get("captions")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("$ref"))
            .and_then(|v| v.as_str())
            .and_then(|r| text_by_ref(texts, r));
        let page = pic.pointer("/prov/0/page_no").and_then(|v| v.as_i64());

        let Some((label, caption_body)) = raw_caption.as_deref().and_then(split_figure_label)
        else {
            continue;
        };
        figures.push(DoclingFigure {
            image_path,
            label: Some(label),
            caption: caption_body,
            page,
        });
    }
    Ok(figures)
}

/// キャプション先頭のラベルと本文を分離する（例: `Figure 1: Foo.` → `("Figure 1", "Foo.")`）。
///
/// figure / table 形式でない場合は `None` を返す。
#[must_use]
pub fn split_figure_label(caption: &str) -> Option<(String, Option<String>)> {
    let re =
        Regex::new(r"(?i)^\s*(figure|fig\.?|table|図|表)\s*([0-9]{1,3}[a-zA-Z]?)\s*[.:：]?\s*")
            .expect("figure label regex");
    let caps = re.captures(caption)?;
    let word = caps.get(1)?.as_str().to_lowercase();
    let num = caps.get(2)?.as_str().to_string();
    let kind = if word.starts_with("fig") {
        "Figure"
    } else if word.starts_with("tab") {
        "Table"
    } else {
        // 日本語（図 / 表）はそのまま使う
        caps.get(1)?.as_str()
    };
    let label = format!("{kind} {num}");
    let rest = caption[caps.get(0)?.end()..].trim();
    let body = if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    };
    Some((label, body))
}

/// docling が JSON / Markdown に書いた画像参照から実ファイルパスを解決する。
fn resolve_image_path(uri: &str, out_dir: &Path) -> Option<PathBuf> {
    let raw = PathBuf::from(uri);
    if raw.is_file() {
        return Some(raw);
    }
    // 相対参照は out_dir 基準 / カレント基準 / out_dir 直下のファイル名で探す
    let filename = raw.file_name()?;
    let candidates = [
        out_dir.join(&raw),
        out_dir.join(filename),
        std::env::current_dir().ok()?.join(&raw),
    ];
    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    // 最終手段: out_dir 配下を再帰的に探す（_artifacts ディレクトリ配下を想定）
    find_file_recursive(out_dir, filename, 0)
}

/// `dir` 配下を再帰的に探索してファイル名一致のパスを返す（深さ上限あり）。
fn find_file_recursive(dir: &Path, filename: &std::ffi::OsStr, depth: usize) -> Option<PathBuf> {
    if depth > 4 {
        return None;
    }
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name() == Some(filename) {
            return Some(path);
        }
        if path.is_dir()
            && let Some(found) = find_file_recursive(&path, filename, depth + 1)
        {
            return Some(found);
        }
    }
    None
}

/// 抽出キーをファイル名として安全な文字列に変換する。
///
/// `ArXiv:1706.03762` の `:` のように、ファイル名に使えない・問題を起こす
/// 文字は `_` に置換する。
pub fn sanitize_filename_component(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_figure_images_drops_only_image_lines() {
        assert_eq!(
            strip_figure_images("A\n\n![Image](/x/.docling-figures-tmp/a.png)\n\nB\n"),
            "A\n\n\nB\n"
        );
        assert_eq!(strip_figure_images("  <!-- image -->  \nB"), "B");
        // 文中の画像・最終行の改行なし・CRLF
        let inline = "see ![x](y.png) here\n";
        assert_eq!(strip_figure_images(inline), inline);
        assert_eq!(strip_figure_images("A\r\n![i](p)\r\nB"), "A\r\nB");
        assert_eq!(strip_figure_images("plain\n"), "plain\n");
    }

    #[test]
    fn split_figure_label_figure_and_table() {
        assert_eq!(
            split_figure_label("Figure 1: HyperTune workflow. Arrows show."),
            Some((
                "Figure 1".to_string(),
                Some("HyperTune workflow. Arrows show.".to_string())
            ))
        );
        assert_eq!(
            split_figure_label("Fig. 2. Results."),
            Some(("Figure 2".to_string(), Some("Results.".to_string())))
        );
        assert_eq!(
            split_figure_label("Table 3: Dataset"),
            Some(("Table 3".to_string(), Some("Dataset".to_string())))
        );
        // ラベルだけのキャプション
        assert_eq!(
            split_figure_label("Figure 4"),
            Some(("Figure 4".to_string(), None))
        );
        // figure / table 形式でないものは図として扱わない
        assert_eq!(split_figure_label("CC BY 4.0"), None);
        assert_eq!(split_figure_label(""), None);
    }

    #[test]
    fn text_by_ref_resolves_index() {
        let texts = vec![
            serde_json::json!({"text": "zero"}),
            serde_json::json!({"text": "one"}),
        ];
        assert_eq!(text_by_ref(&texts, "#/texts/1"), Some("one".to_string()));
        assert_eq!(text_by_ref(&texts, "#/texts/9"), None);
        assert_eq!(text_by_ref(&texts, "garbage"), None);
    }

    #[test]
    fn sanitize_filename_component_replaces_colon() {
        assert_eq!(
            sanitize_filename_component("ArXiv:1706.03762"),
            "ArXiv_1706.03762"
        );
        assert_eq!(
            sanitize_filename_component("DOI:10.1038/nature12373"),
            "DOI_10.1038_nature12373"
        );
        assert_eq!(
            sanitize_filename_component("ff312b6027d012403760c288fd6531145006cb9f"),
            "ff312b6027d012403760c288fd6531145006cb9f"
        );
    }
}
