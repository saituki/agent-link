//! Cross-platform filesystem path normalization.

use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

/// Resolve a path through the filesystem while preserving the native path form
/// expected by applications and users on every supported platform.
///
/// Windows' standard canonicalization API returns extended-length (`\\?\\`)
/// paths. They identify the same filesystem object but are unsuitable as the
/// application-wide representation because they leak into persisted metadata
/// and normal UI rendering. `dunce` removes that transport prefix whenever a
/// regular Windows path can represent the same location.
///
/// # Errors
///
/// Returns an error when the filesystem cannot resolve the supplied path.
pub fn canonicalize_path(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    dunce::canonicalize(path)
}

/// `<dir>` が研究プロジェクトのルートかどうかを判定する。
///
/// 判定条件（この順で見る）:
/// 1. `<dir>/.sai/paper/` が存在する（研究プロジェクトのデータ置き場）
/// 2. `<dir>/Cargo.toml` が "sai-paper" を含む（開発リポジトリの検出用）
///
/// 旧レイアウト（`<dir>/data/papr.db` + `<dir>/crates/`）はプロジェクトと
/// みなさない（段階 2d で判定を削除。開発リポジトリ自身も `.sai/paper/` だけを使う）。
/// `.sai/` 単独でもプロジェクトとみなさない。`~/.sai-moon/` があるだけで
/// ホームがプロジェクト扱いされ、ホームに DB が作られる問題を避けるため。
/// 環境に依存しないよう、判定はこの関数に切り出してある。
#[must_use]
pub fn is_project_root(dir: &Path) -> bool {
    if dir.join(".sai").join("paper").exists() {
        return true;
    }
    if dir.join("Cargo.toml").exists()
        && fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|txt| txt.contains("sai-paper"))
    {
        return true;
    }
    false
}

/// sai-paper ワークスペースのルートを実行時に解決する。
///
/// 優先順位:
/// 1. `SAI_PAPER_ROOT` 環境変数（存在するパスのみ採用）
/// 2. `cwd` / 実行ファイル位置から上向き探索（[`is_project_root`] と
///    `<dir>/sai-paper/Cargo.toml` の存在で判定）
/// 3. ビルド時の `CARGO_MANIFEST_DIR` から導いたワークスペースルート
///    （in-tree ビルド時の最終フォールバック）
#[must_use]
pub fn project_root() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("SAI_PAPER_ROOT") {
        let p = PathBuf::from(root);
        if p.exists() {
            return Some(p);
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(par) = exe.parent()
    {
        candidates.push(par.to_path_buf());
    }
    for base in candidates {
        let mut p = base;
        for _ in 0..7 {
            // 研究プロジェクト（`.sai/paper/` がある等）を検出する。
            // （sai-paper を開発リポジトリ外の研究プロジェクトで実行する場合）
            if is_project_root(&p) {
                return Some(p);
            }
            if p.join("sai-paper").join("Cargo.toml").exists() {
                return Some(p.join("sai-paper"));
            }
            if let Some(par) = p.parent() {
                p = par.to_path_buf();
            } else {
                break;
            }
        }
    }
    let def = compile_time_workspace_root();
    if def.join("Cargo.toml").exists() {
        return Some(def);
    }
    None
}

/// 研究プロジェクトのデータルート解決結果。
///
/// `sai-paper` は研究プロジェクトごとに 1 つの `papr.db` を持ち、
/// データは `<project>/.sai/paper/` に置く（旧レイアウトの `data/` は使わない）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperRoots {
    /// 研究プロジェクトルート。
    pub root: PathBuf,
    /// `papr.db` / `settings.json` / `papers/` を格納するデータディレクトリ。
    pub data_dir: PathBuf,
}

/// 研究プロジェクトのデータルートを解決する。
///
/// データは `<root>/.sai/paper/` だけを使う（`sai-paper init` が作成する）。
/// 見つからない場合も同じ場所を返す（作成は書き込む側の仕事）。
#[must_use]
pub fn paper_roots() -> Option<PaperRoots> {
    let root = project_root()?;
    let dot_sai_paper = root.join(".sai").join("paper");
    Some(PaperRoots {
        root,
        data_dir: dot_sai_paper,
    })
}

/// ビルド時のクレート位置（`<root>/crates/<crate>`）からワークスペースルートを導く。
///
/// 絶対パスのハードコードを避けるための最終フォールバック。
/// out-of-tree にバイナリを移動した場合は機能しない（`project_root()` が優先される）。
#[must_use]
pub fn compile_time_workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// プロジェクトルートの `.env` を読み込む（既存の環境変数は上書きしない）。
///
/// そのあと全プロジェクト共通の API キー置き場 `~/.sai-moon/paper.env` も読む。
/// `dotenvy` は既に設定済みの値を上書きしないため、プロジェクトの `.env` が
/// `~/.sai-moon/paper.env` より優先される（dotenvy 済みの環境変数も両方に対して優先）。
///
/// ルート解決に失敗した場合は `dotenvy::dotenv()` による cwd からの通常探索に
/// フォールバックする。読み込み失敗は呼び出し側に影響しない。
pub fn load_project_dotenv() {
    if let Some(root) = project_root() {
        let _ = dotenvy::from_path(root.join(".env"));
    }
    let _ = dotenvy::dotenv();
    // 全プロジェクト共通のキー置き場（プロジェクトの .env が優先。上は読まない）。
    if let Some(base) = sai_home_path("paper.env") {
        load_keychain_env(&base);
        let _ = dotenvy::from_path(base);
    }
}

/// `paper.env` の中の、キーチェーンから読む行を解決して環境変数にする。
///
/// `.zshrc` と同じ書き方の `export NAME=$(security find-generic-password -s "<項目>" -a "$USER" -w 2>/dev/null)`
/// は、シェルでない `dotenvy` では実行されず、キーが入らない（S2 がキー無しの扱いになり 429 が続く）。
/// そこでこの形の行だけを見分け、シェルを通さずに `security` を直接呼んでキーを取る。
///
/// 上書きするのは、その変数が未設定・空・`$(` で始まる壊れた値のときだけ
/// （プロジェクトの `.env` や、シェルで正しく入った値を優先する）。取れなければ何もしない。
fn load_keychain_env(path: &Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let mut resolved = String::new();
    for line in keychain_lines(&text) {
        let current = std::env::var(&line.name).unwrap_or_default();
        let current = current.trim();
        if !current.is_empty() && !current.starts_with("$(") {
            continue;
        }
        if let Some(value) = keychain_value(&line.service, line.account.as_deref()) {
            // 値はキーだけ（英数字と記号）。改行や引用符を含むものは使わない。
            if !value.contains(['\n', '\r', '"', '\'']) {
                resolved.push_str(&format!("{}=\"{}\"\n", line.name, value));
            }
        }
    }
    if !resolved.is_empty() {
        let _ = dotenvy::from_read_override(resolved.as_bytes());
    }
}

/// キーチェーンから読む 1 行（変数名・キーチェーンの項目名・アカウント）。
#[derive(Debug, PartialEq, Eq)]
struct KeychainLine {
    name: String,
    service: String,
    account: Option<String>,
}

/// `[export ]NAME=$(security find-generic-password -s "<項目>" [-a "<アカウント>"] -w [2>/dev/null])` の行を取り出す。
///
/// アカウントの `$USER` は環境変数 `USER` に置き換える。これ以外の形の行は無視する。
fn keychain_lines(text: &str) -> Vec<KeychainLine> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let Some(inner) = value
            .trim()
            .strip_prefix("$(")
            .and_then(|rest| rest.strip_suffix(')'))
        else {
            continue;
        };
        let words = shell_words(inner);
        if words.first().map(String::as_str) != Some("security")
            || words.get(1).map(String::as_str) != Some("find-generic-password")
            || !words.iter().any(|w| w == "-w")
        {
            continue;
        }
        let arg = |flag: &str| {
            words
                .iter()
                .position(|w| w == flag)
                .and_then(|i| words.get(i + 1))
                .cloned()
        };
        let Some(service) = arg("-s") else {
            continue;
        };
        let account = arg("-a").map(|a| {
            if a == "$USER" || a == "${USER}" {
                std::env::var("USER").unwrap_or_default()
            } else {
                a
            }
        });
        out.push(KeychainLine {
            name: name.to_owned(),
            service,
            account,
        });
    }
    out
}

/// 空白で区切り、`"…"` と `'…'` の引用符を外す（`2>/dev/null` などの向け先は捨てる）。
fn shell_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in text.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(current);
    }
    words.retain(|w| !w.contains('>'));
    words
}

/// macOS のキーチェーンから値を読む（シェルを通さずに `/usr/bin/security` を直接呼ぶ）。
fn keychain_value(service: &str, account: Option<&str>) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let mut command = std::process::Command::new("/usr/bin/security");
    command.args(["find-generic-password", "-s", service]);
    if let Some(account) = account.filter(|a| !a.is_empty()) {
        command.args(["-a", account]);
    }
    command.arg("-w");
    let output = command.stderr(std::process::Stdio::null()).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// 利用者全体の置き場（sai のグローバルデータ置き場）を返す。
///
/// 解決順:
/// 1. 環境変数 `SAI_SAI_HOME`（テスト・隔離環境・複数-home で上書きする）
/// 2. `~/.sai-moon`
///
/// `HOME` も設定されていない場合は `None`（呼び出し側は読み書きを飛ばす）。
/// デフォルトは旧 `~/.sai` ではなく `~/.sai-moon`。
///
/// プロジェクトごとの `<root>/.sai/`（`papr.db`・blocks・settings 等）は
/// `paper_roots()` / `project_root()` を使う。ここでは扱わない。
///
/// `sai-paper-core` は `sai_link_engine` に依存しないため、同じ規則の小さな
/// 関数を这边に置いてある。値は必ず `sai_link_engine::project::sai_home` と一致させること。
#[must_use]
pub fn sai_home() -> Option<PathBuf> {
    resolve_sai_home(
        std::env::var_os("SAI_SAI_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// 利用者全体の置き場配下の相対パスを返す（[`sai_home`] が無ければ `None`）。
///
/// 例: `sai_home_path("blocks")` → `~/.sai-moon/blocks`
#[must_use]
pub fn sai_home_path(rel: &str) -> Option<PathBuf> {
    sai_home().map(|h| h.join(rel))
}

/// [`sai_home`] の本体（環境変数の値を受け取る版）。
///
/// 環境変数を書き換えないのでテストが並列実行しても壊れない（この crate は
/// `unsafe_code = "forbid"` で `std::env::set_var` の `unsafe` ブロックも書けない）。
fn resolve_sai_home(sai_home_env: Option<&OsStr>, home_env: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(p) = sai_home_env.filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    home_env
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".sai-moon"))
}

/// 存在しないパスでも使える canonicalize。
///
/// `dunce::canonicalize` は最終成分が無いと失敗する。DB に書く時点ではまだ
/// 存在しないファイル（DL 先など）でも比較できるよう、存在する祖先まで
/// canonicalize して残りの成分をつなげて返す。
fn canonicalize_lenient(path: &Path) -> PathBuf {
    if let Ok(canonical) = canonicalize_path(path) {
        return canonical;
    }
    let mut remaining: Vec<std::ffi::OsString> = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        if let Ok(canonical) = canonicalize_path(&current) {
            let mut resolved = canonical;
            for component in remaining.iter().rev() {
                resolved.push(component);
            }
            return resolved;
        }
        let Some(name) = current.file_name() else {
            break;
        };
        remaining.push(name.to_os_string());
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }
    // どれも解決できなかった（実在しないルート等）は入力そのまま。
    path.to_path_buf()
}

/// 相対パスを区切り `/` の文字列にする（OS 依存の区切りを DB では常に `/` に揃える）。
fn stored_relative_string(relative: &Path) -> String {
    let mut stored = String::new();
    for component in relative.components() {
        if !stored.is_empty() {
            stored.push('/');
        }
        stored.push_str(&component.as_os_str().to_string_lossy());
    }
    stored
}

/// DB に保存するパスへ変換する（段階 2b「DB には相対パスで書く」）。
///
/// - `data_dir`（`.sai/paper/`）の中のファイル → `data_dir` からの相対パス
///   （例 `papers/0030_Foo/paper.pdf`、区切りは常に `/`）
/// - 外のファイル（`<root>/papers/` や `<root>/data/extracts/` 等）→ 絶対パスのまま
///
/// 比較は 2 段階。まず生の接頭辞で比べ、合わなければ**両方を canonicalize してから**
/// 比べる。これにより macOS の `/tmp` ↔ `/private/tmp` のような符号リンク差でも
/// 同じファイルを同じように扱える。存在しないファイルでも、存在する祖先まで
/// canonicalize して残りをつなげる（[`canonicalize_lenient`]）ため失敗しない。
///
/// data_dir の外で、入力が絶対パスならそのまま返す（既存の絶対パス行を
/// 書き換えないため。書き込み側・読み込み側は必ずこの関数を通すこと）。
#[must_use]
pub fn to_stored_path(data_dir: &Path, path: impl AsRef<Path>) -> String {
    let path = path.as_ref();

    // 1) 生の接頭辞で比較（data_dir 直下が symlink でも相対化できる）。
    if let Ok(relative) = path.strip_prefix(data_dir)
        && !relative.as_os_str().is_empty()
    {
        return stored_relative_string(relative);
    }

    // 2) canonicalize してから比較する（符号リンク差を吸収）。
    let canonical_path = canonicalize_lenient(path);
    let canonical_data_dir = canonicalize_lenient(data_dir);
    if let Ok(relative) = canonical_path.strip_prefix(&canonical_data_dir)
        && !relative.as_os_str().is_empty()
    {
        return stored_relative_string(relative);
    }

    // 3) data_dir の外。絶対パスなら入力をそのまま返す。
    if path.is_absolute() {
        return path.to_string_lossy().into_owned();
    }
    canonical_path.to_string_lossy().into_owned()
}

/// DB から読んだパスを実ファイルパスへ直す（段階 2b）。
///
/// 絶対パス（data_dir の外のファイル・古い絶対パスのままの行）はそのまま、
/// 相対パスは `data_dir.join(stored)` で復元する。読む側はすべてこの関数を通す。
#[must_use]
pub fn resolve_stored_path(data_dir: &Path, stored: &str) -> PathBuf {
    let path = PathBuf::from(stored);
    if path.is_absolute() {
        path
    } else {
        data_dir.join(path)
    }
}

/// `path` が `roots` のどれかの中にいる実在ファイルかどうかを判定する。
///
/// DB から読んだ相対パスは `data_dir` 連結で復元するため、パスと `roots` が
/// 符号リンク差（`/tmp` ↔ `/private/tmp` や data_dir 直下の symlink）で
/// 別の見え方をしていることがある。生の接頭辞一致を優先し、合わなければ
/// **両方を canonicalize してから**比較する（段階 2b）。
#[must_use]
pub fn is_file_in_roots(path: &Path, roots: &[PathBuf]) -> bool {
    if !path.is_file() {
        return false;
    }
    if roots.iter().any(|root| path.starts_with(root)) {
        return true;
    }
    let canonical = canonicalize_lenient(path);
    roots
        .iter()
        .any(|root| canonical.starts_with(canonicalize_lenient(root)))
}

/// Sanitizes a string for use as a download filename component.
#[must_use]
pub fn sanitize_download_filename_component(title: &str) -> String {
    let sanitized: String = title
        .chars()
        .map(|c| match c {
            '/' => '_',
            '\n' | '\r' | '\t' => ' ',
            #[cfg(windows)]
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            #[cfg(windows)]
            c if c.is_control() => ' ',
            #[cfg(not(windows))]
            c if c.is_control() => c,
            c => c,
        })
        .collect();
    let sanitized = sanitized.trim();
    #[cfg(windows)]
    {
        sanitized.trim_end_matches(['.', ' ']).to_owned()
    }
    #[cfg(not(windows))]
    {
        sanitized.to_owned()
    }
}

/// Move a PDF file from source to destination.
///
/// # Errors
///
/// Returns an error when neither an atomic rename nor the copy-and-remove
/// fallback can complete safely.
pub fn move_pdf_file(source: &Path, destination: &Path) -> io::Result<()> {
    if let Err(_rename_error) = std::fs::rename(source, destination) {
        std::fs::copy(source, destination)?;
        if let Err(remove_error) = std::fs::remove_file(source) {
            let _ = std::fs::remove_file(destination);
            return Err(remove_error);
        }
    }
    Ok(())
}

/// Error when an invalid collection name is provided.
#[derive(Debug, thiserror::Error)]
#[error("group name must be one safe directory name")]
pub struct InvalidCollectionName;

/// Validates that a collection name is a safe directory name.
///
/// # Errors
///
/// Returns [`InvalidCollectionName`] when the name is empty, reserved, or
/// contains a path separator.
pub fn validate_collection_name(name: &str) -> Result<(), InvalidCollectionName> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        Err(InvalidCollectionName)
    } else {
        Ok(())
    }
}

/// 論文フォルダの固定ファイル名（第 2 段で再利用可）。
///
/// 論文 1 件分のフォルダ内で使用される標準的なファイル名。
pub mod paper_file_names {
    /// オリジナル PDF
    pub const PDF: &str = "paper.pdf";
    /// 日本語訳 PDF
    pub const PDF_JA: &str = "paper-ja.pdf";
    /// 対訳 PDF
    pub const PDF_DUAL: &str = "paper-dual.pdf";
    /// 抽出 Markdown
    pub const EXTRACT: &str = "extract.md";
    /// メモ
    pub const NOTE: &str = "note.md";
    /// Radio の音声（sai-radio が書く。圧縮しなかったときは `radio.mp3`）
    pub const RADIO: &str = "radio.m4a";
    /// 図画像を格納するサブディレクトリ名
    pub const FIGURES_DIR: &str = "figures";
}

/// 論文 1 件分のフォルダ（`<data_dir>/papers/{id:04}_短いタイトル/`）を返す。
///
/// 論文の置き場は常に `<data_dir>/papers/`（`data_dir` は `.sai/paper/`）。
/// 決め方は次の 1 つだけ（段階 2a）。**フォルダは作らない**（作るのは書き込む側）。
///
/// 1. まず `<data_dir>/papers/` の中から、名前が `{id:04}_` で始まるか
///    `{id:04}` と完全に一致する**フォルダ**を探して、あればそれを返す。
///    タイトルが変わっても既存の「番号＿タイトル全文」のフォルダを見つけられる。
///    候補が複数あるときは名前の辞書順で最初のものを返す（並びを固定するため）。
/// 2. 無ければ「番号＋短いタイトル」の名前を返す（短いタイトルの規則は
///    [`short_title_slug`] を参照）。
#[must_use]
pub fn paper_folder(data_dir: &Path, paper_id: i64, title: &str) -> PathBuf {
    let papers_root = data_dir.join("papers");
    let id_prefix = format!("{:04}_", paper_id);
    let id_exact = format!("{:04}", paper_id);

    // 1. 既存フォルダの探索（`0030_...` / `0030`）。`00301_...` は拾わない。
    let mut candidates: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(entries) = fs::read_dir(&papers_root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if (name.starts_with(&id_prefix) || name == id_exact) && entry.path().is_dir() {
                candidates.push((name, entry.path()));
            }
        }
    }
    if !candidates.is_empty() {
        candidates.sort_by(|a, b| a.0.cmp(&b.0));
        return candidates[0].1.clone();
    }

    // 2. 新しい名前（フォルダは作らない）。
    let slug = short_title_slug(title);
    if slug.is_empty() {
        // 英数字が 1 つも残らないときは番号だけ。
        papers_root.join(id_exact)
    } else {
        papers_root.join(format!("{id_prefix}{slug}"))
    }
}

/// ノート（論文フォルダの `note.md`）の数。Dashboard の NOTE 件数（TUI・phone 共通）。
#[must_use]
pub fn note_count(data_dir: &Path) -> usize {
    fs::read_dir(data_dir.join("papers"))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().join(paper_file_names::NOTE).is_file())
                .count()
        })
        .unwrap_or(0)
}

/// 論文フォルダ名のタイトル部（短いタイトル）を作る。
///
/// 規則（段階 2a の決めごと）:
///
/// - ASCII の英数字だけを残し、それ以外はハイフンにする
/// - 連続するハイフンは 1 つにまとめ、先頭と末尾のハイフンを消す
/// - 最大 40 文字（文字単位）で切り、切った後の末尾のハイフンも消す
/// - 英数字が 1 つも残らない場合は空文字（呼び出し側で番号だけにする）
///
/// `sai-paper relayout`（段階 2d）がフォルダ名の変更予定を計算するときも
/// これを使う（新しい名前だけを求めるため）。
#[must_use]
pub fn short_title_slug(title: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            // 連続する区切りは 1 つにまとめる（先頭にハイフンを付けない）。
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(c);
        } else {
            pending_dash = true;
        }
    }
    // 最大 40 文字（必ず文字単位で切る）。
    if slug.chars().count() > 40 {
        slug = slug.chars().take(40).collect();
        // 切った結果末尾がハイフンになったら消す。
        while slug.ends_with('-') {
            slug.pop();
        }
    }
    slug
}

/// 論文フォルダ内のファイルパスを返す。
///
/// `file_type` に応じて `paper_file_names` の定数を使用する。
/// - "pdf" → paper.pdf
/// - "pdf-ja" → paper-ja.pdf
/// - "pdf-dual" → paper-dual.pdf
/// - "extract" → extract.md
/// - "note" → note.md
#[must_use]
pub fn paper_file_path(paper_dir: &Path, file_type: &str) -> PathBuf {
    let filename = match file_type {
        "pdf" => paper_file_names::PDF,
        "pdf-ja" => paper_file_names::PDF_JA,
        "pdf-dual" => paper_file_names::PDF_DUAL,
        "extract" => paper_file_names::EXTRACT,
        "note" => paper_file_names::NOTE,
        _ => "",
    };
    paper_dir.join(filename)
}

/// 論文フォルダにある Radio の音声（`radio.m4a`、無ければ `radio.mp3`）。
#[must_use]
pub fn radio_audio_path(paper_dir: &Path) -> Option<PathBuf> {
    [paper_file_names::RADIO, "radio.mp3"]
        .into_iter()
        .map(|name| paper_dir.join(name))
        .find(|path| path.is_file())
}

/// 日本語訳 PDF のパス（`<stem>-dual.pdf` 優先、無ければ `<stem>-ja.pdf`）。
///
/// 元 PDF のパス（`pdf_path`）と同一フォルダの対訳版を探す。
/// どちらも実在しなければ `None`。
#[must_use]
pub fn translated_pdf_path(pdf_path: &str) -> Option<PathBuf> {
    let p = PathBuf::from(pdf_path);
    let dir = p.parent()?;
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string())?;
    let dual = dir.join(format!("{stem}-dual.pdf"));
    let ja = dir.join(format!("{stem}-ja.pdf"));
    if dual.is_file() {
        Some(dual)
    } else if ja.is_file() {
        Some(ja)
    } else {
        None
    }
}

/// 論文フォルダ配下の図画像ディレクトリ（`<paper_dir>/figures/`）を返す。
#[must_use]
pub fn paper_figures_dir(paper_dir: &Path) -> PathBuf {
    paper_dir.join(paper_file_names::FIGURES_DIR)
}

/// 図画像の保存先パス（`<paper_dir>/figures/fig-NNN.png`）を返す。
///
/// `fig_no` は 1 起算。形式は `fig-001.png` のように 3 桁ゼロ埋めする。
#[must_use]
pub fn paper_figure_path(paper_dir: &Path, fig_no: i64) -> PathBuf {
    paper_figures_dir(paper_dir).join(format!("fig-{fig_no:03}.png"))
}

/// ユーザーの考え（`<data_dir>/mind.md`）を返す。
///
/// ユーザーが自分の考えを書く場所。エージェントは直接書き換えない。
#[must_use]
pub fn mind_path(data_dir: &Path) -> PathBuf {
    data_dir.join("mind.md")
}

/// プロジェクトの現況（`<data_dir>/state.md`）を返す。
///
/// 目標・段階・今の作業・次の一手・更新日をユーザーが書く場所。
#[must_use]
pub fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join("state.md")
}

/// Gets the page count of a PDF using `pdfinfo`.
/// Falls back to returning 1 if it fails.
#[must_use]
pub fn get_pdf_page_count(path: &Path) -> usize {
    if let Ok(output) = std::process::Command::new("pdfinfo").arg(path).output()
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if line.starts_with("Pages:")
                && let Some(pages_str) = line.split_whitespace().nth(1)
                && let Ok(pages) = pages_str.parse::<usize>()
            {
                return pages;
            }
        }
    }
    1
}

#[cfg(test)]
mod keychain_tests {
    use super::{KeychainLine, keychain_lines, shell_words};

    /// `.zshrc` と同じ書き方の行から、変数名・項目名・アカウントを取り出す。
    #[test]
    fn keychain_lines_reads_security_command() {
        let text = concat!(
            "export S2_API_KEY=$(security find-generic-password -s \"SEMANTICSCHOLAR_API_KEY\" -a \"tester\" -w 2>/dev/null)\n",
            "OPENALEX_API_KEY=$(security find-generic-password -s 'OPENALEX_API_KEY' -w)\n",
            "PLAIN_KEY=abc123\n",
            "# コメント\n",
            "BAD=$(rm -rf /tmp/x)\n",
        );
        assert_eq!(
            keychain_lines(text),
            vec![
                KeychainLine {
                    name: "S2_API_KEY".to_owned(),
                    service: "SEMANTICSCHOLAR_API_KEY".to_owned(),
                    account: Some("tester".to_owned()),
                },
                KeychainLine {
                    name: "OPENALEX_API_KEY".to_owned(),
                    service: "OPENALEX_API_KEY".to_owned(),
                    account: None,
                },
            ]
        );
    }

    /// 引用符を外し、向け先（`2>/dev/null`）は捨てる。
    #[test]
    fn shell_words_strips_quotes_and_redirects() {
        assert_eq!(
            shell_words(r#"security find-generic-password -s "A B" -a 'me' -w 2>/dev/null"#),
            vec!["security", "find-generic-password", "-s", "A B", "-a", "me", "-w"]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SAI_SAI_HOME` があればそれ、無ければ `~/.sai-moon` を返す。
    #[test]
    fn sai_home_prefers_env_then_sai_moon_dir() {
        // ① SAI_SAI_HOME があればそれが優先される
        assert_eq!(
            resolve_sai_home(Some(OsStr::new("/tmp/sai-home-test")), Some(OsStr::new("/home/u"))),
            Some(PathBuf::from("/tmp/sai-home-test")),
            "SAI_SAI_HOME があればそれを使う"
        );
        // ② 無ければ ~/.sai-moon（~/.sai ではない）
        assert_eq!(
            resolve_sai_home(None, Some(OsStr::new("/home/u"))),
            Some(PathBuf::from("/home/u/.sai-moon"))
        );
        // ③ 空文字の SAI_SAI_HOME は無視して ~/.sai-moon に落ちる
        assert_eq!(
            resolve_sai_home(Some(OsStr::new("")), Some(OsStr::new("/home/u"))),
            Some(PathBuf::from("/home/u/.sai-moon"))
        );
        // ④ HOME も無ければ None
        assert_eq!(resolve_sai_home(None, None), None);
    }

    #[test]
    #[cfg(not(windows))]
    fn download_filename_preserves_colon_on_supported_platforms() {
        let sanitized = sanitize_download_filename_component("AntiGlitch: Better / Faster");
        assert_eq!(sanitized, "AntiGlitch: Better _ Faster");
    }

    #[test]
    #[cfg(windows)]
    fn download_filename_sanitizes_colon_on_windows() {
        let sanitized = sanitize_download_filename_component("AntiGlitch: Better / Faster");
        assert_eq!(sanitized, "AntiGlitch_ Better _ Faster");
    }

    /// 論文フォルダ探索・命名テスト用の一時 `data_dir`（`papers/` は作らない）。
    fn temp_data_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-folder-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// フォルダ名の末尾（`papers/` の下の 1 段）を取り出す。
    fn folder_name(path: &Path) -> String {
        path.file_name().unwrap().to_string_lossy().into_owned()
    }

    /// 短いタイトルの規則（英語）: 記号はハイフンになり、40 文字で切られる。
    #[test]
    fn paper_folder_short_title_for_english_title() {
        let data = temp_data_dir("en");
        let folder = paper_folder(&data, 30, "Model Context Protocol (MCP) and Agent-to-Agent");
        assert_eq!(
            folder_name(&folder),
            "0030_Model-Context-Protocol-MCP-and-Agent-to"
        );
        assert!(
            folder_name(&folder)
                .rsplit_once('_')
                .is_some_and(|(_, t)| t.chars().count() <= 40)
        );
        let _ = fs::remove_dir_all(&data);
    }

    /// 短いタイトルの規則（記号だらけ）: 連続する記号は 1 つのハイフンにまとまる。
    #[test]
    fn paper_folder_short_title_for_symbol_heavy_title() {
        let data = temp_data_dir("sym");
        // 記号だけ → 英数字が残らないので番号だけ。
        assert_eq!(folder_name(&paper_folder(&data, 7, "!!! ??? ...")), "0007");
        // 記号で挟まれた単語はハイフンでつながる。
        assert_eq!(
            folder_name(&paper_folder(&data, 8, "A!!!B  /  C")),
            "0008_A-B-C"
        );
        let _ = fs::remove_dir_all(&data);
    }

    /// 短いタイトルの規則（日本語のみ）: 英数字が残らないので番号だけ。
    #[test]
    fn paper_folder_short_title_for_japanese_only_title() {
        let data = temp_data_dir("ja");
        assert_eq!(
            folder_name(&paper_folder(&data, 30, "日本語のタイトル")),
            "0030"
        );
        // 100 文字の日本語タイトルでも落ちない（文字単位の処理のみ）。
        let long_ja = "日本語のタイトルです".repeat(10);
        assert_eq!(long_ja.chars().count(), 100);
        assert_eq!(folder_name(&paper_folder(&data, 7, &long_ja)), "0007");
        let _ = fs::remove_dir_all(&data);
    }

    /// 短いタイトルの規則（40 文字超）: 文字単位で切り、末尾のハイフンも消える。
    #[test]
    fn paper_folder_short_title_truncates_at_40_chars() {
        let data = temp_data_dir("long");
        // 英数字 60 文字 → 40 文字で切られる。
        let digits = "abcdefghij".repeat(6);
        let name = folder_name(&paper_folder(&data, 1, &digits));
        assert_eq!(name, format!("0001_{}", "abcdefghij".repeat(4)));
        assert_eq!(name.rsplit_once('_').unwrap().1.chars().count(), 40);

        // 切った位置がハイフンになる場合も、末尾のハイフンは消える。
        let spaced = "word ".repeat(20);
        let slug = folder_name(&paper_folder(&data, 2, &spaced))
            .split_once('_')
            .unwrap()
            .1
            .to_string();
        assert_eq!(
            slug.chars().count(),
            39,
            "末尾のハイフンを消した長さ: {slug}"
        );
        assert!(!slug.ends_with('-'));
        assert!(slug.starts_with("word-word"));
        let _ = fs::remove_dir_all(&data);
    }

    /// 既存の「番号＿タイトル全文」フォルダは、DB のタイトルが変わっていても返る。
    #[test]
    fn paper_folder_finds_existing_folder_even_if_title_changed() {
        let data = temp_data_dir("existing");
        let papers = data.join("papers");
        let existing = papers.join("0030_Model Context Protocol (MCP) and Agent-to-Agent");
        fs::create_dir_all(&existing).unwrap();

        let found = paper_folder(&data, 30, "まったく別の新しいタイトル");
        assert_eq!(found, existing);
        let _ = fs::remove_dir_all(&data);
    }

    /// `0030` だけのフォルダも見つかる。
    #[test]
    fn paper_folder_finds_bare_id_folder() {
        let data = temp_data_dir("bare");
        let bare = data.join("papers").join("0030");
        fs::create_dir_all(&bare).unwrap();
        assert_eq!(paper_folder(&data, 30, "New Title"), bare);
        let _ = fs::remove_dir_all(&data);
    }

    /// 別の番号（`00301_...`）を誤って拾わない。
    #[test]
    fn paper_folder_ignores_other_paper_numbers() {
        let data = temp_data_dir("other");
        let papers = data.join("papers");
        fs::create_dir_all(papers.join("00301_Somebody Else")).unwrap();
        fs::create_dir_all(papers.join("00302_Another One")).unwrap();
        // 30 番は `0030_` / `0030` が無いので新名前になり、既存の 2 つは拾わない。
        let fresh = paper_folder(&data, 30, "My Title");
        assert_eq!(folder_name(&fresh), "0030_My-Title");
        assert!(!fresh.exists(), "フォルダは作られないこと");
        // 301 番の接頭辞は `0301_` なので、`00301_...` とも一致しない。
        let fresh_301 = paper_folder(&data, 301, "Whatever");
        assert_eq!(folder_name(&fresh_301), "0301_Whatever");
        let _ = fs::remove_dir_all(&data);
    }

    /// 候補が複数あるときは名前の辞書順で最初のものを返す（並びを固定する）。
    #[test]
    fn paper_folder_returns_lexicographically_first_candidate() {
        let data = temp_data_dir("multi");
        let papers = data.join("papers");
        fs::create_dir_all(papers.join("0030_bbb")).unwrap();
        fs::create_dir_all(papers.join("0030_aaa")).unwrap();
        fs::create_dir_all(papers.join("0030")).unwrap();
        assert_eq!(
            paper_folder(&data, 30, "Title"),
            papers.join("0030"),
            "`0030` が辞書順で最前"
        );
        let _ = fs::remove_dir_all(&data);
    }

    /// 見つからなければ新しい名前が返り、フォルダは作られない。
    #[test]
    fn paper_folder_returns_new_name_without_creating_it() {
        let data = temp_data_dir("fresh");
        let folder = paper_folder(&data, 5, "Machine Learning Basics");
        assert_eq!(
            folder,
            data.join("papers").join("0005_Machine-Learning-Basics")
        );
        assert!(!folder.exists(), "paper_folder はフォルダを作らないこと");
        assert!(!data.join("papers").exists());
        let _ = fs::remove_dir_all(&data);
    }

    /// 一時ディレクトリで、`.sai/` だけではプロジェクトとみなさず、
    /// `.sai/paper/` があればプロジェクトとみなすこと（段階 1）。
    #[test]
    fn project_root_requires_dot_sai_paper() -> Result<(), Box<dyn std::error::Error>> {
        let temp = std::env::temp_dir().join(format!(
            "papr-project-root-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        // `.sai/` だけ: プロジェクトとみなさない（ホーム誤認の防止）。
        let dot_sai_only = temp.join("dot-sai-only");
        fs::create_dir_all(dot_sai_only.join(".sai"))?;
        assert!(!is_project_root(&dot_sai_only));

        // `.sai/paper/` があればプロジェクトとみなす。
        let dot_sai_paper = temp.join("dot-sai-paper");
        fs::create_dir_all(dot_sai_paper.join(".sai").join("paper"))?;
        assert!(is_project_root(&dot_sai_paper));

        // 旧レイアウト（`data/papr.db` + `crates/`）はプロジェクトとみなさない
        // （段階 2d で旧レイアウトの判定を削除した）。
        let legacy = temp.join("legacy");
        fs::create_dir_all(legacy.join("data"))?;
        fs::create_dir_all(legacy.join("crates"))?;
        fs::write(legacy.join("data").join("papr.db"), b"")?;
        assert!(!is_project_root(&legacy));

        // Cargo.toml 判定もこの段階では残す。
        let dev_repo = temp.join("dev-repo");
        fs::create_dir_all(&dev_repo)?;
        fs::write(dev_repo.join("Cargo.toml"), "[package]\nname = \"sai-paper\"\n")?;
        assert!(is_project_root(&dev_repo));
        // Cargo.toml が無く `.sai/` 単独なら不算出。
        let empty = temp.join("empty");
        fs::create_dir_all(&empty)?;
        assert!(!is_project_root(&empty));

        fs::remove_dir_all(&temp)?;
        Ok(())
    }

    /// `to_stored_path` / `resolve_stored_path` の往復（data_dir の中のファイル）。
    #[test]
    fn stored_path_roundtrip_inside_data_dir() {
        let temp = temp_data_dir("inside");
        let pdf = temp.join("papers").join("0001_Hello").join("paper.pdf");
        fs::create_dir_all(pdf.parent().unwrap()).unwrap();
        fs::write(&pdf, b"%PDF-1.4").unwrap();

        let stored = to_stored_path(&temp, &pdf);
        assert_eq!(stored, "papers/0001_Hello/paper.pdf", "区切りは /");
        assert_eq!(resolve_stored_path(&temp, &stored), pdf);

        // md / 画像も同じ規則。
        let md = pdf.parent().unwrap().join("extract.md");
        fs::write(&md, "# body").unwrap();
        assert_eq!(to_stored_path(&temp, &md), "papers/0001_Hello/extract.md");
        let _ = fs::remove_dir_all(&temp);
    }

    /// data_dir の外のパスは絶対パスのまま残り、resolve でもそのまま返る。
    #[test]
    fn stored_path_outside_data_dir_stays_absolute() {
        let temp = temp_data_dir("outside");
        let data_dir = temp.join(".sai").join("paper");
        fs::create_dir_all(&data_dir).unwrap();
        let outside = temp.join("data").join("extracts").join("old.md");
        fs::create_dir_all(outside.parent().unwrap()).unwrap();
        fs::write(&outside, "# old").unwrap();

        let stored = to_stored_path(&data_dir, &outside);
        assert_eq!(PathBuf::from(&stored), outside, "外のパスは絶対のまま");
        assert_eq!(resolve_stored_path(&data_dir, &stored), outside);
        let _ = fs::remove_dir_all(&temp);
    }

    /// 存在しないファイルでも data_dir の中なら相対パスになる。
    #[test]
    fn stored_path_handles_missing_files() {
        let temp = temp_data_dir("missing");
        // 親だけ存在する
        let folder = temp.join("papers").join("0002_New");
        fs::create_dir_all(&folder).unwrap();
        let pdf = folder.join("paper.pdf");
        assert_eq!(to_stored_path(&temp, &pdf), "papers/0002_New/paper.pdf");
        // 親ごと存在しない
        let future = temp.join("papers").join("0003_Future").join("paper.pdf");
        assert_eq!(
            to_stored_path(&temp, &future),
            "papers/0003_Future/paper.pdf"
        );
        assert_eq!(
            resolve_stored_path(&temp, "papers/0003_Future/paper.pdf"),
            future
        );
        let _ = fs::remove_dir_all(&temp);
    }

    /// 符号リンク経由と実体のパスで渡しても、同じ相対パスになる
    /// （macOS の `/tmp` ↔ `/private/tmp` のような差で失敗しない）。
    #[test]
    fn stored_path_tolerates_symlink_difference() {
        let temp = temp_data_dir("symlink");
        let real = temp.join("real");
        let data_dir = real.join("proj").join(".sai").join("paper");
        let pdf = data_dir.join("papers").join("0005_Link").join("paper.pdf");
        fs::create_dir_all(pdf.parent().unwrap()).unwrap();
        fs::write(&pdf, b"%PDF-1.4").unwrap();
        let link = temp.join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let linked_data_dir = link.join("proj").join(".sai").join("paper");
        let linked_pdf = link
            .join("proj")
            .join(".sai")
            .join("paper")
            .join("papers")
            .join("0005_Link")
            .join("paper.pdf");

        let expected = "papers/0005_Link/paper.pdf";
        // 実体の data_dir + リンク経由のファイル
        assert_eq!(to_stored_path(&data_dir, &linked_pdf), expected);
        // リンク経由の data_dir + 実体のファイル
        assert_eq!(to_stored_path(&linked_data_dir, &pdf), expected);
        // リンク経由どうし
        assert_eq!(to_stored_path(&linked_data_dir, &linked_pdf), expected);
        // 復元したパスはどちらでも実在する
        assert!(resolve_stored_path(&data_dir, expected).is_file());
        assert!(resolve_stored_path(&linked_data_dir, expected).is_file());

        // 存在しないファイルでも符号リンク差で data_dir の中と判定できる。
        let future = linked_data_dir
            .join("papers")
            .join("0006_Tmp")
            .join("paper.pdf");
        assert_eq!(
            to_stored_path(&data_dir, future),
            "papers/0006_Tmp/paper.pdf"
        );
        let _ = fs::remove_dir_all(&temp);
    }

    /// resolve は絶対パスをそのまま、相対パスを data_dir 連結で返す。
    #[test]
    fn resolve_stored_path_keeps_absolute_and_joins_relative() {
        let data_dir = Path::new("/tmp/whatever/.sai/paper");
        assert_eq!(
            resolve_stored_path(data_dir, "/a/b/c.pdf"),
            PathBuf::from("/a/b/c.pdf")
        );
        assert_eq!(
            resolve_stored_path(data_dir, "papers/0001/paper.pdf"),
            PathBuf::from("/tmp/whatever/.sai/paper/papers/0001/paper.pdf")
        );
    }
}
