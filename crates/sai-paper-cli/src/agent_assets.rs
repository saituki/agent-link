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

/// sai-sci のブロック 29 本。並びは Action の種類ごと:
/// 役 4 本（professor・professor-backward・operator・kaizen）→ Q を分ける（start・split・restore・typify）
/// → O を書く（ideate・analogy）→ F を探す（search・screen・read・lookup・extract・cite-check）
/// → Logicality（debate・review）→ ★ の手伝い（compare・seed）→ その他（digest・coach）
/// → 実験（exp-scout・sim-model・sim-compose・exp-sprout・exp-branch・exp-review・result-compare）。
///
/// sci ブロックは同じ共通の決まり（`sci-core`）を先頭で受け、起動直前に範囲（`context: true`）を作ってから動く。
/// frontmatter に Action の種類（`sci_action`）・support か（`sci_support`）・完了基準（`sci_done_agent`）を持つ。
///
/// ただし実験ループの `sci-exp-branch` は例外で、判定をしないので木の範囲も共通の決まりも要らない
/// （受け取るのは実験ループの共通の決まり `exp-core` だけ）。
///
/// `professor`・`professor-backward`・`operator`・`kaizen` は木の中で動かず、木の外から木を見る役。
/// 共通決まり（`sci-core`）も範囲（`context: true`）も持たず、研究プロジェクト向けの
/// 共通の決まり（`sai-agent` / `sai-task` / `sai-cli`）を受ける。
const SCI_BLOCKS: &[(&str, &str)] = &[
    ("professor.md", include_str!("../../../sai-sci/blocks/professor.md")),
    ("professor-backward.md", include_str!("../../../sai-sci/blocks/professor-backward.md")),
    ("operator.md", include_str!("../../../sai-sci/blocks/operator.md")),
    ("kaizen.md", include_str!("../../../sai-sci/blocks/kaizen.md")),
    ("sci-start.md", include_str!("../../../sai-sci/blocks/sci-start.md")),
    ("sci-split.md", include_str!("../../../sai-sci/blocks/sci-split.md")),
    ("sci-restore.md", include_str!("../../../sai-sci/blocks/sci-restore.md")),
    ("sci-typify.md", include_str!("../../../sai-sci/blocks/sci-typify.md")),
    ("sci-ideate.md", include_str!("../../../sai-sci/blocks/sci-ideate.md")),
    ("sci-analogy.md", include_str!("../../../sai-sci/blocks/sci-analogy.md")),
    ("sci-search.md", include_str!("../../../sai-sci/blocks/sci-search.md")),
    ("sci-screen.md", include_str!("../../../sai-sci/blocks/sci-screen.md")),
    ("sci-read.md", include_str!("../../../sai-sci/blocks/sci-read.md")),
    ("sci-lookup.md", include_str!("../../../sai-sci/blocks/sci-lookup.md")),
    ("sci-extract.md", include_str!("../../../sai-sci/blocks/sci-extract.md")),
    ("sci-cite-check.md", include_str!("../../../sai-sci/blocks/sci-cite-check.md")),
    ("sci-debate.md", include_str!("../../../sai-sci/blocks/sci-debate.md")),
    ("sci-review.md", include_str!("../../../sai-sci/blocks/sci-review.md")),
    ("sci-compare.md", include_str!("../../../sai-sci/blocks/sci-compare.md")),
    ("sci-seed.md", include_str!("../../../sai-sci/blocks/sci-seed.md")),
    ("sci-digest.md", include_str!("../../../sai-sci/blocks/sci-digest.md")),
    ("sci-coach.md", include_str!("../../../sai-sci/blocks/sci-coach.md")),
    ("sci-exp-scout.md", include_str!("../../../sai-sci/blocks/sci-exp-scout.md")),
    ("sci-sim-model.md", include_str!("../../../sai-sci/blocks/sci-sim-model.md")),
    ("sci-sim-compose.md", include_str!("../../../sai-sci/blocks/sci-sim-compose.md")),
    ("sci-exp-sprout.md", include_str!("../../../sai-sci/blocks/sci-exp-sprout.md")),
    ("sci-exp-branch.md", include_str!("../../../sai-sci/blocks/sci-exp-branch.md")),
    ("sci-exp-review.md", include_str!("../../../sai-sci/blocks/sci-exp-review.md")),
    ("sci-result-compare.md", include_str!("../../../sai-sci/blocks/sci-result-compare.md")),
];

/// sai-sci の共通の決まり。
///
/// - 木の共通の決まり（`sci-core.md`）・F を探す決まり（`sci-find.md`）・照合の決まり（`sci-match.md`）と実験ループの 4 本（`exp-core.md`・`exp-run.md`・`exp-ref.md`・`sim-core.md`）
/// - 研究プロジェクトに配る 3 本（`sai-agent.md`・`sai-task.md`・`sai-cli.md`。U8）。
///   前の 3 本は researcher が使う。後の 3 本は `professor` などの役が使う
///   （引き継ぎ・質問・子タスク・命令）。
/// - 逆向き（backward）の共通の決まり（`bw-core.md`）。逆向きのブロックと `professor-backward` が使う。
const SCI_TEMPLATES: &[(&str, &str)] = &[
    ("sci-core.md", include_str!("../../../sai-sci/template/sci-core.md")),
    ("sci-find.md", include_str!("../../../sai-sci/template/sci-find.md")),
    ("sci-match.md", include_str!("../../../sai-sci/template/sci-match.md")),
    ("exp-core.md", include_str!("../../../sai-sci/template/exp-core.md")),
    ("exp-run.md", include_str!("../../../sai-sci/template/exp-run.md")),
    ("exp-ref.md", include_str!("../../../sai-sci/template/exp-ref.md")),
    ("sim-core.md", include_str!("../../../sai-sci/template/sim-core.md")),
    ("sai-agent.md", include_str!("../../../sai-sci/template/sai-agent.md")),
    ("sai-task.md", include_str!("../../../sai-sci/template/sai-task.md")),
    ("sai-cli.md", include_str!("../../../sai-sci/template/sai-cli.md")),
    ("bw-core.md", include_str!("../../../sai-sci/template/bw-core.md")),
];

/// sai-sci の道具（`.sai/tools/` に置く。実験ループの台帳、実行先へ投げる分配器、
/// 参照コードの道具と ModelDB のカタログ）。
///
/// ブロックは `$SAI_PROJECT_ROOT/.sai/tools/<名前>` で呼ぶ（環境変数は要らない）。
const SCI_TOOLS: &[(&str, &str)] = &[
    ("exp_summary.py", include_str!("../../../sai-sci/tools/exp_summary.py")),
    ("exp_exec.py", include_str!("../../../sai-sci/tools/exp_exec.py")),
    ("phone_api.py", include_str!("../../../sai-sci/tools/phone_api.py")),
    ("refcode.py", include_str!("../../../sai-sci/tools/refcode.py")),
    ("modeldb.py", include_str!("../../../sai-sci/tools/modeldb.py")),
];

/// sai-sci の実行先の見本（`.sai/exec/` に置く）。3 つ目は「実行の権限を付けるか」。
///
/// 実行先はフォルダ 1 つ。`local` は有効（この PC）、`ssh`（Linux の PC）と `dok`（さくらの高火力 DOK・課金あり）は
/// 無効の見本（`ssh` は写して host を書く）、
/// `_example` は雛形（下線始まりなので使われない）。
/// 他の実行先（LAN の PC・クラウド）は利用者が `.sai/exec/<名前>/` を足す。
const SCI_EXEC: &[(&str, &str, bool)] = &[
    ("local/target.toml", include_str!("../../../sai-sci/exec/local/target.toml"), false),
    ("local/adapter", include_str!("../../../sai-sci/exec/local/adapter"), true),
    ("ssh/target.toml", include_str!("../../../sai-sci/exec/ssh/target.toml"), false),
    ("ssh/adapter", include_str!("../../../sai-sci/exec/ssh/adapter"), true),
    ("ssh/runner.sh", include_str!("../../../sai-sci/exec/ssh/runner.sh"), true),
    ("dok/target.toml", include_str!("../../../sai-sci/exec/dok/target.toml"), false),
    ("dok/adapter", include_str!("../../../sai-sci/exec/dok/adapter"), true),
    ("dok/dok.py", include_str!("../../../sai-sci/exec/dok/dok.py"), true),
    ("dok/README.md", include_str!("../../../sai-sci/exec/dok/README.md"), false),
    ("dok/SETUP.md", include_str!("../../../sai-sci/exec/dok/SETUP.md"), false),
    ("_example/target.toml", include_str!("../../../sai-sci/exec/_example/target.toml"), false),
    ("_example/adapter", include_str!("../../../sai-sci/exec/_example/adapter"), true),
];

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
/// の時点で必ずこのファイルを作る。`sai-moon-paper brief --write` が本物のまとめで
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

/// 実行の権限を付ける（実行先の adapter は分配器が直接起動するため）。Unix 以外では何もしない。
fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
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
    // `project-brief.md` は毎回 `sai-moon-paper brief --write` が上書きするが、
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
/// 3. sci ブロック 18 本と共通の決まり（無いものだけ）。
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

/// sci だけの部品（sci ブロック 18 本と共通の決まり・`context`・土壌のひな形）を配る。
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
    for (name, content) in SCI_TOOLS {
        written += usize::from(write_if_missing(&sai.join("tools").join(name), content)?);
    }
    for (name, content, executable) in SCI_EXEC {
        let path = sai.join("exec").join(name);
        let wrote = write_if_missing(&path, content)?;
        written += usize::from(wrote);
        if wrote && *executable {
            make_executable(&path)?;
        }
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
            "context（実行直前に研究方針の木から SCOPE を作る）を .sai/settings.json に登録しました"
                .to_string(),
        );
    }

    // 保存前の検査（U01 の `sai_write` が呼ぶ。キーが無いときだけ書く）。
    let obj = settings
        .as_object_mut()
        .context(".sai/settings.json がオブジェクトではありません")?;
    if !obj.contains_key("write_check") {
        obj.insert(
            "write_check".to_string(),
            json!([
                sai_paper_bin.to_string_lossy(),
                "report",
                "prepare",
                "--run",
                "{run_id}",
                "--block",
                "{block}",
                "--filename",
                "{filename}"
            ]),
        );
        write_json(&settings_path, &settings)?;
        done.push("write_check（保存前の検査）を .sai/settings.json に登録しました".to_string());
    }

    // 土壌のひな形（既にある mind.md は利用者の考えなので、そのままにする）
    if write_if_missing(&sai.join("paper/mind.md"), SCI_MIND)? {
        done.push("mind.md（SOIL のひな形）を作りました".to_string());
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

    /// 木の範囲も共通の決まり（sci-core）も受け取らない 1 本（実験の実行役）。
    const EXP_WITHOUT_SCI_CORE: &str = "sci-exp-branch.md";

    /// 木の中で動かず、外から木を見る 4 本（professor・professor-backward・operator・kaizen）。
    ///
    /// この 4 本は `sci-core` も `context: true` も持たない。節点の範囲を作ってから動く
    /// のではなく、木そのものを見る窓口だから。共通の決まりは研究向けの 3 本
    /// （`sai-agent` / `sai-task` / `sai-cli`）を受ける。
    const SCI_BLOCKS_WITHOUT_SCI_CORE: &[&str] =
        &[EXP_WITHOUT_SCI_CORE, "professor.md", "professor-backward.md", "operator.md", "kaizen.md"];

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
    /// review の見出し（廃止した review.md 用）を持たない。
    #[test]
    fn research_blocks_declare_brief_without_review_headings() {
        for (name, content) in BLOCKS {
            if DEBATE_PARTICIPANTS.contains(name) {
                continue;
            }
            assert!(
                content.contains("templates:\n  - project-brief\n  - research-core\n"),
                "{name} の templates の先頭が project-brief, research-core ではない"
            );
            assert!(!content.contains("review_headings"), "{name} に review_headings が残っている");
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

    /// sci の部品を配ると、sci ブロック・共通の決まり・分野の観点・`context` の
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

        // sci のブロック 29 本と、今の部品 17 本もそろう
        let sci_blocks = std::fs::read_dir(root.join(".sai/blocks"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("sci-"))
            .count();
        assert_eq!(SCI_BLOCKS.len(), 29, "SCI_BLOCKS が 29 本でない");
        assert_eq!(SCI_TEMPLATES.len(), 11, "SCI_TEMPLATES が 11 本でない");
        assert_eq!(sci_blocks, 25, "配った sci のブロックが 25 本でない");
        for (name, _) in SCI_BLOCKS {
            assert!(root.join(".sai/blocks").join(name).is_file(), "{name}");
        }
        for (name, _) in BLOCKS {
            assert!(root.join(".sai/blocks").join(name).is_file(), "今の部品の {name} が無い");
        }
        // 共通の決まりと分野の観点
        assert!(root.join(".sai/template/sci-core.md").is_file());
        for (name, content) in SCI_TEMPLATES {
            let path = root.join(".sai/template").join(name);
            assert!(path.is_file(), "{name} が配られていない");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), *content, "{name} の中身が違う");
        }
        assert!(root.join(".sai/template/domain.md").is_file());
        assert!(root.join(".sai/template/literature-hard-rules.md").is_file());

        // U8: professor と operator のブロックと研究向けの共通 template 3 本、Web と同じ操作の道具。
        for name in ["professor.md", "operator.md"] {
            assert!(root.join(".sai/blocks").join(name).is_file(), "{name} が配られていない");
        }
        for name in ["sai-agent.md", "sai-task.md", "sai-cli.md"] {
            assert!(root.join(".sai/template").join(name).is_file(), "{name} が配られていない");
        }
        assert!(root.join(".sai/tools/phone_api.py").is_file(), "phone_api.py が配られていない");

        // 実験ループの台帳は `.sai/tools/` に置く。ブロックはこの名前で呼ぶ
        for (name, content) in SCI_TOOLS {
            let path = root.join(".sai/tools").join(name);
            assert!(path.is_file(), "{name} が配られていない");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), *content, "{name} の中身が違う");
        }
        for (name, content) in SCI_BLOCKS.iter().chain(SCI_TEMPLATES.iter()) {
            for old in ["EXP_SUMMARY_PY", "EXP_SSH_HOST", "sai-ssh"] {
                assert!(!content.contains(old), "{name} に廃止した {old} が残っている");
            }
        }
        // 実行先の見本（local は有効・adapter は実行の権限つき）
        for (name, content, executable) in SCI_EXEC {
            let path = root.join(".sai/exec").join(name);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), *content, "{name} の中身が違う");
            #[cfg(unix)]
            if *executable {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path).unwrap().permissions().mode();
                assert!(mode & 0o111 != 0, "{name} に実行の権限が無い");
            }
        }
        for name in ["sci-exp-review.md", "exp-core.md"] {
            let content = SCI_BLOCKS
                .iter()
                .chain(SCI_TEMPLATES.iter())
                .find(|(n, _)| *n == name)
                .map(|(_, c)| *c)
                .unwrap_or_else(|| panic!("{name} が無い"));
            assert!(
                content.contains("$SAI_PROJECT_ROOT/.sai/tools/exp_summary.py"),
                "{name} が台帳の置き場を示していない"
            );
        }

        // 実行直前に範囲を作る命令（実行ファイルの絶対パス + `sci context --run`）
        let settings = read_json(&root.join(".sai/settings.json")).unwrap();
        assert_eq!(
            settings["context"],
            json!(["/opt/bin/sai-moon-paper", "sci", "context", "--run", "{run_id}"]),
            "context が実行ファイルの絶対パスと sci context --run の設定ではない"
        );
        assert_eq!(
            settings["write_check"],
            json!(["/opt/bin/sai-moon-paper", "report", "prepare", "--run", "{run_id}", "--block", "{block}", "--filename", "{filename}"]),
        );
        // 既にある write_check は上書きしない
        let mut custom = settings.clone();
        custom["write_check"] = json!(["mine"]);
        write_json(&root.join(".sai/settings.json"), &custom).unwrap();
        install_sci(&root, bin, std::slice::from_ref(&ext)).unwrap();
        let again = read_json(&root.join(".sai/settings.json")).unwrap();
        assert_eq!(again["write_check"], json!(["mine"]));
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
                .any(|line| line.contains("context（実行直前に研究方針の木から SCOPE を作る）")),
            "context を登録した行が無い: {done:?}"
        );
        assert!(
            done.iter().any(|line| line.contains("mind.md（SOIL のひな形）を作りました")),
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

    /// sci ブロック 25 本はすべて、Action の種類（`sci_action`）・`sci_support`・完了の担い手
    /// （`sci_done_agent`）を持ち、`layer` は持たない。support は 6 本だけ。
    #[test]
    fn sci_blocks_declare_action_and_criteria() {
        let mut support = Vec::new();
        let mut count = 0;
        for (name, content) in SCI_BLOCKS {
            if !name.starts_with("sci-") {
                continue;
            }
            count += 1;
            let frontmatter = content
                .strip_prefix("---\n")
                .and_then(|rest| rest.split("\n---\n").next())
                .unwrap_or_else(|| panic!("{name} に frontmatter が無い"));
            let action = frontmatter
                .lines()
                .find_map(|line| line.strip_prefix("sci_action: "))
                .unwrap_or_else(|| panic!("{name} に sci_action が無い"));
            assert!(
                ["split", "write_o", "find_f", "check_logic", "choose", "experiment", "other"]
                    .contains(&action.trim()),
                "{name} の sci_action「{action}」が決まりに無い"
            );
            assert!(frontmatter.contains("\nsci_done_agent:"), "{name} に sci_done_agent が無い");
            assert!(!frontmatter.contains("\nlayer:"), "{name} に layer が残っている");
            match frontmatter.lines().find_map(|line| line.strip_prefix("sci_support: ")) {
                Some("true") => support.push(*name),
                Some("false") => {}
                other => panic!("{name} の sci_support が true/false でない: {other:?}"),
            }
        }
        assert_eq!(count, 25);
        assert_eq!(
            support,
            [
                "sci-analogy.md",
                "sci-lookup.md",
                "sci-extract.md",
                "sci-exp-scout.md",
                "sci-sim-model.md",
                "sci-sim-compose.md"
            ]
        );
    }

    /// 2 本の実験ブロックが使うテンプレートは、全部 配る物（テンプレート + 分野の観点）にある。
    /// sai-link はテンプレートが見つからないと実行の前に止めるため、ずれると実行できない。
    ///
    /// U8 で足した `professor` と `operator` も同じ検査に含める。
    #[test]
    fn blocks_use_only_shipped_templates() {
        let mut shipped: Vec<&str> = TEMPLATES.iter().map(|(name, _)| *name).collect();
        shipped.extend(SCI_TEMPLATES.iter().map(|(name, _)| *name));
        // `domain` は `install_domain` が `.sai/template/domain.md` に書く（名前は拡張子無し）。
        shipped.push("domain.md");
        shipped.push("domain");

        for (name, content) in SCI_BLOCKS {
            let frontmatter = content
                .strip_prefix("---\n")
                .and_then(|rest| rest.split("\n---\n").next())
                .unwrap_or_else(|| panic!("{name} に frontmatter が無い"));
            let templates: Vec<&str> = frontmatter
                .lines()
                .skip_while(|line| *line != "templates:")
                .skip(1)
                .take_while(|line| line.trim_start().starts_with("- "))
                .map(|line| line.trim_start().trim_start_matches("- ").trim())
                .collect();
            assert!(!templates.is_empty(), "{name} の templates が空だ");
            for template in templates {
                let found = shipped.contains(&template)
                    || shipped.iter().any(|shipped| {
                        Path::new(shipped).file_stem().is_some_and(|stem| stem == template)
                    });
                assert!(found, "{name} の template「{template}」は配る物の中に無い");
            }
        }
    }

    /// sci のブロック 29 本のうち 24 本（`SCI_BLOCKS_WITHOUT_SCI_CORE` を除く）は、全部 共通の決まり
    /// （sci-core）を先頭で受け、状況まとめ（project-brief）を待たず、起動直前に範囲を作り、
    /// 研究向けの review 見出しを持つ。
    ///
    /// `sci-exp-branch` は判定をしないので木の範囲も共通の決まりも要らない。
    /// `professor` と `operator` は木の中で動かないので、同じく検査から外す（下のテストで別に確かめる）。
    #[test]
    fn sci_blocks_declare_sci_core_and_context() {
        assert_eq!(SCI_BLOCKS.len(), 29);
        for id in [
            "sci-analogy.md",
            "sci-cite-check.md",
            "sci-coach.md",
            "sci-compare.md",
            "sci-debate.md",
            "sci-digest.md",
            "sci-exp-review.md",
            "sci-exp-scout.md",
            "sci-exp-sprout.md",
            "sci-extract.md",
            "sci-ideate.md",
            "sci-lookup.md",
            "sci-read.md",
            "sci-restore.md",
            "sci-result-compare.md",
            "sci-review.md",
            "sci-screen.md",
            "sci-search.md",
            "sci-seed.md",
            "sci-sim-compose.md",
            "sci-sim-model.md",
            "sci-split.md",
            "sci-start.md",
            "sci-typify.md",
        ] {
            assert!(
                SCI_BLOCKS.iter().any(|(name, _)| *name == id),
                "{id} が SCI_BLOCKS に無い"
            );
        }

        let mut checked = 0;
        for (name, content) in SCI_BLOCKS {
            let frontmatter = content
                .strip_prefix("---\n")
                .and_then(|rest| rest.split("\n---\n").next())
                .unwrap_or_else(|| panic!("{name} に frontmatter が無い"));
            if SCI_BLOCKS_WITHOUT_SCI_CORE.contains(name) {
                continue;
            }
            checked += 1;
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
            assert!(!content.contains("review_headings"), "{name} に review_headings が残っている");
            // `sci-typify` は木を書き換えず、型として保存するだけなので sci_ops を持たない。
            assert!(
                *name == "sci-typify.md" || frontmatter.contains("\nsci_ops: "),
                "{name} に sci_ops が無い"
            );
            assert!(frontmatter.contains("\nsci_scope: "), "{name} に sci_scope が無い");
            assert!(!content.contains("/Users/"), "{name} に絶対パスがある");
        }
        assert_eq!(checked, 24, "共通の決まりを確かめたのは 24 本でない");
    }

    /// frontmatter から、リスト項目（`templates:` など）に並ぶ値をまとめて読む。
    fn frontmatter_list<'a>(frontmatter: &'a str, key: &str) -> Vec<&'a str> {
        frontmatter
            .lines()
            .skip_while(|line| *line != key)
            .skip(1)
            .take_while(|line| line.trim_start().starts_with("- "))
            .map(|line| line.trim_start().trim_start_matches("- ").trim())
            .collect()
    }

    /// `professor` と `operator` は、木の中で動かず、外から木を見る役。
    ///
    /// 共通の決まり（sci-core）も範囲（context: true）も持たず、研究向けの 3 本を受ける。
    /// どちらも `id` がファイル名と一致し、呼ぶ子（professor だけ）も配るブロックの中に無い。
    #[test]
    fn shipped_sci_blocks_parse_with_engine_parser() {
        for (name, content) in SCI_BLOCKS {
            let parsed = sai_link_engine::parser::parse_block_string(content)
                .unwrap_or_else(|e| panic!("{name} が parser で読めない: {e}"));
            let id = parsed.config.id.as_deref().unwrap_or_default();
            assert_eq!(format!("{id}.md"), *name, "{name} の id");
        }
    }

    #[test]
    fn professor_and_operator_declare_research_templates() {
        for (name, id, subblocks, headings) in [
            (
                "professor.md",
                "professor",
                true,
                &[
                    "## 役割",
                    "## 研究方針の木の読み方",
                    "## 始めにすること",
                    "## 1 回の回し方",
                    "## 質問に答える",
                    "## 評価の観点",
                    "## 実験（sprout と branch）",
                    "## operator を使う",
                    "## Human に上げる",
                    "## 完了する",
                    "## やってはいけないこと",
                ][..],
            ),
            (
                "professor-backward.md",
                "professor-backward",
                true,
                &[
                    "## 役割",
                    "## 研究方針の木の読み方",
                    "## 始めにすること",
                    "## 段の進め方",
                    "## 1 回の回し方",
                    "## 質問に答える",
                    "## 評価の観点",
                    "## operator を使う",
                    "## Human に上げる",
                    "## 完了する",
                    "## やってはいけないこと",
                ][..],
            ),
            (
                "operator.md",
                "operator",
                false,
                &[
                    "## 役割",
                    "## 始めにすること",
                    "## Human が Web でできることと、あなたの命令",
                    "## よく起きる故障と対処",
                    "## 情報を取り出す",
                    "## 学ぶ",
                    "## 報告",
                    "## やってはいけないこと",
                ][..],
            ),
        ] {
            let content = SCI_BLOCKS
                .iter()
                .find(|(block, _)| *block == name)
                .map(|(_, content)| *content)
                .unwrap_or_else(|| panic!("{name} が SCI_BLOCKS に無い"));
            let frontmatter = content
                .strip_prefix("---\n")
                .and_then(|rest| rest.split("\n---\n").next())
                .unwrap_or_else(|| panic!("{name} に frontmatter が無い"));

            assert!(frontmatter.contains(&format!("\nid: {id}\n")), "{name} の id が {id} ではない");
            assert!(!frontmatter.contains("sci-core"), "{name} の templates に sci-core がある");
            assert!(!frontmatter.contains("\ncontext: true\n"), "{name} に context: true がある");
            assert!(!frontmatter.contains("\nsci_ops: "), "{name} に sci_ops がある");
            assert!(frontmatter.contains("model: inherit"), "{name} の model が inherit ではない");
            assert!(frontmatter.contains("worktree: none"), "{name} の worktree が none ではない");
            assert!(frontmatter.contains("enable: true"), "{name} が有効になっていない");
            assert!(frontmatter.contains("\nthinking: high\n"), "{name} の thinking が high ではない");
            // 依頼文は最初のメッセージの「# task_text」節に自動で入る（U04）。
            // 配るブロックに古い記法（{{input.task_text}}・{{context}}）は無い。
            assert!(
                !content.contains("{{input.task_text}}") && !content.contains("{{context}}"),
                "{name} に古い記法の {{input.…}} が残っている"
            );
            assert!(!content.contains("/Users/"), "{name} に絶対パスがある");

            // 本文に必ずある節（役割・禁止など）を確かめる
            for heading in headings {
                assert!(content.contains(heading), "{name} に {heading} が無い");
            }

            // 使う共通の決まりは研究向けの 3 本（配る物の中）。
            let templates = frontmatter_list(frontmatter, "templates:");
            assert!(templates.contains(&"sai-agent"), "{name} の templates に sai-agent が無い");
            if subblocks {
                for template in ["sai-task", "sai-cli"] {
                    assert!(templates.contains(&template), "{name} の templates に {template} が無い");
                }
            }

            // 呼べる子は、operator 以外（professor が researcher を起こす）。
            let children = frontmatter_list(frontmatter, "subblocks:");
            assert_eq!(subblocks, !children.is_empty(), "{name} の subblocks の有無が違う");
            if subblocks {
                assert!(children.contains(&"operator"), "professor の subblocks に operator が無い");
                if id == "professor" {
                    for child in [
                        "sci-start", "sci-split", "sci-ideate", "sci-analogy", "sci-search", "sci-screen",
                        "sci-read", "sci-lookup", "sci-extract", "sci-debate", "sci-review", "sci-compare",
                        "sci-seed", "sci-digest", "sci-coach", "sci-typify", "sci-exp-scout",
                        "sci-sim-model", "sci-sim-compose", "sci-exp-sprout",
                    ] {
                        assert!(children.contains(&child), "professor の subblocks に {child} が無い");
                    }
                    assert_eq!(children.len(), 21, "professor の子が 21 本でない");
                }
                for child in children {
                    let file = SCI_BLOCKS
                        .iter()
                        .map(|(block, _)| *block)
                        .find(|block| block.trim_end_matches(".md") == child);
                    assert!(file.is_some(), "professor の子「{child}」が SCI_BLOCKS に無い");
                }
            }
        }
    }

    /// `professor` の本文に、実際に使う道具（C4 のカード・C11 の現在地・C8 の質問の答え方）と、
    /// Human に上げる 4 つ（課金・鍵・要件の範囲・成功基準）が書いてある。
    #[test]
    fn professor_body_names_cards_status_questions_and_human_asks() {
        for (name, log) in [
            ("professor.md", "professor-log.md"),
            ("professor-backward.md", "professor-backward-log.md"),
        ] {
            let content = SCI_BLOCKS
                .iter()
                .find(|(block, _)| *block == name)
                .map(|(_, content)| *content)
                .unwrap_or_else(|| panic!("{name} が SCI_BLOCKS に無い"));
            for needle in [
                "\"card\": 120",
                "--executer professor",
                "--origin-run \"$SAI_PARENT_RUN_ID\"",
                "sai-moon-paper sci status --brief",
                "sci triage",
                "phone_api.py",
                "question <qid>",
                "answer <qid>",
                "課金",
                "鍵・合言葉",
                "要件の範囲",
                "成功基準",
                log,
            ] {
                assert!(content.contains(needle), "{name} に {needle} が無い");
            }
        }
    }

    /// 逆向き（backward）の 4 本と `professor-backward` は共通の決まり `bw-core` を受け、
    /// `professor-backward` は 5 つの段のブロックを全部呼べる。
    #[test]
    fn backward_blocks_declare_bw_core_and_phases() {
        let find = |name: &str| {
            SCI_BLOCKS
                .iter()
                .find(|(block, _)| *block == name)
                .map(|(_, content)| *content)
                .unwrap_or_else(|| panic!("{name} が SCI_BLOCKS に無い"))
        };
        for name in [
            "sci-restore.md",
            "sci-cite-check.md",
            "sci-review.md",
            "sci-result-compare.md",
            "professor-backward.md",
        ] {
            let content = find(name);
            let frontmatter = content
                .strip_prefix("---\n")
                .and_then(|rest| rest.split("\n---\n").next())
                .unwrap_or_else(|| panic!("{name} に frontmatter が無い"));
            assert!(
                frontmatter_list(frontmatter, "templates:").contains(&"bw-core"),
                "{name} の templates に bw-core が無い"
            );
        }
        let frontmatter = find("professor-backward.md")
            .strip_prefix("---\n")
            .and_then(|rest| rest.split("\n---\n").next())
            .unwrap();
        let children = frontmatter_list(frontmatter, "subblocks:");
        for child in [
            "sci-restore",
            "sci-cite-check",
            "sci-review",
            "sci-exp-sprout",
            "sci-search",
            "sci-seed",
            "sci-result-compare",
        ] {
            assert!(children.contains(&child), "professor-backward の subblocks に {child} が無い");
        }
    }

    /// `operator` の本文に、Web の操作との対応・故障の対処・学びの置き場が書いてある。
    #[test]
    fn operator_body_names_web_map_troubles_and_notes() {
        let content = SCI_BLOCKS
            .iter()
            .find(|(block, _)| *block == "operator.md")
            .map(|(_, content)| *content)
            .expect("operator.md が SCI_BLOCKS に無い");
        for needle in [
            "phone_api.py",
            "sai-moon-link phone tree",
            "phone md --kind",
            "sai-moon-link search -w",
            "task_capture",
            "task_manage",
            "exp_exec.py",
            "operator-notes.md",
            "`task_report` を呼ばずに止まる",
            "Web の答えが子に届かない",
            "子が閉じられていない",
            "`failed`",
            "`timeout`",
            "常駐が動いていない",
            "終了コード 2",
        ] {
            assert!(content.contains(needle), "operator.md に {needle} が無い");
        }
    }

    /// `sai-agent` / `sai-task` / `sai-cli` が配られ、絶対パスと他の言語の語が無い。
    #[test]
    fn research_templates_are_shipped_without_absolute_paths() {
        for name in ["sai-agent.md", "sai-task.md", "sai-cli.md"] {
            let content = SCI_TEMPLATES
                .iter()
                .find(|(block, _)| *block == name)
                .map(|(_, content)| *content)
                .unwrap_or_else(|| panic!("{name} が SCI_TEMPLATES に無い"));
            assert!(content.contains("# "), "{name} に見出しが無い");
            assert!(!content.contains("/Users/"), "{name} に絶対パスがある");
            assert!(content.chars().all(|c| !is_foreign_script(c)), "{name} に他の言語の文字がある");
        }
        // 道具 phone_api.py も配る（professor と operator が Web と同じ操作をするため）。
        assert!(
            SCI_TOOLS.iter().any(|(name, _)| *name == "phone_api.py"),
            "phone_api.py が SCI_TOOLS に無い"
        );
    }

    /// 韓国語・ロシア語・中国簡体字のような、他言語の文字が混ざっていないかを見る。
    fn is_foreign_script(c: char) -> bool {
        let o = c as u32;
        (0x1100..=0x11ff).contains(&o)      // ハングル
            || (0x3130..=0x318f).contains(&o) // ハングル互換
            || (0xac00..=0xd7af).contains(&o) // ハングル
            || (0x0400..=0x04ff).contains(&o) // キリル
            || (0x0370..=0x03ff).contains(&o) // ギリシャ
    }
}
