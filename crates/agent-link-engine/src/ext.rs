//! 拡張（extension）の本体。宣言の検査・置き場への導入・プロジェクトでの有効化と無効化・CLI の起動。
//!
//! - 拡張 1 つ = 置き場の `<name>/` フォルダ（`--link` のときはフォルダを指す symlink）。
//!   置き場は [`extensions_dir`]（`AGENT_LINK_EXTENSIONS_DIR` → `config` → `$AGENT_LINK_HOME/extensions`）。
//! - 有効の記録はプロジェクトの `<root>/.agent-link/settings.json` の `extensions`。
//!   block と template は有効化のとき `<root>/.agent-link/blocks/`・`template/` に写す。
//! - 拡張の CLI が使うデータは `<root>/.agent-link/ext/<name>/`（ここでは作らない・消さない）。
//!
//! 命令の形（終了コード・標準出力・`--json` の包み）は CLI（`agent-link ext`）が決める。
//! ここは JSON の中身（[`ListResult::to_json`]・各 `*Report`）だけを作る。

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::assets::{install_one, same_as_installed, AssetOutcome};
use crate::pre_run::run_child;
use crate::project::sai_home_path;
use crate::registry::list_entries;
use crate::settings::config_var;

/// `init` の時間の上限。
pub const INIT_TIMEOUT: Duration = Duration::from_secs(300);
/// `update` の時間の上限。
pub const UPDATE_TIMEOUT: Duration = Duration::from_secs(300);
/// `ext run` の時間の上限。
pub const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// `extension.json` の中身（宣言）。知らない項目は誤り（綴りの誤りを黙って捨てないため）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub cli: Option<CliSpec>,
    /// 有効化のときに `cli` へ渡す引数。`None` なら起こさない。
    #[serde(default)]
    pub init: Option<Vec<String>>,
    /// `ext update` のときに `cli` へ渡す引数。`None` なら起こさない。
    #[serde(default)]
    pub update: Option<Vec<String>>,
    #[serde(default)]
    pub ui: Option<UiSpec>,
    #[serde(default)]
    pub service: Option<ServiceSpec>,
}

/// `cli`: 拡張の実行ファイルと、Web から呼んでよいサブ命令。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CliSpec {
    pub bin: String,
    #[serde(default)]
    pub web: Vec<String>,
}

/// `ui`: 画面に出す物。ファイルの場所は `ui/` からの相対パス。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiSpec {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    pub scripts: Vec<String>,
    #[serde(default)]
    pub styles: Vec<String>,
}

/// `service`: 外部サービスの宛先（URL は環境変数の名前で指す）と、POST を中継してよい道。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSpec {
    pub url_env: String,
    #[serde(default)]
    pub user_env: Option<String>,
    #[serde(default)]
    pub password_env: Option<String>,
    #[serde(default)]
    pub post: Vec<String>,
}

/// 置き場にある拡張 1 つ（検査を通ったもの）。
#[derive(Debug, Clone)]
pub struct Installed {
    pub manifest: Manifest,
    /// 拡張のフォルダ（`--link` のときは symlink の場所）。
    pub path: PathBuf,
    /// `path` 自体が symlink なら true。
    pub linked: bool,
    /// `blocks/` の `.md` のファイル名（名前順）。
    pub blocks: Vec<String>,
    /// `template/` の `.md` のファイル名（名前順）。
    pub templates: Vec<String>,
}

/// 拡張の操作の失敗。CLI の終了コードへの対応は CLI が決める。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtError {
    /// 入力の誤り（宣言の誤り・無いフォルダ・不正な名前）。
    Input(String),
    /// 見つからない（置き場に無い・プロジェクトで有効でない物の指定）。
    NotFound(String),
    /// 状態の衝突（他から使われている・`requires` が未有効）。
    Conflict(String),
    /// その他（書き込み・起動・`init` と `update` の失敗）。
    Other(String),
}

impl ExtError {
    pub fn message(&self) -> &str {
        match self {
            Self::Input(m) | Self::NotFound(m) | Self::Conflict(m) | Self::Other(m) => m.as_str(),
        }
    }
}

impl std::fmt::Display for ExtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for ExtError {}

/// 拡張の CLI を起こした結果。
#[derive(Debug, Clone)]
pub struct RunOutput {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// 配った（写した）ファイル 1 つ。
#[derive(Debug, Clone, Serialize)]
pub struct AssetReport {
    pub path: PathBuf,
    /// `created`・`updated`・`unchanged`・`kept` のどれか。
    pub outcome: &'static str,
    /// `updated` のとき、元の物を退避した先。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<PathBuf>,
}

/// `ext enable` の結果。
#[derive(Debug, Clone, Serialize)]
pub struct EnableReport {
    pub name: String,
    pub enabled: bool,
    pub assets: Vec<AssetReport>,
    /// `ran`（`init` を起こした）か `none`。
    pub init: &'static str,
}

/// `ext disable` の結果。
#[derive(Debug, Clone, Serialize)]
pub struct DisableReport {
    pub name: String,
    pub enabled: bool,
    pub removed: Vec<PathBuf>,
    /// 利用者が直したため残した物。
    pub kept: Vec<PathBuf>,
}

/// `ext update` の拡張 1 つ分の結果。
#[derive(Debug, Clone, Serialize)]
pub struct UpdateReport {
    pub name: String,
    pub assets: Vec<AssetReport>,
    /// `ran`（`update` を起こした）か `none`。
    pub update: &'static str,
}

/// `ext list` の結果。`errors` は読めなかった置き場の項目（フォルダと理由）。
#[derive(Debug, Clone, Default)]
pub struct ListResult {
    pub extensions: Vec<Listed>,
    pub errors: Vec<(PathBuf, String)>,
}

/// 一覧の 1 つ分。`enabled_in` は、この拡張を有効にしている登録済みプロジェクトの名前。
#[derive(Debug, Clone)]
pub struct Listed {
    pub installed: Installed,
    pub enabled_in: Vec<String>,
}

/// 拡張の置き場（`AGENT_LINK_EXTENSIONS_DIR` → `config` → `$AGENT_LINK_HOME/extensions`）。
pub fn extensions_dir() -> Option<PathBuf> {
    match config_var("AGENT_LINK_EXTENSIONS_DIR") {
        Some(dir) => Some(PathBuf::from(dir)),
        None => sai_home_path("extensions"),
    }
}

/// 拡張のフォルダ 1 つを読み、宣言を検査する。誤りは `extension.json: <理由>` の英語で返す。
pub fn load(dir: &Path) -> Result<Installed, String> {
    let text = std::fs::read_to_string(dir.join("extension.json"))
        .map_err(|e| format!("extension.json: cannot be read: {e}"))?;
    let manifest: Manifest =
        serde_json::from_str(&text).map_err(|e| format!("extension.json: {e}"))?;
    check_manifest(&manifest, dir)?;

    let blocks = md_files(&dir.join("blocks"), &manifest.name, "blocks")?;
    let templates = md_files(&dir.join("template"), &manifest.name, "template")?;
    for file in &blocks {
        check_block_id(&dir.join("blocks").join(file), file)?;
    }

    let linked = std::fs::symlink_metadata(dir)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    Ok(Installed {
        manifest,
        path: dir.to_path_buf(),
        linked,
        blocks,
        templates,
    })
}

/// 置き場の拡張を名前順に並べる。`registry` があれば、有効にしているプロジェクトも出す。
///
/// 置き場が無ければ空。フォルダの名前と `name` が違う物は `errors` に入れる。
pub fn list_in(ext_dir: &Path, registry: Option<&Path>) -> ListResult {
    let mut result = ListResult::default();
    let Ok(entries) = std::fs::read_dir(ext_dir) else {
        return result;
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let projects = registry_entries(registry);
    for path in dirs {
        let folder = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        match load(&path) {
            Err(e) => result.errors.push((path, e)),
            Ok(installed) if installed.manifest.name != folder => {
                let msg = format!(
                    "extension.json: name '{}' does not match the folder '{folder}'",
                    installed.manifest.name
                );
                result.errors.push((path, msg));
            }
            Ok(installed) => {
                let enabled_in = enabled_in(&installed.manifest.name, &projects);
                result.extensions.push(Listed {
                    installed,
                    enabled_in,
                });
            }
        }
    }
    result
        .extensions
        .sort_by(|a, b| a.installed.manifest.name.cmp(&b.installed.manifest.name));
    result
}

impl ListResult {
    /// `ext list --json` の中身（C3）。`ui`・`service`・`cli` は宣言が無ければ `null`。
    pub fn to_json(&self, ext_dir: &Path) -> Value {
        let extensions: Vec<Value> = self.extensions.iter().map(listed_json).collect();
        let errors: Vec<Value> = self
            .errors
            .iter()
            .map(|(p, e)| json!({"path": p.to_string_lossy(), "error": e}))
            .collect();
        json!({
            "dir": ext_dir.to_string_lossy(),
            "extensions": extensions,
            "errors": errors,
        })
    }
}

fn listed_json(l: &Listed) -> Value {
    let m = &l.installed.manifest;
    let title = m.title.clone().unwrap_or_else(|| m.name.clone());
    json!({
        "name": m.name,
        "title": title,
        "version": m.version,
        "description": m.description,
        "path": l.installed.path.to_string_lossy(),
        "linked": l.installed.linked,
        "requires": m.requires,
        "blocks": l.installed.blocks,
        "templates": l.installed.templates,
        "cli": m.cli.as_ref().map(|c| json!({"bin": c.bin, "web": c.web})),
        "ui": m.ui.as_ref().map(|u| json!({
            "title": u.title.clone().unwrap_or_else(|| title.clone()),
            "icon": u.icon,
            "scripts": u.scripts,
            "styles": u.styles,
        })),
        "service": m.service.as_ref().map(|s| json!({
            "url_env": s.url_env,
            "user_env": s.user_env,
            "password_env": s.password_env,
            "post": s.post,
            "ready": service_ready(&s.url_env),
        })),
        "enabled_in": l.enabled_in,
    })
}

/// 宛先の環境変数（`config_var`）が `http://` か `https://` で始まれば ready。
fn service_ready(url_env: &str) -> bool {
    config_var(url_env).is_some_and(|u| u.starts_with("http://") || u.starts_with("https://"))
}

/// 拡張を置き場へ導入する（既にあれば置き換える）。
///
/// `link` のときは `src` を指す symlink を作る。複製のときは `src` の中の symlink を誤りにする
/// （`extension.json`・`bin/`・`lib/`・`blocks/`・`template/`・`ui/` だけを写す）。
pub fn install_in(ext_dir: &Path, src: &Path, link: bool) -> Result<Installed, ExtError> {
    if !src.is_dir() {
        return Err(ExtError::Input(format!(
            "No such folder: {}",
            src.display()
        )));
    }
    let installed = load(src).map_err(ExtError::Input)?;
    if !link {
        if let Some(found) = find_symlink(src).map_err(io_err)? {
            return Err(ExtError::Input(format!(
                "The folder contains a symlink, which cannot be copied: {}",
                found.display()
            )));
        }
    }

    std::fs::create_dir_all(ext_dir).map_err(io_err)?;
    let dest = ext_dir.join(&installed.manifest.name);
    // 置き場の中の実フォルダを、そのまま自分自身で置き換えない（消してから写すため）。
    if is_real_dir(&dest) && same_existing_path(src, &dest) {
        return Err(ExtError::Input(format!(
            "The folder is already the installed extension: {}",
            dest.display()
        )));
    }
    remove_existing(&dest)?;
    if link {
        let abs = absolute(src)?;
        std::os::unix::fs::symlink(&abs, &dest).map_err(io_err)?;
    } else {
        copy_extension(src, &dest).map_err(io_err)?;
    }
    load(&dest).map_err(ExtError::Input)
}

/// 拡張を置き場から消す（symlink なら link だけ）。どこかのプロジェクトで有効なら断る。
pub fn uninstall_in(ext_dir: &Path, name: &str, registry: Option<&Path>) -> Result<(), ExtError> {
    check_name(name)?;
    let dest = ext_dir.join(name);
    if std::fs::symlink_metadata(&dest).is_err() {
        return Err(ExtError::NotFound(format!(
            "Extension {name} is not installed."
        )));
    }
    let users = enabled_in(name, &registry_entries(registry));
    if !users.is_empty() {
        return Err(ExtError::Conflict(format!(
            "Extension {name} is enabled in: {}. Disable it first.",
            users.join(", ")
        )));
    }
    remove_existing(&dest)
}

/// プロジェクトで有効な拡張の名前（`<root>/.agent-link/settings.json` の `extensions`）。
/// 無い・壊れていれば空。
pub fn enabled_names(root: &Path) -> Vec<String> {
    let Some(value) = read_settings(root) else {
        return Vec::new();
    };
    value
        .get("extensions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// 有効の一覧を書く。他のキーは保つ。ファイルが壊れていれば書かずに `Err`（中身を失わないため）。
pub fn set_enabled(root: &Path, names: &[String]) -> Result<(), String> {
    let path = settings_path(root);
    let mut value = match std::fs::read_to_string(&path) {
        Ok(text) if text.trim().is_empty() => json!({}),
        Ok(text) => serde_json::from_str::<Value>(&text)
            .map_err(|e| format!("Cannot read {}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(format!("Cannot read {}: {e}", path.display())),
    };
    let Some(obj) = value.as_object_mut() else {
        return Err(format!("{} is not a JSON object.", path.display()));
    };
    obj.insert("extensions".to_string(), json!(names));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

/// 拡張をプロジェクトで有効にする。
///
/// 手順: `requires` の確認 → block と template を写す → `init` を起こす → 記録する。
/// `init` が失敗したら記録しない（写した block と template は残る）。
pub fn enable_in(
    ext_dir: &Path,
    root: &Path,
    name: &str,
    exe: &str,
) -> Result<EnableReport, ExtError> {
    let installed = load_installed(ext_dir, name)?;
    let mut enabled = enabled_names(root);
    if let Some(missing) = installed
        .manifest
        .requires
        .iter()
        .find(|r| !has(&enabled, r))
    {
        return Err(ExtError::Conflict(format!(
            "Extension {name} requires {missing}. Enable {missing} in this project first."
        )));
    }

    let assets = write_assets(&installed, root, false)?;
    let init = match &installed.manifest.init {
        Some(args) => {
            run_hook(&installed, root, args, exe, INIT_TIMEOUT, "init")?;
            "ran"
        }
        None => "none",
    };

    if !has(&enabled, name) {
        enabled.push(name.to_string());
        set_enabled(root, &enabled).map_err(ExtError::Other)?;
    }
    Ok(EnableReport {
        name: name.to_string(),
        enabled: true,
        assets,
        init,
    })
}

/// 拡張をプロジェクトで無効にする。
///
/// 他の有効な拡張が `requires` していれば断る。写した block と template のうち、
/// 拡張の版と同じ物だけ消す（利用者が直した物は `kept` に残す）。拡張が置き場から
/// 消えていて中身が分からないときは、何も消さずに記録だけ外す。
pub fn disable_in(ext_dir: &Path, root: &Path, name: &str) -> Result<DisableReport, ExtError> {
    check_name(name)?;
    let enabled = enabled_names(root);
    if !has(&enabled, name) {
        return Err(ExtError::NotFound(format!(
            "Extension {name} is not enabled in this project."
        )));
    }
    for other in enabled
        .iter()
        .filter(|n| n.as_str() != name && valid_name(n))
    {
        if let Ok(inst) = load(&ext_dir.join(other)) {
            if has(&inst.manifest.requires, name) {
                return Err(ExtError::Conflict(format!(
                    "Extension {other} requires {name}. Disable {other} first."
                )));
            }
        }
    }

    let mut removed = Vec::new();
    let mut kept = Vec::new();
    if let Ok(inst) = load(&ext_dir.join(name)) {
        let kinds = [
            ("blocks", &inst.blocks, true),
            ("template", &inst.templates, false),
        ];
        for (dir, files, preserve) in kinds {
            for file in files {
                let dst = root.join(".agent-link").join(dir).join(file);
                if !dst.exists() {
                    continue;
                }
                let same = std::fs::read_to_string(inst.path.join(dir).join(file))
                    .is_ok_and(|c| same_as_installed(&dst, &c, preserve));
                if same {
                    std::fs::remove_file(&dst).map_err(io_err)?;
                    removed.push(dst);
                } else {
                    kept.push(dst);
                }
            }
        }
    }

    let rest: Vec<String> = enabled.into_iter().filter(|n| n != name).collect();
    set_enabled(root, &rest).map_err(ExtError::Other)?;
    Ok(DisableReport {
        name: name.to_string(),
        enabled: false,
        removed,
        kept,
    })
}

/// 有効な拡張を更新する（`name` を指定すればそれだけ）。違う物は `.bak` に退避して写す。
///
/// 最初の失敗で止まる。`update` の引数があれば、写した後に起こす。
pub fn update_in(
    ext_dir: &Path,
    root: &Path,
    name: Option<&str>,
    exe: &str,
) -> Result<Vec<UpdateReport>, ExtError> {
    let enabled = enabled_names(root);
    let targets: Vec<String> = match name {
        Some(n) => {
            check_name(n)?;
            if !has(&enabled, n) {
                return Err(ExtError::NotFound(format!(
                    "Extension {n} is not enabled in this project."
                )));
            }
            vec![n.to_string()]
        }
        None => enabled,
    };

    let mut reports = Vec::new();
    for target in targets {
        let installed = load_installed(ext_dir, &target)?;
        let assets = write_assets(&installed, root, true)?;
        let update = match &installed.manifest.update {
            Some(args) => {
                run_hook(&installed, root, args, exe, UPDATE_TIMEOUT, "update")?;
                "ran"
            }
            None => "none",
        };
        reports.push(UpdateReport {
            name: target,
            assets,
            update,
        });
    }
    Ok(reports)
}

/// 有効な拡張の CLI を起こす（`ext run`）。標準出力・終了コードはそのまま返す。
pub fn run_cli(
    ext_dir: &Path,
    root: &Path,
    name: &str,
    args: &[String],
    exe: &str,
    timeout: Duration,
) -> Result<RunOutput, ExtError> {
    let installed = load_installed(ext_dir, name)?;
    if !has(&enabled_names(root), name) {
        return Err(ExtError::Conflict(format!(
            "Extension {name} is not enabled in this project."
        )));
    }
    if installed.manifest.cli.is_none() {
        return Err(ExtError::Input(format!("Extension {name} has no cli.")));
    }
    run_command(&installed, root, args, exe, timeout)
}

/// プロジェクトで有効で `cli` を持つ拡張の、実行ファイルのフォルダ（PATH に足す物）。
///
/// 読めない拡張・名前の形が違う項目は飛ばす。
pub fn enabled_bin_dirs(ext_dir: &Path, root: &Path) -> Vec<PathBuf> {
    enabled_names(root)
        .iter()
        .filter(|n| valid_name(n))
        .filter_map(|name| {
            let inst = load(&ext_dir.join(name)).ok()?;
            let cli = inst.manifest.cli.as_ref()?;
            let bin_dir = inst.path.join(&cli.bin).parent()?.to_path_buf();
            absolute(&bin_dir).ok()
        })
        .collect()
}

// ---- 以下は内部の部品 ----

/// 宣言の検査（`load` の中で使う）。
fn check_manifest(m: &Manifest, dir: &Path) -> Result<(), String> {
    let bad = |msg: String| Err(format!("extension.json: {msg}"));
    if !valid_name(&m.name) {
        return bad(format!(
            "name '{}' is not valid (lowercase letters, digits and '-'; 1 to 32 characters; starts with a letter)",
            m.name
        ));
    }
    if m.version.trim().is_empty() {
        return bad("version is empty".to_string());
    }
    for r in &m.requires {
        if !valid_name(r) {
            return bad(format!("requires '{r}' is not a valid name"));
        }
    }

    match &m.cli {
        Some(cli) => {
            if !is_rel_path(&cli.bin) {
                return bad(format!(
                    "cli.bin '{}' must be a relative path inside the folder",
                    cli.bin
                ));
            }
            if !is_executable_file(&dir.join(&cli.bin)) {
                return bad(format!("cli.bin '{}' is not an executable file", cli.bin));
            }
            for w in &cli.web {
                if !valid_name(w) {
                    return bad(format!("cli.web '{w}' is not a valid name"));
                }
            }
        }
        None => {
            if m.init.is_some() {
                return bad("init needs cli".to_string());
            }
            if m.update.is_some() {
                return bad("update needs cli".to_string());
            }
        }
    }

    if let Some(ui) = &m.ui {
        if let Some(icon) = &ui.icon {
            check_ui_file(dir, "ui.icon", icon, "svg")?;
        }
        if ui.scripts.is_empty() {
            return bad("ui.scripts needs at least one file".to_string());
        }
        for s in &ui.scripts {
            check_ui_file(dir, "ui.scripts", s, "js")?;
        }
        for s in &ui.styles {
            check_ui_file(dir, "ui.styles", s, "css")?;
        }
    }

    if let Some(sv) = &m.service {
        check_env_name(&sv.url_env)?;
        for env in sv.user_env.iter().chain(sv.password_env.iter()) {
            check_env_name(env)?;
        }
        for p in &sv.post {
            if !valid_post_path(p) {
                return bad(format!("service.post '{p}' is not a valid path"));
            }
        }
    }
    Ok(())
}

fn check_ui_file(dir: &Path, field: &str, rel: &str, ext: &str) -> Result<(), String> {
    if !is_rel_path(rel) {
        return Err(format!(
            "extension.json: {field} '{rel}' must be a relative path inside ui/"
        ));
    }
    if Path::new(rel).extension().and_then(|e| e.to_str()) != Some(ext) {
        return Err(format!(
            "extension.json: {field} '{rel}' must be a .{ext} file"
        ));
    }
    if !dir.join("ui").join(rel).is_file() {
        return Err(format!(
            "extension.json: {field} '{rel}' does not exist in ui/"
        ));
    }
    Ok(())
}

fn check_env_name(name: &str) -> Result<(), String> {
    let ok = name.strip_prefix("AGENT_LINK_EXT_").is_some_and(|rest| {
        (1..=40).contains(&rest.len())
            && rest
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    });
    if ok {
        Ok(())
    } else {
        Err(format!(
            "extension.json: '{name}' must be AGENT_LINK_EXT_ followed by A-Z, 0-9 or _"
        ))
    }
}

/// `blocks/` か `template/` の `.md` の名前（`<name>.md` か `<name>-*.md`）を確かめ、ファイル名の一覧を返す。
fn md_files(dir: &Path, name: &str, kind: &str) -> Result<Vec<String>, String> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("extension.json: cannot read {kind}/: {e}"))?;
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        if !file.ends_with(".md") || !entry.path().is_file() {
            continue;
        }
        let stem = &file[..file.len() - 3];
        let ok = stem == name
            || stem
                .strip_prefix(name)
                .is_some_and(|rest| rest.starts_with('-') && rest.len() > 1);
        if !ok {
            return Err(format!(
                "extension.json: {kind}/{file} must be named {name}.md or {name}-*.md"
            ));
        }
        files.push(file);
    }
    files.sort();
    Ok(files)
}

/// block の frontmatter の `id` が、ファイル名（拡張子を除く）と同じか確かめる。
fn check_block_id(path: &Path, file: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("extension.json: blocks/{file} cannot be read: {e}"))?;
    let parsed = crate::parser::parse_block_string(&content)
        .map_err(|e| format!("extension.json: blocks/{file}: {e}"))?;
    let stem = file.trim_end_matches(".md");
    match parsed.config.id.as_deref() {
        Some(id) if id == stem => Ok(()),
        Some(id) => Err(format!(
            "extension.json: blocks/{file} has id '{id}', expected '{stem}'"
        )),
        None => Err(format!("extension.json: blocks/{file} has no id")),
    }
}

/// 有効化・更新で block / template を写す（`root/.agent-link/` の下へ）。
fn write_assets(
    installed: &Installed,
    root: &Path,
    update: bool,
) -> Result<Vec<AssetReport>, ExtError> {
    let mut out = Vec::new();
    let kinds = [
        ("blocks", &installed.blocks, true),
        ("template", &installed.templates, false),
    ];
    for (dir, files, preserve) in kinds {
        for file in files {
            let content =
                std::fs::read_to_string(installed.path.join(dir).join(file)).map_err(io_err)?;
            let dst = root.join(".agent-link").join(dir).join(file);
            let outcome = install_one(&dst, &content, update, preserve)
                .map_err(|e| ExtError::Other(e.to_string()))?;
            out.push(asset_report(dst, outcome));
        }
    }
    Ok(out)
}

fn asset_report(path: PathBuf, outcome: AssetOutcome) -> AssetReport {
    let (name, backup) = match outcome {
        AssetOutcome::Created => ("created", None),
        AssetOutcome::Updated { backup } => ("updated", Some(backup)),
        AssetOutcome::Unchanged => ("unchanged", None),
        AssetOutcome::Kept => ("kept", None),
    };
    AssetReport {
        path,
        outcome: name,
        backup,
    }
}

/// `init` / `update` を起こす。失敗（0 以外）は標準エラーの最後の行を理由にする。
fn run_hook(
    installed: &Installed,
    root: &Path,
    args: &[String],
    exe: &str,
    timeout: Duration,
    what: &str,
) -> Result<(), ExtError> {
    let out = run_command(installed, root, args, exe, timeout)?;
    if out.code == 0 {
        return Ok(());
    }
    let reason = String::from_utf8_lossy(&out.stderr)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("exit code {}", out.code));
    Err(ExtError::Other(format!(
        "Extension {what} failed: {reason}"
    )))
}

/// 拡張の `cli.bin` を、シェルを通さずに起こす。cwd は `root`、`AGENT_LINK_BIN` を足す。
fn run_command(
    installed: &Installed,
    root: &Path,
    args: &[String],
    exe: &str,
    timeout: Duration,
) -> Result<RunOutput, ExtError> {
    let Some(cli) = &installed.manifest.cli else {
        return Err(ExtError::Input(format!(
            "Extension {} has no cli.",
            installed.manifest.name
        )));
    };
    let bin = absolute(&installed.path.join(&cli.bin))?;
    let mut cmd = vec![bin.to_string_lossy().to_string()];
    cmd.extend(args.iter().cloned());
    let (status, stdout, stderr) = run_child(
        &cmd,
        root,
        &[("AGENT_LINK_BIN", exe.to_string())],
        None,
        timeout,
    )
    .map_err(ExtError::Other)?;
    match status {
        None => Err(ExtError::Other(format!(
            "Extension command timed out after {} seconds.",
            timeout.as_secs()
        ))),
        Some(s) => Ok(RunOutput {
            code: s.code().unwrap_or(1),
            stdout,
            stderr,
        }),
    }
}

/// 置き場の拡張を名前で読む。無ければ `NotFound`、壊れていれば `Other`。
fn load_installed(ext_dir: &Path, name: &str) -> Result<Installed, ExtError> {
    check_name(name)?;
    let dir = ext_dir.join(name);
    if std::fs::symlink_metadata(&dir).is_err() {
        return Err(ExtError::NotFound(format!(
            "Extension {name} is not installed."
        )));
    }
    load(&dir).map_err(ExtError::Other)
}

fn registry_entries(registry: Option<&Path>) -> Vec<(String, PathBuf)> {
    registry.map(list_entries).unwrap_or_default()
}

/// `name` を有効にしているプロジェクトの名前（登録簿の順）。
fn enabled_in(name: &str, projects: &[(String, PathBuf)]) -> Vec<String> {
    projects
        .iter()
        .filter(|(_, root)| has(&enabled_names(root), name))
        .map(|(project, _)| project.clone())
        .collect()
}

fn settings_path(root: &Path) -> PathBuf {
    root.join(".agent-link").join("settings.json")
}

fn read_settings(root: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(settings_path(root)).ok()?;
    serde_json::from_str(&text).ok()
}

fn has(list: &[String], name: &str) -> bool {
    list.iter().any(|n| n == name)
}

/// 拡張の名前の形（`^[a-z][a-z0-9-]{0,31}$`）。
fn valid_name(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=32).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

fn check_name(name: &str) -> Result<(), ExtError> {
    if valid_name(name) {
        Ok(())
    } else {
        Err(ExtError::Input(format!("Invalid extension name: {name}")))
    }
}

/// 拡張の中の相対パス（`..`・絶対パス・空・`\` は不可）。
fn is_rel_path(p: &str) -> bool {
    !p.is_empty()
        && !p.contains('\\')
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// 道（`^[A-Za-z0-9._/-]{1,200}$`）。`..` の区切りは不可。
fn valid_post_path(p: &str) -> bool {
    (1..=200).contains(&p.len())
        && p.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
        && !p.split('/').any(|s| s == "..")
}

fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn absolute(p: &Path) -> Result<PathBuf, ExtError> {
    std::path::absolute(p)
        .map_err(|e| ExtError::Other(format!("Cannot resolve {}: {e}", p.display())))
}

fn io_err(e: std::io::Error) -> ExtError {
    ExtError::Other(e.to_string())
}

fn is_real_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
}

fn same_existing_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// 既にある物（フォルダ・symlink・ファイル）を消す。symlink は link だけを消す。
fn remove_existing(path: &Path) -> Result<(), ExtError> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Ok(());
    };
    if meta.is_dir() {
        std::fs::remove_dir_all(path).map_err(io_err)
    } else {
        std::fs::remove_file(path).map_err(io_err)
    }
}

/// symlink を含む物を探す（フォルダの中を再帰で見る）。
fn find_symlink(dir: &Path) -> std::io::Result<Option<PathBuf>> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Ok(Some(entry.path()));
        }
        if file_type.is_dir() {
            if let Some(found) = find_symlink(&entry.path())? {
                return Ok(Some(found));
            }
        }
    }
    Ok(None)
}

/// 複製: `extension.json` と、`bin/`・`lib/`・`blocks/`・`template/`・`ui/` を写す。
fn copy_extension(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    std::fs::copy(src.join("extension.json"), dest.join("extension.json"))?;
    for sub in ["bin", "lib", "blocks", "template", "ui"] {
        if src.join(sub).is_dir() {
            copy_tree(&src.join(sub), &dest.join(sub))?;
        }
    }
    Ok(())
}

/// フォルダを再帰で写す（`std::fs::copy` は実行の権限を保つ）。symlink は呼ぶ前に除く。
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dst = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &dst)?;
        } else {
            std::fs::copy(entry.path(), dst)?;
        }
    }
    Ok(())
}
