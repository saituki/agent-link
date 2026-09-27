//! sai-radio ポッドキャスト生成 API のクライアント。
//!
//! sai-radio HTTP サーバーへリクエストを送信し、
//! 抽出済み Markdown からポッドキャスト音声を生成する。

use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;
use thiserror::Error;

/// sai-radio に送る本文の文字数の上限。
///
/// ponytail: 英文 1 トークン約 4 文字として、sai-radio の LLM の 128k トークンに
/// 収まる目安。モデルを変えたら見直す。超えたら送らずに止める。
const RADIO_MAX_CHARS: usize = 300_000;

/// 起動しているか確かめるサーバーへの要求の時間の上限。
const RADIO_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// ポッドキャスト生成のエラー。
#[derive(Debug, Error)]
pub enum PodcastClientError {
    /// HTTP リクエストが失敗した。
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    /// サーバーが HTTP エラーレスポンスを返した。
    #[error("server error: {0}")]
    Service(String),
}

/// ポッドキャスト生成操作の結果型。
pub type Result<T> = std::result::Result<T, PodcastClientError>;

/// sai-radio に送る論文 ID（`<root のフォルダ名>-<論文番号>`、計画 16 §3.3）。
///
/// TUI の `r` キーと CLI の `phone radio` / `phone paper` が同じ名前になるよう、
/// 音声の名前はこの関数だけで決める。sai-paper はフォルダ名だけを知っていればよく、
/// 「フォルダ名 → プロジェクト」の引き当ては sai-phone 側で行う。
/// ルートが取れない場合は番号だけを返す（従来どおり・プロジェクト不明扱い）。
#[must_use]
pub fn radio_paper_id(paper_id: i64) -> String {
    radio_paper_id_in(crate::paths::project_root().as_deref(), paper_id)
}

/// [`radio_paper_id`] のルート明示版（ルート解決のテスト用）。
#[must_use]
pub fn radio_paper_id_in(root: Option<&Path>, paper_id: i64) -> String {
    let folder = root
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty());
    match folder {
        Some(name) => format!("{name}-{paper_id}"),
        None => paper_id.to_string(),
    }
}

/// sai-radio が返したエラーのうち、声のサーバーに届かなかったことを示す文。
const VOICE_UNREACHABLE_DETAIL: &str = "All connection attempts failed";

/// sai-radio API の from-extract リクエストボディ。
#[derive(Serialize)]
struct FromExtractRequest<'a> {
    paper_id: &'a str,
    title: &'a str,
    markdown: &'a str,
    mode: &'a str,
    /// 原稿と音声の設定（`settings.json` の `radio.script` / `radio.voice`）。
    settings: RadioRequestSettings<'a>,
    /// 音声を書く場所（論文フォルダの絶対パス）。
    audio_dir: String,
}

#[derive(Serialize)]
struct RadioRequestSettings<'a> {
    script: &'a crate::config::RadioScript,
    voice: &'a crate::config::RadioVoice,
}

/// sai-radio API の from-extract レスポンスボディ。
#[derive(Deserialize)]
pub struct FromExtractResponse {
    /// 論文の ID（sai-radio 側の表記）。
    pub paper_id: String,
    /// 番組の題名。
    pub title: String,
    /// 作り方（原稿の種類）。
    pub mode: String,
}

/// sai-radio サーバーへポッドキャスト生成をリクエストする。
///
/// sai-radio は `http://127.0.0.1:8000` で常時起動している前提。
/// リクエストは数分かかる場合があるため、タイムアウトは長め（既定 1800 秒）に設定している。
pub async fn generate_podcast(
    config: &crate::config::RadioConfig,
    paper_id: &str,
    title: &str,
    markdown: &str,
    audio_dir: &Path,
) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(config.timeout_sec))
        .build()?;

    let mut req = client.post(format!(
        "{}/api/papers/from-extract",
        config.server_url.trim_end_matches('/')
    ));

    // API キー認証（設定されていれば）
    if let Some(env_name) = &config.token_env {
        if let Ok(token) = std::env::var(env_name) {
            if !token.is_empty() {
                req = req.bearer_auth(token);
            }
        }
    }

    let payload = from_extract_request(config, paper_id, title, markdown, audio_dir);

    let resp = req.json(&payload).send().await?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(PodcastClientError::Service(body));
    }

    Ok(())
}

/// Markdown から文字で埋め込まれた画像（`data:image/...`）を取り除く。
///
/// extract.md には図が base64 の `data:image/png;base64,...` として埋め込まれており、
/// そのまま送ると sai-radio の LLM の文脈長の上限を超える（1 枚で数十万トークン）。
/// 通常の画像（`![](fig1.png)`）と画像以外の本文はそのまま残す。
#[must_use]
pub fn strip_embedded_images(markdown: &str) -> String {
    static MD_DATA_IMAGE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"!\[[^\]]*\]\(data:image/[^)]*\)").expect("markdown data image regex")
    });
    static HTML_DATA_IMAGE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"<img[^>]*src=["']data:image/[^"']*["'][^>]*>"#)
            .expect("html data image regex")
    });
    let text = MD_DATA_IMAGE.replace_all(markdown, "");
    HTML_DATA_IMAGE.replace_all(&text, "").into_owned()
}

/// 送る前の長さの判定（画像を取り除いたあとの文字数が上限を超えるか）。
#[must_use]
pub fn radio_markdown_too_long(markdown: &str) -> Option<usize> {
    let n = markdown.chars().count();
    (n > RADIO_MAX_CHARS).then_some(n)
}

/// サーバーへ届くか確かめる（HTTP の応答が返れば「届いた」とみなす）。
///
/// 返り値が `false` なら、接続できない・時間切れ。sai-radio と AivisSpeech Engine の
/// どちらが stop しているかを切り分けるために使う。
pub async fn server_reachable(base_url: &str) -> bool {
    let url = format!("{}/", base_url.trim_end_matches('/'));
    match Client::builder()
        .timeout(RADIO_PROBE_TIMEOUT)
        .build()
    {
        Ok(client) => client.get(&url).send().await.is_ok(),
        Err(_) => false,
    }
}

/// sai-radio が返したエラーの文を利用者向けにする。
///
/// 声のサーバーに届かなかった場合は、AivisSpeech Engine が止まっていると分かる文に
/// 置き換える（どちらが落ちていたか分からなかったため）。
fn radio_error_message(detail: String, voice_base_url: &str) -> String {
    if detail.contains(VOICE_UNREACHABLE_DETAIL) {
        voice_unreachable_message(voice_base_url)
    } else {
        detail
    }
}

/// 音声サーバーに届かないときの、利用者向けの文。
#[must_use]
pub fn voice_unreachable_message(voice_base_url: &str) -> String {
    format!("音声サーバー（{voice_base_url}）に接続できません。AivisSpeech Engine を起動してください")
}

/// sai-radio へ送る本文のチェックと結果。
///
/// `Ok` は送れる本文（埋め込み画像は取り除いたあと）、`Err` は送らずに止まる理由。
pub fn prepare_radio_markdown(markdown: &str) -> std::result::Result<String, String> {
    let stripped = strip_embedded_images(markdown);
    if let Some(n) = radio_markdown_too_long(&stripped) {
        return Err(format!(
            "本文が長すぎます（{n} 文字。上限 {RADIO_MAX_CHARS} 文字）"
        ));
    }
    Ok(stripped)
}

fn from_extract_request<'a>(
    config: &'a crate::config::RadioConfig,
    paper_id: &'a str,
    title: &'a str,
    markdown: &'a str,
    audio_dir: &Path,
) -> FromExtractRequest<'a> {
    FromExtractRequest {
        paper_id,
        title,
        markdown,
        mode: "podcast",
        settings: RadioRequestSettings {
            script: &config.script,
            voice: &config.voice,
        },
        audio_dir: audio_dir.to_string_lossy().into_owned(),
    }
}

/// Library の論文 1 本を音声化する（TUI の `r` キーと CLI の `phone radio` の共通処理）。
///
/// `paper_podcasts` の状態（generating → ready / failed）もここで書く。
/// 戻り値の `Err` は利用者向けの日本語メッセージ、`Ok` は sai-radio に送った
/// 論文 ID（`radio_paper_id` が決める `<フォルダ名>-<番号>`）。
///
/// sai-radio は数分かかる同期 API のため、呼び出し側は完了まで待つ。
pub async fn generate_for_paper(
    db_file: &Path,
    paper_id: i64,
    config: &crate::config::RadioConfig,
) -> std::result::Result<String, String> {
    // `jobs` 表に記録し、同じ論文の音声化が実行中なら始めない（段階 5）。
    crate::jobs::run(db_file, "radio", Some(paper_id), || {
        generate_for_paper_inner(db_file, paper_id, config)
    })
    .await
}

async fn generate_for_paper_inner(
    db_file: &Path,
    paper_id: i64,
    config: &crate::config::RadioConfig,
) -> std::result::Result<String, String> {
    // 0. sai-radio に送る論文 ID（TUI / CLI で同じ名前になるようここで決める）。
    let radio_id = radio_paper_id(paper_id);

    // 1. DB に generating ステータスを記録（書けなくても処理は続行）。
    if let Ok(db) = crate::Database::open(db_file) {
        let _ = db.connection().execute(
            "INSERT INTO paper_podcasts (paper_id, status, error) VALUES (?1, 'generating', NULL) ON CONFLICT(paper_id) DO UPDATE SET status='generating', error=NULL",
            rusqlite::params![paper_id],
        );
    }

    // 失敗を `paper_podcasts` に記録して、そのままメッセージを返す。
    let fail = |message: String| -> String {
        if let Ok(db) = crate::Database::open(db_file) {
            let _ = db.connection().execute(
                "INSERT INTO paper_podcasts (paper_id, status, error) VALUES (?1, 'failed', ?2) ON CONFLICT(paper_id) DO UPDATE SET status='failed', error=?2",
                rusqlite::params![paper_id, message],
            );
        }
        message
    };

    // 2. extract.md を読み込む（DB 接続は await の前で閉じる）。
    let (title, markdown, paper_dir) = {
        let db = match crate::Database::open(db_file) {
            Ok(db) => db,
            Err(e) => return Err(fail(format!("DB open error {e}"))),
        };
        let Some(paper) = db.library_paper_by_id(paper_id).ok().flatten() else {
            return Err(fail("論文が見つかりません".to_string()));
        };
        let Some(roots) = crate::paths::paper_roots() else {
            return Err(fail("データディレクトリが見つかりません".to_string()));
        };

        // 論文フォルダ（paths::paper_folder）の extract.md を優先し、無ければ
        // 本文表示と同じ解決順（引用抽出の記録 → PDF のあるフォルダ → 旧 extracts/）で探す。
        let paper_dir = crate::paths::paper_folder(&roots.data_dir, paper_id, &paper.title);
        let primary = crate::paths::paper_file_path(&paper_dir, "extract");
        let found = if primary.is_file() {
            Some(primary)
        } else {
            crate::citation::resolve_extract_md(
                db.connection(),
                Some(paper_id),
                None,
                &roots.data_dir,
            )
        };
        let markdown = match found {
            Some(path) => match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(e) => return Err(fail(format!("extract.md 読み込み失敗: {e}"))),
            },
            None => {
                return Err(fail("extract.md 読み込み失敗: extract.md が見つかりません".to_string()));
            }
        };
        (paper.title, markdown, paper_dir)
    };

    // 3. 送る前に 2 つのサーバーに届くか確かめる。どちらが止まっているかを名前にする。
    if !server_reachable(&config.server_url).await {
        return Err(fail(format!(
            "sai-radio（{}）に接続できません。sai-radio を起動してください",
            config.server_url
        )));
    }
    if !server_reachable(&config.voice.base_url).await {
        return Err(fail(voice_unreachable_message(&config.voice.base_url)));
    }

    // 4. 埋め込み画像を取り除き、長すぎるときは送らずに止める。
    let markdown = match prepare_radio_markdown(&markdown) {
        Ok(stripped) => stripped,
        Err(message) => return Err(fail(message)),
    };

    // 5. sai-radio API を呼び出す（数分かかる）。音声は論文フォルダに書かれる。
    match generate_podcast(config, &radio_id, &title, &markdown, &paper_dir).await {
        Ok(()) => {
            // 成功: ready ステータスで記録。
            if let Ok(db) = crate::Database::open(db_file) {
                let _ = db.connection().execute(
                    "INSERT INTO paper_podcasts (paper_id, status, generated_at, error) VALUES (?1, 'ready', ?2, NULL) ON CONFLICT(paper_id) DO UPDATE SET status='ready', generated_at=?2, error=NULL",
                    rusqlite::params![paper_id, chrono::Utc::now().to_rfc3339()],
                );
                // Dashboard の出来事に「radio」（音声の作成）として残す
                let _ = db.record_activity("radio", Some(paper_id), Some(&radio_id));
            }
            Ok(radio_id)
        }
        Err(e) => {
            // 失敗: ステータスとエラーを記録し、利用者向けメッセージを返す。
            let error_msg = match e {
                PodcastClientError::Request(req_err) => {
                    if req_err.is_timeout() || req_err.is_connect() {
                        "sai-radioサーバーに接続できません。起動しているか確認してください".to_string()
                    } else {
                        format!("接続エラー: {req_err}")
                    }
                }
                PodcastClientError::Service(detail) => {
                    radio_error_message(detail, &config.voice.base_url)
                }
            };
            Err(fail(error_msg))
        }
    }
}

// --- Radio の出来事（Dashboard の RECENT ACTIVITY に混ぜる） ---

/// sai-radio から一度に読む記録の件数（全プロジェクト分が混ざって返るため多め）。
const RADIO_LOG_LIMIT: u32 = 2000;
/// sai-radio の記録を待つ秒数（止まっていても Dashboard がすぐ出るよう短く）。
const RADIO_LOG_TIMEOUT_SEC: u64 = 1;

/// sai-radio の `GET /api/logs` の 1 件。
#[derive(Debug, Clone, Deserialize)]
pub struct RadioEvent {
    /// 音声の名前（`<フォルダ名>-<番号>` か、古い音声の 16 進 ID）
    pub paper_id: String,
    /// `listen`（聴いた）か `question`（質問した）
    pub kind: String,
    /// 質問の中身など
    #[serde(default)]
    pub payload: serde_json::Value,
    /// UNIX 秒
    pub created_at: f64,
}

/// sai-radio の記録を、このプロジェクトの論文の分だけ出来事の行にする。
///
/// - 音声の名前が `<folder>-<番号>` のものだけを拾う（別プロジェクト・古い音声は捨てる）。
/// - `listen` は `radio_play`、`question` は `radio_question`。それ以外の種類は捨てる。
/// - 題名は `titles`（論文番号 → 題名）から引く。無ければ音声の名前を出す。
#[must_use]
pub fn radio_events_to_activity(
    events: &[RadioEvent],
    folder: &str,
    titles: &std::collections::HashMap<i64, String>,
) -> Vec<crate::ActivityItem> {
    let prefix = format!("{folder}-");
    events
        .iter()
        .filter_map(|event| {
            let number: i64 = event.paper_id.strip_prefix(&prefix)?.parse().ok()?;
            let kind = match event.kind.as_str() {
                "listen" => "radio_play",
                "question" => "radio_question",
                _ => return None,
            };
            let title = titles
                .get(&number)
                .cloned()
                .unwrap_or_else(|| event.paper_id.clone());
            let label = match event.payload.get("question").and_then(|q| q.as_str()) {
                Some(question) if kind == "radio_question" => {
                    format!("{title} — Q: {}", question.trim())
                }
                _ => title,
            };
            let secs = event.created_at.trunc() as i64;
            let nanos = ((event.created_at.fract()) * 1e9) as u32;
            Some(crate::ActivityItem {
                kind: kind.to_string(),
                paper_id: titles.contains_key(&number).then_some(number),
                label,
                occurred_at: chrono::DateTime::from_timestamp(secs, nanos)?,
                radio_id: Some(event.paper_id.clone()),
            })
        })
        .collect()
}

/// sai-radio の記録を読む（止まっている・読めないときは空）。
///
/// TUI の同期処理からも呼べるよう、別スレッドで blocking の HTTP を使う
/// （tokio の中で blocking クライアントを直接使うと panic するため）。
fn fetch_radio_events(config: &crate::config::RadioConfig) -> Vec<RadioEvent> {
    let url = format!(
        "{}/api/logs?limit={RADIO_LOG_LIMIT}",
        config.server_url.trim_end_matches('/')
    );
    let token = config
        .token_env
        .as_deref()
        .and_then(|name| std::env::var(name).ok())
        .filter(|token| !token.is_empty());
    std::thread::spawn(move || -> Option<Vec<RadioEvent>> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(RADIO_LOG_TIMEOUT_SEC))
            .build()
            .ok()?;
        let mut req = client.get(url);
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        req.send().ok()?.error_for_status().ok()?.json().ok()
    })
    .join()
    .ok()
    .flatten()
    .unwrap_or_default()
}

/// 出来事の一覧に Radio の出来事（聴いた・質問した）を混ぜ、新しい順に並べ直す。
///
/// sai-radio が止まっているときは何も足さない（sai-paper の出来事だけ出る）。
/// ponytail: 呼ぶたびに sai-radio へ 1 回問い合わせる（待ちは最大 1 秒）。遅く感じたら結果を覚えておく。
pub fn merge_radio_activity(
    items: &mut Vec<crate::ActivityItem>,
    config: &crate::config::RadioConfig,
    conn: &rusqlite::Connection,
) {
    let Some(folder) = crate::paths::project_root()
        .as_deref()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return;
    };
    let events = fetch_radio_events(config);
    if events.is_empty() {
        return;
    }
    let titles: std::collections::HashMap<i64, String> = conn
        .prepare("SELECT id, title FROM papers")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .unwrap_or_default();
    items.extend(radio_events_to_activity(&events, &folder, &titles));
    items.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at));
}

#[cfg(test)]
mod tests {
    use super::{
        RadioEvent, prepare_radio_markdown, radio_events_to_activity, radio_paper_id_in,
        radio_error_message, strip_embedded_images, voice_unreachable_message,
    };
    use std::path::Path;

    /// 音声の名前は `<root のフォルダ名>-<論文番号>`（計画 16 §3.3）。
    #[test]
    fn radio_paper_id_uses_folder_name_and_number() {
        assert_eq!(
            radio_paper_id_in(Some(Path::new("/w/sai-paper")), 30),
            "sai-paper-30"
        );
        // フォルダ名に `-` が入っていても、末尾の `-<数字>` だけが論文番号
        assert_eq!(
            radio_paper_id_in(Some(Path::new("/w/my-proj-2")), 5),
            "my-proj-2-5"
        );
    }

    /// ルートが取れない／フォルダ名が無い場合は番号だけ（従来どおり・プロジェクト不明扱い）。
    #[test]
    fn radio_paper_id_falls_back_to_number() {
        assert_eq!(radio_paper_id_in(None, 30), "30");
        assert_eq!(radio_paper_id_in(Some(Path::new("/")), 7), "7");
    }

    /// Radio の記録はこのプロジェクトの音声だけを拾い、種類と題名を付け替える。
    #[test]
    fn radio_events_pick_this_project_only() {
        let event = |id: &str, kind: &str, payload: serde_json::Value| RadioEvent {
            paper_id: id.to_string(),
            kind: kind.to_string(),
            payload,
            created_at: 1_700_000_000.5,
        };
        let events = vec![
            event("sai-paper-30", "listen", serde_json::json!({})),
            event("sai-paper-31", "question", serde_json::json!({"question": " なぜ? "})),
            event("other-30", "listen", serde_json::json!({})),
            event("4ee6474932af", "listen", serde_json::json!({})),
            event("sai-paper-30", "unknown", serde_json::json!({})),
        ];
        let titles = std::collections::HashMap::from([(30, "Paper A".to_string())]);
        let items = radio_events_to_activity(&events, "sai-paper", &titles);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, "radio_play");
        assert_eq!(items[0].label, "Paper A");
        assert_eq!(items[0].paper_id, Some(30));
        assert_eq!(items[0].radio_id.as_deref(), Some("sai-paper-30"));
        // 題名が無い論文は音声の名前を出し、質問は中身を添える
        assert_eq!(items[1].kind, "radio_question");
        assert_eq!(items[1].label, "sai-paper-31 — Q: なぜ?");
        assert_eq!(items[1].paper_id, None);
    }

    /// sai-radio へ送る JSON に、原稿・音声の設定と音声の置き場所が入る。
    #[test]
    fn from_extract_request_carries_settings_and_audio_dir() {
        let config = crate::config::RadioConfig::default();
        let request = super::from_extract_request(
            &config,
            "proj-7",
            "Title",
            "# md",
            std::path::Path::new("/p/.sai/paper/papers/0007_Title"),
        );
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["paper_id"], "proj-7");
        assert_eq!(json["mode"], "podcast");
        assert_eq!(json["audio_dir"], "/p/.sai/paper/papers/0007_Title");
        assert_eq!(json["settings"]["script"]["outline_model"], "openai/gpt-4o-mini");
        assert_eq!(json["settings"]["script"]["num_segments"], 5);
        assert_eq!(json["settings"]["voice"]["base_url"], "http://127.0.0.1:8600");
        assert_eq!(json["settings"]["voice"]["speakers"].as_array().unwrap().len(), 2);
    }

    /// 文字で埋め込まれた画像だけが消え、ほかの文と普通の画像は残る。
    #[test]
    fn embedded_images_are_stripped_but_text_survives() {
        let markdown = concat!(
            "# Title\n",
            "\n",
            "This is a paragraph about the model.\n",
            "\n",
            "![Figure 1](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB)\n",
            "\n",
            "More text after the figure.\n",
            "\n",
            "![Figure 2](fig1.png)\n",
            "\n",
            "<img src=\"data:image/jpeg;base64,/9j/4AAQSkZJRg==\" width=\"400\">\n",
            "<img src='data:image/webp;base64,UklGRg=='>\n",
            "\n",
            "The last line.\n",
        );
        let stripped = strip_embedded_images(markdown);
        assert!(!stripped.contains("data:image"), "{stripped}");
        // 普通の画像と HTML の画像は残る（data:image ではないから）
        assert!(stripped.contains("![Figure 2](fig1.png)"), "{stripped}");
        // 本文は残る
        assert!(stripped.contains("This is a paragraph about the model."));
        assert!(stripped.contains("More text after the figure."));
        assert!(stripped.contains("The last line."));
        assert!(stripped.contains("# Title"));
    }

    /// 上限以下の本文はそのまま送れる（画像は取り除かれる）。
    #[test]
    fn short_markdown_is_accepted() {
        let markdown = format!("# T\n\ntext ![x](data:image/png;base64,AAAA)\n{}", "a".repeat(100));
        let prepared = prepare_radio_markdown(&markdown).unwrap();
        assert!(!prepared.contains("data:image"));
        assert!(prepared.contains("text"));
    }

    /// 上限を超える本文は、送らずに止める（文字数も文に入れる）。
    #[test]
    fn overlong_markdown_is_stopped_before_sending() {
        // 画像を取り除いても 300,000 文字を超える
        let markdown = format!(
            "{}\n![x](data:image/png;base64,AAAA)\n{}",
            "a".repeat(299_000),
            "b".repeat(2_000)
        );
        let err = prepare_radio_markdown(&markdown).unwrap_err();
        assert!(err.starts_with("本文が長すぎます"), "{err}");
        assert!(err.contains(&format!("上限 {}", super::RADIO_MAX_CHARS)), "{err}");
        // 埋め込み画像だけだと通り、图片の長さでは止まらない
        let only_images = "![](" .to_string() + &format!("data:image/png;base64,{}\n", "A".repeat(400_000)) + ")";
        assert!(prepare_radio_markdown(&only_images).is_ok());
    }

    /// 声のサーバーに届かなかった旨の文には、音声サーバーの URL が出る。
    #[test]
    fn voice_failure_message_names_the_voice_server() {
        let config = crate::config::RadioConfig::default();
        assert_eq!(
            voice_unreachable_message(&config.voice.base_url),
            "音声サーバー（http://127.0.0.1:8600）に接続できません。AivisSpeech Engine を起動してください"
        );
        // sai-radio の中から声のサーバーに届かなかった場合も同じ文にする
        assert_eq!(
            radio_error_message(
                "Failed to generate speech: All connection attempts failed".to_string(),
                &config.voice.base_url
            ),
            voice_unreachable_message(&config.voice.base_url)
        );
        // それ以外のエラーはそのまま出す
        assert_eq!(
            radio_error_message("too many segments".to_string(), &config.voice.base_url),
            "too many segments"
        );
    }
}
