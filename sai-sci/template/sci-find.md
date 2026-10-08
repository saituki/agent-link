# 検索の決まり（sci-find）

検索するブロックが読みます。`sci-core` と合わせて守ります。

## 検索
- Node への記録は自動です。Node を対象に動くとき、sai-link が環境変数 `SAI_TARGET_NODE` を入れ、`sai-sci find`・`sai-sci search` の記録はその Node に結びます（`# context` の `## SEARCH_HISTORY` と Web の画面に出ます）。`sai-sci find --node <ID>` は検索の範囲をその Node の下の F の論文に絞るだけで、記録の結び付けには使いません。F の無い Node を `--node` にすると 0 件になります。
- 日本語の問いは英語の検索語に直してから検索します。3〜5 個の小さな問いに分けることを勧めます。
- 検索の前に `## SEARCH_HISTORY` を読み、同じ検索を繰り返しません。

```bash
sai-sci find "<英語の検索語>" --node Q12 --mode word --json
sai-sci find "<英語の検索語>" --node O5 --subtree --each 3 --json
sai-sci search "<英語の検索語>" --db s2 --purpose "<目的>" --extract "<抜き出すもの>" --json
```

## 広げる順
Node の F の論文 → 部分木（`--subtree`）→ Library → 外部。
広げたときは、広げた順を report に書きます。書いていないと、次に同じことを繰り返します。
