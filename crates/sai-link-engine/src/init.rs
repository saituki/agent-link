//! プロジェクト初期化（`sai-link init`・outputs 統一・R802）
//!
//! プロジェクトの初期化処理を提供します。何度実行しても、足りないものだけを作ります。
//!
//! - `PROJECTROOT/.sai/` ディレクトリと必要なサブフォルダを作成する（既存は触らない）
//! - `settings.json` をシード作成する（`{}`。既にあれば触らない）
//! - `models.json` は作成しない（`~/.sai/models.json` のグローバル設定にフォールバック）
//! - Claude Code の設定 `.mcp.json` と `.claude/settings.json` を作る。
//!   既にある場合は `<名前>.bak`（既にあれば `<名前>.bak.<日時>`）へ退避してから作り直す
//!
//! ## 作成される構成
//!
//! ```text
//! <root>/
//! ├── .mcp.json                                # cc の MCP（sai / serena / headroom / donsetch）
//! ├── .claude/settings.json                    # cc の道具制限・Skill 抑止・statusLine・hook
//! ├── .claude/commands/sai-finalize.md         # /sai-finalize（hook が LLM を通さず処理）
//! └── .sai/
//!     ├── blocks/                              # ブロック定義 (*.md)
//!     ├── template/                            # templates: で参照される md
//!     ├── outputs/                             # run フォルダ集約（todo/running/review/done/R*）
//!     ├── sai-engine/
//!     │   └── notifications/{pending,processed,results}/   # 通知ディレクトリ
//!     └── settings.json                        # {}
//! ```
//!
//! 旧 `sessions/` `reflection/` `waggle-cache` `todo/` `review/` `message/` は
//! 作成しない（outputs 統一後は run フォルダが正本・`migrate-outputs` で移行）。
//!
//! ## 使用例
//!
//! ```no_run
//! use sai_link_engine::init::init_project;
//!
//! // カレントディレクトリをプロジェクトルートとして初期化
//! let report = init_project(std::path::Path::new("."))
//!     .expect("プロジェクトの初期化に失敗しました");
//!
//! for path in &report.created {
//!     println!("作成: {}", path.display());
//! }
//! ```

use std::path::{Path, PathBuf};

use thiserror::Error;

/// 初期化で作成するディレクトリ一覧（`.sai/` からの相対パス）。
///
/// 通知ディレクトリは `common::NOTIFY_BASE_REL`（`.sai/sai-engine/notifications`）の
/// 直下にあるため、ここでまとめて作成する。
const INIT_DIRS: &[&str] = &[
    "blocks",
    "template",
    "outputs",
    "outputs/todo",
    "outputs/running",
    "outputs/review",
    "outputs/done",
    "sai-engine",
    "sai-engine/notifications",
    "sai-engine/notifications/pending",
    "sai-engine/notifications/processed",
    "sai-engine/notifications/results",
];

/// 初期化エラー。
#[derive(Error, Debug)]
pub enum InitError {
    /// ディレクトリ・ファイル作成時の I/O エラー。
    #[error("初期化中に I/O エラーが発生しました: {0}")]
    Io(#[from] std::io::Error),
}

/// 初期化の結果。
#[derive(Debug, Default)]
pub struct InitReport {
    /// 新しく作ったファイル・フォルダ
    pub created: Vec<PathBuf>,
    /// 退避した既存ファイル（元のパス, 退避先）
    pub backed_up: Vec<(PathBuf, PathBuf)>,
}

/// プロジェクトルート直下の `.sai/` ディレクトリのパスを返す。
pub fn sai_dir(root: &Path) -> PathBuf {
    root.join(".sai")
}

/// プロジェクトを初期化する（何度実行してもよい）。
///
/// `.sai/` 配下は無いものだけ作り、DB・outputs・blocks などの既存物には触らない。
/// cc の設定ファイル（`.mcp.json` / `.claude/settings.json`）は毎回作り直す
/// （既存は退避する）。
pub fn init_project(root: &Path) -> Result<InitReport, InitError> {
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

    // Claude Code の設定ファイル
    std::fs::create_dir_all(root.join(".claude/commands"))?;
    let json = |v: serde_json::Value| -> std::io::Result<String> {
        Ok(serde_json::to_string_pretty(&v).map_err(std::io::Error::other)? + "\n")
    };
    for (path, text) in [
        (root.join(".mcp.json"), json(crate::cc_command::mcp_config_json())?),
        (root.join(".claude/settings.json"), json(crate::cc_command::cc_settings_json())?),
        (
            root.join(".claude/commands/sai-finalize.md"),
            crate::cc_command::SAI_FINALIZE_COMMAND_MD.to_string(),
        ),
    ] {
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

/// 退避先 `<名前>.bak`（既にあれば `<名前>.bak.<YYYYMMDD-HHMMSS>`）。
fn backup_path(path: &Path) -> PathBuf {
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

        let created = init_project(root).unwrap().created;

        // .sai/ 直下の主要フォルダが存在する
        for rel in [
            "blocks",
            "template",
            "outputs",
            "outputs/todo",
            "outputs/running",
            "outputs/review",
            "outputs/done",
            "sai-engine/notifications/pending",
            "sai-engine/notifications/processed",
            "sai-engine/notifications/results",
        ] {
            let p = root.join(".sai").join(rel);
            assert!(p.is_dir(), "欠落: {}", p.display());
        }

        // settings.json が空オブジェクトで作成される
        let settings = root.join(".sai").join("settings.json");
        assert!(settings.is_file());
        let content = std::fs::read_to_string(&settings).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed.is_object());

        // 返り値に settings.json が含まれる
        assert!(created.iter().any(|p| p == &settings));
    }

    /// 再実行してもエラーにならず、`.sai/` の既存内容を壊さず、cc 設定は退避して作り直すこと。
    #[test]
    fn test_init_project_rerun_keeps_existing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let first = init_project(root).unwrap();
        assert!(first.backed_up.is_empty());
        assert!(root.join(".mcp.json").is_file());
        assert!(root.join(".claude/settings.json").is_file());

        // 既存 settings.json と outputs 配下に印を付ける
        let settings = root.join(".sai").join("settings.json");
        std::fs::write(&settings, "{\"marker\":true}\n").unwrap();
        let marker = root.join(".sai/outputs/done/keep.txt");
        std::fs::write(&marker, "keep").unwrap();
        std::fs::write(root.join(".mcp.json"), "old").unwrap();

        let second = init_project(root).unwrap();

        // .sai/ の既存ファイルは書き換えられていない
        assert!(std::fs::read_to_string(&settings).unwrap().contains("marker"));
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "keep");
        // cc 設定は 3 つとも退避され、新しく作られる
        assert_eq!(second.backed_up.len(), 3);
        assert!(root.join(".claude/commands/sai-finalize.md").is_file());
        assert_eq!(
            std::fs::read_to_string(root.join(".mcp.json.bak")).unwrap(),
            "old"
        );
        assert!(root.join(".claude/settings.json.bak").is_file());
        let mcp: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(".mcp.json")).unwrap())
                .unwrap();
        assert!(mcp["mcpServers"]["sai"].is_object());

        // 3 回目は .bak が既にあるので日時付きで退避される
        let third = init_project(root).unwrap();
        assert!(third.backed_up[0].1.to_string_lossy().contains(".bak."));
    }

    /// `sai_dir` がプロジェクトルート基準のパスを返すこと。
    #[test]
    fn test_sai_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(sai_dir(dir.path()), dir.path().join(".sai"));
    }
}
