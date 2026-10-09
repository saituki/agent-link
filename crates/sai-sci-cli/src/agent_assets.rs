//! `sai-sci init` が研究プロジェクトに配る研究の部品。
//!
//! - `.sai/blocks/*.md`（[`SCI_BLOCKS`] 33 本）と `.sai/template/*.md`（[`SCI_TEMPLATES`] 14 本・
//!   分野の観点の見本）・`.sai/tools/`・`.sai/exec/`・`.sai/paper/mind.md` を書き出す。
//!   既にあるファイルは上書きしない（部品の配り方。`--update` のときだけ退避して置き換える）。
//! - `.sai/settings.json` に sai-sci の 4 つのキーを書く（共有契約「sai-sci が書く設定」）。
//! - 論文側の部品（論文の block・template）は `sai-paper init` が配る。ここでは配らない。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

/// sai-sci のブロック 32 本。並びは Action の種類ごと:
/// 役 4 本（professor・professor-backward・operator・kaizen）→ Q を分ける（start・split・restore・typify）
/// → O を書く（ideate・analogy）→ F を探す（search・read・lookup・extract・cite-check）
/// → Logicality（debate・review）→ ★ の手伝い（compare）→ その他（digest・coach）
/// → 実験（exp-scout・sim-model・sim-compose・exp-sprout・exp-branch・exp-review・result-compare）
/// → データ分析（data-explore・data-test）。
///
/// sci ブロックは同じ共通の決まり（`sci-core`）を先頭で受け、起動直前に範囲（`context: true`）を作ってから動く。
/// frontmatter に分類（`sci_type`: node / manage / support）と使う op（`sci_ops`）を持つ。
///
/// ただし実験ループの `sci-exp-branch` は例外で、判定をしないので research-tree の範囲も共通の決まりも要らない
/// （受け取るのは実験ループの共通の決まり `exp-core` だけ）。
///
/// `professor`・`professor-backward`・`operator`・`kaizen` は research-tree の中で動かず、research-tree の外から research-tree を見る役。
/// 共通決まり（`sci-core`）も範囲（`context: true`）も持たず、研究プロジェクト向けの
/// 共通の決まり（`sai-agent` / `sci-task` / `sci-cli`）を受ける。
const SCI_BLOCKS: &[(&str, &str)] = &[
    ("professor.md", include_str!("../../../sai-sci/blocks/professor.md")),
    ("professor-backward.md", include_str!("../../../sai-sci/blocks/professor-backward.md")),
    ("operator.md", include_str!("../../../sai-sci/blocks/operator.md")),
    ("kaizen.md", include_str!("../../../sai-sci/blocks/kaizen.md")),
    ("sci-start.md", include_str!("../../../sai-sci/blocks/sci-start.md")),
    ("sci-restore.md", include_str!("../../../sai-sci/blocks/sci-restore.md")),
    ("sci-typify.md", include_str!("../../../sai-sci/blocks/sci-typify.md")),
    // LabAgent（研究の進め方の 6 場面）と検証（U08）。
    ("lab-read.md", include_str!("../../../sai-sci/blocks/lab-read.md")),
    ("lab-search.md", include_str!("../../../sai-sci/blocks/lab-search.md")),
    ("lab-grow.md", include_str!("../../../sai-sci/blocks/lab-grow.md")),
    ("lab-survey.md", include_str!("../../../sai-sci/blocks/lab-survey.md")),
    ("lab-place.md", include_str!("../../../sai-sci/blocks/lab-place.md")),
    ("lab-progress.md", include_str!("../../../sai-sci/blocks/lab-progress.md")),
    ("sci-verify.md", include_str!("../../../sai-sci/blocks/sci-verify.md")),
    ("sci-analogy.md", include_str!("../../../sai-sci/blocks/sci-analogy.md")),
    ("sci-lookup.md", include_str!("../../../sai-sci/blocks/sci-lookup.md")),
    ("sci-extract.md", include_str!("../../../sai-sci/blocks/sci-extract.md")),
    ("sci-cite-check.md", include_str!("../../../sai-sci/blocks/sci-cite-check.md")),
    ("sci-digest.md", include_str!("../../../sai-sci/blocks/sci-digest.md")),
    ("sci-coach.md", include_str!("../../../sai-sci/blocks/sci-coach.md")),
    ("sci-exp-scout.md", include_str!("../../../sai-sci/blocks/sci-exp-scout.md")),
    ("sci-sim-model.md", include_str!("../../../sai-sci/blocks/sci-sim-model.md")),
    ("sci-sim-compose.md", include_str!("../../../sai-sci/blocks/sci-sim-compose.md")),
    ("sci-exp-sprout.md", include_str!("../../../sai-sci/blocks/sci-exp-sprout.md")),
    ("sci-exp-branch.md", include_str!("../../../sai-sci/blocks/sci-exp-branch.md")),
    ("sci-exp-review.md", include_str!("../../../sai-sci/blocks/sci-exp-review.md")),
    ("sci-result-compare.md", include_str!("../../../sai-sci/blocks/sci-result-compare.md")),
    ("sci-data-explore.md", include_str!("../../../sai-sci/blocks/sci-data-explore.md")),
    ("sci-data-test.md", include_str!("../../../sai-sci/blocks/sci-data-test.md")),
    // U13 で sai-sci へ移した 2 本（議論の司会から子として呼ばれる）。
    ("paper-debater.md", include_str!("../../../sai-sci/blocks/paper-debater.md")),
    ("paper-reviewer.md", include_str!("../../../sai-sci/blocks/paper-reviewer.md")),
];

/// 消えた block（旧 7 本。新しい LabAgent と検証に替わった）。
///
/// `sai-sci init --update` は、プロジェクトの `.sai/blocks/` に残っていれば退避して消す
/// （古い id のまま動くと、消えた op を打つため）。退避の置き場は `write_asset` と同じ。
const SCI_RETIRED_BLOCKS: &[&str] = &[
    "sci-read.md",
    "sci-search.md",
    "sci-ideate.md",
    "sci-split.md",
    "sci-review.md",
    "sci-debate.md",
    "sci-compare.md",
];

/// sai-sci の共通の決まり。
///
/// - research-tree の共通の決まり（`sci-core.md`）・F を探す決まり（`sci-find.md`）・照合の決まり（`sci-match.md`）と実験ループの 4 本（`exp-core.md`・`exp-run.md`・`exp-ref.md`・`sim-core.md`）
/// - データ分析の 2 本（`data-core.md`・`stat-core.md`）。`sci-data-explore` と `sci-data-test` が読む。
/// - 研究プロジェクトに配る 3 本（`sai-agent.md`・`sci-task.md`・`sci-cli.md`。U8）。
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
    ("data-core.md", include_str!("../../../sai-sci/template/data-core.md")),
    ("stat-core.md", include_str!("../../../sai-sci/template/stat-core.md")),
    ("sai-agent.md", include_str!("../../../sai-sci/template/sai-agent.md")),
    ("sci-task.md", include_str!("../../../sai-sci/template/sci-task.md")),
    ("sci-cli.md", include_str!("../../../sai-sci/template/sci-cli.md")),
    ("bw-core.md", include_str!("../../../sai-sci/template/bw-core.md")),
    // U13 で sai-sci へ移した 1 本（議論の参加者の作法）。
    ("debate-participant.md", include_str!("../../../sai-sci/template/debate-participant.md")),
];

/// sai-sci の道具（`.sai/tools/` に置く。実験ループの台帳、実行先へ投げる分配器、
/// 参照コードの道具と ModelDB のカタログ）。
///
/// データ分析の検査と記録（`data_result.py`）と、K-Dense の EDA・生体信号の CLI の写し（`eda/`・`biosignal/`、MIT）。
///
/// ブロックは `$SAI_PROJECT_ROOT/.sai/tools/<名前>` で呼ぶ（環境変数は要らない）。
/// フォルダごと（`eda/<名前>`・`biosignal/<名前>`）で呼ぶ。
const SCI_TOOLS: &[(&str, &str)] = &[
    ("exp_summary.py", include_str!("../../../sai-sci/tools/exp_summary.py")),
    ("exp_exec.py", include_str!("../../../sai-sci/tools/exp_exec.py")),
    ("refcode.py", include_str!("../../../sai-sci/tools/refcode.py")),
    ("modeldb.py", include_str!("../../../sai-sci/tools/modeldb.py")),
    ("data_result.py", include_str!("../../../sai-sci/tools/data_result.py")),
    // 写し: K-Dense の EDA（MIT）。
    ("eda/README.md", include_str!("../../../sai-sci/tools/eda/README.md")),
    ("eda/LICENSE", include_str!("../../../sai-sci/tools/eda/LICENSE")),
    ("eda/_capabilities.py", include_str!("../../../sai-sci/tools/eda/_capabilities.py")),
    ("eda/_common.py", include_str!("../../../sai-sci/tools/eda/_common.py")),
    ("eda/_structured.py", include_str!("../../../sai-sci/tools/eda/_structured.py")),
    ("eda/_tabular.py", include_str!("../../../sai-sci/tools/eda/_tabular.py")),
    ("eda/capability_manifest.py", include_str!("../../../sai-sci/tools/eda/capability_manifest.py")),
    ("eda/distribution_sensitivity.py", include_str!("../../../sai-sci/tools/eda/distribution_sensitivity.py")),
    ("eda/eda_analyzer.py", include_str!("../../../sai-sci/tools/eda/eda_analyzer.py")),
    ("eda/image_inspector.py", include_str!("../../../sai-sci/tools/eda/image_inspector.py")),
    ("eda/missingness_leakage_audit.py", include_str!("../../../sai-sci/tools/eda/missingness_leakage_audit.py")),
    ("eda/report_scaffold.py", include_str!("../../../sai-sci/tools/eda/report_scaffold.py")),
    ("eda/sequence_inspector.py", include_str!("../../../sai-sci/tools/eda/sequence_inspector.py")),
    ("eda/tabular_profile.py", include_str!("../../../sai-sci/tools/eda/tabular_profile.py")),
    // 写し: K-Dense の生体信号の CLI（MIT）。
    ("biosignal/README.md", include_str!("../../../sai-sci/tools/biosignal/README.md")),
    ("biosignal/LICENSE", include_str!("../../../sai-sci/tools/biosignal/LICENSE")),
    ("biosignal/_common.py", include_str!("../../../sai-sci/tools/biosignal/_common.py")),
    ("biosignal/ecg_hrv_pipeline.py", include_str!("../../../sai-sci/tools/biosignal/ecg_hrv_pipeline.py")),
    ("biosignal/eda_pipeline.py", include_str!("../../../sai-sci/tools/biosignal/eda_pipeline.py")),
    ("biosignal/generate_synthetic.py", include_str!("../../../sai-sci/tools/biosignal/generate_synthetic.py")),
    ("biosignal/inspect_signal.py", include_str!("../../../sai-sci/tools/biosignal/inspect_signal.py")),
    ("biosignal/plan_epochs.py", include_str!("../../../sai-sci/tools/biosignal/plan_epochs.py")),
    ("biosignal/validate_multimodal.py", include_str!("../../../sai-sci/tools/biosignal/validate_multimodal.py")),
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
/// research-tree が無いプロジェクトの `mind.md` は、今の `project_files::MIND_TEMPLATE` の
/// 代わりにこれを書く（決まったこと・仮説は research-tree へ移すので、「土壌」の形が合う）。
const SCI_MIND: &str = include_str!("../../../sai-sci/mind.md");

/// 分野の観点の見本（分野の値, ファイル名, 中身）。
///
/// 3 つとも `.sai/template/` に置き、選んだ分野の中身を `domain.md` に写す。
const DOMAIN_SAMPLES: &[(&str, &str, &str)] = &[
    (
        "medical",
        "domain-medical.md",
        include_str!("../../../sai-sci/template/domain-medical.md"),
    ),
    (
        "signal-ml",
        "domain-signal-ml.md",
        include_str!("../../../sai-sci/template/domain-signal-ml.md"),
    ),
    (
        "general",
        "domain-general.md",
        include_str!("../../../sai-sci/template/domain-general.md"),
    ),
];

/// 分野の値から見本の中身を返す（`general` など）。知らない値なら `None`。
fn domain_sample(domain: &str) -> Option<&'static str> {
    DOMAIN_SAMPLES
        .iter()
        .find(|(value, _, _)| *value == domain)
        .map(|(_, _, content)| *content)
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

/// 配った結果（部品の配り方の「作成・そのまま・更新」）。
#[derive(Debug)]
enum WriteOutcome {
    /// 無かったので書いた。
    Created,
    /// 既に新しい版と同じだった（何もしない）。
    Unchanged,
    /// 古い版を退避して新しい版を書いた（退避先のパス）。
    Updated(PathBuf),
}

/// 更新のときに作る退避先（`<名前>.bak`、既にあれば `<名前>.bak.<YYYYmmdd-HHMMSS>`）。
fn backup_path(path: &Path) -> PathBuf {
    let with_suffix = |suffix: &str| {
        let mut name = path.file_name().map_or_else(std::ffi::OsString::new, |n| n.to_os_string());
        name.push(suffix);
        path.with_file_name(name)
    };
    let plain = with_suffix(".bak");
    if !plain.exists() {
        return plain;
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    with_suffix(&format!(".bak.{stamp}"))
}

/// 部品を 1 つ配る。`update` のときは古い物を退避して新しい版に置き換える。
fn write_asset(path: &Path, content: &str, update: bool) -> Result<WriteOutcome> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        return Ok(WriteOutcome::Created);
    }
    if !update {
        return Ok(WriteOutcome::Unchanged);
    }
    let old = std::fs::read_to_string(path).unwrap_or_default();
    if old == content {
        return Ok(WriteOutcome::Unchanged);
    }
    let backup = backup_path(path);
    std::fs::rename(path, &backup)?;
    std::fs::write(path, content)?;
    Ok(WriteOutcome::Updated(backup))
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

/// 分野の観点を配る。
///
/// - `domain-medical.md` / `domain-signal-ml.md` / `domain-general.md` の見本を
///   `.sai/template/` に置く（部品の配り方。`--update` なら古い物を退避して置き換える）。
/// - `.sai/template/domain.md` が無ければ、選んだ分野の中身をそのまま書く。
///   あれば `--update` でも上書きしない。`explicit`（`--domain` を付けた）で中身が違うときだけ案内を出す。
///
/// # Errors
/// ファイルの読み書きに失敗したとき、または知らない分野のとき。
pub fn install_domain(
    root: &Path,
    domain: &str,
    update: bool,
    explicit: bool,
) -> Result<Vec<String>> {
    let template = root.join(".sai/template");
    let mut created = 0usize;
    let mut updated: Vec<PathBuf> = Vec::new();
    for (_, name, content) in DOMAIN_SAMPLES {
        match write_asset(&template.join(name), content, update)? {
            WriteOutcome::Created => created += 1,
            WriteOutcome::Unchanged => {}
            WriteOutcome::Updated(backup) => updated.push(backup),
        }
    }
    let content = domain_sample(domain).with_context(|| format!("知らない分野です: {domain}"))?;

    let mut done = Vec::new();
    done.push(format!(
        "分野の観点の見本を {created} 件置きました（既にあるものはそのまま）"
    ));
    for backup in &updated {
        done.push(format!("更新しました（古い物: {}）", backup.display()));
    }
    // `domain.md` は利用者が選んだ物なので `--update` でも上書きしない。
    let domain_path = template.join("domain.md");
    if write_if_missing(&domain_path, content)? {
        done.push(format!("分野の観点: {domain}（.sai/template/domain.md）"));
    } else if explicit {
        let current = std::fs::read_to_string(&domain_path).unwrap_or_default();
        if current != content {
            done.push(format!(
                "domain.md は既にあります。分野を変えるには .sai/template/domain.md を消してから `sai-sci init --domain {domain}` を実行するか、`domain-{domain}.md` の中身を domain.md に写してください"
            ));
        }
    }
    Ok(done)
}

/// `sai-sci init` の部品配り（共有契約「部品の配り方」と「sai-sci が書く設定」）。
///
/// 配るのは: block（[`SCI_BLOCKS`]）・template（[`SCI_TEMPLATES`]）・tools（[`SCI_TOOLS`]）・
/// exec（[`SCI_EXEC`]）・分野の観点の見本と `domain.md`・`.sai/paper/mind.md`・
/// `.sai/settings.json` の 4 つのキー。
///
/// 返すのは表示用の行（`✅ …`・`⚠ …` を含む）。
///
/// # Errors
/// ファイルの読み書きに失敗したとき。
pub fn install_sci_init(
    root: &Path,
    sai_sci_bin: &Path,
    domain: &str,
    domain_explicit: bool,
    update: bool,
) -> Result<Vec<String>> {
    let mut done = Vec::new();
    let sai = root.join(".sai");

    // block: 源の版にそろえる（違えば元を `.bak` に退避する。frontmatter の model も源の値になる）。
    let mut created = 0usize;
    let mut updated: Vec<PathBuf> = Vec::new();
    let mut retired: Vec<PathBuf> = Vec::new();
    for (name, content) in SCI_BLOCKS {
        match write_asset(&sai.join("blocks").join(name), content, update)? {
            WriteOutcome::Created => created += 1,
            WriteOutcome::Unchanged => {}
            WriteOutcome::Updated(backup) => updated.push(backup),
        }
    }
    // 消えた block: `--update` のときだけ、残っていれば退避して消す（利用者が直した中身も退避に残る）。
    if update {
        for name in SCI_RETIRED_BLOCKS {
            let path = sai.join("blocks").join(name);
            if path.is_file() {
                let backup = backup_path(&path);
                std::fs::rename(&path, &backup)?;
                retired.push(backup);
            }
        }
    }
    // template は丸ごと置き換える（部品の配り方）。
    for (name, content) in SCI_TEMPLATES {
        match write_asset(&sai.join("template").join(name), content, update)? {
            WriteOutcome::Created => created += 1,
            WriteOutcome::Unchanged => {}
            WriteOutcome::Updated(backup) => updated.push(backup),
        }
    }
    // tools: 実行の印は付けない（今のとおり）。
    for (name, content) in SCI_TOOLS {
        match write_asset(&sai.join("tools").join(name), content, update)? {
            WriteOutcome::Created => created += 1,
            WriteOutcome::Unchanged => {}
            WriteOutcome::Updated(backup) => updated.push(backup),
        }
    }
    // exec: 実行先の adapter には実行の印を付ける。`Updated` は退避してから書き直すので、印が消えるため付け直す。
    for (name, content, executable) in SCI_EXEC {
        let path = sai.join("exec").join(name);
        match write_asset(&path, content, update)? {
            WriteOutcome::Created => {
                created += 1;
                if *executable {
                    make_executable(&path)?;
                }
            }
            WriteOutcome::Unchanged => {}
            WriteOutcome::Updated(backup) => {
                if *executable {
                    make_executable(&path)?;
                }
                updated.push(backup);
            }
        }
    }
    done.push(format!("✅ 研究の assets を {created} 件置きました（既にあるものはそのまま）"));
    for backup in &updated {
        done.push(format!("✅ 更新しました（古い物: {}）", backup.display()));
    }
    for backup in &retired {
        done.push(format!("✅ 廃止した block を退避しました（古い物: {}）", backup.display()));
    }

    // 分野の観点（見本 3 つと `domain.md`）。
    for line in install_domain(root, domain, update, domain_explicit)? {
        done.push(format!("✅ {line}"));
    }

    // 土壌のひな形（既にある mind.md は利用者の考えなので、そのままにする）。
    if write_if_missing(&sai.join("paper/mind.md"), SCI_MIND)? {
        done.push("✅ mind.md（土壌のひな形）を作りました".to_string());
    }

    // `.sai/settings.json` の 4 つのキー（共有契約「sai-sci が書く設定」）。
    done.extend(install_sci_settings(&sai, sai_sci_bin, update)?);
    Ok(done)
}

/// `.sai/settings.json` に sai-sci の 4 つのキーを書く（共有契約「sai-sci が書く設定」）。
///
/// キーが無ければ書く。あって値が違うとき、`--update` なしなら残して `⚠` を出し、
/// `--update` ありなら `settings.json` を 1 回だけ退避してから書き換える。ほかのキーは触らない。
fn install_sci_settings(sai: &Path, bin: &Path, update: bool) -> Result<Vec<String>> {
    let settings_path = sai.join("settings.json");
    let existed = settings_path.exists();
    let mut settings = read_json(&settings_path)?;
    let obj = settings
        .as_object_mut()
        .context(".sai/settings.json がオブジェクトではありません")?;
    let bin = bin.to_string_lossy().to_string();
    let desired: [(&str, Value); 4] = [
        ("pre_run", json!([bin.clone(), "brief", "--write"])),
        ("context", json!([bin.clone(), "context", "--run", "{run_id}"])),
        (
            "write_check",
            json!([
                bin.clone(),
                "report",
                "prepare",
                "--run",
                "{run_id}",
                "--block",
                "{block}",
                "--filename",
                "{filename}"
            ]),
        ),
        (
            "job_status",
            json!([
                "uv",
                "run",
                "--no-project",
                "--python",
                ">=3.11",
                "python",
                ".sai/tools/exp_exec.py",
                "status",
                "{job}",
                "--json"
            ]),
        ),
    ];
    let mut done = Vec::new();
    let mut changed = false;
    for (key, value) in desired {
        match obj.get(key).cloned() {
            None => {
                obj.insert(key.to_string(), value);
                changed = true;
            }
            Some(existing) if existing == value => {}
            Some(_) => {
                if update {
                    obj.insert(key.to_string(), value);
                    changed = true;
                } else {
                    done.push(format!(
                        "⚠ {key} は今の値を残しました（`sai-sci init --update` で書き換えます）"
                    ));
                }
            }
        }
    }
    if changed {
        if update && existed {
            let backup = backup_path(&settings_path);
            std::fs::rename(&settings_path, &backup)?;
            done.push(format!("✅ 更新しました（古い物: {}）", backup.display()));
        }
        write_json(&settings_path, &settings)?;
    }
    Ok(done)
}

/// この PC の実行ファイルを探して [`install_sci_init`] を既定の指定で呼ぶ
/// （`sci.rs` の `init_with_assets` から呼ぶ）。
///
/// # Errors
/// ファイルの読み書きに失敗したとき。
pub fn install_sci_here(root: &Path) -> Result<Vec<String>> {
    let bin = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_else(|_| PathBuf::from("sai-sci"));
    install_sci_init(root, &bin, "general", false, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// research-tree の範囲も共通の決まり（sci-core）も受け取らない 1 本（実験の実行役）。
    const EXP_WITHOUT_SCI_CORE: &str = "sci-exp-branch.md";

    /// research-tree の中で動かず、外から research-tree を見る 4 本（professor・professor-backward・operator・kaizen）と、
    /// U13 で sai-sci へ移した議論の 2 本（paper-debater・paper-reviewer）。
    ///
    /// この 4 本は `sci-core` も `context: true` も持たない。Node の範囲を作ってから動く
    /// のではなく、research-tree そのものを見る窓口だから。共通の決まりは研究向けの 3 本
    /// （`sai-agent` / `sci-task` / `sci-cli`）を受ける。議論の 2 本は司会から子として
    /// 呼ばれ、依頼文の背景だけを読む。
    const SCI_BLOCKS_WITHOUT_SCI_CORE: &[&str] = &[
        EXP_WITHOUT_SCI_CORE,
        "professor.md",
        "professor-backward.md",
        "operator.md",
        "kaizen.md",
        "paper-debater.md",
        "paper-reviewer.md",
    ];

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

        let done = install_domain(&root, "medical", false, true).unwrap();
        let domain_md = std::fs::read_to_string(root.join(".sai/template/domain.md")).unwrap();
        assert_eq!(domain_md, domain_sample("medical").unwrap());
        assert!(done.iter().any(|line| line.contains("分野の観点: medical")));
        for (_, name, _) in DOMAIN_SAMPLES {
            assert!(root.join(".sai/template").join(name).is_file(), "{name}");
        }

        // 既にある domain.md は上書きせず、変え方を案内する
        let done = install_domain(&root, "signal-ml", false, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join(".sai/template/domain.md")).unwrap(),
            domain_md
        );
        assert!(
            done.iter()
                .any(|line| line.contains("domain.md は既にあります"))
        );

        // 知らない分野はエラー
        assert!(install_domain(&root, "nope", false, true).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    /// sci の部品を配ると、sci ブロック・共通の決まり・分野の観点・`context` の
    /// 設定・土壌のひな形（mind.md）がそろう。今の部品も一緒に配る。
    #[test]
    fn install_sci_writes_blocks_templates_context_and_mind() {
        let root = std::env::temp_dir().join(format!("sai-paper-sci-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sai")).unwrap();
        let bin = Path::new("/opt/bin/sai-sci");

        let done = install_sci_init(&root, bin, "general", false, false).unwrap();

        // sci のブロック 32 本と、分野の観点を配る。
        let sci_blocks = std::fs::read_dir(root.join(".sai/blocks"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                name.starts_with("sci-") || name.starts_with("lab-")
            })
            .count();
        assert_eq!(SCI_BLOCKS.len(), 31, "SCI_BLOCKS が 31 本でない");
        assert_eq!(SCI_TEMPLATES.len(), 14, "SCI_TEMPLATES が 14 本でない");
        assert_eq!(sci_blocks, 25, "配った sci のブロックが 25 本でない");
        for (name, _) in SCI_BLOCKS {
            assert!(root.join(".sai/blocks").join(name).is_file(), "{name}");
        }
        // 共通の決まりと分野の観点
        assert!(root.join(".sai/template/sci-core.md").is_file());
        for (name, content) in SCI_TEMPLATES {
            let path = root.join(".sai/template").join(name);
            assert!(path.is_file(), "{name} が配られていない");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), *content, "{name} の中身が違う");
        }
        assert!(root.join(".sai/template/domain.md").is_file());

        // U8: professor と operator のブロックと研究向けの共通 template 3 本。
        for name in ["professor.md", "operator.md"] {
            assert!(root.join(".sai/blocks").join(name).is_file(), "{name} が配られていない");
        }
        for name in ["sai-agent.md", "sci-task.md", "sci-cli.md"] {
            assert!(root.join(".sai/template").join(name).is_file(), "{name} が配られていない");
        }

        // 実験ループの台帳は `.sai/tools/` に置く。ブロックはこの名前で呼ぶ
        for (name, content) in SCI_TOOLS {
            let path = root.join(".sai/tools").join(name);
            assert!(path.is_file(), "{name} が配られていない");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), *content, "{name} の中身が違う");
        }
        for (name, content) in SCI_BLOCKS.iter().chain(SCI_TEMPLATES.iter()) {
            for old in [
                "EXP_SUMMARY_PY",
                "EXP_SSH_HOST",
                "sai-ssh",
                "sai-sci propose",
                "sai-sci triage",
                "sai-sci proposals",
                "add_o",
                "record_logic",
                "Objectivity",
                "Logicality",
                "TRIAGE",
                "PENDING",
            ] {
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

        // 実行直前に範囲を作る命令（実行ファイルの絶対パス + `context --run`）
        let settings = read_json(&root.join(".sai/settings.json")).unwrap();
        assert_eq!(
            settings["context"],
            json!(["/opt/bin/sai-sci", "context", "--run", "{run_id}"]),
            "context が実行ファイルの絶対パスと context --run の設定ではない"
        );
        assert_eq!(
            settings["write_check"],
            json!(["/opt/bin/sai-sci", "report", "prepare", "--run", "{run_id}", "--block", "{block}", "--filename", "{filename}"]),
        );
        // job_status は固定（sai-link が組み立てる）。
        assert_eq!(
            settings["job_status"],
            json!(["uv", "run", "--no-project", "--python", ">=3.11", "python", ".sai/tools/exp_exec.py", "status", "{job}", "--json"]),
        );
        // 既にある write_check は上書きしない
        let mut custom = settings.clone();
        custom["write_check"] = json!(["mine"]);
        write_json(&root.join(".sai/settings.json"), &custom).unwrap();
        install_sci_init(&root, bin, "general", false, false).unwrap();
        let again = read_json(&root.join(".sai/settings.json")).unwrap();
        assert_eq!(again["write_check"], json!(["mine"]));
        // 今からある pre_run は、そのまま残す
        assert_eq!(settings["pre_run"], json!(["/opt/bin/sai-sci", "brief", "--write"]));

        // 土壌のひな形
        assert_eq!(
            std::fs::read_to_string(root.join(".sai/paper/mind.md")).unwrap(),
            SCI_MIND
        );
        assert!(
            done.iter().any(|line| line.contains("研究の assets を")),
            "assets を置いた行が無い: {done:?}"
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
        let bin = Path::new("/opt/bin/sai-sci");

        install_sci_init(&root, bin, "general", false, false).unwrap();
        // 2 回目は、既に置いた sci のブロックを動かさない。
        let before = std::fs::read_to_string(root.join(".sai/blocks/sci-start.md")).unwrap();
        install_sci_init(&root, bin, "general", false, false).unwrap();

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

    /// 消えた 7 本の block は、`--update` のときだけ退避して `.sai/blocks/` から消える。
    /// `--update` なしでは残す（利用者が直した物を、黙って動かさない）。
    #[test]
    fn retired_blocks_are_moved_on_update() {
        let root = std::env::temp_dir().join(format!("sai-paper-sci-retired-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sai/blocks")).unwrap();
        let bin = Path::new("/opt/bin/sai-sci");
        std::fs::write(root.join(".sai/blocks/sci-read.md"), "古い sci-read").unwrap();

        install_sci_init(&root, bin, "general", false, false).unwrap();
        assert!(root.join(".sai/blocks/sci-read.md").is_file());

        let done = install_sci_init(&root, bin, "general", false, true).unwrap();
        assert!(!root.join(".sai/blocks/sci-read.md").exists());
        let backup = root.join(".sai/blocks/sci-read.md.bak");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "古い sci-read");
        assert!(
            done.iter().any(|line| line.contains("廃止した block を退避しました") && line.contains("sci-read.md.bak")),
            "退避の行が無い: {done:?}"
        );

        // 消えた 7 本は配る一覧に無く、配った先にも無い。
        for name in SCI_RETIRED_BLOCKS {
            assert!(SCI_BLOCKS.iter().all(|(block, _)| block != name), "{name} が配る一覧に残っている");
            assert!(!root.join(".sai/blocks").join(name).exists(), "{name} が .sai/blocks に残っている");
        }
        std::fs::remove_dir_all(&root).ok();
    }

    /// 手で `model:` を書き換えた block も、`--update` で源の値にそろう。元の物は `.bak` に残る。
    #[test]
    fn update_resets_block_frontmatter_to_source() {
        let root = std::env::temp_dir().join(format!("sai-paper-sci-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = Path::new("/opt/bin/sai-sci");
        install_sci_init(&root, bin, "general", false, false).unwrap();

        let block = root.join(".sai/blocks/sci-start.md");
        let source = std::fs::read_to_string(&block).unwrap();
        assert!(source.contains("model: inherit"), "sci-start.md の前提が違う");
        let edited = source.replacen("model: inherit", "model: haiku", 1);
        std::fs::write(&block, &edited).unwrap();

        let done = install_sci_init(&root, bin, "general", false, true).unwrap();
        assert_eq!(std::fs::read_to_string(&block).unwrap(), source, "源の frontmatter に戻っていない");
        let backup = root.join(".sai/blocks/sci-start.md.bak");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), edited, "手で書いた model が .bak に残っていない");
        assert!(
            done.iter().any(|line| line.contains("更新しました") && line.contains("sci-start.md.bak")),
            "更新の行が無い: {done:?}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// `--update` は tools と exec を源の版にそろえる。exec の adapter は退避・書き直しの後も実行の権限を保つ。
    #[cfg(unix)]
    #[test]
    fn update_refreshes_tools_and_keeps_adapter_executable() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("sai-paper-sci-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = Path::new("/opt/bin/sai-sci");
        install_sci_init(&root, bin, "general", false, false).unwrap();

        let tool = root.join(".sai/tools/exp_exec.py");
        let adapter = root.join(".sai/exec/local/adapter");
        std::fs::write(&tool, "# 古い版\n").unwrap();
        std::fs::write(&adapter, "#!/bin/sh\n# 古い版\n").unwrap();

        let done = install_sci_init(&root, bin, "general", false, true).unwrap();
        let source_tool = SCI_TOOLS.iter().find(|(name, _)| *name == "exp_exec.py").unwrap().1;
        assert_eq!(std::fs::read_to_string(&tool).unwrap(), source_tool, "exp_exec.py が源の版でない");
        assert_eq!(std::fs::read_to_string(tool.with_file_name("exp_exec.py.bak")).unwrap(), "# 古い版\n");
        let mode = std::fs::metadata(&adapter).unwrap().permissions().mode();
        assert_ne!(mode & 0o100, 0, "adapter の実行の権限が無い");
        assert_eq!(
            std::fs::read_to_string(adapter.with_file_name("adapter.bak")).unwrap(),
            "#!/bin/sh\n# 古い版\n"
        );
        assert!(
            done.iter().any(|line| line.contains("更新しました") && line.contains("exp_exec.py.bak")),
            "更新の行が無い: {done:?}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// sci ブロック 25 本と `paper-debater`・`paper-reviewer` は `sci_type`（node / manage / support）を持ち、
    /// 役 4 本（professor・professor-backward・operator・kaizen）は持たない。sci ブロックは `layer` を持たない。
    /// 完了の判定に使っていた旧キーはどのブロックにも無い。
    #[test]
    fn sci_blocks_declare_sci_type() {
        // 共有契約の表（sci_type → ブロック）。
        let node = [
            "lab-read.md",
            "lab-search.md",
            "lab-grow.md",
            "lab-survey.md",
            "sci-verify.md",
            "sci-coach.md",
            "sci-cite-check.md",
            "sci-data-explore.md",
            "sci-data-test.md",
            "sci-exp-review.md",
        ];
        let manage = [
            "sci-start.md",
            "sci-digest.md",
            "sci-restore.md",
            "sci-typify.md",
            "sci-result-compare.md",
            "sci-exp-sprout.md",
            "lab-place.md",
            "lab-progress.md",
        ];
        let support = [
            "sci-analogy.md",
            "sci-extract.md",
            "sci-lookup.md",
            "sci-exp-scout.md",
            "sci-sim-model.md",
            "sci-sim-compose.md",
            "sci-exp-branch.md",
            "paper-debater.md",
            "paper-reviewer.md",
        ];
        let without = ["professor.md", "professor-backward.md", "kaizen.md", "operator.md"];

        let mut want: Vec<String> = Vec::new();
        for (list, value) in [(&node[..], "node"), (&manage[..], "manage"), (&support[..], "support")] {
            for name in list {
                want.push(format!("{name}: {value}"));
            }
        }
        want.sort();

        let mut got: Vec<String> = Vec::new();
        for (name, content) in SCI_BLOCKS {
            let frontmatter = content
                .strip_prefix("---\n")
                .and_then(|rest| rest.split("\n---\n").next())
                .unwrap_or_else(|| panic!("{name} に frontmatter が無い"));
            if without.contains(name) {
                assert!(!frontmatter.contains("\nsci_type:"), "{name} に sci_type がある");
            } else {
                let value = frontmatter
                    .lines()
                    .find_map(|line| line.strip_prefix("sci_type: "))
                    .unwrap_or_else(|| panic!("{name} に sci_type が無い"));
                got.push(format!("{name}: {value}"));
            }
            // 消した旧キーの名前。この名前そのものをコードに残さないよう、組み立てて作る。
            let removed: Vec<String> = ["action", "support"]
                .iter()
                .map(|s| format!("sci_{s}"))
                .chain(["agent", "user"].iter().map(|s| format!("sci_{}_{s}", "done")))
                .collect();
            for key in &removed {
                assert!(!frontmatter.contains(&format!("\n{key}:")), "{name} に {key} が残っている");
            }
            if name.starts_with("sci-") {
                assert!(!frontmatter.contains("\nlayer:"), "{name} に layer が残っている");
            }
        }
        got.sort();
        assert_eq!(got, want);
    }

    /// 2 本の実験ブロックが使うテンプレートは、全部 配る物（テンプレート + 分野の観点）にある。
    /// sai-link はテンプレートが見つからないと実行の前に止めるため、ずれると実行できない。
    ///
    /// U8 で足した `professor` と `operator` も同じ検査に含める。
    #[test]
    fn blocks_use_only_shipped_templates() {
        // 論文側の template は `sai-paper init` が配る（sai-sci は配らない）。
        let mut shipped: Vec<&str> = [
            "literature-hard-rules.md",
            "paper-search-strategy.md",
            "rag-deep-search.md",
            "rag-simple.md",
            "search-external.md",
            "search-internal.md",
        ]
        .to_vec();
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

    /// sci のブロック 31 本のうち 24 本（`SCI_BLOCKS_WITHOUT_SCI_CORE` を除く）は、全部 共通の決まり
    /// （sci-core）を先頭で受け、状況まとめ（project-brief）を待たず、起動直前に範囲を作り、
    /// 研究向けの review 見出しを持つ。
    ///
    /// `sci-exp-branch` は判定をしないので research-tree の範囲も共通の決まりも要らない。
    /// `professor` と `operator` は research-tree の中で動かないので、同じく検査から外す（下のテストで別に確かめる）。
    #[test]
    fn sci_blocks_declare_sci_core_and_context() {
        assert_eq!(SCI_BLOCKS.len(), 31);
        for id in [
            "sci-analogy.md",
            "sci-cite-check.md",
            "sci-coach.md",
            "sci-data-explore.md",
            "sci-data-test.md",
            "sci-digest.md",
            "sci-exp-review.md",
            "sci-exp-scout.md",
            "sci-exp-sprout.md",
            "sci-extract.md",
            "sci-lookup.md",
            "sci-restore.md",
            "sci-result-compare.md",
            "sci-sim-compose.md",
            "sci-sim-model.md",
            "sci-start.md",
            "sci-typify.md",
            "sci-verify.md",
            "lab-read.md",
            "lab-search.md",
            "lab-grow.md",
            "lab-survey.md",
            "lab-place.md",
            "lab-progress.md",
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
            // `sci-typify` は research-tree を書き換えず、RESEARCH_TYPE として保存するだけなので sci_ops を持たない。
            // `sci-digest` も木に op を出さず（Q の案は報告に書く）、sci_ops を持たない。
            // `lab-progress` も op を出さない（木の数と穴を報告し、CARD を勧めるだけ）。
            // `sci-exp-scout` も木を変えない（実験・データの block は F を足さない。事実は調査書に書く）。
            assert!(
                *name == "sci-typify.md"
                    || *name == "sci-digest.md"
                    || *name == "lab-progress.md"
                    || *name == "sci-exp-scout.md"
                    || frontmatter.contains("\nsci_ops: "),
                "{name} に sci_ops が無い"
            );
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

    /// `professor` と `operator` は、research-tree の中で動かず、外から research-tree を見る役。
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
            // 「…の読み方」の節がある（前の語は block 側の呼び名に従う。U43 が改名する）。
            if name == "professor.md" || name == "professor-backward.md" {
                assert!(
                    content.lines().any(|l| l.starts_with("## ") && l.ends_with("の読み方")),
                    "{name} に「…の読み方」の節が無い"
                );
            }

            // 使う共通の決まりは研究向けの 3 本（配る物の中）。
            let templates = frontmatter_list(frontmatter, "templates:");
            assert!(templates.contains(&"sai-agent"), "{name} の templates に sai-agent が無い");
            if subblocks {
                for template in ["sci-task", "sci-cli"] {
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
                        "sci-start", "lab-read", "lab-search", "lab-survey", "lab-grow", "lab-place",
                        "lab-progress", "sci-verify", "sci-analogy", "sci-lookup", "sci-extract",
                        "sci-digest", "sci-coach", "sci-typify", "sci-exp-scout",
                        "sci-sim-model", "sci-sim-compose", "sci-exp-sprout",
                        "sci-data-explore", "sci-data-test",
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

    /// `professor` の本文に、実際に使う道具（C4 の CARD・C11 の現在地・C8 の質問の答え方）と、
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
                "--executer agent",
                "--origin-run \"$SAI_PARENT_RUN_ID\"",
                "sai-sci status --brief",
                "sai-sci op",
                "sai-web",
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
            "sci-verify.md",
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
            "sci-verify",
            "sci-exp-sprout",
            "lab-search",
            "lab-read",
            "lab-grow",
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
            "sai-web",
            "sai-link run tree",
            "sai-link run md --kind",
            "sai-link session search -w",
            "sai-link task capture",
            "sai-link task",
            "exp_exec.py",
            "operator-notes.md",
            "`sai-link task report` を呼ばずに止まる",
            "Web の答えが子に届かない",
            "子が閉じられていない",
            "`failed`",
            "`timeout`",
            "常駐が動いていない",
            "終了コード 1",
        ] {
            assert!(content.contains(needle), "operator.md に {needle} が無い");
        }
    }

    /// `sai-agent` / `sci-task` / `sci-cli` が配られ、絶対パスと他の言語の語が無い。
    #[test]
    fn research_templates_are_shipped_without_absolute_paths() {
        for name in ["sai-agent.md", "sci-task.md", "sci-cli.md"] {
            let content = SCI_TEMPLATES
                .iter()
                .find(|(block, _)| *block == name)
                .map(|(_, content)| *content)
                .unwrap_or_else(|| panic!("{name} が SCI_TEMPLATES に無い"));
            assert!(content.contains("# "), "{name} に見出しが無い");
            assert!(!content.contains("/Users/"), "{name} に絶対パスがある");
            assert!(content.chars().all(|c| !is_foreign_script(c)), "{name} に他の言語の文字がある");
        }
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
