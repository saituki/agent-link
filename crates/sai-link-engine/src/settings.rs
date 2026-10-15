//! # 設定（`.sai/settings.json`）と全 runner 共通の上限・環境変数
//!
//! `.sai/settings.json`（プロジェクトと `~/.sai/`）を読む関数と、WIP・同時実行の上限、
//! 起動コマンドに付ける環境変数（`env`）を提供します。pi の起動コマンドは U04 で消した。
//! 旧 `pi_command` から移した（名前と引数は変えていない。共有契約 C-SETTINGS）。

// ============================================================================
// SettingsJson — .sai/settings.json
// ============================================================================

/// `.sai/settings.json` の構造。
///
/// ブロック定義の frontmatter に `model`/`provider`/`thinking` が無い場合の
/// デフォルト値を提供します。
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
    /// エージェント起動直前に走らせるコマンド（配列の先頭がプログラム、残りが引数。シェルは通さない）
    #[serde(default)]
    pub pre_run: Option<Vec<String>>,
    /// `sai-link doc write` が保存の前に本文を標準入力で通す命令（配列。`{run_id}`・`{block}`・`{filename}` を置き換える。シェルは通さない）
    #[serde(default)]
    pub write_check: Option<Vec<String>>,
    /// エージェントの起動コマンドの先頭に付ける環境変数（KEY → 値）。
    ///
    /// `.sai/settings.json` と `~/.sai/settings.json` の両方で書ける（後者が優先）。
    /// 接続先などを 1 か所に書いておく例:
    ///
    /// ```json
    /// "env": {
    ///   "EXAMPLE_URL": "http://127.0.0.1:5001"
    /// }
    /// ```
    ///
    /// 値の検証（キーの形・予約キー・改行・長さ）は [`merge_settings_file`] で行い、
    /// 通らないものは警告だけ出して破棄する。
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

/// `env` のキーが予約キー（sai-link 自身が必ず上書きする 4 つ）のときは破棄する。
///
/// sai-link は `SAI_PROJECT_ROOT` / `SAI_PARENT_RUN_ID` / `SAI_BLOCK_NAME` /
/// `SAI_IS_SUBTASK` を起動コマンドの後ろ（= より強い位置）で付ける。
/// 利用者が設定に書いても二重にならないようここで弾く。
const RESERVED_ENV_KEYS: [&str; 4] = [
    "SAI_PROJECT_ROOT",
    "SAI_PARENT_RUN_ID",
    "SAI_BLOCK_NAME",
    "SAI_IS_SUBTASK",
];

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
    let global_path = crate::project::sai_home_path("settings.json");

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
    if parsed.pre_run.is_some() {
        merged.pre_run = parsed.pre_run;
    }
    if parsed.write_check.is_some() {
        merged.write_check = parsed.write_check;
    }
    if !parsed.env.is_empty() {
        // `env` もキー単位のマージ。同じキーが両方にあればグローバル側が勝つ。
        // 不正な項目（使えないキー・予約キー・改行・長すぎる値）は破棄し、
        // 警告を 1 件ずつ積むだけにしてエラーにはしない（既存の extensions と同じ方針）。
        for (key, value) in parsed.env {
            match check_env_entry(&key, &value) {
                Ok(()) => {
                    merged.env.insert(key, value);
                }
                Err(reason) => warnings.push(reason),
            }
        }
    }
}

/// `env` の 1 項目を検査する。通れば `Ok(())`、破棄すべきなら理由つきの `Err`。
///
/// - キーは `^[A-Za-z_][A-Za-z0-9_]*$`
/// - 予約キー（[`RESERVED_ENV_KEYS`]）は使わない
/// - 値は 1 行（`\n` / `\r` を含まない）・長さ 1〜2000
fn check_env_entry(key: &str, value: &str) -> Result<(), String> {
    let mut chars = key.chars();
    let head_ok = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_');
    let tail_ok = chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !head_ok || !tail_ok {
        return Err(format!(
            "settings.json の env のキー {:?} は A-Za-z_ で始まり A-Za-z0-9_ だけからなる名前ではありません",
            key
        ));
    }
    if RESERVED_ENV_KEYS.contains(&key) {
        return Err(format!(
            "settings.json の env のキー {} は sai-link 自身が使う予約キーのため破棄しました",
            key
        ));
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(format!(
            "settings.json の env の値 {} が 1 行に収まらないため破棄しました",
            key
        ));
    }
    if value.is_empty() || value.chars().count() > 2000 {
        return Err(format!(
            "settings.json の env の値 {} は 1〜2000 文字のあいだにしてください（{} 文字）",
            key,
            value.chars().count()
        ));
    }
    Ok(())
}

/// 設定の `env` を検証済みの `Vec<(String, String)>` で返す。無効なものは除いてある。
///
/// 返り値が `None` のときは `env` が空（= 何も付けない）のとき。
/// 警告は [`load_settings_quiet`] の場合に戻すが、ここでは出さない
/// （`prepare_pipeline` のような起動途中の経路で stderr に出すと画面が壊れるため）。
/// 設定の読み込みはここで 1 回だけ行う（呼び出し側が重ねて読まないこと）。
pub fn agent_env_pairs() -> Option<Vec<(String, String)>> {
    let (settings, _warnings) = load_settings_quiet();
    if settings.env.is_empty() {
        return None;
    }
    // BTreeMap から取り出したので既にキー順に整っている（並びが毎回同じになる）。
    let pairs: Vec<(String, String)> = settings
        .env
        .iter()
        .filter(|(k, v)| check_env_entry(k, v).is_ok())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Some(pairs)
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
// 内部ヘルパー関数（モデル ID からプロバイダを引く）
// ============================================================================

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

#[cfg(test)]
mod tests {
    use super::*;


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

        // プロジェクト側: wip / max_concurrent / default_provider を設定
        // （廃止した extensions が残っていても読み込みが失敗しないことも見る）
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
        // グローバル側: wip / default_model のみ上書き（旧 extensions が残っていても無視される）
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
    // 設定順序: settings.json の env（エージェントに渡す環境変数）
    // ------------------------------------------------------------------

    /// `env` のキーの検査。不正なキー（ハイフン・数字で始まる）と予約キーは弾く。
    #[test]
    fn env_rejects_bad_keys() {
        // 正しいキーは通る
        assert!(check_env_entry("EXAMPLE_URL", "http://127.0.0.1:5001").is_ok());
        assert!(check_env_entry("_A1", "x").is_ok());

        // ハイフン・数字で始まる・空文字は通らない
        assert!(check_env_entry("BAD-KEY", "1").is_err());
        assert!(check_env_entry("1BAD", "1").is_err());
        assert!(check_env_entry("", "1").is_err());

        // sai-link 自身が上書きする 4 つは予約
        for reserved in RESERVED_ENV_KEYS {
            assert!(
                check_env_entry(reserved, "x").is_err(),
                "{reserved} は予約キーでなければならない"
            );
        }
    }

    /// `env` の値の検査。改行を含む値と長すぎる値は破棄する。
    #[test]
    fn env_rejects_bad_values() {
        // 1 行なら何でもいい（空白・シングルクォートを含む値も通す。エスケープはする）
        assert!(check_env_entry("MULTI", "a b 'c'").is_ok());

        // 改行（\n / \r）は通らない
        assert!(check_env_entry("MULTI", "a\nb").is_err());
        assert!(check_env_entry("MULTI", "a\rb").is_err());

        // 空と 2001 文字は通らない。2000 文字ちょうどは通る
        assert!(check_env_entry("EMPTY", "").is_err());
        assert!(check_env_entry("LONG", &"a".repeat(2001)).is_err());
        assert!(check_env_entry("LONG", &"a".repeat(2000)).is_ok());
    }

    /// `env` もキー単位のマージ規則に従い、不正な項目は警告だけ出して破棄すること。
    #[test]
    fn test_settings_merge_env() {
        let dir = std::env::temp_dir().join(format!(
            "sai_settings_env_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let local = dir.join("local.json");
        let global = dir.join("global.json");

        // ローカル側: 正しい項目・予約キー・不正なキー・改行を含む値
        std::fs::write(
            &local,
            r#"{"env": {
                "EXAMPLE_URL": "http://127.0.0.1:5001",
                "AWS_DEFAULT_REGION": "us-east-1",
                "SAI_PROJECT_ROOT": "/tmp/x",
                "BAD-KEY": "1",
                "MULTI": "a\nb"
            }}"#,
        )
        .unwrap();
        std::fs::write(&global, r#"{"wip": 3}"#).unwrap();
        let (settings, _) = merge_settings_from(Some(&local), Some(&global));
        assert_eq!(
            settings.env.get("EXAMPLE_URL").map(String::as_str),
            Some("http://127.0.0.1:5001"),
            "ローカルの env が取り込まれる"
        );
        assert_eq!(
            settings.env.get("AWS_DEFAULT_REGION").map(String::as_str),
            Some("us-east-1")
        );
        // 不正なキー（ハイフン・数字で始まる）と予約キー、改行を含む値は
        // 破棄して 1 件ずつ警告する（エラーにはしない）
        let (_settings, warnings) = merge_settings_from(Some(&local), Some(&global));
        assert!(
            !settings.env.contains_key("SAI_PROJECT_ROOT"),
            "予約キーは破棄される"
        );
        assert!(
            !settings.env.contains_key("BAD-KEY"),
            "不正なキーは破棄される"
        );
        assert!(
            !settings.env.contains_key("MULTI"),
            "改行を含む値は破棄される"
        );
        assert_eq!(
            warnings.len(),
            3,
            "破棄した 3 件ぶんの警告が出る: {:?}",
            warnings
        );

        // グローバル側にも env がある → キー単位でグローバルが優先される
        std::fs::write(
            &global,
            r#"{"env": {"EXAMPLE_URL": "http://192.168.0.2:8443"}}"#,
        )
        .unwrap();
        let (settings, _) = merge_settings_from(Some(&local), Some(&global));
        assert_eq!(
            settings.env.get("EXAMPLE_URL").map(String::as_str),
            Some("http://192.168.0.2:8443"),
            "同じキーはグローバルが優先される"
        );
        assert_eq!(
            settings.env.get("AWS_DEFAULT_REGION").map(String::as_str),
            Some("us-east-1"),
            "グローバルが持たないキーはローカルの値が残る"
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
