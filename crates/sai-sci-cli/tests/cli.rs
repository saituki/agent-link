//! `sai-sci` の CLI の約束と、`init`・`find`・`search` の試験。
//!
//! 子の実行ファイル（sai-link・sai-paper）は偽の sh を `SAI_LINK_BIN`・
//! `SAI_PAPER_BIN` で渡す。環境変数は子のプロセスにだけ渡し、同じ試験の中で戻す。

use std::path::{Path, PathBuf};
use std::process::Command;

use sai_sci_core::rt::{Kind, defs, store};

/// 試験の中だけで使う一時フォルダ（並列でもぶつからないよう名前を分ける）。
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sai-sci-cli-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos())
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `sai-sci` を呼ぶための基本の Command（親の環境の研究用の変数は外す）。
fn command(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sai-sci"));
    cmd.current_dir(dir)
        .env_remove("SAI_PROJECT_ROOT")
        .env_remove("SAI_PARENT_RUN_ID")
        .env_remove("SAI_BLOCK_NAME")
        .env_remove("SAI_TARGET_NODE")
        .env_remove("SAI_ORIGIN");
    cmd
}

/// 実行の結果。
struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

/// 実行して結果をまとめる。
fn run(mut cmd: Command) -> Out {
    let out = cmd.output().expect("sai-sci を起動できる");
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    }
}

/// 標準入力に JSON を流して実行する（`op` は標準入力を読む）。
fn run_with_stdin(mut cmd: Command, input: &str) -> Out {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sai-sci を起動できる");
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    }
}

/// sh のスクリプトを置いて実行の権限を付ける。
fn write_script(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// 偽の `sai-link`。
///
/// - 呼ばれた引数を `ARGS_LOG` に足す。
/// - `run list --json` は空の run の一覧を出す。
/// - `card create` は標準入力の JSON を残し、todo に task.md を作って
///   `{"run_id","path","executer"}` を出す。
/// - `init` では settings.json を無ければ作る。
fn fake_link(dir: &Path) -> PathBuf {
    let path = dir.join("fake-link.sh");
    write_script(
        &path,
        "#!/bin/sh\n\
         echo \"link $@\" >> \"$ARGS_LOG\"\n\
         case \"$1 $2\" in\n\
           \"run list\")\n\
             echo '{\"runs\":[]}'\n\
             exit 0\n\
             ;;\n\
           \"card create\")\n\
             cat > \"${FAKE_CARD:-/dev/null}\"\n\
             mkdir -p .sai/outputs/todo/1\n\
             printf '# task_text\\n' > .sai/outputs/todo/1/task.md\n\
             echo \"{\\\"run_id\\\":1,\\\"path\\\":\\\"$PWD/.sai/outputs/todo/1/task.md\\\",\\\"executer\\\":\\\"agent\\\"}\"\n\
             exit 0\n\
             ;;\n\
         esac\n\
         [ -f .sai/settings.json ] || { mkdir -p .sai; echo '{}' > .sai/settings.json; }\n\
         echo \"link done\"\n",
    );
    path
}

/// 偽の `sai-paper`（呼ばれた引数を `ARGS_LOG` に足し、決まった JSON を出す）。
fn fake_paper(dir: &Path) -> PathBuf {
    let path = dir.join("fake-paper.sh");
    write_script(
        &path,
        "#!/bin/sh\n\
         echo \"paper $@\" >> \"$ARGS_LOG\"\n\
         case \"$1\" in\n\
           find) printf '%s' \"{\\\"log\\\":{\\\"id\\\":$FAKE_FIND_LOG_ID,\\\"node\\\":null},\\\"hits\\\":[{\\\"paper_id\\\":3,\\\"title\\\":\\\"T\\\",\\\"text\\\":\\\"abc\\\",\\\"cite\\\":\\\"#3 §1\\\"}]}\" ;;\n\
           search) printf '%s' \"{\\\"log\\\":{\\\"id\\\":$FAKE_SEARCH_LOG_ID,\\\"source_dbs\\\":\\\"s2\\\"},\\\"items\\\":[{\\\"title\\\":\\\"P\\\",\\\"year\\\":\\\"2024\\\"}],\\\"failed\\\":[]}\" ;;\n\
         esac\n",
    );
    path
}

/// 約束: `.sai/` の無い所では 2・標準出力は空。知らない命令は 2 で最後の行が `引数の誤り:`。
#[test]
fn without_project_returns_two() {
    let home = temp_dir("noproj");
    let out = run({
        let mut cmd = command(&home);
        cmd.env("HOME", &home).args(["status", "--json"]);
        cmd
    });
    assert_eq!(out.code, 2, "stdout={} stderr={}", out.stdout, out.stderr);
    assert!(out.stdout.is_empty(), "失敗の標準出力が空でない: {}", out.stdout);
    assert!(
        out.stderr.lines().last().unwrap_or("").contains("プロジェクトが見つかりません"),
        "stderr={}",
        out.stderr
    );

    let unknown = run({
        let mut cmd = command(&home);
        cmd.env("HOME", &home).args(["frobnicate"]);
        cmd
    });
    assert_eq!(unknown.code, 2);
    assert!(
        unknown.stderr.lines().last().unwrap_or("").starts_with("引数の誤り:"),
        "stderr={}",
        unknown.stderr
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// 約束: research-tree のあるプロジェクトで `node Q999 --json` は 3。
#[test]
fn unknown_node_returns_three() {
    let root = temp_dir("node3");
    let home = temp_dir("node3-home");
    let env = project_with_tree(&root);
    let _ = env;

    let out = run({
        let mut cmd = command(&root);
        cmd.env("HOME", &home)
            .env("SAI_PROJECT_ROOT", &root)
            .args(["node", "Q999", "--json"]);
        cmd
    });
    assert_eq!(out.code, 3, "stdout={} stderr={}", out.stdout, out.stderr);
    assert!(out.stdout.is_empty(), "失敗の標準出力が空でない: {}", out.stdout);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
}

/// 一時の研究プロジェクトに、新しい research-tree（Q1）と DB を作る。
fn project_with_tree(root: &Path) -> (PathBuf, i64) {
    let data_dir = root.join(".sai").join("paper");
    std::fs::create_dir_all(&data_dir).unwrap();
    let db = sai_paper_core::Database::open(&data_dir.join("papr.db")).unwrap();
    let defs = defs::load(root);
    defs::init_tree(db.connection(), &defs, "problem", "user").unwrap();
    let q: i64 = db
        .connection()
        .query_row("SELECT id FROM rt_nodes WHERE kind = 'Q' AND parent_id IS NULL", [], |r| {
            r.get(0)
        })
        .unwrap();
    (data_dir, q)
}

/// `Q1` の下に論文 3 の F を足す。
fn add_f_with_paper(data_dir: &Path, q: i64) {
    let db = sai_paper_core::Database::open(&data_dir.join("papr.db")).unwrap();
    store::insert_node(
        db.connection(),
        &store::NewNode {
            kind: Kind::F,
            parent_id: Some(q),
            guide_key: None,
            text: "根拠",
            body: "",
            stance: Some("support"),
            cites: &[],
            paper_id: Some(3),
            run_id: None,
            report_id: None,
            mlflow_ref: None,
            paths: &[],
            created_by: "user",
        },
    )
    .unwrap();
}

/// `schema --json` の形と `web: false` の命令。
#[test]
fn schema_has_shape_and_web_flags() {
    let dir = temp_dir("schema");
    let out = run({
        let mut cmd = command(&dir);
        cmd.env("HOME", &dir).args(["schema", "--json"]);
        cmd
    });
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let value: serde_json::Value = serde_json::from_str(&out.stdout).expect("JSON を読める");
    assert_eq!(value["bin"], "sai-sci");
    let commands = value["commands"].as_array().unwrap();
    let find = |path: &[&str]| {
        commands.iter().find(|cmd| {
            cmd["path"].as_array().is_some_and(|words| {
                words.iter().map(|w| w.as_str().unwrap_or("")).collect::<Vec<_>>() == path
            })
        })
    };
    for path in [&["init"][..], &["schema"][..], &["context"][..], &["op"][..], &["report", "prepare"][..]] {
        assert_eq!(find(path).unwrap_or_else(|| panic!("{path:?}"))["web"], false, "{path:?}");
    }
    assert_eq!(find(&["card"]).unwrap()["stdin"], "json");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Web は `apply --json` で呼ぶ（`for-sai-developers.md` の約束）。`--json` を引数の誤りにしない。
#[test]
fn apply_accepts_json_flag() {
    let dir = temp_dir("apply-json");
    let out = run({
        let mut cmd = command(&dir);
        cmd.env("HOME", &dir).args(["apply", "--json"]);
        cmd
    });
    assert!(!out.stderr.contains("unexpected argument"), "stderr={}", out.stderr);
    assert!(out.stdout.is_empty(), "失敗のとき標準出力は空: {}", out.stdout);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `init` は link → paper の順に呼び、4 つの設定と部品を配る。
/// 2 回目は変えず、利用者の `pre_run` は残し、`--update` で退避して書き換える。
#[test]
fn init_installs_parts_and_settings() {
    let root = temp_dir("init-root");
    let home = temp_dir("init-home");
    let tmp = temp_dir("init-tmp");
    let link = fake_link(&tmp);
    let paper = fake_paper(&tmp);
    let args_log = tmp.join("args.log");

    let init = |update: bool, extra: &[&str]| {
        let mut cmd = command(&root);
        cmd.env("HOME", &home)
            .env("SAI_HOME", &home)
            .env("SAI_PROJECT_ROOT", &root)
            .env("SAI_LINK_BIN", &link)
            .env("SAI_PAPER_BIN", &paper)
            .env("ARGS_LOG", &args_log)
            .env("FAKE_FIND_LOG_ID", "1")
            .env("FAKE_SEARCH_LOG_ID", "2");
        cmd.arg("init");
        if update {
            cmd.arg("--update");
        }
        cmd.args(extra);
        cmd
    };

    // 1 回目。
    let first = run(init(false, &["--domain", "medical"]));
    assert_eq!(first.code, 0, "stderr={}", first.stderr);
    let settings = std::fs::read_to_string(root.join(".sai/settings.json")).unwrap();
    let settings: serde_json::Value = serde_json::from_str(&settings).unwrap();
    for key in ["pre_run", "context", "write_check", "job_status"] {
        assert!(settings.get(key).is_some(), "{key} が無い: {settings}");
    }
    assert_eq!(settings["context"][1], "context");
    assert_eq!(settings["job_status"][0], "uv");
    // 部品。
    let blocks = std::fs::read_dir(root.join(".sai/blocks")).unwrap().count();
    assert!(blocks >= 31, "block が 31 本未満: {blocks}");
    assert!(root.join(".sai/template/debate-participant.md").is_file());
    assert!(root.join(".sai/template/domain.md").is_file());
    assert!(root.join(".sai/paper/mind.md").is_file());
    assert!(root.join(".sai/paper/tree.md").exists() == false, "research-tree はまだ無い");

    // 呼ぶ順は link → paper。
    let log = std::fs::read_to_string(&args_log).unwrap();
    let link_at = log.find("link init").expect("link init が呼ばれた");
    let paper_at = log.find("paper init").expect("paper init が呼ばれた");
    assert!(link_at < paper_at, "link → paper の順でない: {log}");

    // 2 回目は何も変わらない。
    let second = run(init(false, &[]));
    assert_eq!(second.code, 0, "stderr={}", second.stderr);
    let again = std::fs::read_to_string(root.join(".sai/settings.json")).unwrap();
    assert_eq!(settings.to_string(), serde_json::from_str::<serde_json::Value>(&again).unwrap().to_string());
    assert!(!root.join(".sai/settings.json.bak").exists(), "変化なしで .bak を作った");

    // 利用者が直した pre_run は、--update なしで残る。
    let custom = r#"{"pre_run": ["mine", "--x"], "context": ["c"], "write_check": ["w"], "job_status": ["j"]}"#;
    std::fs::write(root.join(".sai/settings.json"), custom).unwrap();
    let kept = run(init(false, &[]));
    assert_eq!(kept.code, 0, "stderr={}", kept.stderr);
    assert!(kept.stdout.contains("pre_run は今の値を残しました"), "stdout={}", kept.stdout);
    let after = std::fs::read_to_string(root.join(".sai/settings.json")).unwrap();
    assert!(after.contains("\"mine\""), "利用者の値を消した: {after}");
    assert!(!root.join(".sai/settings.json.bak").exists());

    // --update で .bak ができて書き換わる。
    let updated = run(init(true, &[]));
    assert_eq!(updated.code, 0, "stderr={}", updated.stderr);
    assert!(root.join(".sai/settings.json.bak").is_file(), ".bak が無い");
    let after = std::fs::read_to_string(root.join(".sai/settings.json")).unwrap();
    assert!(!after.contains("\"mine\""), "--update で書き換わっていない: {after}");

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `init --type problem` で research-tree ができ、もう一度 `--type` を付けると 4。
#[test]
fn init_with_type_builds_tree_and_conflicts() {
    let root = temp_dir("init-type-root");
    let home = temp_dir("init-type-home");
    let tmp = temp_dir("init-type-tmp");
    let link = fake_link(&tmp);
    let paper = fake_paper(&tmp);
    let args_log = tmp.join("args.log");

    let init = || {
        let mut cmd = command(&root);
        cmd.env("HOME", &home)
            .env("SAI_HOME", &home)
            .env("SAI_PROJECT_ROOT", &root)
            .env("SAI_LINK_BIN", &link)
            .env("SAI_PAPER_BIN", &paper)
            .env("ARGS_LOG", &args_log)
            .args(["init", "--type", "problem"]);
        cmd
    };

    let first = run(init());
    assert_eq!(first.code, 0, "stdout={} stderr={}", first.stdout, first.stderr);
    assert!(root.join(".sai/paper/tree.md").is_file(), "tree.md が無い");

    let second = run(init());
    assert_eq!(second.code, 4, "stdout={} stderr={}", second.stdout, second.stderr);
    assert!(second.stdout.is_empty(), "失敗の標準出力が空でない: {}", second.stdout);

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `find` は子を呼び、`rt_searches` に 1 行残し、`log.node` を Node にする。
/// Node の下に論文が無いときは子を呼ばない。
#[test]
fn find_records_and_sets_node() {
    let root = temp_dir("find-root");
    let home = temp_dir("find-home");
    let tmp = temp_dir("find-tmp");
    let paper = fake_paper(&tmp);
    let args_log = tmp.join("args.log");

    let (data_dir, q) = project_with_tree(&root);

    let find = |node: &str| {
        let mut cmd = command(&root);
        cmd.env("HOME", &home)
            .env("SAI_PROJECT_ROOT", &root)
            .env("SAI_PAPER_BIN", &paper)
            .env("ARGS_LOG", &args_log)
            .env("FAKE_FIND_LOG_ID", "7")
            .env("FAKE_SEARCH_LOG_ID", "8")
            .env("SAI_TARGET_NODE", node)
            .args(["find", "発作", "--node", node, "--json"]);
        cmd
    };

    // 論文が無いときは子を呼ばず、0 件で記録する。
    let empty = run(find("Q1"));
    assert_eq!(empty.code, 0, "stderr={}", empty.stderr);
    let empty_value: serde_json::Value = serde_json::from_str(&empty.stdout).expect("JSON を読める");
    assert!(empty_value["log"].is_null());
    assert_eq!(empty_value["hits"].as_array().map(Vec::len), Some(0));
    assert!(!std::fs::read_to_string(&args_log).unwrap_or_default().contains("paper find"));

    // 論文を足すと、子を呼び、記録して Node を返す。
    add_f_with_paper(&data_dir, q);
    let out = run(find("Q1"));
    assert_eq!(out.code, 0, "stdout={} stderr={}", out.stdout, out.stderr);
    let value: serde_json::Value = serde_json::from_str(&out.stdout).expect("JSON を読める");
    assert_eq!(value["log"]["node"]["id"], "Q1");
    assert_eq!(value["hits"][0]["paper_id"], 3);

    let db = sai_paper_core::Database::open(&data_dir.join("papr.db")).unwrap();
    let rows: Vec<(String, i64, Option<i64>)> = {
        let mut stmt = db
            .connection()
            .prepare("SELECT tool, hits, log_id FROM rt_searches ORDER BY id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        rows
    };
    assert_eq!(rows.len(), 2, "0 件と 1 件の 2 行が残る: {rows:?}");
    assert_eq!(rows[1].0, "find");
    assert_eq!(rows[1].1, 1);
    assert_eq!(rows[1].2, Some(7));
    drop(db);

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `search` は子を呼び、検索履歴に研究の欄を書き、`log.node` を Node にする。
#[test]
fn search_records_and_sets_node() {
    let root = temp_dir("search-root");
    let home = temp_dir("search-home");
    let tmp = temp_dir("search-tmp");
    let paper = fake_paper(&tmp);
    let args_log = tmp.join("args.log");

    let (data_dir, _q) = project_with_tree(&root);
    // 子が返す log.id の行を先に作る（本物の子は自分で作る）。
    let db = sai_paper_core::Database::open(&data_dir.join("papr.db")).unwrap();
    let log_id = sai_paper_core::search_history::record(
        db.connection(),
        "cli",
        "human",
        None,
        "発作",
        "",
        "10",
        "s2",
        &[],
    )
    .unwrap();
    drop(db);

    let out = run({
        let mut cmd = command(&root);
        cmd.env("HOME", &home)
            .env("SAI_PROJECT_ROOT", &root)
            .env("SAI_PAPER_BIN", &paper)
            .env("ARGS_LOG", &args_log)
            .env("FAKE_FIND_LOG_ID", "7")
            .env("FAKE_SEARCH_LOG_ID", log_id.to_string())
            .env("SAI_TARGET_NODE", "Q1")
            .args(["search", "発作", "--purpose", "候補", "--json"]);
        cmd
    });
    assert_eq!(out.code, 0, "stdout={} stderr={}", out.stdout, out.stderr);
    let value: serde_json::Value = serde_json::from_str(&out.stdout).expect("JSON を読める");
    assert_eq!(value["log"]["node"]["id"], "Q1");

    let db = sai_paper_core::Database::open(&data_dir.join("papr.db")).unwrap();
    let purpose: Option<String> = db
        .connection()
        .query_row("SELECT purpose FROM search_history WHERE id = ?1", [log_id], |r| r.get(0))
        .unwrap();
    assert_eq!(purpose.as_deref(), Some("候補"));
    let searches: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM rt_searches WHERE tool = 'search'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(searches, 1);
    drop(db);

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `find --node` だけでは記録しない。存在しない `SAI_TARGET_NODE` は警告だけで検索は成功する。
/// `search --node` は引数の誤り（終了コード 2）。
#[test]
fn node_flag_scopes_only_and_env_records() {
    let root = temp_dir("scope-root");
    let home = temp_dir("scope-home");
    let tmp = temp_dir("scope-tmp");
    let paper = fake_paper(&tmp);
    let (data_dir, q) = project_with_tree(&root);
    add_f_with_paper(&data_dir, q);

    let find = |env_node: Option<&str>| {
        let mut cmd = command(&root);
        cmd.env("HOME", &home)
            .env("SAI_PROJECT_ROOT", &root)
            .env("SAI_PAPER_BIN", &paper)
            .env("ARGS_LOG", tmp.join("args.log"))
            .env("FAKE_FIND_LOG_ID", "7")
            .args(["find", "発作", "--node", "Q1", "--json"]);
        if let Some(node) = env_node {
            cmd.env("SAI_TARGET_NODE", node);
        }
        run(cmd)
    };
    let count = || -> i64 {
        let db = sai_paper_core::Database::open(&data_dir.join("papr.db")).unwrap();
        db.connection()
            .query_row("SELECT COUNT(*) FROM rt_searches", [], |r| r.get(0))
            .unwrap()
    };

    let only_flag = find(None);
    assert_eq!(only_flag.code, 0, "stderr={}", only_flag.stderr);
    assert_eq!(count(), 0, "--node だけでは記録しない");

    let bad_env = find(Some("Q99"));
    assert_eq!(bad_env.code, 0, "stderr={}", bad_env.stderr);
    assert_eq!(bad_env.stderr.lines().filter(|l| l.contains("SAI_TARGET_NODE")).count(), 1);
    assert_eq!(count(), 0);

    let mut cmd = command(&root);
    cmd.env("HOME", &home)
        .env("SAI_PROJECT_ROOT", &root)
        .args(["search", "発作", "--node", "Q1"]);
    assert_eq!(run(cmd).code, 2);

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// 対応表の `--json` の出力が、古い命令と同じ形であること。
#[test]
fn json_outputs_have_expected_shape() {
    let root = temp_dir("json-root");
    let home = temp_dir("json-home");
    let tmp = temp_dir("json-tmp");
    let link = fake_link(&tmp);
    let paper = fake_paper(&tmp);
    let args_log = tmp.join("args.log");

    let run_args = |args: &[&str]| {
        let mut cmd = command(&root);
        cmd.env("HOME", &home)
            .env("SAI_HOME", &home)
            .env("SAI_PROJECT_ROOT", &root)
            .env("SAI_LINK_BIN", &link)
            .env("SAI_PAPER_BIN", &paper)
            .env("ARGS_LOG", &args_log)
            .env("FAKE_FIND_LOG_ID", "7")
            .env("FAKE_SEARCH_LOG_ID", "8")
            .args(args);
        run(cmd)
    };

    let init = run_args(&["init", "--type", "problem"]);
    assert_eq!(init.code, 0, "stderr={}", init.stderr);

    // status --json は initialized を持つ。
    let status = run_args(&["status", "--json"]);
    assert_eq!(status.code, 0, "stderr={}", status.stderr);
    let status: serde_json::Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(status["initialized"], true);

    // types --json は types の配列。
    let types = run_args(&["types", "--json"]);
    let types: serde_json::Value = serde_json::from_str(&types.stdout).unwrap();
    assert!(types["types"].as_array().is_some_and(|a| !a.is_empty()));

    // node --json は node を持つ。
    let node = run_args(&["node", "Q1", "--json"]);
    assert_eq!(node.code, 0, "stderr={}", node.stderr);
    let node: serde_json::Value = serde_json::from_str(&node.stdout).unwrap();
    assert!(node.get("node").is_some(), "{node}");

    // project show --json は 4 つのキー。
    let project = run_args(&["project", "show", "--json"]);
    assert_eq!(project.code, 0, "stderr={}", project.stderr);
    let project: serde_json::Value = serde_json::from_str(&project.stdout).unwrap();
    for key in ["state", "mind", "brief", "attention"] {
        assert!(project.get(key).is_some(), "{key}: {project}");
    }

    // attention list --json は配列。
    let attention = run_args(&["attention", "list", "--json"]);
    assert_eq!(attention.code, 0, "stderr={}", attention.stderr);
    assert!(serde_json::from_str::<serde_json::Value>(&attention.stdout).unwrap().is_array());

    // report content --json は id・title・text。
    let md = root.join("report.md");
    std::fs::write(&md, "# 題\n\n本文です。\n").unwrap();
    let db = sai_paper_core::Database::open(&root.join(".sai/paper/papr.db")).unwrap();
    let report_id = sai_sci_core::reports::create_report(
        db.connection(),
        &sai_sci_core::NewReport {
            title: None,
            md_path: md.to_string_lossy().to_string(),
            run_id: None,
            block: Some("sci-split".to_string()),
            sources: vec![],
            summary: None,
        },
    )
    .unwrap();
    drop(db);

    let content = run_args(&["report", "content", report_id.as_str(), "--json"]);
    assert_eq!(content.code, 0, "stderr={}", content.stderr);
    let content: serde_json::Value = serde_json::from_str(&content.stdout).unwrap();
    assert_eq!(content["id"], report_id);
    assert!(content.get("title").is_some());
    assert!(content["text"].as_str().unwrap_or("").contains("本文です"));

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// 約束: 許可が要る op は、承認（`approval record --answer 許可する`）が 1 件あるときだけ通る。
/// 通ったあと承認は使い切り、rt_log の行に `approved_by`・`qid`・`question` が入る。同じ `qid` の記録は 2 回目が 4。
#[test]
fn approval_record_then_op_add_q() {
    let root = temp_dir("approval");
    let home = temp_dir("approval-home");
    let (data_dir, _) = project_with_tree(&root);
    // 許可を使えるのは sci_ops に書いた block だけ（`--block` で渡す）。
    let blocks = root.join(".sai").join("blocks");
    std::fs::create_dir_all(&blocks).unwrap();
    std::fs::write(blocks.join("t-add.md"), "---\nsci_ops: add_q\n---\n# t\n").unwrap();
    let body = r#"{"ops":[{"op":"add_q","parent":"SEED","text":"新しい根"}]}"#;
    let op = || {
        let mut cmd = command(&root);
        cmd.env("HOME", &home).env("SAI_PROJECT_ROOT", &root).env("SAI_PARENT_RUN_ID", "5").args(["op", "--block", "t-add"]);
        cmd
    };
    let record = |qid: &str, answer: &str| {
        let mut cmd = command(&root);
        cmd.env("HOME", &home).env("SAI_PROJECT_ROOT", &root).args([
            "approval", "record", "--run", "5", "--qid", qid, "--question", "根を足してよいか", "--answer", answer,
            "--answered-by", "saituki",
        ]);
        cmd
    };

    let rec = run(record("a1b2c3d4e5f6", "許可する"));
    assert_eq!(rec.code, 0, "stdout={} stderr={}", rec.stdout, rec.stderr);
    let v: serde_json::Value = serde_json::from_str(&rec.stdout).expect("JSON 1 つ");
    assert_eq!(v["approval"]["consumable"], true);
    assert_eq!(v["approval"]["run_id"], 5);

    let ok = run_with_stdin(op(), body);
    assert_eq!(ok.code, 0, "stdout={} stderr={}", ok.stdout, ok.stderr);
    let done: serde_json::Value = serde_json::from_str(&ok.stdout).expect("JSON 1 つ");
    assert_eq!(done["approval"], serde_json::json!({"qid": "a1b2c3d4e5f6", "answered_by": "saituki"}));

    // 承認は使い切ったので、もう一度の op は 4。
    let again = run_with_stdin(op(), body);
    assert_eq!(again.code, 4, "stdout={} stderr={}", again.stdout, again.stderr);

    // 同じ質問の承認を 2 回記録すると 4。
    let dup = run(record("a1b2c3d4e5f6", "許可する"));
    assert_eq!(dup.code, 4, "stderr={}", dup.stderr);
    assert!(dup.stdout.is_empty());

    let db = sai_paper_core::Database::open(&data_dir.join("papr.db")).unwrap();
    let row: (String, String, String) = db
        .connection()
        .query_row(
            "SELECT approved_by, qid, question FROM rt_log WHERE action = 'add_q' ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row, ("saituki".into(), "a1b2c3d4e5f6".into(), "根を足してよいか".into()));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
}

/// 約束: 承認が無い op は終了コード 4（標準出力は空）。「許可しない」の回答では通らない。run_id が無いときも 4。
#[test]
fn op_without_approval_returns_four() {
    let root = temp_dir("no-approval");
    let home = temp_dir("no-approval-home");
    let (data_dir, _) = project_with_tree(&root);
    let blocks = root.join(".sai").join("blocks");
    std::fs::create_dir_all(&blocks).unwrap();
    std::fs::write(blocks.join("t-add.md"), "---\nsci_ops: add_q\n---\n# t\n").unwrap();
    let body = r#"{"ops":[{"op":"add_q","parent":"SEED","text":"新しい根"}]}"#;
    let op = |run: Option<&str>| {
        let mut cmd = command(&root);
        cmd.env("HOME", &home).env("SAI_PROJECT_ROOT", &root).args(["op", "--block", "t-add"]);
        if let Some(run) = run {
            cmd.env("SAI_PARENT_RUN_ID", run);
        }
        cmd
    };

    let none = run_with_stdin(op(Some("5")), body);
    assert_eq!(none.code, 4, "stdout={} stderr={}", none.stdout, none.stderr);
    assert!(none.stdout.is_empty(), "失敗の標準出力が空でない: {}", none.stdout);
    assert!(none.stderr.lines().last().unwrap_or("").contains("許可がありません"), "stderr={}", none.stderr);

    let rec = run({
        let mut cmd = command(&root);
        cmd.env("HOME", &home).env("SAI_PROJECT_ROOT", &root).args([
            "approval", "record", "--run", "5", "--qid", "no1", "--question", "作ってよいか", "--answer", "許可しない",
            "--answered-by", "saituki",
        ]);
        cmd
    });
    assert_eq!(rec.code, 0, "stderr={}", rec.stderr);
    let denied = run_with_stdin(op(Some("5")), body);
    assert_eq!(denied.code, 4, "stderr={}", denied.stderr);

    let no_run = run_with_stdin(op(None), body);
    assert_eq!(no_run.code, 4, "stderr={}", no_run.stderr);
    assert!(no_run.stderr.lines().last().unwrap_or("").contains("run_id"), "stderr={}", no_run.stderr);

    // 失敗は何も残さない（add_q の行が無い）。
    let db = sai_paper_core::Database::open(&data_dir.join("papr.db")).unwrap();
    let n: i64 = db.connection().query_row("SELECT COUNT(*) FROM rt_log WHERE action = 'add_q'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
}

/// 台帳（`SAI_PARENT_RUN_ID` の run）: `find` の番号を残し、`op` の `add_f` は台帳に無い番号を止める。
/// 利用者の `apply` は台帳を見ない。
#[test]
fn ledger_checks_op_add_f_and_not_apply() {
    let root = temp_dir("ledger-root");
    let home = temp_dir("ledger-home");
    let tmp = temp_dir("ledger-tmp");
    let paper = fake_paper(&tmp);
    let args_log = tmp.join("args.log");
    let (data_dir, q) = project_with_tree(&root);
    // 論文 3 の F を Q1 の下に足すと、`find --node Q1` が論文を見つける。
    add_f_with_paper(&data_dir, q);
    let db = sai_paper_core::Database::open(&data_dir.join("papr.db")).unwrap();
    db.connection().execute("INSERT INTO papers(id, title) VALUES (3, 't')", []).unwrap();
    let a = store::insert_node(
        db.connection(),
        &store::NewNode {
            kind: Kind::A,
            parent_id: Some(q),
            guide_key: None,
            text: "A",
            body: "",
            stance: None,
            cites: &[],
            paper_id: None,
            run_id: None,
            report_id: None,
            mlflow_ref: None,
            paths: &[],
            created_by: "user",
        },
    )
    .unwrap();
    drop(db);

    // `find` は論文 3 の hit（`#3 §1`）を出す。台帳には `#3` だけが残る（`§1` は節として拾わない）。
    let mut find = command(&root);
    find.env("HOME", &home)
        .env("SAI_PROJECT_ROOT", &root)
        .env("SAI_PAPER_BIN", &paper)
        .env("ARGS_LOG", &args_log)
        .env("FAKE_FIND_LOG_ID", "7")
        .env("FAKE_SEARCH_LOG_ID", "8")
        .env("SAI_TARGET_NODE", "Q1")
        .env("SAI_PARENT_RUN_ID", "9")
        .args(["find", "発作", "--node", "Q1"]);
    let found = run(find);
    assert_eq!(found.code, 0, "stderr={}", found.stderr);
    let ledger = std::fs::read_to_string(root.join(".sai/seen_ids/9")).unwrap_or_default();
    assert_eq!(ledger, "#3\n", "stdout={} stderr={}", found.stdout, found.stderr);

    let blocks = root.join(".sai").join("blocks");
    std::fs::create_dir_all(&blocks).unwrap();
    std::fs::write(blocks.join("t-add.md"), "---\nsci_ops: add_f\n---\n# t\n").unwrap();
    let batch = |cite: &str| {
        serde_json::json!({"ops": [{"op": "add_f", "parent": format!("A{a}"), "text": "根拠", "stance": "support", "cites": [cite]}]})
            .to_string()
    };
    let op = |cite: &str| {
        let mut cmd = command(&root);
        cmd.env("HOME", &home)
            .env("SAI_PROJECT_ROOT", &root)
            .env("SAI_PARENT_RUN_ID", "9")
            .args(["op", "--block", "t-add"]);
        run_with_stdin(cmd, &batch(cite))
    };

    let ok = op("#3");
    assert_eq!(ok.code, 0, "stdout={} stderr={}", ok.stdout, ok.stderr);
    let bad = op("#3.1");
    assert_eq!(bad.code, 2, "stdout={} stderr={}", bad.stdout, bad.stderr);
    assert!(bad.stdout.is_empty(), "失敗の標準出力が空でない: {}", bad.stdout);
    assert!(
        bad.stderr.lines().last().unwrap_or("").contains("台帳に無い番号です"),
        "stderr={}",
        bad.stderr
    );

    let mut apply = command(&root);
    apply.env("HOME", &home).env("SAI_PROJECT_ROOT", &root).args(["apply"]);
    let applied = run_with_stdin(
        apply,
        &serde_json::json!({"op": "add_f", "parent": format!("A{a}"), "text": "根拠", "stance": "support", "cites": ["#3.1"]})
            .to_string(),
    );
    assert_eq!(applied.code, 0, "stdout={} stderr={}", applied.stdout, applied.stderr);

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `report prepare` は台帳に無い引用で終了コード 2、台帳にある引用で通る。
#[test]
fn report_prepare_checks_citations_against_ledger() {
    let root = temp_dir("prepare-root");
    let home = temp_dir("prepare-home");
    let (data_dir, _) = project_with_tree(&root);
    let db = sai_paper_core::Database::open(&data_dir.join("papr.db")).unwrap();
    db.connection().execute("INSERT INTO papers(id, title) VALUES (3, 't')", []).unwrap();
    drop(db);
    std::fs::create_dir_all(root.join(".sai").join("seen_ids")).unwrap();
    std::fs::write(root.join(".sai").join("seen_ids").join("9"), "#3\n").unwrap();

    let prepare = |body: &str| {
        let mut cmd = command(&root);
        cmd.env("HOME", &home)
            .env("SAI_PROJECT_ROOT", &root)
            .env("SAI_PARENT_RUN_ID", "9")
            .env("SAI_WRITE_DESCRIPTION", "作業報告書 U12: 台帳の試験")
            .args(["report", "prepare", "--run", "1", "--block", "t-add", "--filename", "report-u12.md"]);
        run_with_stdin(cmd, body)
    };
    let ok = prepare("# 作業報告書\n\n根拠 [#3]\n");
    assert_eq!(ok.code, 0, "stdout={} stderr={}", ok.stdout, ok.stderr);
    let bad = prepare("# 作業報告書\n\n根拠 [#3.1.1]\n");
    assert_eq!(bad.code, 2, "stdout={} stderr={}", bad.stdout, bad.stderr);
    assert!(bad.stderr.contains("台帳に無い番号です: #3.1.1"), "stderr={}", bad.stderr);

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
}
