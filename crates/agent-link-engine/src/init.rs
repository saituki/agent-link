//! プロジェクト初期化（`agent-link init`）
//!
//! プロジェクトの初期化処理を提供します。何度実行しても、足りないものだけを作ります。
//!
//! - `PROJECTROOT/.agent-link/` ディレクトリと必要なサブフォルダを作成する（既存は触らない）
//! - `settings.json` をシード作成する（`{}`。既にあれば触らない）
//! - `models.json` は作成しない（`~/.agent-link/models.json` のグローバル設定にフォールバック）
//! - Claude Code の設定 `.mcp.json` と `.agent-link/cc-settings.json` を作る（プロジェクトの `.claude/settings.json` は書かない）。
//!   既にある場合は `<名前>.bak`（既にあれば `<名前>.bak.<日時>`）へ退避してから作り直す
//! - serena の設定 `.serena/project.yml` を作る（使う言語はプロジェクトのファイルの拡張子から選ぶ）。
//!   既にある場合は他の設定と同じく退避する。`.serena/` の中の他のファイルには触らない
//! - design skill（`frontend-design` と `ui-ux-rules`）を `.claude/skills/` に作る。既存は他の設定と同じく退避する
//!
//! ## 作成される構成
//!
//! ```text
//! <root>/
//! ├── .mcp.json                                # cc の外部 MCP（serena / headroom / donsetch / playwright）
//! ├── .claude/commands/agent-link-finalize.md         # /agent-link-finalize（hook が LLM を通さず処理）
//! ├── .claude/skills/                         # design skill（cc が読む）
//! │   ├── frontend-design/{SKILL.md,LICENSE.txt}
//! │   └── ui-ux-rules/{SKILL.md,LICENSE}
//! ├── .serena/project.yml                      # serena の言語サーバ（拡張子から選ぶ）
//! └── .agent-link/
//!     ├── blocks/                              # ブロック定義 (*.md)
//!     ├── template/                            # templates: で参照される md
//!     ├── cc-settings.json                     # cc の道具制限・同梱 skill の抑止・statusLine・hook（`--settings` で渡す）
//!     ├── outputs/                             # run フォルダ集約（todo/running/review と日付 yymmdd）
//!     ├── engine/
//!     │   └── notifications/{pending,processed,results}/   # 通知ディレクトリ
//!     └── settings.json                        # {}
//! ```
//!
//! 旧 `sessions/` `reflection/` `waggle-cache` `todo/` `review/` `message/` は
//! 作成しない（outputs 統一後は run フォルダが正本）。
//!
//! ## 使用例
//!
//! ```no_run
//! use agent_link_engine::init::init_project;
//!
//! // カレントディレクトリをプロジェクトルートとして初期化
//! let report = init_project(std::path::Path::new("."), false)
//!     .expect("プロジェクトの初期化に失敗しました");
//!
//! for path in &report.created {
//!     println!("作成: {}", path.display());
//! }
//! ```

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::assets::AssetOutcome;

/// 初期化で作成するディレクトリ一覧（`.agent-link/` からの相対パス）。
///
/// 通知ディレクトリは `common::NOTIFY_BASE_REL`（`.agent-link/engine/notifications`）の
/// 直下にあるため、ここでまとめて作成する。
const INIT_DIRS: &[&str] = &[
    "blocks",
    "template",
    "outputs",
    "outputs/todo",
    "outputs/running",
    "outputs/review",
    "engine",
    "engine/notifications",
    "engine/notifications/pending",
    "engine/notifications/processed",
    "engine/notifications/results",
];

/// 拡張子から serena の言語サーバを選ぶときの対応表。
///
/// 値は serena の `project.template.yml` に載っている言語サーバ名（`language_servers`）。
/// JavaScript は `typescript`、C は `cpp` でよい（serena の雛形の注記と同じ）。
const SERENA_LANGUAGE_BY_EXT: &[(&str, &str)] = &[
    ("rs", "rust"),
    ("py", "python"),
    ("ts", "typescript"),
    ("tsx", "typescript"),
    ("js", "typescript"),
    ("jsx", "typescript"),
    ("mjs", "typescript"),
    ("cjs", "typescript"),
    ("go", "go"),
    ("c", "cpp"),
    ("h", "cpp"),
    ("cc", "cpp"),
    ("cpp", "cpp"),
    ("hpp", "cpp"),
    ("java", "java"),
    ("swift", "swift"),
    ("sh", "bash"),
    ("bash", "bash"),
];

/// `language_servers` に並べる順番（見つけた順番ではなく、常にこの順で並べる）。
///
/// serena は先頭の言語を既定に使うため、見つけた順番ではなく常にこの順で返す。
const SERENA_LANGUAGE_ORDER: &[&str] =
    &["rust", "python", "typescript", "go", "cpp", "java", "swift", "bash"];

/// 言語を探すときに入らないフォルダの名前。
///
/// `.` で始まるフォルダ（`.git`・`.claude` など）はまとめて飛ばす。
const SERENA_SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".venv",
    "venv",
    ".worktrees",
    ".agent-link",
    "dist",
    "build",
    "__pycache__",
];

/// 言語を探すときに見るファイル数の上限。
// ponytail: 20,000 個で止める。大きな入れ物でも init が止まらないようにするためで、
// 言語の判定には十分（同じ言語は 1 個見つければよい）。
const SERENA_SCAN_MAX_FILES: usize = 20_000;

/// 初期化エラー。
#[derive(Error, Debug)]
pub enum InitError {
    /// ディレクトリ・ファイル作成時の I/O エラー。
    #[error("I/O error during init: {0}")]
    Io(#[from] std::io::Error),
}

/// 初期化の結果。
#[derive(Debug, Default)]
pub struct InitReport {
    /// 新しく作ったファイル・フォルダ
    pub created: Vec<PathBuf>,
    /// 退避した既存ファイル（元のパス, 退避先）
    pub backed_up: Vec<(PathBuf, PathBuf)>,
    /// 配った基盤の block・template（パス, 結果）。
    ///
    /// 設定ファイルと違い、block・template は毎回作り直さない。`--update` のときだけ
    /// 中身が違う物を退避して置き換える（[`crate::assets`] を参照）。
    pub assets: Vec<(PathBuf, AssetOutcome)>,
}

/// プロジェクトルート直下の `.agent-link/` ディレクトリのパスを返す。
pub fn sai_dir(root: &Path) -> PathBuf {
    root.join(crate::project::PROJECT_DIR)
}

/// プロジェクトを初期化する（何度実行してもよい）。
///
/// `.agent-link/` 配下は無いものだけ作り、DB・outputs・blocks などの既存物には触らない。
/// cc の設定ファイル（`.mcp.json` / `.claude/settings.json`）は
/// 毎回作り直す（既存は退避する）。design skill も同じように作り直す。
///
/// 基盤の block・template は、無ければ書き、`update` が true なら中身が違う物を退避して
/// 新しい版に置き換える（`<root>/.agent-link/blocks/`・`<root>/.agent-link/template/`）。
pub fn init_project(root: &Path, update: bool) -> Result<InitReport, InitError> {
    let base = sai_dir(root);
    let mut report = InitReport::default();

    // サブフォルダの作成（既存はそのまま）
    for rel in INIT_DIRS {
        let dir = base.join(rel);
        if !dir.is_dir() {
            std::fs::create_dir_all(&dir)?;
            report.created.push(dir);
        }
    }

    // settings.json のシード作成（空オブジェクト・既存はそのまま）
    let settings_path = base.join("settings.json");
    if !settings_path.exists() {
        std::fs::write(&settings_path, "{}\n")?;
        report.created.push(settings_path);
    }

    // 基盤の block・template（無ければ書き、--update なら中身が違う物を置き換える）
    crate::assets::install_assets(&base, update, &mut report)?;

    // Claude Code の設定ファイル
    std::fs::create_dir_all(root.join(".claude/commands"))?;
    let json = |v: serde_json::Value| -> std::io::Result<String> {
        Ok(serde_json::to_string_pretty(&v).map_err(std::io::Error::other)? + "\n")
    };
    let mut files = vec![
        (root.join(".mcp.json"), json(crate::cc_command::mcp_config_json())?),
        (crate::cc_command::cc_settings_path(root), json(crate::cc_command::cc_settings_json())?),
        (
            root.join(".claude/commands/agent-link-finalize.md"),
            crate::cc_command::AGENT_LINK_FINALIZE_COMMAND_MD.to_string(),
        ),
    ];
    // serena の設定（使う言語は、今 init しているプロジェクトの拡張子から選ぶ）
    std::fs::create_dir_all(root.join(".serena"))?;
    files.push((
        root.join(".serena/project.yml"),
        serena_project_yml(root, &detect_serena_languages(root)),
    ));
    // design skill（cc が読む `.claude/skills/`）
    for (rel, text) in crate::cc_command::DESIGN_SKILL_FILES {
        let path = root.join(".claude/skills").join(rel);
        std::fs::create_dir_all(path.parent().expect("skill file has a parent folder"))?;
        files.push((path, text.to_string()));
    }
    for (path, text) in files {
        if path.exists() {
            let bak = backup_path(&path);
            std::fs::rename(&path, &bak)?;
            report.backed_up.push((path.clone(), bak));
        }
        std::fs::write(&path, text)?;
        report.created.push(path);
    }

    Ok(report)
}

/// プロジェクトで使う serena の言語サーバを、ファイルの拡張子から選ぶ。
///
/// `root` から下を `std::fs::read_dir` で再帰的にたどり、[`SERENA_LANGUAGE_BY_EXT`] で
/// 拡張子から言語を選ぶ。使わない言語のサーバを起動しないため。1 つも見つからなければ
/// serena の雛形の既定と同じ `["python"]` を返す。
fn detect_serena_languages(root: &Path) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = Vec::new();
    let mut seen = 0usize;
    let mut stack = vec![root.to_path_buf()];

    'walk: while let Some(dir) = stack.pop() {
        // 読めないフォルダ（権限が無い・消えた）は黙って飛ばす（init は止まらない）
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            // シンボリックリンクは型が symlink なので入れない（ループを避ける）
            let Ok(kind) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().to_string();
            if kind.is_dir() {
                if !is_serena_skipped_dir(&name) {
                    stack.push(entry.path());
                }
                continue;
            }
            if seen >= SERENA_SCAN_MAX_FILES {
                break 'walk;
            }
            seen += 1;
            let ext = Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase());
            let Some(lang) = ext.and_then(|e| {
                SERENA_LANGUAGE_BY_EXT.iter().find(|(x, _)| *x == e).map(|(_, l)| *l)
            }) else {
                continue;
            };
            if !found.contains(&lang) {
                found.push(lang);
            }
        }
    }

    if found.is_empty() {
        return vec!["python"];
    }
    SERENA_LANGUAGE_ORDER
        .iter()
        .copied()
        .filter(|l| found.contains(l))
        .collect()
}

/// 言語を探すときに入らないフォルダかどうか。
fn is_serena_skipped_dir(name: &str) -> bool {
    name.starts_with('.') || SERENA_SKIP_DIRS.contains(&name)
}

/// `.serena/project.yml` の内容（`agent-link init` が書く）。
///
/// 項目は serena の `project.yml` のうち必ず要るもの（`project_name` と `language_servers`）と、
/// 併せて決めておく既定（`encoding`・`ignore_all_files_in_gitignore`・`read_only`）。
/// `project_name` は `root` のフォルダ名。
fn serena_project_yml(root: &Path, languages: &[&str]) -> String {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "project".to_string());
    let mut yml = String::new();
    yml.push_str("# agent-link init が書いた serena の設定（言語はプロジェクトのファイルの拡張子から選んだ）\n");
    yml.push_str(&format!("project_name: {}\n", yaml_quoted(&name)));
    yml.push_str("language_servers:\n");
    for lang in languages {
        yml.push_str(&format!("- {}\n", lang));
    }
    yml.push_str("encoding: \"utf-8\"\n");
    yml.push_str("ignore_all_files_in_gitignore: true\n");
    yml.push_str("read_only: false\n");
    yml
}

/// YAML の文字列（`"` と `\` を `\"` `\\` にして二重引用符で囲む）。
fn yaml_quoted(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// 退避先 `<名前>.bak`（既にあれば `<名前>.bak.<YYYYMMDD-HHMMSS>`）。
///
/// 設定ファイルと基盤の部品（[`crate::assets`]）の両方で使う。
pub(crate) fn backup_path(path: &Path) -> PathBuf {
    let bak = PathBuf::from(format!("{}.bak", path.display()));
    if !bak.exists() {
        return bak;
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    PathBuf::from(format!("{}.bak.{}", path.display(), stamp))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 新規プロジェクトに必要なディレクトリ・ファイルが全て作成されること。
    #[test]
    fn test_init_project_creates_structure() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let created = init_project(root, false).unwrap().created;

        // .agent-link/ 直下の主要フォルダが存在する
        for rel in [
            "blocks",
            "template",
            "outputs",
            "outputs/todo",
            "outputs/running",
            "outputs/review",
            "engine/notifications/pending",
            "engine/notifications/processed",
            "engine/notifications/results",
        ] {
            let p = root.join(".agent-link").join(rel);
            assert!(p.is_dir(), "欠落: {}", p.display());
        }

        // settings.json が空オブジェクトで作成される
        let settings = root.join(".agent-link").join("settings.json");
        assert!(settings.is_file());
        let content = std::fs::read_to_string(&settings).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed.is_object());

        // 返り値に settings.json が含まれる
        assert!(created.iter().any(|p| p == &settings));

        // serena の設定が作られ、言語が並んでいる
        let serena = root.join(".serena").join("project.yml");
        assert!(serena.is_file(), "欠落: {}", serena.display());
        let yml = std::fs::read_to_string(&serena).unwrap();
        assert!(yml.contains("language_servers:"), "{}", yml);
        assert!(yml.contains("project_name:"), "{}", yml);
    }

    /// 使っている言語だけを選び、飛ばすフォルダの中と不可視フォルダの中は見ないこと。
    #[test]
    fn test_detect_serena_languages() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for rel in [
            "a.rs",
            "sub/b.py",
            "web/c.js",
            "node_modules/x.go",
            "target/y.java",
            ".hidden/z.swift",
            ".claude/w.cpp",
        ] {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "").unwrap();
        }
        assert_eq!(detect_serena_languages(root), vec!["rust", "python", "typescript"]);
    }

    /// 1 つも見つからなければ serena の雛形の既定（python）になること。
    #[test]
    fn test_detect_serena_languages_defaults_to_python() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_serena_languages(dir.path()), vec!["python"]);
    }

    /// `project_name` がフォルダ名、`language_servers` が渡した言語の並びになること。
    #[test]
    fn test_serena_project_yml() {
        let yml = serena_project_yml(Path::new("/tmp/demo"), &["rust", "python"]);
        assert!(yml.contains("project_name: \"demo\""), "{}", yml);
        assert!(yml.contains("language_servers:\n- rust\n- python\n"), "{}", yml);
        assert!(yml.contains("ignore_all_files_in_gitignore: true"), "{}", yml);
    }

    /// design skill が `.claude/skills/` に作られ、中身が同じであること。`.pi/` は作らない。
    #[test]
    fn test_init_project_creates_design_skills() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        init_project(root, false).unwrap();

        for (rel, text) in crate::cc_command::DESIGN_SKILL_FILES {
            let p = root.join(".claude/skills").join(rel);
            assert!(p.is_file(), "欠落: {}", p.display());
            assert_eq!(&std::fs::read_to_string(&p).unwrap(), text);
        }
        for name in ["frontend-design", "ui-ux-rules"] {
            let skill = std::fs::read_to_string(root.join(".claude/skills").join(name).join("SKILL.md"))
                .unwrap();
            assert!(skill.starts_with("---\n"), "{}", &skill[..20]);
            assert!(skill.contains(&format!("name: {}", name)), "{}", &skill[..200]);
        }
        assert!(!root.join(".pi").exists(), "agent-link init は .pi/ を作らない");
    }

    /// 再実行してもエラーにならず、`.agent-link/` の既存内容を壊さず、cc 設定は退避して作り直すこと。
    #[test]
    fn test_init_project_rerun_keeps_existing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let first = init_project(root, false).unwrap();
        assert!(first.backed_up.is_empty());
        assert!(root.join(".mcp.json").is_file());
        assert!(root.join(".agent-link/cc-settings.json").is_file());
        assert!(!root.join(".claude/settings.json").exists());

        // 既存 settings.json と outputs 配下に印を付ける
        let settings = root.join(".agent-link").join("settings.json");
        std::fs::write(&settings, "{\"marker\":true}\n").unwrap();
        let marker = root.join(".agent-link/outputs/review/keep.txt");
        std::fs::write(&marker, "keep").unwrap();
        std::fs::write(root.join(".mcp.json"), "old").unwrap();

        let second = init_project(root, false).unwrap();

        // .agent-link/ の既存ファイルは書き換えられていない
        assert!(std::fs::read_to_string(&settings).unwrap().contains("marker"));
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "keep");
        // cc と serena の設定と design skill のファイルは退避され、新しく作られる
        assert_eq!(second.backed_up.len(), 4 + crate::cc_command::DESIGN_SKILL_FILES.len());
        assert!(root.join(".claude/commands/agent-link-finalize.md").is_file());
        assert_eq!(
            std::fs::read_to_string(root.join(".mcp.json.bak")).unwrap(),
            "old"
        );
        assert!(root.join(".agent-link/cc-settings.json.bak").is_file());
        assert!(root.join(".serena/project.yml.bak").is_file());
        assert!(root.join(".claude/skills/frontend-design/SKILL.md.bak").is_file());
        let mcp: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(".mcp.json")).unwrap())
                .unwrap();
        assert!(mcp["mcpServers"].get("sai").is_none(), ".mcp.json に sai を書かない: {}", mcp);
        assert!(mcp["mcpServers"]["serena"].is_object());

        // 3 回目は .bak が既にあるので日時付きで退避される
        let third = init_project(root, false).unwrap();
        assert!(third.backed_up[0].1.to_string_lossy().contains(".bak."));
    }

    /// `sai_dir` がプロジェクトルート基準のパスを返すこと。
    #[test]
    fn test_sai_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(sai_dir(dir.path()), dir.path().join(".agent-link"));
    }

    /// 空のフォルダに基盤の block 11 本と template 13 本が配られ、
    /// block の frontmatter が `model:` を持つこと（既定値は block ごと）。
    #[test]
    fn test_init_project_installs_base_assets() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        init_project(root, false).unwrap();

        assert_eq!(crate::assets::BASE_BLOCKS.len(), 11);
        assert_eq!(crate::assets::BASE_TEMPLATES.len(), 13);
        for (name, _) in crate::assets::BASE_BLOCKS {
            let p = root.join(".agent-link/blocks").join(name);
            assert!(p.is_file(), "欠落: {}", p.display());
            let text = std::fs::read_to_string(&p).unwrap();
            let fm = text.split("---\n").nth(1).unwrap();
            assert!(fm.lines().any(|l| l.starts_with("model:")), "{name}: {text}");
        }
        for (name, _) in crate::assets::BASE_TEMPLATES {
            let p = root.join(".agent-link/template").join(name);
            assert!(p.is_file(), "欠落: {}", p.display());
        }
    }

    /// template は `--update` のときだけ置き換え、変えた物は `.bak` に逃がすこと。
    #[test]
    fn test_init_project_updates_template_only_with_flag() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init_project(root, false).unwrap();

        let path = root.join(".agent-link/template/link-task.md");
        let shipped = crate::assets::BASE_TEMPLATES
            .iter()
            .find(|(n, _)| *n == "link-task.md")
            .unwrap()
            .1;
        std::fs::write(&path, "古い中身\n").unwrap();

        // --update なしでは変わらない
        init_project(root, false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "古い中身\n");
        assert!(!root.join(".agent-link/template/link-task.md.bak").exists());

        // --update ありでは新しい版になり、.bak に古い中身が残る
        init_project(root, true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), shipped);
        assert_eq!(
            std::fs::read_to_string(root.join(".agent-link/template/link-task.md.bak")).unwrap(),
            "古い中身\n"
        );
    }

    /// block の更新では `model`・`thinking` を元の値で残し、
    /// 本文は新しい版に置き換えること。
    #[test]
    fn test_init_project_update_block_keeps_user_settings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init_project(root, false).unwrap();

        let path = root.join(".agent-link/blocks/kantoku.md");
        let old = "---\nname: 🎬 kantoku\nid: kantoku\ndescription: 手で直した\nmodel: sonnet\nthinking: max\ntemplates: link-base\nworktree: none\nenable: true\n---\n\n利用者が書いた古い本文\n";
        std::fs::write(&path, old).unwrap();

        init_project(root, true).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("model: sonnet\n"), "{text}");
        assert!(text.contains("thinking: max\n"), "{text}");
        assert!(!text.contains("model: inherit"), "{text}");
        assert!(!text.contains("利用者が書いた古い本文"), "{text}");
        // 本文は新しい版（frontmatter の後ろが一致する）
        let shipped = crate::assets::BASE_BLOCKS
            .iter()
            .find(|(n, _)| *n == "kantoku.md")
            .unwrap()
            .1;
        assert_eq!(
            text.split("---\n").nth(2).unwrap(),
            shipped.split("---\n").nth(2).unwrap()
        );
        // 元のファイルは .bak に逃がす
        assert!(root.join(".agent-link/blocks/kantoku.md.bak").is_file());
    }

    /// 中身が新しい版と同じときは `--update` でも `.bak` を作らないこと。
    #[test]
    fn test_init_project_update_no_backup_when_same() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init_project(root, false).unwrap();

        init_project(root, true).unwrap();

        assert!(!root.join(".agent-link/template/link-task.md.bak").exists());
        assert!(!root.join(".agent-link/blocks/kantoku.md.bak").exists());
    }

    /// 配る block がすべて既存の parser で読めること。
    #[test]
    fn test_base_blocks_parse() {
        for (name, content) in crate::assets::BASE_BLOCKS {
            let parsed = crate::parser::parse_block_string(content)
                .unwrap_or_else(|e| panic!("{name} を解析できない: {e}"));
            assert_eq!(
                parsed.config.id.as_deref(),
                Some(name.trim_end_matches(".md")),
                "{name}"
            );
        }
    }
}
