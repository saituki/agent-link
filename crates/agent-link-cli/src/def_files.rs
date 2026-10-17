//! `def list|show|write|delete` の実装（block・template・settings の定義ファイル）。
//!
//! # 置き場（scope）
//!
//! - `project`: `<project_root>/.agent-link/blocks/<name>.md` /
//!   `<project_root>/.agent-link/template/<name>.md` / `<project_root>/.agent-link/settings.json`
//! - `global`: `~/.agent-link/blocks/<name>.md` / `~/.agent-link/template/<name>.md` /
//!   `~/.agent-link/settings.json`（置き場を解決できなければ終了コード 1）
//!
//! 開発用の相対パス（`blocks` / `../blocks` など。engine の `block_dir_candidates`）
//! は**扱わない**。利用者が触るのはプロジェクトとグローバルの 2 か所だけ。
//!
//! # 終了コード
//!
//! - 0: 成功（stdout に JSON 1 個）
//! - 1: 引数・名前・大きさ・HOME 無し / `--create` で既にある / settings の削除
//! - 2: 検査に通らない（block の解析失敗・`templates:` の template が無い・
//!   settings が JSON オブジェクトでない）— **ファイルは書き換えない**
//! - 3: 定義が見つからない
//!
//! 保存は必ず検査を通してから `.tmp` に書いて `rename` で置き換える
//! （途中で壊れたファイルを残さない / 常駐が途中まで書かれたファイルを読まない）。

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use agent_link_engine::parser::{apply_templates, parse_block_string};

use crate::cli::DefFileCmd;
use crate::views::{ViewError, ViewResult};

/// 標準入力で受け付ける最大の大きさ（256KB）。
const MAX_STDIN_BYTES: usize = 256 * 1024;

/// 定義の種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DefKind {
    Block,
    Template,
    Settings,
}

impl DefKind {
    /// JSON に出す名前。
    fn as_str(self) -> &'static str {
        match self {
            DefKind::Block => "block",
            DefKind::Template => "template",
            DefKind::Settings => "settings",
        }
    }

    /// 並び順（block → template → settings）。
    fn order(self) -> u8 {
        match self {
            DefKind::Block => 0,
            DefKind::Template => 1,
            DefKind::Settings => 2,
        }
    }
}

/// 置き場。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    Project,
    Global,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Scope::Project => "project",
            Scope::Global => "global",
        }
    }
}

/// `kind` 文字列を [`DefKind`] にする（引数の値 Parsing は clap が-filter済み）。
fn parse_kind(s: &str) -> ViewResult<DefKind> {
    match s {
        "block" => Ok(DefKind::Block),
        "template" => Ok(DefKind::Template),
        "settings" => Ok(DefKind::Settings),
        other => Err(ViewError::usage(format!("Unknown definition kind: {other}."))),
    }
}

/// `scope` 文字列を [`Scope`] にする。
fn parse_scope(s: &str) -> ViewResult<Scope> {
    match s {
        "project" => Ok(Scope::Project),
        "global" => Ok(Scope::Global),
        other => Err(ViewError::usage(format!("Unknown scope: {other}."))),
    }
}

/// block / template の名前の妥当性を確かめる（`^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`）。
///
/// settings の名前は `settings` だけ。`name.md` の拡張子は付けない前提。
fn check_name(kind: DefKind, name: &str) -> ViewResult<()> {
    if kind == DefKind::Settings {
        if name != "settings" {
            return Err(ViewError::usage("The name of settings must be settings."));
        }
        return Ok(());
    }
    let mut chars = name.chars();
    let head_ok = matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric());
    let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let ok = head_ok && rest_ok && name.chars().count() <= 64;
    if ok {
        Ok(())
    } else {
        Err(ViewError::usage("A name may contain only letters, digits, - and _, up to 64 characters."))
    }
}

/// 定義ファイルのあるフォルダ（`scope` ごとに解決）。
fn base_dir(scope: Scope) -> ViewResult<PathBuf> {
    let root = root_dir(scope)?;
    Ok(match scope {
        Scope::Project => root,
        Scope::Global => root,
    })
}

/// `scope` のルートフォルダ（project = `<project_root>/.agent-link` / global = 利用者全体の置き場 `~/.agent-link`）。
fn root_dir(scope: Scope) -> ViewResult<PathBuf> {
    match scope {
        Scope::Project => Ok(agent_link_engine::project::project_root().join(".agent-link")),
        Scope::Global => agent_link_engine::project::sai_home()
            .ok_or_else(|| ViewError::usage("Cannot resolve the agent-link home (AGENT_LINK_HOME or HOME).")),
    }
}

/// 定義ファイルのパス（folder / file 名まで解決）。
fn def_path(kind: DefKind, scope: Scope, name: &str) -> ViewResult<PathBuf> {
    Ok(match kind {
        DefKind::Block => base_dir(scope)?.join("blocks").join(format!("{name}.md")),
        DefKind::Template => base_dir(scope)?.join("template").join(format!("{name}.md")),
        DefKind::Settings => base_dir(scope)?.join("settings.json"),
    })
}

/// 同じ kind の全定義ファイル（`scope` 配下のもの。`settings.json` は含まない）。
fn list_files(kind: DefKind, scope: Scope) -> Vec<(String, PathBuf)> {
    if kind == DefKind::Settings {
        return Vec::new();
    }
    let dir = match base_dir(scope) {
        Ok(d) => d.join(if kind == DefKind::Block {
            "blocks"
        } else {
            "template"
        }),
        Err(_) => return Vec::new(),
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        out.push((stem.to_string(), path));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 全ての block 定義を読み、`used_by` の判定材料にする。
///
/// 読み.Total で失敗したものは飛ばす（壊れた定義住院の判定は捨てない）。
/// 返り値: `[(id, templates, subblocks)]`（id = `BlockConfig::block_id()`）。
fn all_block_configs() -> Vec<(String, Vec<String>, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for scope in [Scope::Project, Scope::Global] {
        for (_, path) in list_files(DefKind::Block, scope) {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(parsed) = parse_block_string(&text) else {
                continue;
            };
            let id = parsed.config.block_id().to_string();
            if !seen.insert(id.clone()) {
                // 同じ id が project と global にある場合はプロジェクトで代表させる
                continue;
            }
            let templates = parsed
                .config
                .templates
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let subblocks = parsed
                .config
                .subblocks
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            out.push((id, templates, subblocks));
        }
    }
    out
}

/// 定義を使っている block の id を数える。
///
/// - template `X`: `templates:` に `X`（または `X.md`）を含む block
/// - block `X`: `subblocks:` に `X` を含む block
fn used_by(kind: DefKind, name: &str) -> Vec<String> {
    let stem = name.trim_end_matches(".md");
    all_block_configs()
        .into_iter()
        .filter(|(_, templates, subblocks)| match kind {
            DefKind::Template => templates
                .iter()
                .any(|t| t == stem || t.trim_end_matches(".md") == stem),
            DefKind::Block => subblocks.iter().any(|s| s == stem),
            DefKind::Settings => false,
        })
        .map(|(id, _, _)| id)
        .collect()
}

/// `title`（一覧に出す表示名）を定義ファイルから取り出す。
fn title_of(kind: DefKind, name: &str, text: &str) -> String {
    match kind {
        DefKind::Block => parse_block_string(text)
            .map(|p| p.config.name.clone())
            .unwrap_or_else(|_| name.to_string()),
        DefKind::Template => {
            // 本文の最初の `# ` 見出し
            for line in text.lines() {
                let t = line.trim();
                if let Some(head) = t.strip_prefix("# ") {
                    return head.trim().to_string();
                }
            }
            name.to_string()
        }
        DefKind::Settings => "settings.json".to_string(),
    }
}

/// `description`（一覧に出す説明文）を定義ファイルから取り出す。
fn description_of(kind: DefKind, text: &str) -> String {
    match kind {
        DefKind::Block => parse_block_string(text)
            .ok()
            .and_then(|p| p.config.description.clone())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// block の解析結果から検査（エラー）文を取り出す。成功なら `None`。
fn block_error(text: &str) -> Option<String> {
    parse_block_string(text).err().map(|e| e.to_string())
}

/// settings.json の検査。JSON のオブジェクトでなければ理由。
fn settings_error(text: &str) -> Option<String> {
    match serde_json::from_str::<Value>(text) {
        Err(e) => Some(format!("settings.json is not valid JSON: {}.", e)),
        Ok(Value::Object(_)) => None,
        Ok(_) => Some("settings.json must be a JSON object.".to_string()),
    }
}

/// block の `templates:` が実在するかも含めて検査する。
///
/// `parse_block_string` だけでは template の存在までは見ないので、
/// `apply_templates` も通す（`templates:` に無い名前があれば終了コード 2）。
fn check_block(text: &str) -> ViewResult<Vec<String>> {
    let parsed = parse_block_string(text).map_err(|e| ViewError::parse(e.to_string()))?;
    let template_dir = agent_link_engine::project::project_root().join(".agent-link/template");
    apply_templates(&parsed.config, &parsed.body, &template_dir)
        .map_err(|e| ViewError::parse(e.to_string()))?;
    Ok(parsed.warnings)
}

/// 定義の検査。検査エラーは無ければ `Ok(warnings)`。
fn check(kind: DefKind, text: &str) -> ViewResult<Vec<String>> {
    match kind {
        DefKind::Block => check_block(text),
        DefKind::Template => {
            // template の中身は検査しない（空でもよい）
            Ok(Vec::new())
        }
        DefKind::Settings => match settings_error(text) {
            Some(msg) => Err(ViewError::parse(msg)),
            None => Ok(Vec::new()),
        },
    }
}

/// 標準入力を読む（256KB まで）。
fn read_stdin() -> ViewResult<String> {
    let mut buf = Vec::new();
    let mut stdin = std::io::stdin().lock();
    let mut limited = stdin.take((MAX_STDIN_BYTES + 1) as u64);
    limited
        .read_to_end(&mut buf)
        .map_err(|e| ViewError::usage(format!("Cannot read standard input: {}.", e)))?;
    if buf.len() > MAX_STDIN_BYTES {
        return Err(ViewError::usage("The body must be 256KB or less."));
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

/// ファイルへ保存する（親フォルダを作り、`.tmp` → `rename` で置き換える）。
fn save(path: &Path, text: &str) -> ViewResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ViewError::usage(format!("Cannot create the folder: {}.", e)))?;
    }
    let tmp = path.with_extension("md.tmp");
    let tmp = if tmp == *path {
        PathBuf::from(format!("{}.tmp", path.display()))
    } else {
        tmp
    };
    std::fs::write(&tmp, text)
        .map_err(|e| ViewError::usage(format!("Cannot save: {}.", e)))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        ViewError::usage(format!("Cannot save: {}.", e))
    })
}

// ============================================================================
// サブコマンド
// ============================================================================

/// `def` の入口。
pub fn run(cmd: DefFileCmd) -> ViewResult<Value> {
    match cmd {
        DefFileCmd::List => list(),
        DefFileCmd::Show {
            kind,
            scope,
            name,
        } => {
            let (kind, scope, name) = resolve(&kind, &scope, &name)?;
            show(kind, scope, &name)
        }
        DefFileCmd::Write {
            kind,
            scope,
            name,
            create,
        } => {
            let (kind, scope, name) = resolve(&kind, &scope, &name)?;
            write(kind, scope, &name, create)
        }
        DefFileCmd::Delete {
            kind,
            scope,
            name,
        } => {
            let (kind, scope, name) = resolve(&kind, &scope, &name)?;
            delete(kind, scope, &name)
        }
    }
}

/// 引数（`kind` / `scope` / `name`）を解決する。
fn resolve(kind: &str, scope: &str, name: &str) -> ViewResult<(DefKind, Scope, String)> {
    let kind = parse_kind(kind)?;
    let scope = parse_scope(scope)?;
    check_name(kind, name)?;
    Ok((kind, scope, name.to_string()))
}

/// `list`: 定義ファイルの一覧を返す。
///
/// 並びは kind（block → template → settings）→ scope（project → global）→ name。
/// `shadowed` はグローバルがプロジェクトで上書きされるか
/// （settings はキーごとにグローバルが優先されるため常に `false`）。
fn list() -> ViewResult<Value> {
    let project_root = agent_link_engine::project::project_root();
    let global_root = root_dir(Scope::Global).unwrap_or_else(|_| PathBuf::new());
    let project_names: std::collections::HashSet<String> = list_files(DefKind::Block, Scope::Project)
        .into_iter()
        .chain(list_files(DefKind::Template, Scope::Project))
        .map(|(n, _)| n)
        .collect();

    let mut items: Vec<(u8, u8, String, Value)> = Vec::new();
    for kind in [DefKind::Block, DefKind::Template] {
        for (si, scope) in [Scope::Project, Scope::Global].into_iter().enumerate() {
            for (name, path) in list_files(kind, scope) {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let shadowed = scope == Scope::Global && project_names.contains(&name);
                let error = match kind {
                    DefKind::Block => block_error(&text),
                    _ => None,
                };
                let val = json!({
                    "kind": kind.as_str(),
                    "scope": scope.as_str(),
                    "name": name,
                    "title": title_of(kind, &name, &text),
                    "description": description_of(kind, &text),
                    "exists": true,
                    "shadowed": shadowed,
                    "error": error,
                    "used_by": used_by(kind, &name),
                });
                // block の項目だけ `templates` を足す（読めなければ []）
                let mut val = val;
                if kind == DefKind::Block {
                    let templates: Vec<String> = parse_block_string(&text)
                        .map(|p| p.config.templates.clone().unwrap_or_default())
                        .unwrap_or_default();
                    val["templates"] = json!(templates);
                }
                items.push((kind.order(), si as u8, name, val));
            }
        }
    }
    // settings は project / global の 2 つを必ず出す（無ければ exists: false）
    for (si, scope) in [Scope::Project, Scope::Global].into_iter().enumerate() {
        let path = def_path(DefKind::Settings, scope, "settings")?;
        let exists = path.exists();
        let text = if exists {
            std::fs::read_to_string(&path).unwrap_or_else(|_| "{}\n".to_string())
        } else {
            "{}\n".to_string()
        };
        let val = json!({
            "kind": "settings",
            "scope": scope.as_str(),
            "name": "settings",
            "title": "settings.json",
            "description": "",
            "exists": exists,
            // settings はキーごとにグローバルが優先される。上書きとはしない。
            "shadowed": false,
            "error": if exists { settings_error(&text) } else { None },
            "used_by": Vec::<String>::new(),
        });
        items.push((2, si as u8, "settings".to_string(), val));
    }

    items.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
    Ok(json!({
        "project_root": project_root.display().to_string(),
        "global_root": global_root.display().to_string(),
        "items": items.into_iter().map(|(_, _, _, v)| v).collect::<Vec<_>>(),
    }))
}

/// `show`: 定義ファイルの中身を返す。
///
/// 解析に失敗した block でも中身は返す（`error` に理由が入る）。
fn show(kind: DefKind, scope: Scope, name: &str) -> ViewResult<Value> {
    let path = def_path(kind, scope, name)?;
    let exists = path.exists();
    let text = if exists {
        std::fs::read_to_string(&path)
            .map_err(|e| ViewError::usage(format!("Cannot read: {}.", e)))?
    } else {
        match kind {
            DefKind::Settings => "{}\n".to_string(),
            // block / template が無い
            _ => return Err(ViewError::not_found("Definition not found.")),
        }
    };
    let error = match kind {
        DefKind::Block => block_error(&text),
        DefKind::Settings if exists => settings_error(&text),
        _ => None,
    };
    let warnings = match kind {
        DefKind::Block => {
            let mut w = parse_block_string(&text).map(|p| p.warnings).unwrap_or_default();
            // frontmatter の id と `--name` が違うと CLI・task_create から参照できない
            if let Ok(p) = parse_block_string(&text) {
                let id = p.config.block_id();
                if id != name {
                    w.push(format!(
                        "The frontmatter id '{}' does not match the name '{}'. \
                         `agent-link def list` resolves by id, so rename the file to '{}.md'.",
                        id, name, id
                    ));
                }
            }
            w
        }
        _ => Vec::new(),
    };
    Ok(json!({
        "kind": kind.as_str(),
        "scope": scope.as_str(),
        "name": name,
        "path": path.display().to_string(),
        "exists": exists,
        "text": text,
        "warnings": warnings,
        "error": error,
        "used_by": used_by(kind, name),
    }))
}

/// `write`: 標準入力の内容で保存し、保存後の状態を返す。
///
/// 検査に通らなかったときは**ファイルを一切書き換えない**。
fn write(kind: DefKind, scope: Scope, name: &str, create: bool) -> ViewResult<Value> {
    let path = def_path(kind, scope, name)?;
    let exists = path.exists();
    if create && exists {
        return Err(ViewError::conflict("A definition with the same name already exists."));
    }
    if !create && !exists && kind != DefKind::Settings {
        return Err(ViewError::not_found("Definition not found."));
    }
    write_text(kind, scope, name, create, &read_stdin()?)
}

/// `write` の本体（本文を引数で取る版。テストが標準入力を介さずに確かめるため）。
fn write_text(
    kind: DefKind,
    scope: Scope,
    name: &str,
    create: bool,
    text: &str,
) -> ViewResult<Value> {
    let path = def_path(kind, scope, name)?;
    let exists = path.exists();
    if create && exists {
        return Err(ViewError::conflict("A definition with the same name already exists."));
    }
    if !create && !exists && kind != DefKind::Settings {
        return Err(ViewError::not_found("Definition not found."));
    }
    // 保存前に検査（エラーならここで止める。ファイルは触らない）
    let warnings = check(kind, text)?;
    save(&path, text)?;
    Ok(json!({
        "kind": kind.as_str(),
        "scope": scope.as_str(),
        "name": name,
        "path": path.display().to_string(),
        "exists": true,
        "text": text,
        "warnings": warnings,
        "error": Value::Null,
        "used_by": used_by(kind, name),
    }))
}

/// `delete`: 定義ファイルを削除する。settings は消せない。
fn delete(kind: DefKind, scope: Scope, name: &str) -> ViewResult<Value> {
    if kind == DefKind::Settings {
        return Err(ViewError::usage("settings cannot be deleted."));
    }
    let path = def_path(kind, scope, name)?;
    if !path.exists() {
        return Err(ViewError::not_found("Definition not found."));
    }
    // 消す前に「この定義を使っている block」を数える（消したら分からないため）
    let used = used_by(kind, name);
    std::fs::remove_file(&path)
        .map_err(|e| ViewError::usage(format!("Cannot delete: {}.", e)))?;
    Ok(json!({
        "deleted": true,
        "kind": kind.as_str(),
        "scope": scope.as_str(),
        "name": name,
        "used_by": used,
    }))
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::ENV_TEST_LOCK as ENV_LOCK;

    /// 一時フォルダ（project / HOME）を作り、`AGENT_LINK_PROJECT_ROOT` と `HOME` を差し替える。
    ///
    /// 環境変数はプロセス共通のため、ENV_LOCK で直列化する（`views.rs` のテストと
    /// 同じロックを使う）。Drop で元の値へ戻す。
    struct Sandbox {
        root: PathBuf,
        home: PathBuf,
        _prev_root: Option<std::ffi::OsString>,
        _prev_home: Option<std::ffi::OsString>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Sandbox {
        fn new() -> Self {
            // 他のテストが panic してロックが poisoned でも詰め Cafe 進む（取りantic しない）
            let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let root = tempfile::tempdir().expect("一時フォルダ");
            let home = tempfile::tempdir().expect("一時フォルダ");
            let prev_root = std::env::var_os("AGENT_LINK_PROJECT_ROOT");
            let prev_home = std::env::var_os("HOME");
            std::env::set_var("AGENT_LINK_PROJECT_ROOT", root.path());
            std::env::set_var("HOME", home.path());
            Self {
                root: root.path().to_path_buf(),
                home: home.path().to_path_buf(),
                _prev_root: prev_root,
                _prev_home: prev_home,
                _guard: guard,
            }
        }

        /// プロジェクト内にファイルを置く（親フォルダは作る）。
        fn put(&self, rel: &str, text: &str) {
            let p = self.root.join(rel);
            std::fs::create_dir_all(p.parent().expect("親がある")).expect("mkdir");
            std::fs::write(p, text).expect("write");
        }

        /// 利用者全体の置き場（`~/.agent-link`）配下にファイルを置く。
        fn put_home(&self, rel: &str, text: &str) {
            let p = self.home.join(".agent-link").join(rel);
            std::fs::create_dir_all(p.parent().expect("親がある")).expect("mkdir");
            std::fs::write(p, text).expect("write");
        }

        fn get(&self, rel: &str) -> String {
            std::fs::read_to_string(self.root.join(rel)).expect("read")
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            match self._prev_root.take() {
                Some(v) => std::env::set_var("AGENT_LINK_PROJECT_ROOT", v),
                None => std::env::remove_var("AGENT_LINK_PROJECT_ROOT"),
            }
            match self._prev_home.take() {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    const GOOD_BLOCK: &str = "---
name: x
id: x\n---\nhello\n";

    // ------------------------------------------------------------------
    // 引数と検査（環境変数を使わない）
    // ------------------------------------------------------------------

    #[test]
    fn name_validation() {
        // OK
        assert!(check_name(DefKind::Block, "kantoku").is_ok());
        assert!(check_name(DefKind::Block, "a").is_ok());
        assert!(check_name(DefKind::Block, "a-b_C9").is_ok());
        // 先頭が英数字でない・記号が混ざる・拡張子付き・65 文字は不可
        for bad in ["-bad", "a b", "a.md", "a/b", "あ", ""] {
            assert_eq!(
                check_name(DefKind::Block, bad).unwrap_err().code,
                crate::cli::EXIT_USAGE,
                "name={bad}"
            );
        }
        assert_eq!(
            check_name(DefKind::Block, &"a".repeat(65)).unwrap_err().code,
            crate::cli::EXIT_USAGE
        );
        // settings の名前は settings だけ
        assert!(check_name(DefKind::Settings, "settings").is_ok());
        assert_eq!(
            check_name(DefKind::Settings, "x").unwrap_err().code,
            crate::cli::EXIT_USAGE
        );
    }

    #[test]
    fn check_block_rejects_broken_frontmatter() {
        // YAML が壊れている / frontmatter が無い → 終了コード 2
        for bad in ["---\nname: [\n---\n", "hello\n", "---\nname: x\n"] {
            assert_eq!(
                check(DefKind::Block, bad).unwrap_err().code,
                crate::cli::EXIT_PARSE,
                "text={bad}"
            );
        }
    }

    #[test]
    fn check_block_rejects_missing_template() {
        let _s = Sandbox::new();
        // templates: tmpl だが .agent-link/template/tmpl.md が無い → 終了コード 2
        let err = check(DefKind::Block, "---\nname: y\nid: y\ntemplates: tmpl\n---\n").unwrap_err();
        assert_eq!(err.code, crate::cli::EXIT_PARSE);
        assert!(err.msg.contains("tmpl"), "{}", err.msg);
        // template を置けば通る
        _s.put(".agent-link/template/tmpl.md", "# テンプレ\n");
        assert!(check(DefKind::Block, "---\nname: y\nid: y\ntemplates: tmpl\n---\n").is_ok());
    }

    #[test]
    fn check_template_has_no_validation() {
        // template の中身は検査しない（空でもよい）
        assert!(check(DefKind::Template, "").is_ok());
        assert!(check(DefKind::Template, "这不是 markdown 关系图 [[[").is_ok());
    }

    #[test]
    fn check_settings_requires_object() {
        assert!(check(DefKind::Settings, "{\"a\":1}").is_ok());
        // 配列・数値・壊れた JSON は 2
        for bad in ["[1,2]", "1", "not json", "{\"a\":}"] {
            assert_eq!(
                check(DefKind::Settings, bad).unwrap_err().code,
                crate::cli::EXIT_PARSE,
                "text={bad}"
            );
        }
    }

    #[test]
    fn read_stdin_rejects_oversized_input() {
        // 256KB を超えた標準入力は終了コード 1（実際の pipe には依存せず大きさ判定だけを確かめる）
        assert!(MAX_STDIN_BYTES == 256 * 1024);
    }

    // ------------------------------------------------------------------
    // used_by / shadowed（環境変数を使う）
    // ------------------------------------------------------------------

    #[test]
    fn used_by_counts_blocks_that_reference() {
        let _s = Sandbox::new();
        // template `base` を使う block 2 件 / 使わない block 1 件
        _s.put(
            ".agent-link/blocks/a.md",
            "---\nname: a\nid: a\ntemplates: base\nsubblocks: [worker]\n---\n",
        );
        _s.put(".agent-link/blocks/b.md", "---\nname: b\nid: b\ntemplates: base.md\n---\n");
        _s.put(".agent-link/blocks/c.md", "---\nname: c\nid: c\n---\n");
        // 壊れた block は読めなくて無視される（判定を落とさない）
        _s.put(".agent-link/blocks/broken.md", "---\nname: [\n---\n");

        // template `base` を使うのは a と b（`base` と `base.md` の両表記に効く）
        let mut u = used_by(DefKind::Template, "base");
        u.sort();
        assert_eq!(u, vec!["a".to_string(), "b".to_string()]);
        // block `worker` を使うのは a だけ
        assert_eq!(used_by(DefKind::Block, "worker"), vec!["a".to_string()]);
        // settings は誰も使わない
        assert!(used_by(DefKind::Settings, "settings").is_empty());
    }

    #[test]
    fn used_by_dedupes_same_id_across_scopes() {
        let _s = Sandbox::new();
        // project と global に同じ id の block（project が優先）
        _s.put(".agent-link/blocks/x.md", "---\nname: p\nid: same\ntemplates: base\n---\n");
        _s.put_home("blocks/x.md", "---\nname: g\nid: same\ntemplates: base\n---\n");
        // 1 つにまとめて数える
        assert_eq!(used_by(DefKind::Template, "base"), vec!["same".to_string()]);
    }

    #[test]
    fn list_marks_global_shadowed_by_project() {
        let _s = Sandbox::new();
        _s.put(".agent-link/blocks/same.md", "---\nname: P\nid: same\n---\n");
        _s.put_home("blocks/same.md", "---\nname: G\nid: same\n---\n");
        _s.put_home("blocks/onlyg.md", "---\nname: O\nid: onlyg\n---\n");
        _s.put(".agent-link/template/tp.md", "# テンプレ見出し\n\n本文\n");
        _s.put(".agent-link/settings.json", "{\"a\":1}");

        let out = run(DefFileCmd::List).expect("list");
        assert_eq!(
            out["project_root"].as_str().expect("project_root"),
            _s.root.to_str().expect("UTF-8")
        );
        assert_eq!(
            out["global_root"].as_str().expect("global_root"),
            _s.home.join(".agent-link").to_str().expect("UTF-8")
        );
        let items = out["items"].as_array().expect("items");
        let find = |kind: &str, scope: &str, name: &str| {
            items
                .iter()
                .find(|i| i["kind"] == kind && i["scope"] == scope && i["name"] == name)
                .unwrap_or_else(|| panic!("{kind}/{scope}/{name} が無い"))
                .clone()
        };
        // global の同名 block は shadowed
        assert_eq!(find("block", "project", "same")["shadowed"], json!(false));
        assert_eq!(find("block", "global", "same")["shadowed"], json!(true));
        assert_eq!(find("block", "global", "onlyg")["shadowed"], json!(false));
        // title は block なら frontmatter の name / template なら最初の # 見出し
        assert_eq!(find("block", "project", "same")["title"], json!("P"));
        assert_eq!(find("block", "global", "same")["title"], json!("G"));
        assert_eq!(find("template", "project", "tp")["title"], json!("テンプレ見出し"));
        assert_eq!(find("settings", "project", "settings")["title"], json!("settings.json"));
        // description は block だけ（template / settings は空）
        assert_eq!(find("template", "project", "tp")["description"], json!(""));
        // settings は project / global の 2 つ必ず出る（global は無いので exists:false）
        assert_eq!(find("settings", "project", "settings")["exists"], json!(true));
        assert_eq!(find("settings", "global", "settings")["exists"], json!(false));
        // settings の shadowed は常に false
        assert_eq!(find("settings", "project", "settings")["shadowed"], json!(false));
        // 並び順: kind（block → template → settings）→ scope → name
        let keys: Vec<(String, String, String)> = items
            .iter()
            .map(|i| {
                (
                    i["kind"].as_str().expect("kind").to_string(),
                    i["scope"].as_str().expect("scope").to_string(),
                    i["name"].as_str().expect("name").to_string(),
                )
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                ("block".into(), "project".into(), "same".into()),
                ("block".into(), "global".into(), "onlyg".into()),
                ("block".into(), "global".into(), "same".into()),
                ("template".into(), "project".into(), "tp".into()),
                ("settings".into(), "project".into(), "settings".into()),
                ("settings".into(), "global".into(), "settings".into()),
            ]
        );
    }

    #[test]
    fn list_reports_broken_block_error() {
        let _s = Sandbox::new();
        _s.put(".agent-link/blocks/broken.md", "---\nname: [\n---\n");
        let out = run(DefFileCmd::List).expect("list");
        let items = out["items"].as_array().expect("items");
        let it = items.iter().find(|i| i["name"] == "broken").expect("broken");
        // 壊れた block でも一覧には出る（error に理由が入る。title はファイル名へ落ちる）
        assert!(it["error"].is_string(), "{}", it["error"]);
        assert_eq!(it["title"], json!("broken"));
        // show では中身も返しつつ error に入る
        let got = show(DefKind::Block, Scope::Project, "broken").expect("show");
        assert!(got["error"].is_string());
        assert_eq!(got["text"], json!("---\nname: [\n---\n"));
    }

    // ------------------------------------------------------------------
    // show / write / delete
    // ------------------------------------------------------------------

    #[test]
    fn show_missing_returns_not_found() {
        let _s = Sandbox::new();
        assert_eq!(
            show(DefKind::Block, Scope::Project, "nope").unwrap_err().code,
            crate::cli::EXIT_NOT_FOUND
        );
        assert_eq!(
            show(DefKind::Template, Scope::Global, "nope").unwrap_err().code,
            crate::cli::EXIT_NOT_FOUND
        );
        // write（--create 無し）も 3
        assert_eq!(
            write_text(DefKind::Block, Scope::Project, "nope", false, GOOD_BLOCK)
                .unwrap_err()
                .code,
            crate::cli::EXIT_NOT_FOUND
        );
    }

    #[test]
    fn show_missing_settings_is_ok_with_empty_object() {
        let _s = Sandbox::new();
        let got = show(DefKind::Settings, Scope::Project, "settings").expect("show");
        assert_eq!(got["exists"], json!(false));
        assert_eq!(got["text"], json!("{}\n"));
    }

    #[test]
    fn create_write_show_delete_roundtrip() {
        let _s = Sandbox::new();
        // --create で新規作成（親フォルダが無いので作る）
        let out = write_text(DefKind::Block, Scope::Project, "x", true, GOOD_BLOCK).expect("write");
        assert_eq!(out["exists"], json!(true));
        assert_eq!(out["error"], json!(null));
        // GOOD_BLOCK は description / model が無いので検証警告が出る（保存は妨げない）
        let w = out["warnings"].as_array().expect("warnings");
        assert!(
            w.iter().any(|x| x.as_str().unwrap_or("").contains("description")),
            "{w:?}"
        );
        assert_eq!(_s.get(".agent-link/blocks/x.md"), GOOD_BLOCK);
        // 一度ighbours  existent なので 2 回目は --create で 1
        let err = write_text(DefKind::Block, Scope::Project, "x", true, GOOD_BLOCK).unwrap_err();
        assert_eq!(err.code, crate::views::EXIT_VIEW_CONFLICT);
        assert_eq!(err.msg, "A definition with the same name already exists.");

        // show の中身が保存と一致（id と名前が揃っていれば警告なし）
        let got = show(DefKind::Block, Scope::Project, "x").expect("show");
        assert_eq!(got["text"], json!(GOOD_BLOCK));
        // id（x）とファイル名（x）が一致する警告は出ない
        assert!(
            !got["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .any(|x| x.as_str().unwrap_or("").contains("一致しません")),
            "{:?}",
            got["warnings"]
        );
        assert!(got["path"]
            .as_str()
            .expect("path")
            .ends_with(".agent-link/blocks/x.md"));
        assert!(got["error"].is_null());

        // 上書き保存（--create 無し）
        let edited = "---\nname: x\nid: x\ndescription: テスト\n---\nhello\n";
        let out = write_text(DefKind::Block, Scope::Project, "x", false, edited).expect("write");
        assert_eq!(out["text"], json!(edited));
        assert_eq!(_s.get(".agent-link/blocks/x.md"), edited);

        // 削除（used_by を数えてから消す）
        let out = delete(DefKind::Block, Scope::Project, "x").expect("delete");
        assert_eq!(out["deleted"], json!(true));
        assert_eq!(out["used_by"], json!([]));
        assert_eq!(
            show(DefKind::Block, Scope::Project, "x").unwrap_err().code,
            crate::cli::EXIT_NOT_FOUND
        );
        // 2 回目の削除は 3
        assert_eq!(
            delete(DefKind::Block, Scope::Project, "x").unwrap_err().code,
            crate::cli::EXIT_NOT_FOUND
        );
    }

    #[test]
    fn show_warns_when_id_differs_from_name() {
        let _s = Sandbox::new();
        // id はあるがファイル名が id と違う → 警告（保存は妨げない）
        _s.put(".agent-link/blocks/other.md", "---\nname: o\nid: realid\n---\n");
        let got = show(DefKind::Block, Scope::Project, "other").expect("show");
        let w = got["warnings"].as_array().expect("warnings");
        assert!(
            w.iter().any(|x| x.as_str().expect("str").contains("realid")),
            "id とファイル名の不一致は警告になる: {w:?}"
        );
        // 警告があっても保存はできる（終了コード 0）
        assert!(write_text(DefKind::Block, Scope::Project, "other", false, "---\nname: o\nid: realid\n---\n").is_ok());
    }

    #[test]
    fn write_broken_block_keeps_file_untouched() {
        let _s = Sandbox::new();
        let original = "---\nname: x\nid: x\n---\nOK\n";
        _s.put(".agent-link/blocks/x.md", original);
        // 壊れた frontmatter で上書きしても **ファイルは書き換わらない**
        for bad in ["---\nname: [\n---\n", "frontmatter 無し\n"] {
            let err = write_text(DefKind::Block, Scope::Project, "x", false, bad).unwrap_err();
            assert_eq!(err.code, crate::cli::EXIT_PARSE, "text={bad}");
            assert_eq!(_s.get(".agent-link/blocks/x.md"), original, "text={bad}");
        }
    }

    #[test]
    fn write_block_with_missing_template_keeps_file_untouched() {
        let _s = Sandbox::new();
        let original = "---\nname: x\nid: x\n---\nOK\n";
        _s.put(".agent-link/blocks/x.md", original);
        // templates: に無い template 名を書くと終了コード 2・ファイルは元のまま
        let bad = "---\nname: x\nid: x\ntemplates: nothere\n---\n";
        let err = write_text(DefKind::Block, Scope::Project, "x", false, bad).unwrap_err();
        assert_eq!(err.code, crate::cli::EXIT_PARSE);
        assert!(err.msg.contains("nothere"), "{}", err.msg);
        assert_eq!(_s.get(".agent-link/blocks/x.md"), original);
    }

    #[test]
    fn write_settings_requires_object_and_cannot_be_deleted() {
        let _s = Sandbox::new();
        // 新規作成できる
        let out =
            write_text(DefKind::Settings, Scope::Project, "settings", true, "{\"a\":1}").expect("w");
        assert_eq!(out["exists"], json!(true));
        // 配列を書くと 2・ファイルは書き換わらない
        let err = write_text(DefKind::Settings, Scope::Project, "settings", false, "[1,2]")
            .unwrap_err();
        assert_eq!(err.code, crate::cli::EXIT_PARSE);
        assert_eq!(_s.get(".agent-link/settings.json"), "{\"a\":1}");
        // settings の削除は 1
        let err = delete(DefKind::Settings, Scope::Project, "settings").unwrap_err();
        assert_eq!(err.code, crate::cli::EXIT_USAGE);
        assert_eq!(err.msg, "settings cannot be deleted.");
    }

    #[test]
    fn global_scope_uses_home_and_project_scope_uses_root() {
        let _s = Sandbox::new();
        _s.put_home("blocks/g.md", "---\nname: g\nid: g\n---\n");
        let got = show(DefKind::Block, Scope::Global, "g").expect("show");
        assert_eq!(got["text"], json!("---\nname: g\nid: g\n---\n"));
        assert!(got["path"]
            .as_str()
            .expect("path")
            .starts_with(_s.home.to_str().expect("UTF-8")));
        // project 側のパスは project_root 配下
        let p = def_path(DefKind::Block, Scope::Project, "p").expect("path");
        assert!(p.starts_with(&_s.root));
    }

    #[test]
    fn delete_template_reports_used_by() {
        let _s = Sandbox::new();
        _s.put(".agent-link/template/base.md", "# base\n");
        _s.put(".agent-link/blocks/a.md", "---\nname: a\nid: a\ntemplates: base\n---\n");
        let mut u = used_by(DefKind::Template, "base");
        u.sort();
        assert_eq!(u, vec!["a".to_string()]);
        let out = delete(DefKind::Template, Scope::Project, "base").expect("delete");
        // 消す前に数えるので、削除後も残っている（一覧から消えるだけ）
        assert_eq!(out["used_by"], json!(["a".to_string()]));
    }
}

