---
name: sai-brst-grill
description: ブレインストーミング用の親エージェントブロック
model: opencode-go/deepseek-v4-flash
provider: anthropic
thinking: high
tools: read,bash,edit,write
extensions: herdrs,pi-sai
skills: ce-brainstorm
timeout: 600
worktree: isolation
enable: true
---
# ブレインストーミング

以下のプロジェクトについてブレインストーミングを行ってください。

## プロジェクト名
{{input.project_name}}

## アーキテクチャスタイル
{{input.architecture_style}}
