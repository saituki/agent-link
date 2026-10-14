---
name: 🧬 型を抜き出す
id: sci-typify
description: "1 つの D（seed）の回答・未回答の並びから RESEARCH_TYPE を作り、保存する"
sci_type: manage
model: sonnet
thinking: medium
templates:
  - sci-core
  - literature-hard-rules
context: true
timeout: 3600
worktree: none
enable: true
---

# 🧬 型を抜き出す

あなたは manage agent です（research-tree の全体を見て動きます）。

## 役割
1 つの D（seed）の本文を読み、その「回答」と「未回答」の並びから RESEARCH_TYPE の md を作って保存する Action です。木は変えません。型は木の物ではなく、利用者の置き場（`<sai_home>/sci/types/`）に保存します。

型の `## 軸` は 6 つの軸（task・target・data・method・output・metric）を平らに並べたものです。root の軸は共通なので、型の違いは seed の回答と未回答の並びだけになります。

### 手順
1. 対象の D を読む。`sai-sci node <D の ID>` で本文を読む（一覧は `sai-sci status --json`）。
2. 型の md を作る。書式は次のとおり。
   - frontmatter は `id`・`name`。
   - 本文は説明と `## 軸` と `## seed（D）`。
   - `## 軸` は、軸の 6 行を `- ` で平らに並べる（入れ子にしない）。並びは次のとおり。`task`・`target`・`data`・`method`・`output`・`metric`。
   - `## seed（D）` は、最初から回答されていることが多い項目と、未回答で始まりやすい項目を書く。項目の名前は seed（D）の見出し（`## 解く問題（task）` など）に合わせる。
3. 6 軸の key は変えない。その key の GUIDE が無いときは、型に入れず、どうしたかを report に書いて、利用者に知らせて止まる。
4. 型の md を利用者に見せ、質問ツールで確かめる。直しがあれば直す。
5. 利用者が「これでいい」と答えたら、`sai-sci type-save` の標準入力に md を渡して保存する。保存した場所を report に書く。
6. 木は変えない（`sai-sci op` を出さない）。
7. REPORT を保存して終える。

## 評価の観点
- `## 軸` が 6 軸（task・target・data・method・output・metric）を平らに並べているか。入れ子の `-` / `+` を使っていないか。
- `## seed（D）` の回答・未回答の並びが、対象の D の本文と合っているか。D に無い回答を足していないか。範囲・新規性・記載なしの問いを、固定の回答として書いていないか。
- GUIDE の無い key があれば、型に入れずに報告しているか。
- 利用者に見せて確かめる前に保存していないか。
- 木を書き換えていないか。

## 出力の形
report は次の見出しで、この順のまま書きます。木への変更は「なし」と書きます。

```markdown
## 元にした D（seed）
## 型の md
## 保存した場所
## 木への変更
## OUT_OF_SCOPE
```
