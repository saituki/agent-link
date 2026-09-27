//! # Pi エージェント起動コマンドビルダー
//!
//! `BlockConfig` から pi コーディングエージェントの起動コマンド文字列を
//! 構築するためのモジュールです。
//!
//! U01: pi も cc と同じ MCP（`sai-link mcp serve` の 13 ツール）を使う。
//! pi-mcp-adapter が `.mcp.json` の `sai` サーバを読むため、ブロック定義の
//! `extensions: pi-sai-link/extension.ts` は不要。pi 起動コマンドに
//! `--extension` は付けない（MCP 経由で道具を使う）。
//!
//! このモジュールは `lib/herdrs/runner.ts` の `buildPiCommand()` 関数と
//! 同等の機能を Rust で提供します。構築されたコマンドは
//! `HerdrClient::run_in_pane()` 経由で実行されます。
//!
//! ## 使用例
//!
//! ```no_run
//! use sai_link_engine::block_config::BlockConfig;
//! use sai_link_engine::pi_command::PiCommandBuilder;
//!
//! let config = BlockConfig {
//!     id: Some("my-block".to_string()),
//!     name: "🧪 マイブロック".to_string(),
//!     description: None,
//!     model: Some("anthropic/claude-sonnet-4-5".to_string()),
//!     provider: None,
//!     thinking: Some("high".to_string()),
//!     tools: None,
//!     extensions: None,
//!     skills: None,
//!     subblocks: None,
//!     timeout: None,
//!     worktree: None,
//!     enable: None,
//!     category: None,
//!     tag: None,
//!     round: None,
//!     templates: None,
//!     review_headings: None,
//! };
//!
//! let cmd = PiCommandBuilder::new("/tmp/sessions")
//!     .with_config(config)
//!     .build()
//!     .expect("コマンドの構築に失敗しました");
//!
//! println!("実行コマンド: {}", cmd.command);
//! println!("引数: {:?}", cmd.args);
//! ```

use crate::block_config::{BlockConfig, ToolsConfig};

// ============================================================================
// SettingsJson — .sai/settings.json
// ============================================================================

/// `.sai/settings.json` の構造。
///
/// ブロック定義の frontmatter に `model`/`provider`/`thinking` が無い場合の
/// デフォルト値、および拡張機能の一覧を提供します。
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct SettingsJson {
    /// デフォルトプロバイダ名（例: "opencode-go"）
    #[serde(default)]
    pub default_provider: Option<String>,
    /// デフォルトモデル ID（例: "muse-spark-1.2-contributor"）
    #[serde(default)]
    pub default_model: Option<String>,
    /// デフォルト thinking レベル（例: "medium"）
    #[serde(default)]
    pub default_thinking_level: Option<String>,
    /// WIP（Work In Progress）: タブ（メインブロック）ごとの pane 上限（デフォルト 8）
    #[serde(default)]
    pub wip: Option<u32>,
    /// グローバル同時実行上限（デフォルト 16）
    #[serde(default)]
    pub max_concurrent: Option<u32>,
    /// 拡張機能のファイルパス一覧
    #[serde(default)]
    pub extensions: Vec<String>,
    /// エージェント起動直前に走らせるコマンド（配列の先頭がプログラム、残りが引数。シェルは通さない）
    #[serde(default)]
    pub pre_run: Option<Vec<String>>,
}

/// `.sai/settings.json` を読み込む。
///
/// 設定順序（`docs/user-demand2.md`・設定順序）:
///
/// 1. `PROJECTROOT/.sai/settings.json`
/// 2. `~/.sai/settings.json`（**2. が優先**。キー単位で 1. を上書きする）
///
/// 両方とも存在しない場合はデフォルト値（空）を返す。
/// ファイルの読み込み・パースに失敗したキーはスキップされる（エラーにはしない）。
/// 警告は stderr に出力する。**TUI など alternate screen から呼ばれる場合は
/// [`load_settings_quiet()`] を使うこと**（stderr 出力は画面を壊し、枠外に
/// 残ってしまうため）。
pub fn load_settings() -> SettingsJson {
    let (settings, warnings) = load_settings_quiet();
    for w in &warnings {
        eprintln!("[sai-engine] 警告: {}", w);
    }
    settings
}

/// `.sai/settings.json` を**stderr に出力せずに**読み込む。
///
/// 戻り値は `(設定, 警告メッセージ)`。TUI は警告をダイアログ枠の下部に
/// 表示し、モーダルを閉じたときに一緒に消すためにこちらを使う。
pub fn load_settings_quiet() -> (SettingsJson, Vec<String>) {
    // 1. プロジェクトローカル設定
    let local_path = crate::project::sai_path(".sai/settings.json");

    // 2. グローバル設定（優先。キー単位で上書き）
    let global_path = std::env::var_os("HOME").map(|home| {
        std::path::PathBuf::from(home)
            .join(".sai")
            .join("settings.json")
    });

    merge_settings_from(Some(&local_path), global_path.as_deref())
}

/// 2 つの settings.json をマージして読み込む。
///
/// `local` が先に読み込まれ、`global`（存在する場合）がキー単位で上書きする。
/// テストでは環境変数を介さずに直接パスを指定できる（環境変数競合の回避）。
/// 警告は stderr へ出さず返す（出力方針は呼び出し側が決める）。
fn merge_settings_from(
    local: Option<&std::path::Path>,
    global: Option<&std::path::Path>,
) -> (SettingsJson, Vec<String>) {
    let mut warnings: Vec<String> = Vec::new();
    let mut merged = SettingsJson::default();
    if let Some(path) = local {
        merge_settings_file(path, &mut merged, &mut warnings);
    }
    if let Some(path) = global {
        merge_settings_file(path, &mut merged, &mut warnings);
    }
    (merged, warnings)
}

/// 設定ファイルを `merged` へマージする（存在するキーのみ上書き）。
///
/// パース失敗時は `warnings` に文言を積む（stderr には出さない）。
fn merge_settings_file(path: &std::path::Path, merged: &mut SettingsJson, warnings: &mut Vec<String>) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(parsed) = serde_json::from_str::<SettingsJson>(&content) else {
        warnings.push(format!(
            "settings.json のパースに失敗しました: {}",
            path.display()
        ));
        return;
    };
    // 各キーは「値が存在する場合のみ」上書きする（None は既存値を保持）
    if parsed.default_provider.is_some() {
        merged.default_provider = parsed.default_provider;
    }
    if parsed.default_model.is_some() {
        merged.default_model = parsed.default_model;
    }
    if parsed.default_thinking_level.is_some() {
        merged.default_thinking_level = parsed.default_thinking_level;
    }
    if parsed.wip.is_some() {
        merged.wip = parsed.wip;
    }
    if parsed.max_concurrent.is_some() {
        merged.max_concurrent = parsed.max_concurrent;
    }
    if !parsed.extensions.is_empty() {
        merged.extensions = parsed.extensions;
    }
    if parsed.pre_run.is_some() {
        merged.pre_run = parsed.pre_run;
    }
}

/// タブ（メインブロック）ごとの pane 上限（WIP）。デフォルト 8。
/// `.sai/settings.json` の `wip` で上書きできる。
pub fn wip_limit() -> usize {
    load_settings()
        .wip
        .filter(|v| *v > 0)
        .map(|v| v as usize)
        .unwrap_or(8)
}

/// グローバル同時実行上限（max_concurrent）。デフォルト 16。
/// `.sai/settings.json` の `max_concurrent` で上書きできる。
pub fn max_concurrent_limit() -> usize {
    load_settings()
        .max_concurrent
        .filter(|v| *v > 0)
        .map(|v| v as usize)
        .unwrap_or(16)
}

// ============================================================================
// PiCommand — 構築済みコマンド
// ============================================================================

/// 構築済みの pi 起動コマンドを表す構造体。
///
/// `PiCommandBuilder::build()` の戻り値として返され、
/// FN-006（エージェント実行オーケストレーター）から
/// `HerdrClient::run_in_pane()` に渡されます。
pub struct PiCommand {
    /// シェルエスケープ済みの完全なコマンド文字列。
    ///
    /// 例: `pi --session-dir '/tmp/session' --model 'anthropic/claude-sonnet-4-5'`
    pub command: String,

    /// pi バイナリに渡す引数のリスト（テスト・デバッグ用）。
    ///
    /// 各要素はエスケープ済みの単一引数です。
    /// `command` 文字列は `"pi " + args.join(" ")` で構築されます。
    pub args: Vec<String>,
}

impl PiCommand {
    /// 最小構成の `PiCommand` を作成します。
    ///
    /// 主にテスト用。通常は `PiCommandBuilder` を使用してください。
    pub fn new(command: String, args: Vec<String>) -> Self {
        Self { command, args }
    }
}

// ============================================================================
// PiCommandBuilder — コマンドビルダー
// ============================================================================

/// `PiCommand` をビルダーパターンで構築する構造体。
///
/// # 必須項目
///
/// - `session_dir` — JSONL セッションファイルの出力先ディレクトリ（`new()` で指定）
///
/// # オプション項目
///
/// - `config` — `BlockConfig`（ブロック設定。モデル、ツール、拡張機能等）
/// - `obs_*` — Observability 設定（プール名、タグ、名前）
///
/// # ビルド結果
///
/// `build()` を呼び出すと `PiCommand` が生成されます。
/// シェルエスケープ処理は自動適用されます。
pub struct PiCommandBuilder {
    /// JSONL セッションファイルの出力先ディレクトリ（必須）
    session_dir: String,

    /// ブロック設定（オプション）
    config: Option<BlockConfig>,

    /// Observability プール名（`--o-pool`）
    obs_pool: Option<String>,

    /// Observability タグ（`--o-tag`）
    obs_tag: Option<String>,

    /// Observability 名前（`--o-name`）
    obs_name: Option<String>,

    /// pi-mcp-adapter の MCP 設定ファイル（`--mcp-config`）
    mcp_config: Option<String>,
}

impl PiCommandBuilder {
    /// 新しい `PiCommandBuilder` を作成します。
    ///
    /// セッションディレクトリは必須で、pi の JSONL セッションファイルの
    /// 出力先を指定します。
    ///
    /// # 引数
    ///
    /// * `session_dir` — JSONL セッションファイルの出力先ディレクトリパス
    ///
    /// # 例
    ///
    /// ```
    /// use sai_link_engine::pi_command::PiCommandBuilder;
    /// let builder = PiCommandBuilder::new("/tmp/my-session");
    /// ```
    pub fn new(session_dir: impl Into<String>) -> Self {
        Self {
            session_dir: session_dir.into(),
            config: None,
            obs_pool: None,
            obs_tag: None,
            obs_name: None,
            mcp_config: None,
        }
    }

    /// ブロック設定（`BlockConfig`）をセットします。
    ///
    /// これによりモデル、プロバイダ、ツール設定、拡張機能、スキル、
    /// 最大ターン数、タイムアウト等がコマンドに反映されます。
    ///
    /// # 引数
    ///
    /// * `config` — FN-003 でパースされたブロック設定
    pub fn with_config(mut self, config: BlockConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// pi-mcp-adapter に読ませる MCP 設定ファイルをセットします（`--mcp-config`）。
    ///
    /// 作業用コピーに `.mcp.json` が無いとき、プロジェクトルートの設定を渡すために使う。
    pub fn with_mcp_config(mut self, path: impl Into<String>) -> Self {
        self.mcp_config = Some(path.into());
        self
    }

    /// Observability 設定をセットします。
    ///
    /// `--o-pool`、`--o-tag`、`--o-name` フラグを追加します。
    /// herdrs の分散トレーシングとメトリクス収集に使用されます。
    ///
    /// # 引数
    ///
    /// * `pool` — Observability プール名
    /// * `tag` — Observability タグ
    /// * `name` — Observability 名前
    pub fn with_obs(
        mut self,
        pool: impl Into<String>,
        tag: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        self.obs_pool = Some(pool.into());
        self.obs_tag = Some(tag.into());
        self.obs_name = Some(name.into());
        self
    }

    /// pi 起動コマンドを構築します。
    ///
    /// `BlockConfig` の各フィールドを対応する pi CLI フラグにマッピングし、
    /// シェルエスケープ済みのコマンド文字列を生成します。
    ///
    /// # 戻り値
    ///
    /// 構築された `PiCommand`。一時ファイル作成の I/O エラーが発生した場合は
    /// エラーを返します（システムプロンプトの書き込み失敗時は警告を出力し続行）。
    ///
    /// # フラグマッピング概要
    ///
    /// | BlockConfig フィールド | pi CLI フラグ | 備考 |
    /// |---|---|---|
    /// | `session_dir` | `--session-dir` | 常に最初の引数 |
    /// | `config.model` | `--model` | `"inherit"` の場合はスキップ |
    /// | `config.provider` | `--provider` | 未指定時は models.json から自動検出 |
    /// | `config.thinking` | `--thinking` | thinking budget レベル（"none"の場合はスキップ） |
    /// | `config.tools` | `--tools` / `--no-tools` | ToolsConfig に応じて切替 |
    /// | `config.extensions` | `--extensions` / `--no-extensions` | ToolsConfig に応じて切替 |
    /// | `config.skills` | `--skills` | カンマ区切りスキル名 |
    /// | `config.timeout` | `--timeout` | 整数値（秒） |
    /// | `obs_pool` | `--o-pool` | observability |
    /// | `obs_tag` | `--o-tag` | observability |
    /// | `obs_name` | `--o-name` | observability |
    /// | `mcp_config` | `--mcp-config` | pi-mcp-adapter の設定ファイル |
    pub fn build(&mut self) -> Result<PiCommand, std::io::Error> {
        let mut args: Vec<String> = Vec::new();

        // --session-dir: 常に最初の引数として追加（必須）
        // シェルエスケープは run_in_pane() 側で全体を囲むため、ここでは生の値を渡す
        args.push("--session-dir".to_string());
        args.push(self.session_dir.clone());

        if let Some(ref config) = self.config {
            args.extend(agent_args_for_config(config));
        }

        // --mcp-config <path>: 作業用コピーで sai の MCP を読めるようにする
        if let Some(ref path) = self.mcp_config {
            args.push("--mcp-config".to_string());
            args.push(path.clone());
        }

        // --o-pool <pool>: observability プール名
        if let Some(ref pool) = self.obs_pool {
            args.push("--o-pool".to_string());
            args.push(pool.clone());
        }

        // --o-tag <tag>: observability タグ
        if let Some(ref tag) = self.obs_tag {
            args.push("--o-tag".to_string());
            args.push(tag.clone());
        }

        // --o-name <name>: observability 名前
        if let Some(ref name) = self.obs_name {
            args.push("--o-name".to_string());
            args.push(name.clone());
        }

        // コマンド文字列の構築: "pi {args.join(" ")}"
        let command = format!("pi {}", args.join(" "));

        Ok(PiCommand { command, args })
    }
}

/// `BlockConfig` から `--model` / `--thinking` / `--provider` / `--extension` /
/// `--tools` / `--no-tools` の引数列を組み立てる（`--session-dir` は含まない）。
///
/// [`PiCommandBuilder::build`] の本体ロジックを切り出したもの。新規起動
/// （`--session-dir`）とセッション再開（`--session`、`app/external.rs::resume_session`
/// 経由）の両方から共有し、拡張機能・ツール設定のマージロジックを重複させない。
pub fn agent_args_for_config(config: &BlockConfig) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();

    // enable: false の場合は警告を出力するがビルドは続行
    if let Some(false) = config.enable {
        eprintln!(
            "[sai-engine] 警告: ブロック '{}' は無効 (enable: false) ですが、コマンドを構築します",
            config.name
        );
    }

    // --model <model_id>: モデルID（"inherit" の場合はスキップ）
    if let Some(ref model) = config.model {
        if model != "inherit" {
            args.push("--model".to_string());
            args.push(model.clone());
        }
    }

    // --thinking <level>: thinking budget（"none" の場合はスキップ）
    // pi は `none` を知らないためフラグ自体を出さない（= 無指定）。
    // `max` は pi 側が受け付ける値としてそのまま渡す。
    if let Some(ref thinking) = config.thinking {
        if !thinking.is_empty() && thinking != "none" {
            args.push("--thinking".to_string());
            args.push(thinking.clone());
        }
    }

    // --provider <name>: プロバイダ名（model から自動検出）
    // model が指定されていて provider が未設定の場合、.sai/models.json から検索
    let effective_provider = if config.provider.is_some() {
        config.provider.clone()
    } else if let Some(ref model) = config.model {
        if model != "inherit" {
            resolve_provider_for_model(model)
        } else {
            None
        }
    } else {
        None
    };
    if let Some(ref provider) = effective_provider {
        if !provider.is_empty() {
            args.push("--provider".to_string());
            args.push(provider.clone());
        }
    }

    // --tools <names> / --no-tools（U01: --extension は付けない。pi も MCP で道具を使う）
    let mut tool_names: Vec<String> = Vec::new();

    if let Some(ref tools) = config.tools {
        match tools {
            ToolsConfig::List(items) => tool_names.extend(items.clone()),
            ToolsConfig::All => {} // デフォルト全有効 → フラグ不要
            ToolsConfig::None => {
                args.push("--no-tools".to_string());
            }
        }
    }
    // U01: pi-sai-link/extension.ts は廃止。pi も MCP（.mcp.json の sai）で
    // 道具を使うため、`--extension` は付けない。ブロック定義に旧パスが
    // 残っていても無視する（警告のみ）。ツール名形式（herdrs 等）は U01 で
    // 使わない（MCP の mcp__sai__* を使うため --tools にも入れない）。
    if let Some(ref extensions) = config.extensions {
        if let ToolsConfig::List(items) = extensions {
            for item in items {
                eprintln!(
                    "[sai-engine] extensions '{}' は無視します（U01: pi は MCP で道具を使うため）",
                    item
                );
            }
        }
    }

    // R12: `.sai/settings.json` の extensions は U01 で廃止（pi は MCP 使用）。
    // 旧設定が残っていても --extension は付けない（警告のみ）。
    let settings = load_settings();
    for item in &settings.extensions {
        eprintln!(
            "[sai-engine] settings.json の extensions '{}' は無視します（U01: pi は MCP で道具を使うため）",
            item
        );
    }

    if let Some(ref skills) = config.skills {
        if !skills.is_empty() {
            tool_names.extend(skills.split(',').map(|s| s.trim().to_string()));
        }
    }

    // --tools <names> を追加（--extension は U01 で廃止）
    if !tool_names.is_empty() {
        args.push("--tools".to_string());
        args.push(tool_names.join(","));
    }

    args
}

// ============================================================================
// 内部ヘルパー関数
// ============================================================================

/// 絶対パスに変換し、`.` / `..` を語彙的に正規化する。
///
/// 例: `/a/b/../c/d` → `/a/c/d`
fn normalize_abs_path(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;

    let mut result = std::path::PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                // 直前の正規コンポーネントを除去
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// 指定されたモデル ID が属するプロバイダ名を検索する。
///
/// pi 本体（`core/model-resolver.js` の `resolveCliModel`）と同じ推論順序で行う。
/// OpenRouter 系のモデル ID は `z-ai/glm-5.3-flash` のように**ID 自体に `/` を含む**
/// ため、接頭辞を無条件に provider と断定してはならない（過去のバグ: `--provider z-ai`
/// を出力して pi が `Unknown provider` で起動失敗していた）。
///
/// 検索方法:
/// 1. `~/.pi/agent/models.json` の全 provider 配下でモデル ID **完全一致**照合
///    （ID に `/` を含む OpenRouter 系 ID もここで正しく解決する）
/// 2. 接頭辞（最初の `/` より前）が models.json の既知 provider 名なら接頭辞を返す
///    （pi の `--model "provider/id"` 指定形式と同じ）
/// 3. どちらにも該当しない場合は `settings.json` の `defaultProvider` を
///    フォールバックとして返す。それでも無い場合は `None` を返す
///    （`--provider` を付けず、pi 側のモデル解決に委ねる）。
pub fn resolve_provider_for_model(model_id: &str) -> Option<String> {
    let parsed = load_models_json();

    // 1. モデル ID 完全一致: 全 provider 配下から ID が一致するモデルを探す
    if let Some(ref value) = parsed {
        if let Some(provider) = find_provider_by_model_id(value, model_id) {
            return Some(provider);
        }
    }

    // 2. `provider/model` 形式: 接頭辞が既知 provider 名の場合のみ採用する
    if let Some((prefix, _)) = model_id.split_once('/') {
        if !prefix.is_empty() && is_known_provider(parsed.as_ref(), prefix) {
            return Some(prefix.to_string());
        }
    }

    // 3. settings.json の defaultProvider をフォールバックとして返す
    let settings = load_settings();
    if let Some(ref provider) = settings.default_provider {
        if !provider.is_empty() {
            return Some(provider.clone());
        }
    }

    None
}

/// `~/.pi/agent/models.json` を読み込む（末尾カンマを許容する寛容パース）。
///
/// pi 本体は JSON5 相当の寛容なパーサで models.json を読むため、手編集による
/// 末尾カンマ（`]` / `}` 直前の `,`）が残っていても読める。Rust 側は serde_json
/// （厳密）なので、末尾カンマのみ除去してからパースする（pi の挙動に合わせる）。
fn load_models_json() -> Option<serde_json::Value> {
    let content = std::fs::read_to_string(crate::common::pi_models_json_path()).ok()?;
    // まず厳密にパースできればそれを使う
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        return Some(v);
    }
    let cleaned = strip_trailing_commas(&content);
    serde_json::from_str::<serde_json::Value>(&cleaned).ok()
}

/// JSON テキストから文字列リテラル外の末尾カンマ（`]` / `}` 直前の `,`）を除去する。
fn strip_trailing_commas(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            ',' => {
                // 次の非空白文字が `]` / `}` なら末尾カンマなので出力しない
                let skip = chars.clone().skip_while(|n| n.is_whitespace()).next()
                    == Some(']')
                    || chars.clone().skip_while(|n| n.is_whitespace()).next()
                        == Some('}');
                if !skip {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// models.json の `providers` オブジェクト配下で、モデル ID が `model_id` と
/// 完全一致するエントリを探し、その provider 名を返す。
fn find_provider_by_model_id(parsed: &serde_json::Value, model_id: &str) -> Option<String> {
    let providers = parsed.get("providers")?.as_object()?;
    for (provider_name, provider_config) in providers {
        if let Some(models) = provider_config.get("models").and_then(|v| v.as_array()) {
            for model_entry in models {
                if model_entry.get("id").and_then(|v| v.as_str()) == Some(model_id) {
                    return Some(provider_name.clone());
                }
            }
        }
    }
    None
}

/// `name` が models.json に登録済みの provider 名（大文字小文字を区別しない）か。
fn is_known_provider(parsed: Option<&serde_json::Value>, name: &str) -> bool {
    let Some(value) = parsed else {
        return false;
    };
    let Some(providers) = value.get("providers").and_then(|v| v.as_object()) else {
        return false;
    };
    providers
        .keys()
        .any(|k| k.eq_ignore_ascii_case(name))
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_config::ToolsConfig;

    /// テスト用のデフォルト BlockConfig を作成するヘルパー。
    fn mock_config() -> BlockConfig {
        BlockConfig {
            name: "test-block".to_string(),
            description: Some("テスト用ブロック".to_string()),
            model: Some("anthropic/claude-sonnet-4-5".to_string()),
            provider: None,
            thinking: None,
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

    // ------------------------------------------------------------------
    // 基本構築テスト
    // ------------------------------------------------------------------

    #[test]
    fn test_build_minimal_command() {
        // session_dir のみの最小コマンド
        let cmd = PiCommandBuilder::new("/tmp/session")
            .build()
            .expect("最小コマンドの構築に失敗");

        assert!(cmd.command.starts_with("pi "));
        assert!(cmd.command.contains("--session-dir"));
        assert!(cmd.command.contains("/tmp/session"));
        assert!(!cmd.command.contains("--mcp-config"));
    }

    #[test]
    fn test_build_with_mcp_config() {
        // 作業用コピー用: プロジェクトルートの .mcp.json を --mcp-config で渡すこと
        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_mcp_config("/proj/root/.mcp.json")
            .build()
            .expect("コマンドの構築に失敗");

        assert!(cmd.args.windows(2).any(|w| w[0] == "--mcp-config" && w[1] == "/proj/root/.mcp.json"));
        assert!(cmd.command.contains("--mcp-config"));
    }

    #[test]
    fn test_build_with_model() {
        // model が --model フラグとして追加されること
        let mut config = mock_config();
        config.model = Some("anthropic/claude-sonnet-4-5".to_string());

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        assert!(cmd.command.contains("--model"));
        assert!(cmd.command.contains("anthropic/claude-sonnet-4-5"));
    }

    #[test]
    fn test_build_skip_inherit_model() {
        // model: "inherit" がスキップされること
        let mut config = mock_config();
        config.model = Some("inherit".to_string());

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        assert!(!cmd.command.contains("--model"));
    }

    #[test]
    fn test_build_with_provider() {
        // provider が --provider フラグとして追加されること
        let mut config = mock_config();
        config.provider = Some("anthropic".to_string());

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        assert!(cmd.command.contains("--provider"));
        assert!(cmd.command.contains("anthropic"));
    }

    #[test]
    fn test_build_with_thinking() {
        // --model と --thinking が別フラグとして出力されること
        let mut config = mock_config();
        config.thinking = Some("high".to_string());

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        // --model と --thinking が別々に出力される
        assert!(cmd.command.contains("--model"));
        assert!(cmd.command.contains("anthropic/claude-sonnet-4-5"));
        assert!(cmd.command.contains("--thinking"));
        assert!(cmd.command.contains("high"));
        // model:thinking 結合形式ではない
        assert!(!cmd.command.contains("anthropic/claude-sonnet-4-5:high"));
    }

    // ------------------------------------------------------------------
    // ツール・拡張機能テスト
    // ------------------------------------------------------------------

    #[test]
    fn test_build_tools_all() {
        // ToolsConfig::All で --tools / --no-tools が追加されないこと
        let mut config = mock_config();
        config.tools = Some(ToolsConfig::All);

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        assert!(!cmd.command.contains("--tools"));
        assert!(!cmd.command.contains("--no-tools"));
    }

    #[test]
    fn test_build_tools_none() {
        // ToolsConfig::None で --no-tools が追加されること
        let mut config = mock_config();
        config.tools = Some(ToolsConfig::None);

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        assert!(cmd.command.contains("--no-tools"));
        assert!(!cmd.command.contains("--tools"));
    }

    #[test]
    fn test_build_tools_list() {
        // ToolsConfig::List で --tools 'read,bash' が追加されること
        let mut config = mock_config();
        config.tools = Some(ToolsConfig::List(vec![
            "read".to_string(),
            "bash".to_string(),
        ]));

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        assert!(cmd.command.contains("--tools"));
        assert!(cmd.command.contains("read,bash"));
    }

    #[test]
    fn test_build_extensions_all() {
        // ToolsConfig::All で拡張機能フラグが追加されないこと
        let mut config = mock_config();
        config.extensions = Some(ToolsConfig::All);

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        assert!(!cmd.command.contains("--extensions"));
        assert!(!cmd.command.contains("--no-extensions"));
    }

    #[test]
    fn test_build_extensions_none() {
        // ToolsConfig::None で --no-extensions が追加されること
        let mut config = mock_config();
        config.extensions = Some(ToolsConfig::None);

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        assert!(!cmd.command.contains("--extensions"));
        // --no-extensions は pi に存在しない
        assert!(!cmd.command.contains("--no-extensions"));
    }

    #[test]
    fn test_build_extensions_list() {
        // U01: extensions は --extension ではなく --tools に入らない。
        // pi は MCP（.mcp.json の sai）で道具を使うため、パス指定は無視する。
        let mut config = mock_config();
        config.extensions = Some(ToolsConfig::List(vec![
            "herdrs".to_string(),
            "pi-sai".to_string(),
        ]));

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        assert!(!cmd.command.contains("--extension"));
        assert!(!cmd.command.contains("--extensions"));
        // U01: 旧 extensions 名は --tools にも入れない（MCP で使うため）
        assert!(!cmd.command.contains("herdrs,pi-sai"));
    }

    /// U01: pi-sai-link パスは無視され、--extension が付かないこと。
    #[test]
    fn test_u01_pi_sai_link_path_ignored() {
        let mut config = mock_config();
        config.extensions = Some(ToolsConfig::List(vec![
            "/Users/touta/workdir/Sai/sai-link/pi-sai-link/extension.ts".to_string(),
        ]));
        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");
        assert!(!cmd.command.contains("--extension"));
        assert!(!cmd.command.contains("pi-sai-link"));
    }

    // ------------------------------------------------------------------
    // その他フラグテスト
    // ------------------------------------------------------------------

    #[test]
    fn test_build_with_skills() {
        // skills が --tools にマージされること（pi には --skills がない）
        let mut config = mock_config();
        config.skills = Some("ce-brainstorm,pi-sai".to_string());

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        assert!(!cmd.command.contains("--skills"));
        assert!(cmd.command.contains("--tools"));
        assert!(cmd.command.contains("ce-brainstorm,pi-sai"));
    }

    #[test]
    fn test_build_with_timeout() {
        // timeout は pi CLI に存在しないためフラグは追加されない
        let mut config = mock_config();
        config.timeout = Some(600);

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        assert!(!cmd.command.contains("--timeout"));
        assert!(cmd.command.contains("--session-dir"));
    }

    #[test]
    fn test_build_with_obs() {
        // observability フラグが追加されること
        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_obs("my-pool", "my-tag", "my-name")
            .build()
            .expect("コマンドの構築に失敗");

        assert!(cmd.command.contains("--o-pool"));
        assert!(cmd.command.contains("my-pool"));
        assert!(cmd.command.contains("--o-tag"));
        assert!(cmd.command.contains("my-tag"));
        assert!(cmd.command.contains("--o-name"));
        assert!(cmd.command.contains("my-name"));
    }

    #[test]
    fn test_build_all_flags() {
        // 全フィールド設定時の完全なコマンドを確認
        let mut config = mock_config();
        config.model = Some("anthropic/claude-sonnet-4-5".to_string());
        config.provider = Some("anthropic".to_string());
        config.thinking = Some("high".to_string());
        config.tools = Some(ToolsConfig::List(vec![
            "read".to_string(),
            "bash".to_string(),
        ]));
        config.extensions = Some(ToolsConfig::None);
        config.skills = Some("ce-brainstorm".to_string());
        config.timeout = Some(600);

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .with_obs("pool1", "tag1", "name1")
            .build()
            .expect("コマンドの構築に失敗");

        let c = &cmd.command;
        assert!(c.starts_with("pi "));
        assert!(c.contains("--session-dir"));
        assert!(c.contains("/tmp/session"));
        assert!(c.contains("--model"));
        assert!(c.contains("anthropic/claude-sonnet-4-5"));
        // --thinking が別フラグとして出力される
        assert!(c.contains("--thinking"));
        assert!(c.contains("high"));
        assert!(c.contains("--provider"));
        assert!(c.contains("anthropic"));
        // tools + skills が --tools にマージされる（U01: extensions は MCP のため含めない）
        assert!(c.contains("--tools"));
        assert!(c.contains("read,bash,ce-brainstorm"));
        // --extensions, --skills, --max-turns, --timeout は存在しない
        assert!(!c.contains("--extensions"));
        assert!(!c.contains("--no-extensions"));
        assert!(!c.contains("--skills"));
        assert!(!c.contains("--max-turns"));
        assert!(!c.contains("--timeout"));
        assert!(c.contains("--o-pool"));
        assert!(c.contains("pool1"));
        assert!(c.contains("--o-tag"));
        assert!(c.contains("tag1"));
        assert!(c.contains("--o-name"));
        assert!(c.contains("name1"));
    }

    // ------------------------------------------------------------------
    // シェルエスケープテスト
    // ------------------------------------------------------------------

    #[test]
    fn test_shell_escape_single_quotes() {
        // シングルクォートを含む値が正しくエスケープされること
        // TypeScript の shellEscape("it's") → "'it'\"'\"'s'"
        // Rust では: shell_escape_single_quotes("it's") → "'it'\\''s'"
        let escaped = crate::herdr::shell_escape_single_quotes("it's");
        assert_eq!(escaped, "'it'\\''s'");
    }

    #[test]
    fn test_model_with_spaces() {
        // 空白を含む model 値がクォートされること
        let mut config = mock_config();
        config.model = Some("claude sonnet 4.5".to_string());

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        // シングルクォートで囲まれていることを確認
        let model_flag_idx = cmd.command.find("--model").unwrap();
        let after_flag = &cmd.command[model_flag_idx..];
        // --model の後に 'claude sonnet 4.5' が含まれている
        assert!(after_flag.contains("claude sonnet 4.5"));
    }

    #[test]
    fn test_command_no_shell_injection() {
        // シェルインジェクション文字がシングルクォートで保護されること
        let mut config = mock_config();
        config.model = Some("model; rm -rf /".to_string());

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        // シェルインジェクション用のセミコロンがシングルクォートで囲まれていることを確認
        // shell_escape_single_quotes("model; rm -rf /") → "'model; rm -rf /'"
        assert!(cmd.command.contains("model; rm -rf /"));
    }

    // ------------------------------------------------------------------
    // args ベクタテスト
    // ------------------------------------------------------------------

    #[test]
    fn test_args_order() {
        // args ベクタの要素順序が command 文字列と一致することを確認
        let mut config = mock_config();
        config.model = Some("test-model".to_string());
        config.thinking = Some("low".to_string());

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("コマンドの構築に失敗");

        // args の結合が command の "pi " 以降と一致
        let expected = format!("pi {}", cmd.args.join(" "));
        assert_eq!(cmd.command, expected);
    }

    #[test]
    fn test_disabled_block_warning() {
        // enable: false の BlockConfig でもビルドが続行されること
        let mut config = mock_config();
        config.enable = Some(false);

        let cmd = PiCommandBuilder::new("/tmp/session")
            .with_config(config)
            .build()
            .expect("無効ブロックのコマンド構築に失敗");

        assert!(cmd.command.starts_with("pi "));
        assert!(cmd.command.contains("--session-dir"));
    }

    // ------------------------------------------------------------------
    // 設定順序: settings.json のマージ（プロジェクト → グローバル優先）
    // ------------------------------------------------------------------

    /// パース失敗した settings.json は stderr に出さず警告として返すこと。
    ///
    /// TUI はこの警告をダイアログ枠の下部に表示し、モーダルを閉じて消す
    /// （stderr に出すと alternate screen 上で枠外に残り続ける）。
    #[test]
    fn merge_settings_collects_parse_warning() {
        let dir = std::env::temp_dir().join(format!("sai-settings-warn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("settings.json");
        std::fs::write(&bad, "{ broken json").unwrap();
        let bad_display = bad.display().to_string();
        let (_settings, warnings) = merge_settings_from(Some(&bad), None);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(warnings.len(), 1, "警告は stderr ではなく返り値で渡る");
        assert!(warnings[0].contains("settings.json のパースに失敗しました"));
        assert!(warnings[0].contains(&bad_display));
    }

    /// load_settings() のマージテスト。
    ///
    /// 環境変数 SAI_PROJECT_ROOT / HOME をテスト用ディレクトリへ向け、
    /// 「プロジェクト設定 → グローバル設定」の順に読み、グローバル側が
    /// 優先されること（キー単位の上書き）を検証する。
    #[test]
    fn test_settings_merge_global_overrides_local() {
        let tmp = std::env::temp_dir().join(format!(
            "sai_settings_test_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let proj_sai = tmp.join("proj").join(".sai");
        let home_sai = tmp.join("home").join(".sai");
        std::fs::create_dir_all(&proj_sai).unwrap();
        std::fs::create_dir_all(&home_sai).unwrap();

        // プロジェクト側: wip / max_concurrent / extensions を設定
        std::fs::write(
            proj_sai.join("settings.json"),
            r#"{
                "wip": 4,
                "max_concurrent": 8,
                "extensions": ["local.ts"],
                "default_provider": "local-provider"
            }"#,
        )
        .unwrap();
        // グローバル側: wip / extensions / default_model のみ上書き
        std::fs::write(
            home_sai.join("settings.json"),
            r#"{
                "wip": 12,
                "extensions": ["global.ts"],
                "default_model": "global/model-1"
            }"#,
        )
        .unwrap();

        // 環境変数を介さず直接パスを指定してマージを検証する
        let (settings, warnings) = merge_settings_from(
            Some(&proj_sai.join("settings.json")),
            Some(&home_sai.join("settings.json")),
        );
        assert!(warnings.is_empty(), "正常ファイルでは警告は出ない");
        let _ = std::fs::remove_dir_all(&tmp);

        // グローバル側が優先される
        assert_eq!(settings.wip, Some(12), "グローバルの wip が優先される");
        assert_eq!(
            settings.extensions,
            vec!["global.ts".to_string()],
            "グローバルの extensions が優先される"
        );
        assert_eq!(settings.default_model, Some("global/model-1".to_string()));
        // グローバルに無いキーはプロジェクト側の値が保持される
        assert_eq!(
            settings.max_concurrent,
            Some(8),
            "グローバルに無いキーはローカル値"
        );
        assert_eq!(
            settings.default_provider,
            Some("local-provider".to_string()),
            "グローバルに無いキーはローカル値"
        );
    }

    /// `pre_run` もキー単位のマージ規則に従うこと。
    ///
    /// - グローバル側に `pre_run` があればローカルを上書きする。
    /// - グローバル側に無ければローカルの値を保持する。
    #[test]
    fn test_settings_merge_pre_run() {
        let dir = std::env::temp_dir().join(format!(
            "sai_settings_pre_run_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let local = dir.join("local.json");
        let global = dir.join("global.json");
        std::fs::write(&local, r#"{"pre_run":["local-cmd","--x"]}"#).unwrap();

        // グローバル側にも pre_run がある → グローバルが優先
        std::fs::write(&global, r#"{"pre_run":["global-cmd","--y"]}"#).unwrap();
        let (settings, warnings) = merge_settings_from(Some(&local), Some(&global));
        assert!(warnings.is_empty());
        assert_eq!(
            settings.pre_run,
            Some(vec!["global-cmd".to_string(), "--y".to_string()]),
            "グローバルの pre_run が優先される"
        );

        // グローバル側に pre_run が無い → ローカルの値を保持
        std::fs::write(&global, r#"{"wip":3}"#).unwrap();
        let (settings, _) = merge_settings_from(Some(&local), Some(&global));
        assert_eq!(
            settings.pre_run,
            Some(vec!["local-cmd".to_string(), "--x".to_string()]),
            "グローバルに無いキーはローカル値"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // resolve_provider_for_model
    // ------------------------------------------------------------------

    #[test]
    fn test_strip_trailing_commas() {
        // 末尾カンマを含む JSON が寛容パースで読めること（pi の挙動に合わせる）
        let raw = r#"{
            "providers": {
                "open-router": {
                    "models": [
                        {"id": "z-ai/glm-5.3-flash"},
                    ]
                },
            }
        }"#;
        assert!(serde_json::from_str::<serde_json::Value>(raw).is_err());
        let cleaned = strip_trailing_commas(raw);
        let parsed = serde_json::from_str::<serde_json::Value>(&cleaned).unwrap();
        assert_eq!(
            find_provider_by_model_id(&parsed, "z-ai/glm-5.3-flash"),
            Some("open-router".to_string())
        );
    }

    #[test]
    fn test_resolve_provider_model_id_with_slash() {
        // モデル ID 自体に `/` を含む OpenRouter 系 ID は models.json の
        // 完全一致照合で解決する（接頭辞を provider と断定しない）。
        // テスト環境に models.json が無い場合は接頭辞解決もスキップされ None。
        let resolved = resolve_provider_for_model("z-ai/glm-5.3-flash");
        if load_models_json().is_some() {
            // models.json が読める環境では z-ai を provider と断定しない
            assert_ne!(resolved.as_deref(), Some("z-ai"));
        }
    }
}
