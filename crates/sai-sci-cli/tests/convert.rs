//! `sai-sci convert` の CLI の約束（終了コード 2・3・4、`--json` の 1 行、元が変わらないこと）。
//! 木の中身の確かめはコア（`rt::convert` の単体試験）で行う。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sai_paper_core::Database;
use sai_sci_core::rt::{Kind, store};

/// 試験の一時フォルダ（並列でもぶつからないよう名前を分ける）。
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sai-sci-convert-cli-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos())
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// `sai-sci convert` を呼ぶ。研究の環境変数は外し、利用者の SAI_HOME は一時フォルダへ向ける。
fn convert(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_sai-sci"))
        .arg("convert")
        .args(args)
        .current_dir(dir)
        .env_remove("SAI_PROJECT_ROOT")
        .env_remove("SAI_PARENT_RUN_ID")
        .env_remove("SAI_BLOCK_NAME")
        .env_remove("SAI_TARGET_NODE")
        .env_remove("SAI_ORIGIN")
        .env("SAI_HOME", dir.join("home"))
        .output()
        .expect("sai-sci を起動できる");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// 試験用の Node（本文と根拠は空）。
fn node(kind: Kind, parent: Option<i64>, text: &str) -> store::NewNode<'_> {
    store::NewNode {
        kind,
        parent_id: parent,
        guide_key: (kind == Kind::Q).then_some("guide-check"),
        text,
        body: "",
        stance: None,
        cites: &[],
        paper_id: None,
        run_id: None,
        report_id: None,
        mlflow_ref: None,
        dataset_ref: None,
        paths: &[],
        created_by: "test",
    }
}

/// 旧い木の元を作る（A → Q → A。移行 61 を外して移行 60 までの形にする）。
fn make_origin(root: &Path) -> PathBuf {
    let data = root.join(".sai").join("paper");
    fs::create_dir_all(&data).unwrap();
    let db = Database::open(&data.join("papr.db")).unwrap();
    let conn = db.connection();
    store::meta_set(conn, "initialized_at", "2026-10-01 00:00:00").unwrap();
    let axis = store::insert_node(conn, &node(Kind::Q, None, "軸")).unwrap();
    let a = store::insert_node(conn, &node(Kind::A, Some(axis), "答え")).unwrap();
    let q = store::insert_node(conn, &node(Kind::Q, Some(a), "分け方の基準: 種。")).unwrap();
    store::insert_node(conn, &node(Kind::A, Some(q), "ネコ")).unwrap();
    conn.execute_batch(
        "ALTER TABLE rt_nodes DROP COLUMN rel; DELETE FROM schema_migrations WHERE version = 61;",
    )
    .unwrap();
    data.join("papr.db")
}

#[test]
fn convert_writes_a_copy_and_prints_the_report_as_one_json_line() {
    let dir = temp_dir("json");
    let origin = make_origin(&dir.join("from"));
    let before = fs::read(&origin).unwrap();

    let (code, stdout, stderr) = convert(&dir, &["--from", "from", "--to", "to", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(stdout.trim().lines().count(), 1);
    assert_eq!(report["before"]["q_other"], 1);
    assert_eq!(report["after"]["q_other"], 0);
    assert_eq!(report["after"]["a"], 2);
    assert_eq!(report["flattened_q"], 1);
    assert_eq!(report["moved_a"], 1);
    assert_eq!(report["ord_exact"], true);
    assert!(dir.join("to/.sai/paper/tree-idmap.json").is_file());
    assert!(dir.join("to/.sai/paper/papr.db").is_file());
    assert_eq!(
        fs::read(&origin).unwrap(),
        before,
        "元の papr.db は変わらない"
    );
}

#[test]
fn convert_without_json_prints_a_japanese_summary() {
    let dir = temp_dir("human");
    make_origin(&dir.join("from"));
    let (code, stdout, stderr) = convert(&dir, &["--from", "from", "--to", "to"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.starts_with("複製して変換しました"), "{stdout}");
    assert!(stdout.contains("対応表:"), "{stdout}");
}

#[test]
fn convert_exit_codes_follow_the_common_contract() {
    let dir = temp_dir("codes");
    make_origin(&dir.join("from"));
    fs::create_dir_all(dir.join("exists")).unwrap();

    // 元が無い → 3。
    let (code, _, stderr) = convert(&dir, &["--from", "none", "--to", "to"]);
    assert_eq!(code, 3, "{stderr}");
    // `--to` が既にある → 4（空のフォルダでも断る）。
    let (code, _, stderr) = convert(&dir, &["--from", "from", "--to", "exists"]);
    assert_eq!(code, 4, "{stderr}");
    // `--from` と `--to` が同じ → 2。標準エラーの最後の行に理由を 1 行。
    let (code, stdout, stderr) = convert(&dir, &["--from", "from", "--to", "from", "--json"]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stdout.is_empty(), "失敗のときは標準出力に出さない");
    assert!(
        stderr.lines().last().is_some_and(|line| !line.is_empty()),
        "{stderr}"
    );
    // 引数が足りない → 2（clap の誤り）。
    let (code, _, _) = convert(&dir, &["--from", "from"]);
    assert_eq!(code, 2);
}
