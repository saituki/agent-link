//! 論文の移植（`library export`・`library import-migration`）の CLI 結合テスト。
//!
//! 一時の 2 つのプロジェクト（移植元・移植先）を作り、実際の実行ファイルを動かす。
//! JSON の形と終了コード（2 = 入力の誤り・3 = 見つからない）を確かめる。ネットには出ない。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// テストで動かす実行ファイル。
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_sai-paper")
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
            "sai-paper-migration-cli-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 環境をそろえて実行する（`SAI_PROJECT_ROOT` は指定したプロジェクトに向ける）。
fn run(cwd: &Path, home: &Path, project: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .env("HOME", home)
        .env("SAI_HOME", home.join(".sai"))
        .env("SAI_PROJECT_ROOT", project)
        .env_remove("SAI_ORIGIN")
        .env_remove("SAI_PARENT_RUN_ID")
        .env_remove("SAI_BLOCK_NAME")
        .current_dir(cwd)
        .output()
        .expect("実行ファイルを動かせる")
}

/// 標準出力の文字。
fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// 一時プロジェクトを初期化する（移植元・移植先のそれぞれ）。
fn init_project(root: &Path, name: &str) -> (PathBuf, PathBuf) {
    let home = root.join(format!("{name}-home"));
    let project = root.join(name);
    std::fs::create_dir_all(&project).unwrap();
    let out = run(&project, &home, &project, &["init"]);
    assert!(
        out.status.success(),
        "init が失敗: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (home, project)
}

/// 移植元に、PDF の無い論文を 1 本入れて `papers.id` を返す。
fn keep_paper(home: &Path, project: &Path) -> i64 {
    let bib = project.join(".sai/paper/bibtex");
    std::fs::create_dir_all(&bib).unwrap();
    std::fs::write(
        bib.join("refs.bib"),
        "@article{mig2020,\n  title = {Migration Test Paper},\n  author = {Doe, Jane},\n  year = {2020},\n  doi = {10.1000/mig},\n}\n",
    )
    .unwrap();
    let out = run(
        project,
        home,
        project,
        &["paper", "keep", "--import-id", "DOI:10.1000/mig", "--json"],
    );
    assert!(out.status.success(), "keep が失敗");
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("JSON");
    value["paper_id"].as_i64().expect("paper_id")
}

/// 書き出し → 取り込みで論文が移り、2 回目の書き出し先の衝突は終了コード 2 になる。
#[test]
fn export_and_import_migration_round_trip_with_exit_codes() {
    let temp = Temp::new("roundtrip");
    let (src_home, src) = init_project(temp.path(), "src");
    let (dst_home, dst) = init_project(temp.path(), "dst");
    let paper_id = keep_paper(&src_home, &src);
    let pkg = temp.path().join("pkg");

    let export = run(
        &src,
        &src_home,
        &src,
        &[
            "library",
            "export",
            "--ids",
            &paper_id.to_string(),
            "--out",
            pkg.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        export.status.success(),
        "export が失敗: {}",
        String::from_utf8_lossy(&export.stderr)
    );
    let value: serde_json::Value = serde_json::from_str(&stdout(&export)).expect("JSON");
    assert_eq!(value["papers"], serde_json::json!([paper_id]));
    assert_eq!(value["missing"], serde_json::json!([]));
    assert!(pkg.join("manifest.json").is_file());
    assert!(pkg.join("papr.db").is_file());

    // 書き出し先が既にあれば終了コード 2。
    let again = run(
        &src,
        &src_home,
        &src,
        &[
            "library",
            "export",
            "--ids",
            &paper_id.to_string(),
            "--out",
            pkg.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(again.status.code(), Some(2), "終了コード");
    assert!(stdout(&again).is_empty());

    // 1 つも見つからなければ終了コード 3。書き出し先は作らない。
    let none_out = temp.path().join("none");
    let missing = run(
        &src,
        &src_home,
        &src,
        &[
            "library",
            "export",
            "--ids",
            "9999",
            "--out",
            none_out.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(missing.status.code(), Some(3), "終了コード");
    assert!(!none_out.exists());

    // 移植先で取り込む。新しい id で入り、library pending に DOI 付きで出る。
    let import = run(
        &dst,
        &dst_home,
        &dst,
        &[
            "library",
            "import-migration",
            "--dir",
            pkg.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        import.status.success(),
        "import-migration が失敗: {}",
        String::from_utf8_lossy(&import.stderr)
    );
    let value: serde_json::Value = serde_json::from_str(&stdout(&import)).expect("JSON");
    let imported = value["imported"].as_array().expect("imported");
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0]["source_id"], paper_id);
    assert_eq!(imported[0]["title"], "Migration Test Paper");
    assert_eq!(imported[0]["mode"], "new");
    assert_eq!(value["skipped"], serde_json::json!([]));
    assert_eq!(value["failed"], serde_json::json!([]));

    let pending = run(&dst, &dst_home, &dst, &["library", "pending", "--json"]);
    assert!(pending.status.success());
    let value: serde_json::Value = serde_json::from_str(&stdout(&pending)).expect("JSON");
    let found = value["papers"]
        .as_array()
        .expect("papers")
        .iter()
        .find(|paper| paper["title"] == "Migration Test Paper")
        .expect("取り込んだ論文が pending に出る");
    assert_eq!(found["doi"], "10.1000/mig");
}

/// 束の形が違えば import-migration は終了コード 2。
#[test]
fn import_migration_rejects_a_folder_without_manifest() {
    let temp = Temp::new("reject");
    let (dst_home, dst) = init_project(temp.path(), "dst");
    let empty = temp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let out = run(
        &dst,
        &dst_home,
        &dst,
        &[
            "library",
            "import-migration",
            "--dir",
            empty.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(2), "終了コード");
    assert!(stdout(&out).is_empty());
}
