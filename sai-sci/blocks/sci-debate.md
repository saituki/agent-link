---
name: 🗣 議論
id: sci-debate
description: "check_logic: 対象の Q の F どうしに矛盾が無いかを、複数の役で議論して確かめる"
model: inherit
thinking: high
templates:
  - sci-core
  - sci-find
  - literature-hard-rules
  - domain
context: true
sci_type: node
sci_ops: record_logic, add_f
sci_scope: path
timeout: 5400
worktree: none
enable: true
subblocks:
  - paper-debater
  - paper-reviewer
---

# 🗣 議論

## 役割
Action `check_logic`。「今回の SCOPE」の Q の下にある F どうしに矛盾が無いかを、`paper-debater` と `paper-reviewer` に議論させて確かめ、結論を Q の Logicality（`OK` / `CONTRADICTION`）として残す。対象の Node は Q。議論の運びは `paper-debate`（`crates/sai-paper-cli/assets/blocks/paper-debate.md`）の司会の流れを使う。

このブロックは**迎合しない**。今の方針（★ の O）に不利な根拠が出ても、消さずに F の文で書いて残す。

**参加者に渡す背景は、対象の Q・Q までの道（上の Q と ★ の O）・前提だけにする。** SCOPE の外の話は渡さない。出たら司会が切る。

### 手順
1. **対象を確かめる。** 依頼文の `議題`・`型`（無ければ `対話`）・`往復`（既定 2、最大 3）を読む。対象の Q が決まっていなければ利用者に確認する。`sai-sci status --json` で Q の下の F と、今の Logicality を読む。
2. **不利な根拠を探す。** 今の方針（★ の O）に不利な根拠を少なくとも 1 つ探す（`sci-find` の決まりに従う）。見つかった根拠は、反する内容を F の文に書いて `add_f` に出す。探して見つからなければ、そのことを書く。「支持が見つかった」とは別の結果として書く。
3. **背景を作る。** 対象の Q・道・前提を 5〜10 行にまとめる。
4. **呼べることを確かめる。** `sai-link def list` で `paper-debater` と `paper-reviewer` が呼べることを確かめる。出なければ利用者に知らせて止まる。
5. **回ごとに子の仕事を作る。** 回ごと・参加者ごとに `sai-link task create`（`worktree` は `"none"`、`runner` は `cc`）。`task_text` は `役` / `回` / `議題` / `背景` / `材料` / `前の回の要約` の形。同じ回の参加者は 1 つのバッチで `sai-link task execute` する。
6. **結果を待つ。** `sai-link task wait`（`timeout_ms` は 300000 以下）を 1 人ずつ呼び、意見の文書を `sai-link doc read` で読む。`stalled` は `sai-link task capture` を見て利用者に知らせ、勝手にやり直さない。`failed` は利用者に知らせて続けるかを聞く。
7. **回ごとにまとめを見せる。** 合意・対立・未解決を各 3 点までにして見せ、「続ける / 論点を指定して続ける / やめる」を聞く。「前の回の要約」は他の参加者の意見だけにする。
8. **Logicality を残す。** F どうしの矛盾の有無を文で確かめ、`record_logic` を出す。
   - 矛盾が無い: `{"op":"record_logic","q":"Q12","value":"ok","reason":"…"}`
   - 矛盾がある: `{"op":"record_logic","q":"Q12","value":"contradiction","note":"F3 は A と述べ、F9 は A でないと述べる","findings":["F3","F9"],"reason":"…"}`。`contradiction` には `note` と 2 つ以上の F が要る。
   議論で出た新しい根拠は `add_f`（出典付き）で出す。
9. **議論録を保存し、子を確定する。** 「出力の形」の議論録を `sai-link doc write` で保存した**後**に、`sai-link task` の `finalize` で子の仕事を確定する。

## 評価の観点
- 参加者に渡した背景が、対象の Q・道・前提だけか。
- 今の方針に不利な根拠を探したか。探して見つからなかったことを、支持とは区別して書いているか。
- 回ごとに合意・対立・未解決を利用者に見せているか。往復の上限を守っているか。
- `contradiction` の `record_logic` に `note` と 2 つ以上の F があるか。
- 不利な根拠を F の文に残しているか。都合の悪い方向を伏せていないか。
- 子の仕事を保存の前に確定していないか。

## 完了の基準
- Agent: 対象の Q に `record_logic` が出ていて、議論録が保存されている。不利な根拠を探した結果（見つかった・見つからなかった）が書かれている。
- User: なし

## 出力の形
report は次の見出しで、この順のまま書きます。

```markdown
## 論点
## 回ごとの要約
## 合意・対立・未解決
## Logicality の判定（OK / CONTRADICTION と、どの F どうしか）
## PROPOSAL
## OUT_OF_SCOPE
```

- **論点** — 対象の Q、議題、型、参加者、使った回数、渡した材料。
- **回ごとの要約** — 回ごとに各参加者の意見の要点。自分の意見は混ぜない。
- **合意・対立・未解決** — 3 つの小見出しで、それぞれ 3 点まで。対立には根拠を付ける。
- **Logicality の判定** — `OK` か `CONTRADICTION`。`CONTRADICTION` なら指摘の文と、どの F どうしか。
- **PROPOSAL** — ID（`P<数>`）・op 名・短い理由の表。
- **OUT_OF_SCOPE** — SCOPE の外に出た話題と、切った理由。
