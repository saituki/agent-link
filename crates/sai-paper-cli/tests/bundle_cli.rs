//! U12: `paper keep`・`library pending`・`paper import-bundle`・サーバで PDF を
//! 落とす命令の停止（終了コード 4）の CLI 結合テスト。
//!
//! 一時プロジェクトを作り、実際の実行ファイルを動かす。ネットにつながる命令は打たない。
//! `import-bundle` は `DOCLING_BIN`・`PDF2ZH_BIN` に「呼ばれたら印のファイルを作る」
//! 偽の実行ファイルを子プロセスの環境変数で渡し、印が作られないことを確かめる
//! （core 側のテストは edition 2024 で `set_var` を使えないため、ここで確かめる）。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sai_paper_core::bundle::sha256_file;

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
            "sai-paper-bundle-cli-{tag}-{}-{unique}",
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

/// 環境をそろえて実行する。
fn run(
    cwd: &Path,
    home: &Path,
    project: &Path,
    envs: &[(&str, &str)],
    args: &[&str],
) -> Output {
    let mut command = Command::new(bin());
    command
        .args(args)
        .env("HOME", home)
        .env("SAI_HOME", home.join(".sai"))
        .env("SAI_PROJECT_ROOT", project)
        .env_remove("SAI_ORIGIN")
        .env_remove("SAI_PARENT_RUN_ID")
        .env_remove("SAI_BLOCK_NAME")
        .current_dir(cwd);
    for (key, value) in envs {
        command.env(key, value);
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

/// 一時プロジェクトを初期化する。
fn init_project(temp: &Temp) -> PathBuf {
    let project = temp.project();
    std::fs::create_dir_all(&project).unwrap();
    let out = run(&project, &temp.home(), &project, &[], &["init"]);
    assert!(
        out.status.success(),
        "init が失敗: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    project
}

/// BibTeX フォルダに .bib を 1 件置く（`--import-id` の対象になる）。
fn write_bib(project: &Path, name: &str, content: &str) {
    let dir = project.join(".sai/paper/bibtex");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), content).unwrap();
}

/// AC3: `paper keep --import-id` で PDF 無しの論文ができ、2 回目は `exists`。
/// `library pending` にその論文が `doi` 付きで出る。
#[test]
fn keep_adds_a_paper_and_pending_lists_it() {
    let temp = Temp::new("keep");
    let project = init_project(&temp);
    write_bib(
        &project,
        "refs.bib",
        "@article{keep2020,\n  title = {Keep Test Paper},\n  author = {Doe, Jane},\n  year = {2020},\n  doi = {10.1000/keep},\n}\n",
    );

    // 1 回目は added。
    let first = run(
        &project,
        &temp.home(),
        &project,
        &[],
        &["paper", "keep", "--import-id", "DOI:10.1000/keep", "--json"],
    );
    assert!(
        first.status.success(),
        "keep が失敗: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let value: serde_json::Value = serde_json::from_str(&stdout(&first)).expect("JSON");
    assert_eq!(value["status"], "added");
    assert_eq!(value["title"], "Keep Test Paper");
    let paper_id = value["paper_id"].as_i64().expect("paper_id");

    // 2 回目は exists（同じ論文）。
    let second = run(
        &project,
        &temp.home(),
        &project,
        &[],
        &["paper", "keep", "--import-id", "DOI:10.1000/keep", "--json"],
    );
    assert!(second.status.success(), "keep 2 回目が失敗");
    let value: serde_json::Value = serde_json::from_str(&stdout(&second)).expect("JSON");
    assert_eq!(value["status"], "exists");
    assert_eq!(value["paper_id"].as_i64(), Some(paper_id));

    // library pending に DOI 付きで出る。
    let pending = run(
        &project,
        &temp.home(),
        &project,
        &[],
        &["library", "pending", "--json"],
    );
    assert!(
        pending.status.success(),
        "library pending が失敗: {}",
        String::from_utf8_lossy(&pending.stderr)
    );
    let value: serde_json::Value = serde_json::from_str(&stdout(&pending)).expect("JSON");
    let papers = value["papers"].as_array().expect("papers");
    let found = papers
        .iter()
        .find(|paper| paper["id"].as_i64() == Some(paper_id))
        .expect("keep した論文が pending に出る");
    assert_eq!(found["doi"], "10.1000/keep");
    assert_eq!(found["title"], "Keep Test Paper");
}

/// AC3: `paper download` は終了コード 4（サーバでは PDF を落とさない）。
#[test]
fn download_is_exit_4() {
    let temp = Temp::new("download");
    let project = init_project(&temp);
    write_bib(
        &project,
        "refs.bib",
        "@article{dl2020,\n  title = {Download Test Paper},\n  author = {Doe, Jane},\n  year = {2020},\n  doi = {10.1000/dl},\n}\n",
    );
    let out = run(
        &project,
        &temp.home(),
        &project,
        &[],
        &["paper", "download", "--import-id", "DOI:10.1000/dl", "--json"],
    );
    assert_eq!(out.status.code(), Some(4), "終了コード: {:?}", out.status.code());
    assert!(stdout(&out).is_empty(), "標準出力: {}", stdout(&out));
    let last = stderr_last_line(&out);
    assert!(last.contains("sai-prep"), "最後の行: {last}");
}

/// AC1（CLI 側）: `paper import-bundle` は偽の docling・pdf2zh を呼ばずに成功し、
/// 図・節・引用・翻訳の表を作る。
#[test]
fn import_bundle_cli_does_not_call_docling_or_pdf2zh() {
    let temp = Temp::new("import-bundle");
    let project = init_project(&temp);
    let data_dir = project.join(".sai/paper");

    // PDF 無しの論文を 1 件作る。
    let paper_id = {
        let db = rusqlite::Connection::open(data_dir.join("papr.db")).unwrap();
        db.execute(
            "INSERT INTO papers (title, doi) VALUES ('Bundle Paper', '10.1/bundle')",
            [],
        )
        .unwrap();
        db.last_insert_rowid()
    };

    // 偽の docling・pdf2zh（呼ばれたら印のファイルを作る）。
    let marker_docling = temp.path().join("docling-called");
    let marker_pdf2zh = temp.path().join("pdf2zh-called");
    let fake_docling = temp.path().join("fake-docling.sh");
    let fake_pdf2zh = temp.path().join("fake-pdf2zh.sh");
    std::fs::write(
        &fake_docling,
        format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker_docling.display()),
    )
    .unwrap();
    std::fs::write(
        &fake_pdf2zh,
        format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker_pdf2zh.display()),
    )
    .unwrap();
    for path in [&fake_docling, &fake_pdf2zh] {
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            perms.set_mode(0o755);
        }
        std::fs::set_permissions(path, perms).unwrap();
    }

    let bundle_dir = temp.path().join("bundle");
    write_bundle(&bundle_dir, paper_id, true);

    let out = run(
        &project,
        &temp.home(),
        &project,
        &[
            ("DOCLING_BIN", fake_docling.to_str().unwrap()),
            ("PDF2ZH_BIN", fake_pdf2zh.to_str().unwrap()),
        ],
        &[
            "paper",
            "import-bundle",
            "--dir",
            bundle_dir.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        out.status.success(),
        "import-bundle が失敗: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("JSON");
    assert_eq!(value["paper_id"].as_i64(), Some(paper_id));
    assert_eq!(value["figures"], 2);
    assert!(value["sections"].as_i64().unwrap_or(0) >= 1);
    assert_eq!(value["translated"], true);

    // docling も pdf2zh も呼ばれていない（印が無い）。
    assert!(!marker_docling.exists(), "docling を呼んでいる");
    assert!(!marker_pdf2zh.exists(), "pdf2zh を呼んでいる");

    // ファイルができている。
    let folder = sai_paper_core::paths::paper_folder(&data_dir, paper_id, "Bundle Paper");
    assert!(folder.join("paper.pdf").is_file());
    assert!(folder.join("extract.md").is_file());
    assert!(folder.join("figures/fig-001.png").is_file());
    assert!(folder.join("paper-ja.pdf").is_file());
    assert!(folder.join("paper-dual.pdf").is_file());
}

/// `manifest.json` の誤りは終了コード 2。
#[test]
fn import_bundle_bad_manifest_is_exit_2() {
    let temp = Temp::new("bad-manifest");
    let project = init_project(&temp);
    let bundle_dir = temp.path().join("bundle");
    std::fs::create_dir_all(&bundle_dir).unwrap();
    std::fs::write(bundle_dir.join("manifest.json"), "{ not json").unwrap();
    let out = run(
        &project,
        &temp.home(),
        &project,
        &[],
        &[
            "paper",
            "import-bundle",
            "--dir",
            bundle_dir.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(2), "終了コード: {:?}", out.status.code());
    assert!(stdout(&out).is_empty());
}

/// C12 の形の束を 1 つ作る。
fn write_bundle(dir: &Path, paper_id: i64, translated: bool) {
    std::fs::create_dir_all(dir.join("docling/paper_artifacts")).unwrap();
    std::fs::create_dir_all(dir.join("translated")).unwrap();
    let pdf = b"%PDF-1.4 bundle\n%%EOF\n".to_vec();
    std::fs::write(dir.join("paper.pdf"), &pdf).unwrap();
    std::fs::write(
        dir.join("docling/paper.md"),
        "# Introduction\n\nAs shown in Figure 1, the method works.\n\n# References\n\n[1] Doe, J. 2020. A study.\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("docling/paper.json"),
        r##"{
  "texts": [{"text": "Figure 1: The workflow."}],
  "pictures": [
    {"image": {"uri": "paper_artifacts/fig1.png"}, "captions": [{"$ref": "#/texts/0"}], "prov": [{"page_no": 1}]},
    {"image": {"uri": "paper_artifacts/fig2.png"}, "captions": [{"$ref": "#/texts/0"}], "prov": [{"page_no": 2}]}
  ]
}"##,
    )
    .unwrap();
    std::fs::write(dir.join("docling/paper_artifacts/fig1.png"), b"\x89PNG-one").unwrap();
    std::fs::write(dir.join("docling/paper_artifacts/fig2.png"), b"\x89PNG-two").unwrap();
    let translation = if translated {
        std::fs::write(dir.join("translated/paper-mono.pdf"), b"%PDF-1.4 mono\n").unwrap();
        std::fs::write(dir.join("translated/paper-dual.pdf"), b"%PDF-1.4 dual\n").unwrap();
        serde_json::json!({
            "mono": "translated/paper-mono.pdf",
            "dual": "translated/paper-dual.pdf",
            "service": "mlx:test",
        })
    } else {
        serde_json::Value::Null
    };
    let manifest = serde_json::json!({
        "format": "sai-prep-bundle",
        "version": 1,
        "target": {"paper_id": paper_id},
        "pdf": "paper.pdf",
        "pdf_sha256": sha256_file(&dir.join("paper.pdf")).unwrap(),
        "docling": {"md": "docling/paper.md", "json": "docling/paper.json", "dir": "docling"},
        "translation": translation,
    });
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
}
