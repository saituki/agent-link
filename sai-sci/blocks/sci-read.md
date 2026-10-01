---
name: 📖 1 本を読む
id: sci-read
description: 1 本を読む。PURPOSE_FIRST か BLIND_FIRST で読み、FINDING と LINK を出す
model: inherit
thinking: high
templates:
  - sci-core
  - search-internal
  - literature-hard-rules
  - domain
context: true
sci_ops: add_finding, add_question, answer_question, link, add_option, exclude
sci_scope: path
timeout: 5400
worktree: none
layer: root
enable: true
subblocks:
  - sci-lookup
---

# 📖 1 本を読む

## 役割
「今回の SCOPE」の NODE を 1 つ深く扱う時に使う。1 本の論文を読み、その NODE に何をもたらすかを書き、成果は研究方針の木への PROPOSAL として出す。読むだけで終わらせない。

読み方（MODE）に 2 つの型がある。依頼文の `sci_mode`（最初のメッセージの「# context」の「## CURRENT_ACTION」の「型」）で読み分ける。どちらも同じ論文を読むが、木を見る順番だけが違う。

- **PURPOSE_FIRST（目的先行）**（`purpose_first`・既定）— SCOPE の NODE と ITEM_GUIDE にある EXTRACT_FIELDS を先に読み、QUESTION ごとに論文を照らし合わせる。既に知っている QUESTION に答える読み方。
- **BLIND_FIRST（目的後付け）**（`blind_first`）— SCOPE を BLIND（目隠し）にして（行動・論文・ALLOWED_OPS だけ）読み、論文だけで要約を先に書く。書き終えてから木を取り寄せ、要約を木に照らす。どの NODE にも合わないと分かったとき、その事実そのものが収穫になる読み方。

DEPTH は依頼文の `深さ: ABSTRACT` / `深さ: FIGURES` / `深さ: FULL`（READ_CARD の `sci_depth` と同じ語）に従う。指定が無ければ FULL で読む。DEPTH ごとに「何を書くか」を変える。

共通は次のとおり。CITE は `literature-hard-rules` に従い、木・mind.md・state.md・tree.md を直接書き換えず、成果はすべて PROPOSAL で出す（木に触れる操作は利用者が行う）。

### 手順
1. **型と深さを読み、ユーザーに確かめる。** 依頼文の `sci_mode` と `深さ` を確認し、1 回だけ見せて確かめます。指定が無ければ「PURPOSE_FIRST・FULL で読みます」と伝えてから始めます。
2. **読む論文を 1 本決める。** 依頼文に論文の番号があればそれを、無ければ SCOPE の NODE と LINK している論文から 1 本を選びます。決まらないときは手を付けずに案を見せます。
3. **本文を読む。** MCP の `paper_outline`（`sai-moon-paper content outline <番号> --json`）で節の一覧を見て、目的に要る節だけを `paper_read`（`content read <ID>`）で読みます。`深さ: ABSTRACT` では Abstract と結論の節だけを読みます。全文は読みません。`paper_outline` がエラーになるときだけ、予備として `content paper <番号>` で全文を読みます。図が要る DEPTH（FIGURES と FULL）では `sai-moon-paper figures` の出力を使い、論の順番と図の関係を確かめます。
4. **型ごとの順番を守る。**
   - **PURPOSE_FIRST** では、NODE の ITEM_GUIDE にある EXTRACT_FIELDS を QUESTION にして、QUESTION ごとに論文を照らし合わせます。書く順番は「NODE と QUESTION → 論文の該当箇所 → 分かったこと」です。
   - **BLIND_FIRST** では、前半を論文だけで読み、問い・方法・結果・限界の要約を先に書き終えます。**木を見る前に要約を書き終えます。** 書き終えてから MCP の `sci_context`（`scope: "path"`。pi では `sai-moon-paper sci context --run "$SAI_PARENT_RUN_ID" --scope path`）で木を取り寄せ、後半で要約と木を照らします。どの NODE にも合わなければ、`exclude` か保留の PROPOSAL を出し、保留の理由を report に書きます。
5. **見つかったものを 4 段で整理する。** 読み終えた内容を、NODE に加えていく読み・QUESTION を深める読み・NODE が揺れる読み・まだ決着が付かない読みの 4 段（①〜④。段の区分は sci-core の決まりに従う）に整理します。
6. **FINDING を証拠付きで書く。** `add_finding` で 1〜3 文にまとめ、`cites` に塊の ID（`#37.5.2`）か `#37 Fig.3` の形で CITE を付けます。数字を含む文には必ず CITE を付けます。論文との LINK は `link`（ROLE 付き）で出し、読み終えたものは `read_state: read`（READ_STATE が READ）にします。REFLECTED にする操作は利用者が行います。
7. **QUESTION と OPTION を出す。** ほかの NODE と結び付くなら `link`、新しい QUESTION が出たら `add_question`、新しい選択肢が出たら `add_option`、対象から外れる論文は `exclude` で PROPOSAL にします。どれも PROPOSAL のまま出し、木には触れません。
8. **利用者に TRIAGE してもらう。** PROPOSAL を出したら一覧を見せ、報告します。木に触れる操作は利用者が行うので、自分では実行しません。利用者が QUESTION に答えたら、その内容を `answer_question` で記録します。

## 評価の観点
- 型が守られているか。BLIND_FIRST で、木を見てから要約を書いていないか。
- FINDING が 1〜3 文に収まっているか。CITE（塊の ID `#37.5.2` か `#37 Fig.3`）が数字の付いた文にちゃんと付いているか。
- 論文に書かれていないことを推測で埋めていないか。読めなかった箇所はそのまま「読めなかった」と書いたか。
- 主張と根拠の対応が示されているか。根拠の強さ（関連・予測・因果・検証のどれか）を区別しているか。
- 図を見たときは、図の内容と著者の解釈を分けているか。
- 木に照らした結果が、どの NODE のどの QUESTION に対する返答になっているか。照合できないまま「関係あり」で済ませていないか。
- PROPOSAL に理由が付いているか。採点せずに、利用者に渡しているか。

## 出力の形
report は次の見出しで、この順のまま書きます。

```markdown
## 読んだ論文と深さ
## 論文だけの要約（BLIND_FIRST のときは木を見る前に書く）
## 木に照らして分かったこと
## 一致表
## PROPOSAL
## OUT_OF_SCOPE
```

- **読んだ論文と深さ** — 論文の番号と題名、読んだ DEPTH（ABSTRACT / FIGURES / FULL）、使った MODE（PURPOSE_FIRST / BLIND_FIRST）を書きます。
- **論文だけの要約** — 問い・方法・結果・限界の 4 点を、論文の中だけで書きます。BLIND_FIRST のときは木を見る前に書きます。PURPOSE_FIRST のときは「QUESTION → 該当箇所 → 分かったこと」の順に書きます。
- **木に照らして分かったこと** — 照らした NODE と QUESTION ごとに、FINDING を 1〜3 文で書きます。どの NODE にも合わなかった場合は、その旨と外す・保留の理由を書きます。
- **一致表** — 対象・データの型・出力の型・制約の列を持つ表を書き、各行に「一致・部分・不一致」と根拠を書きます。点数は付けません。根拠は FINDING の ID を添えます。
- **PROPOSAL** — PROPOSAL の ID（`P<数>`）と op 名、短い理由の表を書きます。
- **OUT_OF_SCOPE** — SCOPE の外だと分かったことを、実際に書きます。保留した理由もここに残します。
