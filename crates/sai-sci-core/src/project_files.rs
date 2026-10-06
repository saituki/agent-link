//! 研究プロジェクトの「考え」と「現況」のファイル。
//!
//! - `mind.md`: ユーザーが自分の考え（大きな問い・仮説・制約など）を書く。
//! - `state.md`: プロジェクトの現況（目標・段階・今の作業・次の一手・更新日）。
//!
//! どちらもユーザーが書くもので、エージェントは直接書き換えない。ひな形は
//! ファイルが無いときだけ [`ensure`] が作り、既にあるファイルには触らない。

use std::fs;
use std::path::{Path, PathBuf};

/// `mind.md` のひな形。
pub const MIND_TEMPLATE: &str = r#"# mind

## 大きな問い

## 今の仮説
<!-- 仮説ごとに名前を付け、sai-link のカードの tag にも同じ名前を付けると、エージェントの記録と対応させやすい -->

## 前提・制約（使えるデータ・計算資源・期間など）

## 興味がある方向 / 興味が無い方向

## 決まったこと（日付・理由）
<!-- 例: - 2026-09-24: 対象は成人の頭皮脳波に絞る（理由: 公開データが多い） -->

## メモ
"#;

/// `state.md` のひな形。
pub const STATE_TEMPLATE: &str = r#"# 現況
<!-- エディタで直接編集する。エージェントは直接書き換えず、report の「引き継ぎ」に更新案を書く -->
<!-- 段階は 探索 / 絞り込み / 精読・議論 / 計画化 / 外部委託 のどれか -->

- 目標:
- 段階: 探索
- 今の作業:
- 次の一手:
- 更新日:
"#;

/// `state.md` から読み取ったプロジェクトの現況。
///
/// 取り出せなかった項目は `None` になる（ファイルが無い・読めない場合も同じ）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ProjectState {
    /// 目標（`- 目標:`）。
    pub goal: Option<String>,
    /// 段階（`- 段階:`）。
    pub stage: Option<String>,
    /// 今の作業（`- 今の作業:`）。
    pub current_task: Option<String>,
    /// 次の一手（`- 次の一手:`）。
    pub next_action: Option<String>,
    /// 更新日（`- 更新日:`）。
    pub updated: Option<String>,
}

/// `mind.md` と `state.md` が無いときだけひな形を書き、書いたファイルのパスを返す。
///
/// 既にあるファイルは 1 バイトも変更しない。`data_dir` が無ければ作る。
///
/// # Errors
///
/// ディレクトリの作成や書き込みに失敗したときに返る。
pub fn ensure(data_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    fs::create_dir_all(data_dir)?;
    let mut written = Vec::new();
    for (path, template) in [
        (sai_paper_core::paths::mind_path(data_dir), MIND_TEMPLATE),
        (sai_paper_core::paths::state_path(data_dir), STATE_TEMPLATE),
    ] {
        if ensure_one(&path, template)? {
            written.push(path);
        }
    }
    Ok(written)
}

/// 1 ファイル版の [`ensure`]。ひな形を書いたら `true`、既にあったら `false`。
///
/// # Errors
///
/// 書き込みに失敗したときに返る。
pub fn ensure_one(path: &Path, template: &str) -> std::io::Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    fs::write(path, template)?;
    Ok(true)
}

/// `mind.md` の「## メモ」の節に 1 行足す（S05）。
///
/// - 「## メモ」の節があれば、その節で最後の空でない行の直後に足す
///   （節の後ろの空行と、次に続く `## ` の見出しはそのまま残す）。
/// - 節が無ければファイルの末尾に `## メモ` の節を作って足す。
/// - `mind.md` が無ければ [`MIND_TEMPLATE`] で作ってから足す。
/// - 書きかけが読まれないよう、同じフォルダの一時ファイルへ書いてから rename する。
///
/// `stamp` は `MM/DD HH:MM` の形。足した行 `- <stamp> <text>` を返す。
///
/// # Errors
///
/// `text` が空（空白だけ）のとき、またはファイルの読み書きに失敗したとき。
pub fn append_memo(data_dir: &Path, text: &str, stamp: &str) -> std::io::Result<String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "空です",
        ));
    }
    fs::create_dir_all(data_dir)?;
    let path = sai_paper_core::paths::mind_path(data_dir);
    // 無ければひな形から作る（ひな形には「## メモ」がある）。
    let content = fs::read_to_string(&path).unwrap_or_else(|_| MIND_TEMPLATE.to_string());

    let line = format!("- {stamp} {text}");
    let updated = insert_memo(&content, &line);
    write_atomic(&path, &updated)?;
    Ok(line)
}

/// `content` の「## メモ」の節の末尾に `line` を挿入した本文を返す。
fn insert_memo(content: &str, line: &str) -> String {
    // 行末の改行を保ったまま行に分ける。
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let heading = lines
        .iter()
        .position(|l| matches!(l.trim_end_matches(['\n', '\r']).trim(), "## メモ" | "## NOTES"));
    let Some(h) = heading else {
        // 節が無ければ末尾に `## メモ` を作る。
        let mut out = content.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str("## メモ\n");
        out.push_str(line);
        out.push('\n');
        return out;
    };

    // 節の終わり = 次に現れる `## ` の見出し（無ければ末尾）。
    let end = lines[h + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with("## "))
        .map_or(lines.len(), |offset| h + 1 + offset);
    // 節の中で最後の空でない行の直後に入れる（無ければ見出しの直後）。
    let mut insert_at = h + 1;
    for (i, l) in lines[h + 1..end].iter().enumerate() {
        if !l.trim().is_empty() {
            insert_at = h + 1 + i + 1;
        }
    }

    let mut out = String::with_capacity(content.len() + line.len() + 2);
    for (i, l) in lines.iter().enumerate() {
        if i == insert_at {
            push_line(&mut out, line);
        }
        out.push_str(l);
    }
    if insert_at >= lines.len() {
        push_line(&mut out, line);
    }
    out
}

/// 直前が改行で終わっていなければ改行を足してから `line` と改行を積む。
fn push_line(out: &mut String, line: &str) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
}

/// 一時ファイルへ書いてから rename で置き換える（途中で壊れないように）。
fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    // `mind.md` → `mind.md.tmp-<pid>`。
    let tmp = path.with_extension(format!("md.tmp-{}", std::process::id()));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// `state.md` を読み、現況の項目を取り出す。
///
/// ファイルが無い・読めないときは全項目 `None` を返す（エラーにはしない）。
#[must_use]
pub fn read_state(data_dir: &Path) -> ProjectState {
    match fs::read_to_string(sai_paper_core::paths::state_path(data_dir)) {
        Ok(text) => parse_state(&text),
        Err(_) => ProjectState::default(),
    }
}

/// `state.md` の本文を読み取り、現況の項目を取り出す（[`read_state`] の本体）。
///
/// 行頭（空白を除く）が `- 目標:` などの項目行だけを見る。コロンは全角 `：` も
/// 受け付ける。値が空なら `None`。同じ項目が 2 回あれば最初の行を採用する。
/// `<!-- ... -->` の中は複数行にわたって無視する。
#[must_use]
pub fn parse_state(text: &str) -> ProjectState {
    let mut state = ProjectState::default();
    // 同じ項目が 2 回以上あっても最初の行だけを採用する（空の値も「最初の行」）。
    let mut seen = [false; 5];
    let mut in_comment = false;
    for raw in text.lines() {
        let line = strip_comments(raw, &mut in_comment);
        let Some(rest) = line.trim_start().strip_prefix('-') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some((idx, sep)) = rest.char_indices().find(|(_, c)| *c == ':' || *c == '：') else {
            continue;
        };
        let key = rest[..idx].trim();
        let value = nonempty(rest[idx + sep.len_utf8()..].trim());
        match key {
            "目標" => fill(&mut state.goal, &mut seen[0], value),
            "段階" => fill(&mut state.stage, &mut seen[1], value),
            "今の作業" => fill(&mut state.current_task, &mut seen[2], value),
            "次の一手" => fill(&mut state.next_action, &mut seen[3], value),
            "更新日" => fill(&mut state.updated, &mut seen[4], value),
            _ => {}
        }
    }
    state
}

/// その項目をまだ見ていなければ、最初の値（空なら `None`）を採用する。
fn fill(slot: &mut Option<String>, seen: &mut bool, value: Option<String>) {
    if !*seen {
        *seen = true;
        *slot = value;
    }
}

/// 空文字なら `None`、そうでなければ `Some`（所有権を持たせる）。
fn nonempty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// 1 行から HTML コメントを除いた文字列を返す。
///
/// `in_comment` は行をまたぐコメントの状態を保持する。コメントの外側だけを残し、
/// 閉じたあとに続く文字列は再びコメントの判定にかける。
fn strip_comments(line: &str, in_comment: &mut bool) -> String {
    let mut out = String::new();
    let mut rest = line;
    loop {
        if *in_comment {
            match rest.find("-->") {
                Some(i) => {
                    rest = &rest[i + 3..];
                    *in_comment = false;
                }
                None => break,
            }
        } else {
            match rest.find("<!--") {
                Some(i) => {
                    out.push_str(&rest[..i]);
                    rest = &rest[i + 4..];
                    *in_comment = true;
                }
                None => {
                    out.push_str(rest);
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の一時 `data_dir`。
    fn temp_data_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-project-files-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ensure_keeps_existing() {
        let dir = temp_data_dir("keep");
        let mind = sai_paper_core::paths::mind_path(&dir);
        let state = sai_paper_core::paths::state_path(&dir);
        fs::write(&mind, "# 自分の考え\n\n消してはだめ\n").unwrap();

        let written = ensure(&dir).unwrap();

        assert_eq!(fs::read_to_string(&mind).unwrap(), "# 自分の考え\n\n消してはだめ\n");
        assert_eq!(written, vec![state.clone()]);
        assert_eq!(fs::read_to_string(&state).unwrap(), STATE_TEMPLATE);

        // 2 回目は何も書かない
        assert!(ensure(&dir).unwrap().is_empty());
    }

    #[test]
    fn ensure_creates_data_dir() {
        let dir = temp_data_dir("create").join("nested").join("paper");
        let written = ensure(&dir).unwrap();

        assert_eq!(written.len(), 2);
        assert_eq!(fs::read_to_string(sai_paper_core::paths::mind_path(&dir)).unwrap(), MIND_TEMPLATE);
        assert_eq!(fs::read_to_string(sai_paper_core::paths::state_path(&dir)).unwrap(), STATE_TEMPLATE);
    }

    #[test]
    fn parse_state_basic() {
        let text = "- 目標: 発作予測\n- 段階:\n余計な行\n- 次の一手：データを比べる";
        let state = parse_state(text);

        assert_eq!(state.goal.as_deref(), Some("発作予測"));
        assert_eq!(state.stage, None);
        assert_eq!(state.next_action.as_deref(), Some("データを比べる"));
        assert_eq!(state.current_task, None);
        assert_eq!(state.updated, None);
    }

    #[test]
    fn parse_state_full_width_and_spaces() {
        let text = "# 現況\n\n- 目標：　研究の問いを 1 文にする　\n- 段階： 絞り込み\n- 今の作業: 論文を 5 本読む\n- 次の一手: データを比べる\n- 更新日: 2026-09-25\n";
        let state = parse_state(text);

        assert_eq!(state.goal.as_deref(), Some("研究の問いを 1 文にする"));
        assert_eq!(state.stage.as_deref(), Some("絞り込み"));
        assert_eq!(state.current_task.as_deref(), Some("論文を 5 本読む"));
        assert_eq!(state.next_action.as_deref(), Some("データを比べる"));
        assert_eq!(state.updated.as_deref(), Some("2026-09-25"));
    }

    #[test]
    fn parse_state_ignores_comments() {
        let text = "# 現況\n<!-- 段階は 探索 / 絞り込み のどれか -->\n<!-- 複数行の\nコメントの\n中身 -->\n- 目標: 本物\n<!-- - 目標: 偽物 -->\n- 段階: 探索 <!-- 補足 -->\n";
        let state = parse_state(text);

        assert_eq!(state.goal.as_deref(), Some("本物"));
        assert_eq!(state.stage.as_deref(), Some("探索"));
    }

    #[test]
    fn parse_state_uses_first_occurrence() {
        let text = "- 目標: 最初\n- 目標: 二番目\n- 更新日:\n- 更新日: 2026-09-25\n";
        let state = parse_state(text);

        assert_eq!(state.goal.as_deref(), Some("最初"));
        assert_eq!(state.updated, None);
    }

    #[test]
    fn read_state_missing_file_is_empty() {
        let dir = temp_data_dir("missing");
        assert_eq!(read_state(&dir), ProjectState::default());
    }

    #[test]
    fn state_template_parses_to_stage_only() {
        let state = parse_state(STATE_TEMPLATE);

        assert_eq!(state.stage.as_deref(), Some("探索"));
        assert_eq!(state.goal, None);
        assert_eq!(state.current_task, None);
        assert_eq!(state.next_action, None);
        assert_eq!(state.updated, None);
    }

    #[test]
    fn append_memo_into_existing_section() {
        let dir = temp_data_dir("memo-section");
        let mind = sai_paper_core::paths::mind_path(&dir);
        fs::write(
            &mind,
            "# mind\n\n## メモ\n- 09/24 10:00 古い\n\n## 次の節\n本文\n",
        )
        .unwrap();

        let line = append_memo(&dir, "新しいメモ", "09/26 21:40").unwrap();

        assert_eq!(line, "- 09/26 21:40 新しいメモ");
        assert_eq!(
            fs::read_to_string(&mind).unwrap(),
            "# mind\n\n## メモ\n- 09/24 10:00 古い\n- 09/26 21:40 新しいメモ\n\n## 次の節\n本文\n",
        );
    }

    #[test]
    fn append_memo_into_notes_section() {
        let dir = temp_data_dir("memo-notes");
        let mind = sai_paper_core::paths::mind_path(&dir);
        fs::write(&mind, "# mind\n\n## NOTES\n- 09/24 10:00 古い\n\n## 次\n本文\n").unwrap();
        append_memo(&dir, "新しいメモ", "09/26 21:40").unwrap();
        assert_eq!(
            fs::read_to_string(&mind).unwrap(),
            "# mind\n\n## NOTES\n- 09/24 10:00 古い\n- 09/26 21:40 新しいメモ\n\n## 次\n本文\n",
        );
    }

    #[test]
    fn append_memo_creates_section_at_end() {
        let dir = temp_data_dir("memo-create-section");
        let mind = sai_paper_core::paths::mind_path(&dir);
        fs::write(&mind, "# mind\n\n## 大きな問い\n").unwrap();

        append_memo(&dir, "浮かんだこと", "09/26 21:40").unwrap();

        assert_eq!(
            fs::read_to_string(&mind).unwrap(),
            "# mind\n\n## 大きな問い\n\n## メモ\n- 09/26 21:40 浮かんだこと\n",
        );
    }

    #[test]
    fn append_memo_creates_mind_from_template() {
        let dir = temp_data_dir("memo-create-file");
        let mind = sai_paper_core::paths::mind_path(&dir);
        assert!(!mind.exists());

        append_memo(&dir, "最初のメモ", "09/26 21:40").unwrap();

        assert_eq!(
            fs::read_to_string(&mind).unwrap(),
            format!("{MIND_TEMPLATE}- 09/26 21:40 最初のメモ\n"),
        );
    }

    #[test]
    fn append_memo_rejects_empty_text() {
        let dir = temp_data_dir("memo-empty");
        assert!(append_memo(&dir, "   \n", "09/26 21:40").is_err());
        // 空のときは mind.md を作らない。
        assert!(!sai_paper_core::paths::mind_path(&dir).exists());
    }
}
