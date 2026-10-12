//! `sai-sci onto` の結合の試験。終了コード（0・2・3）・`--json` の項目・表・`schema` の 4 命令を確かめる。
//!
//! 共有データは一時フォルダの小さな `onto.db`。研究プロジェクトは無くても動くことを、
//! `SAI_PROJECT_ROOT` を外した一時フォルダで確かめる。

use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::Connection;

/// 試験の中だけで使う共有データのフォルダ（並列でもぶつからないよう名前を分ける）。
fn shared_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sai-sci-onto-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let conn = Connection::open(dir.join("onto.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
         CREATE TABLE term(id TEXT PRIMARY KEY, source TEXT NOT NULL, label TEXT NOT NULL);
         CREATE TABLE synonym(term_id TEXT NOT NULL, text TEXT NOT NULL, norm TEXT NOT NULL, is_label INTEGER NOT NULL);
         CREATE TABLE edge(child TEXT NOT NULL, parent TEXT NOT NULL, PRIMARY KEY(child, parent));
         CREATE TABLE xref(term_id TEXT NOT NULL, other TEXT NOT NULL);
         INSERT INTO meta VALUES ('schema_version','1'), ('mondo_version','releases/2026-10-06'),
           ('mesh_version','2026'), ('built_at','2026-10-12T00:00:00+09:00');
         INSERT INTO term VALUES
           ('MONDO:0000001','mondo','disease'),
           ('MONDO:0005027','mondo','epilepsy'),
           ('MONDO:0018097','mondo','infantile spasms'),
           ('MESH:D013036','mesh','Spasms, Infantile');
         INSERT INTO synonym SELECT id, label, lower(label), 1 FROM term;
         INSERT INTO synonym VALUES ('MONDO:0018097','IESS','iess',0);
         INSERT INTO edge VALUES ('MONDO:0005027','MONDO:0000001'), ('MONDO:0018097','MONDO:0005027');
         INSERT INTO xref VALUES ('MONDO:0018097','MESH:D013036');",
    )
    .unwrap();
    dir
}

/// `sai-sci` を呼ぶ。親の研究用の環境変数は外し、`SAI_SHARED_DIR` は引数で決める。
fn sai_sci(cwd: &Path, shared: Option<&Path>, args: &[&str]) -> (i32, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sai-sci"));
    cmd.current_dir(cwd)
        .env_remove("SAI_PROJECT_ROOT")
        .env_remove("SAI_PARENT_RUN_ID")
        .env_remove("SAI_BLOCK_NAME")
        .env_remove("SAI_TARGET_NODE")
        .env_remove("SAI_ORIGIN")
        .env_remove("SAI_SHARED_DIR")
        .args(args);
    if let Some(dir) = shared {
        cmd.env("SAI_SHARED_DIR", dir);
    }
    let out = cmd.output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// 標準エラーの最後の行。
fn last_line(stderr: &str) -> &str {
    stderr.trim_end().lines().last().unwrap_or("")
}

/// 試験の作業フォルダ（研究プロジェクトは無い）。
fn workdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sai-sci-onto-cwd-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn lookup_json_has_contract_fields_and_text_is_a_table() {
    let shared = shared_dir("lookup");
    let cwd = workdir("lookup");
    let (code, stdout, stderr) = sai_sci(
        &cwd,
        Some(&shared),
        &["onto", "lookup", "ＩＥＳＳ", "--json"],
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["candidates"][0]["id"], "MONDO:0018097");
    assert_eq!(v["candidates"][0]["match"], "exact");
    assert_eq!(v["candidates"][0]["matched"], "IESS");
    assert_eq!(v["map_version"]["mondo"], "releases/2026-10-06");

    let (code, stdout, _) = sai_sci(&cwd, Some(&shared), &["onto", "lookup", "IESS"]);
    assert_eq!(code, 0);
    assert!(
        stdout.starts_with("MONDO:0018097  mondo  exact"),
        "stdout: {stdout}"
    );
}

#[test]
fn show_relation_and_ancestors_json_shapes() {
    let shared = shared_dir("shapes");
    let cwd = workdir("shapes");
    let (code, stdout, _) = sai_sci(
        &cwd,
        Some(&shared),
        &["onto", "show", "MONDO:0018097", "--json"],
    );
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["xref"][0], "MESH:D013036");
    assert_eq!(v["depth"], 2);
    assert_eq!(v["children"]["total"], 0);

    let (code, stdout, _) = sai_sci(
        &cwd,
        Some(&shared),
        &[
            "onto",
            "relation",
            "MONDO:0018097",
            "MONDO:0000001",
            "--json",
        ],
    );
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["relation"], "descendant");
    assert_eq!(v["distance"], 2);
    assert_eq!(v["xref_between"], false);
    // 事実だけを返す（警告・score・閾値の項目は無い）
    for banned in ["warning", "score", "threshold", "warn"] {
        assert!(v.get(banned).is_none(), "{banned} は返さない");
    }

    let (code, stdout, _) = sai_sci(
        &cwd,
        Some(&shared),
        &[
            "onto",
            "relation",
            "MONDO:0018097",
            "MESH:D013036",
            "--json",
        ],
    );
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["relation"], "different_source");
    assert_eq!(v["xref_between"], true);

    let (code, stdout, _) = sai_sci(
        &cwd,
        Some(&shared),
        &["onto", "ancestors", "MONDO:0018097", "--json"],
    );
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        v["ancestors"][0],
        serde_json::json!({"id":"MONDO:0005027","label":"epilepsy","distance":1})
    );

    let (code, stdout, _) = sai_sci(&cwd, Some(&shared), &["onto", "ancestors", "MONDO:0018097"]);
    assert_eq!(code, 0);
    assert!(
        stdout.starts_with("1  MONDO:0005027  epilepsy"),
        "stdout: {stdout}"
    );
}

#[test]
fn exit_codes_follow_the_contract() {
    let shared = shared_dir("codes");
    let cwd = workdir("codes");

    // 共有データが無い → 3（標準出力は空・標準エラーの最後の行に理由）
    let (code, stdout, stderr) = sai_sci(&cwd, None, &["onto", "lookup", "IESS", "--json"]);
    assert_eq!(code, 3);
    assert!(stdout.is_empty());
    assert!(
        last_line(&stderr).contains("SAI_SHARED_DIR"),
        "stderr: {stderr}"
    );

    // onto.db が無いフォルダ → 3
    let empty = workdir("codes-empty");
    let (code, _, _) = sai_sci(&cwd, Some(&empty), &["onto", "show", "MONDO:0018097"]);
    assert_eq!(code, 3);

    // ID が無い → 3
    let (code, stdout, _) = sai_sci(
        &cwd,
        Some(&shared),
        &["onto", "show", "MONDO:9999999", "--json"],
    );
    assert_eq!(code, 3);
    assert!(stdout.is_empty());

    // ID の形が違う → 2
    let (code, stdout, stderr) = sai_sci(&cwd, Some(&shared), &["onto", "show", "foo", "--json"]);
    assert_eq!(code, 2);
    assert!(stdout.is_empty());
    assert!(last_line(&stderr).contains("ID の形"), "stderr: {stderr}");

    // 引数の誤り（source に無い値）→ 2
    let (code, _, _) = sai_sci(
        &cwd,
        Some(&shared),
        &["onto", "lookup", "IESS", "--source", "icd"],
    );
    assert_eq!(code, 2);
}

#[test]
fn schema_lists_the_four_onto_commands() {
    let cwd = workdir("schema");
    let (code, stdout, _) = sai_sci(&cwd, None, &["schema", "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let paths: Vec<String> = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            c["path"]
                .as_array()
                .unwrap()
                .iter()
                .map(|w| w.as_str().unwrap())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|p| p.starts_with("onto "))
        .collect();
    assert_eq!(
        paths,
        [
            "onto lookup",
            "onto show",
            "onto relation",
            "onto ancestors"
        ]
    );
}
