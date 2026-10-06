//! `sai-moon-paper` の CLI 共通の約束（終了コード・出力・プロジェクトの決め方）の結合テスト。
//!
//! 一時フォルダの中に一時プロジェクトを作り、`SAI_PROJECT_ROOT` と `HOME` を付けて
//! 実際の実行ファイルを動かす。ネットにつながる命令は打たない。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// テストで動かす実行ファイル。
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_sai-moon-paper")
}

/// 一時フォルダ（`Drop` で消す）。
struct Temp {
    dir: PathBuf,
}

impl Temp {
    fn new(tag: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-cli-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn path(&self) -> &Path {
        &self.dir
    }

    fn home(&self) -> PathBuf {
        self.dir.join("home")
    }

    fn project(&self) -> PathBuf {
        self.dir.join("proj")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 環境をそろえて実行する。`project` が `None` なら `SAI_PROJECT_ROOT` を付けない。
fn run(cwd: &Path, home: &Path, project: Option<&Path>, args: &[&str]) -> Output {
    let mut command = Command::new(bin());
    command
        .args(args)
        .env("HOME", home)
        .env("SAI_SAI_HOME", home.join(".sai-moon"))
        .env_remove("SAI_PAPER_ROOT")
        .env_remove("SAI_ORIGIN")
        .env_remove("SAI_PARENT_RUN_ID")
        .env_remove("SAI_BLOCK_NAME")
        .current_dir(cwd);
    match project {
        Some(project) => {
            command.env("SAI_PROJECT_ROOT", project);
        }
        None => {
            command.env_remove("SAI_PROJECT_ROOT");
        }
    }
    command.output().expect("実行ファイルを動かせる")
}

/// 標準出力の文字。
fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// 標準エラーの最後の行。
fn stderr_last_line(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .last()
        .unwrap_or("")
        .to_string()
}

/// 一時プロジェクトを初期化する（sai-link の初期化は無し）。
fn init_project(temp: &Temp) -> PathBuf {
    let project = temp.project();
    std::fs::create_dir_all(&project).unwrap();
    let out = run(&project, &temp.home(), Some(&project), &["init"]);
    assert!(out.status.success(), "init が失敗: {}", String::from_utf8_lossy(&out.stderr));
    project
}

/// `library list` と `phone library` が同じ JSON を返す（古い別名の出力を変えない）。
#[test]
fn library_list_matches_phone_library() {
    let temp = Temp::new("library");
    let project = init_project(&temp);
    let home = temp.home();

    let list = run(&project, &home, Some(&project), &["library", "list"]);
    assert!(list.status.success(), "library list が失敗");
    let phone = run(&project, &home, Some(&project), &["phone", "library"]);
    assert!(phone.status.success(), "phone library が失敗");
    assert_eq!(stdout(&list), stdout(&phone), "library list と phone library の出力");
    assert!(stdout(&list).starts_with("{\"papers\""), "JSON ではない: {}", stdout(&list));
}

/// `paper show abc` は終了コード 2 で、標準エラーの最後の行が `引数の誤り: ` で始まる。
#[test]
fn paper_show_bad_id_is_exit_2() {
    let temp = Temp::new("bad-id");
    let project = init_project(&temp);
    let out = run(&project, &temp.home(), Some(&project), &["paper", "show", "abc"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stdout(&out).is_empty(), "標準出力に何か出た: {}", stdout(&out));
    assert!(
        stderr_last_line(&out).starts_with("引数の誤り: "),
        "最後の行: {}",
        stderr_last_line(&out)
    );
}

/// `paper show 999999` は終了コード 3。標準出力は空、標準エラーの最後の行は日本語。
#[test]
fn paper_show_missing_is_exit_3() {
    let temp = Temp::new("missing");
    let project = init_project(&temp);
    let out = run(&project, &temp.home(), Some(&project), &["paper", "show", "999999"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(stdout(&out).is_empty(), "標準出力に何か出た: {}", stdout(&out));
    let last = stderr_last_line(&out);
    assert!(last.contains("見つかりません"), "最後の行: {last}");
}

/// `.sai/` の無いフォルダで `status` を打つと終了コード 2。
#[test]
fn status_without_project_is_exit_2() {
    let temp = Temp::new("no-project");
    let cwd = temp.path().join("empty");
    std::fs::create_dir_all(&cwd).unwrap();
    let out = run(&cwd, &temp.home(), None, &["status"]);
    assert_eq!(out.status.code(), Some(2), "標準エラー: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout(&out).is_empty());
}

/// ホームの `.sai/` だけがあるときも終了コード 2（ホームはプロジェクトではない）。
#[test]
fn home_dot_sai_is_not_a_project() {
    let temp = Temp::new("home-sai");
    let home = temp.home();
    std::fs::create_dir_all(home.join(".sai")).unwrap();
    let cwd = home.join("sub");
    std::fs::create_dir_all(&cwd).unwrap();
    let out = run(&cwd, &home, None, &["status"]);
    assert_eq!(out.status.code(), Some(2), "標準エラー: {}", String::from_utf8_lossy(&out.stderr));
}

/// `SAI_PAPER_ROOT` に別のプロジェクトを入れても使われない。
#[test]
fn sai_paper_root_is_ignored() {
    let temp = Temp::new("paper-root");
    let other = temp.path().join("other");
    std::fs::create_dir_all(other.join(".sai/paper")).unwrap();
    let cwd = temp.path().join("empty");
    std::fs::create_dir_all(&cwd).unwrap();

    let out = Command::new(bin())
        .args(["status"])
        .env("HOME", temp.home())
        .env("SAI_SAI_HOME", temp.home().join(".sai-moon"))
        .env("SAI_PAPER_ROOT", &other)
        .env_remove("SAI_PROJECT_ROOT")
        .env_remove("SAI_ORIGIN")
        .current_dir(&cwd)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "SAI_PAPER_ROOT を使っている");
}

/// `.sai/` はあるが `.sai/paper/` が無いときは終了コード 2（論文の置き場がありません）。
#[test]
fn paper_dir_missing_is_exit_2() {
    let temp = Temp::new("paper-dir");
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai")).unwrap();
    let out = run(&project, &temp.home(), Some(&project), &["status"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr_last_line(&out).contains("論文の置き場"),
        "最後の行: {}",
        stderr_last_line(&out)
    );
}

/// `set mark` の知らない判定は終了コード 2。
#[test]
fn set_mark_bad_decision_is_exit_2() {
    let temp = Temp::new("set-mark");
    let project = init_project(&temp);
    let home = temp.home();
    let create = run(
        &project,
        &home,
        Some(&project),
        &["set", "create", "demo", "--query", "q"],
    );
    assert!(create.status.success(), "set create が失敗: {}", String::from_utf8_lossy(&create.stderr));
    let out = run(
        &project,
        &home,
        Some(&project),
        &["set", "mark", "demo", "--position", "0", "--decision", "maybe"],
    );
    assert_eq!(out.status.code(), Some(2), "標準エラー: {}", String::from_utf8_lossy(&out.stderr));
}

/// `schema --json` の形が読め、隠した命令が出ない。
#[test]
fn schema_json_lists_visible_commands() {
    let out = Command::new(bin()).args(["schema", "--json"]).output().unwrap();
    assert!(out.status.success());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON として読める");
    assert_eq!(value["bin"], "sai-moon-paper");
    let paths: Vec<Vec<String>> = value["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|cmd| {
            cmd["path"]
                .as_array()
                .unwrap()
                .iter()
                .map(|w| w.as_str().unwrap().to_string())
                .collect()
        })
        .collect();
    assert!(paths.contains(&vec!["paper".to_string(), "note".to_string()]));
    for hidden in ["phone", "get-pdf", "extract", "translate-pdf", "figures", "graph-show"] {
        assert!(
            !paths.iter().any(|path| path == &vec![hidden.to_string()]),
            "隠した {hidden} が出ている"
        );
    }
}

/// `set create --purpose/--extract` はセットの覚え書きとして残る（Node は書かない）。
#[test]
fn set_create_keeps_purpose_and_extract() {
    let temp = Temp::new("set-purpose");
    let project = init_project(&temp);
    let home = temp.home();
    let create = run(
        &project,
        &home,
        Some(&project),
        &[
            "set", "create", "demo", "--query", "q", "--purpose", "確かめる", "--extract", "効果",
        ],
    );
    assert!(
        create.status.success(),
        "set create が失敗: {}",
        String::from_utf8_lossy(&create.stderr)
    );
    let shown = run(&project, &home, Some(&project), &["set", "show", "demo"]);
    assert!(shown.status.success(), "set show が失敗: {}", String::from_utf8_lossy(&shown.stderr));
    let value: serde_json::Value = serde_json::from_slice(&shown.stdout).expect("JSON として読める");
    assert_eq!(value["set"]["purpose"], "確かめる");
    assert_eq!(value["set"]["extract"], "効果");
    assert!(value["set"]["node"].is_null(), "Node を書いている");
}

/// `init`（sai-link の初期化なし）は `.sai/paper/papr.db` を作り、`.sai/blocks/` は作らない。
#[test]
fn init_without_sai_link_creates_only_paper_dir() {
    let temp = Temp::new("init-plain");
    let project = temp.project();
    std::fs::create_dir_all(&project).unwrap();
    let out = run(&project, &temp.home(), Some(&project), &["init"]);
    assert!(out.status.success(), "init が失敗: {}", String::from_utf8_lossy(&out.stderr));
    assert!(project.join(".sai/paper/papr.db").is_file(), "papr.db が無い");
    assert!(!project.join(".sai/blocks").exists(), ".sai/blocks を作っている");
}

/// `init`（`.sai/settings.json` あり）は template を置き、もう一度実行しても 0。
/// 研究の部品（block・pre_run・分野の観点・`project-brief.md`）は作らない。
#[test]
fn init_with_settings_places_templates_and_is_idempotent() {
    let temp = Temp::new("init-settings");
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai")).unwrap();
    std::fs::write(project.join(".sai/settings.json"), "{}\n").unwrap();

    let first = run(&project, &temp.home(), Some(&project), &["init"]);
    assert!(first.status.success(), "1 回目: {}", String::from_utf8_lossy(&first.stderr));
    assert!(project.join(".sai/template/rag-simple.md").is_file(), "template が無い");
    assert!(!project.join(".sai/blocks").exists(), "block を作っている");
    assert!(
        !project.join(".sai/template/project-brief.md").exists(),
        "project-brief.md を作っている"
    );
    assert!(!project.join(".sai/paper/mind.md").exists(), "mind.md を作っている");
    let settings = std::fs::read_to_string(project.join(".sai/settings.json")).unwrap();
    assert!(!settings.contains("pre_run"), "pre_run を書いている");

    let second = run(&project, &temp.home(), Some(&project), &["init"]);
    assert_eq!(second.status.code(), Some(0), "2 回目: {}", String::from_utf8_lossy(&second.stderr));
}

/// `init --update` は template を入れ替えて `.bak` を作る。
#[test]
fn init_update_replaces_template_and_makes_a_backup() {
    let temp = Temp::new("init-update");
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai")).unwrap();
    std::fs::write(project.join(".sai/settings.json"), "{}\n").unwrap();
    assert!(run(&project, &temp.home(), Some(&project), &["init"]).status.success());

    // 利用者が template を書き換えたことにする。
    let template = project.join(".sai/template/rag-simple.md");
    let shipped_template = include_str!("../assets/template/rag-simple.md");
    std::fs::write(&template, "利用者が直した\n").unwrap();

    let out = run(&project, &temp.home(), Some(&project), &["init", "--update"]);
    assert!(out.status.success(), "init --update が失敗: {}", String::from_utf8_lossy(&out.stderr));

    // template は新しい版になり、古い物は `.bak` に退避される。
    assert_eq!(std::fs::read_to_string(&template).unwrap(), shipped_template);
    assert_eq!(
        std::fs::read_to_string(project.join(".sai/template/rag-simple.md.bak")).unwrap(),
        "利用者が直した\n",
        "`.bak` が無い"
    );
}
