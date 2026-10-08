//! `pdf2zh` (PDFMathTranslate) CLI を呼び出して PDF を日本語へ翻訳する。
//!
//! 実行形式: `pdf2zh <pdf> -o <output_dir> -li en -lo ja --service openailiked`
//! 出力は `<stem>-mono.pdf` / `<stem>-dual.pdf` が生成される。本モジュールは
//! `papers/<stem>-ja.pdf` へリネーム（mono を既定）する。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

use super::TranslationError;

/// pdf2zh の終了を待つ時間の上限。
///
/// ponytail: 長い PDF で 60 分で足りなければ設定項目にする。
const PDF2ZH_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// `pdf2zh` 実行バイナリの解決順序:
/// 1. 環境変数 `PDF2ZH_BIN`（明示指定）
/// 2. `$PATH` 上の `pdf2zh`
/// 3. プロジェクトルート / 親ディレクトリの uv venv（`<root>/.venv/bin/pdf2zh`）
fn resolve_pdf2zh_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("PDF2ZH_BIN")
        && !bin.trim().is_empty()
    {
        let p = PathBuf::from(&bin);
        if p.is_file() {
            return p;
        }
        // ディレクトリではなくコマンド名の場合も許可
        return PathBuf::from(bin);
    }
    if let Ok(out) = std::process::Command::new("which").arg("pdf2zh").output()
        && out.status.success()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return PathBuf::from(s);
        }
    }
    let mut roots = Vec::new();
    if let Ok(root) = crate::config::Paths::discover() {
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
            root.join(".venv/bin/pdf2zh"),
            root.parent()
                .map(|p| p.join(".venv/bin/pdf2zh"))
                .unwrap_or_default(),
            // lib2/PDFMathTranslate を直接指すケース
            root.join("../lib2/PDFMathTranslate/.venv/bin/pdf2zh"),
        ] {
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from("pdf2zh")
}

/// 日本語PDFの出力先パス（`papers/<stem>-ja.pdf` 慣例）。
///
/// `pdf_path` が `Some` の場合は同一ディレクトリに `-ja.pdf` を生成、
/// `None` の場合は `"Not available"` を返す（UI表示用）。
#[must_use]
pub fn ja_output_path(pdf_path: Option<&str>) -> String {
    match pdf_path {
        Some(p) if !p.trim().is_empty() => {
            let trimmed = p.trim();
            if trimmed.ends_with(".pdf") {
                format!("{}-ja.pdf", &trimmed[..trimmed.len() - 4])
            } else {
                format!("{trimmed}-ja.pdf")
            }
        }
        _ => "Not available".to_string(),
    }
}

/// API キー（`api_key_env` が空なら vLLM 用のダミー `EMPTY` を返す）。
///
/// vLLM はキーを見ない。名前があるのに値が空のときはエラー（設定の書き忘れに気づかせる）。
fn pdf2zh_api_key(translate: &crate::config::TranslateService) -> Result<String, TranslationError> {
    if translate.api_key_env.trim().is_empty() {
        return Ok("EMPTY".to_string());
    }
    std::env::var(&translate.api_key_env)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            TranslationError::Service(format!(
                "環境変数 {} が未設定のため PDF 翻訳を実行できません",
                translate.api_key_env
            ))
        })
}

/// pdf2zh へ渡す translateGemma のプロンプトテンプレートの置き場。
fn translategemma_prompt_path() -> PathBuf {
    std::env::temp_dir().join("sai-paper-translategemma-prompt.txt")
}

/// pdf2zh へ渡す translateGemma のプロンプトテンプレートの本文。
///
/// pdf2zh はこのファイルを `string.Template` として読み、`${lang_in}` / `${lang_out}` /
/// `${text}` を差し込む（pdf2zh 1.9.x の `pdf2zh.py` / `translator.py`）。
#[must_use]
pub fn translategemma_prompt_template() -> String {
    // `$` はそのまま、`${` はリテラルの `${`。
    "<<<source>>>${lang_in}<<<target>>>${lang_out}<<<text>>>${text}\n".to_string()
}

/// プロンプトテンプレートのファイルを書いて、そのパスを返す。
async fn write_translategemma_prompt() -> Result<PathBuf, TranslationError> {
    let path = translategemma_prompt_path();
    let mut file = tokio::fs::File::create(&path).await.map_err(|e| {
        TranslationError::Service(format!(
            "プロンプトファイルを作成できません {}: {e}",
            path.display()
        ))
    })?;
    file.write_all(translategemma_prompt_template().as_bytes())
        .await
        .map_err(|e| {
            TranslationError::Service(format!(
                "プロンプトファイルを書けません {}: {e}",
                path.display()
            ))
        })?;
    file.flush().await.map_err(|e| {
        TranslationError::Service(format!(
            "プロンプトファイルを閉じられません {}: {e}",
            path.display()
        ))
    })?;
    Ok(path)
}

/// pdf2zh へ渡す引数（テストできる形にしてある）。
///
/// `prompt` が `Some` のときは `--prompt <path>` を付ける（translateGemma の場合）。
fn pdf2zh_args(
    pdf_path: &Path,
    output_dir: &Path,
    lang_in: &str,
    lang_out: &str,
    service: &str,
    prompt: Option<&Path>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        pdf_path.as_os_str().to_os_string(),
        "-o".into(),
        output_dir.as_os_str().to_os_string(),
        "-li".into(),
        lang_in.into(),
        "-lo".into(),
        lang_out.into(),
        "--service".into(),
        service.into(),
    ];
    if let Some(prompt) = prompt {
        args.push("--prompt".into());
        args.push(prompt.as_os_str().to_os_string());
    }
    args
}

/// `pdf2zh` を実行して PDF を翻訳する。
///
/// 翻訳サービスは `translate.pdf.service`（既定: `openailiked`）。OpenAI 互換サービスは
/// `OPENAILIKED_BASE_URL` / `OPENAILIKED_API_KEY` / `OPENAILIKED_MODEL` の 3 つの
/// 環境変数を読むため、設定値をそのまま子プロセスへ渡す。API キーは設定ファイルには
/// 置かず、`translate.pdf.api_key_env` で指定した環境変数から読む。`api_key_env` が空の
/// ときはキーを見ない vLLM 用のダミー値 `EMPTY` を渡す。
///
/// `prompt_format` が `translategemma` のときは、区切り記号つきのプロンプトテンプレートを
/// 一時ファイルに書いて `--prompt` で渡す。
///
/// # 引数
/// - `pdf_path`: 入力 PDF
/// - `translate`: 翻訳サービス設定（`settings.json` の `translate.pdf`）
/// - `lang_in` / `lang_out`: 言語コード（`en` / `ja`）
///
/// # Errors
///
/// API キーの環境変数が未設定のとき、`pdf2zh` の起動・実行に失敗したとき、
/// [`PDF2ZH_TIMEOUT`] を超えて終わらなかったとき、出力 PDF が見つからないときに
/// エラーを返す。
pub async fn run_pdf2zh(
    pdf_path: &Path,
    translate: &crate::config::TranslateService,
    lang_in: &str,
    lang_out: &str,
) -> Result<PathBuf, TranslationError> {
    // API キーは環境変数からのみ読む（設定にもログにも値は出さない）。
    let api_key = pdf2zh_api_key(translate)?;
    let output_ja = PathBuf::from(ja_output_path(Some(&pdf_path.to_string_lossy())));
    let output_dir = output_ja
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    std::fs::create_dir_all(&output_dir)
        .map_err(|e| TranslationError::Service(format!("出力ディレクトリ作成失敗: {e}")))?;

    // translateGemma のときは区切り記号つきのプロンプトを渡す
    let prompt = if translate.prompt_format == "translategemma" {
        Some(write_translategemma_prompt().await?)
    } else {
        None
    };

    // pdf2zh は出力ディレクトリを指定し、内部で <stem>-mono.pdf / -dual.pdf を生成する
    let bin = resolve_pdf2zh_bin();
    let args = pdf2zh_args(
        pdf_path,
        &output_dir,
        lang_in,
        lang_out,
        &translate.service,
        prompt.as_deref(),
    );
    let child = Command::new(&bin)
        .args(&args)
        .env("OPENAILIKED_BASE_URL", &translate.base_url)
        .env("OPENAILIKED_API_KEY", &api_key)
        .env("OPENAILIKED_MODEL", &translate.model)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // TUI を閉じて待っている処理が捨てられても、pdf2zh だけ動き続けないようにする
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            TranslationError::Service(format!(
                "サーバでは pdf2zh を動かしません。各自の PC の sai-prep で前処理して送ってください（pdf2zh の起動に失敗しました: {e}。bin={}）",
                bin.display()
            ))
        })?;

    // 終わらないまま「実行中」で残らないように、時間の上限を設ける
    let output = timeout(PDF2ZH_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| {
            TranslationError::Service(format!(
                "PDF 翻訳が {} 分で終わらなかったので止めました",
                PDF2ZH_TIMEOUT.as_secs() / 60
            ))
        })?
        .map_err(|e| TranslationError::Service(format!("pdf2zh の終了待いに失敗しました: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(TranslationError::Service(format!(
            "pdf2zh が異常終了しました ({}): {}\n{}",
            output.status.code().unwrap_or(-1),
            stderr.chars().take(800).collect::<String>(),
            stdout.chars().take(400).collect::<String>()
        )));
    }

    // pdf2zh の出力規約: <output_dir>/<stem>-mono.pdf と <stem>-dual.pdf
    let stem = pdf_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let mono = output_dir.join(format!("{stem}-mono.pdf"));
    let dual = output_dir.join(format!("{stem}-dual.pdf"));

    let generated = if mono.exists() {
        mono
    } else if dual.exists() {
        dual
    } else {
        return Err(TranslationError::Service(format!(
            "pdf2zh は成功したが出力PDFが見つかりません: {} / {}",
            output_dir.join(format!("{stem}-mono.pdf")).display(),
            output_dir.join(format!("{stem}-dual.pdf")).display()
        )));
    };

    // 既定の ja パスへリネーム（mono を優先）
    if generated != output_ja {
        // 既存の ja があれば上書き
        let _ = std::fs::remove_file(&output_ja);
        std::fs::rename(&generated, &output_ja).map_err(|e| {
            TranslationError::Service(format!(
                "生成PDFのリネームに失敗しました {} -> {}: {e}",
                generated.display(),
                output_ja.display()
            ))
        })?;
        // もう一方（dual）が残っていれば削除しない（ユーザがどちらを使うか選べるよう残す）
    }

    Ok(output_ja)
}

#[cfg(test)]
mod tests {
    use super::{PDF2ZH_TIMEOUT, pdf2zh_api_key, pdf2zh_args, translategemma_prompt_template};
    use crate::config::TranslateService;
    use std::path::Path;

    /// translateGemma のプロンプトファイルの中身は pdf2zh の `string.Template` に合う。
    #[test]
    fn translategemma_prompt_template_has_separators_and_placeholders() {
        assert_eq!(
            translategemma_prompt_template(),
            "<<<source>>>${lang_in}<<<target>>>${lang_out}<<<text>>>${text}\n"
        );
    }

    /// translateGemma のときは引数に `--prompt <path>` が付く（付けない設定もある）。
    #[test]
    fn translategemma_adds_prompt_argument() {
        let args = pdf2zh_args(
            Path::new("/p/a.pdf"),
            Path::new("/p"),
            "en",
            "ja",
            "openailiked",
            Some(Path::new("/tmp/sai-paper-translategemma-prompt.txt")),
        );
        let text: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            text,
            [
                "/p/a.pdf",
                "-o",
                "/p",
                "-li",
                "en",
                "-lo",
                "ja",
                "--service",
                "openailiked",
                "--prompt",
                "/tmp/sai-paper-translategemma-prompt.txt",
            ]
        );
        // -PROMPT の直後がパスであること
        let at = text.iter().position(|a| a == "--prompt").unwrap();
        assert_eq!(text[at + 1], "/tmp/sai-paper-translategemma-prompt.txt");
    }

    /// プロンプトを渡さないときは `--prompt` が付かない。
    #[test]
    fn no_prompt_argument_without_translategemma() {
        let args = pdf2zh_args(
            Path::new("/p/a.pdf"),
            Path::new("/p"),
            "en",
            "ja",
            "openailiked",
            None,
        );
        let text: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(!text.iter().any(|a| a == "--prompt"), "{text:?}");
    }

    /// `api_key_env` が空なら vLLM 用のダミーを返す。名前があるのに値が空ならエラー。
    #[test]
    fn api_key_is_dummy_when_env_name_is_empty() {
        let service = TranslateService {
            api_key_env: String::new(),
            ..TranslateService::default()
        };
        assert_eq!(pdf2zh_api_key(&service).unwrap(), "EMPTY");

        let named = TranslateService {
            api_key_env: "SAI_PAPER_TEST_KEY_MISSING".into(),
            ..TranslateService::default()
        };
        let err = pdf2zh_api_key(&named).unwrap_err();
        assert!(err.to_string().contains("SAI_PAPER_TEST_KEY_MISSING"), "{err}");
    }

    /// 待つ時間の上限が設定されている（0 だと永久に待つことになる）。
    #[test]
    fn timeout_is_sixty_minutes() {
        assert_eq!(PDF2ZH_TIMEOUT.as_secs(), 3600);
    }
}
