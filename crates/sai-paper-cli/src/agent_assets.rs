//! `sai-moon-paper init` が研究プロジェクトに配るエージェントの部品（段階 7）。
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

/// sai-sci のブロック 16 本（C-16 の表の順）。
///
/// 研究方針の木で動く 3 類（outline / detail / support）。どれも同じ共通の決まり
/// （`sci-core`）を先頭で受け、起動直前に範囲（`context: true`）を作ってから動く。
const SCI_BLOCKS: &[(&str, &str)] = &[
    ("sci-start.md", include_str!("../../../sai-sci/blocks/sci-start.md")),
    ("sci-outline.md", include_str!("../../../sai-sci/blocks/sci-outline.md")),
    ("sci-digest.md", include_str!("../../../sai-sci/blocks/sci-digest.md")),
    ("sci-seed.md", include_str!("../../../sai-sci/blocks/sci-seed.md")),
    ("sci-read.md", include_str!("../../../sai-sci/blocks/sci-read.md")),
    ("sci-compare.md", include_str!("../../../sai-sci/blocks/sci-compare.md")),
    ("sci-probe.md", include_str!("../../../sai-sci/blocks/sci-probe.md")),
    ("sci-draft.md", include_str!("../../../sai-sci/blocks/sci-draft.md")),
    ("sci-ideate.md", include_str!("../../../sai-sci/blocks/sci-ideate.md")),
    ("sci-debate.md", include_str!("../../../sai-sci/blocks/sci-debate.md")),
    ("sci-coach.md", include_str!("../../../sai-sci/blocks/sci-coach.md")),
    ("sci-search.md", include_str!("../../../sai-sci/blocks/sci-search.md")),
    ("sci-screen.md", include_str!("../../../sai-sci/blocks/sci-screen.md")),
    ("sci-extract.md", include_str!("../../../sai-sci/blocks/sci-extract.md")),
    ("sci-lookup.md", include_str!("../../../sai-sci/blocks/sci-lookup.md")),
    ("sci-analogy.md", include_str!("../../../sai-sci/blocks/sci-analogy.md")),
];

/// sai-sci の共通の決まり（`sci-core.md`）。
const SCI_TEMPLATES: &[(&str, &str)] =
    &[("sci-core.md", include_str!("../../../sai-sci/template/sci-core.md"))];

/// sai-sci の土壌（`.sai/paper/mind.md`）のひな形。
///
/// 研究方針の木が無いプロジェクトの `mind.md` は、今の `project_files::MIND_TEMPLATE` の
/// 代わりにこれを書く（決まったこと・仮説は木へ移すので、「土壌」の形が合う）。
const SCI_MIND: &str = include_str!("../../../sai-sci/mind.md");

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
/// sai-link はテンプレートが見つからないと実行の前に止まるため、`sai-moon-paper init`
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

/// 分野の観点を配る（`sai-moon-paper init --domain <分野>` から呼ぶ）。
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
            "domain.md は既にあります。分野を変えるには .sai/template/domain.md を消してから `sai-moon-paper init --assets-only --domain {domain}` を実行するか、`domain-{domain}.md` の中身を domain.md に写してください"
        ));
    }
    Ok(done)
}

/// この PC の拡張を探して配る（`sai-moon-paper init` から呼ぶ）。
///
/// # Errors
/// ファイルの読み書きに失敗したとき。
pub fn install_here(root: &Path) -> Result<Vec<String>> {
    let bin = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_else(|_| PathBuf::from("sai-moon-paper"));
    install(root, &bin, &pi_extensions())
}

/// sci の部品を研究プロジェクト（`root`）に配る。何をしたかの一覧を返す（表示用）。
///
/// 1. 今の部品（[`install`]）: sci ブロックが使うテンプレートと議論の参加者が
///    無ければ sai-link が止まるため、必ず先に配る。
/// 2. 分野の観点: `.sai/template/domain.md` が無いときだけ `general` を配る。
/// 3. sci ブロック 16 本と sci-core（無いものだけ）。
/// 4. `.sai/settings.json` の `context`（無いときだけ）。
/// 5. `.sai/paper/mind.md`（無いときだけ）: 土壌のひな形。
///
/// 既にあるものは上書きしない（利用者が直したファイルを消さない）。
///
/// # Errors
/// ファイルの読み書きに失敗したとき。
pub fn install_sci(root: &Path, sai_paper_bin: &Path, extensions: &[PathBuf]) -> Result<Vec<String>> {
    let mut done = install(root, sai_paper_bin, extensions)?;
    let sai = root.join(".sai");

    // 分野の観点: sci ブロックはすべて `domain` を使うので、無ければ普通の分野を配る
    // （`init --domain` で別の分野を選んだプロジェクトは既に domain.md がある）。
    if !sai.join("template/domain.md").exists() {
        done.extend(install_domain(root, "general")?);
    }
    done.extend(install_sci_parts(root, sai_paper_bin)?);
    Ok(done)
}

/// sci だけの部品（sci ブロック 16 本と sci-core・`context`・土壌のひな形）を配る。
/// 今の部品と分野の観点は配らない（`init` は先に [`install_here`] と [`install_domain`] を呼ぶので、
/// 同じ行を 2 度出さないためにこちらを使う）。
fn install_sci_parts(root: &Path, sai_paper_bin: &Path) -> Result<Vec<String>> {
    let mut done = Vec::new();
    let sai = root.join(".sai");
    let mut written = 0;
    for (name, content) in SCI_BLOCKS {
        written += usize::from(write_if_missing(&sai.join("blocks").join(name), content)?);
    }
    for (name, content) in SCI_TEMPLATES {
        written += usize::from(write_if_missing(&sai.join("template").join(name), content)?);
    }
    done.push(format!(
        "sci のブロックとテンプレートを {written} 件置きました（既にあるものはそのまま）"
    ));

    // 実行直前に研究方針の木から範囲を作る命令（`{run_id}` は sai-link が展開する）。
    // 絶対パスは PC ごとに違うので、ブロック定義ではなく設定に書く（`pre_run` と同じ置き方）。
    let settings_path = sai.join("settings.json");
    let mut settings = read_json(&settings_path)?;
    let obj = settings
        .as_object_mut()
        .context(".sai/settings.json がオブジェクトではありません")?;
    if !obj.contains_key("context") {
        obj.insert(
            "context".to_string(),
            json!([sai_paper_bin.to_string_lossy(), "sci", "context", "--run", "{run_id}"]),
        );
        write_json(&settings_path, &settings)?;
        done.push(
            "context（実行直前に研究方針の木から範囲を作る）を .sai/settings.json に登録しました"
                .to_string(),
        );
    }

    // 土壌のひな形（既にある mind.md は利用者の考えなので、そのままにする）
    if write_if_missing(&sai.join("paper/mind.md"), SCI_MIND)? {
        done.push("mind.md（土壌のひな形）を作りました".to_string());
    }
    Ok(done)
}

/// この PC の実行ファイルと拡張を探して sci の部品を配る
/// （`sci init`・`phone sci init`・`sai-moon-paper init --sci` から呼ぶ）。
///
/// # Errors
/// ファイルの読み書きに失敗したとき。
pub fn install_sci_here(root: &Path) -> Result<Vec<String>> {
    let bin = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_else(|_| PathBuf::from("sai-moon-paper"));
    install_sci(root, &bin, &pi_extensions())
}

/// この PC の実行ファイルで [`install_sci_parts`] を呼ぶ（`init --sci` 用）。
pub fn install_sci_parts_here(root: &Path) -> Result<Vec<String>> {
    let bin = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_else(|_| PathBuf::from("sai-moon-paper"));
    install_sci_parts(root, &bin)
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

    /// sci の部品を配ると、sci ブロック 16 本・sci-core・分野の観点・`context` の
    /// 設定・土壌のひな形（mind.md）がそろう。今の部品も一緒に配る。
    #[test]
    fn install_sci_writes_blocks_templates_context_and_mind() {
        let root = std::env::temp_dir().join(format!("sai-paper-sci-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sai")).unwrap();
        let ext = root.join("ext.ts");
        std::fs::write(&ext, "").unwrap();
        let bin = Path::new("/opt/bin/sai-moon-paper");

        let done = install_sci(&root, bin, std::slice::from_ref(&ext)).unwrap();

        // sci のブロック 16 本と、今の部品 17 本もそろう
        let sci_blocks = std::fs::read_dir(root.join(".sai/blocks"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("sci-"))
            .count();
        assert_eq!(SCI_BLOCKS.len(), 16, "SCI_BLOCKS が 16 本でない");
        assert_eq!(sci_blocks, 16, "配った sci のブロックが 16 本でない");
        for (name, _) in SCI_BLOCKS {
            assert!(root.join(".sai/blocks").join(name).is_file(), "{name}");
        }
        for (name, _) in BLOCKS {
            assert!(root.join(".sai/blocks").join(name).is_file(), "今の部品の {name} が無い");
        }
        // 共通の決まりと分野の観点
        assert!(root.join(".sai/template/sci-core.md").is_file());
        assert_eq!(
            std::fs::read_to_string(root.join(".sai/template/sci-core.md")).unwrap(),
            SCI_TEMPLATES[0].1
        );
        assert!(root.join(".sai/template/domain.md").is_file());
        assert!(root.join(".sai/template/literature-hard-rules.md").is_file());

        // 実行直前に範囲を作る命令（実行ファイルの絶対パス + `sci context --run`）
        let settings = read_json(&root.join(".sai/settings.json")).unwrap();
        assert_eq!(
            settings["context"],
            json!(["/opt/bin/sai-moon-paper", "sci", "context", "--run", "{run_id}"]),
            "context が実行ファイルの絶対パスと sci context --run の設定ではない"
        );
        // 今からある pre_run と拡張は、そのまま残す
        assert_eq!(settings["pre_run"], json!(["/opt/bin/sai-moon-paper", "brief", "--write"]));
        assert_eq!(settings["extensions"].as_array().unwrap().len(), 1);

        // 土壌のひな形
        assert_eq!(
            std::fs::read_to_string(root.join(".sai/paper/mind.md")).unwrap(),
            SCI_MIND
        );
        assert!(
            done.iter().any(|line| line.contains("sci のブロックとテンプレートを")),
            "部品を置いた行が無い: {done:?}"
        );
        assert!(
            done.iter()
                .any(|line| line.contains("context（実行直前に研究方針の木から範囲を作る）")),
            "context を登録した行が無い: {done:?}"
        );
        assert!(
            done.iter().any(|line| line.contains("mind.md（土壌のひな形）を作りました")),
            "mind.md を作った行が無い: {done:?}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// 直したブロック・既にある `context`・既にある mind.md・`pre_run`・`domain.md` は
    /// 上書きしない。2 回目も既に置いたものを動かさない。
    #[test]
    fn install_sci_keeps_user_files() {
        let root = std::env::temp_dir().join(format!("sai-paper-sci-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sai/blocks")).unwrap();
        std::fs::create_dir_all(root.join(".sai/paper")).unwrap();
        std::fs::create_dir_all(root.join(".sai/template")).unwrap();
        std::fs::write(root.join(".sai/blocks/sci-compare.md"), "利用者が直した").unwrap();
        std::fs::write(root.join(".sai/paper/mind.md"), "利用者が書いた土壌").unwrap();
        std::fs::write(root.join(".sai/template/domain.md"), "利用者が直した分野").unwrap();
        std::fs::write(
            root.join(".sai/settings.json"),
            r#"{"pre_run": ["my-command"], "context": ["my-range-maker"]}"#,
        )
        .unwrap();
        let bin = Path::new("/opt/bin/sai-moon-paper");

        install_sci(&root, bin, &[]).unwrap();
        // 2 回目は、既に置いた sci のブロックを動かさない。
        let before = std::fs::read_to_string(root.join(".sai/blocks/sci-start.md")).unwrap();
        install_sci(&root, bin, &[]).unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join(".sai/blocks/sci-compare.md")).unwrap(),
            "利用者が直した"
        );
        assert_eq!(
            std::fs::read_to_string(root.join(".sai/paper/mind.md")).unwrap(),
            "利用者が書いた土壌"
        );
        assert_eq!(
            std::fs::read_to_string(root.join(".sai/template/domain.md")).unwrap(),
            "利用者が直した分野"
        );
        let settings = read_json(&root.join(".sai/settings.json")).unwrap();
        assert_eq!(settings["context"], json!(["my-range-maker"]));
        assert_eq!(settings["pre_run"], json!(["my-command"]));
        assert_eq!(
            before,
            std::fs::read_to_string(root.join(".sai/blocks/sci-start.md")).unwrap()
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// sci のブロック 16 本は、全部 共通の決まり（sci-core）を先頭で受け、
    /// 状況まとめ（project-brief）を待たず、起動直前に範囲を作り、
    /// 研究向けの review 見出しを持つ。
    #[test]
    fn sci_blocks_declare_sci_core_and_context() {
        assert_eq!(SCI_BLOCKS.len(), 16);
        for id in [
            "sci-analogy.md",
            "sci-coach.md",
            "sci-compare.md",
            "sci-debate.md",
            "sci-digest.md",
            "sci-draft.md",
            "sci-extract.md",
            "sci-ideate.md",
            "sci-lookup.md",
            "sci-outline.md",
            "sci-probe.md",
            "sci-read.md",
            "sci-screen.md",
            "sci-search.md",
            "sci-seed.md",
            "sci-start.md",
        ] {
            assert!(
                SCI_BLOCKS.iter().any(|(name, _)| *name == id),
                "{id} が SCI_BLOCKS に無い"
            );
        }

        for (name, content) in SCI_BLOCKS {
            let frontmatter = content
                .strip_prefix("---\n")
                .and_then(|rest| rest.split("\n---\n").next())
                .unwrap_or_else(|| panic!("{name} に frontmatter が無い"));
            assert!(
                frontmatter.contains("templates:\n  - sci-core\n"),
                "{name} の templates の先頭が sci-core ではない"
            );
            assert!(
                !frontmatter.contains("project-brief"),
                "{name} の templates に project-brief がある"
            );
            assert!(
                frontmatter.contains("\ncontext: true\n"),
                "{name} に context: true が無い"
            );
            assert!(frontmatter.contains("model: inherit"), "{name} の model が inherit ではない");
            assert!(frontmatter.contains("worktree: none"), "{name} の worktree が none ではない");
            assert!(frontmatter.contains("enable: true"), "{name} が有効になっていない");
            assert!(
                content.contains(
                    "review_headings:\n  - 反映したこと\n  - 反映しなかったこと（理由）\n  - 次に知りたいこと\n"
                ),
                "{name} に研究向けの review_headings が無い"
            );
            assert!(frontmatter.contains("\nsci_ops: "), "{name} に sci_ops が無い");
            assert!(frontmatter.contains("\nsci_scope: "), "{name} に sci_scope が無い");
            assert!(!content.contains("/Users/"), "{name} に絶対パスがある");
        }
    }
}
