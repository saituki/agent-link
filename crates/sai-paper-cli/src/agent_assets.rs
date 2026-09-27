//! `sai-paper init` が研究プロジェクトに配るエージェントの部品（段階 7）。
//!
//! - `.sai/blocks/paper-*.md`（sai-link のブロック）と `.sai/template/*.md` を書き出す。
//!   既にあるファイルは上書きしない（利用者が直したものを消さない）。
//! - ブロックには拡張（pi の `--extension`）の絶対パスを書かない。代わりに、この PC での
//!   pi-sai-link と pi-litbase の場所を探し、プロジェクトの `.sai/settings.json` の
//!   `extensions` に足す（sai-link は全ブロックでこれを読み込む）。
//! - cc（Claude Code）用に、プロジェクトの `.mcp.json` に `sai-paper` の MCP サーバーを足す。
//!   既にある項目は消さない。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

/// 配るブロック（ファイル名, 中身）。
const BLOCKS: &[(&str, &str)] = &[
    ("paper-brainstorm.md", include_str!("../assets/blocks/paper-brainstorm.md")),
    ("paper-check.md", include_str!("../assets/blocks/paper-check.md")),
    ("paper-clarify.md", include_str!("../assets/blocks/paper-clarify.md")),
    ("paper-critic.md", include_str!("../assets/blocks/paper-critic.md")),
    ("paper-debate.md", include_str!("../assets/blocks/paper-debate.md")),
    ("paper-debater.md", include_str!("../assets/blocks/paper-debater.md")),
    ("paper-discuss.md", include_str!("../assets/blocks/paper-discuss.md")),
    ("paper-gap.md", include_str!("../assets/blocks/paper-gap.md")),
    ("paper-library-qa.md", include_str!("../assets/blocks/paper-library-qa.md")),
    ("paper-map.md", include_str!("../assets/blocks/paper-map.md")),
    ("paper-next.md", include_str!("../assets/blocks/paper-next.md")),
    ("paper-plan.md", include_str!("../assets/blocks/paper-plan.md")),
    ("paper-read.md", include_str!("../assets/blocks/paper-read.md")),
    ("paper-reviewer.md", include_str!("../assets/blocks/paper-reviewer.md")),
    ("paper-screen.md", include_str!("../assets/blocks/paper-screen.md")),
    ("paper-search.md", include_str!("../assets/blocks/paper-search.md")),
    ("paper-template-maker.md", include_str!("../assets/blocks/paper-template-maker.md")),
];

/// 配るテンプレート（ファイル名, 中身）。
const TEMPLATES: &[(&str, &str)] = &[
    ("debate-participant.md", include_str!("../assets/template/debate-participant.md")),
    ("literature-hard-rules.md", include_str!("../assets/template/literature-hard-rules.md")),
    ("paper-screening.md", include_str!("../assets/template/paper-screening.md")),
    ("paper-search-strategy.md", include_str!("../assets/template/paper-search-strategy.md")),
    ("rag-deep-search.md", include_str!("../assets/template/rag-deep-search.md")),
    ("rag-simple.md", include_str!("../assets/template/rag-simple.md")),
    ("research-core.md", include_str!("../assets/template/research-core.md")),
    ("search-external.md", include_str!("../assets/template/search-external.md")),
    ("search-internal.md", include_str!("../assets/template/search-internal.md")),
];

/// 分野の観点の見本（分野の値, ファイル名, 中身）。
///
/// 3 つとも `.sai/template/` に置き、選んだ分野の中身を `domain.md` に写す。
const DOMAIN_SAMPLES: &[(&str, &str, &str)] = &[
    (
        "medical",
        "domain-medical.md",
        include_str!("../assets/template/domain-medical.md"),
    ),
    (
        "signal-ml",
        "domain-signal-ml.md",
        include_str!("../assets/template/domain-signal-ml.md"),
    ),
    (
        "general",
        "domain-general.md",
        include_str!("../assets/template/domain-general.md"),
    ),
];

/// 分野の値から見本の中身を返す（`general` など）。知らない値なら `None`。
fn domain_sample(domain: &str) -> Option<&'static str> {
    DOMAIN_SAMPLES
        .iter()
        .find(|(value, _, _)| *value == domain)
        .map(|(_, _, content)| *content)
}

/// `project-brief.md` がまだ無いときに置くプレースホルダ。
///
/// sai-link はテンプレートが見つからないと実行の前に止まるため、`sai-paper init`
/// の時点で必ずこのファイルを作る。`sai-paper brief --write` が本物のまとめで
/// 置き換える。
const BRIEF_PLACEHOLDER: &str = "<!-- sai-paper brief がエージェントの起動直前に作り直す -->\n# プロジェクトの状況（sai-paper より）\n\n（まだ作られていません）\n";

/// `start` から親へたどり、`relative` があるフォルダを探す。
fn find_upwards(start: &Path, relative: &str) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join(relative))
        .find(|path| path.is_file())
}

/// PATH の中から実行ファイルを探し、実体（シンボリックリンクの先）を返す。
fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
        .and_then(|path| std::fs::canonicalize(path).ok())
}

/// この PC での pi の拡張の場所（見つかったものだけ）。
///
/// 環境変数 `SAI_LINK_PI_EXTENSION` / `SAI_PAPER_PI_LITBASE` があればそれを使う。
/// 無ければ、`sai-link` と `sai-paper` の実行ファイルの場所から、それぞれの
/// リポジトリの `pi-sai-link/extension.ts` と `pi-litbase/index.ts` を探す。
fn pi_extensions() -> Vec<PathBuf> {
    let from_env = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_file());
    let sai_link = from_env("SAI_LINK_PI_EXTENSION").or_else(|| {
        which("sai-moon-link").and_then(|bin| find_upwards(&bin, "pi-sai-link/extension.ts"))
    });
    let litbase = from_env("SAI_PAPER_PI_LITBASE").or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|bin| std::fs::canonicalize(bin).ok())
            .and_then(|bin| find_upwards(&bin, "pi-litbase/index.ts"))
    });
    [sai_link, litbase].into_iter().flatten().collect()
}

/// JSON ファイルを読む（無ければ空のオブジェクト）。
fn read_json(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(text) if !text.trim().is_empty() => {
            serde_json::from_str(&text).with_context(|| format!("{} を読めません", path.display()))
        }
        _ => Ok(json!({})),
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    std::fs::write(path, text)?;
    Ok(())
}

/// ファイルが無ければ書く。書いたら `true`。
fn write_if_missing(path: &Path, content: &str) -> Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(true)
}

/// 研究プロジェクト（`root`）に部品を配る。何をしたかの一覧を返す（表示用）。
///
/// `sai_paper_bin` は `.mcp.json` に書く `sai-paper` の実行ファイル。
///
/// # Errors
/// ファイルの読み書きに失敗したとき。
pub fn install(root: &Path, sai_paper_bin: &Path, extensions: &[PathBuf]) -> Result<Vec<String>> {
    let mut done = Vec::new();
    let sai = root.join(".sai");

    let mut written = 0;
    for (name, content) in BLOCKS {
        written += usize::from(write_if_missing(&sai.join("blocks").join(name), content)?);
    }
    for (name, content) in TEMPLATES {
        written += usize::from(write_if_missing(&sai.join("template").join(name), content)?);
    }
    // `project-brief.md` は毎回 `sai-paper brief --write` が上書きするが、
    // sai-link はテンプレートが無いと止まるので、無ければここでプレースホルダを置く。
    written += usize::from(write_if_missing(
        &sai.join("template").join("project-brief.md"),
        BRIEF_PLACEHOLDER,
    )?);
    done.push(format!("ブロックとテンプレートを {written} 件置きました（既にあるものはそのまま）"));

    // pi の拡張: .sai/settings.json の extensions に足す（重複は足さない）
    let settings_path = sai.join("settings.json");
    let mut settings = read_json(&settings_path)?;
    let obj = settings
        .as_object_mut()
        .context(".sai/settings.json がオブジェクトではありません")?;
    {
        let list = obj.entry("extensions").or_insert_with(|| json!([]));
        if let Some(list) = list.as_array_mut() {
            for path in extensions {
                let path = Value::String(path.to_string_lossy().into_owned());
                if !list.contains(&path) {
                    list.push(path);
                }
            }
        }
    }
    // 実行直前の準備コマンド: 無ければ足す（あれば利用者の設定をそのまま残す）
    let mut pre_run_added = false;
    if !obj.contains_key("pre_run") {
        obj.insert(
            "pre_run".to_string(),
            json!([sai_paper_bin.to_string_lossy(), "brief", "--write"]),
        );
        pre_run_added = true;
    }
    write_json(&settings_path, &settings)?;
    if extensions.is_empty() {
        done.push(
            "⚠ pi の拡張が見つかりません。SAI_LINK_PI_EXTENSION / SAI_PAPER_PI_LITBASE を指定して init し直すか、.sai/settings.json の extensions に書いてください".to_string(),
        );
    } else {
        done.push(format!("pi の拡張を .sai/settings.json に登録しました（{} 件）", extensions.len()));
    }
    if pre_run_added {
        done.push(
            "pre_run（実行直前に状況まとめを作る）を .sai/settings.json に登録しました".to_string(),
        );
    }

    // cc 用: .mcp.json に sai-paper の MCP サーバーを足す（既にあれば触らない）
    let mcp_path = root.join(".mcp.json");
    let mut mcp = read_json(&mcp_path)?;
    let servers = mcp
        .as_object_mut()
        .context(".mcp.json がオブジェクトではありません")?
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    if let Some(servers) = servers.as_object_mut()
        && !servers.contains_key("sai-paper")
    {
        servers.insert(
            "sai-paper".to_string(),
            json!({
                "command": sai_paper_bin.to_string_lossy(),
                "args": ["mcp"],
                "env": {"SAI_PAPER_ROOT": root.to_string_lossy()},
            }),
        );
        write_json(&mcp_path, &mcp)?;
        done.push(".mcp.json に sai-paper の MCP サーバーを足しました".to_string());
    }
    Ok(done)
}

/// 分野の観点を配る（`sai-paper init --domain <分野>` から呼ぶ）。
///
/// - `domain-medical.md` / `domain-signal-ml.md` / `domain-general.md` の見本を
///   `.sai/template/` に置く（無いものだけ）。
/// - `.sai/template/domain.md` が無ければ、選んだ分野の中身をそのまま書く。
///   あれば上書きせず、変え方を案内する。
///
/// # Errors
/// ファイルの読み書きに失敗したとき、または知らない分野のとき。
pub fn install_domain(root: &Path, domain: &str) -> Result<Vec<String>> {
    let template = root.join(".sai/template");
    let mut written = 0;
    for (_, name, content) in DOMAIN_SAMPLES {
        written += usize::from(write_if_missing(&template.join(name), content)?);
    }
    let content = domain_sample(domain).with_context(|| format!("知らない分野です: {domain}"))?;

    let mut done = Vec::new();
    done.push(format!(
        "分野の観点の見本を {written} 件置きました（既にあるものはそのまま）"
    ));
    if write_if_missing(&template.join("domain.md"), content)? {
        done.push(format!("分野の観点: {domain}（.sai/template/domain.md）"));
    } else {
        done.push(format!(
            "domain.md は既にあります。分野を変えるには .sai/template/domain.md を消してから `sai-paper init --assets-only --domain {domain}` を実行するか、`domain-{domain}.md` の中身を domain.md に写してください"
        ));
    }
    Ok(done)
}

/// この PC の拡張を探して配る（`sai-paper init` から呼ぶ）。
///
/// # Errors
/// ファイルの読み書きに失敗したとき。
pub fn install_here(root: &Path) -> Result<Vec<String>> {
    let bin = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_else(|_| PathBuf::from("sai-moon-paper"));
    install(root, &bin, &pi_extensions())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 議論の参加者（`paper-debater` / `paper-reviewer`）のブロック名。
    ///
    /// この 2 つは司会（`paper-debate`）から子として呼ばれ、依頼文に書かれた
    /// 背景だけで短く意見を返す役なので、状況まとめ（`project-brief`）と共通の
    /// 決まり（`research-core`）を意図的に持たない。研究用ブロックの共通検査から外す。
    const DEBATE_PARTICIPANTS: &[&str] = &["paper-debater.md", "paper-reviewer.md"];

    /// 部品を配ると、ブロック・テンプレート・設定・.mcp.json ができる。
    /// 既にある項目とファイルは残り、2 回目も重複しない。
    #[test]
    fn install_writes_assets_and_keeps_existing_entries() {
        let root = std::env::temp_dir().join(format!("sai-paper-assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sai/blocks")).unwrap();
        std::fs::write(root.join(".sai/blocks/paper-search.md"), "利用者が直した").unwrap();
        std::fs::write(
            root.join(".mcp.json"),
            r#"{"mcpServers": {"other": {"command": "x"}}}"#,
        )
        .unwrap();
        let ext = root.join("ext.ts");
        std::fs::write(&ext, "").unwrap();

        for _ in 0..2 {
            install(&root, Path::new("/bin/sai-paper"), std::slice::from_ref(&ext)).unwrap();
        }

        assert_eq!(
            std::fs::read_to_string(root.join(".sai/blocks/paper-search.md")).unwrap(),
            "利用者が直した"
        );
        for (name, _) in BLOCKS {
            assert!(root.join(".sai/blocks").join(name).is_file(), "{name}");
        }
        for (name, content) in TEMPLATES {
            assert!(root.join(".sai/template").join(name).is_file(), "{name}");
            assert!(!content.contains("/Users/"), "{name} に絶対パスが無い");
        }
        for (name, content) in BLOCKS {
            assert!(!content.contains("/Users/"), "{name} に絶対パスが無い");
        }
        let settings = read_json(&root.join(".sai/settings.json")).unwrap();
        assert_eq!(settings["extensions"].as_array().unwrap().len(), 1);
        assert_eq!(
            settings["pre_run"],
            json!(["/bin/sai-paper", "brief", "--write"]),
            "pre_run を 1 回だけ書く"
        );
        assert!(root.join(".sai/template/project-brief.md").is_file());
        let mcp = read_json(&root.join(".mcp.json")).unwrap();
        assert_eq!(mcp["mcpServers"]["other"]["command"], "x");
        assert_eq!(mcp["mcpServers"]["sai-paper"]["args"][0], "mcp");
        std::fs::remove_dir_all(&root).ok();
    }

    /// 既に `pre_run` があるときは、その設定をそのまま残す。
    #[test]
    fn install_keeps_existing_pre_run() {
        let root = std::env::temp_dir().join(format!("sai-paper-pre-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sai")).unwrap();
        std::fs::write(
            root.join(".sai/settings.json"),
            r#"{"pre_run": ["my-command", "--flag"]}"#,
        )
        .unwrap();

        install(&root, Path::new("/bin/sai-paper"), &[]).unwrap();

        let settings = read_json(&root.join(".sai/settings.json")).unwrap();
        assert_eq!(settings["pre_run"], json!(["my-command", "--flag"]));
        std::fs::remove_dir_all(&root).ok();
    }

    /// 分野の観点の見本を 3 つ配り、選んだ分野の中身を `domain.md` に写す。
    /// 既にある `domain.md` は上書きしない。
    #[test]
    fn install_domain_writes_samples_and_selected_domain() {
        let root = std::env::temp_dir().join(format!("sai-paper-domain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sai/template")).unwrap();

        for (domain, _, content) in DOMAIN_SAMPLES {
            assert!(
                content.starts_with("<!-- "),
                "{domain} の先頭が説明のコメントではない"
            );
            for heading in [
                "## 問いを整理するときの枠",
                "## 証拠の強さの見方",
                "## よくある落とし穴",
            ] {
                assert!(content.contains(heading), "{domain} に {heading} が無い");
            }
            assert_eq!(
                content.matches("\n## ").count(),
                3,
                "{domain} の節が 3 つではない"
            );
        }

        let done = install_domain(&root, "medical").unwrap();
        let domain_md = std::fs::read_to_string(root.join(".sai/template/domain.md")).unwrap();
        assert_eq!(domain_md, domain_sample("medical").unwrap());
        assert!(done.iter().any(|line| line.contains("分野の観点: medical")));
        for (_, name, _) in DOMAIN_SAMPLES {
            assert!(root.join(".sai/template").join(name).is_file(), "{name}");
        }

        // 既にある domain.md は上書きせず、変え方を案内する
        let done = install_domain(&root, "signal-ml").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join(".sai/template/domain.md")).unwrap(),
            domain_md
        );
        assert!(
            done.iter()
                .any(|line| line.contains("domain.md は既にあります"))
        );

        // 知らない分野はエラー
        assert!(install_domain(&root, "nope").is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    /// 研究用のブロックはすべて、`templates:` に状況まとめ（`project-brief`）と
    /// 分野の観点（`domain`）を含む。ただし `paper-template-maker` だけは分野に依存しない。
    #[test]
    fn research_blocks_declare_brief_and_domain() {
        for (name, content) in BLOCKS {
            if DEBATE_PARTICIPANTS.contains(name) {
                continue;
            }
            let frontmatter = content
                .strip_prefix("---\n")
                .and_then(|rest| rest.split("\n---\n").next())
                .unwrap_or_else(|| panic!("{name} に frontmatter が無い"));
            assert!(
                frontmatter.contains("  - project-brief\n"),
                "{name} の templates に project-brief が無い"
            );
            if *name == "paper-template-maker.md" {
                continue;
            }
            assert!(
                frontmatter.contains("  - domain\n"),
                "{name} の templates に domain が無い"
            );
        }
    }

    /// 研究用のブロックは、状況まとめと共通の決まりを先頭で受け取り、
    /// 研究向けの review 見出しを持つ。
    #[test]
    fn research_blocks_declare_brief_and_review_headings() {
        for (name, content) in BLOCKS {
            if DEBATE_PARTICIPANTS.contains(name) {
                continue;
            }
            assert!(
                content.contains("templates:\n  - project-brief\n  - research-core\n"),
                "{name} の templates の先頭が project-brief, research-core ではない"
            );
            assert!(
                content.contains(
                    "review_headings:\n  - 納得した点\n  - 疑問に思った点\n  - 次に知りたいこと\n"
                ),
                "{name} に研究向けの review_headings が無い"
            );
            let frontmatter = content
                .strip_prefix("---\n")
                .and_then(|rest| rest.split("\n---\n").next())
                .unwrap_or_else(|| panic!("{name} に frontmatter が無い"));
            assert!(
                !frontmatter.contains("litbase"),
                "{name} の frontmatter に廃止した litbase が残っている"
            );
        }
    }

    /// 議論の参加者（`paper-debater` / `paper-reviewer`）は、司会から子として
    /// 呼ばれる短い意見役なので、状況まとめ（`project-brief`）と共通の決まり
    /// （`research-core`）を持たない。議題に必要な背景だけを司会が依頼文に書く。
    #[test]
    fn debate_participants_declare_neither_brief_nor_core() {
        for name in DEBATE_PARTICIPANTS {
            let (_, content) = BLOCKS
                .iter()
                .find(|(block, _)| block == name)
                .unwrap_or_else(|| panic!("{name} が BLOCKS に無い"));
            assert!(
                !content.contains("  - project-brief\n"),
                "{name} の templates に project-brief がある"
            );
            assert!(
                !content.contains("  - research-core\n"),
                "{name} の templates に research-core がある"
            );
        }
    }
}
