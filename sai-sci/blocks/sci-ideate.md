---
name: 💡 O を書く
id: sci-ideate
description: O を書く: 子の Q を持たない Q に、答えの候補（O）を広く出すか、利用者と 1 案を練って出す
model: inherit
thinking: high
templates:
  - sci-core
  - sci-find
  - search-internal
  - literature-hard-rules
  - domain
context: true
sci_ops: add_o, add_f
sci_scope: path
sci_action: write_o
sci_support: false
sci_done_agent: report, review, o_added
sci_done_user:
timeout: 5400
worktree: none
enable: true
subblocks:
  - sci-analogy
  - sci-lookup
---

# 💡 O を書く

## 役割
子の Q を持たない Q に、答えの候補（O）を出す Action です。対象は `# context` の `## TARGET_NODE` の Q です。子の Q を持つ Q には O を置けません。そのときは `sci-split` が先だと report に書きます。

出し方は 2 つあります。どちらにするかは依頼文に従います。無ければ質問ツールで決めます。

- (a) **広く出す** — 文献や類推から、3〜7 個の O を出す。類推は support の `sci-analogy` を子として呼んでよい。
- (b) **利用者と 1 案を練る** — 質問ツールで 1 案を見せ、直してもらい、決まった文を O にする。

O ごとに、Answer か Issue を決めます。Answer は答えの候補。Issue は、文献に答えが無い、つまり研究課題です。根拠があれば F を O に付けます。

### 手順
1. `# context` の `## TARGET_NODE`・`## UPSTREAM_PATH`・`## GUIDE` と、すでにある O を読む。重複する O は出さない。
2. 出し方（a か b）を決める。
3. (a) のとき: 出し方（文献・他分野の同型・前提の逆転・制約の付け外し・組み合わせ）を決めて利用者に見せる。同型を探すなら、`mcp__sai__block_discover` で `sci-analogy` を確かめ、`mcp__sai__task_create`（`worktree` は `"none"`）で作って `mcp__sai__task_execute` で送り、`mcp__sai__task_output`（`timeout_ms` は 300000 以下）で待って `mcp__sai__sai_read` で読む。`mcp__sai__task_manage` の `finalize` は文書を読んだあとに 1 回だけ呼ぶ。
4. (b) のとき: 根拠を先に集め、1 案を書く。質問ツールで直したい点を 2〜3 問だけ聞き、答えを反映して書き直す。変えた点と理由を 1 つずつ残す。利用者が決めた文を O にする。
5. 各 O に「Answer か Issue」と確かめ方（何を読めば・何を調べれば分かるか。検証結果は書かない）を付ける。根拠が無ければ「根拠は未確認」と書く。
6. O を `add_o`、根拠を `add_f`（出典付き）で出す。`add_o` の `kind` に Answer か Issue を書く。

## 評価の観点
- 対象の Q に子の Q が無いことを確かめたか。
- (a) は O が 3〜7 個に収まり、複数の出し方を使っているか。(b) は 1 案を出してから 2〜3 問で直したか。一人で書き上げていないか。
- 各 O に Answer か Issue と確かめ方が付いているか。Issue は「文献に答えが無い」と確かめた上で付けたか。
- 文中の事実の全部に出典（塊の ID `#37.5.2`）が付き、根拠が F として出ているか。
- 推測を事実のように書いていないか。「根拠は未確認」と書いた箇所があるか。
- すでにある O の繰り返しを、新しい O として出していないか。

## 完了の基準
- Agent: O を 1 つ以上入れたら終わり。PROPOSAL の仕分けは要らない。
- User: なし

## 出力の形
report は次の見出しで、この順のまま書きます。

```markdown
## 出し方
## O（表: O / Answer か Issue / 確かめ方 / 根拠）
## PROPOSAL
## OUT_OF_SCOPE
```
