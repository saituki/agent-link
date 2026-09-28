//! Kanban カード md（`.sai/todo/*.md`）の読み書き・レーン分類。
//!
//! TUI 非依存の純関数群。`app/kanban.rs`（TUI 側）はこのモジュールに委譲する。
//!
//! ## カード書式（v2・汎用コンテナ）
//!
//! ```md
//! ---
//! creation_date: 2026/09/05 17:50:22
//! execution_date: -
//! run_id: -
//! source_doc: waggle:AAA,waggle:BBB
//! ---
//!
//! # task_text
//! xxx を修正して
//! ```
//!
//! - 入力は本文の `# task_text` セクションただ 1 つ（旧 `inputs:` は廃止）
//! - `source_doc` / `scheduled_at` はメタ情報（入力検査の対象外）
//! - 本文 `# <id>`（レベル1）は `text` 入力。`task_text` だけが実行時の入力になる
//! - 旧カードの `origin_run_id` / `origin_block` は読み飛ばす（review の引き継ぎは廃止）

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// カード置き場（プロジェクトルート基準）。
pub const TODO_DIR: &str = ".sai/todo";

/// 固定 frontmatter キー（v2 汎用コンテナの予約キー）。
///
/// これ以外のキーは `extra`（旧 `file_token` 入力の名残）として `id: 値` で保持する。
/// `description` は Card 表示用（Workflow / Kanban 共通・3行目以降に表示）。
const FIXED_FM_KEYS: [&str; 13] = [
    "creation_date",
    "execution_date",
    "run_id",
    "description",
    "runner",
    "block",
    "thinking",
    "model",
    "provider",
    "round",
    "global_round",
    "tag",
    "inputs",
];

/// メタ情報キー（`block.md` の入力定義とは無関係・入力検査の対象外）。
///
/// - `source_doc`: 元実行の出力トークン列（R112 自動充填・`waggle:` トークン）
/// - `scheduled_at`: todo のカードの実行予定時刻（`YYYY-MM-DD HH:MM`・Mac の時刻）。常駐が見回って実行する
const META_FM_KEYS: [&str; 2] = ["source_doc", "scheduled_at"];

/// 廃止したキー（旧カードに残っていても読み飛ばす・書き出さない）。
const LEGACY_FM_KEYS: [&str; 2] = ["origin_run_id", "origin_block"];

/// `scheduled_at:` の値（`YYYY-MM-DD HH:MM`）を読む。読めなければ `None`。
pub fn parse_scheduled_at(s: &str) -> Option<chrono::NaiveDateTime> {
    chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M").ok()
}

/// カードの frontmatter（`---` ... `---` の中身）。
#[derive(Debug, Clone, PartialEq)]
pub struct CardFrontmatter {
    /// 作成日時（`Shift+C` 作成時）
    pub creation_date: String,
    /// 実行日時（`block run --kanban` 要求時に更新。未実行は `-`）
    pub execution_date: String,
    /// 実行時に割り当てられた execution id（未実行は `-`）
    pub run_id: String,
    /// Card 表示用の説明文（Workflow / Kanban 共通・3行目以降に表示）。
    ///
    /// `Shift+C` 作成時は空。`c` で開いた md の frontmatter `description:` に
    /// 記載した文字がそのまま表示される。欠損した旧カードは空扱い。
    pub description: String,
    /// 実行主体（`pi` / `user`）。未記載は `pi` 扱い。
    ///
    /// - `pi`: `pi agent` が実行すべきタスク（デーモンへ実行要求）
    /// - `user`: ユーザーが実行すべきタスク（DB＋フォルダ移動のみ・特別な処理なし）
    pub runner: String,
    /// 実行ブロック ID（`block:`）。未記載時はエラー（CLI 引数での上書き可）。
    pub block: String,
    /// thinking budget（空欄時は block.md の値を使用）
    pub thinking: String,
    /// モデル（空欄時は block.md の値を使用）
    pub model: String,
    /// pi agent の provider（空欄時は model 解決・block.md の値を使用）
    pub provider: String,
    /// ラウンド番号（block.md にデフォルトなし・task.md 必須）
    pub round: String,
    /// グローバルラウンド番号（block.md にデフォルトなし・task.md 必須）
    pub global_round: String,
    /// タグ（任意）
    pub tag: String,
    /// 固定キー以外の frontmatter（旧 `file_token` 入力・後方互換用。`id → 値`）
    pub extra: HashMap<String, String>,
    /// メタ情報（`source_doc` / `scheduled_at`。入力検査の対象外）
    pub meta: HashMap<String, String>,
}

/// カードの `creation_date` 用の現在時刻文字列（`YYYY/MM/DD HH:MM:SS`）。
///
/// 実装計画 12: カード作成（TUI ウィザード / `sai-link phone card create`）が
/// 共通で使う。widgets 側に chrono 依存を増やさないため engine に置く。
pub fn card_creation_date() -> String {
    chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string()
}

impl CardFrontmatter {
    /// 新規カード用の frontmatter を作る（実行系は `-`・説明は空）。
    pub fn new_card(creation_date: String) -> Self {
        Self {
            creation_date,
            execution_date: "-".to_string(),
            run_id: "-".to_string(),
            description: String::new(),
            runner: "pi".to_string(),
            block: String::new(),
            thinking: String::new(),
            model: String::new(),
            provider: String::new(),
            round: String::new(),
            global_round: String::new(),
            tag: String::new(),
            extra: HashMap::new(),
            meta: HashMap::new(),
        }
    }

    /// 実行主体が `user` か（未記載は `pi` 扱い）。
    pub fn is_user_runner(&self) -> bool {
        self.runner.trim().eq_ignore_ascii_case("user")
    }

    /// メタ情報を設定する（`source_doc` / `scheduled_at`）。
    pub fn set_meta(&mut self, id: String, value: String) {
        self.meta.insert(id, value);
    }

    /// `file_token` 入力値を取得する（拡張キー・メタキー両方）。
    pub fn file_token(&self, id: &str) -> Option<&str> {
        self.extra
            .get(id)
            .or_else(|| self.meta.get(id))
            .map(|s| s.as_str())
    }

    /// `file_token` 入力値を設定する。
    pub fn set_file_token(&mut self, id: String, value: String) {
        self.extra.insert(id, value);
    }

    /// `run_id` が未実行（`-`・空）かどうか。
    pub fn is_unexecuted(&self) -> bool {
        let t = self.run_id.trim();
        t.is_empty() || t == "-"
    }
}

/// カード md 全体（frontmatter + 本文）。
#[derive(Debug, Clone, PartialEq)]
pub struct KanbanCard {
    /// frontmatter
    pub frontmatter: CardFrontmatter,
    /// 本文（frontmatter 以降・自由編集）
    pub body: String,
}

/// カードの所属レーン（表示専用・自動移動のみ）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KanbanLane {
    /// 未実行カード（`run_id: -`）
    Todo,
    /// 対応 execution が `running` / `pending`
    Running,
    /// 対応 execution が `completed`（finalize 後）かつ review が空
    Review,
    /// review に実質文字あり（`r` で書いたもの）
    Done,
}

/// カード md テキストをパースする。
///
/// frontmatter が無い・壊れている場合は `None` を返す。
pub fn parse_card(text: &str) -> Option<KanbanCard> {
    let mut lines = text.lines();
    // 先頭の `---`
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut fm_lines: Vec<&str> = Vec::new();
    let mut body_start: Option<String> = None;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            body_start = Some(lines.collect::<Vec<_>>().join("\n"));
            break;
        }
        fm_lines.push(line);
    }
    let body = body_start?;
    let get = |key: &str| {
        fm_lines
            .iter()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                if k.trim() == key {
                    Some(v.trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_default()
    };
    let frontmatter = CardFrontmatter {
        creation_date: get("creation_date"),
        execution_date: get("execution_date"),
        run_id: get("run_id"),
        // 旧カード（`description:` なし）は空扱い（後方互換）
        description: get("description"),
        runner: {
            let r = get("runner");
            if r.is_empty() {
                "pi".to_string()
            } else {
                r
            }
        },
        block: get("block"),
        thinking: get("thinking"),
        model: get("model"),
        provider: get("provider"),
        round: get("round"),
        global_round: get("global_round"),
        tag: get("tag"),
        extra: {
            // メタキーと廃止したキーは extra から除外する。
            let mut extra = HashMap::new();
            for line in fm_lines.iter() {
                let Some((k, v)) = line.split_once(':') else {
                    continue;
                };
                let k = k.trim();
                if FIXED_FM_KEYS.contains(&k)
                    || META_FM_KEYS.contains(&k)
                    || LEGACY_FM_KEYS.contains(&k)
                    || k.is_empty()
                {
                    continue;
                }
                extra.insert(k.to_string(), v.trim().to_string());
            }
            extra
        },
        meta: {
            let mut meta = HashMap::new();
            for k in META_FM_KEYS {
                let v = get(k);
                if !v.is_empty() {
                    meta.insert(k.to_string(), v);
                }
            }
            meta
        },
    };
    // creation_date が無いものはカードとみなさない
    if frontmatter.creation_date.is_empty() {
        return None;
    }
    Some(KanbanCard { frontmatter, body })
}

/// カードを md テキストに描画する。
///
/// 固定キー＋メタ（`source_doc` / `scheduled_at`）＋
/// `extra`（旧 `file_token` 入力・キー順ソート）を出力する。
pub fn render_card(card: &KanbanCard) -> String {
    let mut extra_keys: Vec<&String> = card.frontmatter.extra.keys().collect();
    extra_keys.sort();
    let mut fm = format!(
        "---\nrunner: {}\ncreation_date: {}\nexecution_date: {}\nrun_id: {}\nblock: {}\nthinking: {}\nmodel: {}\nprovider: {}\nround: {}\nglobal_round: {}\ntag: {}\ndescription: {}",
        card.frontmatter.runner,
        card.frontmatter.creation_date,
        card.frontmatter.execution_date,
        card.frontmatter.run_id,
        card.frontmatter.block,
        card.frontmatter.thinking,
        card.frontmatter.model,
        card.frontmatter.provider,
        card.frontmatter.round,
        card.frontmatter.global_round,
        card.frontmatter.tag,
        card.frontmatter.description,
    );
    for k in META_FM_KEYS {
        if let Some(v) = card.frontmatter.meta.get(k) {
            if !v.is_empty() {
                fm.push_str(&format!("\n{}: {}", k, v));
            }
        }
    }
    for k in extra_keys {
        fm.push_str(&format!("\n{}: {}", k, card.frontmatter.extra[k]));
    }
    format!("{}\n---\n\n{}\n", fm, card.body.trim_matches('\n'))
}

/// 本文セクション（`# <id>`（レベル1）の `type=text` の入力）。
#[derive(Debug, Clone, PartialEq)]
pub enum BodySection {
    /// `type=text` の入力セクション（`id` と本文）
    TextInput { id: String, body: String },
}

/// 本文を行単位でセクション分割する（純関数）。
///
/// `#` レベル1 が `TextInput`。レベル2以下は直前のセクションの本文に含める。
/// コードフェンス（``` / ~~~）の中は見出しとして扱わず、本文として追記する。
/// セクション外の行（先頭のゴミ行等）は無視する。
pub fn split_body_sections(body: &str) -> Vec<BodySection> {
    let mut out: Vec<BodySection> = Vec::new();
    let mut cur: Option<BodySection> = None;
    // コードフェンスの中かどうか（``` / ~~~ の行で反転する）
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        } else if !in_fence && trimmed.chars().take_while(|c| *c == '#').count() == 1 {
            let rest = trimmed.trim_start_matches('#').trim();
            if is_section_id(rest) {
                out.extend(cur.take());
                cur = Some(BodySection::TextInput {
                    id: rest.to_string(),
                    body: String::new(),
                });
                continue;
            }
        }
        // 本文行（フェンス行・レベル2以下・通常行）は直前セクションに追記
        if let Some(BodySection::TextInput { body, .. }) = cur.as_mut() {
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(line);
        }
    }
    out.extend(cur);
    // 末尾空白を整理
    for BodySection::TextInput { body, .. } in out.iter_mut() {
        *body = body.trim().to_string();
    }
    out
}

/// `# <id>` が text 入力の id として妥当か（`[a-z0-9_]+`）。
fn is_section_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// 本文から `type=text` の入力セクション（`id → 本文`）を取り出す。
///
pub fn text_inputs(body: &str) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    for BodySection::TextInput { id, body } in split_body_sections(body) {
        // 同じ id が複数回出たら上書きせずつなぐ（切り詰め防止）
        map.entry(id)
            .and_modify(|b| {
                b.push_str("\n\n");
                b.push_str(&body);
            })
            .or_insert(body);
    }
    map
}

/// task.md から実行時の入力（`task_text` のみ）を取り出す。
///
/// 本文の `# task_text` セクションをそのまま使い（重複 id は連結済み）、
/// `overrides`（`--field task_text=…`）があればそちらを優先する。
pub fn resolve_card_inputs(
    card: &KanbanCard,
    overrides: &HashMap<String, String>,
) -> Result<HashMap<String, String>, String> {
    let text = overrides
        .get("task_text")
        .cloned()
        .or_else(|| text_inputs(&card.body).remove("task_text"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or("task.md 本文に `# task_text` セクションがありません（または空です）")?;
    Ok(HashMap::from([("task_text".to_string(), text)]))
}

/// カードファイル名を生成する（`card-<YYYYMMDD-HHMMSS>-<乱数4桁>.md`）。
pub fn card_file_name(now: &chrono::NaiveDateTime, rand4: u16) -> String {
    format!(
        "card-{}-{:04}.md",
        now.format("%Y%m%d-%H%M%S"),
        rand4 % 10_000
    )
}

/// 未実行カード一覧（`.sai/todo/*.md` のうち `run_id: -`）のパスを返す。
///
/// 実行要求送信済み・未開始のカード（`pending/` に `block_run_request` あり）は
/// 除外する。送信直後に選択肢から消え、二重選択と `todo`/`running` 重複を防ぐ。
pub fn list_todo_cards(todo_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let Ok(entries) = std::fs::read_dir(todo_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(card) = parse_card(&text) else {
            continue;
        };
        if card.frontmatter.is_unexecuted() {
            if is_card_pending(&path) {
                continue;
            }
            out.push(path);
        }
    }
    out.sort();
    out
}

/// カードが実行要求送信済み・未開始か（`pending/` の `block_run_request` 照合）。
///
/// 照合はフルパス完全一致のみ（`outputs/todo/<run_id>/task.md` のように全カードが
/// 同名 `task.md` のため、ファイル名照合だと送信 1 枚で残り全件が消えていた）。
/// 通知ディレクトリの読み失敗時は `false`（選択肢を止めない）。
fn is_card_pending(card_path: &Path) -> bool {
    let pending = crate::project::sai_path(
        format!(
            "{}/{}",
            crate::common::NOTIFY_BASE_REL,
            crate::common::NOTIFY_PENDING
        )
        .as_str(),
    );
    let Ok(entries) = std::fs::read_dir(&pending) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("block_run_request") {
            continue;
        }
        if let Some(rel) = value.get("card_path").and_then(|v| v.as_str()) {
            let abs = crate::project::project_root().join(rel);
            if abs == card_path {
                return true;
            }
        }
    }
    false
}

/// カードの所属先（どちらか一方にしか存在できない）。
///
/// * `Kanban` — todo カード（未実行）/ running / review / done（review ファイルあり）
/// * `Workflow` — `d` で Kanban から送られた実行（review ファイルなし）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardOwner {
    Kanban,
    Workflow,
}

/// 実行の所属先を判定する（純関数・IO なし）。
///
/// * 実行中（`running` / `pending`）→ `Kanban`（running レーン）
/// * 終端（`completed` / `failed`）→ `Kanban`（review / done レーン。
///   review ファイルの有無は問わない。d で送られたものは TUI 側で除外する）
/// * `d` で Kanban から送られた実行 → `Workflow`（TUI 側 `is_kanban_owned` で判定）
///
/// `review_exists` は `.sai/review/<run_id>.md` の有無。
/// 実行開始直後（review 未生成）の running が Workflow に漏れないよう、
/// ステータス判定をファイル有無より優先する。
pub fn owner_of(status: &str, review_exists: bool) -> CardOwner {
    match status {
        // 実行中は review ファイル未生成でも Kanban（running レーン）
        "running" | "pending" => CardOwner::Kanban,
        // 終端は Kanban（review / done レーン）。d で送られたもの（review 削除済み）は
        // TUI 側 `is_kanban_owned` で Workflow へ振り分けるため、ここでは Kanban 扱い。
        "completed" | "failed" => CardOwner::Kanban,
        // 未知のステータスは review ファイルの有無で振り分ける（フォールバック）
        _ => {
            if review_exists {
                CardOwner::Kanban
            } else {
                CardOwner::Workflow
            }
        }
    }
}

/// カードのレーンを分類する（表示専用・自動移動のみ）。
///
/// * `card` — カード内容
/// * `exec_status` — カードの `run_id` に対応する execution の `status`
///  （カード未実行時は `None`）
/// * `review_done` — DB `execution_history.review_done`（run-tui `e` で更新）
pub fn classify_lane(
    card: &KanbanCard,
    exec_status: Option<&str>,
    review_done: bool,
) -> KanbanLane {
    if card.frontmatter.is_unexecuted() {
        return KanbanLane::Todo;
    }
    match exec_status {
        Some("running") | Some("pending") => KanbanLane::Running,
        _ => {
            if review_done {
                KanbanLane::Done
            } else {
                KanbanLane::Review
            }
        }
    }
}

/// カード実行時に frontmatter の `execution_date` / `run_id` を更新する。
pub fn mark_card_executed(card: &mut KanbanCard, execution_date: String, run_id: i64) {
    card.frontmatter.execution_date = execution_date;
    card.frontmatter.run_id = run_id.to_string();
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\ncreation_date: 2026/08/01 10:00:00\nexecution_date: -\nrun_id: -\n---\n\n- xxx の修正を行って\n- zzz の追加機能を実装して．\n";

    const SAMPLE_V2: &str = "---\ncreation_date: 2026/09/05 17:50:22\nexecution_date: -\nrun_id: -\nsource_doc: waggle:AAA,waggle:BBB\n---\n\n# task_text\nxxx を修正して\n\n# memo\nメモ\n";

    #[test]
    fn parse_and_render_roundtrip() {
        let card = parse_card(SAMPLE).expect("パースできる");
        assert_eq!(card.frontmatter.creation_date, "2026/08/01 10:00:00");
        assert_eq!(card.frontmatter.execution_date, "-");
        assert!(card.frontmatter.is_unexecuted());
        assert!(card.body.contains("xxx の修正"));
        let rendered = render_card(&card);
        let reparsed = parse_card(&rendered).expect("再パースできる");
        assert_eq!(card, reparsed);
    }

    #[test]
    fn parse_rejects_non_card() {
        assert!(parse_card("ただのメモ\n").is_none());
        assert!(parse_card("---\nrun_id: -\n---\n\n本文\n").is_none());
    }

    #[test]
    fn card_file_name_format() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 8, 1)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        assert_eq!(card_file_name(&now, 42), "card-20260801-100000-0042.md");
    }

    #[test]
    fn owner_of_splits_kanban_and_workflow() {
        // 実行中は review ファイルの有無にかかわらず Kanban
        assert_eq!(owner_of("running", false), CardOwner::Kanban);
        assert_eq!(owner_of("pending", false), CardOwner::Kanban);
        // 終端（completed/failed）は review ファイルの有無にかかわらず Kanban。
        // d で送られたものは TUI 側 `is_kanban_owned` で Workflow へ振り分ける。
        assert_eq!(owner_of("completed", true), CardOwner::Kanban);
        assert_eq!(owner_of("completed", false), CardOwner::Kanban);
        assert_eq!(owner_of("failed", true), CardOwner::Kanban);
        assert_eq!(owner_of("failed", false), CardOwner::Kanban);
        // 未知ステータスのみ review ファイルの有無で振り分ける
        assert_eq!(owner_of("unknown", true), CardOwner::Kanban);
        assert_eq!(owner_of("unknown", false), CardOwner::Workflow);
    }

    #[test]
    fn classify_lane_four_kinds() {
        let todo = parse_card(SAMPLE).unwrap();
        assert_eq!(classify_lane(&todo, None, false), KanbanLane::Todo);

        let mut running = todo.clone();
        running.frontmatter.run_id = "11".to_string();
        assert_eq!(
            classify_lane(&running, Some("running"), false),
            KanbanLane::Running
        );
        assert_eq!(
            classify_lane(&running, Some("pending"), false),
            KanbanLane::Running
        );
        assert_eq!(
            classify_lane(&running, Some("completed"), false),
            KanbanLane::Review
        );
        assert_eq!(
            classify_lane(&running, Some("completed"), true),
            KanbanLane::Done
        );
    }

    #[test]
    fn mark_card_executed_updates_frontmatter() {
        let mut card = parse_card(SAMPLE).unwrap();
        mark_card_executed(&mut card, "2026/08/02 11:00:00".to_string(), 11);
        assert_eq!(card.frontmatter.execution_date, "2026/08/02 11:00:00");
        assert_eq!(card.frontmatter.run_id, "11");
        assert!(!card.frontmatter.is_unexecuted());
    }

    #[test]
    fn v2_roundtrip_with_extra_and_sections() {
        let card = parse_card(SAMPLE_V2).expect("v2 パースできる");
        assert_eq!(
            card.frontmatter.file_token("source_doc"),
            Some("waggle:AAA,waggle:BBB")
        );
        let inputs = text_inputs(&card.body);
        assert_eq!(
            inputs.get("task_text").map(|s| s.as_str()),
            Some("xxx を修正して")
        );
        let rendered = render_card(&card);
        let reparsed = parse_card(&rendered).expect("再パースできる");
        assert_eq!(card, reparsed);
    }

    /// 旧カードの `origin_run_id` / `origin_block` は読み飛ばし、書き出さない。
    #[test]
    fn legacy_origin_keys_are_dropped() {
        let text = "---\ncreation_date: x\nexecution_date: -\norigin_run_id: 9\norigin_block: 9\nrun_id: -\n---\n\n# task_text\nA\n";
        let card = parse_card(text).unwrap();
        assert!(card.frontmatter.extra.is_empty());
        assert!(card.frontmatter.meta.is_empty());
        assert!(!render_card(&card).contains("origin"));
    }

    #[test]
    fn split_body_sections_ignores_headings_in_code_fence() {
        // コードフェンス内の `# task_text` を本文として扱い、切り詰めない
        let body = "# task_text\nA\n```md\n# task_text\n## Review run_id 1\n```\nB\n";
        let inputs = text_inputs(body);
        let task_text = inputs.get("task_text").expect("task_text がある");
        assert!(task_text.contains('A'), "A を含む: {task_text:?}");
        assert!(task_text.contains("# task_text"), "フェンス内を保持: {task_text:?}");
        assert!(task_text.contains("## Review run_id 1"), "フェンス内を保持: {task_text:?}");
        assert!(task_text.contains('B'), "B を含む: {task_text:?}");
        assert_eq!(split_body_sections(body).len(), 1);
    }

    #[test]
    fn text_inputs_joins_duplicate_ids() {
        // 同じ id が 2 回出ても上書きせずつなぐ
        let inputs = text_inputs("# task_text\nA\n\n# task_text\nB\n");
        assert_eq!(inputs.get("task_text").map(|s| s.as_str()), Some("A\n\nB"));
    }

    #[test]
    fn resolve_card_inputs_returns_only_task_text() {
        // `# task_text` を持つ task.md からは task_text だけが取れる
        let card = parse_card(SAMPLE_V2).expect("v2 パースできる");
        let inputs = resolve_card_inputs(&card, &HashMap::new()).expect("task_text が取れる");
        assert_eq!(inputs.len(), 1, "入力は task_text のみ: {inputs:?}");
        assert_eq!(
            inputs.get("task_text").map(|s| s.as_str()),
            Some("xxx を修正して")
        );
    }

    #[test]
    fn resolve_card_inputs_uses_override() {
        let card = parse_card(SAMPLE_V2).unwrap();
        let overrides = HashMap::from([("task_text".to_string(), "上書き".to_string())]);
        let inputs = resolve_card_inputs(&card, &overrides).expect("上書きが取れる");
        assert_eq!(inputs.get("task_text").map(|s| s.as_str()), Some("上書き"));
    }

    #[test]
    fn resolve_card_inputs_errors_without_task_text() {
        // `# task_text` が無い旧カードはエラー（空文字もエラー）
        let card = parse_card(SAMPLE).unwrap();
        assert!(resolve_card_inputs(&card, &HashMap::new()).is_err());
        let empty = parse_card("---\ncreation_date: 2026/08/01 10:00:00\nrun_id: -\n---\n\n# task_text\n\n")
            .unwrap();
        assert!(resolve_card_inputs(&empty, &HashMap::new()).is_err());
    }

    #[test]
    fn list_todo_cards_filters_executed() {
        let dir = std::env::temp_dir().join(format!("sai_kanban_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), SAMPLE).unwrap();
        let mut executed = parse_card(SAMPLE).unwrap();
        mark_card_executed(&mut executed, "2026/08/02 11:00:00".to_string(), 11);
        std::fs::write(dir.join("b.md"), render_card(&executed)).unwrap();
        std::fs::write(dir.join("note.txt"), "x").unwrap();

        let list = list_todo_cards(&dir);
        assert_eq!(list.len(), 1);
        assert!(list[0].ends_with("a.md"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
