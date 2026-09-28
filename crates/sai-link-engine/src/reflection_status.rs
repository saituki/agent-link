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
}
