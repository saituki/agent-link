---
name: legacy-inputs-block
description: 旧 inputs / output が残ったブロック（読み込めることの確認用）
model: anthropic/claude-sonnet-4-5
inputs:
  - id: project_name
    type: text
    label: プロジェクト名
    required: true
  - id: design_doc
    type: file_token
    label: 設計ドキュメント
    source_blocks:
      - sai-plan-synth
output:
  format: markdown
  waggle_tokenize: true
  description: 旧出力設定
---
# 旧定義ブロック

旧 `inputs:` / `output:` が残っていても無視されて読み込めます。
