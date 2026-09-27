//! Reflection 記録ファイルの記入状況判定（Notification 機能）。
//!
//! `.sai/reflection/<run_id>.md` の中身を解析し、
//! - **内容の記録**: `## Run Reason` / `## Run Evaluation` / `## Run Next`
//! - **確認メモ**: `## Review`
//!
//! がそれぞれ書かれているかを判定する。判定結果（4 色ルール）は
//! sai-link-tui（左 SB の実行履歴・メイン画面のブロック一覧）の文字色に使用する。
//!
//! ## 4 色ルール
//!
//! | 状況 | 色の意味 |
//! |---|---|
//! | `Empty`（赤） | まだ何も書いていない（書く前のまっさら） |
//! | `ReviewOnly`（緑） | 「Review」だけ書いた（内容の記録はまだ） |
//! | `ContentOnly`（青） | 内容の記録だけ書いた（確認メモはまだ） |
//! | `Complete`（白） | 両方書き終えた |
//!
//! ## 判定ルール
//!
//! - 内容の記録は 3 項目（Run Reason / Run Evaluation / Run Next）のどれか 1 つでも
//!   本文が書かれていれば「書いた」とみなす。見出しだけで中身が空、または
//!   見出し自体が無い場合は「未記入」。
//! - 古いひな形の説明文（`（このブロックを実行した理由・意図を記録）` のような
//!   括弧で囲まれた 1 行のみのコメント）は「空」と同じ扱いにする。
//! - `## Review` はその項目の下に実質的な文字があれば「書いた」とする。
//! - ファイル自体が存在しない場合は「まだ何も書いていない（`Empty`）」として扱う。

use std::collections::HashMap;
use std::path::Path;

/// Reflection 記録ファイルの記入状況（Notification の 4 色ルール）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReflectionStatus {
    /// 未記入（赤）: 内容の記録も確認メモも書かれていない
    Empty,
    /// 確認メモのみ（緑）: Review だけ書いた（内容の記録はまだ）
    ReviewOnly,
    /// 内容のみ（青）: Run Reason / Evaluation / Next のどれかを書いた（Review は未記入）
    ContentOnly,
    /// 記入完了（白）: 内容の記録も確認メモも書き終えた
    Complete,
}

impl ReflectionStatus {
    /// Reflection 記録ファイルのテキストから記入状況を判定する。
    ///
    /// # 引数
    ///
    /// * `text` — ファイルの中身。`None` はファイルが存在しないことを表す
    ///   （「まだ何も書いていない（`Empty`）」として扱う）。
    pub fn judge(text: Option<&str>) -> Self {
        let Some(text) = text else {
            return Self::Empty;
        };

        let sections = parse_sections(text);

        // 内容の記録: 3 項目のどれか 1 つでも本文が書かれていれば「書いた」
        let content_written = sections.iter().any(|(name, body)| {
            (name.eq_ignore_ascii_case("run reason")
                || name.eq_ignore_ascii_case("run evaluation")
                || name.eq_ignore_ascii_case("run next"))
                && has_real_text(body)
        });
        // 確認メモ: Review の下に実質的な文字があれば「書いた」
        let review_written = sections
            .iter()
            .any(|(name, body)| name.eq_ignore_ascii_case("review") && has_real_text(body));

        match (content_written, review_written) {
            (false, false) => Self::Empty,
            (false, true) => Self::ReviewOnly,
            (true, false) => Self::ContentOnly,
            (true, true) => Self::Complete,
        }
    }

    /// Reflection 記録ファイルのパスから記入状況を判定する。
    ///
    /// ファイルが存在しない・読み込みに失敗した場合は [`ReflectionStatus::Empty`]
    /// （「まだ何も書いていない」）を返す。
    pub fn judge_file(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::judge(Some(&text)),
            Err(_) => Self::Empty,
        }
    }
}

// ===========================================================================
// ReviewStatus — 新 review ファイル（`.sai/review/<run_id>.md`）の記入状況
// ===========================================================================

/// 新 review 記録ファイルの記入状況（Kanban の `review` → `done` 判定用）。
///
/// review ファイルの見出しはブロックごとに `review_headings` で変えられる
/// （既定は `追加修正` / `追加実装`、研究用は「納得した点 / 疑問に思った点 /
/// 次に知りたいこと」など）。
/// そのため見出しの名前は問わず、名前が空でないどれかの `##` / `###` セクションに
/// 実質文字があれば `Done` と判定する。フロントマターと見出しより前の部分
/// （キー `""`）は見ない。
/// 旧 [`ReflectionStatus`] の4色判定は Workflow 色分け用に残す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewStatus {
    /// 未記入: 見出しセクションに実質文字が無い（またはファイルなし）
    Empty,
    /// 記入済み: どれかの見出しセクションに実質文字あり
    Done,
}

impl ReviewStatus {
    /// review ファイルのテキストから記入状況を判定する。
    ///
    /// `None`（ファイルなし）は `Empty` として扱う。
    pub fn judge(text: Option<&str>) -> Self {
        let Some(text) = text else {
            return Self::Empty;
        };
        let sections = parse_sections(text);
        // 見出しの名前は問わない。フロントマター・見出し前（キー `""`）は見ない。
        let done = sections
            .iter()
            .any(|(name, body)| !name.is_empty() && has_real_text(body));
        if done {
            Self::Done
        } else {
            Self::Empty
        }
    }

    /// review ファイルのパスから記入状況を判定する。
    pub fn judge_file(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::judge(Some(&text)),
            Err(_) => Self::Empty,
        }
    }
}

/// テキストに `## Review` セクション（見出し）が含まれるかどうか。
///
/// 既存（旧形式）の記録ファイルをエディタで開く前に、Review 項目を
/// 自動追記すべきかどうかの判定に使用する。
pub fn has_review_section(text: &str) -> bool {
    parse_sections(text)
        .keys()
        .any(|name| name.eq_ignore_ascii_case("review"))
}

/// マークダウンを「セクション見出し → 本文行一覧」に分解する。
///
/// - フロントマター（先頭の `---` ... `---`）は無視する
/// - `##` / `###` 見出し（`#` 2〜3 個の行）が親セクションの区切り。`####` 以下の
///   小見出しは親セクションの本文として残す（review 判定用。
///   `### 追加修正` 配下の `####` 記入も親の content とみなす）
/// - 見出しより前（タイトルやメタ情報）は空文字キー `""` に集約する
fn parse_sections(text: &str) -> HashMap<String, Vec<String>> {
    let mut sections: HashMap<String, Vec<String>> = HashMap::new();
    let mut current = String::new();
    let mut in_frontmatter = false;
    let mut frontmatter_done = false;

    for line in text.lines() {
        let trimmed = line.trim();
        // 先頭の YAML フロントマター（--- ... ---）をスキップする
        if !frontmatter_done && trimmed == "---" {
            if in_frontmatter {
                in_frontmatter = false;
                frontmatter_done = true;
            } else {
                in_frontmatter = true;
                current = String::new();
            }
            continue;
        }
        if in_frontmatter {
            continue;
        }
        // `##` / `###` 見出しで親セクションを区切る。`####` 以下の小見出しは
        // 親セクションの本文として残す（`####` 記入も親の content とみなす）。
        // レベル判定は先頭 `#` の個数で行う（`### x` → 3、`#### x` → 4）。
        if trimmed.starts_with("##") {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            if level == 2 || level == 3 {
                let name = trimmed.trim_start_matches('#').trim().to_string();
                current = name;
                sections.entry(current.clone()).or_default();
                continue;
            }
        }
        sections
            .entry(current.clone())
            .or_default()
            .push(line.to_string());
    }

    sections
}

/// 本文行一覧に「実質的な文字」が書かれているかどうか。
///
/// - 空行は無視する
/// - 見出し行（`#` で始まる行）は無視する（`####` 小見出しだけでは記入とみなさない）
/// - 古いひな形の説明文（括弧で囲まれた 1 行のみのコメント、
///   例: `（このブロックを実行した理由・意図を記録）`）は「空」と同じに扱う
fn has_real_text(body: &[String]) -> bool {
    body.iter()
        .map(|l| l.trim())
        .filter(|t| !t.is_empty() && !t.starts_with('#'))
        .any(|t| !is_placeholder(t))
}

/// 行が古いひな形の説明文（括弧で囲まれたコメント 1 行）かどうか。
///
/// 例: `（このブロックを実行した理由・意図を記録）` / `(memo)`
fn is_placeholder(line: &str) -> bool {
    let t = line.trim();
    (t.starts_with('（') && t.ends_with('）')) || (t.starts_with('(') && t.ends_with(')'))
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧形式のひな形（orchestrator が実行終了時に作っていた書式）。
    /// 説明文（括弧コメント）のみ → すべて「未記入」扱い。
    const OLD_TEMPLATE: &str = "# Reflection\n\n対象実行: ID10 / plan-ice\n開始: 2026-09-02 10:00:00\n\n## Run Reason\n（このブロックを実行した理由・意図を記録）\n\n## Run Evaluation\n（実行結果の評価を記録）\n\n## Run Next\n（次のアクションを記録）\n";

    /// 新形式のひな形（orchestrator 統一書式）。見出しのみで本文なし。
    const NEW_TEMPLATE: &str = "---\nblock: plan-ice\ndate: 2026-09-02 10:00:00\nrun_id: 10\n---\n\n## Run Reason\n\n## Run Evaluation\n\n## Run Next\n\n## Review\n";

    #[test]
    fn missing_file_is_empty() {
        assert_eq!(ReflectionStatus::judge(None), ReflectionStatus::Empty);
        assert_eq!(ReflectionStatus::judge(Some("")), ReflectionStatus::Empty);
    }

    #[test]
    fn old_template_is_empty() {
        // 古いひな形（説明文のみ）は「空」と同じ扱い → 赤
        assert_eq!(
            ReflectionStatus::judge(Some(OLD_TEMPLATE)),
            ReflectionStatus::Empty
        );
    }

    #[test]
    fn new_template_is_empty() {
        assert_eq!(
            ReflectionStatus::judge(Some(NEW_TEMPLATE)),
            ReflectionStatus::Empty
        );
    }

    #[test]
    fn content_only_is_content_only() {
        let md = "---\nblock: a\n---\n\n## Run Reason\n\n理由を書いた。\n\n## Run Evaluation\n\n\n## Run Next\n\n\n## Review\n";
        assert_eq!(
            ReflectionStatus::judge(Some(md)),
            ReflectionStatus::ContentOnly
        );
    }

    #[test]
    fn review_only_is_review_only() {
        let md = NEW_TEMPLATE.replace("## Review\n", "## Review\n\n確認済み。\n");
        assert_eq!(
            ReflectionStatus::judge(Some(&md)),
            ReflectionStatus::ReviewOnly
        );
    }

    #[test]
    fn complete_when_both_written() {
        let md = "---\nblock: a\n---\n\n## Run Reason\n\n理由\n\n## Run Evaluation\n\n評価\n\n## Run Next\n\n次\n\n## Review\n\n確認した\n";
        assert_eq!(
            ReflectionStatus::judge(Some(md)),
            ReflectionStatus::Complete
        );
    }

    #[test]
    fn missing_section_heading_is_not_written() {
        // 見出し自体が無い場合は「未記入」
        let md = "---\nblock: a\n---\n\nなにもなし\n";
        assert_eq!(ReflectionStatus::judge(Some(md)), ReflectionStatus::Empty);
    }

    #[test]
    fn list_item_counts_as_written() {
        let md = "## Run Next\n\n- 次はテストを書く\n";
        assert_eq!(
            ReflectionStatus::judge(Some(md)),
            ReflectionStatus::ContentOnly
        );
    }

    #[test]
    fn review_section_detection() {
        assert!(
            !has_review_section(OLD_TEMPLATE),
            "旧形式には Review が無い"
        );
        assert!(has_review_section(NEW_TEMPLATE), "新形式には Review がある");
    }

    #[test]
    fn old_template_review_append_makes_review_only() {
        // 旧形式ファイルの末尾に Review を足しただけの状態
        let md = format!("{}\n## Review\n", OLD_TEMPLATE);
        assert_eq!(ReflectionStatus::judge(Some(&md)), ReflectionStatus::Empty);
    }

    // ------------------------------------------------------------------
    // ReviewStatus（新 review ファイル `.sai/review/<run_id>.md` の2項目判定）
    // ------------------------------------------------------------------

    /// 新 review テンプレート（2項目のみ・どちらも空）。
    const REVIEW_TEMPLATE: &str = "---\nblock: plan-ice\ndate: 2026-09-02 10:00:00\nrun_id: 10\n---\n\n### 追加修正\n\n\n### 追加実装\n\n";

    #[test]
    fn review_status_missing_is_empty() {
        assert_eq!(ReviewStatus::judge(None), ReviewStatus::Empty);
        assert_eq!(
            ReviewStatus::judge(Some(REVIEW_TEMPLATE)),
            ReviewStatus::Empty
        );
    }

    #[test]
    fn review_status_either_section_counts_as_done() {
        let md =
            REVIEW_TEMPLATE.replacen("### 追加修正\n", "### 追加修正\n\nxxx の修正を行って\n", 1);
        assert_eq!(ReviewStatus::judge(Some(&md)), ReviewStatus::Done);
        let md2 = REVIEW_TEMPLATE.replacen(
            "### 追加実装\n",
            "### 追加実装\n\nzzz の追加機能を実装して\n",
            1,
        );
        assert_eq!(ReviewStatus::judge(Some(&md2)), ReviewStatus::Done);
    }

    #[test]
    fn review_status_subheading_content_counts_as_done() {
        // `###` 直下に本文が無くても `####` 配下に文章があれば記入ありとみなす。
        // ID38 の実ファイル形式（`####` 配下に記入・`###` 直下は空）を再現する。
        let md = "---\nblock: sai\ndate: 2026-09-07 22:08:01\nrun_id: 38\n---\n\n### 追加修正\n#### shortcut `p`動作不良\n\n本文がある\n\n### 追加実装\n";
        assert_eq!(ReviewStatus::judge(Some(&md)), ReviewStatus::Done);
        // 小見出しだけで本文が無い場合は未記入のまま。
        let md2 = "---\nblock: sai\n---\n\n### 追加修正\n#### 見出しだけ\n\n### 追加実装\n";
        assert_eq!(ReviewStatus::judge(Some(md2)), ReviewStatus::Empty);
    }

    #[test]
    fn review_status_old_names_count_as_done() {
        // 旧見出し（`## 追加修正項目` / `## 追加機能項目`）の既存ファイルも Done
        let old = "---\nblock: a\n---\n\n## 追加修正項目\n\n直す\n\n## 追加機能項目\n";
        assert_eq!(ReviewStatus::judge(Some(old)), ReviewStatus::Done);
        let old2 = "---\nblock: a\n---\n\n## 追加修正項目\n\n\n## 追加機能項目\n\n足す\n";
        assert_eq!(ReviewStatus::judge(Some(old2)), ReviewStatus::Done);
    }

    /// 研究用の見出し（`review_headings` で変えたもの）のひな形。
    const RESEARCH_TEMPLATE: &str = "---\nblock: 論文検索\ndate: 2026-09-25 10:00:00\nrun_id: 5\n---\n\n### 納得した点\n\n\n### 疑問に思った点\n\n\n### 次に知りたいこと\n\n";

    #[test]
    fn review_status_research_template_is_empty() {
        assert_eq!(
            ReviewStatus::judge(Some(RESEARCH_TEMPLATE)),
            ReviewStatus::Empty
        );
    }

    #[test]
    fn review_status_research_headings_count_as_done() {
        // 見出しの名前は問わず、どれかに記入があれば Done
        let md = RESEARCH_TEMPLATE.replacen(
            "### 疑問に思った点\n",
            "### 疑問に思った点\n\nこの点が分からなかった\n",
            1,
        );
        assert_eq!(ReviewStatus::judge(Some(&md)), ReviewStatus::Done);
    }

    #[test]
    fn review_status_frontmatter_only_content_is_empty() {
        // フロントマターと見出しより前の文字は記入とみなさない
        let md = "---\nblock: plan-ice\nnote: メモを書いた\n---\n\n前置きの文章\n\n### 納得した点\n\n\n### 疑問に思った点\n\n\n### 次に知りたいこと\n";
        assert_eq!(ReviewStatus::judge(Some(md)), ReviewStatus::Empty);
    }
}
