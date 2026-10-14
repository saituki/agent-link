# 検索の決まり（sci-find）

検索するブロックが読みます。`sci-core` と合わせて守ります。論文を探す手順は、この文書だけに書いてあります。sai-paper の検索の template は使いません。

## 道具の分担
- 論文を探す: `sai-sci find`（Library の中）・`sai-sci search`（外部の論文 DB）。
- 探した後: `sai-paper search-history <log_id>`・`sai-paper expand <論文の番号>`・`sai-paper neighbors <論文の番号>`・`sai-paper paper show <論文の番号>`・`sai-paper index status`。
- 中身を読む: `sai-paper content outline <論文の番号>` で節を見て、`sai-paper content read <ID>` で読みます。

## 検索
- Node への記録は自動です。Node を対象に動くとき、sai-link が環境変数 `SAI_TARGET_NODE` を入れ、`sai-sci find`・`sai-sci search` の記録はその Node に結びます（`# context` の `## SEARCH_HISTORY` と Web の画面に出ます）。`sai-sci find --node <ID>` は検索の範囲をその Node の下の F の論文に絞るだけで、記録の結び付けには使いません。F の無い Node を `--node` にすると 0 件になります。
- Library 全体を探すときは `--node` を付けません。
- 日本語の問いは英語の検索語に直してから検索します。3〜5 個の小さな問いに分けます。日本語の語は `--mode word` では当たりません。
- 検索の前に `## SEARCH_HISTORY` を読み、同じ検索を繰り返しません。

```bash
sai-sci find "<英語の検索語>" --node A12 --mode word --json   # A12 の F の論文の中
sai-sci find "<英語の検索語>" --node A5 --subtree --each 3 --json   # A5 の部分木
sai-sci find "<英語の検索語>" --json   # Library 全体
sai-sci search "<英語の検索語>" --db s2 --db pubmed --limit 30 --purpose "<目的>" --extract "<抜き出すもの>" --json
```

## 検索語の作り方
- 問いを 3〜5 個の小さな問いに分け、それぞれ英語の検索語にします。
  - 例: 「てんかんの脳波解析で使われる深層学習の手法は？」→ `epilepsy EEG deep learning`・`seizure detection neural network`・`epilepsy EEG feature extraction`
- 外部の検索では、各検索語に同義語（`seizure disorder`）と上位語（`machine learning`）を添えます。
- DB ごとの書き方:
  - Semantic Scholar・arXiv: 自然言語で書きます。同義語を複数試します。
  - PubMed: `--raw-query` で Boolean を渡します。`[tiab]`（題と抄録）・`[mh]`（MeSH）・`[au]`（著者）・`[pt]`（出版種別）・`[dp]`（出版日 `YYYY:YYYY`）を使います。例: `(glioma[tiab] OR glioblastoma[mh]) AND ferroptosis[tiab]`
  - bioRxiv / medRxiv: キーワードを並べます（AND で結ばれます）。例: `glioma ferroptosis cell death`
- DB は `--db` で指定します（`s2`・`arxiv`・`pubmed`・`biorxiv`・`openalex`）。`domain` の指示があれば、それに従います。`--db` を省くと設定の既定を使います。

## 内部と外部の使い分け
- **Library（`sai-sci find`）**: 取り込み済みの論文・ノートです。外部を検索する前に、先に Library を当たります。
- **外部（`sai-sci search`）**: Library に無い論文の候補を集めます。`--purpose` と `--extract` を必ず付けます。
- 外部の検索で見つけた候補は Library に入りません。取り込みを勧める候補は `sai-paper suggest add` で Suggest に残します（題と理由を付ける）。取り込みは利用者が Web の Suggest から行います。
- 索引の状態は `sai-paper index status` で見ます。`indexes.<名前>.chunks` と `chunks_with_embeddings` が 0 なら、索引は作られていません。エージェントは自分で作らず、利用者に `sai-paper index build` を頼みます。
- `--evidence` は費用がかかります。1 回の作業で 3 回までです。

## 結果の読み方
`hits` の各項目は次のとおりです。
- `score`: 相対的な目安です。絶対値として読みません。上位ほど関連が強いとみます。
- `paper_id`: 論文の番号です。同じ `paper_id` の断片は 1 つにまとめます。
- `source_kind`: `abstract`（抄録）・`body`（本文）・`note`（ノート）・`document` のどれかです。同じくらいなら `body` を優先します。
- `title`・`authors`・`year`・`text`・`md_path`: 返り値のとおりに写します。
- 複数の検索語で出た論文は関連が強いので、上に置きます。

## 探した後
- 記録を読み直す: `sai-sci find`・`sai-sci search` の返りの `log.id` を使い、`sai-paper search-history <log_id>` で中身を見ます。前の検索を繰り返す前に読みます。
- 近い論文を広げる: `sai-paper neighbors <論文の番号> --json` で、その論文の近くの断片と関連論文を見ます。
- 引用を 1 段たどる: `sai-paper expand <論文の番号>... --json` を、関係が深い 2〜3 本に使います（既定は `--direction both`・`--limit 20`）。結果は search_history に 1 回として残ります。
- 論文の情報: `sai-paper paper show <論文の番号>` で、Library の 1 件を確かめます。番号が分かっているときは、題名の検索より先にこれを使います。
- 番号が分からないとき: `sai-sci find "<題名の語>" --json`（`--node` を付けない）で Library を探します。

## 見つからないとき
- Library にも外部にも直接の論文が無いときは、「見つからなかった」と書き、探した語と DB を書きます。
- いちばん近い論文の番号を 1〜3 本挙げ、`sai-paper expand <番号>` を次の手として報告に書きます。
- 答えが Library の外で要るときは、`[GAP: 推奨検索: <英語の検索語>]` を残します。

## 広げる順
Node の F の論文 → 部分木（`--subtree`）→ Library（`--node` なし）→ 外部（`sai-sci search`）。
広げたときは、広げた順を report に書きます。書いていないと、次に同じことを繰り返します。

## 文献の書き方
- 題・著者・年・DOI は、`sai-sci find`・`sai-sci search`・`sai-paper` の返り値に出たものだけを書きます。捏造しません。
- 取れなかった項目は、`DOI [利用不可]` のように明示します。
