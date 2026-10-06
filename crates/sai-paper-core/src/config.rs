//! JSON の設定ファイル（`.sai/paper/settings.json`）。

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors produced while resolving or loading configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// 研究プロジェクト（`.sai/`）が見つからなかった（CLI 共通の約束で終了コード 2）。
    #[error("プロジェクトが見つかりません（`sai-moon-link init` か `sai-moon-paper init` を実行してください）")]
    ProjectNotFound,
    /// プロジェクトはあるが、論文の置き場（`.sai/paper/`）が無かった（終了コード 2）。
    #[error("論文の置き場がありません（`sai-moon-paper init` を実行してください）")]
    PaperDirMissing,
    /// Configuration I/O failed.
    #[error("configuration I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// JSON の読み書きに失敗した。
    #[error("invalid configuration: {0}")]
    Json(#[from] serde_json::Error),
    /// 以前の `config.toml` を読めなかった。
    #[error("invalid legacy config.toml: {0}")]
    Toml(#[from] toml::de::Error),
}

/// Resolved platform-specific filesystem locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// 設定ファイル（`settings.json`）。
    pub config_file: PathBuf,
    /// `SQLite` database file.
    pub database_file: PathBuf,
    /// Default PDF download directory.
    pub downloads_dir: PathBuf,
    /// Directory containing external plugin bundles.
    pub plugins_dir: PathBuf,
    /// Plugin configuration file.
    pub plugins_config_file: PathBuf,
}

impl Paths {
    /// 研究プロジェクトの `.sai/paper/` を解決する（CLI 共通の約束の 5）。
    ///
    /// プロジェクトが見つからないときは [`ConfigError::ProjectNotFound`]、
    /// `.sai/paper/` が無いときは [`ConfigError::PaperDirMissing`] を返す。
    /// 勝手にフォルダを作らない。
    ///
    /// # Errors
    ///
    /// プロジェクト、または論文の置き場が見つからないとき。
    pub fn discover() -> Result<Self, ConfigError> {
        let root = crate::paths::project_root().ok_or(ConfigError::ProjectNotFound)?;
        let data_dir = root.join(".sai").join("paper");
        if !data_dir.is_dir() {
            return Err(ConfigError::PaperDirMissing);
        }
        Ok(Self {
            config_file: data_dir.join(SETTINGS_FILE),
            database_file: data_dir.join("papr.db"),
            downloads_dir: data_dir.join("papers"),
            plugins_dir: data_dir.join("plugins"),
            plugins_config_file: data_dir.join("plugins.toml"),
        })
    }

    /// Return the data directory that contains the database file.
    ///
    /// 引用抽出の Markdown 出力先（`extracts/`）など、DB と同じ場所に置く
    /// データの親ディレクトリとして使用する。
    #[must_use]
    pub fn data_dir(&self) -> PathBuf {
        self.database_file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("data"))
    }

}

/// 設定ファイル（`.sai/paper/settings.json`）の名前。
pub const SETTINGS_FILE: &str = "settings.json";
/// 以前の設定ファイル。見つかれば 1 回だけ `settings.json` へ移す。
pub const LEGACY_CONFIG_FILE: &str = "config.toml";

/// 画面まわりの設定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct UiConfig {
    /// 組み込みテーマ名、または独自テーマ（TOML）へのパス。
    pub theme: String,
    /// 起動時に開く画面。
    pub startup_page: String,
    /// PDF を開くコマンド。
    pub pdf_viewer: Option<String>,
    /// Dashboard のおすすめに使う検索語（カンマ区切り）。
    pub dashboard_keywords: String,
}
impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "catppuccin-mocha".into(),
            startup_page: "dashboard".into(),
            pdf_viewer: None,
            dashboard_keywords: String::new(),
        }
    }
}

/// Library（論文の取り込み元・保存先・プラグイン）の設定。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LibraryConfig {
    /// 外の PDF を読み込む元のフォルダ。
    pub import_folders: Vec<PathBuf>,
    /// ダウンロードした論文の保存先。
    pub download_path: Option<PathBuf>,
    /// 実行を許すプラグインの名前。
    pub enabled_plugins: Vec<String>,
}

/// 論文を探す（検索先・問い合わせ用メール）の設定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DiscoverConfig {
    /// 利用する検索先（s2, arxiv, pubmed, biorxiv, openalex）。
    pub providers: Vec<String>,
    /// S2 API キーを読む環境変数の名前（キー本体は書かない）。
    pub s2_api_key_env: String,
    /// OpenAlex API キーを読む環境変数の名前（キー本体は書かない）。
    pub openalex_api_key_env: String,
    /// OpenAlex の User-Agent に付ける連絡先。2026 年 2 月以降、使用量の上限には効かない。
    pub contact_email: Option<String>,
    /// Unpaywall API に渡すメールアドレス（空なら Unpaywall を使わない）。
    pub unpaywall_email: String,
}
impl Default for DiscoverConfig {
    fn default() -> Self {
        Self {
            providers: vec!["s2".into()],
            s2_api_key_env: "SEMANTICSCHOLAR_API_KEY".into(),
            openalex_api_key_env: "OPENALEX_API_KEY".into(),
            contact_email: None,
            unpaywall_email: String::new(),
        }
    }
}

/// Resources shown on PAPER screen (toggleable, default all true).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[allow(missing_docs)]
pub struct ResourcesConfig {
    pub arxiv: bool,
    pub pdf: bool,
    pub citation: bool,
    pub md: bool,
    pub pdf_jp: bool,
}
impl Default for ResourcesConfig {
    fn default() -> Self {
        Self {
            arxiv: true,
            pdf: true,
            citation: true,
            md: true,
            pdf_jp: true,
        }
    }
}

/// 翻訳サービス 1 つ分の設定（要旨・PDF で共通）。
///
/// `openailiked` は OpenAI 互換 API（既定は dgx の vLLM で動く translateGemma）。
/// API キー本体は設定に書かず、環境変数名だけを持つ。
/// `api_key_env` を空にすると、API キー無しで送る（vLLM はキーを見ない）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TranslateService {
    /// サービス名（要旨: `google` / `openailiked`、PDF: pdf2zh の `--service`）。
    pub service: String,
    /// OpenAI 互換エンドポイントのベース URL。
    pub base_url: String,
    /// 翻訳に使うモデル名（`GET {base_url}/models` で確認する）。
    pub model: String,
    /// API キーを読む環境変数の名前。空ならキー無しで送る。
    pub api_key_env: String,
    /// 訳文の送り方。`""` は指示文つきのチャット、`translategemma` は
    /// `<<<source>>>` / `<<<target>>>` / `<<<text>>>` の区切り記号つき 1 通。
    #[serde(default)]
    pub prompt_format: String,
}
impl Default for TranslateService {
    fn default() -> Self {
        Self {
            service: "openailiked".into(),
            base_url: "http://192.168.100.26:8000/v1".into(),
            model: "translategemma-4b".into(),
            api_key_env: String::new(),
            prompt_format: "translategemma".into(),
        }
    }
}

/// 翻訳の設定（要旨と PDF を別々に選べる）。
///
/// 既定は要旨・PDF 訳とも dgx の vLLM（translateGemma）。`TranslateService::default` と同じ。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TranslateConfig {
    /// 要旨の翻訳（PAPER 画面を開いたときに走る）。
    #[serde(rename = "abstract")]
    pub abstract_: TranslateService,
    /// PDF の翻訳（pdf2zh）。
    pub pdf: TranslateService,
}

/// Radio（sai-radio サーバー）の設定。
///
/// `script` と `voice` は音声を作るたびに sai-radio へ送る（sai-radio 側の
/// `config/*.json` より優先される）。質問応答・ライブ会話など、プロジェクトに
/// 関係なく iPhone から使う設定は sai-radio の `.env` に残す。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RadioConfig {
    /// sai-radio サーバーのベース URL。
    pub server_url: String,
    /// 認証トークンを読む環境変数の名前。空なら認証ヘッダを送らない。
    pub token_env: Option<String>,
    /// HTTP リクエストのタイムアウト秒数。
    pub timeout_sec: u64,
    /// 原稿（台本）を作る設定。
    pub script: RadioScript,
    /// 音声（読み上げ）の設定。
    pub voice: RadioVoice,
}
impl Default for RadioConfig {
    fn default() -> Self {
        Self {
            server_url: "http://127.0.0.1:8000".into(),
            token_env: Some("SAI_RADIO_TOKEN".into()),
            timeout_sec: 1800,
            script: RadioScript::default(),
            voice: RadioVoice::default(),
        }
    }
}

/// Radio の原稿を作る LLM と番組の中身。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RadioScript {
    /// 構成（outline）を作る LLM の提供元。
    pub outline_provider: String,
    /// 構成（outline）を作るモデル。
    pub outline_model: String,
    /// 台本（transcript）を作る LLM の提供元。
    pub transcript_provider: String,
    /// 台本（transcript）を作るモデル。
    pub transcript_model: String,
    /// 番組の方針（どんな番組にするか）。
    pub briefing: String,
    /// 話の区切りの数。
    pub num_segments: u32,
    /// 話す言語。
    pub language: String,
}
impl Default for RadioScript {
    fn default() -> Self {
        Self {
            outline_provider: "openrouter".into(),
            outline_model: "openai/gpt-4o-mini".into(),
            transcript_provider: "openrouter".into(),
            transcript_model: "openai/gpt-4o-mini".into(),
            briefing: "英語論文をランニング中のリスナー向けに、日本語で紹介するポッドキャスト。専門用語は簡単な言葉で言い換えること。".into(),
            num_segments: 5,
            language: "ja".into(),
        }
    }
}

/// Radio の読み上げ（音声サーバーと話者）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RadioVoice {
    /// 音声合成の提供元（podcast-creator の `tts_provider`）。
    pub tts_provider: String,
    /// 音声合成のモデル。
    pub tts_model: String,
    /// 音声サーバーの URL。
    pub base_url: String,
    /// 話者（2 人）。
    pub speakers: Vec<RadioSpeaker>,
}
impl Default for RadioVoice {
    fn default() -> Self {
        Self {
            tts_provider: "openai-compatible".into(),
            tts_model: "aivisspeech".into(),
            base_url: "http://127.0.0.1:8600".into(),
            speakers: vec![
                RadioSpeaker {
                    name: "ホスト".into(),
                    voice_id: "1310138977".into(),
                    backstory: "科学者・研究者として論文の背景や意義を分かりやすく紹介する。".into(),
                    personality: "親切で丁寧。難しい概念を簡単な例えで説明するのが得意。".into(),
                },
                RadioSpeaker {
                    name: "ゲスト".into(),
                    voice_id: "888753760".into(),
                    backstory: "聞き手として興味深い質問を投げかけたり、意外な発見に驚いたりする。".into(),
                    personality: "好奇心旺盛で、分からないことは率直に質問する。".into(),
                },
            ],
        }
    }
}

/// Radio の話者 1 人。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RadioSpeaker {
    /// 名前。
    pub name: String,
    /// 音声サーバーの声の ID。
    pub voice_id: String,
    /// 役どころ。
    pub backstory: String,
    /// 話し方・性格。
    pub personality: String,
}

/// 重い処理（DL 後の自動処理・翻訳・Radio）の設定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PipelineConfig {
    /// 同時に走らせる数の上限（変えたら次の起動から効く）。
    pub max_parallel: usize,
}
impl Default for PipelineConfig {
    fn default() -> Self {
        // docling（Mac の CPU を使う）数本 + 翻訳・Radio（外部 API 待ち）。
        Self { max_parallel: 6 }
    }
}

/// 要約付きの検索（`find --evidence`）の設定。
///
/// LLM を余分に呼ぶので、指定したときだけ使う。`model` が空のときは
/// `--evidence` / MCP の `evidence: true` / TUI の切り替えをエラーにする。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EvidenceConfig {
    /// OpenAI 互換エンドポイントのベース URL。
    pub base_url: String,
    /// 要約に使うモデル名（空のときは要約付きの検索を使えない）。
    pub model: String,
    /// API キーを読む環境変数の名前。
    pub api_key_env: String,
    /// 要約する件数（`find` で取る上位 K 件）。
    pub top_k: i64,
    /// この関連度未満（0-10）の結果を外す。
    pub cutoff: u8,
    /// 要約の長さの上限（文字数、プロンプトで LLM に伝える）。
    pub summary_chars: usize,
    /// 同時に呼ぶチャット API の数。
    pub max_concurrent: usize,
}
impl Default for EvidenceConfig {
    fn default() -> Self {
        Self {
            base_url: "https://openrouter.ai/api/v1".into(),
            model: String::new(),
            api_key_env: "OPENROUTER_API_KEY".into(),
            top_k: 10,
            cutoff: 3,
            summary_chars: 300,
            max_concurrent: 4,
        }
    }
}

/// RAG（ハイブリッド検索）の設定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RagConfig {
    /// 埋め込みエンドポイント（OpenAI 互換）。
    pub embed_endpoint: String,
    /// 埋め込みモデル名。
    pub embed_model: String,
    /// 埋め込み次元。
    pub embed_dim: i64,
    /// 埋め込み API キーの環境変数名。空の場合は認証ヘッダを送らない（ローカルサーバ用）。
    pub embed_api_key_env: String,
    /// デフォルトチャンク戦略（abstract/sentence/paragraph/window）。
    pub chunk_strategy: String,
    /// デフォルト検索モード（word/vector/hybrid）。
    pub default_mode: String,
    /// 上位 K 件。
    pub top_k: i64,
    /// 1 本の論文から出す最大の件数（0 で制限なし）。
    /// 検索結果の上位が 1 本の論文の塊で埋まらないようにするための上限。
    pub per_paper: i64,
    /// ハイブリッド検索の重み（0.0-1.0、BM25 のウェイト）。
    pub alpha: f64,
    /// 融合方法（weighted/rrf）。
    pub fusion: String,
    /// ウィンドウ戦略のウィンドウサイズ（文数）。
    pub window_size: i64,
    /// ウィンドウ戦略のオーバーラップ（文数）。
    pub window_overlap: i64,
    /// チャンク本文の最大トークン数（見積もり）。超える場合は分割する。
    /// 実測に基づいた見積もり: ASCII は約 4.4 文字 / トークン、非 ASCII は約 1.28 文字 / トークン。
    /// OpenRouter の embedding モデルの 512 トークン制限に対する保守的なマージンとして 450 を既定とする。
    pub chunk_max_tokens: usize,
    /// 普段使う索引の名前（`find` / TUI の `f` / MCP で `--index` を省いたとき）。
    pub default_index: String,
    /// 名前付きの索引（分け方・大きさ・埋め込みモデルの組）。書かなかった項目は上の値を使う。
    /// `default` という名前は、ここに書かなくても上の値そのままの索引として使える。
    pub indexes: Vec<RagIndexSpec>,
    /// 要約付きの検索（`find --evidence`）の設定。
    pub evidence: EvidenceConfig,
    /// 論文の節の表（`sections`）の設定。
    pub sections: SectionsConfig,
}

/// 論文の節の表の設定。model・base_url・API キーは `rag.evidence` の値を使う。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SectionsConfig {
    /// true のとき、DL 後の自動処理で節の要約（LLM）も作る。
    pub summarize: bool,
}

/// `rag.indexes` の 1 件（書かなかった項目は `rag` の値を使う）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[allow(missing_docs)]
pub struct RagIndexSpec {
    /// 索引の名前（例: `para-lfm`、`window3-lfm`）。
    pub name: String,
    pub chunk_strategy: Option<String>,
    pub window_size: Option<i64>,
    pub window_overlap: Option<i64>,
    pub chunk_max_tokens: Option<usize>,
    pub embed_endpoint: Option<String>,
    pub embed_model: Option<String>,
    pub embed_dim: Option<i64>,
    pub embed_api_key_env: Option<String>,
    /// 埋め込みに渡す文字の先頭に「タイトル（年）— 章」を付けるか（既定 false）。
    /// 保存する `chunks.text` は変えない。
    pub embed_with_meta: bool,
}

/// 名前で選んだ索引の設定（欠けた項目を `rag` の値で埋めたもの）。
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub struct RagIndex {
    pub name: String,
    pub chunk_strategy: String,
    pub window_size: i64,
    pub window_overlap: i64,
    pub chunk_max_tokens: usize,
    pub embed_endpoint: String,
    pub embed_model: String,
    pub embed_dim: i64,
    pub embed_api_key_env: String,
    /// 埋め込みに渡す文字の先頭に「タイトル（年）— 章」を付けるか。
    pub embed_with_meta: bool,
}

impl RagConfig {
    /// 名前で索引を選ぶ（`None` なら `default_index`）。
    ///
    /// # Errors
    /// その名前の索引が `rag.indexes` に無いとき（`default` を除く）。
    pub fn index(&self, name: Option<&str>) -> Result<RagIndex, String> {
        let name = name.unwrap_or(&self.default_index);
        let spec = match self.indexes.iter().find(|spec| spec.name == name) {
            Some(spec) => spec.clone(),
            None if name == "default" => RagIndexSpec {
                name: name.to_string(),
                ..RagIndexSpec::default()
            },
            None => {
                return Err(format!(
                    "索引 '{name}' がありません（settings.json の rag.indexes: {}）",
                    self.index_names().join(", ")
                ));
            }
        };
        Ok(RagIndex {
            name: spec.name,
            chunk_strategy: spec.chunk_strategy.unwrap_or_else(|| self.chunk_strategy.clone()),
            window_size: spec.window_size.unwrap_or(self.window_size),
            window_overlap: spec.window_overlap.unwrap_or(self.window_overlap),
            chunk_max_tokens: spec.chunk_max_tokens.unwrap_or(self.chunk_max_tokens),
            embed_endpoint: spec.embed_endpoint.unwrap_or_else(|| self.embed_endpoint.clone()),
            embed_model: spec.embed_model.unwrap_or_else(|| self.embed_model.clone()),
            embed_dim: spec.embed_dim.unwrap_or(self.embed_dim),
            embed_api_key_env: spec
                .embed_api_key_env
                .unwrap_or_else(|| self.embed_api_key_env.clone()),
            embed_with_meta: spec.embed_with_meta,
        })
    }

    /// 使える索引の名前（`default` を含む）。
    #[must_use]
    pub fn index_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.indexes.iter().map(|spec| spec.name.clone()).collect();
        if !names.iter().any(|name| name == "default") {
            names.insert(0, "default".to_string());
        }
        names
    }
}
impl Default for RagConfig {
    fn default() -> Self {
        Self {
            embed_endpoint: "https://openrouter.ai/api/v1/embeddings".into(),
            embed_model: "liquid/lfm-2.5-embedding-350m:free".into(),
            embed_dim: 1024,
            embed_api_key_env: "OPENROUTER_API_KEY".into(),
            chunk_strategy: "paragraph".into(),
            default_mode: "hybrid".into(),
            top_k: 20,
            per_paper: 2,
            alpha: 0.7,
            fusion: "weighted".into(),
            window_size: 3,
            window_overlap: 1,
            chunk_max_tokens: 450,
            default_index: "default".into(),
            indexes: Vec::new(),
            evidence: EvidenceConfig::default(),
            sections: SectionsConfig::default(),
        }
    }
}

/// エージェントに渡す「プロジェクトの状況のまとめ」（brief）の長さの設定。
///
/// `max_chars` は brief 全体、それ以外は各部分の上限（文字数）。上限を超えた部分は
/// 切られ、末尾に「…（続きは <パス>）」が付く。`handoffs` / `reviews` は載せる件数。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BriefConfig {
    /// brief 全体の上限（これを超えたら古い review → 古い引き継ぎの順に外す）。
    pub max_chars: usize,
    /// mind.md の上限。
    pub mind_chars: usize,
    /// state.md の上限。
    pub state_chars: usize,
    /// 載せる引き継ぎの件数。
    pub handoffs: usize,
    /// 引き継ぎ 1 件の上限。
    pub handoff_chars: usize,
    /// 載せる review の件数。
    pub reviews: usize,
    /// review 1 件の上限。
    pub review_chars: usize,
    /// 理解の状態の節の上限（超えたら `ok` の古い方から削る）。
    pub understanding_chars: usize,
}
impl Default for BriefConfig {
    fn default() -> Self {
        Self {
            max_chars: 8000,
            mind_chars: 3000,
            state_chars: 800,
            handoffs: 3,
            handoff_chars: 1000,
            reviews: 3,
            review_chars: 400,
            understanding_chars: 600,
        }
    }
}

/// sai-sci の設定（`sci`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SciConfig {
    /// エージェントに渡す範囲（context）の上限（文字数）。
    pub context_max_chars: usize,
}
impl Default for SciConfig {
    fn default() -> Self {
        Self {
            context_max_chars: 12000,
        }
    }
}

/// 「読むべき論文」（READ NEXT）の点数の重み（`progress.read_next`）。
///
/// 点数 = 引き継ぎに出た回数 × `handoff_mention`
/// ＋（検索セットで `keep` なのに読書時間が 0 なら `kept_but_unread`）
/// ＋（note が無ければ `no_note`）
/// ＋（最後に開いてから `stale_days` 日以上なら `stale`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ReadNextWeights {
    /// 引き継ぎの「使った論文」に出た 1 回あたりの点数。
    pub handoff_mention: i64,
    /// 検索セットで `keep` なのに読んでいない（読書時間 0）ときの点数。
    pub kept_but_unread: i64,
    /// note が無いときの点数。
    pub no_note: i64,
    /// これだけの日数以上開いていなければ古いとみなす。
    pub stale_days: i64,
    /// 古いとみなしたときの点数。
    pub stale: i64,
}

impl Default for ReadNextWeights {
    fn default() -> Self {
        Self {
            handoff_mention: 3,
            kept_but_unread: 2,
            no_note: 1,
            stale_days: 30,
            stale: 1,
        }
    }
}

/// プロジェクトの進み具合（`progress`）の設定。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProgressConfig {
    /// 「読むべき論文」（READ NEXT）の点数。
    pub read_next: ReadNextWeights,
}

/// 環境変数の名前の一覧を、優先順に並べて返す（S2 のキー）。
#[must_use]
fn s2_api_key_env_names(configured: &str) -> Vec<&str> {
    // 古い既定 `SCHOLAR_API_KEY` が残っているときは、先に新しい `SEMANTICSCHOLAR_API_KEY` を見る
    if configured == "SCHOLAR_API_KEY" {
        return vec!["SEMANTICSCHOLAR_API_KEY", "SCHOLAR_API_KEY", "S2_API_KEY"];
    }
    vec![configured, "S2_API_KEY", "SCHOLAR_API_KEY"]
}

/// 環境変数をこの順に探し、値がある最初のものを返す（空白だけなら無い扱い）。
fn first_env_value(names: &[&str]) -> Option<String> {
    names
        .iter()
        .copied()
        .filter(|name| !name.is_empty())
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.trim().is_empty()))
}

/// sai-paper の設定（`settings.json`）。欠けた項目は既定値で補い、知らない項目は無視する。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// 画面まわり。
    pub ui: UiConfig,
    /// 取り込み元・保存先・プラグイン。
    pub library: LibraryConfig,
    /// 論文を探す。
    pub discover: DiscoverConfig,
    /// PAPER 画面の RESOURCES に出すもの。
    pub resources: ResourcesConfig,
    /// 翻訳（要旨・PDF）。
    pub translate: TranslateConfig,
    /// Radio（sai-radio）。
    pub radio: RadioConfig,
    /// 重い処理の同時実行数。
    pub pipeline: PipelineConfig,
    /// RAG ハイブリッド検索。
    pub rag: RagConfig,
    /// エージェントに渡す状況まとめ（brief）。
    pub brief: BriefConfig,
    /// プロジェクトの進み具合（progress）。
    pub progress: ProgressConfig,
    /// sai-sci（研究方針の木）。
    pub sci: SciConfig,
}

/// 以前の `config.toml` の形（移行のためだけに読む）。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LegacyConfig {
    theme: Option<String>,
    startup_page: Option<String>,
    pdf_viewer: Option<String>,
    library_folders: Vec<PathBuf>,
    download_path: Option<PathBuf>,
    dashboard_keywords: String,
    enabled_plugins: Vec<String>,
    discover: Option<LegacyDiscover>,
    resources: Option<ResourcesConfig>,
    litbase: Option<LegacyLitbase>,
    translate: Option<TranslateService>,
    podcast: Option<LegacyPodcast>,
    rag: Option<RagConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LegacyDiscover {
    providers: Option<Vec<String>>,
    s2_api_key: Option<String>,
    contact_email: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LegacyLitbase {
    unpaywall_email: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct LegacyPodcast {
    base_url: String,
    api_key_env: Option<String>,
    timeout_sec: u64,
}
impl Default for LegacyPodcast {
    fn default() -> Self {
        let radio = RadioConfig::default();
        Self {
            base_url: radio.server_url,
            api_key_env: radio.token_env,
            timeout_sec: radio.timeout_sec,
        }
    }
}

impl From<LegacyConfig> for Config {
    fn from(old: LegacyConfig) -> Self {
        let defaults = Self::default();
        let discover = old.discover.unwrap_or_default();
        let podcast = old.podcast.unwrap_or_default();
        Self {
            ui: UiConfig {
                theme: old.theme.unwrap_or(defaults.ui.theme),
                startup_page: old.startup_page.unwrap_or(defaults.ui.startup_page),
                pdf_viewer: old.pdf_viewer,
                dashboard_keywords: old.dashboard_keywords,
            },
            library: LibraryConfig {
                import_folders: old.library_folders,
                download_path: old.download_path,
                enabled_plugins: old.enabled_plugins,
            },
            discover: DiscoverConfig {
                providers: discover.providers.unwrap_or(defaults.discover.providers),
                contact_email: discover.contact_email,
                unpaywall_email: old.litbase.unwrap_or_default().unpaywall_email,
                ..defaults.discover
            },
            resources: old.resources.unwrap_or_default(),
            translate: TranslateConfig {
                pdf: old.translate.unwrap_or_default(),
                ..defaults.translate
            },
            radio: RadioConfig {
                server_url: podcast.base_url,
                token_env: podcast.api_key_env,
                timeout_sec: podcast.timeout_sec,
                ..defaults.radio
            },
            pipeline: PipelineConfig::default(),
            rag: old.rag.unwrap_or_default(),
            brief: defaults.brief,
            progress: defaults.progress,
            sci: SciConfig::default(),
        }
    }
}

impl Config {
    /// Read an existing configuration file without creating or modifying it.
    ///
    /// This is used by the live-reload path: an editor may briefly expose a
    /// partially-written file, so reloading must never "upgrade" it in place.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or parsed.
    pub fn load(config_file: &std::path::Path) -> Result<Self, ConfigError> {
        Ok(serde_json::from_str(&fs::read_to_string(config_file)?)?)
    }

    /// 設定を読む。`settings.json` が無ければ、同じ場所の `config.toml` から移すか、
    /// 既定値で作る（移した `config.toml` は `config.toml.migrated` に名前を変える）。
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, written, or parsed.
    pub fn load_or_create(paths: &Paths) -> Result<Self, ConfigError> {
        if paths.config_file.exists() {
            return Self::load(&paths.config_file);
        }
        let dir = paths
            .config_file
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        fs::create_dir_all(&dir)?;

        let legacy_file = dir.join(LEGACY_CONFIG_FILE);
        let config = if legacy_file.is_file() {
            let old: LegacyConfig = toml::from_str(&fs::read_to_string(&legacy_file)?)?;
            if old
                .discover
                .as_ref()
                .and_then(|d| d.s2_api_key.as_deref())
                .is_some_and(|key| !key.trim().is_empty())
            {
                eprintln!(
                    "⚠ config.toml の s2_api_key は移しません。キーは .env の SEMANTICSCHOLAR_API_KEY に置いてください"
                );
            }
            Self::from(old)
        } else {
            let default_pdf_viewer = if cfg!(target_os = "macos") {
                "open"
            } else if cfg!(target_os = "windows") {
                "cmd /C start msedge \"\""
            } else {
                "xdg-open"
            };
            Self {
                ui: UiConfig {
                    pdf_viewer: Some(default_pdf_viewer.to_string()),
                    ..UiConfig::default()
                },
                library: LibraryConfig {
                    import_folders: vec![paths.downloads_dir.clone()],
                    download_path: Some(paths.downloads_dir.clone()),
                    enabled_plugins: Vec::new(),
                },
                ..Self::default()
            }
        };
        config.save(&paths.config_file)?;
        if legacy_file.is_file() {
            fs::rename(&legacy_file, dir.join(format!("{LEGACY_CONFIG_FILE}.migrated")))?;
        }
        Ok(config)
    }

    /// 設定を JSON（整形済み）で書き出す。
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be written.
    pub fn save(&self, config_file: &Path) -> Result<(), ConfigError> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        fs::write(config_file, text)?;
        Ok(())
    }

    /// S2 API キー（`discover.s2_api_key_env` の環境変数。無ければ `S2_API_KEY`）。
    ///
    /// 古い既定の `SCHOLAR_API_KEY` が設定に残っている場合は、先に新しい名前の
    /// `SEMANTICSCHOLAR_API_KEY` を見る。`SCHOLAR_API_KEY` と `S2_API_KEY` も
    /// 引き続き読む。
    ///
    /// 探す順番は [`s2_api_key_env_names`] が返す。
    #[must_use]
    pub fn s2_api_key(&self) -> Option<String> {
        first_env_value(&self.s2_api_key_env_names())
    }

    /// S2 API キーを探す環境変数の名前を、優先順に返す。
    #[must_use]
    pub fn s2_api_key_env_names(&self) -> Vec<&str> {
        s2_api_key_env_names(&self.discover.s2_api_key_env)
    }

    /// OpenAlex API キー（`discover.openalex_api_key_env` の環境変数）。
    #[must_use]
    pub fn openalex_api_key(&self) -> Option<String> {
        first_env_value(&[self.discover.openalex_api_key_env.as_str()])
    }

    /// 設定が使う環境変数のうち、値が無いもの（`config show` の警告用）。
    #[must_use]
    pub fn missing_env_vars(&self) -> Vec<(&'static str, String)> {
        let mut wanted: Vec<(&'static str, &str)> = vec![
            ("translate.pdf.api_key_env", &self.translate.pdf.api_key_env),
            ("rag.embed_api_key_env", &self.rag.embed_api_key_env),
        ];
        if !self.discover.openalex_api_key_env.is_empty() {
            wanted.push((
                "discover.openalex_api_key_env",
                &self.discover.openalex_api_key_env,
            ));
        }
        if self.translate.abstract_.service == "openailiked" {
            wanted.push(("translate.abstract.api_key_env", &self.translate.abstract_.api_key_env));
        }
        if let Some(token) = self.radio.token_env.as_deref() {
            wanted.push(("radio.token_env", token));
        }
        wanted
            .into_iter()
            .filter(|(_, name)| !name.is_empty())
            .filter(|(_, name)| std::env::var(name).map_or(true, |v| v.trim().is_empty()))
            .map(|(field, name)| (field, name.to_string()))
            .collect()
    }

    /// Return configured dashboard recommendation terms.
    #[must_use]
    pub fn dashboard_keyword_list(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.ui
            .dashboard_keywords
            .split(',')
            .map(|keyword| keyword.split_whitespace().collect::<Vec<_>>().join(" "))
            .map(|keyword| keyword.to_lowercase())
            .filter(|keyword| !keyword.is_empty() && seen.insert(keyword.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{s2_api_key_env_names, Config, Paths};
    use std::path::PathBuf;

    fn temp_paths(tag: &str) -> Paths {
        let unique_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("papr_{tag}_{unique_id}"));
        std::fs::create_dir_all(&dir).unwrap();
        Paths {
            config_file: dir.join("settings.json"),
            database_file: dir.join("papr.db"),
            downloads_dir: dir.join("papers"),
            plugins_dir: dir.join("plugins"),
            plugins_config_file: dir.join("plugins.toml"),
        }
    }

    /// 環境変数を入れる必要がないよう、探す順番だけを確かめる。
    #[test]
    fn s2_api_key_search_order_prefers_the_new_name() {
        // 古い既定 `SCHOLAR_API_KEY` が残っている場合は、先に新しい名前を見る
        assert_eq!(
            s2_api_key_env_names("SCHOLAR_API_KEY"),
            vec!["SEMANTICSCHOLAR_API_KEY", "SCHOLAR_API_KEY", "S2_API_KEY"]
        );
        // 新しい既定
        assert_eq!(
            s2_api_key_env_names("SEMANTICSCHOLAR_API_KEY"),
            vec!["SEMANTICSCHOLAR_API_KEY", "S2_API_KEY", "SCHOLAR_API_KEY"]
        );
        // 好きな名前を設定しているとき
        assert_eq!(
            s2_api_key_env_names("MY_KEY"),
            vec!["MY_KEY", "S2_API_KEY", "SCHOLAR_API_KEY"]
        );
    }

    #[test]
    fn default_discover_uses_the_new_env_var_names() {
        let config = Config::default();
        assert_eq!(config.discover.s2_api_key_env, "SEMANTICSCHOLAR_API_KEY");
        assert_eq!(config.discover.openalex_api_key_env, "OPENALEX_API_KEY");
        assert_eq!(config.s2_api_key_env_names(), s2_api_key_env_names("SEMANTICSCHOLAR_API_KEY"));
    }

    /// `discover` に `openalex_api_key_env` が無くても読める（今の settings.json との互換）。
    #[test]
    fn discover_config_reads_settings_without_the_openalex_key_env() -> Result<(), serde_json::Error> {
        let config: Config = serde_json::from_str(
            r#"{"discover":{"providers":["openalex"],"s2_api_key_env":"SCHOLAR_API_KEY","contact_email":null,"unpaywall_email":""}}"#,
        )?;
        // 項目が無ければ既定の名前に戻る
        assert_eq!(config.discover.s2_api_key_env, "SCHOLAR_API_KEY");
        assert_eq!(config.discover.openalex_api_key_env, "OPENALEX_API_KEY");
        // 古い既定の S2 の名前は、先に新しい名前を見る
        assert_eq!(
            config.s2_api_key_env_names(),
            vec!["SEMANTICSCHOLAR_API_KEY", "SCHOLAR_API_KEY", "S2_API_KEY"]
        );

        // `config show` の警告に OpenAlex の環境変数が出る
        let mut config = config;
        config.discover.openalex_api_key_env = "SAI_TEST_OPENALEX_KEY_UNSET".into();
        assert!(config
            .missing_env_vars()
            .iter()
            .any(|(field, name)| *field == "discover.openalex_api_key_env"
                && name == "SAI_TEST_OPENALEX_KEY_UNSET"));
        Ok(())
    }

    #[test]
    fn paths_discover_requires_a_project_with_paper_dir() {
        // 環境変数を書き換えられないので、`discover` がプロジェクトを見つけない
        // ことを直接は確かめられない。ここでは `Paths` の組み立てだけを確かめる。
        let paths = Paths {
            config_file: PathBuf::from("/tmp/p/.sai/paper/settings.json"),
            database_file: PathBuf::from("/tmp/p/.sai/paper/papr.db"),
            downloads_dir: PathBuf::from("/tmp/p/.sai/paper/papers"),
            plugins_dir: PathBuf::from("/tmp/p/.sai/paper/plugins"),
            plugins_config_file: PathBuf::from("/tmp/p/.sai/paper/plugins.toml"),
        };
        assert_eq!(paths.data_dir(), PathBuf::from("/tmp/p/.sai/paper"));
    }

    /// 項目が欠けた JSON・知らない項目がある JSON も読め、欠けた所は既定値になる。
    #[test]
    fn partial_json_is_filled_with_defaults() -> Result<(), serde_json::Error> {
        let config: Config = serde_json::from_str(
            r#"{"ui": {"theme": "nord"}, "translate": {"pdf": {"model": "m"}}, "unknown": 1}"#,
        )?;
        assert_eq!(config.ui.theme, "nord");
        assert_eq!(config.ui.startup_page, "dashboard");
        assert_eq!(config.translate.pdf.model, "m");
        assert_eq!(config.translate.pdf.service, "openailiked");
        assert_eq!(config.translate.abstract_.service, "openailiked");
        // 足りない項目は dgx の translateGemma の既定になる
        assert_eq!(config.translate.abstract_.base_url, "http://192.168.100.26:8000/v1");
        assert_eq!(config.translate.abstract_.model, "translategemma-4b");
        assert_eq!(config.translate.abstract_.api_key_env, "");
        assert_eq!(config.translate.abstract_.prompt_format, "translategemma");
        assert_eq!(config.discover.providers, ["s2"]);
        assert_eq!(config.rag.top_k, 20);
        Ok(())
    }

    #[test]
    fn dashboard_keywords_normalize_spacing_and_remove_duplicates() {
        let mut config = Config::default();
        config.ui.dashboard_keywords =
            "Machine   Learning, machine learning, GRAVITATIONAL WAVES,  ".into();
        assert_eq!(
            config.dashboard_keyword_list(),
            ["machine learning", "gravitational waves"]
        );
    }

    /// 設定ファイルが無ければ既定値で settings.json を作り、読み直すと同じになる。
    #[test]
    fn load_or_create_writes_default_json() -> Result<(), Box<dyn std::error::Error>> {
        let paths = temp_paths("create");
        let config = Config::load_or_create(&paths)?;
        assert_eq!(config.library.import_folders, [paths.downloads_dir.clone()]);
        assert!(config.ui.pdf_viewer.is_some());
        assert_eq!(config.discover.providers, ["s2"]);
        assert_eq!(config.discover.unpaywall_email, "");
        assert_eq!(Config::load(&paths.config_file)?, config);
        let dir = paths.config_file.parent().unwrap();
        assert!(!dir.join("config.toml").exists());
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    /// config.toml があれば中身を settings.json へ移し、config.toml.migrated に名前を変える。
    #[test]
    fn legacy_toml_is_migrated_once() -> Result<(), Box<dyn std::error::Error>> {
        let paths = temp_paths("migrate");
        let dir = paths.config_file.parent().unwrap().to_path_buf();
        std::fs::write(
            dir.join("config.toml"),
            r#"theme = "nord"
pdf_viewer = "open"
library_folders = ["/x/papers"]
download_path = "/x/papers"
[discover]
providers = ["arxiv", "s2"]
[litbase]
unpaywall_email = "a@b.c"
[translate]
service = "openailiked"
model = "m1"
[podcast]
base_url = "http://h:1"
timeout_sec = 5
[rag]
top_k = 7
"#,
        )?;
        let config = Config::load_or_create(&paths)?;
        assert_eq!(config.ui.theme, "nord");
        assert_eq!(config.ui.pdf_viewer.as_deref(), Some("open"));
        assert_eq!(config.library.import_folders, [std::path::PathBuf::from("/x/papers")]);
        assert_eq!(config.discover.providers, ["arxiv", "s2"]);
        assert_eq!(config.discover.unpaywall_email, "a@b.c");
        assert_eq!(config.translate.pdf.model, "m1");
        assert_eq!(config.translate.pdf.api_key_env, "");
        assert_eq!(config.translate.abstract_.service, "openailiked");
        assert_eq!(config.radio.server_url, "http://h:1");
        assert_eq!(config.radio.token_env.as_deref(), Some("SAI_RADIO_TOKEN"));
        assert_eq!(config.radio.timeout_sec, 5);
        assert_eq!(config.rag.top_k, 7);
        assert!(!dir.join("config.toml").exists());
        assert!(dir.join("config.toml.migrated").exists());
        assert_eq!(Config::load(&paths.config_file)?, config);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// 索引は名前で選べ、書かなかった項目は rag の値で埋まる。default は書かなくても使える。
    #[test]
    fn rag_indexes_resolve_by_name() {
        let rag: super::RagConfig = serde_json::from_str(
            r#"{"indexes": [{"name": "window3-lfm", "chunk_strategy": "window", "window_size": 3}]}"#,
        )
        .unwrap();
        let window = rag.index(Some("window3-lfm")).unwrap();
        assert_eq!(window.chunk_strategy, "window");
        assert_eq!(window.window_size, 3);
        assert_eq!(window.embed_model, rag.embed_model);
        // `per_paper` が無い settings.json でも既定の 2 になる（`#[serde(default)]`）。
        assert_eq!(rag.per_paper, 2);
        let default = rag.index(None).unwrap();
        assert_eq!(default.name, "default");
        assert_eq!(default.chunk_strategy, "paragraph");
        assert!(rag.index(Some("nope")).is_err());
        assert_eq!(rag.index_names(), ["default", "window3-lfm"]);
    }
}

#[cfg(test)]
mod sci_tests {
    use super::{Config, SciConfig};

    /// `sci` が無ければ既定値、あれば書かれた値を読むこと。
    #[test]
    fn sci_config_defaults_and_reads() {
        let default = Config::default();
        assert_eq!(default.sci.context_max_chars, 12_000);

        let read: Config = serde_json::from_str(r#"{"sci": {"context_max_chars": 500}}"#).unwrap();
        assert_eq!(
            read.sci,
            SciConfig {
                context_max_chars: 500
            }
        );
    }
}
