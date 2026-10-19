//! 拡張の本体（`ext.rs`）の確かめ。すべて一時フォルダで行い、環境変数に頼らない。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_link_engine::ext::{
    call_hook, disable_in, enable_in, enabled_bin_dirs, enabled_names, hook_owner, install_in,
    list_in, load, run_cli, uninstall_in, update_in, ExtError, HookOutcome, HookPoint, RUN_TIMEOUT,
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
    let manifest = format!(r#"{{"name": "{name}", "title": "T", "version": "1.0.0"{extra}}}"#);
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
            r#"{"name": "x", "title": "T", "version": "1", "bogus": 1}"#.to_string(),
            vec![],
            "unknown field",
        ),
        (
            "bad name",
            r#"{"name": "Bad_Name", "title": "T", "version": "1"}"#.to_string(),
            vec![],
            "name 'Bad_Name' is not valid",
        ),
        (
            "parent path in cli.bin",
            r#"{"name": "x", "title": "T", "version": "1", "cli": {"bin": "../x"}}"#.to_string(),
            vec![],
            "cli.bin '../x' must be a relative path",
        ),
        (
            "cli.bin not executable",
            r#"{"name": "x", "title": "T", "version": "1", "cli": {"bin": "bin/x"}}"#.to_string(),
            vec![("bin/x", "#!/bin/sh\n".to_string())],
            "is not an executable file",
        ),
        (
            "block named outside the rule",
            r#"{"name": "x", "title": "T", "version": "1"}"#.to_string(),
            vec![("blocks/other.md", block("other"))],
            "must be named x.md or x-*.md",
        ),
        (
            "block id differs from file name",
            r#"{"name": "x", "title": "T", "version": "1"}"#.to_string(),
            vec![("blocks/x-a.md", block("x-b"))],
            "has id 'x-b', expected 'x-a'",
        ),
        (
            "ui file parent path",
            r#"{"name": "x", "title": "T", "version": "1", "ui": {"scripts": ["../a.js"]}}"#.to_string(),
            vec![],
            "must be a relative path inside ui/",
        ),
        (
            "service env name without prefix",
            r#"{"name": "x", "title": "T", "version": "1", "service": {"url_env": "URL"}}"#.to_string(),
            vec![],
            "must be AGENT_LINK_EXT_",
        ),
        (
            "service post with parent segment",
            r#"{"name": "x", "title": "T", "version": "1", "service": {"url_env": "AGENT_LINK_EXT_URL", "post": ["a/../b"]}}"#
                .to_string(),
            vec![],
            "is not a valid path",
        ),
        (
            "service post with leading slash",
            r#"{"name": "x", "title": "T", "version": "1", "service": {"url_env": "AGENT_LINK_EXT_URL", "post": ["/ok"]}}"#
                .to_string(),
            vec![],
            "is not a valid path",
        ),
        (
            "init without cli",
            r#"{"name": "x", "title": "T", "version": "1", "init": ["init"]}"#.to_string(),
            vec![],
            "init needs cli",
        ),
        (
            "title missing",
            r#"{"name": "x", "version": "1"}"#.to_string(),
            vec![],
            "missing field `title`",
        ),
        (
            "title longer than 40 characters",
            format!(r#"{{"name": "x", "title": "{}"}}"#, "あ".repeat(41)),
            vec![],
            "title must be 1 to 40 characters",
        ),
        (
            "title empty",
            r#"{"name": "x", "title": "  "}"#.to_string(),
            vec![],
            "title must be 1 to 40 characters",
        ),
        (
            "reserved name",
            r#"{"name": "kantoku", "title": "T"}"#.to_string(),
            vec![],
            "name 'kantoku' is reserved",
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

#[test]
fn title_is_kept_and_version_defaults_to_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("x");
    fs::create_dir_all(&dir).unwrap();
    // 40 文字ちょうどの title は通り、version を省くと空文字になる
    let title = "あ".repeat(40);
    fs::write(
        dir.join("extension.json"),
        format!(r#"{{"name": "x", "title": "{title}"}}"#),
    )
    .unwrap();
    let info = load(&dir).expect("version may be omitted");
    assert!(info.manifest.version.is_none());
    let item = list_in(tmp.path(), None).to_json(tmp.path());
    assert_eq!(item["extensions"][0]["title"], title.as_str());
    assert_eq!(item["extensions"][0]["version"], "");
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
        r#"{"name": "broken", "title": "T", "version": "1", "cli": {"bin": "bin/b"}, "init": ["init"]}"#,
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
        None,
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
        None,
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
        None,
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
        r#"{"name": "slow", "title": "T", "version": "1", "cli": {"bin": "bin/s"}}"#,
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
        None,
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

// ---- hooks ----

/// `hooks` を宣言した拡張（`hooks_json` は `"hooks": {...}` の中身）。`bin/x` は引数と標準入力を `seen.txt` に書く。
fn hooked_ext(dir: &Path, name: &str, hooks_json: &str) {
    let manifest = format!(
        r#"{{"name": "{name}", "title": "T", "cli": {{"bin": "bin/x"}}, "hooks": {hooks_json}}}"#
    );
    let script = "#!/bin/sh\n\
        { echo \"$@\"; echo \"$AGENT_LINK_PARENT_RUN_ID $AGENT_LINK_BLOCK_NAME $AGENT_LINK_BIN $AGENT_LINK_X\"; cat; } > \"$AGENT_LINK_PROJECT_ROOT/seen-$(basename \"$0\")-$AGENT_LINK_BLOCK_NAME.txt\"\n\
        echo ctx-out\n";
    make_ext(dir, &manifest, &[("bin/x", script)]);
}

#[test]
fn hooks_declaration_is_checked() {
    let tmp = tempfile::tempdir().unwrap();
    let cases = [
        (
            r#"{"name": "x", "title": "T", "cli": {"bin": "bin/x"}, "hooks": {"context": ["c"], "write_check": ["w", "{filename}"], "question": ["q"]}}"#,
            None,
        ),
        (
            r#"{"name": "x", "title": "T", "cli": {"bin": "bin/x"}, "hooks": {}}"#,
            None,
        ),
        (
            r#"{"name": "x", "title": "T", "hooks": {"context": ["c"]}}"#,
            Some("hooks needs cli"),
        ),
        (
            r#"{"name": "x", "title": "T", "cli": {"bin": "bin/x"}, "hooks": {"context": []}}"#,
            Some("hooks.context must not be empty"),
        ),
        (
            r#"{"name": "x", "title": "T", "cli": {"bin": "bin/x"}, "hooks": {"question": []}}"#,
            Some("hooks.question must not be empty"),
        ),
        (
            r#"{"name": "x", "title": "T", "cli": {"bin": "bin/x"}, "hooks": {"pre_run": ["p"]}}"#,
            Some("unknown field"),
        ),
    ];
    for (i, (manifest, want)) in cases.iter().enumerate() {
        let dir = tmp.path().join(format!("c{i}/x"));
        make_ext(&dir, manifest, &[("bin/x", "#!/bin/sh\n")]);
        match (load(&dir), want) {
            (Ok(_), None) => {}
            (Err(e), Some(w)) => assert!(e.starts_with("extension.json: ") && e.contains(w), "{e}"),
            (got, want) => panic!("case {i}: {got:?} vs {want:?}"),
        }
    }
}

#[test]
fn list_json_has_hooks() {
    let e = env();
    hooked_ext(&e.ext_dir.join("hooked"), "hooked", r#"{"context": ["c", "{run_id}"]}"#);
    install_in(&e.ext_dir, &hello_src(), false).unwrap();
    let json = list_in(&e.ext_dir, None).to_json(&e.ext_dir);
    let items = json["extensions"].as_array().unwrap();
    let hooked = items.iter().find(|i| i["name"] == "hooked").unwrap();
    assert_eq!(
        hooked["hooks"],
        serde_json::json!({"context": ["c", "{run_id}"], "write_check": null, "question": null})
    );
    let hello = items.iter().find(|i| i["name"] == "hello").unwrap();
    assert!(hello["hooks"].is_null());
}

// ---- config ----

#[test]
fn config_declaration_is_checked() {
    let tmp = tempfile::tempdir().unwrap();
    let names33 = (0..33)
        .map(|i| format!("\"AGENT_LINK_EXT_K{i}\""))
        .collect::<Vec<_>>()
        .join(",");
    let names32 = (0..32)
        .map(|i| format!("\"AGENT_LINK_EXT_K{i}\""))
        .collect::<Vec<_>>()
        .join(",");
    let cases = [
        (r#"{"name": "x", "title": "T", "config": ["AGENT_LINK_EXT_X_KEY", "AGENT_LINK_EXT_X_DIR"]}"#.to_string(), None),
        (r#"{"name": "x", "title": "T", "config": []}"#.to_string(), None),
        (format!(r#"{{"name": "x", "title": "T", "config": [{names32}]}}"#), None),
        (r#"{"name": "x", "title": "T", "config": ["X_KEY"]}"#.to_string(), Some("must be AGENT_LINK_EXT_")),
        (r#"{"name": "x", "title": "T", "config": ["AGENT_LINK_EXT_lower"]}"#.to_string(), Some("must be AGENT_LINK_EXT_")),
        (r#"{"name": "x", "title": "T", "config": ["AGENT_LINK_EXT_"]}"#.to_string(), Some("must be AGENT_LINK_EXT_")),
        (
            r#"{"name": "x", "title": "T", "config": ["AGENT_LINK_EXT_A", "AGENT_LINK_EXT_A"]}"#.to_string(),
            Some("config 'AGENT_LINK_EXT_A' is listed twice"),
        ),
        (
            format!(r#"{{"name": "x", "title": "T", "config": [{names33}]}}"#),
            Some("config must have at most 32 names"),
        ),
    ];
    for (i, (manifest, want)) in cases.iter().enumerate() {
        let dir = tmp.path().join(format!("c{i}/x"));
        make_ext(&dir, manifest, &[]);
        match (load(&dir), want) {
            (Ok(_), None) => {}
            (Err(e), Some(w)) => assert!(e.starts_with("extension.json: ") && e.contains(w), "{e}"),
            (got, want) => panic!("case {i}: {got:?} vs {want:?}"),
        }
    }
}

#[test]
fn list_json_has_config_names_without_values() {
    let e = env();
    make_ext(
        &e.ext_dir.join("cfg"),
        r#"{"name": "cfg", "title": "T", "config": ["AGENT_LINK_EXT_CFGTEST_UNSET_ZZ"]}"#,
        &[],
    );
    install_in(&e.ext_dir, &hello_src(), false).unwrap();
    let json = list_in(&e.ext_dir, None).to_json(&e.ext_dir);
    let items = json["extensions"].as_array().unwrap();
    let cfg = items.iter().find(|i| i["name"] == "cfg").unwrap();
    assert_eq!(
        cfg["config"],
        serde_json::json!([{"name": "AGENT_LINK_EXT_CFGTEST_UNSET_ZZ", "set": false}])
    );
    let hello = items.iter().find(|i| i["name"] == "hello").unwrap();
    assert_eq!(hello["config"], serde_json::json!([]));
}

#[test]
fn config_values_follow_declaration_order_and_missing_is_not_found() {
    let e = env();
    make_ext(
        &e.ext_dir.join("cfg"),
        r#"{"name": "cfg", "title": "T", "config": ["AGENT_LINK_EXT_CFGTEST_B_ZZ", "AGENT_LINK_EXT_CFGTEST_A_ZZ"]}"#,
        &[],
    );
    let values = agent_link_engine::ext::config_values(&e.ext_dir, "cfg").unwrap();
    assert_eq!(
        values,
        vec![
            ("AGENT_LINK_EXT_CFGTEST_B_ZZ".to_string(), None),
            ("AGENT_LINK_EXT_CFGTEST_A_ZZ".to_string(), None),
        ]
    );
    assert!(matches!(
        agent_link_engine::ext::config_values(&e.ext_dir, "nosuch"),
        Err(ExtError::NotFound(_))
    ));
}

#[test]
fn hook_owner_selection() {
    let e = env();
    hooked_ext(&e.ext_dir.join("hello"), "hello", r#"{"context": ["c"], "question": ["q"]}"#);
    hooked_ext(&e.ext_dir.join("hello-x"), "hello-x", r#"{"context": ["c"]}"#);
    let name = |block: &str, point| {
        hook_owner(&e.ext_dir, &e.root, block, point).map(|i| i.manifest.name)
    };

    // 有効でなければ持ち主は無い
    assert_eq!(name("hello-agent", HookPoint::Context), None);

    enable_in(&e.ext_dir, &e.root, "hello", EXE).unwrap();
    assert_eq!(name("hello-agent", HookPoint::Context).as_deref(), Some("hello"));
    assert_eq!(name("hello", HookPoint::Context).as_deref(), Some("hello"));
    assert_eq!(name("plan", HookPoint::Context), None);
    // 名前の続きが '-' でなければ当てはまらない
    assert_eq!(name("helloagent", HookPoint::Context), None);
    // hello-x は有効でない
    assert_eq!(name("hello-x-foo", HookPoint::Context).as_deref(), Some("hello"));

    // 両方有効なら長い名前が勝つ。その点を宣言していなければ短い方
    enable_in(&e.ext_dir, &e.root, "hello-x", EXE).unwrap();
    assert_eq!(name("hello-x-foo", HookPoint::Context).as_deref(), Some("hello-x"));
    assert_eq!(name("hello-x-foo", HookPoint::Question).as_deref(), Some("hello"));
    assert_eq!(name("hello-x-foo", HookPoint::WriteCheck), None);
}

#[test]
fn call_hook_expands_args_and_env() {
    let e = env();
    hooked_ext(
        &e.ext_dir.join("hello"),
        "hello",
        r#"{"context": ["context", "--run", "{run_id}", "{block}", "[{filename}]"]}"#,
    );
    enable_in(&e.ext_dir, &e.root, "hello", EXE).unwrap();

    let out = call_hook(
        &e.ext_dir,
        &e.root,
        HookPoint::Context,
        "hello-agent",
        42,
        None,
        &[("AGENT_LINK_X", "extra".to_string())],
        Some("stdin body\n"),
        EXE,
        Duration::from_secs(10),
    );
    match out {
        HookOutcome::Done { ext, code, stdout, .. } => {
            assert_eq!((ext.as_str(), code, stdout.trim()), ("hello", 0, "ctx-out"));
        }
        other => panic!("{other:?}"),
    }
    let seen = fs::read_to_string(e.root.join("seen-x-hello-agent.txt")).unwrap();
    assert_eq!(
        seen,
        format!("context --run 42 hello-agent []\n42 hello-agent {EXE} extra\nstdin body\n")
    );

    let none = call_hook(
        &e.ext_dir, &e.root, HookPoint::Context, "plan", 42, None, &[], None, EXE,
        Duration::from_secs(10),
    );
    assert_eq!(none, HookOutcome::NoOwner);
}

#[test]
fn call_hook_reports_failure_and_timeout() {
    let e = env();
    let dir = e.ext_dir.join("hello");
    make_ext(
        &dir,
        r#"{"name": "hello", "title": "T", "cli": {"bin": "bin/x"}, "hooks": {"context": ["c"]}}"#,
        &[("bin/x", "#!/bin/sh\ncase \"$1\" in c) echo one >&2; echo bad >&2; exit 4;; esac\n")],
    );
    enable_in(&e.ext_dir, &e.root, "hello", EXE).unwrap();
    let out = call_hook(
        &e.ext_dir, &e.root, HookPoint::Context, "hello", 1, None, &[], None, EXE,
        Duration::from_secs(10),
    );
    match out {
        HookOutcome::Done { code, stderr, stderr_last, .. } => {
            assert_eq!((code, stderr.as_str(), stderr_last.as_str()), (4, "one\nbad", "bad"));
        }
        other => panic!("{other:?}"),
    }

    make_ext(
        &dir,
        r#"{"name": "hello", "title": "T", "cli": {"bin": "bin/x"}, "hooks": {"context": ["c"]}}"#,
        &[("bin/x", "#!/bin/sh\nsleep 5\n")],
    );
    let out = call_hook(
        &e.ext_dir, &e.root, HookPoint::Context, "hello", 1, None, &[], None, EXE,
        Duration::from_secs(1),
    );
    match out {
        HookOutcome::Broken { ext, reason } => {
            assert!(ext == "hello" && reason.contains("timed out"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
}
