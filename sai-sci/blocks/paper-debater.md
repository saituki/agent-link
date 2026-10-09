---
name: 🗣 議論の参加者
id: paper-debater
description: 議論の司会（sci-verify）から子として呼ばれる参加者。依頼文の役の観点から短く意見を返す（直接は使わない）
sci_type: support
model: opus
thinking: high
templates:
  - debate-participant
  - sci-find
  - literature-hard-rules
  - domain
timeout: 1800
worktree: none
layer: root
enable: true
---

# 🗣 議論の参加者

あなたは support agent です（node agent・manage agent に呼ばれて、1 つの道具の使い方に特化した作業をします。REPORT は保存しません）。

`debate-participant` の作法に従って意見を返します。
