//! ブロック定義ファイルパーサー
//!
//! Markdown + YAML frontmatter 形式のブロック定義ファイルを読み込み、
//! `BlockConfig` 構造体にデシリアライズします。
//!
//! `templates` フィールドが指定されている場合、`PROJECTROOT/.sai/template/*.md`
//! の内容が指定順に本文の先頭へ挿入されます（`parse_block_file` のみ）。
//!
//! # 使用例
//!
//! ```no_run
//! use sai_link_engine::{parse_block_file, ParsedBlock};
//!
//! let parsed = parse_block_file("path/to/block.md").expect("パースに失敗しました");
//! println!("ブロック名: {}", parsed.config.name);
//! println!("本文: {}", parsed.body);
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use crate::block_config::BlockConfig;
use crate::error::ParseError;
use crate::orchestrator::resolve_block_path;
use crate::validation::validate;

/// パースされたブロック定義の結果を格納する構造体。
#[derive(Debug, Clone)]
pub struct ParsedBlock {
    /// デシリアライズされたブロック設定
    pub config: BlockConfig,
    /// ブロックプロンプト本文（Markdown）
    pub body: String,
    /// 生のフロントマター文字列（デバッグ用）
    pub raw_frontmatter: String,
    /// バリデーション時の警告メッセージ（ブロック定義は有効だが注意すべき内容）
    pub warnings: Vec<String>,
}

/// 指定されたファイルパスからブロック定義ファイルを読み込み、パースする。
///
/// ファイルを読み込み、YAML frontmatter を抽出して `BlockConfig` にデシリアライズし、
/// バリデーションを実行した結果を `ParsedBlock` として返します。
///
/// # 引数
///
/// * `path` - ブロック定義ファイル（`.md`）のパス
///
/// # エラー
///
/// - ファイルが存在しない、または読み取り不可: `ParseError::IoError`
/// - frontmatter が見つからない: `ParseError::MissingFrontmatter`
/// - YAML パースエラー: `ParseError::YamlError`
/// - バリデーションエラー: 各 `ParseError` バリアント
pub fn parse_block_file(path: impl AsRef<Path>) -> Result<ParsedBlock, ParseError> {
    // テンプレートファイルは `PROJECTROOT/.sai/template/` から読み込む
    let template_dir = crate::project::sai_path(".sai/template");
    parse_block_file_with_template_dir(path, &template_dir)
}

/// `parse_block_file` の本体。テンプレートディレクトリを明示指定できる版。
///
/// テスト（テンプレートディレクトリを一時ディレクトリに差し替えたい場合）や
/// `PROJECTROOT/.sai/template` 以外の場所から読み込みたい場合に使用する。
fn parse_block_file_with_template_dir(
    path: impl AsRef<Path>,
    template_dir: &Path,
) -> Result<ParsedBlock, ParseError> {
    let content = fs::read_to_string(path.as_ref())?;
    let mut parsed = parse_block_string(&content)?;

    // id とブロック定義ファイル名（<id>.md）の一致チェック（警告のみ）
    // CLI・task_create の block_type は id で解決されるため、
    // ファイル名と id が異なるとブロックを参照できなくなる。
    if let Some(id) = parsed.config.id.as_deref() {
        let stem = path
            .as_ref()
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if stem != id {
            let msg = format!(
                "ブロック '{}': frontmatter の id '{}' がファイル名 '{}' と一致しません。\
                 `sai-link def list` は id で解決するため、ファイル名を '{}.md' に揃えてください。",
                parsed.config.display_name(),
                id,
                path.as_ref().display(),
                id
            );
            parsed.warnings.push(msg);
        }
    }

    // templates フィールドに指定されたテンプレートファイルを読み込み、
    // 本文（フロントマターとコンテンツの間）の先頭に挿入する。
    parsed.body = apply_templates(&parsed.config, &parsed.body, template_dir)?;

    Ok(parsed)
}

/// 文字列からブロック定義をパースする。
///
/// テスト用や、既にメモリ上に読み込まれたコンテンツを
/// パースする場合に使用します。
///
/// # 引数
///
/// * `content` - ブロック定義ファイルの内容（Markdown + YAML frontmatter）
pub fn parse_block_string(content: &str) -> Result<ParsedBlock, ParseError> {
    let (frontmatter_str, body) = extract_frontmatter(content)?;
    let raw_frontmatter = frontmatter_str.to_string();

    // YAML frontmatter を BlockConfig にデシリアライズ
    let config: BlockConfig = serde_yaml::from_str(frontmatter_str)?;

    // バリデーションを実行
    let validation = validate(&config);

    // エラーがある場合は最初のエラーを返す
    if let Some(err) = validation.errors.into_iter().next() {
        return Err(err);
    }

    Ok(ParsedBlock {
        config,
        body: body.to_string(),
        raw_frontmatter,
        warnings: validation.warnings,
    })
}

/// ブロック定義の `templates` フィールドに指定されたテンプレートファイルを
/// `template_dir`（通常は `PROJECTROOT/.sai/template`）から読み込み、
/// 本文の先頭に指定順で挿入する。
///
/// 挿入結果は以下のようになる（フロントマターとコンテンツの間に展開）:
///
/// ```markdown
/// ---
/// name: xxx
/// templates: tmp1,tmp2
/// ---
///
/// <- tmp1.md の内容
///
/// <- tmp2.md の内容
///
/// # content
/// ```
///
/// `templates` が未指定・空の場合は本文をそのまま返す。
/// テンプレートファイルが存在しない場合は `ParseError::TemplateError` を返す。
///
/// # 引数
///
/// * `config` - ブロック設定（`templates` フィールドを参照）
/// * `body` - ブロック本文（Markdown）
/// * `template_dir` - テンプレートファイルを格納するディレクトリ
pub fn apply_templates(
    config: &BlockConfig,
    body: &str,
    template_dir: &Path,
) -> Result<String, ParseError> {
    // テンプレート探索ディレクトリ（優先度順）:
    // 1. 指定されたディレクトリ（通常は `PROJECTROOT/.sai/template`）
    // 2. グローバル `~/.sai/template`（全プロジェクトで使用可能）
    let mut template_dirs: Vec<PathBuf> = vec![template_dir.to_path_buf()];
    if let Some(global) = crate::project::sai_home_path("template") {
        if global != template_dir.to_path_buf() {
            template_dirs.push(global);
        }
    }
    let mut parts: Vec<String> = Vec::new();

    // ── Step 1: subblocks の情報を挿入 ──
    if let Some(ref subblocks) = config.subblocks {
        if !subblocks.is_empty() {
            let mut subblock_infos: Vec<String> = Vec::new();
            for name in subblocks {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let info = format_subblock_info(trimmed);
                subblock_infos.push(info);
            }
            if !subblock_infos.is_empty() {
                parts.push(subblock_infos.join("\n\n"));
            }
        }
    }

    // ── Step 2: テンプレートを読み込み ──
    if let Some(ref templates) = config.templates {
        if !templates.is_empty() {
            for name in templates {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let filename = if trimmed.ends_with(".md") {
                    trimmed.to_string()
                } else {
                    format!("{}.md", trimmed)
                };

                // 探索ディレクトリ順（ローカル優先）にファイルを探す。
                // どのディレクトリにも無い場合はエラー（参照パス一覧を表示）。
                let mut content: Option<String> = None;
                let mut tried: Vec<String> = Vec::new();
                for dir in &template_dirs {
                    let file_path = dir.join(&filename);
                    tried.push(file_path.display().to_string());
                    match fs::read_to_string(&file_path) {
                        Ok(c) => {
                            content = Some(c);
                            break;
                        }
                        Err(_) => continue,
                    }
                }
                let Some(content) = content else {
                    return Err(ParseError::TemplateError {
                        name: filename.clone(),
                        message: format!("参照パス: {}", tried.join(", ")),
                    });
                };
                parts.push(content.trim_end().to_string());
            }
        }
    }

    if parts.is_empty() {
        return Ok(body.to_string());
    }

    // 全パーツを空行区切りで連結し、本文の先頭へ挿入する
    let mut merged = parts.join("\n\n");
    if !body.trim().is_empty() {
        merged.push_str("\n\n");
        merged.push_str(body);
    }
    Ok(merged)
}

/// サブブロックの定義情報を `block_inspect` 相当のテキストに整形する。
fn format_subblock_info(block_id: &str) -> String {
    let path = resolve_block_path(block_id);
    if !path.exists() {
        return format!(
            "<!-- block_inspect: {} = ブロック定義が見つかりません: {} -->",
            block_id,
            path.display()
        );
    }

    let parsed = match parse_block_file(&path) {
        Ok(p) => p,
        Err(e) => {
            return format!("<!-- block_inspect: {} = パースエラー: {} -->", block_id, e);
        }
    };

    let cfg = &parsed.config;
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("<block_inspect id=\"{}\">", block_id));

    if let Some(ref desc) = cfg.description {
        lines.push(format!("説明: {}", desc));
    }

    if let Some(ref sub) = cfg.subblocks {
        if !sub.is_empty() {
            lines.push(format!("呼び出し可能サブブロック: {}", sub.join(", ")));
        }
    }

    lines.push(format!("</block_inspect>"));
    lines.join("\n")
}

/// コンテンツ文字列から YAML frontmatter 部と Markdown ボディ部を分割する。
///
/// ファイル先頭の `---\n` から次の `\n---` までを frontmatter として抽出し、
/// それ以降をボディ（Markdown 本文）として返します。
///
/// # 戻り値
///
/// `Ok((frontmatter_str, body_str))` — frontmatter 文字列（YAML）とボディ文字列（Markdown）
///
/// # フォーマット
///
/// ```markdown
/// ---
/// name: my-block
/// model: anthropic/claude-sonnet-4-5
/// ---
/// # Markdown 本文
/// ```
fn extract_frontmatter(content: &str) -> Result<(&str, &str), ParseError> {
    // 先頭の空白をトリムしてからチェック
    let trimmed = content.trim_start();

    // 先頭が `---` で始まらない場合は frontmatter なし
    if !trimmed.starts_with("---") {
        return Err(ParseError::MissingFrontmatter);
    }

    // 開始 `---` の後の文字列
    let after_opening = &trimmed[3..];

    // `---` の直後に改行または EOF が必要
    // `---something` のような形式は frontmatter と見なさない
    if !after_opening.is_empty()
        && !after_opening.starts_with('\n')
        && !after_opening.starts_with('\r')
    {
        return Err(ParseError::MissingFrontmatter);
    }

    // 閉じ `\n---` を検索（行頭の `---` のみを認識）
    // find は部分文字列の開始位置を返す
    if let Some(marker_start) = after_opening.find("\n---") {
        // marker_start は `\n` の位置
        // frontmatter は `after_opening[1..marker_start]`
        // （先頭の改行をスキップし、閉じ `\n---` の `\n` の手前まで）
        let frontmatter = &after_opening[1..marker_start];

        // body は閉じ `---` の後（`\n` + `---` = 4 文字スキップ）
        let body_start = marker_start + 4;
        let body = &after_opening[body_start..];

        Ok((frontmatter.trim_end(), body.trim_start()))
    } else {
        // 閉じ `---` が見つからない
        Err(ParseError::UnclosedFrontmatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_config::ToolsConfig;

    // ---- extract_frontmatter のテスト ----

    #[test]
    fn test_extract_frontmatter_basic() {
        let content = "---\nname: test\nmodel: test-model\n---\n# Body";
        let (fm, body) = extract_frontmatter(content).unwrap();
        assert_eq!(fm, "name: test\nmodel: test-model");
        assert_eq!(body, "# Body");
    }

    #[test]
    fn test_extract_frontmatter_no_body() {
        let content = "---\nname: test\n---";
        let (fm, body) = extract_frontmatter(content).unwrap();
        assert_eq!(fm, "name: test");
        assert_eq!(body, "");
    }

    #[test]
    fn test_extract_frontmatter_leading_whitespace() {
        let content = "  \n---\nname: test\n---\n# Body";
        let (fm, body) = extract_frontmatter(content).unwrap();
        assert_eq!(fm, "name: test");
        assert_eq!(body, "# Body");
    }

    #[test]
    fn test_extract_frontmatter_no_frontmatter() {
        let content = "# Just a markdown file";
        let result = extract_frontmatter(content);
        assert!(matches!(result, Err(ParseError::MissingFrontmatter)));
    }

    #[test]
    fn test_extract_frontmatter_unclosed() {
        let content = "---\nname: test\n";
        let result = extract_frontmatter(content);
        assert!(matches!(result, Err(ParseError::UnclosedFrontmatter)));
    }

    #[test]
    fn test_extract_frontmatter_not_at_start() {
        let content = "some text\n---\nname: test\n---\n# Body";
        let result = extract_frontmatter(content);
        assert!(matches!(result, Err(ParseError::MissingFrontmatter)));
    }

    // ---- parse_block_string のテスト ----

    #[test]
    fn test_parse_block_string_minimal() {
        let content = "---\nname: test-block\ndescription: テスト用ブロック\nmodel: anthropic/claude-sonnet-4-5\n---\n# テスト本文";
        let parsed = parse_block_string(content).unwrap();
        assert_eq!(parsed.config.name, "test-block");
        assert_eq!(
            parsed.config.description.as_deref(),
            Some("テスト用ブロック")
        );
        assert_eq!(
            parsed.config.model.as_deref(),
            Some("anthropic/claude-sonnet-4-5")
        );
        assert_eq!(parsed.body, "# テスト本文");
    }

    #[test]
    fn test_parse_block_string_with_body() {
        let content =
            "---\nname: my-block\ndescription: desc\nmodel: m\n---\n# 見出し\n\n本文\n```no_run\nlet x = 1;\n```\n{{input.test}}";
        let parsed = parse_block_string(content).unwrap();
        assert_eq!(
            parsed.body,
            "# 見出し\n\n本文\n```no_run\nlet x = 1;\n```\n{{input.test}}"
        );
    }

    #[test]
    fn test_parse_block_string_no_frontmatter() {
        let content = "# Just body text";
        let result = parse_block_string(content);
        assert!(matches!(result, Err(ParseError::MissingFrontmatter)));
    }

    #[test]
    fn test_parse_block_string_invalid_yaml() {
        let content = "---\nname: test\ninvalid_yaml: [unclosed\n---\n# Body";
        let result = parse_block_string(content);
        assert!(matches!(result, Err(ParseError::YamlError(_))));
    }

    #[test]
    fn test_parse_block_string_tools_config_bool_true() {
        let content = "---\nname: test\ndescription: desc\nmodel: m\ntools: true\n---";
        let parsed = parse_block_string(content).unwrap();
        assert_eq!(parsed.config.tools, Some(ToolsConfig::All));
    }

    #[test]
    fn test_parse_block_string_tools_config_string() {
        let content = "---\nname: test\ndescription: desc\nmodel: m\ntools: read,bash,edit\n---";
        let parsed = parse_block_string(content).unwrap();
        assert_eq!(
            parsed.config.tools,
            Some(ToolsConfig::List(vec![
                "read".to_string(),
                "bash".to_string(),
                "edit".to_string()
            ]))
        );
    }

    #[test]
    fn test_body_preserves_markdown() {
        let content =
            "---\nname: test\ndescription: desc\nmodel: m\n---\n# Heading\n\nSome **bold** text\n\n```\ncode block\n```\n\n{{input.placeholder}}";
        let parsed = parse_block_string(content).unwrap();
        assert!(parsed.body.contains("# Heading"));
        assert!(parsed.body.contains("**bold**"));
        assert!(parsed.body.contains("```"));
        assert!(parsed.body.contains("{{input.placeholder}}"));
    }

    // ---- templates フィールドのデシリアライズ ----

    #[test]
    fn test_parse_templates_comma_string() {
        let content =
            "---\nname: test\ndescription: desc\nmodel: m\ntemplates: tmp1,tmp2\n---\n# Body";
        let parsed = parse_block_string(content).unwrap();
        assert_eq!(
            parsed.config.templates,
            Some(vec!["tmp1".to_string(), "tmp2".to_string()])
        );
    }

    #[test]
    fn test_parse_templates_yaml_list() {
        let content =
            "---\nname: test\ndescription: desc\nmodel: m\ntemplates:\n  - tmp1\n  - tmp2\n---\n# Body";
        let parsed = parse_block_string(content).unwrap();
        assert_eq!(
            parsed.config.templates,
            Some(vec!["tmp1".to_string(), "tmp2".to_string()])
        );
    }

    #[test]
    fn test_parse_templates_absent() {
        let content = "---\nname: test\ndescription: desc\nmodel: m\n---\n# Body";
        let parsed = parse_block_string(content).unwrap();
        assert_eq!(parsed.config.templates, None);
    }

    #[test]
    fn test_parse_templates_empty_string() {
        let content = "---\nname: test\ndescription: desc\nmodel: m\ntemplates: \n---\n# Body";
        let parsed = parse_block_string(content).unwrap();
        assert_eq!(parsed.config.templates, None);
    }

    #[test]
    fn test_parse_templates_invalid_type() {
        let content = "---\nname: test\ndescription: desc\nmodel: m\ntemplates: 123\n---\n# Body";
        let result = parse_block_string(content);
        assert!(matches!(result, Err(ParseError::YamlError(_))));
    }

    // ---- apply_templates ----

    /// テンプレートファイルを格納した一時ディレクトリを作成するヘルパー。
    fn make_template_dir() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        (dir, path)
    }

    #[test]
    fn test_apply_templates_inserts_in_order() {
        let (_keep, template_dir) = make_template_dir();
        std::fs::write(
            template_dir.join("tmp1.md"),
            "## テンプレート1の内容\n\n- 項目A",
        )
        .unwrap();
        std::fs::write(
            template_dir.join("tmp2.md"),
            "## テンプレート2の内容\n\n- 項目B",
        )
        .unwrap();

        let config = BlockConfig {
            name: "test".to_string(),
            description: None,
            model: None,
            provider: None,
            thinking: None,
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
            timeout: None,
            worktree: None,
            enable: None,
            category: None,
            tag: None,
            round: None,
            layer: None,
            templates: Some(vec!["tmp1".to_string(), "tmp2".to_string()]),
            context: None,
            id: None,
        };

        let body = apply_templates(&config, "# content", &template_dir).unwrap();
        // 指定順にテンプレートが挿入され、元の本文は末尾に残る
        assert_eq!(
            body,
            "## テンプレート1の内容\n\n- 項目A\n\n## テンプレート2の内容\n\n- 項目B\n\n# content"
        );
        // テンプレートの位置（先頭）と本文（末尾）の順序を確認
        assert!(body.starts_with("## テンプレート1の内容"));
        assert!(body.ends_with("# content"));
        let pos1 = body.find("テンプレート1").unwrap();
        let pos2 = body.find("テンプレート2").unwrap();
        let pos_content = body.find("# content").unwrap();
        assert!(
            pos1 < pos2 && pos2 < pos_content,
            "テンプレートは指定順に本文の前へ"
        );
    }

    #[test]
    fn test_apply_templates_appends_md_extension() {
        let (_keep, template_dir) = make_template_dir();
        // 拡張子なしの指定でも `.md` が補完される
        std::fs::write(template_dir.join("guide.md"), "# ガイド").unwrap();

        let config = BlockConfig {
            name: "test".to_string(),
            description: None,
            model: None,
            provider: None,
            thinking: None,
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
            timeout: None,
            worktree: None,
            enable: None,
            category: None,
            tag: None,
            round: None,
            layer: None,
            templates: Some(vec!["guide".to_string()]),
            context: None,
            id: None,
        };

        let body = apply_templates(&config, "# content", &template_dir).unwrap();
        assert!(body.starts_with("# ガイド"));
    }

    #[test]
    fn test_apply_templates_missing_file() {
        let (_keep, template_dir) = make_template_dir();
        let config = BlockConfig {
            name: "test".to_string(),
            description: None,
            model: None,
            provider: None,
            thinking: None,
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
            timeout: None,
            worktree: None,
            enable: None,
            category: None,
            tag: None,
            round: None,
            layer: None,
            templates: Some(vec!["not_exist".to_string()]),
            context: None,
            id: None,
        };

        let result = apply_templates(&config, "# content", &template_dir);
        assert!(matches!(
            result,
            Err(ParseError::TemplateError { name, .. }) if name == "not_exist.md"
        ));
    }

    #[test]
    fn test_apply_templates_none_and_empty() {
        let (_keep, template_dir) = make_template_dir();

        // templates 未指定 → 本文そのまま
        let config = BlockConfig {
            name: "test".to_string(),
            description: None,
            model: None,
            provider: None,
            thinking: None,
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
            timeout: None,
            worktree: None,
            enable: None,
            category: None,
            tag: None,
            round: None,
            layer: None,
            templates: None,
            context: None,
            id: None,
        };
        assert_eq!(
            apply_templates(&config, "# content", &template_dir).unwrap(),
            "# content"
        );

        // 空リスト → 本文そのまま
        let mut config = config.clone();
        config.templates = Some(vec![]);
        assert_eq!(
            apply_templates(&config, "# content", &template_dir).unwrap(),
            "# content"
        );
    }

    #[test]
    fn test_apply_templates_empty_body() {
        let (_keep, template_dir) = make_template_dir();
        std::fs::write(template_dir.join("tmp1.md"), "# テンプレート").unwrap();

        let config = BlockConfig {
            name: "test".to_string(),
            description: None,
            model: None,
            provider: None,
            thinking: None,
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
            timeout: None,
            worktree: None,
            enable: None,
            category: None,
            tag: None,
            round: None,
            layer: None,
            templates: Some(vec!["tmp1".to_string()]),
            context: None,
            id: None,
        };

        // 本文が空でもテンプレートのみ返る
        assert_eq!(
            apply_templates(&config, "", &template_dir).unwrap(),
            "# テンプレート"
        );
    }

    // ---- parse_block_file_with_template_dir（パイプ接続のテスト） ----

    #[test]
    fn test_parse_block_file_with_templates() {
        let (_keep, template_dir) = make_template_dir();
        std::fs::write(template_dir.join("tmp1.md"), "# テンプレート1").unwrap();
        std::fs::write(template_dir.join("tmp2.md"), "# テンプレート2").unwrap();

        // ブロック定義ファイルを一時ファイルとして作成
        let block_dir = tempfile::tempdir().unwrap();
        let block_path = block_dir.path().join("test_block.md");
        std::fs::write(
            &block_path,
            "---\nname: test-block\ndescription: desc\nmodel: m\ntemplates: tmp1,tmp2\n---\n# content",
        )
        .unwrap();

        let parsed = parse_block_file_with_template_dir(&block_path, &template_dir).unwrap();
        // config にテンプレート名が保持される
        assert_eq!(
            parsed.config.templates,
            Some(vec!["tmp1".to_string(), "tmp2".to_string()])
        );
        // 本文は「テンプレート1 → テンプレート2 → content」の順に連結される
        assert_eq!(
            parsed.body,
            "# テンプレート1\n\n# テンプレート2\n\n# content"
        );
    }
}
