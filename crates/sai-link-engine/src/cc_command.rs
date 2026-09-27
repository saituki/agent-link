//! # Claude Code（`cc`）起動コマンドビルダー
//!
//! `BlockConfig` から Claude Code の起動コマンド文字列を構築します。
//! `runner=cc` のとき `orchestrator::prepare_pipeline` から使用されます。
//!
//! ## pi との違い
//!
//! - 起動バイナリは `cc`（ユーザーが Claude Code の shim を用意する前提）
//! - セッションは `--session-id <uuid>` で明示指定する（pi の `--session-dir` に相当）
//! - provider は Claude Code に存在しないため無視する
//! - `thinking` は Claude Code の `--effort <level>` として渡す
//!   （pi では `--thinking`、cc では `--effort`。値は同一の 6 段階）
//!
//! ## 設定はファイルで持つ
//!
//! 道具の制限・MCP（`sai` を含む）・Skill の抑止・statusLine は、`sai-link init` が
//! プロジェクト直下に作る `.claude/settings.json` と `.mcp.json` で制御する
//! （[`cc_settings_json`] / [`mcp_config_json`]）。sai-link を通さずに起動した cc にも
//! 同じ設定が効く。起動コマンドに残すのは `--session-id`（復元時は `--resume`）・
//! `--model`・`--effort`・権限モードだけ。権限モード（bypassPermissions）は
//! cc がプロジェクト設定からの指定を受け付けないため、コマンドで渡す。
//!
//! 構築されたコマンドは `orchestrator` が `SAI_*` 環境変数を前置し、
//! `HerdrClient::run_in_pane()` で実行します。

use std::path::{Path, PathBuf};

use crate::block_config::BlockConfig;
use crate::herdr::shell_escape_single_quotes;

/// MCP サーバ名（Claude Code からは `mcp__sai__<tool>` として見える）。
pub const MCP_SERVER_NAME: &str = "sai";

/// MCP サーバが公開するツール名（`plan-05` で MCP サーバが公開する 13 ツール）。
pub const MCP_TOOL_NAMES: &[&str] = &[
    "sai_write",
    "sai_finalize",
    "sai_read",
    "task_create",
    "task_report",
    "task_list",
    "task_execute",
    "task_output",
    "task_manage",
    "task_status",
    "task_capture",
    "block_discover",
    "block_inspect",
];

/// cc に使わせない組み込みツール（`.claude/settings.json` の `permissions.deny` に入れる）。
///
/// 残すのは Read / Write / Edit / Bash / AskUserQuestion / ToolSearch（と、版によって
/// 単独で存在する Glob / Grep）。`Agent` / `Task` を外すことで cc 自身の子エージェント
/// 生成を封じ、子タスクの委譲を `mcp__sai__task_create` / `task_execute` に一本化する。
/// `Skill` を外すと Skill 一覧も context に入らない（plugin の hooks は残る）。
/// `AskUserQuestion` は sai-phone（iPhone 通知）が hook で使うため必ず残す。
pub const CC_DENIED_TOOLS: &[&str] = &[
    "Agent",
    "Task",
    "Skill",
    "Workflow",
    "WebFetch",
    "WebSearch",
    "NotebookEdit",
    "LSP",
    "Monitor",
    "TodoWrite",
    "TaskCreate",
    "TaskGet",
    "TaskList",
    "TaskUpdate",
    "TaskStop",
    "CronCreate",
    "CronDelete",
    "CronList",
    "ScheduleWakeup",
    "RemoteTrigger",
    "PushNotification",
    "EnterPlanMode",
    "ExitPlanMode",
    "EnterWorktree",
    "ExitWorktree",
    "SendMessage",
    "ListAgents",
    "Artifact",
    "ArtifactComments",
    "ArtifactData",
    "DesignSync",
    "ReportFindings",
    "SendFeedback",
    "ShareOnboardingGuide",
    "EndConversation",
    "ListMcpResourcesTool",
    "ReadMcpResourceTool",
    "ReadMcpResourceDirTool",
    // Bash から別のコーディングエージェントを起動する抜け道を塞ぐ
    "Bash(pi *)",
    "Bash(claude *)",
    "Bash(cc *)",
    "Bash(*pi.py*)",
];

/// 構築済みの Claude Code 起動コマンド。
#[derive(Debug, Clone)]
pub struct CcCommand {
    /// シェル実行可能な完全なコマンド文字列（`cc ` で始まる）
    pub command: String,
    /// `cc` に渡す引数リスト（テスト・デバッグ用）
    pub args: Vec<String>,
}

/// `CcCommand` をビルダーパターンで構築する。
///
/// 新規起動（`--session-id`）と `p` の復元（`--resume`）で同じ組み立てを使う。
pub struct CcCommandBuilder {
    /// Claude Code のセッション ID（UUID v4）
    session_id: String,
    /// モデル（block.md の `model`。cc が解釈できない名前は出さない）
    model: Option<String>,
    /// effort（block.md の `thinking`）
    thinking: Option<String>,
    /// true なら `--resume <id>`（既存セッションの復元）
    resume: bool,
    /// sai-link 側の設定ディレクトリ（`.mcp.json` / `.claude/settings.json` を持つ）。
    /// cwd の外で起動する場合に `--mcp-config` / `--settings` で明示的に渡す。
    config_dir: Option<std::path::PathBuf>,
}

impl CcCommandBuilder {
    /// セッション ID を指定してビルダーを作成する。
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            model: None,
            thinking: None,
            resume: false,
            config_dir: None,
        }
    }

    /// 設定ディレクトリ（`.mcp.json` / `.claude/settings.json` を持つディレクトリ）を
    /// セットする。cwd の外で cc を起動するとき、この場所を `--mcp-config` /
    /// `--settings` で明示的に渡す。
    pub fn with_config_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.config_dir = Some(dir);
        self
    }

    /// ブロック設定（モデル・thinking）をセットする。
    pub fn with_config(self, config: BlockConfig) -> Self {
        self.with_model_thinking(config.model, config.thinking)
    }

    /// モデルと thinking を直接セットする（復元時に実行レコードの値を使う）。
    pub fn with_model_thinking(mut self, model: Option<String>, thinking: Option<String>) -> Self {
        self.model = model;
        self.thinking = thinking;
        self
    }

    /// 既存セッションの復元（`--resume`）としてコマンドを作る。
    pub fn resume(mut self) -> Self {
        self.resume = true;
        self
    }

    /// コマンドを構築する。
    pub fn build(&mut self) -> Result<CcCommand, std::io::Error> {
        let mut args: Vec<String> = Vec::new();

        // --session-id <uuid>（新規）/ --resume <uuid>（復元）
        let flag = if self.resume { "--resume" } else { "--session-id" };
        push_pair(&mut args, flag, &self.session_id);

        // --model <model>: Claude Code が解釈できるモデルのみ渡す
        if let Some(model) = self.model.as_deref().and_then(normalize_model)
        {
            push_pair(&mut args, "--model", &model);
        }

        // --effort <level>: block.md の `thinking` を effort として渡す
        // （`none` はフラグを出さない。cc の有効値: low/medium/high/xhigh/max）
        if let Some(effort) = self.thinking.as_deref().and_then(normalize_effort)
        {
            push_pair(&mut args, "--effort", &effort);
        }

        // 権限モード: 無人実行のため既定で全許可（SAI_CC_SKIP_PERMISSIONS=0 で acceptEdits）。
        // cc はプロジェクト設定からの bypassPermissions を無視するため、ここだけはコマンドで渡す。
        if cc_skip_permissions() {
            args.push("--dangerously-skip-permissions".to_string());
        } else {
            args.push("--permission-mode".to_string());
            args.push("acceptEdits".to_string());
        }

        // sai-link 側の設定を明示的に渡す（cwd がプロジェクトルートの外のとき）。
        // cc は cwd の .mcp.json / .claude/settings.json を自動で読むため、
        // 通常（cwd = プロジェクトルート）は渡さない。
        if let Some(dir) = self.config_dir.as_deref() {
            let mcp = dir.join(".mcp.json");
            let settings = dir.join(".claude/settings.json");
            push_pair(&mut args, "--mcp-config", &mcp.to_string_lossy());
            push_pair(&mut args, "--settings", &settings.to_string_lossy());
        }

        let command = format!("cc {}", args.join(" "));
        Ok(CcCommand { command, args })
    }
}

/// シェルエスケープした引数ペアを追加する。
fn push_pair(args: &mut Vec<String>, flag: &str, value: &str) {
    args.push(flag.to_string());
    args.push(shell_escape_single_quotes(value));
}

/// Claude Code で全許可実行するか（既定 true）。
///
/// `SAI_CC_SKIP_PERMISSIONS=0` を設定すると `--permission-mode acceptEdits` に落ちる。
pub fn cc_skip_permissions() -> bool {
    std::env::var("SAI_CC_SKIP_PERMISSIONS")
        .ok()
        .map(|v| v.trim() != "0" && !v.trim().eq_ignore_ascii_case("false"))
        .unwrap_or(true)
}

/// pi 形式のモデル名を Claude Code が解釈できる名前に正規化する。
///
/// - 空 / `inherit` → `None`（フラグ省略）
/// - `anthropic/claude-sonnet-4-5` → `claude-sonnet-4-5`（接頭辞除去）
/// - `sonnet` / `opus` / `haiku` / `default` などのエイリアス → そのまま
/// - `z-ai/glm-5.3-flash` など Claude 系でないモデル → `None`（cc 既定に委ねる）
pub fn normalize_model(model: &str) -> Option<String> {
    let m = model.trim();
    if m.is_empty() || m.eq_ignore_ascii_case("inherit") {
        return None;
    }
    let stripped = m.strip_prefix("anthropic/").unwrap_or(m);
    let lower = stripped.to_ascii_lowercase();
    let is_alias = matches!(
        lower.as_str(),
        "sonnet" | "opus" | "haiku" | "default" | "best" | "sonnet[1m]" | "opusplan"
    );
    if is_alias || lower.starts_with("claude") {
        Some(stripped.to_string())
    } else {
        None
    }
}

/// block.md の `thinking` を Claude Code の effort 値へ正規化する。
///
/// - 空 / `none` / 無効値 → `None`（フラグ省略・cc の既定 effort に委ねる）
/// - `low` / `medium` / `high` / `xhigh` / `max` → そのまま
///
/// sai-link 上の項目名は runner を問わず `thinking` のままで、
/// 実行時にのみ pi → `--thinking`、cc → `--effort` に対応する。
pub fn normalize_effort(thinking: &str) -> Option<String> {
    let t = thinking.trim().to_lowercase();
    if matches!(t.as_str(), "low" | "medium" | "high" | "xhigh" | "max") {
        Some(t)
    } else {
        None
    }
}

/// `sai` MCP サーバの起動コマンド（`SAI_BIN` → `SAI_ENGINE_BIN` → `"sai-moon-link"` の順で解決）。
fn sai_mcp_command() -> String {
    std::env::var("SAI_BIN")
        .ok()
        .or_else(|| std::env::var("SAI_ENGINE_BIN").ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "sai-moon-link".to_string())
}

/// プロジェクト直下の `.mcp.json` の内容（`sai-link init` が書く）。
///
/// `sai`（sai_write / task_create 等）に加え、serena・headroom・donsetch（Web 検索）を使う。
pub fn mcp_config_json() -> serde_json::Value {
    serde_json::json!({
        "mcpServers": {
            MCP_SERVER_NAME: { "command": sai_mcp_command(), "args": ["mcp", "serve"] },
            "serena": {
                "command": "uvx",
                "args": [
                    "--from", "serena-agent", "serena", "start-mcp-server",
                    "--project-from-cwd", "--context", "claude-code",
                    "--open-web-dashboard", "False"
                ]
            },
            "headroom": { "command": "headroom", "args": ["mcp", "serve"] },
            "donsetch": { "command": "donsetch", "args": ["mcp", "--supervised"] }
        }
    })
}

/// sai-link 自身の絶対パス（cc の PATH やシェルに依存せずに sai-link を呼ぶため）。
///
/// `std::env::current_exe()` が存在すればその絶対パス、無ければ環境変数
/// （`SAI_BIN` / `SAI_ENGINE_BIN`）、最後は `sai-link`（PATH 検索）。
pub fn sai_link_exe() -> String {
    std::env::current_exe()
        .ok()
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(sai_mcp_command)
}

/// プロジェクト直下の `.claude/settings.json` の内容（`sai-link init` が書く）。
///
/// - `permissions.deny`: [`CC_DENIED_TOOLS`]（道具の制限・別エージェント起動の禁止）
/// - `permissions.allow`: sai の MCP ツール（acceptEdits 運用時も確認を出さない）
/// - `.mcp.json` のサーバを承認なしで使う / claude.ai 連携と同梱 Skill を読まない
/// - `statusLine`: cc 自身のセッション金額を `sai-link cc-statusline` で受け取る（01 §2.5）。
///   sai-link は `std::env::current_exe()` の絶対パスで呼ぶ（cc の PATH に頼らない）。
pub fn cc_settings_json() -> serde_json::Value {
    let exe = sai_link_exe();
    serde_json::json!({
        "permissions": {
            "allow": [format!("mcp__{}", MCP_SERVER_NAME)],
            "deny": CC_DENIED_TOOLS,
        },
        "enableAllProjectMcpServers": true,
        "disableClaudeAiConnectors": true,
        "disableBundledSkills": true,
        "statusLine": {
            "type": "command",
            "command": format!("{} cc-statusline", shell_escape_single_quotes(&exe)),
        },
        "hooks": cc_hooks_json(&exe, phone_hooks_dir().as_deref()),
    })
}

/// `.claude/settings.json` の `hooks`。
///
/// - `UserPromptSubmit`: `/sai-finalize` を LLM に送らず、その場で完了処理を行う（[`handle_prompt_hook`]）
/// - `PreToolUse`（AskUserQuestion）/ `SessionStart` / `PermissionRequest`: `apps/phone` のフック。
///   質問・セッション・承認を Web の窓口へ知らせる（`phone_dir` が無ければ書かない）。
///   利用者全体の `~/.claude/settings.json` ではなく、init したプロジェクトにだけ入れる。
fn cc_hooks_json(exe: &str, phone_dir: Option<&Path>) -> serde_json::Value {
    let command = |text: String| serde_json::json!([{ "type": "command", "command": text }]);
    let mut hooks = serde_json::json!({
        "UserPromptSubmit": [{
            "hooks": command(format!("{} cc-prompt-hook", shell_escape_single_quotes(exe))),
        }]
    });
    if let Some(dir) = phone_dir {
        let script = |name: &str| shell_escape_single_quotes(&dir.join(name).to_string_lossy());
        hooks["PreToolUse"] = serde_json::json!([{
            "matcher": "AskUserQuestion",
            "hooks": command(script("cc_question.py")),
        }]);
        hooks["SessionStart"] = serde_json::json!([{
            "hooks": command(script("cc_register.py")),
        }]);
        hooks["PermissionRequest"] = serde_json::json!([{
            "matcher": "",
            "hooks": command(script("cc_permission.py")),
        }]);
    }
    hooks
}

/// `apps/phone` の cc フック（`cc_question.py` など 3 本）の置き場。
///
/// 環境変数 `SAI_PHONE_HOOKS_DIR` があればそれ、無ければこのクレートをビルドした
/// リポジトリの `apps/phone/hooks`。3 本が揃っていなければ `None`（壊れたフックを書かない）。
pub fn phone_hooks_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("SAI_PHONE_HOOKS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/phone/hooks"));
    let dir = dir.canonicalize().ok()?;
    ["cc_question.py", "cc_register.py", "cc_permission.py"]
        .iter()
        .all(|name| dir.join(name).is_file())
        .then_some(dir)
}

/// `.claude/commands/sai-finalize.md` の内容（`sai-link init` が書く）。
///
/// 通常は UserPromptSubmit hook が先に処理して LLM には届かない。
/// 入力補完に `/sai-finalize` を出すためと、hook が動かないときの予備として置く。
pub const SAI_FINALIZE_COMMAND_MD: &str = "\
---
description: sai-link のブロック実行を完了として確定する
---

`mcp__sai__sai_finalize` を引数なしで呼び出す。
成功したら「run <id> を完了しました」の 1 行だけを出力して終了する。
失敗したら理由をそのまま 1 行で出力する。
";

/// cc の UserPromptSubmit hook（`sai-link cc-prompt-hook`）の本体。
///
/// 入力が `/sai-finalize` なら LLM に送らずに完了処理（`agent_api::finalize`）を行い、
/// 送信を止める JSON（`decision: block`・`reason` に結果の 1 行）を返す。
/// それ以外の入力は `None`（何も出さず、そのまま LLM に送らせる）。
/// run ID は cc の環境変数 `SAI_PARENT_RUN_ID` から取る。
pub fn handle_prompt_hook(input: &str) -> Option<String> {
    if !is_finalize_prompt(input) {
        return None;
    }
    let reason = match crate::agent_api::finalize(None) {
        Ok(v) => format!("run {} を完了しました", v["run_id"]),
        Err(e) => format!("完了できませんでした: {}", e),
    };
    Some(serde_json::json!({ "decision": "block", "reason": reason }).to_string())
}

/// hook の入力 JSON の `prompt` が `/sai-finalize`（引数付きも可）か。
fn is_finalize_prompt(input: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(input) else {
        return false;
    };
    let prompt = value["prompt"].as_str().unwrap_or("").trim();
    prompt == "/sai-finalize" || prompt.starts_with("/sai-finalize ")
}

/// `dir` の `.mcp.json` に sai の MCP サーバがあるか。
pub fn has_sai_mcp(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(".mcp.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v["mcpServers"][MCP_SERVER_NAME].is_object())
}

/// `dir` に sai-link の cc 設定（`.mcp.json` の sai / `.claude/settings.json`）があるか。
fn has_sai_config(dir: &Path) -> bool {
    has_sai_mcp(dir) && dir.join(".claude/settings.json").is_file()
}

/// pi に `--mcp-config` で渡す、プロジェクトルートの `.mcp.json` を返す。
///
/// worktree は commit 済みのファイルしか持たないため、`.mcp.json` が未 commit だと
/// 作業用コピー内の pi は sai の MCP を読めない。その場合だけルートの設定を渡す。
/// `cwd` 自身に sai があれば `None`（pi が cwd の `.mcp.json` を読む）。
pub fn resolve_sai_mcp_config(cwd: &Path) -> Option<std::path::PathBuf> {
    if has_sai_mcp(cwd) {
        return None;
    }
    let root = crate::project::project_root();
    (root != cwd && has_sai_mcp(&root)).then(|| root.join(".mcp.json"))
}

/// cc に渡す sai-link 設定（`.mcp.json` / `.claude/settings.json`）の場所を解決する。
///
/// 優先順位:
/// 1. `cwd` 自身（プロジェクトルート・作業用コピーで `sai-link init` 済みの場合）
/// 2. sai-link のプロジェクトルート（`cwd` が別プロジェクトのとき。呼び出し元が
///    `--mcp-config` / `--settings` で明示的に渡す）
///
/// どちらにも無ければ `None`。
pub fn resolve_sai_config_dir(cwd: &Path) -> Option<std::path::PathBuf> {
    if has_sai_config(cwd) {
        return Some(cwd.to_path_buf());
    }
    let root = crate::project::project_root();
    if root != cwd && has_sai_config(&root) {
        return Some(root);
    }
    None
}

/// cc の作業フォルダに `sai-link init` の設定ファイルがあるか確かめ、
/// cc に渡すべき設定ディレクトリを返す。
///
/// worktree は commit 済みのファイルしか持たないため、未 commit だと sai の MCP が
/// 読めず sai_write もできない。起動前に分かりやすく止める。
/// `cwd` がプロジェクトルートの外でも、sai-link 側の設定があればそれを返す
/// （呼び出し元が `--mcp-config` / `--settings` で渡す）。
pub fn check_project_files(cwd: &Path) -> Result<std::path::PathBuf, String> {
    if let Some(dir) = resolve_sai_config_dir(cwd) {
        return Ok(dir);
    }
    Err(format!(
        "{} および sai-link のプロジェクトルート ({}) に cc の設定（.mcp.json の sai / .claude/settings.json）がありません。\
         プロジェクトルートで `sai-link init` を実行し、.mcp.json と .claude/ を commit してください",
        cwd.display(),
        crate::project::project_root().display()
    ))
}

/// UUID v4 形式の文字列を生成する。
///
/// `getrandom`（既存依存）で 16 バイト取得し、RFC 4122 の version/variant を設定する。
/// 取得に失敗した場合は時刻と PID からフォールバック生成する（必ず有効な UUID を返す）。
pub fn generate_uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::getrandom(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mix = nanos ^ ((std::process::id() as u128) << 64);
        bytes.copy_from_slice(&mix.to_le_bytes());
    }
    // version 4（0100） / variant（10xx）
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_config::BlockConfig;

    fn mock_config() -> BlockConfig {
        BlockConfig {
            name: "test-block".to_string(),
            description: None,
            model: Some("anthropic/claude-sonnet-4-5".to_string()),
            provider: Some("anthropic".to_string()),
            thinking: Some("high".to_string()),
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
            timeout: None,
            worktree: None,
            enable: Some(true),
            category: None,
            tag: None,
            round: None,
            templates: None,
            review_headings: None,
            id: None,
        }
    }

    /// 起動コマンドは --session-id / --model / --effort / 権限モードだけになること。
    #[test]
    fn build_is_minimal() {
        std::env::remove_var("SAI_CC_SKIP_PERMISSIONS");
        let cmd = CcCommandBuilder::new("11111111-2222-4333-8444-555555555555")
            .with_config(mock_config())
            .build()
            .unwrap();
        assert_eq!(
            cmd.command,
            "cc --session-id '11111111-2222-4333-8444-555555555555' --model 'claude-sonnet-4-5' --effort 'high' --dangerously-skip-permissions"
        );
    }

    /// 復元（`p`）は --session-id の代わりに --resume になり、他は起動時と同じこと。
    #[test]
    fn resume_uses_same_flags() {
        std::env::remove_var("SAI_CC_SKIP_PERMISSIONS");
        let cmd = CcCommandBuilder::new("11111111-2222-4333-8444-555555555555")
            .with_config(mock_config())
            .resume()
            .build()
            .unwrap();
        assert_eq!(
            cmd.command,
            "cc --resume '11111111-2222-4333-8444-555555555555' --model 'claude-sonnet-4-5' --effort 'high' --dangerously-skip-permissions"
        );
    }

    /// block.md の `thinking` が cc の `--effort` に変換されること。
    #[test]
    fn thinking_maps_to_effort() {
        // low / medium / high / xhigh / max はそのまま渡る
        for level in ["low", "medium", "high", "xhigh", "max"] {
            assert_eq!(normalize_effort(level).as_deref(), Some(level));
        }
        // none はフラグを出さない（cc の既定 effort に委ねる）
        assert_eq!(normalize_effort("none"), None);
        // 空・無効値もフラグを出さない
        assert_eq!(normalize_effort(""), None);
        assert_eq!(normalize_effort("off"), None);

        // none のときは --effort 自体が出ない
        let mut config = mock_config();
        config.thinking = Some("none".to_string());
        let cmd = CcCommandBuilder::new("11111111-2222-4333-8444-555555555555")
            .with_config(config)
            .build()
            .unwrap();
        assert!(!cmd.command.contains("--effort"));
    }

    /// フックは UserPromptSubmit に加え、apps/phone の 3 本（質問・登録・承認）を入れる。
    #[test]
    fn cc_hooks_include_phone_hooks_only_when_dir_exists() {
        let dir = std::path::Path::new("/opt/sai moon/hooks");
        let with = cc_hooks_json("/bin/sai-moon-link", Some(dir));
        assert_eq!(with["PreToolUse"][0]["matcher"], "AskUserQuestion");
        assert_eq!(
            with["PreToolUse"][0]["hooks"][0]["command"],
            "'/opt/sai moon/hooks/cc_question.py'"
        );
        assert_eq!(with["SessionStart"][0]["hooks"][0]["command"], "'/opt/sai moon/hooks/cc_register.py'");
        assert_eq!(with["PermissionRequest"][0]["matcher"], "");
        assert_eq!(
            with["PermissionRequest"][0]["hooks"][0]["command"],
            "'/opt/sai moon/hooks/cc_permission.py'"
        );
        assert_eq!(
            with["UserPromptSubmit"][0]["hooks"][0]["command"],
            "'/bin/sai-moon-link' cc-prompt-hook"
        );
        // 置き場が無いときは UserPromptSubmit だけ
        let without = cc_hooks_json("/bin/sai-moon-link", None);
        assert!(without.get("PreToolUse").is_none());
        assert!(without.get("SessionStart").is_none());
        assert!(without.get("PermissionRequest").is_none());
        // このリポジトリでは apps/phone/hooks が見つかる
        assert!(phone_hooks_dir().is_some());
    }

    /// `.claude/settings.json` の内容: 子エージェント禁止・質問は残す・statusLine。
    #[test]
    fn cc_settings_json_restricts_tools() {
        let v = cc_settings_json();
        let deny: Vec<&str> = v["permissions"]["deny"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap())
            .collect();
        for t in ["Agent", "Task", "Skill", "WebSearch", "Bash(claude *)"] {
            assert!(deny.contains(&t), "deny に {} がありません", t);
        }
        for t in ["Read", "Write", "Edit", "Bash", "AskUserQuestion", "ToolSearch"] {
            assert!(!deny.contains(&t), "{} は残す", t);
        }
        assert_eq!(v["permissions"]["allow"], serde_json::json!(["mcp__sai"]));
        assert_eq!(v["enableAllProjectMcpServers"], true);
        assert_eq!(v["disableClaudeAiConnectors"], true);
        assert_eq!(v["statusLine"]["type"], "command");
        assert!(v["statusLine"]["command"].as_str().unwrap().ends_with("cc-statusline"));
    }

    /// SAI_PROJECT_ROOT を一時的に設定し、スコープ終了（パニック含む）で元へ戻す。
    struct EnvGuard {
        prev: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set_project_root(val: &std::path::Path) -> Self {
            let prev = std::env::var_os("SAI_PROJECT_ROOT");
            std::env::set_var("SAI_PROJECT_ROOT", val);
            Self { prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("SAI_PROJECT_ROOT", v),
                None => std::env::remove_var("SAI_PROJECT_ROOT"),
            }
        }
    }

    /// `.mcp.json` が無い / sai が無いフォルダは起動前にエラーになること。
    #[test]
    fn check_project_files_requires_sai() {
        // フォールバック先（SAI_PROJECT_ROOT）を空にして、cwd のみを見るようにする。
        // 実行環境の SAI_PROJECT_ROOT に本物の設定があると結果が変わってしまうため。
        let _env_lock = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let _env_guard = EnvGuard::set_project_root(dir.path());
        assert!(check_project_files(dir.path()).is_err());
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(dir.path().join(".claude/settings.json"), "{}").unwrap();
        std::fs::write(dir.path().join(".mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
        assert!(check_project_files(dir.path()).is_err());
        std::fs::write(
            dir.path().join(".mcp.json"),
            mcp_config_json().to_string(),
        )
        .unwrap();
        assert!(check_project_files(dir.path()).is_ok());
    }

    /// cwd に設定が無くても sai-link のプロジェクトルートに設定があれば
    /// その場所を返す（cwd の外で cc を起動するときに `--mcp-config` /
    /// `--settings` で渡すため）。
    #[test]
    fn check_project_files_falls_back_to_project_root() {
        let _env_lock = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".claude")).unwrap();
        std::fs::write(root.path().join(".claude/settings.json"), "{}").unwrap();
        std::fs::write(
            root.path().join(".mcp.json"),
            mcp_config_json().to_string(),
        )
        .unwrap();
        let other = tempfile::tempdir().unwrap();
        let _env_guard = EnvGuard::set_project_root(root.path());
        let resolved = check_project_files(other.path()).unwrap();
        assert_eq!(resolved, root.path());
    }

    /// pi 用: cwd に sai の MCP が無く、プロジェクトルートにだけあるときに
    /// ルートの `.mcp.json` を返す（作業用コピーに `.mcp.json` が無い場合）。
    #[test]
    fn resolve_sai_mcp_config_falls_back_to_project_root() {
        let _env_lock = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        let _env_guard = EnvGuard::set_project_root(root.path());

        // 両方に無い → None
        assert_eq!(resolve_sai_mcp_config(worktree.path()), None);

        // ルートにだけある → ルートの .mcp.json
        std::fs::write(root.path().join(".mcp.json"), mcp_config_json().to_string()).unwrap();
        assert_eq!(
            resolve_sai_mcp_config(worktree.path()),
            Some(root.path().join(".mcp.json"))
        );

        // cwd にもある → None（pi が cwd の .mcp.json を読む）
        std::fs::write(worktree.path().join(".mcp.json"), mcp_config_json().to_string()).unwrap();
        assert_eq!(resolve_sai_mcp_config(worktree.path()), None);

        // cwd がルート自身 → None
        assert_eq!(resolve_sai_mcp_config(root.path()), None);
    }

    /// cwd の外で起動するとき、sai-link 側の設定が `--mcp-config` /
    /// `--settings` で渡されること。
    #[test]
    fn build_passes_external_config_when_cwd_is_outside_project() {
        std::env::remove_var("SAI_CC_SKIP_PERMISSIONS");
        let dir = tempfile::tempdir().unwrap();
        let cmd = CcCommandBuilder::new("11111111-2222-4333-8444-555555555555")
            .with_config(mock_config())
            .with_config_dir(dir.path().to_path_buf())
            .build()
            .unwrap();
        let mcp = dir.path().join(".mcp.json");
        let settings = dir.path().join(".claude/settings.json");
        assert!(
            cmd.command.contains(&format!("--mcp-config '{}'", mcp.display())),
            "実際: {}",
            cmd.command
        );
        assert!(
            cmd.command.contains(&format!("--settings '{}'", settings.display())),
            "実際: {}",
            cmd.command
        );
    }

    #[test]
    fn uuid_is_valid_v4_shape() {
        let id = generate_uuid_v4();
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
        assert_eq!(&parts[2][0..1], "4", "version 4");
        assert!(matches!(&parts[3][0..1], "8" | "9" | "a" | "b"), "variant");
    }

    #[test]
    fn normalize_model_cases() {
        assert_eq!(normalize_model("inherit"), None);
        assert_eq!(normalize_model(""), None);
        assert_eq!(
            normalize_model("anthropic/claude-sonnet-4-5").as_deref(),
            Some("claude-sonnet-4-5")
        );
        assert_eq!(normalize_model("sonnet").as_deref(), Some("sonnet"));
        assert_eq!(normalize_model("opus").as_deref(), Some("opus"));
        assert_eq!(normalize_model("z-ai/glm-5.3-flash"), None);
        assert_eq!(normalize_model("deepseek-v4-flash"), None);
    }

    #[test]
    fn permission_default_is_skip() {
        // 環境変数に依存するため、既定（未設定）時の文字列のみ確認
        std::env::remove_var("SAI_CC_SKIP_PERMISSIONS");
        let cmd = CcCommandBuilder::new("11111111-2222-4333-8444-555555555555")
            .build()
            .unwrap();
        assert!(cmd.command.contains("--dangerously-skip-permissions"));
    }

    /// hook が反応するのは `/sai-finalize` だけであること（完了処理は環境変数と
    /// ファイル書き込みを伴うため、判定部分だけを確かめる）。
    #[test]
    fn prompt_hook_matches_only_finalize() {
        assert!(is_finalize_prompt(r#"{"prompt":" /sai-finalize "}"#));
        assert!(is_finalize_prompt(r#"{"prompt":"/sai-finalize 12"}"#));
        assert!(!is_finalize_prompt(r#"{"prompt":"こんにちは"}"#));
        assert!(!is_finalize_prompt(r#"{"prompt":"/sai-finalizex"}"#));
        assert!(!is_finalize_prompt("壊れた入力"));
        assert_eq!(handle_prompt_hook(r#"{"prompt":"こんにちは"}"#), None);
    }

    #[test]
    fn mcp_config_json_has_servers() {
        std::env::remove_var("SAI_BIN");
        std::env::remove_var("SAI_ENGINE_BIN");
        let v = mcp_config_json();
        assert_eq!(v["mcpServers"]["sai"]["command"], serde_json::json!("sai-moon-link"));
        assert_eq!(v["mcpServers"]["sai"]["args"], serde_json::json!(["mcp", "serve"]));
        for name in ["serena", "headroom", "donsetch"] {
            assert!(v["mcpServers"][name].is_object(), "{} がありません", name);
        }
    }
}
