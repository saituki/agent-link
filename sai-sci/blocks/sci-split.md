---
name: ✂ Q を分ける
id: sci-split
description: "Q を分ける: 対象の Q を、ガイドの Child Q を手がかりに子の Q に分ける"
model: inherit
thinking: high
templates:
  - sci-core
  - literature-hard-rules
  - domain
context: true
sci_ops: add_q
sci_scope: path
sci_action: split
sci_support: false
sci_done_agent: report, review, q_proposed
sci_done_user: triaged
timeout: 5400
worktree: none
enable: true
subblocks:
  - sci-lookup
---

# ✂ Q を分ける

## 役割
`# context` の `## TARGET_NODE` の Q を、子の Q に分ける Action です。対象は Q です。

- 子の Q を持った Q には、O を置けなくなります。すでに O がある Q は分けません。分けたいときは、その理由を report に書いて利用者に返します。
- 抽象から具体へ分けます。1 つの Q には 1 つの決めることだけを書きます。
- `## GUIDE` の Child Q を手がかりにします。ガイドに無い分け方が要るときは、なぜ要るかを書きます。
- 論文を読み込みません。根拠を調べたいときは、`sci-lookup` を子として呼びます。

### 手順
1. `# context` の `## TARGET_NODE`・`## UPSTREAM_PATH`・`## GUIDE` を読む。対象の Q に O がすでにあるなら、ここで止まって report に書く。
2. 対象の Q が決めることを 1 文で言い直す。複数の決めることが混ざっているところを見つける。
3. `## GUIDE` の Child Q を、候補として並べる。対象の Q に合うものだけ残す。足りない分は、抽象から具体の順で自分の案を足す。
4. 子の Q を 2〜5 個に絞る。各子に「文・ガイド（key。無ければ無し）・なぜ要るか」を付ける。
5. 子の Q を `add_q` で PROPOSAL に出す。親は対象の Q。
6. 一覧を利用者に見せて報告する。仕分けは利用者が行う。

## 評価の観点
- 子の Q が 1 つに 1 つの決めることだけを持つか。
- 抽象から具体へ分かれているか。同じ高さで重なる Q を出していないか。
- ガイドの Child Q を使ったものに、ガイドの key を付けたか。
- すでに O がある Q を分けていないか。
- 「なぜ要るか」が、上の Q を決めるのに必要な理由として書けているか。
- 子の Q が 6 個を超えて散らかっていないか。

## 完了の基準
- Agent: 子の Q を 2〜5 個、`add_q` で PROPOSAL に出したら終わり。
- User: 出した Q の PROPOSAL をすべて仕分けたら終わり。

## 出力の形
report は次の見出しで、この順のまま書きます。

```markdown
## 対象の Q と分け方
## 子の Q の案（表: 文 / ガイド / なぜ要るか）
## PROPOSAL
## OUT_OF_SCOPE
```
