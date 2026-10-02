---
name: 🧬 型を抜き出す
id: sci-typify
description: その他: 今の木から Q の階層を抜き出し、研究の型として保存する
model: inherit
thinking: high
templates:
  - sci-core
  - literature-hard-rules
context: true
sci_scope: summary
sci_action: other
sci_support: false
sci_done_agent: report, review
sci_done_user:
timeout: 3600
worktree: none
enable: true
---

# 🧬 型を抜き出す

## 役割
今の木（forward でも backward でも）の Q の階層を読み、研究の型の md を作って保存する Action です。木は変えません。型は木の物ではなく、利用者の置き場（`<sai_home>/sci/types/`）に保存します。

### 手順
1. `sai-moon-paper sci status --json` を読む。`nodes` の Q と、各 Q の `guide` を取り出す。
2. 型の md を作る。書式は次のとおり。
   - frontmatter は `id`・`name`・`root`。
   - `## Q` に Q の階層を、入れ子の `-` / `+` で書く。
   - `## SEED` に、種の下書きの形を書く。
3. ガイドの無い Q は、近いガイドの key を使うか、key を作らずに型に入れない。どちらにしたかを report に書く。
4. 型の md を利用者に見せ、質問ツールで確かめる。直しがあれば直す。
5. 利用者が「これでいい」と答えたら、`sai-moon-paper sci type-save` の標準入力に md を渡して保存する。保存した場所を report に書く。
6. PROPOSAL は出さない。

## 評価の観点
- Q の階層が今の木と一致しているか。落とした Q と、その理由が書いてあるか。
- ガイドの無い Q の扱い（key を借りた・型に入れない）が Q ごとに書いてあるか。
- 利用者に見せて確かめる前に保存していないか。
- 木を書き換えていないか。

## 完了の基準
- Agent: 利用者が確かめた型の md を `sci type-save` で保存したら終わり。PROPOSAL の仕分けは要らない。
- User: なし

## 出力の形
report は次の見出しで、この順のまま書きます。PROPOSAL は「なし」と書きます。

```markdown
## 元にした木
## 型の md
## 保存した場所
## PROPOSAL
## OUT_OF_SCOPE
```
