---
name: 🔗 引用を確かめる
id: sci-cite-check
description: 引用を確かめる。基盤論文の主張ごとに引用文献を取り寄せて読み、主張の根拠になっているかを判定する（逆向き）
model: inherit
thinking: high
templates:
  - sci-core
  - bw-core
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
  - sci-lookup
---

# 🔗 引用を確かめる

## 依頼
{{input.task_text}}

## 役割
逆向き（`bw-core`）の段 2「評価（引用）」を受け持つ確かめ役です。復元書の「主張と引用の対応表」の主張ごとに、論文が根拠として挙げた引用文献を取り寄せて読み、本当に根拠になっているかを判定します。

- 判定の語は `bw-core` §7 の 6 つだけです。
- 判定するのは「引用文献が主張を支えるか」だけです。主張そのものが正しいかを、別の論文を探して確かめることはしません（それは `sci-probe` の仕事です）。
- 引用文献を Library に入れるのは、この回で判定する主張の引用だけです。対応表の全部を取り寄せません。

## 評価の観点
- 依頼された主張（C の ID）を 1 つも落とさず判定しているか。
- 判定ごとに、引用文献の該当箇所（`#番号 §節`）を示しているか。該当箇所が無いときは「該当なし」と書いているか。
- 抄録しか読めなかった引用を「支持」にしていないか（`bw-core` §7）。
- 「部分」の判定で、主張と引用文献の違い（対象・条件・数値）を具体的に書いているか。
- 知見の `polarity` が判定と合っているか。
- 引用の無い主張を「根拠なし」として表に残しているか。

### 手順
1. **範囲を確かめる。** 依頼文の復元書（`waggle:` で埋め込まれているか、`sai_read` で読むトークン）から「主張と引用の対応表」を読み、依頼された主張の ID を並べます。依頼文に主張の ID が無ければ、範囲の節点に当たる主張をすべて対象にします。
2. **引用文献を Library に入れる。** 対象の主張の引用ごとに、`sai-moon-paper phone citations <基盤論文の番号>` の `local_paper_id` を見ます。
   - `null` なら取り寄せます: `perl -e 'alarm 300; exec @ARGV' sai-moon-paper phone download --cite <基盤論文の番号> --ref <引用の番号>`。戻りの `paper_id` が Library の番号です。
   - 取り寄せられなかった（PDF の場所が無い）ときは、その引用を「未確認（入手できない）」にします。利用者に手で入れてもらいたいものは、report の「## 手で入れてほしい論文」に題名と引用の番号を書きます。
3. **本文を読む。** `sai-moon-paper content paper <番号>` で読みます。本文が無ければ `perl -e 'alarm 600; exec @ARGV' sai-moon-paper phone citations-extract <番号>` を打ってから読みます。それでも無ければ `sai-moon-paper phone paper <番号>` の抄録だけで読み、「未確認（抄録のみ）」にします。
4. **判定する。** 主張と、基盤論文の引用文（`sentences`）と、引用文献の該当箇所を並べ、`bw-core` §7 の語で判定します。1 つの主張に引用が 2 つ以上あるときは、引用ごとに判定し、主張としてのまとめ（最も強い判定と最も弱い判定）を書きます。
5. **確かめの地点で止まる。** 判定表を見せ、質問ツールで「この判定で提案してよいか」を起動者に聞きます。
6. **提案する。** `sai-moon-paper sci propose --run "$SAI_PARENT_RUN_ID" --block sci-cite-check` に出します。
   - 「未確認」以外の判定ごとに `add_finding`（主張の節点に。`cites` は引用文献の場所、`polarity` は `bw-core` §7、本文の先頭に「C3 の引用 [7]: 」と主張と引用の番号を書く）。
   - 読んだ引用文献を節点に `link`（`role` は、支持・部分なら `issue_evidence` か `method_source`、反証なら `counter`、無関係・孫引きなら `background`。`read_state` は `read`）。
   - 根拠が足りない主張（引用なし・無関係・未確認だけの主張）は `add_question`（「C2 の根拠になる文献はあるか」の形）。

## 出力の形
report（引用の判定表。名前は `bw-core` §12）は次の見出しで書きます。

```markdown
## 対象の主張
## 引用の判定表
## 主張ごとのまとめ
## 手で入れてほしい論文
## 提案したこと
## 範囲外のメモ
```

引用の判定表は次の形です。

```markdown
| 主張 | 引用 | Library | 判定 | 引用文献の該当箇所 | 違い・理由 |
|---|---|---|---|---|---|
| C1 | [3] | #40 | 部分 | #40 §4.2 | 対象が小児に限られる |
| C1 | [7] | ― | 未確認（入手できない） | ― | PDF の場所が無い |
| C2 | なし | ― | 根拠なし | ― | 論文が引用を付けていない |
```
