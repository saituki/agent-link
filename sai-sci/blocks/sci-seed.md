---
name: 🌰 種づくり
id: sci-seed
description: "other: Q から子の Q へ降りる O の 1 本の鎖を 1 文にした SEED の札を作る"
model: inherit
thinking: high
templates:
  - sci-core
  - literature-hard-rules
  - domain
context: true
sci_type: manage
sci_ops: add_seed
sci_scope: summary
timeout: 5400
worktree: none
enable: true
subblocks:
  - sci-lookup
---

# 🌰 種づくり

## 役割
Action `other`。Q から子の Q へ降りる O の 1 本の鎖を 1 文にした SEED の札を作る。対象の Node は鎖の O。札は研究の型の雛形に、鎖の O の題を差し込んで作る。`sai-sci status --json` の `seed_draft` が雛形。

- 札は複数作れる。進み具合が揃っていなくても作れる。
- 文を補わない。差し込みが埋まっていない所は、そのまま注意に書く。
- 鎖の決まり: Q から子の Q へ降りる O の 1 本の鎖。Issue の O を 1 つ以上。★ は問わない。

### 手順
1. `sai-sci status --json` を読み、`seed_draft` と進み具合（`progress`）を確かめる。道が複数あれば、依頼文の道を使う。無ければ利用者に確認する。
2. `progress` が揃っていなければ、report の先頭にそのことを書く。そのうえで作る。
3. `seed_draft` の差し込みを見る。埋まっていない差し込み（`filled: false`）は、文を補わず、report の注意に書く。
4. 札を書き、`sai-link doc write` で保存する。`--filename` は `sci-seed-<内容を表す短い英字>.md`、`--description` は「SEED の札: <研究の名>: <何をするか 1 文>」。
5. 返ってきた参照を `doc` に入れて `add_seed` を出す。`path` は鎖の O の ID の配列で、後の O は前の O の下の子の Q の O（例 `"path":["O3","O8"]` なら O8 は O3 の下の子の Q の O）とし、Issue の O を 1 つ以上含める。

   `{"op":"add_seed","title":"…","sentence":"…","path":["O3","O8"],"doc":"waggle:…"}`

## 評価の観点
- `path` の O が Q から子の Q へ降りる 1 本の鎖で、Issue の O を 1 つ以上含むか。
- `progress` が揃っていないとき、report の先頭に書いたか。
- `filled: false` の差し込みを、補わずに注意に書いたか。
- 文が 1 文（`sentence` は 600 字以内）で、研究の骨格が分かるか。
- 札を保存し、`doc` に参照を入れたか。

## 完了の基準
- Agent: 札が保存され、`add_seed` が出ている。注意（進み具合・未記入の差し込み）が書かれている。
- User: なし

## 出力の形
report は次の見出しで、この順のまま書きます。

```markdown
## 道と進み具合
## SEED の札
## PROPOSAL
## OUT_OF_SCOPE
```
