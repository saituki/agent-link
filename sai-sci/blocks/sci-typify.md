---
name: 🧬 型を抜き出す
id: sci-typify
description: "今の木から Q の階層を抜き出し、RESEARCH_TYPE として保存する"
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
今の木（forward でも backward でも）の Q の階層を読み、RESEARCH_TYPE の md を作って保存する Action です。木は変えません。型は木の物ではなく、利用者の置き場（`<sai_home>/sci/types/`）に保存します。

### 手順
1. `sai-sci status --json` を読む。`nodes` の Q と、各 Q の `guide` を取り出す。
2. 型の md を作る。書式は次のとおり。
   - frontmatter は `id`・`name`。
   - 本文は説明と `## Q`。`## Q` に Q の階層を、入れ子の `-` / `+` で書く。
3. GUIDE の無い Q は、近い GUIDE の key を使うか、key を作らずに型に入れない。どちらにしたかを report に書く。
4. 型の md を利用者に見せ、質問ツールで確かめる。直しがあれば直す。
5. 利用者が「これでいい」と答えたら、`sai-sci type-save` の標準入力に md を渡して保存する。保存した場所を report に書く。
6. 木は変えない（`sai-sci op` を出さない）。
7. REPORT を保存して終える。

## 評価の観点
- Q の階層が今の木と一致しているか。落とした Q と、その理由が書いてあるか。
- GUIDE の無い Q の扱い（key を借りた・型に入れない）が Q ごとに書いてあるか。
- 利用者に見せて確かめる前に保存していないか。
- 木を書き換えていないか。

## 出力の形
report は次の見出しで、この順のまま書きます。木への変更は「なし」と書きます。

```markdown
## 元にした木
## 型の md
## 保存した場所
## 木への変更
## OUT_OF_SCOPE
```
