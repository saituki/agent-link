//! 拡張の本体（`ext.rs`）の確かめ。すべて一時フォルダで行い、環境変数に頼らない。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_link_engine::ext::{
    disable_in, enable_in, enabled_bin_dirs, enabled_names, install_in, list_in, load, run_cli,
    uninstall_in, update_in, ExtError, RUN_TIMEOUT,
};
use tempfile::TempDir;

const EXE: &str = "/usr/local/bin/agent-link";

fn hello_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ext/hello")
}

/// 一時フォルダの中に、置き場・プロジェクト・登録簿を作る。
struct Env {
    _tmp: TempDir,
    ext_dir: PathBuf,
    root: PathBuf,
    registry: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().to_path_buf();
    let root = base.join("proj");
    fs::create_dir_all(&root).unwrap();
    let registry = base.join("registry.json");
    fs::write(
        &registry,
        serde_json::json!([{"name": "proj", "root": root.to_string_lossy()}]).to_string(),
    )
    .unwrap();
    Env {
        ext_dir: base.join("extensions"),
        root,
        registry,
        _tmp: tmp,
    }
}

/// 拡張のフォルダを作る（`files` は `(相対パス, 中身)`。`bin/` の下は実行できる物にする）。
fn make_ext(dir: &Path, manifest: &str, files: &[(&str, &str)]) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("extension.json"), manifest).unwrap();
    for (rel, body) in files {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
    }
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("bin");
    if bin.is_dir() {
        for e in fs::read_dir(&bin).unwrap() {
            let p = e.unwrap().path();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

/// フォルダを再帰で写す（symlink は含まない前提）。
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            fs::copy(e.path(), dst).unwrap();
        }
    }
}

fn block(id: &str) -> String {
    format!("---\nname: {id}\nid: {id}\ndescription: test block\nenable: true\n---\n\n本文\n")
}

/// 見本の宣言 1 つ分のフォルダ（`cli` あり、`init` なし）。
fn plain_ext(dir: &Path, name: &str, extra: &str) {
    let manifest = format!(r#"{{"name": "{name}", "version": "1.0.0"{extra}}}"#);
    make_ext(dir, &manifest, &[("bin/x", "#!/bin/sh\nexit 0\n")]);
}

// ---- 宣言の読み込み ----

#[test]
fn hello_fixture_loads() {
    let info = load(&hello_src()).expect("hello fixture must load");
    assert_eq!(info.manifest.name, "hello");
    assert_eq!(info.blocks, vec!["hello-agent.md".to_string()]);
    assert_eq!(info.templates, vec!["hello-rules.md".to_string()]);
    assert_eq!(info.manifest.cli.as_ref().unwrap().bin, "bin/hello");
    assert!(!info.linked);
}

#[test]
fn load_rejects_bad_manifests() {
    let tmp = tempfile::tempdir().unwrap();
    let cases: Vec<(&str, String, Vec<(&str, String)>, &str)> = vec![
        (
            "unknown field",
            r#"{"name": "x", "version": "1", "hooks": {}}"#.to_string(),
            vec![],
            "unknown field",
        ),
        (
            "bad name",
            r#"{"name": "Bad_Name", "version": "1"}"#.to_string(),
            vec![],
            "name 'Bad_Name' is not valid",
        ),
        (
            "parent path in cli.bin",
            r#"{"name": "x", "version": "1", "cli": {"bin": "../x"}}"#.to_string(),
            vec![],
            "cli.bin '../x' must be a relative path",
        ),
        (
            "cli.bin not executable",
            r#"{"name": "x", "version": "1", "cli": {"bin": "bin/x"}}"#.to_string(),
            vec![("bin/x", "#!/bin/sh\n".to_string())],
            "is not an executable file",
        ),
        (
            "block named outside the rule",
            r#"{"name": "x", "version": "1"}"#.to_string(),
            vec![("blocks/other.md", block("other"))],
            "must be named x.md or x-*.md",
        ),
        (
            "block id differs from file name",
            r#"{"name": "x", "version": "1"}"#.to_string(),
            vec![("blocks/x-a.md", block("x-b"))],
            "has id 'x-b', expected 'x-a'",
        ),
        (
            "ui file parent path",
            r#"{"name": "x", "version": "1", "ui": {"scripts": ["../a.js"]}}"#.to_string(),
            vec![],
            "must be a relative path inside ui/",
        ),
        (
            "service env name without prefix",
            r#"{"name": "x", "version": "1", "service": {"url_env": "URL"}}"#.to_string(),
            vec![],
            "must be AGENT_LINK_EXT_",
        ),
        (
            "service post with parent segment",
            r#"{"name": "x", "version": "1", "service": {"url_env": "AGENT_LINK_EXT_URL", "post": ["a/../b"]}}"#
                .to_string(),
            vec![],
            "is not a valid path",
        ),
        (
            "init without cli",
            r#"{"name": "x", "version": "1", "init": ["init"]}"#.to_string(),
            vec![],
            "init needs cli",
        ),
    ];
    for (i, (label, manifest, files, want)) in cases.iter().enumerate() {
        let dir = tmp.path().join(format!("case{i}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("extension.json"), manifest).unwrap();
        for (rel, body) in files {
            let path = dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, body).unwrap();
        }
        let err = load(&dir).expect_err(label);
        assert!(err.starts_with("extension.json:"), "{label}: {err}");
        assert!(err.contains(want), "{label}: {err}");
    }
}

// ---- 導入と一覧 ----

#[test]
fn install_copy_and_link_then_list() {
    let e = env();
    let copied = install_in(&e.ext_dir, &hello_src(), false).unwrap();
    assert!(!copied.linked);
    let list = list_in(&e.ext_dir, None);
    assert!(list.errors.is_empty(), "{:?}", list.errors);
    assert_eq!(list.extensions.len(), 1);
    assert_eq!(list.extensions[0].installed.manifest.name, "hello");
    assert!(list.extensions[0].enabled_in.is_empty());

    // 複製で実行の権限が保たれること
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(e.ext_dir.join("hello/bin/hello"))
        .unwrap()
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0);

    // --link は symlink に置き換える
    let linked = install_in(&e.ext_dir, &hello_src(), true).unwrap();
    assert!(linked.linked);
    assert!(list_in(&e.ext_dir, None).extensions[0].installed.linked);
}

#[test]
fn install_refuses_symlink_inside_source() {
    let e = env();
    let src = e._tmp.path().join("src");
    copy_dir(&hello_src(), &src);
    fs::remove_file(src.join("bin/hello")).unwrap();
    std::os::unix::fs::symlink(hello_src().join("bin/hello"), src.join("bin/hello")).unwrap();
    let err = install_in(&e.ext_dir, &src, false).unwrap_err();
    assert!(matches!(err, ExtError::Input(_)), "{err:?}");
    assert!(err.message().contains("symlink"), "{err}");
}

#[test]
fn list_reports_name_mismatch_as_error() {
    let e = env();
    plain_ext(&e.ext_dir.join("other-folder"), "x", "");
    let list = list_in(&e.ext_dir, None);
    assert!(list.extensions.is_empty());
    assert_eq!(list.errors.len(), 1);
    assert!(
        list.errors[0].1.contains("does not match the folder"),
        "{:?}",
        list.errors
    );
}

// ---- 有効化・無効化 ----

#[test]
fn enable_writes_assets_runs_init_and_records() {
    let e = env();
    fs::create_dir_all(e.root.join(".agent-link")).unwrap();
    fs::write(e.root.join(".agent-link/settings.json"), r#"{"keep": 1}"#).unwrap();
    install_in(&e.ext_dir, &hello_src(), false).unwrap();

    let report = enable_in(&e.ext_dir, &e.root, "hello", EXE).unwrap();
    assert_eq!(report.init, "ran");
    assert!(e.root.join(".agent-link/blocks/hello-agent.md").is_file());
    assert!(e.root.join(".agent-link/template/hello-rules.md").is_file());
    assert!(
        e.root.join(".agent-link/ext/hello").is_dir(),
        "init must create the data folder"
    );
    assert_eq!(enabled_names(&e.root), vec!["hello".to_string()]);

    // 他のキーは保たれる
    let text = fs::read_to_string(e.root.join(".agent-link/settings.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["keep"], 1);

    // プロジェクトの .sai は作らない
    assert!(!e.root.join(".sai").exists());

    // 一覧にプロジェクト名が出る
    let list = list_in(&e.ext_dir, Some(&e.registry));
    assert_eq!(list.extensions[0].enabled_in, vec!["proj".to_string()]);

    // 有効な拡張は置き場から消せない
    let err = uninstall_in(&e.ext_dir, "hello", Some(&e.registry)).unwrap_err();
    assert!(matches!(err, ExtError::Conflict(_)), "{err:?}");
    assert!(e.ext_dir.join("hello").exists());

    // もう一度有効化しても記録は 1 つのまま
    enable_in(&e.ext_dir, &e.root, "hello", EXE).unwrap();
    assert_eq!(enabled_names(&e.root), vec!["hello".to_string()]);
}

#[test]
fn enable_failed_init_does_not_record() {
    let e = env();
    let dir = e.ext_dir.join("broken");
    make_ext(
        &dir,
        r#"{"name": "broken", "version": "1", "cli": {"bin": "bin/b"}, "init": ["init"]}"#,
        &[("bin/b", "#!/bin/sh\necho boom >&2\nexit 3\n")],
    );
    let err = enable_in(&e.ext_dir, &e.root, "broken", EXE).unwrap_err();
    assert!(matches!(err, ExtError::Other(_)), "{err:?}");
    assert!(err.message().contains("boom"), "{err}");
    assert!(enabled_names(&e.root).is_empty());
}

#[test]
fn disable_removes_untouched_and_keeps_edited() {
    let e = env();
    install_in(&e.ext_dir, &hello_src(), false).unwrap();
    enable_in(&e.ext_dir, &e.root, "hello", EXE).unwrap();

    // template だけ利用者が直す
    let template = e.root.join(".agent-link/template/hello-rules.md");
    fs::write(&template, "利用者が直した規則\n").unwrap();

    let report = disable_in(&e.ext_dir, &e.root, "hello").unwrap();
    assert!(!report.enabled);
    assert!(!e.root.join(".agent-link/blocks/hello-agent.md").exists());
    assert_eq!(
        report.removed,
        vec![e.root.join(".agent-link/blocks/hello-agent.md")]
    );
    assert!(template.exists());
    assert_eq!(report.kept, vec![template]);
    assert!(enabled_names(&e.root).is_empty());
}

#[test]
fn requires_is_checked_both_ways() {
    let e = env();
    plain_ext(&e.ext_dir.join("base"), "base", "");
    plain_ext(
        &e.ext_dir.join("addon"),
        "addon",
        r#", "requires": ["base"]"#,
    );

    let err = enable_in(&e.ext_dir, &e.root, "addon", EXE).unwrap_err();
    assert!(matches!(err, ExtError::Conflict(_)), "{err:?}");

    enable_in(&e.ext_dir, &e.root, "base", EXE).unwrap();
    enable_in(&e.ext_dir, &e.root, "addon", EXE).unwrap();

    let err = disable_in(&e.ext_dir, &e.root, "base").unwrap_err();
    assert!(matches!(err, ExtError::Conflict(_)), "{err:?}");

    disable_in(&e.ext_dir, &e.root, "addon").unwrap();
    disable_in(&e.ext_dir, &e.root, "base").unwrap();
    assert!(enabled_names(&e.root).is_empty());
}

// ---- 更新 ----

#[test]
fn update_moves_edited_block_to_backup() {
    let e = env();
    install_in(&e.ext_dir, &hello_src(), false).unwrap();
    enable_in(&e.ext_dir, &e.root, "hello", EXE).unwrap();

    let block_path = e.root.join(".agent-link/blocks/hello-agent.md");
    fs::write(&block_path, "利用者が直した本文\n").unwrap();

    let reports = update_in(&e.ext_dir, &e.root, Some("hello"), EXE).unwrap();
    let block_report = reports[0]
        .assets
        .iter()
        .find(|a| a.path == block_path)
        .unwrap();
    assert_eq!(block_report.outcome, "updated");
    let backup = block_report.backup.clone().unwrap();
    assert_eq!(fs::read_to_string(backup).unwrap(), "利用者が直した本文\n");
    assert!(fs::read_to_string(&block_path)
        .unwrap()
        .contains("hello greet"));

    let err = update_in(&e.ext_dir, &e.root, Some("other"), EXE).unwrap_err();
    assert!(matches!(err, ExtError::NotFound(_)), "{err:?}");
}

// ---- CLI の起動 ----

#[test]
fn run_cli_returns_output_and_codes() {
    let e = env();
    install_in(&e.ext_dir, &hello_src(), false).unwrap();
    enable_in(&e.ext_dir, &e.root, "hello", EXE).unwrap();

    let args = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let out = run_cli(
        &e.ext_dir,
        &e.root,
        "hello",
        &args(&["greet", "x"]),
        EXE,
        RUN_TIMEOUT,
    )
    .unwrap();
    assert_eq!(out.code, 0);
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value, serde_json::json!({"hello": "x"}));

    let out = run_cli(
        &e.ext_dir,
        &e.root,
        "hello",
        &args(&["fail"]),
        EXE,
        RUN_TIMEOUT,
    )
    .unwrap();
    assert_eq!(out.code, 3);
    assert!(String::from_utf8_lossy(&out.stderr).contains("Something went wrong."));

    // 有効でない project からは呼べない
    let other = e._tmp.path().join("other");
    fs::create_dir_all(&other).unwrap();
    let err = run_cli(
        &e.ext_dir,
        &other,
        "hello",
        &args(&["greet"]),
        EXE,
        RUN_TIMEOUT,
    )
    .unwrap_err();
    assert!(matches!(err, ExtError::Conflict(_)), "{err:?}");

    // PATH 用のフォルダ
    assert_eq!(
        enabled_bin_dirs(&e.ext_dir, &e.root),
        vec![e.ext_dir.join("hello/bin")]
    );
}

#[test]
fn run_cli_times_out() {
    let e = env();
    let dir = e.ext_dir.join("slow");
    make_ext(
        &dir,
        r#"{"name": "slow", "version": "1", "cli": {"bin": "bin/s"}}"#,
        &[("bin/s", "#!/bin/sh\nsleep 5\n")],
    );
    enable_in(&e.ext_dir, &e.root, "slow", EXE).unwrap();
    let err = run_cli(
        &e.ext_dir,
        &e.root,
        "slow",
        &[],
        EXE,
        Duration::from_secs(1),
    )
    .unwrap_err();
    assert!(err.message().contains("timed out after 1 seconds"), "{err}");
}

// ---- 一覧の JSON ----

#[test]
fn list_json_has_contract_keys() {
    let e = env();
    install_in(&e.ext_dir, &hello_src(), false).unwrap();
    let json = list_in(&e.ext_dir, None).to_json(&e.ext_dir);
    let item = &json["extensions"][0];
    assert_eq!(item["name"], "hello");
    assert_eq!(item["title"], "Hello");
    assert_eq!(item["ui"]["scripts"][0], "hello.js");
    assert_eq!(item["ui"]["title"], "Hello");
    assert_eq!(item["cli"]["web"][0], "greet");
    assert!(item["service"].is_null());
    assert!(json["errors"].as_array().unwrap().is_empty());
}
