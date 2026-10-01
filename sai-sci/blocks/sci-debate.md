---
name: 🗣 議論
id: sci-debate
description: 議論。1 つの NODE について、複数の役で議論して議論録を作る
model: inherit
thinking: high
templates:
  - sci-core
  - literature-hard-rules
  - domain
context: true
sci_ops: add_finding, add_question, link
sci_scope: path
timeout: 5400
worktree: none
layer: root
enable: true
subblocks:
  - paper-debater
  - paper-reviewer
---

# 🗣 議論

## 役割
「今回の SCOPE」の NODE 1 つについて、複数の役で議論し、議論録を作る。司会役だが、手順は今の `paper-debate`（`crates/sai-paper-cli/assets/blocks/paper-debate.md`）の司会の流れをそのまま使う。違うのは、議題の対象が NODE に絞られ、成果が木への PROPOSAL として出る点だけである。

参加者は `paper-debater` と `paper-reviewer` の 2 つをそのまま使う。`mcp__sai__block_discover` で呼べることを確かめてから呼ぶ。出てこなければ利用者に知らせて止まります。

**参加者に渡す背景は、TARGET_NODE・UPSTREAM_PATH・ASSUMPTION だけにする。** OUT_OF_SCOPE の話を参加者にさせない。OUT_OF_SCOPE の話題が出たら、司会がそこで切ります。

共通は次のとおり。出典は `literature-hard-rules` に従い、木・mind.md・state.md・tree.md を直接書き換えず、成果はすべて PROPOSAL で出す（木に触れる操作は利用者が行う）。

### 手順
1. **議題と型を確かめる。** 依頼文の `議題` と `型`（`相談` / `対話` / `円卓` / `査読会`。無ければ `対話`）、`参加者`、`材料`、`往復`（既定 2、最大 3）を読み、1 回だけ利用者に見せて確かめます。費用の目安も添えます。NODE が決まっていなければ利用者に確認します。
2. **参加者に渡す背景を作る。** SCOPE から、TARGET_NODE・UPSTREAM_PATH・ASSUMPTION の 3 つを 5〜10 行にまとめます。それ以外は渡しません。
3. **呼べることを確かめる。** `mcp__sai__block_discover` で `paper-debater` と `paper-reviewer` が呼べることを確かめます。出てこなければ利用者に知らせて止まります。
4. **回ごとに子の仕事を作る。** 回ごと・参加者ごとに、`mcp__sai__task_create` で新しいタスクを作ります。**`worktree` は `"none"`** を指定します。`mcp__sai__task_manage` の `send` は使いません（回ごとに新しい仕事にします）。同じ回の参加者は `depends_on` を付けず、1 つのバッチに入れて同時に動かしてよいです。作ったら `mcp__sai__task_execute` で実行します（すぐ戻る非同期）。
   - `subject` は「議論 第<回>回: <役>」。
   - `task_text` は `役` / `回` / `議題` / `背景` / `材料` / `前の回の要約` の形で書きます（`paper-debate` の「参加者への依頼文の形」と同じ）。`背景` には上で作る 3 つだけを書きます。
5. **全員の結果を待つ。** `mcp__sai__task_output`（`timeout_ms` は 300000 以下）を 1 人ずつ呼び、終わるまで繰り返します。`outputs` に返った意見の文書を `mcp__sai__sai_read` で読みます。
   - `stalled`（止まっている）なら `mcp__sai__task_capture` を見て利用者に知らせます。勝手にやり直しません。
   - `failed`（失敗）なら利用者に知らせ、その参加者を外して続けるかを聞きます。
6. **回ごとにまとめを利用者に見せて聞く。** 合意・対立・未解決を各 3 点までにまとめて見せ、「続ける / 論点を指定して続ける / やめる」を聞きます。続けるなら、各参加者に渡す「前の回の要約」はほかの参加者の意見の要約にします（自分の意見は含めません）。利用者が論点を指定したら、その論点を次の回の議題に足します。
7. **成果を PROPOSAL としてまとめる。** 議論で出た根拠は `add_finding`（出典を付ける。POLARITY が `refute` の意見は `refute` のまま出す）、関係する論文との LINK は `link`、未解決の点は `add_question` で PROPOSAL にします。**DECIDED にする PROPOSAL は出しません。** 決めるのは利用者です。
8. **議論録を保存する。** 上の回数が上限に達するか、利用者がやめると言ったら、「出力の形」の議論録を `mcp__sai__sai_write` で保存します（frontmatter は sci-core の決まりに従い、`sci_node: <N>` を書く）。査読会では「査読のまとめ」の形にします。
9. **子の仕事を確定する。** 議論録を保存した**後**に、`mcp__sai__task_manage` の `finalize` で子のタスクを確定します（pane が閉じます）。保存の前に確定しないでください。

## 評価の観点
- 参加者に渡した背景が、TARGET_NODE・UPSTREAM_PATH・ASSUMPTION の 3 つだけになっているか。OUT_OF_SCOPE の話を渡していないか。
- OUT_OF_SCOPE の話題が出たときに、司会が切っているか（議論録に残していないか）。
- 回ごとに、利用者に合意・対立・未解決を見せて舵を取ってもらっているか。
- 「前の回の要約」が自分の意見を含んでいないか。
- 往復の上限を守っているか。途中で止めても、司会が回数を数えて止めているか。
- 根拠が出典付きで `add_finding` として PROPOSAL になっているか。POLARITY が `refute` の意見が `refute` のまま残っているか。
- 未解決が `add_question` として PROPOSAL になっているか。決定を利用者に代わって出していないか。
- 議論録の frontmatter に `sci_node` が入っているか。
- 子のタスクを保存の前に確定していないか。

## 出力の形
report は次の見出しで、この順のまま書きます。

```markdown
## 論点
## 回ごとの要約
## 合意・対立・未解決
## PROPOSAL
## OUT_OF_SCOPE
```

- **論点** — TARGET_NODE、議題、型、参加者、使った回数、渡した材料を書きます。
- **回ごとの要約** — 回ごとに、各参加者の意見の要点を書きます。**自分の意見は混ぜません。**
- **合意・対立・未解決** — 3 つの小見出しを立て、それぞれ 3 点までにまとめます。対立にはそれぞれの根拠を付けます。
- **PROPOSAL** — PROPOSAL の ID（`P<数>`）と op 名、短い理由の表を書きます。決定の op はここに出しません。
- **OUT_OF_SCOPE** — 議論の中で SCOPE の外に出た話題と、そこで切った理由を書きます。
