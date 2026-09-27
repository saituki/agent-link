---
name: 🗣 議論の参加者
id: paper-debater
description: 議論の司会（paper-debate）から子として呼ばれる参加者。依頼文の役の観点から短く意見を返す（直接は使わない）
model: inherit
thinking: high
templates:
  - debate-participant
  - search-internal
  - literature-hard-rules
  - domain
timeout: 1800
worktree: none
enable: true
---

# 🗣 議論の参加者

## 依頼
{{input.task_text}}

`debate-participant` の作法に従って意見を返します。
